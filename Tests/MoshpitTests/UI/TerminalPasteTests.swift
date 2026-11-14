import Foundation
import SwiftUI
import Testing
import UIKit
@testable import Moshpit

/// The terminal's own paste — the edit menu's Paste, ⌘V on a hardware
/// keyboard — takes the session's paste path like every other paste: the
/// core's view hands the text to its delegate (``SwiftTerminalView/Coordinator``),
/// never out as typing.
@Suite("terminal paste")
@MainActor
struct TerminalPasteTests {
    private static let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)

    private func terminal() -> (CoreTerminalView, SwiftTerminalView.Coordinator) {
        let surface = TerminalSurfaces.make(font: Self.font, grid: TerminalGrid(cols: 40, rows: 10))
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(surface)
        return (surface as! CoreTerminalView, coordinator)
    }

    /// What the view's Paste and ⌘V do with the clipboard's text.
    private func paste(_ text: String, on view: CoreTerminalView) {
        view.surfaceDelegate?.surface(view, paste: text)
    }

    @Test("The view's paste goes to the coordinator's paste path, not out as typing")
    func pasteReachesThePastePath() {
        let (view, coordinator) = terminal()
        var pasted: [String] = []
        var typed: [Data] = []
        coordinator.onPaste = { pasted.append($0) }
        coordinator.onInput = { typed.append($0) }

        paste("git status\ngit diff", on: view)

        #expect(pasted == ["git status\ngit diff"])
        #expect(typed.isEmpty, "the paste also went out as keystrokes: \(typed)")
    }

    /// With no paste path installed the coordinator takes the paste itself:
    /// bracketed by the program's mode, the markers and the text in one
    /// write, a marker inside the text taken out.
    @Test("With no paste path the coordinator sends the paste itself, in one write")
    func noPastePathSendsOneWrite() {
        let (view, coordinator) = terminal()
        var typed: [Data] = []
        coordinator.onInput = { typed.append($0) }
        coordinator.feed(data: Data("\u{1b}[?2004h".utf8))
        #expect(view.settle { _ in coordinator.modes.bracketedPaste }, "the mode never reached the core")

        paste("a\nb", on: view)
        #expect(typed == [Data("\u{1b}[200~a\nb\u{1b}[201~".utf8)],
                "markers and text must leave as one write: \(typed.map { String(decoding: $0, as: UTF8.self).debugDescription })")
        typed = []
        paste("x\u{1b}[201~y", on: view)
        #expect(typed == [Data("\u{1b}[200~xy\u{1b}[201~".utf8)])
    }

    /// The tmux pane's coordinator is the controller's; the screen installs the
    /// paste path on it when it shows the pane.
    @Test("A shown tmux pane's paste reaches the screen's paste path")
    func paneHostInstallsThePastePath() async throws {
        let (view, coordinator) = terminal()
        var pasted: [String] = []
        let host = UIHostingController(rootView: PaneTerminalHost(
            surface: view, coordinator: coordinator, focusPolicy: .allow,
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

        paste("echo one\necho two", on: view)

        #expect(pasted == ["echo one\necho two"])
    }
}
