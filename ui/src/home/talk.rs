//! The pet talks on Home (moshpit-design `docs/nsh/PET-DESIGN.md` §17):
//! what makes it talk here, and its words. The core
//! (`moshpit_pet::talk::Chatter`) decides whether, and which line; this
//! turns Home's changes into its cues and its lines into the app's words.
//!
//! - **Edges** ([`Edges`]): the perched agent starts working after a quiet
//!   spell, waits on you, finishes (with its minutes); its host fails or
//!   goes offline; the pet goes to sleep on SAVED. Only the perched one,
//!   and nothing on the first state (no talk at launch).
//! - **Milestones** ([`Milestones`]): the hatch, a stage grown, the 100th
//!   and the 1000th task, a roam lived through.
//! - **Visibility** ([`Visible`]): the host says when Home is on screen,
//!   with the local hour and day for the night's and the morning's lines.

use std::collections::HashMap;

use moshpit_pet::{Moment, PetView, Pose, Style, talk::Cue, talk::Speaker};
use nsh_core::t;
use serde::{Deserialize, Serialize};

use super::{
    model::{self, AgentState, Connection},
    pet::{Perch, perch, species_name},
};

/// `{"op": "visible", …}`: Home is on screen (not under a pushed page, the
/// app in front) or not; the local hour (0–23) and the local day's number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Visible {
    pub visible: bool,
    #[serde(default)]
    pub hour: u8,
    #[serde(default)]
    pub day: i64,
}

/// A line's placeholders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Fill {
    /// The agent's name on Home (its row's title).
    pub agent: String,
    /// The connection's name.
    pub host: String,
    pub minutes: i64,
    /// The pet's name.
    pub name: String,
}

/// Whether it talks: the pixel companion on, hatched, and 「让它说话」 on.
pub(crate) fn talks(view: &PetView) -> bool {
    view.talk && view.shown && view.style == Style::Pixel && view.pet.is_some()
}

/// Who talks: the pet's seed, species, crown, and the local hour.
pub(crate) fn speaker(view: &PetView, hour: u8) -> Option<(Vec<u8>, String, bool, u8)> {
    let pet = view.pet.as_ref()?;
    let seed = (0..pet.seed.len() / 2)
        .filter_map(|i| u8::from_str_radix(pet.seed.get(i * 2..i * 2 + 2)?, 16).ok())
        .collect();
    Some((seed, pet.species.clone(), pet.crowned, hour))
}

/// [`speaker`]'s parts as the core takes them.
pub(crate) fn who(parts: &(Vec<u8>, String, bool, u8)) -> Speaker<'_> {
    Speaker {
        seed: &parts.0,
        species: &parts.1,
        sudo: parts.2,
        local_hour: parts.3,
    }
}

/// The pet's name as it says it: the one given, else its species'.
pub(crate) fn pet_name(view: &PetView) -> String {
    view.pet
        .as_ref()
        .map(|p| {
            if p.name.is_empty() {
                species_name(&p.species)
            } else {
                p.name.clone()
            }
        })
        .unwrap_or_default()
}

/// The agent the pet perches beside (§7.3): the first waiting on you, else
/// the busiest live host's busiest; its row's key, its name on Home and its
/// host's name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Perched {
    pub key: (String, String),
    pub name: String,
    pub host: String,
}

fn rank(state: AgentState) -> u8 {
    match state {
        AgentState::NeedsYou => 0,
        AgentState::Working => 1,
        AgentState::Done => 2,
        AgentState::Idle => 3,
    }
}

pub(crate) fn perched(connections: &[Connection], fallback: &str) -> Option<Perched> {
    if let Some(first) = model::needs_you(connections, fallback).into_iter().next() {
        return Some(Perched {
            key: (first.row.connection.clone(), first.row.pane.clone()),
            name: first.row.title,
            host: first.host,
        });
    }
    connections
        .iter()
        .filter(|c| c.link == model::Link::Live)
        .filter_map(|c| {
            model::agent_rows(c, fallback)
                .into_iter()
                .min_by_key(|a| rank(a.state))
                .map(|a| (rank(a.state), c, a))
        })
        .min_by_key(|(r, ..)| *r)
        .map(|(_, c, a)| Perched {
            key: (a.connection.clone(), a.pane.clone()),
            name: a.title,
            host: c.name.clone(),
        })
}

/// A quiet spell an agent must have had for its start to be news.
const QUIET_SECS: f64 = 120.0;

