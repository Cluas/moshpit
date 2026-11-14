import UIKit

/// A confirmation an NSH page's long-press menu asks (a kill, a worktree's
/// removal), as UIKit's action sheet hanging from the row the menu came
/// from (B498).
///
/// SwiftUI's `confirmationDialog` hangs from the view it is attached to. On
/// the NSH pages that is the whole page: from iOS 26 an iPhone's dialog is a
/// popover too, so "Kill pane 2?" came up near the top of the screen, over
/// the toolbar and the status line, its arrow pointing at whatever was in
/// the page's middle. Here it points at the row that asked; with no row on
/// screen it is a phone's bottom sheet, or on an iPad a popover over the
/// page's bottom middle with no arrow — never at a random spot.
@MainActor
enum NSHConfirmation {
    struct Action {
        var title: String
        var destructive = false
        var run: () -> Void
    }

    /// Where the sheet hangs.
    enum Placement: Equatable {
        /// From `rect` of the source view (its bounds), pointing at it.
        case row(CGRect)
        /// A phone's action sheet from the bottom: no popover.
        case bottom
        /// No row to point at on an iPad (a popover needs a source): the
        /// page's bottom middle, no arrow.
        case pageBottom(CGRect)
    }

    /// Where a sheet asked from `source` (the menu's row, nil when it is
    /// gone) hangs, over `page`, on a device of `idiom`.
    static func placement(source: UIView?, page: UIView?, idiom: UIUserInterfaceIdiom) -> Placement {
        if let source, source.window != nil, !source.bounds.isEmpty {
            return .row(source.bounds)
        }
        guard idiom == .pad, let page else { return .bottom }
        return .pageBottom(CGRect(x: page.bounds.midX, y: page.bounds.maxY, width: 0, height: 0))
    }

    /// The sheet: `title`, an optional `message`, `actions`, and Cancel
    /// (`cancel` runs for it and for a tap outside the popover).
    static func sheet(title: String, message: String?, actions: [Action], cancelTitle: String,
                      cancel: @escaping () -> Void) -> UIAlertController {
        let alert = UIAlertController(title: title, message: message, preferredStyle: .actionSheet)
        for action in actions {
            alert.addAction(UIAlertAction(title: action.title, style: action.destructive ? .destructive : .default) { _ in
                action.run()
            })
        }
        alert.addAction(UIAlertAction(title: cancelTitle, style: .cancel) { _ in cancel() })
        return alert
    }

    /// Hangs `alert` as `placement` says (`source`: the row; `page`: the
    /// surface).
    static func hang(_ alert: UIAlertController, _ placement: Placement, source: UIView?, page: UIView?) {
        guard let popover = alert.popoverPresentationController else { return }
        switch placement {
        case .row(let rect):
            popover.sourceView = source
            popover.sourceRect = rect
            popover.permittedArrowDirections = [.up, .down]
        case .pageBottom(let rect):
            popover.sourceView = page
            popover.sourceRect = rect
            popover.permittedArrowDirections = []
        case .bottom:
            break
        }
    }

    /// Shows the sheet over `source`'s (or `page`'s) window, once the menu
    /// that asked has gone (it is still leaving when its action runs).
    /// Says false when there is no window to show it in.
    @discardableResult
    static func present(title: String, message: String? = nil, actions: [Action],
                        cancelTitle: String = String(localized: "Cancel"),
                        from source: UIView?, over page: UIView?,
                        cancel: @escaping () -> Void) -> Bool {
        guard let window = source?.window ?? page?.window else { return false }
        let page = page ?? window
        let alert = sheet(title: title, message: message, actions: actions, cancelTitle: cancelTitle,
                          cancel: cancel)
        Task { @MainActor in
            // The menu's own presentation leaves first: up to a second.
            for _ in 0..<20 {
                let top = topmost(in: window)
                if !(top?.isBeingDismissed ?? false), !(top?.isBeingPresented ?? false) { break }
                try? await Task.sleep(for: .milliseconds(50))
            }
            guard let top = topmost(in: window) else { return cancel() }
            // Placed now: the row may have moved while the menu left.
            hang(alert, placement(source: source, page: page, idiom: window.traitCollection.userInterfaceIdiom),
                 source: source, page: page)
            top.present(alert, animated: true)
        }
        return true
    }

    /// The view controller on top of `window`'s presentations.
    static func topmost(in window: UIWindow) -> UIViewController? {
        var top = window.rootViewController
        while let next = top?.presentedViewController, !next.isBeingDismissed { top = next }
        return top
    }
}
