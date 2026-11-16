import Foundation
import MoshpitKit

/// A Rust session's own SSH connection, lent to what runs beside its
/// terminal — the remote preview and its folder sheet, image uploads and the
/// share queue, the path probe, host maintenance, Install Assist's re-check
/// (W14b). The core lends the live SSH leg's connection, mosh + tmux's
/// control connection, or the one side connection plain mosh keeps
/// (crates/session `Sessions::borrow_ssh`): no second sign-in and no second
/// Face ID per feature, which each feature used to cost with a Swift SSH
/// connection of its own beside the Rust session.
///
/// Each call borrows the connection as it is at that moment, so a reconnect
/// between two calls is no matter, and runs on it (``BorrowedHost``):
///
/// - A call the person made (``waiting``) waits for a reconnect already
///   under way — at most ``reconnectWait`` — then says the session is
///   reconnecting, or isn't connected, in so many words
///   (``HostConnectionError``). The path probe (``immediate``) never waits:
///   no session, no answer, and it asks again later.
/// - A connection that hasn't answered for a while is asked `true` first
///   (``pingTimeout``): after a suspension a socket iOS killed still looks
///   open, and a call on it would wait on a dead wire.
/// - A read that the connection's drop cut off is tried once more, on the
///   connection the session comes back with; a change never is (it may have
///   happened).
/// - Never a connection of the app's own: there is no Swift dial here to
///   fall back to, by design.
@MainActor
final class RustSideChannel {
    enum Mode: Sendable {
        /// Waits for a reconnect under way: what the person asked for.
        case waiting
        /// Answers at once: the path probe, the upload sweep.
        case immediate
    }

    /// How long a call the person made waits for a reconnect under way.
    var reconnectWait: Duration = .seconds(10)
    /// A connection that answered this recently isn't asked `true` first.
    var freshFor: Duration = .seconds(10)
    /// How long the `true` may take.
    var pingTimeout: Double = 4
    /// How long a borrow may take: plain mosh's first is a dial.
    var borrowTimeout: Double = 25
    /// How often a waiting call looks at the session's state again.
    var pollInterval: Duration = .milliseconds(100)

    private weak var driver: RustSessionDriver?
    private weak var session: SessionHub.ActiveSession?
    private let sessions: CoreSessionsProtocol
    /// When the session's connection last answered, for the connection of
    /// `epoch` — bumped at every state change, so an answer from before a
    /// reconnect speaks for nothing.
    private var answeredAt: ContinuousClock.Instant?
    private var epoch = 0

    init(driver: RustSessionDriver, session: SessionHub.ActiveSession, sessions: CoreSessionsProtocol) {
        self.driver = driver
        self.session = session
        self.sessions = sessions
    }

    /// The person's calls: they wait for a reconnect under way.
    nonisolated var waiting: Lent { Lent(side: self, mode: .waiting) }
    /// The path probe's and the sweep's: they never wait.
    nonisolated var immediate: Lent { Lent(side: self, mode: .immediate) }

    /// `acquireFileTransferSSH`'s answer on a Rust session: once the
    /// session's connection is up and answering (a reconnect under way waited
    /// for), what to call — each call borrowing it afresh.
    func ready() async throws -> Lent {
        _ = try await lend(.waiting)
        return waiting
    }

    /// The driver saw the session's state change.
    func stateChanged(_ state: SessionStateValue) {
        epoch += 1
        answeredAt = nil
        if state == .connected { sweepUploads() }
    }

    // MARK: Calls

    /// `body` on the session's connection, borrowed now. A read (`retrying`)
    /// that the connection's drop cut off is tried once more, on the
    /// connection the session comes back with.
    func perform<T: Sendable>(_ mode: Mode, retrying: Bool,
                              _ body: @escaping @Sendable (BorrowedHost) async throws -> T) async throws -> T {
        var retried = false
        while true {
            let (host, epoch) = try await lend(mode)
            do {
                let value = try await body(host)
                answered(epoch)
                return value
            } catch HostConnectionError.notAnswering {
                if epoch == self.epoch { answeredAt = nil }
                if retrying, mode == .waiting, !retried {
                    retried = true
                    Log.ssh.info("side channel: the connection dropped under a call — once more on the session's next")
                    continue
                }
                throw refined(HostConnectionError.notAnswering)
            }
        }
    }

