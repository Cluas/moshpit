import UIKit

/// The native components every NSH surface gets from the app: the pieces of
/// UIKit a screen shows as the SwiftUI page does, where NSH draws no
/// equivalent of its own.
///
/// - `moshpit/symbol`: an SF Symbol (the back chevron of a bar, a row's
///   checkmark). Props: `name`, `pointSize`, `weight` (`light`, `regular`,
///   `medium`, `semibold`, `bold`), `scale` (`small`, `medium`, `large`), `color`
///   (`#RRGGBBAA`), or `palette` (a list of them: SwiftUI's
///   `.foregroundStyle(a, b)`, a color per layer). Centered in its box; a box
///   the screen leaves unsized takes the symbol's own size, as SwiftUI's
///   `Image(systemName:)` frame does.
/// - `moshpit/activity-indicator`: `ProgressView()`'s spinner (the medium
///   `UIActivityIndicatorView`), animating while shown. Props `color`, and
///   `scale` (SwiftUI's grows with the user's text size: the screen passes
///   its font scale).
///
/// UIKit's controls, which SwiftUI's draw as and NSH has no system-looking
/// equivalent of (its own controls are its own look). Each takes
/// `identifier` (its accessibility identifier, on the control itself as the
/// SwiftUI page puts it, so a UI test finds a switch as a switch) and
/// reports its own size; the screen stretches the width it wants.
///
/// - `moshpit/switch`: `UISwitch` (SwiftUI's `Toggle`). Props `on`, `tint`
///   (the on color), `label`, `enabled`; event `change` (the new value).
/// - `moshpit/segmented`: `UISegmentedControl` (a segmented `Picker`, in
///   the app's appearance: `Chrome.installAppearance`). Props `items`
///   (titles), `selected` (an index; -1: none); event `change` (the index).
/// - `moshpit/slider`: `UISlider` (SwiftUI's `Slider(value:in:step:)`).
///   Props `value`, `min`, `max`, `step`, `tint`, `label`; event `change`
///   (each new step while dragging). A value the screen sends while the
///   finger is down waits: the drag wins.
/// - `moshpit/menu`: a clear button over the box that opens the system's
///   pop-up menu (SwiftUI's `Menu` with a row or a bar button as its
///   label). Props `items` (`[{id, title, image?, destructive?,
///   disabled?}]`: `image` an SF Symbol beside the title, `destructive`
///   drawn in red, `disabled` greyed out), `label`; event `select` (the
///   item's id).
/// - `moshpit/progress`: `UIProgressView` (SwiftUI's
///   `ProgressView(value:)`). Props `value` (0…1), `tint`, `track` (the
///   groove's color: the `ui` layer's `muted`), `label`.
/// - `moshpit/color-well`: `UIColorWell` (SwiftUI's `ColorPicker` with its
///   label hidden), at its own size in the middle of the box; a tap
///   presents the system's color picker, without opacity. Props `color`
///   (`#RRGGBB[AA]`), `label`; event `change` (`#RRGGBB`, as the app
///   stores a color: sRGB, clamped).
@MainActor
enum NSHHostComponents {
    static func register(on surface: NSHSurface) {
        surface.register(component: "moshpit/symbol") { SymbolComponent(context: $0) }
        surface.register(component: "moshpit/activity-indicator") { _ in ActivityIndicatorComponent() }
        surface.register(component: "moshpit/switch") { SwitchComponent(context: $0) }
        surface.register(component: "moshpit/segmented") { SegmentedComponent(context: $0) }
        surface.register(component: "moshpit/slider") { SliderComponent(context: $0) }
        surface.register(component: "moshpit/menu") { MenuComponent(context: $0) }
        surface.register(component: "moshpit/progress") { ProgressComponent(context: $0) }
        surface.register(component: "moshpit/color-well") { ColorWellComponent(context: $0) }
    }

    /// A number prop.
    static func number(_ value: Any?) -> Double? {
        (value as? NSNumber)?.doubleValue
    }

    /// `#RRGGBB` or `#RRGGBBAA`.
    static func color(_ value: Any?) -> UIColor? {
        guard var hex = (value as? String), hex.hasPrefix("#") else { return nil }
        hex.removeFirst()
        guard hex.count == 6 || hex.count == 8, let packed = UInt32(hex, radix: 16) else { return nil }
        let rgba = hex.count == 6 ? packed << 8 | 0xFF : packed
        return UIColor(red: CGFloat(rgba >> 24 & 0xFF) / 255,
                       green: CGFloat(rgba >> 16 & 0xFF) / 255,
                       blue: CGFloat(rgba >> 8 & 0xFF) / 255,
                       alpha: CGFloat(rgba & 0xFF) / 255)
    }
}

