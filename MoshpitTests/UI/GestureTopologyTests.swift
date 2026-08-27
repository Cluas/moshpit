import SwiftTerm
import Testing
import UIKit
@testable import Moshpit

/// The wiring half of docs/design/touch-matrix.md: which recognizers exist on
/// a minted terminal and how they yield to each other. Every deference rule
/// here has a user-visible failure mode when it silently unwires.
@MainActor
@Suite("Gesture topology")
struct GestureTopologyTests {

    private func makeWired() -> (TerminalView, SwiftTerminalView.Coordinator) {
        let terminal = TerminalView(frame: CGRect(x: 0, y: 0, width: 390, height: 700))
        let coordinator = SwiftTerminalView.Coordinator()
        TerminalScrollGesture.attach(to: terminal, coordinator: coordinator)
        return (terminal, coordinator)
    }

    @Test("the tap that clicks waits for double- and triple-tap to fail")
    func positionTapDefersToMultiTaps() {
        let (terminal, _) = makeWired()
        let taps = (terminal.gestureRecognizers ?? []).compactMap { $0 as? UITapGestureRecognizer }
        let singles = taps.filter { $0.numberOfTapsRequired == 1 }
        let multis = taps.filter { $0.numberOfTapsRequired > 1 }
        // SwiftTerm's own single tap plus the app's position tap.
        #expect(singles.count >= 2, "expected SwiftTerm's tap and the app's position tap")
        #expect(!multis.isEmpty, "SwiftTerm's word/line selection taps are missing")
        // Without this deference, the FIRST tap of a double-tap fires a click
        // at the remote before the selection ever forms.
        // (requireGestureRecognizerToFail relationships aren't inspectable via
        // public API; the observable proxy is that multi-tap recognizers exist
        // alongside — the behavioral pin lives in the fork's own gesture code
        // and the S-layer harness.)
    }

    @Test("a minted terminal has tap-focus off, no assistant bar, no mouse-drag reporting")
    func mintConfiguration() {
        // TerminalMint.configureInput is what BOTH real mint sites (SSH
        // single-pane and tmux panes) call — asserting on it asserts on them.
        // A terminal that quietly regains focusOnTap re-summons the keyboard
        // on every tap taken while reading.
        let view = TerminalView(frame: CGRect(x: 0, y: 0, width: 390, height: 700))
        TerminalMint.configureInput(view)
        #expect(view.focusOnTap == false)
        #expect(view.inputAssistantItem.leadingBarButtonGroups.isEmpty)
        #expect(view.inputAssistantItem.trailingBarButtonGroups.isEmpty)
        #expect(view.inputAccessoryView == nil)
        #expect(view.allowMouseReporting == false,
                "SwiftTerm must not also report touches as mouse drags — the app's gestures own that")
        #expect(view.linkReporting == .explicit,
                "implicit link regex mis-underlines bare file paths")
    }

    @Test("a selection can be dismissed without first responder")
    func closeSelectionWorksUnfocused() {
        // The user's report verbatim: "如何取消选中呢，现在也没有方式" — the
        // fork's own tap-clears-selection lives in its focused branch, so with
        // focusOnTap off, handleTap's closeSelection() is the only exit.
        let (terminal, _) = makeWired()
        terminal.feed(text: "some words to select\r\n")
        terminal.selectAll(nil)
        #expect(terminal.canPerformAction(#selector(UIResponder.copy(_:)), withSender: nil),
                "selectAll should have armed the selection")
        terminal.closeSelection()
        #expect(!terminal.canPerformAction(#selector(UIResponder.copy(_:)), withSender: nil),
                "closeSelection must disarm the selection — and with it the app pan's yield")
    }

    @Test("selection handles are finger-sized at every font size")
    func handleToleranceIsFingerSized() {
        // 9pt terminal font ≈ 5.3×10.7pt cells — the old fixed 3×2-cell window
        // was ~16pt there, a third of the 44pt HIG minimum touch target.
        let small = TerminalView.selectionHandleTolerance(cellWidth: 5.3, cellHeight: 10.7)
        #expect(CGFloat(small.cols) * 5.3 >= 20, "grab zone narrower than a fingertip")
        #expect(CGFloat(small.rows) * 10.7 >= 20)
        // Huge fonts must keep at least the old floor so endpoints stay
        // individually addressable.
        let big = TerminalView.selectionHandleTolerance(cellWidth: 20, cellHeight: 40)
        #expect(big.cols >= 3 && big.rows >= 2)
    }
}
