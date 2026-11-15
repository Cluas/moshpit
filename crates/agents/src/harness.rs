//! Event scripts for comparing the machine with the Swift monitor it
//! replaces: the same script through each, a transcript of what each did.
//!
//! A script is a list of events at times relative to [`T0`]: a connection
//! tracked or let go, the pane tree, a hook poll, output, a bell, a sweep
//! (with the pane on screen), a push the notification service recorded, a
//! headline cycle. The Swift side (`AgentsScriptTests`, on the device) runs
//! the committed scripts and prints its transcripts; the Rust side runs them
//! here and compares (`tests/swift_equivalence.rs`).

use std::collections::BTreeMap;

use moshpit_seal::copy::{InterruptionLevel, Language};
use moshpit_seal::standing::Entry;
use moshpit_seal::{Date, PushFinishesFile, PushStandingFile};
use serde::{Deserialize, Serialize};

use crate::{Effect, Hook, IslandState, Monitor, PaneWhere, Settings, Tunables, Visible};

/// Every script's time zero (unix seconds).
pub const T0: f64 = 1_790_000_000.0;
/// The one connection a script watches, and what the user calls it.
pub const CONN: &str = "0F0E0D0C-0000-4000-8000-000000000002";
pub const LABEL: &str = "devbox";
/// Every pane lives in session "work", window 1 "api".
pub const SESSION: &str = "work";
pub const WINDOW: &str = "1: api";
/// How often a script's stamps arrive: the monitor's sweep polls tmux's
/// every 2 s, and the scripts were recorded through it.
pub const POLL_INTERVAL: f64 = 2.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Script {
    pub name: String,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "camelCase")]