    /// The session's connection, borrowed and known to answer, with the
    /// epoch it was borrowed in.
    private func lend(_ mode: Mode) async throws -> (BorrowedHost, Int) {
        let clock = ContinuousClock()
        let deadline = clock.now + (mode == .waiting ? reconnectWait : .zero)
        while true {
            try Task.checkCancellation()
            guard let driver, let session, !session.isStopping, !driver.hasClosed else {
                throw HostConnectionError.notConnected
            }
            switch driver.state {
            case .connected:
                if let id = driver.id {
                    let epoch = self.epoch
                    if let host = try await borrow(id) {
                        try await checkAnswers(host, epoch: epoch)
                        return (host, epoch)
                    }
                    // The core is a step ahead of its events: not live any
                    // more, and the state change is on its way here.
                }
            case .connecting, .reconnecting:
                break
            case .failed, .ended:
                throw HostConnectionError.notConnected
            }
            guard clock.now < deadline else { throw refined(HostConnectionError.reconnecting) }
            try await Task.sleep(for: pollInterval)
        }
    }

    /// The session's connection, or nil while the core says it isn't live.
    private func borrow(_ id: UInt64) async throws -> BorrowedHost? {
        let sessions = self.sessions
        do {
            let ssh = try await awaitingCore(timeout: (borrowTimeout, HostConnectionError.notAnswering)) {
                try await sessions.borrowSsh(session: id)
            }
            return BorrowedHost(ssh)
        } catch BorrowFailure.NotConnected {
            return nil
        } catch {
            throw refined(BorrowedHost.mapped(error, writing: false))
        }
    }

    /// Asks the connection `true` unless it answered lately.
    private func checkAnswers(_ host: BorrowedHost, epoch: Int) async throws {
        if epoch == self.epoch, let at = answeredAt, ContinuousClock.now - at < freshFor { return }
        do {
            _ = try await awaitingCore(timeout: (pingTimeout, HostConnectionError.notAnswering)) {
                try await host.executeCommand("true")
            }
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            Log.ssh.info("side channel: the session's connection didn't answer: \(String(describing: error), privacy: .public)")
            throw refined(HostConnectionError.notAnswering)
        }
        answered(epoch)
    }

    private func answered(_ epoch: Int) {
        if epoch == self.epoch { answeredAt = .now }
    }

    /// What a connection failure means now: the session reconnecting, or not
    /// connected at all, says more than the call's own failure.
    private func refined(_ error: Error) -> Error {
        guard let lost = error as? HostConnectionError, lost.isConnectionLost else { return error }
        guard let driver, let session, !session.isStopping, !driver.hasClosed else {
            return HostConnectionError.notConnected
        }
        switch driver.state {
        case .connecting, .reconnecting: return HostConnectionError.reconnecting
        case .failed, .ended: return HostConnectionError.notConnected
        case .connected: return lost
        }
    }

    // MARK: tmux

