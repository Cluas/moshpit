//! The pet talks (moshpit-design `docs/nsh/PET-DESIGN.md` §17): a short
//! line in a bubble on Home when the perched agent starts, needs you or
//! finishes, when the person taps it, at night and in the morning, and at
//! its milestones. Rare on purpose: one unprompted line every four minutes
//! at most, eight an hour; only while Home is on screen; a tap always
//! answers.
//!
//! [`Chatter`] is the page's: it keeps the clock of the rate limit, the
//! last line per cue and the night's and the morning's marks, in memory. It
//! chooses a line from [`LINES`] seeded by the pet's seed, so each pet has
//! its favourites. The words are the app's strings; a line here is an id
//! the screen turns into them.

use std::collections::{BTreeMap, VecDeque};

use sha2::{Digest, Sha256};

/// What made it talk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cue {
    /// The person tapped it.
    Tap,
    /// The perched agent started working after a quiet spell.
    Working,
    /// The perched agent waits on the person.
    Waiting,
    /// The perched agent finished a turn of a minute or more.
    Done,
    /// The same, under a minute.
    DoneQuick,
    /// Its host's connection failed.
    Error,
    /// Its host went offline.
    Offline,
    /// Nothing runs anywhere: it went to sleep.
    Sleeping,
    /// The first Home visit between 00:00 and 04:59.
    Night,
    /// The first Home visit between 06:00 and 09:59.
    Morning,
    /// The hatch.
    Hatched,
    /// It grew a stage.
    Grew,
    /// The 100th task together.
    Tasks100,
    /// A mosh roam it lived through (the easter egg).
    Roam,
    /// The 1000th task (the easter egg).
    Thousand,
    /// Seven quick taps (the easter egg).
    Pogo,
}

impl Cue {
    /// Said whenever it happens on Home, past the four-minute gap; one
    /// that happens while Home is hidden waits for the next visit.
    pub fn milestone(self) -> bool {
        matches!(
            self,
            Cue::Hatched | Cue::Grew | Cue::Tasks100 | Cue::Roam | Cue::Thousand
        )
    }

    /// The person asked (a tap): it always answers.
    pub fn prompted(self) -> bool {
        matches!(self, Cue::Tap | Cue::Pogo)
    }
}

/// One line: its id (the screen's key for its words), what it answers,
/// and the one species it belongs to, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub id: &'static str,
    pub cue: Cue,
    pub species: Option<&'static str>,
    /// An easter egg's line among the taps (the name `sudo`; Lumen in the
    /// small hours): said only when that holds.
    pub only: Only,
}

/// When a tap's line may be said at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Only {
    Always,
    /// The pet is named `sudo`.
    Sudo,
    /// Between 00:00 and 04:59.
    SmallHours,
}

const fn line(id: &'static str, cue: Cue) -> Line {
    Line {
        id,
        cue,
        species: None,
        only: Only::Always,
    }
}

const fn voice(id: &'static str, species: &'static str) -> Line {
    Line {
        id,
        cue: Cue::Tap,
        species: Some(species),
        only: Only::Always,
    }
}

