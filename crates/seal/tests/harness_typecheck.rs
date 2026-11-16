//! Every Swift cross-check harness and live rig in the repo compiles the
//! app's own Swift (as checked out, or as of the revision it holds Rust to),
//! cut where it reaches UIKit, with harness files around it, outside Xcode.
//! Nothing compiled them when the app changed, so a renamed type, a moved
//! function or a file one of them needs that isn't on its list stopped a
//! harness building, and nobody heard until someone ran it:
//!
//! - 6d50b781 had AgentNotificationCopy.swift ask AppLanguage; the seal,
//!   install and host-care cross-checks didn't build until B349, and B264's
//!   divergence went unseen meanwhile;
//! - int121 put the shortcuts on the Rust store and had AppTheme.swift ask
//!   `Ink`; the store's harness didn't build until R18 needed its goldens;
//! - de2bba14 deleted the Swift key reader for the Rust one;
//!   scripts/verify/ssh-live didn't build until this test.
//!
//! Here each one is typechecked as it builds. Its own build script runs
//! unchanged, with a `swiftc` first on its PATH that typechecks instead of
//! compiling (a call that emits a module another one imports emits the
//! module alone): the cuts, the checks and the file lists are the build's,
//! without its time or the Rust library. One is built otherwise: the UniFFI
//! smoke test generates its bindings from a built library, so its Swift is
//! typechecked over the app's generated bindings, which
//! `scripts/rust/gen-bindings.sh --check` keeps the same. (The tmux
//! controller's SwiftPM package went with the Swift controller, E3.)
//!
//! The ssh rigs went with the Swift SSH transport (E4-SSH): scripts/verify/
//! ssh-live is gone, and crates/ssh/harness reads every Swift file and
//! package pin as of 2e75a2e5, so nothing the app changes can stop it
//! building.
//!
//! All of them run at once, a few at a time. Where there is no Xcode (not
//! macOS, or a Mac without it), the test says it skipped: the typechecking
//! `swiftc` hands each call to Xcode's, and the cuts are its tools'. A
//! `swiftc` alone doesn't count (CI's Ubuntu image has one). A harness whose
//! cut reads the app's Swift as of a commit this clone doesn't have says it
//! skipped, naming the commit: a public clone has none of them, since the
//! export folds the dev history they're in.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

/// A harness: its name, the shell commands that typecheck it (run from the
/// repo with `$OUT` an empty directory of its own, its swiftc calls
/// typechecked by the `swiftc` first on PATH), and the commits its cut reads
/// the app's Swift from (`git show` or `git archive` in its build script,
/// which names them).
struct Harness {
    name: &'static str,
    build: &'static str,
    pins: &'static [&'static str],
}

/// In the order they start: the slowest first.
const HARNESSES: &[Harness] = &[
    Harness {
        name: "store (crates/store/harness)",
        build: r#"crates/store/harness/build-swift.sh "$OUT""#,
        pins: &["8c10490a"],
    },
    Harness {
        name: "host care (crates/hostcare/harness)",
        build: r#"crates/hostcare/harness/build-swift.sh "$OUT/care-swift""#,
        // Its Swift is kept in harness/swift/frozen and the install
        // harness's: no commit to read.
        pins: &[],
    },
    Harness {
        name: "UniFFI smoke (crates/ffi/harness)",
        build: r#"swiftc -swift-version 5 -parse-as-library -I crates/ffi/include \
    Moshpit/Services/Rust/Generated/moshpit_ffi.swift crates/ffi/harness/smoke/main.swift"#,
        pins: &[],
    },
    Harness {
        name: "install (crates/install/harness)",
        build: r#"crates/install/harness/build-swift.sh "$OUT/install-swift""#,
        // Its Swift is kept in harness/swift/frozen: no commit to read.
        pins: &[],
    },
    Harness {
        name: "seal (crates/seal/harness)",
        build: r#"crates/seal/harness/build-swift.sh "$OUT/seal-swift""#,
        pins: &["da2146f1", "8ef981c5"],
    },
    Harness {
        name: "Home (crates/app/harness/home)",
        build: r#"crates/app/harness/home/build-swift.sh "$OUT/home-swift""#,
        pins: &["95f649ab"],
    },
    Harness {
        // verify-app-language.sh's build, its one-line stub written the same.
        name: "app language (scripts/verify/app-language)",
        build: r#"printf 'public enum AppGroup { public static let id = "typecheck" }\n' > "$OUT/stub.swift" &&
swiftc -O -o "$OUT/check" Packages/MoshpitKit/Sources/MoshpitKit/AppLanguage.swift "$OUT/stub.swift" \
    scripts/verify/app-language/main.swift"#,
        pins: &[],
    },
];

