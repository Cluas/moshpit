import Foundation
import Testing
@testable import Moshpit

/// `exit` (or Ctrl-D) in a plain SSH shell ends the session; a connection
/// that drops is still redialled.
///
/// Both used to look the same to the app: the PTY's read loop ends. Every
/// such end was taken for a drop and redialled, so the shell the user had
/// just left came straight back as a fresh login. sshd ends a shell in
/// order — exit status, EOF, close — and that is what tells the two apart
/// (``SSHChannelEnd``). These run a real hub and real sessions over a
/// transport whose PTY the test ends either way.
@Suite("SSH shell exit")
@MainActor
struct SSHShellExitTests {

    /// An SSH connection whose PTY the test ends, the way sshd or the network
    /// would. Execs (the capability probe) answer nothing.
    final class EndableTransport: SSHClientTransport, @unchecked Sendable {
        private let lock = NSLock()
        private var onOutput: (@Sendable (Data) -> Void)?
        private var onEnd: (@Sendable (SSHChannelEnd) -> Void)?
        private var _written: [Data] = []
        private var _shutdowns = 0

        /// How often the connection was shut down.
        var shutdowns: Int { lock.withLock { _shutdowns } }

        /// What went down the PTY, one entry per write.
        var written: [String] { lock.withLock { _written.map { String(decoding: $0, as: UTF8.self) } } }

        final class Writer: SSHPTYWriter, @unchecked Sendable {
            private let host: EndableTransport
            init(host: EndableTransport) { self.host = host }
            func write(_ data: Data) async throws { host.record(data) }
            func resize(cols: Int, rows: Int) async throws {}
        }

        fileprivate func record(_ data: Data) { lock.withLock { _written.append(data) } }

        func run(_ command: String) async throws -> Data { Data() }

        func openPTY(rows: Int, cols: Int,
                     onOutput: @escaping @Sendable (Data) -> Void,
                     onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
            try await openPTY(rows: rows, cols: cols, command: nil, onOutput: onOutput, onEnd: onEnd)
        }

        private var _programs: [String?] = []
        /// Each PTY's program (nil: the login shell).
        var programs: [String?] { lock.withLock { _programs } }

        func openPTY(rows: Int, cols: Int, command: String?,
                     onOutput: @escaping @Sendable (Data) -> Void,
                     onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
            lock.withLock {
                self.onOutput = onOutput
                self.onEnd = onEnd
                _programs.append(command)
            }
            return Writer(host: self)
        }

        /// Shutting the connection ends the read loop, as Citadel's does.
        func shutdown() async {
            lock.withLock { _shutdowns += 1 }
            end(.dropped)
        }

        /// The shell prints `text`.
        func say(_ text: String) {
            lock.withLock { onOutput }?(Data(text.utf8))
        }

        /// The channel ends, once.
        func end(_ how: SSHChannelEnd) {
            let handler: (@Sendable (SSHChannelEnd) -> Void)? = lock.withLock {
                defer { onEnd = nil }
                return onEnd
            }
            handler?(how)
        }
    }

    /// Every transport the dialer handed out, in dial order.
    final class Transports: @unchecked Sendable {
        private let lock = NSLock()
        private var list: [EndableTransport] = []
        func append(_ transport: EndableTransport) { lock.withLock { list.append(transport) } }
        var last: EndableTransport? { lock.withLock { list.last } }
        var count: Int { lock.withLock { list.count } }
    }

    final class Heard: @unchecked Sendable {
        private let lock = NSLock()
        private var _ends: [SSHChannelEnd] = []
        func note(_ end: SSHChannelEnd) { lock.withLock { _ends.append(end) } }
        var ends: [SSHChannelEnd] { lock.withLock { _ends } }
    }

    private func connection(_ multiplexer: Multiplexer = .none) -> ServerConnection {
        var connection = ServerConnection(
            name: "shell-exit",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.multiplexer = multiplexer
        // A path the user vouches for: no probe can degrade it to a plain shell.
        if multiplexer == .tmux { connection.tmuxPath = "tmux" }
        return connection
    }

    /// A hub whose sessions dial a fresh ``EndableTransport`` each time.
    private func makeHub() -> (SessionHub, DialScript, Transports) {
        let transports = Transports()
        let script = DialScript { connection in
            let transport = EndableTransport()
            transports.append(transport)
            return SSHSession(connection: connection, transport: transport)
        }
        let hub = SessionHub(metrics: SessionMetricsRegistry())
        hub.makeSession = { connection in
            let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
            session.probeTimeout = 0.2
            return session
        }
        return (hub, script, transports)
    }

    private func until(_ timeout: Duration = .seconds(5),
                       _ done: @MainActor () async -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            if await done() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return await done()
    }

    @Test("`exit` in a plain SSH shell ends the session: out of the hub, the reason said, nothing redialled",
          arguments: [SSHChannelEnd.exited(status: nil), .exited(status: 1)])
    func plainShellExitEndsTheSession(_ end: SSHChannelEnd) async throws {
        let (hub, script, transports) = makeHub()
        let connection = connection()
        let session = hub.prepare(connection)
        await hub.start(session, theme: .githubDark, fontSize: 14)
        try #require(session.viewModel.status == .connected)
        let pty = try #require(transports.last)

        pty.say("logout\r\n")
        pty.end(end)

        #expect(await until { hub.session(for: connection) == nil },
                "the ended session was left in the hub, where the keepalive redials it")
        let reason = String(localized: "The session ended: the remote shell exited.")
        #expect(session.viewModel.status == .ended(reason), "an exit is not a failure")
        #expect(session.viewModel.errorMessage == reason)
        #expect(session.isStopping)
        // Whatever was going to redial has had its chance.
        try await Task.sleep(for: .milliseconds(300))
        #expect(script.dials == 1, "the shell the user left came back as a fresh login")
        #expect(transports.count == 1)
        #expect(pty.shutdowns == 1, "the ended session's SSH connection was left open")
    }

