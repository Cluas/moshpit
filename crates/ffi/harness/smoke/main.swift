// The store's UniFFI API from Swift (crates/ffi/src/store.rs, bindings
// generated from the macOS library by harness/uniffi-smoke.sh): a
// PreferencesHost over a real UserDefaults suite — the shape the app's
// adapter takes, plist nodes to Foundation objects and back — and a
// keychain host over a dictionary, driving the Rust stores.

import Foundation

/// Foundation objects to the store's flat node list and back.
enum Nodes {
    static func value(_ object: Any) -> PlistValue {
        var nodes: [PlistNode] = []
        @discardableResult func push(_ object: Any) -> UInt32 {
            let index = nodes.count
            nodes.append(.bool(value: false))
            let node: PlistNode
            if let data = object as? Data {
                node = .data(value: data)
            } else if let string = object as? String {
                node = .string(value: string)
            } else if let date = object as? Date {
                node = .date(seconds: date.timeIntervalSinceReferenceDate)
            } else if let array = object as? [Any] {
                node = .array(members: array.map { push($0) })
            } else if let dict = object as? NSDictionary {
                let keys = dict.allKeys.map { $0 as! String }
                node = .dictionary(keys: keys, members: keys.map { push(dict[$0]!) })
            } else if let number = object as? NSNumber {
                if CFGetTypeID(number) == CFBooleanGetTypeID() {
                    node = .bool(value: number.boolValue)
                } else if CFNumberIsFloatType(number) {
                    node = .double(value: number.doubleValue)
                } else {
                    node = .int(value: number.int64Value)
                }
            } else {
                fatalError("not a plist value: \(type(of: object))")
            }
            nodes[index] = node
            return UInt32(index)
        }
        push(object)
        return PlistValue(nodes: nodes)
    }

    static func object(_ value: PlistValue) -> Any {
        func build(_ index: Int) -> Any {
            switch value.nodes[index] {
            case .bool(let v): return v
            case .int(let v): return NSNumber(value: v)
            case .double(let v): return NSNumber(value: v)
            case .string(let v): return v
            case .data(let v): return v
            case .date(let s): return Date(timeIntervalSinceReferenceDate: s)
            case .array(let members): return members.map { build(Int($0)) }
            case .dictionary(let keys, let members):
                let dict = NSMutableDictionary()
                for (key, member) in zip(keys, members) { dict[key] = build(Int(member)) }
                return dict
            }
        }
        return build(0)
    }
}

final class DefaultsHost: PreferencesHost, @unchecked Sendable {
    let defaults: UserDefaults
    init(_ defaults: UserDefaults) { self.defaults = defaults }
    func get(key: String) -> PlistValue? { defaults.object(forKey: key).map(Nodes.value) }
    func set(key: String, value: PlistValue?) {
        if let value { defaults.set(Nodes.object(value), forKey: key) } else { defaults.removeObject(forKey: key) }
    }
    func protectedDataAvailable() -> Bool { true }
}

final class DictionaryKeychain: KeychainHost, @unchecked Sendable {
    var items: [String: Data] = [:]
    func save(account: String, secret: Data, requireBiometry: Bool) throws { items[account] = secret }
    func load(account: String, prompt: String) throws -> Data {
        guard let data = items[account] else { throw KeychainFailure.NotFound }
        return data
    }
    func delete(account: String) throws { items.removeValue(forKey: account) }
    func exists(account: String) -> Bool { items[account] != nil }
    func accounts() throws -> [String] { Array(items.keys) }
    func loadSilently(account: String) throws -> Data { try load(account: account, prompt: "") }
}

func check(_ condition: Bool, _ what: String) {
    if !condition { print("FAIL: \(what)"); exit(1) }
    print("ok: \(what)")
}

