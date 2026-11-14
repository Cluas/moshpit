//! The Rust parser against the Swift parser's recorded output, without
//! needing swiftc: `<name>.swift.jsonl` next to each committed capture is
//! what `harness/swift/main.swift` printed for it (`equivalence.py
//! update-goldens`). Rust must print the same bytes — the Swift parser
//! carries every fix the port made (`harness/divergences.json`), so no
//! difference is accepted — and must print the same events however the
//! capture is cut into feeds.

use std::fs;
use std::path::{Path, PathBuf};

use moshpit_tmux::harness;

const PLANS: &[&str] = &[
    "bytes",
    "rand:1:3",
    "rand:2:17",
    "rand:3:256",
    "rand:4:4096",
    "inside:1",
    "inside:2",
];

fn fixtures(group: &str) -> Vec<PathBuf> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(group);
    let mut found: Vec<PathBuf> = fs::read_dir(&folder)
        .unwrap_or_else(|error| panic!("{}: {error}", folder.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "bin"))
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no fixtures in {}", folder.display());
    found
}

fn sibling(capture: &Path, suffix: &str) -> PathBuf {
    capture.with_extension(suffix)
}

/// Event lines, without the trailing `end` record.
fn body(output: &[u8]) -> Vec<&[u8]> {
    let mut lines: Vec<&[u8]> = output.split(|&b| b == b'\n').collect();
    lines.truncate(lines.len().saturating_sub(2));
    lines
}

/// The first line where the two outputs differ, if any.
fn difference(swift: &[u8], rust: &[u8]) -> Option<String> {
    let swift_lines: Vec<&[u8]> = swift.split(|&b| b == b'\n').collect();
    let rust_lines: Vec<&[u8]> = rust.split(|&b| b == b'\n').collect();
    if let Some((number, (s, r))) = swift_lines
        .iter()
        .zip(&rust_lines)
        .enumerate()
        .find(|(_, (s, r))| s != r)
    {
        return Some(format!(
            "line {}: Swift {}\n  Rust {}",
            number + 1,
            String::from_utf8_lossy(s),
            String::from_utf8_lossy(r)
        ));
    }
    (swift_lines.len() != rust_lines.len()).then(|| {
        format!(
            "{} lines from Swift, {} from Rust",
            swift_lines.len(),
            rust_lines.len()
        )
    })
}

fn check_against_swift(group: &str) -> usize {
    let mut checked = 0;
    for capture in fixtures(group) {
        let data = fs::read(&capture).unwrap();
        let golden = fs::read(sibling(&capture, "swift.jsonl")).unwrap();
        let whole = harness::run("whole", &data).unwrap();
        if let Some(why) = difference(&golden, &whole) {
            panic!("{}: {why}", capture.display());
        }
        for plan in PLANS {
            let output = harness::run(plan, &data).unwrap();
            assert!(
                body(&output) == body(&whole),
                "{}: plan {plan} changes the events",
                capture.display()
            );
        }
        checked += 1;
    }
    checked
}

/// Real tmux 3.6a streams: plain output, CJK and emoji, ANSI colours,
/// alternate screen (less, vim), splits and layouts, a session switch with a
/// second client, failing commands, a failed-attach boot chain, pause-after.
#[test]
fn recorded_captures_match_the_swift_parser() {
    assert_eq!(check_against_swift("captures"), 9);
}

/// Recovery and malformed-line paths the Swift parser handles correctly.
#[test]
fn synthetic_recovery_stream_matches_the_swift_parser() {
    assert_eq!(check_against_swift("synthetic"), 1);
}

/// Each documented fix: Rust prints its recorded output under any feed
/// plan (`equivalence.py` holds the Swift parser to the same output), and
/// that output is not what the Swift parser printed before the port —
/// `<fixture>.swift-fd87248f.jsonl` — or the old parser crashed.
#[test]
fn divergence_fixtures_show_each_documented_fix() {
    let mut checked = 0;
    for capture in fixtures("divergences") {
        let data = fs::read(&capture).unwrap();
        let expected = fs::read(sibling(&capture, "rust.jsonl")).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&harness::run("whole", &data).unwrap()),
            String::from_utf8_lossy(&expected),
            "{}",
            capture.display()
        );
        for plan in PLANS {
            let output = harness::run(plan, &data).unwrap();
            assert!(
                body(&output) == body(&expected),
                "{}: plan {plan}",
                capture.display()
            );
        }
        match fs::read(sibling(&capture, "swift-fd87248f.jsonl")) {
            Ok(swift) => assert_ne!(body(&swift), body(&expected), "{}", capture.display()),
            // No Swift output recorded: the old Swift parser trapped on it.
            Err(_) => assert!(capture.ends_with("overflow-trap.bin")),
        }
        checked += 1;
    }
    assert_eq!(checked, 7);
}
