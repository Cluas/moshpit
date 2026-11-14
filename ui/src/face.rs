//! The app's typography (`Moshpit/UI/DesignTokens.swift`'s `Face`), with
//! Dynamic Type.
//!
//! `Face` resolves a design size to the nearest of the system's text styles
//! (11 is `caption2`, 12 `caption`, 17 `body`), so a label follows Dynamic
//! Type the way the rest of iOS does; below the smallest style (9.75) a
//! size stays as it is. The styles do not grow in step: at the largest
//! non-accessibility size `body` is 23 / 17 of its default and `caption2`
//! 17 / 11. So text here does not use NSH's own scaling, which multiplies
//! every size by the font scale: it reads NSH's font scale ([`font_scale`],
//! on iOS the body style's size over 17) for the user's content size
//! category and takes the style's size there from Apple's table, with NSH's
//! scaling off for that text. At the default size both are the design size.

use nsh_core::{LineHeight, prelude::*};

/// A system text style (`Font.TextStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextStyle {
    LargeTitle,
    Title,
    Title2,
    Title3,
    Headline,
    Body,
    Callout,
    Subheadline,
    Footnote,
    Caption,
    Caption2,
}

/// The text styles' sizes in each content size category, from extra small
/// to AX5 (Apple's Human Interface Guidelines, Typography, iOS).
#[rustfmt::skip]
const SIZES: [(TextStyle, [u8; 12]); 11] = [
    (TextStyle::LargeTitle, [31, 32, 33, 34, 36, 38, 40, 44, 48, 52, 56, 60]),
    (TextStyle::Title, [25, 26, 27, 28, 30, 32, 34, 38, 43, 48, 53, 58]),
    (TextStyle::Title2, [19, 20, 21, 22, 24, 26, 28, 34, 39, 44, 50, 56]),
    (TextStyle::Title3, [17, 18, 19, 20, 22, 24, 26, 31, 37, 43, 49, 55]),
    (TextStyle::Headline, [14, 15, 16, 17, 19, 21, 23, 28, 33, 40, 47, 53]),
    (TextStyle::Body, [14, 15, 16, 17, 19, 21, 23, 28, 33, 40, 47, 53]),
    (TextStyle::Callout, [13, 14, 15, 16, 18, 20, 22, 26, 32, 38, 44, 51]),
    (TextStyle::Subheadline, [12, 13, 14, 15, 17, 19, 21, 25, 30, 36, 42, 49]),
    (TextStyle::Footnote, [12, 12, 12, 13, 15, 17, 19, 23, 27, 33, 38, 44]),
    (TextStyle::Caption, [11, 11, 11, 12, 14, 16, 18, 22, 26, 32, 37, 43]),
    (TextStyle::Caption2, [11, 11, 11, 11, 13, 15, 17, 20, 24, 29, 34, 40]),
];

/// The text styles' leading (the distance from one line's baseline to the
/// next in a paragraph) in each content size category (Apple's Human
/// Interface Guidelines, Typography, iOS): what a SwiftUI `Text` of that
/// style wrapping to more lines than one advances by (measured on the iPad:
/// footnote 16 pt at Small, 24 at xxxLarge).
#[rustfmt::skip]
const LEADING: [(TextStyle, [u8; 12]); 11] = [
    (TextStyle::LargeTitle, [38, 39, 40, 41, 43, 46, 48, 52, 57, 61, 66, 70]),
    (TextStyle::Title, [31, 32, 33, 34, 37, 39, 41, 46, 51, 57, 62, 68]),
    (TextStyle::Title2, [24, 25, 26, 28, 30, 32, 34, 41, 46, 52, 59, 66]),
    (TextStyle::Title3, [22, 23, 24, 25, 28, 30, 32, 38, 44, 51, 57, 64]),
    (TextStyle::Headline, [19, 20, 21, 22, 24, 26, 29, 34, 40, 48, 56, 62]),
    (TextStyle::Body, [19, 20, 21, 22, 24, 26, 29, 34, 40, 48, 56, 62]),
    (TextStyle::Callout, [18, 19, 20, 21, 23, 25, 28, 32, 39, 46, 52, 60]),
    (TextStyle::Subheadline, [16, 18, 19, 20, 22, 24, 26, 31, 37, 43, 50, 58]),
    (TextStyle::Footnote, [16, 16, 16, 18, 20, 22, 24, 29, 33, 40, 46, 52]),
    (TextStyle::Caption, [13, 13, 13, 16, 19, 21, 23, 28, 32, 39, 44, 51]),
    (TextStyle::Caption2, [13, 13, 13, 13, 18, 20, 22, 25, 30, 35, 41, 48]),
];

