import Foundation
import Testing
@testable import Moshpit

/// Coming back to a session that has nothing up. The field case: the app is
/// opened with the Tailscale tunnel down, the dial hangs on a SYN nobody
/// answers, the user goes to turn the tunnel on and returns — and expects
/// the connection to come up now, not after the connect timeout and the
/// next keepalive tick.
@Suite("session dial recovery")
@MainActor
struct SessionDialRecoveryTests {

    private func connection() -> ServerConnection {
        var connection = ServerConnection(
            name: "recovery",
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
        return session
    }

    @Test("foreground return redials a dial that is still out")
    func foregroundReturnRedials() async throws {
        let gate = DialGate()
        let script = DialScript { _ in try await gate.wait().get() }
        let session = makeSession(script)
        session.stalledDialGrace = 0

        let first = Task { await session.start(theme: .githubDark, fontSize: 14) }
        try await Task.sleep(for: .milliseconds(50))
        #expect(session.isDialing)
        #expect(!session.viewModel.isAutoReconnectInFlight, "the user asked for this connect")

        script.behavior = { connection in SSHSession(connection: connection, transport: QuietTransport()) }
        await session.resumeIfNeeded(probeTimeout: 0.2)

        #expect(script.dials == 2, "the stuck dial is replaced on the spot")
        #expect(session.viewModel.status == .connected)
        #expect(!session.viewModel.isAutoReconnectInFlight,
                "a redial of a connect the user asked for is still theirs")

        // The abandoned dial finally gives up: nothing changes.
        gate.finish(.failure(DialFailed()))
        await first.value
        #expect(session.viewModel.status == .connected)
        #expect(session.viewModel.errorMessage == nil)
        await session.stop()
    }

    @Test("a dial younger than the grace is left to finish")
    func freshDialIsLeftAlone() async throws {
        let gate = DialGate()
        let script = DialScript { _ in try await gate.wait().get() }
        let session = makeSession(script)
        session.stalledDialGrace = 60

        let first = Task { await session.start(theme: .githubDark, fontSize: 14) }
        try await Task.sleep(for: .milliseconds(50))
        await session.resumeIfNeeded(probeTimeout: 0.2)
        #expect(script.dials == 1)
        #expect(session.isDialing)

        gate.finish(.success(SSHSession(connection: connection(), transport: QuietTransport())))
        await first.value
        #expect(session.viewModel.status == .connected)
        await session.stop()
    }

    @Test("foreground return reconnects a failed session without waiting for the tick")
    func foregroundReturnRetriesFailed() async throws {
        let script = DialScript { _ in throw DialFailed() }
        let session = makeSession(script)
        await session.start(theme: .githubDark, fontSize: 14)
        guard case .failed = session.viewModel.status else {
            Issue.record("expected .failed, got \(session.viewModel.status)")
            return
        }

        script.behavior = { connection in SSHSession(connection: connection, transport: QuietTransport()) }
        await session.resumeIfNeeded(probeTimeout: 0.2)
        #expect(script.dials == 2)
        #expect(session.viewModel.status == .connected)
        await session.stop()
    }

    @Test("a network path change kicks the hub's down sessions")
    func pathChangeKicksDownSessions() async throws {
        let hub = SessionHub(metrics: SessionMetricsRegistry())
        let gate = DialGate()
        let failing = DialScript { _ in throw DialFailed() }
        let hanging = DialScript { _ in try await gate.wait().get() }
        let scripts = ["down": failing, "dialing": hanging]
        hub.makeSession = { connection in
            let session = SessionHub.ActiveSession(connection: connection, dial: scripts[connection.name]!.dialer)
            session.probeTimeout = 0.2
            return session
        }
        var down = connection()
        down.name = "down"
        var dialing = connection()
        dialing.name = "dialing"

        let downSession = hub.prepare(down)
        await hub.start(downSession, theme: .githubDark, fontSize: 14)
        let dialingSession = hub.prepare(dialing)
        let dialingStart = Task { await hub.start(dialingSession, theme: .githubDark, fontSize: 14) }
        try await Task.sleep(for: .milliseconds(50))
        #expect(dialingSession.isDialing)

        // Too young for a path change to restart: it most likely began on
        // the new path already.
        hub.pathRedialAfter = 60
        failing.behavior = { connection in SSHSession(connection: connection, transport: QuietTransport()) }
        hanging.behavior = { connection in SSHSession(connection: connection, transport: QuietTransport()) }
        await hub.kickSessionsAfterPathChange()
        #expect(downSession.viewModel.status == .connected, "the failed session reconnects at once")
        #expect(hanging.dials == 1, "the fresh dial is left to finish")

        hub.pathRedialAfter = 0
        await hub.kickSessionsAfterPathChange()
        #expect(hanging.dials == 2, "an older dial is restarted on the new path")
        #expect(dialingSession.viewModel.status == .connected)

        gate.finish(.failure(DialFailed()))
        await dialingStart.value
        #expect(dialingSession.viewModel.status == .connected)
        await hub.disconnect(down.id)
        await hub.disconnect(dialing.id)
    }
}
