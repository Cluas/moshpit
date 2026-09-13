import CoreGraphics
import Testing
@testable import Moshpit

/// The two pure pieces behind finger-driven scrolling: `ScrollRowCarry` turns
/// travel into rows without losing the remainder, `ScrollFling` is the coast
/// after a lift. Both are plain values so the maths can be pinned here; the
/// gesture that drives them only feeds in translations and frame times.
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

    @Test("a zero cell height or a non-finite delta yields nothing")
    func degenerate() {
        var carry = ScrollRowCarry()
        #expect(carry.take(pixels: 100, cellHeight: 0) == 0)
        #expect(carry.take(pixels: .nan, cellHeight: 10) == 0)
        #expect(carry.remainder == 0)
    }

    @Test("a lift below the flick threshold does not coast")
    func noCoastForSlowLift() {
        #expect(ScrollFling(velocity: 50) == nil)
        #expect(ScrollFling(velocity: -(ScrollFling.minimumVelocity - 1)) == nil)
        #expect(ScrollFling(velocity: .nan) == nil)
        #expect(ScrollFling(velocity: 400) != nil)
    }

    @Test("the coast covers UIScrollView's distance and comes to rest in about two seconds")
    func coastDistance() {
        guard var fling = ScrollFling(velocity: 1000) else {
            Issue.record("1000 pt/s must coast")
            return
        }
        var travelled: CGFloat = 0
        var frames = 0
        while !fling.isAtRest, frames < 10_000 {
            travelled += fling.step(dt: 1.0 / 60)
            frames += 1
        }
        // v0 · rate / (1 − rate) per ms ≈ 499 pt for the normal deceleration
        // rate, less the few points left under the rest threshold.
        #expect(travelled > 480 && travelled < 500)
        #expect(fling.isAtRest)
        #expect(frames > 60 && frames < 300)
    }

    @Test("frame rate does not change where the coast ends")
    func frameRateIndependent() {
        func travel(fps: Double) -> CGFloat {
            var fling = ScrollFling(velocity: 2000)!
            var total: CGFloat = 0
            while !fling.isAtRest { total += fling.step(dt: 1 / fps) }
            return total
        }
        let at60 = travel(fps: 60)
        let at120 = travel(fps: 120)
        #expect(abs(at60 - at120) < 3)
    }

    @Test("direction follows the lift and the speed is capped")
    func directionAndClamp() {
        var back = ScrollFling(velocity: -1000)!
        #expect(back.step(dt: 1.0 / 60) < 0)
        let capped = ScrollFling(velocity: 50_000)!
        #expect(capped.velocity == ScrollFling.maximumVelocity)
    }
}
