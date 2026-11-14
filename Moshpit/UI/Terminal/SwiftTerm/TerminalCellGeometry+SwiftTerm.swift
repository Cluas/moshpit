import SwiftTerm
import UIKit

extension TerminalCellGeometry {
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
}
