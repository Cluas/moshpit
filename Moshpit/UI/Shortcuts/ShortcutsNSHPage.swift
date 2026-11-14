import MoshpitKit
import SwiftUI

/// Settings › Shortcuts: the SwiftUI page (``ShortcutsView``), or the NSH one
/// while its switch is on (``NSHScreen/shortcuts``). With the switch off the
/// SwiftUI page is all there is: no NSH code runs.
struct ShortcutsPage: View {
    var body: some View {
        content
        #if DEBUG
            .modifier(ShortcutsCapture.Present())
        #endif
    }

    @ViewBuilder
    private var content: some View {
        if Self.showsNSH {
            ShortcutsNSHPage()
        } else {
            ShortcutsView()
        }
    }

    static var showsNSH: Bool {
        #if DEBUG
        if CaptureScreen.requested == .shortcutsNSH { return NSHSurface.available }
        #endif
        return NSHScreen.shortcuts.isOn
    }
}

/// Settings › Shortcuts drawn by NSH (`ui/src/shortcuts.rs`) in the sheet's
/// own stack. The bar, the user's own shortcuts and the rest come from the
/// store (``ShortcutsModel``, again whenever it changes); every change the
/// page asks for goes through the store as the SwiftUI page's does (its
/// moves, `removeFromToolbar`, `remove`, `addToToolbar`), which saves in the
/// format every build since 1.0.8 reads. + and a custom shortcut open the
/// editor its own switch picks. If NSH cannot run here, the SwiftUI page
/// shows.
struct ShortcutsNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(ShortcutStore.self) private var store
    @State private var model = ShortcutsModel()
    @State private var failed = false
    @State private var showAdd = false
    @State private var editing: TerminalShortcut?

    var body: some View {
        if failed {
            ShortcutsView()
        } else {
            let snapshot = ShortcutsSnapshot(store: store)
            NavigationStack {
                NSHPage(
                    screen: .shortcuts,
                    attach: { model.attach($0, current: snapshot) },
                    onMessage: handle,
                    onFailure: { failed = true })
                    .nshPage()
                    .navigationTitle("Shortcuts")
                    .onAppear { model.update(snapshot) }
                    .onChange(of: snapshot) { _, new in model.update(new) }
            }
            .appSheet(isPresented: $showAdd) { AddShortcutPage() }
            .appSheet(item: $editing) { shortcut in AddShortcutPage(existing: shortcut) }
        }
    }

    /// What the screen tells the host: what the shared model
    /// (`crates/app/src/shortcuts.rs`) makes of it against the store.
    private func handle(_ message: [String: Any]) {
        guard let op = ShortcutsModel.op(message, in: store) else {
            Log.ui.error("shortcuts: no event \(String(describing: message["ev"]), privacy: .public)")
            return
        }
        switch op {
        case .done:
            dismiss()
        case .add:
            showAdd = true
        case .edit(let id):
            editing = store.shortcuts.first { $0.id == UUID(uuidString: id) }
        default:
            ShortcutsModel.run(op, on: store)
        }
    }
}

/// What the NSH Shortcuts page shows: the store's shortcuts, in order.
struct ShortcutsSnapshot: Equatable {
    var shortcuts: [TerminalShortcut]

    init(store: ShortcutStore) {
        shortcuts = store.shortcuts
    }

    init(shortcuts: [TerminalShortcut]) {
        self.shortcuts = shortcuts
    }
}

/// What the NSH Shortcuts page shows, owned by the host so a surface made
/// again starts from it (``NSHPage``'s third rule), the message that carries
/// it (`{"op": "shortcuts", …}`, `ui/src/shortcuts.rs`'s `Message`), and
/// what the page's changes do to the store.
@MainActor
final class ShortcutsModel {
    private(set) var snapshot: ShortcutsSnapshot?
    private weak var surface: NSHSurface?

    func attach(_ surface: NSHSurface) {
        self.surface = surface
        if let snapshot { surface.post(Self.message(snapshot)) }
    }

    /// ``attach(_:)``, the model filled from `current` (the page's shortcuts as
    /// its body has it now) while it has none (B366): the screen starts from
    /// the page's state whether or not `onAppear` came first. A state the
    /// model has is newer (`onChange` keeps it) and stays.
    func attach(_ surface: NSHSurface, current snapshot: ShortcutsSnapshot) {
        if self.snapshot == nil {
            self.surface = nil
            update(snapshot)
        }
        attach(surface)
    }

    func update(_ snapshot: ShortcutsSnapshot) {
        guard snapshot != self.snapshot else { return }
        self.snapshot = snapshot
        surface?.post(Self.message(snapshot))
    }