/// The `swiftc` the harnesses find first: the call typechecked, not built.
/// A call that emits a module (one the next call imports) emits only that;
/// output, library and optimisation flags go. Each call is logged in
/// `$TYPECHECK_LOG` with how many Swift files it had, so a harness that
/// stopped compiling anything can't pass.
const SWIFTC: &str = r#"#!/bin/bash
emit=
for a in "$@"; do [ "$a" = -emit-module ] && emit=1; done
args=(); skip=
for a in "$@"; do
    if [ -n "$skip" ]; then skip=; continue; fi
    case "$a" in
        -o|-L) skip=1; continue ;;
        -emit-library|-static|-emit-executable|-O|-Onone|-Osize|-wmo|-whole-module-optimization) continue ;;
        -l*|-L*) continue ;;
    esac
    args+=("$a")
done
files=0
for a in "${args[@]}"; do case "$a" in *.swift) files=$((files + 1)) ;; esac; done
echo "$([ -n "$emit" ] && echo module || echo typecheck) $files" >> "$TYPECHECK_LOG"
if [ -n "$emit" ]; then exec /usr/bin/xcrun swiftc -wmo "${args[@]}"; fi
exec /usr/bin/xcrun swiftc -typecheck -wmo "${args[@]}"
"#;

enum Outcome {
    Built,
    Skipped(String),
    Failed(String),
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repo")
}

fn succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Whether this machine has the Swift compiler the typechecking `swiftc`
/// hands each call to: Xcode's, through `/usr/bin/xcrun`. Not on Linux,
/// whose `swiftc` (CI's Ubuntu image has one) comes without xcrun, nor on a
/// Mac without Xcode or its command line tools. `xcode-select -p` is asked
/// first: with no tools at all, xcrun would offer them in a dialog instead
/// of answering.
fn has_xcode_swiftc() -> bool {
    succeeds("xcode-select", &["-p"]) && succeeds("/usr/bin/xcrun", &["swiftc", "-version"])
}

/// Whether this clone has the commit `rev`.
fn has_commit(rev: &str) -> bool {
    let repo = repo();
    let repo = repo.to_str().expect("the repo's path is UTF-8");
    succeeds("git", &["-C", repo, "cat-file", "-e", rev])
}

/// The lines of a failed build worth showing.
fn why(text: &str) -> String {
    let mut lines: Vec<&str> = text
        .lines()
        .filter(|line| {
            line.contains("error:") || line.contains(".sh: ") || line.contains("build-swift")
        })
        .take(8)
        .collect();
    if lines.is_empty() {
        lines = text.lines().rev().take(8).collect();
        lines.reverse();
    }
    lines.join("\n")
}

