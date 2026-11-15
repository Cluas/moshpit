import Foundation
import SwiftUI
import Testing
import UIKit
@testable import Moshpit

/// SSH Keys, Add Key, Select Key, Shortcuts, the shortcut editor and Set up
/// this host in NSH: their switches; the data path between the host and each
/// screen — every state the Swift side posts is a message the Rust screen
/// takes (`ui/src/ssh_keys.rs`, `add_key.rs`, `key_picker.rs`,
/// `shortcuts.rs`, `add_shortcut.rs`, `host_setup.rs` test the screens,
/// `ui/tests/` run them), no message carries key material, and what the
/// screens tell the host is applied as the SwiftUI pages apply it, through
/// the same stores — and real surfaces on the device with UIKit's menus,
/// switches and the host's own key fields.
@Suite("NSH keys, shortcuts and host setup")
@MainActor
struct KeysShortcutsSetupNSHTests {

    /// Defaults of their own, so a test writes nothing the app keeps.
    private static func scratch() -> UserDefaults {
        UserDefaults(suiteName: "w11e.nsh.\(UUID().uuidString)")!
    }

    private static func key(_ name: String, source: SSHKeySource = .imported, algorithm: SSHKeyAlgorithm = .ed25519,
                            publicKey: String = "ssh-ed25519 AAAAC3Nza test",
                            lastUsed: Date? = nil) -> SSHKeyRecord {
        SSHKeyRecord(name: name, algorithm: algorithm, comment: "", fingerprint: "SHA256:\(name)",
                     publicKey: publicKey, source: source, lastUsedAt: lastUsed)
    }

    // MARK: Screens

    @Test("each page runs the Rust screen of its name")
    func names() {
        let screens: [NSHScreen] = [.sshKeys, .addKey, .keyPicker, .shortcuts, .addShortcut, .hostSetup]
        #expect(screens.map(\.name) == ["ssh-keys", "add-key", "key-picker", "shortcuts", "add-shortcut", "host-setup"])
    }

    // MARK: SSH Keys