    /// The screen's message, as the shared model makes it: the bar, the
    /// user's own, and every shortcut off the bar as shown (B233), each
    /// built-in called by its name in the app's language (B234, this
    /// host's words).
    nonisolated static func message(_ snapshot: ShortcutsSnapshot) -> [String: Any] {
        let names = Dictionary(snapshot.shortcuts.map { ($0.id.uuidString, $0.displaySummary) },
                               uniquingKeysWith: { first, _ in first })
        let json = shortcutsPageMessage(shortcuts: snapshot.shortcuts.map(ShortcutRecord.init), names: names)
        return (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
    }

    /// What `message` (`{"ev": …}`) asks, checked against the store by the
    /// shared model: nil for an event it does not know, a shortcut the store
    /// does not have, a built-in's edit or delete, or a move off the bar.
    static func op(_ message: [String: Any], in store: ShortcutStore) -> ShortcutListOp? {
        guard JSONSerialization.isValidJSONObject(message),
              let data = try? JSONSerialization.data(withJSONObject: message) else { return nil }
        return shortcutsListOp(messageJson: String(decoding: data, as: UTF8.self),
                               shortcuts: store.shortcuts.map(ShortcutRecord.init))
    }

    /// Does `op` to `store` (the page's own ops: the host opens and closes).
    static func run(_ op: ShortcutListOp, on store: ShortcutStore) {
        switch op {
        case .move(let from, let to):
            store.moveInToolbar(fromOffsets: IndexSet(integer: Int(from)), toOffset: Int(to))
        case .removeFromToolbar(let id):
            if let id = UUID(uuidString: id) { store.removeFromToolbar(id: id) }
        case .addToToolbar(let id):
            if let id = UUID(uuidString: id) { store.addToToolbar(id: id) }
        case .delete(let id):
            if let id = UUID(uuidString: id) { store.remove(id: id) }
        case .done, .add, .edit:
            break
        }
    }

    /// Applies a change the page asked for to `store`; false for an event
    /// it does not know or a shortcut it does not have.
    @discardableResult
    static func apply(_ message: [String: Any], to store: ShortcutStore) -> Bool {
        guard let op = op(message, in: store) else { return false }
        switch op {
        case .done, .add, .edit: return false
        default:
            run(op, on: store)
            return true
        }
    }
}

/// Shortcuts › Add Shortcut, or a custom shortcut's editor: the SwiftUI page
/// (``AddShortcutView``), or the NSH one while its switch is on
/// (``NSHScreen/addShortcut``).
struct AddShortcutPage: View {
    var existing: TerminalShortcut?

    var body: some View {
        if Self.showsNSH {
            AddShortcutNSHPage(existing: existing)
        } else {
            AddShortcutView(existing: existing)
        }
    }

    static var showsNSH: Bool {
        #if DEBUG
        if [.addShortcutNSH, .editShortcutNSH].contains(CaptureScreen.requested) { return NSHSurface.available }
        #endif
        return NSHScreen.addShortcut.isOn
    }
}

/// The shortcut editor drawn by NSH (`ui/src/add_shortcut.rs`) in the
/// sheet's own stack. The draft is the host's (``AddShortcutModel``): the
/// page tells it each change and Save; saving is the SwiftUI page's
/// (`store.add` or `store.update`). If NSH cannot run here, the SwiftUI page
/// shows.
struct AddShortcutNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(ShortcutStore.self) private var store
    @Environment(ConnectionStoreHolder.self) private var connections
    let existing: TerminalShortcut?
    @State private var model: AddShortcutModel
    @State private var failed = false
    /// Saved while the toolbar was full: said before the page goes (B252).
    @State private var savedOffToolbar = false

    init(existing: TerminalShortcut? = nil) {
        self.existing = existing
        _model = State(initialValue: AddShortcutModel(existing: existing))
    }

    var body: some View {
        if failed {
            AddShortcutView(existing: existing)
        } else {
            NavigationStack {
                NSHPage(
                    screen: .addShortcut,
                    attach: { surface in
                        model.attach(surface, hosts: connections.store.connections.map(\.displayName),
                                     toolbarCount: store.toolbarCount)
                    },
                    onMessage: handle,
                    onFailure: { failed = true })
                    .nshPage()
                    .navigationTitle(existing == nil ? "Add Shortcut" : "Edit Shortcut")
                    .savedOffToolbarAlert(isPresented: $savedOffToolbar) { dismiss() }
            }
        }
    }

