import SwiftUI
import UIKit

/// Settings › SSH Keys, as Settings presents it: the NSH page, with a
/// capture run's sheet and keys over it (DEBUG).
struct SSHKeysPage: View {
    var body: some View {
        SSHKeysNSHPage()
        #if DEBUG
            // A capture run's sheet over the page, and its keys: in memory,
            // never written.
            .modifier(KeysCapture.Present())
            .modifier(KeysCapture.Store())
        #endif
    }
}

/// Settings › SSH Keys drawn by NSH (`ui/src/ssh_keys.rs`) in the sheet's
/// own stack. The keys' metadata comes from the store (``SSHKeysModel``,
/// again whenever it changes); what only the host has stays here: a row's
/// actions copy or share the public key or the fingerprint, Delete asks with
/// the system alert the page shows and deletes the key as the SwiftUI page
/// does (``SSHKeyDeletion``: its private key, then its record — or an alert
/// says which connections still use it), + and an empty group's button
/// present Add Key, on the method the button names. If its screen can't
/// run, the page says so (``NSHPageFailed``).
struct SSHKeysNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(SSHKeyStore.self) private var keyStore
    @Environment(ConnectionStoreHolder.self) private var connections
    @Environment(KeychainServiceHolder.self) private var keychainHolder
    @State private var model = SSHKeysModel()
    @State private var failed = false
    @State private var showAdd = false
    /// The method Add Key opens on: an empty group's button picks it.
    @State private var addMethod: AddKeyMethod = .generate

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            let snapshot = Self.snapshot(keyStore, connections.store)
            NavigationStack {
                NSHPage(
                    screen: .sshKeys,
                    attach: { surface in
                        model.attach(surface, current: snapshot)
                        #if DEBUG
                        if let id = KeysCapture.deleteRequested(in: snapshot) {
                            model.confirmDelete(id)
                        }
                        #endif
                    },
                    onEvent: { if case .sshKeys(let event) = $0 { handle(event) } },
                    onFailure: { failed = true })
                    .nshPage()
                    .navigationTitle("SSH Keys")
                    .onAppear { model.update(snapshot) }
                    .onChange(of: snapshot) { _, new in model.update(new) }
            }
            .appSheet(isPresented: $showAdd) { AddKeyNSHPage(method: addMethod) }
        }
    }

    /// The page's keys and who signs in with each; a capture run's keys,
    /// which its connections don't name, with the users it shows.
    private static func snapshot(_ keys: SSHKeyStore, _ connections: ConnectionStore) -> SSHKeysSnapshot {
        var snapshot = SSHKeysSnapshot(store: keys, connections: connections)
        #if DEBUG
        if KeysCapture.seeded { snapshot.users.merge(KeysCapture.users) { _, capture in capture } }
        #endif
        return snapshot
    }

    /// What the screen tells the host.
    private func handle(_ event: SshKeysEvent) {
        let key = { (id: String) in
            UUID(uuidString: id).flatMap { id in keyStore.keys.first { $0.id == id } }
        }
        switch event {
        case .done:
            dismiss()
        case .add(let method):
            // The bar's + and Generate key open on Generate; Import key on
            // Import.
            addMethod = method == .import ? .importKey : .generate
            showAdd = true
        case .copyPublicKey(let id):
            if let key = key(id) { SSHKeysModel.copy(key.publicKey) }
        case .copyFingerprint(let id):
            if let key = key(id) { SSHKeysModel.copy(key.fingerprint) }
        case .sharePublicKey(let id):
            if let key = key(id) { model.share(key) }
        case .confirmDelete(let id):
            if let key = key(id) {
                model.requestDelete(key, SSHKeyDeletion.check(key, connections: connections.store, keys: keyStore))
            }
        case .cancelDelete:
            model.cancelDelete()
        case .delete(let id):
            if let key = key(id) {
                let keychain = keychainHolder.service
                model.delete(key, from: keyStore, connections: connections.store) {
                    try await keychain.delete(forRef: $0)
                }
            }
        case .dismissNotice:
            model.dismissNotice()
        }
    }
}

