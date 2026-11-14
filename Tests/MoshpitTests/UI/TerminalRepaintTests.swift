import Foundation
import SwiftUI
import Testing
import UIKit
import SwiftTerm
@testable import Moshpit

/// What the app itself asks SwiftTerm to repaint.
///
/// SwiftTerm repaints only the rows that changed (fork patch 27): one echoed
/// key is about 0.5 ms of main thread on a 70×72 iPhone grid instead of
/// about 5 ms for the whole screen. The app gave that back in two ways:
/// re-applying a theme that had not changed (every colour setter repaints
/// the whole screen, and the theme is applied each time a terminal screen
/// appears, to every tmux pane), and forcing a whole-screen repaint itself on
/// every key sent, on a globe-key switch, on a pane switch and on a return
/// from the background.
@Suite("terminal repaints")
@MainActor
struct TerminalRepaintTests {

    /// A terminal view that counts what it is asked to redraw.
    final class CountingTerminalView: TerminalView {
        private(set) var invalidated: [CGRect] = []

        override func setNeedsDisplay(_ rect: CGRect) {
            invalidated.append(rect)
            super.setNeedsDisplay(rect)
        }

        override func setNeedsDisplay() {
            invalidated.append(.infinite)
            super.setNeedsDisplay()
        }

        func forget() {
            invalidated = []
            getTerminal().clearUpdateRange()
        }
    }

