import Foundation
import Testing
@testable import Moshpit

/// B163: a failure that retrying can't fix waits for the user.
///
/// Every failed session used to be dialled again on every keepalive tick
/// (12 s), every foreground return and every network change. For a refused
/// host key that put the "Trust this host?" card back up every 12 s; for a
/// wrong password it sent the password to the server every 12 s — which is
/// how fail2ban bans an address. crates/session's `Failure.retryable` is the
/// rule: credentials the server turned down, a key that needs a passphrase,
/// a host key the user refused stay down until the user tries again. A
/// refused host key is the user's own decision, so it ends quietly instead
/// of failing (B151): the session is retired, and the user reconnects from a
/// new one.
@Suite("session retry policy")
@MainActor
struct SessionRetryPolicyTests {

    private func connection() -> ServerConnection {
        var connection = ServerConnection(
            name: "retry",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.multiplexer = .none   // a plain shell: the shortest start() path
        return connection
    }

    private func makeSession(_ script: DialScript) -> SessionHub.ActiveSession {
        let session = SessionHub.ActiveSession(connection: connection(), dial: script.dialer)
        session.probeTimeout = 0.2
        session.foregroundProbeTimeout = 0.2
        session.stalledDialGrace = 0
        return session
    }

    /// Every way the app tries again on its own.
    private func everyAutomaticRetry(_ session: SessionHub.ActiveSession) async {
        await session.keepAlive()
        await session.resumeIfNeeded(probeTimeout: 0.2)
        await session.forceResume()
        _ = await session.retryIfDown(stalledDialAfter: 0, reason: "test")
        await session.keepAlive()
    }

    /// The failures the server decided, or no credential to give it: what
    /// SSHService.connect throws for each. (A host key the user refused ends
    /// the connect another way — see `refusedHostKeyEndsQuietly`.)
    enum Refusal: String, CaseIterable, Sendable {
        case wrongPassword, methodRejected, passphraseNotGiven, credentialUnreadable

        var error: Error {
            switch self {
            case .wrongPassword: return SSHError.authenticationFailed
            case .methodRejected: return SSHError.authMethodRejected
            case .passphraseNotGiven: return SSHError.keyNeedsPassphrase
            case .credentialUnreadable: return SSHError.credentialUnavailable
            }
        }
    }

    @Test("a refused sign-in is never dialled again on its own", arguments: Refusal.allCases)
    func refusalsAreNotRedialled(_ refusal: Refusal) async {
        let error = refusal.error
        let script = DialScript { _ in throw error }
        let session = makeSession(script)
        await session.start(theme: .githubDark, fontSize: 14)
        guard case .failed = session.viewModel.status else {
            Issue.record("expected .failed, got \(session.viewModel.status)")
            return
        }
        #expect(!session.viewModel.failureRetryable)

        await everyAutomaticRetry(session)
        #expect(script.dials == 1, "\(refusal): dialled again with nobody asking")
        guard case .failed = session.viewModel.status else {
            Issue.record("\(refusal): expected .failed, got \(session.viewModel.status)")
            return
        }

        // The user can.
        script.behavior = { connection in SSHSession(connection: connection, transport: QuietTransport()) }
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(script.dials == 2)
        #expect(session.viewModel.status == .connected)
        await session.stop()
    }

    @Test("a failure retrying can fix is still retried on its own")
    func unreachableIsRetried() async {
        let script = DialScript { _ in throw SSHError.connectionFailed }
        let session = makeSession(script)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.failureRetryable)
        await session.keepAlive()
        #expect(script.dials == 2, "the keepalive retries a host it couldn't reach")
        script.behavior = { connection in SSHSession(connection: connection, transport: QuietTransport()) }
        await session.resumeIfNeeded(probeTimeout: 0.2)
        #expect(script.dials == 3)
        #expect(session.viewModel.status == .connected)
        await session.stop()
    }

    /// The field shape of the card: the user says no, and the card must not
    /// come back 12 s later.
    @Test("a declined Trust-this-host card does not come back on its own")
    func declinedHostKeyCardStaysDown() async throws {
        let dials = DialCounter()
        let dialer: TerminalViewModel.Dialer = { connection, prompts in
            dials.bump()
            // What SSHService.connect does with a host never seen.
            let trusted = await prompts.onUnknownHost(connection.host, connection.port, "SHA256:round22")
            guard trusted else { throw SSHError.map(HostKeyRejected()) }
            return SSHSession(connection: connection, transport: QuietTransport())
        }
        let session = SessionHub.ActiveSession(connection: connection(), dial: dialer)
        session.probeTimeout = 0.2
        session.stalledDialGrace = 0

        let first = Task { await session.start(theme: .githubDark, fontSize: 14) }
        #expect(await eventually { session.viewModel.hostKeyPrompt != nil })
        session.viewModel.hostKeyPrompt?.decide(false)
        await first.value
        // The user's own no ends the connect quietly and retires the session
        // (B151): no error card.
        #expect(session.viewModel.status == .disconnected)
        #expect(session.viewModel.declinedHostKey)
        #expect(session.viewModel.errorMessage == nil)
        #expect(session.isStopping)

        // A tick, the way back from the background, a network change. A
        // card that comes back parks the call on it: bounded, then looked at.
        _ = await withTimeoutValue(1) { await session.keepAlive() }
        _ = await withTimeoutValue(1) { await session.resumeIfNeeded(probeTimeout: 0.2) }
        _ = await withTimeoutValue(1) { await session.forceResume() }
        let cameBack = session.viewModel.hostKeyPrompt != nil
        session.viewModel.hostKeyPrompt?.decide(false)   // let a parked call go
        #expect(!cameBack, "the declined card came back on its own")
        #expect(dials.count == 1)
    }

