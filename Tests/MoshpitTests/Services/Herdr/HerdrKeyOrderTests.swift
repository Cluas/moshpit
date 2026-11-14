import Foundation
import Testing
@testable import Moshpit

/// Over herdr an arrow key is an exec on the control connection while the
/// typing around it rides the terminal channel. These drive the real client
/// and a shared ``WriteLane`` against a fake runner and a fake writer, and
/// read back the order things LANDED in — which is what the pane sees.
///
/// Every scenario here failed before the lane: each key was its own
/// fire-and-forget exec, so an exec slower than the writes around it (all of
/// them, on a real link) landed after text typed later, and two quick
/// arrows raced each other.
@Suite("herdr key order")
@MainActor
struct HerdrKeyOrderTests {

    /// Everything that reached the pane, in arrival order: `text <s>` from
    /// the fake writer, `keys <pane> <key…>` from the fake runner.
    @MainActor
    final class Landing {
        var entries: [String] = []
    }

    /// Holds whoever passes until opened — and lets everyone through after.
    actor Gate {
        private var isOpen = false
        private var waiting: [CheckedContinuation<Void, Never>] = []

        func pass() async {
            guard !isOpen else { return }
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                waiting.append(continuation)
            }
        }

        func open() {
            isOpen = true
            for continuation in waiting { continuation.resume() }
            waiting.removeAll()
        }
    }

    /// Answers `pane send-keys` the way herdr does: the keys are in the pane
    /// by the time the command returns, so that is when they "land". Counts
    /// how many execs are in flight at once.
    final class KeyRunner: HerdrCommandRunner, @unchecked Sendable {
        private let lock = NSLock()
        private var _commands: [String] = []
        private var _inFlight = 0
        private var _maxInFlight = 0
        private let landing: Landing
        private let gate: Gate?
        private let delay: @Sendable (String) -> Duration

        init(landing: Landing, gate: Gate? = nil,
             delay: @escaping @Sendable (String) -> Duration = { _ in .zero }) {
            self.landing = landing
            self.gate = gate
            self.delay = delay
        }

        var commands: [String] { lock.withLock { _commands } }
        var maxInFlight: Int { lock.withLock { _maxInFlight } }

        func run(_ command: String) async throws -> String {
            lock.withLock {
                _commands.append(command)
                _inFlight += 1
                _maxInFlight = max(_maxInFlight, _inFlight)
            }
            if let gate { await gate.pass() }
            let wait = delay(command)
            if wait > .zero { try? await Task.sleep(for: wait) }
            let landed = "keys " + Self.arguments(of: command)
            await MainActor.run { landing.entries.append(landed) }
            lock.withLock { _inFlight -= 1 }
            return ""
        }

        /// `<pane> <key…>` out of a `pane send-keys` command line, unquoted.
        static func arguments(of command: String) -> String {
            guard let start = command.range(of: "pane send-keys ") else { return command }
            let line = command[start.upperBound...].prefix { $0 != ";" }
            return line.split(separator: " ")
                .map { $0.replacingOccurrences(of: "'", with: "") }
                .joined(separator: " ")
        }
    }

    /// A text write on the lane — what the frame channel's `terminal.input`
    /// or a mosh send is to the hub. Takes a little while, like a real write.
    private func type(_ text: String, on lane: WriteLane, landing: Landing,
                      taking duration: Duration = .milliseconds(5)) {
        lane.enqueue {
            try? await Task.sleep(for: duration)
            landing.entries.append("text \(text)")
        }
    }

    /// Poll until `done` holds, up to two seconds.
    private func waitUntil(_ done: @MainActor () -> Bool) async {
        for _ in 0..<200 where !done() {
            try? await Task.sleep(for: .milliseconds(10))
        }
    }

    @Test("Typing and arrows land in the order they were pressed")
    func keysAndTextLandInPressOrder() async {
        let landing = Landing()
        // Every exec slower than every write — so a key that didn't wait its
        // turn would land after the text typed behind it.
        let runner = KeyRunner(landing: landing, delay: { _ in .milliseconds(40) })
        let client = HerdrControlClient(runner: runner)
        let lane = WriteLane()

        type("gi", on: lane, landing: landing, taking: .milliseconds(30))   // a prefix…
        client.sendKey("up", paneId: "w1:p1", on: lane)                     // …searched for
        type("x", on: lane, landing: landing)                               // typed right after ↑
        client.sendKey("down", paneId: "w1:p1", on: lane)
        type("y", on: lane, landing: landing)
        await lane.drain()

        #expect(landing.entries == [
            "text gi", "keys w1:p1 up", "text x", "keys w1:p1 down", "text y",
        ])
    }

    @Test("Quick ↑↓ can't overtake each other, even with no lane to share")
    func arrowsKeepTheirOrder() async {
        let landing = Landing()
        // ↑'s exec is the slow one, and ↓ is pressed while it is in flight.
        let runner = KeyRunner(landing: landing, delay: { command in
            KeyRunner.arguments(of: command).hasSuffix("up") ? .milliseconds(80) : .zero
        })
        let client = HerdrControlClient(runner: runner)

        client.sendKey("up", paneId: "w1:p1")
        await waitUntil { runner.commands.count == 1 }
        client.sendKey("down", paneId: "w1:p1")
        await waitUntil { landing.entries.count == 2 }

        #expect(landing.entries == ["keys w1:p1 up", "keys w1:p1 down"])
    }

    @Test("A held arrow never fans out: one exec in flight, repeats ride the next")
    func heldArrowRidesOneExecAtATime() async {
        let landing = Landing()
        let gate = Gate()
        let runner = KeyRunner(landing: landing, gate: gate)
        let client = HerdrControlClient(runner: runner)
        let lane = WriteLane()

        // Ten repeats before the lane gets to them: one run.
        for _ in 0..<10 { client.sendKey("up", paneId: "w1:p1", on: lane) }
        await waitUntil { runner.commands.count == 1 }
        // Five more while that exec is in flight: they gather for the next.
        for _ in 0..<5 { client.sendKey("up", paneId: "w1:p1", on: lane) }
        await gate.open()
        await lane.drain()

        #expect(runner.commands.count == 2)
        #expect(runner.maxInFlight == 1)
        let ups = { (count: Int) in Array(repeating: "up", count: count).joined(separator: " ") }
        #expect(landing.entries == ["keys w1:p1 \(ups(10))", "keys w1:p1 \(ups(5))"])
    }

    @Test("Keys for another pane start their own run")
    func otherPaneStartsItsOwnRun() async {
        let landing = Landing()
        let runner = KeyRunner(landing: landing)
        let client = HerdrControlClient(runner: runner)
        let lane = WriteLane()

        client.sendKey("up", paneId: "w1:p1", on: lane)
        client.sendKey("up", paneId: "w1:p2", on: lane)
        client.sendKey("down", paneId: "w1:p2", on: lane)
        await lane.drain()

        #expect(landing.entries == ["keys w1:p1 up", "keys w1:p2 up down"])
    }

    @Test("A key whose exec never answers can't freeze the typing behind it")
    func hungKeyReleasesTheLane() async {
        let landing = Landing()
        let gate = Gate()   // opened only at the end
        let runner = KeyRunner(landing: landing, gate: gate)
        let client = HerdrControlClient(runner: runner)
        client.keySendBudget = .milliseconds(100)
        let lane = WriteLane()

        client.sendKey("up", paneId: "w1:p1", on: lane)
        type("ls", on: lane, landing: landing)
        await lane.drain()

        #expect(landing.entries == ["text ls"])
        await gate.open()   // let the parked exec finish rather than outlive the test
    }

    @Test("The landing wait runs after the typing ahead of the key, before its exec")
    func landedRunsBeforeTheExec() async {
        let landing = Landing()
        let runner = KeyRunner(landing: landing)
        let client = HerdrControlClient(runner: runner)
        let lane = WriteLane()

        type("gi", on: lane, landing: landing)
        client.sendKey("up", paneId: "w1:p1", on: lane) {
            try? await Task.sleep(for: .milliseconds(30))
            landing.entries.append("acked")
        }
        type("x", on: lane, landing: landing)
        await lane.drain()

        #expect(landing.entries == ["text gi", "acked", "keys w1:p1 up", "text x"])
    }
}
