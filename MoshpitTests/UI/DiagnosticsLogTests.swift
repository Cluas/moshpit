import Foundation
import OSLog
import Testing
@testable import Moshpit

/// The diagnostics screen is only worth having if the store actually hands
/// back what the app wrote. That is the part that can fail — a wrong
/// subsystem, a level the system declines to persist, a scope that sees
/// nothing — and it fails silently, as an empty list that looks like "nothing
/// happened" rather than "this does not work".
@Suite("diagnostics log")
struct DiagnosticsLogTests {

    @Test("The app can read back its own log entries")
    func readsOwnEntries() async throws {
        let marker = "diagnostics-selftest-\(UUID().uuidString.prefix(8))"
        Log.ssh.notice("\(marker, privacy: .public)")

        // The store is written asynchronously; give it a moment to land.
        var found = false
        for _ in 0..<10 {
            try? await Task.sleep(for: .milliseconds(200))
            let store = try OSLogStore(scope: .currentProcessIdentifier)
            let since = store.position(date: Date().addingTimeInterval(-60))
            let entries = try store.getEntries(
                at: since, matching: NSPredicate(format: "subsystem == %@", "com.cluas.moshpit"))
            if entries.compactMap({ $0 as? OSLogEntryLog })
                .contains(where: { $0.composedMessage.contains(marker) }) {
                found = true
                break
            }
        }
        #expect(found, "the diagnostics screen reads this store — if this is empty, it shows nothing")
    }
}
