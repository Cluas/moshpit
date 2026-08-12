import Foundation
import Testing
import UIKit
@testable import Moshpit

/// Tap → cell, the arithmetic tap-to-position rides on. A column off by one is
/// the difference between landing on the character the user touched and landing
/// next to it, so the flooring and the clamps are pinned here; the "is this
/// really the box SwiftTerm is drawing with" check is pinned via ``grid``, which
/// is the same truncating division SwiftTerm derives its grid from.
@Suite("Terminal cell geometry")
struct TerminalCellGeometryTests {

    typealias G = TerminalCellGeometry

    @Test("a point lands in the cell it is inside, not the nearest one")
    func floors() {
        let cell = CGSize(width: 10, height: 20)
        #expect(G.cell(at: CGPoint(x: 0, y: 0), cell: cell, cols: 80, rows: 24) == (0, 0))
        #expect(G.cell(at: CGPoint(x: 9.9, y: 19.9), cell: cell, cols: 80, rows: 24) == (0, 0))
        #expect(G.cell(at: CGPoint(x: 10, y: 20), cell: cell, cols: 80, rows: 24) == (1, 1))
        #expect(G.cell(at: CGPoint(x: 95, y: 59), cell: cell, cols: 80, rows: 24) == (9, 2))
    }

    @Test("past the last cell clamps to the grid instead of reporting off-screen")
    func clamps() {
        let cell = CGSize(width: 10, height: 20)
        // A tap in the sliver left over past the last whole cell.
        #expect(G.cell(at: CGPoint(x: 805, y: 487), cell: cell, cols: 80, rows: 24) == (79, 23))
        // Negative can arrive from a viewport-relative point on a scrolled view.
        #expect(G.cell(at: CGPoint(x: -4, y: -30), cell: cell, cols: 80, rows: 24) == (0, 0))
    }

    @Test("a degenerate cell or grid reports the origin rather than dividing by zero")
    func degenerate() {
        #expect(G.cell(at: CGPoint(x: 50, y: 50), cell: .zero, cols: 80, rows: 24) == (0, 0))
        #expect(G.cell(at: CGPoint(x: 50, y: 50),
                       cell: CGSize(width: 10, height: 20), cols: 0, rows: 0) == (0, 0))
    }

    @Test("grid divides truncating, the way SwiftTerm sizes its own grid")
    func gridDivision() {
        // 803pt of width at 10pt cells is 80 columns and 3pt of leftover.
        #expect(G.grid(for: CGSize(width: 803, height: 487), cell: CGSize(width: 10, height: 20))
                == (80, 24))
        #expect(G.grid(for: CGSize(width: 803, height: 487), cell: .zero) == (0, 0))
    }

    @Test("the measured box is whole device pixels and matches a monospace advance")
    func measured() {
        let font = UIFont.monospacedSystemFont(ofSize: 13, weight: .regular)
        let cell = G.measuredCell(font: font, scale: 3)
        #expect(cell.width > 0)
        #expect(cell.height > 0)
        // Snapped up to the pixel grid: an exact multiple of 1/scale.
        #expect(abs((cell.width * 3).rounded() - cell.width * 3) < 0.0001)
        #expect(abs((cell.height * 3).rounded() - cell.height * 3) < 0.0001)
        // A monospaced font advances every character by the cell width, so the
        // box has to be as wide as one — this is the number a wrong measurement
        // would drift from.
        let advance = ("W" as NSString).size(withAttributes: [.font: font]).width
        #expect(cell.width >= advance)
        #expect(cell.width - advance < 1)
    }
}
