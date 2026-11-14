import Foundation
import Network
import Testing
@testable import Moshpit

/// The pid the renderer's line records: the login shell's, which the
/// `exec tmux attach` keeps.
private let rendererPid = 4711

/// B212: mosh + tmux on a host that has no tmux session yet.
///
/// The control plane (the `-CC` sidecar on its own SSH connection) gets about
/// six seconds to land on a session, and the renderer — tmux's own client,
/// typed into the mosh shell — attaches the one it landed on. With no
/// session there the sidecar's chain keeps retrying, but the renderer gave
/// up for good: the session the person made next showed in the tree, while
/// the screen stayed a bare shell. crates/session's rule since W10b: the
/// renderer waits for a session and attaches the first there is; Moshpit
/// makes none of its own (the empty state's "New session" is the person's).
///
/// The whole path runs for real over fakes: the hub's start, mosh's
/// bootstrap, a `MoshTransport` whose datagrams are read back with the
/// session key (what mosh-server would type into the shell), the sidecar's
/// tmux answering after a round trip. The attach's windows and waits run on
/// a virtual clock.
@Suite("mosh+tmux: the renderer attaches a session that appears later (B212)")
@MainActor
struct MoshTmuxLateSessionTests {

    /// A mosh + tmux host: the probe finds tmux and mosh-server; the
    /// sidecar's tmux has a session or not, and lists the phone's renderer
    /// (pid ``rendererPid``) and the sidecar itself among its clients.
    private func makeRig(hasSession: Bool, clock: VirtualClock,
                         channel: ShellDatagramChannel) -> MoshSessionRig {
        let rig = MoshSessionRig(multiplexer: .tmux, answer: { command in
            if command.contains("command -v tmux mosh-server") {
                return .output("/usr/bin/tmux\n/usr/bin/mosh-server\n::Linux::/usr/bin/apt-get\n")
            }
            return MoshSessionRig.answer(command)
        }, transport: { credentials, _ in
            // Never hears the server: no return-path verdict during a test.
            try MoshTransport(credentials: credentials, channel: channel,
                              returnPathDeadlineNanos: 600 * 1_000_000_000)
        })
        rig.reaper.tmux = .init(hasSession: hasSession, answerDelay: .milliseconds(40),
                                clients: ["0|\(rendererPid)|$0|/dev/pts/4", "1|5150|$0|client-5150"])
        let reaper = rig.reaper
        rig.reaper.answer { command in
            if command.contains("cat "), command.contains(".moshpit/tmuxr-") {
                return .output("\(rendererPid)\n")
            }
            // What tmux answers the hub's question (B228).
            if command.contains("has-session 2>/dev/null; echo $?") {
                return .output(reaper.tmux?.hasSession == true ? "0\n" : "1\n")
            }
            return .output("")
        }
        rig.session.moshAttachClock = clock
        rig.session.attachTimeoutSeconds = 0.5
        return rig
    }

    /// Move `clock` forward `total` in `step`s, letting the attach take its
    /// turn after each: it sleeps on the clock again (or is done) before the
    /// next step.
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

    private static let forbidden = ["new-session", "-CC new", "tmux new"]

    /// The renderer mosh-server was started with (B267): its program, after
    /// `--` on the bootstrap's command line.
    private func rendererProgram(_ rig: MoshSessionRig) throws -> String {
        let bootstrap = try #require(rig.host.commands.first { $0.contains("mosh-server new") })
        let program = try #require(bootstrap.components(separatedBy: " -- ").last)
        return try #require(PosixShell.shPayload(String(program.dropFirst("/bin/".count))))
    }

    @Test("with no session yet, the renderer waits for one and attaches the session the person makes")
    func rendererAttachesALateSession() async throws {
        let clock = VirtualClock()
        let channel = ShellDatagramChannel()
        let rig = makeRig(hasSession: false, clock: clock, channel: channel)
        await rig.start()
        #expect(rig.session.viewModel.status == .connected)
        // The renderer is mosh-server's own program: it waits for a session
        // itself and attaches the first there is; the pid recorded first.
        let program = try rendererProgram(rig)
        #expect(program.hasPrefix("mkdir -p \"$HOME/.moshpit\"; echo $$ > \"$HOME/.moshpit/tmuxr-\(rig.connection.id.uuidString)-"),
                "the pid recorded before the wait")
        #expect(program.hasSuffix("until tmux has-session 2>/dev/null; do sleep 1; done; exec tmux attach"))

