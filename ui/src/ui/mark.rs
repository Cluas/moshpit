//! Who a row is about (`docs/nsh/UI-DESIGN.md` §4.13): a host's mark and
//! an agent's (a glyph, no tile).
//!
//! A host's mark is a disc with a status dot on its corner. On the disc, by
//! what the host has: the emoji the person picked for it; else its trusted
//! host key's face ([`Face`]: a pattern made from the key's fingerprint, in
//! one of eight hues, that changes when the key does; with the pixel
//! companion on, a pixel creature made the same way, moshpit-design
//! `PET-DESIGN.md` §6.3); else, before any key is trusted, its name's
//! initial.
//!
//! A host mark is decoration beside the host's name: screen readers hear
//! the name and its state from the row or head it sits in, not from the
//! mark. An agent's mark is the state's glyph, filled and colored only for
//! a state (§3.6: the waiting hand is solid amber), muted otherwise.

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ui::{
    icon::{IconSize, Tone, icon},
    status::{DotSize, Status, status_dot},
    tokens::{FACES, creature_ink, token},
    typography::TextToken,
};

/// A trusted host key's face: a square grid of cells, some filled, in one
/// of the hues of [`FACES`]. What fills the grid, and how big it is, is the
/// generator's (the host's model makes a face from the key's fingerprint:
/// the same key, the same face); the mark draws any grid in its box,
/// whatever its side.
///
/// A pixel creature (PET-DESIGN §6.3) is a face too, from another
/// generator: its cells each a kind ([`Face::kinds`]) the mark colours from
/// its hue, its eyes closed when the mark mutes it ([`Face::asleep`]), and
/// a bigger box ([`Face::space`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Face {
    /// The grid's side, in cells.
    pub side: u8,
    /// Its cells, row by row, `side * side` of them: filled or not.
    pub cells: Vec<bool>,
    /// Its hue: an index into [`FACES`].
    pub hue: u8,
    /// A creature's cells, row by row, each a kind (`moshpit_pet`'s
    /// `creature::Kind` by number, coloured by [`creature_ink`]); none on
    /// a key face, whose filled cells take its hue's one colour.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<u8>,
    /// The creature asleep, the same way: what the mark shows when it
    /// mutes it (the host offline or failed).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asleep: Vec<u8>,
}

impl Face {
    /// Whether the cell at `row`, `column` is filled (none past the grid).
    pub fn filled(&self, row: usize, column: usize) -> bool {
        let side = usize::from(self.side);
        column < side
            && self
                .cells
                .get(row * side + column)
                .copied()
                .unwrap_or(false)
    }

    /// Its pattern's colour token and its disc's.
    pub fn tokens(&self) -> (&'static str, &'static str) {
        FACES[usize::from(self.hue) % FACES.len()]
    }

    /// Whether it is a creature (its cells have kinds).
    pub fn is_creature(&self) -> bool {
        !self.kinds.is_empty()
    }

    /// The box its grid fills on a mark of `size`: a key face's 25 / 20 pt,
    /// a creature's 30 / 20 pt (whole points per cell: a 10-cell grid
    /// can't fill 25).
    pub fn space(&self, size: MarkSize) -> f32 {
        if self.is_creature() {
            size.creature()
        } else {
            size.pattern()
        }
    }
}

/// A host mark's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkSize {
    /// 40 pt: a connection card's head.
    Md,
    /// 28 pt: in a row's leading lane (SAVED), which is 28 pt wide.
    Sm,
}

impl MarkSize {
    pub fn points(self) -> f32 {
        match self {
            MarkSize::Md => 40.0,
            MarkSize::Sm => 28.0,
        }
    }

    /// The initial's size: `mono-subhead` on the head's mark, a step down
    /// (`mono-footnote`) in a row's lane.
    fn text(self) -> TextToken {
        match self {
            MarkSize::Md => TextToken::MonoSubhead,
            MarkSize::Sm => TextToken::MonoFootnote,
        }
    }

    /// The space a face's grid takes: 25 pt on the head's mark, 20 pt in a
    /// row's lane, its corners within the circle. A 5-cell grid's cells are
    /// 5 and 4 pt, whole pixels at 3x.
    pub fn pattern(self) -> f32 {
        match self {
            MarkSize::Md => 25.0,
            MarkSize::Sm => 20.0,
        }
    }

    /// The space a creature's grid takes: 30 pt on the head's mark (3 pt a
    /// cell), 20 pt in a row's lane (2 pt).
    pub fn creature(self) -> f32 {
        match self {
            MarkSize::Md => 30.0,
            MarkSize::Sm => 20.0,
        }
    }

