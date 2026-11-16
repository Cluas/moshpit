import Foundation
import MoshpitFFI
import MoshpitKit
import Testing
import UIKit
@testable import Moshpit

/// The app's stores on the Rust store (Services/Rust/RustStores.swift), on
/// this device's Foundation:
///
/// - a launch before the first unlock (protected data unavailable, no
///   sentinel) writes nothing, whatever the user does, until the unlock;
/// - a locked launch over data the file holds changes none of it;
/// - the trusted host keys and the push pairings, read across the app's
///   and the notification service's sides.
///
/// Every suite, keychain and file here is the test's own: nothing the app
/// keeps is read or written (the settings mirror into a suite of the
/// test's). The app's Swift stores, which these ran against until batch C
/// deleted them, are crates/store's harness's record now (1.0.8's read of
/// what the Rust side writes is its phase D).
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

        // Everything the user can do from a first-run card over a locked
        // phone's sealed file.
        let pi = ServerConnection(id: Self.ids[0], name: "pi", host: "pi",
                                  createdAt: Date(timeIntervalSinceReferenceDate: 784_000_000.5))
        connections.add(pi)
        #expect(connections.connections.map(\.id) == [Self.ids[0]], "shown, not written")
        keys.add(SSHKeyRecord(id: Self.ids[1], name: "k", algorithm: .ed25519, source: .generated))
        _ = bar.add(TerminalShortcut(id: Self.ids[2], kind: .text, payload: "x"))
        bar.restoreDefaults()
        var theme = TerminalTheme.fallback
        theme.id = "custom-test0001"
        theme.isBuiltIn = false
        themes.save(theme)
        accents.save(.custom(id: "app-test0001", name: "A", accentHex: "123456"))
        // Coming forward while still sealed reads the file again — the
        // shown connection goes with that read (it was never written).
        connections.reloadIfNeeded()
        #expect(connections.connections.isEmpty)
        settings.reloadIfNeeded()
        #expect(suite.contents.isEmpty, "\(suite.contents.keys.sorted())")
        #expect(mirror.contents.isEmpty, "the mirror waits for a trusted read")

        // Unlocked: the next mutation reads the (empty) file first and
        // writes the user's change into it.
        prefs.available = true
        connections.add(ServerConnection(id: Self.ids[3], name: "nas", host: "nas"))
        settings.reloadIfNeeded()
        #expect(connections.isAuthoritative)
        #expect(ConnectionStore(defaults: suite.defaults).connections.map(\.id) == [Self.ids[3]])
        #expect(suite.defaults.bool(forKey: DefaultsReadiness.sentinelKey))
        #expect(mirror.defaults.object(forKey: "moshpit.push.showDetail") as? Bool == true)
    }

    @Test("a locked launch over data the file still holds changes none of it")
    func lockedLaunchKeepsTheFile() {
        let suite = Scratch()
        defer { suite.drop() }
        ConnectionStore(defaults: suite.defaults).add(ServerConnection(id: Self.ids[0], name: "pi", host: "pi"))
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

    /// An early "unavailable" (a flag read once, at `MoshpitApp.init`)
    /// corrects itself the next time the app becomes active: with no
    /// sentinel yet, nothing could be trusted until a lock and unlock.
    @Test("the protected-data flag asks the system again each time the app becomes active")
    @MainActor
    func protectedDataFlagRereadsOnActivation() {
        let center = NotificationCenter()
        var system = false
        let flag = ProtectedDataFlag(center: center, read: { system })
        #expect(!flag.isAvailable)
        system = true
        center.post(name: UIApplication.didBecomeActiveNotification, object: nil)
        #expect(flag.isAvailable, "the early read stood")
        center.post(name: UIApplication.protectedDataWillBecomeUnavailableNotification, object: nil)
        #expect(!flag.isAvailable, "locked")
        center.post(name: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil)
        #expect(flag.isAvailable, "unlocked")
    }

    // MARK: - Pairings

    @Test("push pairings: the store keeps them, and reads the file 1.0.8 wrote")
    func pairings() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("w5b-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        let ours = dir.appendingPathComponent("rust/push-pairings.json")
        let theirs = dir.appendingPathComponent("108/push-pairings.json")
        let rust = RustPushPairings(url: ours)
        var one = PairingRecord(connectionId: Self.ids[0].uuidString, hostLabel: "pi · 研",
                                secretHex: String(repeating: "ab", count: 32), sendToken: "",
                                relayUrl: "https://relay.example.com", createdAt: 784_000_000.25,
                                apnsToken: nil, apnsEnv: nil, sendTokenIssuedAt: nil)
        let two = PairingRecord(connectionId: Self.ids[1].uuidString, hostLabel: "nas",
                                secretHex: String(repeating: "cd", count: 32), sendToken: "t",
                                relayUrl: "https://relay.example.com", createdAt: 1,
                                apnsToken: "ABC", apnsEnv: "sandbox", sendTokenIssuedAt: 2)
        #expect(rust.upsert(one))
        #expect(rust.upsert(two))
        #expect(rust.remove(connectionId: Self.ids[5].uuidString), "nothing to forget")
        one.sendToken = "minted"
        #expect(rust.upsert(one))
        #expect(RustPushPairings(url: ours).read().count == 2)
        #expect(rust.remove(connectionId: Self.ids[1].uuidString))
        #expect(RustPushPairings(url: ours).read().map(\.connectionId) == [Self.ids[0].uuidString])

        // A file as every Swift build up to 1.0.8 wrote it: Foundation's
        // encoder, one array, dates as seconds since 2001, an absent
        // optional left out (the notification service reads it through
        // crates/seal now, E7b).
        try FileManager.default.createDirectory(at: theirs.deletingLastPathComponent(),
                                                withIntermediateDirectories: true)
        let written = """
            [{"connectionId":"\(Self.ids[0].uuidString)","hostLabel":"pi · 研","secretHex":"\(String(repeating: "ab", count: 32))",\
            "sendToken":"minted","relayURL":"https://relay.example.com","createdAt":784000000.25},\
            {"connectionId":"\(Self.ids[1].uuidString)","hostLabel":"nas","secretHex":"\(String(repeating: "cd", count: 32))",\
            "sendToken":"t","relayURL":"https://relay.example.com","createdAt":1,"apnsToken":"ABC",\
            "apnsEnv":"sandbox","sendTokenIssuedAt":2}]
            """
        try Data(written.utf8).write(to: theirs)
        #expect(RustPushPairings(url: theirs).read() == [one, two])

        let excluded = try ours.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup
        #expect(excluded == true, "out of backups")
        #if !targetEnvironment(simulator)
        // The simulator keeps no data protection class on a file: nil there
        // whatever was asked.
        let protection = try FileManager.default.attributesOfItem(atPath: ours.path)[.protectionKey] as? FileProtectionType
        #expect(protection == .completeUntilFirstUserAuthentication)
        #endif
    }

    // MARK: - The Rust store, always

    @Test("every build links the Rust core its stores run on: no switch (F0, batch C)")
    func alwaysRust() {
        #expect(mp_ffi_abi_version() == UInt32(MP_FFI_ABI_VERSION), "this build links the Rust core")
    }
}
