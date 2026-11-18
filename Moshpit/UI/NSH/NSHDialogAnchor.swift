import ObjectiveC
import QuartzCore
import UIKit

/// Hangs an NSH action sheet's popover from the piece of the screen it
/// belongs to. NSH shows an action sheet as UIKit's `UIAlertController`
/// and hangs it from the surface's bottom middle without an arrow, where
/// SwiftUI's `confirmationDialog` hangs from the view it is on (the
/// terminal's transport pill), its arrow pointing there.
///
/// A screen says before it shows such a sheet (`{"ev": "dialog-anchor"}`);
/// ``arm(surface:rect:)`` then moves the next action sheet the surface
/// presents as it is presented: `present(_:animated:completion:)` is
/// intercepted, and the popover's source set before UIKit sees it. Moving
/// it later, from the run loop, was enough for the iPad's popover, which
/// lays out again, but not for the phone's: since iOS 26 a phone's action
/// sheet hangs from its source too (floating, not docked to the bottom
/// edge), placed once as it is presented, so it stayed at the bottom over
/// the key bar.
@MainActor
final class NSHDialogAnchor {
    private weak var surface: UIView?
    private var rect: CGRect = .zero
    private var deadline: CFTimeInterval = 0
    /// How long a sheet the screen announced may take to come.
    private static let window: CFTimeInterval = 1.0
    /// The anchor waiting for its sheet, if any (one screen at a time).
    private static weak var armed: NSHDialogAnchor?

    /// The next action sheet `surface` shows hangs from `rect` (in the
    /// surface's points).
    func arm(surface: UIView, rect: CGRect) {
        disarm()
        guard rect.width > 0 else { return }
        Self.interceptPresentation()
        self.surface = surface
        self.rect = rect
        deadline = CACurrentMediaTime() + Self.window
        Self.armed = self
        // The word comes from NSH's engine thread, the sheet too: one that
        // beat it here is moved where it is (an iPad's popover lays out
        // again; a phone's sheet may stay where it came up).
        if let alert = Self.sheet(presentedBy: surface), let popover = alert.popoverPresentationController {
            Self.hang(popover, from: rect, in: surface)
            popover.containerView?.setNeedsLayout()
            disarm()
        }
    }

    /// The action sheet on top of `surface`'s window that NSH presented
    /// for it (its popover hangs from the surface or a view in it).
    static func sheet(presentedBy surface: UIView) -> UIAlertController? {
        var top = surface.window?.rootViewController
        while let next = top?.presentedViewController { top = next }
        guard let alert = top as? UIAlertController, alert.preferredStyle == .actionSheet,
              let source = alert.popoverPresentationController?.sourceView,
              source === surface || source.isDescendant(of: surface) else { return nil }
        return alert
    }

    func disarm() {
        if Self.armed === self { Self.armed = nil }
    }

    /// `controller` is about to be presented: the armed anchor's sheet
    /// hangs from its rect.
    static func willPresent(_ controller: UIViewController) {
        guard let anchor = armed, let surface = anchor.surface,
              CACurrentMediaTime() < anchor.deadline else { return }
        guard let alert = controller as? UIAlertController, alert.preferredStyle == .actionSheet,
              let popover = alert.popoverPresentationController,
              let source = popover.sourceView,
              source === surface || source.isDescendant(of: surface) else { return }
        hang(popover, from: anchor.rect, in: surface)
        anchor.disarm()
    }

    /// Moves `popover` to hang from `rect` of `surface`, pointing at it.
    static func hang(_ popover: UIPopoverPresentationController, from rect: CGRect, in surface: UIView) {
        popover.sourceView = surface
        popover.sourceRect = rect
        popover.permittedArrowDirections = .any
    }

    private static var intercepting = false

    /// Routes every `present(_:animated:completion:)` through
    /// ``willPresent(_:)`` first. Once, for the app's life; it does nothing
    /// to a presentation unless an anchor is armed for it.
    private static func interceptPresentation() {
        guard !intercepting else { return }
        intercepting = true
        let cls: AnyClass = UIViewController.self
        guard let original = class_getInstanceMethod(
                cls, #selector(UIViewController.present(_:animated:completion:))),
              let replacement = class_getInstanceMethod(
                cls, #selector(UIViewController.moshpitAnchoredPresent(_:animated:completion:)))
        else { return }
        method_exchangeImplementations(original, replacement)
    }
}

extension UIViewController {
    /// ``NSHDialogAnchor``'s `present`: its implementation and `present`'s
    /// are exchanged, so the call inside runs UIKit's own.
    @objc dynamic func moshpitAnchoredPresent(_ controller: UIViewController, animated: Bool,
                                              completion: (() -> Void)?) {
        NSHDialogAnchor.willPresent(controller)
        moshpitAnchoredPresent(controller, animated: animated, completion: completion)
    }
}
