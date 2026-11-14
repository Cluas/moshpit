//! Drawing a pet (PET-DESIGN §16.2): a pose of a look, composed from the
//! art into a [`Sprite`] — a grid of palette indices with a light and a
//! dark colour per index. The host mark's face is the same shape; the
//! Live Activity gets one as a short string ([`Sprite::encode`]).
//!
//! The composition is the design's reference (moshpit-design
//! `tools/build.py`) rule for rule, checked against its output frame by
//! frame (`tests/golden.json`).

use std::collections::BTreeMap;
use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::art::{Art, Frame, Part, art};
use crate::easter::{FLAG_GOLD, FLAG_ROAMED};
use crate::genome::{Eyes, Genome, Pattern, Seed, Size, Species, Stage, Traits, evolve};

/// The light or the dark appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Appearance {
    Light,
    Dark,
}

/// A colour per appearance, `0xRRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ink {
    pub light: u32,
    pub dark: u32,
}

impl Ink {
    pub fn get(self, appearance: Appearance) -> u32 {
        match appearance {
            Appearance::Light => self.light,
            Appearance::Dark => self.dark,
        }
    }
}

/// A square grid of palette indices; 0 is empty, `n` is `palette[n - 1]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sprite {
    pub side: u8,
    pub cells: Vec<u8>,
    pub palette: Vec<Ink>,
}

/// Why a string is not a sprite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a sprite: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

/// The Live Activity's prefix for a pixel sprite.
pub const ENCODED_PREFIX: &str = "px1:";

impl Sprite {
    /// The colour at `x, y` in `appearance`; none where it's empty.
    pub fn color(&self, x: usize, y: usize, appearance: Appearance) -> Option<u32> {
        let side = usize::from(self.side);
        if x >= side || y >= side {
            return None;
        }
        match self.cells[y * side + x] {
            0 => None,
            i => self
                .palette
                .get(usize::from(i) - 1)
                .map(|ink| ink.get(appearance)),
        }
    }

    /// `px1:` and standard base64 of: a format byte, the side, the number
    /// of inks `n`, a bitmask (`ceil(n / 8)` bytes, ink 0 in the lowest bit)
    /// of the inks whose dark colour differs, every ink's light RGB, the
    /// differing inks' dark RGB, then the cells row by row:
    /// - format 1 (up to 31 inks): runs, a byte each, `index << 3 | (length - 1)`;
    /// - format 2: a byte a cell.
    ///
    /// About 0.3–0.4 KB for a 24 × 24 pose.
    pub fn encode(&self) -> String {
        let n = self.palette.len();
        let runs = n <= 31;
        let mut bytes = vec![if runs { 1 } else { 2 }, self.side, n as u8];
        let mut mask = vec![0u8; n.div_ceil(8)];
        for (i, ink) in self.palette.iter().enumerate() {
            if ink.dark != ink.light {
                mask[i / 8] |= 1 << (i % 8);
            }
        }
        bytes.extend_from_slice(&mask);
        for ink in &self.palette {
            bytes.extend_from_slice(&ink.light.to_be_bytes()[1..]);
        }
        for ink in self.palette.iter().filter(|i| i.dark != i.light) {
            bytes.extend_from_slice(&ink.dark.to_be_bytes()[1..]);
        }
        if runs {
            let mut i = 0;
            while i < self.cells.len() {
                let c = self.cells[i];
                let mut len = 1;
                while len < 8 && i + len < self.cells.len() && self.cells[i + len] == c {
                    len += 1;
                }
                bytes.push(c << 3 | (len as u8 - 1));
                i += len;
            }
        } else {
            bytes.extend_from_slice(&self.cells);
        }
        format!("{ENCODED_PREFIX}{}", STANDARD.encode(bytes))
    }

