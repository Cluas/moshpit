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
/// machine must print instead (`fixtures/swift/divergences.json`) — the
/// whole transcript, so anything else that moves in such a script fails
/// as it would in any other. `id` names the reason; one fix's entries share
/// it (B437's deletes: `B437-delete-withdraws`), so they can go together
/// once the Swift monitor they're held against is gone.
#[derive(serde::Deserialize)]
struct Divergence {
    script: String,
    id: String,
    #[allow(dead_code)]
    why: String,
    rust: Vec<String>,
}

/// Whether `rust`, a divergent script's transcript, is exactly what its
/// entry pins — and still differs from the Swift monitor's.
fn held_to(divergence: &Divergence, rust: &[String], swift: &[String]) -> Result<(), String> {
    if rust != divergence.rust.as_slice() {
        let at = rust.iter().zip(&divergence.rust).position(|(a, b)| a != b).unwrap_or(rust.len().min(divergence.rust.len()));
        return Err(format!(
            "{} ({}): not what the divergence records (line {at}):\n  recorded {:?}\n  rust     {:?}",
            divergence.script,
            divergence.id,
            divergence.rust.get(at).map_or("<end>", String::as_str),
            rust.get(at).map_or("<end>", String::as_str),
        ));
    }
    if normalize(rust) == normalize(swift) {
        return Err(format!("{} ({}): no longer a divergence", divergence.script, divergence.id));
    }
    Ok(())
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
            if let Err(difference) = held_to(divergence, &rust, &swift[&script.name]) {
                differences.push(difference);
            }
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

/// A divergence pins its script's whole transcript: a script with a delete
/// that moves anywhere else — a card more, a line changed — fails as any
/// other script would. Checked on every B437 entry, by moving one line of
/// what the machine prints.
#[test]
fn a_divergence_holds_its_script_to_the_whole_transcript() {
    let (language, swift) = swift();
    let scripts = scripts();
    let divergences = divergences();
    let deletes: Vec<&Divergence> = divergences.iter().filter(|d| d.id == "B437-delete-withdraws").collect();
    assert_eq!(deletes.len(), 136);
    let dir = std::env::temp_dir().join(format!("agents-divergence-{}", std::process::id()));
    for divergence in deletes {
        let script = scripts.iter().find(|s| s.name == divergence.script).expect("its script");
        let rust = harness::run(script, &dir, language);
        let theirs = &swift[&script.name];
        assert_eq!(held_to(divergence, &rust, theirs), Ok(()));
        let mut extra = rust.clone();
        extra.insert(1, "post moshpit.attention.C %9 | claude needs you | — devbox | sound=true | time-sensitive".to_owned());
        assert!(held_to(divergence, &extra, theirs).is_err(), "{}: a card more passed", script.name);
        let mut moved = rust.clone();
        let last = moved.len() - 1;
        moved[last].push_str(",%9=1");
        assert!(held_to(divergence, &moved, theirs).is_err(), "{}: a moved line passed", script.name);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
