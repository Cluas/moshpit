//! Icons (`docs/nsh/UI-DESIGN.md` §3.6, §4.1): one family, SF Symbols,
//! drawn by the host's `moshpit/symbol` component, in the sizes of text
//! styles ([`IconSize`]) and the tones of the semantic tokens. An icon sits
//! in a square slot its size, so a label beside it does not move when the
//! symbol's own width differs.
//!
//! An icon is decorative unless it is given a label: screen readers skip
//! it, and the control around it carries the words.

use nsh_core::{color::Rgba, prelude::*};
use serde_json::json;

use crate::{
    face::{self, TextStyle},
    theme::hex_string,
    ui::{
        tokens::token,
        typography::{TextToken, X_SMALL, XXX_LARGE},
    },
};

/// An icon's size: a text style's, so an icon grows and shrinks with the
/// user's text size as the words beside it do, from extra small to
/// xxxLarge. Past xxxLarge, at the accessibility sizes, an icon keeps
/// xxxLarge's size, as the system's bars keep theirs (the large content
/// viewer speaks for them): the slots and buttons around icons hold them.
///
/// The scale was 13 / 17 / 22 pt, fixed: every icon drew 1.3–1.7 times
/// the old app's symbols (mostly 11–13 pt) and the system's (17 pt in a
/// bar, 15 in a row) (Canary 118).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconSize {
    /// Caption 2, 11 pt at the default text size: inside a badge, a
    /// status, a chip.
    Sm,
    /// Subheadline, 15 pt: in a row, a button, a menu.
    Md,
    /// Body, 17 pt: in a bar (its buttons), over an empty page's words.
    Lg,
    /// On a key of the shortcut bar, and of Settings' preview of it: the
    /// key's words' style and weight (`mono-key`: subheadline, medium),
    /// following the text size with them only to extra large, where the
    /// terminal's chrome stops. [`IconSize::Md`]'s size up to there, but
    /// medium, and capped: a symbol key reads as heavy as `esc` beside it.
    Key,
}

impl IconSize {
    /// The text style it takes its size from.
    pub fn style(self) -> TextStyle {
        match self {
            IconSize::Sm => TextStyle::Caption2,
            IconSize::Md => TextStyle::Subheadline,
            IconSize::Lg => TextStyle::Body,
            IconSize::Key => KEY_TEXT.style(),
        }
    }

    /// The content size categories it follows: (smallest, largest).
    fn range(self) -> (usize, usize) {
        match self {
            IconSize::Key => KEY_TEXT.range(),
            _ => (X_SMALL, XXX_LARGE),
        }
    }

    /// Its symbol's point size at the default text size (Large), which is
    /// also its slot's side.
    pub fn points(self) -> f32 {
        self.points_at(1.0)
    }

    /// Its symbol's point size at NSH's font scale `font_scale` (the body
    /// style's size over 17).
    pub fn points_at(self, font_scale: f32) -> f32 {
        let (min, max) = self.range();
        face::point_size_clamped(self.style(), font_scale, min, max)
    }

    /// Its point size now: read in a binding, it follows the user's text
    /// size.
    pub fn points_now(self) -> f32 {
        self.points_at(font_scale())
    }

    /// The symbol's weight (`moshpit/symbol`'s `weight`): heavier when
    /// small, so its strokes keep their presence beside the text it sits
    /// in; on a key, the key's words' own.
    pub fn weight(self) -> &'static str {
        match self {
            IconSize::Sm => "semibold",
            IconSize::Md | IconSize::Lg => "regular",
            IconSize::Key => symbol_weight(KEY_TEXT.weight()),
        }
    }
}

/// The text style a key's words are set in, which its symbol follows.
const KEY_TEXT: TextToken = TextToken::MonoKey;

/// An SF Symbol's weight for a font weight: the nearest of the weights the
/// host draws (`light` to `bold`).
fn symbol_weight(weight: FontWeight) -> &'static str {
    match weight.0 {
        0..=349 => "light",
        350..=449 => "regular",
        450..=549 => "medium",
        550..=649 => "semibold",
        _ => "bold",
    }
}

/// The color a foreground (an icon, a label) is drawn in: a token by its
/// role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tone {
    Foreground,
    Muted,
    Subtle,
    /// The accent as text and icons (`primary-text`).
    Primary,
    /// On the accent's fill (`primary-foreground`).
    OnPrimary,
    Success,
    Warning,
    Info,
    Destructive,
    /// On a key of the shortcut bar.
    Key,
}