pub enum Event {
    Track { t: f64 },
    Untrack { t: f64, keep: bool },
    /// The tree's panes and their foreground commands.
    Tree { t: f64, panes: BTreeMap<String, String> },
    Hooks { t: f64, panes: Vec<HookIn> },
    Output { t: f64, pane: String },
    Bell { t: f64, pane: String },
    Sweep { t: f64, visible: Option<String> },
    /// The notification service opened an attention push sent at `ts`.
    Push { t: f64, pane: String, ts: f64 },
    /// The notification service rang a finish that ended at `ts`.
    PushDone { t: f64, pane: String, ts: f64 },
    Cycle { t: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookIn {
    pub pane: String,
    pub state: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    /// Relative to [`T0`].
    #[serde(default)]
    pub since: Option<f64>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

/// Run `script` on a fresh machine (its shared files in `dir`) and say
/// what it did, one line per effect, then the record and the waiting set.
pub fn run(script: &Script, dir: &std::path::Path, language: Language) -> Vec<String> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).expect("scratch");
    let standing = PushStandingFile::new(dir.join("standing-attention.json"));
    let finishes = PushFinishesFile::new(dir.join("rung-finishes.json"));
    let settings = Settings { language, ..Settings::default() };
    let mut monitor = Monitor::new(
        Tunables::default(),
        settings,
        Some(standing.clone()),
        Some(finishes.clone()),
        &BTreeMap::new(),
        T0,
    );
    let mut tree: BTreeMap<String, String> = BTreeMap::new();
    let mut lines = Vec::new();
    let mut last_t = 0.0;
    let mut record: BTreeMap<String, f64> = BTreeMap::new();
    let place = |tree: &BTreeMap<String, String>, pane: &str| match tree.get(pane) {
        Some(command) => PaneWhere {
            command: command.clone(),
            session: Some(SESSION.into()),
            window: Some(WINDOW.into()),
            name: None,
            window_index: None,
            pane_index: None,
        },
        None => PaneWhere::default(),
    };
    for event in &script.events {
        match event {
            Event::Untrack { t, keep } => {
                last_t = *t;
                monitor.untrack(CONN, *keep, T0 + t);
            }
            Event::Tree { t, panes } => {
                last_t = *t;
                tree = panes.clone();
                let places = tree.keys().map(|pane| (pane.clone(), place(&tree, pane))).collect();
                monitor.tree(CONN, places);
            }
            Event::Track { t } => {
                last_t = *t;
                monitor.track(CONN, LABEL);
                let places = tree.keys().map(|pane| (pane.clone(), place(&tree, pane))).collect();
                monitor.tree(CONN, places);
            }
            Event::Hooks { t, panes } => {
                last_t = *t;
                let hooks = panes
                    .iter()
                    .map(|h| Hook {
                        pane: h.pane.clone(),
                        state: h.state.clone(),
                        agent: h.agent.clone(),
                        since: h.since.map(|s| T0 + s),
                        command: h.command.clone(),
                        title: h.title.clone(),
                        place: place(&tree, &h.pane),
                    })
                    .collect();
                monitor.hooks(CONN, hooks, T0 + t, POLL_INTERVAL);
            }
            Event::Output { t, pane } => {
                last_t = *t;
                monitor.output(CONN, pane, place(&tree, pane), T0 + t);
            }
            Event::Bell { t, pane } => {
                last_t = *t;
                monitor.bell(CONN, pane, place(&tree, pane), T0 + t);
            }
            Event::Sweep { t, visible } => {
                last_t = *t;
                let visible = visible.as_ref().map(|pane| Visible { conn: CONN.into(), pane: Some(pane.clone()) });
                monitor.sweep(visible.as_ref(), T0 + t);
            }
            Event::Push { t, pane, ts } => {
                last_t = *t;
                standing.note_standing(
                    CONN,
                    Entry {
                        pane: pane.clone(),
                        agent: Some("claude".into()),
                        title: Some("Bash: npm test".into()),
                        location: None,
                        since: (T0 + ts) as i64,
                        recorded_at: Date::from_unix(T0 + t),
                    },
                    Date::from_unix(T0 + t),
                );
            }
            Event::PushDone { t, pane, ts } => {
                last_t = *t;
                finishes.note_ring(CONN, pane, (T0 + ts) as i64, Date::from_unix(T0 + t));
            }
            Event::Cycle { t } => {
                last_t = *t;
                monitor.cycle_headline(T0 + t);
            }
        }
        for effect in monitor.take_effects() {
            if let Effect::SaveAnnounced(saved) = &effect {
                record = saved.clone();
            }
            if let Some(line) = line(&effect) {
                lines.push(line);
            }
        }
    }
    let end = Date::from_unix(T0 + last_t);
    let mut waiting: Vec<String> = standing
        .standing(CONN, end)
        .into_iter()
        .map(|entry| format!("{}@{}", entry.pane, entry.since as f64 - T0))
        .collect();
    waiting.sort();
    lines.push(format!("standing {}", waiting.join(",")));
    let announced: Vec<String> = record
        .iter()
        .map(|(key, at)| format!("{}={}", key.rsplit_once('|').map_or(key.as_str(), |(_, pane)| pane), at - T0))
        .collect();
    lines.push(format!("announced {}", announced.join(",")));
    lines
}

/// The scenarios the suites name, as scripts.
pub fn handwritten() -> Vec<Script> {
    let tree = || Event::Tree {
        t: 0.0,
        panes: [("%1".to_owned(), "claude".to_owned()), ("%2".to_owned(), "codex".to_owned())]
            .into_iter()
            .collect(),
    };
    let hook = |t: f64, pane: &str, state: &str, since: f64| Event::Hooks {
        t,
        panes: vec![HookIn {
            pane: pane.into(),
            state: Some(state.into()),
            agent: Some("claude".into()),
            since: Some(since),
            command: Some("claude".into()),
            title: Some("Bash: npm test".into()),
        }],
    };
    let sweep = |t: f64| Event::Sweep { t, visible: None };
    let script = |name: &str, events: Vec<Event>| Script {
        name: name.into(),
        events: [vec![tree(), Event::Track { t: 0.0 }], events].concat(),
    };
    vec![
        script("announced after the grace, once", vec![
            hook(1.0, "%1", "attention", 0.0), sweep(20.0), sweep(31.0), hook(33.0, "%1", "attention", 0.0), sweep(70.0),
        ]),
        script("answered inside the grace", vec![
            hook(1.0, "%1", "attention", 0.0), hook(11.0, "%1", "working", 10.0), sweep(40.0),
        ]),
        script("a pushed prompt answered while polling", vec![
            Event::Push { t: 31.0, pane: "%1".into(), ts: 31.0 }, hook(32.0, "%1", "attention", 0.0),
            hook(41.0, "%1", "working", 40.0), Event::Push { t: 240.0, pane: "%1".into(), ts: 240.0 },
        ]),
        script("one of two answered; a stale poll", vec![
            Event::Push { t: 31.0, pane: "%1".into(), ts: 31.0 }, Event::Push { t: 35.0, pane: "%2".into(), ts: 35.0 },
            hook(51.0, "%1", "working", 50.0), hook(52.0, "%2", "working", -120.0),
        ]),
        script("disconnect keeps, back and answered", vec![
            hook(1.0, "%1", "attention", 0.0), sweep(31.0), Event::Untrack { t: 40.0, keep: true },
            Event::Track { t: 100.0 }, hook(101.0, "%1", "attention", 0.0), hook(102.0, "%1", "working", 101.0),
        ]),
        script("delete withdraws", vec![
            hook(1.0, "%1", "attention", 0.0), sweep(31.0), Event::Untrack { t: 40.0, keep: false },
        ]),
        script("a long finish the push rang", vec![
            Event::PushDone { t: 600.0, pane: "%1".into(), ts: 601.0 }, hook(599.0, "%1", "working", 0.0),
            hook(601.0, "%1", "done", 600.0),
        ]),
        script("a long finish rung here", vec![
            hook(599.0, "%1", "working", 0.0), hook(601.0, "%1", "done", 600.0),
            Event::PushDone { t: 602.0, pane: "%1".into(), ts: 601.0 },
        ]),
        script("a short finish", vec![hook(19.0, "%1", "working", 0.0), hook(21.0, "%1", "done", 20.0)]),
        script("looking acknowledges", vec![
            hook(1.0, "%1", "attention", 0.0), sweep(31.0), Event::Sweep { t: 33.0, visible: Some("%1".into()) },
        ]),
        script("output earns a row, then done, then gone", vec![
            Event::Output { t: 0.0, pane: "%1".into() }, Event::Output { t: 1.0, pane: "%1".into() },
            Event::Output { t: 2.5, pane: "%1".into() }, sweep(7.0), sweep(33.0),
        ]),
        script("a bell", vec![Event::Bell { t: 0.0, pane: "%1".into() }, sweep(31.0)]),
        script("ordering and the headline", vec![
            Event::Hooks { t: 10.0, panes: (1..=2).map(|i| HookIn {
                pane: format!("%{i}"), state: Some(if i == 2 { "attention" } else { "working" }.into()),
                agent: Some("claude".into()), since: Some(i as f64), command: Some("claude".into()), title: None,
            }).collect() },
            Event::Cycle { t: 11.0 }, Event::Cycle { t: 12.0 },
        ]),
    ]
}

/// One effect as a transcript line; nil for the record (compared once, at
/// the end, by the Swift side's own read of it).
pub fn line(effect: &Effect) -> Option<String> {
    Some(match effect {
        Effect::Island(state) => format!("island {}", compact(state)),
        Effect::Widget(state) => format!("widget {}", compact(state)),
        Effect::EndIsland => "end-island".into(),
        Effect::Post(card) => format!(
            "post {} {} | {} | {} | sound={} | {}",
            card.id.replace(CONN, "C"),
            card.pane_id,
            card.text.title,
            card.text.body,
            card.text.sound,
            level(card.text.level)
        ),
        Effect::Withdraw(id) => format!("withdraw {}", id.replace(CONN, "C")),
        Effect::RequestAuthorization => "auth".into(),
        // The pet's count: no Swift monitor had it, so no transcript line.
        Effect::SaveAnnounced(_) | Effect::Note(_) | Effect::Finished { .. } => return None,
    })
}

pub fn level(level: InterruptionLevel) -> &'static str {
    match level {
        InterruptionLevel::Passive => "passive",
        InterruptionLevel::Active => "active",
        InterruptionLevel::TimeSensitive => "time-sensitive",
    }
}

