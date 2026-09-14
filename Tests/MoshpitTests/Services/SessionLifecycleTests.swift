import Foundation
import Network
import Testing
@testable import Moshpit

/// Lifecycle orchestration under scripted transports — the layer where this
/// week's field bugs all lived (wedged probes, sidecars that never rebuilt,
/// guards poisoned for the rest of a session). A REAL `SSHSession` runs over
/// a fake `SSHClientTransport`, and a REAL `ActiveSession` believes it is a
/// live mosh connection via `installForTesting` — no network anywhere, so
/// "half-open socket after a long suspension" is a deterministic scenario
/// instead of a device report.
@Suite("session lifecycle")
@MainActor
struct SessionLifecycleTests {

    // MARK: - Scripted SSH transport

    /// Per-command behaviors, recorded calls, controllable PTY.
    final class ScriptedTransport: SSHClientTransport, @unchecked Sendable {
        enum Behavior {
            case reply(String)
            case fail
            /// The half-open-socket shape: never completes, never observes
            /// cancellation — an exec bridged off a NIO future waiting for a
            /// reply that will never come.
            case hangIgnoringCancellation
        }

        private let lock = NSLock()
        private var _commands: [String] = []
        private var behavior: Behavior

        init(_ behavior: Behavior = .reply("")) { self.behavior = behavior }

        var commands: [String] { lock.withLock { _commands } }

        func run(_ command: String) async throws -> Data {
            let b = lock.withLock { _commands.append(command); return behavior }
            switch b {
            case .reply(let text):
                return Data(text.utf8)
            case .fail:
                throw SSHError.sessionClosed
            case .hangIgnoringCancellation:
                return await withCheckedContinuation { (_: CheckedContinuation<Data, Never>) in }
            }
        }

        final class RecordingWriter: SSHPTYWriter, @unchecked Sendable {
            private let lock = NSLock()
            private var _written: [Data] = []
            var written: [Data] { lock.withLock { _written } }
            func write(_ data: Data) async throws { lock.withLock { _written.append(data) } }
            func resize(cols: Int, rows: Int) async throws {}
        }

        let writer = RecordingWriter()

        func openPTY(rows: Int, cols: Int,
                     onOutput: @escaping @Sendable (Data) -> Void,
                     onEnd: @escaping @Sendable () -> Void) async throws -> any SSHPTYWriter {
            writer
        }

        func shutdown() async {}
    }

