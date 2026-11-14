//! The type scale (`docs/nsh/UI-DESIGN.md` §3.1): the text styles the `ui`
//! layer and the screens on it name, in place of design sizes.
//!
//! Each token is one of Apple's text styles in a design and a weight, and
//! follows Dynamic Type through [`crate::face`] (the style's size at the
//! user's content size category, from Apple's table), within the token's
//! range: a large title grows from Large to the last size short of the
//! accessibility sizes (as UIKit's bar lets it), the terminal's keys stop
//! at extra large (keyboard-like chrome, as the system keyboard does),
//! everything else follows to AX5.
//!
//! The section title (eyebrow) is the app's one uppercase style, a brand
//! exception the owner picks ([`EYEBROW`]); Chinese, Japanese and Korean
//! never take it (no uppercase, no tracking, no monospace for CJK).

use nsh_core::prelude::*;

use crate::{
    face::{self, Design, TextStyle},
    ui::tokens::token,
};

/// The content size categories, as indexes into [`face`]'s tables.
pub const X_SMALL: usize = 0;
pub const X_LARGE: usize = 4;
pub const XXX_LARGE: usize = 6;
pub const AX3: usize = 9;
pub const AX5: usize = 11;

/// A text style of the type scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextToken {
    /// A root page's large title (`text-large-title`): 34, bold.
    LargeTitle,
    /// An empty state's title, a sheet's hero (`text-title`): 22, semibold.
    Title,
    /// A host's name, a row title that leads (`text-headline`): 17,
    /// semibold.
    Headline,
    /// Row titles, field values, buttons (`text-body`): 17.
    Body,
    /// Row subtitles, row values, a panel's text (`text-subhead`): 15.
    Subhead,
    /// Group footers, field help, errors, banners (`text-footnote`): 13.
    Footnote,
    /// Badges, the version line (`text-caption`): 12, medium.
    Caption,
    /// `user@host`, paths, pane ids (`mono-subhead`): 15.
    MonoSubhead,
    /// Fingerprints, commands, log lines (`mono-footnote`): 13.
    MonoFootnote,
    /// The `SSH` / `mosh` badges (`mono-caption`): 12, medium.
    MonoCaption,
    /// A key of the shortcut bar (`mono-key`): 15, medium, to extra large.
    MonoKey,
}

impl TextToken {
    pub const ALL: [TextToken; 11] = [
        TextToken::LargeTitle,
        TextToken::Title,
        TextToken::Headline,
        TextToken::Body,
        TextToken::Subhead,
        TextToken::Footnote,
        TextToken::Caption,
        TextToken::MonoSubhead,
        TextToken::MonoFootnote,
        TextToken::MonoCaption,
        TextToken::MonoKey,
    ];

