//! The pet record (PET-DESIGN §15.2): the only stored facts. The seed, the
//! hatch, the name, finished tasks per device, the stage and the flags;
//! everything else (species, colours, mutations) is derived. And the rules
//! that change it: the egg's readiness, growing up, the easter-egg flags,
//! renaming, and the sync merge (§15.8).

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use unicode_segmentation::UnicodeSegmentation;

use crate::easter::{self, Egg, FLAG_GOLD, FLAG_ROAMED};
use crate::genome::{Genome, Mutation, Seed, Stage, Traits, evolve};

/// The record's format.
pub const VERSION: u8 = 1;

/// A name's most user-perceived characters.
pub const NAME_MAX: usize = 8;

const DAY: i64 = 86_400;

/// One pet: the person's, on every device they use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PetRecord {
    version: u8,
    #[serde(with = "hex_seed")]
    seed: Seed,
    /// Unix seconds of the reveal.
    hatched_at: i64,
    /// The reveal's local hour, 0–23 (the 03:00 easter egg).
    hatch_hour: u8,
    /// The name it was given; empty: its species' name.
    #[serde(default)]
    name: String,
    #[serde(default)]
    name_at: i64,
    /// Finished agent turns, per device id: a grow-only counter each.
    #[serde(default)]
    finishes: BTreeMap<String, u32>,
    stage: u8,
    /// When each stage was reached (unix seconds).
    #[serde(default)]
    stage_at: [Option<i64>; 3],
    #[serde(default)]
    flags: u8,
}

mod hex_seed {
    use super::*;

    pub fn serialize<S: Serializer>(seed: &Seed, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&seed.iter().map(|b| format!("{b:02x}")).collect::<String>())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Seed, D::Error> {
        let hex = String::deserialize(d)?;
        let bytes: Option<Vec<u8>> = (0..hex.len())
            .step_by(2)
            .map(|i| {
                hex.get(i..i + 2)
                    .and_then(|b| u8::from_str_radix(b, 16).ok())
            })
            .collect();
        bytes
            .and_then(|b| Seed::try_from(b).ok())
            .ok_or_else(|| serde::de::Error::custom("a seed is 32 hex digits"))
    }
}

/// Why stored bytes aren't a record this build can keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// Not a record at all.
    Malformed(String),
    /// A newer format: read-only here, never written over.
    Newer(u8),
}

/// How far a pet has to go to grow (PET-DESIGN §15.4): finished tasks in
/// total, and whole days since the stage before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    pub grown_tasks: u64,
    pub grown_days: i64,
    pub seasoned_tasks: u64,
    pub seasoned_days: i64,
}

impl Thresholds {
    /// 10 tasks and 3 days; 100 tasks and 14 days.
    pub const MEDIUM: Thresholds = Thresholds {
        grown_tasks: 10,
        grown_days: 3,
        seasoned_tasks: 100,
        seasoned_days: 14,
    };
    /// 5 and a day; 50 and a week.
    pub const FASTER: Thresholds = Thresholds {
        grown_tasks: 5,
        grown_days: 1,
        seasoned_tasks: 50,
        seasoned_days: 7,
    };
    /// 20 and a week; 250 and 30 days.
    pub const SLOWER: Thresholds = Thresholds {
        grown_tasks: 20,
        grown_days: 7,
        seasoned_tasks: 250,
        seasoned_days: 30,
    };
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds::MEDIUM
    }
}

/// What it takes to grow next: tasks still to finish and the earliest time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextStage {
    pub stage: Stage,
    pub tasks_left: u64,
    /// Unix seconds.
    pub not_before: i64,
}

impl PetRecord {
    /// A pet revealed at `now` (unix seconds) at local hour `hatch_hour`,
    /// from 16 bytes of the platform's secure random generator.
    pub fn hatch(seed: Seed, now: i64, hatch_hour: u8) -> PetRecord {
        PetRecord {
            version: VERSION,
            seed,
            hatched_at: now,
            hatch_hour: hatch_hour % 24,
            name: String::new(),
            name_at: now,
            finishes: BTreeMap::new(),
            stage: Stage::Hatchling as u8,
            stage_at: [Some(now), None, None],
            flags: 0,
        }
    }

