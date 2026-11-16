import Foundation

/// Everything the NSH Home shows, read from the session hub as the SwiftUI
/// card reads it (`ui/src/home/model.rs`'s `HomeState`).
struct HomeSnapshot: Equatable {
    var connections: [Card]
    var version: String
    /// Home is the iPad's sidebar.
    var sidebar = false

    /// One saved connection and what its session has.
    struct Card: Equatable {
        var id: UUID
        var name: String
        var initial: String
        var user: String
        var host: String
        var mosh: Bool
        var udpPort: Int
        var link: Link
        var transport: String
        var degraded: Bool
        var connectedAt: Date?
        var roaming: Bool
        var disconnecting: Bool
        var control: Control?
    }

    /// Where a session is (`ConnectionCard`'s `isDead`, `isConnecting`,
    /// `isAttachStalled`, `isLive`).
    enum Link: String {
        case saved, connecting, reconnecting, attaching, stalled, dead, live
    }

    /// A multiplexer's control plane, as its snapshot and stamps have it.
    struct Control: Equatable {
        var mux: String
        var attached: Bool
        var refreshing: Bool
        var snapshot: TmuxSnapshot
        var hooks: [String: AgentHook]
        /// The monitor's clock for panes whose stamp has no time of its own.
        var since: [String: Date]
        var mismatch: Mismatch?
        var worktrees: [String: String]
    }

    struct Mismatch: Equatable {
        var notice: String
        var restart: Bool
        var restarting: Bool
    }

    @MainActor
    init(store: ConnectionStore, hub: SessionHub, metrics: SessionMetricsRegistry,
         monitor: AgentActivityMonitor, disconnecting: Set<UUID>, sidebar: Bool = false) {
        self.sidebar = sidebar
        connections = store.connections.map { connection in
            let active = hub.session(for: connection)
            let metric = metrics.metrics[connection.id]
            return Card(
                id: connection.id,
                name: connection.displayName,
                initial: String(connection.displayName.prefix(1)).uppercased(),
                user: connection.username,
                host: connection.host,
                mosh: connection.connectionProtocol == .mosh,
                udpPort: connection.moshPortRangeStart,
                link: Self.link(connection, active),
                transport: Self.transport(active?.viewModel.connState),
                degraded: metric?.moshDegraded == true,
                connectedAt: metric?.connectedAt,
                roaming: metric?.isRoaming == true,
                disconnecting: disconnecting.contains(connection.id),
                control: active.flatMap { Self.control($0, since: monitor.stateSinceByPane(connectionId: connection.id)) })
        }
        version = HomeFooter.line
    }

    /// For tests: a snapshot of `connections` as given.
    init(connections: [Card], version: String = "v1.0.9 (1)") {
        self.connections = connections
        self.version = version
    }

    /// Where `active` is (the SwiftUI card's order of questions: dead, then
    /// connecting, then a stalled attach, then live).
    @MainActor
    static func link(_ connection: ServerConnection, _ active: SessionHub.ActiveSession?) -> Link {
        guard let active else { return .saved }
        return link(status: active.viewModel.status, multiplexer: connection.multiplexer,
                    tmux: active.tmuxControl?.snapshot, stalled: active.attachStalled,
                    awaitingFirstSession: active.awaitingFirstSession)
    }

    /// ``link(_:_:)`` of a session's parts. `awaitingFirstSession`: a server
    /// with no session, waiting for the person's (B228) — live, with the
    /// empty tree and its +, not attaching.
    static func link(status: TerminalViewModel.Status, multiplexer: Multiplexer,
                     tmux: TmuxSnapshot?, stalled: Bool, awaitingFirstSession: Bool = false) -> Link {
        switch status {
        case .failed, .disconnected, .ended: return .dead
        case .connecting: return .connecting
        case .reconnecting: return .reconnecting
        default: break
        }
        if multiplexer == .tmux, let tmux, !tmux.isAttached, !tmux.everAttached, !stalled,
           !awaitingFirstSession, status == .connected {
            return .attaching
        }
        return stalled && !awaitingFirstSession ? .stalled : .live
    }

