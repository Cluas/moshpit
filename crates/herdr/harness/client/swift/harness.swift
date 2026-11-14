// The harness's clock and log, after crates/tmux/harness/controller's. Never
// part of the app.
//
// HarnessClock replaces the client's clocks (build-swift.sh substitutes
// `Task.sleep(`, `ContinuousClock.now` and `Date()`): time stands still but
// for the script's steps, and moving it on fires the sleepers due in
// deadline order (ties in the order they started sleeping), each run to
// quiescence before the next — the Rust client's timer order, step for step.

import Foundation

final class HarnessClock: @unchecked Sendable {
    static let shared = HarnessClock()
    static let startMs: UInt64 = 1_000_000
    static let continuousBase = ContinuousClock.now

    private let lock = NSLock()
    private var nowMs: UInt64 = startMs
    private var seq: UInt64 = 0
    private var sleepers: [UInt64: (deadline: UInt64, seq: UInt64, continuation: CheckedContinuation<Void, Error>)] = [:]
    private var cancelled: Set<UInt64> = []
    private(set) var registrations: UInt64 = 0

    var now: UInt64 { lock.withLock { nowMs } }
    /// Milliseconds since the script started.
    var elapsed: UInt64 { now - Self.startMs }

    static func date() -> Date {
        Date(timeIntervalSince1970: 1_758_000_000 + Double(shared.elapsed) / 1000)
    }

    static var continuousNow: ContinuousClock.Instant {
        continuousBase.advanced(by: .milliseconds(Int64(shared.elapsed)))
    }

    @MainActor
    static func sleep(for duration: Duration) async throws {
        let (seconds, attoseconds) = duration.components
        let ms = (Double(seconds) * 1000 + Double(attoseconds) / 1e15).rounded()
        try await shared.sleep(ms: UInt64(max(0, min(ms, 1e15))))
    }

    @MainActor
    private func sleep(ms: UInt64) async throws {
        let id: UInt64 = lock.withLock { seq += 1; return seq }
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                let resumeNow: Bool = lock.withLock {
                    if cancelled.remove(id) != nil { return true }
                    sleepers[id] = (nowMs + ms, id, continuation)
                    registrations += 1
                    return false
                }
                if resumeNow { continuation.resume(throwing: CancellationError()) }
            }
        } onCancel: {
            let waiting: CheckedContinuation<Void, Error>? = lock.withLock {
                if let sleeper = sleepers.removeValue(forKey: id) { return sleeper.continuation }
                cancelled.insert(id)
                return nil
            }
            waiting?.resume(throwing: CancellationError())
        }
    }

    func takeDue(by target: UInt64) -> CheckedContinuation<Void, Error>? {
        lock.withLock {
            guard let next = sleepers.filter({ $0.value.deadline <= target })
                .min(by: { ($0.value.deadline, $0.value.seq) < ($1.value.deadline, $1.value.seq) })
            else { return nil }
            sleepers.removeValue(forKey: next.key)
            nowMs = max(nowMs, next.value.deadline)
            return next.value.continuation
        }
    }

    func standAt(_ target: UInt64) { lock.withLock { nowMs = max(nowMs, target) } }
}

@MainActor
enum HarnessLog {
    static var items: [String] = []
    static func item(_ text: String) { items.append("t=\(HarnessClock.shared.elapsed) \(text)") }
}
