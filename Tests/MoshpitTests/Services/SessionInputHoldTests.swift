import Foundation
import Testing
@testable import Moshpit

/// B169: what is typed before the session knows where typing goes waits,
/// and then goes there in order.
///
/// An SSH session's PTY opened — and the session read connected — the moment
/// the dial landed; the capability probe, the multiplexer decision and tmux's
/// attach followed. Keys typed in that window went straight into the login
/// shell, ahead of the `-CC attach` line: the shell then ran
/// `<keys>tmux -CC attach`, and the attach failed. crates/session holds input
/// for a pane that isn't attached yet and sends it once the pane is there,
/// never into the shell underneath.
///
/// Since B267 the probe comes before the terminal opens (its program is
/// tmux's control client, decided by the probe), so the session reads
/// connected only once tmux's client is running: what is typed then waits
/// for the attach, and nothing is ever typed into a shell.
@Suite("input before the attach decision", .serialized)
@MainActor
struct SessionInputHoldTests {

    nonisolated static let probeWithTmux = "/usr/bin/tmux\n::Linux::/usr/bin/apt-get\n"

    private func connection(_ multiplexer: Multiplexer) -> ServerConnection {
        var connection = ServerConnection(
            name: "hold",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.multiplexer = multiplexer   // not vouched: a first connect waits for the probe
        return connection
    }

    /// A first connect whose terminal runs tmux's control client, tmux
    /// answering it a round trip later (`answerDelay`, as over any real
    /// link): the session reads connected, the attach is still out.
    private func attaching(tmux: FakeSSHHost.Tmux = .init(answerDelay: .milliseconds(300)))
        async throws -> (SessionHub.ActiveSession, FakeSSHHost, TmuxSessionController) {
        let host = FakeSSHHost(tmux: tmux) { command in
            command.contains("command -v tmux mosh-server") ? .output(Self.probeWithTmux) : .output("")
        }
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection(.tmux), dial: script.dialer)
        session.probeTimeout = 0.2
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.connState == .live, "the session reads connected")
        let controller = try #require(session.tmuxController)
        #expect(!controller.snapshot.isAttached, "tmux hasn't answered the attach yet")
        #expect(host.programs.compactMap { $0 }.first?.contains("-CC attach") == true,
                "the terminal's program is tmux's control client")
        return (session, host, controller)
    }

    private func eventually(_ timeout: Double = 5, _ predicate: @MainActor () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if predicate() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return predicate()
    }

    @Test("keys typed before tmux attaches reach its pane after the attach, never a shell")
    func keysWaitForThePane() async throws {
        let (session, host, controller) = try await attaching()
        session.sendInput(Data("ls".utf8))
        session.sendInput(Data("\r".utf8))
        #expect(host.written.isEmpty, "typed into the channel before the attach: \(host.written)")
        #expect(await eventually { controller.snapshot.panes["%0"] != nil })

        // The first frame: what was typed goes to the pane, in order.
        _ = controller.surface(for: "%0")
        controller.coordinator(for: "%0")?.reveal()
        #expect(await eventually { host.typed.contains("send-keys -t %0 -H 0d") })
        let typed = host.typed
        let keys = try #require(typed.range(of: "send-keys -t %0 -H 6c 73"), "\(typed)")
        let enter = try #require(typed.range(of: "send-keys -t %0 -H 0d"))
        #expect(keys.lowerBound < enter.lowerBound, "out of order")
        #expect(!typed.contains("-CC attach"), "an attach line typed into the channel: \(host.written)")
        await session.stop()
    }

    @Test("a keystroke after the attach doesn't overtake what was held for it")
    func heldKeysGoFirst() async throws {
        let (session, host, controller) = try await attaching()
        session.sendInput(Data("a".utf8))
        #expect(await eventually { controller.snapshot.panes["%0"] != nil })
        session.sendInput(Data("b".utf8))   // attached, first frame not painted yet
        #expect(await eventually { host.typed.contains("send-keys -t %0 -H 62") })
        let typed = host.typed
        let held = try #require(typed.range(of: "send-keys -t %0 -H 61"), "\(typed)")
        let later = try #require(typed.range(of: "send-keys -t %0 -H 62"), "\(typed)")
        #expect(held.lowerBound < later.lowerBound)
        await session.stop()
    }

    /// The first connect's probe runs before the terminal opens: the
    /// connecting cover is up, and touches then are dropped as they are on
    /// any connecting session. Nothing reaches the host; the plain shell's
    /// first input is what is typed once it is open.
    @Test("a plain shell: nothing typed during the probe reaches the host; once open, typing reaches the shell")
    func plainShellGetsTheKeys() async {
        let host = FakeSSHHost(tmux: nil) { command in
            command.contains("command -v tmux mosh-server") ? .held("probe") : .output("")
        }
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection(.none), dial: script.dialer)
        session.probeTimeout = 0.2
        let start = Task { await session.start(theme: .githubDark, fontSize: 14) }
        #expect(await eventually { host.isHolding("probe") })
        #expect(session.viewModel.connState != .live, "the terminal isn't open: still connecting")
        #expect(host.ptyOpened == 0)
        session.sendInput(Data("stray".utf8))
        host.release("probe", with: Self.probeWithTmux)
        await start.value
        #expect(session.viewModel.connState == .live)
        #expect(host.programs == [nil], "a plain shell: the login shell, no program")
        session.sendInput(Data("echo hi\r".utf8))
        #expect(await eventually { host.typed == "echo hi\r" }, "\(host.written)")
        await session.stop()
    }

    @Test("keys typed while tmux refuses the attach go nowhere — never into the shell")
    func refusedAttachDropsTheKeys() async throws {
        // The refusal comes back a round trip after the attach, as it does
        // over any real link: answered at once, it predates the cover's
        // wait, which then reads the silence as the link's and holds for the
        // whole attach bound (22 s).
        let (session, host, _) = try await attaching(
            tmux: .init(hasSession: false, answerDelay: .milliseconds(300)))
        session.firstFrameFallbackSeconds = 0.2
        session.sendInput(Data("zz".utf8))
        #expect(await eventually { !session.awaitingFirstFrame }, "the refusal ends the wait")
        try? await Task.sleep(for: .milliseconds(100))
        #expect(!host.typed.contains("zz"), "\(host.written)")
        #expect(!host.typed.contains("send-keys"))
        await session.stop()
    }

    @Test("a share delivered while the attach is undecided is not taken (its sender keeps it)")
    func deliveryWaitsForTheDecision() async throws {
        let (session, host, _) = try await attaching()
        #expect(await session.deliverInput(Data("/tmp/shared.png".utf8), toPane: "%0") == false)
        #expect(await session.deliverPaste("/tmp/shared.png", toPane: "%0") == false)
        #expect(host.written.isEmpty, "delivered into the channel: \(host.written)")
        await session.stop()
    }

    // MARK: - mosh fallen back to SSH (B197)

    /// A mosh connection to a host with no mosh-server, back on SSH over the
    /// same connection with tmux (`fallbackToSSH`): the session reads
    /// connected the moment tmux's control client runs, and tmux answers it
    /// a round trip later — `answerDelay`, as over any real link (answered
    /// inside the write, the attach would beat everything typed).
    private func fallenBack(tmux: FakeSSHHost.Tmux, firstFrameFallback: Double? = nil) async throws
        -> (SessionHub.ActiveSession, FakeSSHHost, TmuxSessionController) {
        let host = FakeSSHHost(tmux: tmux) { command in
            command.contains("command -v tmux mosh-server") ? .output(Self.probeWithTmux) : .output("")
        }
        var connection = connection(.tmux)
        connection.connectionProtocol = .mosh
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
        session.probeTimeout = 0.2
        if let firstFrameFallback { session.firstFrameFallbackSeconds = firstFrameFallback }
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.degrade?.missing == .moshServer, "no mosh-server: SSH on the same connection")
        #expect(session.viewModel.connState == .live, "the session reads connected")
        let controller = try #require(session.tmuxController)
        #expect(!controller.snapshot.isAttached, "tmux hasn't answered the attach yet")
        return (session, host, controller)
    }

    @Test("mosh fallen back to SSH: keys typed before tmux attaches reach its pane after the attach, in order")
    func fallbackKeysWaitForThePane() async throws {
        let (session, host, controller) = try await fallenBack(tmux: .init(answerDelay: .milliseconds(300)))
        session.sendInput(Data("ls".utf8))
        session.sendInput(Data("\r".utf8))
        #expect(await eventually { controller.snapshot.panes["%0"] != nil }, "tmux attached")

        // The first frame: what was typed goes to the pane, in order.
        _ = controller.surface(for: "%0")
        controller.coordinator(for: "%0")?.reveal()
        #expect(await eventually { host.typed.contains("send-keys -t %0 -H 0d") },
                "dropped without a word: \(host.written)")
        let typed = host.typed
        let keys = try #require(typed.range(of: "send-keys -t %0 -H 6c 73"), "\(typed)")
        let enter = try #require(typed.range(of: "send-keys -t %0 -H 0d"))
        #expect(keys.lowerBound < enter.lowerBound, "out of order")
        #expect(!typed.contains("-CC attach"), "an attach line typed into the channel: \(host.written)")
        #expect(host.programs.compactMap { $0 }.first?.contains("-CC attach") == true,
                "the terminal's program is tmux's control client")
        await session.stop()
    }

    @Test("mosh fallen back to SSH: a keystroke after the attach doesn't overtake what was held for it")
    func fallbackHeldKeysGoFirst() async throws {
        let (session, host, controller) = try await fallenBack(tmux: .init(answerDelay: .milliseconds(300)))
        session.sendInput(Data("a".utf8))
        #expect(await eventually { controller.snapshot.panes["%0"] != nil })
        session.sendInput(Data("b".utf8))   // attached, first frame not painted yet
        #expect(await eventually { host.typed.contains("send-keys -t %0 -H 62") })
        let typed = host.typed
        let held = try #require(typed.range(of: "send-keys -t %0 -H 61"), "the held key was lost: \(typed)")
        let later = try #require(typed.range(of: "send-keys -t %0 -H 62"), "\(typed)")
        #expect(held.lowerBound < later.lowerBound)
        await session.stop()
    }

    @Test("mosh fallen back to SSH: keys typed while tmux refuses the attach go nowhere — never into the shell")
    func fallbackRefusedAttachDropsTheKeys() async throws {
        let (session, host, _) = try await fallenBack(
            tmux: .init(hasSession: false, answerDelay: .milliseconds(300)), firstFrameFallback: 0.2)
        session.sendInput(Data("zz".utf8))
        #expect(await eventually { !session.awaitingFirstFrame }, "the refusal ends the wait")
        try? await Task.sleep(for: .milliseconds(100))
        #expect(!host.typed.contains("zz"), "\(host.written)")
        #expect(!host.typed.contains("send-keys"))
        await session.stop()
    }
}
