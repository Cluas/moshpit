import SwiftUI
import UIKit

/// Settings › SSH Keys: the SwiftUI page (``SSHKeysView``), or the NSH one
/// while its switch is on (``NSHScreen/sshKeys``). With the switch off the
/// SwiftUI page is all there is: no NSH code runs.
struct SSHKeysPage: View {
    var body: some View {
        content
        #if DEBUG
            // A capture run's sheet over the page, and its keys: in memory,
            // never written.
            .modifier(KeysCapture.Present())
            .modifier(KeysCapture.Store())
        #endif
    }

    @ViewBuilder
    private var content: some View {
        if Self.showsNSH {
            SSHKeysNSHPage()
        } else {
            SSHKeysView()
        }
    }

    static var showsNSH: Bool {
        #if DEBUG
        if let screen = CaptureScreen.requested, KeysCapture.forcesNSHKeys(screen) { return NSHSurface.available }
        #endif
        return NSHScreen.sshKeys.isOn
    }
}

/// Settings › SSH Keys drawn by NSH (`ui/src/ssh_keys.rs`) in the sheet's
/// own stack. The keys' metadata comes from the store (``SSHKeysModel``,
/// again whenever it changes); what only the host has stays here: a row's
/// actions copy or share the public key or the fingerprint, Delete asks with
/// the system alert the page shows and deletes the key as the SwiftUI page
/// does (``SSHKeyDeletion``: its private key, then its record — or an alert
/// says which connections still use it), + and an empty group's button
/// present Add Key (the one its own switch picks), on the method the button
/// names. If NSH cannot run here, the SwiftUI page shows.
struct SSHKeysNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(SSHKeyStore.self) private var keyStore
    @Environment(ConnectionStoreHolder.self) private var connections
    @Environment(KeychainServiceHolder.self) private var keychainHolder
    @State private var model = SSHKeysModel()
    @State private var failed = false
    @State private var showAdd = false
    /// The method Add Key opens on: an empty group's button picks it.
    @State private var addMethod: AddKeyView.Method = .generate

    var body: some View {
        if failed {
            SSHKeysView()
        } else {
            let snapshot = SSHKeysSnapshot(store: keyStore)
            NavigationStack {
                NSHPage(
                    screen: .sshKeys,
                    attach: { surface in
                        model.attach(surface, current: snapshot)
                        #if DEBUG
                        if let id = KeysCapture.deleteRequested(nsh: true, in: snapshot) {
                            model.confirmDelete(id)
                        }
                        #endif
                    },
                    onMessage: handle,
                    onFailure: { failed = true })
                    .nshPage()
                    .navigationTitle("SSH Keys")
                    .onAppear { model.update(snapshot) }
                    .onChange(of: snapshot) { _, new in model.update(new) }
            }
            .appSheet(isPresented: $showAdd) { AddKeyPage(method: addMethod) }
        }
    }

    /// What the screen tells the host.
    private func handle(_ message: [String: Any]) {
        let id = (message["id"] as? String).flatMap(UUID.init(uuidString:))
        let key = id.flatMap { id in keyStore.keys.first { $0.id == id } }
        switch message["ev"] as? String {
        case "done":
            dismiss()
        case "add":
            // The bar's + and Generate key open on Generate; Import key on
            // Import.
            addMethod = message["method"] as? String == "import" ? .importKey : .generate
            showAdd = true
        case "copy-public-key":
            if let key { SSHKeysModel.copy(key.publicKey) }
        case "copy-fingerprint":
            if let key { SSHKeysModel.copy(key.fingerprint) }
        case "share-public-key":
            if let key { model.share(key) }
        case "confirm-delete":
            if let key {
                model.requestDelete(key, SSHKeyDeletion.check(key, connections: connections.store, keys: keyStore))
            }
        case "cancel-delete":
            model.cancelDelete()
        case "delete":
            if let key {
                let keychain = keychainHolder.service
                model.delete(key, from: keyStore, connections: connections.store) {
                    try await keychain.delete(forRef: $0)
                }
            }
        case "dismiss-notice":
            model.dismissNotice()
        default:
            break
        }
    }
}

/// What the NSH SSH Keys page shows: the store's keys, as the SwiftUI page
/// splits them.
struct SSHKeysSnapshot: Equatable {
    var device: [SSHKeyRecord]
    var imported: [SSHKeyRecord]

    init(store: SSHKeyStore) {
        device = store.deviceKeys
        imported = store.importedKeys
    }

    init(device: [SSHKeyRecord], imported: [SSHKeyRecord]) {
        self.device = device
        self.imported = imported
    }

    func contains(_ id: UUID) -> Bool {
        device.contains { $0.id == id } || imported.contains { $0.id == id }
    }
}

/// What the NSH SSH Keys page shows, owned by the host so a surface made
/// again starts from it (``NSHPage``'s third rule): the keys, the one the
/// delete alert asks about, and why a key wasn't deleted; the message that
/// carries them (`{"op": "keys", …}`, `ui/src/ssh_keys.rs`'s `Message`):
/// metadata only, never a key; and what the page's intents do.
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
            snapshot = SSHKeysSnapshot(store: store)
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
        surface?.post(Self.message(snapshot, deleting: deleting, notice: notice))
    }

    /// The screen's message.
    nonisolated static func message(_ snapshot: SSHKeysSnapshot, deleting: UUID?,
                                    notice: SSHKeyDeletion.Failure? = nil,
                                    now: Date = Date()) -> [String: Any] {
        var message: [String: Any] = [
            "op": "keys",
            "device": snapshot.device.map { key($0, now: now) },
            "imported": snapshot.imported.map { key($0, now: now) },
        ]
        if let deleting { message["deleting"] = deleting.uuidString }
        if let notice { message["notice"] = ["title": notice.title, "message": notice.message] }
        return message
    }

    /// How long a key goes unused before its row says so (`SSHKeyRow`).
    nonisolated static let staleAfter: TimeInterval = 90 * 86400

    /// One key's metadata, as the SwiftUI row reads it: the avatar its source
    /// (and an RSA key's age) picks, its badge, fingerprint and hosts, and,
    /// for a stale key, the month and year it was last used.
    nonisolated static func key(_ record: SSHKeyRecord, now: Date) -> [String: Any] {
        let stale = record.lastUsedAt.map { now.timeIntervalSince($0) > staleAfter } ?? false
        let avatar = switch record.source {
        case .secureEnclave: "secure-enclave"
        case .hardware: "hardware"
        case .generated, .imported: record.algorithm == .rsa4096 && stale ? "gray" : "imported"
        }
        var key: [String: Any] = [
            "id": record.id.uuidString,
            "name": record.name,
            "badge": record.badgeText,
            "avatar": avatar,
            "stale": stale,
            "fingerprint": record.fingerprint,
            "hosts": record.boundHosts,
            "hasPublicKey": !record.publicKey.isEmpty,
        ]
        if stale, let last = record.lastUsedAt {
            key["unusedSince"] = last.formatted(.dateTime.month(.abbreviated).year())
        }
        return key
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
