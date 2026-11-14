import Testing
@testable import Moshpit

/// The client-side history every terminal keeps (the core's view is made
/// with it: `CoreTerminalView(frame:font:scrollback:)`).
@Suite("TerminalScrollback")
struct TerminalScrollbackTests {
    @Test("the client-side scrollback budget is 10k lines (~19 MB for a full 80-column pane)")
    func budget() {
        #expect(TerminalScrollback.lines == 10_000)
    }
}
