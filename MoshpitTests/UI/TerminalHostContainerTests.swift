import Foundation
import Testing
import UIKit
import SwiftTerm
@testable import Moshpit

/// The birth-grid / zero-bounds contract around `TerminalHostContainer` and
/// the plain-path terminal in `SwiftTerminalView.makeUIView`.
///
/// The incident these pin down (user screenshot, 2026-08-18): over herdr+mosh,
/// output that beat the first layout pass was parsed into a terminal whose
/// `.zero` birth frame derived SwiftTerm's two-column minimum grid. The
/// two-column line fragments sank into the 50k scrollback, and SwiftTerm's
/// narrow→wide reflow does not round-trip — so scrolling up showed a column of
/// 1–2 character shards forever while the visible screen self-corrected.
@Suite("TerminalHostContainer birth grid")
struct TerminalHostContainerTests {

    @MainActor
    private func makeTerminalView(frame: CGRect = .zero) -> TerminalView {
        TerminalView(frame: frame,
                     font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular))
    }

    /// Documents the hazard that motivates the screen-sized birth frame: a
    /// `.zero` frame doesn't mean "no grid yet", it means the MINIMUM grid —
    /// two columns — and everything fed before layout wraps at that width.
    @Test("a terminal born at .zero derives the two-column minimum grid; born at screen size it's phone-shaped")
    @MainActor
    func birthFrameDerivesGrid() {
        let zeroBorn = makeTerminalView()
        #expect(zeroBorn.getTerminal().cols <= 2)

        let screenBorn = makeTerminalView(
            frame: CGRect(origin: .zero, size: UIScreen.main.bounds.size))
        #expect(screenBorn.getTerminal().cols >= 20)
    }

    @Test("a transient zero-bounds layout pass leaves the hosted terminal's frame alone")
    @MainActor
    func zeroBoundsLayoutDoesNotSqueeze() {
        let container = TerminalHostContainer()
        let terminal = makeTerminalView()
        container.host(terminal)

        let real = CGRect(x: 0, y: 0, width: 390, height: 600)
        container.frame = real
        container.layoutSubviews()
        #expect(terminal.frame == real)

        // A sheet transition / remount can run a layout pass at .zero. The
        // terminal must keep its last real frame — adopting .zero reflows the
        // whole buffer to the minimum grid, and that reflow doesn't round-trip.
        container.frame = .zero
        container.layoutSubviews()
        #expect(terminal.frame == real)
    }

    /// `lastReportedSize` is fed ONLY by `sizeChanged` — a layout pass with
    /// no grid change must leave it untouched. Recording the layout-implied
    /// grid there shipped in 368 and pinned tmux windows to a junk grid from
    /// a transient pass (the 499×62 "SSH+tmux 满屏乱码" regression); this
    /// pins the revert.
    @Test("a layout pass alone never writes lastReportedSize")
    @MainActor
    func layoutAloneDoesNotWriteLastReportedSize() {
        let coordinator = SwiftTerminalView.Coordinator()
        let container = TerminalHostContainer()
        let terminal = makeTerminalView()
        container.host(terminal)
        coordinator.hostContainer = container

        container.frame = CGRect(x: 0, y: 0, width: 390, height: 600)
        container.layoutSubviews()

        #expect(coordinator.lastReportedSize == nil,
                "only sizeChanged may write this — see the 368 junk-grid regression")
    }
}
