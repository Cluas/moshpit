//! Writes the scripts both sides run, as JSON:
//! `cargo run -p moshpit-agents --example agents_scripts > Tests/MoshpitTests/Fixtures/agents/scripts.json`
fn main() {
    let mut scripts = moshpit_agents::harness::handwritten();
    scripts.extend(moshpit_agents::harness::generate(41, 300));
    println!("{}", serde_json::to_string(&scripts).expect("scripts"));
}
