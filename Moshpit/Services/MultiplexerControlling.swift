import Foundation

/// Immutable-style snapshot of tmux state. Mutations replace the value in one
/// assignment so SwiftUI sees a single coherent change. Shared by the `-CC`
/// control client (over SSH) and the SSH-exec sidecar (over mosh).
struct TmuxSnapshot: Equatable {
    var sessions: [String: SessionInfo] = [:]
    var windows: [String: WindowInfo] = [:]
    var panes: [String: PaneInfo] = [:]
    var activeSessionId: String?
    var activeWindowId: String?
    var activePaneId: String?
    var isAttached: Bool = false
    /// Latches true once we've attached at least once this connection. Lets the
    /// UI distinguish a first-time attach (show a spinner) from an attached
    /// session that later ended — e.g. the last session was killed and the
    /// server exited — where "Attaching…" would otherwise hang forever.
    var everAttached: Bool = false

    /// Direction of the most recent pane/window switch — `true` = forward
    /// (swipe left / next), `false` = backward (swipe right / previous).
    /// Set by `switchPaneOrWindow(forward:)`; read by `TmuxPaneSplitView` to
    /// pick which edge the incoming pane slides in from. Switches triggered
    /// by a tap (sessions tree, Windows/Sessions/Pane sheets) don't have a
    /// direction of their own, so they just reuse whatever this last was —
    /// an arbitrary but harmless default, not worth threading a "was this a
    /// gesture" flag through every selection call site for.
    var lastSwitchForward: Bool = true

    /// Panes in the currently active window, sorted by pane id.
    var activePanes: [PaneInfo] {
        guard let windowId = activeWindowId else { return [] }
        return panes.values.filter { $0.windowId == windowId }.sorted { $0.id < $1.id }
    }

    /// The ACTIVE session's windows, sorted by tmux index. Discovery loads
    /// every session's windows (`list-windows -a`), so this scopes to the
    /// attached session for the breadcrumb / window-switcher / zoom logic that
    /// assumes "the current session's windows".
    var sortedWindows: [WindowInfo] {
        guard let sid = activeSessionId else {
            return windows.values.sorted { $0.index < $1.index }
        }
        return windows(inSession: sid)
    }

    /// A specific session's windows, sorted by tmux index — drives the tree so
    /// multiple sessions can be expanded at once.
    func windows(inSession sessionId: String) -> [WindowInfo] {
        windows.values.filter { $0.sessionId == sessionId }.sorted { $0.index < $1.index }
    }

    /// Panes in `windowId`, sorted by pane index — drives the tree's pane level.
    func panes(inWindow windowId: String) -> [PaneInfo] {
        panes.values.filter { $0.windowId == windowId }.sorted { $0.index < $1.index }
    }
}

/// A captured session/window/pane selection, replayed after a reconnect so the
/// user lands back where they were. tmux session/window/pane ids are stable
/// across a client re-attach (the server persists them), so the ids survive.
struct TmuxSelection: Equatable, Codable {
    let session: String
    let window: String?
    let pane: String?
}

/// Persists the last selection PER CONNECTION so a reconnect can land back on
/// the user's session/window/pane even when the ActiveSession that captured it
/// is gone — a protocol switch (SSH→mosh) tears the session down, and an app
/// relaunch loses all in-memory state. tmux ids are only meaningful against
/// the same tmux server, which a connection maps to 1:1.
enum TmuxSelectionStore {
    static func key(_ id: UUID) -> String { "moshpit.tmux.lastSelection.\(id.uuidString)" }

    static func load(_ id: UUID, defaults: UserDefaults = .standard) -> TmuxSelection? {
        guard let data = defaults.data(forKey: key(id)) else { return nil }
        return try? JSONDecoder().decode(TmuxSelection.self, from: data)
    }

    static func save(_ selection: TmuxSelection?, for id: UUID, defaults: UserDefaults = .standard) {
        if let selection, let data = try? JSONEncoder().encode(selection) {
            defaults.set(data, forKey: key(id))
        } else {
            defaults.removeObject(forKey: key(id))
        }
    }
}

/// The multiplexer control surface the UI drives, whichever multiplexer is
/// behind it — `TmuxSessionController` (tmux over SSH `-CC`, live push) and
/// `HerdrControlClient` (herdr over an SSH side-channel that polls
/// `herdr api snapshot`) both implement it.
///
/// The vocabulary stays Moshpit's own — session / window / pane. herdr's
/// workspace / tab / pane maps onto it one-for-one (see `HerdrSnapshot`), so
/// every sheet and the home tree work against either without knowing which.
///
/// `AnyObject` + `Observable` so generic SwiftUI views over a concrete
/// conformer keep Observation tracking on `snapshot`. Views take a generic
/// `C: MultiplexerControlling` rather than an existential for exactly that
/// reason — call sites branch on which controller exists.
@MainActor
protocol MultiplexerControlling: AnyObject, Observable {
    /// Which multiplexer is behind this surface. The sheets are generic over
    /// the controller, so this is how they know whose words and keys to print
    /// (see ``MultiplexerVocabulary``).
    var multiplexer: Multiplexer { get }

    var snapshot: TmuxSnapshot { get }

    /// Per-pane agent stamps — lets pickers and the Vibe Island show which
    /// window's agent is working / needs attention.
    ///
    /// Where they come from is the sharpest difference between the two
    /// multiplexers: tmux needs hooks installed on the host writing
    /// `@moshpit_*` options, while herdr reports `agent_status` natively and
    /// needs nothing installed at all.
    var agentHooks: [String: AgentHook] { get }

    /// Invoked on the main actor once ``agentHooks`` has been rebuilt, so the
    /// Vibe Island can re-sync immediately instead of waiting for its next
    /// sweep.
    var onAgentHooksUpdated: (() -> Void)? { get set }

    /// Re-read agent state now. tmux issues a dedicated `list-panes` for the
    /// `@moshpit_*` options; herdr's snapshot poll already carries it, so
    /// this just brings the next read forward.
    func pollAgentHooks()
    /// True while a user-initiated refresh is in flight (drives the spinner).
    var isRefreshing: Bool { get }
    func selectWindow(_ windowId: String)
    func selectSession(_ sessionId: String)
    func selectPane(_ paneId: String)
    func newWindow(named name: String?)
    func newPane()
    func newSession(named name: String?)
    /// Rename a session (`rename-session`) then refresh discovery.
    func renameSession(_ sessionId: String, to name: String)
    /// Kill a session (`kill-session`) then refresh discovery.
    func killSession(_ sessionId: String)
    /// Rename a window (`rename-window`) then refresh discovery.
    func renameWindow(_ windowId: String, to name: String)
    /// Kill a window (`kill-window`) then refresh discovery.
    func killWindow(_ windowId: String)
    /// Kill a single pane (`kill-pane`) then refresh discovery. Panes have no
    /// name, so there is no rename counterpart.
    func killPane(_ paneId: String)
    /// Re-read tmux state (sidecar: SSH-exec query; -CC: resend discovery).
    func refresh()

    /// The repository a session is a git worktree of, or nil when it isn't
    /// one. Only herdr can answer yes — tmux has no notion of a session being
    /// a checkout, so the default below covers it.
    func worktreeRepo(for sessionId: String) -> String?
}

extension MultiplexerControlling {
    func worktreeRepo(for sessionId: String) -> String? { nil }
}