@main
struct Smoke {
    static func main() throws {
        let name = "w5.ffi.smoke.\(ProcessInfo.processInfo.processIdentifier)"
        let defaults = UserDefaults(suiteName: name)!
        let prefs = URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("Library/Preferences")
        defer {
            defaults.removePersistentDomain(forName: name)
            try? FileManager.default.removeItem(at: prefs.appendingPathComponent("\(name).plist"))
        }
        let host = DefaultsHost(defaults)

        // Connections: written by Rust into UserDefaults, read back by a new
        // store, and decodable as the app's JSON.
        let store = ConnectionStoreHandle(host: host, mode: .system)
        let id = UUID().uuidString
        try store.add(connection: ConnectionRecord(
            id: id, name: "studio", host: "studio.example.com", port: 2222, username: "dev",
            authMethod: .key, connectionProtocol: .mosh, sshPort: 2222, moshPortRangeStart: 60001,
            moshPortRangeEnd: 60999, moshServerPath: nil, keychainRef: storeNewKeychainRef(),
            createdAt: 784_000_000.5, lastConnectedAt: nil, useTmux: true, predictModeRaw: "always",
            roamOnCellular: false, tmuxPath: nil, herdrPath: nil, multiplexerRaw: "tmux", sshKeyId: nil,
            sshKeyAlgorithmRaw: nil, useSocksProxy: nil, socksProxyHost: nil, socksProxyPort: nil))
        let reread = ConnectionStoreHandle(host: host, mode: .system).connections()
        check(reread.map(\.id) == [id] && reread[0].createdAt == 784_000_000, "a connection round-trips through UserDefaults")
        let stored = defaults.data(forKey: "moshpit.connections")!
        let entries = try JSONSerialization.jsonObject(with: stored) as! [[String: Any]]
        check(entries[0]["createdAt"] as? String == "2025-11-05T01:46:40Z", "dates are ISO 8601, whole seconds")
        check(entries[0]["keepAliveInterval"] as? Int == 60, "J1's rollback fields are written")
        check(defaults.bool(forKey: "moshpit.defaults.ready"), "the sentinel is set after a trusted read")
        do { try store.delete(id: "nope"); check(false, "a bad id is refused") } catch StoreError.InvalidId { check(true, "a bad id is refused") }

        // Shortcuts and their bytes.
        let shortcuts = ShortcutStoreHandle(host: host, mode: .system)
        check(shortcuts.toolbar().map(\.chipLabel) == ["esc", "tab", "^C", "paste", "✛", "mic", "img", "files"], "the default toolbar")
        let twoStep = shortcuts.shortcuts().first { $0.chipLabel == "⇥⏎" }!
        check(try storeShortcutStages(shortcut: twoStep) == [Data([0x09]), Data([0x0D])], "⇥⏎ is two writes")

        // Settings over UserDefaults, mirrored into a second suite.
        let mirrorName = name + ".group"
        let mirror = UserDefaults(suiteName: mirrorName)!
        defer {
            mirror.removePersistentDomain(forName: mirrorName)
            try? FileManager.default.removeItem(at: prefs.appendingPathComponent("\(mirrorName).plist"))
        }
        let settings = SettingsHandle(host: host, mirror: DefaultsHost(mirror), mode: .system, debugBuild: false)
        check(try settings.get(name: "fontSize") == .double(value: 9), "fontSize defaults to 9")
        try settings.set(name: "attentionSoundEnabled", value: .bool(value: false))
        check(mirror.object(forKey: "moshpit.push.sound") as? Bool == false, "the sound switch is mirrored")
        try settings.set(name: "hostSetupDeclined", value: .stringSet(values: ["b", "a"]))
        check(defaults.stringArray(forKey: "moshpit.settings.hostSetupDeclined") == ["a", "b"], "declined hosts are stored sorted")

        // Themes and the export.
        let themes = ThemeStoreHandle(host: host, mode: .system)
        let imported = try themes.importThemes(json: storeThemeExportJson(theme: themes.theme(id: "nord")))
        check(imported.count == 1 && imported[0].name == "Nord 2", "a theme imports under a fresh name")
        check(storeThemePalette(theme: imported[0]).count == 16, "sixteen colours")

        // Known hosts over the keychain host.
        let keychain = DictionaryKeychain()
        let hosts = KnownHostsHandle(keychain: keychain, preferences: host)
        try hosts.write(entries: [storeKnownHostsEntry(host: "box", port: 22): "SHA256:abc/def"])
        check(String(data: keychain.items["moshpit.knownHosts"]!, encoding: .utf8) == #"{"box:22":"SHA256:abc\/def"}"#, "the trusted map is JSONEncoder's")
        check(try hosts.read() == ["box:22": "SHA256:abc/def"], "and reads back")
        print("all ok")
    }
}
