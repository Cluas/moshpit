import Foundation
import Testing
@testable import Moshpit
import MoshpitKit

/// The app records the language it shows in the App Group at launch
/// (AppLanguage). That is a write into preferences sealed until the first
/// unlock, and the sealed-launch rule holds: nothing is written while the
/// defaults can't be trusted (DefaultsReadiness).
@Suite("The app's language is recorded once the defaults can be trusted")
@MainActor
struct AppLanguageRecordTests {

    @Test("a launch before the first unlock records nothing; the first trusted look does")
    func sealedLaunchRecordsNothing() throws {
        let suite = "test.applanguage.group.\(UUID().uuidString)"
        let own = "test.applanguage.app.\(UUID().uuidString)"
        let group = try #require(UserDefaults(suiteName: suite))
        let app = try #require(UserDefaults(suiteName: own))
        defer {
            group.removePersistentDomain(forName: suite)
            app.removePersistentDomain(forName: own)
        }
        // Sealed: protected data unavailable, and no trusted load has left
        // its sentinel in the app's defaults.
        let sealed = DefaultsReadiness { false }
        #expect(!AppLanguage.recordIfTrusted(readiness: sealed, defaults: app, language: "en", suiteName: suite))
        #expect(group.object(forKey: AppLanguage.key) == nil,
                "written into the App Group while its file could still be sealed")

        // The device unlocked (or a trusted load's sentinel is there).
        let open = DefaultsReadiness { true }
        #expect(AppLanguage.recordIfTrusted(readiness: open, defaults: app, language: "en", suiteName: suite))
        #expect(AppLanguage.stored(suiteName: suite) == "en")
    }
}
