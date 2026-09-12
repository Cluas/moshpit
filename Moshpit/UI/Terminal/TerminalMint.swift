import SwiftTerm
import UIKit

/// The input/interaction configuration every minted terminal gets, whichever
/// site mints it (`SwiftTerminalView.makeUIView` for SSH single-pane,
/// `TmuxSessionController.mintTerminal` for tmux panes).
///
/// Extracted because the two sites carried twin comment blocks begging each
/// other to stay in sync — and one of them still drifted (the tmux site
/// missed the assistant-bar clearing for a day). One function, one test
/// (`GestureTopologyTests.mintConfiguration`), zero twins.
enum TerminalMint {
    @MainActor static func configureInput(_ terminalView: TerminalView) {
        // The app renders its own shortcut bar…
        terminalView.inputAccessoryView = nil
        // …and no system assistant bar either. On iPad the shortcuts/assistant
        // strip (undo · paste · autofill, 45–55pt) rides above the software
        // keyboard and, with a hardware keyboard, floats at the bottom right on
        // top of the app's own bar. SwiftTerm clears these in its
        // setupAccessoryView() path — which the line above opts out of, so the
        // clearing came along with the accessory we didn't want. iPhone has no
        // assistant bar; this is a no-op there.
        terminalView.inputAssistantItem.leadingBarButtonGroups = []
        terminalView.inputAssistantItem.trailingBarButtonGroups = []
        // A tap is a tap, not a keyboard grab. Reading history, tapping a
        // link, selecting output — none of those mean "I want to type", and on
        // iOS focus IS the keyboard. What still raises it: the shortcut bar's
        // toggle, Settings' raise-on-open, and a tap landing on the cursor's
        // own rows — the input box — which TerminalScrollGesture.handleTap
        // detects and answers with a programmatic becomeFirstResponder() this
        // flag does not gate. (fork patch 15)
        terminalView.focusOnTap = false
        TerminalKeyboard.enableComposingInput(on: terminalView)
        TerminalScrollback.enlarge(terminalView)
        // Only underline/open REAL hyperlinks the program declared via OSC-8.
        // SwiftTerm's default `.implicit` also runs a heuristic regex that
        // mis-underlines bare file/relative paths (src/foo, ./build, ~/x) and
        // can truncate real URLs at certain chars — so it's off. Moshpit's own
        // plain-URL detection (PlainLinkDetector) covers the rest precisely.
        terminalView.linkReporting = .explicit
        terminalView.linkHighlightMode = .always   // OSC-8 links open on a plain tap
        // The app owns scrolling (gestures + scroll thumb), so SwiftTerm must
        // NOT also report touches as mouse drags — they leak to the remote
        // during a scroll and (over mosh) toggle copy-mode out from under us.
        terminalView.allowMouseReporting = false
    }
}