fn typecheck(harness: &Harness, out_dir: &Path, shim_path: &str) -> Outcome {
    if let Some(rev) = harness.pins.iter().copied().find(|rev| !has_commit(rev)) {
        return Outcome::Skipped(format!(
            "its cut reads the app's Swift as of {rev}, which this clone doesn't have"
        ));
    }
    std::fs::create_dir_all(out_dir).expect("its output directory");
    let log = out_dir.with_extension("calls");
    let out = Command::new("bash")
        .args(["-o", "pipefail", "-c", harness.build])
        .current_dir(repo())
        .env("PATH", shim_path)
        .env("OUT", out_dir)
        .env("TYPECHECK_LOG", &log)
        .output()
        .expect("bash runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    let typechecked = calls
        .lines()
        .any(|call| call.strip_prefix("typecheck ").is_some_and(|n| n != "0"));
    match (out.status.success(), typechecked) {
        (true, true) => Outcome::Built,
        (true, false) => Outcome::Failed(format!(
            "it built, but typechecked no Swift (swiftc calls: {:?})",
            calls.lines().collect::<Vec<_>>()
        )),
        (false, _) => Outcome::Failed(why(&text)),
    }
}

#[test]
fn every_swift_harness_typechecks_against_the_apps_sources() {
    if !has_xcode_swiftc() {
        eprintln!("skipped: no Xcode here (xcrun's swiftc) to typecheck the Swift harnesses");
        return;
    }
    let root = std::env::temp_dir().join(format!("harness-typecheck-{}", std::process::id()));
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("a temp directory");
    let shim = bin.join("swiftc");
    std::fs::write(&shim, SWIFTC).expect("the typechecking swiftc");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755))
            .expect("the typechecking swiftc runs");
    }
    let shim_path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let started = Instant::now();
    let queue = Mutex::new(HARNESSES.iter().enumerate().collect::<VecDeque<_>>());
    let results = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism()
        .map_or(2, |n| n.get() / 2)
        .clamp(1, 4);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    // Its own statement: a `while let` would hold the lock
                    // through the typecheck, one harness at a time.
                    let next = queue.lock().unwrap().pop_front();
                    let Some((index, harness)) = next else { break };
                    let began = Instant::now();
                    let outcome = typecheck(harness, &root.join(index.to_string()), &shim_path);
                    eprintln!(
                        "{:>6.1}s  {}  {}",
                        began.elapsed().as_secs_f64(),
                        match outcome {
                            Outcome::Built => "ok     ",
                            Outcome::Skipped(_) => "skipped",
                            Outcome::Failed(_) => "FAIL   ",
                        },
                        harness.name
                    );
                    results.lock().unwrap().push((harness.name, outcome));
                }
            });
        }
    });
    let _ = std::fs::remove_dir_all(&root);
    eprintln!(
        "{} harnesses in {:.1}s",
        HARNESSES.len(),
        started.elapsed().as_secs_f64()
    );

    let results = results.into_inner().unwrap();
    assert_eq!(results.len(), HARNESSES.len());
    let mut failures = Vec::new();
    for (name, outcome) in results {
        match outcome {
            Outcome::Built => {}
            Outcome::Skipped(reason) => eprintln!("skipped {name}: {reason}"),
            Outcome::Failed(why) => failures.push(format!("{name}:\n{why}")),
        }
    }
    assert!(
        failures.is_empty(),
        "these Swift harnesses no longer build against the app's sources; fix their \
         cuts or their own Swift before anyone needs them:\n\n{}",
        failures.join("\n\n")
    );
}

/// Each harness pins only commits its build script names, so a pin the
/// script moved on from or stopped reading doesn't keep it skipped where it
/// could run. It reads files only, so it runs everywhere.
#[test]
fn each_harness_pins_commits_its_build_script_names() {
    for harness in HARNESSES {
        let scripts: String = harness
            .build
            .split_whitespace()
            .map(|word| word.trim_matches('"'))
            .filter(|word| word.ends_with(".sh"))
            .filter_map(|path| std::fs::read_to_string(repo().join(path)).ok())
            .collect();
        for rev in harness.pins {
            assert!(
                scripts.contains(*rev),
                "{}: pins {rev}, which its build script doesn't name",
                harness.name
            );
        }
    }
}
