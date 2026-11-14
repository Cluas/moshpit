import Foundation
import Observation
import MoshpitKit

/// herdr on the Rust core, as the chrome reads herdr: its observable
/// surface — the tree, the agents' stamps, the empty state, the skew, the
/// spinners — mirrors the core's herdr read model (`HerdrReadModel`), and
/// its every action is an intent to the core (crates/herdr's client runs
/// there: its command lane, its exec budgets, its push channel). The sheets,
/// the breadcrumb, the Home tree and the New agent task form work on it.
///
/// The ones with an answer (a task, a removal, discovery, a restart) wait
/// for the core's answer to their token.
///
/// It used to be a subclass of the Swift herdr client, whose own polling
/// and commands it overrode; that client is gone (E1).
@MainActor
@Observable
final class RustHerdrController: MultiplexerControlling {
    @ObservationIgnored private weak var driver: RustSessionDriver?

    /// `driver`: the session on the core its actions go to (nil: a surface
    /// that only mirrors, as tests make one).
    init(driver: RustSessionDriver?) {
        self.driver = driver
    }

    nonisolated var multiplexer: Multiplexer { .herdr }

    // MARK: What the core read

    private(set) var snapshot = TmuxSnapshot()
    private(set) var agentHooks: [String: AgentHook] = [:]
    private(set) var agentHooksLoaded = false
    private(set) var isRefreshing = false
    /// The host has herdr but no server running: the empty state's, not
    /// "we haven't managed to read anything yet".
    private(set) var serverNotRunning = false
    /// The server refuses this client over version skew, with its words:
    /// the Home card names it instead of "No workspaces yet". nil while
    /// healthy.
    private(set) var protocolMismatch: String?
    /// Which `herdr` the core's commands run and which server is up, kept
    /// only while the two disagree: the Home row drops its restart button
    /// when restarting would downgrade.
    private(set) var versionSkew: HerdrVersionReport?
    /// A restart is under way: the Home row's spinner.
    private(set) var isRestartingServer = false
    /// The push channel carries herdr's events (the poll is a safety net).
    private(set) var pushActive = false
    /// When what herdr refused is asked again by itself, while a backoff
    /// runs (B490): the banner counts down to it.
    private(set) var retryDeadline: Date?
    /// Working directory per pane, from the core's last read.
    @ObservationIgnored private var paneCwds: [String: String] = [:]
    /// Workspaces that are git worktrees → the repo they came from.
    private var worktreeRepos: [String: String] = [:]

    /// `MultiplexerControlling.onAgentHooksUpdated`: fired by the driver
    /// after every read that carried the agents' stamps.
    @ObservationIgnored var onAgentHooksUpdated: (() -> Void)?

    /// How often this client's agent stamps arrive, in seconds: the core's
    /// poll paces them (slower while push is up). The agents machine holds a
    /// stamp that long and a margin (B346).
    var stampInterval: TimeInterval { herdrStampInterval(pushActive: pushActive) }

    // MARK: Mirroring the core

    /// The backoff's length last mirrored: a new one starts its countdown
    /// now (B490).
    @ObservationIgnored private var mirroredRetryWait: UInt64?

    /// The core's read model, written where the chrome reads it. Only what
    /// changed is written: every write is an Observation change.
    func mirror(_ model: HerdrReadModel) {
        if model.retryWaitMs != mirroredRetryWait {
            mirroredRetryWait = model.retryWaitMs
            retryDeadline = model.retryWaitMs.map { Date().addingTimeInterval(Double($0) / 1000) }
        }
        var next = TmuxSnapshot()
        for session in model.sessions {
            next.sessions[session.id] = SessionInfo(
                id: session.id, name: session.name, isAttached: session.isAttached)
        }
        for window in model.windows {
            next.windows[window.id] = WindowInfo(
                id: window.id, sessionId: window.sessionId, name: window.name,
                index: Int(window.index), layout: window.layout, isActive: window.isActive,
                paneCount: Int(window.paneCount), isZoomed: window.isZoomed)
        }
        for pane in model.panes {
            next.panes[pane.id] = PaneInfo(
                id: pane.id, windowId: pane.windowId, index: Int(pane.index),
                command: pane.command, width: Int(pane.width), height: Int(pane.height),
                isActive: pane.isActive, path: pane.path)
        }
        next.activeSessionId = model.activeSessionId
        next.activeWindowId = model.activeWindowId
        next.activePaneId = model.activePaneId
        next.isAttached = model.isAttached
        next.everAttached = model.everAttached
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
        if snapshot != next { snapshot = next }
        if agentHooks != hooks { agentHooks = hooks }
        if agentHooksLoaded != model.agentHooksLoaded { agentHooksLoaded = model.agentHooksLoaded }
        if isRefreshing != model.refreshing { isRefreshing = model.refreshing }
        if serverNotRunning != model.serverNotRunning { serverNotRunning = model.serverNotRunning }
        if protocolMismatch != model.protocolMismatch { protocolMismatch = model.protocolMismatch }
        if versionSkew != skew { versionSkew = skew }
        if isRestartingServer != model.restarting { isRestartingServer = model.restarting }
        if pushActive != model.pushActive { pushActive = model.pushActive }
        paneCwds = model.paneCwds
        if worktreeRepos != model.worktreeRepos { worktreeRepos = model.worktreeRepos }
    }