    pub fn decode(s: &str) -> Result<Sprite, DecodeError> {
        let body = s
            .strip_prefix(ENCODED_PREFIX)
            .ok_or_else(|| DecodeError("no px1: prefix".into()))?;
        let bytes = STANDARD
            .decode(body)
            .map_err(|e| DecodeError(e.to_string()))?;
        let short = || DecodeError("too short".into());
        let (&format, rest) = bytes.split_first().ok_or_else(short)?;
        let (&side, rest) = rest.split_first().ok_or_else(short)?;
        let (&n, rest) = rest.split_first().ok_or_else(short)?;
        let n = usize::from(n);
        let (mask, rest) = rest.split_at_checked(n.div_ceil(8)).ok_or_else(short)?;
        let differs = |i: usize| mask[i / 8] >> (i % 8) & 1 == 1;
        let darks = (0..n).filter(|&i| differs(i)).count();
        let (lights, rest) = rest.split_at_checked(n * 3).ok_or_else(short)?;
        let (dark_bytes, cells) = rest.split_at_checked(darks * 3).ok_or_else(short)?;
        let rgb = |b: &[u8]| u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        let mut dark_iter = dark_bytes.chunks(3);
        let palette = lights
            .chunks(3)
            .enumerate()
            .map(|(i, l)| {
                let light = rgb(l);
                let dark = if differs(i) {
                    dark_iter.next().map_or(light, rgb)
                } else {
                    light
                };
                Ink { light, dark }
            })
            .collect();
        let count = usize::from(side) * usize::from(side);
        let cells: Vec<u8> = match format {
            1 => cells
                .iter()
                .flat_map(|b| std::iter::repeat_n(b >> 3, usize::from(b & 7) + 1))
                .collect(),
            2 => cells.to_vec(),
            f => return Err(DecodeError(format!("format {f}"))),
        };
        if cells.len() != count || cells.iter().any(|&c| usize::from(c) > n) {
            return Err(DecodeError("cells don't fit".into()));
        }
        Ok(Sprite {
            side,
            cells,
            palette,
        })
    }
}

/// What decides every pixel of a pet: its seed, stage, hatch hour and
/// flags, and whether its name wears the crown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Look {
    pub seed: Seed,
    pub stage: Stage,
    pub hatch_hour: u8,
    pub flags: u8,
    pub crowned: bool,
}

/// What a pet is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pose {
    Idle,
    Working,
    Waiting,
    Done,
    Error,
    Sleeping,
    Petted,
    /// The hatch, as a hatchling.
    Hatch,
    /// The seven-tap easter egg.
    Pogo,
}

impl Pose {
    /// The seven every species has in every stage.
    pub const STATES: [Pose; 7] = [
        Pose::Idle,
        Pose::Working,
        Pose::Waiting,
        Pose::Done,
        Pose::Error,
        Pose::Sleeping,
        Pose::Petted,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Pose::Idle => "idle",
            Pose::Working => "working",
            Pose::Waiting => "waiting",
            Pose::Done => "done",
            Pose::Error => "error",
            Pose::Sleeping => "sleeping",
            Pose::Petted => "petted",
            Pose::Hatch => "hatch",
            Pose::Pogo => "pogo",
        }
    }
}

/// The egg's poses before it hatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EggPose {
    /// Whole, with an occasional wobble.
    Still,
    /// Cracked: ready to hatch.
    Ready,
}

/// Easter-egg items worn on top of the traits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) struct Extras {
    pub crown: bool,
    pub badge: bool,
    pub gold: bool,
}

impl Look {
    pub fn genome(&self) -> Genome {
        Genome::roll(&self.seed, self.hatch_hour)
    }

    fn extras(&self) -> Extras {
        Extras {
            crown: self.crowned,
            badge: self.flags & FLAG_ROAMED != 0,
            gold: self.flags & FLAG_GOLD != 0,
        }
    }

