// Harness-only command line over the app's UNCHANGED storage sources (the
// files harness/build-swift.sh compiles, with the pieces of UIKit files it
// cuts verbatim), built into the app's own module so internal types are in
// reach. examples/store_harness.rs is the same command line over the Rust
// crate (src/harness.rs has the case format); harness/equivalence.py feeds
// both the same cases and compares.
//
// Built twice: as store-swift, with the stores on their own Swift code
// (swift/Stores.swift), and as store-swift-rust, with the same store
// classes on the app's Rust-store adapters over the Rust library
// (adapters/Stores.swift: Moshpit/Services/Rust/RustStores.swift, the
// MOSHPIT_RUST_STORE switch's side) — one command line, one set of cases,
// the store classes' own API either way.
//
//   store-swift run <cases> <out> <finals>
//
// Every case runs on a fresh UserDefaults suite (removed afterwards), so
// what is measured is Foundation's own bridging and storage. The App Group
// suite the settings mirror writes to is reset per case and, if this run
// created it, removed at the end.

import Foundation
import MoshpitKit
import SwiftUI

// MARK: - Dumps

func connOut(_ c: ServerConnection, _ m: Minted) -> String {
    [m.mask(c.id.uuidString), m.out(c.name), m.out(c.host), String(c.port), m.out(c.username),
     out(c.authMethod.rawValue), out(c.connectionProtocol.rawValue), String(c.sshPort),
     String(c.moshPortRangeStart), String(c.moshPortRangeEnd), m.opt(c.moshServerPath), m.opt(c.keychainRef),
     dateOut(c.createdAt), c.lastConnectedAt.map(dateOut) ?? "-", flag(c.useTmux), m.opt(c.predictModeRaw),
     optFlag(c.roamOnCellular), m.opt(c.tmuxPath), m.opt(c.herdrPath), m.opt(c.multiplexerRaw),
     c.sshKeyId.map { m.mask($0.uuidString) } ?? "-", m.opt(c.sshKeyAlgorithmRaw), optFlag(c.useSOCKSProxy),
     m.opt(c.socksProxyHost), c.socksProxyPort.map(String.init) ?? "-", out(c.multiplexer.rawValue),
     out(c.predictMode.rawValue), m.out(c.displayName)].joined(separator: "/")
}

func connState(_ store: ConnectionStore, _ m: Minted) -> String {
    let conns = store.connections.map { connOut($0, m) }.joined(separator: ";")
    let saved: String
    if let ids = store.savedIds {
        saved = ids.isEmpty ? "~" : ids.map { m.mask($0.uuidString) }.sorted().joined(separator: ",")
    } else {
        saved = "-"
    }
    return "auth=\(flag(store.isAuthoritative)) conns=[\(conns)] saved=\(saved)"
}

func keyOut(_ k: SSHKeyRecord, _ m: Minted) -> String {
    let hosts = k.boundHosts.map { m.out($0) }
    return [m.mask(k.id.uuidString), m.out(k.name), out(k.algorithm.rawValue), k.bits.map(String.init) ?? "-",
            m.out(k.comment), m.out(k.fingerprint), m.out(k.publicKey), out(k.source.rawValue),
            hosts.isEmpty ? "~" : hosts.joined(separator: ","), m.opt(k.keychainRef), flag(k.requireBiometry),
            dateOut(k.createdAt), k.lastUsedAt.map(dateOut) ?? "-", m.out(k.badgeText)].joined(separator: "/")
}

func keysState(_ store: SSHKeyStore, _ m: Minted) -> String {
    let keys = store.keys.map { keyOut($0, m) }.joined(separator: ";")
    return "auth=\(flag(store.isAuthoritative)) keys=[\(keys)] dev=\(store.deviceKeys.count) imp=\(store.importedKeys.count)"
}

func modifiersOut(_ s: TerminalShortcut) -> String {
    s.modifiers.isEmpty ? "~" : s.modifiers.sorted().map(\.rawValue).joined(separator: "+")
}

