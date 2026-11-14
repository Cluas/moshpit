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
        /// The remote end finished the session for good — its shell exited,
        /// or mosh-server sent its shutdown — with what to say about it. Not
        /// a failure, and nothing redials it: reconnecting is the user's call.
        case ended(String)
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

    /// The passphrase of a key stored behind one (``StoredKey``), asked for
    /// once: the connect converts the key and keeps it that way. Set on the
    /// main actor while the connect awaits `decide`; nil = cancel.
    struct KeyPassphrasePrompt: Identifiable {
        let id = UUID()
        let host: String
        /// The last passphrase given didn't unlock the key.
        let wrong: Bool
        let decide: (String?) -> Void
    }

    /// What a connect may have to ask the user, through this view model's
    /// cards.
    struct ConnectPrompts: Sendable {
        let onUnknownHost: SSHService.TrustNewHostHandler
        let onChangedHost: SSHService.AcceptChangedHostHandler
        let onKeyPassphrase: SSHService.KeyPassphraseHandler
    }

    let connection: ServerConnection
    private(set) var status: Status = .idle
    var errorMessage: String?
    /// The last connect ended because the user said no to the server's key
    /// (B151): the new-host card's Cancel, the changed-key card's Disconnect.
    /// Their own decision, not a failure — no error card; the screen leaves
    /// and the hub retires the session the way Disconnect does.
    private(set) var declinedHostKey = false

    /// What every surface showing this session's health should read.
    ///
    /// Lives here because it used to live in three places — the Terminal
    /// screen, the transport pill and the home card's row each derived it, and
    /// each derived it differently, so one reconnect could be blue on one
    /// surface, red on another and amber on a third. `status` alone is not
    /// enough to derive it (see the collapse below), which is exactly why
    /// duplicating the derivation went wrong.
    var connState: TransportConnState {
        // One state for the whole automatic reconnect, however many attempts it
        // takes: `status` flips connecting → failed → connecting on every
        // keepalive tick, and letting that through made the screen alternate
        // between "opening the pit" and "line dropped".
        //
        // It collapses to `.reconnecting` and not `.offline` because the app is
        // dialling — `.offline` means nothing is being attempted. Reporting
        // offline here turned the connecting screen red mid-reconnect, and since
        // this flag and `status` change on their own schedules, a single
        // reconnect crossed from blue to red partway through.
        if isAutoReconnectInFlight { return .reconnecting }
        switch status {
        case .connected: return .live
        case .reconnecting: return .reconnecting
        case .connecting, .idle: return .connecting
        case .failed, .disconnected, .ended: return .offline
        }
    }

    /// The remote end finished this session (``Status/ended(_:)``) — the
    /// terminal names it for what it is, not a dropped line or an error.
    var endedByServer: Bool {
        if case .ended = status { return true }
        return false
    }

    /// True from the moment the app starts re-establishing a transport the user
    /// didn't ask to re-establish, until it's back.
    ///
    /// A failure the user asked for is news — they tapped connect and it didn't
    /// happen, so a modal saying why is right. A failure inside an automatic
    /// retry is not news: the hub is already trying again on the next keepalive
    /// tick, and the connecting screen has copy for exactly this. Surfacing both
    /// meant a dropped line showed a modal error card over the terminal, then
    /// the card vanished and "opening the pit" took its place, then the card
    /// came back — two things saying the same thing, in a loop, neither of them
    /// dismissable in any way that helped.
    ///
    /// Set by ``SessionHub/ActiveSession/start(theme:fontSize:fontName:cursorShape:cursorColorId:cursorBlink:automatic:)``,
    /// cleared here the moment a transport comes up.
    private(set) var isAutoReconnectInFlight = false

    /// Whether the failure in ``status`` is one trying again can fix with
    /// nobody doing anything (crates/session's `Failure.retryable`): a
    /// network that comes back, a device that gets unlocked. A host key the
    /// user refused, credentials the server turned down, a key that needs a
    /// passphrase nobody gave are not — and the keepalive, the foreground
    /// return and a network change used to dial them again anyway: a refused
    /// "Trust this host?" card came back every 12 s, and a wrong password went
    /// to the server every 12 s, which is how an address gets banned (B163).
    /// Those wait for the user to try again. See ``retryable(_:)``.
    private(set) var failureRetryable = true

    /// The failure in ``status`` is the device being locked: the keychain
    /// would not give the saved credential, or the trusted host keys, yet
    /// (crates/session's `Failure.waits_for_unlock`). The unlock itself
    /// dials it again (``SessionHub/deviceUnlocked()``), not only the next
    /// keepalive tick or foreground return (B211). See ``waitsForUnlock(_:)``.
    private(set) var failureWaitsForUnlock = false

    private(set) var session: SSHSession?
    /// A connection dialled for ``start(rows:cols:program:)`` whose terminal
    /// isn't open yet (its program is being decided): ``disconnect()``
    /// closes it too, so what waits on it — the capability probe — ends.
    @ObservationIgnored private var opening: SSHSession?
    /// Pending TOFU confirmation; the Terminal screen presents it as an alert.
    var hostKeyPrompt: HostKeyPrompt?
    /// Pending key passphrase; presented as a card beside `hostKeyPrompt`.
    var keyPassphrasePrompt: KeyPassphrasePrompt?

    @ObservationIgnored private let sshService: SSHService
    /// How the primary transport is dialed. Production goes through
    /// `sshService.connect`; tests inject a dialer that hands back a session
    /// over a scripted transport — or hangs, which is what a SYN into a
    /// black hole looks like from here.
    typealias Dialer = @Sendable (ServerConnection, ConnectPrompts) async throws -> SSHSession
    @ObservationIgnored private let dial: Dialer
    /// Which dial the in-flight `start()` / `connectForExec()` belongs to.
    /// `disconnect()` while a dial is still out bumps this, and that dial's
    /// eventual verdict — success or failure — is then nobody's business:
    /// the session it returns is closed, the error it throws is dropped.
    /// Without it a dial parked on a SYN nobody answers (a VPN address with
    /// the tunnel down: 30s to the connect timeout) landed on top of the
    /// redial that had replaced it — as a stale `.failed`, or as a second
    /// session adopted over the live one.
    @ObservationIgnored private var dialGeneration = 0
    /// Chains outgoing writes so rapid-fire input (type a word, then
    /// immediately tap an arrow key) can't reorder at the transport —
    /// independently-spawned `Task`s have no FIFO guarantee relative to each
    /// other (see TmuxSessionController.enqueue's `writeChain` for the same
    /// bug, already fixed on the -CC control path).
    @ObservationIgnored private var writeChain: Task<Void, Never>?

    init(connection: ServerConnection, sshService: SSHService = .shared, dial: Dialer? = nil) {
        self.connection = connection
        self.sshService = sshService
        if let dial {
            self.dial = dial
        } else {
            self.dial = { connection, prompts in
                try await sshService.connect(connection,
                                             onUnknownHost: prompts.onUnknownHost,
                                             onChangedHost: prompts.onChangedHost,
                                             onKeyPassphrase: prompts.onKeyPassphrase)
            }
        }
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

    /// Suspend the connect until the user gives the key's passphrase, or
    /// cancels (nil).
    private func promptForKeyPassphrase(host: String, wrong: Bool) async -> String? {
        await withCheckedContinuation { continuation in
            keyPassphrasePrompt = KeyPassphrasePrompt(host: host, wrong: wrong) { [weak self] passphrase in
                self?.keyPassphrasePrompt = nil
                continuation.resume(returning: passphrase)
            }
        }
    }

    /// Build TOFU handlers that surface real confirmation UI: unknown hosts
    /// ask the user; CHANGED keys ask with a loud warning (default deny is the
    /// alert's cancel action). Returned fresh for each `connect()` call and
    /// passed as parameters (not installed as separate, mutable state on the
    /// shared `SSHService`) — see `SSHService.connect`'s doc comment for why
    /// that used to race concurrent sessions. The key passphrase is asked the
    /// same way.
    private func connectPrompts() -> ConnectPrompts {
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
        let onPassphrase: SSHService.KeyPassphraseHandler = { [weak self] connection, wrong in
            guard let self else { return nil }
            return await self.promptForKeyPassphrase(host: connection.host, wrong: wrong)
        }
        return ConnectPrompts(onUnknownHost: onUnknown, onChangedHost: onChanged, onKeyPassphrase: onPassphrase)
    }

    /// Flag the UI that a live transport dropped and we're auto-reconnecting.
    /// Only from a previously-live state — never interrupts an initial
    /// connect, and never a session the server ended (`.ended`).
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
            keyPassphrasePrompt?.decide(nil)
            keyPassphrasePrompt = nil
            session = nil
            status = .idle
            errorMessage = nil
            declinedHostKey = false
            failureRetryable = true
            failureWaitsForUnlock = false
        }
    }

    /// Declare who asked for the attempt about to run — see
    /// ``isAutoReconnectInFlight``. Call once, before `start`/`connectForExec`.
    func beginAttempt(automatic: Bool) {
        isAutoReconnectInFlight = automatic
    }

    /// Opens the SSH connection and requests a PTY of the given size. Safe to
    /// call only once per view-model lifetime; subsequent calls while not
    /// `.idle` are no-ops so re-renders don't trigger duplicate connects.
    ///
    /// `program`, asked once the connection is up and before the terminal
    /// opens, names the terminal's own program (tmux's control client,
    /// herdr's frame loop), nil for the login shell: what the app starts on
    /// the terminal is never a line typed into a shell that may still be
    /// starting (B267).
    func start(rows: Int = 24, cols: Int = 80,
               program: (SSHSession) async -> String? = { _ in nil }) async {
        guard case .idle = status else { return }
        status = .connecting
        dialGeneration += 1
        let generation = dialGeneration
        do {
            let newSession = try await dial(connection, connectPrompts())
            guard generation == dialGeneration else { await newSession.close(); return }
            // Closed by a disconnect meanwhile (`opening`): the program's
            // decision — a probe on this connection — ends with it.
            opening = newSession
            let command = await program(newSession)
            if opening === newSession { opening = nil }
            guard generation == dialGeneration else { await newSession.close(); return }
            try await newSession.requestPTY(rows: rows, cols: cols, command: command)
            guard generation == dialGeneration else { await newSession.close(); return }
            self.session = newSession
            self.status = .connected
            self.isAutoReconnectInFlight = false
        } catch {
            // A superseded dial's failure is not this attempt's failure.
            guard generation == dialGeneration else { return }
            if declined(error) { return }
            let message: String
            if let sshError = error as? SSHError {
                message = sshError.description
            } else {
                message = error.localizedDescription
            }
            fail(message, retryable: Self.retryable(error), waitsForUnlock: Self.waitsForUnlock(error))
        }
    }

    /// Connect SSH **without** requesting a PTY, for the mosh bootstrap path
    /// (we only need an exec channel to run `mosh-server`). Sets `status` and
    /// surfaces auth/connect errors exactly like `start()`.
    func connectForExec() async -> SSHSession? {
        guard case .idle = status else { return session }
        status = .connecting
        dialGeneration += 1
        let generation = dialGeneration
        do {
            let newSession = try await dial(connection, connectPrompts())
            guard generation == dialGeneration else { await newSession.close(); return nil }
            self.session = newSession
            return newSession
        } catch {
            guard generation == dialGeneration else { return nil }
            if declined(error) { return nil }
            let message = (error as? SSHError)?.description ?? error.localizedDescription
            fail(message, retryable: Self.retryable(error), waitsForUnlock: Self.waitsForUnlock(error))
            return nil
        }
    }

    /// End the connect quietly if `error` is the user's own no to the
    /// server's key (``declinedHostKey``). It used to reach the error card as
    /// "The connection failed for a reason Moshpit didn't recognise…".
    private func declined(_ error: Error) -> Bool {
        guard case .hostKeyRejected? = error as? SSHError else { return false }
        status = .disconnected
        isAutoReconnectInFlight = false
        errorMessage = nil
        declinedHostKey = true
        return true
    }

    /// Mark the session live once the mosh UDP transport is up.
    func markConnected() {
        status = .connected
        isAutoReconnectInFlight = false
    }

    /// The attempt failed: the status carries the reason either way, and only
    /// a connect the user asked for earns a modal on top of it — unless
    /// nothing will try this one again on its own (`retryable` false, see
    /// ``failureRetryable``): then it is news whoever started the attempt,
    /// and the surfaces read offline rather than "reconnecting" forever.
    /// Also how a fatal mosh error is surfaced, the same way as SSH's.
    /// `waitsForUnlock`: the device's unlock may fix it (``failureWaitsForUnlock``).
    func fail(_ message: String, retryable: Bool = true, waitsForUnlock: Bool = false) {
        failureRetryable = retryable
        failureWaitsForUnlock = retryable && waitsForUnlock
        if !retryable { isAutoReconnectInFlight = false }
        status = .failed(message)
        if !isAutoReconnectInFlight { errorMessage = message }
    }

    /// Whether trying `error`'s connect again, with nobody doing anything,
    /// can succeed — see ``failureRetryable``. The server refusing the
    /// credentials, a key that can't sign in as it is, nothing to sign in
    /// with, a host key the user refused: no. Everything about reaching the
    /// server, and a keychain the locked device keeps shut: yes. Exhaustive
    /// on purpose: a new ``SSHError`` case is judged here, never defaulted to
    /// "try again".
    nonisolated static func retryable(_ error: Error) -> Bool {
        if error is HostKeyRejected { return false }
        switch SSHError.map(error) {
        case .authenticationFailed, .authMethodRejected,
             .keyNeedsPassphrase, .keyUnusable, .unsupportedKeyType,
             .missingKeychainRef, .missingSecret, .credentialUnavailable,
             .credentialCancelled, .hostKeyRejected:
            return false
        case .underlying(let underlying):
            // A refused host key that reached here wrapped rather than as
            // `.hostKeyRejected` (which `start()` ends quietly, B151).
            return !(underlying is HostKeyRejected)
        case .ptyAlreadyOpen, .ptyNotReady, .sessionClosed, .channelOpenFailed,
             .connectionFailed, .handshakeFailed, .hostKeysLocked,
             .credentialLocked, .proxyUnreachable, .proxyFailed:
            return true
        }
    }

    /// Whether `error` is the locked device's: the saved credential
    /// (``SSHError/credentialLocked``) or the trusted host keys
    /// (``SSHError/hostKeysLocked``) can't be read until it is unlocked —
    /// crates/session's `Failure.waits_for_unlock` (B201, B211).
    nonisolated static func waitsForUnlock(_ error: Error) -> Bool {
        switch SSHError.map(error) {
        case .credentialLocked, .hostKeysLocked: return true
        default: return false
        }
    }

    /// The remote end finished the session for good (``Status/ended(_:)``):
    /// said whoever started the last attempt, since nothing is retrying it.
    func end(_ message: String) {
        status = .ended(message)
        isAutoReconnectInFlight = false
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

    /// ``send(_:)``, waited for: whether the bytes went out on the session —
    /// in order with everything typed around them (the same `writeChain`).
    /// For a delivery its sender keeps until it lands (the share queue, B172).
    func deliver(_ data: Data) async -> Bool {
        guard let session else { return false }
        let previous = writeChain
        let write = Task { [session] () -> Bool in
            await previous?.value
            do {
                try await session.write(data)
                return true
            } catch {
                return false
            }
        }
        writeChain = Task { _ = await write.value }
        return await write.value
    }

    /// Propagates a viewport size change to the remote PTY, on `writeChain`
    /// with the input around it. Each resize used to be a Task of its own: a
    /// rotation or the keyboard animating in reports several sizes in a burst,
    /// and when an earlier window-change took longer than a later one the PTY
    /// ended at the older size. In order, the last size reported is the one
    /// it ends at. As with ``send``, transient errors are ignored; persistent
    /// failures will surface via the status path when the read loop tears down.
    func resize(rows: Int, cols: Int) {
        guard let session else { return }
        let previous = writeChain
        writeChain = Task { [session] in
            await previous?.value
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
        // Disown a dial still in flight — see `dialGeneration`.
        dialGeneration += 1
        if let opening {
            self.opening = nil
            await opening.close()
        }
        if let session {
            await session.close()
        }
        session = nil
        switch status {
        case .failed, .ended:
            // Keep the failure (or the end) visible to the user — don't
            // overwrite with "disconnected" because the alert is keyed off
            // `errorMessage`.
            break
        default:
            status = .disconnected
        }
    }
}

// MARK: - A session on the Rust core (W14a)

extension TerminalViewModel {
    /// The Rust core's session state, mirrored where every surface reads a
    /// session's health (``RustSessionDriver``): the core dials and redials,
    /// this only says where it stands. `automatic`: nobody asked for what is
    /// happening (a redial) — the collapse into `.reconnecting` and the no-modal
    /// rule of ``isAutoReconnectInFlight``. `error` raises the modal card.
    func adoptRustStatus(_ status: Status, automatic: Bool, error: String? = nil) {
        self.status = status
        isAutoReconnectInFlight = automatic
        if let error { errorMessage = error }
    }

    /// The person's own no to the server's key, on the Rust core: the
    /// connect ends quietly, as ``declined(_:)`` ends it on the Swift path
    /// (B151, B334) — disconnected, not failed, no card, and
    /// ``declinedHostKey`` for the terminal to leave on.
    func adoptRustDeclined() {
        status = .disconnected
        isAutoReconnectInFlight = false
        errorMessage = nil
        declinedHostKey = true
    }
}
