import Foundation
import Testing
import UIKit
import SwiftTerm
@testable import Moshpit

/// `Coordinator.resetInputParser()` — the reconnect-boundary hygiene for the
/// plain-path terminal (mosh, plain SSH), which is REUSED across reconnects.
///
/// A weak-network disconnect routinely cuts the stream mid-CJK-character or
/// mid-OSC (coding agents retitle constantly, so OSC is a fat target). The
/// leftover parse state then poisons the fresh connection's first bytes: a
/// half-character putback mangles the first glyph, and a dangling OSC
/// swallows the whole first repaint up to the next BEL. `SessionHub.start`
/// calls the reset at every (re)connect boundary.
@Suite("Input parser reset at the reconnect boundary")
@MainActor
struct TerminalParserResetTests {

    private static func makeTerminal() -> (TerminalView, SwiftTerminalView.Coordinator) {
        let terminal = TerminalView(
            frame: CGRect(x: 0, y: 0, width: 400, height: 300),
            font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular))
        let coordinator = SwiftTerminalView.Coordinator()
        terminal.terminalDelegate = coordinator
        coordinator.attach(to: terminal)
        return (terminal, coordinator)
    }

    private func rowText(_ terminal: TerminalView, row: Int = 0) -> String {
        terminal.getTerminal().getText(
            start: Position(col: 0, row: row), end: Position(col: 20, row: row))
    }

    @Test("a stream cut mid-CJK-character doesn't mangle the next connection's first bytes")
    func halfCharacterPoisonCleared() {
        let (terminal, coordinator) = Self.makeTerminal()
        terminal.getTerminal().resize(cols: 40, rows: 5)

        // The dead connection's last chunk ends 2 bytes into a 3-byte char.
        coordinator.feed(data: Data(Data("你".utf8).prefix(2)))
        coordinator.resetInputParser()
        coordinator.feed(data: Data("ABC".utf8))

        let text = rowText(terminal)
        #expect(text.contains("ABC"), "got: \(text.debugDescription)")
        #expect(!text.contains("\u{FFFD}"),
                "the dead stream's half-character must not bleed into the new stream")
    }

    @Test("a stream cut mid-OSC doesn't swallow the next connection's first repaint")
    func danglingOSCPoisonCleared() {
        let (terminal, coordinator) = Self.makeTerminal()
        terminal.getTerminal().resize(cols: 40, rows: 5)

        // Title-set OSC with no terminator — the connection died inside it.
        coordinator.feed(data: Data("\u{1b}]0;标题".utf8))
        coordinator.resetInputParser()
        coordinator.feed(data: Data("hello".utf8))

        let text = rowText(terminal)
        #expect(text.contains("hello"), "got: \(text.debugDescription)")
    }

    @Test("the hazard, documented: without the reset a dangling OSC eats the fresh stream")
    func danglingOSCSwallowsWithoutReset() {
        // Pins WHY resetInputParser exists. If SwiftTerm ever learns to
        // recover a dangling OSC on its own, this starts failing and the
        // reset may have become redundant — re-evaluate, don't just delete.
        let (terminal, coordinator) = Self.makeTerminal()
        terminal.getTerminal().resize(cols: 40, rows: 5)

        coordinator.feed(data: Data("\u{1b}]0;标题".utf8))
        coordinator.feed(data: Data("hello".utf8))

        #expect(!rowText(terminal).contains("hello"),
                "printable bytes are OSC payload until a BEL/ST arrives")
    }
}
