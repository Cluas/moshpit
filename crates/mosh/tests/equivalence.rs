//! The Rust port against the Swift implementation's recorded output, without
//! swiftc: each `*.swift.txt` next to a fixture is what
//! `harness/swift/main.swift` printed for it over the app's unchanged
//! sources (`harness/equivalence.py update-goldens`). The Swift client now
//! carries every fix in `harness/divergences.json` but one, so Rust must
//! print the same bytes everywhere, except on the divergence fixtures of
//! that open fix (`excerpt-unicode`). Every divergence fixture must also
//! differ from what the Swift client printed before the fixes
//! (`divergences.swift-b4b8e1b4.txt`), and in scenarios `thr` must be the
//! acked state (`old`) on both sides.

use std::fs;
use std::path::{Path, PathBuf};

use moshpit_mosh::harness;

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(path)
}

fn read(path: &str) -> String {
    let path = fixture(path);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().filter(|line| !line.is_empty()).collect()
}

/// `zlib.compress` results without the compressed bytes, which differ
/// (Apple's DEFLATE is not miniz_oxide's).
fn comparable(line: &str) -> String {
    if line.starts_with("zlib.compress ") {
        line.split(' ')
            .filter(|field| !field.starts_with("out="))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        line.to_owned()
    }
}

fn run_cases(text: &str) -> Vec<String> {
    text.lines().filter_map(harness::wire::run).collect()
}

#[test]
fn wire_cases_match_swift() {
    let cases = read("wire/cases.txt");
    let swift = read("wire/cases.swift.txt");
    let rust = run_cases(&cases);
    let swift = lines(&swift);
    assert_eq!(rust.len(), swift.len());
    for (index, (rust, swift)) in rust.iter().zip(&swift).enumerate() {
        assert_eq!(comparable(rust), comparable(swift), "case {index}");
    }
}

/// Fixes the Swift client does not carry (`harness/divergences.json`).
const OPEN_FIXES: [&str; 1] = ["excerpt-unicode"];

/// The fix id of each case in `wire/divergences.txt`: the `# <id>: ...`
/// comment before it.
fn divergence_fixes(cases: &str) -> Vec<String> {
    let mut fix = String::new();
    let mut out = Vec::new();
    for line in cases.lines() {
        if let Some(comment) = line.strip_prefix("# ") {
            if let Some((id, _)) = comment.split_once(':') {
                fix = id.to_owned();
            }
        } else if !line.is_empty() && !line.starts_with('#') {
            out.push(fix.clone());
        }
    }
    out
}

#[test]
fn divergence_cases_keep_their_recorded_results() {
    let cases = read("wire/divergences.txt");
    let fixes = divergence_fixes(&cases);
    let rust = run_cases(&cases);
    let recorded = read("wire/divergences.rust.txt");
    let swift = read("wire/divergences.swift.txt");
    let before = read("wire/divergences.swift-b4b8e1b4.txt");
    let (swift, before) = (lines(&swift), lines(&before));
    assert_eq!(rust, lines(&recorded));
    assert_eq!(fixes.len(), rust.len());
    assert_eq!(swift.len(), rust.len());
    assert_eq!(before.len(), rust.len());
    for (index, fix) in fixes.iter().enumerate() {
        // The fixture tells a fixed client from one before the fixes.
        assert_ne!(rust[index], before[index], "case {index} ({fix})");
        if OPEN_FIXES.contains(&fix.as_str()) {
            assert_ne!(
                rust[index], swift[index],
                "case {index}: {fix} is fixed in Swift now"
            );
        } else {
            assert_eq!(rust[index], swift[index], "case {index} ({fix})");
        }
    }
}

#[test]
fn recorded_sessions_decode_like_swift() {
    for client in ["swift", "rust"] {
        let session = read(&format!("live/session-{client}.txt"));
        let rust: Vec<String> = harness::wire::session_cases(&session)
            .iter()
            .filter_map(|case| harness::wire::run(case))
            .collect();
        let swift = read(&format!("live/session-{client}.wire.swift.txt"));
        assert_eq!(rust, lines(&swift), "session-{client}");
        // Every datagram of the real mosh-server decoded to an instruction.
        assert!(!rust[0].contains(";x:") && !rust[0].contains("he:"));
    }
}

/// Scenario outputs compared whole; `thr` must be the acked state (`old`).
fn assert_scenario_matches(rust: &str, swift: &str, label: &str) {
    let (rust, swift) = (lines(rust), lines(swift));
    assert_eq!(rust.len(), swift.len(), "{label}");
    for (index, (r, s)) in rust.iter().zip(&swift).enumerate() {
        assert_eq!(r, s, "{label} line {index}");
        if r.starts_with("inst ") {
            let value = |name: &str| {
                r.split(' ')
                    .find_map(|field| field.strip_prefix(name))
                    .unwrap()
            };
            assert_eq!(value("thr="), value("old="), "{label} line {index}");
        }
    }
}

#[test]
fn recorded_sessions_replay_like_swift() {
    for client in ["swift", "rust"] {
        let session = read(&format!("live/session-{client}.txt"));
        let rust = harness::ssp::run(&harness::ssp::from_session(&session));
        let swift = read(&format!("live/session-{client}.ssp.swift.txt"));
        assert_scenario_matches(&rust, &swift, &format!("session-{client}"));
        assert!(
            rust.lines()
                .filter(|line| line.starts_with("frame "))
                .count()
                > 40
        );
    }
}

#[test]
fn seeded_scenarios_match_swift() {
    for seed in [1, 2, 3] {
        let scenario = read(&format!("ssp/s{seed}.txt"));
        let swift = read(&format!("ssp/s{seed}.swift.txt"));
        let rust = harness::ssp::run(&scenario);
        assert_scenario_matches(&rust, &swift, &format!("s{seed}"));
    }
}
