//! What the screens are told about the pet (`{"op": "pet", …}`), as the
//! app's core builds it and the NSH pages read it: the egg or the pet,
//! enough of its look to draw it here (the page composes its sprites with
//! this crate), and what the pet card lists. Words are the page's: traits
//! and mutations cross as ids.

use serde::{Deserialize, Serialize};

use crate::genome::{Eyes, Mutation, Pattern, Seed, Size, Stage};
use crate::record::{PetRecord, Thresholds};
use crate::sprite::Look;

/// The look the person picked (Settings › Appearance › Style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    /// Today's look: no companion.
    Minimal,
    #[default]
    Pixel,
    /// Waits for the illustrator; not offered yet.
    Drawn,
}

impl Style {
    pub fn from_id(id: &str) -> Option<Style> {
        match id {
            "minimal" => Some(Style::Minimal),
            "pixel" => Some(Style::Pixel),
            "drawn" => Some(Style::Drawn),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Style::Minimal => "minimal",
            Style::Pixel => "pixel",
            Style::Drawn => "drawn",
        }
    }
}

/// A moment the page plays once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Moment {
    /// It just hatched: the hatch, then the pet.
    Hatched,
    /// It just grew a stage.
    Grew,
}

/// The pet as the screens see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PetView {
    pub style: Style,
    /// The companion is on (Settings › Appearance › Companion).
    pub shown: bool,
    /// The egg, until it hatches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egg: Option<EggView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pet: Option<PetInfo>,
    /// The egg may hatch once the page is on Home (iCloud has settled).
    #[serde(default)]
    pub may_hatch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moment: Option<Moment>,
    /// It talks (「让它说话」, PET-DESIGN §17): on unless turned off.
    #[serde(default = "talks")]
    pub talk: bool,
    /// The app icon Home offers now (B668), after the hatch or a new look.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<IconOffer>,
}

/// The app icon the pet offers (B668, the owner's call): once it hatches,
/// and again each time it grows into a new look, Home asks
/// 「把 App 图标换成墩墩吗？」 (换 / 先不换). Nothing switches without a yes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IconOffer {
    /// The icon's id in the gallery ([`icon_id`]).
    pub id: String,
    /// The species' id (`tun`), whose name the card says.
    pub species: String,
    /// The person picked their icon by hand in the gallery: 「先不换」 is
    /// the default.
    pub keep_default: bool,
}

/// The gallery's id of a species' icon (`tun`, `tun-shiny`): fur a stage
/// changed keeps the natural icon; only a shiny has its own.
pub fn icon_id(species: &str, shiny: bool) -> String {
    if shiny {
        format!("{species}-shiny")
    } else {
        species.to_owned()
    }
}

/// Which look an offer is about: the species, shiny or not, and the stage.
/// A look asked about once is not asked about again.
pub fn icon_look(species: &str, shiny: bool, stage: u8) -> String {
    format!("{}.{stage}", icon_id(species, shiny))
}

fn talks() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EggView {
    /// Cracked: tapped twice, or ready by itself (the first finished task,
    /// or the third connection).
    pub ready: bool,
    /// The taps so far (the third hatches it).
    #[serde(default)]
    pub taps: u8,
}

/// Everything the page draws and the pet card lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PetInfo {
    /// 32 hex digits.
    pub seed: String,
    pub stage: u8,
    pub hatch_hour: u8,
    pub flags: u8,
    pub crowned: bool,
    /// The name it was given; empty: its species' name.
    pub name: String,
    /// The species' id (`tun`).
    pub species: String,
    /// Unix seconds.
    pub hatched_at: i64,
    pub tasks: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<NextView>,
    pub traits: TraitsView,
    /// Each stage's change, oldest first.
    pub history: Vec<ChangeView>,
    /// The easter eggs found, as ids (`night`, `roam`, `gold`, `sudo`).
    pub found: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextView {
    pub stage: u8,
    pub tasks_left: u64,
    pub not_before: i64,
}

