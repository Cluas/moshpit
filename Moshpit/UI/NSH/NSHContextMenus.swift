import UIKit

/// Long-press menus for parts of an NSH screen: UIKit's own context menu, on
/// the NSH surface, for the view under the finger. NSH draws no menu of its
/// own (docs/nsh/PROGRESS.md, W11b's gaps): the screen marks a part with an
/// accessibility identifier (`.test_id()`), and the host gives the menu of
/// each identifier here; the menu lifts that part, as SwiftUI's
/// `.contextMenu` lifts the view it is on. (``AccentRowMenus`` is the same
/// for the Accent page's rows.)
@MainActor
final class NSHContextMenus: NSObject, UIContextMenuInteractionDelegate {
    /// The actions of the part identified by the key, asked for when the
    /// menu opens.
    var menus: [String: () -> [UIMenuElement]] = [:]
    /// How round each part's lifted preview is (none: square).
    var cornerRadius: [String: CGFloat] = [:]
    private weak var surface: UIView?
    private var interaction: UIContextMenuInteraction?
    /// The part whose menu opened last (B498): what a confirmation one of
    /// its actions asks hangs from (``NSHConfirmation``).
    private weak var openedPart: UIView?

    /// The part whose menu opened last, while it is on screen: the row a
    /// confirmation its action asks points at.
    var openedView: UIView? { openedPart?.window != nil ? openedPart : nil }
    /// The surface the menus are on (a confirmation with no part to point
    /// at shows over it).
    var surfaceView: UIView? { surface }

    /// A screen's message naming the row it came from (`"row"`: a swipe
    /// tray's action, B499) makes that row the one a confirmation the
    /// message asks points at, as a menu opened on it would.
    func anchor(to message: [String: Any]) {
        guard let id = message["row"] as? String, let surface,
              let view = AccentRowMenus.view(identified: id, in: surface) else { return }
        openedPart = view
    }

    /// Puts the menus on `surface` (again: a new surface replaces the old).
    func install(on surface: UIView) {
        if let interaction { interaction.view?.removeInteraction(interaction) }
        let interaction = UIContextMenuInteraction(delegate: self)
        surface.addInteraction(interaction)
        self.interaction = interaction
        self.surface = surface
    }

    /// The part with a menu at `point` (in the surface): its identifier and
    /// view.
    func part(at point: CGPoint) -> (id: String, view: UIView)? {
        guard let surface else { return nil }
        for id in menus.keys.sorted() {
            guard let view = AccentRowMenus.view(identified: id, in: surface), view.window != nil else { continue }
            if view.bounds.contains(view.convert(point, from: surface)) {
                return (id, view)
            }
        }
        return nil
    }

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                configurationForMenuAtLocation location: CGPoint) -> UIContextMenuConfiguration? {
        guard let (id, view) = part(at: location), let actions = menus[id]?() else { return nil }
        openedPart = view
        return UIContextMenuConfiguration(identifier: id as NSString, previewProvider: nil) { _ in
            UIMenu(children: actions)
        }
    }

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                configuration: UIContextMenuConfiguration,
                                highlightPreviewForItemWithIdentifier identifier: any NSCopying) -> UITargetedPreview? {
        preview(for: configuration)
    }

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                configuration: UIContextMenuConfiguration,
                                dismissalPreviewForItemWithIdentifier identifier: any NSCopying) -> UITargetedPreview? {
        preview(for: configuration)
    }

    /// The part the menu is for, lifted in its own shape.
    private func preview(for configuration: UIContextMenuConfiguration) -> UITargetedPreview? {
        guard let id = configuration.identifier as? String, let surface,
              let view = AccentRowMenus.view(identified: id, in: surface), view.window != nil
        else { return nil }
        let parameters = UIPreviewParameters()
        parameters.visiblePath = UIBezierPath(roundedRect: view.bounds, cornerRadius: cornerRadius[id] ?? 0)
        return UITargetedPreview(view: view, parameters: parameters)
    }
}
