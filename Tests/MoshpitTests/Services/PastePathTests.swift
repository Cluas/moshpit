import Foundation
import Testing
import UIKit
@testable import Moshpit

/// The paste roads — tmux's paste buffer, the terminal view's own Paste and
/// ⌘V — arrive with no paste marker inside the paste. None of them removed
/// any. (A plain shell's, on the Rust core, is crates/session's.)
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
    /// handle: `combining`, a marker whose `~` carries
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

    // MARK: - The terminal view's own Paste and ⌘V

    /// The core's view and its coordinator; `paste` is what the view's Paste
    /// and ⌘V do with the clipboard's text.
    private func terminal() -> (CoreTerminalView, SwiftTerminalView.Coordinator) {
        let surface = TerminalSurfaces.make(font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular),
                                            grid: TerminalGrid(cols: 40, rows: 10))
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(surface)
        return (surface as! CoreTerminalView, coordinator)
    }

    private func paste(_ text: String, on view: CoreTerminalView) {
        view.surfaceDelegate?.surface(view, paste: text)
    }

    /// No session paste path installed: the coordinator brackets it by the
    /// program's mode, with the markers in the text taken out.
    @Test("Terminal view: Paste with no session path sends one paste, markers gone",
          arguments: [combining, assembled, bare])
    func terminalViewPaste(_ pasted: String) {
        let (view, coordinator) = terminal()
        var sent = Data()
        coordinator.onInput = { sent.append($0) }
        coordinator.feed(data: Data("\u{1B}[?2004h".utf8))
        #expect(view.settle { _ in coordinator.modes?.bracketedPaste == true })

        paste(pasted, on: view)

        #expect(sent == Data((Self.start + Self.cleaned[pasted]! + Self.end).utf8))
    }

    @Test("Terminal view: Paste outside bracketed mode sends the text, markers gone")
    func terminalViewPasteUnbracketed() {
        let (view, coordinator) = terminal()
        var sent = Data()
        coordinator.onInput = { sent.append($0) }
        #expect(coordinator.modes?.bracketedPaste != true)

        paste(Self.bare, on: view)

        #expect(sent == Data(Self.cleaned[Self.bare]!.utf8))
    }

    @Test("Terminal view: with a session path, Paste hands it the text and sends nothing itself")
    func terminalViewPasteToSession() {
        let (view, coordinator) = terminal()
        var sent = Data()
        var handed: [String] = []
        coordinator.onInput = { sent.append($0) }
        coordinator.onPaste = { handed.append($0) }

        paste(Self.bare, on: view)

        #expect(handed == [Self.bare])
        #expect(sent.isEmpty)
    }

}
