import Foundation
import Observation

/// View model that owns the SSH session lifecycle for a single terminal screen.
///
/// Responsibilities:
///   - Open the ``SSHSession`` via ``SSHService`` and request a PTY.
///   - Expose coarse-grained ``Status`` for the status bar.
///   - Surface fatal errors through ``errorMessage`` so the view can alert.
///   - Forward user input and resize events back into the live session.
///
/// All UI-visible state mutates on the main actor; calls into ``SSHService``
/// hop to the service actor as needed.
@Observable
@MainActor
final class TerminalViewModel {
    enum Status: Equatable {
        case idle
        case connecting
        case connected
        case reconnecting   // transport dropped; auto-reconnect in progress
        case disconnected
        case failed(String)
    }

    /// A host-key decision the user must make before the connection proceeds.
    /// Set on the main actor while the SSH handshake awaits `decide`.
    struct HostKeyPrompt: Identifiable {
        let id = UUID()
        let host: String
        let port: Int
        let fingerprint: String
        /// Non-nil when the host's key CHANGED (potential MITM) — carries the
        /// previously trusted fingerprint.
        let previousFingerprint: String?
        let decide: (Bool) -> Void
    }

    let connection: ServerConnection
    private(set) var status: Status = .idle
    var errorMessage: String?
    private(set) var session: SSHSession?
    /// Pending TOFU confirmation; the Terminal screen presents it as an alert.
    var hostKeyPrompt: HostKeyPrompt?

    @ObservationIgnored private let sshService: SSHService
    /// Chains outgoing writes so rapid-fire input (type a word, then
    /// immediately tap an arrow key) can't reorder at the transport —
    /// independently-spawned `Task`s have no FIFO guarantee relative to each
    /// other (see TmuxSessionController.enqueue's `writeChain` for the same
    /// bug, already fixed on the -CC control path).
    @ObservationIgnored private var writeChain: Task<Void, Never>?

    init(connection: ServerConnection, sshService: SSHService = .shared) {
        self.connection = connection
        self.sshService = sshService
    }

    /// Suspend the SSH handshake until the user answers the host-key prompt.
    private func promptForHostKey(host: String, port: Int,
                                  fingerprint: String,
                                  previous: String?) async -> Bool {
        await withCheckedContinuation { continuation in
            hostKeyPrompt = HostKeyPrompt(
                host: host, port: port,
                fingerprint: fingerprint,
                previousFingerprint: previous
            ) { [weak self] trusted in
                self?.hostKeyPrompt = nil
                continuation.resume(returning: trusted)
            }
        }
    }

    /// Build TOFU handlers that surface real confirmation UI: unknown hosts
    /// ask the user; CHANGED keys ask with a loud warning (default deny is the
    /// alert's cancel action). Returned fresh for each `connect()` call and
    /// passed as parameters (not installed as separate, mutable state on the
    /// shared `SSHService`) — see `SSHService.connect`'s doc comment for why
    /// that used to race concurrent sessions.
    private func hostKeyHandlers() -> (onUnknown: SSHService.TrustNewHostHandler,
                                       onChanged: SSHService.AcceptChangedHostHandler) {
        let onUnknown: SSHService.TrustNewHostHandler = { [weak self] host, port, fingerprint in
            guard let self else { return false }
            return await self.promptForHostKey(
                host: host, port: port, fingerprint: fingerprint, previous: nil)
        }
        let onChanged: SSHService.AcceptChangedHostHandler = { [weak self] host, port, newFingerprint, oldFingerprint in
            guard let self else { return false }
            return await self.promptForHostKey(
                host: host, port: port, fingerprint: newFingerprint, previous: oldFingerprint)
        }
        return (onUnknown, onChanged)
    }

    /// Flag the UI that a live transport dropped and we're auto-reconnecting.
    /// Only from a previously-live state — never interrupts an initial connect.
    func markReconnecting() {
        switch status {
        case .connected, .disconnected, .failed:
            status = .reconnecting
        default:
            break
        }
    }

