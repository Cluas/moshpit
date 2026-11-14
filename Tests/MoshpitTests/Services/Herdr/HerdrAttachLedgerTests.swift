import Foundation
import Testing
@testable import Moshpit

/// The frame channel's bookkeeping on its own (`HerdrAttachLedger`), on a
/// clock the test holds: the Rust client's `attach.rs` cases
/// (`crates/herdr/tests/client.rs`), decision by decision.
@Suite("herdr attach ledger")
struct HerdrAttachLedgerTests {

    private let t0 = ContinuousClock.now

    /// Booted onto `pane`: its attach line written.
    private func attached(_ pane: String = "w1:p1") -> HerdrAttachLedger {
        var ledger = HerdrAttachLedger()
        #expect(ledger.retarget(to: pane, now: t0) == .attach)
        #expect(ledger.attachLineWritten() == pane)
        return ledger
    }

    // MARK: - B186

    /// herdr answers two releases with one close. The app wrote one release
    /// per switch, so after A → B → A it waited for a close that never
    /// comes, and took the next eviction for it.
    @Test("Quick switches write one release, and the next eviction is still seen")
    func quickSwitchesShareOneRelease() {
        var ledger = attached()
        #expect(ledger.retarget(to: "w1:p2", now: t0) == .release)
        #expect(ledger.retarget(to: "w1:p1", now: t0) == .awaitClose,
                "a second release for a switch that overtook the first")
        #expect(ledger.expectedCloses == 1)
        #expect(ledger.releasing == "w1:p1")
        // herdr's one close for it frees the channel for the newest switch.
        #expect(ledger.closed(now: t0 + .milliseconds(40)) == .ownRelease)
        #expect(ledger.releasing == nil)
        #expect(ledger.attachLineWritten() == "w1:p1")
        #expect(ledger.target == "w1:p1")
        #expect(ledger.expectedCloses == 0)
        // Another client's takeover: not ours.
        #expect(ledger.closed(now: t0 + .seconds(1)) == .evicted(pane: "w1:p1"))
        #expect(ledger.target == nil)
        #expect(ledger.attached == nil)
        // Asked for again, nothing to release: the line goes at once.
        #expect(ledger.retarget(to: "w1:p1", now: t0 + .seconds(2)) == .attach)
    }

    @Test("Each switch past a close writes its own release")
    func switchesAfterTheCloseReleaseAgain() {
        var ledger = attached()
        #expect(ledger.retarget(to: "w1:p2", now: t0) == .release)
        #expect(ledger.closed(now: t0) == .ownRelease)
        #expect(ledger.attachLineWritten() == "w1:p2")
        #expect(ledger.retarget(to: "w1:p1", now: t0) == .release)
        #expect(ledger.releasing == "w1:p2")
        #expect(ledger.closed(now: t0) == .ownRelease)
        #expect(ledger.attachLineWritten() == "w1:p1")
    }

    /// The release's own wait gives up after 1.5 s and the switch goes on; a
    /// close that comes after that is still ours for a while, and then not.
    @Test("A close owed but never come is forgotten, so an eviction after it is seen")
    func anOwedCloseIsForgotten() {
        var late = attached()
        #expect(late.retarget(to: "w1:p2", now: t0) == .release)
        late.releaseAckTimedOut()
        #expect(late.attachLineWritten() == "w1:p2")
        // Late, but still ours.
        #expect(late.closed(now: t0 + .seconds(3)) == .ownRelease)
        #expect(late.attached == "w1:p2")

        var never = attached()
        #expect(never.retarget(to: "w1:p2", now: t0) == .release)
        never.releaseAckTimedOut()
        #expect(never.attachLineWritten() == "w1:p2")
        // Past its life: whatever closes now is the attach that reads.
        #expect(never.closed(now: t0 + HerdrAttachLedger.expectedCloseLife + .milliseconds(1))
                == .evicted(pane: "w1:p2"))
    }

    @Test("A close with nothing of ours attached changes nothing")
    func aStrayClose() {
        var ledger = HerdrAttachLedger()
        #expect(ledger.closed(now: t0) == .stray)
        #expect(ledger.retarget(to: "w1:p1", now: t0) == .attach)
        // The line isn't out yet (a prompt's settle): nothing reads.
        #expect(ledger.closed(now: t0) == .stray)
        #expect(ledger.target == "w1:p1")
        #expect(ledger.attachLineWritten() == "w1:p1")
    }

    // MARK: - B187

    /// The pane being left prints until its release's close comes back.
    /// Taken for the new pane's paint, a new attach whose line was then lost
    /// was never asked for again.
    @Test("Frames from the attach being released are its own, not the new pane's paint")
    func framesFromTheReleasedAttachAreItsOwn() {
        var ledger = attached()
        #expect(ledger.frame() == .target(first: true))
        #expect(ledger.frame() == .target(first: false))
        #expect(ledger.retarget(to: "w1:p2", now: t0) == .release)
        #expect(ledger.frame() == .releasing)
        #expect(!ledger.painted)
        #expect(ledger.closed(now: t0) == .ownRelease)
        #expect(ledger.attachLineWritten() == "w1:p2")
        // The line was lost: nothing paints. Asked again past the timeout.
        #expect(!ledger.wantsAttach("w1:p2", now: t0 + .seconds(1)))
        #expect(ledger.wantsAttach("w1:p2", now: t0 + HerdrAttachLedger.attachTimeout + .milliseconds(1)))
        // Re-asked, it paints: no more asking.
        #expect(ledger.retarget(to: "w1:p2", now: t0 + .seconds(4)) == .release)
        #expect(ledger.closed(now: t0 + .seconds(4)) == .ownRelease)
        #expect(ledger.attachLineWritten() == "w1:p2")
        #expect(ledger.frame() == .target(first: true))
        #expect(!ledger.wantsAttach("w1:p2", now: t0 + .seconds(60)))
        #expect(ledger.wantsAttach("w1:p1", now: t0 + .seconds(60)))
    }

    @Test("A frame with no attach of ours asked for is nobody's paint")
    func aStrayFrame() {
        var ledger = HerdrAttachLedger()
        #expect(ledger.frame() == .stray)
        #expect(ledger.retarget(to: "w1:p1", now: t0) == .attach)
        // Asked for, its line not out yet.
        #expect(ledger.frame() == .stray)
        #expect(!ledger.painted)
    }

    // MARK: - B185

    /// herdr 0.9.5 drops input that reaches an attach before its first
    /// frame. The app wrote typing, and a notification's reply, right behind
    /// the attach line.
    @Test("Nothing goes to an attach before it is up; what was held for it follows its first frame, in order")
    func inputWaitsForTheAttach() {
        var ledger = attached()
        // The boot attach isn't up yet, and nobody picked it: dropped.
        #expect(ledger.inputPane == nil)
        #expect(ledger.route(.typed(Data("early".utf8)), to: "w1:p1") == .dropped)
        #expect(ledger.frame() == .target(first: true))
        #expect(ledger.inputPane == "w1:p1")
        #expect(ledger.route(.typed(Data("ls".utf8)), to: "w1:p1") == .send)

        // The person picks w1:p2: typing at it waits for its attach.
        #expect(ledger.hold(for: "w1:p2").isEmpty)
        #expect(ledger.retarget(to: "w1:p2", now: t0) == .release)
        #expect(ledger.inputPane == nil, "input went on to the pane being left")
        #expect(ledger.route(.typed(Data("a".utf8)), to: "w1:p2") == .held)
        #expect(ledger.closed(now: t0) == .ownRelease)
        #expect(ledger.attachLineWritten() == "w1:p2")
        // Its line is out, its attach not up: still held.
        #expect(ledger.route(.delivery(id: 1, Data("b".utf8)), to: "w1:p2") == .held)
        #expect(ledger.route(.typed(Data("c".utf8)), to: "w1:p1") == .dropped, "typed at the pane being left")
        #expect(ledger.frame() == .target(first: true))
        #expect(ledger.takeHeld(for: "w1:p2") == [.typed(Data("a".utf8)), .delivery(id: 1, Data("b".utf8))])
        #expect(ledger.heldPane == nil)
        #expect(ledger.route(.typed(Data("d".utf8)), to: "w1:p2") == .send)
    }

    @Test("A switch the person didn't make holds nothing")
    func aSwitchNobodyPickedHoldsNothing() {
        var ledger = attached()
        #expect(ledger.frame() == .target(first: true))
        // The desktop moved the focus; the poll follows it.
        #expect(ledger.retarget(to: "w1:p2", now: t0) == .release)
        #expect(ledger.route(.typed(Data("y\r".utf8)), to: "w1:p2") == .dropped)
        #expect(ledger.closed(now: t0) == .ownRelease)
        #expect(ledger.attachLineWritten() == "w1:p2")
        #expect(ledger.route(.typed(Data("y\r".utf8)), to: "w1:p2") == .dropped)
        #expect(ledger.frame() == .target(first: true))
        #expect(ledger.route(.typed(Data("z".utf8)), to: "w1:p2") == .send)
    }

    @Test("A hold for another pane ends the one before it, whose input comes back to be dropped")
    func aNewHoldEndsTheOld() {
        var ledger = attached()
        #expect(ledger.hold(for: "w1:p2").isEmpty)
        #expect(ledger.retarget(to: "w1:p2", now: t0) == .release)
        #expect(ledger.route(.delivery(id: 7, Data("y".utf8)), to: "w1:p2") == .held)
        #expect(ledger.hold(for: "w1:p2").isEmpty, "the same pane's hold goes on")
        #expect(ledger.hold(for: "w1:p3") == [.delivery(id: 7, Data("y".utf8))])
        #expect(ledger.heldPane == "w1:p3")
        #expect(ledger.dropHeld().isEmpty)
        #expect(ledger.heldPane == nil)
    }

    @Test("A refused attach names nothing after it")
    func aRefusalEndsTheSwitch() {
        var ledger = attached()
        #expect(ledger.retarget(to: "w1:p2", now: t0) == .release)
        ledger.refused()
        #expect(ledger.target == nil)
        #expect(ledger.closed(now: t0) == .ownRelease)
        #expect(ledger.attachLineWritten() == nil)
    }

    // MARK: - Round twenty-eight: only the person takes over

    @Test("Only the person's word asks for --takeover; a re-ask keeps it, and a pick of the pane on its way upgrades it")
    func takeoverOnlyOnThePersonsWord() {
        // The first attach follows the server's focus: plain.
        var follow = HerdrAttachLedger()
        #expect(follow.retarget(to: "w1:p1", now: t0) == .attach)
        #expect(follow.attachLineWritten() == "w1:p1")
        #expect(!follow.lineTookOver)

        // The person's pick: `--takeover`; asked again by the poll after its
        // attach never painted, still theirs.
        var pick = attached()
        #expect(pick.retarget(to: "w1:p2", takeover: true, now: t0) == .release)
        #expect(pick.closed(now: t0 + .milliseconds(40)) == .ownRelease)
        #expect(pick.attachLineWritten() == "w1:p2")
        #expect(pick.lineTookOver)
        #expect(pick.wantsAttach("w1:p2", now: t0 + .seconds(4)))
        _ = pick.retarget(to: "w1:p2", now: t0 + .seconds(4))
        #expect(pick.takeover)

        // A follow on its way, then the person picks that very pane: theirs.
        var upgraded = attached()
        _ = upgraded.frame()
        #expect(upgraded.retarget(to: "w1:p3", now: t0) == .release)
        #expect(!upgraded.takeover)
        upgraded.pick("w1:p3")
        #expect(upgraded.takeover)
        #expect(upgraded.closed(now: t0 + .milliseconds(40)) == .ownRelease)
        #expect(upgraded.attachLineWritten() == "w1:p3")
        #expect(upgraded.lineTookOver)
        // Another client's takeover: an eviction, which forgets the wish.
        #expect(upgraded.closed(now: t0 + .seconds(9)) == .evicted(pane: "w1:p3"))
        #expect(!upgraded.takeover)
    }
}
