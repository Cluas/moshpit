import Foundation
import Testing
@testable import Moshpit

/// B167: every path that ends a tmux attach hands its window pins back.
///
/// A reconnect retires the tmux controller without a round trip on the dead
/// connection and carries what it owed the server — the windows pinned to
/// the phone's grid, the layouts beneath them — to the replacement
/// (`inheritedPinLedger`). Three ways it was lost: the mosh fallback to SSH
/// never took the debt over; a second retirement before anything took it
/// over overwrote it; and a full `stop()` paid only what a live controller
/// held — so after a drop, a redial that failed, and a disconnect, the
/// desktop's tmux window stayed at the phone's size. crates/tmux's
/// controller and crates/session (`pay_pin_debt`) pay it on the real end.
@Suite("tmux pin debt", .serialized)
@MainActor
struct TmuxPinDebtTests {

    private func connection(_ proto: ConnectionProtocol) -> ServerConnection {
        var connection = ServerConnection(
            name: "pins",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.connectionProtocol = proto
        connection.multiplexer = .tmux
        connection.tmuxPath = "tmux"
        return connection
    }

    private func controller(pinning windows: Set<String>) -> TmuxSessionController {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        controller.inheritPinLedger(TmuxSessionController.PinLedger(resizedWindows: windows))
        return controller
    }

    private func eventually(_ timeout: Double = 5, _ predicate: @MainActor () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if predicate() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return predicate()
    }

    @Test("a second retirement adds to what is owed instead of replacing it")
    func debtsAdd() async {
        let session = SessionHub.ActiveSession(connection: connection(.ssh))
        session.installForTesting(moshTransport: nil, tmuxController: controller(pinning: ["@3"]))
        await session.stop(forReconnect: true)
        #expect(session.inheritedPinLedger?.pinnedWindows == ["@3"])

        // Another controller is retired before anything took the first debt.
        session.installForTesting(moshTransport: nil, tmuxController: controller(pinning: ["@5"]))
        await session.stop(forReconnect: true)
        #expect(session.inheritedPinLedger?.pinnedWindows == ["@3", "@5"],
                "the first controller's pins were dropped")
    }

    @Test("pins no redial took over are handed back when the session ends")
    func unclaimedPinsArePaidAtTheEnd() async throws {
        let hosts = FakeSSHHosts { FakeSSHHost(tmux: .init()) }
        let script = hosts.script
        let payer = FakeSSHHost()
        let session = SessionHub.ActiveSession(connection: connection(.ssh), dial: script.dialer)
        session.probeTimeout = 0.2
        session.sidecarConnect = { connection in SSHSession(connection: connection, transport: payer) }
        await session.start(theme: .githubDark, fontSize: 14)
        let attached = try #require(session.tmuxController)
        #expect(await eventually { attached.snapshot.isAttached })
        attached.inheritPinLedger(TmuxSessionController.PinLedger(resizedWindows: ["@3"]))

        // The connection drops and the redial fails.
        script.behavior = { _ in throw SSHError.connectionFailed }
        await session.reconnect()
        guard case .failed = session.viewModel.status else {
            Issue.record("expected the redial to fail, got \(session.viewModel.status)")
            return
        }
        // (Plus whatever window the controller pinned on its own attach.)
        #expect(session.inheritedPinLedger?.pinnedWindows.contains("@3") == true)

        // The user disconnects.
        session.isStopping = true
        await session.stop()
        // One script now, reading each window before it unpins (round 34).
        #expect(payer.commands.contains { $0.contains("tmux set-option -u -w -t @3 window-size") },
                "the desktop's window stayed at the phone's size: \(payer.commands)")
        #expect(session.inheritedPinLedger == nil)
    }

    @Test("the mosh fallback to SSH takes the retired controller's pins over")
    func fallbackTakesThePinsOver() async throws {
        let hosts = FakeSSHHosts {
            FakeSSHHost(tmux: .init()) { command in
                command.contains("command -v tmux mosh-server")
                    ? .output("/usr/bin/tmux\n::Linux::/usr/bin/apt-get\n") : .output("")
            }
        }
        let session = SessionHub.ActiveSession(connection: connection(.mosh), dial: hosts.script.dialer)
        session.probeTimeout = 0.2
        session.sidecarConnect = { connection in SSHSession(connection: connection, transport: FakeSSHHost()) }
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.degrade?.missing == .moshServer)
        let first = try #require(session.tmuxController)
        #expect(await eventually { first.snapshot.isAttached })
        first.inheritPinLedger(TmuxSessionController.PinLedger(resizedWindows: ["@3"]))

        await session.reconnect()
        let second = try #require(session.tmuxController)
        #expect(second !== first)
        #expect(session.inheritedPinLedger == nil, "the fallback left the debt behind for the next reconnect")
        #expect(second.resizedWindows.contains("@3"))

        // And the disconnect pays it, over the live connection.
        #expect(await eventually { second.snapshot.isAttached })
        session.isStopping = true
        await session.stop()
        let host = try #require(hosts.last)
        #expect(host.commands.contains { $0.contains("tmux set-option -u -w -t @3 window-size") }, "\(host.commands)")
    }
}
