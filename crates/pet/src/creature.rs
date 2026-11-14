//! A host's pixel creature (moshpit-design `docs/nsh/PET-DESIGN.md` §6.3):
//! the pixel style's host mark, made from the host key's fingerprint as the
//! key face is, through the same seam (UI-DESIGN §4.13): the same key, the
//! same creature; another key, another.
//!
//! `fp` is the fingerprint's 32 bytes (OpenSSH's `SHA256:` base64), and
//! `d = SHA-256("moshpit/creature/v1" ‖ 0 ‖ fp)`: the body `d[0] % 8`, the
//! eyes `d[1] % 8`, the mouth `d[2] % 4`, the top `d[3] % 4`, the pattern
//! `d[4] % 4`, with four rules that keep it readable. A 10 × 10 grid, mirrored
//! left to right, outlined where the body ends. Its colour is the key face's
//! hue: the mark colours the cells' kinds from that hue's tokens.
//!
//! The design's reference (`tools/build.py`) pins the grids
//! (`tests/creatures.json`).

use base64::Engine;
use sha2::{Digest, Sha256};

/// What a cell is; the mark colours it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Kind {
    Empty = 0,
    Body,
    Shade,
    Light,
    Horn,
    Outline,
    Eye,
    White,
    Mouth,
    Blush,
    /// A top's accents: an antenna's yellow, a sprout's green, a mohawk's pink.
    Yellow,
    Green,
    Pink,
}

/// A creature: its cells awake, and asleep (a host offline or failed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creature {
    pub side: u8,
    pub awake: Vec<Kind>,
    pub asleep: Vec<Kind>,
}

/// The side of its grid.
pub const SIDE: usize = 10;

const BODIES: [(&str, usize, usize, [&str; 10]); 8] = [
    (
        "blob",
        4,
        6,
        [
            ".....", ".....", "..bbb", ".bbbb", ".bbbb", ".bbbb", ".bbbb", ".bbbb", "..bbb",
            ".....",
        ],
    ),
    (
        "bean",
        4,
        6,
        [
            ".....", "...bb", "..bbb", "..bbb", ".bbbb", ".bbbb", ".bbbb", ".bbbb", "..bbb",
            ".....",
        ],
    ),
    (
        "bot",
        4,
        6,
        [
            ".....", ".....", ".bbbb", ".bbbb", ".bbbb", ".bbbb", ".bbbb", ".bbbb", ".bbbb",
            ".....",
        ],
    ),
    (
        "ghost",
        4,
        6,
        [
            ".....", ".....", "..bbb", ".bbbb", ".bbbb", ".bbbb", ".bbbb", ".bbbb", ".b.bb",
            ".....",
        ],
    ),
    (
        "cat",
        5,
        7,
        [
            ".....", ".b...", ".bb..", ".bbbb", ".bbbb", ".bbbb", ".bbbb", ".bbbb", "..bbb",
            ".....",
        ],
    ),
    (
        "horns",
        5,
        7,
        [
            ".....", ".H...", "..H..", "..bbb", ".bbbb", ".bbbb", ".bbbb", ".bbbb", "..bbb",
            ".....",
        ],
    ),
    (
        "mushroom",
        6,
        7,
        [
            ".....", "..bbb", ".bbbb", ".bbbb", ".bbbb", "..lll", "..lll", "..lll", "..lll",
            ".....",
        ],
    ),
    (
        "slime",
        5,
        7,
        [
            ".....", ".....", ".....", "...bb", "..bbb", ".bbbb", ".bbbb", ".bbbb", ".bbbb",
            ".....",
        ],
    ),
];

/// The traits, by name (for tests and a description).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Traits {
    pub body: &'static str,
    pub eyes: &'static str,
    pub mouth: &'static str,
    pub top: &'static str,
    pub pattern: &'static str,
}

const EYES: [&str; 8] = [
    "dots", "big", "sleepy", "cyclops", "wideset", "visor", "glossy", "blush",
];
const MOUTHS: [&str; 4] = ["flat", "open", "fang", "grin"];
const TOPS: [&str; 4] = ["none", "antenna", "sprout", "mohawk"];
const PATTERNS: [&str; 4] = ["plain", "belly", "spots", "band"];

/// The fingerprint's 32 bytes; a fingerprint that isn't OpenSSH's form
/// stands in by its own SHA-256.
fn bytes(fingerprint: &str) -> [u8; 32] {
    let b64 = fingerprint.strip_prefix("SHA256:").unwrap_or(fingerprint);
    let decoded = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(b64.trim_end_matches('='))
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok());
    decoded.unwrap_or_else(|| Sha256::digest(fingerprint.as_bytes()).into())
}

