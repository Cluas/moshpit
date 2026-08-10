import Foundation
import Testing
@testable import Moshpit

/// Tests for `AppSettings` use an isolated `UserDefaults` suite per test so
/// they never read or pollute the device's standard defaults. Each test
/// removes its persistent domain on exit to guarantee no cross-test leakage.
@Suite("AppSettings")
struct AppSettingsTests {

    /// Helper that allocates a fresh suite-name and matching `UserDefaults`.
    /// Returns the name so the caller can clean it up via `removePersistentDomain`.
    private static func makeDefaults() -> (UserDefaults, String) {
        let name = "test.appsettings.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        return (defaults, name)
    }

    @Test("default values match the prototype: fontSize=9, GitHub Dark, block teal cursor")
    func defaults() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        let settings = AppSettings(defaults: defaults)
        #expect(settings.fontSize == 9)
        #expect(settings.themeId == "github-dark")
        #expect(settings.cursorShape == .block)
        #expect(settings.cursorColorId == "teal")
        #expect(settings.cursorBlink)
        #expect(settings.trailOnPredict)
        #expect(settings.moshByDefault)
        #expect(settings.predictMode == .adaptive)
        #expect(settings.udpRangeStart == 60000)
        #expect(settings.udpRangeEnd == 61000)
    }

    @Test("set then get returns the written value on the same instance")
    func setAndGet() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        let settings = AppSettings(defaults: defaults)
        settings.fontSize = 18
        settings.themeId = "solarized-dark"
        #expect(settings.fontSize == 18)
        #expect(settings.themeId == "solarized-dark")
    }

    @Test("values persist across separate AppSettings instances backed by the same defaults")
    func persistenceAcrossInstances() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        let writer = AppSettings(defaults: defaults)
        writer.fontSize = 22
        writer.themeId = "monokai"

        let reader = AppSettings(defaults: defaults)
        #expect(reader.fontSize == 22)
        #expect(reader.themeId == "monokai")
    }

    @Test("themeId round-trips a non-default value")
    func themeIdRoundTrip() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        let settings = AppSettings(defaults: defaults)
        settings.themeId = "nord"
        #expect(settings.themeId == "nord")

        // Confirm the new value survives a fresh instance too.
        let reread = AppSettings(defaults: defaults)
        #expect(reread.themeId == "nord")
    }
}
