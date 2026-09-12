import Foundation
import Testing
@testable import Moshpit

/// The liveness-probe primitive's ONE load-bearing contract: it returns at
/// the deadline no matter what the probed operation does. Every lifecycle
/// flow (keepalive, sidecar rebuild, reconnect, protocol switch) funnels
/// through guards that a single hung probe wedges forever — which is exactly
/// what happened when the old task-group implementation waited for a
/// cancellation-ignoring SSH exec on a half-open socket.
@Suite("withTimeoutValue contract")
struct TimeoutValueTests {

    @Test("An op that hangs AND ignores cancellation cannot hold the caller past the deadline")
    func hangingUncancellableOpReturnsNilOnTime() async {
        let started = Date()
        let result: Int? = await withTimeoutValue(0.2) {
            // The half-open-socket shape: never completes, never checks
            // Task.isCancelled — like an exec bridged off a NIO future.
            await withCheckedContinuation { (_: CheckedContinuation<Int, Never>) in }
        }
        let elapsed = Date().timeIntervalSince(started)
        #expect(result == nil)
        #expect(elapsed < 2.0, "the deadline arm must win unconditionally; \(elapsed)s means the caller waited on the straggler")
    }

    @Test("A fast op wins and delivers its value")
    func fastOpDeliversValue() async {
        let result: String? = await withTimeoutValue(2.0) { "alive" }
        #expect(result == "alive")
    }

    @Test("A throwing op reads as nil, not a crash")
    func throwingOpIsNil() async {
        struct Boom: Error {}
        let result: Int? = await withTimeoutValue(2.0) { throw Boom() }
        #expect(result == nil)
    }

    @Test("A straggler finishing after the deadline is dropped, not double-resumed")
    func lateCompletionIsHarmless() async throws {
        let result: Int? = await withTimeoutValue(0.05) {
            try? await Task.sleep(nanoseconds: 200_000_000)
            return 42
        }
        #expect(result == nil)
        // Give the straggler time to finish resuming into the once-guard —
        // a double resume would crash the process right here.
        try await Task.sleep(nanoseconds: 300_000_000)
    }
}
