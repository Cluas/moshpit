//! What a pet is, from its seed (PET-DESIGN §15.3, §15.5): the species,
//! its fur, a shiny, pattern, eyes, size and where things grow; and one
//! mutation per stage reached. Nothing is stored but the seed: every
//! device, and the Live Activity's sprite, derives the same pet.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::art::art;

/// A pet's seed: 16 bytes from the platform's secure random generator,
/// taken at the hatch and never changed.
pub type Seed = [u8; 16];

/// The local hour of a hatch that brings the hidden species.
pub const NIGHT_HOUR: u8 = 3;

/// The species, in roll order (the weights accumulate in this order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Species {
    /// 墩墩, a tardigrade: the lead.
    Tun,
    /// 咕咕, a pigeon.
    Coo,
    /// 挖挖, a mole in a miner's lamp.
    Dig,
    /// 六六, an axolotl.
    Hex,
    /// 漂漂, a jellyfish: rare.
    Ether,
    /// 扑扑 (Lumen), a moth: hidden, a 03:00 hatch only.
    Moth,
}

impl Species {
    pub const ALL: [Species; 6] = [
        Species::Tun,
        Species::Coo,
        Species::Dig,
        Species::Hex,
        Species::Ether,
        Species::Moth,
    ];

    /// Its id in the art data.
    pub fn id(self) -> &'static str {
        match self {
            Species::Tun => "tun",
            Species::Coo => "coo",
            Species::Dig => "dig",
            Species::Hex => "hex",
            Species::Ether => "ether",
            Species::Moth => "moth",
        }
    }

    pub fn from_id(id: &str) -> Option<Species> {
        Species::ALL.into_iter().find(|s| s.id() == id)
    }

    /// Its weight in the roll, out of 1000 (the hidden one: 0).
    pub fn weight(self) -> u32 {
        art().characters.get(self.id()).map_or(0, |c| c.weight)
    }

    /// Its name (`zh`, `en`): what a pet is called until it's named.
    pub fn name(self) -> (&'static str, &'static str) {
        let n = &art().characters[self.id()].name;
        (&n.zh, &n.en)
    }

    /// What it is (`zh`, `en`): 水熊虫, tardigrade.
    pub fn kind(self) -> (&'static str, &'static str) {
        let k = &art().characters[self.id()].kind;
        (&k.zh, &k.en)
    }
}

/// How far a pet has grown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    /// 幼崽: smaller, wearing half its eggshell.
    Hatchling = 0,
    /// 成年.
    Grown = 1,
    /// 资深: wider, with its species' mark.
    Seasoned = 2,
}

impl Stage {
    pub const ALL: [Stage; 3] = [Stage::Hatchling, Stage::Grown, Stage::Seasoned];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: u8) -> Option<Stage> {
        Stage::ALL.get(usize::from(i)).copied()
    }

    pub fn next(self) -> Option<Stage> {
        Stage::from_index(self as u8 + 1)
    }
}

/// A pattern on the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pattern {
    Plain,
    Spots,
    Stripes,
    Patch,
}

/// How the open eyes look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Eyes {
    Round,
    Lidded,
    Starry,
    Button,
}

/// A body one row shorter, as drawn, or one row taller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Size {
    Short,
    Standard,
    Tall,
}

/// Where things grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Head,
    Face,
}

/// Something grown at a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Growth {
    Sprout,
    MossTuft,
    Crystal,
    Freckles,
    StarMark,
}

impl Growth {
    pub const ALL: [Growth; 5] = [
        Growth::Sprout,
        Growth::MossTuft,
        Growth::Crystal,
        Growth::Freckles,
        Growth::StarMark,
    ];

    /// Its accessory's id in the art data.
    pub fn id(self) -> &'static str {
        match self {
            Growth::Sprout => "sprout",
            Growth::MossTuft => "moss_tuft",
            Growth::Crystal => "crystal",
            Growth::Freckles => "freckles",
            Growth::StarMark => "star_mark",
        }
    }

    pub fn from_id(id: &str) -> Option<Growth> {
        Growth::ALL.into_iter().find(|g| g.id() == id)
    }

    fn on_head(self) -> bool {
        art()
            .accessories
            .get(self.id())
            .is_some_and(|a| a.slot == "head")
    }
}

/// Everything a pet is born with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Genome {
    pub species: Species,
    /// 0: the species' own colours; 1–9: a fur from the curated set.
    pub fur: u8,
    pub shiny: bool,
    pub pattern: Pattern,
    pub eyes: Eyes,
    pub size: Size,
    pub slot: Slot,
}

const FURS: u8 = 9;

fn pick<T: Copy>(all: &[T], byte: u8) -> T {
    all[usize::from(byte) % all.len()]
}

impl Genome {
    /// The roll (PET-DESIGN §15.3): `g = SHA-256("moshpit/pet/v1" ‖ 0 ‖ seed)`.
    /// The species by `u16(g[0..2]) % 1000` over the weights in
    /// [`Species::ALL`] order (the bias of the modulo is under 0.002 %), or
    /// the moth when the reveal's local hour is [`NIGHT_HOUR`].
    pub fn roll(seed: &Seed, hatch_hour: u8) -> Genome {
        let g: [u8; 32] = Sha256::new()
            .chain_update(b"moshpit/pet/v1\x00")
            .chain_update(seed)
            .finalize()
            .into();
        let value = u32::from(u16::from_be_bytes([g[0], g[1]])) % 1000;
        let mut acc = 0;
        let mut species = Species::Tun;
        for s in Species::ALL {
            acc += s.weight();
            if value < acc {
                species = s;
                break;
            }
        }
        if hatch_hour == NIGHT_HOUR {
            species = Species::Moth;
        }
        Genome {
            species,
            fur: if g[2] < 77 { 0 } else { 1 + (g[2] - 77) % FURS },
            shiny: g[3] < 4,
            pattern: pick(
                &[
                    Pattern::Plain,
                    Pattern::Spots,
                    Pattern::Stripes,
                    Pattern::Patch,
                ],
                g[4],
            ),
            eyes: pick(
                &[Eyes::Round, Eyes::Lidded, Eyes::Starry, Eyes::Button],
                g[5],
            ),
            size: pick(&[Size::Short, Size::Standard, Size::Tall], g[6]),
            slot: pick(&[Slot::Head, Slot::Face], g[7]),
        }
    }
}