impl Tone {
    /// The token it reads.
    pub fn token(self) -> &'static str {
        match self {
            Tone::Foreground => "foreground",
            Tone::Muted => "muted-foreground",
            Tone::Subtle => "subtle-foreground",
            Tone::Primary => "primary-text",
            Tone::OnPrimary => "primary-foreground",
            Tone::Success => "success-text",
            Tone::Warning => "warning-text",
            Tone::Info => "info-text",
            Tone::Destructive => "destructive-text",
            Tone::Key => "key-foreground",
        }
    }
}

/// An icon: the SF Symbol `name` ([`icon`]).
#[derive(Debug, Clone)]
pub struct Icon {
    name: &'static str,
    size: IconSize,
    tone: Tone,
    label: Option<SharedString>,
}

/// The SF Symbol `name` at the middle size, in the foreground's tone.
pub fn icon(name: &'static str) -> Icon {
    Icon {
        name,
        size: IconSize::Md,
        tone: Tone::Foreground,
        label: None,
    }
}

impl Icon {
    pub fn size(mut self, size: IconSize) -> Self {
        self.size = size;
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// What screen readers say for it, when the icon itself means
    /// something (a state with no words beside it).
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl IntoElement for Icon {
    fn into_element(self) -> AnyElement {
        let Icon {
            name,
            size,
            tone,
            label,
        } = self;
        let slot = view()
            .w(move || px(size.points_now()))
            .h(move || px(size.points_now()))
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .child(dynamic(move || {
                let tint = Rgba::from(color(token(tone.token())));
                let points = size.points_now();
                untracked(move || {
                    native("moshpit/symbol")
                        .prop("name", json!(name))
                        .prop("pointSize", json!(points))
                        .prop("weight", json!(size.weight()))
                        .prop("color", json!(hex_string(tint)))
                })
            }));
        match label {
            Some(label) => slot
                .role(AccessibilityRole::Image)
                .label(label)
                .into_element(),
            None => slot.accessibility_elements_hidden(true).into_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NSH's font scale at a content size category: its body size over 17.
    fn scale(body: f32) -> f32 {
        body / 17.0
    }

    #[test]
    fn sizes_are_text_styles_to_xxx_large() {
        // Apple's sizes at xSmall, Large, xLarge, xxxLarge and AX3 (body
        // 14, 17, 19, 23, 40), written out: caption 2, subheadline, body;
        // AX3 stays at xxxLarge's.
        let expected = [
            (IconSize::Sm, [11.0, 11.0, 13.0, 17.0, 17.0], "semibold"),
            (IconSize::Md, [12.0, 15.0, 17.0, 21.0, 21.0], "regular"),
            (IconSize::Lg, [14.0, 17.0, 19.0, 23.0, 23.0], "regular"),
            (IconSize::Key, [12.0, 15.0, 17.0, 17.0, 17.0], "medium"),
        ];
        for (size, points, weight) in expected {
            for (body, points) in [14.0, 17.0, 19.0, 23.0, 40.0].into_iter().zip(points) {
                assert_eq!(
                    size.points_at(scale(body)),
                    points,
                    "{size:?} at body {body}"
                );
            }
            assert_eq!(size.weight(), weight, "{size:?}");
        }
        assert_eq!(IconSize::Sm.points(), 11.0);
        assert_eq!(IconSize::Md.points(), 15.0);
        assert_eq!(IconSize::Lg.points(), 17.0);
    }

    #[test]
    fn a_keys_symbol_is_its_words_size_and_weight() {
        for body in [14.0_f32, 17.0, 19.0, 23.0, 40.0] {
            assert_eq!(
                IconSize::Key.points_at(scale(body)),
                TextToken::MonoKey.size(scale(body)),
                "at body {body}"
            );
        }
        assert_eq!(symbol_weight(TextToken::MonoKey.weight()), "medium");
    }

    #[test]
    fn no_icon_is_larger_than_its_text_style() {
        // The ceiling the owner set: no larger than caption 2 / subheadline
        // / body at the default text size.
        assert!(IconSize::Sm.points() <= 11.0);
        assert!(IconSize::Md.points() <= 15.0);
        assert!(IconSize::Lg.points() <= 17.0);
        assert!(IconSize::Key.points() <= 15.0);
    }

    #[test]
    fn a_font_weight_maps_to_the_nearest_symbol_weight() {
        assert_eq!(symbol_weight(FontWeight::NORMAL), "regular");
        assert_eq!(symbol_weight(FontWeight::MEDIUM), "medium");
        assert_eq!(symbol_weight(FontWeight::SEMIBOLD), "semibold");
        assert_eq!(symbol_weight(FontWeight::BOLD), "bold");
        assert_eq!(symbol_weight(FontWeight(300)), "light");
    }
}
