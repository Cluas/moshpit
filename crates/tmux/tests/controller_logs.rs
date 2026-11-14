//! The controller equivalence harness without swiftc: every script in
//! tests/fixtures/controller/ must still print its committed log
//! (`<name>.log`, written by `harness/controller/equivalence.py
//! update-goldens`). For every script but the divergence fixtures (`div-*`)
//! that log is also what the app's Swift controller prints (byte for byte,
//! or but for the dead-pane normalisation); for those, the Rust log is the
//! right one and divergences.json says why.
//!
//! `MOSHPIT_BLESS_CONTROLLER_LOGS=1` writes the Rust logs as the goldens
//! instead of checking them (a change the harness must then know about:
//! a divergence or a normalisation in divergences.json).

use std::fs;
use std::path::Path;

#[test]
fn every_script_prints_its_committed_log() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/controller");
    let mut scripts: Vec<_> = fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "scn"))
        .collect();
    scripts.sort();
    assert!(scripts.len() >= 70, "{} scripts", scripts.len());
    let mut failures = Vec::new();
    for script in &scripts {
        let log = moshpit_tmux::scenario::run(&fs::read_to_string(script).unwrap());
        if std::env::var("MOSHPIT_BLESS_CONTROLLER_LOGS").is_ok_and(|v| v == "1") {
            // One trailing newline, as the committed logs have.
            let golden = format!("{}\n", log.trim_end_matches('\n'));
            fs::write(script.with_extension("log"), golden).unwrap();
            continue;
        }
        let golden = fs::read_to_string(script.with_extension("log")).unwrap_or_default();
        // The harness writes the log split on newlines and joined back,
        // so a trailing newline is dropped.
        if log.trim_end_matches('\n') != golden.trim_end_matches('\n') {
            failures.push(script.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    assert!(failures.is_empty(), "logs changed: {failures:?}");
}