    // MARK: What the core read, asked

    /// Repos the panes are sitting in right now. Instant — no remote call,
    /// just the cwds the last read carried — so the New agent task form can
    /// open with something in it while discovery runs.
    var openRepoGuesses: [String] {
        var seen: Set<String> = []
        return paneCwds.values.sorted().filter { seen.insert($0).inserted }
    }

    /// `MultiplexerControlling.worktreeRepo` — only a workspace herdr created
    /// from a repo answers yes.
    func worktreeRepo(for sessionId: String) -> String? { worktreeRepos[sessionId] }

    /// Where the pane's foreground program runs, from the last read — what
    /// an agent's relative paths are relative to.
    func workingDirectory(ofPane paneId: String) -> String? { paneCwds[paneId] }

    // MARK: Actions, on the core

    /// The core reads on its own cadence (and at once after its commands).
    func pollAgentHooks() {}
    func refresh() { driver?.herdr(.refresh) }

    func selectSession(_ sessionId: String) { driver?.herdr(.selectSession(id: sessionId)) }
    func selectWindow(_ windowId: String) { driver?.herdr(.selectWindow(id: windowId)) }
    func selectPane(_ paneId: String) { driver?.herdr(.selectPane(id: paneId)) }
    func newSession(named name: String?) { driver?.herdr(.newSession(name: name)) }
    func newWindow(named name: String?) { driver?.herdr(.newWindow(name: name)) }
    func newPane() { driver?.herdr(.newPane) }
    func newWindow(inSession sessionId: String, named name: String?) {
        driver?.herdr(.newWindowInSession(session: sessionId, name: name))
    }
    func newPane(inWindow windowId: String) {
        driver?.herdr(.newPaneInWindow(window: windowId))
    }
    func renameSession(_ sessionId: String, to name: String) {
        driver?.herdr(.renameSession(session: sessionId, name: name))
    }
    func killSession(_ sessionId: String) { driver?.herdr(.killSession(id: sessionId)) }
    func renameWindow(_ windowId: String, to name: String) {
        driver?.herdr(.renameWindow(window: windowId, name: name))
    }
    func killWindow(_ windowId: String) { driver?.herdr(.killWindow(id: windowId)) }
    func killPane(_ paneId: String) { driver?.herdr(.killPane(id: paneId)) }

    func startAgentTask(_ request: AgentTaskRequest) async -> AgentTaskOutcome {
        if let invalid = request.validationError { return .failed(invalid) }
        let intent = HerdrIntent.startAgentTask(
            repoPath: request.repoPath, branch: request.branch.trimmingCharacters(in: .whitespaces),
            agent: request.agent, prompt: request.prompt)
        guard case .task(let outcome)? = await driver?.ask(intent) else {
            return .failed(String(localized: "Couldn't reach the host"))
        }
        return RustCopy.task(outcome)
    }

    /// What happened when we tried to remove a task's worktree.
    enum WorktreeRemoval: Equatable {
        case removed
        /// The checkout has modified or untracked files. Deleting anyway
        /// destroys them, so this comes back to the caller for a second, much
        /// sharper confirmation instead of being forced silently.
        case needsForce
        case failed(String)
    }

    /// Remove a task's worktree — the branch checkout and its workspace.
    /// Never forced on the first attempt: herdr refuses a dirty checkout and
    /// leaves every file in place.
    func removeWorktree(sessionId: String, force: Bool = false) async -> WorktreeRemoval {
        guard case .removal(let outcome)? = await driver?.ask(
            .removeWorktree(session: sessionId, force: force)) else {
            return .failed(String(localized: "Couldn't reach the host"))
        }
        return RustCopy.removal(outcome)
    }

    /// Repositories on the host, most recently touched first.
    func gitRepos() async -> [String] {
        guard case .repos(let repos)? = await driver?.ask(.discoverRepos) else { return [] }
        return repos
    }

    /// The agents the host's herdr knows how to launch.
    func agentNames() async -> [String] {
        guard case .agents(let names, _)? = await driver?.ask(.agentNames) else { return [] }
        return names
    }

    enum RestartOutcome: Equatable {
        case restarted
        case failed
        /// Refused: the `herdr` we would restart with is older than the
        /// running server. The remedy is upgrading the host's herdr.
        case wouldDowngrade(HerdrVersionReport)
        /// Nothing done: the server already runs the host's herdr (B408).
        case unneeded
    }

    /// Restart the host's herdr server with the binary the core's commands
    /// resolve to. Never run on its own: `server stop` exits pane processes,
    /// and only the person knows whether what's in them survives.
    @discardableResult
    func restartServer() async -> RestartOutcome {
        guard case .restart(let outcome)? = await driver?.ask(.restartServer) else { return .failed }
        return RustCopy.restart(outcome)
    }
}