/// `moshpit/symbol`.
@MainActor
private final class SymbolComponent: NSHComponent {
    private let imageView = UIImageView()
    private let context: NSHComponentContext
    var view: UIView { imageView }

    private var name = ""
    private var pointSize: CGFloat = 17
    private var weight = UIImage.SymbolWeight.regular
    private var scale = UIImage.SymbolScale.default
    private var palette: [UIColor] = []
    /// The size last reported to the screen.
    private var reported: CGSize?

    init(context: NSHComponentContext) {
        self.context = context
        imageView.contentMode = .center
        imageView.isUserInteractionEnabled = false
        imageView.isAccessibilityElement = false
    }

    func setProp(_ name: String, _ value: Any?) {
        switch name {
        case "name":
            self.name = value as? String ?? ""
        case "pointSize":
            pointSize = CGFloat((value as? NSNumber)?.doubleValue ?? 17)
        case "weight":
            weight = switch value as? String {
            case "light": .light
            case "medium": .medium
            case "semibold": .semibold
            case "bold": .bold
            default: .regular
            }
        case "scale":
            scale = switch value as? String {
            case "small": .small
            case "medium": .medium
            case "large": .large
            default: .default
            }
        case "color":
            imageView.tintColor = NSHHostComponents.color(value)
            return
        case "palette":
            palette = (value as? [Any] ?? []).compactMap(NSHHostComponents.color)
        default:
            return
        }
        var configuration = UIImage.SymbolConfiguration(pointSize: pointSize, weight: weight, scale: scale)
        if !palette.isEmpty {
            configuration = configuration.applying(UIImage.SymbolConfiguration(paletteColors: palette))
        }
        let image = UIImage(systemName: self.name, withConfiguration: configuration)
        imageView.image = palette.isEmpty ? image?.withRenderingMode(.alwaysTemplate) : image
        if let size = image?.size, size != reported {
            reported = size
            context.setSize(size)
        }
    }
}

/// `moshpit/activity-indicator`: the spinner centered in the box NSH lays
/// out, scaled inside it (a transform on the laid-out view itself would
/// leave its frame undefined).
@MainActor
private final class ActivityIndicatorComponent: NSHComponent {
    private let container = UIView()
    private let indicator = UIActivityIndicatorView(style: .medium)
    var view: UIView { container }

    init() {
        container.isUserInteractionEnabled = false
        indicator.hidesWhenStopped = false
        indicator.center = CGPoint(x: container.bounds.midX, y: container.bounds.midY)
        indicator.autoresizingMask = [.flexibleLeftMargin, .flexibleRightMargin,
                                      .flexibleTopMargin, .flexibleBottomMargin]
        container.addSubview(indicator)
        indicator.startAnimating()
    }

    func setProp(_ name: String, _ value: Any?) {
        switch name {
        case "color":
            indicator.color = NSHHostComponents.color(value)
        case "scale":
            let scale = CGFloat((value as? NSNumber)?.doubleValue ?? 1)
            indicator.transform = CGAffineTransform(scaleX: scale, y: scale)
        default:
            break
        }
    }
}

/// The props every control takes: its accessibility identifier and label.
@MainActor
private func applyAccessibility(_ name: String, _ value: Any?, to view: UIView) -> Bool {
    switch name {
    case "identifier":
        view.accessibilityIdentifier = value as? String
    case "label":
        view.accessibilityLabel = value as? String
    default:
        return false
    }
    return true
}

/// `moshpit/switch`.
@MainActor
private final class SwitchComponent: NSHComponent {
    private let control = UISwitch()
    private let context: NSHComponentContext
    var view: UIView { control }

    init(context: NSHComponentContext) {
        self.context = context
        control.addAction(UIAction { [weak control, context] _ in
            guard let control else { return }
            context.emit("change", control.isOn)
        }, for: .valueChanged)
        context.setSize(control.sizeThatFits(.zero))
    }

    func setProp(_ name: String, _ value: Any?) {
        if applyAccessibility(name, value, to: control) { return }
        switch name {
        case "on":
            let on = (value as? Bool) ?? false
            if control.isOn != on { control.setOn(on, animated: control.window != nil) }
        case "tint":
            control.onTintColor = NSHHostComponents.color(value)
        // The `ui` layer's off track (`muted`). UISwitch has no off-tint:
        // its background, rounded to the track, shows where the track is
        // off.
        case "offTint":
            control.backgroundColor = NSHHostComponents.color(value)
            control.layer.cornerRadius = control.sizeThatFits(.zero).height / 2
        case "enabled":
            control.isEnabled = (value as? Bool) ?? true
        default:
            break
        }
    }
}

