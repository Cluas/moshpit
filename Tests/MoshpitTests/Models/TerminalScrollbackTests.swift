import Foundation
import Testing
import UIKit
import SwiftTerm
@testable import Moshpit

/// `TerminalScrollback` is a thin policy shim over SwiftTerm: a fixed line
/// budget plus an `enlarge` step that bumps a freshly-built terminal's
/// scrollback before any data is fed. The budget is a pure constant; `enlarge`
/// needs a real `TerminalView`, which UIKit requires on the main actor.
@Suite("TerminalScrollback")
struct TerminalScrollbackTests {

    @Test("the client-side scrollback budget is 10k lines (~19 MB for a full 80-column pane)")
    func budget() {
        #expect(TerminalScrollback.lines == 10_000)
    }

    @MainActor
    private func makeTerminalView() -> TerminalView {
        TerminalView(frame: .zero,
                     font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular))
    }

    @Test("enlarge raises a freshly-built terminal from the 500-line default to the budget")
    @MainActor
    func enlargeBumpsScrollback() {
        let view = makeTerminalView()
        // SwiftTerm ships a 500-line default; anything below the budget qualifies.
        #expect(view.getTerminal().options.scrollback < TerminalScrollback.lines)

        TerminalScrollback.enlarge(view)
        #expect(view.getTerminal().options.scrollback == TerminalScrollback.lines)
    }

    @Test("enlarge is idempotent — a second call leaves the budget untouched")
    @MainActor
    func enlargeIdempotent() {
        let view = makeTerminalView()
        TerminalScrollback.enlarge(view)
        TerminalScrollback.enlarge(view)
        #expect(view.getTerminal().options.scrollback == TerminalScrollback.lines)
    }

    /// The option is not the history. `enlarge` used to set the option and
    /// re-run `setup(isReset: false)`, which only resizes the buffer SwiftTerm
    /// already built — at 500 lines. So the option said 50 000 and the pane
    /// kept 500 lines plus its screen: count what the buffer holds.
    @Test("an enlarged terminal keeps far more than SwiftTerm's 500 lines of history")
    @MainActor
    func enlargedTerminalKeepsTheHistory() {
        let view = TerminalView(frame: CGRect(x: 0, y: 0, width: 390, height: 300),
                                font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular))
        TerminalScrollback.enlarge(view)
        let fed = 3000
        for i in 0 ..< fed {
            view.feed(text: "line \(i)\r\n")
        }

        // One row per line fed, and the cursor's row under them.
        let text = String(decoding: view.getTerminal().getBufferAsData(kind: .normal), as: UTF8.self)
        let kept = text.split(separator: "\n", omittingEmptySubsequences: false).dropLast()
        #expect(kept.count == fed + 1, "kept \(kept.count) rows of \(fed) lines fed")
        #expect(kept.first == "line 0", "the oldest line kept is \(kept.first.map(String.init) ?? "nothing")")
    }
}
