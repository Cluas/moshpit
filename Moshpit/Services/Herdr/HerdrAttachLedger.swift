import Foundation

/// The frame channel's attaches, as bookkeeping: which pane the channel is
/// pointed at, which attach reads what is written into it, which one is
/// being released, and which `terminal.closed` is our own release coming
/// back. The I/O stays in `SessionHub` (lines on the write lane, the veil,
/// the timers); this is what it decides by. After the Rust client's
/// `crates/herdr/src/client/attach.rs`, the reference (docs/nsh/PROGRESS.md:
/// the Rust core is the primary implementation).
///
/// The channel renders one pane at a time, and an attach owns it until it
/// exits. Pointing it elsewhere is: release the attach that is reading
/// (`terminal.release`), wait for it to let go — its `terminal.closed` — and
/// then name the next pane, one line to the reader loop (or the boot line, or
/// the one-shot start line). A line written before the old attach has let go
/// is read by it and lost.
///
/// herdr answers releases written back to back with ONE close (herdr 0.9.5,
/// measured): the first ends the attach, and the reader loop's `read` drops
/// the second (a line opening with `{`). So only one release is ever out. A
/// switch that overtakes another while its release is out writes none of its
/// own and waits for that same close (B186: a release per switch left the
/// count of closes we were owed one too high, and the next close nobody
/// asked for — another client's `--takeover`, the pane closed — was taken
/// for it: the pane stayed "attached", typing went into the loop's `read`,
/// the screen froze until the next switch). And a close that is owed but
/// never comes is forgotten after ``expectedCloseLife``, so no count can
/// stay high for good.
///
/// Frames belong to the attach that printed them (B187). Until its close,
/// the attach being released still prints: those frames are the pane being
/// left's — not the new pane's screen, and not its paint. Taken for its
/// paint, a new attach whose line was then lost never counted as unpainted,
/// so the poll's repeated focus report never asked for it again.
///
/// Input goes to a pane only once that pane's attach is up — its first frame
/// painted (B185). herdr 0.9.5 drops what reaches an attach before then:
/// probed on a PTY with the reader loop as the app boots it, 9 of 10 lines
/// written right behind the attach line were lost, 0 of 10 written after
/// its first frame. What the person typed at the pane they just picked, and
/// what was delivered to one pane (a notification's reply, a share), is held
/// for that pane and follows its first frame, in order; anything else is
/// dropped, and said — never typed into a pane nobody chose.
///
/// Only the person takes a pane from another device: an attach line asks for
/// `--takeover` (``takeover``) for a pick (a tap, a reply for the pane) and
/// "Open here", never for a switch that follows the shared focus — herdr
/// refuses that one while another client holds the terminal.
struct HerdrAttachLedger {

    /// How long a close we are owed is waited for before it is forgotten.
    /// Past the release's own wait (1.5 s) on purpose: a close that comes
    /// late is still ours, and taken for an eviction it would cost a
    /// re-attach for nothing.
    static let expectedCloseLife: Duration = .seconds(5)

    /// An attach asked for that hasn't painted may be asked for again past
    /// this: a line the shell ate, a boot the host couldn't run.
    static let attachTimeout: Duration = .seconds(3)

    /// The pane the channel is pointed at (`herdrFrameTarget`).
    private(set) var target: String?
    /// The pane whose attach line has been written: its attach reads what
    /// is written from then on.
    private(set) var attached: String?
    /// The pane whose release is out: its attach still reads until its close.
    private(set) var releasing: String?
    /// The pane a switch waits to name once the channel is free.
    private(set) var pending: String?
    /// The target's attach has painted since the target was asked for.
    private(set) var painted = false
    /// When the target was last asked for.
    private(set) var askedAt: ContinuousClock.Instant?
    /// The person chose the target (a pick, a reply, "Open here"): its
    /// attach line asks for `--takeover`. A follow's doesn't.
    private(set) var takeover = false
    /// What the attach line written last asked for.
    private(set) var lineTookOver = false

    private struct Expectation {
        let id: Int
        let until: ContinuousClock.Instant
    }

    /// Releases whose close hasn't come back, oldest first.
    private var expected: [Expectation] = []
    private var lastExpectation = 0

    /// How many closes we are owed.
    var expectedCloses: Int { expected.count }

    /// What a switch writes first.
    enum Step: Equatable {
        /// Release the attach that is reading, then wait for its close.
        case release
        /// A release is out already (a switch it was written for was
        /// overtaken): its close frees the channel for this one too.
        case awaitClose
        /// Nothing holds the channel: name the pane now.
        case attach
    }

    /// Whether a report of `pane` asks for an attach — the poll repeats the
    /// server's focus on every read: any pane but the target, or the target
    /// asked for and still unpainted past ``attachTimeout``. Free otherwise.
    func wantsAttach(_ pane: String, now: ContinuousClock.Instant) -> Bool {
        guard pane == target else { return true }
        guard !painted, let askedAt else { return false }
        return now - askedAt > Self.attachTimeout
    }