/// v1's lines (§17.2), 38 of them.
pub const LINES: &[Line] = &[
    line("tap.hm", Cue::Tap),
    line("tap.here", Cue::Tap),
    line("tap.tickles", Cue::Tap),
    line("tap.watching", Cue::Tap),
    voice("tap.tun", "tun"),
    voice("tap.coo", "coo"),
    voice("tap.dig", "dig"),
    voice("tap.hex", "hex"),
    voice("tap.ether", "ether"),
    voice("tap.moth", "moth"),
    Line {
        id: "egg.sudo",
        cue: Cue::Tap,
        species: None,
        only: Only::Sudo,
    },
    Line {
        id: "egg.small_hours",
        cue: Cue::Tap,
        species: Some("moth"),
        only: Only::SmallHours,
    },
    line("working.on_it", Cue::Working),
    line("working.off", Cue::Working),
    line("working.watch", Cue::Working),
    line("waiting.needs", Cue::Waiting),
    line("waiting.turn", Cue::Waiting),
    line("waiting.question", Cue::Waiting),
    line("done.minutes", Cue::Done),
    line("done.finished", Cue::Done),
    line("done.wrap", Cue::Done),
    line("done.quick", Cue::DoneQuick),
    line("error.broke", Cue::Error),
    line("error.silent", Cue::Error),
    line("offline.lost", Cue::Offline),
    line("offline.wait", Cue::Offline),
    line("sleeping.nap", Cue::Sleeping),
    line("sleeping.quiet", Cue::Sleeping),
    line("night.up", Cue::Night),
    line("night.late", Cue::Night),
    line("morning.hi", Cue::Morning),
    line("morning.go", Cue::Morning),
    line("hatched.hi", Cue::Hatched),
    line("grew.little", Cue::Grew),
    line("tasks.100", Cue::Tasks100),
    line("egg.roam", Cue::Roam),
    line("egg.thousand", Cue::Thousand),
    line("egg.pogo", Cue::Pogo),
];

/// The rate limit (§17.1).
pub const GAP_SECS: f64 = 240.0;
pub const PER_HOUR: usize = 8;
/// Home settles this long after it appears before anything is said.
pub const SETTLE_SECS: f64 = 1.5;

/// Who talks: its seed (its favourites), its species (its voice), whether
/// it's named `sudo`, and the local hour.
#[derive(Debug, Clone, Copy)]
pub struct Speaker<'a> {
    pub seed: &'a [u8],
    pub species: &'a str,
    pub sudo: bool,
    pub local_hour: u8,
}

/// The page's chatter: whether, and what, the pet says.
#[derive(Debug, Default)]
pub struct Chatter {
    /// When Home last appeared; none while it's hidden.
    shown_since: Option<f64>,
    /// When unprompted lines were said, the last hour's.
    said: VecDeque<f64>,
    /// The last line per cue (no line twice in a row).
    last: BTreeMap<Cue, &'static str>,
    /// Milestones that happened while Home was hidden, said on the next
    /// visit (once each).
    pending: Vec<Cue>,
    /// The local day of the night's and the morning's lines.
    night_on: Option<i64>,
    morning_on: Option<i64>,
    /// How many lines it has chosen: the draw's counter.
    drawn: u64,
}

impl Chatter {
    /// Home is on screen from `now` (or hidden).
    pub fn visible(&mut self, visible: bool, now: f64) {
        self.shown_since = visible.then_some(now);
    }

    fn settled(&self, now: f64) -> bool {
        self.shown_since
            .is_some_and(|since| now - since >= SETTLE_SECS)
    }

