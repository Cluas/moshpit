import Foundation
import Testing
import MoshpitKit

/// The Live Activity's companion still is decoded here exactly as the core
/// encodes it (`crates/pet`'s `Sprite::encode`; its test
/// `the_live_activitys_format_is_pinned` pins the same string).
struct PixelSpriteTests {
    /// A 2 × 2 sprite, one ink (#123456 light, #ABCDEF dark), on the
    /// diagonal from the top right.
    static let pinned = "px1:AQIBARI0VqvN7wAJAA=="

    @Test func thePinnedSpriteDecodes() throws {
        let sprite = try #require(PixelSprite(encoded: Self.pinned))
        #expect(sprite.side == 2)
        #expect(sprite.cells == [0, 1, 1, 0])
        #expect(sprite.inks == [PixelSprite.Ink(light: 0x123456, dark: 0xABCDEF)])
    }

    @Test func whatIsNotASpriteIsNone() {
        for bad in ["", "px1:", "px2:AQIBARI0VqvN7wAJAA==", "px1:!!!!", "px1:AQIB", "px1:AQMBARI0VqvN7wAJAA=="] {
            #expect(PixelSprite(encoded: bad) == nil, "\(bad)")
        }
    }
}