/// The default category, Large.
pub const LARGE: usize = 3;

/// Extra extra large: where a navigation bar's text stops growing (it never
/// shrinks below Large either).
pub const XX_LARGE: usize = 5;

/// The first accessibility size (`DynamicTypeSize.isAccessibilitySize`):
/// layouts that put things side by side stack them from here.
pub const AX1: usize = 7;

/// The content size category whose body size is nearest `font_scale` times
/// the default's (17): its index in [`SIZES`]'s rows.
pub fn category(font_scale: f32) -> usize {
    if !font_scale.is_finite() || font_scale <= 0.0 {
        return LARGE;
    }
    let body = font_scale * 17.0;
    let sizes = &SIZES[5].1;
    (0..sizes.len())
        .min_by(|&a, &b| {
            let da = (f32::from(sizes[a]) - body).abs();
            let db = (f32::from(sizes[b]) - body).abs();
            da.total_cmp(&db)
        })
        .unwrap_or(LARGE)
}

/// `style`'s size at `font_scale`.
pub fn point_size(style: TextStyle, font_scale: f32) -> f32 {
    let (_, sizes) = SIZES
        .iter()
        .find(|(s, _)| *s == style)
        .expect("every style has sizes");
    f32::from(sizes[category(font_scale)])
}

/// `style`'s size at `font_scale`, but no smaller than at category `min`
/// and no larger than at category `max` (indexes of the table's columns:
/// [`LARGE`], [`XX_LARGE`]).
pub fn point_size_clamped(style: TextStyle, font_scale: f32, min: usize, max: usize) -> f32 {
    let (_, sizes) = SIZES
        .iter()
        .find(|(s, _)| *s == style)
        .expect("every style has sizes");
    f32::from(sizes[category(font_scale).clamp(min, max)])
}

/// The text style a design size stands for (`Face.style(for:)`), by nearest
/// default size; `None` below `caption2`.
pub fn style_for(size: f32) -> Option<TextStyle> {
    Some(match size {
        s if s < 9.75 => return None,
        s if s < 11.75 => TextStyle::Caption2,
        s if s < 12.75 => TextStyle::Caption,
        s if s < 14.0 => TextStyle::Footnote,
        s if s < 15.5 => TextStyle::Subheadline,
        s if s < 16.5 => TextStyle::Callout,
        s if s < 19.0 => TextStyle::Body,
        s if s < 21.0 => TextStyle::Title3,
        s if s < 25.0 => TextStyle::Title2,
        s if s < 31.0 => TextStyle::Title,
        _ => TextStyle::LargeTitle,
    })
}

/// The size design size `size` draws at with `font_scale`: its style's
/// there, or itself below the smallest style.
pub fn scaled(size: f32, font_scale: f32) -> f32 {
    style_for(size).map_or(size, |style| point_size(style, font_scale))
}

/// The leading of a paragraph of design size `size` at `font_scale`: its
/// style's ([`LEADING`]), or the line's own height below the smallest style.
pub fn leading(size: f32, font_scale: f32) -> f32 {
    match style_for(size) {
        Some(style) => {
            let (_, leading) = LEADING
                .iter()
                .find(|(s, _)| *s == style)
                .expect("every style has a leading");
            f32::from(leading[category(font_scale)])
        }
        None => size * LINE_HEIGHT,
    }
}