/// Trait ids: fur (`natural`, `mint`, … `aurora`), pattern, eyes, size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraitsView {
    pub fur: String,
    pub shiny: bool,
    pub pattern: String,
    pub eyes: String,
    pub size: String,
}

/// A stage's change: `kind` (`fur`, `pattern`, `grew`, `eyes`) and what to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeView {
    pub stage: u8,
    pub kind: String,
    pub what: String,
    /// When the stage was reached, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
}

fn pattern_id(p: Pattern) -> &'static str {
    match p {
        Pattern::Plain => "plain",
        Pattern::Spots => "spots",
        Pattern::Stripes => "stripes",
        Pattern::Patch => "patch",
    }
}

fn eyes_id(e: Eyes) -> &'static str {
    match e {
        Eyes::Round => "round",
        Eyes::Lidded => "lidded",
        Eyes::Starry => "starry",
        Eyes::Button => "button",
    }
}

fn size_id(s: Size) -> &'static str {
    match s {
        Size::Short => "short",
        Size::Standard => "standard",
        Size::Tall => "tall",
    }
}

impl PetInfo {
    pub fn of(record: &PetRecord, thresholds: &Thresholds) -> PetInfo {
        let (traits, history) = record.traits();
        let look = record.look();
        PetInfo {
            seed: record.seed().iter().map(|b| format!("{b:02x}")).collect(),
            stage: record.stage() as u8,
            hatch_hour: record.hatch_hour(),
            flags: record.flags(),
            crowned: look.crowned,
            name: record.name().to_owned(),
            species: traits.species.id().to_owned(),
            hatched_at: record.hatched_at(),
            tasks: record.tasks(),
            next: record.next_stage(thresholds).map(|n| NextView {
                stage: n.stage as u8,
                tasks_left: n.tasks_left,
                not_before: n.not_before,
            }),
            traits: TraitsView {
                fur: traits.fur_name().0.to_owned(),
                shiny: traits.shiny,
                pattern: pattern_id(traits.pattern).to_owned(),
                eyes: eyes_id(traits.eyes).to_owned(),
                size: size_id(traits.size).to_owned(),
            },
            history: history
                .into_iter()
                .map(|(stage, m)| {
                    let (kind, what) = match m {
                        Mutation::Fur(f) => (
                            "fur",
                            crate::art::art().genome.furs[usize::from(f)].id.clone(),
                        ),
                        Mutation::Pattern(p) => ("pattern", pattern_id(p).to_owned()),
                        Mutation::Grew(g) => ("grew", g.id().to_owned()),
                        Mutation::Eyes(e) => ("eyes", eyes_id(e).to_owned()),
                    };
                    ChangeView {
                        stage: stage as u8,
                        kind: kind.to_owned(),
                        what,
                        at: record.stage_at(stage),
                    }
                })
                .collect(),
            found: record
                .found()
                .into_iter()
                .filter_map(|e| match e {
                    crate::Egg::NightHatch => Some("night"),
                    crate::Egg::MadeItThrough => Some("roam"),
                    crate::Egg::Task1000 => Some("gold"),
                    crate::Egg::Sudo => Some("sudo"),
                    crate::Egg::Pogo => None,
                })
                .map(str::to_owned)
                .collect(),
        }
    }

    /// The look to draw it with; none if the seed or stage doesn't parse.
    pub fn look(&self) -> Option<Look> {
        let bytes: Option<Vec<u8>> = (0..self.seed.len())
            .step_by(2)
            .map(|i| {
                self.seed
                    .get(i..i + 2)
                    .and_then(|b| u8::from_str_radix(b, 16).ok())
            })
            .collect();
        Some(Look {
            seed: Seed::try_from(bytes?).ok()?,
            stage: Stage::from_index(self.stage)?,
            hatch_hour: self.hatch_hour,
            flags: self.flags,
            crowned: self.crowned,
        })
    }
}