    /// Point the channel at `pane`. The caller has decided this is a switch
    /// (or a re-ask of an attach that never painted). `takeover`: the
    /// person chose it; a re-ask of the same pane keeps their word.
    mutating func retarget(to pane: String, takeover: Bool = false, now: ContinuousClock.Instant) -> Step {
        self.takeover = takeover || (target == pane && self.takeover)
        target = pane
        pending = pane
        painted = false
        askedAt = now
        if let live = attached {
            attached = nil
            releasing = live
            lastExpectation += 1
            expected.append(Expectation(id: lastExpectation, until: now + Self.expectedCloseLife))
            return .release
        }
        return releasing != nil ? .awaitClose : .attach
    }

    /// The person picked the pane the channel is on its way to (its attach
    /// not up yet): theirs, asked again with `--takeover` should a follow's
    /// plain attach be refused.
    mutating func pick(_ pane: String) {
        if target == pane, !painted { takeover = true }
    }

    /// The pending switch's attach line goes out now: the pane it names,
    /// whose attach reads from here on (``takeover``: whether the line asks
    /// for `--takeover`). nil when no switch is waiting.
    mutating func attachLineWritten() -> String? {
        guard let pane = pending else { return nil }
        pending = nil
        attached = pane
        lineTookOver = takeover
        return pane
    }

    /// Whose a frame is.
    enum Frame: Equatable {
        /// The attach being released: its last frames are the pane being
        /// left's, and say nothing about the pane asked for.
        case releasing
        /// The target's attach; `first`: its first frame since it was asked
        /// for — the attach is up.
        case target(first: Bool)
        /// No attach of ours asked for reads the channel.
        case stray
    }

    /// A frame came off the channel.
    mutating func frame() -> Frame {
        if releasing != nil { return .releasing }
        guard let pane = attached, pane == target else { return .stray }
        let first = !painted
        painted = true
        return .target(first: first)
    }

    /// What a `terminal.closed` was.
    enum Close: Equatable {
        /// Our release coming back: the channel is free.
        case ownRelease
        /// The attach reading the channel was ended by someone else — another
        /// client's takeover, its pane closed. Nothing is attached now.
        case evicted(pane: String)
        /// Nothing of ours was attached.
        case stray
    }

    mutating func closed(now: ContinuousClock.Instant) -> Close {
        expected.removeAll { $0.until <= now }
        if !expected.isEmpty {
            expected.removeFirst()
            releasing = nil
            return .ownRelease
        }
        guard let pane = attached else { return .stray }
        attached = nil
        target = nil
        takeover = false
        return .evicted(pane: pane)
    }

    /// The release's close didn't come in time: the channel may have stopped
    /// answering. The next line goes anyway; the close, if it still comes,
    /// is still ours until its expectation runs out.
    mutating func releaseAckTimedOut() {
        releasing = nil
    }

    /// herdr refused the attach (it said why): stop claiming the pane, and
    /// name no other — a refusal doesn't heal by asking again.
    mutating func refused() {
        target = nil
        attached = nil
        pending = nil
        takeover = false
    }

    /// The channel is gone (the session stops or rebuilds): nothing of it
    /// carries over. Whatever is held goes with it — take it first
    /// (``dropHeld()``) to answer it.
    mutating func reset() {
        self = HerdrAttachLedger()
    }

    // MARK: Input

    /// Input held for one pane until its attach is up.
    enum Held: Equatable {
        /// Typed at the pane the person just picked.
        case typed(Data)
        /// Delivered to one pane; a caller waits on it, by `id`.
        case delivery(id: Int, Data)
    }

    /// The pane input is held for, and what is held for it, in order.
    private(set) var heldPane: String?
    private(set) var held: [Held] = []

    /// Where input goes now: the target, once its attach has painted —
    /// never the pane being left, never the reader loop's `read`, never an
    /// attach that isn't up.
    var inputPane: String? {
        guard painted, let attached, attached == target else { return nil }
        return attached
    }

    /// What became of input.
    enum Route: Equatable {
        /// Its pane's attach is up: out now.
        case send
        /// Held for its pane until that attach paints.
        case held
        /// No pane it can rightly reach.
        case dropped
    }

    /// Input for `pane`: out now if its attach is up, held if input is held
    /// for it, dropped otherwise.
    mutating func route(_ input: Held, to pane: String?) -> Route {
        guard let pane else { return .dropped }
        if inputPane == pane { return .send }
        guard heldPane == pane else { return .dropped }
        held.append(input)
        return .held
    }

    /// Hold input for `pane` until its attach is up. A hold for another pane
    /// ends; what it held comes back, to be dropped.
    mutating func hold(for pane: String) -> [Held] {
        guard heldPane != pane else { return [] }
        let dropped = dropHeld()
        heldPane = pane
        return dropped
    }

    /// End the hold; what it held comes back, to be dropped.
    mutating func dropHeld() -> [Held] {
        let dropped = held
        heldPane = nil
        held = []
        return dropped
    }

    /// `pane`'s attach is up: what was held for it comes back to go out, in
    /// order, and the hold ends.
    mutating func takeHeld(for pane: String) -> [Held] {
        guard heldPane == pane else { return [] }
        return dropHeld()
    }
}
