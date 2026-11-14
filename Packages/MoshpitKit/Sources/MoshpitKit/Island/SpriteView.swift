import Foundation
import SwiftUI

/// The companion's still on the Live Activity (moshpit-design
/// `docs/nsh/PET-DESIGN.md` §5, §16.2): a pixel sprite the app's core
/// composed and encoded (`moshpit-pet`'s `Sprite::encode`, `px1:` and
/// base64), drawn here as filled rects. The widget never derives a pet: Rust
/// is the only renderer, and this only draws what it was given.
public struct PixelSprite: Equatable, Sendable {
    public struct Ink: Equatable, Sendable {
        public let light: UInt32
        public let dark: UInt32

        /// An ink's two colours, `0xRRGGBB` (the tests compare decoded
        /// inks against these; a struct's own initializer isn't public).
        public init(light: UInt32, dark: UInt32) {
            self.light = light
            self.dark = dark
        }
    }

    public let side: Int
    /// Palette indices, row by row; 0 is empty, `n` is `inks[n - 1]`.
    public let cells: [UInt8]
    public let inks: [Ink]

    /// The sprite `encoded` says, or nil when it isn't one.
    public init?(encoded: String) {
        guard encoded.hasPrefix("px1:"),
              let bytes = Data(base64Encoded: String(encoded.dropFirst(4))).map(Array.init),
              bytes.count >= 3 else { return nil }
        let format = bytes[0], side = Int(bytes[1]), n = Int(bytes[2])
        var at = 3
        let maskLength = (n + 7) / 8
        guard bytes.count >= at + maskLength + n * 3 else { return nil }
        let mask = Array(bytes[at..<at + maskLength])
        at += maskLength
        func differs(_ i: Int) -> Bool { mask[i / 8] >> (i % 8) & 1 == 1 }
        func rgb(_ i: Int) -> UInt32 { UInt32(bytes[i]) << 16 | UInt32(bytes[i + 1]) << 8 | UInt32(bytes[i + 2]) }
        var lights: [UInt32] = []
        for i in 0..<n { lights.append(rgb(at + i * 3)) }
        at += n * 3
        var inks: [Ink] = []
        for i in 0..<n {
            if differs(i) {
                guard bytes.count >= at + 3 else { return nil }
                inks.append(Ink(light: lights[i], dark: rgb(at)))
                at += 3
            } else {
                inks.append(Ink(light: lights[i], dark: lights[i]))
            }
        }
        let rest = bytes[at...]
        var cells: [UInt8] = []
        switch format {
        case 1:
            for b in rest { cells.append(contentsOf: repeatElement(b >> 3, count: Int(b & 7) + 1)) }
        case 2:
            cells = Array(rest)
        default:
            return nil
        }
        guard side > 0, cells.count == side * side, cells.allSatisfy({ Int($0) <= n }) else { return nil }
        self.side = side
        self.cells = cells
        self.inks = inks
    }

    /// The cells of ink `index` (1-based), as one path of row runs, `cell`
    /// points a cell.
    func path(ink index: UInt8, cell: CGFloat) -> Path {
        var path = Path()
        for y in 0..<side {
            var x = 0
            while x < side {
                guard cells[y * side + x] == index else { x += 1; continue }
                var run = 1
                while x + run < side, cells[y * side + x + run] == index { run += 1 }
                path.addRect(CGRect(x: CGFloat(x) * cell, y: CGFloat(y) * cell,
                                    width: CGFloat(run) * cell, height: cell))
                x += run
            }
        }
        return path
    }
}

/// Draws a ``PixelSprite`` `size` points square: a whole number of pixels
/// a cell at 24, 32, 40 and 48 pt on a 3x screen. Decorative to VoiceOver:
/// the words beside it say the state.
public struct SpriteView: View {
    private let sprite: PixelSprite?
    private let size: CGFloat
    /// The island is always dark, whatever the system's appearance.
    private let alwaysDark: Bool
    @Environment(\.colorScheme) private var scheme

    public init(encoded: String?, size: CGFloat, alwaysDark: Bool = false) {
        sprite = encoded.flatMap(PixelSprite.init(encoded:))
        self.size = size
        self.alwaysDark = alwaysDark
    }

    public var body: some View {
        if let sprite {
            let cell = size / CGFloat(sprite.side)
            let dark = alwaysDark || scheme == .dark
            ZStack(alignment: .topLeading) {
                ForEach(0..<sprite.inks.count, id: \.self) { i in
                    let ink = sprite.inks[i]
                    sprite.path(ink: UInt8(i + 1), cell: cell).fill(Self.color(dark ? ink.dark : ink.light))
                }
            }
            .frame(width: size, height: size, alignment: .topLeading)
            .accessibilityHidden(true)
        }
    }

    static func color(_ rgb: UInt32) -> Color {
        Color(red: Double(rgb >> 16 & 0xFF) / 255, green: Double(rgb >> 8 & 0xFF) / 255,
              blue: Double(rgb & 0xFF) / 255)
    }
}
