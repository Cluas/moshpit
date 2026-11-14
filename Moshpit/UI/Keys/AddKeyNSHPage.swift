import MoshpitKit
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// Add Key: the SwiftUI page (``AddKeyView``), or the NSH one while its switch
/// is on (``NSHScreen/addKey``), from SSH Keys and from Select Key alike.
/// With the switch off the SwiftUI page is all there is: no NSH code runs.
struct AddKeyPage: View {
    /// The method the page opens on (Generate, unless a capture run asks).
    var method: AddKeyView.Method = .generate
    /// Hears the new key right before the page dismisses itself.
    var onCreated: ((SSHKeyRecord) -> Void)?

    var body: some View {
        if Self.showsNSH {
            AddKeyNSHPage(method: method, onCreated: onCreated)
        } else {
            AddKeyView(onCreated: onCreated, method: method)
        }
    }

    static var showsNSH: Bool {
        #if DEBUG
        if [.addKeyNSH, .addKeyImportNSH].contains(CaptureScreen.requested) { return NSHSurface.available }
        #endif
        return NSHScreen.addKey.isOn
    }
}

/// Add Key drawn by NSH (`ui/src/add_key.rs`) in the sheet's own stack. The
/// form lives in the host (``AddKeyModel``); the private key's text and its
/// passphrase are typed into the host's own views, which the page lays out
/// (`moshpit/key-pem`, `moshpit/key-passphrase`), so key material never
/// reaches NSH. Generating and importing are the SwiftUI page's
/// (``SSHKeyCreation``): formats converted at import, the private half in
/// the Keychain behind Face ID when asked, the record in the store. A file
/// is picked with the system's document picker, as on the SwiftUI page. If
/// NSH cannot run here, the SwiftUI page shows.
struct AddKeyNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(SSHKeyStore.self) private var keyStore
    @Environment(ConnectionStoreHolder.self) private var connections
    @Environment(KeychainServiceHolder.self) private var keychainHolder

    let method: AddKeyView.Method
    var onCreated: ((SSHKeyRecord) -> Void)?

    @State private var model: AddKeyModel
    @State private var failed = false
    /// The document picker is up (its destination is the model's).
    @State private var importing = false

    init(method: AddKeyView.Method = .generate, onCreated: ((SSHKeyRecord) -> Void)? = nil) {
        self.method = method
        self.onCreated = onCreated
        _model = State(initialValue: AddKeyModel(method: method))
    }

    var body: some View {
        if failed {
            AddKeyView(onCreated: onCreated, method: method)
        } else {
            NavigationStack {
                NSHPage(
                    screen: .addKey,
                    prepare: { model.register(on: $0) },
                    attach: { surface in
                        model.hosts = connections.store.connections.map(\.displayName)
                        model.attach(surface)
                    },
                    onMessage: handle,
                    onFailure: { failed = true })
                    .nshPage()
                    .navigationTitle("Add Key")
            }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.data]) { result in
                model.importFile(result)
            }
        }
    }

    /// What the screen tells the host.
    private func handle(_ message: [String: Any]) {
        switch message["ev"] as? String {
        case "cancel":
            dismiss()
        case "submit":
            Task {
                guard let record = await model.submit(to: keyStore, keychain: keychainHolder.service) else { return }
                onCreated?(record)
                dismiss()
            }
        case "import-file":
            if model.apply(message) { importing = true }
        default:
            if !model.apply(message) {
                Log.ui.error("add-key: no event \(String(describing: message["ev"]), privacy: .public)")
            }
        }
    }
}

/// The NSH Add Key page's form, owned by the host so a surface made again
/// starts from it (``NSHPage``'s third rule); the message that carries it
/// (`{"op": "add-key", …}`, `ui/src/add_key.rs`'s `Message`), which says
/// of the private key only whether there is one and whether it is behind a
/// passphrase; and what the page's intents do, as the SwiftUI page's
/// bindings and submit do.
@MainActor
final class AddKeyModel {
    /// Which field a picked file lands in.
    enum ImportTarget { case privateKey, publicKey }

    var method: AddKeyView.Method
    var name = ""
    var comment = ""
    var algorithm: SSHKeyAlgorithm = .ed25519
    var requireFaceID = true
    var storeInSecureEnclave = false
    var boundHosts: Set<String> = []
    /// Every host the user has (for the chips).
    var hosts: [String] = [] {
        didSet { if hosts != oldValue { post() } }
    }
    /// The private key as pasted: in the host's editor, never posted.
    private(set) var importPEM = ""
    /// Its passphrase, in the host's secure field, never posted.
    private(set) var importPassphrase = ""
    var importPublicLine = ""
    private(set) var generatedFingerprint: String?
    private(set) var working = false
    private(set) var errorMessage: String?
    private(set) var importTarget: ImportTarget = .privateKey
    /// The text fields' revision: the page takes their text when it moves.
    private(set) var fields: UInt64 = 1

