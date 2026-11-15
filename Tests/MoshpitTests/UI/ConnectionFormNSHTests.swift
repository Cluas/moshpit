import Foundation
import Testing
import UIKit
@testable import Moshpit

/// Add Connection / Edit Connection in NSH, the host's side: the draft the
/// host posts (every one a message the Rust screen takes), the
/// edits the screen tells and the intents reaching the shared model, the
/// secrets kept in the host, the draft read from and written back to a
/// saved connection through the records, and a real surface on the device
/// that shows the draft and reports an edit and Save. The model's rules
/// (ports, Save, the credential) are tested where they live,
/// `crates/app/src/connection_form/tests.rs`.
@Suite("NSH connection form")
@MainActor
struct ConnectionFormNSHTests {

    static func connection() -> ServerConnection {
        var c = ServerConnection()
        c.name = "studio"
        c.host = "192.0.2.22"
        c.port = 2222
        c.sshPort = 2222
        c.username = "cluas"
        c.authMethod = .key
        // The key pasted into it when it was made (Save keeps it).
        c.keychainRef = "moshpit.secret.studio"
        c.connectionProtocol = .mosh
        c.predictMode = .never
        c.roamOnCellular = false
        c.multiplexer = .herdr
        c.herdrPath = "/opt/herdr/bin/herdr"
        c.useSOCKSProxy = true
        c.socksProxyHost = "proxy.example.com"
        c.socksProxyPort = 1081
        return c
    }

    /// A new connection's draft with SSH (the setting aside).
    static func blank() -> ConnectionDraft {
        connectionDraftNew(moshByDefault: false)
    }

    @Test("the form runs the Rust screen of its name")
    func name() {
        #expect(NSHScreen.connectionForm.name == "connection-form")
    }

    @Test("every draft is a message the Rust screen takes; bad ones are refused")
    func messages() throws {
        let screen = NSHScreen.connectionForm.name
        let settings = AppSettings.shared
        let fresh = ConnectionFormModel(draft: Self.blank())
            .message(settings: settings, keys: [])
        #expect(NSHSurface.check(screen: screen, message: fresh), "\(fresh)")
        #expect(fresh["editing"] as? Bool == false)
        #expect(fresh["port"] as? String == "22")
        #expect(fresh["mux"] as? String == "tmux", "tmux unless picked otherwise")
        #expect(fresh["key"] == nil)

        let edit = ConnectionFormModel(draft: ConnectionDraft(Self.connection()))
            .message(settings: settings, keys: [])
        #expect(NSHSurface.check(screen: screen, message: edit), "\(edit)")
        #expect(edit["auth"] as? String == "key")
        #expect(edit["predictMode"] as? String == "never")
        #expect(edit["socksPort"] as? String == "1081")
        #expect(edit["udpRange"] as? String == "\(settings.udpRangeStart) – \(settings.udpRangeEnd)")

        var bad = edit
        bad["mux"] = "screen"
        #expect(!NSHSurface.check(screen: screen, message: bad))
        bad = edit
        bad["colour"] = "red"
        #expect(!NSHSurface.check(screen: screen, message: bad), "a field the screen does not know")
        // The screen refuses a secret: it takes only whether there is one.
        for key in ["password", "pem"] {
            bad = edit
            bad[key] = "hunter2"
            #expect(!NSHSurface.check(screen: screen, message: bad), "\(key)")
        }
    }

    /// B294: the NSH form used to post the password and the pasted key to
    /// the screen and take them back through its text inputs and the
    /// channel. They stay in the host's draft now: posted as "there is one"
    /// only, never taken from the screen.
    /// §6.5: a failed Save is a message the screen takes and shows under its
    /// bar — the secret the keychain did not keep and Try again, or the
    /// refusal's words and no retry — in place of the old notice card.
    @Test("a failed save is a message the Rust screen takes")
    func failedSaveMessages() {
        let screen = NSHScreen.connectionForm.name
        let keychain = ConnectionFormModel.failure(
            ConnectionKeychainFailure(secret: .password, error: KeychainError.interactionNotAllowed))
        #expect(keychain["secret"] as? String == "password")
        #expect(keychain["retry"] as? Bool == true)
        #expect(NSHSurface.check(screen: screen, message: keychain), "\(keychain)")
        let key = ConnectionFormModel.failure(
            ConnectionKeychainFailure(secret: .key, error: KeychainError.unhandled(-25_293)))
        #expect(NSHSurface.check(screen: screen, message: key), "\(key)")
        let refused = ConnectionFormModel.failure(ConnectionSaveRefusal.Incomplete)
        #expect(refused["secret"] == nil)
        #expect(refused["retry"] as? Bool == false)
        #expect(NSHSurface.check(screen: screen, message: refused), "\(refused)")
    }

