import Foundation
import Testing
@testable import Moshpit

/// herdr control commands land on the server in the order the user issued
/// them, and a read the server answered before the last of them never pulls
/// the app back.
///
/// Every command used to be its own Task — its own exec, racing the rest.
/// Tapping pane A then pane B, with A's exec the slower one (any real link),
/// landed B first and A second: the server stayed on A, and the poll after
/// that pulled the app back to A.
@Suite("herdr command order")
@MainActor
struct HerdrCommandOrderTests {

    /// A herdr server with one tab of two panes. `agent focus <pane>` moves
    /// the focus when its exec FINISHES — after `focusDelay(pane)` — which is
    /// what the user sees: whichever lands last wins. Snapshots report the
    /// focus as it is when the read reaches the server.
    final class FocusServer: HerdrCommandRunner, @unchecked Sendable {
        private let lock = NSLock()
        private var _focused: String
        private var _focusesLanded: [String] = []
        private var _inFlight = 0
        private var _maxInFlight = 0
        private let focusDelay: @Sendable (String) -> Duration
        /// Holds snapshot replies until released, the reply already decided —
        /// a read the server answered before a later command landed.
        private var _snapshotGate: CheckedContinuation<Void, Never>?
        private var _gateSnapshots = false

        init(focused: String, focusDelay: @escaping @Sendable (String) -> Duration = { _ in .zero }) {
            _focused = focused
            self.focusDelay = focusDelay
        }

        var focused: String { lock.withLock { _focused } }
        var focusesLanded: [String] { lock.withLock { _focusesLanded } }
        var maxFocusInFlight: Int { lock.withLock { _maxInFlight } }

        func holdNextSnapshot() { lock.withLock { _gateSnapshots = true } }
        var snapshotHeld: Bool { lock.withLock { _snapshotGate != nil } }
        func releaseSnapshot() {
            let gate: CheckedContinuation<Void, Never>? = lock.withLock {
                defer { _snapshotGate = nil }
                return _snapshotGate
            }
            gate?.resume()
        }

        func run(_ command: String) async throws -> String {
            if command.contains("api snapshot") {
                let (reply, gated) = lock.withLock { () -> (String, Bool) in
                    defer { _gateSnapshots = false }
                    return (Self.snapshot(focused: _focused), _gateSnapshots)
                }
                if gated {
                    await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                        lock.withLock { _snapshotGate = continuation }
                    }
                }
                return reply
            }
            guard let target = Self.focusTarget(command) else { return "" }
            lock.withLock {
                _inFlight += 1
                _maxInFlight = max(_maxInFlight, _inFlight)
            }
            let delay = focusDelay(target)
            if delay > .zero { try? await Task.sleep(for: delay) }
            lock.withLock {
                _focused = target
                _focusesLanded.append(target)
                _inFlight -= 1
            }
            return ""
        }

        /// The pane named by `agent focus '<pane>'`, or by a mosh-mode
        /// `pane zoom --pane '<pane>' --on`.
        static func focusTarget(_ command: String) -> String? {
            for marker in ["agent focus '", "pane zoom --pane '"] {
                guard let start = command.range(of: marker) else { continue }
                let rest = command[start.upperBound...]
                return String(rest.prefix { $0 != "'" })
            }
            return nil
        }

