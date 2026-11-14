//! The machine against the Swift monitor it replaces, without the device:
//! `tests/fixtures/swift/transcripts.txt` is what `AgentsScriptTests`
//! printed on the iPad for the committed scripts
//! (`Tests/MoshpitTests/Fixtures/agents/scripts.json`) through the Swift
//! `AgentActivityMonitor`, before it was deleted. Rust must print the same,
//! up to the documented differences (`normalize`), each of which is a
//! choice, explained where it is made.

use std::collections::BTreeMap;
use std::path::Path;

use moshpit_agents::harness::{self, Script};
use moshpit_seal::copy::Language;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn scripts() -> Vec<Script> {
    let path = root().join("../../Tests/MoshpitTests/Fixtures/agents/scripts.json");
    serde_json::from_slice(&std::fs::read(&path).expect("scripts")).expect("scripts parse")
}

/// name → the Swift monitor's lines, and the language it ran in.
fn swift() -> (Language, BTreeMap<String, Vec<String>>) {
    let text = std::fs::read_to_string(root().join("tests/fixtures/swift/transcripts.txt")).expect("transcripts");
    let mut language = Language::En;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in text.lines() {
        if let Some(tag) = line.strip_prefix("AGENTS-LANGUAGE\t") {
            language = Language::from_tag(tag);
        } else if let Some(rest) = line.strip_prefix("AGENTS\t") {
            let (name, line) = rest.split_once('\t').expect("name\\tline");
            out.entry(name.to_owned()).or_default().push(line.to_owned());
        }
    }
    (language, out)
}

/// The scripts on which the two may differ, each with why and what the
/// machine must print instead (`fixtures/swift/divergences.json`).
#[derive(serde::Deserialize)]
struct Divergence {
    script: String,
    #[allow(dead_code)]
    why: String,
    rust: Vec<String>,
}

fn divergences() -> Vec<Divergence> {
    serde_json::from_slice(&std::fs::read(root().join("tests/fixtures/swift/divergences.json")).expect("divergences"))
        .expect("divergences parse")
}

/// The differences that are choices.
///
/// * `end-island`: the Swift monitor ended every island on every sync while
///   no agent was listed — ending what is already ended, every two seconds.
///   The machine ends it once, and again only after an island was shown.
fn normalize(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut ended = false;
    for line in lines {
        if line == "end-island" {
            if ended {
                continue;
            }
            ended = true;
        } else if line.starts_with("island ") {
            ended = false;
        }
        out.push(line.clone());
    }
    out
}

#[test]
fn every_script_does_what_the_swift_monitor_did() {
    let (language, swift) = swift();
    let scripts = scripts();
    assert!(!swift.is_empty(), "no Swift transcripts");
    let dir = std::env::temp_dir().join(format!("agents-equivalence-{}", std::process::id()));
    let divergences = divergences();
    let mut differences = Vec::new();
    for script in &scripts {
        if let Some(divergence) = divergences.iter().find(|d| d.script == script.name) {
            let rust = harness::run(script, &dir, language);
            assert_eq!(rust, divergence.rust, "{}: not what the divergence records", script.name);
            assert_ne!(
                normalize(&rust),
                normalize(&swift[&script.name]),
                "{}: no longer a divergence",
                script.name
            );
            continue;
        }
        let raw = harness::run(script, &dir, language);
        let rust = normalize(&raw);
        let Some(theirs) = swift.get(&script.name) else {
            differences.push(format!("{}: no Swift transcript", script.name));
            continue;
        };
        let theirs = normalize(theirs);
        if rust != theirs {
            let at = rust.iter().zip(&theirs).position(|(a, b)| a != b).unwrap_or(rust.len().min(theirs.len()));
            differences.push(format!(
                "{} (line {at}):\n  swift {:?}\n  rust  {:?}",
                script.name,
                theirs.get(at).map_or("<end>", String::as_str),
                rust.get(at).map_or("<end>", String::as_str),
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        differences.is_empty(),
        "{} of {} scripts differ:\n{}",
        differences.len(),
        scripts.len(),
        differences.join("\n")
    );
}
