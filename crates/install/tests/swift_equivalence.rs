//! The Rust port against the Swift install code's recorded output, without
//! swiftc: `tests/fixtures/swift/cases.swift.txt` is what
//! `harness/swift/main.swift` printed for `cases.txt` over the app's
//! unchanged sources, kept in `harness/swift/frozen`
//! (`harness/equivalence.py update-goldens`) — host
//! scripts and their digests, push confs, all seven agents' merged and
//! unmerged configs, manifests, probe parses, every command, and installer
//! flows against a simulated host. Rust must print exactly the same, except
//! on the documented divergences, where it must print what
//! `divergences.rust.txt` holds and never what Swift did.

use std::path::Path;

use moshpit_install::harness;

fn lines(name: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/swift")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn committed_cases_print_what_swift_printed() {
    let cases = lines("cases.txt");
    let swift = lines("cases.swift.txt");
    assert_eq!(cases.len(), swift.len());
    let kinds = [
        "scripts", "conf", "merge", "remove", "manifest", "mread", "facts", "caps", "cmd", "flow",
    ];
    for kind in kinds {
        assert!(
            cases.iter().any(|c| c.starts_with(kind)),
            "no {kind} case committed"
        );
    }
    let mut differences = Vec::new();
    for (case, swift) in cases.iter().zip(&swift) {
        let rust = harness::run(case).expect("a case");
        if &rust != swift {
            differences.push(format!(
                "{case:.160}\n  swift {swift:.300}\n  rust  {rust:.300}"
            ));
        }
    }
    assert!(
        differences.is_empty(),
        "{} differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

/// The scripts' digests decide whether a host is reinstalled: Rust's must be
/// the app's, whatever else changes.
#[test]
fn the_script_digests_are_the_apps() {
    let cases = lines("cases.txt");
    let swift = lines("cases.swift.txt");
    let at = cases
        .iter()
        .position(|c| c == "scripts")
        .expect("a scripts case");
    assert_eq!(harness::run("scripts").as_deref(), Some(swift[at].as_str()));
}

#[test]
fn every_divergence_still_tells_the_two_apart() {
    let cases = lines("divergences.txt");
    let swift = lines("divergences.swift.txt");
    let recorded = lines("divergences.rust.txt");
    assert_eq!(cases.len(), swift.len());
    assert_eq!(cases.len(), recorded.len());
    for ((case, swift), recorded) in cases.iter().zip(&swift).zip(&recorded) {
        let rust = harness::run(case).expect("a case");
        assert_eq!(&rust, recorded, "{case:.120}");
        assert_ne!(&rust, swift, "no longer a divergence: {case:.120}");
    }
}
