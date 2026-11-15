import Foundation
import MoshpitKit

// The app's stores on the Rust core's store (crates/store, through
// crates/ffi/src/store.rs; ``AppStores`` makes them): one adapter per store,
// each the backing its store class delegates to (``ConnectionStoreBacking``
// and the rest), the class keeping the API the views and services know.
//
// Each adapter holds one Rust handle over the host traits: the preferences
// (``RustPreferencesHost`` on `UserDefaults.standard` — the same keys and
// formats, nothing migrates, docs/nsh/DESIGN.md §3.5); the key store only
// ever names a keychain item. Whether protected data is
// available (the sealed-launch rule, ``DefaultsReadiness``) is the
// preferences host's to say: a Rust store made before the first unlock
// reads, shows, never writes, and reads again when asked to reload.
//
// Nothing here is a hot path: a store is read when the app starts and comes
// forward, written when the user changes something. A list crosses whole,
// converted field for field.

// MARK: - Connections

/// ``ConnectionStore`` on the Rust store.
final class RustConnectionStore: ConnectionStoreBacking {
    private let handle: ConnectionStoreHandle

    init(preferences: PreferencesHost, mode: ReadinessMode) {
        handle = ConnectionStoreHandle(host: preferences, mode: mode)
    }

    var connections: [ServerConnection] { handle.connections().compactMap(ServerConnection.init) }
    var isAuthoritative: Bool { handle.isAuthoritative() }

    func add(_ connection: ServerConnection) {
        RustStoreCall.run("connections: add") { try handle.add(connection: ConnectionRecord(connection)) }
    }

    func update(_ connection: ServerConnection) {
        RustStoreCall.run("connections: update") { try handle.update(connection: ConnectionRecord(connection)) }
    }

    func delete(id: UUID) {
        RustStoreCall.run("connections: delete") { try handle.delete(id: id.uuidString) }
    }

    func load() { handle.load() }
    func reloadIfNeeded() { handle.reloadIfNeeded() }

    var savedIds: Set<UUID>? {
        handle.savedIds().map { Set($0.compactMap(UUID.init(uuidString:))) }
    }

    func keychainRefs(excluding: UUID?) -> Set<String>? {
        RustStoreCall.value("connections: keychain refs") {
            try handle.keychainRefs(excluding: excluding?.uuidString)
        }.flatMap { $0.map(Set.init) }
    }

    func names(usingKey keyId: UUID, ref: String?) -> [String]? {
        RustStoreCall.value("connections: names using a key") {
            try handle.namesUsingKey(key: keyId.uuidString, keychainRef: ref)
        } ?? nil
    }

    func sweepSessionMemory(keys: [String]) -> [String] {
        handle.sweepSessionMemory(keys: keys)
    }
}

extension ConnectionRecord {
    init(_ c: ServerConnection) {
        self.init(
            id: c.id.uuidString, name: c.name, host: c.host, port: Int64(c.port), username: c.username,
            authMethod: c.authMethod == .key ? .key : .password,
            connectionProtocol: c.connectionProtocol == .mosh ? .mosh : .ssh,
            sshPort: Int64(c.sshPort), moshPortRangeStart: Int64(c.moshPortRangeStart),
            moshPortRangeEnd: Int64(c.moshPortRangeEnd), moshServerPath: c.moshServerPath,
            keychainRef: c.keychainRef, createdAt: c.createdAt.timeIntervalSinceReferenceDate,
            lastConnectedAt: c.lastConnectedAt?.timeIntervalSinceReferenceDate, useTmux: c.useTmux,
            predictModeRaw: c.predictModeRaw, roamOnCellular: c.roamOnCellular, tmuxPath: c.tmuxPath,
            herdrPath: c.herdrPath, multiplexerRaw: c.multiplexerRaw, sshKeyId: c.sshKeyId?.uuidString,
            sshKeyAlgorithmRaw: c.sshKeyAlgorithmRaw, useSocksProxy: c.useSOCKSProxy,
            socksProxyHost: c.socksProxyHost, socksProxyPort: c.socksProxyPort.map(Int64.init),
            emoji: c.emoji)
    }
}