func sortedHexSet(_ strings: [String], _ m: Minted) -> String {
    strings.isEmpty ? "~" : strings.sorted(by: utf8Less).map { m.out($0) }.joined(separator: ",")
}

func shortcutOut(_ s: TerminalShortcut, _ m: Minted) -> String {
    [m.mask(s.id.uuidString), m.out(s.summary), flag(s.isBuiltin), s.kind.rawValue, modifiersOut(s), m.out(s.key),
     m.out(s.payload), flag(s.appendReturn), flag(s.repeatOnHold), m.out(s.chipLabel), m.out(s.colorId),
     sortedHexSet(Array(s.scopeHosts), m), flag(s.onlyInTmux), flag(s.inToolbar)].joined(separator: "/")
}

func shortState(_ store: ShortcutStore, _ m: Minted) -> String {
    let all = store.shortcuts.map { shortcutOut($0, m) }.joined(separator: ";")
    let bar = store.toolbar.map { m.mask($0.id.uuidString) }
    return "auth=\(flag(store.isAuthoritative)) sc=[\(all)] bar=\(bar.isEmpty ? "~" : bar.joined(separator: ",")) custom=\(store.custom.count) avail=\(store.available.count)"
}

func themeOut(_ t: TerminalTheme, _ m: Minted) -> String {
    [m.out(t.id), m.out(t.name), out(t.backgroundHex), out(t.foregroundHex), out(t.cursorHex),
     t.ansiHex.map(out).joined(separator: ","), t.brightHex.map { $0.map(out).joined(separator: ",") } ?? "-",
     flag(t.isBuiltIn)].joined(separator: "/")
}

func themeState(_ store: ThemeStore, _ m: Minted) -> String {
    "auth=\(flag(store.isAuthoritative)) themes=[\(store.customThemes.map { themeOut($0, m) }.joined(separator: ";"))]"
}

func appThemeOut(_ t: AppTheme, _ m: Minted) -> String {
    [m.out(t.id), m.out(t.name), out(t.accentHex), flag(t.isBuiltIn)].joined(separator: "/")
}

func appThemeState(_ store: AppThemeStore, _ m: Minted) -> String {
    "auth=\(flag(store.isAuthoritative)) themes=[\(store.customThemes.map { appThemeOut($0, m) }.joined(separator: ";"))]"
}

/// `AppSettings`' properties in `ALL`'s order (settings.rs), as typed values.
let settingNames = ["fontSize", "themeId", "appThemeId", "appIconId", "fontName", "cursorShape", "cursorColorId",
                    "cursorBlink", "trailOnPredict", "keepConnectionsAlive", "moshByDefault", "predictMode",
                    "moshServerBinary", "udpRangeStart", "udpRangeEnd", "notificationsEnabled", "hostSetupDeclined",
                    "pushRelayURL", "liveActivityEnabled", "attentionSoundEnabled", "lockScreenDetailEnabled",
                    "raiseKeyboardOnOpen", "remoteClipboardReadEnabled", "uploadRetentionDays", "voiceInputEnabled",
                    "voiceInputLocaleId", "voiceEngine", "whisperLanguage", "whisperModelId", "appearance"]