/// The island's state on one line, times relative to [`T0`].
pub fn compact(state: &IslandState) -> String {
    let rows: Vec<String> = state
        .agents
        .iter()
        .map(|a| {
            format!(
                "{}:{}:{}:{}:{}:{}",
                a.pane_id,
                a.state.raw(),
                a.command,
                a.location,
                a.detail.as_deref().unwrap_or("-"),
                a.started_at - T0
            )
        })
        .collect();
    let headline = state
        .headline_id
        .as_deref()
        .and_then(|id| id.rsplit_once(':'))
        .map_or("-", |(_, pane)| pane);
    format!(
        "[{}] w={} a={} h={}",
        rows.join(";"),
        state.working_count,
        state.attention_count,
        headline
    )
}

/// `count` scripts from `seed`: two panes on one connection, an agent's
/// life — prompts, answers, finishes, pushes, output, bells, a disconnect
/// and back — at uneven times, with the grace, the hook window and the
/// sets' lifetimes in reach.
pub fn generate(seed: u64, count: usize) -> Vec<Script> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    (0..count)
        .map(|n| {
            let mut t = 0.0;
            let mut events = vec![
                Event::Tree {
                    t,
                    panes: [("%1".to_owned(), "claude".to_owned()), ("%2".to_owned(), "codex".to_owned())]
                        .into_iter()
                        .collect(),
                },
                Event::Track { t },
            ];
            let mut since: BTreeMap<&str, f64> = BTreeMap::new();
            let mut tracked = true;
            for _ in 0..(12 + rng.below(20)) {
                t += match rng.below(10) {
                    0 => 31.0 + rng.below(10) as f64,
                    1 => 600.0 + rng.below(400) as f64,
                    2..=4 => 2.0,
                    _ => 0.5 + rng.below(8) as f64,
                };
                let pane = if rng.below(3) == 0 { "%2" } else { "%1" };
                let event = match rng.below(14) {
                    0..=4 if tracked => {
                        let state = ["working", "attention", "done"][rng.below(3) as usize];
                        let stamp = match rng.below(4) {
                            0 => since.get(pane).copied().unwrap_or(t),
                            1 => t - 200.0,
                            _ => t,
                        };
                        since.insert(pane, stamp);
                        Event::Hooks {
                            t,
                            panes: vec![HookIn {
                                pane: pane.into(),
                                state: Some(state.into()),
                                agent: Some(if pane == "%1" { "claude" } else { "codex" }.into()),
                                since: Some(stamp),
                                command: Some(if rng.below(12) == 0 { "zsh" } else { "claude" }.into()),
                                title: Some(
                                    ["Bash: npm test", "Claude is waiting for your input", "Edit: src/main.rs"]
                                        [rng.below(3) as usize]
                                        .into(),
                                ),
                            }],
                        }
                    }
                    5..=7 => Event::Sweep {
                        t,
                        visible: match rng.below(4) {
                            0 => Some(pane.into()),
                            _ => None,
                        },
                    },
                    8 => Event::Push { t, pane: pane.into(), ts: t - rng.below(5) as f64 },
                    9 => Event::PushDone { t, pane: pane.into(), ts: since.get(pane).copied().unwrap_or(t) },
                    10 if tracked => Event::Output { t, pane: pane.into() },
                    11 if tracked => Event::Bell { t, pane: pane.into() },
                    12 => {
                        if tracked {
                            tracked = false;
                            Event::Untrack { t, keep: rng.below(2) == 0 }
                        } else {
                            tracked = true;
                            Event::Track { t }
                        }
                    }
                    13 => Event::Cycle { t },
                    _ => Event::Sweep { t, visible: None },
                };
                events.push(event);
            }
            Script { name: format!("random-{seed}-{n}"), events }
        })
        .collect()
}

/// A small deterministic generator (splitmix-ish), so scripts are the same
/// on every machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}
