import Foundation
import Testing
@testable import Moshpit

/// herdr's native frame channel over SSH, driven through a real
/// `ActiveSession.start()`: the PTY's program is the reader loop around
/// `herdr terminal session control`,
/// so what comes out of it must be parsed (never painted), and what goes
/// into it must be protocol lines (never the user's raw keystrokes).
@Suite("herdr frame mode over SSH")
@MainActor
struct HerdrSSHFrameModeTests {

    private func connection(_ transport: ConnectionProtocol) -> ServerConnection {
        var connection = ServerConnection(
            name: "herdr-frames",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.connectionProtocol = transport
        connection.multiplexer = .herdr
        // The user vouches for the binary: no probe decides the multiplexer.
        connection.herdrPath = "/usr/local/bin/herdr"
        return connection
    }

    private func makeSession(_ connection: ServerConnection, host: some SSHClientTransport) -> SessionHub.ActiveSession {
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        return SessionHub.ActiveSession(connection: connection, dial: script.dialer)
    }

    /// The probe result lands in the shared capability cache, which mirrors
    /// into the standard defaults; keep the suite from leaving it behind.
    private func forgetProbe(_ connection: ServerConnection) {
        UserDefaults.standard.removeObject(forKey: "hostcaps." + connection.id.uuidString)
    }

    private func waitUntil(_ done: () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(10)
        while !done() {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    // MARK: - One reader (mosh falling back to SSH)

    /// The trigger: a Mosh + herdr connection to a host without mosh-server.
    /// The fallback reused the authenticated SSH session, started a plain
    /// output pump on its PTY — and then the frame pump on top of it, without
    /// stopping the first. Two readers split one stream chunk by chunk:
    /// frame JSON painted into the terminal as text, and the frames the plain
    /// pump took never reached the parser.
    @Test("Mosh with no mosh-server falls back to SSH with one reader: frames parsed, never painted raw")
    func fallbackHasOneReader() async throws {
        let connection = connection(.mosh)
        defer { forgetProbe(connection) }
        let host = HerdrFrameHost()
        let session = makeSession(connection, host: host)

        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.degrade != nil, "fell back for the missing mosh-server")
        #expect(session.viewModel.status == .connected)
        #expect(host.ptyOpen)

        // A dozen frames, each its own chunk, as a PTY delivers them.
        let payloads = (0..<12).map { Data("\u{1b}[1;1Hframe \($0)".utf8) }
        for (seq, bytes) in payloads.enumerated() {
            host.emit(Data(HerdrFrameHost.frameLine(seq: seq, bytes: bytes).utf8))
            try? await Task.sleep(for: .milliseconds(5))
        }
        _ = await waitUntil { session.coordinator.unattachedFeed.count >= payloads.count }
        try? await Task.sleep(for: .milliseconds(50))   // anything else still in flight

        let fed = session.coordinator.unattachedFeed
        #expect(!fed.contains { String(decoding: $0, as: UTF8.self).contains("terminal.frame") },
                "frame JSON was painted into the terminal as text")
        #expect(fed == payloads, "every frame decoded, in order, and nothing else")
        await session.stop()
    }

    // MARK: - No raw typing into the frame loop

    /// With no pane attached — before the first attach, backing off another
    /// client, after a refusal — `herdrFrameTarget` is nil, and typing used
    /// to fall through to the plain-shell branch: raw bytes into the PTY,
    /// which is the frame loop's stdin. Its `read` took them and, the tty
    /// being raw, they prefixed the next retarget line, so that attach failed
    /// too.
    @Test("Typing before the frame channel attaches never reaches the PTY, and says so")
    func typingBeforeAttachIsDroppedWithANotice() async throws {
        let connection = connection(.ssh)
        defer { forgetProbe(connection) }
        // The first snapshot read is held: the channel can't attach yet.
        let host = HerdrFrameHost(snapshot: HerdrFrameHost.onePane, holdSnapshots: true)
        let session = makeSession(connection, host: host)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.connState == .live)
        #expect(host.ptyOpen)
        let early = Data("typed-early".utf8)

        session.sendInput(early)
        try? await Task.sleep(for: .milliseconds(150))

        #expect(!host.writtenText.contains("typed-early"),
                "typed bytes went raw into the frame loop's stdin")
        #expect(session.herdrNotice != nil, "the user is told the input went nowhere")

