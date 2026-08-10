import Foundation

/// What the terminal top bar's breadcrumb shows, decided away from the view.
///
/// The interesting case is a pane with an agent in it. "Which agent is this,
/// and is it waiting on me" is exactly what the person glancing at the top bar
/// wants to know — but the third crumb only knew the pane's foreground
/// command, which under herdr 0.7.3 is nothing at all (`pane N`).
///
/// Fitting the agent in is a width problem before it is anything else: three
/// full segments don't fit 402pt, which was discovered by drawing it (see the
/// flow prototype's stage-⑤ sketch). The answer here: when the pane carries an
/// agent, the session crumb gives up its text and keeps only its icon — still
/// tappable, still discoverable, but no longer spending ~90pt saying a
/// workspace name that the Sessions sheet repeats anyway.
///
/// Pure — the snapshot and hooks go in, the strings come out — so the
/// squeeze rule and the fallback chain are testable without a connection.
struct BreadcrumbPlan: Equatable {
    var sessionTitle: String
    /// True when the pane segment carries an agent and needs the room: the
    /// session crumb renders icon-only.
    var sessionIconOnly: Bool
    var windowTitle: String
    /// nil = no pane crumb at all (no active pane in the snapshot).
    var paneTitle: String?
    /// The dot inside the pane crumb — the same signal the Agents section and
    /// the island show, in the same colours. nil = plain pane, no dot.
    var paneSignal: AgentSignal?

    static func make(snapshot: TmuxSnapshot,
                     hooks: [String: AgentHook]) -> BreadcrumbPlan? {
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
        let paneTitle: String? = pane.map { pane in
            agentName
                ?? (pane.command.isEmpty ? String(localized: "pane \(pane.index)") : pane.command)
        }

        return BreadcrumbPlan(
            sessionTitle: sessionTitle,
            sessionIconOnly: agentName != nil || signal != nil,
            windowTitle: windowTitle,
            paneTitle: paneTitle,
            paneSignal: signal)
    }
}
