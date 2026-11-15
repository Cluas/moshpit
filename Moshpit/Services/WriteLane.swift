import Foundation

/// Waiting on an operation with a budget: the caller stops waiting when the
/// budget runs out, and the operation goes on. What remains of the ordered
/// write lane the Swift transports shared (their writes are the core's now,
/// E1–E5); dictation's finalizer waits through it.
@MainActor
enum WriteLane {

    /// Wait for `operation`, or for `budget` to run out — whichever comes
    /// first. The operation is not cancelled (a remote command already sent
    /// cannot be unsent); the caller just stops waiting for it. Returns
    /// whether it finished in time.
    @discardableResult
    static func wait(atMost budget: Duration,
                     for operation: @escaping @MainActor () async -> Void) async -> Bool {
        await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            let outcome = Outcome(continuation)
            let timer = Task { @MainActor in
                try? await Task.sleep(for: budget)
                outcome.settle(false)
            }
            Task { @MainActor in
                await operation()
                timer.cancel()
                outcome.settle(true)
            }
        }
    }

    /// Resumes its continuation exactly once — whichever of the operation
    /// and the timer gets there first.
    private final class Outcome {
        private var continuation: CheckedContinuation<Bool, Never>?

        init(_ continuation: CheckedContinuation<Bool, Never>) {
            self.continuation = continuation
        }

        func settle(_ finished: Bool) {
            continuation?.resume(returning: finished)
            continuation = nil
        }
    }
}
