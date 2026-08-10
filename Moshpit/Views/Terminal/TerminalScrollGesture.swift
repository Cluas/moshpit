import UIKit
import SwiftTerm

/// One-finger swipe to scroll the scrollback + two-finger pinch to zoom the
/// font, on a SwiftTerm ``TerminalView``.
///
/// ### Why this is non-trivial
///
/// SwiftTerm owns a thicket of one-finger recognizers — single/double/triple
/// tap (focus, word/line select), a 0.7s long-press (start selection), and a
/// *selection* pan that it attaches ONLY while a selection is live. An earlier
/// one-finger scroll attempt fought tap + selection (typing and long-press
/// stopped working), which is why scroll was briefly moved to two fingers — but
/// two-finger scroll on a phone is unnatural and, worse, the recognizer was
/// never even attached to tmux panes (they're minted by the controller, not by
/// `SwiftTerminalView.makeUIView`), so it silently did nothing.
///
/// This version goes back to one finger and coexists cleanly:
///  - `gestureRecognizerShouldBegin` only lets the scroll pan start when the
///    buffer actually has scrollback (`canScroll` — false on the alternate
///    screen, so full-screen apps keep their own pan), no text selection is
///    active (SwiftTerm's selection pan handles that case), and the drag is
///    vertical. A motionless press never moves, so tap + long-press still fire.
///  - `cancelsTouchesInView = false` — the recognizer never swallows the
///    touches the terminal needs.
///
/// ### Reading while output streams
///
/// SwiftTerm's emulator-level "user is scrolled up" flag is never set, so every
/// new line snaps the viewport to the bottom — you could never read an agent's
/// live output. We can't reach that flag, so instead the ``Coordinator`` HOLDS
/// incoming output while the user is scrolled up (see `engageScrollHold`) and
/// flushes it the instant they return to the bottom or type. This gesture is
/// what tells the coordinator which side of that line the viewport is on.
final class TerminalScrollGesture: NSObject, UIGestureRecognizerDelegate {
    private static var key: UInt8 = 0
    private weak var coordinator: SwiftTerminalView.Coordinator?
    private var pinchBaseSize: CGFloat = 13

    /// Axis a one-finger pan committed to, decided once it has moved enough.
    private enum Axis { case vertical, horizontal }
    private var axisLock: Axis?

