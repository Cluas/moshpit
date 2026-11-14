import Foundation
import Testing
@testable import Moshpit

/// B228: before the control plane's first attach — a mosh + tmux host with
/// no session yet — "New session" did nothing. The controller dropped the
/// command (control mode isn't live, so a write would land in the shell),
/// Home showed "Attaching…" and then the stalled notice with no + anywhere,
/// and the terminal showed the renderer's bare shell. Now the hub asks the
/// server whether it has a session at all; with none the card offers the
/// empty SESSIONS list and its +, the terminal its empty state, and the
/// person's "New session" is made over the control plane's connection — as
/// crates/session's `first_session` does. Moshpit still makes none of its
/// own.
@Suite("tmux: the first session, before anything is attached (B228)", .serialized)
@MainActor
struct TmuxFirstSessionTests {

    @Test("New session with nothing attached goes to the hub, with the name as typed, and nothing is written")
    func newSessionBeforeTheAttachGoesToTheHub() async throws {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        var asked: [String?] = []
        controller.createsFirstSession = { asked.append($0) }

        controller.newSession(named: "  work  ")
        controller.newSession(named: nil)
        controller.newSession(named: "bad\nname")
        #expect(asked == ["work", nil], "the person's New session was dropped before the attach")
        try await Task.sleep(for: .milliseconds(50))
        #expect(await transport.recordedCommands().isEmpty, "a command written into the shell")
        #expect(controller.notice != nil, "a refused name says why")

        // Attached, it is control mode's, as before.
        await controller.attach()
        controller.newSession(named: "play")
        let deadline = ContinuousClock.now + .seconds(5)
        while await !transport.recordedCommands().contains(where: { $0.hasPrefix("new-session -d") }),
              ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(5))
        }
        #expect(await transport.recordedCommands().contains { $0.hasPrefix("new-session -d -P -F") })
        #expect(asked == ["work", nil])
    }

    @Test("a session name as a shell word: quoted, every # doubled (tmux expands it)")
    func shellWord() {
        #expect(TmuxSessionController.tmuxNameShellWord("work") == "'work'")
        #expect(TmuxSessionController.tmuxNameShellWord("issue #12") == "'issue ##12'")
        #expect(TmuxSessionController.tmuxNameShellWord("it's") == #"'it'\''s'"#)
        #expect(SessionHub.ActiveSession.tmuxFirstSessionCommand(tmux: "tmux", name: nil)
                == #"tmux new-session -d 2>&1; echo "moshpit-status:$?""#)
        #expect(SessionHub.ActiveSession.tmuxFirstSessionCommand(tmux: "/opt/tmux", name: "a#b")
                == #"/opt/tmux new-session -d -s 'a##b' 2>&1; echo "moshpit-status:$?""#)
    }

    // MARK: - SSH + tmux, end to end over fakes

    /// B267: over SSH tmux's control client is the terminal channel's own
    /// program, waiting for a session while the server has none. "New
    /// session" is an exec on the same connection, and the waiting client
    /// attaches it. It used to be `tmux -CC new` typed into the shell the
    /// failed attach left behind — and the attach itself a line typed into a
    /// login shell that may still have been starting.
    @Test("SSH + tmux with no session: New session is an exec, the waiting control client attaches it, nothing typed")
    func sshNewSessionBeforeTheFirstAttach() async throws {
        final class HostBox: @unchecked Sendable { var host: FakeSSHHost? }
        let box = HostBox()
        let host = FakeSSHHost(tmux: .init(hasSession: false)) { command in
            guard command.contains("new-session -d") else { return .output("") }
            box.host?.sessionAppears()
            return .output("moshpit-status:0\n")
        }
        box.host = host
        var connection = ServerConnection(name: "first", host: "192.0.2.1", port: 22,
                                          username: "tester", authMethod: .password)
        connection.multiplexer = .tmux
        connection.tmuxPath = "tmux"   // vouched for: no probe degrades it
        defer { UserDefaults.standard.removeObject(forKey: "hostcaps." + connection.id.uuidString) }
        let script = DialScript { SSHSession(connection: $0, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
        session.probeTimeout = 0.2
        await session.start(theme: .githubDark, fontSize: 14)

        let program = try #require(host.programs.compactMap { $0 }.first)
        #expect(program == SessionHub.ActiveSession.tmuxControlAttachProgram(tmux: "tmux"))
        let deadline = ContinuousClock.now + .seconds(5)
        while !host.retryingAttach, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(host.retryingAttach, "the control client isn't waiting for a session")
        let controller = try #require(session.tmuxController)
        #expect(!controller.snapshot.isAttached)

        session.createFirstTmuxSession(named: "work")
        while !controller.snapshot.isAttached, ContinuousClock.now < deadline + .seconds(5) {
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(controller.snapshot.isAttached, "the waiting control client never attached the new session")
        #expect(host.commands.contains(#"tmux new-session -d -s 'work' 2>&1; echo "moshpit-status:$?""#),
                "\(host.commands)")
        // Control mode's own commands go down the channel once attached;
        // nothing that starts or makes anything ever does.
        #expect(!host.typed.contains("-CC"), "typed into the terminal's shell: \(host.typed.debugDescription)")
        #expect(!host.typed.contains("new-session -d -s"))
        await session.stop()
    }

    // MARK: - mosh + tmux, end to end over fakes

    nonisolated private static let rendererPid = 4711

    /// A mosh + tmux host whose server has no session. The control plane's
    /// connection (`reaper`) answers `has-session` as tmux does, and makes a
    /// session on `new-session` after a round trip — or refuses it.
    private func makeRig(clock: VirtualClock, channel: ShellDatagramChannel,
                         refuse: String? = nil) -> MoshSessionRig {
        let rig = MoshSessionRig(multiplexer: .tmux, answer: { command in
            if command.contains("command -v tmux mosh-server") {
                return .output("/usr/bin/tmux\n/usr/bin/mosh-server\n::Linux::/usr/bin/apt-get\n")
            }
            return MoshSessionRig.answer(command)
        }, transport: { credentials, _ in
            try MoshTransport(credentials: credentials, channel: channel,
                              returnPathDeadlineNanos: 600 * 1_000_000_000)
        })
        rig.reaper.tmux = .init(hasSession: false, answerDelay: .milliseconds(40),
                                clients: ["0|\(Self.rendererPid)|$0|/dev/pts/4", "1|5150|$0|client-5150"])
        let reaper = rig.reaper
        rig.reaper.answer { command in
            if command.contains("cat "), command.contains(".moshpit/tmuxr-") {
                return .output("\(Self.rendererPid)\n")
            }
            if command.contains("has-session 2>/dev/null; echo $?") {
                return .output(reaper.tmux?.hasSession == true ? "0\n" : "1\n")
            }
            if command.contains("new-session -d") {
                Thread.sleep(forTimeInterval: 0.03)
                if let refuse { return .output("\(refuse)\nmoshpit-status:1\n") }
                reaper.sessionAppears()
                return .output("moshpit-status:0\n")
            }
            return .output("")
        }
        rig.session.moshAttachClock = clock
        rig.session.attachTimeoutSeconds = 0.5
        return rig
    }

    private func run(_ clock: VirtualClock, for total: Duration, step: Duration) async {
        var moved = Duration.zero
        while moved < total {
            clock.advance(by: step)
            moved += step
            let deadline = Date().addingTimeInterval(1)
            repeat {
                try? await Task.sleep(for: .milliseconds(2))
            } while clock.sleeping == 0 && Date() < deadline
        }
    }

    @Test("mosh + tmux with no session: no stalled notice, and New session makes it over the control plane")
    func moshNewSessionBeforeTheFirstAttach() async throws {
        let clock = VirtualClock()
        let channel = ShellDatagramChannel()
        let rig = makeRig(clock: clock, channel: channel)
        await rig.start()
        #expect(await rig.eventually { rig.reaper.retryingAttach })
        #expect(await rig.eventually { clock.sleeping == 1 })
        #expect(!rig.session.awaitingFirstSession, "decided before the server was asked")
        await run(clock, for: .seconds(6), step: .milliseconds(250))

        #expect(await rig.eventually { rig.session.awaitingFirstSession },
                "the server was never asked whether it has a session")
        try await Task.sleep(for: .milliseconds(700))
        #expect(!rig.session.attachStalled, "Home: \"Attaching…\", then the stalled notice")
        // The renderer — mosh-server's program — waits for a session
        // itself; nothing is typed into the mosh shell (B267).
        #expect(channel.typed().isEmpty)

        // The person's New session, from Home's + (a name) or the terminal's
        // empty state.
        let control = try #require(rig.session.moshControl)
        control.newSession(named: "issue #12")
        #expect(await rig.eventually {
            rig.reaper.commands.contains(#"tmux new-session -d -s 'issue ##12' 2>&1; echo "moshpit-status:$?""#)
        }, "the New session never reached the server: \(rig.reaper.commands)")
        #expect(await rig.eventually { control.snapshot.isAttached }, "the control plane never landed on it")
        #expect(!rig.session.awaitingFirstSession)
        #expect(!rig.session.attachStalled)
        // Nothing typed into the shell for it: the renderer's waiting
        // program attaches by itself; nothing made by `-CC new` or anywhere
        // else.
        #expect(channel.typed().isEmpty)
        #expect(!rig.reaper.typed.contains("-CC new"))
        #expect(rig.reaper.commands.filter { $0.contains("new-session") }.count == 1)
        await rig.session.stop()
    }

    @Test("mosh + tmux: tmux's refusal of the first session is the controller's notice")
    func moshNewSessionRefused() async throws {
        let clock = VirtualClock()
        let channel = ShellDatagramChannel()
        let rig = makeRig(clock: clock, channel: channel, refuse: "duplicate session: work")
        await rig.start()
        #expect(await rig.eventually { clock.sleeping == 1 })
        await run(clock, for: .seconds(6), step: .milliseconds(250))
        #expect(await rig.eventually { rig.session.awaitingFirstSession })
        let control = try #require(rig.session.moshControl)
        control.newSession(named: "work")
        #expect(await rig.eventually { control.notice == "duplicate session: work" })
        #expect(rig.session.awaitingFirstSession)
        await rig.session.stop()
    }
}