func settingValue(_ s: AppSettings, _ name: String, _ m: Minted) -> String {
    func b(_ v: Bool) -> String { "b" + flag(v) }
    func i(_ v: Int) -> String { "i\(v)" }
    func t(_ v: String) -> String { "s" + m.out(v) }
    switch name {
    case "fontSize": return "f" + bits(s.fontSize)
    case "themeId": return t(s.themeId)
    case "appThemeId": return t(s.appThemeId)
    case "appIconId": return t(s.appIconId)
    case "fontName": return t(s.fontName)
    case "cursorShape": return t(s.cursorShape.rawValue)
    case "cursorColorId": return t(s.cursorColorId)
    case "cursorBlink": return b(s.cursorBlink)
    case "trailOnPredict": return b(s.trailOnPredict)
    case "keepConnectionsAlive": return b(s.keepConnectionsAlive)
    case "moshByDefault": return b(s.moshByDefault)
    case "predictMode": return t(s.predictMode.rawValue)
    case "moshServerBinary": return t(s.moshServerBinary)
    case "udpRangeStart": return i(s.udpRangeStart)
    case "udpRangeEnd": return i(s.udpRangeEnd)
    case "notificationsEnabled": return b(s.notificationsEnabled)
    case "hostSetupDeclined": return "a" + sortedHexSet(Array(s.hostSetupDeclined), m)
    case "pushRelayURL": return t(s.pushRelayURL)
    case "liveActivityEnabled": return b(s.liveActivityEnabled)
    case "attentionSoundEnabled": return b(s.attentionSoundEnabled)
    case "lockScreenDetailEnabled": return b(s.lockScreenDetailEnabled)
    case "raiseKeyboardOnOpen": return b(s.raiseKeyboardOnOpen)
    case "remoteClipboardReadEnabled": return b(s.remoteClipboardReadEnabled)
    case "uploadRetentionDays": return i(s.uploadRetentionDays)
    case "voiceInputEnabled": return b(s.voiceInputEnabled)
    case "voiceInputLocaleId": return t(s.voiceInputLocaleId)
    case "voiceEngine": return t(s.voiceEngine.rawValue)
    case "whisperLanguage": return t(s.whisperLanguage)
    case "whisperModelId": return t(s.whisperModelId)
    case "appearance": return t(s.appearance.rawValue)
    default: fatalError("no setting \(name)")
    }
}

func putSetting(_ s: AppSettings, _ name: String, _ token: Substring) {
    let text = arg(token)
    let bool = text == "1"
    switch name {
    case "fontSize": s.fontSize = Double(bitPattern: UInt64(text, radix: 16)!)
    case "themeId": s.themeId = text
    case "appThemeId": s.appThemeId = text
    case "appIconId": s.appIconId = text
    case "fontName": s.fontName = text
    case "cursorShape": if let v = CursorShape(rawValue: text) { s.cursorShape = v } else { s.store.set(text, forKey: "moshpit.settings.cursorShape") }
    case "cursorColorId": s.cursorColorId = text
    case "cursorBlink": s.cursorBlink = bool
    case "trailOnPredict": s.trailOnPredict = bool
    case "keepConnectionsAlive": s.keepConnectionsAlive = bool
    case "moshByDefault": s.moshByDefault = bool
    case "predictMode": if let v = PredictMode(rawValue: text) { s.predictMode = v } else { s.store.set(text, forKey: "moshpit.settings.predictMode") }
    case "moshServerBinary": s.moshServerBinary = text
    case "udpRangeStart": s.udpRangeStart = Int(text)!
    case "udpRangeEnd": s.udpRangeEnd = Int(text)!
    case "notificationsEnabled": s.notificationsEnabled = bool
    case "hostSetupDeclined":
        let items = (try? JSONSerialization.jsonObject(with: Data(text.utf8))) as? [Any] ?? []
        s.hostSetupDeclined = Set(items.compactMap { $0 as? String })
    case "pushRelayURL": s.pushRelayURL = text
    case "liveActivityEnabled": s.liveActivityEnabled = bool
    case "attentionSoundEnabled": s.attentionSoundEnabled = bool
    case "lockScreenDetailEnabled": s.lockScreenDetailEnabled = bool
    case "raiseKeyboardOnOpen": s.raiseKeyboardOnOpen = bool
    case "remoteClipboardReadEnabled": s.remoteClipboardReadEnabled = bool
    case "uploadRetentionDays": s.uploadRetentionDays = Int(text)!
    case "voiceInputEnabled": s.voiceInputEnabled = bool
    case "voiceInputLocaleId": s.voiceInputLocaleId = text
    case "voiceEngine": if let v = VoiceEngineKind(rawValue: text) { s.voiceEngine = v } else { s.store.set(text, forKey: "moshpit.settings.voiceEngine") }
    case "whisperLanguage": s.whisperLanguage = text
    case "whisperModelId": s.whisperModelId = text
    case "appearance": if let v = AppAppearance(rawValue: text) { s.appearance = v } else { s.store.set(text, forKey: "moshpit.settings.appearance") }
    default: fatalError("no setting \(name)")
    }
}

