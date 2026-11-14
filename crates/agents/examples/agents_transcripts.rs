//! The Rust side's transcripts of the committed scripts, in the Swift
//! side's print format (`AGENTS\t<name>\t<line>`), for diffing:
//! `cargo run -p moshpit-agents --example agents_transcripts [<language tag>]`
fn main() {
    let tag = std::env::args().nth(1).unwrap_or_else(|| "en".into());
    let language = moshpit_seal::copy::Language::from_tag(&tag);
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Tests/MoshpitTests/Fixtures/agents/scripts.json");
    let scripts: Vec<moshpit_agents::harness::Script> =
        serde_json::from_slice(&std::fs::read(path).expect("scripts")).expect("parse");
    let dir = std::env::temp_dir().join(format!("agents-transcripts-{}", std::process::id()));
    println!("AGENTS-LANGUAGE\t{tag}");
    for script in &scripts {
        for line in moshpit_agents::harness::run(script, &dir, language) {
            println!("AGENTS\t{}\t{line}", script.name);
        }
    }
    println!("AGENTS-END\t{}", scripts.len());
    let _ = std::fs::remove_dir_all(&dir);
}