/// What the NSH SSH Keys page shows: the store's keys, as the SwiftUI page
/// splits them, and the saved connections that sign in with each (B602:
/// the row listed the hosts picked once at Add Key, a label that bound
/// nothing and went stale with every rename or delete).
struct SSHKeysSnapshot: Equatable {
    var device: [SSHKeyRecord]
    var imported: [SSHKeyRecord]
    /// The names of the connections that sign in with each key, by key id;
    /// none for a key while that can't be told.
    var users: [UUID: [String]]

    init(store: SSHKeyStore, connections: ConnectionStore) {
        device = store.deviceKeys
        imported = store.importedKeys
        users = Dictionary(store.keys.map {
            ($0.id, connections.names(usingKey: $0.id, ref: $0.keychainRef) ?? [])
        }, uniquingKeysWith: { first, _ in first })
    }

    init(device: [SSHKeyRecord], imported: [SSHKeyRecord], users: [UUID: [String]] = [:]) {
        self.device = device
        self.imported = imported
        self.users = users
    }

    func contains(_ id: UUID) -> Bool {
        device.contains { $0.id == id } || imported.contains { $0.id == id }
    }
}

/// What the NSH SSH Keys page shows, owned by the host so a surface made
/// again starts from it (``NSHPage``'s third rule): the keys, the one the
/// delete alert asks about, and why a key wasn't deleted; the state that
/// carries them (``SshKeysPageState``, the screen's own through the page
/// bridge): metadata only, never a key; and what the page's intents do.
@MainActor
final class SSHKeysModel {
    /// The key the delete alert asks about, while it shows.
    private(set) var deleting: UUID?
    /// Why a key wasn't deleted, while the alert saying so shows.
    private(set) var notice: SSHKeyDeletion.Failure?
    private(set) var snapshot: SSHKeysSnapshot?
    private weak var surface: NSHSurface?

    /// `surface` shows the keys from now on, starting with the current ones.
    func attach(_ surface: NSHSurface) {
        self.surface = surface
        post()
    }

    /// ``attach(_:)``, the model filled from `current` (the page's keys as
    /// its body has it now) while it has none (B366): the screen starts from
    /// the page's state whether or not `onAppear` came first. A state the
    /// model has is newer (`onChange` keeps it) and stays.
    func attach(_ surface: NSHSurface, current snapshot: SSHKeysSnapshot) {
        if self.snapshot == nil {
            self.surface = nil
            update(snapshot)
        }
        attach(surface)
    }

    /// The keys as they are now. An alert asking about a key that is gone
    /// goes with it.
    func update(_ snapshot: SSHKeysSnapshot) {
        guard snapshot != self.snapshot else { return }
        self.snapshot = snapshot
        if let deleting, !snapshot.contains(deleting) { self.deleting = nil }
        post()
    }

    /// Delete Key was picked in key `id`'s actions: the alert asks.
    func confirmDelete(_ id: UUID) {
        deleting = id
        post()
    }

    /// Delete Key was picked in `key`'s menu, and `check` is what deleting
    /// it would do: the alert asks when nothing uses the key, and otherwise
    /// says why it can't go.
    func requestDelete(_ key: SSHKeyRecord, _ check: SSHKeyDeletion.Check) {
        switch check {
        case .free: confirmDelete(key.id)
        case .inUse(let names): show(.inUse(names))
        case .unknown: show(.unknown)
        }
    }

    /// The alert's Cancel.
    func cancelDelete() {
        guard deleting != nil else { return }
        deleting = nil
        post()
    }

    /// The notice's OK.
    func dismissNotice() {
        guard notice != nil else { return }
        notice = nil
        post()
    }

