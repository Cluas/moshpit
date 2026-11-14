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

    /// The window's style (R18, Settings › Appearance): the appearance
    /// chosen while Home is the NSH one; dark while the SwiftUI Home shows,
    /// whose pages draw with the dark-only `Ink` palette (a build with the
    /// NSH switches off, or an NSH Home that failed to run).
    static func windowStyle(chosen: AppAppearance, nshHome: Bool) -> UIUserInterfaceStyle {
        nshHome ? chosen.interfaceStyle : .dark
    }
}

/// Sets the style of the window it is in (R18): the app's appearance, set
/// once on the window so every page, sheet and alert in it follows. What
/// keeps an appearance of its own sets it lower down: the terminal's colour
/// scheme on the terminal's NSH surface, a SwiftUI fallback page still on the
/// dark-only palette with its `.preferredColorScheme(.dark)`.
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

extension View {
    /// A SwiftUI page still on the dark-only `Ink` palette: the fallback
    /// while its NSH page's switch is off or the NSH page failed to run, and
    /// the few SwiftUI sheets with no NSH page yet. Dark whatever the app's
    /// appearance (R18), until the page is replaced.
    func darkOnlyPage() -> some View {
        preferredColorScheme(.dark)
    }
}
