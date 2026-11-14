//! The fake herdr host (`client::rig::FakeHerdr`) as a co-process, for the
//! Swift side of the client equivalence runs (`harness/client/`): the app's
//! `HerdrControlClient.swift` runs its commands against the same host the
//! Rust client runs against in-process. Never part of the app.
//!
//!   herdr_host <script>
//!
//! Takes the script's `host` lines, then one request per line on stdin:
//!
//!   exec <t> <hex command>   → plan <id> after <ms> | plan <id> never | plan <id> fail
//!   land <t> <id>            → output <hex>   (what it printed, applied at t)
//!   set <setting…>           → ok             (a `host` line, now)
//!   landed                   → landed <mutation>… then end
//!   focused                  → focused <pane|nil>

use std::collections::BTreeMap;
use std::io::{BufRead, Write};

use moshpit_herdr::client::rig::{FakeHerdr, Plan};
use moshpit_herdr::client::scenario;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .filter_map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok())
        .collect()
}

fn main() {
    let path = std::env::args().nth(1).expect("herdr_host <script>");
    let text = std::fs::read_to_string(&path).expect("script readable");
    let script = scenario::parse(&text).expect("script parses");
    let mut host = FakeHerdr::default();
    for words in &script.host {
        scenario::apply_host(&mut host, words).expect("host setting");
    }
    let mut commands: BTreeMap<u64, String> = BTreeMap::new();
    let mut next = 0u64;
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let words: Vec<&str> = line.split_whitespace().collect();
        let reply = match words.first().copied() {
            Some("exec") => {
                next += 1;
                let command = String::from_utf8_lossy(&unhex(words.get(2).copied().unwrap_or("")))
                    .into_owned();
                let plan = host.plan(&command);
                commands.insert(next, command);
                match plan {
                    Plan::After(ms) => format!("plan {next} after {ms}"),
                    Plan::Never => format!("plan {next} never"),
                    Plan::Fail => format!("plan {next} fail"),
                }
            }
            Some("land") => {
                let now: u64 = words.get(1).and_then(|t| t.parse().ok()).unwrap_or(0);
                let id: u64 = words.get(2).and_then(|t| t.parse().ok()).unwrap_or(0);
                let command = commands.remove(&id).unwrap_or_default();
                format!("output {}", hex(&host.land(now, &command)))
            }
            Some("set") => {
                let setting: Vec<String> = words[1..].iter().map(|w| (*w).to_owned()).collect();
                match scenario::apply_host(&mut host, &setting) {
                    Ok(()) => "ok".to_owned(),
                    Err(error) => format!("error {error}"),
                }
            }
            Some("landed") => {
                let mut lines: Vec<String> =
                    host.landed.iter().map(|l| format!("landed {l}")).collect();
                lines.push("end".to_owned());
                lines.join("\n")
            }
            Some("focused") => format!("focused {}", host.focused.as_deref().unwrap_or("nil")),
            _ => "error unknown request".to_owned(),
        };
        let _ = writeln!(out, "{reply}");
        let _ = out.flush();
    }
}
