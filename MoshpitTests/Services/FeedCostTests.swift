import Foundation
import Testing
import SwiftTerm
@testable import Moshpit

/// What a backfill costs the main actor, as a number rather than an intuition.
///
/// Chunking the feed — parse a big dump in slices with a yield between them so
/// the first screen paints early — was on the optimisation list and was NOT
/// done. This suite is why. Measured on the simulator with dense content (SGR
/// colour, CJK, box-drawing — the shapes that cost a terminal parser most):
///
///   400 lines  (33 KB): ~26 ms
///   2000 lines (166 KB): ~123 ms
///
/// Cutting `backfillHistoryLines` from 2 000 to 400 therefore took the per-pane
/// stall from ~123 ms to ~26 ms on its own. What remained was not worth a queue:
/// each pane's dump arrives as its own control-mode reply and is fed in its own
/// main-actor turn, so four panes are four 26 ms turns, not one 104 ms freeze —
/// and the machinery to chunk them would have gone into the most
/// ordering-sensitive path in the app, where every past bug has been an ordering
/// bug.
///
/// The assertion below is the guard rail for that decision: raise the history
/// depth and this says what it now costs, before a user feels it.
@MainActor
@Suite("Feed cost")
struct FeedCostTests {

    /// The budget a single pane's backfill has on the main actor. Two frames at
    /// 60 Hz is 33 ms; past that a switch stops feeling instant.
    static let budgetMilliseconds = 60.0

    private func denseLine(_ i: Int) -> String {
        "\u{1b}[32m\(i)\u{1b}[0m | ASCII=abcXYZ | CJK=你好世界 | box ┌──┬──┐ │AB│"
    }

    private func feedMilliseconds(lines count: Int) -> Double {
        let term = Terminal(delegate: SilentTerminalDelegate())
        term.resize(cols: 80, rows: 50)
        let bytes = [UInt8]((1...count).map(denseLine).joined(separator: "\r\n").utf8)
        let start = ContinuousClock.now
        term.feed(byteArray: bytes)
        return Double((ContinuousClock.now - start).components.attoseconds) / 1e15
    }

    @Test("a backfill-sized dump stays inside its main-actor budget")
    func backfillFeedIsAffordable() {
        // 400 is `TmuxSessionController.backfillHistoryLines`. It is duplicated
        // here rather than read, because the constant is private and because the
        // point of this test is to fail loudly when someone changes it — a
        // silently-tracking test would raise its own bar and notice nothing.
        let ms = feedMilliseconds(lines: 400)
        let note = "400 lines took \(String(format: "%.1f", ms)) ms on the main actor — "
            + "if backfillHistoryLines went up, this is what it now costs a pane switch"
        #expect(ms < Self.budgetMilliseconds, "\(note)")
    }

    @Test("the cost is roughly linear, so the depth is the dial that matters")
    func costScalesWithDepth() {
        // Establishes that trimming the DEPTH is the lever, not some fixed
        // per-feed overhead a chunking queue could have amortised away.
        let small = feedMilliseconds(lines: 200)
        let large = feedMilliseconds(lines: 800)
        let note = "800 lines (\(String(format: "%.1f", large)) ms) is not meaningfully "
            + "dearer than 200 (\(String(format: "%.1f", small)) ms) — the cost model assumed here is wrong"
        #expect(large > small * 2, "\(note)")
    }
}

/// A delegate that does nothing: these tests measure the parser, not the app.
final class SilentTerminalDelegate: TerminalDelegate {
    func send(source: Terminal, data: ArraySlice<UInt8>) {}
}