        // The control plane is up, its chain retrying the attach: nothing to
        // land on.
        #expect(await rig.eventually { rig.reaper.retryingAttach })
        #expect(await rig.eventually { clock.sleeping == 1 }, "the attach waits on its clock")

        // Its window passes.
        await run(clock, for: .seconds(6), step: .milliseconds(250))
        // A server with no session isn't a stalled attach (B228): the card
        // waits for the person's first session, past the (shortened) bound.
        #expect(await rig.eventually { rig.session.awaitingFirstSession })
        try? await Task.sleep(for: .milliseconds(700))
        #expect(!rig.session.attachStalled, "the stalled notice over a server that merely has no session")

        // A while later the person makes a session.
        await run(clock, for: .seconds(30), step: .seconds(1))
        #expect(rig.session.moshControl?.snapshot.isAttached == false)
        rig.reaper.sessionAppears()
        #expect(await rig.eventually { rig.session.moshControl?.snapshot.isAttached == true })

        // The renderer attached by itself (its own loop); the control plane
        // learns which client it is by the pid the program recorded, and
        // follows it.
        clock.advance(by: .seconds(1))
        #expect(await rig.eventually { rig.session.moshControl?.rendererClient?.pid == rendererPid },
                "the control plane never learned which client is the phone's")
        let control = try #require(rig.session.moshControl)
        #expect(control.rendererClient == TmuxClientIdentity(pid: rendererPid, name: "/dev/pts/4"))
        #expect(control.rendererSessionId == "$0")
        #expect(rig.reaper.commands.contains { $0.contains("cat \"$HOME/.moshpit/tmuxr-\(rig.connection.id.uuidString)-") })
        // The card's "attach didn't complete" lifts over the tree it waited for.
        #expect(await rig.eventually { !rig.session.attachStalled })
        #expect(await rig.eventually { clock.sleeping == 0 }, "the attach is done")

        // Nothing anywhere made a session: not the renderer, not the control
        // plane, not an exec. And nothing was ever typed into the mosh shell.
        let everything = [channel.typed(), rig.reaper.typed] + rig.reaper.commands + rig.host.commands
        for word in Self.forbidden {
            #expect(!everything.contains { $0.contains(word) }, "\(word)")
        }
        #expect(channel.typed().isEmpty, "typed into the mosh shell: \(channel.typed().debugDescription)")
        await rig.session.stop()
    }

    /// B267 / B170: the renderer's attach line was typed into the mosh shell,
    /// up to four times while the attach wasn't seen; a copy that ran late
    /// was read by tmux's client as keystrokes in the pane. The renderer is
    /// mosh-server's program now, and nothing is typed.
    @Test("a server with a session: the renderer is mosh-server's program, adopted by its pid, nothing typed")
    func sessionAlreadyThere() async throws {
        let clock = VirtualClock()
        let channel = ShellDatagramChannel()
        let rig = makeRig(hasSession: true, clock: clock, channel: channel)
        await rig.start()
        let program = try rendererProgram(rig)
        #expect(program.hasSuffix("exec tmux attach"))
        #expect(await rig.eventually { rig.session.moshControl?.snapshot.isAttached == true })
        // The window's next look finds the control plane on `main`.
        #expect(await rig.eventually { clock.sleeping == 1 })
        clock.advance(by: .milliseconds(250))
        #expect(await rig.eventually { rig.session.moshControl?.rendererClient?.pid == rendererPid },
                "the control plane never learned which client is the phone's")
        #expect(await rig.eventually { clock.sleeping == 0 }, "the attach is done")
        #expect(channel.typed().isEmpty, "typed into the mosh shell: \(channel.typed().debugDescription)")
        await rig.session.stop()
    }

    @Test("a session stopped while its renderer waits stops waiting")
    func stopEndsTheWait() async throws {
        let clock = VirtualClock()
        let channel = ShellDatagramChannel()
        let rig = makeRig(hasSession: false, clock: clock, channel: channel)
        await rig.start()
        #expect(await rig.eventually { clock.sleeping == 1 })
        await run(clock, for: .seconds(6), step: .milliseconds(250))
        #expect(await rig.eventually { rig.session.awaitingFirstSession })
        #expect(await rig.eventually { clock.sleeping == 1 }, "waiting")

        await rig.session.stop()
        #expect(await rig.eventually { clock.sleeping == 0 }, "still waiting for a session that is gone")
        rig.reaper.sessionAppears()
        clock.advance(by: .seconds(5))
        try? await Task.sleep(for: .milliseconds(100))
        #expect(!rig.reaper.commands.contains { $0.contains(".moshpit/tmuxr-") && $0.contains("cat ") })
    }

    /// ^C at the renderer's wait hands the shell back: the control plane
    /// finds a session the person made some other way, but no client with
    /// the pid.
    @Test("a renderer that never attaches is let go after ten looks")
    func rendererThatNeverComes() async throws {
        let clock = VirtualClock()
        let channel = ShellDatagramChannel()
        let rig = makeRig(hasSession: false, clock: clock, channel: channel)
        rig.reaper.tmux?.clients = ["0|9001|$0|/dev/ttys002", "1|5150|$0|client-5150"]
        await rig.start()
        #expect(await rig.eventually { clock.sleeping == 1 })
        await run(clock, for: .seconds(6), step: .milliseconds(250))
        #expect(await rig.eventually { rig.session.awaitingFirstSession })
        rig.reaper.sessionAppears()
        #expect(await rig.eventually { rig.session.moshControl?.snapshot.isAttached == true })
        await run(clock, for: .seconds(12), step: .seconds(1))
        #expect(await rig.eventually { clock.sleeping == 0 }, "still looking for a renderer that isn't coming")
        #expect(rig.session.moshControl?.rendererClient == nil, "a desktop's client taken for the phone's")
        await rig.session.stop()
    }

    @Test("the remembered session rides in the renderer's program by its id")
    func rememberedSessionInTheProgram() async throws {
        let clock = VirtualClock()
        let channel = ShellDatagramChannel()
        let rig = makeRig(hasSession: true, clock: clock, channel: channel)
        TmuxSelectionStore.save(TmuxSelection(session: "$7", window: "@3", pane: "%9"),
                                for: rig.connection.id)
        defer { TmuxSelectionStore.save(nil, for: rig.connection.id) }
        await rig.start()
        let program = try rendererProgram(rig)
        #expect(program.contains("if tmux has-session -t '$7' 2>/dev/null; then exec tmux attach -t '$7'; fi; "))
        await rig.session.stop()
    }
}

