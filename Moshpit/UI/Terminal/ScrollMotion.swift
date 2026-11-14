import CoreGraphics
import Foundation

/// Finger travel in points → whole rows, carrying the sub-row remainder to the
/// next tick instead of throwing it away. Routes that can only move by rows (a
/// forwarded wheel, tmux copy-mode over mosh, herdr) use it so a slow drag
/// adds up to the same rows a fast one does — the old `Int(dy / cellHeight)`
/// dropped up to a row per tick and made a slow drag barely move.
struct ScrollRowCarry {
    private(set) var remainder: CGFloat = 0

    /// Positive = toward older output. Returns the whole rows now due.
    ///
    /// Any finite travel is taken (B149): `Int(_:)` trapped on rows past
    /// Int's range — a delta of 1e300 points, or a cell height near zero —
    /// so the rows saturate at Int's ends, as moshpit-tmux's carry does (Fix
    /// (carry-overflow)). A remainder pushed past Double's range starts over:
    /// left infinite or NaN it would have answered every later drag with
    /// Int.min, or nothing.
    mutating func take(pixels: CGFloat, cellHeight: CGFloat) -> Int {
        guard cellHeight > 0, pixels.isFinite else { return 0 }
        remainder += pixels
        let rows = (remainder / cellHeight).rounded(.towardZero)
        remainder -= rows * cellHeight
        if !remainder.isFinite { remainder = 0 }
        if rows.isNaN { return 0 }
        if rows >= CGFloat(Int.max) { return .max }
        if rows <= CGFloat(Int.min) { return .min }
        return Int(rows)
    }

    mutating func reset() { remainder = 0 }
}