        // The poll names the focused pane; the channel boots onto it.
        host.releaseSnapshots()
        #expect(await waitUntil { host.writtenText.contains("w1:p1 ") }, "the loop never got its first pane")
        // Its attach isn't up until its first frame, and herdr drops what
        // reaches it before then (B185): nothing goes, nobody picked it.
        let beforeItsFrame = Data("typed-before-its-frame".utf8)
        session.sendInput(beforeItsFrame)
        try? await Task.sleep(for: .milliseconds(150))
        #expect(!host.writtenText.contains(beforeItsFrame.base64EncodedString()),
                "typing was written into an attach that wasn't up")

        // The first painted frame: the attach is up, the notice comes down.
        host.emit(Data(HerdrFrameHost.frameLine(seq: 1, bytes: Data("ok".utf8), full: true).utf8))
        #expect(await waitUntil { session.herdrNotice == nil })
        let late = Data("pwd".utf8)
        session.sendInput(late)
        #expect(await waitUntil { host.writtenText.contains(HerdrFrameCommand.input(late)) },
                "attached, typing rides the frame channel as protocol lines")
        #expect(!host.writtenText.contains(early.base64EncodedString()),
                "what was dropped stays dropped — not replayed into whatever pane attached")
        #expect(!host.writtenText.contains(beforeItsFrame.base64EncodedString()))
        await session.stop()
    }

    // MARK: - Typing during a pane switch

    /// A herdr host for a switch: two panes (`HerdrCommandOrderTests`'
    /// snapshot), the focus moving when `agent focus` lands, the PTY's output
    /// driven by the test and everything written into it recorded.
    final class SwitchHost: SSHClientTransport, @unchecked Sendable {
        private let lock = NSLock()
        private var _output: (@Sendable (Data) -> Void)?
        private var _written: [Data] = []
        private var _focused = "w1:p1"

        var writtenText: String {
            lock.withLock { _written }.map { String(bytes: $0, encoding: .utf8) ?? "" }.joined()
        }

        func emit(_ data: Data) {
            let output = lock.withLock { _output }
            output?(data)
        }

        /// The desktop moves the server's focus; the next read reports it.
        func focus(_ pane: String) { lock.withLock { _focused = pane } }

        func run(_ command: String) async throws -> Data {
            if command == HostCapabilities.probeCommand {
                return Data("/usr/local/bin/herdr\n::Linux::/usr/bin/apt-get\n".utf8)
            }
            if let target = HerdrCommandOrderTests.FocusServer.focusTarget(command) {
                lock.withLock { _focused = target }
                return Data()
            }
            guard command.contains("api snapshot") else { return Data() }
            let focused = lock.withLock { _focused }
            return Data(HerdrCommandOrderTests.FocusServer.snapshot(focused: focused).utf8)
        }

        final class Writer: SSHPTYWriter, @unchecked Sendable {
            private weak var host: SwitchHost?
            init(host: SwitchHost) { self.host = host }
            func write(_ data: Data) async throws { host?.record(data) }
            func resize(cols: Int, rows: Int) async throws {}
        }

        private func record(_ data: Data) { lock.withLock { _written.append(data) } }

        func openPTY(rows: Int, cols: Int,
                     onOutput: @escaping @Sendable (Data) -> Void,
                     onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
            lock.withLock { _output = onOutput }
            return Writer(host: self)
        }

        func shutdown() async {}
    }

    /// The channel booted onto w1:p1, its loop up and the pane painted.
    private func painted(_ host: SwitchHost, _ connection: ServerConnection) async throws -> SessionHub.ActiveSession {
        let session = makeSession(connection, host: host)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.connState == .live)
        // The loop is the channel's program (B267): it says it reads, then
        // gets its first pane as one word.
        host.emit(Data((HerdrFrameCommand.readyMarker + "\n").utf8))
        #expect(await waitUntil { host.writtenText.contains("w1:p1 ") }, "the loop never got its first pane")
        let painted = Data("on p1".utf8)
        host.emit(Data(HerdrFrameHost.frameLine(seq: 1, bytes: painted, full: true).utf8))
        #expect(await waitUntil { session.coordinator.unattachedFeed.contains(painted) })
        #expect(session.herdrNotice == nil)
        return session
    }

    /// A switch writes `release`, waits for the old attach to let go, then
    /// writes the new target, whose attach is up at its first frame. Typing
    /// went straight onto the lane: before the release it reached the pane
    /// being LEFT; between the release and the target the frame loop's
    /// `read` took it for a command and dropped it; behind the target line
    /// herdr dropped it, its attach not up yet (B185). Typed at the pane the
    /// person picked, it now waits for that pane's first frame and follows it.
    @Test("Typing while the channel switches to the pane picked reaches that pane, in order, after its first frame")
    func typingDuringAPickedSwitchFollowsTheAttach() async throws {
        let connection = connection(.ssh)
        defer { forgetProbe(connection) }
        let host = SwitchHost()
        let session = try await painted(host, connection)

        let control = try #require(session.herdrControl)
        control.selectPane("w1:p2")
        let atTheSwitch = Data("typed-at-the-switch".utf8)
        session.sendInput(atTheSwitch)
        // The release goes out; the old attach hasn't let go yet.
        #expect(await waitUntil { host.writtenText.contains(HerdrFrameCommand.release) })
        let afterRelease = Data("typed-after-the-release".utf8)
        session.sendInput(afterRelease)
        // The old attach lets go; the new target follows.
        host.emit(Data((#"{"type":"terminal.closed"}"# + "\n").utf8))
        #expect(await waitUntil { host.writtenText.contains("w1:p2 ") }, "the switch never wrote its target")
        let beforeItsFrame = Data("typed-before-its-first-frame".utf8)
        session.sendInput(beforeItsFrame)
        try? await Task.sleep(for: .milliseconds(150))
        for typed in [atTheSwitch, afterRelease, beforeItsFrame] {
            #expect(!host.writtenText.contains(HerdrFrameCommand.input(typed)),
                    "went out before the new pane's attach was up: \(String(decoding: typed, as: UTF8.self))")
        }
        #expect(session.herdrNotice == nil, "typing at the pane just picked was dropped")

        // Its first frame: everything typed at it follows, in order, behind
        // its attach line.
        host.emit(Data(HerdrFrameHost.frameLine(seq: 2, bytes: Data("on p2".utf8), full: true).utf8))
        #expect(await waitUntil { host.writtenText.contains(HerdrFrameCommand.input(beforeItsFrame)) })
        let text = host.writtenText
        var cursor = try #require(text.range(of: "w1:p2 ")).upperBound
        for typed in [atTheSwitch, afterRelease, beforeItsFrame] {
            let line = try #require(text.range(of: HerdrFrameCommand.input(typed), range: cursor..<text.endIndex),
                                    "out of order: \(String(decoding: typed, as: UTF8.self))")
            cursor = line.upperBound
        }
        let typed = Data("typed-on-p2".utf8)
        session.sendInput(typed)
        #expect(await waitUntil { host.writtenText.contains(HerdrFrameCommand.input(typed)) })
        await session.stop()
    }

    /// A switch the person didn't make — the desktop moved the focus, the
    /// poll followed — holds nothing: "y⏎" typed at one agent must never
    /// answer another's prompt. Dropped, and said, until the new pane is up.
    @Test("Typing while the channel follows a focus nobody here picked is dropped, said, never sent on")
    func typingDuringAnUnpickedSwitchIsDropped() async throws {
        let connection = connection(.ssh)
        defer { forgetProbe(connection) }
        let host = SwitchHost()
        let session = try await painted(host, connection)
        let control = try #require(session.herdrControl)

        host.focus("w1:p2")
        control.refresh()
        #expect(await waitUntil { host.writtenText.contains(HerdrFrameCommand.release) })
        let atTheSwitch = Data("typed-at-the-switch".utf8)
        session.sendInput(atTheSwitch)
        host.emit(Data((#"{"type":"terminal.closed"}"# + "\n").utf8))
        #expect(await waitUntil { host.writtenText.contains("w1:p2 ") }, "the switch never wrote its target")
        let beforeItsFrame = Data("typed-before-its-first-frame".utf8)
        session.sendInput(beforeItsFrame)
        #expect(session.herdrNotice != nil, "the keys were dropped without a word")

        host.emit(Data(HerdrFrameHost.frameLine(seq: 2, bytes: Data("on p2".utf8), full: true).utf8))
        #expect(await waitUntil { session.herdrNotice == nil })
        let typed = Data("typed-on-p2".utf8)
        session.sendInput(typed)
        #expect(await waitUntil { host.writtenText.contains(HerdrFrameCommand.input(typed)) })
        #expect(!host.writtenText.contains(HerdrFrameCommand.input(atTheSwitch)))
        #expect(!host.writtenText.contains(HerdrFrameCommand.input(beforeItsFrame)),
                "written behind the attach line, before it was up: herdr drops it")
        await session.stop()
    }

    // MARK: - Pastes

    /// Whether a paste was bracketed came from the LOCAL emulator's mode, and
    /// herdr's frames carry no mode changes — so that never turned on, and
    /// every paste went out bare: a multi-line prompt ran line by line, a
    /// shared image path read as typing. herdr's server re-encodes an input
    /// that is one complete bracketed paste against the pane's real mode, so
    /// that is what goes out — for the active pane and for one named pane.
    @Test("A paste reaches herdr as one bracketed paste, for the server to fit to the pane")
    func pasteIsBracketedForTheServer() async throws {
        let connection = connection(.ssh)
        defer { forgetProbe(connection) }
        let host = HerdrFrameHost(snapshot: HerdrFrameHost.onePane)
        let session = makeSession(connection, host: host)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.connState == .live)
        // The loop is the channel's program (B267): it says it reads, then
        // gets its first pane as one word.
        host.emit(Data((HerdrFrameCommand.readyMarker + "\n").utf8))
        #expect(await waitUntil { host.writtenText.contains("w1:p1 ") }, "the loop never got its first pane")
        let prompt = Data("$ ".utf8)
        host.emit(Data(HerdrFrameHost.frameLine(seq: 1, bytes: prompt, full: true).utf8))
        #expect(await waitUntil { session.coordinator.unattachedFeed.contains(prompt) })

        let text = "first line\nsecond line"
        session.sendPaste(text)
        let bracketed = Data("\u{1b}[200~\(text)\u{1b}[201~".utf8)
        #expect(await waitUntil { host.writtenText.contains(HerdrFrameCommand.input(bracketed)) },
                "the paste went out bare")
        #expect(!host.writtenText.contains(HerdrFrameCommand.input(Data(text.utf8))))

        // To one pane — the Home card's image entry, the share queue.
        let path = "/tmp/moshpit-uploads/shot.png"
        #expect(await session.deliverPaste(path, toPane: "w1:p1"))
        let bracketedPath = Data("\u{1b}[200~\(path)\u{1b}[201~".utf8)
        #expect(await waitUntil { host.writtenText.contains(HerdrFrameCommand.input(bracketedPath)) },
                "the image path went out bare")
        await session.stop()
    }

    @Test("An end marker inside the pasted text can't end the paste early")
    func pastedEndMarkerIsDropped() {
        let payload = SessionHub.ActiveSession.herdrPaste("one\u{1b}[201~two")
        #expect(payload == Data("\u{1b}[200~onetwo\u{1b}[201~".utf8))
    }
}