extension ServerConnection {
    /// nil only for an id that isn't a UUID, which the Rust store never
    /// hands out (it parsed every id it holds).
    init?(_ r: ConnectionRecord) {
        guard let id = UUID(uuidString: r.id) else { return nil }
        self.init(
            id: id, name: r.name, host: r.host, port: Int(r.port), username: r.username,
            authMethod: r.authMethod == .key ? .key : .password,
            connectionProtocol: r.connectionProtocol == .mosh ? .mosh : .ssh,
            sshPort: Int(r.sshPort), moshPortRangeStart: Int(r.moshPortRangeStart),
            moshPortRangeEnd: Int(r.moshPortRangeEnd), moshServerPath: r.moshServerPath,
            keychainRef: r.keychainRef, createdAt: Date(timeIntervalSinceReferenceDate: r.createdAt),
            lastConnectedAt: r.lastConnectedAt.map(Date.init(timeIntervalSinceReferenceDate:)),
            useTmux: r.useTmux, predictModeRaw: r.predictModeRaw, roamOnCellular: r.roamOnCellular,
            tmuxPath: r.tmuxPath, herdrPath: r.herdrPath, multiplexerRaw: r.multiplexerRaw)
        sshKeyId = r.sshKeyId.flatMap(UUID.init(uuidString:))
        sshKeyAlgorithmRaw = r.sshKeyAlgorithmRaw
        useSOCKSProxy = r.useSocksProxy
        socksProxyHost = r.socksProxyHost
        socksProxyPort = r.socksProxyPort.map { Int($0) }
        emoji = r.emoji
    }
}

// MARK: - SSH keys

/// ``SSHKeyStore`` on the Rust store: the records only — every private key
/// stays in the keychain, named by its record's `keychainRef`.
final class RustSSHKeyStore: SSHKeyStoreBacking {
    private let handle: KeyStoreHandle

    init(preferences: PreferencesHost, mode: ReadinessMode) {
        handle = KeyStoreHandle(host: preferences, mode: mode)
    }

    var keys: [SSHKeyRecord] { handle.keys().compactMap(SSHKeyRecord.init) }
    var isAuthoritative: Bool { handle.isAuthoritative() }

    func add(_ record: SSHKeyRecord) {
        RustStoreCall.run("keys: add") { try handle.add(record: KeyRecord(record)) }
    }

    func remove(id: UUID) {
        RustStoreCall.run("keys: remove") { try handle.remove(id: id.uuidString) }
    }

    func reloadIfNeeded() { handle.reloadIfNeeded() }

    func keychainRefs(excluding: UUID?) -> Set<String>? {
        RustStoreCall.value("keys: keychain refs") {
            try handle.keychainRefs(excluding: excluding?.uuidString)
        }.flatMap { $0.map(Set.init) }
    }
}

extension KeyRecord {
    init(_ k: SSHKeyRecord) {
        self.init(
            id: k.id.uuidString, name: k.name, algorithm: KeyAlgorithm(k.algorithm), bits: k.bits.map(Int64.init),
            comment: k.comment, fingerprint: k.fingerprint, publicKey: k.publicKey, source: KeySource(k.source),
            boundHosts: k.boundHosts, keychainRef: k.keychainRef, requireBiometry: k.requireBiometry,
            createdAt: k.createdAt.timeIntervalSinceReferenceDate,
            lastUsedAt: k.lastUsedAt?.timeIntervalSinceReferenceDate, badgeText: k.badgeText)
    }
}

extension SSHKeyRecord {
    init?(_ r: KeyRecord) {
        guard let id = UUID(uuidString: r.id) else { return nil }
        self.init(
            id: id, name: r.name, algorithm: SSHKeyAlgorithm(r.algorithm), bits: r.bits.map { Int($0) },
            comment: r.comment, fingerprint: r.fingerprint, publicKey: r.publicKey,
            source: SSHKeySource(r.source), boundHosts: r.boundHosts, keychainRef: r.keychainRef,
            requireBiometry: r.requireBiometry, createdAt: Date(timeIntervalSinceReferenceDate: r.createdAt),
            lastUsedAt: r.lastUsedAt.map(Date.init(timeIntervalSinceReferenceDate:)))
    }
}

