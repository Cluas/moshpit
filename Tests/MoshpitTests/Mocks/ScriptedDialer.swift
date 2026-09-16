import Foundation
@testable import Moshpit

/// A dialer the test steers: swap `behavior` between dials, or hand out a
/// `DialGate` so a dial hangs until the test says how it ends. Stands in for
/// `SSHService.connect` through `TerminalViewModel.Dialer`.
final class DialScript: @unchecked Sendable {
    typealias Behavior = @Sendable (ServerConnection) async throws -> SSHSession

    private let lock = NSLock()
    private var _behavior: Behavior
    private var _dials = 0

    init(_ behavior: @escaping Behavior) { _behavior = behavior }

    var behavior: Behavior {
        get { lock.withLock { _behavior } }
        set { lock.withLock { _behavior = newValue } }
    }

    /// How many dials were placed, whatever became of them.
    var dials: Int { lock.withLock { _dials } }

    func dial(_ connection: ServerConnection) async throws -> SSHSession {
        let current = lock.withLock { _dials += 1; return _behavior }
        return try await current(connection)
    }

    /// The `TerminalViewModel.Dialer` shape.
    var dialer: TerminalViewModel.Dialer {
        { [self] connection, _, _ in try await self.dial(connection) }
    }
}

/// A dial that hangs until `finish` — the shape of a SYN nobody answers.
final class DialGate: @unchecked Sendable {
    typealias Verdict = Result<SSHSession, Error>

    private let lock = NSLock()
    private var waiters: [CheckedContinuation<Verdict, Never>] = []
    private var verdict: Verdict?

    func wait() async -> Verdict {
        await withCheckedContinuation { continuation in
            lock.withLock {
                if let verdict {
                    continuation.resume(returning: verdict)
                } else {
                    waiters.append(continuation)
                }
            }
        }
    }

    func finish(_ result: Verdict) {
        let released: [CheckedContinuation<Verdict, Never>] = lock.withLock {
            verdict = result
            defer { waiters = [] }
            return waiters
        }
        for waiter in released { waiter.resume(returning: result) }
    }
}

struct DialFailed: Error {}

/// A transport that answers every exec with nothing, opens a PTY that never
/// speaks, and counts how often the session over it was shut down.
final class QuietTransport: SSHClientTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var _shutdowns = 0
    var shutdowns: Int { lock.withLock { _shutdowns } }

    final class Writer: SSHPTYWriter, @unchecked Sendable {
        func write(_ data: Data) async throws {}
        func resize(cols: Int, rows: Int) async throws {}
    }

    func run(_ command: String) async throws -> Data { Data() }

    func openPTY(rows: Int, cols: Int,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable () -> Void) async throws -> any SSHPTYWriter {
        Writer()
    }

    func shutdown() async { lock.withLock { _shutdowns += 1 } }
}