let groupSuite = UserDefaults(suiteName: PushPrefs.suite)!

func settingsState(_ s: AppSettings, _ m: Minted) -> String {
    let vals = settingNames.map { "\($0)=\(settingValue(s, $0, m))" }.joined(separator: ";")
    let detail = groupSuite.object(forKey: PushPrefs.detailKey) as? Bool
    let sound = groupSuite.object(forKey: PushPrefs.soundKey) as? Bool
    return "vals=[\(vals)] mirror=\(optFlag(detail))/\(optFlag(sound))"
}

// MARK: - Running

final class Phone { var unlocked: Bool; init(_ u: Bool) { unlocked = u } }

func jsonData(_ token: Substring) -> Data { Data(arg(token).utf8) }

func isoDecoder() -> JSONDecoder {
    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    return decoder
}

/// `@n` modulo the count, or nil.
func indexArg(_ token: Substring, _ count: Int) -> Int? {
    guard token.hasPrefix("@"), let n = Int(token.dropFirst()), count > 0 else { return nil }
    return n % count
}

func commonOp(_ parts: [Substring], _ suite: Suite, _ phone: Phone) -> String? {
    switch parts[0] {
    case "unlock": phone.unlocked = true
    case "lock": phone.unlocked = false
    case "set":
        let value = try! JSONSerialization.jsonObject(with: jsonData(parts[2]))
        suite.defaults.set(plistFromJSON(value), forKey: arg(parts[1]))
    case "del": suite.defaults.removeObject(forKey: arg(parts[1]))
    default: return nil
    }
    return String(parts[0])
}

func segment(_ name: Substring, _ result: String, _ state: String, _ snap: String) -> String {
    "\(name)=\(result) \(state) \(snap)"
}

let touchID = UUID(uuid: (7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7))

func runConn(_ suite: Suite, _ phone: Phone, _ ops: [Substring], _ m: Minted) -> String {
    var store = makeConnectionStore(suite.defaults, phone)
    var segments = [segment("init", "ok", connState(store, m), suite.snap(m))]
    for op in ops {
        let parts = op.split(separator: ":", omittingEmptySubsequences: false)
        var result = commonOp(parts, suite, phone) ?? "ok"
        switch parts[0] {
        case "unlock", "lock", "set", "del": break
        case "fresh": store = makeConnectionStore(suite.defaults, phone)
        case "reload": store.reloadIfNeeded()
        case "add", "update":
            if let c = try? isoDecoder().decode(ServerConnection.self, from: jsonData(parts[1])) {
                if parts[0] == "add" { store.add(c) } else { store.update(c) }
            } else { result = "undecodable" }
        case "touch":
            if let first = store.connections.first { store.update(first) } else {
                store.add(ServerConnection(id: touchID, createdAt: Date(timeIntervalSinceReferenceDate: 0)))
                store.delete(id: touchID)
            }
        case "delete":
            let id: UUID?
            if let i = indexArg(parts[1], store.connections.count) { id = store.connections[i].id }
            else if parts[1].hasPrefix("@") { id = nil }
            else { id = UUID(uuidString: arg(parts[1])) }
            if let id { store.delete(id: id) } else { result = "skip" }
        default: fatalError("no conn op \(parts[0])")
        }
        segments.append(segment(parts[0], result, connState(store, m), suite.snap(m)))
    }
    return segments.joined(separator: " | ")
}