// MARK: - The mosh shell, as mosh-server reads it

/// A datagram channel that keeps what the client sent (thread-safe: the
/// transport sends from its own executor) and never answers.
final class ShellDatagramChannel: DatagramChannel, @unchecked Sendable {
    private let lock = NSLock()
    private var datagrams: [Data] = []
    var state: NWConnection.State = .ready
    var onStateChange: (@Sendable (NWConnection.State) -> Void)?
    var onBetterPath: (@Sendable (Bool) -> Void)?
    func start() {}
    func restart() {}
    func cancel() {}
    func send(_ datagram: Data) { lock.withLock { datagrams.append(datagram) } }
    func receive(_ completion: @escaping @Sendable (Data?, Error?) -> Void) {}

    var sent: [Data] { lock.withLock { datagrams } }

    /// Everything typed into the mosh shell so far, as mosh-server would
    /// hand it to the shell: each instruction opened with the session key
    /// and its keystrokes applied to the state it names as its base; the
    /// newest state's bytes.
    func typed() -> String {
        guard let crypto = try? MoshCrypto(key: MoshSessionRig.key) else { return "" }
        var assembler = FragmentAssembler()
        var states: [UInt64: Data] = [0: Data()]
        var newest: UInt64 = 0
        for datagram in sent {
            guard let message = try? crypto.open(datagram),
                  let fragment = try? TransportFragment(parsing: message.payload),
                  let payload = assembler.receive(fragment),
                  let bytes = try? MoshCompression.decompress(payload),
                  let instruction = try? TransportInstruction(parsing: bytes),
                  states[instruction.newNum] == nil,
                  var state = states[instruction.oldNum],
                  let events = try? UserMessage(parsing: instruction.diff).events else { continue }
            for case .keystroke(let keys) in events { state.append(keys) }
            states[instruction.newNum] = state
            newest = max(newest, instruction.newNum)
        }
        return String(decoding: states[newest] ?? Data(), as: UTF8.self)
    }
}