    /// What the screen tells the host.
    private func handle(_ message: [String: Any]) {
        switch message["ev"] as? String {
        case "change":
            if let draft = message["draft"] as? [String: Any] { model.apply(draft) }
        case "save":
            switch model.save(to: store) {
            case .saved: dismiss()
            case .savedOffToolbar: savedOffToolbar = true
            case .notSaved: break
            }
        case "cancel":
            dismiss()
        default:
            break
        }
    }
}

/// The NSH shortcut editor's host: the shared model's editor
/// (`crates/app/src/shortcuts.rs`'s `ShortcutEditor`, through
/// ``ShortcutEditorHandle``), owned by the host so a surface made again
/// starts from it (``NSHPage``'s third rule): the draft, the message that
/// carries it (`{"op": "shortcut", …}`), and Save — on the store's current
/// copy, a second Save of a new shortcut updating the first (B288).
@MainActor
final class AddShortcutModel {
    let existing: TerminalShortcut?
    private let editor: ShortcutEditorHandle
    private var hosts: [String] = []
    private var toolbarCount = 0
    private weak var surface: NSHSurface?

    /// A new shortcut's defaults, or `existing`'s fields.
    init(existing: TerminalShortcut?) {
        self.existing = existing
        editor = ShortcutEditorHandle(existing: existing.map(ShortcutRecord.init))
    }

    /// `surface` edits the draft from now on, starting with it.
    func attach(_ surface: NSHSurface, hosts: [String], toolbarCount: Int) {
        self.surface = surface
        self.hosts = hosts
        self.toolbarCount = toolbarCount
        surface.post(message)
    }

    /// Whether Save may save.
    var canSave: Bool { editor.canSave() }

    /// The draft as the page has it (`{"kind", "modifiers", …}`).
    var draft: [String: Any] {
        (try? JSONSerialization.jsonObject(with: Data(editor.draftJson().utf8))) as? [String: Any] ?? [:]
    }

    /// The page's whole draft, as it told it: the model takes what it reads.
    func apply(_ draft: [String: Any]) {
        guard JSONSerialization.isValidJSONObject(draft),
              let data = try? JSONSerialization.data(withJSONObject: draft) else { return }
        editor.apply(draftJson: String(decoding: data, as: UTF8.self))
    }

    /// What Save did.
    enum Saved: Equatable {
        case saved
        /// A new shortcut, saved while the toolbar was full: under AVAILABLE,
        /// which the page says before it goes (B252).
        case savedOffToolbar
        /// It can't be saved yet.
        case notSaved
    }

    /// Saves what the model says to `store`.
    @discardableResult
    func save(to store: ShortcutStore) -> Saved {
        switch editor.save(shortcuts: store.shortcuts.map(ShortcutRecord.init)) {
        case .add(let record)?:
            guard let shortcut = TerminalShortcut(record) else { return .notSaved }
            return store.add(shortcut) ? .saved : .savedOffToolbar
        case .update(let record)?:
            guard let shortcut = TerminalShortcut(record) else { return .notSaved }
            store.update(shortcut)
            return .saved
        case nil:
            return .notSaved
        }
    }

    /// The screen's message.
    var message: [String: Any] {
        let json = editor.message(hosts: hosts, toolbarCount: UInt32(max(0, toolbarCount)))
        return (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
    }
}

#if DEBUG
/// A capture run of the shortcuts pages (DEBUG only): Add Shortcut, or the
/// editor of a custom shortcut held in memory (Save is never pressed), over
/// Shortcuts, so the SwiftUI and NSH editors can be shot in the same state.
enum ShortcutsCapture {
    /// A custom text shortcut, as a user makes one.
    static let custom: TerminalShortcut = {
        var sc = TerminalShortcut()
        sc.id = UUID(uuidString: "5C000000-0000-4000-8000-0000000000A1")!
        sc.kind = .text
        sc.payload = "git status\\r"
        sc.chipLabel = "gs"
        sc.summary = "git status"
        sc.colorId = "mosh"
        return sc
    }()

    struct Present: ViewModifier {
        @State private var showAdd = false
        @State private var editing: TerminalShortcut?

        func body(content: Content) -> some View {
            content
                .task {
                    guard let screen = CaptureScreen.requested else { return }
                    try? await Task.sleep(for: .milliseconds(800))
                    if [.addShortcut, .addShortcutNSH].contains(screen) { showAdd = true }
                    if [.editShortcut, .editShortcutNSH].contains(screen) { editing = ShortcutsCapture.custom }
                }
                .appSheet(isPresented: $showAdd) { AddShortcutPage() }
                .appSheet(item: $editing) { shortcut in AddShortcutPage(existing: shortcut) }
        }
    }
}
#endif