extension KeyAlgorithm {
    init(_ a: SSHKeyAlgorithm) {
        switch a {
        case .ed25519: self = .ed25519
        case .ecdsaSK: self = .ecdsaSk
        case .rsa4096: self = .rsa
        case .seP256: self = .seP256
        case .ecdsa: self = .ecdsa
        }
    }
}

extension SSHKeyAlgorithm {
    init(_ a: KeyAlgorithm) {
        switch a {
        case .ed25519: self = .ed25519
        case .ecdsaSk: self = .ecdsaSK
        case .rsa: self = .rsa4096
        case .seP256: self = .seP256
        case .ecdsa: self = .ecdsa
        }
    }
}

extension KeySource {
    init(_ s: SSHKeySource) {
        switch s {
        case .secureEnclave: self = .secureEnclave
        case .generated: self = .generated
        case .imported: self = .imported
        case .hardware: self = .hardware
        }
    }
}

extension SSHKeySource {
    init(_ s: KeySource) {
        switch s {
        case .secureEnclave: self = .secureEnclave
        case .generated: self = .generated
        case .imported: self = .imported
        case .hardware: self = .hardware
        }
    }
}

// MARK: - Shortcuts

/// ``ShortcutStore`` on the Rust store: the built-ins, their reconcile
/// step and the stored form 1.0.8 reads are the Rust store's.
final class RustShortcutStore: ShortcutStoreBacking {
    private let handle: ShortcutStoreHandle

    init(preferences: PreferencesHost, mode: ReadinessMode) {
        handle = ShortcutStoreHandle(host: preferences, mode: mode)
    }

    var shortcuts: [TerminalShortcut] { handle.shortcuts().compactMap(TerminalShortcut.init) }
    var isAuthoritative: Bool { handle.isAuthoritative() }

    func toolbar(forHost hostName: String?, inMultiplexer: Bool) -> [TerminalShortcut] {
        handle.toolbarFor(hostName: hostName, inMultiplexer: inMultiplexer).compactMap(TerminalShortcut.init)
    }

    func add(_ shortcut: TerminalShortcut) -> Bool {
        RustStoreCall.value("shortcuts: add") { try handle.add(shortcut: ShortcutRecord(shortcut)) } ?? false
    }

    func update(_ shortcut: TerminalShortcut) {
        RustStoreCall.run("shortcuts: update") { try handle.update(shortcut: ShortcutRecord(shortcut)) }
    }

    func remove(id: UUID) {
        RustStoreCall.run("shortcuts: remove") { try handle.remove(id: id.uuidString) }
    }

    func addToToolbar(id: UUID) -> Bool {
        RustStoreCall.value("shortcuts: add to the toolbar") { try handle.addToToolbar(id: id.uuidString) } ?? false
    }

    func removeFromToolbar(id: UUID) {
        RustStoreCall.run("shortcuts: off the toolbar") { try handle.removeFromToolbar(id: id.uuidString) }
    }

    func restoreDefaults() { handle.restoreDefaults() }

    func moveInToolbar(fromOffsets: IndexSet, toOffset: Int) {
        handle.moveInToolbar(from: fromOffsets.map { UInt32($0) }, to: UInt32(toOffset))
    }

    func reloadIfNeeded() { handle.reloadIfNeeded() }
}

extension ShortcutRecord {
    init(_ s: TerminalShortcut) {
        self.init(
            id: s.id.uuidString, kind: StoreShortcutKind(s.kind),
            modifiers: s.modifiers.sorted().map(StoreShortcutModifier.init), key: s.key, payload: s.payload,
            appendReturn: s.appendReturn, repeatOnHold: s.repeatOnHold, chipLabel: s.chipLabel,
            summary: s.summary, colorId: s.colorId, scopeHosts: Array(s.scopeHosts), onlyInTmux: s.onlyInTmux,
            isBuiltin: s.isBuiltin, inToolbar: s.inToolbar)
    }
}

