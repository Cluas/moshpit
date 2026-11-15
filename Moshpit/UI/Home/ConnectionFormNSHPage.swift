import MoshpitKit
import SwiftUI
import UIKit

/// Home › Add Connection and Edit Connection drawn by NSH
/// (`ui/src/connection_form.rs`). If its screen can't run, the sheet says
/// so (``NSHPageFailed``).
///
/// The draft is the shared Rust model's, kept by the host
/// (``ConnectionFormModel``): the screen tells each edit as it is made and
/// the model takes it, so a page made again starts from it. Saving is
/// ``ConnectionDraft/save(existing:store:keychain:settings:keys:secrets:release:forgetPairing:)``:
/// the app's connection store and keychain. The key picker, the error card and
/// the sheet's dismissal are the host's.
struct ConnectionFormNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(AppSettings.self) private var settings
    @Environment(SSHKeyStore.self) private var keyStore

    let store: ConnectionStore
    let keychain: KeychainService
    var existing: ServerConnection?
    /// Deletes the connection edited, once the person confirmed here (Home's
    /// delete: its session, push pairing and keychain item go with it).
    var onDelete: ((ServerConnection) -> Void)?
    /// Drops a keychain item the saved connection left (B325), as Home
    /// decides (not while its session runs).
    var releaseSecret: (UUID, String) async -> Void = { _, _ in }

    @State private var model: ConnectionFormModel
    @State private var failed = false
    @State private var showKeyPicker = false
    @State private var selectedKeyId: UUID?
    @State private var saving = false
    @State private var confirmingDelete = false

    init(store: ConnectionStore, keychain: KeychainService, existing: ServerConnection? = nil,
         onDelete: ((ServerConnection) -> Void)? = nil,
         releaseSecret: @escaping (UUID, String) async -> Void = { _, _ in }) {
        self.store = store
        self.keychain = keychain
        self.existing = existing
        self.onDelete = onDelete
        self.releaseSecret = releaseSecret
        // A new connection starts from the settings (B256): the app's one
        // settings object, the environment's too — an init can't read that.
        let draft = existing.map(ConnectionDraft.init) ?? ConnectionDraft(newWith: .shared)
        _model = State(initialValue: ConnectionFormModel(
            draft: draft, saved: store.connections, editing: existing?.id))
        _selectedKeyId = State(initialValue: draft.selectedKeyId)
    }

    /// Keys that can sign in: all but hardware tokens' (an ECDSA-sk key's
    /// private half lives on the token).
    private var connectableKeys: [SSHKeyRecord] {
        keyStore.keys.filter { $0.algorithm != .ecdsaSK }
    }

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            NavigationStack {
                NSHPage(
                    screen: .connectionForm,
                    prepare: { surface in
                        HomeNSHComponents.register(on: surface)
                        model.register(on: surface)
                    },
                    attach: { surface in
                        model.attach(surface, settings: settings, keys: keyStore.keys)
                    },
                    onMessage: handle,
                    onFailure: { failed = true })
                    .nshPage()
                    .navigationTitle(existing == nil ? "Add Connection" : "Edit Connection")
                    .appSheet(isPresented: $showKeyPicker) {
                        KeyPickerNSHPage(keys: connectableKeys, selectedKeyId: $selectedKeyId)
                    }
                    .onChange(of: selectedKeyId) { _, id in
                        model.pickKey(id, settings: settings, keys: keyStore.keys)
                    }
                    .onChange(of: keyStore.keys) { _, keys in
                        model.keysChanged(settings: settings, keys: keys)
                    }
                    .confirmationDialog(
                        "Delete \(existing?.displayName ?? String(localized: "connection"))?",
                        isPresented: $confirmingDelete,
                        titleVisibility: .visible
                    ) {
                        Button("Delete", role: .destructive) {
                            guard let existing else { return }
                            dismiss()
                            onDelete?(existing)
                        }
                        Button("Cancel", role: .cancel) {}
                    } message: {
                        Text("Removes the saved server and its stored credentials from this device.")
                    }
            }
        }
    }

    private func handle(_ message: [String: Any]) {
        switch model.handle(message) {
        case .pickKey?:
            showKeyPicker = true
        case .cancel?:
            dismiss()
        case .delete?:
            if existing != nil, onDelete != nil { confirmingDelete = true }
        case .save?:
            guard model.draft.canSave(keys: keyStore.keys, secrets: model.secrets), !saving else { return }
            saving = true
            Task {
                defer { saving = false }
                do {
                    let id = existing?.id
                    try await model.draft.save(existing: existing, store: store, keychain: keychain,
                                               settings: settings, keys: keyStore.keys,
                                               secrets: model.secrets,
                                               release: { ref in if let id { await releaseSecret(id, ref) } })
                    dismiss()
                } catch {
                    // The form keeps what it holds and says why under its
                    // bar (§6.5).
                    model.failed(error)
                }
            }
        case nil:
            break
        }
    }
}

