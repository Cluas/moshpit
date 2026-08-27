import Foundation
import Testing
@testable import Moshpit

/// The App Group mirror that lets the notification service extension honor the
/// two switches a pushed notification must respect.
@Suite("Push preference mirror", .serialized)
struct PushPrefsTests {

    /// A throwaway suite so these tests never touch the real App Group.
    private func withSuite<T>(_ body: (String) throws -> T) rethrows -> T {
        let name = "moshpit.tests.pushprefs"
        defer { UserDefaults(suiteName: name)?.removePersistentDomain(forName: name) }
        UserDefaults(suiteName: name)?.removePersistentDomain(forName: name)
        return try body(name)
    }

    @Test("an unwritten mirror reads as both switches on — the settings' own defaults")
    func defaultsAreOn() {
        withSuite { suite in
            let values = PushPrefs.read(suiteName: suite)
            #expect(values.showDetail)
            #expect(values.sound)
        }
    }

    @Test("what the app writes is what the extension reads")
    func roundTrip() {
        withSuite { suite in
            PushPrefs.write(showDetail: false, sound: true, suiteName: suite)
            #expect(PushPrefs.read(suiteName: suite) == .init(showDetail: false, sound: true))
            PushPrefs.write(showDetail: true, sound: false, suiteName: suite)
            #expect(PushPrefs.read(suiteName: suite) == .init(showDetail: true, sound: false))
        }
    }
}
