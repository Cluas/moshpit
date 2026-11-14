// Harness-only command line over 1.0.8's UNCHANGED storage sources (the
// release commit 8c10490a, built by harness/build-swift.sh): what 1.0.8
// reads of a store, and what it writes back.
//
//   store-swift-108 view <in> <out> <finals>
//
// Each input line is `STORE SNAPSHOT` (a store's key in a snapshot token,
// src/harness.rs). The output line is `BEFORE || AFTER`: the store as
// 1.0.8's one-piece decode reads it (`undecodable` when that decode fails —
// 1.0.8 then shows nothing, and its next save writes nothing back over the
// list), and as it reads its own write after re-saving the store (`touch`).
// The finals line is the snapshot 1.0.8 left. The views are in the format
// of `moshpit_store::harness::view108`.

import Foundation
import MoshpitKit

func connOut108(_ c: ServerConnection, _ m: Minted) -> String {
    [m.mask(c.id.uuidString), m.out(c.name), m.out(c.host), String(c.port), m.out(c.username),
     out(c.authMethod.rawValue), out(c.connectionProtocol.rawValue), String(c.sshPort),
     String(c.moshPortRangeStart), String(c.moshPortRangeEnd), m.opt(c.moshServerPath), m.opt(c.keychainRef),
     dateOut(c.createdAt), c.lastConnectedAt.map(dateOut) ?? "-", flag(c.useTmux), m.opt(c.predictModeRaw),
     optFlag(c.roamOnCellular), m.opt(c.tmuxPath), m.opt(c.herdrPath), m.opt(c.multiplexerRaw),
     c.sshKeyId.map { m.mask($0.uuidString) } ?? "-", m.opt(c.sshKeyAlgorithmRaw), optFlag(c.useSOCKSProxy),
     m.opt(c.socksProxyHost), c.socksProxyPort.map(String.init) ?? "-",
     m.opt(c.startupCommand), flag(c.forwardAgent), flag(c.compression), String(c.keepAliveInterval),
     m.opt(c.jumpHost), flag(c.useTailscale), flag(c.tmuxAutoAttach), flag(c.tmuxCreateNew)]
        .joined(separator: "/")
}

func keyOut108(_ k: SSHKeyRecord, _ m: Minted) -> String {
    let hosts = k.boundHosts.map { m.out($0) }
    return [m.mask(k.id.uuidString), m.out(k.name), out(k.algorithm.rawValue), m.out(k.comment),
            m.out(k.fingerprint), m.out(k.publicKey), out(k.source.rawValue),
            hosts.isEmpty ? "~" : hosts.joined(separator: ","), m.opt(k.keychainRef), flag(k.requireBiometry),
            dateOut(k.createdAt), k.lastUsedAt.map(dateOut) ?? "-"].joined(separator: "/")
}

func shortcutOut108(_ s: TerminalShortcut, _ m: Minted) -> String {
    let mods = s.modifiers.isEmpty ? "~" : s.modifiers.sorted().map(\.rawValue).joined(separator: "+")
    let hosts = s.scopeHosts.isEmpty ? "~" : s.scopeHosts.sorted(by: utf8Less).map { m.out($0) }.joined(separator: ",")
    return [m.mask(s.id.uuidString), m.out(s.summary), flag(s.isBuiltin), s.kind.rawValue, mods, m.out(s.key),
            m.out(s.payload), flag(s.appendReturn), flag(s.repeatOnHold), m.out(s.chipLabel), m.out(s.colorId),
            hosts, flag(s.onlyInTmux), flag(s.inToolbar)].joined(separator: "/")
}

func themeOut108(_ t: TerminalTheme, _ m: Minted) -> String {
    [m.out(t.id), m.out(t.name), out(t.backgroundHex), out(t.foregroundHex), out(t.cursorHex),
     t.ansiHex.map(out).joined(separator: ","), t.brightHex.map { $0.map(out).joined(separator: ",") } ?? "-",
     flag(t.isBuiltIn)].joined(separator: "/")
}

let groupSuite = UserDefaults(suiteName: PushPrefs.suite)!