/// UIKit's height of a line of text `size` points tall
/// (`UIFont.lineHeight`).
pub fn line(size: f32) -> f32 {
    size * LINE_HEIGHT
}

/// The height of a one-line SwiftUI `Text` of `size` points: the line's,
/// rounded up to the device's pixel (`pixel_ratio`), the line at its
/// bottom. CoreText measures the same line up to a whole point, with the
/// line at its top, so a one-line text is pinned to this height and its
/// line moved down ([`one_line`]): the glyphs land where SwiftUI's do.
pub fn frame(size: f32, pixel_ratio: f32) -> f32 {
    let ratio = pixel_ratio.max(1.0);
    // A hair under, so a size whose line is a whole pixel stays one.
    (line(size) * ratio - 1e-3).ceil() / ratio
}

/// `text` pinned to one line at SwiftUI's height for its design size
/// `size` ([`frame`]).
pub fn one_line(text: Text, size: f32) -> Text {
    let slack = move || {
        let size = scaled(size, font_scale());
        frame(size, pixel_ratio()) - line(size)
    };
    text.number_of_lines(1u32)
        .h(move || px(frame(scaled(size, font_scale()), pixel_ratio())))
        .pt(move || px(slack()))
}

/// Where a wrapping text of `size` points with lines `pitch` apart sits
/// against SwiftUI's: `(top, bottom)`, the margins that move CoreText's
/// box onto SwiftUI's. CoreText centers each line in its pitch; SwiftUI
/// starts the first line at its frame's top (below the frame's rounding
/// slack, as [`frame`]) and ends the last with its line, the pitch between
/// lines only: its frame is `(n - 1) × pitch + frame`, CoreText's
/// `n × pitch`. A quarter point of the centering stays: measured on the
/// iPad, footnote paragraphs at Large and xxxLarge land on SwiftUI's pixel
/// rows with it, half a point high without.
pub fn wrapping_margins(size: f32, pitch: f32, pixel_ratio: f32) -> (f32, f32) {
    let (line, frame) = (line(size), frame(size, pixel_ratio));
    let top = -(pitch - line) / 2.0 + (frame - line) + 0.25;
    (top, frame - pitch - top)
}

/// `text` that may wrap, its lines `pitch(font scale)` apart, laid out as
/// SwiftUI lays out a paragraph of design size `size`
/// ([`wrapping_margins`]). CoreText measures the box up to a whole point:
/// once it is laid out, what it has over its lines comes off the bottom
/// margin too, so what follows starts where it does under SwiftUI.
pub fn wrapping(text: Text, size: f32, pitch: impl Fn(f32) -> f32 + Copy + 'static) -> Text {
    let margins = move || {
        let scale = font_scale();
        wrapping_margins(scaled(size, scale), pitch(scale), pixel_ratio())
    };
    let excess = signal(0.0_f32);
    text.line_height(move || LineHeight::Points(px(pitch(font_scale()))))
        .mt(move || px(margins().0))
        .mb(move || px(margins().1 - excess.get()))
        .on_layout(move |bounds: &Bounds<Points>| {
            let measured = rounding_excess(bounds.size.height.0, pitch(font_scale()));
            if (excess.get_untracked() - measured).abs() > 0.01 {
                excess.set(measured);
            }
        })
}

/// What a box `height` tall holding whole lines `pitch` apart has over
/// them: CoreText's rounding up (under a point).
pub fn rounding_excess(height: f32, pitch: f32) -> f32 {
    if pitch.is_nan() || pitch <= 0.0 || !height.is_finite() {
        return 0.0;
    }
    let lines = (height / pitch).round().max(1.0);
    (height - lines * pitch).clamp(0.0, 1.0)
}