/// The NSH form's host: the shared draft (``ConnectionDraft``, the Rust
/// model's) and its data path (`ui/src/connection_form.rs`'s `FormState`
/// and `Field`): what the screen starts from, the edits it tells. The
/// password and the pasted key are typed into this host's own fields
/// (``register(on:)``) and kept in ``secrets``, not the screen's: they never
/// cross the channel, enter the NSH engine (NSH-GAPS #11) or reach the
/// model; the screen and the model hear only whether each holds anything.
@MainActor
final class ConnectionFormModel {
    private(set) var draft: ConnectionDraft
    /// The password and the pasted key, as the host's fields hold them.
    private(set) var secrets = ConnectionSecrets()
    private weak var surface: NSHSurface?
    /// The key row as last posted: nil when no key was picked, or the
    /// picked one was gone.
    private var postedKey: ConnectionKeyRow?

    /// The saved connections and the one edited: for the warning of a
    /// duplicate (B532).
    private let saved: [ServerConnection]
    private let editing: UUID?

    init(draft: ConnectionDraft, saved: [ServerConnection] = [], editing: UUID? = nil) {
        self.draft = draft
        self.saved = saved
        self.editing = editing
    }

    /// Registers the form's own fields on `surface`, before its screen
    /// runs: the password's secure field and the private key's editor,
    /// each starting from what was typed (a field made again, the method
    /// switched back, shows it) and writing every change to ``secrets``.
    func register(on surface: NSHSurface) {
        surface.register(component: "moshpit/connection-password") { [weak self] context in
            NSHSecretField(text: self?.secrets.password ?? "", context: context,
                           filled: { !$0.isEmpty }) { [weak self] text in
                self?.passwordChanged(text)
            }
        }
        surface.register(component: "moshpit/connection-pem") { [weak self] context in
            NSHSecretEditor(text: self?.secrets.privateKeyPEM ?? "", context: context,
                            filled: ConnectionSecrets.holdsKey) { [weak self] text in
                self?.pemChanged(text)
            }
        }
    }

    /// The password, as its field holds it now.
    func passwordChanged(_ text: String) {
        secrets.password = text
    }

    /// The pasted private key, as its editor holds it now.
    func pemChanged(_ text: String) {
        secrets.privateKeyPEM = text
    }

    /// `surface` shows the form from now on, starting from the draft.
    func attach(_ surface: NSHSurface, settings: AppSettings, keys: [SSHKeyRecord]) {
        self.surface = surface
        post(settings: settings, keys: keys)
    }

    /// The key picker picked `id` (nil: paste a PEM).
    func pickKey(_ id: UUID?, settings: AppSettings, keys: [SSHKeyRecord]) {
        guard id != draft.selectedKeyId else { return }
        draft = connectionDraftPickKey(draft: draft, id: id?.uuidString)
        post(settings: settings, keys: keys)
    }

    /// The keys changed under the form (one deleted or renamed in another
    /// window): the form shows the picked key as it is now — a deleted one
    /// as "Paste PEM" with the PEM editor, not a key that Save would find
    /// gone and store a connection with no credential for (B290). True
    /// when it posted.
    @discardableResult
    func keysChanged(settings: AppSettings, keys: [SSHKeyRecord]) -> Bool {
        let now = connectionFormKeyRow(draft: draft, keys: keys.map(KeyRecord.init))
        guard now != postedKey else { return false }
        post(settings: settings, keys: keys)
        return true
    }

    private func post(settings: AppSettings, keys: [SSHKeyRecord]) {
        postedKey = connectionFormKeyRow(draft: draft, keys: keys.map(KeyRecord.init))
        surface?.post(message(settings: settings, keys: keys))
    }

    /// Save failed with `error`: the screen shows why under its bar
    /// (`ui/src/connection_form.rs`'s `SaveFailure`).
    func failed(_ error: Error) {
        surface?.post(Self.failure(error))
    }

    /// `{"op": "failed", …}` for `error`: the secret the keychain did not
    /// keep and Try again, or the refusal's words and no retry.
    static func failure(_ error: Error) -> [String: Any] {
        switch error {
        case let failure as ConnectionKeychainFailure:
            ["op": "failed", "secret": failure.secret.rawValue, "reason": failure.description, "retry": true]
        case let refusal as ConnectionSaveRefusal:
            ["op": "failed", "reason": refusal.description, "retry": false]
        default:
            ["op": "failed", "reason": String(describing: error), "retry": true]
        }
    }

    /// What the screen told: an edit made to the draft (the model's
    /// rules: a secret told through the screen is not taken), or an intent.
    func handle(_ message: [String: Any]) -> ConnectionFormIntent? {
        guard JSONSerialization.isValidJSONObject(message),
              let data = try? JSONSerialization.data(withJSONObject: message) else { return nil }
        let handled = connectionDraftHandle(draft: draft, messageJson: String(decoding: data, as: UTF8.self))
        draft = handled.draft
        return handled.intent
    }

    /// `{"op": "form", …}` of the draft, as the model makes it: of the
    /// password and the pasted key, only whether there is one.
    func message(settings: AppSettings, keys: [SSHKeyRecord]) -> [String: Any] {
        let json = connectionFormMessage(
            draft: draft, given: secrets.given, keys: keys.map(KeyRecord.init),
            udpStart: Int64(settings.udpRangeStart), udpEnd: Int64(settings.udpRangeEnd),
            saved: saved.map(ConnectionRecord.init), editing: editing?.uuidString)
        return (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
    }
}
