import SwiftTerm
import Testing
import UIKit
@testable import Moshpit

/// PlainLinkDetector against the wrap shapes a real transcript produces.
/// The hard-wrap cases are the user's report verbatim: Claude Code lays out
/// its own transcript, so a long URL arrives as two physical lines with a
/// hard newline between them — no isRowWrapped bit — and tapping the
/// underlined text opened just the first row's half of the address.
@MainActor
@Suite("Plain link wrapping")
struct PlainLinkifyTests {

    private func makeTerminal() -> Terminal {
        let view = TerminalView(frame: CGRect(x: 0, y: 0, width: 390, height: 700))
        return view.getTerminal()
    }

    private func link(_ t: Terminal, row: Int, col: Int) -> String? {
        t.link(at: .screen(Position(col: col, row: row)), mode: .explicitOnly)
    }

    @Test("a hard-wrapped URL taps as the full address on both rows")
    func hardWrapJoins() {
        let t = makeTerminal()
        let prefix = "Published "
        let head = "https://claude.ai/code/artifact/" +
            String(repeating: "a", count: t.cols - prefix.count - 32)
        #expect((prefix + head).count == t.cols, "test line must end flush at the right edge")
        // The renderer indented the continuation, as Claude Code does in
        // tool-result blocks.
        let tail = "4780-408d-8ebc"
        t.feed(text: prefix + head + "\r\n  " + tail + " done\r\n")
        PlainLinkDetector.linkify(terminal: t)

        let full = head + tail
        #expect(link(t, row: 0, col: prefix.count) == full,
                "first row must carry the JOINED url, not its own half")
        #expect(link(t, row: 1, col: 2) == full,
                "the continuation row's cells must be tappable too")
        #expect(link(t, row: 1, col: 2 + tail.count + 1) == nil,
                "the prose after the tail must not be part of the link")
    }

    @Test("a CJK-prefixed hard-wrapped URL tags the correct columns")
    func cjkPrefixHardWrap() {
        let t = makeTerminal()
        // The report's exact shape: Claude Code's transcript line, wide
        // characters before the link. Measure the prefix in COLUMNS with the
        // emulator itself — guessing east-asian widths in the test would
        // re-introduce the very drift this pins against.
        let prefix = "⏺ 产品文档已发布：⧉ "
        let probe = makeTerminal()
        probe.feed(text: prefix)
        let prefixCols = probe.buffer.x
        let full = "https://claude.ai/code/artifact/40373e79-4780-408d-8ebc-b183e6c374e6"
        let headLen = t.cols - prefixCols
        let head = String(full.prefix(headLen))
        let tail = String(full.dropFirst(headLen))
        t.feed(text: prefix + head + "\r\n" + tail + "\r\n")
        PlainLinkDetector.linkify(terminal: t)

        #expect(link(t, row: 0, col: prefixCols) == full,
                "the url's first column must be tagged despite wide chars before it")
        #expect(link(t, row: 0, col: t.cols - 1) == full,
                "the url's last column on the first row must be tagged")
        #expect(link(t, row: 0, col: prefixCols - 2) == nil,
                "the CJK prefix itself must not be part of the link")
        #expect(link(t, row: 1, col: 0) == full,
                "the continuation row must carry the joined url")
    }

    @Test("a complete URL ending at the right edge is not extended into prose")
    func edgeURLBeforeProseUntouched() {
        let t = makeTerminal()
        let url = "https://e.example/p/" + String(repeating: "b", count: t.cols - 20)
        #expect(url.count == t.cols)
        t.feed(text: url + "\r\nThe rest is prose\r\n")
        PlainLinkDetector.linkify(terminal: t)

        #expect(link(t, row: 0, col: 0) == url,
                "a url that legitimately fills the row must keep working unmodified")
        #expect(link(t, row: 1, col: 0) == nil,
                "prose must never be pulled into a link")
    }

    @Test("an emulator-soft-wrapped URL still joins (the existing path)")
    func softWrapStillJoins() {
        let t = makeTerminal()
        let url = "https://claude.ai/code/artifact/" + String(repeating: "c", count: t.cols)
        t.feed(text: url + "\r\n")
        PlainLinkDetector.linkify(terminal: t)

        #expect(link(t, row: 0, col: 0) == url)
        #expect(link(t, row: 1, col: 0) == url,
                "the wrapped second row is the same link")
    }
}