func runKeys(_ suite: Suite, _ phone: Phone, _ ops: [Substring], _ m: Minted) -> String {
    var store = makeKeyStore(suite.defaults, phone)
    var segments = [segment("init", "ok", keysState(store, m), suite.snap(m))]
    for op in ops {
        let parts = op.split(separator: ":", omittingEmptySubsequences: false)
        var result = commonOp(parts, suite, phone) ?? "ok"
        switch parts[0] {
        case "unlock", "lock", "set", "del": break
        case "fresh": store = makeKeyStore(suite.defaults, phone)
        case "reload": store.reloadIfNeeded()
        case "add":
            if let k = try? JSONDecoder().decode(SSHKeyRecord.self, from: jsonData(parts[1])) { store.add(k) }
            else { result = "undecodable" }
        case "touch":
            store.add(SSHKeyRecord(id: touchID, name: "touch", algorithm: .ed25519, source: .generated,
                                   createdAt: Date(timeIntervalSinceReferenceDate: 0)))
            store.remove(id: touchID)
        case "remove":
            let id: UUID?
            if let i = indexArg(parts[1], store.keys.count) { id = store.keys[i].id }
            else if parts[1].hasPrefix("@") { id = nil }
            else { id = UUID(uuidString: arg(parts[1])) }
            if let id { store.remove(id: id) } else { result = "skip" }
        default: fatalError("no keys op \(parts[0])")
        }
        segments.append(segment(parts[0], result, keysState(store, m), suite.snap(m)))
    }
    return segments.joined(separator: " | ")
}

func runShort(_ suite: Suite, _ phone: Phone, _ ops: [Substring], _ m: Minted) -> String {
    var store = makeShortcutStore(suite.defaults, phone)
    var segments = [segment("init", "ok", shortState(store, m), suite.snap(m))]
    for op in ops {
        let parts = op.split(separator: ":", omittingEmptySubsequences: false)
        var result = commonOp(parts, suite, phone) ?? "ok"
        func nth(_ token: Substring) -> TerminalShortcut? {
            indexArg(token, store.shortcuts.count).map { store.shortcuts[$0] }
        }
        switch parts[0] {
        case "unlock", "lock", "set", "del": break
        case "fresh": store = makeShortcutStore(suite.defaults, phone)
        case "reload": store.reloadIfNeeded()
        case "restore": store.restoreDefaults()
        case "touch":
            if let first = store.shortcuts.first { store.update(first) } else { store.restoreDefaults() }
        case "add":
            if let s = try? JSONDecoder().decode(TerminalShortcut.self, from: jsonData(parts[1])) { store.add(s) }
            else { result = "undecodable" }
        case "update":
            let target = nth(parts[1])
            let decoded = try? JSONDecoder().decode(TerminalShortcut.self, from: jsonData(parts[2]))
            if let target, var shortcut = decoded {
                shortcut.id = target.id
                shortcut.inToolbar = target.inToolbar
                store.update(shortcut)
            } else { result = target == nil ? "skip" : "undecodable" }
        case "remove", "addbar", "rmbar":
            if let target = nth(parts[1]) {
                switch parts[0] {
                case "remove": store.remove(id: target.id)
                case "addbar": store.addToToolbar(id: target.id)
                default: store.removeFromToolbar(id: target.id)
                }
            } else { result = "skip" }
        case "move":
            store.reloadIfNeeded()
            let bar = store.toolbarCount
            if bar == 0 { result = "skip" } else {
                let from = IndexSet(parts[1].split(separator: ",").map { Int($0)! % bar })
                store.moveInToolbar(fromOffsets: from, toOffset: Int(parts[2])! % (bar + 1))
            }
        case "bar":
            let host: String? = parts[1] == "-" ? nil : arg(parts[1])
            let ids = store.toolbar(forHost: host, inMultiplexer: parts[2] == "1").map { m.mask($0.id.uuidString) }
            result = ids.isEmpty ? "~" : ids.joined(separator: ",")
        default: fatalError("no short op \(parts[0])")
        }
        segments.append(segment(parts[0], result, shortState(store, m), suite.snap(m)))
    }
    return segments.joined(separator: " | ")
}