    @Test("the password and the pasted key are the host's: never posted, never taken from the screen")
    func secretsStayInTheHost() throws {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----"
        let model = ConnectionFormModel(draft: Self.blank())
        #expect(model.handle(["ev": "set", "key": "password", "value": "from-the-screen"]) == nil)
        #expect(model.handle(["ev": "set", "key": "pem", "value": pem]) == nil)
        #expect(model.secrets == ConnectionSecrets(), "a secret told through the screen was taken")
        #expect(model.draft == Self.blank())

        model.passwordChanged("hunter2")
        model.pemChanged(pem)
        #expect(model.secrets.password == "hunter2" && model.secrets.privateKeyPEM == pem)
        let message = model.message(settings: .shared, keys: [])
        #expect(NSHSurface.check(screen: NSHScreen.connectionForm.name, message: message), "\(message)")
        let json = String(decoding: try JSONSerialization.data(withJSONObject: message), as: UTF8.self)
        #expect(!json.contains("hunter2"), "\(json)")
        #expect(!json.contains("b3BlbnNzaC1rZXktdjEAAAAA"), "\(json)")
        #expect((message["secrets"] as? [String: Bool])?["password"] == true)
        #expect((message["secrets"] as? [String: Bool])?["pem"] == true)

        model.pemChanged(" \n\t")
        #expect((model.message(settings: .shared, keys: [])["secrets"] as? [String: Bool])?["pem"] == false,
                "white space is no key")
    }

    /// A key picked in the form, then deleted (or renamed) in SSH Keys in
    /// another window: the form used to go on showing it, and Save then
    /// stored a key connection with no key and no PEM.
    @Test("a picked key deleted elsewhere: the form shows it gone, as SwiftUI's live key does")
    func pickedKeyChangedElsewhere() {
        let key = SSHKeyRecord(name: "laptop", algorithm: .ed25519, source: .generated,
                               keychainRef: "moshpit.secret.key-k")
        let model = ConnectionFormModel(draft: Self.blank())
        model.pickKey(key.id, settings: .shared, keys: [key])
        #expect(!model.keysChanged(settings: .shared, keys: [key]), "nothing about the picked key changed")
        var renamed = key
        renamed.name = "work laptop"
        #expect(model.keysChanged(settings: .shared, keys: [renamed]), "the old name stayed on the key row")
        #expect(model.keysChanged(settings: .shared, keys: []), "the deleted key stayed on the key row")
        #expect(model.message(settings: .shared, keys: [])["key"] == nil)
        #expect(!model.keysChanged(settings: .shared, keys: []))
    }

    @Test("what the screen tells: an edit the draft takes as the SwiftUI form's bindings do, or an intent")
    func edits() {
        let model = ConnectionFormModel(draft: Self.blank())
        for (key, value) in [("name", "box"), ("host", "box.example.com"), ("port", "2200"),
                             ("username", "root"),
                             ("moshServerPath", "/usr/bin/mosh-server"), ("socksHost", "127.0.0.1"),
                             ("socksPort", "9050"), ("tmuxPath", "/usr/bin/tmux"),
                             ("herdrPath", "/usr/bin/herdr")] {
            #expect(model.handle(["ev": "set", "key": key, "value": value]) == nil)
        }
        #expect(model.handle(["ev": "set", "key": "mosh", "value": true]) == nil)
        #expect(model.handle(["ev": "set", "key": "socks", "value": true]) == nil)
        #expect(model.handle(["ev": "set", "key": "auth", "value": "key"]) == nil)
        #expect(model.handle(["ev": "set", "key": "predictMode", "value": "always"]) == nil)
        #expect(model.handle(["ev": "set", "key": "mux", "value": "none"]) == nil)
        let d = model.draft
        #expect(d.name == "box" && d.host == "box.example.com" && d.port == "2200" && d.username == "root")
        #expect(d.moshServerPath == "/usr/bin/mosh-server")
        #expect(d.socksProxyHost == "127.0.0.1" && d.socksProxyPort == "9050")
        #expect(d.tmuxPath == "/usr/bin/tmux" && d.herdrPath == "/usr/bin/herdr")
        #expect(d.useMosh && d.useSocksProxy)
        #expect(d.auth == .key && d.predictModeRaw == "always" && d.multiplexerRaw == "none")
        #expect(!d.canSave(keys: [], secrets: model.secrets), "a key connection with no key")

        // What it does not know changes nothing.
        let before = model.draft
        #expect(model.handle(["ev": "set", "key": "mosh", "value": "yes"]) == nil)
        #expect(model.handle(["ev": "set", "key": "mux", "value": "screen"]) == nil)
        #expect(model.handle(["ev": "set", "key": "colour", "value": "red"]) == nil)
        #expect(model.draft == before)

        #expect(model.handle(["ev": "pick-key"]) == .pickKey)
        #expect(model.handle(["ev": "save"]) == .save)
        #expect(model.handle(["ev": "cancel"]) == .cancel)
        #expect(model.handle(["ev": "delete"]) == .delete)
        #expect(model.handle(["ev": "jump"]) == nil)
    }

    /// The host's wrappers reach the model's rules (`connection_form`'s
    /// tests hold the rules themselves, B294).
    @Test("Save's rule through the host: a port that is not one, a key connection with no key")
    func saveRuleThroughTheHost() {
        let key = SSHKeyRecord(name: "laptop", algorithm: .ed25519, source: .generated,
                               keychainRef: "moshpit.secret.key-k")
        var draft = Self.blank()
        draft.name = "box"
        draft.host = "box.example.com"
        #expect(!draft.canSave(keys: [], secrets: ConnectionSecrets(password: "p")), "no user (B517)")
        draft.username = "me"
        #expect(!draft.canSave(keys: [], secrets: ConnectionSecrets()), "no password, which nothing asks for later")
        #expect(draft.canSave(keys: [], secrets: ConnectionSecrets(password: " ")))
        draft.port = "70000"
        #expect(!draft.canSave(keys: [], secrets: ConnectionSecrets(password: "p")), "70000 is no port")
        draft.port = ""
        draft.auth = .key
        #expect(!draft.canSave(keys: [key], secrets: ConnectionSecrets(privateKeyPEM: " \n\t")),
                "white space saved as a key")
        draft.selectedKeyId = key.id
        #expect(draft.canSave(keys: [key], secrets: ConnectionSecrets()))
        #expect(!draft.canSave(keys: [], secrets: ConnectionSecrets()), "a picked key deleted since is no key")

        var pasted = ServerConnection()
        pasted.authMethod = .key
        pasted.keychainRef = "moshpit.secret.own"
        #expect(ConnectionDraft(pasted).stored == .pastedKey)
        let message = ConnectionFormModel(draft: ConnectionDraft(pasted)).message(settings: .shared, keys: [])
        #expect(NSHSurface.check(screen: NSHScreen.connectionForm.name, message: message), "\(message)")
        let secrets = message["secrets"] as? [String: Bool]
        #expect(secrets?["storedKey"] == true && secrets?["storedPassword"] == false)
    }

    @Test("a saved connection's draft writes it back as it was, as the SwiftUI form saves it")
    func roundTrip() {
        let settings = AppSettings.shared
        let saved = Self.connection()
        let draft = ConnectionDraft(saved)
        #expect(draft.port == "2222" && draft.useMosh && draft.multiplexerRaw == "herdr")
        #expect(draft.moshServerPath.isEmpty, "no path: mosh-server on PATH")
        let written = draft.applied(to: saved, settings: settings)
        #expect(written.id == saved.id)
        #expect(written.name == saved.name && written.host == saved.host && written.port == 2222)
        #expect(written.sshPort == 2222 && written.username == "cluas")
        #expect(written.connectionProtocol == .mosh && written.predictMode == .never)
        #expect(written.roamOnCellular == false && written.multiplexer == .herdr)
        #expect(written.herdrPath == "/opt/herdr/bin/herdr" && written.tmuxPath == nil)
        #expect(written.moshServerPath == nil, "an empty path is none")
        #expect(written.useSOCKSProxy == true && written.socksProxyHost == "proxy.example.com")
        #expect(written.socksProxyPort == 1081)
        #expect(written.moshPortRangeStart == settings.udpRangeStart)

        var typed = Self.blank()
        typed.port = "ssh"
        typed.socksProxyPort = ""
        let fallback = typed.applied(to: ServerConnection(), settings: settings)
        #expect(fallback.port == 22 && fallback.socksProxyPort == 1080, "what is not a number falls back")
    }

    // MARK: On the device

    @Test("on a real surface: the draft's fields, and an edit and Save reach the host")
    func surface() async throws {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) }
            ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        let surface = try #require(NSHSurface.make(frame: window.bounds))
        HomeNSHComponents.register(on: surface)
        window.addSubview(surface)
        window.isHidden = false
        defer {
            surface.removeFromSuperview()
            window.isHidden = true
        }
        let heard = SettingsNSHTests.Heard()
        surface.onMessage = { heard.append($0) }
        #expect(surface.run(screen: NSHScreen.connectionForm.name, params: NSHScreenParams.current()))
        let model = ConnectionFormModel(draft: ConnectionDraft(Self.connection()))
        model.attach(surface, settings: AppSettings.shared, keys: [])

        // Each field's control is `<field>-input` (`ui/src/ui/field.rs`).
        #expect(await SettingsNSHTests.view(identified: "connection-form-name-input", in: surface) != nil)
        #expect(await SettingsNSHTests.view(identified: "connection-form-herdr-path-input", in: surface) != nil)
        #expect(await SettingsNSHTests.view(identified: "connection-form-proxy-host-input", in: surface) != nil)
        await NSHTouchAudit.pressablesTakeTouches(in: surface)

        // The multiplexer's menu is UIKit's, ticked on the one picked.
        let menu = try #require(await SettingsNSHTests.view(identified: "multiplexer-picker", in: surface)
            as? UIButton)
        #expect(menu.showsMenuAsPrimaryAction)
        let actions = menu.menu?.children
            .flatMap { ($0 as? UIMenu)?.children ?? [$0] }
            .compactMap { $0 as? UIAction }
        #expect(actions?.map(\.title) == [String(localized: "None"), "tmux", "herdr"])
        #expect(actions?.last?.subtitle == String(localized: "Built for coding agents — agent status needs no hooks"))

        let save = try #require(await SettingsNSHTests.view(identified: "save-connection", in: surface))
        #expect(save.accessibilityActivate())
        #expect(try #require(await heard.next())["ev"] as? String == "save")
    }

    /// Every text field and text view under `root`.
    static func textViews(in root: UIView) -> [UIView] {
        var found: [UIView] = []
        func walk(_ view: UIView) {
            if view is UITextField || view is UITextView { found.append(view) }
            view.subviews.forEach(walk)
        }
        walk(root)
        return found
    }

    /// Waits up to ``NSHTestWait/ceiling`` for `condition`.
    static func settle(_ condition: () -> Bool) async {
        let deadline = Date().addingTimeInterval(NSHTestWait.ceiling)
        while !condition(), Date() < deadline {
            try? await Task.sleep(for: .milliseconds(10))
        }
    }

    /// B294 on a real surface: the password and the key are typed into the
    /// host's own views, every keystroke reaches the draft (the screen
    /// hearing `filled` and redrawing meanwhile makes no new field), and
    /// nothing the screen tells — nor any text the screen's own inputs
    /// hold — carries either.
    @Test("on a real surface: the secrets are typed into the host's fields, whole, and never cross")
    func secretsOnTheSurface() async throws {
        let password = "correct-horse-battery-staple-42"
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmU\n-----END OPENSSH PRIVATE KEY-----"
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) }
            ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        let surface = try #require(NSHSurface.make(frame: window.bounds))
        let model = ConnectionFormModel(draft: Self.blank())
        HomeNSHComponents.register(on: surface)
        model.register(on: surface)
        window.addSubview(surface)
        window.isHidden = false
        defer {
            surface.removeFromSuperview()
            window.isHidden = true
        }
        var told: [String] = []
        surface.onMessage = { message in
            told.append(String(decoding: (try? JSONSerialization.data(withJSONObject: message)) ?? Data(),
                               as: UTF8.self))
            _ = model.handle(message)
        }
        #expect(surface.run(screen: NSHScreen.connectionForm.name, params: NSHScreenParams.current()))
        model.attach(surface, settings: AppSettings.shared, keys: [])

        let field = try #require(await SettingsNSHTests.view(identified: "connection-form-password", in: surface)
            as? UITextField)
        #expect(field.isSecureTextEntry)
        // Typed a key at a time, the screen left to redraw now and then.
        for (index, character) in password.enumerated() {
            field.text = (field.text ?? "") + String(character)
            field.sendActions(for: .editingChanged)
            if index % 5 == 0 { try await Task.sleep(for: .milliseconds(15)) }
        }
        try await Task.sleep(for: .milliseconds(200))
        #expect(model.secrets.password == password, "a keystroke was lost: \(model.secrets.password)")
        let still = await SettingsNSHTests.view(identified: "connection-form-password", in: surface)
        #expect(still === field, "the host's password field was made again while typing")

        // The key: its segment, then its editor.
        let auth = try #require(await SettingsNSHTests.view(identified: "connection-form-auth", in: surface)
            as? UISegmentedControl)
        auth.selectedSegmentIndex = 1
        auth.sendActions(for: .valueChanged)
        await Self.settle { model.draft.auth == .key }
        let editor = try #require(await SettingsNSHTests.view(identified: "private-key-editor", in: surface)
            as? UITextView)
        for (index, character) in pem.enumerated() {
            editor.text += String(character)
            editor.delegate?.textViewDidChange?(editor)
            if index % 9 == 0 { try await Task.sleep(for: .milliseconds(15)) }
        }
        try await Task.sleep(for: .milliseconds(200))
        #expect(model.secrets.privateKeyPEM == pem)
        let editorStill = await SettingsNSHTests.view(identified: "private-key-editor", in: surface)
        #expect(editorStill === editor, "the host's key editor was made again while pasting")

        // Back to the password: its new field starts from what was typed.
        auth.selectedSegmentIndex = 0
        auth.sendActions(for: .valueChanged)
        await Self.settle { model.draft.auth == .password }
        let again = try #require(await SettingsNSHTests.view(identified: "connection-form-password", in: surface)
            as? UITextField)
        #expect(again.text == password)
        model.keysChanged(settings: .shared, keys: [])
        model.pickKey(UUID(), settings: .shared, keys: [])
        try await Task.sleep(for: .milliseconds(200))

        // Nothing told, nothing posted, no text of the screen's own holds either.
        let posted = String(decoding: try JSONSerialization.data(
            withJSONObject: model.message(settings: .shared, keys: [])), as: UTF8.self)
        for secret in [password, "b3BlbnNzaC1rZXktdjEAAAAABG5vbmU"] {
            #expect(!told.contains { $0.contains(secret) }, "told: \(told)")
            #expect(!posted.contains(secret), "posted: \(posted)")
            for view in Self.textViews(in: surface) {
                let id = view.accessibilityIdentifier
                guard id != "connection-form-password", id != "private-key-editor" else { continue }
                let text = (view as? UITextField)?.text ?? (view as? UITextView)?.text ?? ""
                #expect(!text.contains(secret), "\(String(describing: id)) holds a secret")
            }
        }
    }

    /// A saved password connection, its port `port`.
    static func passwordConnection(port: Int) -> ServerConnection {
        var c = connection()
        c.authMethod = .password
        c.keychainRef = "moshpit.secret.studio"
        c.port = port
        c.sshPort = port
        return c
    }

    /// R9's pictures: Edit connection filled with a saved password, with a
    /// port that is not one (an old app saved 70000 as it was: the field
    /// says so at once), and with a save the keychain refused (the banner
    /// under the bar), in each appearance, to the app's tmp folder as
    /// `form-<state>-<look>.png`.
    @Test("R9 shots: filled with a saved password, a port that is not one, a failed save; dark and light")
    func shots() async throws {
        for light in [false, true] {
            for (state, port) in [("filled", 2222), ("port", 70000), ("failed", 2222)] {
                let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
                let window = scene.map { UIWindow(windowScene: $0) }
                    ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 402, height: 874))
                window.overrideUserInterfaceStyle = light ? .light : .dark
                let surface = try #require(NSHSurface.make(frame: window.bounds))
                HomeNSHComponents.register(on: surface)
                let model = ConnectionFormModel(draft: ConnectionDraft(Self.passwordConnection(port: port)))
                model.register(on: surface)
                window.addSubview(surface)
                window.isHidden = false
                var params = NSHScreenParams.current()
                params["light"] = light
                #expect(surface.run(screen: NSHScreen.connectionForm.name, params: params))
                model.attach(surface, settings: AppSettings.shared, keys: [])
                #expect(await SettingsNSHTests.view(identified: "connection-form-name-input", in: surface) != nil)
                #expect(await SettingsNSHTests.view(identified: "connection-form-password", in: surface)
                    is UITextField)
                if state == "failed" {
                    // The keychain refused the password: the banner under the bar.
                    model.failed(ConnectionKeychainFailure(
                        secret: .password, error: KeychainError.interactionNotAllowed))
                }
                try? await Task.sleep(for: .milliseconds(600))
                Self.shoot(surface, "form-\(state)-\(light ? "light" : "dark")")
                surface.removeFromSuperview()
                window.isHidden = true
            }
        }
    }

    /// Draws the window where `surface` is to the app's tmp folder as
    /// `<name>.png`.
    static func shoot(_ surface: UIView, _ name: String) {
        guard let window = surface.window else { return }
        let frame = surface.convert(surface.bounds, to: window)
        let image = UIGraphicsImageRenderer(bounds: frame).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        try? image.pngData()?.write(
            to: FileManager.default.temporaryDirectory.appendingPathComponent("\(name).png"))
    }
}