func settingsView(_ s: AppSettings, _ m: Minted) -> String {
    func b(_ v: Bool) -> String { "b" + flag(v) }
    func t(_ v: String) -> String { "s" + m.out(v) }
    let declined = s.hostSetupDeclined.isEmpty ? "~"
        : s.hostSetupDeclined.sorted(by: utf8Less).map { m.out($0) }.joined(separator: ",")
    let vals: [(String, String)] = [
        ("fontSize", "f" + bits(s.fontSize)), ("themeId", t(s.themeId)), ("appThemeId", t(s.appThemeId)),
        ("appIconId", t(s.appIconId)), ("fontName", t(s.fontName)), ("cursorShape", t(s.cursorShape.rawValue)),
        ("cursorColorId", t(s.cursorColorId)), ("cursorBlink", b(s.cursorBlink)),
        ("trailOnPredict", b(s.trailOnPredict)), ("keepConnectionsAlive", b(s.keepConnectionsAlive)),
        ("moshByDefault", b(s.moshByDefault)), ("predictMode", t(s.predictMode.rawValue)),
        ("moshServerBinary", t(s.moshServerBinary)), ("udpRangeStart", "i\(s.udpRangeStart)"),
        ("udpRangeEnd", "i\(s.udpRangeEnd)"), ("notificationsEnabled", b(s.notificationsEnabled)),
        ("hostSetupDeclined", "a" + declined), ("pushRelayURL", t(s.pushRelayURL)),
        ("liveActivityEnabled", b(s.liveActivityEnabled)), ("attentionSoundEnabled", b(s.attentionSoundEnabled)),
        ("lockScreenDetailEnabled", b(s.lockScreenDetailEnabled)), ("raiseKeyboardOnOpen", b(s.raiseKeyboardOnOpen)),
        ("remoteClipboardReadEnabled", b(s.remoteClipboardReadEnabled)),
        ("uploadRetentionDays", "i\(s.uploadRetentionDays)"), ("voiceInputEnabled", b(s.voiceInputEnabled)),
        ("voiceInputLocaleId", t(s.voiceInputLocaleId)), ("voiceEngine", t(s.voiceEngine.rawValue)),
        ("whisperLanguage", t(s.whisperLanguage)), ("whisperModelId", t(s.whisperModelId)),
    ]
    return "vals=[" + vals.map { "\($0.0)=\($0.1)" }.joined(separator: ";") + "]"
}

/// Every setting written back through 1.0.8's own setter.
func rewriteSettings(_ s: AppSettings) {
    s.fontSize = s.fontSize; s.themeId = s.themeId; s.appThemeId = s.appThemeId; s.appIconId = s.appIconId
    s.fontName = s.fontName; s.cursorShape = s.cursorShape; s.cursorColorId = s.cursorColorId
    s.cursorBlink = s.cursorBlink; s.trailOnPredict = s.trailOnPredict
    s.keepConnectionsAlive = s.keepConnectionsAlive; s.moshByDefault = s.moshByDefault
    s.predictMode = s.predictMode; s.moshServerBinary = s.moshServerBinary
    s.udpRangeStart = s.udpRangeStart; s.udpRangeEnd = s.udpRangeEnd
    s.notificationsEnabled = s.notificationsEnabled; s.hostSetupDeclined = s.hostSetupDeclined
    s.liveActivityEnabled = s.liveActivityEnabled; s.attentionSoundEnabled = s.attentionSoundEnabled
    s.lockScreenDetailEnabled = s.lockScreenDetailEnabled; s.raiseKeyboardOnOpen = s.raiseKeyboardOnOpen
    s.remoteClipboardReadEnabled = s.remoteClipboardReadEnabled; s.uploadRetentionDays = s.uploadRetentionDays
    s.voiceInputEnabled = s.voiceInputEnabled; s.voiceInputLocaleId = s.voiceInputLocaleId
    s.voiceEngine = s.voiceEngine; s.whisperLanguage = s.whisperLanguage; s.whisperModelId = s.whisperModelId
}

/// What 1.0.8's one-piece decode of `key` reads.
func decoded<T: Decodable>(_ suite: Suite, _ key: String, _ type: T.Type, iso: Bool = false) -> [T]? {
    guard let data = suite.defaults.data(forKey: key) else { return [] }
    let decoder = JSONDecoder()
    if iso { decoder.dateDecodingStrategy = .iso8601 }
    return try? decoder.decode([T].self, from: data)
}

func view(_ store: Substring, _ suite: Suite, _ keychain: TestKeychain, _ m: Minted) -> String {
    switch store {
    case "conn":
        guard let all = decoded(suite, "moshpit.connections", ServerConnection.self, iso: true) else { return "undecodable" }
        return "conns=[" + all.map { connOut108($0, m) }.joined(separator: ";") + "]"
    case "keys":
        guard let all = decoded(suite, "moshpit.sshkeys.v1", SSHKeyRecord.self) else { return "undecodable" }
        return "keys=[" + all.map { keyOut108($0, m) }.joined(separator: ";") + "]"
    case "short":
        guard let all = decoded(suite, "moshpit.shortcuts.v1", TerminalShortcut.self) else { return "undecodable" }
        return "sc=[" + all.map { shortcutOut108($0, m) }.joined(separator: ";") + "]"
    case "theme":
        guard let all = decoded(suite, "moshpit.settings.customThemes", TerminalTheme.self) else { return "undecodable" }
        return "themes=[" + all.map { theme -> String in
            var t = theme
            t.isBuiltIn = false
            return themeOut108(t, m)
        }.joined(separator: ";") + "]"
    case "apptheme":
        let themes = AppThemeStore(defaults: suite.defaults, readiness: .always).customThemes
        if let data = suite.defaults.data(forKey: "moshpit.settings.customAppThemes"),
           ((try? JSONSerialization.jsonObject(with: data)) as? [Any])?.count != themes.count {
            return "undecodable"
        }
        return "themes=[" + themes.map { [m.out($0.id), m.out($0.name), out($0.accentHex), flag($0.isBuiltIn)]
            .joined(separator: "/") }.joined(separator: ";") + "]"
    case "settings":
        return settingsView(AppSettings(defaults: suite.defaults, readiness: .always), m)
    case "hosts":
        guard let data = keychain.items["moshpit.knownHosts"] else { return "map=[]" }
        guard let map = try? JSONDecoder().decode([String: String].self, from: data) else { return "undecodable" }
        return "map=[" + map.keys.sorted(by: utf8Less).map { m.out($0) + "=" + m.out(map[$0]!) }
            .joined(separator: ",") + "]"
    default: fatalError("no store \(store)")
    }
}

