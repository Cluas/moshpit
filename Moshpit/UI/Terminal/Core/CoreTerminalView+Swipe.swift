import UIKit

// The sideways drag that switches panes, when the host moves the terminal
// with it (the NSH stage, ``TerminalSwipe``): reported from the first move
// past the axis lock to the lift, instead of one switch on the lift. A drag
// that starts at the leading edge toward the trailing side is the system's
// back gesture, and is left to it. VoiceOver can't swipe through its own
// gestures: every row offers Next pane and Previous pane instead.

extension CoreTerminalView {
    /// The strip along the leading edge the system's back gesture owns.
    static let backEdge: CGFloat = 20

    /// One move of a sideways drag (`handlePan`'s `.changed` once the axis
    /// locked sideways).
    func trackSwipe(_ g: UIPanGestureRecognizer) {
        guard let delegate = surfaceDelegate, delegate.surfaceTracksSwipe, !gestures.swipeRefused else { return }
        let t = g.translation(in: self)
        if !gestures.swiping {
            let ltr = effectiveUserInterfaceLayoutDirection == .leftToRight
            let start = g.location(in: self).x - t.x
            let fromEdge = ltr ? start < Self.backEdge : start > bounds.width - Self.backEdge
            let towardTrailing = ltr ? t.x > 0 : t.x < 0
            if fromEdge, towardTrailing {
                gestures.swipeRefused = true
                return
            }
            gestures.swiping = true
            delegate.surface(self, swipe: .began)
        }
        delegate.surface(self, swipe: .moved(t.x))
    }

    /// The end of a sideways drag: true when the host had it (moved it, or
    /// left it to the back gesture), so no switch on the lift.
    func endSwipe(_ g: UIPanGestureRecognizer, cancelled: Bool = false) -> Bool {
        defer {
            gestures.swiping = false
            gestures.swipeRefused = false
        }
        if gestures.swipeRefused { return true }
        guard gestures.swiping else { return surfaceDelegate?.surfaceTracksSwipe == true }
        let swipe: TerminalSwipe = cancelled
            ? .cancelled
            : .ended(g.translation(in: self).x, velocity: g.velocity(in: self).x)
        surfaceDelegate?.surface(self, swipe: swipe)
        return true
    }

    /// Next pane and Previous pane, on every row VoiceOver reads (empty
    /// where nothing switches).
    func paneSwitchActions() -> [UIAccessibilityCustomAction] {
        guard surfaceDelegate?.surfaceCanSwitch == true else { return [] }
        return [(String(localized: "Next pane"), true), (String(localized: "Previous pane"), false)]
            .map { name, forward in
                UIAccessibilityCustomAction(name: name) { [weak self] _ in
                    guard let self, let delegate = self.surfaceDelegate, delegate.surfaceCanSwitch else { return false }
                    delegate.surface(self, switchForward: forward)
                    return true
                }
            }
    }
}