    /// A UDP channel that connects and then stays silent — enough for an
    /// ActiveSession to hold a "live" mosh transport without sockets.
    final class IdleDatagramChannel: DatagramChannel, @unchecked Sendable {
        var state: NWConnection.State = .ready
        var onStateChange: (@Sendable (NWConnection.State) -> Void)?
        var onBetterPath: (@Sendable (Bool) -> Void)?
        func start() {}
        func restart() {}
        func cancel() {}
        func send(_ datagram: Data) {}
        func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void) {}
    }

    /// Thread-safe call counter for @Sendable connector closures.
    final class Counter: @unchecked Sendable {
        private let lock = NSLock()
        private var value = 0
        func bump() { lock.withLock { value += 1 } }
        var count: Int { lock.withLock { value } }
    }

    // MARK: - Fixtures

    private func makeSession(_ transport: ScriptedTransport,
                             moshControl: TmuxSessionController? = nil) -> SessionHub.ActiveSession {
        var connection = ServerConnection(
            name: "lifecycle",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — guaranteed non-routable
            port: 22,
            username: "tester",
            authMethod: .password)
        // The rebuild branches are per-multiplexer; a bare connection is
        // `.none` and resumeIfNeeded would have nothing to rebuild.
        connection.multiplexer = .tmux
        let session = SessionHub.ActiveSession(connection: connection)
        session.probeTimeout = 0.2
        let key = Data((0..<16).map { UInt8($0) })
        let mosh = try! MoshTransport(
            credentials: MoshCredentials(host: "192.0.2.1", udpPort: 60001, key: key),
            channel: IdleDatagramChannel())
        let sidecar = SSHSession(connection: connection, transport: transport)
        session.installForTesting(moshTransport: mosh, sidecarSSH: sidecar, moshControl: moshControl)
        return session
    }

    /// Wire a connector that counts dials and hands back a session over the
    /// given transport behavior.
    private func wireConnector(_ session: SessionHub.ActiveSession,
                               replacement: ScriptedTransport.Behavior) -> Counter {
        let counter = Counter()
        session.sidecarConnect = { connection in
            counter.bump()
            return SSHSession(connection: connection, transport: ScriptedTransport(replacement))
        }
        return counter
    }

    /// A mosh session that BELIEVES it has a server pid to reap, with a
    /// caller-chosen first-contact state — the two inputs `stop()`'s
    /// fallback reap branches on.
    private func makeMoshSession(pid: Int?, hadFirstContact: Bool) -> SessionHub.ActiveSession {
        let connection = ServerConnection(
            name: "lifecycle-mosh",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — guaranteed non-routable
            port: 22,
            username: "tester",
            authMethod: .password)
        let session = SessionHub.ActiveSession(connection: connection)
        let key = Data((0..<16).map { UInt8($0) })
        let mosh = try! MoshTransport(
            credentials: MoshCredentials(host: "192.0.2.1", udpPort: 60001, key: key),
            channel: IdleDatagramChannel())
        session.installForTesting(moshTransport: mosh,
                                  moshServerPid: pid,
                                  moshHadFirstContact: hadFirstContact)
        return session
    }

    /// Bound a lifecycle call so a regression to the wedge bug fails the
    /// test instead of hanging the suite.
    private func bounded(_ seconds: Double = 5,
                         _ op: @escaping @Sendable () async -> Void) async -> Bool {
        await withTimeoutValue(seconds) { await op(); return true } ?? false
    }

    // MARK: - The wedge regression

    @Test("A hung sidecar probe cannot wedge recovery: resume returns and stays re-entrant")
    func hungProbeDoesNotWedge() async {
        let session = makeSession(ScriptedTransport(.hangIgnoringCancellation))
        // Replacement sidecars hang too — a host that stays broken.
        let connects = wireConnector(session, replacement: .hangIgnoringCancellation)

        // First resume: probe times out (0.2s), rebuild dials a fresh
        // sidecar. The call itself must come back — the old implementation
        // parked here forever on the hung exec.
        #expect(await bounded { await session.resumeIfNeeded() },
                "resumeIfNeeded must return once the probe deadline fires")
        #expect(connects.count == 1)

        // Second resume: the guard was RELEASED (isResuming not poisoned) —
        // the exact regression that kept sidecars dead for whole sessions.
        #expect(await bounded { await session.resumeIfNeeded() },
                "a second resume must run — a wedged guard blocks it forever")
        #expect(connects.count == 2)
    }

    @Test("A dead sidecar (probe errors) is rebuilt through the connector")
    func deadSidecarRebuilds() async {
        let session = makeSession(ScriptedTransport(.fail))
        let connects = wireConnector(session, replacement: .reply(""))

        #expect(await bounded { await session.resumeIfNeeded() })
        #expect(connects.count == 1, "an erroring probe must trigger the sidecar rebuild")
    }

    @Test("A live sidecar without a control plane is still retried every cycle")
    func liveSidecarWithoutControllerRetries() async {
        // Documented behavior, not an accident: `moshControl == nil` retries
        // on every keepalive tick — a failed -CC attach would otherwise
        // leave the breadcrumb blank until app restart.
        let session = makeSession(ScriptedTransport(.reply("")))
        let connects = wireConnector(session, replacement: .reply(""))

        #expect(await bounded { await session.resumeIfNeeded() })
        #expect(connects.count == 1,
                "a healthy socket with no controller must still rebuild toward an attach")
    }

    @Test("keepAlive funnels a mosh session through the same recovery path")
    func keepAliveFunnelsThroughResume() async {
        let session = makeSession(ScriptedTransport(.fail))
        let connects = wireConnector(session, replacement: .reply(""))

        #expect(await bounded { await session.keepAlive() })
        #expect(connects.count == 1, "keepAlive on mosh must reach the sidecar rebuild")
    }

    // MARK: - Liveness probing

    /// A dead socket and a slow one look identical to a boolean probe, and
    /// treating them alike is what tore down working sessions on a lossy link:
    /// the reconnect that follows costs a full re-handshake, so a false
    /// positive is far more expensive than a late true one.
    @Test("The probe tells a refusal apart from a silence")
    func probeDistinguishesFailureFromTimeout() async {
        func session(_ behavior: ScriptedTransport.Behavior) -> SSHSession {
            let connection = ServerConnection(name: "probe", host: "192.0.2.1", port: 22,
                                              username: "tester", authMethod: .password)
            return SSHSession(connection: connection, transport: ScriptedTransport(behavior))
        }
        #expect(await SessionHub.ActiveSession.probe(session(.reply("")), timeout: 0.2) == .alive)
        #expect(await SessionHub.ActiveSession.probe(session(.fail), timeout: 0.2) == .failed)
        #expect(await SessionHub.ActiveSession.probe(session(.hangIgnoringCancellation),
                                                     timeout: 0.2) == .timedOut)
    }

    /// Traffic is better evidence than a probe, and free. A connection that
    /// just delivered bytes must not be asked to prove itself — that costs a
    /// fresh exec channel every tick on exactly the links least able to
    /// spare one.
    @Test("Recent inbound traffic counts as proof of life")
    func inboundTrafficIsProofOfLife() async {
        let session = makeSession(ScriptedTransport(.reply("")))
        #expect(session.inboundAge() > 60, "a session that never read anything is stale")
        session.noteInbound()
        #expect(session.inboundAge() < 1)
    }

    // MARK: - stop()'s mosh-server reap fallback

    /// The gap `onReturnPathDead`'s watchdog can't cover: it only fires a few
    /// seconds after bootstrap, so a user who backs out (or kills the app)
    /// before then leaves a mosh-server waiting forever for a client that's
    /// never coming (7 zombies on one host, 2026-08-19). `stop()` is the
    /// backstop — this pins it to a pid that never got a reply.
    @Test("stop() reaps a mosh-server pid that never made first contact")
    func stopReapsUncontactedMoshServer() async {
        let session = makeMoshSession(pid: 4242, hadFirstContact: false)
        let reapTransport = ScriptedTransport(.reply(""))
        session.sidecarConnect = { connection in
            SSHSession(connection: connection, transport: reapTransport)
        }

        #expect(await bounded { await session.stop() },
                "stop() must return promptly even while reaping — the timeout exists precisely so a dark host can't hang teardown")
        #expect(reapTransport.commands.contains { $0.contains("kill 4242") },
                "a mosh-server that never got a reply back must be killed on teardown")
    }

    /// The critical trap this whole feature has to avoid: a session that
    /// really did talk to its server must NEVER be killed just because
    /// `stop()` happens to run — mosh's entire point is that the server
    /// outlives a torn-down client so a later reconnect can resume it.
    @Test("stop() never kills a mosh-server that already made first contact")
    func stopLeavesContactedMoshServerAlone() async {
        let session = makeMoshSession(pid: 4242, hadFirstContact: true)
        let connects = wireConnector(session, replacement: .reply(""))

        #expect(await bounded { await session.stop() })
        // swiftlint:disable:next empty_count - `Counter.count` is a tally, not a collection
        #expect(connects.count == 0,
                "first contact must latch stop() off the reap path entirely — no sidecar should even be dialed")
    }

    // MARK: - Reconnect teardown

    /// The measured cause of "reconnect is slow": a half-open socket held the
    /// courtesy unpin exec for Citadel's full 15s channel-open timeout. The
    /// reconnect teardown must not spend a single round trip on the transport
    /// it is replacing — and the pins it owed must travel to the replacement.
    @Test("stop(forReconnect:) issues no exec on the dying transport and carries the pin ledger")
    func reconnectTeardownSkipsCourtesyExecs() async {
        let transport = ScriptedTransport(.hangIgnoringCancellation)
        let control = TmuxSessionController(sshSession: MockTmuxTransport(), rendersOutput: false)
        control.inheritPinLedger(TmuxSessionController.PinLedger(resizedWindows: ["@3"]))
        let session = makeSession(transport, moshControl: control)

        #expect(await bounded(1) { await session.stop(forReconnect: true) },
                "a reconnect teardown must return at once — the old transport is already judged dead")
        #expect(transport.commands.isEmpty,
                "no unpin / layout / status exec may be spent on the transport being replaced")
        #expect(session.inheritedPinLedger?.pinnedWindows == ["@3"],
                "the retired controller's pins travel to the replacement instead of being paid now")
        #expect(control.resizedWindows.isEmpty)
    }

    /// The full teardown still pays the debt — but bounded, so a socket that
    /// died between the decision to disconnect and this line cannot hold the
    /// teardown for a channel timeout per command.
    @Test("stop() pays the pin debt over the sidecar, bounded as a whole")
    func fullTeardownCourtesyIsBounded() async {
        let transport = ScriptedTransport(.hangIgnoringCancellation)
        let control = TmuxSessionController(sshSession: MockTmuxTransport(), rendersOutput: false)
        control.inheritPinLedger(TmuxSessionController.PinLedger(resizedWindows: ["@3", "@5"]))
        let session = makeSession(transport, moshControl: control)

        let began = Date()
        #expect(await bounded(6) { await session.stop() })
        let took = Date().timeIntervalSince(began)
        #expect(took < 5, "two hung execs must cost one bound (3s), not one channel timeout each — took \(took)s")
        #expect(transport.commands.contains { $0.contains("set-option -u -w -t @3 window-size") },
                "the unpin is still attempted on a real disconnect")
        #expect(session.inheritedPinLedger == nil)
    }

    /// The foreground-return check waits less for silence than the periodic
    /// tick: the person is watching, and a socket iOS held through a
    /// suspension is dead far more often than not.
    @Test("resumeIfNeeded honours a shorter probe deadline for the foreground return")
    func foregroundProbeDeadlineIsHonoured() async {
        let session = makeSession(ScriptedTransport(.hangIgnoringCancellation))
        session.probeTimeout = 5   // the periodic deadline — must NOT be what applies here
        let connects = wireConnector(session, replacement: .reply(""))

        // The rebuild dials the connector the moment the probe gives up, so
        // "when was the sidecar re-dialed" is the probe deadline made visible.
        let began = Date()
        let resume = Task { await session.resumeIfNeeded(probeTimeout: 0.2) }
        let deadline = Date().addingTimeInterval(2)
        // swiftlint:disable:next empty_count - `Counter.count` is a tally, not a collection
        while connects.count == 0, Date() < deadline {
            try? await Task.sleep(for: .milliseconds(20))
        }
        #expect(connects.count >= 1, "a probe that timed out at the deadline still rebuilds the sidecar")
        #expect(Date().timeIntervalSince(began) < 2,
                "a hung sidecar must be given up on at the foreground deadline, not the periodic one")
        _ = await bounded(10) { await resume.value }
    }

    /// While the replacement dials and attaches, the screen keeps showing the
    /// dropped connection's last frame — but only a frame with content on it,
    /// and never past a real disconnect.
    @Test("A reconnect retires a painted tmux controller as the ghost; a full stop drops it")
    func reconnectRetiresPaintedController() async {
        let session = makeSession(ScriptedTransport(.reply("")))
        let unpainted = TmuxSessionController(sshSession: MockTmuxTransport())
        session.installForTesting(moshTransport: nil, tmuxController: unpainted)
        #expect(await bounded(1) { await session.stop(forReconnect: true) })
        #expect(session.retiredTmuxController == nil,
                "a controller that never revealed a frame has nothing worth keeping on screen")
        #expect(session.tmuxController == nil)

        let painted = TmuxSessionController(sshSession: MockTmuxTransport())
        _ = painted.terminalView(for: "%0")
        painted.coordinator(for: "%0")?.reveal()
        #expect(painted.hasPaintedSinceAttach)
        session.installForTesting(moshTransport: nil, tmuxController: painted)
        #expect(await bounded(1) { await session.stop(forReconnect: true) })
        #expect(session.retiredTmuxController === painted)
        #expect(session.awaitingFirstFrame == false)

        // A second drop before the replacement painted keeps the OLDER ghost.
        let replacement = TmuxSessionController(sshSession: MockTmuxTransport())
        _ = replacement.terminalView(for: "%0")
        replacement.coordinator(for: "%0")?.reveal()
        session.installForTesting(moshTransport: nil, tmuxController: replacement)
        #expect(await bounded(1) { await session.stop(forReconnect: true) })
        #expect(session.retiredTmuxController === painted)

        #expect(await bounded(1) { await session.stop() })
        #expect(session.retiredTmuxController == nil, "a real disconnect has nothing to stand in for")
    }
}