    /// The user refused the host key: the connect ends quietly, the hub takes
    /// the session out, and nothing automatic dials the host again — the user
    /// reconnects from a new session when they choose to.
    @Test("a refused host key ends quietly and is never dialled again on its own")
    func refusedHostKeyEndsQuietly() async {
        let hub = SessionHub(metrics: SessionMetricsRegistry())
        let script = DialScript { _ in throw SSHError.map(HostKeyRejected()) }
        hub.makeSession = { connection in
            let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
            session.probeTimeout = 0.2
            session.stalledDialGrace = 0
            return session
        }
        hub.pathRedialAfter = 0
        let connection = connection()
        let session = hub.prepare(connection)
        await hub.start(session, theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.status == .disconnected)
        #expect(session.viewModel.declinedHostKey)
        #expect(session.viewModel.errorMessage == nil, "the user's own decision is not an error")
        #expect(await eventually { hub.session(for: connection) == nil }, "the hub retires it")

        await everyAutomaticRetry(session)
        await hub.kickSessionsAfterPathChange()
        await hub.resumeAll(force: false)
        await hub.resumeAll(force: true)
        #expect(script.dials == 1, "dialled again with nobody asking")

        // The user reconnects: a new session, a new dial.
        script.behavior = { connection in SSHSession(connection: connection, transport: QuietTransport()) }
        let again = hub.prepare(connection)
        #expect(again !== session)
        await hub.start(again, theme: .githubDark, fontSize: 14)
        #expect(script.dials == 2)
        #expect(again.viewModel.status == .connected)
        await hub.disconnect(connection.id)
    }

    /// Nobody will retry it, so it is news even when an automatic reconnect
    /// hit it — not "reconnecting" forever.
    @Test("an automatic attempt the server refused reads offline and says why")
    func automaticRefusalIsNews() async {
        let script = DialScript { _ in throw SSHError.authenticationFailed }
        let viewModel = TerminalViewModel(connection: connection(), dial: script.dialer)
        viewModel.beginAttempt(automatic: true)
        await viewModel.start()
        #expect(viewModel.connState == .offline)
        #expect(viewModel.errorMessage == SSHError.authenticationFailed.description)
        #expect(!viewModel.isAutoReconnectInFlight)

        // A retryable one stays inside the reconnecting screen, as before.
        let flaky = DialScript { _ in throw SSHError.connectionFailed }
        let retried = TerminalViewModel(connection: connection(), dial: flaky.dialer)
        retried.beginAttempt(automatic: true)
        await retried.start()
        #expect(retried.connState == .reconnecting)
        #expect(retried.errorMessage == nil)
    }

    @Test("the hub's path change and foreground return leave a refused session alone")
    func hubLeavesRefusalsAlone() async {
        let hub = SessionHub(metrics: SessionMetricsRegistry())
        let script = DialScript { _ in throw SSHError.authenticationFailed }
        hub.makeSession = { connection in
            let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
            session.probeTimeout = 0.2
            return session
        }
        hub.pathRedialAfter = 0
        let session = hub.prepare(connection())
        await hub.start(session, theme: .githubDark, fontSize: 14)
        await hub.kickSessionsAfterPathChange()
        await hub.resumeAll(force: false)
        await hub.resumeAll(force: true)
        #expect(script.dials == 1)
        await hub.disconnect(session.id)
    }

    @Test("the classification follows crates/session's Failure.retryable")
    func classification() {
        for error: Error in [SSHError.authenticationFailed, SSHError.authMethodRejected,
                             SSHError.keyNeedsPassphrase, SSHError.missingSecret,
                             SSHError.missingKeychainRef, SSHError.credentialUnavailable,
                             SSHError.unsupportedKeyType("x"), HostKeyRejected(),
                             SSHError.map(HostKeyRejected())] {
            #expect(!TerminalViewModel.retryable(error), "\(error)")
        }
        // A keychain the locked device keeps shut is crates/session's
        // `CredentialsLocked`: retried, unlike a lost credential (B211).
        for error: Error in [SSHError.connectionFailed, SSHError.handshakeFailed,
                             SSHError.channelOpenFailed, SSHError.hostKeysLocked,
                             SSHError.credentialLocked,
                             SSHError.proxyFailed, SSHError.proxyUnreachable("p:1080"),
                             SSHError.sessionClosed, DialFailed()] {
            #expect(TerminalViewModel.retryable(error), "\(error)")
        }
    }

    /// B513: Connect on a dead card hands Home the session before it starts
    /// again; the error its last attempt left must not come with it, or the
    /// card shows that reason over the new attempt and its OK ends it.
    @Test("a dead session prepared again comes without its last error (B513)")
    func preparedAgainWithoutTheLastError() {
        let hub = SessionHub(metrics: SessionMetricsRegistry())
        hub.makeSession = { SessionHub.ActiveSession(connection: $0) }
        let connection = connection()
        let session = hub.prepare(connection)
        session.viewModel.fail("refused", retryable: false)
        #expect(session.viewModel.errorMessage == "refused")
        let again = hub.prepare(connection)
        #expect(again === session, "the same dead session, as saved")
        #expect(again.viewModel.errorMessage == nil, "the last attempt's error came along")
    }

    private func eventually(_ timeout: Double = 2, _ predicate: @MainActor () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if predicate() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return predicate()
    }
}

/// A thread-safe tally for `@Sendable` dialers.
final class DialCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    func bump() { lock.withLock { value += 1 } }
    var count: Int { lock.withLock { value } }
}
