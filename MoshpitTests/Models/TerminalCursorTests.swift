import Foundation
import Testing
import UIKit
import SwiftTerm
@testable import Moshpit

/// The user's cursor style/colour must be AUTHORITATIVE: remote output
/// carries the same controls (DECSCUSR for shape/blink, OSC 12 for colour)
/// and vim, zsh plugins, and coding agents emit them freely. These tests pin
/// the enforcement path — apply, feed a hostile remote sequence through the
/// coordinator, and expect the user's choice to survive.
@Suite("TerminalCursor enforcement")
@MainActor
struct TerminalCursorTests {

    /// A live TerminalView + attached coordinator, as minted by the tmux
    /// controller / SwiftTerminalView.
    private static func makeTerminal() -> (TerminalView, SwiftTerminalView.Coordinator) {
        let terminal = TerminalView(frame: .zero, font: .monospacedSystemFont(ofSize: 13, weight: .regular))
        let coordinator = SwiftTerminalView.Coordinator()
        terminal.terminalDelegate = coordinator
        coordinator.attach(to: terminal)
        return (terminal, coordinator)
    }

    @Test("shape+blink map to the matching SwiftTerm caret style")
    func styleMapping() {
        #expect(TerminalCursor.style(shape: .block, blink: true) == .blinkBlock)
        #expect(TerminalCursor.style(shape: .block, blink: false) == .steadyBlock)
        #expect(TerminalCursor.style(shape: .underline, blink: true) == .blinkUnderline)
        #expect(TerminalCursor.style(shape: .underline, blink: false) == .steadyUnderline)
        #expect(TerminalCursor.style(shape: .bar, blink: true) == .blinkBar)
        #expect(TerminalCursor.style(shape: .bar, blink: false) == .steadyBar)
    }

    @Test("apply sets the caret style via the API, not the feed channel")
    func applySetsStyle() {
        let (terminal, _) = Self.makeTerminal()
        TerminalCursor.apply(shape: .bar, colorId: "teal", blink: false, to: terminal)
        #expect(terminal.getTerminal().options.cursorStyle == .steadyBar)
    }

    @Test("remote DECSCUSR in fed output cannot override the enforced style")
    func remoteDecscusrIsOverruled() {
        let (terminal, coordinator) = Self.makeTerminal()
        coordinator.enforcedCursor = TerminalCursor.apply(
            shape: .underline, colorId: "green", blink: false, to: terminal)

        // A remote app (vim insert mode) asks for a blinking bar.
        coordinator.feed(data: Data("\u{1B}[5 q".utf8))

        #expect(terminal.getTerminal().options.cursorStyle == .steadyUnderline,
                "the user's Settings choice must survive remote DECSCUSR")
    }

    @Test("remote OSC 12 in fed output cannot override the enforced colour")
    func remoteOsc12IsOverruled() {
        let (terminal, coordinator) = Self.makeTerminal()
        let desired = TerminalCursor.apply(
            shape: .block, colorId: "teal", blink: true, to: terminal)
        coordinator.enforcedCursor = desired

        // A remote program recolours the cursor red.
        coordinator.feed(data: Data("\u{1B}]12;#ff0000\u{07}".utf8))

        #expect(terminal.caretColor == desired.color,
                "the user's cursor colour must survive remote OSC 12")
    }

    @Test("without an enforced cursor, remote DECSCUSR still works (no enforcement leak)")
    func noEnforcementWithoutDesired() {
        let (terminal, coordinator) = Self.makeTerminal()
        coordinator.feed(data: Data("\u{1B}[6 q".utf8))
        #expect(terminal.getTerminal().options.cursorStyle == .steadyBar)
    }

    @Test("a DECSCUSR split across feed chunks is still overruled after the closing byte")
    func splitSequenceIsOverruled() {
        let (terminal, coordinator) = Self.makeTerminal()
        coordinator.enforcedCursor = TerminalCursor.apply(
            shape: .bar, colorId: "white", blink: true, to: terminal)

        // Chunk boundary mid-sequence — the parser holds state across feeds.
        coordinator.feed(data: Data("\u{1B}[3".utf8))
        coordinator.feed(data: Data(" q".utf8))

        #expect(terminal.getTerminal().options.cursorStyle == .blinkBar)
    }

    @Test("a UTF-8 char split across two feeds renders intact — no U+FFFD")
    func splitUTF8AcrossFeedsReassembles() {
        // The tmux -CC pipeline delivers %output payloads byte-exact, so a
        // multi-byte char can legitimately arrive half in one feed and half
        // in the next. SwiftTerm's reading buffer must reassemble it — this
        // pins the end-to-end behaviour the control-client fix relies on.
        let (terminal, coordinator) = Self.makeTerminal()
        terminal.getTerminal().resize(cols: 40, rows: 5)

        var first = Data("AB".utf8)
        first.append(contentsOf: [0xE2, 0x94])          // half of ─
        coordinator.feed(data: first)
        var second = Data([0x80])                        // rest of ─
        second.append(contentsOf: Data("你".utf8).prefix(2))  // 2/3 of 你
        coordinator.feed(data: second)
        coordinator.feed(data: Data(Data("你".utf8).suffix(1)))

        let text = terminal.getTerminal().getText(
            start: Position(col: 0, row: 0), end: Position(col: 10, row: 0))
        #expect(text.contains("AB─你"), "got: \(text.debugDescription)")
        #expect(!text.contains("\u{FFFD}"))
    }
}
