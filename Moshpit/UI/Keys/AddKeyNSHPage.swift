import MoshpitKit
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// How Add Key makes the key: generated on this phone, imported, or a
/// hardware key's.
enum AddKeyMethod: String, CaseIterable { case generate, importKey, hardware }

/// Add Key drawn by NSH (`ui/src/add_key.rs`) in the sheet's own stack. The
/// form lives in the host (``AddKeyModel``); the private key's text and its
/// passphrase are typed into the host's own views, which the page lays out
/// (`moshpit/key-pem`, `moshpit/key-passphrase`), so key material never
/// reaches NSH. Generating and importing are the SwiftUI page's
/// (``SSHKeyCreation``): formats converted at import, the private half in
/// the Keychain behind Face ID when asked, the record in the store. A file
/// is picked with the system's document picker. If its screen can't run,
/// the page says so (``NSHPageFailed``).
struct AddKeyNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(SSHKeyStore.self) private var keyStore
    @Environment(ConnectionStoreHolder.self) private var connections
    @Environment(KeychainServiceHolder.self) private var keychainHolder

    let method: AddKeyMethod
    var onCreated: ((SSHKeyRecord) -> Void)?

    @State private var model: AddKeyModel
    @State private var failed = false
    /// The document picker is up (its destination is the model's).
    @State private var importing = false

    init(method: AddKeyMethod = .generate, onCreated: ((SSHKeyRecord) -> Void)? = nil) {
        self.method = method
        self.onCreated = onCreated
        _model = State(initialValue: AddKeyModel(method: method))
    }

    /// Import Again (B781): `record`'s private key, which isn't on this
    /// phone, imported into the record itself.
    init(refilling record: SSHKeyRecord) {
        self.method = .importKey
        self.onCreated = nil
        _model = State(initialValue: AddKeyModel(refilling: record))
    }

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            NavigationStack {
                NSHPage(
                    screen: .addKey,
                    prepare: { model.register(on: $0) },
                    attach: { model.attach($0) },
                    onEvent: { if case .addKey(let event) = $0 { handle(event) } },
                    onFailure: { failed = true })
                    .nshPage()
                    .navigationTitle(model.refilling == nil ? "Add Key" : "Import Again")
            }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.data]) { result in
                model.importFile(result)
            }
        }
    }

    /// What the screen tells the host.
    private func handle(_ event: AddKeyEvent) {
        switch event {
        case .cancel:
            dismiss()
        case .submit:
            Task {
                guard let record = await model.submit(to: keyStore, keychain: keychainHolder.service) else { return }
                onCreated?(record)
                dismiss()
            }
        case .importFile:
            if model.apply(event) { importing = true }
        default:
            model.apply(event)
        }
    }
}

/// The NSH Add Key page's form, owned by the host so a surface made again
/// starts from it (``NSHPage``'s third rule); the state that carries it
/// (``AddKeyPageState``, the screen's own through the page bridge), which
/// says of the private key only whether there is one and whether it is
/// behind a passphrase; and what the page's intents do, as the SwiftUI
/// page's bindings and submit do.
@MainActor
final class AddKeyModel {
    /// Which field a picked file lands in.
    enum ImportTarget { case privateKey, publicKey }

    var method: AddKeyMethod
    var name = ""
    var comment = ""
    var algorithm: SSHKeyAlgorithm = .ed25519
    var requireFaceID = true
    var storeInSecureEnclave = false
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

    /// The key Import Again brings back into its own record (B781): its
    /// name and comment are kept, not asked for.
    let refilling: SSHKeyRecord?

    init(method: AddKeyMethod = .generate) {
        self.method = method
        self.refilling = nil
    }