impl Traits {
    /// Its fur's id and Chinese name, for the pet card (`natural`/原色,
    /// `aurora`/极光 for a shiny).
    pub fn fur_name(&self) -> (&'static str, &'static str) {
        let g = &art().genome;
        if self.shiny {
            return (&g.shiny.id, &g.shiny.zh);
        }
        g.furs
            .get(usize::from(self.fur))
            .map_or(("natural", "原色"), |f| (f.id.as_str(), f.zh.as_str()))
    }
}

/// A change at a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mutation {
    /// The fur moved to a neighbouring one (or, from its own colours, to one of the set).
    Fur(u8),
    /// A pattern appeared or changed.
    Pattern(Pattern),
    /// Something grew.
    Grew(Growth),
    /// The eyes changed.
    Eyes(Eyes),
}

/// What a pet looks like at a stage: its genome, as mutated.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Traits {
    pub species: Species,
    pub fur: u8,
    pub shiny: bool,
    pub pattern: Pattern,
    pub eyes: Eyes,
    pub size: Size,
    pub slot: Slot,
    pub grown: Vec<Growth>,
}

impl From<Genome> for Traits {
    fn from(g: Genome) -> Self {
        Traits {
            species: g.species,
            fur: g.fur,
            shiny: g.shiny,
            pattern: g.pattern,
            eyes: g.eyes,
            size: g.size,
            slot: g.slot,
            grown: Vec::new(),
        }
    }
}

const PATTERNS: [Pattern; 4] = [
    Pattern::Plain,
    Pattern::Spots,
    Pattern::Stripes,
    Pattern::Patch,
];
const EYES: [Eyes; 4] = [Eyes::Round, Eyes::Lidded, Eyes::Starry, Eyes::Button];

fn grown_in(slot: Slot) -> Vec<Growth> {
    let name = match slot {
        Slot::Head => "head",
        Slot::Face => "face",
    };
    art()
        .genome
        .grown
        .get(name)
        .map(|ids| ids.iter().filter_map(|id| Growth::from_id(id)).collect())
        .unwrap_or_default()
}

/// The traits at `stage`, and the one mutation each stage reached rolled
/// (PET-DESIGN §15.5): `m = SHA-256("moshpit/pet/mutation/v1" ‖ 0 ‖ seed ‖ k)`
/// for stage `k`; `m[0] % 4` the kind, `m[1]` the choice.
pub fn evolve(seed: &Seed, genome: Genome, stage: Stage) -> (Traits, Vec<(Stage, Mutation)>) {
    let mut t = Traits::from(genome);
    let mut history = Vec::new();
    for k in 1..=stage as u8 {
        let m: [u8; 32] = Sha256::new()
            .chain_update(b"moshpit/pet/mutation/v1\x00")
            .chain_update(seed)
            .chain_update([k])
            .finalize()
            .into();
        let mut kind = m[0] % 4;
        if kind == 0 && t.shiny {
            kind = 3; // a shiny keeps its colours
        }
        let mutation = match kind {
            0 => {
                t.fur = if t.fur == 0 {
                    1 + m[1] % FURS
                } else {
                    1 + (t.fur - 1 + if m[1] & 1 == 1 { 1 } else { FURS - 1 }) % FURS
                };
                Mutation::Fur(t.fur)
            }
            1 => {
                let choices: Vec<Pattern> = PATTERNS[1..]
                    .iter()
                    .copied()
                    .filter(|p| *p != t.pattern)
                    .collect();
                t.pattern = pick(&choices, m[1]);
                Mutation::Pattern(t.pattern)
            }
            2 => {
                let mut slot = t.slot;
                let mut options: Vec<Growth> = grown_in(slot)
                    .into_iter()
                    .filter(|g| !t.grown.contains(g))
                    .collect();
                if options.is_empty() || (t.grown.iter().any(|g| g.on_head()) && slot == Slot::Head)
                {
                    slot = if slot == Slot::Head {
                        Slot::Face
                    } else {
                        Slot::Head
                    };
                    options = grown_in(slot)
                        .into_iter()
                        .filter(|g| !t.grown.contains(g))
                        .collect();
                }
                match options.is_empty() {
                    // Never with two stages; the eyes change rather than nothing.
                    true => {
                        let choices: Vec<Eyes> =
                            EYES.iter().copied().filter(|e| *e != t.eyes).collect();
                        t.eyes = pick(&choices, m[1]);
                        Mutation::Eyes(t.eyes)
                    }
                    false => {
                        let g = pick(&options, m[1]);
                        t.grown.push(g);
                        Mutation::Grew(g)
                    }
                }
            }
            _ => {
                let choices: Vec<Eyes> = EYES.iter().copied().filter(|e| *e != t.eyes).collect();
                t.eyes = pick(&choices, m[1]);
                Mutation::Eyes(t.eyes)
            }
        };
        history.push((Stage::from_index(k).unwrap_or(Stage::Seasoned), mutation));
    }
    (t, history)
}