    /// Its name in the design (`text-body`).
    pub fn name(self) -> &'static str {
        match self {
            TextToken::LargeTitle => "text-large-title",
            TextToken::Title => "text-title",
            TextToken::Headline => "text-headline",
            TextToken::Body => "text-body",
            TextToken::Subhead => "text-subhead",
            TextToken::Footnote => "text-footnote",
            TextToken::Caption => "text-caption",
            TextToken::MonoSubhead => "mono-subhead",
            TextToken::MonoFootnote => "mono-footnote",
            TextToken::MonoCaption => "mono-caption",
            TextToken::MonoKey => "mono-key",
        }
    }

    /// Apple's text style it is.
    pub fn style(self) -> TextStyle {
        match self {
            TextToken::LargeTitle => TextStyle::LargeTitle,
            TextToken::Title => TextStyle::Title2,
            TextToken::Headline => TextStyle::Headline,
            TextToken::Body => TextStyle::Body,
            TextToken::Subhead | TextToken::MonoSubhead | TextToken::MonoKey => {
                TextStyle::Subheadline
            }
            TextToken::Footnote | TextToken::MonoFootnote => TextStyle::Footnote,
            TextToken::Caption | TextToken::MonoCaption => TextStyle::Caption,
        }
    }

    /// Its typeface: SF Pro, or SF Mono for what a machine names.
    pub fn design(self) -> Design {
        match self {
            TextToken::MonoSubhead
            | TextToken::MonoFootnote
            | TextToken::MonoCaption
            | TextToken::MonoKey => Design::Monospaced,
            _ => Design::Default,
        }
    }

    pub fn weight(self) -> FontWeight {
        match self {
            TextToken::LargeTitle => FontWeight::BOLD,
            TextToken::Title | TextToken::Headline => FontWeight::SEMIBOLD,
            TextToken::Caption | TextToken::MonoCaption | TextToken::MonoKey => FontWeight::MEDIUM,
            _ => FontWeight::NORMAL,
        }
    }

    /// The content size categories it follows: (smallest, largest).
    pub fn range(self) -> (usize, usize) {
        match self {
            TextToken::LargeTitle => (face::LARGE, XXX_LARGE),
            TextToken::MonoKey => (X_SMALL, X_LARGE),
            _ => (X_SMALL, AX5),
        }
    }

    /// Its size at `font_scale` (NSH's, the body style's size over 17).
    pub fn size(self, font_scale: f32) -> f32 {
        let (min, max) = self.range();
        face::point_size_clamped(self.style(), font_scale, min, max)
    }

    /// `text` set in this style, following the user's text size.
    pub fn apply(self, text: Text) -> Text {
        set(text, self.design(), self.weight(), move || {
            self.size(font_scale())
        })
    }
}

/// `text` in `design` and `weight` at `size(font scale)`, on the system
/// fonts' line height, NSH's own scaling off (the size already follows
/// Dynamic Type as [`face::face`] has it).
fn set(text: Text, design: Design, weight: FontWeight, size: impl Fn() -> f32 + 'static) -> Text {
    let text = text
        .text_size(move || px(size()))
        .line_height(face::line_height())
        .font_weight(weight)
        .allow_font_scaling(false);
    match design {
        Design::Default => text,
        Design::Monospaced => text.font_family("monospace"),
        Design::Rounded => text.font_family("ui-rounded"),
    }
}

// ---------------------------------------------------------------------------
// The section title.

/// How a section's title (its eyebrow) is set (UI-DESIGN §0 #6,
/// `section-titles`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eyebrow {
    /// SF Mono, upper case, tracked: today's brand.
    MonoCaps,
    /// SF Pro, upper case, tracked.
    UiCaps,
    /// SF Pro, as written (sentence case).
    Sentence,
}

/// The owner's pick for §0 #6; the recommendation is [`Eyebrow::MonoCaps`].
/// Another pick is this one line.
pub const EYEBROW: Eyebrow = Eyebrow::MonoCaps;

/// What an eyebrow is set in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EyebrowStyle {
    pub design: Design,
    pub style: TextStyle,
    pub weight: FontWeight,
    pub uppercase: bool,
    /// Letter spacing, in points.
    pub tracking: f32,
}

impl Eyebrow {
    /// The style of this pick in a language: CJK takes neither case nor
    /// tracking nor monospace, whatever the pick.
    pub fn style(self, cjk: bool) -> EyebrowStyle {
        if cjk {
            return EyebrowStyle {
                design: Design::Default,
                style: TextStyle::Footnote,
                weight: FontWeight::MEDIUM,
                uppercase: false,
                tracking: 0.0,
            };
        }
        match self {
            Eyebrow::MonoCaps => EyebrowStyle {
                design: Design::Monospaced,
                style: TextStyle::Caption,
                weight: FontWeight::SEMIBOLD,
                uppercase: true,
                tracking: 0.6,
            },
            Eyebrow::UiCaps => EyebrowStyle {
                design: Design::Default,
                style: TextStyle::Caption,
                weight: FontWeight::SEMIBOLD,
                uppercase: true,
                tracking: 0.5,
            },
            Eyebrow::Sentence => EyebrowStyle {
                design: Design::Default,
                style: TextStyle::Footnote,
                weight: FontWeight::SEMIBOLD,
                uppercase: false,
                tracking: 0.0,
            },
        }
    }
}

