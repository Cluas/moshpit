// The harness's clock, log and probes. Never part of the app.
//
// HarnessClock replaces the controller's clocks (generate.py substitutes
// `Task.sleep(`, `Date()` and `ContinuousClock.now`): time stands still but
// for `wait` steps, and a wait fires the sleepers due in deadline order (ties
// in the order they started sleeping), letting each run to quiescence before
// the next — the Rust controller's timer queue, step for step.

import Foundation

/// The script's clock, in milliseconds (the Rust rig's START_MS and
/// START_UNIX_MS).
final class HarnessClock: @unchecked Sendable {
    static let shared = HarnessClock()
    static let startMs: UInt64 = 1_000_000
    static let startUnixMs: Int64 = 1_758_000_000_000
    static let continuousBase = ContinuousClock.now

    private let lock = NSLock()
    private var nowMs: UInt64 = startMs
    private var seq: UInt64 = 0
    private var sleepers: [UInt64: (deadline: UInt64, seq: UInt64, continuation: CheckedContinuation<Void, Error>)] = [:]
    private var cancelled: Set<UInt64> = []
    /// Bumped whenever a sleeper registers: part of the quiescence check.
    private(set) var registrations: UInt64 = 0

    var now: UInt64 { lock.withLock { nowMs } }

    static func date() -> Date {
        let ms = Int64(shared.now) - Int64(startMs) + startUnixMs
        return Date(timeIntervalSince1970: Double(ms) / 1000)
    }

    static var continuousNow: ContinuousClock.Instant {
        continuousBase.advanced(by: .milliseconds(Int64(shared.now - startMs)))
    }

    // Main-actor isolated: a nonisolated async function would hop the
    // controller's main-actor timer tasks onto the concurrent pool, where
    // they would register in a racy order. On the main actor they register
    // in the order they started, which is the order they were made.
    @MainActor
    static func sleep(for duration: Duration) async throws {
        let (seconds, attoseconds) = duration.components
        let ms = (Double(seconds) * 1000 + Double(attoseconds) / 1e15).rounded()
        try await shared.sleep(ms: UInt64(max(0, ms)))
    }

    @MainActor
    static func sleep(nanoseconds: UInt64) async throws {
        try await shared.sleep(ms: UInt64((Double(nanoseconds) / 1e6).rounded()))
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

    /// The next sleeper due by `target`, taken off the list with the clock
    /// moved to its deadline.
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

/// Everything but commands, in the order it happened.
@MainActor
enum HarnessLog {
    static var items: [String] = []
    static func item(_ text: String) { items.append(text) }
}

/// Counts the events the controller's event pump has handled (generate.py
/// adds the `defer` to `handle(_:)`).
@MainActor
enum HarnessProbe {
    static var handled = 0
}

/// The view's cell height for pixel scrolls.
@MainActor
enum HarnessView {
    static var cellHeight: CGFloat = 16
}

/// The script's escape for bytes: printable ASCII as is (backslash doubled),
/// the rest `\xHH` (the Rust runner's `escape`).
func harnessEscape(_ data: Data) -> String {
    var out = ""
    for byte in data {
        switch byte {
        case 0x5C: out += "\\\\"
        case 0x20...0x7E: out.unicodeScalars.append(Unicode.Scalar(byte))
        default: out += String(format: "\\x%02x", byte)
        }
    }
    return out
}

func harnessEscape(_ text: String) -> String { harnessEscape(Data(text.utf8)) }
