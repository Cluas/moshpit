import Foundation

/// When a terminal asks its layer for a drawable (B754).
///
/// `CAMetalLayer.nextDrawable()` holds the main thread until a drawable is
/// free, up to a second (`allowsNextDrawableTimeout`). A layer whose
/// drawables do not come back (the screen not taking frames: a launch
/// still settling, a window not on screen yet) answered nil after that
/// second, and the terminal asked again at the next refresh: the main
/// thread spent nearly all its time waiting, and the app froze until
/// something laid it out again.
///
/// So: no ask while every drawable but the one on screen is ours and on
/// its way (the next to come back frees one). After an ask that got
/// nothing, the next waits 0.5 s, then a second between asks, sooner when
/// there is a reason to think a drawable is free: a frame of ours
/// completed, a new size or window, the app active again, a new layer. A
/// frame still owed is never given up on (B445, the diff-peer contract):
/// the slow ask goes on, with no event needed, until one is drawn.
struct CoreTermDrawablePacing: Equatable {
    enum Decision: Equatable {
        /// Ask now.
        case attempt
        /// Not this refresh: every drawable is ours and on its way, and the
        /// next to come back frees one.
        case skip
        /// Not before `until` (`CACurrentMediaTime`'s clock).
        case wait(until: TimeInterval)
    }

    /// The wait after the first ask that got nothing.
    static let firstPause: TimeInterval = 0.5
    /// The wait after each later one.
    static let pause: TimeInterval = 1
    /// How long drawables of ours may go unheard of before they count as
    /// gone: a frame's fate timeout (``CoreTermPresentation/fateTimeout``).
    static let heldFor: TimeInterval = 0.5

    private var failures = 0
    private var lastFailureAt: TimeInterval = 0
    private var stallStart: TimeInterval = 0
    private var reason = false

    /// Whether to ask now. `held`: drawables of this layer taken and not
    /// heard of since; `drawables`: how many the layer has;
    /// `lastAcquiredAt`: when one last came.
    func decide(now: TimeInterval, held: Int, drawables: Int, lastAcquiredAt: TimeInterval) -> Decision {
        if failures > 0, !reason {
            let due = lastFailureAt + (failures == 1 ? Self.firstPause : Self.pause)
            if now < due { return .wait(until: due) }
        }
        // One drawable is the screen's (the picture shown); the others,
        // all ours and on their way, leave none to take without waiting.
        if held >= max(drawables - 1, 1), now - lastAcquiredAt < Self.heldFor { return .skip }
        return .attempt
    }

    /// An ask came back with nothing at `now`.
    mutating func failed(at now: TimeInterval) {
        if failures == 0 { stallStart = now }
        failures += 1
        lastFailureAt = now
        reason = false
    }

    /// A drawable came at `now`: the stall it ends, if one was on.
    mutating func acquired(at now: TimeInterval) -> Stall? {
        defer { self = Self() }
        guard failures > 0 else { return nil }
        return Stall(start: stallStart, end: now, attempts: failures + 1)
    }

    /// The stall on, if one is, given up at `now` with no drawable (the
    /// view left its window: coming back is a reason of its own).
    mutating func abandon(at now: TimeInterval) -> Stall? {
        defer { self = Self() }
        guard failures > 0 else { return nil }
        return Stall(start: stallStart, end: now, attempts: failures)
    }

    /// Something that may have freed a drawable: a frame of ours completed,
    /// a new size or window, the app active again, a new layer.
    mutating func noteReason() {
        if failures > 0 { reason = true }
    }

    /// Whether asks are coming back with nothing.
    var stalled: Bool { failures > 0 }

    /// A run of asks that came back with nothing, as it ended.
    struct Stall: Equatable {
        var start: TimeInterval
        var end: TimeInterval
        /// Asks, the one that got a drawable included.
        var attempts: Int
    }
}

/// The drawables a layer handed out and has not heard of since, each by
/// when it was taken (B754). Thread-safe: taken on the main thread, given
/// back on Metal's.
///
/// By time, not a bare count: a drawable the system lets go without a word
/// (B445) is never given back, and a count would keep it held for good,
/// a terminal that skips every refresh for it. One taken longer ago than
/// the pacing's window stops counting.
final class CoreTermTakenDrawables: @unchecked Sendable {
    private let lock = NSLock()
    private var next = 0
    private var taken: [Int: TimeInterval] = [:]
    private var last: TimeInterval = 0

    /// How long one is kept unheard of before it is forgotten.
    static let forgetAfter: TimeInterval = 5

    /// A drawable taken at `now`: its ticket, to give back.
    func take(at now: TimeInterval) -> Int {
        lock.lock()
        defer { lock.unlock() }
        next += 1
        taken[next] = now
        last = now
        if taken.count > 8 { taken = taken.filter { now - $0.value < Self.forgetAfter } }
        return next
    }

    /// Its fate is known: ticket `ticket` is no longer held.
    func giveBack(_ ticket: Int) {
        lock.lock()
        defer { lock.unlock() }
        taken[ticket] = nil
    }

    /// Those taken less than `within` seconds before `now` and not given
    /// back.
    func held(now: TimeInterval, within: TimeInterval) -> Int {
        lock.lock()
        defer { lock.unlock() }
        return taken.values.count { now - $0 < within }
    }

    /// When one was last taken, 0 before any.
    var lastTakenAt: TimeInterval {
        lock.lock()
        defer { lock.unlock() }
        return last
    }
}
