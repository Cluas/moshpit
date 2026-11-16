import Foundation
import Testing
@testable import Moshpit

/// B754: a layer whose drawables do not come back must not hold the main
/// thread in `nextDrawable()` a second at a time, refresh after refresh;
/// and a frame still owed is never given up on (B445).
@Suite("Drawable pacing (B754)")
struct CoreTermDrawablePacingTests {

    @Test("No stall: ask, unless every drawable but the one on screen is ours and on its way")
    func noStall() {
        let p = CoreTermDrawablePacing()
        #expect(p.decide(now: 10, held: 0, drawables: 3, lastAcquiredAt: 9.99) == .attempt)
        #expect(p.decide(now: 10, held: 1, drawables: 3, lastAcquiredAt: 9.99) == .attempt)
        #expect(p.decide(now: 10, held: 2, drawables: 3, lastAcquiredAt: 9.99) == .skip)
        #expect(p.decide(now: 10, held: 3, drawables: 3, lastAcquiredAt: 9.99) == .skip)
        // Ours, unheard of past a frame's fate timeout: counted as gone
        // (a drawable let go without a word), so the frame is drawn.
        #expect(p.decide(now: 10, held: 3, drawables: 3, lastAcquiredAt: 9.4) == .attempt)
        #expect(!p.stalled)
    }

    @Test("After an ask that got nothing: 0.5 s, then 1 s between asks")
    func backOff() {
        var p = CoreTermDrawablePacing()
        p.failed(at: 100)
        #expect(p.stalled)
        #expect(p.decide(now: 100.1, held: 0, drawables: 3, lastAcquiredAt: 0) == .wait(until: 100.5))
        #expect(p.decide(now: 100.5, held: 0, drawables: 3, lastAcquiredAt: 0) == .attempt)
        p.failed(at: 101.5)
        #expect(p.decide(now: 102, held: 0, drawables: 3, lastAcquiredAt: 0) == .wait(until: 102.5))
        #expect(p.decide(now: 102.5, held: 0, drawables: 3, lastAcquiredAt: 0) == .attempt)
        p.failed(at: 103.5)
        #expect(p.decide(now: 103.6, held: 0, drawables: 3, lastAcquiredAt: 0) == .wait(until: 104.5))
    }

    @Test("A reason to think a drawable is free: ask at once, then pace again")
    func reason() {
        var p = CoreTermDrawablePacing()
        p.failed(at: 100)
        p.failed(at: 101.5)
        p.noteReason()
        #expect(p.decide(now: 101.6, held: 0, drawables: 3, lastAcquiredAt: 0) == .attempt)
        p.failed(at: 102.6)
        #expect(p.decide(now: 102.7, held: 0, drawables: 3, lastAcquiredAt: 0) == .wait(until: 103.6))
    }

    @Test("A reason with no stall on changes nothing")
    func reasonWithoutStall() {
        var p = CoreTermDrawablePacing()
        p.noteReason()
        #expect(p == CoreTermDrawablePacing())
    }

    @Test("A drawable ends the stall: one line's worth (start, end, asks), then nothing")
    func stallEnds() {
        var p = CoreTermDrawablePacing()
        #expect(p.acquired(at: 5) == nil, "no stall, nothing to say")
        p.failed(at: 100)
        p.failed(at: 101.5)
        p.failed(at: 103.5)
        #expect(p.acquired(at: 105.2) == .init(start: 100, end: 105.2, attempts: 4))
        #expect(!p.stalled)
        #expect(p.acquired(at: 105.3) == nil, "said once")
        #expect(p.decide(now: 105.3, held: 0, drawables: 3, lastAcquiredAt: 105.2) == .attempt)
    }

    @Test("A stall the view leaves: its line counts the asks that got nothing, then nothing")
    func stallAbandoned() {
        var p = CoreTermDrawablePacing()
        #expect(p.abandon(at: 5) == nil)
        p.failed(at: 100)
        p.failed(at: 101.5)
        #expect(p.abandon(at: 102) == .init(start: 100, end: 102, attempts: 2))
        #expect(!p.stalled)
        #expect(p.abandon(at: 103) == nil)
    }

    /// B445 (the diff-peer contract): a frame that has not reached the
    /// screen is drawn until it does. However long the layer gives
    /// nothing, and with nothing else happening (no completion, no size,
    /// no window, no foreground), the frame is asked for again, at most a
    /// second after the last ask; and the asks leave the main thread free
    /// at least half the time even though each holds it the full second.
    @Test("A frame still owed after many failures is asked for again with no event")
    func neverGivesUp() {
        var p = CoreTermDrawablePacing()
        var now: TimeInterval = 1000
        var asks: [TimeInterval] = []
        // Drawables taken before the stall that are never heard of again.
        let held = 3, lastAcquiredAt: TimeInterval = 999.9
        for _ in 0..<200 {
            var decision = p.decide(now: now, held: held, drawables: 3, lastAcquiredAt: lastAcquiredAt)
            var steps = 0
            while decision != .attempt {
                steps += 1
                guard steps < 100 else { break }
                switch decision {
                case .wait(let until):
                    #expect(until > now, "a wait moves time on")
                    now = until
                case .skip:
                    now += 1.0 / 60   // the next refresh
                case .attempt:
                    break
                }
                decision = p.decide(now: now, held: held, drawables: 3, lastAcquiredAt: lastAcquiredAt)
            }
            #expect(decision == .attempt, "asked again after \(asks.count) asks")
            guard decision == .attempt else { return }
            asks.append(now)
            now += 1.0          // nextDrawable's timeout, the main thread held
            p.failed(at: now)
        }
        let gaps = zip(asks.dropFirst(), asks).map { $0 - $1 }
        #expect(gaps.allSatisfy { $0 <= 2.0 + 1e-9 }, "an ask at most a second after the last came back")
        let span = asks.last! + 1 - asks.first!
        #expect(Double(asks.count) / span <= 0.5 + 0.01, "the main thread held at most half the time")
    }

    @Test("Drawables held: those taken lately and not given back; one never heard of stops counting")
    func taken() {
        let t = CoreTermTakenDrawables()
        #expect(t.held(now: 0, within: 0.5) == 0)
        #expect(t.lastTakenAt == 0)
        let a = t.take(at: 10), b = t.take(at: 10.01)
        #expect(t.held(now: 10.02, within: 0.5) == 2)
        #expect(t.lastTakenAt == 10.01)
        t.giveBack(a)
        #expect(t.held(now: 10.02, within: 0.5) == 1)
        t.giveBack(a)
        #expect(t.held(now: 10.02, within: 0.5) == 1, "given back once")
        // b is never heard of again (let go without a word).
        #expect(t.held(now: 10.6, within: 0.5) == 0)
        _ = b
        // Many never heard of: forgotten, the ledger stays small.
        for k in 0..<1000 { _ = t.take(at: 20 + Double(k) * 0.016) }
        #expect(t.held(now: 36.1, within: 0.5) <= 32)
    }
}