func runTheme(_ suite: Suite, _ phone: Phone, _ ops: [Substring], _ m: Minted) -> String {
    var store = makeThemeStore(suite.defaults, phone)
    var segments = [segment("init", "ok", themeState(store, m), suite.snap(m))]
    for op in ops {
        let parts = op.split(separator: ":", omittingEmptySubsequences: false)
        var result = commonOp(parts, suite, phone) ?? "ok"
        switch parts[0] {
        case "unlock", "lock", "set", "del": break
        case "fresh": store = makeThemeStore(suite.defaults, phone)
        case "reload": store.reloadIfNeeded()
        case "save":
            if let t = try? JSONDecoder().decode(TerminalTheme.self, from: jsonData(parts[1])) { store.save(t) }
            else { result = "undecodable" }
        case "savebuiltin": store.save(TerminalTheme.builtIns[Int(parts[1])! % 8])
        case "touch":
            var temp = TerminalTheme.fallback
            temp.id = "touch"
            temp.isBuiltIn = false
            store.save(temp)
            store.delete(id: "touch")
        case "delete": store.delete(id: arg(parts[1]))
        case "removeall": store.removeAllCustom()
        case "dup":
            let all = store.allThemes
            let copy = store.duplicate(all[indexArg(parts[1], all.count)!])
            result = themeOut(copy, m)
            store.save(copy)
        case "draft":
            let draft = store.makeDraft()
            result = themeOut(draft, m)
            store.save(draft)
        case "get": result = themeOut(store.theme(id: arg(parts[1])), m)
        case "import":
            do {
                result = try store.importThemes(json: arg(parts[1])).map { themeOut($0, m) }.joined(separator: ";")
            } catch ThemeStore.ImportError.notJSON { result = "notJSON" }
            catch ThemeStore.ImportError.noThemes { result = "noThemes" }
            catch { result = "other" }
        default: fatalError("no theme op \(parts[0])")
        }
        segments.append(segment(parts[0], result, themeState(store, m), suite.snap(m)))
    }
    return segments.joined(separator: " | ")
}

func runAppTheme(_ suite: Suite, _ phone: Phone, _ ops: [Substring], _ m: Minted) -> String {
    var store = makeAppThemeStore(suite.defaults, phone)
    var segments = [segment("init", "ok", appThemeState(store, m), suite.snap(m))]
    for op in ops {
        let parts = op.split(separator: ":", omittingEmptySubsequences: false)
        var result = commonOp(parts, suite, phone) ?? "ok"
        switch parts[0] {
        case "unlock", "lock", "set", "del": break
        case "fresh": store = makeAppThemeStore(suite.defaults, phone)
        case "reload": store.reloadIfNeeded()
        case "save": store.save(AppTheme.custom(id: arg(parts[1]), name: arg(parts[2]), accentHex: arg(parts[3])))
        case "savebuiltin": store.save(AppThemeCatalog.builtIns[Int(parts[1])! % 4])
        case "touch":
            store.save(AppTheme.custom(id: "touch", name: "touch", accentHex: "000000"))
            store.delete(id: "touch")
        case "delete": store.delete(id: arg(parts[1]))
        case "removeall": store.removeAllCustom()
        case "draft":
            let id = arg(parts[1])
            // `AppThemeCatalog.theme(for:)` reads the shared store; this one
            // is resolved the same way over the store under test.
            let base = (AppThemeCatalog.builtIns + store.customThemes).first { $0.id == id } ?? AppThemeCatalog.signalRoom
            let draft = store.makeDraft(basedOn: base)
            result = appThemeOut(draft, m)
            store.save(draft)
        default: fatalError("no apptheme op \(parts[0])")
        }
        segments.append(segment(parts[0], result, appThemeState(store, m), suite.snap(m)))
    }
    return segments.joined(separator: " | ")
}

func runSettings(_ suite: Suite, _ phone: Phone, _ ops: [Substring], _ m: Minted) -> String {
    groupSuite.removeObject(forKey: PushPrefs.detailKey)
    groupSuite.removeObject(forKey: PushPrefs.soundKey)
    var settings = makeSettings(suite.defaults, phone)
    var segments = [segment("init", "ok", settingsState(settings, m), suite.snap(m))]
    for op in ops {
        let parts = op.split(separator: ":", omittingEmptySubsequences: false)
        let result = commonOp(parts, suite, phone) ?? "ok"
        switch parts[0] {
        case "unlock", "lock", "set", "del": break
        case "fresh": settings = makeSettings(suite.defaults, phone)
        case "reload", "touch": settings.reloadIfNeeded()
        case "put": putSetting(settings, arg(parts[1]), parts[2])
        default: fatalError("no settings op \(parts[0])")
        }
        segments.append(segment(parts[0], result, settingsState(settings, m), suite.snap(m)))
    }
    return segments.joined(separator: " | ")
}

