import Foundation
import Testing
import UIKit
@testable import Moshpit

/// The keyboard cover's two decisions: is this move worth hiding, and when
/// has it been hidden long enough?
///
/// It exists because a resize makes the REMOTE app redraw, and we would
/// otherwise paint the half-erased screen it goes through (measured against a
/// real Claude Code pane: tmux's own copy loses its footer for ~50ms). Hiding
/// that is worth a beat of frozen picture. Hiding NOTHING is not — switching
/// input method fires the same notification with an identical frame, and
/// covering it just makes the switch feel slow. And the beat must not end
/// before the resize it hides, nor last as long as a pane keeps printing.
/// Serialized: the timing tests sleep, and run best one at a time.
@Suite("keyboard cover", .serialized)
@MainActor
struct KeyboardCoverTests {

    private static let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)

    /// A terminal filling 320×480 (the core's view).
    private func makeTerminal() -> CoreTerminalView {
        let cell = TerminalCellGeometry.measuredCell(font: Self.font, scale: UITraitCollection.current.displayScale)
        return TerminalSurfaces.make(font: Self.font, grid: TerminalGrid(cols: Int(320 / cell.width),
                                                                          rows: Int(480 / cell.height)))
    }

    private func makeCoordinator() -> CoreTerminalHost.Coordinator {
        let coordinator = CoreTerminalHost.Coordinator()
        let terminal = makeTerminal()
        let container = TerminalHostContainer(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        container.host(terminal)
        container.layoutSubviews()
        coordinator.attach(terminal)
        coordinator.hostContainer = container
        // The notification handler ignores a terminal with no window that is
        // not first responder; a hosted one in a window passes.
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 800))
        window.addSubview(container)
        return coordinator
    }

    private func post(begin: CGRect, end: CGRect) {
        NotificationCenter.default.post(
            name: UIResponder.keyboardWillChangeFrameNotification, object: nil,
            userInfo: [
                UIResponder.keyboardFrameBeginUserInfoKey: NSValue(cgRect: begin),
                UIResponder.keyboardFrameEndUserInfoKey: NSValue(cgRect: end),
                UIResponder.keyboardAnimationDurationUserInfoKey: 0.25,
                UIResponder.keyboardAnimationCurveUserInfoKey: 7,
            ])
    }

    @Test("An input-method switch — same frame both ends — is not covered")
    func sameFrameIsNotCovered() {
        let coordinator = makeCoordinator()
        let up = CGRect(x: 0, y: 500, width: 320, height: 300)
        post(begin: up, end: up)
        #expect(coordinator.coverRequests == 0,
                "covering a move that changes nothing just makes it feel slow")
    }

    @Test("A keyboard actually arriving is covered")
    func realMoveIsCovered() {
        let coordinator = makeCoordinator()
        post(begin: CGRect(x: 0, y: 800, width: 320, height: 300),
             end: CGRect(x: 0, y: 500, width: 320, height: 300))
        #expect(coordinator.coverRequests == 1)
    }

    // MARK: - When the cover comes off

    /// A pane in its container, driven the way the keyboard notifications
    /// drive it (`lockFrame`, `coverForKeyboard`, `unlockFrame`). Out of any
    /// window, so the app-wide keyboard notifications other suites post or
    /// raise pass it by.
    /// The cover is a picture of the pane, which the core's view draws from
    /// its own rows (a view never drawn, as a test host's, has one too).
    private func makeHostedPane() -> (CoreTerminalHost.Coordinator, TerminalHostContainer, UIView) {
        let coordinator = CoreTerminalHost.Coordinator()
        let terminal = makeTerminal()
        let container = TerminalHostContainer(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        container.host(terminal)
        container.layoutSubviews()
        coordinator.attach(terminal)
        coordinator.hostContainer = container
        return (coordinator, container, terminal.view)
    }

    /// The cover is a plate over the pane, faded to nothing when it comes off.
    private func covered(_ container: TerminalHostContainer, _ terminal: UIView) -> Bool {
        container.subviews.contains { $0 !== terminal && $0.alpha > 0 }
    }

    /// An agent's spinner repaints every ~100 ms, more often than the 150 ms
    /// of quiet the cover waits for, so only the cap can take it off — and
    /// the cap was recomputed at every check, so it never came: the frozen
    /// picture stayed up for as long as the agent ran (6 s of output every
    /// 100 ms on the Mac, still covered).
    @Test("A pane that never goes quiet loses its cover at the cap, 1.6 s after it went up")
    func streamingPaneLosesItsCoverAtTheCap() async throws {
        let (coordinator, container, terminal) = makeHostedPane()
        coordinator.lockFrame(for: 0.25)
        coordinator.coverForKeyboard(duration: 0.25, options: [])
        try #require(covered(container, terminal), "no cover went up")

        let start = ContinuousClock.now
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
        var frame = 0
        while covered(container, terminal), start.duration(to: .now) < .seconds(4) {
            coordinator.feed(data: Data("\r\(spinner[frame % spinner.count]) Working…".utf8))
            frame += 1
            try? await Task.sleep(for: .milliseconds(30))
        }

        #expect(!covered(container, terminal),
                "still covered after \(start.duration(to: .now)) of output every 30 ms")
    }

    /// Waits until the cover is off, up to `limit` after `start`.
    private func waitUncovered(_ container: TerminalHostContainer, _ terminal: UIView,
                               from start: ContinuousClock.Instant,
                               limit: Duration = .seconds(3)) async -> ContinuousClock.Instant {
        while covered(container, terminal), start.duration(to: .now) < limit {
            try? await Task.sleep(for: .milliseconds(10))
        }
        return .now
    }

    /// The keyboard's animation ends at `duration` (0.25 s); the terminal is
    /// resized only when the move is over — didChangeFrame, ~0.52 s in on an
    /// iPad, or the frame lock's own timeout at 0.4 s — and the remote
    /// repaints after that. The cover came off at `duration` if the pane was
    /// quiet, so the reflow and the repaint it exists to hide happened in the
    /// open (spike G5: 24 of 24 transitions).
    @Test("The cover stays until the terminal has been resized, and a quiet beat after its repaint")
    func coverWaitsForTheResize() async throws {
        let (coordinator, container, terminal) = makeHostedPane()
        // A move whose end has not been heard yet: only `unlockFrame`
        // (didChangeFrame) ends it; the lock's own timeout is far off.
        coordinator.lockFrame(for: 10)
        coordinator.coverForKeyboard(duration: 0.25, options: [])
        try #require(covered(container, terminal), "no cover went up")
        // The keyboard takes the bottom of the pane; the locked terminal
        // keeps its size meanwhile.
        container.frame.size.height = 300
        container.layoutIfNeeded()
        #expect(terminal.frame.height == 480)

        try? await Task.sleep(for: .milliseconds(600))
        #expect(covered(container, terminal), "the cover came off before the terminal was resized")

        coordinator.unlockFrame()
        let resized = ContinuousClock.now
        #expect(terminal.frame.size == container.bounds.size, "the move is over: the terminal takes its size")
        #expect(covered(container, terminal), "the cover came off with the resize")
        // The remote's repaint for the new size, a beat later.
        try? await Task.sleep(for: .milliseconds(50))
        coordinator.feed(data: Data("\u{1b}[H\u{1b}[2Jrepainted for the new size".utf8))
        let repainted = ContinuousClock.now

        let off = await waitUncovered(container, terminal, from: resized)
        #expect(!covered(container, terminal), "still covered \(resized.duration(to: off)) after the resize")
        #expect(repainted.duration(to: off) >= .milliseconds(140),
                "off \(repainted.duration(to: off)) after the repaint, before a quiet beat")
    }

    /// Over tmux a shrink is held until the capture that replaces what it
    /// destroys is in hand (`shouldDeferResize`), then applied with it. The
    /// resize the cover waits for is that one, not the end of the lock.
    @Test("Over a resize held for its repaint, the cover waits for that repaint")
    func coverWaitsForAHeldResize() async throws {
        let (coordinator, container, terminal) = makeHostedPane()
        coordinator.shouldDeferResize = { _, _ in true }
        coordinator.lockFrame(for: 10)
        coordinator.coverForKeyboard(duration: 0.25, options: [])
        try #require(covered(container, terminal), "no cover went up")
        container.frame.size.height = 300

        coordinator.unlockFrame()
        #expect(terminal.frame.height == 480, "the shrink waits for its capture")
        try? await Task.sleep(for: .milliseconds(600))
        #expect(covered(container, terminal), "the cover came off while the resize was still held")

        // The capture arrives: the held resize, then the repaint, in one turn.
        coordinator.applyDeferredResize()
        coordinator.feed(data: Data("\u{1b}[H\u{1b}[2Jthe capture".utf8))
        let applied = ContinuousClock.now
        #expect(terminal.frame.size == container.bounds.size, "the held resize was applied")

        let off = await waitUncovered(container, terminal, from: applied)
        #expect(!covered(container, terminal), "still covered \(applied.duration(to: off)) after the capture")
        #expect(applied.duration(to: off) >= .milliseconds(140),
                "off \(applied.duration(to: off)) after the capture, before a quiet beat")
    }
}
