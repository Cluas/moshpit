import Foundation

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

    /// Whether a swipe has switched panes on this control plane yet. Until one
    /// has, a pane appearing in the host is not the result of a gesture — it
    /// is the attach, or a reconnect's replacement taking over from the frozen
    /// frame — and has no edge to slide in from; it fades instead.
    var hasSwipedSinceAttach = false

    /// Panes in the currently active window, sorted by pane id — in the
    /// order they were made (``idPrecedes(_:_:)``).
    var activePanes: [PaneInfo] {
        guard let windowId = activeWindowId else { return [] }
        return panes.values.filter { $0.windowId == windowId }.sorted { Self.idPrecedes($0.id, $1.id) }
    }

    /// The sessions, in the order they were made (``idPrecedes(_:_:)``).
    var sortedSessions: [SessionInfo] {
        sessions.values.sorted { Self.idPrecedes($0.id, $1.id) }
    }

    /// Whether id `a` comes before id `b` by the numbers in them — the order
    /// tmux mints its ids in (`$2` before `$10`, `%9` before `%11`), and
    /// herdr's (`w1:p2` before `w1:p10`). A plain string comparison put `$10`
    /// ahead of `$2`, so the fallback session after a kill or for a fresh
    /// listing, the fallback pane input went to, and the order the sessions
    /// were listed in all went wrong once there were ten of anything.
    static func idPrecedes(_ a: String, _ b: String) -> Bool {
        a.compare(b, options: .numeric) == .orderedAscending
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
