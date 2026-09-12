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

    @Test("the client-side scrollback budget is 50k lines")
    func budget() {
        #expect(TerminalScrollback.lines == 50_000)
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
}
