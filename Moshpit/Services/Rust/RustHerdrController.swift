import Foundation
import MoshpitKit

/// herdr on the Rust core, as the chrome reads herdr: a
/// ``HerdrControlClient`` whose observable surface — the tree, the agents'
/// stamps, the empty state, the skew, the spinners — mirrors the core's
/// herdr read model (`HerdrReadModel`), and whose every action is an intent
/// to the core (W2b's client runs there: its command lane, its exec
/// budgets, its push channel). The sheets, the breadcrumb, the Home tree
/// and the New agent task form work on it as they do on the Swift client,
/// unchanged; none of the Swift client's own polling or commands run.
///
/// The ones with an answer (a task, a removal, discovery, a restart) wait
/// for the core's answer to their token.
@MainActor
final class RustHerdrController: HerdrControlClient {
    private weak var driver: RustSessionDriver?

    init(driver: RustSessionDriver) {
        self.driver = driver
        super.init(runner: RustNullRunner(), customPath: nil)
    }

    // MARK: Mirroring the core

    func mirror(_ model: HerdrReadModel) {
        var snapshot = TmuxSnapshot()
        for session in model.sessions {
            snapshot.sessions[session.id] = SessionInfo(
                id: session.id, name: session.name, isAttached: session.isAttached)
        }
        for window in model.windows {
            snapshot.windows[window.id] = WindowInfo(
                id: window.id, sessionId: window.sessionId, name: window.name,
                index: Int(window.index), layout: window.layout, isActive: window.isActive,
                paneCount: Int(window.paneCount), isZoomed: window.isZoomed)
        }
        for pane in model.panes {
            snapshot.panes[pane.id] = PaneInfo(
                id: pane.id, windowId: pane.windowId, index: Int(pane.index),
                command: pane.command, width: Int(pane.width), height: Int(pane.height),
                isActive: pane.isActive, path: pane.path)
        }
        snapshot.activeSessionId = model.activeSessionId
        snapshot.activeWindowId = model.activeWindowId
        snapshot.activePaneId = model.activePaneId
        snapshot.isAttached = model.isAttached
        snapshot.everAttached = model.everAttached
        var hooks: [String: AgentHook] = [:]
        for hook in model.agentHooks {
            hooks[hook.pane] = AgentHook(
                state: hook.state, agent: hook.agent,
                since: hook.since.map { Date(timeIntervalSince1970: $0) },
                command: hook.command, title: hook.title)
        }
        let skew = model.skewClient.flatMap { client in
            model.skewServer.map { HerdrVersionReport(client: client, server: $0) }
        }
        mirrorRust(snapshot: snapshot, agentHooks: hooks, agentHooksLoaded: model.agentHooksLoaded,
                   isRefreshing: model.refreshing, serverNotRunning: model.serverNotRunning,
                   protocolMismatch: model.protocolMismatch, versionSkew: skew,
                   isRestartingServer: model.restarting, pushActive: model.pushActive,
                   paneSwitchInFlight: model.paneSwitchInFlight, paneCwds: model.paneCwds,
                   worktreeRepos: model.worktreeRepos)
    }

    // MARK: The client's surface, on the core

    override func start() {}
    override func stop() {}
    /// The core reads on its own cadence (and at once after its commands).
    override func quicken() {}
    override func pollAgentHooks() {}

    /// The core's poll paces the stamps here, not this client's (B346).
    override var stampInterval: TimeInterval { herdrStampInterval(pushActive: pushActive) }
    override func refresh() { driver?.herdr(.refresh) }
    override func requestImmersiveZoom() {}

    override func selectSession(_ sessionId: String) { driver?.herdr(.selectSession(id: sessionId)) }
    override func selectWindow(_ windowId: String) { driver?.herdr(.selectWindow(id: windowId)) }
    override func selectPane(_ paneId: String) { driver?.herdr(.selectPane(id: paneId)) }
    override func newSession(named name: String?) { driver?.herdr(.newSession(name: name)) }
    override func newWindow(named name: String?) { driver?.herdr(.newWindow(name: name)) }
    override func newPane() { driver?.herdr(.newPane) }
    override func newWindow(inSession sessionId: String, named name: String?) {
        driver?.herdr(.newWindowInSession(session: sessionId, name: name))
    }
    override func newPane(inWindow windowId: String) {
        driver?.herdr(.newPaneInWindow(window: windowId))
    }
    override func renameSession(_ sessionId: String, to name: String) {
        driver?.herdr(.renameSession(session: sessionId, name: name))
    }
    override func killSession(_ sessionId: String) { driver?.herdr(.killSession(id: sessionId)) }
    override func renameWindow(_ windowId: String, to name: String) {
        driver?.herdr(.renameWindow(window: windowId, name: name))
    }
    override func killWindow(_ windowId: String) { driver?.herdr(.killWindow(id: windowId)) }
    override func killPane(_ paneId: String) { driver?.herdr(.killPane(id: paneId)) }

    override func startAgentTask(_ request: AgentTaskRequest) async -> AgentTaskOutcome {
        if let invalid = request.validationError { return .failed(invalid) }
        let intent = HerdrIntent.startAgentTask(
            repoPath: request.repoPath, branch: request.branch.trimmingCharacters(in: .whitespaces),
            agent: request.agent, prompt: request.prompt)
        guard case .task(let outcome)? = await driver?.ask(intent) else {
            return .failed(String(localized: "Couldn't reach the host"))
        }
        return RustCopy.task(outcome)
    }

    override func removeWorktree(sessionId: String, force: Bool = false) async -> WorktreeRemoval {
        guard case .removal(let outcome)? = await driver?.ask(
            .removeWorktree(session: sessionId, force: force)) else {
            return .failed(String(localized: "Couldn't reach the host"))
        }
        return RustCopy.removal(outcome)
    }

    override func gitRepos(scanBudget: Duration = .seconds(8)) async -> [String] {
        guard case .repos(let repos)? = await driver?.ask(.discoverRepos) else { return [] }
        return repos
    }

    override func agentNames() async -> [String] {
        guard case .agents(let names, _)? = await driver?.ask(.agentNames) else { return [] }
        return names
    }

    override func restartServer() async -> RestartOutcome {
        guard case .restart(let outcome)? = await driver?.ask(.restartServer) else { return .failed }
        return RustCopy.restart(outcome)
    }

    override func confirmedFocus(within budget: Duration) async -> String? {
        let ms = UInt64(max(0, budget.components.seconds) * 1000
                        + budget.components.attoseconds / 1_000_000_000_000_000)
        guard case .focus(let pane)? = await driver?.ask(.confirmFocus(budgetMs: ms)) else { return nil }
        return pane
    }

    /// The core keeps the skew while the two disagree: mirrored already.
    override func diagnoseVersionSkew() async -> HerdrVersionReport? { versionSkew }
}

/// What the Swift client underneath is made with: nothing runs through it
/// (every action goes to the core).
private struct RustNullRunner: HerdrCommandRunner {
    func run(_ command: String) async throws -> String { "" }
}
