import Foundation
import Testing
import UIKit
import SwiftTerm
@testable import Moshpit

/// The paste roads besides herdr's frame channel — tmux's paste buffer, a
/// plain SSH (or mosh) shell, the terminal view's own Paste and ⌘V — arrive
/// with no paste marker inside the paste. None of them removed any.
///
/// A program ends a bracketed paste at the first `ESC[201~` it reads; one
/// inside the text ended the paste early, and the next line ran (live, into
/// zsh, through tmux 3.6a's `paste-buffer -p` and a mosh 1.4.0 session). Each
/// check drives a path's own entry point with crafted clipboard text and
/// reads what reached the wire.
@Suite("paste paths", .serialized)
@MainActor
struct PastePathTests {

    static let start = "\u{1B}[200~"
    static let end = "\u{1B}[201~"

    /// Crafted clipboard texts, the ones herdr's paste had to learn to
    /// handle (`HerdrPasteTests`): `combining`, a marker whose `~` carries
    /// U+0301; `assembled`, `ESC[20` + a marker + `1~`; `bare`, a plain one.
    /// These paths removed none of them.
    static let combining = "ls\u{1B}[201~\u{301}\ntouch INJECTED\n"
    static let assembled = "\u{1B}[20\u{1B}[201~1~\ntouch INJECTED\n"
    static let bare = "ls\u{1B}[201~\ntouch INJECTED\n"

    /// What each must arrive as: the text with the markers gone.
    static let cleaned = [
        combining: "ls\u{301}\ntouch INJECTED\n",
        assembled: "\ntouch INJECTED\n",
        bare: "ls\ntouch INJECTED\n",
    ]

    private func occurrences(of marker: String, in data: Data) -> Int {
        let bytes = [UInt8](data), pattern = [UInt8](marker.utf8)
        guard bytes.count >= pattern.count else { return 0 }
        return (0...(bytes.count - pattern.count))
            .filter { Array(bytes[$0..<($0 + pattern.count)]) == pattern }.count
    }

    private func waitUntil(timeout: Duration = .seconds(10), _ done: () async -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while !(await done()) {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    // MARK: - tmux

    private func attachedController() async -> (TmuxSessionController, MockTmuxTransport) {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        await controller.attach()
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        _ = await waitUntil(timeout: .seconds(1)) { await transport.recordedCommands().count >= 3 }
        return (controller, transport)
    }

    /// tmux 3.6a brackets the buffer as it is: `paste-buffer -p` with a
    /// marker in the buffer ended the paste early in zsh.
    @Test("tmux: the paste buffer never holds a marker", arguments: [combining, assembled, bare])
    func tmuxPasteBuffer(_ pasted: String) async throws {
        let (controller, transport) = await attachedController()
        let baseline = await transport.recordedCommands().count

        controller.sendPaste(pasted, paneId: "%0")

        #expect(await waitUntil(timeout: .seconds(2)) {
            await transport.recordedCommands().count >= baseline + 2
        })
        let commands = await transport.recordedCommands().dropFirst(baseline)
            .map { $0.trimmingCharacters(in: .newlines) }
        let literal = TmuxSessionController.tmuxCommandStringLiteral(Self.cleaned[pasted]!)
        #expect(commands.first == "set-buffer -b moshpit-paste -- \(literal)")
        #expect(!(commands.first ?? "").contains("\\u001b[201~"), "an end marker went into the buffer")
        #expect(commands.dropFirst().first == "paste-buffer -p -b moshpit-paste -d -t %0")
    }

    @Test("tmux: text that is nothing but markers pastes nothing")
    func tmuxOnlyMarkers() async throws {
        let (controller, transport) = await attachedController()
        let baseline = await transport.recordedCommands().count

        controller.sendPaste("\(Self.end)\(Self.start)\u{1B}[20\(Self.end)1~", paneId: "%0")

        try? await Task.sleep(for: .milliseconds(100))
        #expect(await transport.recordedCommands().count == baseline)
    }

    // MARK: - The terminal view's own Paste and ⌘V

    private func terminal() -> (TerminalView, SwiftTerminalView.Coordinator) {
        let view = TerminalView(frame: CGRect(x: 0, y: 0, width: 400, height: 300),
                                font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular))
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(to: view)
        return (view, coordinator)
    }

    /// No session paste path installed: the view's paste used to be
    /// SwiftTerm's own, which wraps the text as it is.
    @Test("Terminal view: Paste with no session path sends one paste, markers gone",
          arguments: [combining, assembled, bare])
    func terminalViewPaste(_ pasted: String) {
        let (view, coordinator) = terminal()
        var sent = Data()
        coordinator.onInput = { sent.append($0) }
        coordinator.feed(data: Data("\u{1B}[?2004h".utf8))
        #expect(view.getTerminal().bracketedPasteMode)

        view.paste(text: pasted)

        #expect(sent == Data((Self.start + Self.cleaned[pasted]! + Self.end).utf8))
    }

    @Test("Terminal view: Paste outside bracketed mode sends the text, markers gone")
    func terminalViewPasteUnbracketed() {
        let (view, coordinator) = terminal()
        var sent = Data()
        coordinator.onInput = { sent.append($0) }
        #expect(!view.getTerminal().bracketedPasteMode)

        view.paste(text: Self.bare)

        #expect(sent == Data(Self.cleaned[Self.bare]!.utf8))
    }

    @Test("Terminal view: with a session path, Paste hands it the text and sends nothing itself")
    func terminalViewPasteToSession() {
        let (view, coordinator) = terminal()
        var sent = Data()
        var handed: [String] = []
        coordinator.onInput = { sent.append($0) }
        coordinator.onPaste = { handed.append($0) }

        view.paste(text: Self.bare)

        #expect(handed == [Self.bare])
        #expect(sent.isEmpty)
    }

    // MARK: - Plain SSH (the mosh path builds the same payload)

    /// A plain shell over SSH whose program turned bracketed paste on: the
    /// session wraps by the local emulator's flag, and wrapped the text with
    /// its marker inside.
    @Test("Plain SSH: a paste reaches the PTY as one paste, markers gone",
          arguments: [true, false])
    func plainSSH(bracketed: Bool) async throws {
        var connection = ServerConnection(name: "paste-plain", host: "192.0.2.1", port: 22,
                                          username: "tester", authMethod: .password)
        connection.connectionProtocol = .ssh
        connection.multiplexer = .none
        defer { UserDefaults.standard.removeObject(forKey: "hostcaps." + connection.id.uuidString) }
        let host = HerdrFrameHost()
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
        let view = TerminalView(frame: CGRect(x: 0, y: 0, width: 400, height: 300),
                                font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular))
        session.coordinator.attach(to: view)

        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.connState == .live)
        if bracketed {
            host.emit(Data("\u{1B}[?2004h".utf8))
            #expect(await waitUntil { view.getTerminal().bracketedPasteMode })
        }

        session.sendPaste(Self.bare)

        #expect(await waitUntil { host.writtenText.contains("touch INJECTED") })
        let written = Data(host.written.joined())
        let clean = Self.cleaned[Self.bare]!
        #expect(written == Data((bracketed ? Self.start + clean + Self.end : clean).utf8))
        #expect(occurrences(of: Self.end, in: written) == (bracketed ? 1 : 0))
        await session.stop()
    }
}