let touchID = UUID(uuid: (7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7))

/// Make 1.0.8 write the store back.
func touch(_ store: Substring, _ suite: Suite, _ keychain: TestKeychain) {
    switch store {
    case "conn":
        let s = ConnectionStore(defaults: suite.defaults, readiness: .always)
        if let first = s.connections.first { s.update(first) } else {
            s.add(ServerConnection(id: touchID, createdAt: Date(timeIntervalSinceReferenceDate: 0)))
            s.delete(id: touchID)
        }
    case "keys":
        let s = SSHKeyStore(defaults: suite.defaults, readiness: .always)
        s.add(SSHKeyRecord(id: touchID, name: "touch", algorithm: .ed25519, source: .generated,
                           createdAt: Date(timeIntervalSinceReferenceDate: 0)))
        s.remove(id: touchID)
    case "short":
        let s = ShortcutStore(defaults: suite.defaults, readiness: .always)
        if let first = s.shortcuts.first { s.update(first) }
    case "theme":
        let s = ThemeStore(defaults: suite.defaults, readiness: .always)
        var temp = TerminalTheme.fallback
        temp.id = "touch"
        temp.isBuiltIn = false
        s.save(temp)
        s.delete(id: "touch")
    case "apptheme":
        let s = AppThemeStore(defaults: suite.defaults, readiness: .always)
        s.save(AppTheme.custom(id: "touch", name: "touch", accentHex: "000000"))
        s.delete(id: "touch")
    case "settings":
        rewriteSettings(AppSettings(defaults: suite.defaults, readiness: .always))
    case "hosts":
        let s = KeychainKnownHostsStore(backend: keychain)
        s.write(s.read())
    default: fatalError("no store \(store)")
    }
}

/// 1.0.8's `PushPairingStore.read`: the whole array in one decode.
func pairingView(_ fileHex: Substring) -> String {
    guard let pairings = try? JSONDecoder().decode([PushPairing].self, from: Data(unhex(fileHex) ?? [])) else {
        return "undecodable"
    }
    if pairings.isEmpty { return "~" }
    return pairings.map { p in
        [p.connectionId.uuidString, out(p.hostLabel), out(p.secretHex), out(p.sendToken), out(p.relayURL),
         dateOut(p.createdAt), p.apnsToken.map(out) ?? "-", p.apnsEnv.map(out) ?? "-",
         p.sendTokenIssuedAt.map(dateOut) ?? "-"].joined(separator: "/")
    }.joined(separator: ";")
}

@main
struct Harness108 {
    static func main() {
        let args = CommandLine.arguments
        if args.count == 4, args[1] == "pairings" {
            let input = try! String(contentsOfFile: args[2], encoding: .utf8)
            let views = input.split(separator: "\n").map(pairingView)
            try! (views.joined(separator: "\n") + "\n").write(toFile: args[3], atomically: true, encoding: .utf8)
            return
        }
        guard args.count == 5, args[1] == "view" else {
            FileHandle.standardError.write(Data("usage: store-swift-108 view <in> <out> <finals>\n".utf8))
            exit(2)
        }
        let groupExisted = UserDefaults.standard.persistentDomain(forName: PushPrefs.suite) != nil
        let input = try! String(contentsOfFile: args[2], encoding: .utf8)
        var results: [String] = []
        var finals: [String] = []
        for (n, line) in input.split(separator: "\n").enumerated() where !line.isEmpty {
            let fields = line.split(separator: " ")
            let store = fields[0]
            let snapshot = parseSnapshot(fields[1])
            let m = Minted(snapshotTexts(snapshot) + [""])
            let suite = Suite(n)
            for (key, value) in snapshot.defaults { suite.defaults.set(value, forKey: key) }
            let keychain = TestKeychain(snapshot.keychain)
            groupSuite.removeObject(forKey: PushPrefs.detailKey)
            groupSuite.removeObject(forKey: PushPrefs.soundKey)
            let before = view(store, suite, keychain, m)
            touch(store, suite, keychain)
            let after = view(store, suite, keychain, m)
            results.append("\(before) || \(after)")
            finals.append("\(store) \(suite.token(keychain.items))")
            suite.remove()
        }
        try! (results.joined(separator: "\n") + "\n").write(toFile: args[3], atomically: true, encoding: .utf8)
        try! (finals.joined(separator: "\n") + "\n").write(toFile: args[4], atomically: true, encoding: .utf8)
        if !groupExisted { UserDefaults.standard.removePersistentDomain(forName: PushPrefs.suite) }
    }
}
