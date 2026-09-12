import Foundation
import Testing
@testable import Moshpit

/// The wait that decides whether a pairing is reported as proven.
///
/// `HostSetupModelTests` drives the screen through `FakePush`, which is the
/// right seam for the state machine but means the REAL implementation of the one
/// method that can hang had no coverage at all — and it did hang. A
/// `withTaskGroup` racing a sleeper against a `withCheckedContinuation`
/// deadlocks on the timeout path: the group awaits every child before returning
/// and `cancelAll()` cannot resume a continuation. So "the host never sent a
/// push that reached us" left the sheet spinning in `.proving` forever instead
/// of saying so.
///
/// Each test here is written to FAIL rather than hang if that returns: the wait
/// runs in its own task and the assertion is on a result box polled with a
/// ceiling. A regression must not take the whole suite down with it.
@MainActor
@Suite("Push self-test wait", .serialized)
struct PushServiceTests {

    /// Somewhere for a detached wait to leave its answer.
    @MainActor final class Outcome {
        var value: Bool?
    }

    /// Start the wait, then let it run for at most `within` — polling, so a
    /// working implementation finishes in milliseconds and a broken one still
    /// returns control to the test.
    private func wait(nonce: String, timeout: Duration,
                      within ceiling: Duration = .seconds(3),
                      then meanwhile: @escaping @MainActor () async -> Void = {}) async -> Bool? {
        let outcome = Outcome()
        Task { @MainActor in
            outcome.value = await PushService.shared.awaitSelfTest(nonce: nonce, timeout: timeout)
        }
        await meanwhile()
        let deadline = ContinuousClock.now.advanced(by: ceiling)
        while outcome.value == nil, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(20))
        }
        PushService.shared.forgetSelfTest(nonce: nonce)
        return outcome.value
    }

    @Test("a push that never arrives ends the wait, rather than hanging on it")
    func timeoutResolves() async {
        let answer = await wait(nonce: "selftest-timeout", timeout: .milliseconds(200))
        #expect(answer == false,
                "awaitSelfTest did not return after its timeout — provePush() would spin in .proving forever")
    }

    @Test("the push arriving is what proves the pairing")
    func arrivalResolves() async {
        let answer = await wait(nonce: "selftest-arrives", timeout: .seconds(30)) {
            // Long after the wait began, as a real host → relay → Apple → phone
            // round trip is.
            try? await Task.sleep(for: .milliseconds(60))
            PushService.shared.noteSelfTestArrived(nonce: "selftest-arrives")
        }
        #expect(answer == true)
    }

    @Test("a push that beats the wait still counts")
    func arrivalBeforeTheWait() async {
        // The race the arrivals set exists for: on a fast link the notification
        // is already in before the sheet starts waiting for it.
        PushService.shared.noteSelfTestArrived(nonce: "selftest-early")
        let answer = await wait(nonce: "selftest-early", timeout: .milliseconds(200))
        #expect(answer == true)
    }

    @Test("only the nonce that was asked for satisfies the wait")
    func aStaleTestProvesNothing() async {
        // The whole reason the nonce exists: the flow this replaced resolved on
        // any evidence it could find and called that success.
        PushService.shared.noteSelfTestArrived(nonce: "selftest-from-an-earlier-attempt")
        let answer = await wait(nonce: "selftest-this-attempt", timeout: .milliseconds(200))
        #expect(answer == false)
        PushService.shared.forgetSelfTest(nonce: "selftest-from-an-earlier-attempt")
    }
}

@MainActor
@Suite("Foreground re-announcement floor")
struct PushRefreshFloorTests {

    /// Re-announcing on every foreground is what makes a relay that lost its
    /// registry recover at all. The floor is what stops that from costing a post
    /// per glance at the app switcher.
    @Test("a recent success is not re-announced")
    func recentSuccessSkipped() {
        let now = Date()
        #expect(PushService.shouldSkipRefresh(
            syncPending: false, lastSuccess: now.addingTimeInterval(-5), now: now))
    }

    @Test("an older success is re-announced, so a relay that lost its registry recovers")
    func staleSuccessRefreshes() {
        let now = Date()
        #expect(!PushService.shouldSkipRefresh(
            syncPending: false, lastSuccess: now.addingTimeInterval(-120), now: now))
    }

    @Test("a pending failure ignores the floor")
    func pendingIsEager() {
        let now = Date()
        #expect(!PushService.shouldSkipRefresh(
            syncPending: true, lastSuccess: now.addingTimeInterval(-1), now: now))
    }

    @Test("never having succeeded always announces")
    func neverSucceeded() {
        #expect(!PushService.shouldSkipRefresh(
            syncPending: false, lastSuccess: nil, now: Date()))
    }
}