    @Test("every SSH Keys state is a message the Rust screen takes, with metadata only")
    func keysMessages() throws {
        let now = Date()
        let stale = Self.key("old", algorithm: .rsa4096, publicKey: "ssh-rsa AAAAB3Nza old",
                             lastUsed: now.addingTimeInterval(-100 * 86400))
        let se = Self.key("iPad", source: .secureEnclave, algorithm: .seP256)
        let plain = Self.key("laptop")
        let snapshot = SSHKeysSnapshot(device: [se], imported: [plain, stale],
                                       users: [se.id: ["m1-pro"], plain.id: ["m1-pro", "nas"]])
        for deleting in [nil, plain.id] {
            let state = SSHKeysModel.state(snapshot, deleting: deleting, now: now)
            #expect(state.deleting == deleting?.uuidString)
            #expect(NSHSurface.check(screen: NSHScreen.sshKeys.name,
                                     json: try pageMessage(message: .sshKeys(state: state))))
        }
        let state = SSHKeysModel.state(snapshot, deleting: nil, now: now)
        try #require(state.imported.count == 2 && state.device.count == 1)
        #expect(state.imported[1].avatar == .gray, "an RSA key unused for 90 days")
        #expect(state.imported[1].stale)
        #expect(state.imported[1].unusedSince != nil)
        #expect(state.imported[0].avatar == .imported)
        #expect(state.imported[0].hosts == ["m1-pro", "nas"])
        #expect(state.device[0].avatar == .secureEnclave)
        #expect(state.device[0].badge == "SE · P256")
        // Metadata only: no key, no Keychain reference.
        let json = try pageMessage(message: .sshKeys(state: state))
        #expect(!json.contains("publicKey\":\"ssh"))
        #expect(!json.contains("keychainRef"))
        #expect(!NSHSurface.check(screen: NSHScreen.sshKeys.name, message: ["op": "keys", "device": []]))
    }

    /// B602: a key's row said "used for" the hosts picked once at Add Key —
    /// a label stored on the key that bound nothing — whatever the
    /// connections said, stale after any rename or delete. It lists the
    /// saved connections that sign in with the key: by its id, or by the
    /// keychain item it holds.
    @Test("a key's row lists the connections that sign in with it, not hosts picked at Add Key")
    func keyRowListsItsUsers() {
        let defaults = Self.scratch()
        let keys = SSHKeyStore(defaults: defaults, readiness: .always)
        let connections = ConnectionStore(defaults: defaults)
        var laptop = Self.key("laptop")
        laptop.boundHosts = ["old-box"]
        laptop.keychainRef = "moshpit.secret.laptop"
        keys.add(laptop)
        var nas = ServerConnection(name: "nas", host: "192.0.2.7", username: "me", authMethod: .key,
                                   keychainRef: "moshpit.secret.laptop")
        nas.sshKeyId = laptop.id
        connections.add(nas)
        connections.add(ServerConnection(name: "pi", host: "192.0.2.8", username: "me", authMethod: .key,
                                         keychainRef: "moshpit.secret.laptop"))
        connections.add(ServerConnection(name: "box", host: "192.0.2.9", username: "me", authMethod: .password))
        let state = SSHKeysModel.state(SSHKeysSnapshot(store: keys, connections: connections), deleting: nil)
        #expect(state.imported.first?.hosts == ["nas", "pi"])
    }

    @Test("the card asks about a key; Delete deletes it — record and private key — as the SwiftUI page does")
    func keysCard() async throws {
        let defaults = Self.scratch()
        let store = SSHKeyStore(defaults: defaults, readiness: .always)
        let connections = ConnectionStore(defaults: defaults)
        var laptop = Self.key("laptop")
        laptop.keychainRef = "moshpit.secret.laptop"
        store.add(laptop)
        let keychain = KeychainService.inMemory(seed: ["moshpit.secret.laptop": Data("PRIVATE".utf8)])
        let model = SSHKeysModel()
        model.update(SSHKeysSnapshot(store: store, connections: connections))
        model.confirmDelete(laptop.id)
        #expect(model.deleting == laptop.id)
        model.cancelDelete()
        #expect(model.deleting == nil)
        model.confirmDelete(laptop.id)
        model.delete(laptop, from: store, connections: connections) { try await keychain.delete(forRef: $0) }
        #expect(model.deleting == nil)
        let deadline = ContinuousClock.now + .seconds(5)
        while !store.keys.isEmpty, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(5))
        }
        #expect(store.keys.isEmpty)
        #expect(await !keychain.contains(ref: "moshpit.secret.laptop"),
                "the deleted key's private key stayed in the keychain (B232)")
        #expect(model.notice == nil)
        // A card about a key gone from the store goes with it.
        let other = Self.key("other")
        store.add(other)
        model.update(SSHKeysSnapshot(store: store, connections: connections))
        model.confirmDelete(other.id)
        store.remove(id: other.id)
        model.update(SSHKeysSnapshot(store: store, connections: connections))
        #expect(model.deleting == nil)
        #expect(SSHKeysModel.rowID(other.id) == "ssh-key-row-\(other.id.uuidString)")
    }

    @Test("Copy Public Key and Copy Fingerprint put the key's own text on the pasteboard")
    func keysCopy() {
        let pasteboard = UIPasteboard.withUniqueName()
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        SSHKeysModel.copy("ssh-ed25519 AAAA", pasteboard: pasteboard)
        #expect(pasteboard.string == "ssh-ed25519 AAAA")
        SSHKeysModel.copy("", pasteboard: pasteboard)
        #expect(pasteboard.string == "ssh-ed25519 AAAA", "nothing to copy changes nothing")
    }

    // MARK: Add Key

    @Test("every Add Key state is a message the Rust screen takes; the key itself never is")
    func addKeyMessages() throws {
        let model = AddKeyModel()
        #expect(NSHSurface.check(screen: NSHScreen.addKey.name,
                                 json: try pageMessage(message: .addKey(state: model.page))))
        #expect(model.apply(.method(id: "import")))
        #expect(model.method == .importKey)
        #expect(model.apply(.name(text: "work")))
        #expect(model.apply(.publicLine(text: "ssh-ed25519 AAAA work")))
        #expect(model.apply(.set(key: .requireFaceId, on: false)))
        #expect(!model.requireFaceID)
        #expect(model.apply(.algorithm(id: "rsa")))
        #expect(model.algorithm == .rsa4096)
        #expect(!model.apply(.algorithm(id: "dsa")))
        #expect(!model.apply(.submit), "the page's own")
        let page = model.page
        let json = try pageMessage(message: .addKey(state: page))
        #expect(NSHSurface.check(screen: NSHScreen.addKey.name, json: json), "\(json)")
        #expect(page.pem == AddKeyPem(empty: true, encrypted: false))
        #expect(!page.canSubmit, "no key pasted")
        var bad = Self.object(json)
        #expect(bad.keys.allSatisfy { !["privateKey", "passphrase", "pemText"].contains($0) })
        bad["privateKey"] = "-----BEGIN OPENSSH PRIVATE KEY-----"
        #expect(!NSHSurface.check(screen: NSHScreen.addKey.name, message: bad), "the screen refuses key material")
    }

    @Test("Generate makes the key, its private half goes to the Keychain, its record to the store")
    func addKeyGenerates() async throws {
        let store = SSHKeyStore(defaults: Self.scratch(), readiness: .always)
        let keychain = KeychainService.inMemory()
        let model = AddKeyModel()
        #expect(model.apply(.name(text: "phone")))
        #expect(model.canSubmit)
        let record = try #require(await model.submit(to: store, keychain: keychain))
        #expect(record.name == "phone")
        #expect(record.algorithm == .ed25519)
        #expect(record.source == .generated)
        #expect(record.boundHosts.isEmpty, "Add Key binds nothing (B602)")
        #expect(record.publicKey.hasPrefix("ssh-ed25519 "))
        #expect(store.keys.map(\.id) == [record.id])
        let ref = try #require(record.keychainRef)
        let secret = try await keychain.loadSecret(forRef: ref)
        #expect(!secret.isEmpty)
        #expect(model.page.fingerprint == record.fingerprint)
    }

    /// Where a key generation ran.
    private final class Seen: @unchecked Sendable {
        var onMain: Bool?
    }

    /// B605: an import was unlocked off the main thread, a generation made
    /// on it — an RSA-4096 key's search for its primes froze the sheet,
    /// spinner and all, for seconds on a phone.
    @Test("a key is made off the main thread, as an import is")
    func keyGenerationLeavesTheMainThread() async throws {
        let seen = Seen()
        let request = SSHKeyCreation.Request(
            importing: false, name: "box", algorithm: .ed25519, comment: "", requireFaceID: false,
            secureEnclave: false, privateKey: "", passphrase: "", publicKeyLine: "")
        let record = try await SSHKeyCreation.create(request, keychain: KeychainService.inMemory(),
                                                     generate: { algorithm, comment in
            seen.onMain = Thread.isMainThread
            return try SSHKeyFactory.generate(algorithm: algorithm, comment: comment)
        }) { _ in }
        #expect(seen.onMain == false, "the key was made on the main thread")
        #expect(record.algorithm == .ed25519)
        #expect(record.publicKey.hasSuffix(" box"), "an empty comment is the name, as before")
    }

    @Test("Import converts what is pasted, and says why it can't")
    func addKeyImports() async throws {
        let store = SSHKeyStore(defaults: Self.scratch(), readiness: .always)
        let keychain = KeychainService.inMemory()
        let generated = try SSHKeyFactory.generate(algorithm: .ed25519, comment: "pasted")
        let model = AddKeyModel(method: .importKey)
        #expect(model.apply(.name(text: "pasted")))
        #expect(!model.canSubmit, "nothing pasted")
        // What the host's editor does as the user pastes.
        let editor = KeyPEMEditor(model: model, context: nil)
        editor.paste(String(decoding: generated.privateBlob, as: UTF8.self))
        #expect(model.canSubmit)
        #expect(model.page.pem == AddKeyPem(empty: false, encrypted: false))
        let record = try #require(await model.submit(to: store, keychain: keychain))
        #expect(record.source == .imported)
        #expect(record.fingerprint == generated.fingerprint)
        // Garbage: the card says why, nothing is stored.
        let bad = AddKeyModel(method: .importKey)
        #expect(bad.apply(.name(text: "bad")))
        KeyPEMEditor(model: bad, context: nil).paste("-----BEGIN OPENSSH PRIVATE KEY-----\nnot a key\n-----END OPENSSH PRIVATE KEY-----")
        #expect(await bad.submit(to: store, keychain: keychain) == nil)
        #expect(bad.page.error != nil)
        #expect(store.keys.count == 1)
        #expect(bad.apply(.dismissError))
        #expect(bad.page.error == nil)
    }

    // MARK: Select Key

    @Test("Select Key on a real surface: a touch on a key lands in it")
    func keyPickerSurface() async throws {
        let (surface, _, done) = try Self.surface(running: .keyPicker)
        defer { done() }
        let snapshot = KeyPickerSnapshot(keys: [Self.key("laptop"), Self.key("desk")], selected: nil)
        #expect(surface.post(KeyPickerModel.message(snapshot)))
        await NSHTouchAudit.pressablesTakeTouches(in: surface)

    }

    @Test("Select Key's state is a message the Rust screen takes; a key the app can't sign with can't be picked")
    func keyPicker() {
        let ok = Self.key("laptop")
        let p384 = Self.key("old", algorithm: .ecdsa, publicKey: "ssh-dss AAAAB3NzaC1kc3M old")
        let snapshot = KeyPickerSnapshot(keys: [ok, p384], selected: ok.id)
        let message = KeyPickerModel.message(snapshot)
        #expect(NSHSurface.check(screen: NSHScreen.keyPicker.name, message: message), "\(message)")
        let keys = message["keys"] as? [[String: Any]]
        #expect(keys?[0]["refused"] == nil)
        #expect(keys?[1]["refused"] as? String != nil)
        #expect(NSHSurface.check(screen: NSHScreen.keyPicker.name,
                                 message: KeyPickerModel.message(KeyPickerSnapshot(keys: [], selected: nil))))
        #expect(KeyPickerModel.pick(["ev": "select", "id": ok.id.uuidString], among: [ok, p384], selected: nil) == .key(ok.id))
        #expect(KeyPickerModel.pick(["ev": "select", "id": p384.id.uuidString], among: [ok, p384], selected: nil) == nil)
        #expect(KeyPickerModel.pick(["ev": "select", "id": p384.id.uuidString], among: [ok, p384], selected: p384.id)
                == .key(p384.id), "unless it is the one already picked")
        #expect(KeyPickerModel.pick(["ev": "select", "id": NSNull()], among: [ok], selected: ok.id) == .paste)
        #expect(KeyPickerModel.pick(["ev": "select", "id": UUID().uuidString], among: [ok], selected: nil) == nil)
    }

    // MARK: Shortcuts

    @Test("the Shortcuts state is a message the Rust screen takes; AVAILABLE has every shortcut off the bar")
    func shortcutsMessages() throws {
        let store = ShortcutStore(defaults: Self.scratch(), readiness: .always)
        var mine = TerminalShortcut()
        mine.kind = .text
        mine.payload = "ls"
        mine.chipLabel = "ls"
        mine.summary = "list"
        store.add(mine)
        store.removeFromToolbar(id: mine.id)
        let json = ShortcutsModel.message(ShortcutsSnapshot(store: store))
        #expect(NSHSurface.check(screen: NSHScreen.shortcuts.name, json: json), "\(json)")
        let message = Self.object(json)
        let available = try #require(message["available"] as? [[String: Any]])
        #expect(available.contains { $0["id"] as? String == mine.id.uuidString },
                "a custom shortcut off the bar can go back")
        let toolbar = try #require(message["toolbar"] as? [[String: Any]])
        #expect(toolbar.count == store.toolbarCount)
        #expect(toolbar.contains { $0["kind"] as? String == "files" }, "the files chip, by its real kind")
    }

    @Test("the page's changes go through the store, which keeps 1.0.8's format")
    func shortcutsApply() throws {
        let defaults = Self.scratch()
        let store = ShortcutStore(defaults: defaults, readiness: .always)
        let first = try #require(store.toolbar.first)
        let second = try #require(store.toolbar.dropFirst().first)
        #expect(ShortcutsModel.apply(.move(from: 0, to: 2), to: store))
        #expect(store.toolbar.prefix(2).map(\.id) == [second.id, first.id])
        #expect(!ShortcutsModel.apply(.move(from: 40, to: 0), to: store))
        #expect(ShortcutsModel.apply(.removeFromToolbar(id: first.id.uuidString), to: store))
        #expect(!store.toolbar.contains { $0.id == first.id })
        #expect(store.shortcuts.contains { $0.id == first.id }, "a built-in stays, off the bar")
        #expect(ShortcutsModel.apply(.addToToolbar(id: first.id.uuidString), to: store))
        #expect(store.toolbar.contains { $0.id == first.id })
        #expect(!ShortcutsModel.apply(.delete(id: first.id.uuidString), to: store), "a built-in is not deleted")
        var mine = TerminalShortcut()
        mine.chipLabel = "x"
        mine.key = "x"
        store.add(mine)
        #expect(ShortcutsModel.apply(.delete(id: mine.id.uuidString), to: store))
        #expect(!store.shortcuts.contains { $0.id == mine.id })
        // B153: the files chip is stored as a kind 1.0.8 reads, its real kind beside it.
        let stored = try #require(defaults.data(forKey: "moshpit.shortcuts.v1"))
        let list = try #require(try JSONSerialization.jsonObject(with: stored) as? [[String: Any]])
        let files = try #require(list.first { $0["shortcutKind"] as? String == "files" })
        #expect(files["kind"] as? String == "keyCombo")
    }

    @Test("the editor's draft is a message the Rust screen takes; Save saves as the SwiftUI page does")
    func shortcutEditor() throws {
        let store = ShortcutStore(defaults: Self.scratch(), readiness: .always)
        let model = AddShortcutModel(existing: nil)
        #expect(NSHSurface.check(screen: NSHScreen.addShortcut.name, json: model.message))
        #expect(model.save(to: store) == .notSaved, "nothing to save yet")
        model.apply(draftJson: """
            {"kind": "keyCombo", "modifiers": ["ctrl", "nonsense"], "key": "bbbbbbbb", "chipLabel": "C-b-long",
             "summary": "", "colorId": "mosh", "scopeHosts": ["nas"], "onlyInTmux": true,
             "payload": "", "appendReturn": false, "repeatOnHold": true}
            """)
        let draft = Self.object(model.draftJson)
        #expect(draft["key"] as? String == "bbbbbb", "six characters at most")
        #expect(draft["chipLabel"] as? String == "C-b-lo")
        #expect(draft["modifiers"] as? [String] == ["ctrl"])
        #expect(NSHSurface.check(screen: NSHScreen.addShortcut.name, json: model.message), "\(model.message)")
        let count = store.shortcuts.count
        #expect(model.save(to: store) == .saved)
        let saved = try #require(store.shortcuts.last)
        #expect(store.shortcuts.count == count + 1)
        #expect(saved.summary == "C-b-lo", "an empty summary is the chip label")
        #expect(saved.colorId == "mosh")
        #expect(saved.scopeHosts == ["nas"])
        #expect(saved.onlyInTmux)
        // Editing keeps its id and updates in place.
        let editor = AddShortcutModel(existing: saved)
        #expect(Self.object(editor.message)["editing"] as? Bool == true)
        editor.apply(draftJson: #"{"summary": "prefix"}"#)
        #expect(editor.save(to: store) == .saved)
        #expect(store.shortcuts.count == count + 1)
        #expect(store.shortcuts.last?.summary == "prefix")
        #expect(store.shortcuts.last?.id == saved.id)
    }

    /// B252: a new shortcut saved while the toolbar is full went under
    /// AVAILABLE in silence. Save says so (the host shows it before the
    /// page goes); an edit of one already saved never does.
    @Test("Save with the toolbar full says the shortcut is under AVAILABLE")
    func shortcutEditorSaysAFullToolbar() throws {
        let store = ShortcutStore(defaults: Self.scratch(), readiness: .always)
        while !store.toolbarIsFull {
            var filler = TerminalShortcut()
            filler.chipLabel = "f\(store.toolbarCount)"
            filler.kind = .text
            filler.payload = "x"
            store.add(filler)
        }
        let model = AddShortcutModel(existing: nil)
        model.apply(draftJson: #"{"kind": "text", "payload": "git status", "chipLabel": "gs"}"#)
        #expect(model.save(to: store) == .savedOffToolbar, "saved off a full bar in silence")
        let saved = try #require(store.shortcuts.last)
        #expect(saved.chipLabel == "gs" && !saved.inToolbar)
        #expect(store.available.contains { $0.id == saved.id })

        let editor = AddShortcutModel(existing: saved)
        editor.apply(draftJson: #"{"summary": "status"}"#)
        #expect(editor.save(to: store) == .saved, "an edit isn't a new place on the bar")
    }

    /// A second Save of a new shortcut (a double tap while the sheet goes
    /// away) used to add a second copy under a fresh id.
    @Test("a new shortcut saved twice is one shortcut")
    func shortcutEditorSavesOnce() throws {
        let store = ShortcutStore(defaults: Self.scratch(), readiness: .always)
        let count = store.shortcuts.count
        let model = AddShortcutModel(existing: nil)
        model.apply(draftJson: #"{"kind": "text", "payload": "git status", "chipLabel": "gs"}"#)
        #expect(model.save(to: store) == .saved)
        model.apply(draftJson: #"{"summary": "status"}"#)
        #expect(model.save(to: store) == .saved)
        #expect(store.shortcuts.count == count + 1, "the second Save added another copy")
        #expect(store.shortcuts.last?.summary == "status", "the second Save is the first one updated")
    }

    // MARK: Set up this host

    /// The demo's page, once the core has read its canned host.
    private static func demoPage(_ scenario: HostSetupDemo.Scenario) async -> (DemoHostCare, UInt64) {
        let care = DemoHostCare(scenario)
        let page = care.openSetup(HostSetupDemo.connection, testPane: "%3")
        for _ in 0..<300 where !care.isIdle(page) {
            try? await Task.sleep(for: .milliseconds(10))
        }
        return (care, page)
    }

    private static func object(_ json: String?) -> [String: Any] {
        json.flatMap { try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any] } ?? [:]
    }

    @Test("every host setup state the demo scenarios reach is a message the Rust screen takes")
    func hostSetupMessages() async {
        for scenario in HostSetupDemo.Scenario.allCases {
            let care = DemoHostCare(scenario)
            let page = care.openSetup(HostSetupDemo.connection, testPane: "%3")
            let before = Self.object(care.setupMessage(page))
            #expect(NSHSurface.check(screen: NSHScreen.hostSetup.name, message: before), "\(scenario)")
            let (read, readPage) = await Self.demoPage(scenario)
            let after = Self.object(read.setupMessage(readPage))
            #expect(after["phase"] as? String == "idle", "\(scenario)")
            #expect(after["phaseLabel"] == nil)
            #expect(NSHSurface.check(screen: NSHScreen.hostSetup.name, message: after), "\(scenario): \(after)")
            switch scenario {
            case .installed:
                #expect(after["pairing"] as? String == "current")
            case .stale, .nothingWillPush:
                #expect(after["hooks"] as? String == "stale")
                #expect((after["hooksProof"] as? [String: Any])?["state"] as? String == "unavailable")
            case .missingTools:
                let facts = after["facts"] as? [String: Any]
                #expect((facts?["missingForPush"] as? [String])?.contains("openssl") == true)
            case .relayUnreachable:
                #expect(after["relayError"] as? String != nil)
            case .fresh:
                #expect(after["hooks"] as? String == "absent")
            }
        }
    }

    // MARK: On the device

    /// A surface in a window of the test host's scene, its page's own
    /// components registered by `prepare`, running `screen`.
    private static func surface(running screen: NSHScreen, prepare: (NSHSurface) -> Void = { _ in })
        throws -> (NSHSurface, SettingsNSHTests.Heard, () -> Void) {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        // A controller to present the system's alerts and action sheets from.
        // The surface goes in its view: added to the window itself, the
        // controller's view (installed when the window shows) covered it and
        // took every touch.
        let root = UIViewController()
        window.rootViewController = root
        window.isHidden = false
        let surface = try #require(NSHSurface.make(frame: window.bounds))
        root.view.addSubview(surface)
        let heard = SettingsNSHTests.Heard()
        surface.onMessage = { heard.append($0) }
        prepare(surface)
        #expect(surface.run(screen: screen.name, params: NSHScreenParams.current()))
        return (surface, heard, {
            window.rootViewController?.dismiss(animated: false)
            surface.removeFromSuperview()
            window.isHidden = true
        })
    }

    /// The alert or action sheet (`style`) `surface` shows (UIKit's
    /// `UIAlertController`): the topmost one presented, once its
    /// presentation has finished. One still coming in can't be dismissed,
    /// and the next would be presented over it (keysSurface pressed the
    /// action sheet a second time, its Delete Key, looking for the alert's
    /// Delete).
    private static func alert(over surface: UIView,
                              style: UIAlertController.Style) async -> UIAlertController? {
        for _ in 0..<150 {
            var top = surface.window?.rootViewController?.presentedViewController
            while let next = top?.presentedViewController { top = next }
            if let alert = top as? UIAlertController, alert.preferredStyle == style,
               !alert.isBeingPresented, !alert.isBeingDismissed, alert.view.window != nil {
                return alert
            }
            try? await Task.sleep(for: .milliseconds(20))
        }
        return nil
    }

    /// Presses `alert`'s action titled `title`, as a tap on it does: the
    /// alert goes, then the action's handler runs.
    private static func press(_ alert: UIAlertController, _ title: String) async {
        typealias Handler = @convention(block) (UIAlertAction) -> Void
        guard let action = alert.actions.first(where: { $0.title == title }),
              let block = action.value(forKey: "handler") else {
            Issue.record("no action \(title) in \(alert.actions.map(\.title))")
            return
        }
        await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
            alert.dismiss(animated: false) { done.resume() }
        }
        unsafeBitCast(block as AnyObject, to: Handler.self)(action)
    }

    @Test("SSH Keys on a real surface: a key's row opens UIKit's action sheet, the alert's Delete reaches the host")
    func keysSurface() async throws {
        let (surface, heard, done) = try Self.surface(running: .sshKeys)
        defer { done() }
        let laptop = Self.key("laptop")
        #expect(surface.post(.sshKeys(state: SSHKeysModel.state(
            SSHKeysSnapshot(device: [], imported: [laptop]), deleting: nil))))
        let row = try #require(await SettingsNSHTests.view(identified: SSHKeysModel.rowID(laptop.id), in: surface))
        let add = try #require(await SettingsNSHTests.view(identified: "ssh-keys-add", in: surface))
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(add.accessibilityActivate())
        #expect(await heard.next()?["ev"] as? String == "add")
        // The row's actions: UIKit's action sheet, Delete Key in red.
        #expect(row.accessibilityActivate())
        let sheet = try #require(await Self.alert(over: surface, style: .actionSheet))
        #expect(sheet.preferredStyle == .actionSheet)
        #expect(sheet.actions.map(\.title) == [
            String(localized: "Copy Public Key"), String(localized: "Copy Fingerprint"),
            String(localized: "Share Public Key"), String(localized: "Delete Key"), String(localized: "Cancel"),
        ])
        #expect(sheet.actions.first { $0.title == String(localized: "Delete Key") }?.style == .destructive)
        await Self.press(sheet, String(localized: "Delete Key"))
        let asked = try #require(await heard.next())
        #expect(asked["ev"] as? String == "confirm-delete")
        #expect(asked["id"] as? String == laptop.id.uuidString)
        // The host asks: the system's alert, its Delete in red.
        #expect(surface.post(.sshKeys(state: SSHKeysModel.state(
            SSHKeysSnapshot(device: [], imported: [laptop]), deleting: laptop.id))))
        let alert = try #require(await Self.alert(over: surface, style: .alert))
        #expect(alert.preferredStyle == .alert)
        #expect(alert.actions.first { $0.title == String(localized: "Delete") }?.style == .destructive)
        await Self.press(alert, String(localized: "Delete"))
        let delete = try #require(await heard.next())
        #expect(delete["ev"] as? String == "delete")
        #expect(delete["id"] as? String == laptop.id.uuidString)
    }

    @Test("SSH Keys on a real surface: an empty group's button opens Add Key on its method")
    func keysEmptySurface() async throws {
        let (surface, heard, done) = try Self.surface(running: .sshKeys)
        defer { done() }
        #expect(surface.post(.sshKeys(state: SSHKeysModel.state(
            SSHKeysSnapshot(device: [], imported: []), deleting: nil))))
        let generate = try #require(await SettingsNSHTests.view(identified: "ssh-keys-generate", in: surface))
        let importKey = try #require(await SettingsNSHTests.view(identified: "ssh-keys-import", in: surface))
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(generate.accessibilityActivate())
        let first = try #require(await heard.next())
        #expect(first["ev"] as? String == "add")
        #expect(first["method"] as? String == "generate")
        #expect(importKey.accessibilityActivate())
        #expect(await heard.next()?["method"] as? String == "import")
    }

    @Test("SSH Keys on a real surface: the notice for a key a connection uses, in the system's alert, and its OK")
    func keysNoticeSurface() async throws {
        let (surface, heard, done) = try Self.surface(running: .sshKeys)
        defer { done() }
        let laptop = Self.key("laptop")
        let notice = SSHKeyDeletion.Failure.inUse(["pi", "nas"])
        #expect(surface.post(.sshKeys(state: SSHKeysModel.state(
            SSHKeysSnapshot(device: [], imported: [laptop]), deleting: nil, notice: notice))))
        let alert = try #require(await Self.alert(over: surface, style: .alert))
        #expect(alert.actions.map(\.title) == [String(localized: "OK")], "only the way out")
        await Self.press(alert, String(localized: "OK"))
        #expect(await heard.next()?["ev"] as? String == "dismiss-notice")
    }

    @Test("Add Key on a real surface: the key's text is typed into the host's view and stays there")
    func addKeySurface() async throws {
        let model = AddKeyModel(method: .importKey)
        let (surface, heard, done) = try Self.surface(running: .addKey) { model.register(on: $0) }
        defer { done() }
        model.attach(surface)
        let editor = try #require(await SettingsNSHTests.view(identified: "add-key-private-key", in: surface)
            as? UITextView)
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(editor.font?.fontDescriptor.symbolicTraits.contains(.traitMonoSpace) == true)
        let generated = try SSHKeyFactory.generate(algorithm: .ed25519, comment: "x")
        let pem = String(decoding: generated.privateBlob, as: UTF8.self)
        editor.text = pem
        editor.delegate?.textViewDidChange?(editor)
        #expect(!(try pageMessage(message: .addKey(state: model.page))).contains("PRIVATE KEY-----"),
                "the post says only that there is one")
        #expect(!model.page.pem.empty)
        let method = try #require(await SettingsNSHTests.view(identified: "add-key-method", in: surface)
            as? UISegmentedControl)
        #expect(method.selectedSegmentIndex == 1)
        let cancel = try #require(await SettingsNSHTests.view(identified: "add-key-cancel", in: surface))
        #expect(cancel.accessibilityActivate())
        // Whatever else the page told first (the fields it made), the cancel arrives.
        var event = await heard.next()
        while let ev = event?["ev"] as? String, ev != "cancel" { event = await heard.next() }
        #expect(event?["ev"] as? String == "cancel")
    }

    @Test("Shortcuts and its editor on a real surface: − takes off, a handle reorders, UIKit's controls carry the state")
    func shortcutsSurface() async throws {
        let store = ShortcutStore(defaults: Self.scratch(), readiness: .always)
        let (surface, heard, done) = try Self.surface(running: .shortcuts)
        defer { done() }
        #expect(surface.post(json: ShortcutsModel.message(ShortcutsSnapshot(store: store))))
        let first = try #require(store.toolbar.first)
        let remove = try #require(await SettingsNSHTests.view(identified: "shortcut-remove-\(first.id.uuidString)",
                                                              in: surface))
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        // Off the bar at once: it goes to Available, nothing asks.
        #expect(remove.accessibilityActivate())
        let event = try #require(await heard.next())
        #expect(event["ev"] as? String == "remove-from-toolbar")
        #expect(event["id"] as? String == first.id.uuidString)
        // VoiceOver's swipe down on a grip moves its shortcut a place later.
        let second = store.toolbar[1]
        let grip = try #require(await SettingsNSHTests.view(identified: "shortcut-grip-\(second.id.uuidString)",
                                                            in: surface))
        #expect(grip.accessibilityTraits.contains(.adjustable))
        grip.accessibilityDecrement()
        let move = try #require(await heard.next())
        #expect(move["ev"] as? String == "move")
        #expect((move["from"] as? NSNumber)?.intValue == 1)
        #expect((move["to"] as? NSNumber)?.intValue == 3)

        let (editor, told, close) = try Self.surface(running: .addShortcut)
        defer { close() }
        let model = AddShortcutModel(existing: nil)
        model.attach(editor, hosts: ["m1-pro"], toolbarCount: 6)
        let type = try #require(await SettingsNSHTests.view(identified: "shortcut-type", in: editor)
            as? UISegmentedControl)
        await NSHTouchAudit.pressablesTakeTouches(in: editor)
        #expect(type.selectedSegmentIndex == 0)
        // TYPE is on screen in the test's surface (SCOPE's switch is far
        // below it): picking Text changes the draft's kind.
        type.selectedSegmentIndex = 1
        type.sendActions(for: .valueChanged)
        let change = try #require(await told.next())
        #expect(change["ev"] as? String == "change")
        #expect((change["draft"] as? [String: Any])?["kind"] as? String == "text")
    }

    // MARK: Pictures (R10, R11)

    /// Draws the window where `surface` is to the app's tmp folder as
    /// `r10-<name>.png`.
    private static func shoot(_ surface: UIView, _ name: String) {
        guard let window = surface.window else { return }
        let frame = surface.convert(surface.bounds, to: window)
        let image = UIGraphicsImageRenderer(bounds: frame).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        try? image.pngData()?.write(to: FileManager.default.temporaryDirectory
            .appendingPathComponent("r10-\(name).png"))
    }

    /// `screen` at the owner's iPhone's size (402 × 874 pt), dark or
    /// `light`, given what to show by `post`, drawn once `ready` shows.
    private static func picture(_ screen: NSHScreen, light: Bool, ready: String, name: String,
                                prepare: (NSHSurface) -> Void = { _ in },
                                post: (NSHSurface) -> Void) async throws {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: NavBarNSHTests.phone)
        // The phone's size: a window as big as the iPad's scene in landscape
        // (744 pt tall) cut the pictures off at that height.
        window.frame = NavBarNSHTests.phone
        let root = UIViewController()
        window.rootViewController = root
        window.isHidden = false
        root.view.frame = window.bounds
        let surface = try #require(NSHSurface.make(frame: NavBarNSHTests.phone))
        root.view.addSubview(surface)
        defer {
            surface.removeFromSuperview()
            window.isHidden = true
        }
        prepare(surface)
        var params = NSHScreenParams.current()
        params["light"] = light
        #expect(surface.run(screen: screen.name, params: params), "\(name)")
        post(surface)
        #expect(await SettingsNSHTests.view(identified: ready, in: surface) != nil, "\(name): \(ready)")
        try? await Task.sleep(for: .milliseconds(600))
        Self.shoot(surface, "\(name)-\(light ? "light" : "dark")")
    }

    @Test("SSH Keys, Add Key, Select Key, Shortcuts and Add Shortcut drawn dark and light (R10, R11)")
    func pictures() async throws {
        let laptop = Self.key("laptop")
        let old = Self.key("old box", algorithm: .rsa4096, publicKey: "ssh-rsa AAAAB3Nza old",
                           lastUsed: Date(timeIntervalSinceNow: -200 * 86_400))
        let store = ShortcutStore(defaults: Self.scratch(), readiness: .always)
        for light in [false, true] {
            try await Self.picture(.sshKeys, light: light, ready: SSHKeysModel.rowID(laptop.id), name: "ssh-keys") {
                $0.post(.sshKeys(state: SSHKeysModel.state(
                    SSHKeysSnapshot(device: [], imported: [laptop, old],
                                    users: [laptop.id: ["build-box", "gpu-01"]]), deleting: nil)))
            }
            let generate = AddKeyModel(method: .generate)
            try await Self.picture(.addKey, light: light, ready: "add-key-name", name: "add-key",
                                   prepare: { generate.register(on: $0) }) { generate.attach($0) }
            let importing = AddKeyModel(method: .importKey)
            try await Self.picture(.addKey, light: light, ready: "add-key-private-key", name: "add-key-import",
                                   prepare: { importing.register(on: $0) }) { importing.attach($0) }
            try await Self.picture(.keyPicker, light: light, ready: "key-picker-paste-pem", name: "key-picker") {
                $0.post(KeyPickerModel.message(KeyPickerSnapshot(keys: [laptop, old], selected: laptop.id)))
            }
            try await Self.picture(.shortcuts, light: light, ready: "shortcuts-preview", name: "shortcuts") {
                $0.post(json: ShortcutsModel.message(ShortcutsSnapshot(store: store)))
            }
            let editor = AddShortcutModel(existing: nil)
            try await Self.picture(.addShortcut, light: light, ready: "shortcut-type", name: "add-shortcut") {
                editor.attach($0, hosts: ["m1-pro", "build-box"], toolbarCount: 7)
            }
        }
    }

    @Test("Set up this host on a real surface: the agent menu is UIKit's, a button reaches the host")
    func hostSetupSurface() async throws {
        let (surface, heard, done) = try Self.surface(running: .hostSetup)
        defer { done() }
        let (care, page) = await Self.demoPage(.installed)
        #expect(surface.post(json: try #require(care.setupMessage(page))))
        let agent = try #require(await SettingsNSHTests.view(identified: "hostsetup-agent", in: surface) as? UIButton)
        let agents = try #require(agent.menu?.children)
        #expect(agents.count == 7, "the seven agents the core knows")
        // R16: the agent menu is the `ui` layer's select, which checks the
        // option in use (UIKit's own checkmark) and says each agent's status.
        let first = try #require(agents.first as? UIAction)
        #expect(first.state == UIMenuElement.State.on, "the agent in use is checked")
        #expect(first.subtitle != nil, "its status scripts' status, under its name")
        let doneButton = try #require(await SettingsNSHTests.view(identified: "hostsetup-done", in: surface))
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(doneButton.accessibilityActivate())
        #expect(await heard.next()?["ev"] as? String == "done")
    }
}
