import SwiftUI
import UIKit

/// Home › a host's sessions, drawn by NSH (`ui/src/sessions`, UI-DESIGN
/// §6.2): the multiplexer's whole tree, which Home's cards no longer show.
///
/// Home's things serve it. The host's card is Home's record of it
/// (``HomeRecord``, built by the core as Home's is), posted as
/// `{"op": "sessions", "connection": …}`; the tree's disclosure lives in
/// Home's model (``HomeNSHModel``), which both pages tell their changes to
/// and which gives a page made again what was last told. What the rows ask
/// is Home's own intents (``HomeIntent``), run as Home runs them
/// (``HomeTreeActions``), and the rows carry Home's identifiers, so Home's
/// long-press menus (``HomeNSHMenus``) are theirs. If NSH can't run here,
/// the page says so (``NSHPageFailed``).
struct SessionsNSHPage: View {
    let connectionId: UUID
    let store: ConnectionStore
    let hub: SessionHub
    let model: HomeNSHModel
    let actions: HomeNSHActions
    /// What its back does where there is nothing to pop: the iPad's detail
    /// column, which Home empties (B554: `dismiss()` did nothing there, and
    /// the page's own bar hides the system's). nil: `dismiss()`.
    var onLeave: (() -> Void)?

    @Environment(\.dismiss) private var dismiss
    @Environment(AgentActivityMonitor.self) private var monitor
    @State private var page = SessionsNSHModel()
    @State private var killTarget: HomeKillTarget?
    @State private var worktreeTarget: ConnectionCard.WorktreeTarget?
    @State private var worktreeForceTarget: ConnectionCard.WorktreeTarget?
    @State private var agentTask: HomeAgentTaskTarget?
    @State private var menus = NSHContextMenus()
    @State private var failed = false

    var body: some View {
        if failed {
            // Under the system's bar: its back button (or, on the iPad, the
            // sidebar's) leaves.
            NSHPageFailed { failed = false }
        } else {
            content(record: record)
        }
    }

    /// This host's card as Home's record has it.
    private var record: HomeRecord {
        // This host only: the record is a pure reading of the registry, so
        // a record of one card is that card as Home's has it.
        HomeRecord.now(
            core: .shared, connections: store.connections.filter { $0.id == connectionId },
            session: { hub.session(for: $0)?.rust?.id },
            agents: monitor.machine, disconnecting: [], sidebar: false)
    }

    /// The page, drawn by NSH.
    @ViewBuilder
    private func content(record: HomeRecord) -> some View {
        let rows = tree.rows(of: record)
        NSHPage(
            screen: .sessions,
            prepare: { HomeNSHComponents.register(on: $0) },
            attach: { surface in
                page.attach(surface, trees: model.treesMessage, current: record, connection: connectionId)
                menus.install(on: surface)
            },
            onMessage: { message in
                if message["ev"] as? String == "back" {
                    if let onLeave { onLeave() } else { dismiss() }
                    return
                }
                guard let intent = model.handle(message) else { return }
                // A tray's action: its confirmation hangs from its row (B499).
                menus.anchor(to: message)
                perform(intent)
            },
            onFailure: { failed = true })
            .nshPage()
            // The title the terminal's back button shows.
            .navigationTitle(store.connections.first { $0.id == connectionId }?.displayName ?? "")
            .onAppear {
                page.update(record, connection: connectionId)
                show(rows)
            }
            .onChange(of: record) { _, new in page.update(new, connection: connectionId) }
            .onChange(of: rows) { _, new in show(new) }
            .modifier(HomeTreeDialogs(
                killTarget: $killTarget, worktreeTarget: $worktreeTarget,
                worktreeForceTarget: $worktreeForceTarget, agentTask: $agentTask,
                menus: menus, actions: actions))
    }

    /// The host's rows' menus to the surface.
    private func show(_ rows: [HomeNSHMenus.Row]) {
        menus.menus = HomeNSHMenus.menus(for: rows, perform: perform)
        menus.cornerRadius = HomeNSHMenus.corners(for: rows)
    }

    private var tree: HomeTreeActions {
        HomeTreeActions(store: store, hub: hub, actions: actions,
                        confirmKill: { killTarget = $0 },
                        removeWorktree: { worktreeTarget = $0 },
                        startAgentTask: { agentTask = $0 })
    }

    private func perform(_ intent: HomeIntent) {
        if tree.perform(intent) { return }
        switch intent {
        case .connect(let id): actions.connect(id)
        case .enter(let id): actions.enter(id)
        default: break
        }
    }
}

/// What the sessions page shows, kept by the page so a surface made again
/// starts from it (``NSHPage``'s third rule): the host's card as last
/// posted.
@MainActor
final class SessionsNSHModel {
    private var message: String?
    private weak var surface: NSHSurface?

    /// `surface` shows the page from now on: the host, then the disclosure
    /// Home's model keeps (the page reads it for the host it shows).
    func attach(_ surface: NSHSurface, trees: [String: Any]) {
        self.surface = surface
        if let message { surface.post(json: message) }
        surface.post(trees)
    }

    /// ``attach(_:trees:)``, the model filled from `record` (the host as the
    /// page's body has it now) while it has none (B366): the page starts
    /// from it whether or not `onAppear` came first; a host the model has
    /// is newer and stays.
    func attach(_ surface: NSHSurface, trees: [String: Any], current record: HomeRecord, connection id: UUID) {
        if message == nil {
            self.surface = nil
            update(record, connection: id)
        }
        attach(surface, trees: trees)
    }

    /// The host as `record` has it now; nothing is posted while it is
    /// unchanged, or when the record has no card for it.
    func update(_ record: HomeRecord, connection id: UUID) {
        guard let message = Self.message(record, connection: id), message != self.message else { return }
        self.message = message
        surface?.post(json: message)
    }

    /// `{"op": "sessions", "connection": …}` (`ui/src/sessions`'s
    /// `Message`): connection `id`'s card as Home's record carries it
    /// (`ui/src/home/model.rs`'s `Connection`, which the page reads too).
    static func message(_ record: HomeRecord, connection id: UUID) -> String? {
        guard let home = try? JSONSerialization.jsonObject(with: Data(record.message.utf8)) as? [String: Any],
              let cards = home["connections"] as? [[String: Any]],
              let card = cards.first(where: { ($0["id"] as? String).flatMap(UUID.init(uuidString:)) == id }),
              let data = try? JSONSerialization.data(
                  withJSONObject: ["op": "sessions", "connection": card] as [String: Any], options: [.sortedKeys])
        else { return nil }
        return String(decoding: data, as: UTF8.self)
    }
}