    /// An emoji's size: most of the disc, fixed (it is a picture, not
    /// words: it doesn't grow with the text size past the disc).
    fn emoji(self) -> f32 {
        match self {
            MarkSize::Md => 22.0,
            MarkSize::Sm => 16.0,
        }
    }
}

/// How dim an emoji goes on the mark of a host offline or failed.
const EMOJI_DOWN: f32 = 0.45;

/// The ring around a mark's status dot, in the card's color, so the dot
/// reads apart from the disc under it.
const RING: f32 = 2.0;

/// A host's mark ([`host_mark`]).
pub struct HostMark {
    initial: SharedString,
    face: Option<Face>,
    emoji: Option<SharedString>,
    size: MarkSize,
    status: Option<Status>,
    test_id: Option<SharedString>,
}

/// The mark of a host whose name starts with `initial`.
pub fn host_mark(initial: impl Into<SharedString>) -> HostMark {
    HostMark {
        initial: initial.into(),
        face: None,
        emoji: None,
        size: MarkSize::Md,
        status: None,
        test_id: None,
    }
}

impl HostMark {
    pub fn size(mut self, size: MarkSize) -> Self {
        self.size = size;
        self
    }

    /// Its trusted host key's face, which shows in place of the initial.
    pub fn face(mut self, face: Option<Face>) -> Self {
        self.face = face;
        self
    }

    /// The emoji picked for it, which shows in place of the face.
    pub fn emoji(mut self, emoji: Option<impl Into<SharedString>>) -> Self {
        self.emoji = emoji.map(Into::into);
        self
    }

    /// The connection's state, as a dot on the mark's corner: live, a
    /// spinner while it connects, a hollow ring offline, red when it
    /// failed. Offline or failed, the disc goes grey too: the initial and
    /// the face's pattern grey, an emoji dimmed.
    pub fn status(mut self, status: Status) -> Self {
        self.status = Some(status);
        self
    }

    pub fn test_id(mut self, id: &str) -> Self {
        self.test_id = Some(SharedString::from(id.to_owned()));
        self
    }
}

impl IntoElement for HostMark {
    fn into_element(self) -> AnyElement {
        let HostMark {
            initial,
            face,
            emoji,
            size,
            status,
            test_id,
        } = self;
        let side = size.points();
        let down = matches!(status, Some(Status::Offline | Status::Error));
        let (fill, ink) = match (down, &face, &emoji) {
            (true, _, _) => ("muted", "muted-foreground"),
            (false, _, Some(_)) => ("secondary", "foreground"),
            (false, Some(face), None) => {
                let (pattern, wash) = face.tokens();
                (wash, pattern)
            }
            (false, None, None) => ("primary-soft", "primary-text"),
        };
        let glyph = match (emoji, face) {
            (Some(emoji), _) => view()
                .opacity(if down { EMOJI_DOWN } else { 1.0 })
                .child(
                    text(emoji)
                        .text_size(px(size.emoji()))
                        .allow_font_scaling(false)
                        .number_of_lines(1u32),
                )
                .into_element(),
            (None, Some(face)) if face.is_creature() => creature(&face, face.space(size), down),
            (None, Some(face)) => pattern(&face, face.space(size), ink),
            (None, None) => size
                .text()
                .apply(text(initial))
                .font_weight(FontWeight::SEMIBOLD)
                .number_of_lines(1u32)
                .text_color(move || token(ink))
                .into_element(),
        };
        let mut mark = view()
            .relative()
            .w(px(side))
            .h(px(side))
            .flex_shrink_0()
            .rounded_full()
            .bg(move || token(fill))
            .items_center()
            .justify_center()
            .accessibility_elements_hidden(true)
            .child(glyph);
        if let Some(status) = status {
            let dot = match size {
                MarkSize::Md => DotSize::Md,
                MarkSize::Sm => DotSize::Sm,
            };
            // A spinner is wider than a dot: it sits on the corner in its
            // own 16 pt.
            let inner = if status == Status::Connecting {
                16.0
            } else {
                dot.points()
            };
            let outer = inner + 2.0 * RING;
            mark = mark.child(
                view()
                    .absolute()
                    .right(px(-RING))
                    .bottom(px(-RING))
                    .w(px(outer))
                    .h(px(outer))
                    .rounded_full()
                    .bg(move || token("card"))
                    .items_center()
                    .justify_center()
                    .child(status_dot(status).size(dot)),
            );
        }
        if let Some(id) = test_id {
            mark = mark.test_id(&id);
        }
        mark.into_element()
    }
}

