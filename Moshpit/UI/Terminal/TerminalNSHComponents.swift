import SwiftUI
import UIKit

/// What the terminal's NSH screen places that NSH draws no equivalent of,
/// registered on its surface before it runs (with Home's signal grid,
/// ``HomeNSHComponents``):
///
/// - `moshpit/terminal`: the slot the session's terminal shows in
///   (``TerminalNSHSlot``, the page's ``TerminalNSHStage``).
/// - `moshpit/glass`: the top bar's glass: Liquid
///   Glass (`UIGlassEffect`, regular, interactive) on iOS 26 and later; the
///   bar's material (SwiftUI's `.bar`, the system chrome material) with a
///   hairline border below. A leaf: what sits on it is its sibling. Props
///   `shape` (`capsule`, `circle`: both round to half the height),
///   `interactive`.
@MainActor
enum TerminalNSHComponents {
    static func register(on surface: NSHSurface, stage: TerminalNSHStage) {
        HomeNSHComponents.register(on: surface)
        surface.register(component: "moshpit/terminal") { [weak stage] _ in
            guard let stage else { return EmptyComponent() }
            return TerminalNSHSlot(stage: stage)
        }
        surface.register(component: "moshpit/glass") { _ in GlassComponent() }
    }
}

/// A component with nothing to show (its owner went away).
@MainActor
private final class EmptyComponent: NSHComponent {
    let view = UIView()
    func setProp(_ name: String, _ value: Any?) {}
}

/// `moshpit/glass`.
@MainActor
private final class GlassComponent: NSHComponent {
    private let glass = TerminalGlassView()
    var view: UIView { glass }

    init() {
        glass.isUserInteractionEnabled = false
    }

    func setProp(_ name: String, _ value: Any?) {
        if name == "interactive" { glass.interactive = (value as? Bool) ?? false }
    }
}

/// The glass, or the bar's material where there is none.
final class TerminalGlassView: UIVisualEffectView {
    /// Whether this is Liquid Glass (iOS 26 and later).
    let isGlass: Bool

    var interactive = false {
        didSet { applyGlass() }
    }

    init() {
        if #available(iOS 26.0, *) {
            isGlass = true
            super.init(effect: nil)
            applyGlass()
            // The glass draws its own capsule; masking it would flatten it.
            cornerConfiguration = .capsule()
        } else {
            isGlass = false
            // `.bar`: the system's chrome material, dark as the app is.
            super.init(effect: UIBlurEffect(style: .systemChromeMaterialDark))
            clipsToBounds = true
            layer.borderWidth = 1
            layer.borderColor = UIColor(white: 1, alpha: 0.075).cgColor
        }
        accessibilityIdentifier = isGlass ? "terminal-glass" : "terminal-glass-fallback"
    }

    required init?(coder: NSCoder) {
        fatalError("TerminalGlassView is made in code")
    }

    private func applyGlass() {
        guard isGlass, #available(iOS 26.0, *) else { return }
        let glass = UIGlassEffect(style: .regular)
        glass.isInteractive = interactive
        effect = glass
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        if !isGlass {
            layer.cornerRadius = bounds.height / 2
            layer.cornerCurve = .continuous
        }
    }
}

