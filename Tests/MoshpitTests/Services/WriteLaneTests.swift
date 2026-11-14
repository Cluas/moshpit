import Foundation
import Testing
@testable import Moshpit

/// The ordered path transport writes take — the contract every byte on the
/// terminal path relies on: steps finish in the order they were queued, no
/// matter how long each takes, and nothing queued can hold the rest forever.
@Suite("write lane")
@MainActor
struct WriteLaneTests {

    @MainActor
    final class Log {
        var entries: [String] = []
    }

    @Test("Steps finish in the order they were queued, whatever each costs")
    func stepsKeepTheirOrder() async {
        let log = Log()
        let lane = WriteLane()

        // Slowest first: a bare Task per step would finish b, c, a.
        lane.enqueue {
            try? await Task.sleep(for: .milliseconds(60))
            log.entries.append("a")
        }
        lane.enqueue { log.entries.append("b") }
        lane.enqueue {
            try? await Task.sleep(for: .milliseconds(10))
            log.entries.append("c")
        }
        await lane.drain()

        #expect(log.entries == ["a", "b", "c"])
    }

    @Test("A step starts only after the one before it has finished")
    func stepsNeverOverlap() async {
        let log = Log()
        let lane = WriteLane()

        for name in ["a", "b", "c"] {
            lane.enqueue {
                log.entries.append("\(name) start")
                try? await Task.sleep(for: .milliseconds(15))
                log.entries.append("\(name) end")
            }
        }
        await lane.drain()

        #expect(log.entries == ["a start", "a end", "b start", "b end", "c start", "c end"])
    }

    @Test("Keys queued before their run starts go out together; a step between runs splits them")
    func keyRunsJoinUntilStarted() async {
        let log = Log()
        let lane = WriteLane()
        let send: @MainActor ([String], String) async -> Void = { keys, pane in
            log.entries.append("\(pane): \(keys.joined(separator: " "))")
        }

        lane.enqueueKey("up", paneId: "p1", holdingAtMost: .seconds(5), send: send)
        lane.enqueueKey("up", paneId: "p1", holdingAtMost: .seconds(5), send: send)
        lane.enqueue { log.entries.append("text") }
        lane.enqueueKey("down", paneId: "p1", holdingAtMost: .seconds(5), send: send)
        await lane.drain()

        #expect(log.entries == ["p1: up up", "text", "p1: down"])
    }

    @Test("A key step that never finishes holds the lane only for its budget")
    func budgetReleasesTheLane() async {
        let log = Log()
        let lane = WriteLane()

        lane.enqueueKey("up", paneId: "p1", holdingAtMost: .milliseconds(50)) { _, _ in
            // Stands in for an exec on a half-open socket.
            try? await Task.sleep(for: .seconds(2))
        }
        lane.enqueue { log.entries.append("typed") }
        let started = ContinuousClock.now
        await lane.drain()

        #expect(log.entries == ["typed"])
        #expect(ContinuousClock.now - started < .seconds(1))
    }

    @Test("wait(atMost:) says whether the operation beat the budget")
    func waitReportsTheWinner() async {
        let quick = await WriteLane.wait(atMost: .seconds(5)) {}
        #expect(quick)

        let slow = await WriteLane.wait(atMost: .milliseconds(20)) {
            try? await Task.sleep(for: .milliseconds(300))
        }
        #expect(!slow)
    }
}