    /// The frames of a pose: what to compose and for how long. The hatch
    /// and the pogo come from the species' own frames where it has them;
    /// a species drawn without them (Coo, Dig) hatches from the shared egg
    /// frames into its hatchling's idle, and pogos as it's petted.
    fn frames(&self, pose: Pose) -> Vec<(Species, &'static Frame, Stage)> {
        let species = self.genome().species;
        let art = art();
        let own = art.states(species.id());
        let get = |p: Pose| own.and_then(|s| s.get(p.id()));
        match pose {
            Pose::Hatch => match get(Pose::Hatch) {
                Some(f) => f.iter().map(|fr| (species, fr, Stage::Hatchling)).collect(),
                None => {
                    let tun = art
                        .states(Species::Tun.id())
                        .and_then(|s| s.get(Pose::Hatch.id()));
                    let egg = tun
                        .map(|f| {
                            f.iter()
                                .take_while(|fr| !fr.accessories)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let mut out: Vec<_> = egg
                        .into_iter()
                        .filter(|fr| !fr.layers.iter().any(|l| l.name.starts_with("body")))
                        .map(|fr| (Species::Tun, fr, Stage::Hatchling))
                        .collect();
                    if let Some(idle) = get(Pose::Idle).and_then(|f| f.first()) {
                        out.push((species, idle, Stage::Hatchling));
                    }
                    out
                }
            },
            Pose::Pogo => get(Pose::Pogo)
                .or_else(|| get(Pose::Petted))
                .map(|f| f.iter().map(|fr| (species, fr, self.stage)).collect())
                .unwrap_or_default(),
            p => get(p)
                .map(|f| f.iter().map(|fr| (species, fr, self.stage)).collect())
                .unwrap_or_default(),
        }
    }

    /// How many frames a pose has.
    pub fn frame_count(&self, pose: Pose) -> usize {
        self.frames(pose).len()
    }

    /// Each frame's duration, in milliseconds.
    pub fn durations(&self, pose: Pose) -> Vec<u32> {
        self.frames(pose)
            .into_iter()
            .map(|(_, f, _)| f.ms)
            .collect()
    }

    /// One frame of a pose; none past the last.
    pub fn compose(&self, pose: Pose, frame: usize) -> Option<Sprite> {
        let (species, fr, stage) = *self.frames(pose).get(frame)?;
        let traits = if species == self.genome().species {
            evolve(&self.seed, self.genome(), stage).0
        } else {
            Traits::from(Genome {
                species,
                ..self.genome()
            })
        };
        Some(compose_frame(
            art(),
            species.id(),
            fr,
            Some(&traits),
            stage,
            self.extras(),
        ))
    }

    /// The Live Activity's still for a pose: its first frame, encoded.
    pub fn island(&self, pose: Pose) -> Option<String> {
        self.compose(pose, 0).map(|s| s.encode())
    }
}

/// One frame of the egg (no pet in it yet).
pub fn egg(pose: EggPose, frame: usize) -> Option<Sprite> {
    let name = match pose {
        EggPose::Still => "egg",
        EggPose::Ready => "egg_ready",
    };
    let fr = art().egg_states.get(name)?.get(frame)?;
    Some(compose_frame(
        art(),
        Species::Tun.id(),
        fr,
        None,
        Stage::Grown,
        Extras::default(),
    ))
}

/// The egg's frame durations.
pub fn egg_durations(pose: EggPose) -> Vec<u32> {
    let name = match pose {
        EggPose::Still => "egg",
        EggPose::Ready => "egg_ready",
    };
    art()
        .egg_states
        .get(name)
        .map(|f| f.iter().map(|fr| fr.ms).collect())
        .unwrap_or_default()
}

// ---- composition ----------------------------------------------------

/// What fills a cell, before it's a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Fill {
    Key(char),
    BandTop,
    BandBottom,
    Gold,
}

fn xmap(x: i32, w: i32, c: i32) -> Vec<i32> {
    match w {
        0 => vec![x],
        w if w > 0 => {
            if x < c {
                if x == c - 1 {
                    vec![x - 1, x]
                } else {
                    vec![x - 1]
                }
            } else if x == c {
                vec![x + 1, x]
            } else {
                vec![x + 1]
            }
        }
        _ => {
            if x == c - 1 || x == c {
                vec![]
            } else if x < c - 1 {
                vec![x + 1]
            } else {
                vec![x - 1]
            }
        }
    }
}

fn ymap(y: i32, h: i32, c: i32) -> Vec<i32> {
    match h {
        0 => vec![y],
        h if h > 0 => {
            if y < c {
                if y == c - 1 {
                    vec![y - 1, y]
                } else {
                    vec![y - 1]
                }
            } else {
                vec![y]
            }
        }
        _ => {
            if y == c - 1 {
                vec![]
            } else if y < c - 1 {
                vec![y + 1]
            } else {
                vec![y]
            }
        }
    }
}

const SPOTS: [(i32, i32); 6] = [(7, 8), (15, 10), (10, 17), (16, 17), (6, 13), (13, 7)];

fn pattern_key(pattern: Pattern, x: i32, y: i32) -> Option<char> {
    match pattern {
        Pattern::Spots if SPOTS.contains(&(x, y)) => Some('s'),
        Pattern::Stripes if y == 9 || y == 13 => Some('s'),
        Pattern::Patch if (9..=14).contains(&x) && (14..=20).contains(&y) => Some('h'),
        _ => None,
    }
}

/// A part by name for a species at a stage: its stage part, its own, its
/// template's (not the template's stage parts), then an accessory, then a
/// prop. `true` when it's part of the pet (and so moves with it).
fn lookup<'a>(art: &'a Art, species: &str, name: &str, stage: Stage) -> Option<(&'a Part, bool)> {
    let ch = art.characters.get(species)?;
    let staged = format!("{name}@{}", stage.index());
    let mut chain = vec![ch];
    if let Some(t) = ch.template.as_deref().and_then(|t| art.characters.get(t)) {
        chain.push(t);
    }
    for (i, c) in chain.iter().enumerate() {
        for nm in [staged.as_str(), name] {
            if i > 0 && nm.contains('@') {
                continue;
            }
            if let Some(p) = c.parts.get(nm) {
                return Some((p, true));
            }
        }
    }
    if name == "leg" {
        return lookup(art, species, "limb", stage);
    }
    if let Some(a) = art.accessories.get(name) {
        return Some((&a.part, true));
    }
    art.props.get(name).map(|p| (p, false))
}

