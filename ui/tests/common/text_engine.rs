//! The headless engine's text measure, with East Asian wide characters at
//! their width: NSH's `ApproxTextEngine` takes every character as 0.55 of
//! its font size, which is about right for Latin text and half of what a
//! Chinese, Japanese or Korean character takes on a phone, so a line of
//! kana or hangul that doesn't fit on the device fit here. This one
//! measures a wide or fullwidth character (Unicode's East Asian Width W
//! or F) as the font size itself, and anything else as Approx does; where
//! the text goes inside its box (lines, carets, hit tests) is Approx's.

use std::ops::Range;

use nsh_core::{
    ApproxTextEngine, AttachmentAlignments, AttributedText, Bounds, Point, Points, Size,
    TextEngine, TextLayout, TextLine, TextPosition,
};

/// Approx's advance, in ems.
const NARROW: f32 = 0.55;

/// A wide character's advance, in ems.
const WIDE: f32 = 1.0;

/// Whether `c` is East Asian Wide or Fullwidth: CJK ideographs and their
/// punctuation, kana, hangul, fullwidth forms.
pub fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF | 0xA000..=0xA4CF | 0xA960..=0xA97F | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF | 0xFE10..=0xFE19 | 0xFE30..=0xFE6F | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6 | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF
        | 0x20000..=0x2FFFD | 0x30000..=0x3FFFD)
}

/// `line`'s narrow and wide characters, counted.
fn counts(line: &str) -> (usize, usize) {
    line.chars().fold((0, 0), |(narrow, wide), c| {
        if is_wide(c) {
            (narrow, wide + 1)
        } else {
            (narrow + 1, wide)
        }
    })
}

/// [`ApproxTextEngine`] measuring wide characters at their width.
#[derive(Debug, Default, Clone, Copy)]
pub struct WideTextEngine;

/// Approx's prepared text, and each line's narrow and wide characters
/// (the text's, when it has one line).
pub struct Prepared {
    approx: <ApproxTextEngine as TextEngine>::Prepared,
    lines: Vec<(usize, usize)>,
    font: Points,
    max_lines: u32,
}

impl WideTextEngine {
    fn prepared(
        text: &AttributedText,
        approx: <ApproxTextEngine as TextEngine>::Prepared,
    ) -> Prepared {
        let font = text
            .spans
            .first()
            .map_or(Points(14.0), |s| s.style.font_size);
        Prepared {
            approx,
            lines: text.text.split('\n').map(counts).collect(),
            font,
            max_lines: text.max_lines,
        }
    }
}

impl TextEngine for WideTextEngine {
    type Prepared = Prepared;

    fn prepare(&self, text: &AttributedText) -> Prepared {
        Self::prepared(text, ApproxTextEngine.prepare(text))
    }

    fn prepare_aligned(
        &self,
        text: &AttributedText,
        alignments: &AttachmentAlignments,
    ) -> Prepared {
        Self::prepared(text, ApproxTextEngine.prepare_aligned(text, alignments))
    }

    /// As Approx measures, each line by its characters' widths: wrapped at
    /// `max_width` into as many lines as it fills, at most the text's
    /// line limit.
    fn measure(&self, prepared: &Prepared, max_width: Option<Points>) -> Size<Points> {
        let font = prepared.font.0;
        let (mut width, mut lines) = (0.0_f32, 0.0_f32);
        for &(narrow, wide) in &prepared.lines {
            // Approx's product, in its order, so a Latin line measures to
            // the bit as it did.
            let w = narrow as f32 * font * NARROW + wide as f32 * font * WIDE;
            let (lw, n) = match max_width {
                Some(max) if w > max.0 && max.0 > 0.0 => (max.0, (w / max.0).ceil()),
                _ => (w, 1.0),
            };
            width = width.max(lw);
            lines += n;
        }
        if prepared.max_lines > 0 {
            lines = lines.min(prepared.max_lines as f32);
        }
        Size::new(Points(width), Points(font * 1.2 * lines))
    }

    fn first_baseline(&self, prepared: &Prepared, width: Points) -> Option<Points> {
        ApproxTextEngine.first_baseline(&prepared.approx, width)
    }

    fn breaks_lines_greedily(&self) -> bool {
        ApproxTextEngine.breaks_lines_greedily()
    }

    fn draws_colors_from_text(&self) -> bool {
        ApproxTextEngine.draws_colors_from_text()
    }

    fn layout(&self, prepared: &Prepared, size: Size<Points>) -> TextLayout {
        ApproxTextEngine.layout(&prepared.approx, size)
    }

    fn lines(&self, layout: &TextLayout) -> Vec<TextLine> {
        ApproxTextEngine.lines(layout)
    }

    fn attachments(&self, layout: &TextLayout) -> Vec<Option<Point<Points>>> {
        ApproxTextEngine.attachments(layout)
    }

    fn hit_test(&self, layout: &TextLayout, point: Point<Points>) -> TextPosition {
        ApproxTextEngine.hit_test(layout, point)
    }

    fn caret_rect(&self, layout: &TextLayout, position: TextPosition) -> Option<Bounds<Points>> {
        ApproxTextEngine.caret_rect(layout, position)
    }

    fn range_rects(&self, layout: &TextLayout, range: Range<u32>, out: &mut Vec<Bounds<Points>>) {
        ApproxTextEngine.range_rects(layout, range, out)
    }

    fn register_font(&self, data: &[u8]) -> Result<(), String> {
        ApproxTextEngine.register_font(data)
    }
}