func tableOut(_ table: [String: String], _ m: Minted) -> String {
    table.keys.sorted(by: utf8Less).map { m.out($0) + "=" + m.out(table[$0]!) }.joined(separator: ",")
}

func runHosts(_ keychain: TestKeychain, _ ops: [Substring], _ m: Minted) -> String {
    let store = makeKnownHosts(keychain)
    var segments = ["init=ok \(kcOut(keychain, m))"]
    for op in ops {
        let parts = op.split(separator: ":", omittingEmptySubsequences: false)
        var result = "ok"
        func failure(_ error: Error) -> String {
            (error as? KnownHostsUnreadable)?.locked == true ? "locked" : "unavailable"
        }
        switch parts[0] {
        case "read":
            do { result = "[" + tableOut(try store.read(), m) + "]" } catch { result = failure(error) }
        case "write":
            let object = (try? JSONSerialization.jsonObject(with: jsonData(parts[1]))) as? [String: Any] ?? [:]
            do { try store.write(object.compactMapValues { $0 as? String }) } catch {
                result = (error as? KeychainError) == .interactionNotAllowed ? "locked" : "unavailable"
            }
        case "touch":
            do {
                let table = try store.read()
                do { try store.write(table) } catch { result = "unwritable" }
            } catch { result = "unreadable" }
        case "fresh": break
        case "kclock": keychain.locked = true
        case "kcunlock": keychain.locked = false
        default: fatalError("no hosts op \(parts[0])")
        }
        segments.append("\(parts[0])=\(result) \(kcOut(keychain, m))")
    }
    return segments.joined(separator: " | ")
}

func runReady(_ suite: Suite, _ phone: Phone, _ ops: [Substring], _ m: Minted) -> String {
    var segments = ["init=ok \(suite.snap(m))"]
    for op in ops {
        let parts = op.split(separator: ":", omittingEmptySubsequences: false)
        var result = commonOp(parts, suite, phone) ?? "ok"
        switch parts[0] {
        case "unlock", "lock", "set", "del": break
        case "check": result = flag(readinessCheck(suite.defaults, phone))
        case "mark": readinessMark(suite.defaults, phone)
        case "touch", "fresh": break
        default: fatalError("no ready op \(parts[0])")
        }
        segments.append("\(parts[0])=\(result) \(suite.snap(m))")
    }
    return segments.joined(separator: " | ")
}

func bytesOp(_ op: Substring) -> String {
    let parts = op.split(separator: ":", omittingEmptySubsequences: false)
    guard let s = try? JSONDecoder().decode(TerminalShortcut.self, from: jsonData(parts[1])) else {
        return "bytes=undecodable"
    }
    let stages = s.encodedStages().map { hex([UInt8]($0)) }
    return "bytes=\(stages.isEmpty ? "~" : stages.joined(separator: ","))"
}

func hexOp(_ op: Substring) -> String {
    let parts = op.split(separator: ":", omittingEmptySubsequences: false)
    func f(_ token: Substring) -> Double { Double(bitPattern: UInt64(token, radix: 16)!) }
    let value: String
    switch parts[0] {
    case "norm": value = out(Color.normalizedHex(arg(parts[1])))
    case "rgb":
        let (r, g, b) = Color.rgbComponents(hex: arg(parts[1]))
        value = "\(bits(r))/\(bits(g))/\(bits(b))"
    case "blend": value = out(Color.blend(arg(parts[1]), toward: arg(parts[2]), by: f(parts[3])))
    case "bright": value = out(TerminalTheme.derivedBright(from: arg(parts[1])))
    case "hexstr": value = out(Color.hexString(r: f(parts[1]), g: f(parts[2]), b: f(parts[3])))
    case "custom":
        // AppTheme keeps its derived colours as `Color`, which can't be read
        // back without UIKit: the three blends `AppTheme.custom` makes, with
        // its constants, from the accent it stores.
        let theme = AppTheme.custom(id: "x", name: "x", accentHex: arg(parts[1]))
        value = [theme.accentHex, Color.blend(theme.accentHex, toward: "000000", by: 0.18),
                 Color.blend(theme.accentHex, toward: "000000", by: 0.955),
                 Color.blend(theme.accentHex, toward: "000000", by: 0.982)].map(out).joined(separator: "/")
    default: fatalError("no hex op \(parts[0])")
    }
    return "\(parts[0])=\(value)"
}