    /// Home has settled after appearing (the screen asks [`SETTLE_SECS`]
    /// after it does): a milestone kept for this visit, else the night's or
    /// the morning's line, once a day. `day` is the local day's number.
    pub fn arrived(&mut self, who: Speaker<'_>, now: f64, day: i64) -> Option<&'static str> {
        if !self.settled(now) {
            return None;
        }
        if !self.pending.is_empty() {
            let cue = self.pending.remove(0);
            return self.say(cue, who, now);
        }
        match who.local_hour {
            0..=4 if self.night_on != Some(day) => {
                let said = self.say(Cue::Night, who, now);
                if said.is_some() {
                    self.night_on = Some(day);
                }
                said
            }
            6..=9 if self.morning_on != Some(day) => {
                let said = self.say(Cue::Morning, who, now);
                if said.is_some() {
                    self.morning_on = Some(day);
                }
                said
            }
            _ => None,
        }
    }

    /// `cue` happened at `now`: the line to say, if it says one.
    pub fn cue(&mut self, cue: Cue, who: Speaker<'_>, now: f64) -> Option<&'static str> {
        if cue.prompted() {
            return self.pick(cue, who);
        }
        if !self.settled(now) {
            if cue.milestone() && !self.pending.contains(&cue) {
                self.pending.push(cue);
            }
            return None;
        }
        self.say(cue, who, now)
    }

    /// An unprompted line within the rate limit (a milestone past the
    /// gap), counted.
    fn say(&mut self, cue: Cue, who: Speaker<'_>, now: f64) -> Option<&'static str> {
        while self.said.front().is_some_and(|at| now - at >= 3600.0) {
            self.said.pop_front();
        }
        let gap_ok = self.said.back().is_none_or(|at| now - at >= GAP_SECS);
        if !cue.milestone() && (!gap_ok || self.said.len() >= PER_HOUR) {
            return None;
        }
        let line = self.pick(cue, who)?;
        self.said.push_back(now);
        Some(line)
    }

    /// One of `cue`'s lines for `who`: its favourite three times as likely,
    /// its species' twice, never the last one again.
    fn pick(&mut self, cue: Cue, who: Speaker<'_>) -> Option<&'static str> {
        let small_hours = who.local_hour <= 4;
        let lines: Vec<&Line> = LINES
            .iter()
            .filter(|l| l.cue == cue)
            .filter(|l| l.species.is_none_or(|s| s == who.species))
            .filter(|l| match l.only {
                Only::Always => true,
                Only::Sudo => who.sudo,
                Only::SmallHours => small_hours,
            })
            .collect();
        if lines.is_empty() {
            return None;
        }
        let last = self.last.get(&cue).copied();
        let favourite = usize::from(digest(who.seed, cue_name(cue), 0)[0]) % lines.len();
        let weighted: Vec<(&Line, u32)> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| lines.len() == 1 || Some(l.id) != last)
            .map(|(i, l)| {
                let weight = match (l.only, l.species) {
                    (Only::Sudo | Only::SmallHours, _) => 4,
                    (_, Some(_)) => 2,
                    _ if i == favourite => 3,
                    _ => 1,
                };
                (*l, weight)
            })
            .collect();
        let total: u32 = weighted.iter().map(|(_, w)| w).sum();
        self.drawn += 1;
        let d = digest(who.seed, cue_name(cue), self.drawn);
        let mut at = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) % total;
        let chosen = weighted
            .iter()
            .find(|(_, w)| {
                if at < *w {
                    true
                } else {
                    at -= w;
                    false
                }
            })
            .map(|(l, _)| l.id)?;
        self.last.insert(cue, chosen);
        Some(chosen)
    }
}

fn cue_name(cue: Cue) -> &'static str {
    match cue {
        Cue::Tap => "tap",
        Cue::Working => "working",
        Cue::Waiting => "waiting",
        Cue::Done => "done",
        Cue::DoneQuick => "done_quick",
        Cue::Error => "error",
        Cue::Offline => "offline",
        Cue::Sleeping => "sleeping",
        Cue::Night => "night",
        Cue::Morning => "morning",
        Cue::Hatched => "hatched",
        Cue::Grew => "grew",
        Cue::Tasks100 => "tasks_100",
        Cue::Roam => "roam",
        Cue::Thousand => "thousand",
        Cue::Pogo => "pogo",
    }
}

fn digest(seed: &[u8], cue: &str, n: u64) -> [u8; 32] {
    Sha256::new()
        .chain_update(b"moshpit/talk/v1\x00")
        .chain_update(seed)
        .chain_update(cue.as_bytes())
        .chain_update(n.to_le_bytes())
        .finalize()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(seed: &[u8]) -> Speaker<'_> {
        Speaker {
            seed,
            species: "tun",
            sudo: false,
            local_hour: 14,
        }
    }

    const SEED: [u8; 16] = [7; 16];
    const T: f64 = 1_800_000_000.0;

    fn shown(at: f64) -> Chatter {
        let mut c = Chatter::default();
        c.visible(true, at);
        c
    }