    private weak var surface: NSHSurface?
    /// The private key's editor while the page shows one.
    fileprivate weak var pemEditor: KeyPEMEditor?

    init(method: AddKeyView.Method = .generate) {
        self.method = method
    }

    /// The pasted key is behind a passphrase, which has to be entered to read it.
    var needsPassphrase: Bool { SSHKeyFactory.isEncrypted(importPEM) }

    /// As the SwiftUI page decides: a name, and for Import a key (and its
    /// passphrase when it needs one); never while working.
    var canSubmit: Bool {
        guard !name.isEmpty, !working else { return false }
        switch method {
        case .generate: return true
        case .importKey: return !importPEM.isEmpty && (!needsPassphrase || !importPassphrase.isEmpty)
        case .hardware: return false
        }
    }

    /// Registers the page's own components on `surface`, before its screen
    /// runs: the private key's editor and the passphrase's field, bound to
    /// this form.
    func register(on surface: NSHSurface) {
        surface.register(component: "moshpit/key-pem") { [weak self] context in
            KeyPEMEditor(model: self, context: context)
        }
        surface.register(component: "moshpit/key-passphrase") { [weak self] context in
            NSHSecretField(text: self?.importPassphrase ?? "", context: context) { [weak self] text in
                self?.passphraseChanged(text)
            }
        }
    }

    /// `surface` shows the form from now on, starting with what it holds.
    func attach(_ surface: NSHSurface) {
        self.surface = surface
        post()
    }

    /// Applies what the screen told (`{"ev": …}`); false for an event it
    /// does not know (submit and cancel are the page's).
    @discardableResult
    func apply(_ message: [String: Any]) -> Bool {
        let value = message["value"]
        switch message["ev"] as? String {
        case "method":
            guard let id = value as? String, let method = Self.method(id) else { return false }
            self.method = method
        case "name":
            guard let text = value as? String else { return false }
            name = text
        case "comment":
            guard let text = value as? String else { return false }
            comment = text
        case "public-line":
            guard let text = value as? String else { return false }
            importPublicLine = text
        case "algorithm":
            switch value as? String {
            case "ed25519": algorithm = .ed25519
            case "rsa": algorithm = .rsa4096
            default: return false
            }
        case "set":
            guard let on = value as? Bool else { return false }
            switch message["key"] as? String {
            case "requireFaceID": requireFaceID = on
            case "secureEnclave": storeInSecureEnclave = on
            default: return false
            }
        case "host":
            guard let host = message["name"] as? String else { return false }
            if boundHosts.contains(host) { boundHosts.remove(host) } else { boundHosts.insert(host) }
        case "import-file":
            switch message["target"] as? String {
            case "private": importTarget = .privateKey
            case "public": importTarget = .publicKey
            default: return false
            }
            return true
        case "dismiss-error":
            errorMessage = nil
        default:
            return false
        }
        post()
        return true
    }

    /// The private key's text changed in its editor.
    fileprivate func pemChanged(_ text: String) {
        guard text != importPEM else { return }
        importPEM = text
        post()
    }

    /// The passphrase changed in its field.
    fileprivate func passphraseChanged(_ text: String) {
        guard text != importPassphrase else { return }
        importPassphrase = text
        post()
    }

    /// A file picked in the document picker, into the field it was picked
    /// for (the SwiftUI page's `handleImportedFile`): security-scoped, read
    /// as UTF-8 text.
    func importFile(_ result: Result<URL, Error>) {
        do {
            let url = try result.get()
            guard url.startAccessingSecurityScopedResource() else {
                throw SSHKeyFactory.KeyError.unsupported("couldn't access the selected file")
            }
            defer { url.stopAccessingSecurityScopedResource() }
            let text = try SSHKeyFactory.decodeImportedText(try Data(contentsOf: url))
            switch importTarget {
            case .privateKey:
                importPEM = text
                pemEditor?.show(text)
            case .publicKey:
                importPublicLine = text
                fields += 1
            }
        } catch {
            errorMessage = error.localizedDescription
        }
        post()
    }

    /// Generates or imports the key as the SwiftUI page does
    /// (``SSHKeyCreation``) and adds its record to `store`: the record, or
    /// nil with the reason on the card.
    func submit(to store: SSHKeyStore, keychain: KeychainService) async -> SSHKeyRecord? {
        guard canSubmit else { return nil }
        working = true
        post()
        defer {
            working = false
            post()
        }
        do {
            let record = try await SSHKeyCreation.create(request, keychain: keychain) { fingerprint in
                self.generatedFingerprint = fingerprint
            }
            store.add(record)
            return record
        } catch {
            errorMessage = error.localizedDescription
            return nil
        }
    }

