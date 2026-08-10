import SwiftUI

/// Moshpit's mark: a glowing cursor block crowd-surfing over a row of three
/// grounded blocks. The row below reads as an ellipsis — sessions waiting
/// their turn — while the pit carries the live one overhead.
///
/// The geometry is authored in a 24×24 design grid (``MoshpitGlyph.Metrics``)
/// shared verbatim with `scripts/generate-app-icons.swift`, so the in-app
/// mark and the shipped app icons cannot drift apart.
struct MoshpitMark: View {
    var size: CGFloat = 32

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.18, style: .continuous)
                .fill(Ink.hostIcon)

            RoundedRectangle(cornerRadius: size * 0.16, style: .continuous)
                .fill(Ink.terminalBG.opacity(0.86))
                .padding(size * 0.12)

            MoshpitSlashShape()
                .fill(Ink.accent.opacity(0.22))
                .padding(size * 0.08)

            // 0.72 (was 0.66): at header/tile sizes the crowd row got too
            // small to read as an ellipsis — the glyph can afford more of the
            // tile now that it has no arc reaching the edges.
            MoshpitGlyph(side: size * 0.72)
        }
        .frame(width: size, height: size)
        .shadow(color: Ink.accent.opacity(0.22), radius: size * 0.18, y: size * 0.06)
        .accessibilityHidden(true)
    }
}

/// The bare crowd-surf glyph, without the tile behind it. Split out so the
/// mark, and anywhere else that needs the raw logo, share one geometry.
struct MoshpitGlyph: View {
    /// Side of the square the 24×24 design grid is scaled into.
    var side: CGFloat

    /// The design grid, in 24ths. Mirrored in `generate-app-icons.swift`.
    enum Metrics {
        /// The airborne cursor — the live session the pit is carrying.
        static let surfer = CGRect(x: 8.4, y: 8.0, width: 6.8, height: 4.0)
        /// Right edge up by this many degrees. SwiftUI's y-down space rotates
        /// clockwise for positive angles, so the view applies `-surferTilt`;
        /// the icon script's y-up CoreGraphics context applies `+surferTilt`.
        static let surferTilt: CGFloat = 12
        static let surferCorner: CGFloat = 1.1
        /// The crowd row: three grounded cursor blocks. Deliberately reads as
        /// an ellipsis — the sessions waiting underneath.
        static let crowdXs: [CGFloat] = [3.6, 9.8, 16.0]
        static let crowdY: CGFloat = 15.6
        static let crowdSize = CGSize(width: 4.4, height: 2.8)
        static let crowdCorner: CGFloat = 0.8
        static let crowdAlpha: CGFloat = 0.9
    }

    var body: some View {
        let k = side / 24
        ZStack {
            ForEach(Metrics.crowdXs, id: \.self) { x in
                RoundedRectangle(cornerRadius: Metrics.crowdCorner * k, style: .continuous)
                    .fill(Ink.primary.opacity(Metrics.crowdAlpha))
                    .frame(width: Metrics.crowdSize.width * k,
                           height: Metrics.crowdSize.height * k)
                    .offset(x: (x + Metrics.crowdSize.width / 2 - 12) * k,
                            y: (Metrics.crowdY + Metrics.crowdSize.height / 2 - 12) * k)
            }

            RoundedRectangle(cornerRadius: Metrics.surferCorner * k, style: .continuous)
                .fill(Ink.accent)
                .frame(width: Metrics.surfer.width * k, height: Metrics.surfer.height * k)
                .rotationEffect(.degrees(-Metrics.surferTilt))
                .offset(x: (Metrics.surfer.midX - 12) * k,
                        y: (Metrics.surfer.midY - 12) * k)
                .shadow(color: Ink.accent.opacity(0.8), radius: 2.6 * k)
        }
        .frame(width: side, height: side)
    }
}

private struct MoshpitSlashShape: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: rect.maxX * 0.78, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX * 0.37, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.maxX * 0.15, y: rect.maxY))
        path.closeSubpath()
        return path
    }
}

/// Top-level screen backdrop. A dim terminal grid and diagonal scan band give
/// the app a physical "signal console" identity while staying quiet behind
/// dense controls.
struct MoshpitBackground: View {
    var body: some View {
        ZStack {
            LinearGradient(
                colors: [Ink.backgroundTop, Ink.backgroundMid, Ink.backgroundBottom],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )

            SignalGrid()
                .stroke(Ink.terminalGrid, lineWidth: 0.6)
                .ignoresSafeArea()

            LinearGradient(
                colors: [.clear, Ink.accent.opacity(0.035), .clear],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            .blendMode(.screen)
        }
        .ignoresSafeArea()
    }
}

/// Internal (not private): the terminal's connecting screen reuses this grid
/// as its stage texture, so the two backdrops can't drift apart.
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
        MoshpitBackground()
        VStack(spacing: 24) {
            MoshpitMark(size: 96)
            MoshpitMark(size: 44)
            MoshpitMark(size: 24)
        }
    }
}
