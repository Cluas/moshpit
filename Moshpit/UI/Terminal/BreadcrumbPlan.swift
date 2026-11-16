import Foundation

/// What the terminal top bar's breadcrumb shows, decided away from the view.
///
/// The interesting case is a pane with an agent in it. "Which agent is this,
/// and is it waiting on me" is exactly what the person glancing at the top bar
/// wants to know — but the third crumb only knew the pane's foreground
/// command, which under herdr 0.7.3 is nothing at all (`pane N`).
///
/// Fitting the agent in is a width problem before it is anything else: three
/// full segments don't always fit 402pt, which was discovered by drawing it
/// (see the flow prototype's stage-⑤ sketch). The answer here: when the pane
/// carries an agent, the session crumb is ALLOWED to give up its text and
/// keep only its icon — still tappable, still discoverable, but no longer
/// spending ~90pt saying a workspace name that the Sessions sheet repeats
/// anyway. Whether it actually has to is the view's call, measured against
/// the bar: this used to squeeze unconditionally, and a session called "0"
/// lost its one character to make room that was already there (真机
/// 2026-09-18).
///
/// Pure — the snapshot and hooks go in, the strings come out — so the
/// squeeze rule and the fallback chain are testable without a connection.
///
/// `isComplete` is the bar's gate. An attach marks the tree live BEFORE
/// `list-windows` / `list-panes` and the hook poll have answered, so a plan
/// built at that instant says "0 › —", then the pane's command without its
/// agent, then — one more reply later — the agent with the session squeezed
/// to its icon. Three different bars in under a second, on every reconnect
/// (真机 2026-09-16). The view keeps the previous plan until this one is whole.
struct BreadcrumbPlan: Equatable {
    var sessionTitle: String
    /// True when the pane segment carries an agent and may need the room: the
    /// view tries the full session name first and falls back to the icon
    /// alone only if three full segments overflow the bar. False = the name
    /// is always spelled out and the window crumb truncates instead.
    var sessionSqueezable: Bool
    var windowTitle: String
    /// nil = no pane crumb at all (no active pane in the snapshot).
    var paneTitle: String?
    /// The dot inside the pane crumb — the same signal the Agents section and
    /// the island show, in the same colours. nil = plain pane, no dot.
    var paneSignal: AgentSignal?
    /// False while the tree is still filling in after an attach (no active
    /// window or pane yet) or the agent stamps have not been read once.
    var isComplete = true
    /// What the segments are named from, on the NSH screen
    /// (`ui/src/terminal/bar.rs`): every session, the active window and
    /// pane and the pane's stamp, so the screen names them by Home's rules
    /// (the project, then the agent: UI-DESIGN §6.1, §6.3). Kept with the
    /// plan, so a remembered plan keeps its names.
    var tree: Tree?

    static func make(snapshot: TmuxSnapshot,
                     hooks: [String: AgentHook],
                     hooksLoaded: Bool = true,
                     mux: String = "tmux") -> BreadcrumbPlan? {
        guard snapshot.isAttached else { return nil }

        let sessionTitle = snapshot.activeSessionId
            .flatMap { snapshot.sessions[$0]?.name } ?? "—"
        let window = snapshot.activeWindowId.flatMap { snapshot.windows[$0] }
        // herdr names tabs after their number by default, and "1:1" says "1"
        // twice. tmux windows are named after their command, so both halves
        // carry information there.
        let windowTitle = window.map {
            $0.name == String($0.index) ? "\($0.index)" : "\($0.index):\($0.name)"
        } ?? "—"

        let pane = snapshot.activePaneId.flatMap { snapshot.panes[$0] }
        let hook = snapshot.activePaneId.flatMap { hooks[$0] }
        let agentName = hook?.agent.flatMap { $0.isEmpty ? nil : $0 }
        let signal = hook.flatMap { AgentSignal($0.state) }

        // Agent name beats the foreground command (`claude` is written on the
        // pane either way, but the hook's spelling is the authoritative one),
        // which beats the pane number — the herdr-0.7.3 floor, kept because a
        // vanished crumb takes the Select Pane sheet's only entry with it.
        // The command is the pane's display name, not the raw comm: Claude
        // Code's binary is named after its version (see `displayCommand`).
        let paneTitle: String? = pane.map { pane in
            agentName
                ?? (pane.command.isEmpty ? String(localized: "pane \(pane.index)") : pane.displayCommand)
        }

        // Only what names the segments: a change elsewhere in the tree
        // (another window's panes, their stamps) is no change of the plan.
        var trimmed = snapshot
        trimmed.windows = window.map { [$0.id: $0] } ?? [:]
        trimmed.panes = pane.map { [$0.id: $0] } ?? [:]
        let tree = Tree(
            mux: mux, attached: snapshot.isAttached, snapshot: trimmed,
            hooks: snapshot.activePaneId.flatMap { id in hook.map { [id: $0] } } ?? [:])

        return BreadcrumbPlan(
            sessionTitle: sessionTitle,
            sessionSqueezable: agentName != nil || signal != nil,
            windowTitle: windowTitle,
            paneTitle: paneTitle,
            paneSignal: signal,
            isComplete: window != nil && pane != nil && hooksLoaded,
            tree: tree)
    }
}

extension BreadcrumbPlan {
    /// The trimmed tree the NSH bar names its segments from, posted in the
    /// shape of a Home card's `control` (the ui crate reads it with Home's
    /// model). Its own type since the move to the core removed Swift's
    /// HomeSnapshot, whose Control this used to borrow.
    struct Tree: Equatable {
        var mux: String
        var attached: Bool
        var snapshot: TmuxSnapshot
        var hooks: [String: AgentHook]

        var json: [String: Any] {
            var control: [String: Any] = [
                "mux": mux,
                "attached": attached,
                "refreshing": false,
                "sessions": snapshot.sessions.values.sorted { $0.id < $1.id }
                    .map { ["id": $0.id, "name": $0.name] },
                "windows": snapshot.windows.values.sorted { $0.id < $1.id }.map {
                    ["id": $0.id, "session": $0.sessionId, "index": $0.index, "name": $0.name,
                     "paneCount": $0.paneCount] as [String: Any]
                },
                "panes": snapshot.panes.values.sorted { $0.id < $1.id }.map { pane -> [String: Any] in
                    var entry: [String: Any] = ["id": pane.id, "window": pane.windowId,
                                                "index": pane.index, "command": pane.command]
                    if let path = pane.path { entry["path"] = path }
                    return entry
                },
                "hooks": hooks.keys.sorted().map { pane -> [String: Any] in
                    let hook = hooks[pane]!
                    var entry: [String: Any] = ["pane": pane]
                    if let state = hook.state { entry["state"] = state }
                    if let agent = hook.agent { entry["agent"] = agent }
                    if let at = hook.since { entry["since"] = at.timeIntervalSince1970 }
                    return entry
                },
                "worktrees": [] as [Any],
            ]
            if let id = snapshot.activeSessionId { control["activeSession"] = id }
            if let id = snapshot.activeWindowId { control["activeWindow"] = id }
            if let id = snapshot.activePaneId { control["activePane"] = id }
            return control
        }
    }
}