/// `moshpit/segmented`.
@MainActor
private final class SegmentedComponent: NSHComponent {
    private let control = UISegmentedControl()
    private let context: NSHComponentContext
    var view: UIView { control }
    private var items: [String] = []
    private var selected = UISegmentedControl.noSegment

    init(context: NSHComponentContext) {
        self.context = context
        control.addAction(UIAction { [weak control, context] _ in
            guard let control, control.selectedSegmentIndex != UISegmentedControl.noSegment else { return }
            context.emit("change", control.selectedSegmentIndex)
        }, for: .valueChanged)
    }

    func setProp(_ name: String, _ value: Any?) {
        if applyAccessibility(name, value, to: control) { return }
        switch name {
        case "items":
            let titles = (value as? [Any] ?? []).compactMap { $0 as? String }
            guard titles != items else { return }
            items = titles
            control.removeAllSegments()
            for (index, title) in titles.enumerated() {
                control.insertSegment(withTitle: title, at: index, animated: false)
            }
            control.selectedSegmentIndex = selected < titles.count ? selected : UISegmentedControl.noSegment
            context.setSize(control.sizeThatFits(CGSize(width: CGFloat.greatestFiniteMagnitude,
                                                        height: CGFloat.greatestFiniteMagnitude)))
        // The `ui` layer's colors, by token (they win over the app-wide
        // appearance the SwiftUI pages set).
        case "thumb":
            control.selectedSegmentTintColor = NSHHostComponents.color(value)
        case "text":
            if let color = NSHHostComponents.color(value) {
                control.setTitleTextAttributes([.foregroundColor: color], for: .normal)
            }
        case "selectedText":
            if let color = NSHHostComponents.color(value) {
                control.setTitleTextAttributes([.foregroundColor: color], for: .selected)
            }
        case "selected":
            let index = Int(NSHHostComponents.number(value) ?? -1)
            selected = index >= 0 ? index : UISegmentedControl.noSegment
            if index >= 0, index < items.count {
                control.selectedSegmentIndex = index
            } else {
                control.selectedSegmentIndex = UISegmentedControl.noSegment
            }
        default:
            break
        }
    }
}

/// `moshpit/slider`.
@MainActor
private final class SliderComponent: NSHComponent {
    private let control = UISlider()
    private let context: NSHComponentContext
    var view: UIView { control }
    private var step: Float = 0
    /// The last value the screen heard, so a step is told once.
    private var told: Float?
    /// What the screen sent while the finger was down.
    private var pending: Float?

    init(context: NSHComponentContext) {
        self.context = context
        control.addAction(UIAction { [weak self] _ in self?.moved() }, for: .valueChanged)
        control.addAction(UIAction { [weak self] _ in self?.released() },
                          for: [.touchUpInside, .touchUpOutside, .touchCancel])
        context.setSize(control.sizeThatFits(CGSize(width: CGFloat.greatestFiniteMagnitude,
                                                    height: CGFloat.greatestFiniteMagnitude)))
    }

    /// The finger moved it: it snaps to a step, as SwiftUI's stepped slider
    /// does, and a new step is told.
    private func moved() {
        var value = control.value
        if step > 0 {
            value = (value / step).rounded() * step
            control.value = value
        }
        guard value != told else { return }
        told = value
        context.emit("change", Double(value))
    }

    private func released() {
        if let pending {
            self.pending = nil
            control.setValue(pending, animated: false)
        }
    }

    func setProp(_ name: String, _ value: Any?) {
        if applyAccessibility(name, value, to: control) { return }
        let number = NSHHostComponents.number(value).map(Float.init)
        switch name {
        case "value":
            guard let number else { return }
            told = number
            if control.isTracking {
                pending = number
            } else {
                control.setValue(number, animated: false)
            }
        case "min":
            control.minimumValue = number ?? 0
        case "max":
            control.maximumValue = number ?? 1
        case "step":
            step = number ?? 0
        case "tint":
            control.minimumTrackTintColor = NSHHostComponents.color(value)
        default:
            break
        }
    }
}