    /// The stored JSON, refusing what this build can't keep.
    pub fn from_json(bytes: &[u8]) -> Result<PetRecord, RecordError> {
        let record: PetRecord =
            serde_json::from_slice(bytes).map_err(|e| RecordError::Malformed(e.to_string()))?;
        if record.version > VERSION {
            return Err(RecordError::Newer(record.version));
        }
        if Stage::from_index(record.stage).is_none() {
            return Err(RecordError::Malformed(format!("stage {}", record.stage)));
        }
        Ok(record)
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn seed(&self) -> &Seed {
        &self.seed
    }

    pub fn hatched_at(&self) -> i64 {
        self.hatched_at
    }

    pub fn hatch_hour(&self) -> u8 {
        self.hatch_hour
    }

    pub fn stage(&self) -> Stage {
        Stage::from_index(self.stage).unwrap_or(Stage::Seasoned)
    }

    /// When a stage was reached.
    pub fn stage_at(&self, stage: Stage) -> Option<i64> {
        self.stage_at[stage.index()]
    }

    pub fn flags(&self) -> u8 {
        self.flags
    }

    /// The name it was given; empty when it has its species' name.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn genome(&self) -> Genome {
        Genome::roll(&self.seed, self.hatch_hour)
    }

    /// What it looks like now, and the mutation of each stage reached.
    pub fn traits(&self) -> (Traits, Vec<(Stage, Mutation)>) {
        evolve(&self.seed, self.genome(), self.stage())
    }

    /// Finished tasks, every device's.
    pub fn tasks(&self) -> u64 {
        self.finishes.values().map(|n| u64::from(*n)).sum()
    }

    /// An agent finished a turn, seen on `device`. The easter egg it found,
    /// if any (the thousandth task).
    pub fn record_finish(&mut self, device: &str) -> Option<Egg> {
        let n = self.finishes.entry(device.to_owned()).or_insert(0);
        *n = n.saturating_add(1);
        if easter::gold(self.tasks()) && self.flags & FLAG_GOLD == 0 {
            self.flags |= FLAG_GOLD;
            return Some(Egg::Task1000);
        }
        None
    }

    /// A mosh session roamed and kept going. The easter egg, the first time.
    pub fn record_roam(&mut self) -> Option<Egg> {
        if self.flags & FLAG_ROAMED == 0 {
            self.flags |= FLAG_ROAMED;
            return Some(Egg::MadeItThrough);
        }
        None
    }

    /// Renames it at `now`: trimmed, at most [`NAME_MAX`] characters; an
    /// empty name gives it back its species' name.
    pub fn rename(&mut self, name: &str, now: i64) {
        self.name = name.trim().graphemes(true).take(NAME_MAX).collect();
        self.name_at = now;
    }

    /// The next stage and what it takes, or none when it's seasoned.
    pub fn next_stage(&self, t: &Thresholds) -> Option<NextStage> {
        let (stage, tasks, days, since) = match self.stage() {
            Stage::Hatchling => (Stage::Grown, t.grown_tasks, t.grown_days, self.hatched_at),
            Stage::Grown => (
                Stage::Seasoned,
                t.seasoned_tasks,
                t.seasoned_days,
                self.stage_at(Stage::Grown).unwrap_or(self.hatched_at),
            ),
            Stage::Seasoned => return None,
        };
        Some(NextStage {
            stage,
            tasks_left: tasks.saturating_sub(self.tasks()),
            not_before: since + days * DAY,
        })
    }

    /// Whether it may grow at `now`.
    pub fn due(&self, now: i64, t: &Thresholds) -> Option<Stage> {
        self.next_stage(t)
            .filter(|n| n.tasks_left == 0 && now >= n.not_before)
            .map(|n| n.stage)
    }

    /// Grows one stage if it's due — only while it's `shown`: counting goes
    /// on while the companion is off, and the moment waits for it. The new
    /// stage, to play the moment for.
    pub fn grow(&mut self, now: i64, t: &Thresholds, shown: bool) -> Option<Stage> {
        if !shown {
            return None;
        }
        let stage = self.due(now, t)?;
        self.stage = stage as u8;
        self.stage_at[stage.index()] = Some(now);
        Some(stage)
    }

    /// The easter eggs it has found, for the pet card.
    pub fn found(&self) -> Vec<Egg> {
        let mut eggs = Vec::new();
        if easter::night_hatch(self.hatch_hour) {
            eggs.push(Egg::NightHatch);
        }
        if self.flags & FLAG_ROAMED != 0 {
            eggs.push(Egg::MadeItThrough);
        }
        if self.flags & FLAG_GOLD != 0 {
            eggs.push(Egg::Task1000);
        }
        if easter::crowned(&self.name) {
            eggs.push(Egg::Sudo);
        }
        eggs
    }