    static func transport(_ state: TransportConnState?) -> String {
        switch state {
        case .connecting: "connecting"
        case .reconnecting: "reconnecting"
        case .offline: "offline"
        case .live, nil: "live"
        }
    }

    /// The control plane of `active`, whichever multiplexer is behind it.
    @MainActor
    static func control(_ active: SessionHub.ActiveSession, since: [String: Date]) -> Control? {
        if let tmux = active.tmuxControl {
            return control(of: tmux, mismatch: nil, since: since)
        }
        if let herdr = active.herdrControl {
            let mismatch = herdr.protocolMismatch.map { message in
                let skew = herdr.versionSkew
                return Mismatch(notice: skew?.notice ?? message,
                                restart: !(skew?.clientIsOlder ?? HerdrSnapshot.clientIsOlder(message)),
                                restarting: herdr.isRestartingServer)
            }
            return control(of: herdr, mismatch: mismatch, since: since)
        }
        return nil
    }

    @MainActor
    private static func control<C: MultiplexerControlling>(of control: C, mismatch: Mismatch?,
                                                           since: [String: Date]) -> Control {
        let snapshot = control.snapshot
        var worktrees: [String: String] = [:]
        if control.multiplexer == .herdr {
            for id in snapshot.sessions.keys {
                if let repo = control.worktreeRepo(for: id) { worktrees[id] = repo }
            }
        }
        return Control(
            mux: control.multiplexer == .herdr ? "herdr" : "tmux",
            attached: snapshot.isAttached,
            refreshing: control.isRefreshing,
            snapshot: snapshot,
            hooks: control.agentHooks,
            since: since,
            mismatch: mismatch,
            worktrees: worktrees)
    }

    /// `{"op": "home", …}`.
    var message: [String: Any] {
        ["op": "home",
         "connections": connections.map(\.json),
         "version": version,
         "sidebar": sidebar]
    }
}

extension HomeSnapshot.Card {
    var json: [String: Any] {
        var card: [String: Any] = [
            "id": id.uuidString,
            "name": name,
            "initial": initial,
            "user": user,
            "host": host,
            "mosh": mosh,
            "udpPort": udpPort,
            "link": link.rawValue,
            "transport": transport,
            "degraded": degraded,
            "roaming": roaming,
            "disconnecting": disconnecting,
        ]
        if let connectedAt { card["connectedAt"] = connectedAt.timeIntervalSince1970 }
        if let control { card["control"] = control.json }
        return card
    }
}

extension HomeSnapshot.Control {
    var json: [String: Any] {
        var control: [String: Any] = [
            "mux": mux,
            "attached": attached,
            "refreshing": refreshing,
            "sessions": snapshot.sessions.values.sorted { $0.id < $1.id }
                .map { ["id": $0.id, "name": $0.name] },
            "windows": snapshot.windows.values.sorted { $0.id < $1.id }.map {
                ["id": $0.id, "session": $0.sessionId, "index": $0.index, "name": $0.name,
                 "paneCount": $0.paneCount] as [String: Any]
            },
            "panes": snapshot.panes.values.sorted { $0.id < $1.id }.map { pane -> [String: Any] in
                var entry: [String: Any] = ["id": pane.id, "window": pane.windowId, "index": pane.index,
                                            "command": pane.command]
                if let path = pane.path { entry["path"] = path }
                return entry
            },
            "hooks": hooks.keys.sorted().map { pane -> [String: Any] in
                let hook = hooks[pane]!
                var entry: [String: Any] = ["pane": pane]
                if let state = hook.state { entry["state"] = state }
                if let agent = hook.agent { entry["agent"] = agent }
                if let at = hook.since ?? since[pane] { entry["since"] = at.timeIntervalSince1970 }
                return entry
            },
            "worktrees": worktrees.keys.sorted().map { ["session": $0, "repo": worktrees[$0]!] },
        ]
        if let id = snapshot.activeSessionId { control["activeSession"] = id }
        if let id = snapshot.activeWindowId { control["activeWindow"] = id }
        if let id = snapshot.activePaneId { control["activePane"] = id }
        if let mismatch {
            control["mismatch"] = ["notice": mismatch.notice, "restart": mismatch.restart,
                                   "restarting": mismatch.restarting]
        }
        return control
    }
}

