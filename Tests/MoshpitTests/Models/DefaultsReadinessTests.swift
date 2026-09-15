import Foundation
import Testing
@testable import Moshpit

/// The gate every UserDefaults-backed store reads through. The truth table is
/// small enough to pin whole; what each store does with the answer is in its
/// own suite (ConnectionStoreTests, ShortcutStoreSealedLaunchTests) — and the
/// stores without a suite of their own get their sealed launch here.
@Suite("DefaultsReadiness")
struct DefaultsReadinessTests {

    private static func makeDefaults() -> (UserDefaults, String) {
        let name = "test.readiness.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        return (defaults, name)
    }

    private final class Phone { var unlocked = false }

    @Test("unlocked is readable; locked without the sentinel is not")
    func truthTable() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let phone = Phone()
        let readiness = DefaultsReadiness { phone.unlocked }

        #expect(!readiness.isReadable(defaults))
        phone.unlocked = true
        #expect(readiness.isReadable(defaults))
    }

    @Test("the sentinel outranks the lock: a file once read stays readable")
    func sentinelOutranksLock() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let readiness = DefaultsReadiness { false }

        readiness.markReadable(defaults)
        #expect(readiness.isReadable(defaults))
        #expect(defaults.bool(forKey: DefaultsReadiness.sentinelKey))
    }

    // Each store below: sealed at launch (empty suite, phone locked), shown
    // empty, written over by nothing, and re-read once the file is there.

    @Test("SSH keys survive a sealed launch")
    func sshKeysSealedLaunch() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let (scratch, scratchName) = Self.makeDefaults()
        defer { scratch.removePersistentDomain(forName: scratchName) }
        let key = "moshpit.sshkeys.v1"
        SSHKeyStore(defaults: scratch).add(SSHKeyRecord(name: "laptop", algorithm: .ed25519, source: .imported))
        let payload = try #require(scratch.data(forKey: key))

        let phone = Phone()
        let store = SSHKeyStore(defaults: defaults, readiness: DefaultsReadiness { phone.unlocked })
        #expect(store.keys.isEmpty)
        #expect(!store.isAuthoritative)
        store.add(SSHKeyRecord(name: "phone", algorithm: .seP256, source: .secureEnclave))
        #expect(defaults.data(forKey: key) == nil, "nothing is written over a sealed file")

        defaults.set(payload, forKey: key)
        phone.unlocked = true
        store.reloadIfNeeded()
        #expect(store.keys.map(\.name) == ["laptop"])
        #expect(store.isAuthoritative)
    }

    @Test("custom terminal themes survive a sealed launch")
    func terminalThemesSealedLaunch() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let (scratch, scratchName) = Self.makeDefaults()
        defer { scratch.removePersistentDomain(forName: scratchName) }
        let key = "moshpit.settings.customThemes"
        let writer = ThemeStore(defaults: scratch)
        writer.save(writer.duplicate(.fallback))
        let payload = try #require(scratch.data(forKey: key))

        let phone = Phone()
        let store = ThemeStore(defaults: defaults, readiness: DefaultsReadiness { phone.unlocked })
        #expect(store.customThemes.isEmpty)
        store.save(store.makeDraft())
        #expect(defaults.data(forKey: key) == nil, "nothing is written over a sealed file")

        defaults.set(payload, forKey: key)
        phone.unlocked = true
        store.reloadIfNeeded()
        #expect(store.customThemes.count == 1)
        #expect(store.isAuthoritative)
    }

    @Test("custom app themes survive a sealed launch")
    func appThemesSealedLaunch() throws {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let (scratch, scratchName) = Self.makeDefaults()
        defer { scratch.removePersistentDomain(forName: scratchName) }
        let key = "moshpit.settings.customAppThemes"
        AppThemeStore(defaults: scratch).save(AppTheme.custom(id: "app-1", name: "Mine", accentHex: "3366FF"))
        let payload = try #require(scratch.data(forKey: key))

        let phone = Phone()
        let store = AppThemeStore(defaults: defaults, readiness: DefaultsReadiness { phone.unlocked })
        #expect(store.customThemes.isEmpty)
        store.save(AppTheme.custom(id: "app-2", name: "Other", accentHex: "FF3366"))
        #expect(defaults.data(forKey: key) == nil, "nothing is written over a sealed file")

        defaults.set(payload, forKey: key)
        phone.unlocked = true
        store.reloadIfNeeded()
        #expect(store.customThemes.map(\.id) == ["app-1"])
        #expect(store.isAuthoritative)
    }

    @Test("settings hold the App Group mirror until the defaults can be trusted")
    func settingsMirrorWaits() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let phone = Phone()

        let settings = AppSettings(defaults: defaults, readiness: DefaultsReadiness { phone.unlocked })
        #expect(defaults.object(forKey: DefaultsReadiness.sentinelKey) == nil,
                "a sealed launch is not a trusted read")
        phone.unlocked = true
        settings.reloadIfNeeded()
        #expect(defaults.bool(forKey: DefaultsReadiness.sentinelKey))
    }
}
