import Foundation
import Testing
@testable import Moshpit

/// `WriteLane.wait(atMost:)`, the budgeted wait voice dictation uses. (The
/// lane's ordered queue went with the Swift transports: the core orders the
/// terminal's bytes.)
@Suite("write lane")
@MainActor
struct WriteLaneTests {

    @Test("wait(atMost:) says whether the operation beat the budget")
    func waitReportsTheWinner() async {
        let quick = await WriteLane.wait(atMost: .seconds(5)) {}
        #expect(quick)

        let slow = await WriteLane.wait(atMost: .milliseconds(20)) {
            try? await Task.sleep(for: .milliseconds(300))
        }
        #expect(!slow)
    }
}
