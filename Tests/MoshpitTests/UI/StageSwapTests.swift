import Foundation
import Testing
import UIKit
@testable import Moshpit

/// B445: the terminal still came up blank (Canary 122, herdr), and leaving
/// the screen and entering it again was the cure. The stage only said a
/// terminal arrived when a slot adopted it, not when it hosted one, so a
/// swap's terminal looked as if it never got there; it changed terminals
/// only when SwiftUI handed it a new target, so a terminal made again under
/// the same target waited for the screen to be entered again; on the app's
/// return a screen out of sight took the terminal from the one in sight;
/// and nothing looked at what showed after it was put there. What the stage
/// now does: every terminal hosted is said, drawn whole and looked at a
/// moment later (``TerminalNSHStage/check(why:now:)``); a terminal minted
/// is checked against the target on the next turn
/// (``TerminalNSHStage/verify(why:)``); only a slot on screen takes it.
@Suite("the terminal stage holds what the target names (B445)", .serialized)
@MainActor
struct StageSwapTests {

    /// A controller whose pane's terminal is made again under the same
    /// pane id (what a controller's own re-mint does): the target, the same
    /// value, names a new terminal.
    final class RemintingController: TmuxSessionController {
        var reminted: TerminalSurface?
        override func surface(for paneId: String) -> TerminalSurface {
            reminted ?? super.surface(for: paneId)
        }
    }

    private static let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)

    @Test("A terminal made again under the same target is in the slot, sized, on the next turn: no re-run")
    func aNewTerminalUnderTheSameTargetIsHosted() async throws {
        let controller = RemintingController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (slot, window) = TerminalNSHTests.slot(for: stage)
        defer { window.isHidden = true }
        let target = TerminalNSHStage.Target(source: .pane(controller, "%1"), inset: 4)
        stage.show(target)
        let first = try #require(stage.terminal)

        // The same target, a new terminal: SwiftUI hands the stage nothing
        // new (the value is equal), the screen is not entered again.
        let fresh = TerminalSurfaces.make(font: Self.font, grid: TerminalGrid(cols: 80, rows: 24))
        controller.reminted = fresh
        try await TerminalNSHTests.settle { stage.terminal === fresh.view }
        #expect(stage.terminal === fresh.view, "the new terminal shows")
        #expect(fresh.view.superview?.superview === slot, "in the slot on screen")
        let container = try #require(stage.container)
        #expect(container.frame == slot.bounds.insetBy(dx: 4, dy: 4), "sized to the slot")
        #expect(container.alpha == 1)
        #expect(first.window == nil, "the old one gone at once")
        #expect(stage.target == target, "still the target SwiftUI gave")
    }

    @Test("A terminal put in the slot is drawn whole there")
    func hostingDrawsWhole() throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (_, window) = TerminalNSHTests.slot(for: stage)
        defer { window.isHidden = true }
        guard let core = controller.surface(for: "%1") as? CoreTerminalView else { return }
        let before = core.fullRedraws
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        #expect(core.fullRedraws > before)
    }

    @Test("Nothing changes for a terminal minted for another pane")
    func anotherPanesTerminalChangesNothing() async throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (_, window) = TerminalNSHTests.slot(for: stage)
        defer { window.isHidden = true }
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        let first = try #require(stage.terminal)
        let container = try #require(stage.container)
        _ = controller.surface(for: "%2")
        try await Task.sleep(for: .milliseconds(60))
        #expect(stage.terminal === first)
        #expect(stage.container === container, "not hosted again")
        #expect(stage.verify(why: "test") == .fine)
    }

    @Test("A screen out of sight never takes the terminal from the one in sight")
    func aStageOutOfSightKeepsItsHands() throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let target = TerminalNSHStage.Target(source: .pane(controller, "%1"), inset: 4)
        // The screen below in the stack (or one on its way out): its slot is
        // in no window.
        let hidden = TerminalNSHStage()
        hidden.register(TerminalNSHSlotView(frame: CGRect(x: 0, y: 0, width: 300, height: 400)))
        hidden.show(target)
        // The screen in sight takes the terminal.
        let shown = TerminalNSHStage()
        let (slot, window) = TerminalNSHTests.slot(for: shown)
        defer { window.isHidden = true }
        shown.show(target)
        let terminal = try #require(shown.terminal)

        // The app comes back: both stages hear it.
        #expect(hidden.reassert() != .rehosted, "no slot on screen: nothing to take it into")
        #expect(hidden.verify(why: "test") == .nothing)
        #expect(shown.reassert() == .fine)
        #expect(shown.terminal === terminal)
        #expect(terminal.superview?.superview === slot)
        #expect(terminal.window === window)
    }

    @Test("The look after hosting puts back a host knocked out of its slot and faded, and says so")
    func theCheckPutsItRight() throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (slot, window) = TerminalNSHTests.slot(for: stage)
        defer { window.isHidden = true }
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        let healthy = try #require(stage.check(why: "test"))
        #expect(healthy.fixed.isEmpty, "\(healthy.line)")
        #expect(healthy.inSlot)
        #expect(healthy.alpha == 1)
        #expect(healthy.line.hasSuffix(": fine"))

        let container = try #require(stage.container)
        container.removeFromSuperview()
        container.alpha = 0
        let found = try #require(stage.check(why: "test"))
        #expect(found.fixed.contains("moved it into the slot"), "\(found.line)")
        #expect(found.fixed.contains("made it opaque"), "\(found.line)")
        #expect(container.superview === slot)
        #expect(container.alpha == 1)
        #expect(found.line.contains(": fixed "))
    }

    @Test("The look lifts a pane-switch veil stuck over the terminal")
    func theCheckLiftsAStuckVeil() throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (_, window) = TerminalNSHTests.slot(for: stage)
        defer { window.isHidden = true }
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        let coordinator = try #require(controller.coordinator(for: "%1"))
        coordinator.veilForSwitch()
        let early = try #require(stage.check(why: "test"))
        #expect(early.veil != nil, "a veil inside its cap is said")
        #expect(early.fixed.isEmpty, "and stays")
        let late = try #require(stage.check(why: "test", now: CACurrentMediaTime() + 60))
        #expect(late.fixed.contains { $0.hasPrefix("lifted a veil") }, "\(late.line)")
        #expect(!coordinator.isCoverPresented)
    }

    @Test("No target or no slot on screen: nothing to look at")
    func nothingToLookAt() {
        let stage = TerminalNSHStage()
        #expect(stage.check(why: "test") == nil)
        #expect(stage.verify(why: "test") == .nothing)
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        stage.register(TerminalNSHSlotView(frame: CGRect(x: 0, y: 0, width: 300, height: 400)))
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        #expect(stage.check(why: "test") == nil, "the slot is in no window")
    }
}