/// `text` that may wrap, its lines `leading` apart as SwiftUI sets a
/// paragraph of its style ([`leading`]).
pub fn paragraph(text: Text, size: f32) -> Text {
    wrapping(text, size, move |scale| leading(size, scale))
}

/// `Face`'s designs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Design {
    /// SF Pro (`Face.text`).
    Default,
    /// SF Mono (`Face.mono`).
    Monospaced,
    /// SF Pro Rounded (`Face.display`).
    Rounded,
}

/// The line height of Apple's system fonts, SF Pro and SF Mono alike, per
/// point of size: UIKit's `UIFont.lineHeight` (ascender less descender),
/// which a SwiftUI `Text`'s frame is (rounded up to a pixel). CoreText's own
/// line box for SF Mono differs (iPad, iOS 18.2: 12 pt for 9 pt text where
/// SwiftUI's is 11, 13 for 11 pt where SwiftUI's is 13.5), so a list of log
/// lines drifted half a point a row from the SwiftUI page's.
pub const LINE_HEIGHT: f32 = 1.19336;

/// [`LINE_HEIGHT`] as a style.
pub fn line_height() -> LineHeight {
    LineHeight::Multiple(LINE_HEIGHT)
}

/// `text` in `Face`'s `design` at design size `size` and `weight`,
/// following the user's text size as the Swift app's does, with the
/// system's line height.
pub fn face(text: Text, design: Design, size: f32, weight: FontWeight) -> Text {
    let text = text
        .text_size(move || px(scaled(size, font_scale())))
        .line_height(line_height())
        .font_weight(weight)
        .allow_font_scaling(false);
    match design {
        Design::Default => text,
        Design::Monospaced => text.font_family("monospace"),
        Design::Rounded => text.font_family("ui-rounded"),
    }
}

