import UIKit

/// Cell boxes and point → grid cell: which character did the user touch?
/// SwiftTerm's view is measured in `TerminalCellGeometry+SwiftTerm.swift`;
/// the core's view knows its own cells (``CoreTermMetrics``).
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

    /// Clamped viewport-point → cell. Pure, for testing.
    static func cell(at point: CGPoint, cell: CGSize, cols: Int, rows: Int) -> (col: Int, row: Int) {
        guard cell.width > 0, cell.height > 0 else { return (0, 0) }
        let col = min(max(0, Int((point.x / cell.width).rounded(.down))), max(0, cols - 1))
        let row = min(max(0, Int((point.y / cell.height).rounded(.down))), max(0, rows - 1))
        return (col, row)
    }
}
