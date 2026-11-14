import Testing
import UIKit
@testable import Moshpit

/// B302's lesson, for every NSH screen: a finger on a pressable lands in it.
/// NSH's presses are gesture recognizers on the views a touch lands in, so a
/// view drawn over a pressable (a border, a scrim, a decoration) takes its
/// taps whatever the screen meant; NSH's own `pointer_events(None)` does not
/// keep a view out of UIKit's hit test on iOS (NSH-GAPS #24), and the
/// headless press goes past the hit test. So: UIKit's own hit test at the
/// centre of every pressable a person can see.
@MainActor
enum NSHTouchAudit {
    /// Hit-tests the centre of every visible pressable in `surface` (a view
    /// with an NSH press recognizer), records an issue for each whose touch
    /// lands outside it. The layout is given a moment first.
    static func pressablesTakeTouches(in surface: UIView, sourceLocation: SourceLocation = #_sourceLocation) async {
        // The screen laid out, and drawn what it presses.
        let deadline = Date().addingTimeInterval(5)
        repeat {
            try? await Task.sleep(for: .milliseconds(100))
        } while !descendants(of: surface).contains(where: isPressable) && Date() < deadline
        try? await Task.sleep(for: .milliseconds(200))
        guard let window = surface.window else {
            Issue.record("the surface is in no window", sourceLocation: sourceLocation)
            return
        }
        var checked = 0
        for view in descendants(of: surface) where isPressable(view) && isShown(view) {
            let frame = view.convert(view.bounds, to: window)
            guard frame.width > 1, frame.height > 1 else { continue }
            let centre = CGPoint(x: frame.midX, y: frame.midY)
            guard window.bounds.contains(centre), isUnclipped(view, at: centre, in: window) else { continue }
            checked += 1
            let hit = window.hitTest(centre, with: nil)
            if !(hit === view || hit?.isDescendant(of: view) == true) {
                Issue.record("a touch on \(describe(view)) lands on \(describe(hit)), outside it",
                             sourceLocation: sourceLocation)
            }
        }
        if checked == 0 {
            Issue.record("no pressable on screen to check", sourceLocation: sourceLocation)
        }
    }

    private static func descendants(of view: UIView) -> [UIView] {
        view.subviews.flatMap { [$0] + descendants(of: $0) }
    }

    /// A view NSH made pressable: its press recognizer, enabled.
    private static func isPressable(_ view: UIView) -> Bool {
        view.gestureRecognizers?.contains {
            $0.isEnabled && String(describing: type(of: $0)).contains("PressRecognizer")
        } ?? false
    }

    /// Neither it nor a view around it is hidden or transparent.
    private static func isShown(_ view: UIView) -> Bool {
        var current: UIView? = view
        while let v = current {
            if v.isHidden || v.alpha <= 0.01 { return false }
            current = v.superview
        }
        return true
    }

    /// No view around it that clips (a scroll view) cuts `point` off.
    private static func isUnclipped(_ view: UIView, at point: CGPoint, in window: UIWindow) -> Bool {
        var current = view.superview
        while let v = current, v !== window {
            if v.clipsToBounds, !v.bounds.contains(v.convert(point, from: window)) { return false }
            current = v.superview
        }
        return true
    }

    private static func describe(_ view: UIView?) -> String {
        guard let view else { return "nothing" }
        let id = view.accessibilityIdentifier.map { " \($0)" } ?? ""
        let label = view.accessibilityLabel.map { " \"\($0.prefix(40))\"" } ?? ""
        let f = view.window.map { view.convert(view.bounds, to: $0) } ?? view.frame
        return "\(type(of: view))\(id)\(label) at \(Int(f.minX)),\(Int(f.minY)) \(Int(f.width))×\(Int(f.height))"
    }
}
