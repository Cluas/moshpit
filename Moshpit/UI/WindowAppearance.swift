import SwiftUI
import UIKit

extension AppAppearance {
    /// The style a window takes for it: the system's for `.system`.
    var interfaceStyle: UIUserInterfaceStyle {
        switch self {
        case .system: .unspecified
        case .light: .light
        case .dark: .dark
        }
    }
}

/// Sets the style of the window it is in (R18): the app's appearance, set
/// once on the window so every page, sheet and alert in it follows. What
/// keeps an appearance of its own sets it lower down: the terminal's colour
/// scheme on the terminal's NSH surface.
///
/// The window, not a root `.preferredColorScheme`: SwiftUI's `nil` after a
/// `.dark` does not reliably give the system's appearance back.
struct WindowAppearance: UIViewRepresentable {
    let style: UIUserInterfaceStyle

    func makeUIView(context: Context) -> Probe { Probe() }

    func updateUIView(_ view: Probe, context: Context) {
        view.style = style
    }

    /// An empty view that sets its window's style when it reaches a window
    /// and whenever the style changes.
    final class Probe: UIView {
        var style: UIUserInterfaceStyle = .unspecified {
            didSet { if style != oldValue { apply() } }
        }

        override init(frame: CGRect) {
            super.init(frame: frame)
            isUserInteractionEnabled = false
            isAccessibilityElement = false
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

        override func didMoveToWindow() {
            super.didMoveToWindow()
            apply()
        }

        private func apply() {
            guard let window, window.overrideUserInterfaceStyle != style else { return }
            window.overrideUserInterfaceStyle = style
        }
    }
}

