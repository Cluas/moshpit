//! This crate against what the app's Swift stores printed for the same
//! cases (tests/fixtures/swift, written by `harness/equivalence.py
//! update-goldens` from the app's unchanged sources and 1.0.8's), without
//! swiftc. The results are kept as the SHA-256 of each canonical line (they
//! run to megabytes); on a mismatch, `harness/equivalence.py run` shows the
//! lines themselves.

use std::fs;
use std::path::PathBuf;

use moshpit_store::harness;
use sha2::{Digest, Sha256};

fn fixture(name: &str) -> String {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "fixtures",
        "swift",
        name,
    ]
    .iter()
    .collect();
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn sha(line: &str) -> String {
    Sha256::digest(line.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Every generated case prints, in canonical form, what the current app's
/// Swift printed.
#[test]
fn generated_cases_print_what_the_apps_swift_printed() {
    let cases = fixture("cases.txt");
    let swift = fixture("cases.swift.sha256");
    let (results, _) = harness::run(&cases);
    let mut checked = 0;
    for ((case, rust), want) in cases.lines().zip(results.lines()).zip(swift.lines()) {
        let line = harness::canonical(rust);
        assert_eq!(sha(&line), want, "case {case}\nrust: {line}");
        checked += 1;
    }
    assert_eq!(checked, cases.lines().count());
    assert!(checked >= 100, "{checked} cases");
}

/// 1.0.8's one-piece decode reads every store this crate wrote as this
/// crate says it should.
#[test]
fn version_1_0_8_reads_what_this_crate_writes() {
    let items = fixture("views108.txt");
    let swift = fixture("views108.swift.sha256");
    let mut checked = 0;
    for (item, want) in items.lines().zip(swift.lines()) {
        let (store, token) = item.split_once(' ').expect("STORE SNAPSHOT");
        let view = harness::canonical(&harness::view108(store, token));
        assert_eq!(sha(&view), want, "{item}\nrust: {view}");
        checked += 1;
    }
    assert_eq!(checked, items.lines().count());
}

/// The documented divergences (harness/divergences.json) are still exactly
/// these: Rust prints what it printed, Swift printed something else. A
/// files-downgrade case compares without the files fold, which is its
/// difference.
#[test]
fn the_divergences_are_the_documented_ones() {
    let cases = fixture("divergences.txt");
    let ids = fixture("divergences.ids");
    let swift = fixture("divergences.swift.sha256");
    let rust_expected = fixture("divergences.rust.sha256");
    let (results, _) = harness::run(&cases);
    let mut checked = 0;
    for ((((case, id), rust), swift), want) in cases
        .lines()
        .zip(ids.lines())
        .zip(results.lines())
        .zip(swift.lines())
        .zip(rust_expected.lines())
    {
        let line = if id == "files-downgrade" {
            harness::canonical_strict(rust)
        } else {
            harness::canonical(rust)
        };
        assert_eq!(sha(&line), want, "{id}: {case}\nrust: {line}");
        assert_ne!(sha(&line), swift, "{id} is no longer a divergence: {case}");
        checked += 1;
    }
    assert_eq!(checked, cases.lines().count());
    assert!(checked >= 7);
}
