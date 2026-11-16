import Foundation
import SwiftUI
import Testing
import UIKit
@testable import Moshpit
import MoshpitKit

/// The lock-screen card reads on every lock screen (B811).
///
/// iOS 27 resolves a Live Activity's background tint apart from its content,
/// and the two can come out in different appearances: on a light lock screen
/// the tint came out light (white) while the words were drawn dark (white),
/// and the card was white on white until the next update. So the card is
/// checked here in all four pairings of the two — content light or dark,
/// tint light or dark — over a white and a black wallpaper, with the tint
/// the card itself names: in each, the header's words must stand out from
/// the ground behind them.
@MainActor
@Suite("Lock screen card appearance")
struct LockScreenAppearanceTests {

    /// B811's card: one waiting, one working (their timers tick, so this
    /// one is only read, never compared render to render).
    static let waiting: AgentActivityAttributes.ContentState = {
        let agents = [
            LockScreenHeightTests.agent("gemini", state: .attention,
                                        detail: "Allow execution of: 'python eval.py'?"),
            LockScreenHeightTests.agent("claude", state: .working),
        ]
        return AgentActivityAttributes.ContentState(
            agents: agents, workingCount: 1, attentionCount: 1, headlineDeepLink: nil)
    }()

    /// A card with no clock on it (done and idle rows draw no timer), so two
    /// renders of it can be compared pixel for pixel.
    static let still: AgentActivityAttributes.ContentState = {
        let agents = [
            LockScreenHeightTests.agent("claude", state: .done),
            LockScreenHeightTests.agent("gemini", state: .idle),
        ]
        return AgentActivityAttributes.ContentState(
            agents: agents, workingCount: 0, attentionCount: 0, headlineDeepLink: nil)
    }()

    /// The card as the lock screen composes it: the wallpaper, the card's
    /// tint resolved in `tint`, and the card itself drawn in `content`.
    static func render(_ state: AgentActivityAttributes.ContentState, content: ColorScheme,
                       tint: ColorScheme, wallpaper: Color) -> CardPixels? {
        let card = LockScreenCard(state: state)
            .environment(\.colorScheme, content)
            .frame(width: LockScreenHeightTests.width)
            .background {
                ZStack {
                    wallpaper
                    Rectangle().fill(LockScreenCard.tint)
                }
                .environment(\.colorScheme, tint)
            }
        let renderer = ImageRenderer(content: card)
        renderer.scale = 1
        return renderer.cgImage.flatMap(CardPixels.init)
    }

    @Test("the card's tint and system colour are one colour in either appearance")
    func fixedColours() {
        var light = EnvironmentValues()
        light.colorScheme = .light
        var dark = EnvironmentValues()
        dark.colorScheme = .dark
        #expect(LockScreenCard.tint.resolve(in: light) == LockScreenCard.tint.resolve(in: dark))
        #expect(LockScreenCard.actionColor.resolve(in: light) == LockScreenCard.actionColor.resolve(in: dark))
        #expect(LockScreenCard.scheme == .dark)
    }

    @Test("the card draws the same in a light and a dark appearance")
    func contentIgnoresTheAppearance() throws {
        for wallpaper in [Color.white, Color.black] {
            let light = try #require(Self.render(Self.still, content: .light, tint: .light, wallpaper: wallpaper))
            let dark = try #require(Self.render(Self.still, content: .dark, tint: .light, wallpaper: wallpaper))
            #expect(light.width == dark.width && light.height == dark.height)
            #expect(light.pixels == dark.pixels, "the card's words followed the lock screen's appearance")
        }
    }

    @Test("the header reads whichever appearance the tint and the words come out in")
    func headerReadsInEveryPairing() throws {
        for wallpaper in [Color.white, Color.black] {
            for content in [ColorScheme.light, .dark] {
                for tint in [ColorScheme.light, .dark] {
                    let card = try #require(Self.render(Self.waiting, content: content, tint: tint,
                                                        wallpaper: wallpaper))
                    let what = "content \(content), tint \(tint), wallpaper \(wallpaper)"
                    // The ground: a corner, inside the card's padding.
                    let ground = card.luminance(x: 2, y: 2)
                    #expect(ground < 0.05, "\(what): the ground isn't dark (\(ground))")
                    // "Moshpit" and the summary, in the header's band
                    // (12 pt padding, 13 pt type): enough of it at 4.5:1
                    // or more against that ground to be words, not edges.
                    let readable = card.count(rows: 10..<30) { luminance in
                        CardPixels.contrast(luminance, ground) >= 4.5
                    }
                    #expect(readable >= 60, "\(what): \(readable) readable pixels in the header")
                }
            }
        }
    }
}

/// A rendered image's pixels, RGBA in sRGB, 8 bits each.
struct CardPixels {
    let width: Int
    let height: Int
    let pixels: [UInt8]

    init?(_ image: CGImage) {
        // Locals, not `self.width`/`self.height`: the closure below runs
        // before `pixels` is set, and a closure may not capture `self` then.
        let width = image.width
        let height = image.height
        var bytes = [UInt8](repeating: 0, count: width * height * 4)
        let drawn = bytes.withUnsafeMutableBytes { buffer -> Bool in
            guard let space = CGColorSpace(name: CGColorSpace.sRGB),
                  let context = CGContext(data: buffer.baseAddress, width: width, height: height,
                                          bitsPerComponent: 8, bytesPerRow: width * 4, space: space,
                                          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            else { return false }
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        guard drawn else { return nil }
        self.width = width
        self.height = height
        pixels = bytes
    }

    /// Relative luminance (WCAG) of the pixel at `x`, `y` (from the top).
    func luminance(x: Int, y: Int) -> Double {
        let at = (y * width + x) * 4
        return Self.luminance(pixels[at], pixels[at + 1], pixels[at + 2])
    }

    /// How many pixels in `rows` (from the top) satisfy `test`, by luminance.
    func count(rows: Range<Int>, where test: (Double) -> Bool) -> Int {
        var n = 0
        for y in rows where y < height {
            for x in 0..<width where test(luminance(x: x, y: y)) { n += 1 }
        }
        return n
    }

    static func luminance(_ r: UInt8, _ g: UInt8, _ b: UInt8) -> Double {
        func linear(_ c: UInt8) -> Double {
            let s = Double(c) / 255
            return s <= 0.04045 ? s / 12.92 : pow((s + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    /// WCAG contrast ratio of two luminances.
    static func contrast(_ a: Double, _ b: Double) -> Double {
        (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }
}
