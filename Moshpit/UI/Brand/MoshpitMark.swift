import SwiftUI
import MoshpitKit

/// Moshpit's mark: a tmux window. Three panes on the accent tile — the one you
/// are in is white, with the cursor as a hole the tile shows through; the two
/// beside it are half-transparent, open and waiting.
///
/// The three panes were the shipped app icon until the pet's egg took over
/// (B668); they ship on as the Classic alternate,
/// `Moshpit/Resources/AlternateIcons/AppIcon-Classic.icon` (Liquid Glass on
/// iOS 26, flattened by Xcode for iOS 18/19). Its layers are authored in a
/// 1024-unit tile, and ``MoshpitGlyph/Metrics`` carries the same numbers, so
/// this mark and that icon cannot drift apart.
///
/// The tile follows the accent, which is the point of this view: it is the
/// THEME's mark. Anywhere that means "this app" to the user uses ``AppIconMark``.
struct MoshpitMark: View {
    var size: CGFloat = 32
    /// Overrides the theme accent. The icon gallery passes ``iconViolet`` so
    /// its primary tile shows the icon as it ships, not as the theme tints it.
    var accent: Color?

    /// The shipped icon's tile colour — the seed of `AppIcon.icon`'s automatic
    /// gradient. Fixed: iOS never tints an icon.
    static let iconViolet = Color(hex: "6C6BEF")

    var body: some View {
        ZStack {
            IconTile(accent: accent ?? Ink.accent, side: size)
            MoshpitGlyph(side: size)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// The app's mark **as the user has chosen it**: the panes on the accent, or
/// whichever alternate they picked from the icon gallery.
///
/// Picking an easter-egg icon used to change the home screen and nothing else,
/// so the app went on showing a mark the user had just replaced. The gallery
/// already bundles each icon's rendered preview (240 px), so following the
/// choice costs no new assets — and that is sharp well past the sizes used here.
///
/// For placements that mean "this app" to the user. Anywhere demonstrating the
/// THEME keeps ``MoshpitMark``, whose accent colour is the whole point.
struct AppIconMark: View {
    var size: CGFloat = 32
    @Environment(AppSettings.self) private var settings

    var body: some View {
        let option = AppIconCatalog.option(for: settings.appIconId)
        // The primary icon IS this vector, drawn from shared geometry — always
        // prefer it over a PNG, which would only be a lower-resolution copy
        // that ignores the accent colour.
        if option.isPrimary {
            MoshpitMark(size: size)
        } else {
            AppIconThumb(option: option, side: size)
        }
    }
}

/// The icon's background: the accent as iOS's own "automatic gradient" renders
/// a single seed colour — lighter at the top, deeper at the bottom — under the
/// home-screen corner mask. Built from overlays rather than colour math so it
/// works for any `Color`, including the transient tints the connecting screen
/// passes in.
struct IconTile: View {
    var accent: Color
    var side: CGFloat

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: side * MoshpitGlyph.Metrics.cornerRatio,
                                     style: .continuous)
        shape
            .fill(accent)
            .overlay(
                shape.fill(LinearGradient(
                    colors: [.white.opacity(0.22), .clear, .black.opacity(0.16)],
                    startPoint: .top, endPoint: .bottom)))
            .frame(width: side, height: side)
    }
}

/// The stage texture under the home screen and the connecting screen. Forms
/// and editors sit flat on the screen colour; these two are the app's floor.
struct SignalGrid: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        let step: CGFloat = 34
        var x = rect.minX - step
        while x <= rect.maxX + step {
            path.move(to: CGPoint(x: x, y: rect.minY))
            path.addLine(to: CGPoint(x: x + rect.height * 0.22, y: rect.maxY))
            x += step
        }

        var y = rect.minY + 22
        while y <= rect.maxY {
            path.move(to: CGPoint(x: rect.minX, y: y))
            path.addLine(to: CGPoint(x: rect.maxX, y: y))
            y += step
        }
        return path
    }
}

#Preview {
    ZStack {
        Ink.screenBG.ignoresSafeArea()
        VStack(spacing: 24) {
            MoshpitMark(size: 96)
            MoshpitMark(size: 44)
            MoshpitMark(size: 24)
        }
    }
}