/// `moshpit/menu`.
@MainActor
private final class MenuComponent: NSHComponent {
    private let button = UIButton(type: .custom)
    private let context: NSHComponentContext
    var view: UIView { button }

    init(context: NSHComponentContext) {
        self.context = context
        button.showsMenuAsPrimaryAction = true
        button.backgroundColor = .clear
    }

    func setProp(_ name: String, _ value: Any?) {
        if applyAccessibility(name, value, to: button) { return }
        guard name == "items" else { return }
        let items = (value as? [Any] ?? []).compactMap { $0 as? [String: Any] }
        let context = context
        button.menu = UIMenu(children: items.compactMap { item in
            guard let id = item["id"] as? String, let title = item["title"] as? String else { return nil }
            let image = (item["image"] as? String).flatMap { UIImage(systemName: $0) }
            var attributes: UIMenuElement.Attributes = []
            if item["destructive"] as? Bool == true { attributes.insert(.destructive) }
            if item["disabled"] as? Bool == true { attributes.insert(.disabled) }
            let action = UIAction(title: title, image: image, attributes: attributes) { _ in
                context.emit("select", id)
            }
            // The `ui` layer's Select: the option in use is checked, and an
            // option may say in a line what it is for.
            if item["checked"] as? Bool == true { action.state = .on }
            if let detail = item["detail"] as? String { action.subtitle = detail }
            return action
        })
    }
}

/// `moshpit/progress`.
@MainActor
private final class ProgressComponent: NSHComponent {
    private let control = UIProgressView(progressViewStyle: .default)
    var view: UIView { control }

    init(context: NSHComponentContext) {
        context.setSize(control.sizeThatFits(CGSize(width: CGFloat.greatestFiniteMagnitude,
                                                    height: CGFloat.greatestFiniteMagnitude)))
    }

    func setProp(_ name: String, _ value: Any?) {
        if applyAccessibility(name, value, to: control) { return }
        switch name {
        case "value":
            control.setProgress(Float(NSHHostComponents.number(value) ?? 0), animated: false)
        case "tint":
            control.progressTintColor = NSHHostComponents.color(value)
        case "track":
            control.trackTintColor = NSHHostComponents.color(value)
        default:
            break
        }
    }
}

/// `moshpit/color-well`.
@MainActor
private final class ColorWellComponent: NSHComponent {
    private let container = Centering()
    private let well = UIColorWell()
    private let context: NSHComponentContext
    var view: UIView { container }
    /// The color shown, as the screen and the user last set it (`#RRGGBB`):
    /// a value echoed back is not set again, a pick is told once.
    private var shown: String?

    init(context: NSHComponentContext) {
        self.context = context
        well.supportsAlpha = false
        container.addSubview(well)
        well.addAction(UIAction { [weak self] _ in self?.picked() }, for: .valueChanged)
    }

    /// Lays its one subview out at that view's own size, in its middle (as
    /// SwiftUI's `.frame(width:)` places a `ColorPicker`).
    private final class Centering: UIView {
        override func layoutSubviews() {
            super.layoutSubviews()
            let scale = max(traitCollection.displayScale, 1)
            let pixel = { (v: CGFloat) in (v * scale).rounded() / scale }
            for subview in subviews {
                var size = subview.intrinsicContentSize
                // UIKit's well is 28 pt; a size it does not give is that.
                if size.width <= 0 || size.height <= 0 { size = CGSize(width: 28, height: 28) }
                subview.frame = CGRect(x: pixel((bounds.width - size.width) / 2),
                                       y: pixel((bounds.height - size.height) / 2),
                                       width: size.width, height: size.height)
            }
        }
    }

    private func picked() {
        guard let color = well.selectedColor else { return }
        let hex = Self.hex(color)
        guard hex != shown else { return }
        shown = hex
        context.emit("change", hex)
    }

    func setProp(_ name: String, _ value: Any?) {
        if applyAccessibility(name, value, to: well) { return }
        guard name == "color", let color = NSHHostComponents.color(value) else { return }
        let hex = Self.hex(color)
        guard hex != shown else { return }
        shown = hex
        well.selectedColor = color
    }

    /// `color` as the app stores a color (`Color.hexString`): its sRGB
    /// components, clamped to 0…1.
    static func hex(_ color: UIColor) -> String {
        var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
        color.getRed(&red, green: &green, blue: &blue, alpha: &alpha)
        let byte = { (c: CGFloat) in Int((min(max(c, 0), 1) * 255).rounded()) }
        return String(format: "#%02X%02X%02X", byte(red), byte(green), byte(blue))
    }
}