    /// Attach once per terminal; idempotent. Retained via an associated object
    /// so its lifetime matches the terminal (gesture targets are held weakly).
    /// `coordinator` is the data-input owner whose output-hold this drives.
    static func attach(to terminal: TerminalView, coordinator: SwiftTerminalView.Coordinator) {
        guard objc_getAssociatedObject(terminal, &key) == nil else { return }
        let handler = TerminalScrollGesture()
        handler.coordinator = coordinator
        terminal.isMultipleTouchEnabled = true   // required for the 2-finger pinch

        // One one-finger pan handles BOTH axes: it locks to the dominant axis
        // from accumulated translation (not velocity, which an automated/jittery
        // swipe reports unreliably at the start), then routes — vertical = scroll,
        // horizontal = switch pane/window. A single recognizer avoids two pans
        // racing for the same touch.
        let pan = UIPanGestureRecognizer(target: handler, action: #selector(handlePan(_:)))
        pan.minimumNumberOfTouches = 1
        pan.maximumNumberOfTouches = 1
        pan.cancelsTouchesInView = false
        pan.delegate = handler
        terminal.addGestureRecognizer(pan)

        let pinch = UIPinchGestureRecognizer(target: handler, action: #selector(handlePinch(_:)))
        pinch.cancelsTouchesInView = false
        pinch.delegate = handler
        terminal.addGestureRecognizer(pinch)

        // The built-in scroll-view pan reveals blank space (SwiftTerm draws the
        // current buffer view, not the scrolled content), so leave it off.
        terminal.panGestureRecognizer.isEnabled = false
        objc_setAssociatedObject(terminal, &key, handler, .OBJC_ASSOCIATION_RETAIN)
    }

    @objc private func handlePan(_ gesture: UIPanGestureRecognizer) {
        guard let terminal = gesture.view as? TerminalView else { return }
        switch gesture.state {
        case .began:
            axisLock = nil
            // Burst start: let tmux refresh whether the active pane's app wants
            // the mouse, so the wheel-vs-copy-mode decision is fresh for a scroll.
            coordinator?.scrollWillBegin()
        case .changed:
            let t = gesture.translation(in: terminal)
            // Commit to an axis once the drag is unambiguous (~12pt), then stick
            // with it for the rest of the gesture.
            if axisLock == nil, abs(t.x) >= 12 || abs(t.y) >= 12 {
                axisLock = abs(t.x) > abs(t.y) ? .horizontal : .vertical
            }
            guard axisLock == .vertical else { return }   // horizontal commits on .ended
            // Dragging down (positive y) pulls older content into view → the
            // coordinator routes to tmux copy-mode / wheel / local scrollback.
            let rows = max(terminal.getTerminal().rows, 1)
            let cellHeight = max(8, terminal.bounds.height / CGFloat(rows))
            let lines = Int(t.y / cellHeight)
            guard lines != 0 else { return }
            coordinator?.scroll(lines: lines)
            gesture.setTranslation(.zero, in: terminal)
        case .ended:
            if axisLock == .horizontal, abs(gesture.translation(in: terminal).x) >= 40,
               let onSwitch = coordinator?.onSwitch {
                // A deliberate horizontal travel commits a pane/window switch:
                // drag left (negative dx) = next, drag right = previous.
                Haptics.select()
                onSwitch(gesture.translation(in: terminal).x < 0)
            }
            axisLock = nil
        default:
            axisLock = nil
        }
    }

    @objc private func handlePinch(_ gesture: UIPinchGestureRecognizer) {
        guard let terminal = gesture.view as? TerminalView else { return }
        switch gesture.state {
        case .began:
            pinchBaseSize = terminal.font.pointSize
        case .changed:
            let target = min(max(pinchBaseSize * gesture.scale, 8), 32)
            if abs(target - terminal.font.pointSize) >= 0.5 {
                terminal.font = terminal.font.withSize(target)
            }
        case .ended, .cancelled, .failed:
            // Persist — otherwise the next appearance pass (theme change,
            // reconnect, even a snapshot-driven re-render) silently snaps the
            // size back to the settings value. Cancelled/failed (the system
            // interrupting the pinch mid-gesture — a call, notification,
            // backgrounding) left this uncommitted before: the live size
            // stuck until the next appearance pass quietly reverted it,
            // same bug class as the D-pad/scroll/hold-repeat gestures that
            // only handled `.ended`.
            coordinator?.onFontSizeCommit?(Double(terminal.font.pointSize))
        default:
            break
        }
    }

    // MARK: - UIGestureRecognizerDelegate

    /// Gate the one-finger scroll so it never steals taps, long-press selection,
    /// or full-screen-app drags.
    func gestureRecognizerShouldBegin(_ gesture: UIGestureRecognizer) -> Bool {
        guard let pan = gesture as? UIPanGestureRecognizer,
              pan.maximumNumberOfTouches == 1,
              let terminal = pan.view as? TerminalView else { return true }
        // A live selection: yield to SwiftTerm's selection pan (either axis).
        // `copy:` is permitted exactly when a selection is active.
        if terminal.canPerformAction(#selector(UIResponder.copy(_:)), withSender: nil) {
            return false
        }
        // Begin if EITHER a scroll or a switch can consume the drag; ``handlePan``
        // locks the axis and routes. Scroll is possible when:
        //  - a tmux pane (onScroll set): copy-mode, or a wheel forwarded to a
        //    mouse app — both work with no local scrollback;
        //  - a local mouse app is on screen (Claude Code / vim --mouse over plain
        //    SSH / degraded mosh), even on the alternate screen (canScroll false);
        //  - a plain shell has local scrollback (canScroll).
        // Switch is possible whenever there are panes/windows to cycle (onSwitch).
        let canScroll = coordinator?.onScroll != nil
            || (coordinator?.localAppWantsMouse ?? false)
            || terminal.canScroll
        let canSwitch = coordinator?.onSwitch != nil
        return canScroll || canSwitch
    }

    func gestureRecognizer(_ gesture: UIGestureRecognizer,
                           shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        true
    }
}