/// Home's changes as the talk's cues (§17.1).
#[derive(Debug, Default)]
pub(crate) struct Edges {
    /// Every agent's state and since when, at the last state.
    agents: HashMap<(String, String), (AgentState, Option<f64>)>,
    perch: Option<(Perch, Pose)>,
    primed: bool,
}

impl Edges {
    /// Home's next state at `now`: the cue it gives, if any, and its
    /// placeholders.
    pub(crate) fn next(
        &mut self,
        connections: &[Connection],
        fallback: &str,
        now: f64,
    ) -> Option<(Cue, Fill)> {
        let agents: HashMap<_, _> = connections
            .iter()
            .flat_map(|c| model::agent_rows(c, fallback))
            .map(|a| ((a.connection, a.pane), (a.state, a.since)))
            .collect();
        let at = perch(connections, fallback);
        let cue = if self.primed {
            self.cue(connections, fallback, &agents, at.as_ref(), now)
        } else {
            None
        };
        self.agents = agents;
        self.perch = at;
        self.primed = true;
        cue
    }

    fn cue(
        &self,
        connections: &[Connection],
        fallback: &str,
        agents: &HashMap<(String, String), (AgentState, Option<f64>)>,
        at: Option<&(Perch, Pose)>,
        now: f64,
    ) -> Option<(Cue, Fill)> {
        if let Some(p) = perched(connections, fallback)
            && let Some((state, since)) = agents.get(&p.key).copied()
        {
            let before = self.agents.get(&p.key).copied();
            let fill = Fill {
                agent: p.name,
                host: p.host,
                ..Fill::default()
            };
            match (before, state) {
                (Some((AgentState::Working, Some(started))), AgentState::Done) => {
                    let minutes = ((since.unwrap_or(now) - started) / 60.0).floor() as i64;
                    let cue = if minutes >= 1 {
                        Cue::Done
                    } else {
                        Cue::DoneQuick
                    };
                    return Some((cue, Fill { minutes, ..fill }));
                }
                (Some((AgentState::NeedsYou, _)), AgentState::NeedsYou) => {}
                (_, AgentState::NeedsYou) => return Some((Cue::Waiting, fill)),
                (None, AgentState::Working) => return Some((Cue::Working, fill)),
                (Some((AgentState::Done | AgentState::Idle, quiet)), AgentState::Working)
                    if quiet.is_none_or(|t| now - t >= QUIET_SECS) =>
                {
                    return Some((Cue::Working, fill));
                }
                _ => {}
            }
        }
        let before = self.perch.as_ref();
        let host = |id: &str| {
            connections
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_default()
        };
        match at {
            Some((Perch::Host(id), Pose::Error)) if before != at => Some((
                Cue::Error,
                Fill {
                    host: host(id),
                    ..Fill::default()
                },
            )),
            Some((Perch::Host(id), Pose::Sleeping)) if before != at => Some((
                Cue::Offline,
                Fill {
                    host: host(id),
                    ..Fill::default()
                },
            )),
            Some((Perch::Saved, _)) if before.is_some_and(|(p, _)| *p != Perch::Saved) => {
                Some((Cue::Sleeping, Fill::default()))
            }
            _ => None,
        }
    }
}

/// The pet's milestones as cues: the hatch, a stage grown (its moments),
/// the 100th and the 1000th task, a roam lived through (its badge).
#[derive(Debug, Default)]
pub(crate) struct Milestones {
    seen: Option<(u64, u8)>,
    seq: u64,
}

impl Milestones {
    pub(crate) fn next(&mut self, view: &PetView, seq: u64) -> Option<Cue> {
        let moment = (seq > self.seq).then_some(view.moment).flatten();
        self.seq = self.seq.max(seq);
        let now = view.pet.as_ref().map(|p| (p.tasks, p.flags));
        let before = self.seen;
        if now.is_some() {
            self.seen = now;
        }
        match moment {
            Some(Moment::Hatched) => return Some(Cue::Hatched),
            Some(Moment::Grew) => return Some(Cue::Grew),
            None => {}
        }
        let ((tasks, flags), (was, had)) = (now?, before?);
        if was < 1000 && tasks >= 1000 {
            Some(Cue::Thousand)
        } else if was < 100 && tasks >= 100 {
            Some(Cue::Tasks100)
        } else if had & 1 == 0 && flags & 1 == 1 {
            Some(Cue::Roam)
        } else {
            None
        }
    }
}