    #[test]
    fn unprompted_lines_wait_four_minutes_and_stop_at_eight_an_hour() {
        let mut c = shown(T);
        let t = T + SETTLE_SECS;
        assert!(c.cue(Cue::Working, who(&SEED), t).is_some());
        assert_eq!(
            c.cue(Cue::Done, who(&SEED), t + 60.0),
            None,
            "inside the gap"
        );
        assert!(c.cue(Cue::Done, who(&SEED), t + GAP_SECS).is_some());
        // Over a long day: never more than eight in any hour.
        let mut c = shown(T);
        let said: Vec<f64> = (0..200)
            .map(|i| t + f64::from(i) * 30.0)
            .filter(|at| c.cue(Cue::Waiting, who(&SEED), *at).is_some())
            .collect();
        for (i, at) in said.iter().enumerate() {
            let in_hour = said[i..].iter().take_while(|b| **b - at < 3600.0).count();
            assert!(in_hour <= PER_HOUR, "{in_hour} in the hour from {at}");
        }
        assert!(said.windows(2).all(|w| w[1] - w[0] >= GAP_SECS));
    }

    #[test]
    fn a_tap_always_answers_and_leaves_the_clock_alone() {
        let mut c = shown(T);
        let t = T + SETTLE_SECS;
        assert!(c.cue(Cue::Working, who(&SEED), t).is_some());
        for i in 0..20 {
            assert!(
                c.cue(Cue::Tap, who(&SEED), t + f64::from(i)).is_some(),
                "tap {i}"
            );
        }
        assert!(c.cue(Cue::Pogo, who(&SEED), t + 21.0) == Some("egg.pogo"));
        assert_eq!(
            c.cue(Cue::Done, who(&SEED), t + 30.0),
            None,
            "still the gap"
        );
        // Hidden or not yet settled, a tap still answers.
        let mut hidden = Chatter::default();
        assert!(hidden.cue(Cue::Tap, who(&SEED), t).is_some());
    }

    #[test]
    fn nothing_while_home_is_hidden_and_a_milestone_waits_for_the_next_visit() {
        let mut c = Chatter::default();
        assert_eq!(c.cue(Cue::Working, who(&SEED), T), None, "hidden");
        assert_eq!(c.cue(Cue::Grew, who(&SEED), T), None, "kept");
        c.visible(true, T + 100.0);
        assert_eq!(c.cue(Cue::Waiting, who(&SEED), T + 100.5), None, "settling");
        assert_eq!(c.arrived(who(&SEED), T + 100.5, 1), None, "not settled yet");
        assert_eq!(c.arrived(who(&SEED), T + 102.0, 1), Some("grew.little"));
        assert_eq!(c.arrived(who(&SEED), T + 103.0, 1), None, "said once");
        // The milestone counts: an edge waits out the gap after it; another
        // milestone doesn't.
        assert_eq!(c.cue(Cue::Waiting, who(&SEED), T + 104.0), None);
        assert_eq!(
            c.cue(Cue::Tasks100, who(&SEED), T + 105.0),
            Some("tasks.100")
        );
        c.visible(false, T + 106.0);
        assert_eq!(c.cue(Cue::Waiting, who(&SEED), T + 9999.0), None);
    }

    #[test]
    fn night_and_morning_once_a_day() {
        let mut c = shown(T);
        let night = Speaker {
            local_hour: 2,
            ..who(&SEED)
        };
        let morning = Speaker {
            local_hour: 7,
            ..who(&SEED)
        };
        let noon = who(&SEED);
        let t = T + SETTLE_SECS;
        assert!(
            c.arrived(night, t, 10)
                .is_some_and(|l| l.starts_with("night."))
        );
        c.visible(true, t + 600.0);
        assert_eq!(c.arrived(night, t + 602.0, 10), None, "once a night");
        assert_eq!(c.arrived(noon, t + 602.0, 10), None);
        c.visible(true, t + 30_000.0);
        assert!(
            c.arrived(morning, t + 30_002.0, 10)
                .is_some_and(|l| l.starts_with("morning."))
        );
        c.visible(true, t + 90_000.0);
        assert!(
            c.arrived(night, t + 90_002.0, 11).is_some(),
            "the next night"
        );
    }