extension TerminalShortcut {
    init?(_ r: ShortcutRecord) {
        guard let id = UUID(uuidString: r.id) else { return nil }
        self.init()
        self.id = id
        kind = ShortcutKind(r.kind)
        modifiers = Set(r.modifiers.map(ShortcutModifier.init))
        key = r.key
        payload = r.payload
        appendReturn = r.appendReturn
        repeatOnHold = r.repeatOnHold
        chipLabel = r.chipLabel
        summary = r.summary
        colorId = r.colorId
        scopeHosts = Set(r.scopeHosts)
        onlyInTmux = r.onlyInTmux
        isBuiltin = r.isBuiltin
        inToolbar = r.inToolbar
    }
}

extension StoreShortcutKind {
    init(_ k: ShortcutKind) {
        switch k {
        case .keyCombo: self = .keyCombo
        case .text: self = .text
        case .command: self = .command
        case .dpad: self = .dpad
        case .arrows: self = .arrows
        case .paste: self = .paste
        case .scroll: self = .scroll
        case .ctrl: self = .ctrl
        case .mic: self = .mic
        case .image: self = .image
        case .files: self = .files
        }
    }
}

extension ShortcutKind {
    init(_ k: StoreShortcutKind) {
        switch k {
        case .keyCombo: self = .keyCombo
        case .text: self = .text
        case .command: self = .command
        case .dpad: self = .dpad
        case .arrows: self = .arrows
        case .paste: self = .paste
        case .scroll: self = .scroll
        case .ctrl: self = .ctrl
        case .mic: self = .mic
        case .image: self = .image
        case .files: self = .files
        }
    }
}

extension StoreShortcutModifier {
    init(_ m: ShortcutModifier) {
        switch m {
        case .ctrl: self = .ctrl
        case .alt: self = .alt
        case .shift: self = .shift
        case .cmd: self = .cmd
        }
    }
}

extension ShortcutModifier {
    init(_ m: StoreShortcutModifier) {
        switch m {
        case .ctrl: self = .ctrl
        case .alt: self = .alt
        case .shift: self = .shift
        case .cmd: self = .cmd
        }
    }
}

// MARK: - Terminal themes

/// ``ThemeStore`` on the Rust store.
final class RustThemeStore: ThemeStoreBacking {
    private let handle: ThemeStoreHandle

    /// `untitled`: the name a theme saved without one gets, in the app's
    /// language.
    init(preferences: PreferencesHost, mode: ReadinessMode, untitled: String) {
        handle = ThemeStoreHandle(host: preferences, mode: mode)
        handle.setUntitledName(untitled: untitled)
    }

    var customThemes: [TerminalTheme] { handle.customThemes().map(TerminalTheme.init) }
    var isAuthoritative: Bool { handle.isAuthoritative() }

    func reloadIfNeeded() { handle.reloadIfNeeded() }
    func save(_ theme: TerminalTheme) { handle.save(theme: ThemeRecord(theme)) }
    func delete(id: String) { handle.delete(id: id) }
    func removeAllCustom() { handle.removeAllCustom() }
    func duplicate(_ theme: TerminalTheme) -> TerminalTheme { TerminalTheme(handle.duplicate(theme: ThemeRecord(theme))) }
    func makeDraft() -> TerminalTheme { TerminalTheme(handle.makeDraft()) }
    func exportJSON(_ theme: TerminalTheme) -> String { storeThemeExportJson(theme: ThemeRecord(theme)) }

    func importThemes(json: String) throws -> [TerminalTheme] {
        do {
            return try handle.importThemes(json: json).map(TerminalTheme.init)
        } catch ThemeImportError.NoThemes {
            throw ThemeStore.ImportError.noThemes
        } catch {
            throw ThemeStore.ImportError.notJSON
        }
    }
}

