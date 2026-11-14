import QuartzCore
import UIKit

/// Hangs an NSH action sheet's popover from the piece of the screen it
/// belongs to. NSH shows an action sheet as UIKit's `UIAlertController`,
/// which on the iPad is a popover; NSH has no anchor to give it, so it
/// hangs it from the surface's bottom middle without an arrow, where
/// SwiftUI's `confirmationDialog` hangs from the view it is on (the
/// terminal's transport pill), its arrow pointing there.
///
/// A screen says before it shows such a sheet (`{"ev": "dialog-anchor"}`);
/// ``arm(surface:rect:)`` then watches the main run loop for a short while
/// with an observer ordered before Core Animation's commit, so the alert
/// the surface presents is moved in the turn it is presented, before UIKit
/// lays its popover out. On a phone the sheet is no popover: nothing to do.
@MainActor
final class NSHDialogAnchor {
    private var observer: CFRunLoopObserver?
    private weak var surface: UIView?
    private var rect: CGRect = .zero
    private var deadline: CFTimeInterval = 0
    /// How long a sheet the screen announced may take to come.
    private static let window: CFTimeInterval = 1.0

    /// The next action sheet `surface` shows hangs from `rect` (in the
    /// surface's points).
    func arm(surface: UIView, rect: CGRect) {
        disarm()
        guard UIDevice.current.userInterfaceIdiom != .phone, rect.width > 0 else { return }
        self.surface = surface
        self.rect = rect
        deadline = CACurrentMediaTime() + Self.window
        // Before Core Animation's commit (2000000), so before layout.
        let observer = CFRunLoopObserverCreateWithHandler(
            nil, CFRunLoopActivity.beforeWaiting.rawValue, true, 1_000
        ) { [weak self] _, _ in
            MainActor.assumeIsolated { self?.check() }
        }
        CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes)
        self.observer = observer
    }

    func disarm() {
        if let observer {
            CFRunLoopObserverInvalidate(observer)
            self.observer = nil
        }
    }

    private func check() {
        guard let surface, CACurrentMediaTime() < deadline else { return disarm() }
        guard let alert = Self.sheet(presentedBy: surface),
              let popover = alert.popoverPresentationController else { return }
        Self.hang(popover, from: rect, in: surface)
        disarm()
    }

    /// Moves `popover` to hang from `rect` of `surface`, pointing at it.
    static func hang(_ popover: UIPopoverPresentationController, from rect: CGRect, in surface: UIView) {
        popover.sourceView = surface
        popover.sourceRect = rect
        popover.permittedArrowDirections = .any
        // Laid out already (a sheet that came a turn late): again.
        popover.containerView?.setNeedsLayout()
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
}
