import Foundation

/// The one ordered path a transport's writes take.
///
/// Each step starts only after the step enqueued before it has FINISHED, and
/// the order is fixed when the step is enqueued — synchronously, on the main
/// actor — never left to how tasks happen to be scheduled. Two bare
/// `Task { await write(…) }`s have no ordering guarantee relative to each
/// other; that is the whole class of bug this exists to rule out (the
/// garbled-rendering saga was a per-chunk Task reordering bytes).
///
/// A step can be bytes on the wire (a keystroke, a resize, a protocol line)
/// or a remote command that has to land BETWEEN two of them — herdr's
/// semantic arrow keys, which travel as a `pane send-keys` exec while the
/// typing around them goes down the terminal channel. Sharing a lane is what
/// keeps the two in the order they were pressed: a key waits for the text
/// before it, the text after it waits for the key.
@MainActor
final class WriteLane {

    private var tail: Task<Void, Never>?

    /// The key run at the end of the lane that has not started yet — the only
    /// place a new key may join instead of queuing a step of its own.
    private var openRun: KeyRun?

    private final class KeyRun {
        let paneId: String
        var keys: [String]
        var started = false

        init(paneId: String, key: String) {
            self.paneId = paneId
            self.keys = [key]
        }
    }

    /// Append a step.
    func enqueue(_ step: @escaping @MainActor () async -> Void) {
        // Anything behind a key run closes it: a key typed after this step
        // must not be folded into a send that runs before it.
        openRun = nil
        append(step)
    }

    /// Append a key for `paneId`, sent by `send` when its turn comes.
    ///
    /// Keys that arrive while the run at the tail is still waiting its turn
    /// JOIN that run, so `send` gets them together: a held arrow repeats
    /// ~14 times a second, and each send is a remote round trip — one step
    /// per repeat would either fan out (unordered) or queue up and keep the
    /// cursor moving long after the key was released. With joining there is
    /// at most one send in flight and one gathering.
    ///
    /// The lane waits for `send` at most `budget`. A key that cannot be
    /// delivered — its channel stopped answering — must not freeze everything
    /// typed after it; past the budget the lane moves on and the key lands
    /// whenever its send does.
    func enqueueKey(_ key: String, paneId: String, holdingAtMost budget: Duration,
                    send: @escaping @MainActor (_ keys: [String], _ paneId: String) async -> Void) {
        if let run = openRun, run.paneId == paneId, !run.started {
            run.keys.append(key)
            return
        }
        let run = KeyRun(paneId: paneId, key: key)
        openRun = run
        append {
            run.started = true
            let keys = run.keys
            await WriteLane.wait(atMost: budget) { await send(keys, run.paneId) }
        }
    }

    /// Wait until every step enqueued so far has finished.
    func drain() async {
        await tail?.value
    }

    private func append(_ step: @escaping @MainActor () async -> Void) {
        let previous = tail
        tail = Task { @MainActor in
            await previous?.value
            await step()
        }
    }

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
