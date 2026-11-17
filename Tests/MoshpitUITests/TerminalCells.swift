import XCTest

/// Where a column of the pane sits across one of its rows, for a tap or a
/// drag on a terminal's text. A row's accessibility element
/// (CoreTermRowElement) is the terminal view's whole width; its cells are
/// the app's own: the "W" of the system monospaced font at the terminal's
/// size, in whole pixels (TerminalCellGeometry.measuredCell), as many as
/// fit, the grid centred in the view to the pixel
/// (CoreTerminalView.gridOrigin). A row's label is its text, a character a
/// column for the ASCII the tests aim at.
///
/// B817: test06 guessed 40 columns and tapped two columns past its path on
/// a row of 62.
enum TerminalCells {
    /// The terminal's size when a flow doesn't set it (AppSettings.fontSize).
    static let defaultSize: CGFloat = 9

    /// The x of `column` (fractional: 2.5 is the middle of the third)
    /// across `row`, as a fraction of the row, and the row's columns.
    @MainActor
    static func x(ofColumn column: CGFloat, in row: XCUIElement, of app: XCUIApplication,
                  size: CGFloat = defaultSize) -> (CGFloat, Int) {
        // The screen's pixels a point: a screenshot's against the app's frame.
        let pixels = XCUIScreen.main.screenshot().image.cgImage.map { CGFloat(max($0.width, $0.height)) } ?? 0
        let points = max(app.frame.width, app.frame.height)
        let scale = pixels > 0 && points > 0 ? max((pixels / points).rounded(), 1) : 3
        let font = UIFont.monospacedSystemFont(ofSize: size, weight: .regular)
        let cell = ceil(("W" as NSString).size(withAttributes: [.font: font]).width * scale) / scale
        let width = row.frame.width
        guard cell > 0, width > cell else { return (0.3, 0) }
        let columns = Int(width / cell)
        let spare = width - CGFloat(columns) * cell
        let origin = min(spare, ((spare / 2) * scale).rounded() / scale)
        let x = (origin + column * cell) / width
        return (min(max(x, 0.02), 0.98), columns)
    }

    /// The middle of `count` columns from `column` across `row`.
    @MainActor
    static func middle(ofColumns column: Int, _ count: Int, in row: XCUIElement, of app: XCUIApplication,
                       size: CGFloat = defaultSize) -> (CGFloat, Int) {
        x(ofColumn: CGFloat(column) + CGFloat(count) / 2, in: row, of: app, size: size)
    }
}