    /// What decides every pixel: a few bytes.
    pub fn look(&self) -> crate::sprite::Look {
        crate::sprite::Look {
            seed: self.seed,
            stage: self.stage(),
            hatch_hour: self.hatch_hour,
            flags: self.flags,
            crowned: easter::crowned(&self.name),
        }
    }

    fn identity(&self) -> (i64, Seed) {
        (self.hatched_at, self.seed)
    }
}

/// Two copies of the person's pet, as one (PET-DESIGN §15.8). Commutative,
/// associative and idempotent:
/// - two different pets (two devices hatched offline before ever syncing):
///   the earlier hatch wins, a tie the lower seed; the tasks are the
///   person's, so both devices' counts carry over;
/// - the same pet: the last name written, every device's highest count,
///   the highest stage with the earliest time each was reached, and every
///   flag either has.
pub fn merge(a: &PetRecord, b: &PetRecord) -> PetRecord {
    let finishes = {
        let mut f = a.finishes.clone();
        for (device, n) in &b.finishes {
            let e = f.entry(device.clone()).or_insert(0);
            *e = (*e).max(*n);
        }
        f
    };
    let (first, second) = if a.identity() <= b.identity() {
        (a, b)
    } else {
        (b, a)
    };
    if first.seed != second.seed {
        return PetRecord {
            finishes,
            ..first.clone()
        };
    }
    let (name_at, name) = (first.name_at, &first.name).max((second.name_at, &second.name));
    let mut stage_at = [None; 3];
    for (i, at) in stage_at.iter_mut().enumerate() {
        *at = match (first.stage_at[i], second.stage_at[i]) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, y) => x.or(y),
        };
    }
    PetRecord {
        version: first.version.max(second.version),
        seed: first.seed,
        hatched_at: first.hatched_at,
        hatch_hour: first.hatch_hour,
        name: name.clone(),
        name_at,
        finishes,
        stage: first.stage.max(second.stage),
        stage_at,
        flags: first.flags | second.flags,
    }
}

/// What to keep after reading this device's record and the cloud's, and
/// which side needs writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciled {
    pub record: Option<PetRecord>,
    pub write_local: bool,
    pub write_cloud: bool,
}

/// Brings this device and the cloud to one record.
pub fn reconcile(local: Option<&PetRecord>, cloud: Option<&PetRecord>) -> Reconciled {
    let record = match (local, cloud) {
        (Some(l), Some(c)) => Some(merge(l, c)),
        (l, c) => l.or(c).cloned(),
    };
    Reconciled {
        write_local: record.as_ref() != local,
        write_cloud: record.as_ref() != cloud,
        record,
    }
}

/// The egg before it hatches: whether it's ready (PET-DESIGN §15.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EggProgress {
    /// Successful connections seen.
    #[serde(default)]
    pub connections: u32,
    /// Whether an agent has finished a turn.
    #[serde(default)]
    pub agent_finished: bool,
}

/// Connections that ready the egg for someone who runs no agents.
pub const EGG_CONNECTIONS: u32 = 3;

impl EggProgress {
    /// Cracked: the first finished agent turn, or the third connection.
    pub fn ready(&self) -> bool {
        self.agent_finished || self.connections >= EGG_CONNECTIONS
    }

    pub fn connected(&mut self) {
        self.connections = self.connections.saturating_add(1);
    }

    pub fn agent_finished(&mut self) {
        self.agent_finished = true;
    }
}

/// When the hatch plays: the egg is ready, the person is on Home, the
/// companion is shown, and the first iCloud sync has settled (or there's
/// no iCloud), so a pet on their other device is adopted rather than a
/// second one hatched.
pub fn may_hatch(
    egg: &EggProgress,
    on_home: bool,
    companion_shown: bool,
    cloud_settled: bool,
) -> bool {
    egg.ready() && on_home && companion_shown && cloud_settled
}

#[cfg(test)]
impl PetRecord {
    pub(crate) fn with_stage(mut self, stage: Stage, at: i64) -> PetRecord {
        self.stage = stage as u8;
        self.stage_at[stage.index()] = Some(at);
        self
    }

    pub(crate) fn with_finishes(mut self, device: &str, n: u32) -> PetRecord {
        self.finishes.insert(device.to_owned(), n);
        self
    }
}