/// The traits of the key `fingerprint`.
pub fn traits(fingerprint: &str) -> Traits {
    let d: [u8; 32] = Sha256::new()
        .chain_update(b"moshpit/creature/v1\x00")
        .chain_update(bytes(fingerprint))
        .finalize()
        .into();
    let body = BODIES[usize::from(d[0] % 8)].0;
    let mut eyes = EYES[usize::from(d[1] % 8)];
    let mut top = TOPS[usize::from(d[3] % 4)];
    let mut pattern = PATTERNS[usize::from(d[4] % 4)];
    if body == "cat" || body == "horns" {
        top = "none"; // they have their own top
    }
    if body == "mushroom" {
        pattern = "spots"; // a cap has spots
        if eyes == "cyclops" {
            eyes = "big"; // one eye on a stem reads as a door
        }
    }
    if eyes == "visor" && pattern == "band" {
        pattern = "plain"; // two bars read as one
    }
    Traits {
        body,
        eyes,
        mouth: MOUTHS[usize::from(d[2] % 4)],
        top,
        pattern,
    }
}

/// The creature of the key `fingerprint`.
pub fn creature(fingerprint: &str) -> Creature {
    let t = traits(fingerprint);
    Creature {
        side: SIDE as u8,
        awake: grid(&t, false),
        asleep: grid(&t, true),
    }
}

fn kind(c: char) -> Kind {
    match c {
        'b' => Kind::Body,
        's' => Kind::Shade,
        'l' => Kind::Light,
        'H' => Kind::Horn,
        'o' => Kind::Outline,
        'e' => Kind::Eye,
        'w' => Kind::White,
        'm' => Kind::Mouth,
        'c' => Kind::Blush,
        'y' => Kind::Yellow,
        'g' => Kind::Green,
        'p' => Kind::Pink,
        _ => Kind::Empty,
    }
}

