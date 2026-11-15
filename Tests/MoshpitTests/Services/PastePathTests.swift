import Foundation
import Testing
import UIKit
@testable import Moshpit

/// The terminal view's own Paste and ⌘V take the session's paste path when
/// it has one, and send nothing themselves. (What a paste sends with no
/// session path — bracketed as the program asked, never a marker inside —
/// is the core's: crates/term `input.rs`
/// `markers_leave_whatever_follows_them_and_whatever_the_mode`, `view.rs`
/// `v30_…` and `b574_…`.)
@Suite("paste paths", .serialized)
@MainActor
struct PastePathTests {

    /// A crafted clipboard text: a marker inside it.
    static let bare = "ls\u{1B}[201~\ntouch INJECTED\n"

    // MARK: - The terminal view's own Paste and ⌘V

    /// The core's view and its coordinator; `paste` is what the view's Paste
    /// and ⌘V do with the clipboard's text.
    private func terminal() -> (CoreTerminalView, CoreTerminalHost.Coordinator) {
        let surface = TerminalSurfaces.make(font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular),
                                            grid: TerminalGrid(cols: 40, rows: 10))
        let coordinator = CoreTerminalHost.Coordinator()
        coordinator.attach(surface)
        return (surface as! CoreTerminalView, coordinator)
    }

    private func paste(_ text: String, on view: CoreTerminalView) {
        view.surfaceDelegate?.surface(view, paste: text)
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
