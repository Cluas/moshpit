import SwiftUI

// Moshpit's glyph, shared by the app's marks (`Moshpit/UI/Brand/
// MoshpitMark.swift`) and the Live Activity's (``IslandMark``): one
// geometry, so the marks and the home-screen icon cannot drift apart.

/// The bare panes, without the tile behind them. Split out so the mark, the
/// connecting screen's loader, and anywhere else that needs the raw glyph
/// share one geometry.
public struct MoshpitGlyph: View {
    /// Side of the square the 1024-unit tile is scaled into.
    public var side: CGFloat
    /// Pane colour. White in the icon; callers drawing on light surfaces pass
    /// something else.
    public var ink: Color = .white

    public init(side: CGFloat, ink: Color = .white) {
        self.side = side
        self.ink = ink
    }

    /// The design geometry, in the icon's 1024-unit tile. Mirrors the layer
    /// PNGs in `Moshpit/Resources/AppIcon.icon/Assets`; change both together.
    public enum Metrics {
        public static let tile: CGFloat = 1024
        /// The pane you are in — full height, white.
        public static let focused = CGRect(x: 190, y: 262, width: 307, height: 500)
        public static let focusedCorner: CGFloat = 72
        /// The two panes beside it, stacked. Open, not active: half alpha.
        public static let beside = [CGRect(x: 527, y: 262, width: 307, height: 235),
                             CGRect(x: 527, y: 527, width: 307, height: 235)]
        public static let besideCorner: CGFloat = 64
        public static let besideAlpha: Double = 0.55
        /// The cursor: a hole in the focused pane, so the tile shows through.
        public static let cursor = CGRect(x: 262, y: 344, width: 100, height: 124)
        public static let cursorCorner: CGFloat = 24
        /// iOS's home-screen icon mask, as a fraction of the side.
        public static let cornerRatio: CGFloat = 0.2237
        /// The glass layers' drop shadow, as fractions of the side.
        public static let shadowAlpha: Double = 0.18
        public static let shadowBlur: CGFloat = 0.016
        public static let shadowDrop: CGFloat = 0.012
    }

    public var body: some View {
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
public struct FocusedPaneShape: Shape {
    /// False draws the pane solid — the loader blinks the cursor by toggling it.
    public var cursorOpen = true

    public init(cursorOpen: Bool = true) {
        self.cursorOpen = cursorOpen
    }

    public func path(in rect: CGRect) -> Path {
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

/// Moshpit's mark on the Dynamic Island and the lock screen, where the
/// companion would be when it's switched off: the panes on the shipped
/// icon's violet tile (fixed: the extension has no theme).
public struct IslandMark: View {
    public var size: CGFloat

    public init(size: CGFloat) {
        self.size = size
    }

    /// The shipped icon's tile colour, `#6C6BEF` (the app's
    /// `MoshpitMark.iconViolet`).
    static let violet = Color(red: 0x6C / 255, green: 0x6B / 255, blue: 0xEF / 255)

    public var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: size * MoshpitGlyph.Metrics.cornerRatio, style: .continuous)
                .fill(Self.violet)
            MoshpitGlyph(side: size)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}
