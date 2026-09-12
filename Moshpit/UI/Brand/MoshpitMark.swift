import SwiftUI

/// Moshpit's mark: a tmux window. Three panes on the accent tile — the one you
/// are in is white, with the cursor as a hole the tile shows through; the two
/// beside it are half-transparent, open and waiting.
///
/// The shipped app icon is the Icon Composer document `Moshpit/Resources/AppIcon.icon`
/// (Liquid Glass on iOS 26, flattened by Xcode for iOS 18/19). Its layers are
/// authored in a 1024-unit tile, and ``MoshpitGlyph/Metrics`` carries the same
/// numbers, so the in-app mark and the home-screen icon cannot drift apart.
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

/// The bare panes, without the tile behind them. Split out so the mark, the
/// connecting screen's loader, and anywhere else that needs the raw glyph
/// share one geometry.
struct MoshpitGlyph: View {
    /// Side of the square the 1024-unit tile is scaled into.
    var side: CGFloat
    /// Pane colour. White in the icon; callers drawing on light surfaces pass
    /// something else.
    var ink: Color = .white

    /// The design geometry, in the icon's 1024-unit tile. Mirrors the layer
    /// PNGs in `Moshpit/Resources/AppIcon.icon/Assets` (rendered by
    /// `marketing/redesign/icon-drafts/glass/layers.mjs`).
    enum Metrics {
        static let tile: CGFloat = 1024
        /// The pane you are in — full height, white.
        static let focused = CGRect(x: 190, y: 262, width: 307, height: 500)
        static let focusedCorner: CGFloat = 72
        /// The two panes beside it, stacked. Open, not active: half alpha.
        static let beside = [CGRect(x: 527, y: 262, width: 307, height: 235),
                             CGRect(x: 527, y: 527, width: 307, height: 235)]
        static let besideCorner: CGFloat = 64
        static let besideAlpha: Double = 0.55
        /// The cursor: a hole in the focused pane, so the tile shows through.
        static let cursor = CGRect(x: 262, y: 344, width: 100, height: 124)
        static let cursorCorner: CGFloat = 24
        /// iOS's home-screen icon mask, as a fraction of the side.
        static let cornerRatio: CGFloat = 0.2237
        /// The glass layers' drop shadow, as fractions of the side.
        static let shadowAlpha: Double = 0.18
        static let shadowBlur: CGFloat = 0.016
        static let shadowDrop: CGFloat = 0.012
    }

    var body: some View {
        let k = side / Metrics.tile
        ZStack(alignment: .topLeading) {
            FocusedPaneShape()
                .fill(ink, style: FillStyle(eoFill: true))
                .frame(width: side, height: side)
                .shadow(color: .black.opacity(Metrics.shadowAlpha),
                        radius: side * Metrics.shadowBlur, y: side * Metrics.shadowDrop)

            ForEach(Array(Metrics.beside.enumerated()), id: \.offset) { _, r in
                RoundedRectangle(cornerRadius: Metrics.besideCorner * k, style: .continuous)
                    .fill(ink.opacity(Metrics.besideAlpha))
                    .frame(width: r.width * k, height: r.height * k)
                    .offset(x: r.minX * k, y: r.minY * k)
                    .shadow(color: .black.opacity(Metrics.shadowAlpha),
                            radius: side * Metrics.shadowBlur, y: side * Metrics.shadowDrop)
            }
        }
        .frame(width: side, height: side)
    }
}

/// The focused pane with the cursor cut out of it. Fill with `eoFill` so the
/// inner rounded rectangle reads as a hole, not a second pane.
struct FocusedPaneShape: Shape {
    /// False draws the pane solid — the loader blinks the cursor by toggling it.
    var cursorOpen = true

    func path(in rect: CGRect) -> Path {
        typealias M = MoshpitGlyph.Metrics
        let k = rect.width / M.tile
        func scaled(_ r: CGRect) -> CGRect {
            CGRect(x: rect.minX + r.minX * k, y: rect.minY + r.minY * k,
                   width: r.width * k, height: r.height * k)
        }
        var path = Path()
        path.addRoundedRect(in: scaled(M.focused),
                            cornerSize: CGSize(width: M.focusedCorner * k, height: M.focusedCorner * k),
                            style: .continuous)
        if cursorOpen {
            path.addRoundedRect(in: scaled(M.cursor),
                                cornerSize: CGSize(width: M.cursorCorner * k, height: M.cursorCorner * k),
                                style: .continuous)
        }
        return path
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