    /// Allow a fresh `start()` after the transport died (app suspension,
    /// network drop). Only valid from a terminal state — never interrupts a
    /// live connection.
    func resetForReconnect() {
        switch status {
        case .connecting:
            return
        default:
            // A TOFU prompt the user never answered would otherwise leak its
            // continuation into the next attempt (and re-present a stale
            // alert). Deny it so the old handshake fails cleanly.
            hostKeyPrompt?.decide(false)
            hostKeyPrompt = nil
            session = nil
            status = .idle
            errorMessage = nil
        }
    }

    /// Opens the SSH connection and requests a PTY of the given size. Safe to
    /// call only once per view-model lifetime; subsequent calls while not
    /// `.idle` are no-ops so re-renders don't trigger duplicate connects.
    func start(rows: Int = 24, cols: Int = 80) async {
        guard case .idle = status else { return }
        status = .connecting
        do {
            let handlers = hostKeyHandlers()
            let newSession = try await sshService.connect(
                connection, onUnknownHost: handlers.onUnknown, onChangedHost: handlers.onChanged)
            try await newSession.requestPTY(rows: rows, cols: cols)
            self.session = newSession
            self.status = .connected
        } catch {
            let message: String
            if let sshError = error as? SSHError {
                message = sshError.description
            } else {
                message = error.localizedDescription
            }
            self.status = .failed(message)
            self.errorMessage = message
        }
    }

    /// Connect SSH **without** requesting a PTY, for the mosh bootstrap path
    /// (we only need an exec channel to run `mosh-server`). Sets `status` and
    /// surfaces auth/connect errors exactly like `start()`.
    func connectForExec() async -> SSHSession? {
        guard case .idle = status else { return session }
        status = .connecting
        do {
            let handlers = hostKeyHandlers()
            let newSession = try await sshService.connect(
                connection, onUnknownHost: handlers.onUnknown, onChangedHost: handlers.onChanged)
            self.session = newSession
            return newSession
        } catch {
            let message = (error as? SSHError)?.description ?? error.localizedDescription
            self.status = .failed(message)
            self.errorMessage = message
            return nil
        }
    }

    /// Mark the session live once the mosh UDP transport is up.
    func markConnected() { status = .connected }

    /// Surface a fatal mosh error through the same path as SSH failures.
    func fail(_ message: String) {
        status = .failed(message)
        errorMessage = message
    }

    /// Forwards bytes from the input bar / terminal coordinator down to the
    /// remote shell. Failures are swallowed — a closed session simply drops
    /// the write and the view will reflect the new status independently.
    func send(_ data: Data) {
        guard let session else { return }
        let previous = writeChain
        writeChain = Task { [session] in
            await previous?.value
            try? await session.write(data)
        }
    }

    /// Propagates a viewport size change to the remote PTY. As with ``send``,
    /// transient errors are ignored; persistent failures will surface via the
    /// status path when the read loop tears down.
    func resize(rows: Int, cols: Int) {
        guard let session else { return }
        Task { [session] in
            try? await session.resize(rows: rows, cols: cols)
        }
    }

    /// Tears down the session cleanly: closes the underlying SSH client (which
    /// also breaks its `dataStream`, ending whichever pump loop is reading it)
    /// and clears local session state. The pump task itself lives on
    /// `SessionHub.ActiveSession.pumpTask`, not here — this view model owns
    /// the SSH session, not the byte-pump that feeds the terminal, and
    /// `ActiveSession.stop()` cancels its own pump task before calling this.
    func disconnect() async {
        if let session {
            await session.close()
        }
        session = nil
        if case .failed = status {
            // Keep the failure visible to the user — don't overwrite with
            // "disconnected" because the alert is keyed off `errorMessage`.
        } else {
            status = .disconnected
        }
    }
}