/// `face`'s grid in a square `space` points wide, its filled cells in
/// `ink`.
fn pattern(face: &Face, space: f32, ink: &'static str) -> AnyElement {
    let side = usize::from(face.side.max(1));
    let cell = space / side as f32;
    view()
        .flex_col()
        .children((0..side).map(|row| {
            view()
                .flex_row()
                .children((0..side).map(|column| {
                    let mut cell_view = view().w(px(cell)).h(px(cell));
                    if face.filled(row, column) {
                        cell_view = cell_view.bg(move || token(ink));
                    }
                    cell_view.into_element()
                }))
                .into_element()
        }))
        .into_element()
}

/// A creature's grid in a square `space` points wide, whole points a cell:
/// each row's runs of one kind as one block in that kind's colour; asleep
/// and in grey when `muted`.
fn creature(face: &Face, space: f32, muted: bool) -> AnyElement {
    let side = usize::from(face.side.max(1));
    let cell = (space / side as f32).floor().max(1.0);
    let cells = if muted && face.asleep.len() == face.kinds.len() {
        &face.asleep
    } else {
        &face.kinds
    };
    let hue = face.hue;
    view()
        .flex_col()
        .children(cells.chunks(side).map(|row| {
            view()
                .flex_row()
                .children(runs(row).map(|(kind, count)| {
                    let mut block = view().w(px(cell * count as f32)).h(px(cell));
                    if let Some(ink) = creature_ink(kind, hue, muted) {
                        block = block.bg(move || token(ink));
                    }
                    block.into_element()
                }))
                .into_element()
        }))
        .into_element()
}

/// `row`'s runs: each kind and how many cells of it in a row.
fn runs(row: &[u8]) -> impl Iterator<Item = (u8, usize)> + '_ {
    let mut at = 0;
    std::iter::from_fn(move || {
        let kind = *row.get(at)?;
        let count = row[at..].iter().take_while(|k| **k == kind).count();
        at += count;
        Some((kind, count))
    })
}

/// An agent's mark for `status` (an agent's: attention, working, done,
/// idle), in a row's 28 pt leading lane: the waiting hand solid amber, the
/// done check blue, the sparkle muted (dimmer while idle).
pub fn agent_mark(status: Status) -> AnyElement {
    let (name, tone) = match status {
        Status::Attention => ("hand.raised.fill", Tone::Warning),
        Status::Done => ("checkmark.circle", Tone::Info),
        Status::Idle => ("sparkle", Tone::Subtle),
        _ => ("sparkle", Tone::Muted),
    };
    icon(name).size(IconSize::Md).tone(tone).into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creature_face() -> Face {
        Face {
            side: 10,
            cells: vec![true; 100],
            hue: 3,
            kinds: vec![1; 100],
            asleep: vec![1; 100],
        }
    }

    #[test]
    fn a_creature_fills_a_bigger_box_in_whole_points() {
        let key = Face {
            side: 5,
            cells: vec![true; 25],
            hue: 3,
            kinds: Vec::new(),
            asleep: Vec::new(),
        };
        assert_eq!(
            (key.space(MarkSize::Md), key.space(MarkSize::Sm)),
            (25.0, 20.0)
        );
        let creature = creature_face();
        assert!(creature.is_creature() && !key.is_creature());
        for (size, cell) in [(MarkSize::Md, 3.0), (MarkSize::Sm, 2.0)] {
            assert_eq!(creature.space(size) / 10.0, cell);
            assert!(creature.space(size) < size.points());
        }
    }

    #[test]
    fn a_rows_runs_cover_it() {
        let row = [0, 0, 5, 1, 1, 1, 6, 5, 0, 0];
        let runs: Vec<_> = runs(&row).collect();
        assert_eq!(runs, [(0, 2), (5, 1), (1, 3), (6, 1), (5, 1), (0, 2)]);
        assert_eq!(runs.iter().map(|(_, n)| n).sum::<usize>(), row.len());
        assert_eq!(super::runs(&[]).count(), 0);
    }

    #[test]
    fn a_key_face_reads_as_before_and_a_creature_round_trips() {
        let old = r#"{"side":5,"cells":[true,false],"hue":2}"#;
        let face: Face = serde_json::from_str(old).unwrap();
        assert!(!face.is_creature());
        assert_eq!(serde_json::to_string(&face).unwrap(), old);
        let creature = creature_face();
        let text = serde_json::to_string(&creature).unwrap();
        assert_eq!(serde_json::from_str::<Face>(&text).unwrap(), creature);
    }
}
