import Foundation
import MoshpitKit
import Testing
@testable import Moshpit

/// The app's stores on the Rust store (the `MOSHPIT_RUST_STORE` switch,
/// Services/Rust/RustStores.swift) against the same store classes on their
/// own Swift code, on this device's Foundation:
///
/// - the same operations through each, on UserDefaults suites of their own,
///   leave the same suites (every JSON list compared as JSON, ids by where
///   they first appear) and the same lists on screen;
/// - switching on over what the Swift stores saved, using the app, switching
///   off: the Swift stores read everything, as the adapters left it;
/// - a launch before the first unlock (protected data unavailable, no
///   sentinel) writes nothing, whatever the user does, until the unlock;
/// - the trusted host keys and the push pairings, each side reading the
///   other's.
///
/// Every suite, keychain and file here is the test's own: nothing the app
/// keeps is read or written (no settings mirror into the App Group: the
/// Swift settings here never mirror, the Rust ones mirror into a suite of
/// the test's). The 1.0.8 read of what the Rust side writes is the Mac
/// harness's (crates/store/harness, `equivalence.py`, phase R).
@Suite("Rust store adapters")
@MainActor
struct RustStoreAdapterTests {

    // MARK: - Fixtures

    /// A UserDefaults suite of the test's own, removed by `drop()`.
    final class Scratch {
        let name = "w5b.rust-store.\(UUID().uuidString)"
        lazy var defaults = UserDefaults(suiteName: name)!
        var contents: [String: Any] { defaults.persistentDomain(forName: name) ?? [:] }
        func drop() { defaults.removePersistentDomain(forName: name) }
    }

    /// The app's preferences host over a test suite, with protected data
    /// available or not as the test says.
    final class Prefs: PreferencesHost, @unchecked Sendable {
        let inner: RustPreferencesHost
        var available: Bool
        init(_ defaults: UserDefaults, available: Bool = true) {
            inner = RustPreferencesHost(defaults: defaults, protectedData: ProtectedDataFlag(fixed: true))
            self.available = available
        }
        func get(key: String) -> PlistValue? { inner.get(key: key) }
        func set(key: String, value: PlistValue?) { inner.set(key: key, value: value) }
        func protectedDataAvailable() -> Bool { available }
    }

    static let ids = (0..<8).map { UUID(uuidString: String(format: "0F0E0D0C-0000-4000-8000-%012d", $0))! }

    /// A suite's contents as values to compare: each stored JSON list
    /// parsed and written again with sorted keys (the member order is the
    /// encoder's, and Swift's changes per process), each UUID and minted
    /// theme id by the order it first appears in (built-ins seed fresh ids
    /// on each side).
    /// Fields that are Swift `Set`s, written in hash order — which Swift
    /// seeds afresh in every process (the store harness compares them as
    /// sets too): sorted, so two writers of the same set compare equal.
    static let setFields: Set<String> = ["modifiers", "scopeHosts"]

    static func sortingSets(_ value: Any) -> Any {
        if let object = value as? [String: Any] {
            var sorted: [String: Any] = [:]
            for (key, field) in object {
                if setFields.contains(key), let strings = field as? [String] {
                    sorted[key] = strings.sorted()
                } else {
                    sorted[key] = sortingSets(field)
                }
            }
            return sorted
        }
        if let array = value as? [Any] { return array.map(sortingSets) }
        return value
    }

