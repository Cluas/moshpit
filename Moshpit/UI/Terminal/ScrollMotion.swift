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
    mutating func take(pixels: CGFloat, cellHeight: CGFloat) -> Int {
        guard cellHeight > 0, pixels.isFinite else { return 0 }
        remainder += pixels
        let rows = Int((remainder / cellHeight).rounded(.towardZero))
        remainder -= CGFloat(rows) * cellHeight
        return rows
    }

    mutating func reset() { remainder = 0 }
}