func exportOp(_ op: Substring, _ scratch: UserDefaults) -> String {
    let parts = op.split(separator: ":", omittingEmptySubsequences: false)
    guard let theme = try? JSONDecoder().decode(TerminalTheme.self, from: jsonData(parts[1])) else {
        return "export=undecodable"
    }
    let m = Minted([arg(parts[1])])
    let text = try! makeThemeStore(scratch, nil).exportJSON(theme)
    return "export=\(out(m.mask(text)))"
}

func runCase(_ line: Substring, _ n: Int) -> (String, String) {
    let fields = line.split(separator: " ", omittingEmptySubsequences: false)
    let store = fields[0]
    let snapshot = parseSnapshot(fields[2])
    let ops = Array(fields.dropFirst(3))
    var inputs = snapshotTexts(snapshot)
    for op in ops {
        for part in op.split(separator: ":", omittingEmptySubsequences: false).dropFirst() {
            if let bytes = unhex(part) { inputs.append(String(decoding: bytes, as: UTF8.self)) }
        }
    }
    let m = Minted(inputs)
    let suite = Suite(n)
    defer { suite.remove() }
    for (key, value) in snapshot.defaults { suite.defaults.set(value, forKey: key) }
    let keychain = TestKeychain(snapshot.keychain)
    let phone = Phone(fields[1] == "U")
    let result: String
    switch store {
    case "conn": result = runConn(suite, phone, ops, m)
    case "keys": result = runKeys(suite, phone, ops, m)
    case "short": result = runShort(suite, phone, ops, m)
    case "theme": result = runTheme(suite, phone, ops, m)
    case "apptheme": result = runAppTheme(suite, phone, ops, m)
    case "settings": result = runSettings(suite, phone, ops, m)
    case "hosts": result = runHosts(keychain, ops, m)
    case "ready": result = runReady(suite, phone, ops, m)
    case "bytes": result = ops.map(bytesOp).joined(separator: " | ")
    case "hex": result = ops.map(hexOp).joined(separator: " | ")
    case "export": result = ops.map { exportOp($0, suite.defaults) }.joined(separator: " | ")
    default: fatalError("no store \(store)")
    }
    return (result, "\(store) \(suite.token(keychain.items))")
}

@main
struct Harness {
    static func main() {
        let args = CommandLine.arguments
        if runOtherCommand(args) { return }
        guard args.count == 5, args[1] == "run" else {
            FileHandle.standardError.write(Data("usage: store-swift run <cases> <out> <finals>\n".utf8))
            exit(2)
        }
        let groupExisted = UserDefaults.standard.persistentDomain(forName: PushPrefs.suite) != nil
        let cases = try! String(contentsOfFile: args[2], encoding: .utf8)
        var results: [String] = []
        var finals: [String] = []
        for (n, line) in cases.split(separator: "\n").enumerated() where !line.isEmpty {
            let (result, last) = runCase(line, n)
            results.append(result)
            finals.append(last)
        }
        try! (results.joined(separator: "\n") + "\n").write(toFile: args[3], atomically: true, encoding: .utf8)
        try! (finals.joined(separator: "\n") + "\n").write(toFile: args[4], atomically: true, encoding: .utf8)
        if !groupExisted { UserDefaults.standard.removePersistentDomain(forName: PushPrefs.suite) }
    }
}
