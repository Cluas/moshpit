import Foundation
import Testing
import UIKit
@testable import Moshpit

/// The birth-grid / zero-bounds contract around `TerminalHostContainer` and
/// the terminals it hosts.
///
/// The incident these pin down (user screenshot, 2026-08-18): over herdr+mosh,
/// output that beat the first layout pass was parsed into a terminal whose
/// `.zero` birth frame derived a two-column grid, and the narrow→wide reflow
/// did not round-trip — so scrolling up showed a column of 1–2 character
/// shards forever. A terminal is born at a grid now
/// (``TerminalSurfaces/make(font:grid:)``), and a zero-bounds pass never
/// reaches it.
@Suite("TerminalHostContainer birth grid")
struct TerminalHostContainerTests {

    @MainActor
    private func makeTerminal(_ grid: TerminalGrid = TerminalGrid(cols: 40, rows: 12)) -> CoreTerminalView {
        TerminalSurfaces.make(font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular), grid: grid)
    }

    @Test("a terminal is born at the grid it was made for, before any layout")
    @MainActor
    func birthGridHolds() {
        let terminal = makeTerminal(TerminalGrid(cols: 52, rows: 20))
        #expect(terminal.grid == TerminalGrid(cols: 52, rows: 20))
    }

    @Test("a transient zero-bounds layout pass leaves the hosted terminal's frame alone")
    @MainActor
    func zeroBoundsLayoutDoesNotSqueeze() {
        let container = TerminalHostContainer()
        let terminal = makeTerminal()
        container.host(terminal)

        let real = CGRect(x: 0, y: 0, width: 390, height: 600)
        container.frame = real
        container.layoutSubviews()
        #expect(terminal.view.frame.size == real.size)
        let settled = terminal.view.frame

        // A sheet transition / remount can run a layout pass at .zero. The
        // terminal must keep its last real frame — adopting .zero reflows the
        // whole buffer to the minimum grid, and that reflow doesn't round-trip.
        container.frame = .zero
        container.layoutSubviews()
        #expect(terminal.view.frame == settled)
    }

    /// `lastReportedSize` is fed ONLY by `sizeChanged` — a layout pass with
    /// no grid change must leave it untouched. Recording the layout-implied
    /// grid there shipped in 368 and pinned tmux windows to a junk grid from
    /// a transient pass (the 499×62 "SSH+tmux 满屏乱码" regression); this
    /// pins the revert.
    @Test("a layout pass alone never writes lastReportedSize")
    @MainActor
    func layoutAloneDoesNotWriteLastReportedSize() {
        let coordinator = CoreTerminalHost.Coordinator()
        let container = TerminalHostContainer()
        let terminal = makeTerminal()
        container.host(terminal)
        coordinator.hostContainer = container

        container.frame = CGRect(x: 0, y: 0, width: 390, height: 600)
        container.layoutSubviews()

        #expect(coordinator.lastReportedSize == nil,
                "only sizeChanged may write this — see the 368 junk-grid regression")
    }
}