extension ThemeRecord {
    init(_ t: TerminalTheme) {
        self.init(id: t.id, name: t.name, backgroundHex: t.backgroundHex, foregroundHex: t.foregroundHex,
                  cursorHex: t.cursorHex, ansiHex: t.ansiHex, brightHex: t.brightHex, isBuiltIn: t.isBuiltIn)
    }
}

extension TerminalTheme {
    /// The record as it is: no colour normalised again on the way in.
    init(_ r: ThemeRecord) {
        self.init(id: r.id, name: r.name, background: r.backgroundHex, foreground: r.foregroundHex,
                  cursor: r.cursorHex, ansi: r.ansiHex, bright: r.brightHex, isBuiltIn: r.isBuiltIn)
        backgroundHex = r.backgroundHex
        foregroundHex = r.foregroundHex
        cursorHex = r.cursorHex
        ansiHex = r.ansiHex
        brightHex = r.brightHex
    }
}

// MARK: - Accent themes

/// ``AppThemeStore`` on the Rust store: only a custom theme's accent is
/// stored, the rest derived from it, as ever.
final class RustAppThemeStore: AppThemeStoreBacking {
    private let handle: AccentThemeStoreHandle

    init(preferences: PreferencesHost, mode: ReadinessMode, untitled: String) {
        handle = AccentThemeStoreHandle(host: preferences, mode: mode)
        handle.setUntitledName(untitled: untitled)
    }

    var customThemes: [AppTheme] { handle.customThemes().map(AppTheme.init) }
    var isAuthoritative: Bool { handle.isAuthoritative() }

    func reloadIfNeeded() { handle.reloadIfNeeded() }
    func save(_ theme: AppTheme) { handle.save(theme: AccentThemeRecord(theme)) }
    func delete(id: String) { handle.delete(id: id) }
    func removeAllCustom() { handle.removeAllCustom() }

    func makeDraft(basedOn theme: AppTheme) -> AppTheme {
        AppTheme(handle.makeDraft(basedOn: AccentThemeRecord(theme)))
    }
}

extension AccentThemeRecord {
    /// What the Rust store reads of a theme handed to it: its id, name,
    /// accent and whether it is built in (the rest it derives again).
    init(_ t: AppTheme) {
        self.init(id: t.id, name: t.name, accentHex: t.accentHex, accentPressedHex: "", screenBgHex: "",
                  terminalBgHex: "", isBuiltIn: t.isBuiltIn)
    }
}

extension AppTheme {
    /// A custom theme as the Rust store keeps it: built from its accent, as
    /// ``AppThemeStore`` builds one it reads.
    init(_ r: AccentThemeRecord) {
        self = .custom(id: r.id, name: r.name, accentHex: r.accentHex)
    }
}

// MARK: - Settings

/// ``AppSettings`` on the Rust store, by preferences key. The Rust settings
/// keep the notification service's mirror (`mirror`, the App Group's
/// preferences) of their two switches, as ``AppSettings`` does.
final class RustSettings: AppSettingsBacking {
    private let handle: SettingsHandle

    init(preferences: PreferencesHost, mirror: PreferencesHost?, mode: ReadinessMode, debugBuild: Bool) {
        handle = SettingsHandle(host: preferences, mirror: mirror, mode: mode, debugBuild: debugBuild)
    }

    func value(forKey key: String) -> Any? {
        switch handle.value(key: key) {
        case .bool(let value): return value
        case .int(let value): return Int(value)
        case .double(let value): return value
        case .string(let value): return value
        case .stringSet(let values): return values
        case nil: return nil
        }
    }

    func set(_ value: Any, forKey key: String) {
        let setting: SettingValue
        switch value {
        case let flag as Bool: setting = .bool(value: flag)
        case let number as Int: setting = .int(value: Int64(number))
        case let number as Double: setting = .double(value: number)
        case let text as String: setting = .string(value: text)
        case let strings as [String]: setting = .stringSet(values: strings)
        default:
            Log.store.error("rust store: settings: no setting value of type \(String(describing: type(of: value)), privacy: .public)")
            return
        }
        if !handle.setValue(key: key, value: setting) {
            Log.store.error("rust store: settings: \(key, privacy: .public) refused that value")
        }
    }