    /// Import Again of `record` (B781): Face ID as it had it.
    init(refilling record: SSHKeyRecord) {
        self.method = .importKey
        self.refilling = record
        self.name = record.name
        self.comment = record.comment
        self.requireFaceID = record.requireBiometry
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

    /// Applies what the screen told; false for an event that is not the
    /// form's (submit and cancel are the page's).
    @discardableResult
    func apply(_ event: AddKeyEvent) -> Bool {
        switch event {
        case .method(let id):
            guard let method = Self.method(id) else { return false }
            self.method = method
        case .name(let text):
            name = text
        case .comment(let text):
            comment = text
        case .publicLine(let text):
            importPublicLine = text
        case .algorithm(let id):
            switch id {
            case "ed25519": algorithm = .ed25519
            case "rsa": algorithm = .rsa4096
            default: return false
            }
        case .set(.requireFaceId, let on):
            requireFaceID = on
        case .set(.secureEnclave, let on):
            storeInSecureEnclave = on
        case .importFile(let target):
            importTarget = target == .privateKey ? .privateKey : .publicKey
            return true
        case .dismissError:
            errorMessage = nil
        case .submit, .cancel:
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
            if let refilling {
                return try await SSHKeyCreation.importAgain(request, into: refilling, keychain: keychain,
                                                             store: store)
            }
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
            privateKey: importPEM, passphrase: importPassphrase,
            publicKeyLine: importPublicLine)
    }

    private func post() {
        surface?.post(.addKey(state: page))
    }

    /// The form now, as the page bridge carries it.
    var page: AddKeyPageState {
        AddKeyPageState(
            method: Self.id(method), algorithm: algorithm == .rsa4096 ? "rsa" : "ed25519",
            requireFaceId: requireFaceID, secureEnclave: storeInSecureEnclave,
            pem: AddKeyPem(empty: importPEM.isEmpty, encrypted: needsPassphrase),
            name: name, comment: comment, publicLine: importPublicLine, fields: fields,
            canSubmit: canSubmit, working: working, fingerprint: generatedFingerprint, error: errorMessage,
            again: refilling != nil)
    }

    /// What the screen calls a method.
    nonisolated static func id(_ method: AddKeyMethod) -> String {
        switch method {
        case .generate: "generate"
        case .importKey: "import"
        case .hardware: "hardware"
        }
    }

    nonisolated static func method(_ id: String) -> AddKeyMethod? {
        AddKeyMethod.allCases.first { Self.id($0) == id }
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
        var privateKey: String
        var passphrase: String
        var publicKeyLine: String
    }

    /// The new key's record, once its private half is in the Keychain;
    /// `generated` hears its fingerprint first (shown even if the Keychain
    /// then refuses it). `generate` makes a new key (a test's own).
    @MainActor
    static func create(_ request: Request, keychain: KeychainService,
                       generate: @escaping @Sendable (SSHKeyAlgorithm, String) throws -> SSHKeyFactory.Generated
                           = { try SSHKeyFactory.generate(algorithm: $0, comment: $1) },
                       generated: (String) -> Void) async throws -> SSHKeyRecord {
        let key: SSHKeyFactory.Generated
        if request.importing {
            key = try await imported(request)
        } else {
            let (algorithm, comment) = (request.algorithm, request.comment.isEmpty ? request.name : request.comment)
            // Off the main thread too, as an import is (B605): an RSA-4096
            // key is a search for two large primes, seconds on a phone, and
            // made here it froze the sheet — its spinner with it — for all
            // of them.
            key = try await Task.detached(priority: .userInitiated) {
                try generate(algorithm, comment)
            }.value
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
            keychainRef: ref,
            requireBiometry: request.requireFaceID)
    }
}

extension SSHKeyCreation {
    /// The key `request` pastes, read in any format the app reads — off the
    /// main thread: unlocking a passphrase-protected OpenSSH key is bcrypt,
    /// a deliberately slow fraction of a second (longer for keys made with
    /// more rounds).
    static func imported(_ request: Request) async throws -> SSHKeyFactory.Generated {
        let (pem, passphrase, publicLine, comment) =
            (request.privateKey, request.passphrase, request.publicKeyLine, request.comment)
        return try await Task.detached(priority: .userInitiated) {
            try SSHKeyFactory.importKey(
                privatePEM: pem,
                passphrase: passphrase.isEmpty ? nil : passphrase,
                publicKeyLine: publicLine.isEmpty ? nil : publicLine,
                comment: comment)
        }.value
    }

    /// Why Import Again kept nothing.
    enum ImportAgainError: LocalizedError {
        /// The key isn't the one the record was made with.
        case differentKey

        var errorDescription: String? {
            String(localized: "That's a different key from the one this entry was made with. Import that key, or delete this entry.")
        }
    }

    /// Import Again (B781): `record`'s private key, which didn't come with a
    /// restore, imported into the record itself — its item under the
    /// record's own keychain account, so every connection that signs in
    /// with it (by key or by item) does again; its name, comment and
    /// connections kept. A key with another fingerprint is refused before
    /// anything is written (and again by the store).
    @MainActor
    static func importAgain(_ request: Request, into record: SSHKeyRecord, keychain: KeychainService,
                            store: SSHKeyStore) async throws -> SSHKeyRecord {
        let key = try await imported(request)
        guard record.fingerprint.isEmpty || record.fingerprint == key.fingerprint else {
            throw ImportAgainError.differentKey
        }
        // A key found missing names the item it was missing (B781).
        guard let ref = record.keychainRef else { throw KeychainError.notFound }
        try await keychain.saveSecret(key.privateBlob, forRef: ref, requireBiometry: request.requireFaceID)
        switch store.refill(record.id, with: key, requireBiometry: request.requireFaceID) {
        case .refilled:
            return store.keys.first { $0.id == record.id } ?? record
        case .differentKey:
            throw ImportAgainError.differentKey
        case .unknown:
            throw KeychainError.notFound
        }
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
