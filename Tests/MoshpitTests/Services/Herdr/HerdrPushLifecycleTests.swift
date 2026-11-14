import Foundation
import Testing
@testable import Moshpit

/// herdr's push channel over SSH, through a real `ActiveSession.start()`:
/// the frame channel on the session's connection (``HerdrLoopHost``, whose
/// probe says python3 is there), the pump on its own (``HerdrPumpHost``).
/// After the Rust client's `push.rs`
/// (`a_push_channel_that_dies_puts_the_poll_back_in_charge_and_comes_back`).
@Suite("herdr push channel's life", .serialized)
@MainActor
struct HerdrPushLifecycleTests {

    private func connection() -> ServerConnection {
        var connection = ServerConnection(
            name: "herdr-push",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.connectionProtocol = .ssh
        connection.multiplexer = .herdr
        connection.herdrPath = "/usr/local/bin/herdr"
        return connection
    }

    private func waitUntil(_ timeout: Duration = .seconds(10), _ done: () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while !done() {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    /// Started with push up.
    private func pushing(_ connection: ServerConnection, _ pumps: HerdrPumpHost,
                         retryAfter delay: Duration? = nil) async throws -> SessionHub.ActiveSession {
        let host = HerdrLoopHost(python3: true)
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
        session.sidecarConnect = { connection in SSHSession(connection: connection, transport: pumps.transport()) }
        if let delay { session.herdrPushRetryDelay = delay }
        await session.start(theme: .githubDark, fontSize: 14)
        let control = try #require(session.herdrControl)
        #expect(await waitUntil { control.pushActive }, "push never came up")
        #expect(pumps.dials == 1)
        return session
    }

    /// A herdr server restart closes the socket under the pump: the pump
    /// exits and its channel ends. `pushActive` stayed true, so the poll —
    /// eased to its 45 s safety net once push was up — stayed there with no
    /// events coming: agent states and the tree lagged by up to 45 s.
    @Test("Push that dies puts the poll back in charge at once")
    func pushDeathPutsThePollBackInCharge() async throws {
        let connection = connection()
        defer { UserDefaults.standard.removeObject(forKey: "hostcaps." + connection.id.uuidString) }
        let pumps = HerdrPumpHost()
        let session = try await pushing(connection, pumps)
        let control = try #require(session.herdrControl)

        pumps.endCurrent()
        #expect(await waitUntil { !control.pushActive },
                "push is gone and the poll still counts on it")
        #expect(control.pollInterval == HerdrControlClient.fastPollInterval)
        await session.stop()
    }

    /// B267: the pump's start line was typed into the login shell the push
    /// channel opened, and a shell that flushes what was typed while it
    /// started (Powerlevel10k's instant prompt; live, a 3 s zsh rc that
    /// flushes) lost it: push never came up. The pump is the channel's own
    /// program now, and nothing is typed into a shell.
    @Test("The push pump is its channel's program, never typed into a login shell")
    func thePumpIsTheChannelsProgram() async throws {
        let connection = connection()
        defer { UserDefaults.standard.removeObject(forKey: "hostcaps." + connection.id.uuidString) }
        let pumps = HerdrPumpHost(flushingShell: true)
        let session = try await pushing(connection, pumps)
        #expect(pumps.programs == [HerdrPushBoot.bootLine()])
        #expect(pumps.typedIntoShells.isEmpty, "typed into a login shell: \(pumps.typedIntoShells)")
        await session.stop()
    }

    @Test("Push that dies is tried again, and comes back")
    func pushComesBack() async throws {
        let connection = connection()
        defer { UserDefaults.standard.removeObject(forKey: "hostcaps." + connection.id.uuidString) }
        let pumps = HerdrPumpHost()
        let session = try await pushing(connection, pumps, retryAfter: .milliseconds(300))
        let control = try #require(session.herdrControl)

        pumps.endCurrent()
        #expect(await waitUntil { !control.pushActive })
        #expect(await waitUntil { pumps.dials == 2 && control.pushActive }, "push was never tried again")
        await session.stop()
    }
}
