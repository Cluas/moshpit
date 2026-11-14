import Foundation

// What the app's HomeSnapshot reads, without the app: the store, the session
// hub, the metrics, the monitor and the two control planes as data. Only
// what HomeSnapshot touches is here; everything it decides with — the
// snapshot itself, TmuxSnapshot and its infos, AgentHook, the herdr skew's
// words, TerminalViewModel's status and connState — is the app's own source,
// taken from git by build-swift.sh.

enum ConnectionProtocol { case ssh, mosh }

enum Multiplexer { case none, tmux, herdr }

struct ServerConnection {
    var id: UUID
    var displayName: String
    var username: String
    var host: String
    var connectionProtocol: ConnectionProtocol
    var moshPortRangeStart: Int
    var multiplexer: Multiplexer
}

final class ConnectionStore {
    var connections: [ServerConnection] = []
}

struct SessionMetrics {
    var connectedAt: Date?
    var isRoaming = false
    var moshDegraded = false
}

final class SessionMetricsRegistry {
    var metrics: [UUID: SessionMetrics] = [:]
}

/// `AgentActivityMonitor.stateSinceByPane(connectionId:)`: the machine's
/// clock for a connection's panes.
final class AgentActivityMonitor {
    var since: [UUID: [String: Date]] = [:]
    func stateSinceByPane(connectionId: UUID) -> [String: Date] { since[connectionId] ?? [:] }
}

enum HomeFooter {
    nonisolated(unsafe) static var line = ""
}

/// The requirements HomeSnapshot reads (`MultiplexerControlling.swift`).
protocol MultiplexerControlling: AnyObject {
    var multiplexer: Multiplexer { get }
    var snapshot: TmuxSnapshot { get }
    var agentHooks: [String: AgentHook] { get }
    var isRefreshing: Bool { get }
    func worktreeRepo(for sessionId: String) -> String?
}

/// `RustTmuxController` as `mirror` leaves it.
final class TmuxSessionController: MultiplexerControlling {
    var multiplexer: Multiplexer { .tmux }
    var snapshot = TmuxSnapshot()
    var agentHooks: [String: AgentHook] = [:]
    var isRefreshing = false
    func worktreeRepo(for sessionId: String) -> String? { nil }
}

/// `RustHerdrController` as `mirror` leaves it (`mirrorRust`).
final class HerdrControlClient: MultiplexerControlling {
    var multiplexer: Multiplexer { .herdr }
    var snapshot = TmuxSnapshot()
    var agentHooks: [String: AgentHook] = [:]
    var isRefreshing = false
    var protocolMismatch: String?
    var versionSkew: HerdrVersionReport?
    var isRestartingServer = false
    var worktreeRepos: [String: String] = [:]
    func worktreeRepo(for sessionId: String) -> String? { worktreeRepos[sessionId] }
}

final class SessionHub {
    final class ActiveSession {
        let viewModel = TerminalViewModel()
        var attachStalled = false
        /// Set only by the Swift session path's mosh attach
        /// (`attachMoshTmux`); the Rust path's driver never sets it.
        var moshServerHadNoSession = false
        var tmuxController: TmuxSessionController?
        var moshControl: TmuxSessionController?
        var herdrControl: HerdrControlClient?
        var tmuxControl: TmuxSessionController? { tmuxController ?? moshControl }
        // `awaitingFirstSession`, as the app has it (build-swift.sh).
        //__AWAITING_FIRST_SESSION__
    }

    var sessions: [UUID: ActiveSession] = [:]

    func session(for connection: ServerConnection) -> ActiveSession? {
        sessions[connection.id]
    }
}

/// The view model's status and the transport state every surface reads
/// from it, as the app has them (build-swift.sh).
final class TerminalViewModel {
    //__TERMINAL_VIEW_MODEL__
    private(set) var status: Status = .idle
    var errorMessage: String?
    private(set) var isAutoReconnectInFlight = false
}
