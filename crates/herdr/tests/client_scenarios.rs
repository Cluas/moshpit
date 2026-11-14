//! The equivalence scenarios (`harness/client/scenarios/`), replayed without
//! swiftc: the Rust client prints its recorded log, and every scenario
//! `harness/client/divergences.json` names still differs from what the app's
//! Swift client printed for it (`<name>.swift.log`, recorded by
//! `harness/client/equivalence.py update-goldens`).

use std::fs;
use std::path::Path;

const SCENARIOS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/harness/client/scenarios");

fn scenarios() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(SCENARIOS)
        .expect("scenarios")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".txt").map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

fn read(name: &str, suffix: &str) -> String {
    fs::read_to_string(Path::new(SCENARIOS).join(format!("{name}.{suffix}")))
        .unwrap_or_else(|_| panic!("{name}.{suffix}"))
}

/// The lines that are effects, not reads: what landed, the outcomes' texts,
/// the commands but reads, the final state.
fn effects(log: &str) -> Vec<String> {
    log.lines()
        .filter_map(|line| {
            if line.starts_with("landed ") || line.starts_with("state ") {
                return Some(line.to_owned());
            }
            let (_, rest) = line.split_once(' ')?;
            if rest == "exec api snapshot" || rest == "exec status" {
                return None;
            }
            (rest.starts_with("outcome ") || rest.starts_with("exec ")).then(|| rest.to_owned())
        })
        .collect()
}

#[test]
fn every_scenario_prints_its_recorded_log() {
    let names = scenarios();
    assert!(names.len() >= 27, "{names:?}");
    for name in names {
        let log = moshpit_herdr::client::scenario::run(&read(&name, "txt"))
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(log, read(&name, "rust.log"), "{name}");
    }
}

#[test]
fn every_documented_divergence_still_separates_the_two_clients() {
    let divergences = read("../divergences", "json");
    for name in scenarios() {
        let named = divergences.contains(&format!("\"{name}\""));
        let rust = effects(&read(&name, "rust.log"));
        let swift = effects(&read(&name, "swift.log"));
        if named {
            assert_ne!(
                rust, swift,
                "{name} no longer differs from the app's client"
            );
        }
    }
}
