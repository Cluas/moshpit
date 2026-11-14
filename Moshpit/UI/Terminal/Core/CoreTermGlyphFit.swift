import CoreGraphics
import CoreText

/// How a glyph wider than its cells is fitted to them (B472), the same in
/// the Metal atlas (``CoreTermAtlas``) and the CoreGraphics snapshot
/// (``CoreTermCGRenderer``).
///
/// Both narrowed such a glyph on x alone, to its advance: a colour emoji
/// one or two cells wide drawn from Apple Color Emoji (Claude Code's "⏺"
/// bullets, U+23FA, one cell by its text presentation) kept its full
/// height and came out a tall thin pill. Now its ink is scaled uniformly
/// to fit the cells (and never taller than the line: ascent to descent),
/// keeping its shape, and centred: across the cells, and between the
/// baseline and the ascent, kept inside the line. A glyph whose advance is wide but
/// whose ink fits (herdr's and Claude's "⎿", from a CJK fallback) is only
/// moved into its cells, not shrunk. Box drawing, block elements and
/// powerline glyphs must fill the cell's height to join their neighbours:
/// they are narrowed on x, as before.
///
/// The transform applies at the baseline origin, in points, y up:
/// translate by (`dx`, `dy`), then scale by (`scaleX`, `scaleY`).
struct CoreTermGlyphFit: Equatable {
    var scaleX: CGFloat
    var scaleY: CGFloat
    var dx: CGFloat
    var dy: CGFloat

    /// The fit for a line `advance` points wide whose ink is `ink` (from
    /// the baseline origin, y up), drawn into `span` points of cells whose
    /// font rises `ascent` above the baseline and falls `descent` below it;
    /// nil when it fits as it is. `scalar`: the line's first character,
    /// which picks the glyphs that keep the cell's height.
    static func fit(advance: CGFloat, ink: CGRect, span: CGFloat, ascent: CGFloat, descent: CGFloat,
                    scalar: UInt32) -> CoreTermGlyphFit? {
        guard span > 0, advance > span * 1.05 else { return nil }
        if keepsCellHeight(scalar) || ink.isNull || ink.width <= 0 || ink.height <= 0 {
            return CoreTermGlyphFit(scaleX: span / advance, scaleY: 1, dx: 0, dy: 0)
        }
        let line = ascent + descent
        let s = min(1, span / ink.width, line > 0 ? line / ink.height : 1)
        let dx = (span - ink.width * s) / 2 - ink.minX * s
        // Not shrunk: where its font put it. Shrunk: centred between the
        // baseline and the ascent, then kept inside the line.
        guard s < 1 else { return CoreTermGlyphFit(scaleX: 1, scaleY: 1, dx: dx, dy: 0) }
        var dy = ascent / 2 - ink.midY * s
        let top = dy + ink.maxY * s, bottom = dy + ink.minY * s
        if top > ascent {
            dy -= top - ascent
        } else if bottom < -descent {
            dy += -descent - bottom
        }
        return CoreTermGlyphFit(scaleX: s, scaleY: s, dx: dx, dy: dy)
    }

    /// Box drawing, block elements, the legacy computing symbols and the
    /// powerline glyphs: drawn edge to edge, they join only at the cell's
    /// full height.
    static func keepsCellHeight(_ scalar: UInt32) -> Bool {
        (0x2500...0x259F).contains(scalar) || (0x1FB00...0x1FBFF).contains(scalar)
            || (0xE0A0...0xE0D7).contains(scalar)
    }

    /// Applies it to `ctx`, at the baseline origin. `flipped`: the
    /// context's y runs down and its text matrix turns the glyphs upright
    /// (the snapshot's), so up is negative.
    func apply(to ctx: CGContext, flipped: Bool = false) {
        ctx.translateBy(x: dx, y: flipped ? -dy : dy)
        ctx.scaleBy(x: scaleX, y: scaleY)
    }

    /// `line`'s ink from its origin, y up, whatever text matrix `ctx` has.
    static func ink(of line: CTLine, in ctx: CGContext) -> CGRect {
        let matrix = ctx.textMatrix
        ctx.textMatrix = .identity
        defer { ctx.textMatrix = matrix }
        return CTLineGetImageBounds(line, ctx)
    }
}