    /// The form, as key creation takes it.
    var request: SSHKeyCreation.Request {
        SSHKeyCreation.Request(
            importing: method == .importKey, name: name,
            algorithm: storeInSecureEnclave ? .seP256 : algorithm, comment: comment,
            requireFaceID: requireFaceID, secureEnclave: storeInSecureEnclave,
            boundHosts: Array(boundHosts), privateKey: importPEM, passphrase: importPassphrase,
            publicKeyLine: importPublicLine)
    }

    private func post() {
        surface?.post(message)
    }

    /// The screen's message for the form now.
    var message: [String: Any] {
        var message: [String: Any] = [
            "op": "add-key",
            "method": Self.id(method),
            "algorithm": algorithm == .rsa4096 ? "rsa" : "ed25519",
            "requireFaceID": requireFaceID,
            "secureEnclave": storeInSecureEnclave,
            "hosts": hosts,
            "bound": boundHosts.sorted(),
            "pem": ["empty": importPEM.isEmpty, "encrypted": needsPassphrase],
            "name": name,
            "comment": comment,
            "publicLine": importPublicLine,
            "fields": fields,
            "canSubmit": canSubmit,
            "working": working,
        ]
        if let generatedFingerprint { message["fingerprint"] = generatedFingerprint }
        if let errorMessage { message["error"] = errorMessage }
        return message
    }

    /// What the screen calls a method.
    nonisolated static func id(_ method: AddKeyView.Method) -> String {
        switch method {
        case .generate: "generate"
        case .importKey: "import"
        case .hardware: "hardware"
        }
    }

    nonisolated static func method(_ id: String) -> AddKeyView.Method? {
        AddKeyView.Method.allCases.first { Self.id($0) == id }
    }
}

/// Making a key and keeping it, for both Add Key pages: generated on the
/// device (or in the Secure Enclave), or imported (any format the app reads,
/// stored as an unencrypted OpenSSH key), its private half in the Keychain,
/// behind Face ID when asked; the record the store keeps.
enum SSHKeyCreation {
    struct Request: Sendable {
        var importing: Bool
        var name: String
        /// What to generate (the Secure Enclave's P-256 when it holds it).
        var algorithm: SSHKeyAlgorithm
        var comment: String
        var requireFaceID: Bool
        var secureEnclave: Bool
        var boundHosts: [String]
        var privateKey: String
        var passphrase: String
        var publicKeyLine: String
    }

    /// The new key's record, once its private half is in the Keychain;
    /// `generated` hears its fingerprint first (shown even if the Keychain
    /// then refuses it).
    @MainActor
    static func create(_ request: Request, keychain: KeychainService,
                       generated: (String) -> Void) async throws -> SSHKeyRecord {
        let key: SSHKeyFactory.Generated
        if request.importing {
            let (pem, passphrase, publicLine, comment) =
                (request.privateKey, request.passphrase, request.publicKeyLine, request.comment)
            // Off the main thread: unlocking a passphrase-protected OpenSSH
            // key is bcrypt, a deliberately slow fraction of a second
            // (longer for keys made with more rounds).
            key = try await Task.detached(priority: .userInitiated) {
                try SSHKeyFactory.importKey(
                    privatePEM: pem,
                    passphrase: passphrase.isEmpty ? nil : passphrase,
                    publicKeyLine: publicLine.isEmpty ? nil : publicLine,
                    comment: comment)
            }.value
        } else {
            key = try SSHKeyFactory.generate(
                algorithm: request.algorithm,
                comment: request.comment.isEmpty ? request.name : request.comment)
        }
        generated(key.fingerprint)
        let ref = keychain.generateRef()
        try await keychain.saveSecret(key.privateBlob, forRef: ref, requireBiometry: request.requireFaceID)
        return SSHKeyRecord(
            name: request.name,
            algorithm: key.algorithm,
            bits: key.bits,
            comment: request.comment,
            fingerprint: key.fingerprint,
            publicKey: key.publicKeyLine,
            source: request.importing ? .imported : (request.secureEnclave ? .secureEnclave : .generated),
            boundHosts: request.boundHosts,
            keychainRef: ref,
            requireBiometry: request.requireFaceID)
    }
}

/// `moshpit/key-pem`: Add Key's private key editor, the host's own
/// (``NSHSecretEditor``): what is typed goes to the form and nowhere else.
typealias KeyPEMEditor = NSHSecretEditor

extension NSHSecretEditor {
    /// Add Key's private key editor: it starts from the form's key and
    /// tells it each change.
    convenience init(model: AddKeyModel?, context: NSHComponentContext?) {
        self.init(text: model?.importPEM ?? "", context: context) { [weak model] text in
            model?.pemChanged(text)
        }
        model?.pemEditor = self
    }
}
