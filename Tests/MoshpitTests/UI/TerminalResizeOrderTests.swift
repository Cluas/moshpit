import Foundation
import Testing
@testable import Moshpit

/// Terminal resizes reach the remote end in the order the view reported
/// them, so the size it ends at is the last one.
///
/// `TerminalViewModel.resize` spawned a Task per size. A rotation or the
/// keyboard animating in reports several sizes in a burst; when an earlier
/// one's window-change took longer than a later one's, the PTY ended at the
/// older size — wrapped wrong until the next resize happened to come.
@Suite("terminal resize order")
@MainActor
struct TerminalResizeOrderTests {

    /// A PTY whose window-change takes `delay(cols)` to land, recording the
    /// sizes in the order they landed and how many were out at once. Writes
    /// land in order too, so the two can be read as one sequence.
    final class SizingHost: SSHClientTransport, @unchecked Sendable {
        private let lock = NSLock()
        private var _events: [String] = []
        private var _inFlight = 0
        private var _maxInFlight = 0
        let delay: @Sendable (Int) -> Duration

        init(delay: @escaping @Sendable (Int) -> Duration = { _ in .zero }) { self.delay = delay }

        /// `"<cols>x<rows>"` per landed resize, `"text <s>"` per write.
        var events: [String] { lock.withLock { _events } }
        var sizes: [String] { events.filter { !$0.hasPrefix("text ") } }
        var maxInFlight: Int { lock.withLock { _maxInFlight } }

        final class Writer: SSHPTYWriter, @unchecked Sendable {
            private weak var host: SizingHost?
            init(host: SizingHost) { self.host = host }

            func write(_ data: Data) async throws {
                host?.record("text " + String(decoding: data, as: UTF8.self))
            }

            func resize(cols: Int, rows: Int) async throws {
                guard let host else { return }
                host.enter()
                let wait = host.delay(cols)
                if wait > .zero { try? await Task.sleep(for: wait) }
                host.record("\(cols)x\(rows)")
                host.leave()
            }
        }

        fileprivate func record(_ event: String) { lock.withLock { _events.append(event) } }
        fileprivate func enter() {
            lock.withLock {
                _inFlight += 1
                _maxInFlight = max(_maxInFlight, _inFlight)
            }
        }
        fileprivate func leave() { lock.withLock { _inFlight -= 1 } }

        func run(_ command: String) async throws -> Data { Data() }

        func openPTY(rows: Int, cols: Int,
                     onOutput: @escaping @Sendable (Data) -> Void,
                     onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
            Writer(host: self)
        }

        func shutdown() async {}
    }

    private func connectedViewModel(_ host: SizingHost) async -> TerminalViewModel {
        let connection = ServerConnection(
            name: "resize",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — never routable
            port: 22,
            username: "tester",
            authMethod: .password)
        let script = DialScript { connection in SSHSession(connection: connection, transport: host) }
        let viewModel = TerminalViewModel(connection: connection, dial: script.dialer)
        await viewModel.start(rows: 24, cols: 80)
        return viewModel
    }

    private func waitUntil(_ done: () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(10)
        while !done() {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    @Test("Two quick resizes leave the PTY at the second size")
    func lastResizeWins() async {
        // The first window-change is the slow one.
        let host = SizingHost(delay: { $0 == 100 ? .milliseconds(150) : .zero })
        let viewModel = await connectedViewModel(host)
        #expect(viewModel.status == .connected)

        viewModel.resize(rows: 40, cols: 100)   // the keyboard going away…
        viewModel.resize(rows: 30, cols: 60)    // …and coming straight back
        #expect(await waitUntil { host.sizes.count == 2 })
        try? await Task.sleep(for: .milliseconds(200))   // anything still out lands

        #expect(host.sizes == ["100x40", "60x30"], "the old view model left the PTY at 100x40")
        #expect(host.maxInFlight == 1, "one window-change at a time")
        await viewModel.disconnect()
    }

    @Test("A resize keeps its place among the writes around it")
    func resizeKeepsItsPlaceAmongWrites() async {
        let host = SizingHost(delay: { $0 == 100 ? .milliseconds(100) : .zero })
        let viewModel = await connectedViewModel(host)

        viewModel.send(Data("a".utf8))
        viewModel.resize(rows: 40, cols: 100)
        viewModel.send(Data("b".utf8))
        #expect(await waitUntil { host.events.count == 3 })

        #expect(host.events == ["text a", "100x40", "text b"])
        await viewModel.disconnect()
    }
}
