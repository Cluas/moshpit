import Foundation
import Testing
@testable import Moshpit

/// The gate every UserDefaults-backed store reads through: the core's
/// readiness rule (crates/store readiness.rs, whose tests pin its table),
/// read here through the app's preferences host, the way the stores and
/// the agents monitor read it. What each store does with the answer is in
/// its own suite (ConnectionStoreTests, ShortcutStoreSealedLaunchTests);
/// the stores without a suite of their own get their sealed launch here.
@Suite("Sealed launch")
struct SealedLaunchTests {

    private static func makeDefaults() -> (UserDefaults, String) {
        let name = "test.readiness.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        return (defaults, name)
    }

    private final class Phone { var unlocked = false }

    private static func readable(_ defaults: UserDefaults, _ protectedData: @escaping () -> Bool) -> Bool {
        storeDefaultsReadable(host: RustPreferencesHost(defaults: defaults, available: protectedData), mode: .system)
    }

    @Test("unlocked is readable; locked without the sentinel is not")
    func truthTable() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let phone = Phone()

        #expect(!Self.readable(defaults) { phone.unlocked })
        phone.unlocked = true
        #expect(Self.readable(defaults) { phone.unlocked })
    }

    @Test("the sentinel outranks the lock: a file once read stays readable")
    func sentinelOutranksLock() {
        let (defaults, name) = Self.makeDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        // What the store writes after its first trusted read (crates/store).
        defaults.set(true, forKey: storeDefaultsSentinelKey())
        #expect(Self.readable(defaults) { false })
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
        let store = SSHKeyStore(defaults: defaults, protectedData: { phone.unlocked })
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
        let store = ThemeStore(defaults: defaults, protectedData: { phone.unlocked })
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
        let store = AppThemeStore(defaults: defaults, protectedData: { phone.unlocked })
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

        let settings = AppSettings(defaults: defaults, protectedData: { phone.unlocked })
        #expect(defaults.object(forKey: storeDefaultsSentinelKey()) == nil,
                "a sealed launch is not a trusted read")
        phone.unlocked = true
        settings.reloadIfNeeded()
        #expect(defaults.bool(forKey: storeDefaultsSentinelKey()))
    }
}
