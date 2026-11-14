import Foundation
import Testing
@testable import Moshpit

/// B166: after a reconnect, the frame kept from the dropped connection comes
/// down once there is something new to show.
///
/// A reconnect keeps the last painted tmux frame on screen (the screen shows
/// `retiredTmuxController`, with touches off) until the replacement paints —
/// and only `armFirstFrameWait`'s wait takes it down. The mosh fallback to
/// SSH never armed it, so a mosh session on a host without mosh-server, with
/// tmux, froze on its old frame after every reconnect and ignored every
/// touch. A reconnect that lands on a plain shell — or on mosh proper — has
/// no tmux frame coming at all, and left the kept one up for good too.
@Suite("reconnect: the kept frame comes down", .serialized)
@MainActor
struct ReconnectFirstFrameTests {

    nonisolated static let tmuxOnly = "/usr/bin/tmux\n::Linux::/usr/bin/apt-get\n"
    nonisolated static let tmuxAndMosh = "/usr/bin/tmux\n/usr/bin/mosh-server\n::Linux::/usr/bin/apt-get\n"

    /// A host whose probe says `probe`, with a tmux that has a session, and
    /// that starts a mosh-server when asked.
    nonisolated static func host(probe: String) -> FakeSSHHost {
        FakeSSHHost(tmux: .init()) { command in
            if command.contains("command -v tmux mosh-server") { return .output(probe) }
            if command.contains("mosh-server new") { return .output(MoshSessionRig.bootstrapOutput) }
            return .output("")
        }
    }

    private func session(_ proto: ConnectionProtocol, hosts: FakeSSHHosts,
                         vouched: Bool = true) -> SessionHub.ActiveSession {
        var connection = ServerConnection(
            name: "first-frame",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.connectionProtocol = proto
        connection.multiplexer = .tmux
        if vouched { connection.tmuxPath = "tmux" }
        let session = SessionHub.ActiveSession(connection: connection, dial: hosts.script.dialer)
        session.probeTimeout = 0.2
        // Only a painted frame — not the fallback deadline — may take the
        // kept frame down in these tests.
        session.firstFrameFallbackSeconds = 30
        session.firstFrameAttachBoundSeconds = 30
        session.sidecarConnect = { connection in SSHSession(connection: connection, transport: FakeSSHHost()) }
        session.makeMoshTransport = {
            try MoshTransport(credentials: $0, channel: SessionLifecycleTests.IdleDatagramChannel())
        }
        return session
    }

    private func eventually(_ timeout: Double = 5, _ predicate: @MainActor () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if predicate() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return predicate()
    }

    /// The controller attaches and the user reads its frame.
    private func painted(_ session: SessionHub.ActiveSession) async throws -> TmuxSessionController {
        let controller = try #require(session.tmuxController)
        #expect(await eventually { controller.snapshot.panes["%0"] != nil }, "tmux attached")
        _ = controller.terminalView(for: "%0")
        controller.coordinator(for: "%0")?.reveal()
        #expect(controller.hasPaintedSinceAttach)
        return controller
    }

    @Test("mosh fallen back to SSH with tmux: the reconnect's first frame takes the kept one down")
    func fallbackReconnectRevealsTheNewFrame() async throws {
        let hosts = FakeSSHHosts { Self.host(probe: Self.tmuxOnly) }
        let session = session(.mosh, hosts: hosts)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.degrade?.missing == .moshServer, "no mosh-server: SSH on the same connection")
        let first = try await painted(session)

        await session.reconnect()   // the connection dropped
        #expect(session.retiredTmuxController === first, "the last frame stands in during the reconnect")
        let second = try #require(session.tmuxController)
        #expect(second !== first)
        #expect(session.awaitingFirstFrame, "the fallback's attach waits for its first frame")
        #expect(await eventually { second.snapshot.panes["%0"] != nil })
        _ = second.terminalView(for: "%0")
        second.coordinator(for: "%0")?.reveal()   // the replacement paints
        #expect(session.retiredTmuxController == nil,
                "the old frame never came down: frozen, and deaf to touches")
        #expect(!session.awaitingFirstFrame)
        await session.stop()
    }

    @Test("an SSH reconnect that lands on a plain shell takes the kept frame down")
    func plainShellReconnectDropsTheKeptFrame() async throws {
        let hosts = FakeSSHHosts { Self.host(probe: Self.tmuxOnly) }
        let session = session(.ssh, hosts: hosts, vouched: false)
        await session.start(theme: .githubDark, fontSize: 14)
        let first = try await painted(session)

        // tmux is gone from the host by the time the session comes back.
        HostCapabilityCache.shared.store(
            HostCapabilities(hasTmux: false, hasMoshServer: false, os: "Linux", packageManager: nil),
            for: session.connection.id)
        await session.reconnect()
        #expect(session.degrade?.missing == .tmux)
        #expect(session.tmuxController == nil)
        #expect(session.retiredTmuxController == nil, "kept over a plain shell that paints on its own")
        _ = first
        await session.stop()
    }

    @Test("a fallback session that comes back over mosh proper takes the kept frame down")
    func moshReconnectDropsTheKeptFrame() async throws {
        let hosts = FakeSSHHosts { Self.host(probe: Self.tmuxOnly) }
        let session = session(.mosh, hosts: hosts)
        await session.start(theme: .githubDark, fontSize: 14)
        _ = try await painted(session)

        // mosh-server was installed meanwhile.
        hosts.make = { Self.host(probe: Self.tmuxAndMosh) }
        await session.reconnect()
        #expect(session.moshTransport != nil, "mosh proper this time")
        #expect(session.retiredTmuxController == nil, "kept over the mosh terminal for good")
        await session.stop()
    }
}