struct Canvas<'a> {
    n: i32,
    cells: Vec<Option<Fill>>,
    traits: Option<&'a Traits>,
    shiny_band: Option<i32>,
    gold: bool,
    w: i32,
    h: i32,
    stretch: (i32, i32),
    shift: (i32, i32),
}

impl Canvas<'_> {
    fn put(
        &mut self,
        rows: &[Vec<char>],
        x0: i32,
        y0: i32,
        flip: bool,
        character: bool,
        part: &str,
    ) {
        let is_body = part.starts_with("body");
        let is_eyes = part.starts_with("eyes_open") || part.starts_with("eye_open");
        for (dy, row) in rows.iter().enumerate() {
            let len = row.len();
            for dx in 0..len {
                let mut key = if flip { row[len - 1 - dx] } else { row[dx] };
                if key == '.' {
                    continue;
                }
                let (x, y) = (x0 + dx as i32, y0 + dy as i32);
                if character && let Some(t) = self.traits {
                    if is_body && key == 'b' {
                        key = pattern_key(t.pattern, x, y).unwrap_or(key);
                    }
                    if is_eyes {
                        key = match t.eyes {
                            Eyes::Lidded
                                if rows.len() > 1 && dy == 0 && (key == 'e' || key == 'w') =>
                            {
                                's'
                            }
                            Eyes::Starry if key == 'w' => 'x',
                            Eyes::Button if key == 'w' => 'e',
                            _ => key,
                        };
                    }
                }
                let fill = match (key, self.shiny_band) {
                    ('b', Some(band)) if character => {
                        if y < band {
                            Fill::BandTop
                        } else {
                            Fill::BandBottom
                        }
                    }
                    ('o', _) if self.gold && character => Fill::Gold,
                    _ => Fill::Key(key),
                };
                if !character {
                    self.set(x, y, fill);
                    continue;
                }
                for tx in xmap(x, self.w, self.stretch.0) {
                    for ty in ymap(y, self.h, self.stretch.1) {
                        self.set(tx + self.shift.0, ty + self.shift.1, fill);
                    }
                }
            }
        }
    }

    fn set(&mut self, x: i32, y: i32, fill: Fill) {
        if (0..self.n).contains(&x) && (0..self.n).contains(&y) {
            self.cells[(y * self.n + x) as usize] = Some(fill);
        }
    }
}

/// The colours of a species' keys in an appearance, with the traits' fur.
fn palette(
    art: &Art,
    species: &str,
    traits: Option<&Traits>,
    appearance: Appearance,
) -> BTreeMap<char, u32> {
    let pick = |m: &crate::art::Modes<BTreeMap<char, u32>>| match appearance {
        Appearance::Light => m.light.clone(),
        Appearance::Dark => m.dark.clone(),
    };
    let mut pal = pick(&art.props_palette);
    if let Some(ch) = art.characters.get(species) {
        pal.extend(pick(&ch.palette));
    }
    if let Some(t) = traits {
        if t.shiny {
            pal.insert('s', art.genome.shiny.shade);
            pal.insert('h', art.genome.shiny.light);
        } else if let Some((b, s, h)) = art
            .genome
            .furs
            .get(usize::from(t.fur))
            .and_then(|f| f.colors)
        {
            pal.insert('b', b);
            pal.insert('s', s);
            pal.insert('h', h);
        }
    }
    pal
}