/// Whether `language` (a BCP 47 language subtag) is Chinese, Japanese or
/// Korean.
pub fn is_cjk(language: &str) -> bool {
    matches!(language, "zh" | "ja" | "ko")
}

/// `text` set as a section's title: [`EYEBROW`] in the surface's language,
/// in `muted-foreground` (its color is part of the style, §3.1). The caller
/// passes the words as written; upper case is the style's.
pub fn eyebrow(text: Text) -> Text {
    let style = EYEBROW.style(is_cjk(locale().language()));
    let size = move || face::point_size(style.style, font_scale());
    let text =
        set(text, style.design, style.weight, size).text_color(move || token("muted-foreground"));
    let text = if style.tracking > 0.0 {
        text.letter_spacing(px(style.tracking))
    } else {
        text
    };
    if style.uppercase {
        text.uppercase()
    } else {
        text
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
    fn sizes_are_apples_at_large_xxxlarge_and_ax3() {
        // Apple's Human Interface Guidelines, Typography, iOS (Large,
        // xxxLarge, AX3), written out here rather than read from `face`'s
        // table; a token's own range caps it (large title at xxxLarge,
        // keys at xLarge).
        let expected = [
            (TextToken::LargeTitle, [34.0, 40.0, 40.0]),
            (TextToken::Title, [22.0, 28.0, 44.0]),
            (TextToken::Headline, [17.0, 23.0, 40.0]),
            (TextToken::Body, [17.0, 23.0, 40.0]),
            (TextToken::Subhead, [15.0, 21.0, 36.0]),
            (TextToken::Footnote, [13.0, 19.0, 33.0]),
            (TextToken::Caption, [12.0, 18.0, 32.0]),
            (TextToken::MonoSubhead, [15.0, 21.0, 36.0]),
            (TextToken::MonoFootnote, [13.0, 19.0, 33.0]),
            (TextToken::MonoCaption, [12.0, 18.0, 32.0]),
            (TextToken::MonoKey, [15.0, 17.0, 17.0]),
        ];
        assert_eq!(
            expected.len(),
            TextToken::ALL.len(),
            "every token is checked"
        );
        for (token, sizes) in expected {
            for (body, size) in [17.0, 23.0, 40.0].into_iter().zip(sizes) {
                assert_eq!(
                    token.size(scale(body)),
                    size,
                    "{} at body {body}",
                    token.name()
                );
            }
        }
        // A large title never shrinks below Large.
        assert_eq!(TextToken::LargeTitle.size(scale(14.0)), 34.0);
    }

    #[test]
    fn identifiers_are_monospaced_and_words_are_not() {
        for token in TextToken::ALL {
            let mono = token.name().starts_with("mono-");
            assert_eq!(
                token.design() == Design::Monospaced,
                mono,
                "{}",
                token.name()
            );
        }
    }

    #[test]
    fn the_eyebrow_is_the_pick_and_cjk_never_takes_caps() {
        assert_eq!(
            EYEBROW,
            Eyebrow::MonoCaps,
            "the recommendation, until the owner picks"
        );
        let latin = EYEBROW.style(false);
        assert!(latin.uppercase && latin.tracking > 0.0);
        assert_eq!(latin.design, Design::Monospaced);
        for pick in [Eyebrow::MonoCaps, Eyebrow::UiCaps, Eyebrow::Sentence] {
            let cjk = pick.style(true);
            assert!(!cjk.uppercase, "{pick:?}");
            assert_eq!(cjk.tracking, 0.0, "{pick:?}");
            assert_eq!(cjk.design, Design::Default, "{pick:?}");
        }
        assert!(is_cjk("zh") && is_cjk("ja") && is_cjk("ko"));
        assert!(!is_cjk("en") && !is_cjk("de"));
    }
}
