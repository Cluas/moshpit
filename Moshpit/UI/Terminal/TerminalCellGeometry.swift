import UIKit
import SwiftTerm

/// Point → grid cell for a SwiftTerm ``TerminalView``: which character did the
/// user touch?
///
/// SwiftTerm knows this exactly — it lays the grid out on a `cellDimension`
/// computed from the font's "W" advance and line metrics, snapped to the pixel
/// grid — but keeps that value internal. So the box is measured here from the
/// same public font, the same way, and then **checked against the grid the view
/// itself reports**: if our box doesn't divide the view's bounds into the exact
/// `cols × rows` the view has, it isn't the box the view is drawing with and we
/// don't use it. The fallback divides the bounds by the grid, which can drift by
/// at most one cell across a full row — a wrong-by-one column beats a
/// wrong-by-anything one.
enum TerminalCellGeometry {

    /// The visible cell box in points, or the divided-bounds estimate.
    static func cellSize(of terminalView: TerminalView) -> CGSize {
        let terminal = terminalView.getTerminal()
        let cols = max(1, terminal.cols)
        let rows = max(1, terminal.rows)
        let bounds = terminalView.bounds.size
        let divided = CGSize(width: bounds.width / CGFloat(cols),
                             height: bounds.height / CGFloat(rows))
        guard bounds.width > 0, bounds.height > 0 else { return divided }
        let scale = terminalView.window?.screen.scale ?? 0
        guard scale > 0 else { return divided }
        let measured = measuredCell(font: terminalView.font, scale: scale)
        guard grid(for: bounds, cell: measured) == (cols, rows) else { return divided }
        return measured
    }

    /// The `cols × rows` a cell box yields for `bounds` — SwiftTerm's own
    /// truncating division, so a box that really is the view's reproduces the
    /// view's grid. Pure, for testing.
    static func grid(for bounds: CGSize, cell: CGSize) -> (cols: Int, rows: Int) {
        guard cell.width > 0, cell.height > 0 else { return (0, 0) }
        return (Int(bounds.width / cell.width), Int(bounds.height / cell.height))
    }

    /// The cell box a font implies, mirroring SwiftTerm's `computeFontDimensions`:
    /// the "W" advance for the width, ascent + descent + leading for the height,
    /// both snapped up to whole device pixels so adjacent cells share no seam.
    static func measuredCell(font: UIFont, scale: CGFloat) -> CGSize {
        let ct = font as CTFont
        let height = ceil(CTFontGetAscent(ct) + CTFontGetDescent(ct) + CTFontGetLeading(ct))
        let width = ("W" as NSString).size(withAttributes: [.font: font]).width
        guard scale > 0 else { return CGSize(width: width, height: height) }
        return CGSize(width: ceil(width * scale) / scale,
                      height: ceil(height * scale) / scale)
    }

    /// The cell under `point` (in `terminalView`'s own coordinate space),
    /// clamped to the visible grid.
    ///
    /// Rows count from the top of the VIEWPORT, not of the buffer: the caller
    /// turns this into a mouse report, and the program reading it thinks in
    /// screen rows. `TerminalView` is a scroll view, so a point in its
    /// coordinate space is a *content* point — subtracting `contentOffset` is
    /// what makes the two agree once there's scrollback above the fold.
    static func cell(at point: CGPoint, in terminalView: TerminalView) -> (col: Int, row: Int) {
        let terminal = terminalView.getTerminal()
        let cell = cellSize(of: terminalView)
        return Self.cell(at: CGPoint(x: point.x, y: point.y - terminalView.contentOffset.y),
                         cell: cell,
                         cols: terminal.cols,
                         rows: terminal.rows)
    }

    /// Clamped viewport-point → cell. Pure, for testing.
    static func cell(at point: CGPoint, cell: CGSize, cols: Int, rows: Int) -> (col: Int, row: Int) {
        guard cell.width > 0, cell.height > 0 else { return (0, 0) }
        let col = min(max(0, Int((point.x / cell.width).rounded(.down))), max(0, cols - 1))
        let row = min(max(0, Int((point.y / cell.height).rounded(.down))), max(0, rows - 1))
        return (col, row)
    }
}
