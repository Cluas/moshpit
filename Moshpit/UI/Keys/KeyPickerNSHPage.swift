import SwiftUI

/// Add Connection › Select Key drawn by NSH (`ui/src/key_picker.rs`) in the
/// sheet's own stack: the saved keys, the one picked and why a key can't be
/// used come from the host (``KeyPickerModel``); a pick sets the connection's
/// key and closes the sheet, + (or the empty group's Generate) opens Add Key,
/// whose new key is picked. If its screen can't run, the page says so
/// (``NSHPageFailed``).
struct KeyPickerNSHPage: View {
    let keys: [SSHKeyRecord]
    @Binding var selectedKeyId: UUID?

    @Environment(\.dismiss) private var dismiss
    @State private var model = KeyPickerModel()
    @State private var failed = false
    @State private var showAddKey = false

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            let snapshot = KeyPickerSnapshot(keys: keys, selected: selectedKeyId)
            NavigationStack {
                NSHPage(
                    screen: .keyPicker,
                    attach: { model.attach($0, current: snapshot) },
                    onMessage: handle,
                    onFailure: { failed = true })
                    .nshPage()
                    .navigationTitle("Select Key")
                    .onAppear { model.update(snapshot) }
                    .onChange(of: snapshot) { _, new in model.update(new) }
            }
            .appSheet(isPresented: $showAddKey) {
                AddKeyNSHPage(onCreated: { record in
                    selectedKeyId = record.id
                    showAddKey = false
                    dismiss()
                })
            }
        }
    }

    /// What the screen tells the host.
    private func handle(_ message: [String: Any]) {
        switch message["ev"] as? String {
        case "select":
            guard let pick = KeyPickerModel.pick(message, among: keys, selected: selectedKeyId) else { return }
            Haptics.select()
            selectedKeyId = pick.id
            dismiss()
        case "add":
            showAddKey = true
        case "cancel":
            dismiss()
        default:
            break
        }
    }
}

/// What the NSH Select Key page shows.
struct KeyPickerSnapshot: Equatable {
    var keys: [SSHKeyRecord]
    var selected: UUID?
}

/// What the NSH Select Key page shows, owned by the host so a surface made
/// again starts from it (``NSHPage``'s third rule), and the message that
/// carries it (`{"op": "keys", …}`, `ui/src/key_picker.rs`'s `Message`):
/// each key's name and badge, and the key type the app can't sign in with
/// when it is that key's (``StoredKey/refusedType(ofPublicKeyLine:)``).
@MainActor
final class KeyPickerModel {
    private(set) var snapshot: KeyPickerSnapshot?
    private weak var surface: NSHSurface?

    func attach(_ surface: NSHSurface) {
        self.surface = surface
        if let snapshot { surface.post(Self.message(snapshot)) }
    }

    /// ``attach(_:)``, the model filled from `current` (the page's keys as
    /// its body has it now) while it has none (B366): the screen starts from
    /// the page's state whether or not `onAppear` came first. A state the
    /// model has is newer (`onChange` keeps it) and stays.
    func attach(_ surface: NSHSurface, current snapshot: KeyPickerSnapshot) {
        if self.snapshot == nil {
            self.surface = nil
            update(snapshot)
        }
        attach(surface)
    }

    func update(_ snapshot: KeyPickerSnapshot) {
        guard snapshot != self.snapshot else { return }
        self.snapshot = snapshot
        surface?.post(Self.message(snapshot))
    }

    /// The screen's message.
    nonisolated static func message(_ snapshot: KeyPickerSnapshot) -> [String: Any] {
        [
            "op": "keys",
            "keys": snapshot.keys.map { key -> [String: Any] in
                var entry: [String: Any] = ["id": key.id.uuidString, "name": key.name, "badge": key.badgeText]
                if let refused = StoredKey.refusedType(ofPublicKeyLine: key.publicKey) { entry["refused"] = refused }
                return entry
            },
            "selected": snapshot.selected.map { $0.uuidString as Any } ?? NSNull(),
        ]
    }

    /// What a pick on the page sets the connection's key to.
    enum Pick: Equatable {
        /// A saved key.
        case key(UUID)
        /// None: a PEM pasted by hand instead.
        case paste

        var id: UUID? {
            if case .key(let id) = self { return id }
            return nil
        }
    }

    /// The pick `message` asks for (`{"ev": "select", "id": id | null}`),
    /// as the SwiftUI page allows it: a saved key the app can sign in with,
    /// the one already picked, or none; nil for anything else.
    nonisolated static func pick(_ message: [String: Any], among keys: [SSHKeyRecord],
                                 selected: UUID?) -> Pick? {
        if message["id"] == nil || message["id"] is NSNull { return .paste }
        guard let id = (message["id"] as? String).flatMap(UUID.init(uuidString:)),
              let key = keys.first(where: { $0.id == id })
        else { return nil }
        let refused = StoredKey.refusedType(ofPublicKeyLine: key.publicKey) != nil
        guard !refused || key.id == selected else { return nil }
        return .key(id)
    }

    /// The accessibility identifier of key `id`'s row on both pages.
    nonisolated static func rowID(_ id: UUID) -> String { "key-picker-key-\(id.uuidString)" }
}