        static func snapshot(focused: String) -> String {
            """
            {"id":"cli:api:snapshot","result":{"snapshot":{
              "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],
              "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":2}],
              "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":\(focused == "w1:p1")},
                       {"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","focused":\(focused == "w1:p2")}],
              "layouts":[],
              "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"\(focused)"}}}
            """
        }
    }

    /// A herdr server that makes worktrees. `worktree create … --focus`
    /// lands after `createDelay` and focuses the new worktree's root pane;
    /// `agent focus <pane>` focuses that pane at once. Records what landed,
    /// in order, and how many commands were out at once.
    final class TaskServer: HerdrCommandRunner, @unchecked Sendable {
        private let lock = NSLock()
        private var _focused: String
        private var _landed: [String] = []
        private var _createStarted = false
        private var _inFlight = 0
        private var _maxInFlight = 0
        private let createDelay: Duration

        init(focused: String, createDelay: Duration) {
            _focused = focused
            self.createDelay = createDelay
        }

        var focused: String { lock.withLock { _focused } }
        var landed: [String] { lock.withLock { _landed } }
        var createStarted: Bool { lock.withLock { _createStarted } }
        var maxInFlight: Int { lock.withLock { _maxInFlight } }

        static let worktreeCreated = """
        {"id":"cli:worktree:create","result":{
          "type":"worktree_created",
          "workspace":{"workspace_id":"w4","label":"fix-scroll","focused":true},
          "root_pane":{"pane_id":"w4:p1","cwd":"/Users/dev/.herdr/worktrees/moshpit/fix-scroll"}}}
        """

        func run(_ command: String) async throws -> String {
            if command.contains("api snapshot") {
                return FocusServer.snapshot(focused: focused)
            }
            let isCreate = command.contains("worktree create")
            let focusTarget = FocusServer.focusTarget(command)
            let isRun = command.contains("pane run")
            // A real host prints the exit-status trailer the client appends;
            // since round eighteen (B90) an answer without it is not success.
            let rc = command.contains(HerdrControlClient.exitCodeMarker)
                ? "\n\(HerdrControlClient.exitCodeMarker)0" : ""
            guard isCreate || focusTarget != nil || isRun else { return rc }
            lock.withLock {
                _inFlight += 1
                _maxInFlight = max(_maxInFlight, _inFlight)
                if isCreate { _createStarted = true }
            }
            if isCreate { try? await Task.sleep(for: createDelay) }
            lock.withLock {
                if isCreate {
                    _focused = "w4:p1"
                    _landed.append("create")
                } else if let focusTarget {
                    _focused = focusTarget
                    _landed.append("focus \(focusTarget)")
                } else {
                    _landed.append("pane run")
                }
                _inFlight -= 1
            }
            return (isCreate ? Self.worktreeCreated : "") + rc
        }
    }

    @MainActor
    final class Reports {
        var panes: [String] = []
    }

    /// Poll until `done` holds, up to ten seconds — generous on purpose: the
    /// suite runs in parallel with everything else on a loaded device.
    private func waitUntil(_ done: @MainActor () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(10)
        while !done() {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    @Test("Focus A then B: the server ends on B, and so does the app")
    func lastFocusWins() async {
        // A's exec is the slow one; B is tapped while it is still out.
        let server = FocusServer(focused: "w1:p2", focusDelay: { $0 == "w1:p1" ? .milliseconds(150) : .zero })
        let client = HerdrControlClient(runner: server)
        await client.pollNowForTesting()
        #expect(client.snapshot.activePaneId == "w1:p2")

        client.selectPane("w1:p1")
        client.selectPane("w1:p2")
        #expect(await waitUntil { server.focusesLanded.count == 2 }, "both focus commands must land")
        await client.drainCommandsForTesting()

        #expect(server.focusesLanded == ["w1:p1", "w1:p2"], "in the order they were tapped")
        #expect(server.focused == "w1:p2", "the old client left the server on the pane tapped FIRST")
        #expect(server.maxFocusInFlight == 1, "one focus exec at a time")

        // What the next poll does with it.
        await client.pollNowForTesting()
        #expect(client.snapshot.activePaneId == "w1:p2", "the old client was pulled back to w1:p1 here")
    }

    @Test("Mosh immersive switches keep their order too")
    func immersiveZoomsKeepTheirOrder() async {
        let server = FocusServer(focused: "w1:p2", focusDelay: { $0 == "w1:p1" ? .milliseconds(150) : .zero })
        let client = HerdrControlClient(runner: server)
        client.immersiveZoom = true
        await client.pollNowForTesting()

        client.selectPane("w1:p1")
        client.selectPane("w1:p2")
        #expect(await waitUntil { server.focusesLanded.count == 2 })
        await client.drainCommandsForTesting()

        #expect(server.focusesLanded == ["w1:p1", "w1:p2"])
        #expect(server.focused == "w1:p2")
    }

    @Test("A read the server answered before a tap landed doesn't pull the app back")
    func staleReadIsDropped() async {
        let server = FocusServer(focused: "w1:p2")
        let client = HerdrControlClient(runner: server)
        await client.pollNowForTesting()
        let reports = Reports()
        client.onFocusedPaneChanged = { reports.panes.append($0) }

        // A periodic read goes out and the server answers "w1:p2" — the reply
        // is still on its way when the user taps w1:p1.
        server.holdNextSnapshot()
        let staleRead = Task { await client.pollNowForTesting() }
        #expect(await waitUntil { server.snapshotHeld })
        client.selectPane("w1:p1")
        #expect(await waitUntil { server.focused == "w1:p1" })
        server.releaseSnapshot()
        await staleRead.value

        #expect(client.snapshot.activePaneId == "w1:p1",
                "the old client applied the stale read and moved the app back to w1:p2")
        #expect(!reports.panes.contains("w1:p2"), "the renderer was never pointed back at the pane left behind")

        // The re-read that follows the command is the one that counts.
        await client.drainCommandsForTesting()
        #expect(await waitUntil { reports.panes.count >= 2 })
        #expect(client.snapshot.activePaneId == "w1:p1")
        #expect(reports.panes.allSatisfy { $0 == "w1:p1" })
    }

    /// `startAgentTask` ran its commands beside the lane. `worktree create
    /// --focus` moves the server's focus like a tap, so a pane picked while
    /// the worktree was being made was overridden when the create landed
    /// after it — the user ends up looking at a pane they just left.
    @Test("A new task's worktree takes its turn among the taps")
    func agentTaskRidesTheLane() async {
        let server = TaskServer(focused: "w1:p2", createDelay: .milliseconds(150))
        let client = HerdrControlClient(runner: server)
        await client.pollNowForTesting()
        let request = AgentTaskRequest(repoPath: "/Users/dev/code/moshpit", branch: "fix-scroll",
                                       agent: "claude", prompt: "")

        let task = Task { await client.startAgentTask(request) }
        #expect(await waitUntil { server.createStarted })
        client.selectPane("w1:p1")   // tapped while the worktree is still being made
        #expect(await task.value == .started)
        await client.drainCommandsForTesting()

        #expect(Array(server.landed.prefix(2)) == ["create", "focus w1:p1"],
                "the tap landed before the create it came after: \(server.landed)")
        #expect(server.focused == "w1:p1", "the worktree's --focus overrode the pane picked after it")
        #expect(server.maxInFlight == 1, "a task step ran beside a tap")
        #expect(server.landed.contains("pane run"), "the agent still starts in the worktree's pane")
    }

    /// A host with herdr installed and no server running. The daemon line
    /// starts one with no workspaces; `workspace create --focus` makes one
    /// after `createDelay` and focuses its pane. Records the creates in the
    /// order they landed, and how many were out at once.
    final class ColdServer: HerdrCommandRunner, @unchecked Sendable {
        private let lock = NSLock()
        private var _running = false
        private var _workspaces = 0
        private var _landed: [String] = []
        private var _createStarted = false
        private var _inFlight = 0
        private var _maxInFlight = 0
        private let createDelay: Duration

        init(createDelay: Duration) { self.createDelay = createDelay }

        var landed: [String] { lock.withLock { _landed } }
        var createStarted: Bool { lock.withLock { _createStarted } }
        var maxInFlight: Int { lock.withLock { _maxInFlight } }

        func run(_ command: String) async throws -> String {
            if command.contains("nohup") {
                lock.withLock { _running = true }
                return ""
            }
            if command.contains("api snapshot") {
                let (running, workspaces) = lock.withLock { (_running, _workspaces) }
                guard running else {
                    return #"{"error":{"code":"server_not_running","message":"no herdr server is running"}}"#
                        + "\n__moshpit_rc=1"
                }
                return workspaces == 0
                    ? #"{"id":"cli:api:snapshot","result":{"snapshot":{"workspaces":[],"tabs":[],"panes":[],"layouts":[]}}}"#
                    : FocusServer.snapshot(focused: "w1:p1")
            }
            guard command.contains("workspace create") else { return "" }
            let label = command.contains("--label") ? "create (tapped)" : "create"
            lock.withLock {
                _createStarted = true
                _inFlight += 1
                _maxInFlight = max(_maxInFlight, _inFlight)
            }
            try? await Task.sleep(for: label == "create" ? createDelay : .zero)
            lock.withLock {
                _workspaces += 1
                _landed.append(label)
                _inFlight -= 1
            }
            return ""
        }
    }

    /// The empty state's "create a session" on a host with no server starts
    /// one, then makes its first workspace with `workspace create --focus` —
    /// which ran beside the lane, so a command tapped meanwhile ran beside it
    /// too and landed first, and a read out across it was applied.
    @Test("The first workspace of a server we started takes its turn on the lane")
    func bootstrapCreateRidesTheLane() async {
        let server = ColdServer(createDelay: .milliseconds(300))
        let client = HerdrControlClient(runner: server)
        await client.pollNowForTesting()
        #expect(client.serverNotRunning)

        client.newSession(named: nil)   // starts the server, then makes its workspace
        #expect(await waitUntil { server.createStarted }, "the bootstrap never made a workspace")
        client.newSession(named: "second")   // tapped again while that one is being made
        #expect(await waitUntil { server.landed.count == 2 })
        await client.drainCommandsForTesting()

        #expect(server.landed == ["create", "create (tapped)"],
                "the tap landed before the bootstrap's create it came after: \(server.landed)")
        #expect(server.maxInFlight == 1, "the bootstrap's create ran beside the lane")
    }

    /// A herdr server that records every command it is sent, in order, and
    /// answers each as `reply` says: at once or after a delay, with the
    /// exit-status trailer a real host prints, or cut off before it.
    final class SequenceServer: HerdrCommandRunner, @unchecked Sendable {
        enum Reply: Sendable { case done, after(Duration), cutOff }
        private let lock = NSLock()
        private var _received: [String] = []
        private var _landed: [String] = []
        private let reply: @Sendable (String) -> Reply

        init(reply: @escaping @Sendable (String) -> Reply = { _ in .done }) { self.reply = reply }

        var received: [String] { lock.withLock { _received } }
        var landed: [String] { lock.withLock { _landed } }

        func run(_ command: String) async throws -> String {
            if command.contains("api snapshot") {
                return FocusServer.snapshot(focused: "w1:p1")
            }
            let name = Self.name(command)
            lock.withLock { _received.append(name) }
            let reply = reply(name)
            if case .after(let delay) = reply { try? await Task.sleep(for: delay) }
            lock.withLock { _landed.append(name) }
            let trailer = "\n\(HerdrControlClient.exitCodeMarker)0"
            switch reply {
            case .cutOff: return ""
            case _ where name == "worktree create": return TaskServer.worktreeCreated + trailer
            default: return trailer
            }
        }

        /// What a command does, without herdr's path, quoting or trailer.
        static func name(_ command: String) -> String {
            for verb in ["workspace focus", "tab create", "worktree create", "pane run"]
            where command.contains(verb) { return verb }
            if let pane = FocusServer.focusTarget(command) { return "focus \(pane)" }
            return command
        }
    }

    /// B191: the lane waits for a sequence at most `commandBudget`, then
    /// moves on to the commands queued behind it — but the sequence kept
    /// going, so its later steps landed after them: New tab in another
    /// workspace, a pane tapped while `workspace focus` was slow, and the
    /// `tab create --focus` landed last and took the focus from the pane.
    @Test("A sequence whose step outlasts the lane's hold sends nothing after it")
    func sequenceStopsOnceTheLaneLetsItGo() async {
        let server = SequenceServer { $0 == "workspace focus" ? .after(.milliseconds(600)) : .done }
        let client = HerdrControlClient(runner: server)
        client.commandBudget = .milliseconds(150)
        await client.pollNowForTesting()

        client.newWindow(inSession: "w9", named: nil)   // workspace focus 'w9', then tab create
        client.selectPane("w1:p2")                      // tapped while the focus is slow
        #expect(await waitUntil { server.landed.contains("workspace focus") })
        await client.drainCommandsForTesting()
        try? await Task.sleep(for: .milliseconds(300))

        #expect(server.received == ["workspace focus", "focus w1:p2"],
                "the sequence's tab create landed after the tap it came before: \(server.received)")
    }

    /// B121: a sequence's next step went on when the one before it had lost
    /// its exit-status trailer — no status and no herdr result, so not known
    /// done — and `tab create --focus` ran in whichever workspace was focused.
    @Test("A step whose answer was cut off stops the sequence")
    func cutOffStepStopsTheSequence() async {
        let server = SequenceServer { $0 == "workspace focus" ? .cutOff : .done }
        let client = HerdrControlClient(runner: server)
        await client.pollNowForTesting()

        client.newWindow(inSession: "w9", named: nil)
        #expect(await waitUntil { server.landed.contains("workspace focus") })
        await client.drainCommandsForTesting()

        #expect(server.received == ["workspace focus"],
                "the tab create went on after a focus that may not have happened: \(server.received)")
    }

    @Test("A sequence whose steps are done runs to its end")
    func healthySequenceRunsToItsEnd() async {
        let server = SequenceServer()
        let client = HerdrControlClient(runner: server)
        await client.pollNowForTesting()

        client.newWindow(inSession: "w9", named: nil)
        #expect(await waitUntil { server.landed.count == 2 })
        await client.drainCommandsForTesting()

        #expect(server.received == ["workspace focus", "tab create"])
    }

    /// B191: a command a caller queued and stopped waiting for still ran
    /// when its turn came — the New agent task form said the host stopped
    /// answering and no agent was started, then the worktree was made anyway,
    /// focus and all.
    @Test("A worktree create still queued when its form gives up is never sent")
    func abandonedCreateIsNeverSent() async {
        let server = SequenceServer { $0 == "focus w1:p1" ? .after(.seconds(30)) : .done }
        let client = HerdrControlClient(runner: server)
        client.commandBudget = .milliseconds(600)
        client.worktreeCreateBudget = .milliseconds(100)
        await client.pollNowForTesting()

        client.selectPane("w1:p1")   // stands in for an exec on a link that stopped answering
        let request = AgentTaskRequest(repoPath: "/Users/dev/code/moshpit", branch: "fix-scroll",
                                       agent: "claude", prompt: "")
        let outcome = await client.startAgentTask(request)
        guard case .failed = outcome else {
            Issue.record("the form was told \(outcome), not that the host stopped answering")
            return
        }
        await client.drainCommandsForTesting()   // the lane lets the hung focus go, then the create's turn
        try? await Task.sleep(for: .milliseconds(200))

        #expect(!server.received.contains("worktree create"),
                "the create was sent after the form said none was: \(server.received)")
    }

    @Test("A command whose exec never answers holds the next one only for its budget")
    func hungCommandReleasesTheLane() async {
        let server = FocusServer(focused: "w1:p2", focusDelay: { $0 == "w1:p1" ? .seconds(30) : .zero })
        let client = HerdrControlClient(runner: server)
        client.commandBudget = .milliseconds(100)

        client.selectPane("w1:p1")   // stands in for an exec on a half-open socket
        client.selectPane("w1:p2")

        #expect(await waitUntil { server.focusesLanded.contains("w1:p2") },
                "a wedged command must not freeze every tap after it")
    }
}