    /// The hub takes an ended session out, but nothing automatic may bring
    /// the session itself back meanwhile: the keepalive tick and a foreground
    /// return run over whatever the hub still holds, and a failed session is
    /// exactly what they redial.
    @Test("A session its shell ended stays ended: no keepalive, resume or forced resume redials it")
    func endedSessionStaysEnded() async throws {
        let transports = Transports()
        let script = DialScript { connection in
            let transport = EndableTransport()
            transports.append(transport)
            return SSHSession(connection: connection, transport: transport)
        }
        let session = SessionHub.ActiveSession(connection: connection(), dial: script.dialer)
        session.probeTimeout = 0.2
        await session.start(theme: .githubDark, fontSize: 14)
        let pty = try #require(transports.last)

        pty.end(.exited(status: nil))
        let reason = String(localized: "The session ended: the remote shell exited.")
        try #require(await until { session.viewModel.status == .ended(reason) })
        #expect(session.viewModel.connState == .offline)
        #expect(session.viewModel.endedByServer)

        await session.keepAlive()
        await session.resumeIfNeeded(probeTimeout: 0.2)
        await session.forceResume()
        session.viewModel.markReconnecting()

        #expect(script.dials == 1, "an ended session was redialled")
        #expect(session.viewModel.status == .ended(reason))
        await session.stop()
        #expect(session.viewModel.status == .ended(reason), "stopping it read as a plain disconnect")
    }

    /// The positive control: what the test above expects not to happen does
    /// happen when the connection, not the shell, went away.
    @Test("A dropped SSH connection is still redialled")
    func droppedConnectionRedials() async throws {
        let (hub, script, transports) = makeHub()
        let connection = connection()
        let session = hub.prepare(connection)
        await hub.start(session, theme: .githubDark, fontSize: 14)
        try #require(session.viewModel.status == .connected)
        let pty = try #require(transports.last)

        pty.end(.dropped)

        #expect(await until { script.dials == 2 && session.viewModel.status == .connected },
                "a dropped connection was not redialled")
        #expect(hub.session(for: connection) === session)
        #expect(session.viewModel.errorMessage == nil)
        await hub.disconnect(connection.id)
    }

    /// Over tmux the PTY's shell only carries `tmux -CC`; the sessions the
    /// user works in are tmux's and outlive it. Its end is redialled, and the
    /// redial reattaches them.
    @Test("A tmux control channel whose shell exits is redialled: the tmux sessions outlive it")
    func controlShellExitRedials() async throws {
        let (hub, script, transports) = makeHub()
        let connection = connection(.tmux)
        let session = hub.prepare(connection)
        await hub.start(session, theme: .githubDark, fontSize: 14)
        let pty = try #require(transports.last)
        try #require(await until { pty.programs.contains { $0?.contains("-CC attach") == true } },
                     "the tmux control client never started")

        pty.end(.exited(status: nil))

        #expect(await until { script.dials == 2 }, "the tmux control channel was not redialled")
        #expect(hub.session(for: connection) === session)
        await hub.disconnect(connection.id)
    }

    /// The PTY is up before the hub gets to register its handler, and a login
    /// shell can exit at once (`nologin`, a profile that exits): that end was
    /// lost, and the screen sat on a dead shell reading connected.
    @Test("A channel that ended before anyone listened is still reported")
    func endBeforeTheHandlerIsReported() async throws {
        let transport = EndableTransport()
        let ssh = SSHSession(connection: connection(), transport: transport)
        try await ssh.requestPTY(rows: 24, cols: 80)
        transport.end(.exited(status: 1))
        try #require(await until { !(await ssh.isConnected) })

        let heard = Heard()
        await ssh.setOnUnexpectedClose { heard.note($0) }

        #expect(heard.ends == [.exited(status: 1)])
    }

    /// sshd keeps the connection open after the shell exits: closing it is
    /// the client's job. `close()` used to return early once the channel had
    /// ended on its own, so every session whose shell exited left its
    /// connection established for as long as the app ran.
    @Test("A session whose shell exited still closes its connection, once")
    func exitedSessionClosesItsConnection() async throws {
        let transport = EndableTransport()
        let ssh = SSHSession(connection: connection(), transport: transport)
        try await ssh.requestPTY(rows: 24, cols: 80)
        transport.end(.exited(status: nil))
        try #require(await until { !(await ssh.isConnected) })

        await ssh.close()
        await ssh.close()

        #expect(transport.shutdowns == 1, "the connection under an exited shell was left open")
    }

    @Test("Our own close is nobody's news")
    func closeIsNotReported() async throws {
        let transport = EndableTransport()
        let ssh = SSHSession(connection: connection(), transport: transport)
        try await ssh.requestPTY(rows: 24, cols: 80)
        let heard = Heard()
        await ssh.setOnUnexpectedClose { heard.note($0) }

        await ssh.close()
        try await Task.sleep(for: .milliseconds(50))

        #expect(heard.ends.isEmpty)
    }

    /// Citadel finishes the PTY's output stream cleanly when sshd closes the
    /// channel in order, throws `CommandFailed` for a non-zero exit status,
    /// and throws what took the channel down otherwise.
    @Test("A read loop that finished without an error is an exit; one that failed is a drop")
    func readLoopEndsAreClassified() {
        struct Reset: Error {}
        #expect(CitadelClientTransport.channelEnd(after: nil) == .exited(status: nil))
        #expect(CitadelClientTransport.channelEnd(after: Reset()) == .dropped)
        #expect(CitadelClientTransport.channelEnd(after: SSHError.sessionClosed) == .dropped)
    }
}
