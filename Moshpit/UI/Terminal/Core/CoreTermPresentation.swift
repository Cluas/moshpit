import Foundation

/// Whether the frames a terminal drew reached the screen (B445).
///
/// The core's terminal draws only when something changes: bytes, a scroll,
/// the cursor's blink (off for some), a new size. A frame the layer drops
/// (drawn while the screen was not taking it: a push still settling, the
/// first layout of a page) or the GPU refuses never shows, and an idle pane
/// sends nothing that would draw again. The terminal stayed blank over rows
/// that were right — the keyboard's cover, drawn from the rows, flashed
/// them — until something unrelated drew (pulling down Notification Center
/// and letting go: the app active again). So every frame is followed to the
/// screen: one that never got there, with nothing after it on its way, is
/// drawn again, a few times in a row at most.
///
/// Thread-safe: frames are resolved on Metal's threads, decided on the main.
final class CoreTermPresentation: @unchecked Sendable {
    /// Frames in a row that may be drawn again before the terminal gives up
    /// on its layer (and makes a new one).
    static let retryCap = 8

    private let lock = NSLock()
    private var issued = 0
    private var resolvedUpTo = 0
    private var shownUpTo = 0
    private var shownCount = 0
    private var neverShown = 0
    private var streak = 0

    /// What to do about a frame that never reached the screen.
    enum Verdict: Equatable {
        /// Nothing is lost: a later frame reached the screen or is on its
        /// way, or the app is not active (its return draws).
        case nothingLost
        /// Draw again: the screen has nothing newer than what was lost.
        case drawAgain
        /// ``retryCap`` frames in a row never got there: the layer is not
        /// to be trusted (a new canvas). Said once a streak.
        case giveUp
    }

    /// A frame is on its way to the screen: its number.
    func issue() -> Int {
        lock.lock()
        defer { lock.unlock() }
        issued += 1
        return issued
    }

    /// Frame `frame` reached the screen.
    func shown(_ frame: Int) {
        lock.lock()
        defer { lock.unlock() }
        shownUpTo = max(shownUpTo, frame)
        resolvedUpTo = max(resolvedUpTo, frame)
        shownCount += 1
        streak = 0
    }

    /// Frame `frame` never reached the screen (dropped by the layer, or
    /// refused by the GPU); `active`: the app is active, the view in a
    /// window and drawing.
    func neverReached(_ frame: Int, active: Bool) -> Verdict {
        lock.lock()
        defer { lock.unlock() }
        neverShown += 1
        resolvedUpTo = max(resolvedUpTo, frame)
        // A later frame got there, or one is still on its way (its own fate
        // decides).
        guard frame > shownUpTo, frame == issued, active else { return .nothingLost }
        streak += 1
        if streak <= Self.retryCap { return .drawAgain }
        return streak == Self.retryCap + 1 ? .giveUp : .nothingLost
    }

    /// Whether nothing has been heard of frame `frame` yet: a frame whose
    /// fate never comes (a drawable the system let go without a word) is
    /// taken for one that never got there.
    func unresolved(_ frame: Int) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return frame > resolvedUpTo
    }

    /// How long a frame may go unheard of before it counts as never shown.
    static let fateTimeout: TimeInterval = 0.5

    /// Frames that reached the screen so far.
    var shownFrames: Int {
        lock.lock()
        defer { lock.unlock() }
        return shownCount
    }

    /// Frames that never did.
    var framesNeverShown: Int {
        lock.lock()
        defer { lock.unlock() }
        return neverShown
    }

    /// Frames in a row that never got there, now.
    var currentStreak: Int {
        lock.lock()
        defer { lock.unlock() }
        return streak
    }
}

/// One frame's fate, said once: the drawable's presentation and the
/// command buffer's error can both report it.
final class CoreTermFrameFate: @unchecked Sendable {
    private let lock = NSLock()
    private var told = false
    private let tell: @Sendable (Bool) -> Void

    init(_ tell: @escaping @Sendable (Bool) -> Void) {
        self.tell = tell
    }

    /// Whether the frame reached the screen; only the first word counts.
    func resolve(shown: Bool) {
        lock.lock()
        let first = !told
        told = true
        lock.unlock()
        if first { tell(shown) }
    }
}
