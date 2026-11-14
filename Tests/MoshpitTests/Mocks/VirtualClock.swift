import Foundation

/// A clock that moves only when the test moves it. Every sleep on it waits
/// until ``advance(by:)`` carries ``now`` past its deadline, or until its
/// task is cancelled (then it throws, as `Task.sleep` does). Code that
/// takes an `any Clock<Duration>` — the mosh+tmux attach's windows and
/// waits (`SessionHub.ActiveSession.moshAttachClock`) — runs on it with no
/// wall time spent and every deadline landing exactly.
final class VirtualClock: Clock, @unchecked Sendable {
    struct Instant: InstantProtocol {
        var offset: Swift.Duration

        func advanced(by duration: Swift.Duration) -> Instant { Instant(offset: offset + duration) }
        func duration(to other: Instant) -> Swift.Duration { other.offset - offset }
        static func < (lhs: Instant, rhs: Instant) -> Bool { lhs.offset < rhs.offset }
    }

    private struct Sleeper {
        let deadline: Instant
        let continuation: CheckedContinuation<Void, Error>
    }

    private let lock = NSLock()
    private var current = Instant(offset: .zero)
    private var sleepers: [UUID: Sleeper] = [:]
    /// Sleeps cancelled before they were registered: they must not wait.
    private var cancelledEarly: Set<UUID> = []

    var now: Instant { lock.withLock { current } }
    var minimumResolution: Swift.Duration { .zero }

    /// How many sleeps are waiting on the clock.
    var sleeping: Int { lock.withLock { sleepers.count } }

    func sleep(until deadline: Instant, tolerance: Swift.Duration?) async throws {
        let id = UUID()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                enum Verdict { case wait, wake, cancel }
                let verdict: Verdict = lock.withLock {
                    if cancelledEarly.remove(id) != nil { return .cancel }
                    if deadline <= current { return .wake }
                    sleepers[id] = Sleeper(deadline: deadline, continuation: continuation)
                    return .wait
                }
                switch verdict {
                case .wait: break
                case .wake: continuation.resume()
                case .cancel: continuation.resume(throwing: CancellationError())
                }
            }
        } onCancel: {
            let sleeper: Sleeper? = lock.withLock {
                guard let sleeper = sleepers.removeValue(forKey: id) else {
                    cancelledEarly.insert(id)
                    return nil
                }
                return sleeper
            }
            sleeper?.continuation.resume(throwing: CancellationError())
        }
    }

    /// Move the clock forward and wake every sleep whose deadline it passed.
    func advance(by duration: Swift.Duration) {
        let due: [Sleeper] = lock.withLock {
            current = current.advanced(by: duration)
            let ready = sleepers.filter { $0.value.deadline <= current }
            for id in ready.keys { sleepers[id] = nil }
            return Array(ready.values)
        }
        for sleeper in due { sleeper.continuation.resume() }
    }
}
