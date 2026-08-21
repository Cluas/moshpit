import Foundation
import Testing
import UIKit
import SwiftTerm
@testable import Moshpit

/// The keyboard cover's one decision: is this move worth hiding?
///
/// It exists because a resize makes the REMOTE app redraw, and we would
/// otherwise paint the half-erased screen it goes through (measured against a
/// real Claude Code pane: tmux's own copy loses its footer for ~50ms). Hiding
/// that is worth a beat of frozen picture. Hiding NOTHING is not — switching
/// input method fires the same notification with an identical frame, and
/// covering it just makes the switch feel slow.
@Suite("keyboard cover")
@MainActor
struct KeyboardCoverTests {

    private func makeCoordinator() -> SwiftTerminalView.Coordinator {
        let coordinator = SwiftTerminalView.Coordinator()
        let terminal = TerminalView(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let container = TerminalHostContainer(frame: terminal.frame)
        container.host(terminal)
        coordinator.attach(to: terminal)
        coordinator.hostContainer = container
        // The notification handler ignores a terminal with no window that is
        // not first responder; a hosted one in a window passes.
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 800))
        window.addSubview(container)
        return coordinator
    }

    private func post(begin: CGRect, end: CGRect) {
        NotificationCenter.default.post(
            name: UIResponder.keyboardWillChangeFrameNotification, object: nil,
            userInfo: [
                UIResponder.keyboardFrameBeginUserInfoKey: NSValue(cgRect: begin),
                UIResponder.keyboardFrameEndUserInfoKey: NSValue(cgRect: end),
                UIResponder.keyboardAnimationDurationUserInfoKey: 0.25,
                UIResponder.keyboardAnimationCurveUserInfoKey: 7,
            ])
    }

    @Test("An input-method switch — same frame both ends — is not covered")
    func sameFrameIsNotCovered() {
        let coordinator = makeCoordinator()
        let up = CGRect(x: 0, y: 500, width: 320, height: 300)
        post(begin: up, end: up)
        #expect(coordinator.coverRequests == 0,
                "covering a move that changes nothing just makes it feel slow")
    }

    @Test("A keyboard actually arriving is covered")
    func realMoveIsCovered() {
        let coordinator = makeCoordinator()
        post(begin: CGRect(x: 0, y: 800, width: 320, height: 300),
             end: CGRect(x: 0, y: 500, width: 320, height: 300))
        #expect(coordinator.coverRequests == 1)
    }
}