/// `Face.mono(size)`.
pub fn mono(text: Text, size: f32) -> Text {
    face(text, Design::Monospaced, size, FontWeight::NORMAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_size_is_the_design_size_of_each_style() {
        for (size, style) in [
            (11.0, TextStyle::Caption2),
            (12.0, TextStyle::Caption),
            (13.0, TextStyle::Footnote),
            (15.0, TextStyle::Subheadline),
            (16.0, TextStyle::Callout),
            (17.0, TextStyle::Body),
            (20.0, TextStyle::Title3),
            (22.0, TextStyle::Title2),
            (28.0, TextStyle::Title),
            (34.0, TextStyle::LargeTitle),
        ] {
            assert_eq!(style_for(size), Some(style), "{size}");
            assert_eq!(scaled(size, 1.0), size, "{size}");
        }
    }

    #[test]
    fn sizes_resolve_to_the_nearest_style_as_face_does() {
        assert_eq!(style_for(9.0), None);
        assert_eq!(style_for(9.75), Some(TextStyle::Caption2));
        assert_eq!(
            scaled(10.5, 1.0),
            11.0,
            "a design size takes its style's size"
        );
        assert_eq!(style_for(11.75), Some(TextStyle::Caption));
        assert_eq!(style_for(14.0), Some(TextStyle::Subheadline));
    }

    #[test]
    fn small_labels_stay_fixed() {
        assert_eq!(scaled(9.0, 23.0 / 17.0), 9.0);
        assert_eq!(scaled(9.0, 53.0 / 17.0), 9.0);
    }

    #[test]
    fn each_style_follows_its_own_dynamic_type_row() {
        // NSH's font scale on iOS is the body style's size over 17.
        let xxx_large = 23.0 / 17.0;
        assert_eq!(scaled(17.0, xxx_large), 23.0);
        assert_eq!(
            scaled(11.0, xxx_large),
            17.0,
            "caption2 grows faster than body"
        );
        assert_eq!(scaled(12.0, xxx_large), 18.0);
        let x_small = 14.0 / 17.0;
        assert_eq!(scaled(17.0, x_small), 14.0);
        assert_eq!(scaled(11.0, x_small), 11.0, "caption2 never shrinks");
        let ax5 = 53.0 / 17.0;
        assert_eq!(scaled(11.0, ax5), 40.0);
    }

    #[test]
    fn a_clamped_size_stays_in_its_range() {
        let bar = |scale: f32| point_size_clamped(TextStyle::Body, scale, LARGE, XX_LARGE);
        assert_eq!(bar(1.0), 17.0);
        assert_eq!(bar(19.0 / 17.0), 19.0);
        assert_eq!(bar(21.0 / 17.0), 21.0);
        assert_eq!(bar(23.0 / 17.0), 21.0);
        assert_eq!(bar(53.0 / 17.0), 21.0);
        assert_eq!(bar(14.0 / 17.0), 17.0, "it does not shrink below Large");
    }

    #[test]
    fn paragraphs_advance_by_their_styles_leading() {
        // Measured on the iPad: a footnote paragraph's lines are 16 pt apart
        // at Small (12 pt text) and 24 at xxxLarge (19 pt).
        assert_eq!(leading(13.0, 15.0 / 17.0), 16.0);
        assert_eq!(leading(13.0, 23.0 / 17.0), 24.0);
        assert_eq!(leading(13.0, 1.0), 18.0);
        assert_eq!(leading(17.0, 1.0), 22.0);
        assert_eq!(leading(9.0, 1.0), 9.0 * LINE_HEIGHT);
    }

    #[test]
    fn wrapping_text_takes_swiftuis_frame() {
        // A footnote paragraph at Large: lines 18 apart, 15.51 tall, in a
        // frame of 16 per line; SwiftUI's frame is (n - 1) × 18 + 16.
        let (top, bottom) = wrapping_margins(13.0, 18.0, 2.0);
        let expected = -(18.0 - line(13.0)) / 2.0 + (16.0 - line(13.0)) + 0.25;
        assert!((top - expected).abs() < 1e-4);
        assert!(
            (top + bottom - (16.0 - 18.0)).abs() < 1e-4,
            "two points less, all told"
        );
        // A line exactly its pitch: nothing moves but the rounding.
        let (top, bottom) = wrapping_margins(17.0, line(17.0), 2.0);
        assert!((top - (20.5 - line(17.0) + 0.25)).abs() < 1e-4);
        assert!((top + bottom - (20.5 - line(17.0))).abs() < 1e-4);
    }

    #[test]
    fn coretexts_rounding_is_found_from_the_box() {
        // Four footer lines 17.32 apart: 69.28, rounded up to 70.
        let pitch = line(12.0) + 3.0;
        assert!((rounding_excess(70.0, pitch) - (70.0 - 4.0 * pitch)).abs() < 1e-4);
        assert_eq!(rounding_excess(36.0, 18.0), 0.0);
        assert_eq!(rounding_excess(16.0, 18.0), 0.0, "one line, measured short");
        assert_eq!(rounding_excess(10.0, 0.0), 0.0);
    }

    #[test]
    fn a_one_line_frame_is_the_line_rounded_up_to_a_pixel() {
        assert_eq!(frame(17.0, 2.0), 20.5);
        assert_eq!(frame(15.0, 2.0), 18.0);
        assert_eq!(frame(13.0, 2.0), 16.0);
        assert_eq!(frame(11.0, 2.0), 13.5);
        assert_eq!(frame(17.0, 3.0), 61.0 / 3.0);
    }

    #[test]
    fn every_body_size_names_one_category() {
        for (i, body) in SIZES[5].1.iter().enumerate() {
            assert_eq!(category(f32::from(*body) / 17.0), i);
        }
        assert_eq!(category(f32::NAN), LARGE);
        assert_eq!(category(0.0), LARGE);
        // A scale between two (Android's font scale): the nearest.
        assert_eq!(category(1.05), LARGE);
        assert_eq!(category(1.15), 4);
    }
}
