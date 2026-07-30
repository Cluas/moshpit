import Foundation
@testable import Ringdown

/// In-memory implementation of ``TmuxTransport`` for ``TmuxSessionController``
/// unit tests.
///
/// Tests can:
///   - `pushBytes(_:)` — inject bytes that the controller's pump task will
///     read off `dataStream` (simulates tmux output).
///   - `recordedWrites` — observe every chunk the controller `write(_:)`s
///     toward the remote shell (commands the controller sent).
///   - `finish()` — close the data stream so the pump task's `for await`
///     exits cleanly.
///
/// The transport is an actor so test code on the main thread and the
/// controller's detached pump task can both touch it without races.
actor MockTmuxTransport: TmuxTransport {

    // MARK: - Outbound stream (transport → controller)

    nonisolated let dataStream: AsyncStream<Data>
    nonisolated private let continuation: AsyncStream<Data>.Continuation

    // MARK: - Inbound writes (controller → transport)

    /// Every payload the controller sent via `write(_:)`, captured in send
    /// order. Tests inspect this to assert which commands the controller
    /// dispatched.
    private(set) var recordedWrites: [Data] = []

    /// Optional injectable error so tests can force a write failure path.
    /// When set, the *next* `write(_:)` call throws this and clears the slot.
    private var nextWriteError: Error?

    // MARK: - Init

    init() {
        var captured: AsyncStream<Data>.Continuation!
        self.dataStream = AsyncStream<Data>(bufferingPolicy: .unbounded) { cont in
            captured = cont
        }
        self.continuation = captured
    }

    // MARK: - TmuxTransport

    func write(_ data: Data) async throws {
        if let error = nextWriteError {
            nextWriteError = nil
            throw error
        }
        recordedWrites.append(data)
    }

    // MARK: - Test affordances

    /// Inject a chunk of bytes as if tmux had emitted them. The controller's
    /// pump task picks this up via the `dataStream` async iterator.
    nonisolated func pushBytes(_ data: Data) {
        continuation.yield(data)
    }

    /// Inject text using UTF-8 encoding.
    nonisolated func pushText(_ string: String) {
        if let data = string.data(using: .utf8) {
            continuation.yield(data)
        }
    }

    /// End the inbound stream so the controller's pump task `for await` loop
    /// can exit cleanly during teardown.
    nonisolated func finish() {
        continuation.finish()
    }

    /// Read out everything the controller sent as a list of UTF-8 strings,
    /// one per recorded write. Convenient assertion target.
    func recordedCommands() -> [String] {
        recordedWrites.compactMap { String(data: $0, encoding: .utf8) }
    }

    /// Arrange for the next `write(_:)` call to throw the given error.
    func scheduleNextWriteError(_ error: Error) {
        nextWriteError = error
    }
}
