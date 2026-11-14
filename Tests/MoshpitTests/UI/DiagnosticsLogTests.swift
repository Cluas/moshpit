import Foundation
import OSLog
import Testing
@testable import Moshpit
import MoshpitKit

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

    @Test("the clock is a fixed format: en_US_POSIX, in the phone's time zone (B128)")
    func clockIsFixed() throws {
        // In the user's locale the system may rewrite a fixed format (the
        // 12/24-hour setting, other digits): copied lines read differently
        // on every phone.
        let clock = DiagnosticsLog.clock
        #expect(clock.locale.identifier == "en_US_POSIX")
        #expect(clock.timeZone == TimeZone.current)
        let text = clock.string(from: Date(timeIntervalSince1970: 1_731_300_000.25))
        #expect(text.wholeMatch(of: /[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}/) != nil, "\(text)")
    }

    @Test("a log that can't be read is said in the user's language (B127)",
          arguments: ["zh-Hans", "zh-Hant", "ja", "ko", "de", "es", "fr", "pt-BR"])
    func readFailureIsLocalized(language: String) throws {
        // Both pages showed "Couldn't read the log: …" built in English and
        // in no catalog, in every language.
        let path = try #require(Bundle.main.path(forResource: language, ofType: "lproj"),
                                "no \(language).lproj in the app")
        let bundle = try #require(Bundle(path: path))
        let key = "Couldn't read the log: %@"
        #expect(DiagnosticsLog.readFailureKey == key)
        #expect(bundle.localizedString(forKey: key, value: nil, table: nil) != key,
                "no \(language) text for the read failure")
        let error = NSError(domain: "test", code: 1, userInfo: [NSLocalizedDescriptionKey: "denied"])
        let text = DiagnosticsLog.readFailure(error, bundle: bundle)
        #expect(text.contains("denied"))
        #expect(!text.hasPrefix("Couldn't read the log"), "\(text)")
    }
}
