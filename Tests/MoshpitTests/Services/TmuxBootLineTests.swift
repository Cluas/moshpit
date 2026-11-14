import Foundation
import Testing
@testable import Moshpit

/// Moshpit attaches to the user's tmux and changes nothing on it.
///
/// Every line that boots a control client started with
/// `tmux set -g history-limit 50000`: a global, persistent change to the
/// user's own server, made on every attach, that also LOWERED a limit the
/// user had set higher (tmux 3.6a, private socket: 200000 → 50000, and every
/// pane made after it got 50000). These run a real session over a scripted
/// SSH transport and read each control channel's program.
///
/// And nothing is typed into the channel's shell (B267): the control client
/// is the terminal channel's own program. Typed into the login shell, a
/// shell still starting swallowed it — a 3 s zsh rc that flushes pending
/// input, as Powerlevel10k's instant prompt does — and nothing attached.
@Suite("tmux boot lines leave the user's server alone")
@MainActor
struct TmuxBootLineTests {

    typealias Transport = SessionLifecycleTests.ScriptedTransport

    /// What a host with tmux and no mosh-server answers the capability probe.
    static let tmuxWithoutMoshServer = "/usr/bin/tmux\n::Linux::\n"

    private func connection(_ transport: ConnectionProtocol) -> ServerConnection {
        var connection = ServerConnection(
            name: "boot-line",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.connectionProtocol = transport
        connection.multiplexer = .tmux
        return connection
    }

    private func typed(_ transport: Transport) -> [String] {
        transport.writer.written.map { String(decoding: $0, as: UTF8.self) }
    }

    private func programs(_ transport: Transport) -> [String] {
        transport.programs.compactMap { $0 }
    }

    private func until(_ done: @MainActor () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(10)
        while ContinuousClock.now < deadline {
            if done() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return done()
    }

    /// The control channel's program attaches, nothing in it sets anything
    /// on the server, and nothing was typed into the channel.
    private func expectAttachOnly(_ transport: Transport, _ path: String) {
        let lines = programs(transport)
        #expect(lines.contains { $0.contains("-CC attach") }, "\(path): no control-mode program")
        #expect(typed(transport).isEmpty, "\(path) typed into the channel's shell: \(typed(transport))")
        for line in lines {
            #expect(!line.contains("history-limit"), "\(path) changed the user's history-limit: \(line)")
            #expect(!line.contains("set -g") && !line.contains("set-option"),
                    "\(path) set an option on the user's server: \(line)")
        }
    }

    @Test("the SSH session's boot line only attaches")
    func sshBootOnlyAttaches() async throws {
        let transport = Transport(.reply(""))
        var connection = connection(.ssh)
        connection.tmuxPath = "tmux"   // vouched for: no probe degrades it
        let script = DialScript { SSHSession(connection: $0, transport: transport) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)

        await session.start(theme: .githubDark, fontSize: 14)
        try #require(await until { programs(transport).contains { $0.contains("-CC attach") } })

        expectAttachOnly(transport, "SSH")
        await session.stop()
    }

    /// mosh to a host without mosh-server falls back to SSH, and boots tmux
    /// there the way the SSH path does.
    @Test("the mosh fallback's boot line only attaches")
    func moshFallbackBootOnlyAttaches() async throws {
        let transport = Transport(.reply(Self.tmuxWithoutMoshServer))
        let script = DialScript { SSHSession(connection: $0, transport: transport) }
        let session = SessionHub.ActiveSession(connection: connection(.mosh), dial: script.dialer)

        await session.start(theme: .githubDark, fontSize: 14)
        try #require(await until { programs(transport).contains { $0.contains("-CC attach") } },
                     "the fallback never booted tmux")

        #expect(session.degrade?.missing == .moshServer)
        expectAttachOnly(transport, "the mosh fallback")
        await session.stop()
    }

    /// The mosh sidecar's chain, as its rebuild boots it.
    @Test("the mosh sidecar's boot line only attaches")
    func sidecarBootOnlyAttaches() async throws {
        let session = SessionHub.ActiveSession(connection: connection(.mosh))
        session.probeTimeout = 0.2
        let key = Data((0..<16).map { UInt8($0) })
        let mosh = try MoshTransport(
            credentials: MoshCredentials(host: "192.0.2.1", udpPort: 60001, key: key),
            channel: SessionLifecycleTests.IdleDatagramChannel())
        session.installForTesting(moshTransport: mosh)
        let sidecar = Transport(.reply(""))
        session.sidecarConnect = { SSHSession(connection: $0, transport: sidecar) }

        await session.resumeIfNeeded(probeTimeout: 0.2)

        expectAttachOnly(sidecar, "the mosh sidecar")
        await session.stop()
    }
}
