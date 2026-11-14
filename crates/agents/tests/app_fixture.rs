//! The app's cross-check (`Tests/MoshpitTests/Island/AgentsScriptTests`)
//! holds the app's monitor, which runs this machine, to the machine's own
//! transcripts of the committed scripts:
//! `Tests/MoshpitTests/Fixtures/agents/rust-transcripts.txt`, written by
//! `cargo run -p moshpit-agents --example agents_transcripts`. Nothing held
//! that file to the machine, so B433 and B437 changed the machine and left
//! it behind, and the app's test failed against a machine that no longer
//! existed (the first cloud run of the app's tests). This is that check.

use std::path::Path;

use moshpit_agents::harness::{self, Script};
use moshpit_seal::copy::Language;

/// What the example prints for the committed scripts in `tag`'s language.
fn transcripts(tag: &str) -> String {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Tests/MoshpitTests/Fixtures/agents");
    let scripts: Vec<Script> =
        serde_json::from_slice(&std::fs::read(fixtures.join("scripts.json")).expect("scripts"))
            .expect("parse");
    let language = Language::from_tag(tag);
    let dir = std::env::temp_dir().join(format!("agents-app-fixture-{}", std::process::id()));
    let mut out = format!("AGENTS-LANGUAGE\t{tag}\n");
    for script in &scripts {
        for line in harness::run(script, &dir, language) {
            out += &format!("AGENTS\t{}\t{line}\n", script.name);
        }
    }
    out += &format!("AGENTS-END\t{}\n", scripts.len());
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn the_apps_fixture_is_what_the_machine_does_now() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../Tests/MoshpitTests/Fixtures/agents/rust-transcripts.txt");
    let committed = std::fs::read_to_string(&path).expect("the app's fixture");
    let now = transcripts("en");
    if committed == now {
        return;
    }
    let (n, (was, is)) = committed
        .lines()
        .chain(std::iter::repeat("<end>"))
        .zip(now.lines().chain(std::iter::repeat("<end>")))
        .enumerate()
        .find(|(_, (was, is))| was != is)
        .expect("they differ somewhere");
    panic!(
        "the app's fixture is not what the machine does now; from line {}:\n  committed {was}\n  machine   {is}\n\
         regenerate it: cargo run -p moshpit-agents --example agents_transcripts > \
         Tests/MoshpitTests/Fixtures/agents/rust-transcripts.txt",
        n + 1
    );
}