    static func canonical(_ contents: [String: Any]) -> [String: String] {
        var seen: [String: Int] = [:]
        func mask(_ text: String) -> String {
            let pattern = #"[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}|(custom|app)-[0-9a-f]{8}"#
            let regex = try! NSRegularExpression(pattern: pattern)
            var result = ""
            var last = text.startIndex
            for match in regex.matches(in: text, range: NSRange(text.startIndex..., in: text)) {
                let range = Range(match.range, in: text)!
                let id = text[range].uppercased()
                // The test's own fixed ids stay as they are.
                let fixed = ids.contains { $0.uuidString == id } || id.hasPrefix("CUSTOM-TEST") || id.hasPrefix("APP-TEST")
                result += text[last..<range.lowerBound]
                result += fixed ? String(text[range]) : "#\(seen[id] ?? { seen[id] = seen.count; return seen.count - 1 }())"
                last = range.upperBound
            }
            return result + text[last...]
        }
        func describe(_ value: Any) -> String {
            if let data = value as? Data {
                if let json = try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed]),
                   let again = try? JSONSerialization.data(withJSONObject: sortingSets(json),
                                                           options: [.sortedKeys, .fragmentsAllowed]) {
                    return "json:" + mask(String(decoding: again, as: UTF8.self))
                }
                return "data:" + data.base64EncodedString()
            }
            if let array = value as? [Any] { return "[" + array.map(describe).joined(separator: ",") + "]" }
            if let number = value as? NSNumber {
                if CFGetTypeID(number) == CFBooleanGetTypeID() { return "bool:\(number.boolValue)" }
                return CFNumberIsFloatType(number) ? "double:\(number.doubleValue)" : "int:\(number.int64Value)"
            }
            return "\(type(of: value)):" + mask("\(value)")
        }
        var out: [String: String] = [:]
        for key in contents.keys.sorted() { out[key] = describe(contents[key]!) }
        return out
    }

    // MARK: - Same operations, same suites

    @Test("connections: the same operations leave the same suite and list")
    func connections() {
        let swift = Scratch(), rust = Scratch()
        defer { swift.drop(); rust.drop() }
        let a = ConnectionStore(defaults: swift.defaults)
        let b = ConnectionStore(backing: RustConnectionStore(preferences: Prefs(rust.defaults), mode: .system))
        for store in [a, b] {
            var pi = ServerConnection(id: Self.ids[0], name: "pi", host: "pi.local", port: 2222, username: "me",
                                      authMethod: .key, connectionProtocol: .mosh,
                                      createdAt: Date(timeIntervalSinceReferenceDate: 784_000_000.75))
            pi.multiplexer = .tmux
            pi.sshKeyId = Self.ids[5]
            pi.sshKeyAlgorithmRaw = "ED25519"
            pi.predictMode = .never
            store.add(pi)
            store.add(ServerConnection(id: Self.ids[1], name: "\u{FEFF}büro · 研究所", host: "::1",
                                       keychainRef: "moshpit.secret.x",
                                       createdAt: Date(timeIntervalSinceReferenceDate: 1)))
            pi.name = "pi 2"
            pi.lastConnectedAt = Date(timeIntervalSinceReferenceDate: 784_000_100)
            store.update(pi)
            store.delete(id: Self.ids[1])
            var nas = ServerConnection(id: Self.ids[2], host: "nas", username: "root",
                                       createdAt: Date(timeIntervalSinceReferenceDate: 784_000_000))
            nas.useSOCKSProxy = true
            nas.socksProxyHost = "127.0.0.1"
            nas.socksProxyPort = 1080
            nas.keychainRef = "moshpit.secret.nas"
            store.add(nas)
        }
        #expect(a.connections == b.connections)
        #expect(a.isAuthoritative && b.isAuthoritative)
        #expect(a.savedIds == b.savedIds)
        #expect(a.keychainRefs() == b.keychainRefs())
        #expect(a.keychainRefs(excluding: Self.ids[2]) == b.keychainRefs(excluding: Self.ids[2]))
        #expect(a.names(usingKey: Self.ids[5], ref: nil) == ["pi 2"])
        #expect(b.names(usingKey: Self.ids[5], ref: nil) == ["pi 2"])
        #expect(Self.canonical(swift.contents) == Self.canonical(rust.contents))
        // Both read back as each other.
        #expect(ConnectionStore(defaults: rust.defaults).connections.map(\.id) == a.connections.map(\.id))
    }

    @Test("SSH key records: the same operations leave the same suite and list")
    func keys() {
        let swift = Scratch(), rust = Scratch()
        defer { swift.drop(); rust.drop() }
        let a = SSHKeyStore(defaults: swift.defaults)
        let b = SSHKeyStore(backing: RustSSHKeyStore(preferences: Prefs(rust.defaults), mode: .system))
        for store in [a, b] {
            store.add(SSHKeyRecord(id: Self.ids[0], name: "laptop", algorithm: .ed25519, fingerprint: "SHA256:a",
                                   publicKey: "ssh-ed25519 AAAA", source: .generated, keychainRef: "moshpit.secret.k0",
                                   createdAt: Date(timeIntervalSinceReferenceDate: 784_000_000.5)))
            store.add(SSHKeyRecord(id: Self.ids[1], name: "rsa", algorithm: .rsa4096, bits: 3072, comment: "c",
                                   source: .imported, boundHosts: ["a", "b"], keychainRef: "moshpit.secret.k1",
                                   requireBiometry: false, createdAt: Date(timeIntervalSinceReferenceDate: 2),
                                   lastUsedAt: Date(timeIntervalSinceReferenceDate: 3)))
            store.add(SSHKeyRecord(id: Self.ids[2], name: "ec", algorithm: .ecdsa,
                                   publicKey: "ecdsa-sha2-nistp256 AAAA", source: .imported,
                                   createdAt: Date(timeIntervalSinceReferenceDate: 4)))
            store.add(SSHKeyRecord(id: Self.ids[3], name: "se", algorithm: .seP256, source: .secureEnclave,
                                   createdAt: Date(timeIntervalSinceReferenceDate: 5)))
            store.remove(id: Self.ids[0])
        }
        #expect(a.keys == b.keys)
        #expect(a.deviceKeys == b.deviceKeys && a.importedKeys == b.importedKeys)
        #expect(a.keychainRefs() == b.keychainRefs())
        #expect(a.keychainRefs(excluding: Self.ids[1]) == b.keychainRefs(excluding: Self.ids[1]))
        #expect(Self.canonical(swift.contents) == Self.canonical(rust.contents))
    }

    @Test("shortcuts: the same operations leave the same suite and bar")
    func shortcuts() {
        let swift = Scratch(), rust = Scratch()
        defer { swift.drop(); rust.drop() }
        let a = ShortcutStore(defaults: swift.defaults)
        let b = ShortcutStore(backing: RustShortcutStore(preferences: Prefs(rust.defaults), mode: .system))
        for store in [a, b] {
            func builtin(_ summary: String) -> UUID { store.shortcuts.first { $0.summary == summary }!.id }
            var git = TerminalShortcut(id: Self.ids[0], kind: .command, payload: "git status", chipLabel: "gst",
                                       summary: "Git status", colorId: "accent", scopeHosts: ["pi", "nas"])
            store.add(git)
            store.add(TerminalShortcut(id: Self.ids[1], kind: .keyCombo, modifiers: [.ctrl, .shift], key: "t",
                                       chipLabel: "^⇧T", summary: "New tab", onlyInTmux: true))
            #expect(store.addToToolbar(id: builtin("Clear screen")))
            store.removeFromToolbar(id: builtin("Escape"))
            store.moveInToolbar(fromOffsets: IndexSet([0, 2]), toOffset: 4)
            git.chipLabel = "gs"
            store.update(git)
            store.remove(id: Self.ids[1])
            store.remove(id: builtin("Paste clipboard"))
        }
        #expect(a.shortcuts.count == b.shortcuts.count)
        func shape(_ s: ShortcutStore) -> [String] {
            s.shortcuts.map { [$0.summary, $0.kind.rawValue, $0.chipLabel, "\($0.inToolbar)", $0.payload].joined(separator: "/") }
        }
        #expect(shape(a) == shape(b))
        #expect(a.toolbar.map(\.summary) == b.toolbar.map(\.summary))
        #expect(a.toolbar(forHost: "pi", inMultiplexer: false).map(\.summary)
                == b.toolbar(forHost: "pi", inMultiplexer: false).map(\.summary))
        #expect(a.shortcuts.first { $0.id == Self.ids[0] } == b.shortcuts.first { $0.id == Self.ids[0] })
        #expect(Self.canonical(swift.contents) == Self.canonical(rust.contents))
    }

    @Test("themes and accents: the same operations leave the same suites")
    func themes() throws {
        let swift = Scratch(), rust = Scratch()
        defer { swift.drop(); rust.drop() }
        let a = ThemeStore(defaults: swift.defaults)
        let b = ThemeStore(backing: RustThemeStore(preferences: Prefs(rust.defaults), mode: .system,
                                                   untitled: String(localized: "Untitled")))
        for store in [a, b] {
            var mine = TerminalTheme.fallback
            mine.id = "custom-test0001"
            mine.isBuiltIn = false
            mine.name = "Mine"
            store.save(mine)
            mine.id = "custom-test0002"
            store.save(mine)        // the same name: "Mine 2"
            mine.id = "custom-test0003"
            mine.name = "  "
            store.save(mine)        // no name: "Untitled"
            store.save(TerminalTheme.builtIns[1])   // refused
            store.delete(id: "custom-test0001")
            _ = try store.importThemes(json: ##"{"name":"Pasted","background":"#101010","foreground":"#eeeeee","cursor":"#ff0000","red":"#c00"}"##)
        }
        #expect(a.customThemes.map(\.name) == b.customThemes.map(\.name))
        #expect(a.customThemes.dropLast() == b.customThemes.dropLast(), "the imported one has a minted id")
        #expect(throws: ThemeStore.ImportError.self) { try b.importThemes(json: "nope") }
        #expect(try a.exportJSON(a.customThemes[0]) == b.exportJSON(b.customThemes[0]))
        #expect(Self.canonical(swift.contents) == Self.canonical(rust.contents))

        let sa = Scratch(), ra = Scratch()
        defer { sa.drop(); ra.drop() }
        let x = AppThemeStore(defaults: sa.defaults)
        let y = AppThemeStore(backing: RustAppThemeStore(preferences: Prefs(ra.defaults), mode: .system,
                                                         untitled: String(localized: "Untitled")))
        for store in [x, y] {
            store.save(.custom(id: "app-test0001", name: "Lime", accentHex: "#A0F000"))
            store.save(.custom(id: "app-test0002", name: "Lime", accentHex: "ff8800"))
            store.save(.custom(id: "app-test0001", name: "Lemon", accentHex: "#A0F000"))
            store.save(AppThemeCatalog.builtIns[2])   // refused
            store.save(.custom(id: "app-test0003", name: "gone", accentHex: "000000"))
            store.delete(id: "app-test0003")
        }
        #expect(x.customThemes.map { [$0.id, $0.name, $0.accentHex] } == y.customThemes.map { [$0.id, $0.name, $0.accentHex] })
        #expect(Self.canonical(sa.contents) == Self.canonical(ra.contents))
    }

    @Test("settings: the same writes leave the same suite and read the same")
    func settings() {
        let swift = Scratch(), rust = Scratch(), mirror = Scratch()
        defer { swift.drop(); rust.drop(); mirror.drop() }
        // Not readable, no sentinel: neither side mirrors at launch — the
        // Swift side's mirror is the App Group's, which this test leaves be.
        let a = AppSettings(defaults: swift.defaults, readiness: DefaultsReadiness { false })
        let b = AppSettings(defaults: rust.defaults,
                            backing: RustSettings(preferences: Prefs(rust.defaults, available: false),
                                                  mirror: Prefs(mirror.defaults), mode: .system, debugBuild: false))
        for s in [a, b] {
            s.fontSize = 12.5
            s.themeId = "nord"
            s.cursorShape = .bar
            s.predictMode = .always
            s.hostSetupDeclined = ["b", "a", "研"]
            s.udpRangeStart = 60010
            s.moshByDefault = false
            s.voiceEngine = .whisper
            s.uploadRetentionDays = 0
            s.store.set("nonsense", forKey: "moshpit.settings.cursorColor")
            s.store.set(7, forKey: "moshpit.settings.fontName")   // not a String: the default
        }
        #expect(Self.canonical(swift.contents) == Self.canonical(rust.contents))
        #expect(mirror.contents.isEmpty, "nothing mirrored before a trusted read")
        for s in [a, b] {
            #expect(s.fontSize == 12.5 && s.themeId == "nord" && s.cursorShape == .bar && s.predictMode == .always)
            #expect(s.hostSetupDeclined == ["a", "b", "研"] && s.udpRangeStart == 60010 && !s.moshByDefault)
            #expect(s.voiceEngine == .whisper && s.uploadRetentionDays == 0 && s.cursorColorId == "nonsense")
            #expect(s.fontName == "system" && s.udpRangeEnd == 61000 && s.notificationsEnabled)
        }
        // The Rust side's mirror follows its two switches, into the test's
        // own suite.
        b.lockScreenDetailEnabled = false
        #expect(mirror.defaults.object(forKey: PushPrefs.detailKey) as? Bool == false)
        #expect(mirror.defaults.object(forKey: PushPrefs.soundKey) as? Bool == true)
    }

    // MARK: - Switching on and off

    @Test("switch on over the Swift stores' data, use the app, switch off: the Swift stores read it all")
    func switchOnAndOff() {
        let suite = Scratch()
        defer { suite.drop() }
        // Before: the Swift stores.
        let connections = ConnectionStore(defaults: suite.defaults)
        let keys = SSHKeyStore(defaults: suite.defaults)
        let bar = ShortcutStore(defaults: suite.defaults)
        let themes = ThemeStore(defaults: suite.defaults)
        let accents = AppThemeStore(defaults: suite.defaults)
        connections.add(ServerConnection(id: Self.ids[0], name: "pi", host: "pi",
                                         createdAt: Date(timeIntervalSinceReferenceDate: 784_000_000)))
        keys.add(SSHKeyRecord(id: Self.ids[1], name: "k", algorithm: .ed25519, source: .generated,
                              createdAt: Date(timeIntervalSinceReferenceDate: 1)))
        bar.add(TerminalShortcut(id: Self.ids[2], kind: .text, payload: "ls", chipLabel: "ls", summary: "List"))
        var theme = TerminalTheme.fallback
        theme.id = "custom-test0001"
        theme.isBuiltIn = false
        theme.name = "T"
        themes.save(theme)
        accents.save(.custom(id: "app-test0001", name: "A", accentHex: "123456"))

        // On: the adapters over the same preferences read all of it…
        let prefs = Prefs(suite.defaults)
        let rc = ConnectionStore(backing: RustConnectionStore(preferences: prefs, mode: .system))
        let rk = SSHKeyStore(backing: RustSSHKeyStore(preferences: prefs, mode: .system))
        let rb = ShortcutStore(backing: RustShortcutStore(preferences: prefs, mode: .system))
        let rt = ThemeStore(backing: RustThemeStore(preferences: prefs, mode: .system, untitled: "Untitled"))
        let ra = AppThemeStore(backing: RustAppThemeStore(preferences: prefs, mode: .system, untitled: "Untitled"))
        #expect(rc.connections == connections.connections)
        #expect(rk.keys == keys.keys)
        #expect(rb.shortcuts == bar.shortcuts)
        #expect(rt.customThemes == themes.customThemes)
        #expect(ra.customThemes.map(\.name) == accents.customThemes.map(\.name))
        // …and the app is used.
        rc.add(ServerConnection(id: Self.ids[3], name: "nas", host: "nas",
                                createdAt: Date(timeIntervalSinceReferenceDate: 784_000_001)))
        var pi = rc.connections[0]
        pi.name = "pi renamed"
        rc.update(pi)
        rk.add(SSHKeyRecord(id: Self.ids[4], name: "k2", algorithm: .rsa4096, bits: 4096, source: .imported,
                            createdAt: Date(timeIntervalSinceReferenceDate: 2)))
        rb.update({ var s = rb.shortcuts.first { $0.id == Self.ids[2] }!; s.payload = "ls -la"; return s }())
        _ = rb.addToToolbar(id: rb.shortcuts.first { $0.summary == "Return" }!.id)
        theme.name = "T2"
        rt.save(theme)
        ra.delete(id: "app-test0001")

        // Off: the Swift stores, made fresh, read everything as left.
        let after = (ConnectionStore(defaults: suite.defaults), SSHKeyStore(defaults: suite.defaults),
                     ShortcutStore(defaults: suite.defaults), ThemeStore(defaults: suite.defaults),
                     AppThemeStore(defaults: suite.defaults))
        #expect(after.0.connections == rc.connections)
        #expect(after.0.connections.map(\.name) == ["pi renamed", "nas"])
        #expect(after.1.keys == rk.keys && after.1.keys.count == 2)
        #expect(after.2.shortcuts == rb.shortcuts)
        #expect(after.2.shortcuts.first { $0.id == Self.ids[2] }?.payload == "ls -la")
        #expect(after.3.customThemes == rt.customThemes && after.3.customThemes.map(\.name) == ["T2"])
        #expect(after.4.customThemes.isEmpty)
        #expect([after.0.isAuthoritative, after.1.isAuthoritative, after.2.isAuthoritative,
                 after.3.isAuthoritative, after.4.isAuthoritative].allSatisfy { $0 })
    }

    // MARK: - Sealed launch

    @Test("a launch before the first unlock writes nothing until the unlock")
    func sealedLaunchWritesNothing() {
        // A sealed preferences file reads as nothing: the suite is empty,
        // protected data unavailable, no sentinel.
        let suite = Scratch(), mirror = Scratch()
        defer { suite.drop(); mirror.drop() }
        let prefs = Prefs(suite.defaults, available: false)
        let connections = ConnectionStore(backing: RustConnectionStore(preferences: prefs, mode: .system))
        let keys = SSHKeyStore(backing: RustSSHKeyStore(preferences: prefs, mode: .system))
        let bar = ShortcutStore(backing: RustShortcutStore(preferences: prefs, mode: .system))
        let themes = ThemeStore(backing: RustThemeStore(preferences: prefs, mode: .system, untitled: "Untitled"))
        let accents = AppThemeStore(backing: RustAppThemeStore(preferences: prefs, mode: .system, untitled: "Untitled"))
        let settings = AppSettings(defaults: suite.defaults,
                                   backing: RustSettings(preferences: prefs, mirror: Prefs(mirror.defaults),
                                                         mode: .system, debugBuild: false))
        #expect(![connections.isAuthoritative, keys.isAuthoritative, bar.isAuthoritative,
                  themes.isAuthoritative, accents.isAuthoritative].contains(true))
        #expect(!bar.shortcuts.isEmpty, "the built-ins shown, never seeded into the file")
        #expect(connections.savedIds == nil && connections.keychainRefs() == nil && keys.keychainRefs() == nil)

        // The Swift store beside it, on a sealed suite of its own: what
        // the adapter must show, and when.
        let swiftSuite = Scratch()
        defer { swiftSuite.drop() }
        let swift = ConnectionStore(defaults: swiftSuite.defaults, readiness: DefaultsReadiness { false })

        // Everything the user can do from a first-run card over a locked
        // phone's sealed file.
        let pi = ServerConnection(id: Self.ids[0], name: "pi", host: "pi",
                                  createdAt: Date(timeIntervalSinceReferenceDate: 784_000_000.5))
        for store in [connections, swift] { store.add(pi) }
        #expect(connections.connections.map(\.id) == [Self.ids[0]], "shown, not written")
        #expect(connections.connections == swift.connections)
        keys.add(SSHKeyRecord(id: Self.ids[1], name: "k", algorithm: .ed25519, source: .generated))
        _ = bar.add(TerminalShortcut(id: Self.ids[2], kind: .text, payload: "x"))
        bar.restoreDefaults()
        var theme = TerminalTheme.fallback
        theme.id = "custom-test0001"
        theme.isBuiltIn = false
        themes.save(theme)
        accents.save(.custom(id: "app-test0001", name: "A", accentHex: "123456"))
        // Coming forward while still sealed reads the file again — the
        // shown connection goes with that read, on either side (it was never
        // written).
        for store in [connections, swift] { store.reloadIfNeeded() }
        #expect(connections.connections == swift.connections)
        #expect(connections.connections.isEmpty)
        settings.reloadIfNeeded()
        #expect(suite.contents.isEmpty, "\(suite.contents.keys.sorted())")
        #expect(swiftSuite.contents.isEmpty)
        #expect(mirror.contents.isEmpty, "the mirror waits for a trusted read")

        // Unlocked: the next mutation reads the (empty) file first and
        // writes the user's change into it.
        prefs.available = true
        connections.add(ServerConnection(id: Self.ids[3], name: "nas", host: "nas"))
        settings.reloadIfNeeded()
        #expect(connections.isAuthoritative)
        #expect(ConnectionStore(defaults: suite.defaults).connections.map(\.id) == [Self.ids[3]])
        #expect(suite.defaults.bool(forKey: DefaultsReadiness.sentinelKey))
        #expect(mirror.defaults.object(forKey: PushPrefs.detailKey) as? Bool == true)
    }

    @Test("a locked launch over data the file still holds changes none of it")
    func lockedLaunchKeepsTheFile() {
        let suite = Scratch()
        defer { suite.drop() }
        let swift = ConnectionStore(defaults: suite.defaults)
        swift.add(ServerConnection(id: Self.ids[0], name: "pi", host: "pi"))
        ShortcutStore(defaults: suite.defaults).add(TerminalShortcut(id: Self.ids[1], kind: .text, payload: "x"))
        suite.defaults.removeObject(forKey: DefaultsReadiness.sentinelKey)
        let before = Self.canonical(suite.contents)
        let prefs = Prefs(suite.defaults, available: false)
        let connections = ConnectionStore(backing: RustConnectionStore(preferences: prefs, mode: .system))
        let bar = ShortcutStore(backing: RustShortcutStore(preferences: prefs, mode: .system))
        #expect(!connections.isAuthoritative && !bar.isAuthoritative)
        connections.delete(id: Self.ids[0])
        bar.restoreDefaults()
        bar.remove(id: Self.ids[1])
        #expect(Self.canonical(suite.contents) == before)
    }

    // MARK: - Known hosts and pairings

    /// The item through the keychain host as the app's known hosts make it
    /// (``RustStoreSwitch/keychain``): read as stored. The hosts a DEBUG
    /// seed trusts for the launch (``LaunchTrust``) are the validator's own
    /// to decide, and are no part of the item here.
    static func keychain(_ backend: KeychainBackend) -> RustKeychainHost {
        RustKeychainHost(backend: backend, launchTrust: nil)
    }

    @Test("trusted host keys: each side reads the other's item, and locked is never empty")
    func knownHosts() throws {
        let swiftBackend = InMemoryKeychainBackend(), rustBackend = InMemoryKeychainBackend()
        let suite = Scratch()
        defer { suite.drop() }
        let prefs = Prefs(suite.defaults)
        let swift = KeychainKnownHostsStore(backend: swiftBackend)
        let rust = RustKnownHostsStore(keychain: Self.keychain(rustBackend), preferences: prefs)
        let map = ["pi:22": "SHA256:abc", "::1:2222": "SHA256:+/=", "büro:22": "SHA256:x"]
        #expect(try rust.read().isEmpty)
        try swift.write(map)
        try rust.write(map)
        #expect(try rust.read() == map)
        let crossed = RustKnownHostsStore(keychain: Self.keychain(swiftBackend), preferences: prefs)
        #expect(try crossed.read() == map)
        #expect(try KeychainKnownHostsStore(backend: rustBackend).read() == map)

        // Before the first unlock the keychain shows no item: not an empty map.
        let sealed = RustKnownHostsStore(keychain: Self.keychain(InMemoryKeychainBackend()),
                                         preferences: Prefs(suite.defaults, available: false))
        do {
            let read = try sealed.read()
            Issue.record("a sealed keychain read as \(read)")
        } catch {
            #expect((error as? KnownHostsUnreadable)?.locked == true, "\(error)")
        }
    }

    @Test("push pairings: each side reads the other's file")
    func pairings() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("w5b-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        let ours = dir.appendingPathComponent("rust/push-pairings.json")
        let theirs = dir.appendingPathComponent("swift/push-pairings.json")
        let rust = RustPushPairings(url: ours)
        let swift = PushPairingFile(url: theirs)
        var one = PushPairing(connectionId: Self.ids[0], hostLabel: "pi · 研", secretHex: String(repeating: "ab", count: 32),
                              sendToken: "", relayURL: "https://relay.example.com",
                              createdAt: Date(timeIntervalSinceReferenceDate: 784_000_000.25))
        let two = PushPairing(connectionId: Self.ids[1], hostLabel: "nas", secretHex: String(repeating: "cd", count: 32),
                              sendToken: "t", relayURL: "https://relay.example.com", createdAt: Date(timeIntervalSinceReferenceDate: 1),
                              apnsToken: "ABC", apnsEnv: "sandbox", sendTokenIssuedAt: Date(timeIntervalSinceReferenceDate: 2))
        #expect(rust.upsert(one) && swift.upsert(one))
        #expect(rust.upsert(two) && swift.upsert(two))
        #expect(rust.remove(connectionId: Self.ids[5]) && swift.remove(connectionId: Self.ids[5]), "nothing to forget")
        one.sendToken = "minted"
        #expect(rust.upsert(one) && swift.upsert(one))
        #expect(rust.read() == swift.read())
        #expect(PushPairingFile(url: ours).read() == rust.read())
        #expect(RustPushPairings(url: theirs).read() == swift.read())
        #expect(rust.remove(connectionId: Self.ids[1]))
        #expect(PushPairingFile(url: ours).read().map(\.connectionId) == [Self.ids[0]])
        let excluded = try ours.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup
        #expect(excluded == true, "out of backups, as the Swift file's")
        #if !targetEnvironment(simulator)
        // The simulator keeps no data protection class on a file: nil there
        // whatever was asked.
        let protection = try FileManager.default.attributesOfItem(atPath: ours.path)[.protectionKey] as? FileProtectionType
        #expect(protection == .completeUntilFirstUserAuthentication)
        #endif
    }

    // MARK: - The Rust store, always

    @Test("every build runs its stores on the Rust core: no switch (F0)")
    func alwaysRust() {
        #expect(RustStoreSwitch.available, "this build links the Rust core")
        #expect(RustStoreSwitch.isOn)
    }
}
