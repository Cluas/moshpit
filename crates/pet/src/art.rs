//! The pixel art, as data: `data/pixel.json`, the same file the design's
//! mockups render (moshpit-design `docs/nsh/ui-mockups/pet`). Palettes per
//! appearance, parts as rows of palette keys (`.` is transparent), frames
//! as layers of parts, and the genome's tables.
//!
//! Parsed once, on first use. The file is checked by this crate's tests
//! ([`check`]): a broken file fails the build's tests, never a phone.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

const PIXEL_JSON: &str = include_str!("../data/pixel.json");

/// Something per appearance.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Modes<T> {
    pub light: T,
    pub dark: T,
}

/// A part: rows of palette keys, an offset from where a layer puts it, and
/// the points an accessory hangs from (a body's head, face, brow, belly).
#[derive(Debug, Clone, Default)]
pub(crate) struct Part {
    pub rows: Vec<Vec<char>>,
    pub dx: i32,
    pub dy: i32,
    pub anchors: BTreeMap<String, (i32, i32)>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawPart {
    Rows(Vec<String>),
    Full {
        rows: Vec<String>,
        #[serde(default)]
        dx: i32,
        #[serde(default)]
        dy: i32,
        #[serde(default)]
        anchors: BTreeMap<String, [i32; 2]>,
    },
}

impl From<RawPart> for Part {
    fn from(raw: RawPart) -> Self {
        let rows = |rows: Vec<String>| rows.into_iter().map(|r| r.chars().collect()).collect();
        match raw {
            RawPart::Rows(r) => Part {
                rows: rows(r),
                ..Part::default()
            },
            RawPart::Full {
                rows: r,
                dx,
                dy,
                anchors,
            } => Part {
                rows: rows(r),
                dx,
                dy,
                anchors: anchors.into_iter().map(|(k, [x, y])| (k, (x, y))).collect(),
            },
        }
    }
}

/// Something grown, earned or worn, hung by its pivot from a body's anchor.
#[derive(Debug, Clone)]
pub(crate) struct Accessory {
    pub part: Part,
    pub pivot: (i32, i32),
    pub slot: String,
}

/// One layer of a frame: a part at `x, y`, mirrored when `flip`.
#[derive(Debug, Clone)]
pub(crate) struct Layer {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub flip: bool,
}

/// A frame: how long it shows and what it's made of.
#[derive(Debug, Clone)]
pub(crate) struct Frame {
    pub ms: u32,
    pub layers: Vec<Layer>,
    /// Whether stage marks, grown things and easter-egg items are hung on
    /// it (not while the egg is still whole).
    pub accessories: bool,
    /// Moves every part of the pet (a jump), never a prop.
    pub shift: (i32, i32),
}

#[derive(Debug, Deserialize)]
struct RawFrame {
    ms: u32,
    layers: Vec<Vec<serde_json::Value>>,
    #[serde(default = "yes")]
    accessories: bool,
    #[serde(default)]
    shift: [i32; 2],
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Names {
    pub zh: String,
    pub en: String,
}

/// A species as drawn.
#[derive(Debug, Clone)]
pub(crate) struct Character {
    pub name: Names,
    pub kind: Names,
    pub weight: u32,
    pub template: Option<String>,
    pub stretch: (i32, i32),
    pub stage_marks: BTreeMap<String, Vec<String>>,
    pub palette: Modes<BTreeMap<char, u32>>,
    pub parts: BTreeMap<String, Part>,
    pub states: Option<BTreeMap<String, Vec<Frame>>>,
}

#[derive(Debug, Deserialize)]
struct RawCharacter {
    name: Names,
    kind: Names,
    weight: u32,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    stretch: Option<[i32; 2]>,
    #[serde(default)]
    stage_marks: BTreeMap<String, Vec<String>>,
    palette: Modes<BTreeMap<String, String>>,
    parts: BTreeMap<String, RawPart>,
    #[serde(default)]
    states: Option<BTreeMap<String, Vec<RawFrame>>>,
}

/// A fur colour: the body, its shade and its light.
#[derive(Debug, Clone)]
pub(crate) struct Fur {
    pub id: String,
    pub zh: String,
    pub colors: Option<(u32, u32, u32)>,
}

/// The shiny body: two bands, a shade and a light.
#[derive(Debug, Clone)]
pub(crate) struct Shiny {
    pub id: String,
    pub zh: String,
    pub top: u32,
    pub bottom: u32,
    pub shade: u32,
    pub light: u32,
    pub band_y: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct GenomeTables {
    pub furs: Vec<Fur>,
    pub shiny: Shiny,
    /// What grows in each slot, by accessory name.
    pub grown: BTreeMap<String, Vec<String>>,
}

/// All of `pixel.json`.
#[derive(Debug, Clone)]
pub(crate) struct Art {
    pub grid: usize,
    pub props_palette: Modes<BTreeMap<char, u32>>,
    pub props: BTreeMap<String, Part>,
    pub accessories: BTreeMap<String, Accessory>,
    pub characters: BTreeMap<String, Character>,
    pub egg_states: BTreeMap<String, Vec<Frame>>,
    pub genome: GenomeTables,
}

#[derive(Debug, Deserialize)]
struct RawArt {
    grid: usize,
    props_palette: Modes<BTreeMap<String, String>>,
    props: BTreeMap<String, RawPart>,
    accessories: BTreeMap<String, RawAccessory>,
    characters: BTreeMap<String, RawCharacter>,
    egg_states: BTreeMap<String, Vec<RawFrame>>,
    genome: RawGenome,
}

#[derive(Debug, Deserialize)]
struct RawAccessory {
    rows: Vec<String>,
    pivot: [i32; 2],
    slot: String,
}

#[derive(Debug, Deserialize)]
struct RawGenome {
    palettes: Vec<RawFur>,
    shiny: RawShiny,
    grown: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RawFur {
    id: String,
    zh: String,
    b: Option<String>,
    s: Option<String>,
    h: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawShiny {
    id: String,
    zh: String,
    top: String,
    bottom: String,
    s: String,
    h: String,
    band_y: i32,
}

/// `#RRGGBB` as `0xRRGGBB`.
pub(crate) fn color(hex: &str) -> Result<u32, String> {
    let digits = hex
        .strip_prefix('#')
        .filter(|d| d.len() == 6)
        .ok_or_else(|| format!("{hex:?} is not #RRGGBB"))?;
    u32::from_str_radix(digits, 16).map_err(|_| format!("{hex:?} is not #RRGGBB"))
}

fn palette(raw: BTreeMap<String, String>) -> Result<BTreeMap<char, u32>, String> {
    raw.into_iter()
        .map(|(k, v)| {
            let mut chars = k.chars();
            match (chars.next(), chars.next()) {
                (Some(key), None) => Ok((key, color(&v)?)),
                _ => Err(format!("palette key {k:?} is not one character")),
            }
        })
        .collect()
}

fn modes(raw: Modes<BTreeMap<String, String>>) -> Result<Modes<BTreeMap<char, u32>>, String> {
    Ok(Modes {
        light: palette(raw.light)?,
        dark: palette(raw.dark)?,
    })
}

fn frame(raw: RawFrame) -> Result<Frame, String> {
    let layers = raw
        .layers
        .into_iter()
        .map(|l| {
            let name = l
                .first()
                .and_then(|v| v.as_str())
                .ok_or("a layer without a name")?;
            let int = |i: usize| -> Result<i32, String> {
                l.get(i)
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or_else(|| format!("layer {name}: no {}", if i == 1 { "x" } else { "y" }))
            };
            Ok(Layer {
                name: name.to_owned(),
                x: int(1)?,
                y: int(2)?,
                flip: l.get(3).and_then(|v| v.as_str()) == Some("flip"),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Frame {
        ms: raw.ms,
        layers,
        accessories: raw.accessories,
        shift: (raw.shift[0], raw.shift[1]),
    })
}

fn frames(raw: BTreeMap<String, Vec<RawFrame>>) -> Result<BTreeMap<String, Vec<Frame>>, String> {
    raw.into_iter()
        .map(|(k, v)| {
            Ok((
                k,
                v.into_iter()
                    .map(frame)
                    .collect::<Result<Vec<_>, String>>()?,
            ))
        })
        .collect()
}

impl Art {
    pub(crate) fn parse(json: &str) -> Result<Art, String> {
        let raw: RawArt = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let characters = raw
            .characters
            .into_iter()
            .map(|(id, c)| {
                Ok((
                    id,
                    Character {
                        name: c.name,
                        kind: c.kind,
                        weight: c.weight,
                        template: c.template,
                        stretch: c.stretch.map_or((12, 16), |[x, y]| (x, y)),
                        stage_marks: c.stage_marks,
                        palette: modes(c.palette)?,
                        parts: c.parts.into_iter().map(|(k, p)| (k, p.into())).collect(),
                        states: c.states.map(frames).transpose()?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let furs = raw
            .genome
            .palettes
            .into_iter()
            .map(|f| {
                let colors = match (f.b, f.s, f.h) {
                    (Some(b), Some(s), Some(h)) => Some((color(&b)?, color(&s)?, color(&h)?)),
                    (None, None, None) => None,
                    _ => return Err(format!("fur {}: b, s and h go together", f.id)),
                };
                Ok(Fur {
                    id: f.id,
                    zh: f.zh,
                    colors,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let s = raw.genome.shiny;
        Ok(Art {
            grid: raw.grid,
            props_palette: modes(raw.props_palette)?,
            props: raw.props.into_iter().map(|(k, p)| (k, p.into())).collect(),
            accessories: raw
                .accessories
                .into_iter()
                .map(|(k, a)| {
                    let part = Part {
                        rows: a.rows.into_iter().map(|r| r.chars().collect()).collect(),
                        ..Part::default()
                    };
                    (
                        k,
                        Accessory {
                            part,
                            pivot: (a.pivot[0], a.pivot[1]),
                            slot: a.slot,
                        },
                    )
                })
                .collect(),
            characters,
            egg_states: frames(raw.egg_states)?,
            genome: GenomeTables {
                furs,
                shiny: Shiny {
                    id: s.id,
                    zh: s.zh,
                    top: color(&s.top)?,
                    bottom: color(&s.bottom)?,
                    shade: color(&s.s)?,
                    light: color(&s.h)?,
                    band_y: s.band_y,
                },
                grown: raw.genome.grown,
            },
        })
    }

    /// A species' frames by state: its own, or its template's.
    pub(crate) fn states(&self, species: &str) -> Option<&BTreeMap<String, Vec<Frame>>> {
        let ch = self.characters.get(species)?;
        match &ch.states {
            Some(states) => Some(states),
            None => self
                .characters
                .get(ch.template.as_deref()?)?
                .states
                .as_ref(),
        }
    }
}

/// The art, parsed on first use.
pub(crate) fn art() -> &'static Art {
    static ART: OnceLock<Art> = OnceLock::new();
    // The file is part of the crate and checked by its tests (`check`).
    ART.get_or_init(|| {
        Art::parse(PIXEL_JSON).expect("data/pixel.json parses (checked by the tests)")
    })
}

#[cfg(test)]
pub(crate) fn parse_bundled() -> Result<Art, String> {
    Art::parse(PIXEL_JSON)
}
