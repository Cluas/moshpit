import SwiftUI
import UIKit

/// What the terminal's NSH screen places that NSH draws no equivalent of,
/// registered on its surface before it runs (with Home's signal grid,
/// ``HomeNSHComponents``):
///
/// - `moshpit/terminal`: the slot the session's terminal shows in
///   (``TerminalNSHSlot``, the page's ``TerminalNSHStage``).
/// - `moshpit/glass`: the top bar's glass, `glassChrome(in:)`'s: Liquid
///   Glass (`UIGlassEffect`, regular, interactive) on iOS 26 and later; the
///   bar's material (SwiftUI's `.bar`, the system chrome material) with a
///   hairline border below. A leaf: what sits on it is its sibling. Props
///   `shape` (`capsule`, `circle`: both round to half the height),
///   `interactive`.
/// - `moshpit/glow`: the connecting poster's accent pool, SwiftUI's
///   `RadialGradient(colors: [color, .clear], center:, startRadius: 0,
///   endRadius:)`. Props `color` (`#RRGGBBAA`), `center` (`[x, y]`, unit
///   point), `radius` (points).
/// - `moshpit/pane-loader`: the poster's mark, alive, in the icon the user
///   picked (`PaneLoaderMark`), at the box's size. Prop `accent`.
@MainActor
enum TerminalNSHComponents {
    static func register(on surface: NSHSurface, stage: TerminalNSHStage) {
        HomeNSHComponents.register(on: surface)
        surface.register(component: "moshpit/terminal") { [weak stage] _ in
            guard let stage else { return EmptyComponent() }
            return TerminalNSHSlot(stage: stage)
        }
        surface.register(component: "moshpit/glass") { _ in GlassComponent() }
        surface.register(component: "moshpit/glow") { _ in GlowComponent() }
        surface.register(component: "moshpit/pane-loader") { _ in PaneLoaderComponent() }
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

/// `moshpit/glow`.
@MainActor
private final class GlowComponent: NSHComponent {
    private final class Glow: UIView {
        override class var layerClass: AnyClass { CAGradientLayer.self }
        var gradient: CAGradientLayer { layer as! CAGradientLayer }
        var focus = CGPoint(x: 0.5, y: 0.5)
        var radius: CGFloat = 300

        override func layoutSubviews() {
            super.layoutSubviews()
            guard bounds.width > 0, bounds.height > 0 else { return }
            gradient.type = .radial
            gradient.startPoint = focus
            gradient.endPoint = CGPoint(x: focus.x + radius / bounds.width,
                                        y: focus.y + radius / bounds.height)
        }
    }

    private let glow = Glow()
    var view: UIView { glow }

    init() {
        glow.isUserInteractionEnabled = false
        glow.backgroundColor = .clear
    }

    func setProp(_ name: String, _ value: Any?) {
        switch name {
        case "color":
            let color = NSHHostComponents.color(value) ?? .clear
            glow.gradient.colors = [color.cgColor, color.withAlphaComponent(0).cgColor]
        case "center":
            let point = (value as? [Any])?.compactMap(NSHHostComponents.number) ?? []
            if point.count == 2 { glow.focus = CGPoint(x: point[0], y: point[1]) }
            glow.setNeedsLayout()
        case "radius":
            glow.radius = CGFloat(NSHHostComponents.number(value) ?? 300)
            glow.setNeedsLayout()
        default:
            break
        }
    }
}

/// `moshpit/pane-loader`: ``PaneLoaderMark`` at its box's size.
@MainActor
private final class PaneLoaderComponent: NSHComponent {
    private final class Host: UIView {
        var accent: Color = Ink.accent {
            didSet { rebuild() }
        }
        private var hosting: UIHostingController<AnyView>?
        private var side: CGFloat = 0

        override func layoutSubviews() {
            super.layoutSubviews()
            let side = min(bounds.width, bounds.height)
            guard side > 0 else { return }
            if side != self.side { self.side = side; rebuild() }
            hosting?.view.frame = CGRect(x: (bounds.width - side) / 2, y: (bounds.height - side) / 2,
                                         width: side, height: side)
        }

        private func rebuild() {
            guard side > 0 else { return }
            hosting?.view.removeFromSuperview()
            let controller = UIHostingController(rootView: AnyView(
                PaneLoaderMark(side: side, accent: accent).environment(AppSettings.shared)))
            controller.view.backgroundColor = .clear
            controller.view.isUserInteractionEnabled = false
            addSubview(controller.view)
            hosting = controller
            setNeedsLayout()
        }
    }

    private let host = Host()
    var view: UIView { host }

    init() {
        host.isAccessibilityElement = false
        host.isUserInteractionEnabled = false
    }

    func setProp(_ name: String, _ value: Any?) {
        guard name == "accent", let color = NSHHostComponents.color(value) else { return }
        host.accent = Color(uiColor: color)
    }
}