    /// The alert's Delete: the key's private key, then its record, as on the
    /// SwiftUI page (``SSHKeyDeletion``); the notice says why when either
    /// stays. `deleteItem` is the keychain's delete.
    func delete(_ key: SSHKeyRecord, from store: SSHKeyStore, connections: ConnectionStore,
                deleteItem: @escaping (String) async throws -> Void) {
        deleting = nil
        post()
        Task {
            do {
                try await SSHKeyDeletion.delete(key, from: store, connections: connections,
                                                deleteItem: deleteItem)
            } catch let failure as SSHKeyDeletion.Failure {
                show(failure)
            } catch {
                show(.keychain(String(describing: error)))
            }
            snapshot = SSHKeysSnapshot(store: store, connections: connections)
            post()
        }
    }

    private func show(_ failure: SSHKeyDeletion.Failure) {
        deleting = nil
        notice = failure
        post()
    }

    /// Copy Public Key and Copy Fingerprint.
    static func copy(_ text: String, pasteboard: UIPasteboard = .general) {
        guard !text.isEmpty else { return }
        pasteboard.string = text
        Haptics.tap()
    }

    /// Share Public Key: the system's share sheet, from the key's row.
    func share(_ key: SSHKeyRecord) {
        guard !key.publicKey.isEmpty, let surface else { return }
        let activity = UIActivityViewController(activityItems: [key.publicKey], applicationActivities: nil)
        let row = AccentRowMenus.view(identified: Self.rowID(key.id), in: surface) ?? surface
        activity.popoverPresentationController?.sourceView = row
        activity.popoverPresentationController?.sourceRect = row.bounds
        surface.presenter?.present(activity, animated: true)
    }

    private func post() {
        guard let snapshot else { return }
        surface?.post(.sshKeys(state: Self.state(snapshot, deleting: deleting, notice: notice)))
    }

    /// What the page shows.
    nonisolated static func state(_ snapshot: SSHKeysSnapshot, deleting: UUID?,
                                  notice: SSHKeyDeletion.Failure? = nil,
                                  now: Date = Date()) -> SshKeysPageState {
        SshKeysPageState(
            device: snapshot.device.map { key($0, users: snapshot.users[$0.id] ?? [], now: now) },
            imported: snapshot.imported.map { key($0, users: snapshot.users[$0.id] ?? [], now: now) },
            deleting: deleting?.uuidString,
            notice: notice.map { SshKeysNotice(title: $0.title, message: $0.message) })
    }

    /// How long a key goes unused before its row says so.
    nonisolated static let staleAfter: TimeInterval = 90 * 86400

    /// One key's metadata, as the SwiftUI row reads it: the avatar its source
    /// (and an RSA key's age) picks, its badge, fingerprint and the
    /// connections that sign in with it (B602), and,
    /// for a stale key, the month and year it was last used.
    nonisolated static func key(_ record: SSHKeyRecord, users: [String], now: Date) -> SshKeyRow {
        let stale = record.lastUsedAt.map { now.timeIntervalSince($0) > staleAfter } ?? false
        let avatar: SshKeyAvatar = switch record.source {
        case .secureEnclave: .secureEnclave
        case .hardware: .hardware
        case .generated, .imported: record.algorithm == .rsa4096 && stale ? .gray : .imported
        }
        return SshKeyRow(
            id: record.id.uuidString, name: record.name, badge: record.badgeText, avatar: avatar,
            stale: stale, fingerprint: record.fingerprint, hosts: users,
            unusedSince: stale ? record.lastUsedAt?.formatted(.dateTime.month(.abbreviated).year()) : nil,
            hasPublicKey: !record.publicKey.isEmpty)
    }

    /// The accessibility identifier of key `id`'s row on both pages.
    nonisolated static func rowID(_ id: UUID) -> String { "ssh-key-row-\(id.uuidString)" }
}

extension UIView {
    /// The view controller that can present from this view: the one whose
    /// view holds it, or what that presents on top.
    @MainActor
    var presenter: UIViewController? {
        var responder: UIResponder? = self
        while let next = responder?.next {
            if let controller = next as? UIViewController {
                var top = controller
                while let presented = top.presentedViewController, !presented.isBeingDismissed { top = presented }
                return top
            }
            responder = next
        }
        return window?.rootViewController
    }
}