pub(crate) fn compose_frame(
    art: &Art,
    species: &str,
    frame: &Frame,
    traits: Option<&Traits>,
    stage: Stage,
    extras: Extras,
) -> Sprite {
    let ch = &art.characters[species];
    let n = art.grid as i32;
    let mut canvas = Canvas {
        n,
        cells: vec![None; (n * n) as usize],
        traits,
        shiny_band: traits.filter(|t| t.shiny).map(|_| art.genome.shiny.band_y),
        gold: extras.gold,
        w: match stage {
            Stage::Hatchling => -2,
            Stage::Grown => 0,
            Stage::Seasoned => 2,
        },
        h: traits.map_or(0, |t| match t.size {
            Size::Short => -1,
            Size::Standard => 0,
            Size::Tall => 1,
        }),
        stretch: ch.stretch,
        shift: frame.shift,
    };

    let mut accessories: Vec<&str> = Vec::new();
    if frame.accessories {
        if let Some(marks) = ch.stage_marks.get(&stage.index().to_string()) {
            accessories.extend(marks.iter().map(String::as_str));
        }
        if let Some(t) = traits {
            accessories.extend(t.grown.iter().map(|g| g.id()));
        }
        if extras.crown {
            accessories.push("crown");
        }
        if extras.badge {
            accessories.push("badge");
        }
        if accessories.contains(&"crown") {
            accessories.retain(|a| {
                *a == "crown" || art.accessories.get(*a).is_none_or(|acc| acc.slot != "head")
            });
        }
    }

    let layers = &frame.layers;
    let insert_at = layers
        .iter()
        .position(|l| l.name.starts_with("body"))
        .map(|i| {
            let j = i + 1;
            if layers.get(j).is_some_and(|l| l.name.starts_with("deco")) {
                j + 1
            } else {
                j
            }
        });
    let mut anchors: BTreeMap<String, (i32, i32)> = BTreeMap::new();
    for i in 0..=layers.len() {
        if Some(i) == insert_at {
            for a in &accessories {
                let Some(acc) = art.accessories.get(*a) else {
                    continue;
                };
                let Some(&(ax, ay)) = anchors.get(&acc.slot) else {
                    continue;
                };
                canvas.put(
                    &acc.part.rows,
                    ax - acc.pivot.0,
                    ay - acc.pivot.1,
                    false,
                    true,
                    a,
                );
            }
        }
        let Some(layer) = layers.get(i) else { break };
        let Some((part, character)) = lookup(art, species, &layer.name, stage) else {
            continue;
        };
        if part.rows.is_empty() {
            continue;
        }
        let (px, py) = (layer.x + part.dx, layer.y + part.dy);
        if !part.anchors.is_empty() {
            anchors = part
                .anchors
                .iter()
                .map(|(k, (ax, ay))| (k.clone(), (px + ax, py + ay)))
                .collect();
        }
        canvas.put(&part.rows, px, py, layer.flip, character, &layer.name);
    }

    // Fills to inks, in the order they first appear.
    let light = palette(art, species, traits, Appearance::Light);
    let dark = palette(art, species, traits, Appearance::Dark);
    let shiny = &art.genome.shiny;
    let ink = |f: Fill| -> Ink {
        match f {
            Fill::Key(k) => Ink {
                light: light.get(&k).copied().unwrap_or(0),
                dark: dark.get(&k).copied().unwrap_or(0),
            },
            Fill::BandTop => Ink {
                light: shiny.top,
                dark: shiny.top,
            },
            Fill::BandBottom => Ink {
                light: shiny.bottom,
                dark: shiny.bottom,
            },
            Fill::Gold => Ink {
                light: light.get(&'T').copied().unwrap_or(0),
                dark: dark.get(&'T').copied().unwrap_or(0),
            },
        }
    };
    let mut inks: Vec<Ink> = Vec::new();
    let cells = canvas
        .cells
        .iter()
        .map(|c| match c {
            None => 0,
            Some(f) => {
                let i = ink(*f);
                let at = inks.iter().position(|x| *x == i).unwrap_or_else(|| {
                    inks.push(i);
                    inks.len() - 1
                });
                (at + 1) as u8
            }
        })
        .collect();
    Sprite {
        side: n as u8,
        cells,
        palette: inks,
    }
}

#[cfg(test)]
pub(crate) fn lookup_exists(art: &Art, species: &str, name: &str, stage: Stage) -> bool {
    lookup(art, species, name, stage).is_some()
}
