import Foundation
import SwiftUI
import Testing
import UIKit
import SwiftTerm
@testable import Moshpit

/// The terminal's own paste — the edit menu's Paste, ⌘V on a hardware
/// keyboard — takes the session's paste path like every other paste.
///
/// It used to go round it: SwiftTerm bracketed it by the local emulator's
/// `bracketedPasteMode`, which nothing ever sets over SSH+tmux (a `-CC`
/// attach does not replay the pane's modes) or SSH+herdr (its frames carry
/// none), so a multi-line paste ran line by line; and it sent the markers and
/// the text as three writes, which herdr cannot read as one paste.
@Suite("terminal paste")
@MainActor
struct TerminalPasteTests {

    private func makeTerminalView() -> TerminalView {
        TerminalView(frame: CGRect(x: 0, y: 0, width: 390, height: 600),
                     font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular))
    }

    @Test("The view's paste goes to the coordinator's paste path, not out as typing")
    func pasteReachesThePastePath() {
        let view = makeTerminalView()
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(to: view)
        var pasted: [String] = []
        var typed: [Data] = []
        coordinator.onPaste = { pasted.append($0) }
        coordinator.onInput = { typed.append($0) }

        view.paste(text: "git status\ngit diff")

        #expect(pasted == ["git status\ngit diff"])
        #expect(typed.isEmpty, "the paste also went out as keystrokes: \(typed)")
    }

    /// With no paste path installed the coordinator still takes the paste:
    /// SwiftTerm's own would wrap a marker inside the text as it is, ending
    /// the paste early. It brackets by the local flag, as SwiftTerm does.
    @Test("With no paste path the coordinator sends the paste itself, in one write")
    func noPastePathSendsOneWrite() {
        let view = makeTerminalView()
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(to: view)
        var typed: [Data] = []
        coordinator.onInput = { typed.append($0) }

        view.feed(text: "\u{1b}[?2004h")
        view.paste(text: "a\nb")

        #expect(typed == [Data("\u{1b}[200~a\nb\u{1b}[201~".utf8)],
                "markers and text must leave as one write: \(typed.map { String(decoding: $0, as: UTF8.self).debugDescription })")
        typed = []
        #expect(view.terminalDelegate?.pasteRequested(source: view, text: "x\u{1b}[201~y") == true,
                "SwiftTerm's own paste is never used")
        #expect(typed == [Data("\u{1b}[200~xy\u{1b}[201~".utf8)])
    }

    /// The tmux pane's coordinator is the controller's; the screen installs the
    /// paste path on it when it shows the pane.
    @Test("A shown tmux pane's paste reaches the screen's paste path")
    func paneHostInstallsThePastePath() async throws {
        let view = makeTerminalView()
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(to: view)
        var pasted: [String] = []
        let host = UIHostingController(rootView: PaneTerminalHost(
            surface: SwiftTermSurface.of(view), coordinator: coordinator, focusPolicy: .allow,
            onPaste: { pasted.append($0) }))
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow()
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 600)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        host.view.layoutIfNeeded()
        let deadline = ContinuousClock.now + .seconds(5)
        while coordinator.onPaste == nil, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }

        view.paste(text: "echo one\necho two")

        #expect(pasted == ["echo one\necho two"])
    }

    // MARK: - SSH+herdr, end to end

    private func herdrConnection() -> ServerConnection {
        var connection = ServerConnection(
            name: "herdr-paste",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        connection.connectionProtocol = .ssh
        connection.multiplexer = .herdr
        connection.herdrPath = "/usr/local/bin/herdr"
        return connection
    }

    private func waitUntil(_ done: () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(10)
        while !done() {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    /// herdr's frames never carry the pane's modes, so the local flag is
    /// always off; and herdr re-encodes an input only when it is exactly one
    /// complete bracketed paste. The terminal's own Paste has to arrive as
    /// that, like the paste chip's.
    @Test("Paste from the terminal reaches herdr as one bracketed paste")
    func herdrGetsOneBracketedPaste() async throws {
        let connection = herdrConnection()
        defer { UserDefaults.standard.removeObject(forKey: "hostcaps." + connection.id.uuidString) }
        let host = HerdrFrameHost(snapshot: HerdrFrameHost.onePane)
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let session = SessionHub.ActiveSession(connection: connection, dial: script.dialer)
        await session.start(theme: .githubDark, fontSize: 14)
        #expect(session.viewModel.connState == .live)
        // The loop is the channel's program (B267): it says it reads, then
        // gets its first pane as one word.
        host.emit(Data((HerdrFrameCommand.readyMarker + "\n").utf8))
        #expect(await waitUntil { host.writtenText.contains("w1:p1 ") }, "the loop never got its first pane")
        let prompt = Data("$ ".utf8)
        host.emit(Data(HerdrFrameHost.frameLine(seq: 1, bytes: prompt, full: true).utf8))
        #expect(await waitUntil { session.coordinator.unattachedFeed.contains(prompt) })

        let view = makeTerminalView()
        session.coordinator.attach(to: view)
        let text = "first line\nsecond line"
        view.paste(text: text)

        let bracketed = Data("\u{1b}[200~\(text)\u{1b}[201~".utf8)
        #expect(await waitUntil { host.writtenText.contains(HerdrFrameCommand.input(bracketed)) },
                "the paste did not reach herdr as one bracketed paste")
        #expect(!host.writtenText.contains(HerdrFrameCommand.input(Data(text.utf8))),
                "the paste went out bare")
        await session.stop()
    }
}