    private func makeView() -> CountingTerminalView {
        CountingTerminalView(frame: CGRect(x: 0, y: 0, width: 390, height: 600),
                             font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular))
    }

    /// Let SwiftTerm's queued display pass (one frame later) run.
    private func settle() async {
        try? await Task.sleep(for: .milliseconds(80))
    }

    private func coversScreen(_ view: TerminalView) -> Bool {
        guard let range = view.getTerminal().getUpdateRange() else { return false }
        return range.startY <= 0 && range.endY >= view.getTerminal().rows - 1
    }

    private func same(_ a: UIColor, _ b: SwiftUI.Color) -> Bool {
        var ar: CGFloat = 0, ag: CGFloat = 0, ab: CGFloat = 0, aa: CGFloat = 0
        var br: CGFloat = 0, bg: CGFloat = 0, bb: CGFloat = 0, ba: CGFloat = 0
        a.getRed(&ar, green: &ag, blue: &ab, alpha: &aa)
        UIColor(b).getRed(&br, green: &bg, blue: &bb, alpha: &ba)
        return abs(ar - br) < 0.002 && abs(ag - bg) < 0.002 && abs(ab - bb) < 0.002 && abs(aa - ba) < 0.002
    }

    // MARK: - Theme

    @Test("Applying the theme a view already has repaints nothing")
    func sameThemeRepaintsNothing() async {
        let view = makeView()
        TerminalTheme.githubDark.apply(to: view)
        await settle()
        view.forget()

        TerminalTheme.githubDark.apply(to: view)

        #expect(view.getTerminal().getUpdateRange() == nil,
                "an unchanged theme marked rows for repaint")
        await settle()
        #expect(view.invalidated.isEmpty, "an unchanged theme repainted \(view.invalidated)")
    }

    @Test("A different theme still repaints everything")
    func newThemeRepaints() async {
        let view = makeView()
        TerminalTheme.githubDark.apply(to: view)
        await settle()
        view.forget()

        TerminalTheme.dracula.apply(to: view)

        #expect(coversScreen(view))
        #expect(same(view.nativeBackgroundColor, TerminalTheme.dracula.background))
        #expect(same(view.nativeForegroundColor, TerminalTheme.dracula.foreground))
    }

    @Test("A theme that changes only one palette colour still installs it")
    func paletteOnlyChangeRepaints() async {
        let view = makeView()
        var theme = TerminalTheme.githubDark
        theme.apply(to: view)
        await settle()
        view.forget()

        theme.setAnsi("123456", for: .red)
        theme.apply(to: view)

        #expect(coversScreen(view), "the new red never reached the screen")
    }

    /// The remote may set the background itself (OSC 11). Applying the theme
    /// again still puts the theme's back.
    @Test("A background the remote set gives way to the theme")
    func remoteBackgroundGivesWay() async {
        let view = makeView()
        let theme = TerminalTheme.githubDark
        theme.apply(to: view)
        view.feed(text: "\u{1b}]11;rgb:ffff/0000/0000\u{7}")
        #expect(!same(view.nativeBackgroundColor, theme.background), "the remote's background took")

        theme.apply(to: view)

        #expect(same(view.nativeBackgroundColor, theme.background))
    }

    // MARK: - Fonts

    /// SwiftTerm's font setter repaints the whole screen, tries a resize and
    /// drops the selection whatever the value, and the tmux controller set
    /// the font on every pane each time a terminal screen appeared and on
    /// every reconnect. The selection is what the user sees go.
    @Test("Re-applying the appearance a tmux pane already has keeps its selection")
    func sameAppearanceKeepsPaneSelection() throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        controller.configureAppearance(theme: .githubDark, fontSize: 12, fontName: "system")
        let pane = controller.terminalView(for: "%1")
        pane.feed(text: "output the user is selecting\r\n")
        pane.selectAll()
        try #require(pane.selectionActive)

        controller.configureAppearance(theme: .githubDark, fontSize: 12, fontName: "system")

        #expect(pane.selectionActive, "the same font was set again, and the selection went with it")
        #expect(pane.font.pointSize == 12)
    }

    @Test("A new font size still reaches every tmux pane")
    func newFontSizeReachesPanes() {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        controller.configureAppearance(theme: .githubDark, fontSize: 12, fontName: "system")
        let pane = controller.terminalView(for: "%1")

        controller.configureAppearance(theme: .githubDark, fontSize: 15, fontName: "system")

        #expect(pane.font.pointSize == 15)
    }

    // MARK: - Keys

    @Test("A key sent to the remote repaints nothing until its echo comes back")
    func sentKeyRepaintsNothing() async {
        let view = makeView()
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(to: view)
        var sent: [Data] = []
        coordinator.onInput = { sent.append($0) }
        await settle()
        view.forget()

        // What SwiftTerm sends for a typed key: through its delegate.
        view.terminalDelegate?.send(source: view, data: ArraySlice("a".utf8))

        #expect(sent == [Data("a".utf8)])
        #expect(view.getTerminal().getUpdateRange() == nil, "a key marked every row for repaint")
        await settle()
        #expect(view.invalidated.isEmpty, "a key repainted \(view.invalidated)")
    }

    /// The globe key: tmux still resyncs the pane from the server (which
    /// repaints whatever really differs), but the view is not redrawn whole.
    @Test("Switching the input method resyncs without a whole-screen repaint")
    func inputModeSwitchResyncsOnly() async throws {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow()
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 600)
        let view = makeView()
        window.addSubview(view)
        window.makeKeyAndVisible()
        defer {
            _ = view.resignFirstResponder()
            window.isHidden = true
        }
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(to: view)
        var resyncs = 0
        coordinator.onInputModeChange = { resyncs += 1 }
        _ = view.becomeFirstResponder()
        try #require(view.isFirstResponder, "only the focused terminal answers the switch")
        try? await Task.sleep(for: .milliseconds(600))   // the keyboard comes up
        view.forget()

        NotificationCenter.default.post(name: UITextInputMode.currentInputModeDidChangeNotification,
                                        object: nil)

        #expect(view.getTerminal().getUpdateRange() == nil, "the switch marked every row for repaint")
        await settle()
        #expect(resyncs == 1)
        #expect(!view.invalidated.contains { $0.isInfinite || $0.size == view.bounds.size },
                "the switch repainted the whole view: \(view.invalidated)")
    }
}
