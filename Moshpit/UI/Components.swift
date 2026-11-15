import SwiftUI
import UIKit

// MARK: - Haptics

/// Lightweight haptic feedback for user actions, so a tap registers physically
/// even when the on-screen result is subtle or slightly delayed (the "did that
/// do anything?" gap). Generators are cheap to create per call.
enum Haptics {
    /// Navigation / selection change (switch session/window/pane, pick a row).
    static func select() { UISelectionFeedbackGenerator().selectionChanged() }
    /// A discrete action fired (create, send, toggle).
    static func tap(_ style: UIImpactFeedbackGenerator.FeedbackStyle = .light) {
        UIImpactFeedbackGenerator(style: style).impactOccurred()
    }
    static func success() { UINotificationFeedbackGenerator().notificationOccurred(.success) }
}

/// UIKit-level styling for the few system controls SwiftUI leaves unstyled.
enum Chrome {
    /// The segmented control in the app's ink — dark track, raised pill — so
    /// Password / SSH Key and Block / Bar / Underline stop reading as a
    /// Settings.app control dropped into a dark screen. UIAppearance, because
    /// `.pickerStyle(.segmented)` exposes nothing else. Every segmented
    /// control takes it, the NSH pages' too, so its colours follow the
    /// appearance where the control shows (R18).
    @MainActor static func installAppearance() {
        let seg = UISegmentedControl.appearance()
        seg.backgroundColor = InkUI.segTrack
        seg.selectedSegmentTintColor = InkUI.segActive
        seg.setTitleTextAttributes([.foregroundColor: InkUI.secondary], for: .normal)
        seg.setTitleTextAttributes([.foregroundColor: InkUI.primary], for: .selected)
    }
}

// MARK: - Status pills (§3.10)

/// What the transport pill says (``TerminalNSHPage``'s `pill`).
enum TransportPillKind {
    case mosh, moshRoaming, ssh
}

/// Live connection state, surfaced on the transport pill so a dropped/
/// reconnecting session is never silent.
enum TransportConnState { case live, connecting, reconnecting, offline }

// MARK: - Sheet surface and chrome

extension View {
    /// The tmux sheets' surface: nothing on iOS 26 (the system sheet is
    /// already glass, and any explicit background replaces it with paint),
    /// the app's sheet fill on iOS 18 where the default is flat: dark, or
    /// light under an NSH sheet in the light appearance (R18).
    @ViewBuilder func sheetSurface() -> some View {
        if #available(iOS 26.0, *) {
            self
        } else {
            self.presentationBackground(Ink.sheet)
        }
    }
}

/// A presentation that follows the size class: a popover anchored to the
/// modified view on a regular-width iPad, a sheet everywhere else. The tmux
/// pickers use it — on an iPad the whole-screen sheet for a five-row list was
/// the iPhone shape stretched, and the breadcrumb they belong to is a natural
/// anchor. A Max-size iPhone in landscape is regular too and stays a sheet:
/// the idiom check, not the size class alone, decides.
struct AdaptivePresentation<Sheet: View>: ViewModifier {
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    let isPresented: Binding<Bool>
    let sheet: () -> Sheet

    private var asPopover: Bool {
        UIDevice.current.userInterfaceIdiom == .pad && horizontalSizeClass == .regular
    }

    @ViewBuilder
    func body(content: Content) -> some View {
        if asPopover {
            content.appPopover(isPresented: isPresented, arrowEdge: .top) {
                sheet()
                    // A List has no ideal size of its own; the popover needs one.
                    // Three rows and the footer fit; longer lists scroll.
                    .frame(width: 400, height: 380)
                    .presentationCompactAdaptation(.sheet)
            }
        } else {
            content.appSheet(isPresented: isPresented, content: sheet)
        }
    }
}

extension View {
    func adaptivePresentation<Sheet: View>(
        isPresented: Binding<Bool>, @ViewBuilder content: @escaping () -> Sheet
    ) -> some View {
        modifier(AdaptivePresentation(isPresented: isPresented, sheet: content))
    }
}