fn grid(t: &Traits, asleep: bool) -> Vec<Kind> {
    let (name, ey, my, half) = BODIES
        .iter()
        .find(|b| b.0 == t.body)
        .copied()
        .unwrap_or(BODIES[0]);
    let n = SIDE as i32;
    let mut g = vec!['.'; SIDE * SIDE];
    for (y, row) in half.iter().enumerate() {
        let full: Vec<char> = row.chars().chain(row.chars().rev()).collect();
        for (x, c) in full.into_iter().enumerate() {
            g[y * SIDE + x] = c;
        }
    }
    let body: Vec<bool> = g.iter().map(|c| *c != '.').collect();
    let at = |x: i32, y: i32| (0..n).contains(&x) && (0..n).contains(&y);
    let idx = |x: i32, y: i32| (y * n + x) as usize;
    let put = |g: &mut Vec<char>, x: i32, y: i32, c: char, need_body: bool| {
        if at(x, y) && (!need_body || body[idx(x, y)]) {
            g[idx(x, y)] = c;
        }
    };
    let (ey, my) = (ey as i32, my as i32);

    match t.pattern {
        "belly" => {
            for y in my..n {
                for x in [3, 4, 5, 6] {
                    if g[idx(x, y)] == 'b' {
                        put(&mut g, x, y, 'l', true);
                    }
                }
            }
        }
        "spots" => {
            let spots: &[(i32, i32)] = if name == "mushroom" {
                &[(2, 2), (7, 2), (4, 3), (5, 3), (1, 4), (8, 4)]
            } else {
                &[(2, 3), (7, 3), (1, 6), (8, 6), (3, 8), (6, 8)]
            };
            for &(x, y) in spots {
                if g[idx(x, y)] == 'b' {
                    put(
                        &mut g,
                        x,
                        y,
                        if name == "mushroom" { 'l' } else { 's' },
                        true,
                    );
                }
            }
        }
        "band" => {
            let y = if my + 1 < n - 1 { my + 1 } else { my };
            for x in 0..n {
                if g[idx(x, y)] == 'b' {
                    put(&mut g, x, y, 's', true);
                }
            }
        }
        _ => {}
    }

    match if asleep { "sleepy" } else { t.eyes } {
        "dots" => {
            put(&mut g, 3, ey, 'e', true);
            put(&mut g, 6, ey, 'e', true);
        }
        "big" => {
            for x0 in [2, 6] {
                put(&mut g, x0, ey - 1, 'w', true);
                put(&mut g, x0 + 1, ey - 1, 'e', true);
                put(&mut g, x0, ey, 'e', true);
                put(&mut g, x0 + 1, ey, 'e', true);
            }
        }
        "sleepy" => {
            for x in [2, 3, 6, 7] {
                put(&mut g, x, ey, 'e', true);
            }
        }
        "cyclops" => {
            for x in [3, 4, 5, 6] {
                put(&mut g, x, ey - 1, 'w', true);
            }
            put(&mut g, 3, ey, 'w', true);
            put(&mut g, 4, ey, 'e', true);
            put(&mut g, 5, ey, 'e', true);
            put(&mut g, 6, ey, 'w', true);
        }
        "wideset" => {
            for x in [2, 7] {
                put(&mut g, x, ey - 1, 'e', true);
                put(&mut g, x, ey, 'e', true);
            }
        }
        "visor" => {
            for x in 2..8 {
                put(&mut g, x, ey, 'e', true);
            }
            put(&mut g, 3, ey, 'w', true);
        }
        "glossy" => {
            for x in [3, 6] {
                put(&mut g, x, ey - 1, 'w', true);
                put(&mut g, x, ey, 'e', true);
            }
        }
        _ => {
            put(&mut g, 3, ey, 'e', true);
            put(&mut g, 6, ey, 'e', true);
            put(&mut g, 2, ey + 1, 'c', true);
            put(&mut g, 7, ey + 1, 'c', true);
        }
    }

    match t.mouth {
        "flat" => {
            put(&mut g, 4, my, 'e', true);
            put(&mut g, 5, my, 'e', true);
        }
        "open" => {
            put(&mut g, 4, my, 'm', true);
            put(&mut g, 5, my, 'm', true);
        }
        "fang" => {
            for x in [3, 4, 5, 6] {
                put(&mut g, x, my, 'e', true);
            }
            put(&mut g, 4, my + 1, 'w', true);
        }
        _ => {
            put(&mut g, 3, my, 'e', true);
            put(&mut g, 6, my, 'e', true);
            put(&mut g, 4, my + 1, 'e', true);
            put(&mut g, 5, my + 1, 'e', true);
        }
    }

    // The outline: every empty cell beside a filled one (4-neighbours).
    let filled: Vec<bool> = g.iter().map(|c| *c != '.').collect();
    let snapshot = g.clone();
    for y in 0..n {
        for x in 0..n {
            let beside = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|(dx, dy)| at(x + dx, y + dy) && filled[idx(x + dx, y + dy)]);
            if snapshot[idx(x, y)] == '.' && beside {
                g[idx(x, y)] = 'o';
            }
        }
    }

    let top_row = (0..n)
        .find(|y| (0..n).any(|x| filled[idx(x, *y)]))
        .unwrap_or(0);
    match t.top {
        "antenna" => {
            put(&mut g, 4, top_row - 1, 'o', false);
            put(&mut g, 5, top_row - 1, 'o', false);
            put(&mut g, 4, top_row - 2, 'y', false);
            put(&mut g, 5, top_row - 2, 'y', false);
        }
        "sprout" => {
            put(&mut g, 4, top_row - 1, 'g', false);
            put(&mut g, 5, top_row - 1, 'g', false);
            put(&mut g, 3, top_row - 2, 'g', false);
            put(&mut g, 6, top_row - 2, 'g', false);
        }
        "mohawk" => {
            for y in [top_row - 1, top_row - 2] {
                put(&mut g, 4, y, 'p', false);
                put(&mut g, 5, y, 'p', false);
            }
        }
        _ => {}
    }
    g.into_iter().map(kind).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str = include_str!("../tests/creatures.json");

    fn rows(k: &[Kind]) -> Vec<String> {
        let c = |k: &Kind| match k {
            Kind::Empty => '.',
            Kind::Body => 'b',
            Kind::Shade => 's',
            Kind::Light => 'l',
            Kind::Horn => 'H',
            Kind::Outline => 'o',
            Kind::Eye => 'e',
            Kind::White => 'w',
            Kind::Mouth => 'm',
            Kind::Blush => 'c',
            Kind::Yellow => 'y',
            Kind::Green => 'g',
            Kind::Pink => 'p',
        };
        k.chunks(SIDE).map(|r| r.iter().map(c).collect()).collect()
    }

    #[test]
    fn the_sample_keys_make_the_designs_creatures() {
        let golden: serde_json::Value = serde_json::from_str(GOLDEN).unwrap();
        for s in golden["samples"].as_array().unwrap() {
            let fp = s["fingerprint"].as_str().unwrap();
            let t = traits(fp);
            let want = &s["traits"];
            assert_eq!(
                (t.body, t.eyes, t.mouth, t.top, t.pattern),
                (
                    want["body"].as_str().unwrap(),
                    want["eyes"].as_str().unwrap(),
                    want["mouth"].as_str().unwrap(),
                    want["top"].as_str().unwrap(),
                    want["pattern"].as_str().unwrap()
                ),
                "{fp}"
            );
            let c = creature(fp);
            let strings = |v: &serde_json::Value| -> Vec<String> {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r.as_str().unwrap().to_owned())
                    .collect()
            };
            assert_eq!(rows(&c.awake), strings(&s["awake"]), "{fp} awake");
            assert_eq!(rows(&c.asleep), strings(&s["asleep"]), "{fp} asleep");
        }
    }

    #[test]
    fn any_fingerprint_makes_a_symmetric_outlined_creature() {
        for i in 0..500 {
            let fp = format!("SHA256:test{i}");
            let c = creature(&fp);
            assert_eq!(c, creature(&fp), "the same key, the same creature");
            for y in 0..SIDE {
                for x in 0..SIDE {
                    let (a, b) = (c.awake[y * SIDE + x], c.awake[y * SIDE + SIDE - 1 - x]);
                    assert_eq!(a == Kind::Empty, b == Kind::Empty, "{fp}: mirrored");
                }
            }
            assert!(c.awake.contains(&Kind::Outline) && c.awake.contains(&Kind::Eye));
        }
    }
}