    #[test]
    fn each_pet_has_its_favourites_and_never_repeats_a_line() {
        // The same seed draws the same lines.
        let draw = |seed: &[u8]| {
            let mut c = Chatter::default();
            (0..30)
                .map(|_| c.cue(Cue::Tap, who(seed), T).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(draw(&SEED), draw(&SEED));
        let lines = draw(&SEED);
        assert!(lines.windows(2).all(|w| w[0] != w[1]), "{lines:?}");
        // Over many pets, every tap line a Tun may say gets said, and pets
        // differ in what they say most.
        let mut seen = std::collections::BTreeSet::new();
        let mut most = std::collections::BTreeSet::new();
        for n in 0..200u8 {
            let lines = draw(&[n; 16]);
            let mut counts = BTreeMap::new();
            for l in &lines {
                *counts.entry(*l).or_insert(0) += 1;
                seen.insert(*l);
            }
            most.insert(counts.into_iter().max_by_key(|(_, n)| *n).unwrap().0);
        }
        let tun: Vec<_> = LINES
            .iter()
            .filter(|l| l.cue == Cue::Tap && l.only == Only::Always)
            .filter(|l| l.species.is_none_or(|s| s == "tun"))
            .map(|l| l.id)
            .collect();
        assert!(tun.iter().all(|id| seen.contains(id)), "{seen:?}");
        assert!(most.len() >= 3, "{most:?}");
        assert!(!seen.contains("tap.coo"), "Coo's voice is Coo's");
    }

    #[test]
    fn the_easter_eggs_speak_only_when_they_hold() {
        let draw = |who: Speaker<'_>| {
            let mut c = Chatter::default();
            (0..60)
                .map(|_| c.cue(Cue::Tap, who, T).unwrap())
                .collect::<Vec<_>>()
        };
        assert!(!draw(who(&SEED)).contains(&"egg.sudo"));
        assert!(
            draw(Speaker {
                sudo: true,
                ..who(&SEED)
            })
            .contains(&"egg.sudo")
        );
        let lumen = Speaker {
            species: "moth",
            ..who(&SEED)
        };
        assert!(!draw(lumen).contains(&"egg.small_hours"));
        assert!(
            draw(Speaker {
                local_hour: 3,
                ..lumen
            })
            .contains(&"egg.small_hours")
        );
    }

    #[test]
    fn every_cue_has_a_line_and_the_table_is_v1s() {
        assert_eq!(LINES.len(), 38);
        let ids: std::collections::BTreeSet<_> = LINES.iter().map(|l| l.id).collect();
        assert_eq!(ids.len(), LINES.len(), "ids are unique");
        for cue in [
            Cue::Tap,
            Cue::Working,
            Cue::Waiting,
            Cue::Done,
            Cue::DoneQuick,
            Cue::Error,
            Cue::Offline,
            Cue::Sleeping,
            Cue::Night,
            Cue::Morning,
            Cue::Hatched,
            Cue::Grew,
            Cue::Tasks100,
            Cue::Roam,
            Cue::Thousand,
            Cue::Pogo,
        ] {
            let mut c = shown(T);
            let t = T + SETTLE_SECS;
            let who = Speaker {
                local_hour: 2,
                ..who(&SEED)
            };
            assert!(
                c.cue(cue, who, t).is_some() || cue == Cue::Night || cue == Cue::Morning,
                "{cue:?}"
            );
            assert!(LINES.iter().any(|l| l.cue == cue), "{cue:?}");
        }
    }
}