/// A line's words in the app's language, its placeholder filled: the
/// agent's Home name, the host, the minutes or the pet's name.
pub(crate) fn words(id: &str, fill: &Fill) -> Option<String> {
    let agent = fill.agent.clone();
    let host = fill.host.clone();
    let name = fill.name.clone();
    Some(match id {
        "tap.hm" => t!("Hm?"),
        "tap.here" => t!("I'm here."),
        "tap.tickles" => t!("That tickles."),
        "tap.watching" => t!("Still watching."),
        "tap.tun" => t!("No rush."),
        "tap.coo" => t!("Oh hi! Busy day?"),
        "tap.dig" => t!("All in order."),
        "tap.hex" => t!("We've got this!"),
        "tap.ether" => t!("Drifting a little…"),
        "tap.moth" => t!("The night is young."),
        "working.on_it" => t!("%@ is on it.", arg1 = agent),
        "working.off" => t!("Off it goes."),
        "working.watch" => t!("I'll keep watch."),
        "waiting.needs" => t!("%@ needs you.", arg1 = agent),
        "waiting.turn" => t!("Your turn."),
        "waiting.question" => t!("It has a question."),
        "done.minutes" => t!("Done in %lld min.", arg1 = fill.minutes),
        "done.finished" => t!("%@ finished.", arg1 = agent),
        "done.wrap" => t!("That's a wrap."),
        "done.quick" => t!("That was quick."),
        "error.broke" => t!("Something broke over there."),
        "error.silent" => t!("%@ isn't answering.", arg1 = host),
        "offline.lost" => t!("Lost %@.", arg1 = host),
        "offline.wait" => t!("I'll wait for it."),
        "sleeping.nap" => t!("Nap time."),
        "sleeping.quiet" => t!("All quiet."),
        "night.up" => t!("Still up?"),
        "night.late" => t!("Late one tonight."),
        "morning.hi" => t!("Morning."),
        "morning.go" => t!("New day. Let's go."),
        "hatched.hi" => t!("Hi. I'm %@.", arg1 = name),
        "grew.little" => t!("I grew a little."),
        "tasks.100" => t!("100 tasks together."),
        "egg.roam" => t!("New network. Still here."),
        "egg.thousand" => t!("A thousand. Shiny."),
        "egg.sudo" => t!("Yes, boss."),
        "egg.pogo" => t!("Mosh!"),
        "egg.small_hours" => t!("Small hours. My time."),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use moshpit_pet::talk::LINES;

    use super::*;
    use crate::home::model::{Control, Hook, Link, Mux, PaneInfo, SessionInfo, WindowInfo};

    /// A tmux host `name` with one agent pane, in `state` since `since`, in
    /// a window named `trainer`.
    fn host(id: &str, name: &str, link: Link, state: &str, since: Option<f64>) -> Connection {
        let control = Control {
            mux: Mux::Tmux,
            attached: true,
            sessions: vec![SessionInfo {
                id: "$1".into(),
                name: "work".into(),
            }],
            windows: vec![WindowInfo {
                id: "@1".into(),
                session: "$1".into(),
                index: 1,
                name: "trainer".into(),
                pane_count: 1,
            }],
            panes: vec![PaneInfo {
                id: "%1".into(),
                window: "@1".into(),
                index: 0,
                command: "claude".into(),
                path: None,
            }],
            hooks: vec![Hook {
                pane: "%1".into(),
                state: Some(state.into()),
                agent: Some("claude".into()),
                since,
            }],
            ..Control::default()
        };
        Connection {
            id: id.into(),
            name: name.into(),
            link,
            control: Some(control),
            ..Connection::default()
        }
    }

    const T: f64 = 1_800_000_000.0;

    #[test]
    fn the_perched_agents_edges_are_its_cues_and_launch_says_nothing() {
        let mut e = Edges::default();
        let at = |state, since| vec![host("c", "gpu-01", Link::Live, state, since)];
        assert_eq!(e.next(&at("working", Some(T)), "agent", T), None, "launch");
        // It finishes after 12 minutes: "Done in 12 min.", named as Home does.
        let (cue, fill) = e
            .next(&at("done", Some(T + 720.0)), "agent", T + 721.0)
            .unwrap();
        assert_eq!(
            (cue, fill.minutes, fill.agent.as_str(), fill.host.as_str()),
            (Cue::Done, 12, "trainer", "gpu-01")
        );
        // Back to work within two minutes: no news.
        assert_eq!(
            e.next(&at("working", Some(T + 760.0)), "agent", T + 760.0),
            None
        );
        // A quick turn.
        let (cue, fill) = e
            .next(&at("done", Some(T + 790.0)), "agent", T + 790.0)
            .unwrap();
        assert_eq!((cue, fill.minutes), (Cue::DoneQuick, 0));
        // After a quiet spell, a start is news.
        let (cue, _) = e
            .next(&at("working", Some(T + 1000.0)), "agent", T + 1000.0)
            .unwrap();
        assert_eq!(cue, Cue::Working);
        let (cue, fill) = e
            .next(&at("attention", Some(T + 1100.0)), "agent", T + 1100.0)
            .unwrap();
        assert_eq!((cue, fill.agent.as_str()), (Cue::Waiting, "trainer"));
        assert_eq!(
            e.next(&at("attention", Some(T + 1100.0)), "agent", T + 1105.0),
            None,
            "still"
        );
    }

    #[test]
    fn its_hosts_failure_or_going_offline_and_sleep_on_saved_are_cues() {
        let mut e = Edges::default();
        let up = vec![host("c", "gpu-01", Link::Live, "idle", None)];
        e.next(&up, "agent", T);
        let mut failed = host("c", "gpu-01", Link::Dead, "idle", None);
        failed.failure = Some(crate::home::model::ConnectionFailure {
            reason: crate::home::model::FailureReason::Other,
            detail: String::new(),
        });
        let (cue, fill) = e.next(&[failed], "agent", T + 1.0).unwrap();
        assert_eq!((cue, fill.host.as_str()), (Cue::Error, "gpu-01"));
        let mut e = Edges::default();
        e.next(&up, "agent", T);
        let offline = host("c", "gpu-01", Link::Dead, "idle", None);
        assert_eq!(
            e.next(&[offline], "agent", T + 1.0).map(|(c, _)| c),
            Some(Cue::Offline)
        );
        let mut saved = host("c", "gpu-01", Link::Saved, "idle", None);
        saved.control = None;
        assert_eq!(
            e.next(&[saved.clone()], "agent", T + 2.0).map(|(c, _)| c),
            Some(Cue::Sleeping)
        );
        assert_eq!(e.next(&[saved], "agent", T + 3.0), None, "once");
    }

    fn view(json: serde_json::Value) -> PetView {
        serde_json::from_value(json).unwrap()
    }

    fn pet(tasks: u64, flags: u8, moment: Option<&str>) -> PetView {
        view(serde_json::json!({
            "style": "pixel", "shown": true, "may_hatch": false, "moment": moment,
            "pet": {
                "seed": "07070707070707070707070707070707", "stage": 0, "hatch_hour": 14,
                "flags": flags, "crowned": false, "name": "", "species": "tun",
                "hatched_at": 1_800_000_000, "tasks": tasks, "next": null,
                "traits": {"fur": "natural", "shiny": false, "pattern": "plain", "eyes": "dot", "size": "regular"},
                "history": [], "found": []
            }
        }))
    }

    #[test]
    fn the_milestones_are_cues_once() {
        let mut m = Milestones::default();
        assert_eq!(m.next(&pet(0, 0, Some("hatched")), 1), Some(Cue::Hatched));
        assert_eq!(
            m.next(&pet(0, 0, Some("hatched")), 1),
            None,
            "the same moment"
        );
        assert_eq!(m.next(&pet(99, 0, None), 1), None);
        assert_eq!(m.next(&pet(100, 0, None), 1), Some(Cue::Tasks100));
        assert_eq!(m.next(&pet(101, 1, None), 1), Some(Cue::Roam));
        assert_eq!(m.next(&pet(1000, 1, None), 1), Some(Cue::Thousand));
        assert_eq!(m.next(&pet(1000, 1, Some("grew")), 2), Some(Cue::Grew));
        // A pet already past 100 when the page starts: nothing.
        let mut fresh = Milestones::default();
        assert_eq!(fresh.next(&pet(500, 1, None), 0), None);
    }

    #[test]
    fn every_line_has_its_words_and_each_placeholder_its_argument() {
        // The words are the app's strings (`t!`, read on screen: the Home
        // tests show one); here, each line has its arm, and a line with a
        // placeholder fills it.
        let source = include_str!("talk.rs");
        for line in LINES {
            let at = source
                .find(&format!("\"{}\" =>", line.id))
                .unwrap_or_else(|| panic!("no words for {}", line.id));
            let arm = &source[at..at + source[at..].find("),\n").unwrap()];
            let placeholder = arm.contains("%@") || arm.contains("%lld");
            assert_eq!(placeholder, arm.contains("arg1 ="), "{arm}");
        }
        assert!(talks(&pet(0, 0, None)));
        let mut quiet = pet(0, 0, None);
        quiet.talk = false;
        assert!(!talks(&quiet));
    }
}