    func reloadIfNeeded() { handle.reloadIfNeeded() }
}

// MARK: - Push pairings

/// The push pairings (the App Group's `push-pairings.json`, which the
/// notification service reads) through the Rust store: the same file and
/// rules. What the Rust file leaves to its host is done here after every
/// write — the protection and backup exclusion ``PushPairingFile`` gives
/// each file it writes, the copies kept beside it too.
final class RustPushPairings: PushPairingBacking, @unchecked Sendable {
    private let handle: PushPairingFileHandle
    private let url: URL

    init(url: URL) {
        self.url = url
        handle = PushPairingFileHandle(path: url.path)
    }

    func read() -> [PushPairing] { handle.read().compactMap(PushPairing.init) }

    func write(_ pairings: [PushPairing]) -> Bool {
        settle("write") { try handle.write(pairings: pairings.map(PairingRecord.init)) }
    }

    func upsert(_ pairing: PushPairing) -> Bool {
        settle("upsert") { try handle.upsert(pairing: PairingRecord(pairing)) }
    }

    func remove(connectionId: UUID) -> Bool {
        settle("remove") { try handle.remove(connectionId: connectionId.uuidString) }
    }

    private func settle(_ what: String, _ body: () throws -> Bool) -> Bool {
        let saved = RustStoreCall.value("pairings: \(what)", body) ?? false
        if !saved { Log.push.error("pairings: the Rust store's \(what, privacy: .public) didn't read back") }
        protect()
        return saved
    }

    /// Complete until first user authentication, out of backups: the file
    /// and every copy kept beside it (`<file>.backup`, `<file>.backup-<n>`).
    private func protect() {
        let manager = FileManager.default
        let directory = url.deletingLastPathComponent()
        let name = url.lastPathComponent
        let siblings = (try? manager.contentsOfDirectory(atPath: directory.path)) ?? []
        for sibling in siblings where sibling == name || sibling.hasPrefix(name + ".backup") {
            var file = directory.appendingPathComponent(sibling)
            #if os(iOS)
            try? manager.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
                                       ofItemAtPath: file.path)
            #endif
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try? file.setResourceValues(values)
        }
    }
}

extension PairingRecord {
    init(_ p: PushPairing) {
        self.init(connectionId: p.connectionId.uuidString, hostLabel: p.hostLabel, secretHex: p.secretHex,
                  sendToken: p.sendToken, relayUrl: p.relayURL, createdAt: p.createdAt.timeIntervalSinceReferenceDate,
                  apnsToken: p.apnsToken, apnsEnv: p.apnsEnv,
                  sendTokenIssuedAt: p.sendTokenIssuedAt?.timeIntervalSinceReferenceDate)
    }
}

extension PushPairing {
    init?(_ r: PairingRecord) {
        guard let id = UUID(uuidString: r.connectionId) else { return nil }
        self.init(connectionId: id, hostLabel: r.hostLabel, secretHex: r.secretHex, sendToken: r.sendToken,
                  relayURL: r.relayUrl, createdAt: Date(timeIntervalSinceReferenceDate: r.createdAt),
                  apnsToken: r.apnsToken, apnsEnv: r.apnsEnv,
                  sendTokenIssuedAt: r.sendTokenIssuedAt.map(Date.init(timeIntervalSinceReferenceDate:)))
    }
}

// MARK: - Calls

/// A Rust store call that can fail only for an id that isn't a UUID —
/// which a Swift `UUID` always is. Logged, never thrown at a caller whose
/// API can't fail.
enum RustStoreCall {
    static func run(_ what: String, _ body: () throws -> Void) {
        do {
            try body()
        } catch {
            Log.store.error("rust store: \(what, privacy: .public) refused: \(String(describing: error), privacy: .public)")
        }
    }

    static func value<T>(_ what: String, _ body: () throws -> T) -> T? {
        do {
            return try body()
        } catch {
            Log.store.error("rust store: \(what, privacy: .public) refused: \(String(describing: error), privacy: .public)")
            return nil
        }
    }
}