    /// A tmux pane's working directory, asked of tmux on the session's own
    /// connection — the Swift controller's `display-message` over `-CC`:
    /// where relative paths in the pane resolve and the Files key opens. nil
    /// when tmux can't say, or the session isn't there to ask.
    func panePath(_ paneId: String) async -> String? {
        // Spliced bare: a pane id or nothing (B146).
        guard paneId.hasPrefix("%"), TmuxId.isValid(paneId),
              let connection = session?.connection else { return nil }
        let custom = connection.tmuxPath?.trimmingCharacters(in: .whitespaces) ?? ""
        let tmux = custom.isEmpty ? "tmux" : custom
        // An exec channel's shell has no login PATH: tmux where the probe looks.
        let command = "PATH=\"$PATH:\(hostExtraPathDirs())\" \(tmux) display-message -p -t \(paneId) '#{pane_current_path}'"
        let host = immediate
        guard let output = try? await awaitingCore(timeout: (4, HostConnectionError.notAnswering), {
            try await host.executeCommand(command)
        }) else { return nil }
        // The line as tmux printed it, its newline cut: spaces at the end
        // belong to the directory's name (B57).
        var path = String(decoding: output, as: UTF8.self)
        if path.hasSuffix("\n") { path.removeLast() }
        // Absolute = the first byte is `/` (B39).
        return path.utf8.first == UInt8(ascii: "/") ? path : nil
    }

    // MARK: The upload sweep

    /// The retention sweep for `~/.moshpit/uploads/`, once per connect, as
    /// the Swift path runs it on its own connection — not where it would
    /// make plain mosh dial its side connection for nobody.
    private func sweepUploads() {
        guard let connection = session?.connection,
              connection.connectionProtocol == .ssh || connection.multiplexer == .tmux else { return }
        let days = AppSettings.shared.uploadRetentionDays
        guard days > 0 else { return }
        let command = SessionHub.ActiveSession.uploadCleanupCommand(days: days)
        let host = immediate
        Task { _ = try? await host.executeCommand(command) }
    }
}

/// A ``RustSideChannel`` as a ``HostConnection``: each call borrows the
/// session's connection as it is at that moment and runs on it
/// (``BorrowedHost``). Reads are tried once more after a drop; changes never.
struct Lent: HostConnection {
    let side: RustSideChannel
    let mode: RustSideChannel.Mode

    func executeCommand(_ command: String) async throws -> Data {
        try await side.perform(mode, retrying: true) { try await $0.executeCommand(command) }
    }

    func remoteStat(_ path: String) async throws -> RemoteFileEntry {
        try await side.perform(mode, retrying: true) { try await $0.remoteStat(path) }
    }

    func openRemoteFile(_ path: String) async throws -> RemoteFileHandle {
        try await side.perform(mode, retrying: true) { try await $0.openRemoteFile(path) }
    }

    func remoteHomeDirectory() async throws -> String {
        try await side.perform(mode, retrying: true) { try await $0.remoteHomeDirectory() }
    }

    func listRemoteDirectory(_ path: String) async throws -> [RemoteFileEntry] {
        try await side.perform(mode, retrying: true) { try await $0.listRemoteDirectory(path) }
    }

    func downloadRemoteFile(_ path: String, to destination: URL,
                            progress: (@Sendable (Double) -> Void)?) async throws {
        try await side.perform(mode, retrying: true) {
            try await $0.downloadRemoteFile(path, to: destination, progress: progress)
        }
    }

    func createRemoteDirectory(_ path: String) async throws {
        try await side.perform(mode, retrying: false) { try await $0.createRemoteDirectory(path) }
    }

    func renameRemoteItem(_ path: String, to newPath: String) async throws {
        try await side.perform(mode, retrying: false) { try await $0.renameRemoteItem(path, to: newPath) }
    }

    func deleteRemoteItem(_ entry: RemoteFileEntry) async throws {
        try await side.perform(mode, retrying: false) { try await $0.deleteRemoteItem(entry) }
    }

    func uploadRemoteFile(from local: URL, to remotePath: String,
                          progress: (@Sendable (Double) -> Void)?) async throws {
        try await side.perform(mode, retrying: false) {
            try await $0.uploadRemoteFile(from: local, to: remotePath, progress: progress)
        }
    }

    /// The same file name again is the same file: tried once more after a drop.
    func uploadToUploadsDirectory(_ data: Data, named filename: String,
                                  progress: (@Sendable (Double) -> Void)?) async throws -> String {
        try await side.perform(mode, retrying: true) {
            try await $0.uploadToUploadsDirectory(data, named: filename, progress: progress)
        }
    }
}
