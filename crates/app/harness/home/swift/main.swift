import Foundation

// home-swift <scenarios.json> [scenario ...]: one line per scenario,
// {"scenario": name, "message": HomeSnapshot.message}, the snapshot built as
// the NSH Home page builds it (`HomeSnapshot(store:hub:metrics:monitor:…)`)
// from session objects set up as the Rust session path's driver sets them.

func list(_ value: Any?) -> [[String: Any]] { value as? [[String: Any]] ?? [] }
func date(_ value: Any?) -> Date? { (value as? Double).map(Date.init(timeIntervalSince1970:)) }

/// A control plane's tree, as `RustTmuxController.mirror` and
/// `RustHerdrController.mirror` build it from the core's read model.
func tree(_ t: [String: Any], cwds: [String: String] = [:]) -> TmuxSnapshot {
    var snapshot = TmuxSnapshot()
    for s in list(t["sessions"]) {
        let id = s["id"] as! String
        snapshot.sessions[id] = SessionInfo(id: id, name: s["name"] as? String ?? "",
                                            isAttached: s["attached"] as? Bool ?? false)
    }
    for w in list(t["windows"]) {
        let id = w["id"] as! String
        snapshot.windows[id] = WindowInfo(id: id, sessionId: w["session"] as! String,
                                          name: w["name"] as? String ?? "", index: w["index"] as? Int ?? 0,
                                          paneCount: w["paneCount"] as? Int ?? 1)
    }
    for p in list(t["panes"]) {
        let id = p["id"] as! String
        // herdr's pane carries its directory from the client's cwds
        // (`herdr_read_model`'s `path: view.pane_cwds.get(..)`).
        snapshot.panes[id] = PaneInfo(id: id, windowId: p["window"] as! String, index: p["index"] as? Int ?? 0,
                                      command: p["command"] as? String ?? "",
                                      path: p["path"] as? String ?? cwds[id])
    }
    snapshot.activeSessionId = t["activeSession"] as? String
    snapshot.activeWindowId = t["activeWindow"] as? String
    snapshot.activePaneId = t["activePane"] as? String
    snapshot.isAttached = t["attached"] as? Bool ?? false
    snapshot.everAttached = t["everAttached"] as? Bool ?? false
    return snapshot
}

func hooks(_ t: [String: Any]) -> [String: AgentHook] {
    var out: [String: AgentHook] = [:]
    for h in list(t["hooks"]) {
        out[h["pane"] as! String] = AgentHook(state: h["state"] as? String, agent: h["agent"] as? String,
                                              since: date(h["since"]), command: nil, title: nil)
    }
    return out
}

let arguments = CommandLine.arguments
let fixture = try! JSONSerialization.jsonObject(
    with: Data(contentsOf: URL(fileURLWithPath: arguments[1]))) as! [String: Any]
let wanted = Set(arguments.dropFirst(2))

MainActor.assumeIsolated {
    for scenario in list(fixture["scenarios"]) {
        let name = scenario["name"] as! String
        guard wanted.isEmpty || wanted.contains(name) else { continue }
        let store = ConnectionStore()
        let hub = SessionHub()
        let metrics = SessionMetricsRegistry()
        let monitor = AgentActivityMonitor()
        var disconnecting: Set<UUID> = []
        HomeFooter.line = scenario["version"] as? String ?? "v1.0.9 (1)"
        for c in list(scenario["connections"]) {
            let id = UUID(uuidString: c["id"] as! String)!
            let mux: Multiplexer = switch c["mux"] as? String ?? "tmux" {
            case "none": .none
            case "herdr": .herdr
            default: .tmux
            }
            let connection = ServerConnection(
                id: id, displayName: c["name"] as! String, username: c["user"] as? String ?? "cluas",
                host: c["host"] as? String ?? "studio.local",
                connectionProtocol: c["mosh"] as? Bool == true ? .mosh : .ssh,
                moshPortRangeStart: c["udpPort"] as? Int ?? 60001, multiplexer: mux)
            store.connections.append(connection)
            if c["disconnecting"] as? Bool == true { disconnecting.insert(id) }
            guard let s = c["session"] as? [String: Any] else { continue }
            let active = SessionHub.ActiveSession()
            // `RustSessionDriver.apply`.
            switch s["state"] as! String {
            case "connecting": active.viewModel.adoptRustStatus(.connecting, automatic: false)
            case "reconnecting": active.viewModel.adoptRustStatus(.reconnecting, automatic: true)
            case "connected": active.viewModel.adoptRustStatus(.connected, automatic: false)
            case "ended": active.viewModel.adoptRustStatus(.disconnected, automatic: false)
            case "failed": active.viewModel.adoptRustStatus(.failed("failed"), automatic: false)
            case let other: fatalError("state \(other)")
            }
            active.attachStalled = s["stalled"] as? Bool ?? false
            if let t = s["tmux"] as? [String: Any] {
                let control = TmuxSessionController()
                control.snapshot = tree(t)
                control.agentHooks = hooks(t)
                control.isRefreshing = t["refreshing"] as? Bool ?? false
                if t["noSession"] as? Bool == true {
                    // mosh + tmux, the server asked and found with no
                    // session (`attachMoshTmux`, B228): the sidecar's
                    // controller.
                    active.moshControl = control
                    active.moshServerHadNoSession = true
                } else {
                    active.tmuxController = control
                }
            }
            if let h = s["herdr"] as? [String: Any] {
                let control = HerdrControlClient()
                let cwds = h["cwds"] as? [String: String] ?? [:]
                control.snapshot = tree(h, cwds: cwds)
                control.agentHooks = hooks(h)
                control.isRefreshing = h["refreshing"] as? Bool ?? false
                control.protocolMismatch = h["mismatch"] as? String
                control.versionSkew = (h["skew"] as? [String: String]).map {
                    HerdrVersionReport(client: $0["client"]!, server: $0["server"]!)
                }
                control.isRestartingServer = h["restarting"] as? Bool ?? false
                control.worktreeRepos = h["worktrees"] as? [String: String] ?? [:]
                active.herdrControl = control
            }
            hub.sessions[id] = active
            metrics.metrics[id] = SessionMetrics(connectedAt: date(s["connectedAt"]),
                                                 isRoaming: s["roaming"] as? Bool ?? false,
                                                 moshDegraded: s["degraded"] as? Bool ?? false)
            if let since = s["since"] as? [String: Double] {
                monitor.since[id] = since.mapValues(Date.init(timeIntervalSince1970:))
            }
        }
        let snapshot = HomeSnapshot(store: store, hub: hub, metrics: metrics, monitor: monitor,
                                    disconnecting: disconnecting,
                                    sidebar: scenario["sidebar"] as? Bool ?? false)
        let line = try! JSONSerialization.data(
            withJSONObject: ["scenario": name, "message": snapshot.message], options: [.sortedKeys])
        print(String(decoding: line, as: UTF8.self))
    }
}
