import Foundation
import Testing
@testable import Moshpit

/// B122, B168: every await on the way to a connected session resumes into a
/// check that the attempt still owns the session.
///
/// `start()`, `startMosh()` and `fallbackToSSH()` checked the dial alone. A
/// disconnect during the first connect's capability probe still got a tmux
/// controller, its timers and its monitor (B122); one while mosh-server was
/// starting left a transport running and a server nobody reaped, outside the
/// hub; and an attempt a redial had replaced could mark the new attempt
/// failed, or install a second transport over it (B168). crates/session runs
/// the attempt as one task whose every result is checked against the
/// session's generation.
@Suite("start attempts and their generation", .serialized)
@MainActor
struct SessionStartGenerationTests {

    private func eventually(_ timeout: Double = 5, _ predicate: @MainActor () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if predicate() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return predicate()
    }

    /// What hub.disconnect does.
    private func disconnect(_ session: SessionHub.ActiveSession) async {
        session.isStopping = true
        await session.stop()
    }

    @Test("a disconnect during the first connect's capability probe leaves no tmux controller behind (B122)")
    func disconnectDuringTheProbe() async throws {
        var connection = ServerConnection(name: "probe", host: "192.0.2.1", port: 22,
                                          username: "tester", authMethod: .password)
        connection.multiplexer = .tmux   // not vouched: the first connect waits for the probe
        let host = FakeSSHHost(tmux: .init()) { command in
            command.contains("command -v tmux mosh-server") ? .held("probe") : .output("")
        }
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
        session.probeTimeout = 0.2

        let start = Task { await session.start(theme: .githubDark, fontSize: 14) }
        #expect(await eventually { host.isHolding("probe") })
        await disconnect(session)
        await start.value

        #expect(session.tmuxController == nil, "a controller wired onto a session already gone")
        #expect(!host.typed.contains("-CC attach"), "the boot line went out after the disconnect")
        #expect(!host.programs.contains { $0?.contains("-CC attach") == true },
                "tmux's control client started after the disconnect")
        #expect(session.viewModel.status != .connected)
    }

    /// A rig whose first `mosh-server new` is held under "first" (its reply
    /// still arriving after the connection was closed), every later one
    /// answered at once.
    private func heldBootstrapRig() -> MoshSessionRig {
        let bootstraps = DialCounter()
        let rig = MoshSessionRig(answer: { command in
            if command.contains("mosh-server new") {
                bootstraps.bump()
                if bootstraps.count == 1 { return .held("first") }
            }
            return MoshSessionRig.answer(command)
        })
        rig.host.heldSurviveShutdown = true
        return rig
    }

    @Test("a disconnect while mosh-server starts installs no transport and reaps the server (B168)")
    func disconnectDuringTheBootstrap() async throws {
        let rig = heldBootstrapRig()
        let start = Task { await rig.start() }
        #expect(await rig.eventually { rig.host.isHolding("first") })
        await disconnect(rig.session)

        // mosh-server's answer lands after the session is gone.
        rig.host.release("first", with: MoshSessionRig.bootstrapOutput)
        await start.value

        #expect(rig.built.isEmpty, "a transport was built for a session already stopped")
        #expect(rig.session.moshTransport == nil)
        #expect(rig.session.viewModel.status != .connected)
        #expect(await rig.eventually { rig.killed() }, "the server it started was left for nobody to reap")
    }

    @Test("a replaced attempt's failure doesn't fail the attempt that replaced it (B168)")
    func replacedAttemptFailureIsDropped() async throws {
        let rig = heldBootstrapRig()
        let first = Task { await rig.start() }
        #expect(await rig.eventually { rig.host.isHolding("first") })

        await rig.session.reconnect()   // a redial replaces the attempt
        #expect(rig.session.viewModel.status == .connected)
        let replacement = try #require(rig.session.moshTransport)

        rig.host.release("first", with: "mosh-server: not what anyone expected\n")
        await first.value
        #expect(rig.session.viewModel.status == .connected, "the replaced attempt marked the new one failed")
        #expect(rig.session.viewModel.errorMessage == nil)
        #expect(rig.session.moshTransport === replacement)
        await rig.session.stop(forReconnect: true)
    }

    @Test("a replaced attempt that started its server installs no second transport, and reaps it (B168)")
    func replacedAttemptInstallsNothing() async throws {
        let rig = heldBootstrapRig()
        let first = Task { await rig.start() }
        #expect(await rig.eventually { rig.host.isHolding("first") })

        await rig.session.reconnect()
        let replacement = try #require(rig.session.moshTransport)
        #expect(rig.built.count == 1)

        let late = "MOSH CONNECT 60002 \(MoshSessionRig.keyText)\n[mosh-server detached, pid = 5151]\n"
        rig.host.release("first", with: late)
        await first.value
        #expect(rig.built.count == 1, "a second transport was built")
        #expect(rig.session.moshTransport === replacement, "the replaced attempt's transport took over")
        #expect(await rig.eventually { rig.killed(5151) }, "its server was left running for nobody")
        #expect(!rig.killed(MoshSessionRig.pid), "the live session's server was killed")
        await rig.session.stop(forReconnect: true)
    }

    @Test("a disconnect while the mosh fallback opens its PTY wires nothing up (B168)")
    func disconnectDuringTheFallback() async throws {
        let rig = MoshSessionRig(multiplexer: .tmux, answer: { command in
            command.contains("command -v tmux mosh-server")
                ? .output("/usr/bin/tmux\n::Linux::/usr/bin/apt-get\n") : .output("")
        })
        rig.host.tmux = .init()
        rig.host.heldSurviveShutdown = true
        rig.host.holdPTY = "pty"
        let start = Task { await rig.start() }
        #expect(await rig.eventually { rig.host.isHolding("pty") })
        #expect(rig.session.degrade?.missing == .moshServer)
        await disconnect(rig.session)

        rig.host.release("pty", with: "")
        await start.value
        #expect(rig.session.tmuxController == nil, "a tmux controller wired onto a stopped session")
        #expect(rig.session.viewModel.status != .connected, "a stopped session marked connected")
    }
}
