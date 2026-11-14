import CoreGraphics
import Testing
@testable import Moshpit

/// The pure piece behind finger-driven scrolling: `ScrollRowCarry` turns
/// travel into rows without losing the remainder. A plain value so the maths
/// can be pinned here; the gesture that drives it only feeds in translations.
@Suite("Scroll motion")
struct ScrollMotionTests {

    @Test("sub-row travel carries over instead of being dropped")
    func carry() {
        var carry = ScrollRowCarry()
        #expect(carry.take(pixels: 7, cellHeight: 10) == 0)
        #expect(carry.take(pixels: 7, cellHeight: 10) == 1)    // 14 → one row, 4 left
        #expect(carry.take(pixels: -9, cellHeight: 10) == 0)   // 4 − 9 = −5
        #expect(carry.take(pixels: -6, cellHeight: 10) == -1)  // −11 → one row back, −1 left
        #expect(carry.take(pixels: 31, cellHeight: 10) == 3)   // 30 → three rows, nothing left
        #expect(carry.remainder == 0)
    }

    @Test("a slow drag adds up to the same rows as one fast one")
    func slowEqualsFast() {
        var slow = ScrollRowCarry()
        var fast = ScrollRowCarry()
        // 3.25 is exact in binary, so a hundred of them are exactly 325.
        var slowRows = 0
        for _ in 0..<100 { slowRows += slow.take(pixels: 3.25, cellHeight: 13) }
        let fastRows = fast.take(pixels: 325, cellHeight: 13)
        #expect(fastRows == 25)
        #expect(slowRows == fastRows)
    }

    /// B149: `Int(_:)` trapped on rows past Int's range.
    @Test("absurd travel saturates instead of trapping, and the carry keeps working after it")
    func absurdTravel() {
        var carry = ScrollRowCarry()
        // 1e300 / 1e-300 is past Double: infinite rows, the most Int says.
        #expect(carry.take(pixels: 1e300, cellHeight: 1e-300) == .max)
        #expect(carry.remainder == 0, "a remainder past Double's range starts over")
        #expect(carry.take(pixels: 25, cellHeight: 10) == 2, "and the next drag counts as ever")
        var down = ScrollRowCarry()
        #expect(down.take(pixels: -1e300, cellHeight: 1) == .min)
        #expect(down.take(pixels: -1e300, cellHeight: .leastNonzeroMagnitude) == .min)
        #expect(down.take(pixels: -15, cellHeight: 10) == -1)
    }

    @Test("a zero cell height or a non-finite delta yields nothing")
    func degenerate() {
        var carry = ScrollRowCarry()
        #expect(carry.take(pixels: 100, cellHeight: 0) == 0)
        #expect(carry.take(pixels: .nan, cellHeight: 10) == 0)
        #expect(carry.remainder == 0)
    }
}
