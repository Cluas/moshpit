import Foundation
import Testing
@testable import Moshpit

/// Drives the whole control client against an in-memory runner: what commands
/// it sends, and what it does with what comes back.
@Suite("herdr control client")
@MainActor
struct HerdrControlClientTests {

    /// Records every command and replies with whatever the test queued.
    final class FakeRunner: HerdrCommandRunner, @unchecked Sendable {
        /// Per-command behaviour for tests that speak more than one command.
        /// `deafHang`: never returns AND never notices it was cancelled — an
        /// exec on a half-open connection.
        enum Scripted { case reply(String), hang, deafHang, fail }

        private let lock = NSLock()
        private var _commands: [String] = []
        private var _response: String
        private var _failNext = false
        private var _script: (@Sendable (String) -> Scripted)?

        init(response: String = "") { _response = response }

        var commands: [String] { lock.withLock { _commands } }
        func setResponse(_ value: String) { lock.withLock { _response = value } }
        func failNext() { lock.withLock { _failNext = true } }
        /// Route each command to its own behaviour — the single-response
        /// fields above cover the older single-command tests.
        func setScript(_ script: @escaping @Sendable (String) -> Scripted) {
            lock.withLock { _script = script }
        }

        func run(_ command: String) async throws -> String {
            let action: Scripted = lock.withLock {
                _commands.append(command)
                if let script = _script { return script(command) }
                if _failNext {
                    _failNext = false
                    return .fail
                }
                return .reply(_response)
            }
            switch action {
            case .reply(let text):
                return text
            case .fail:
                throw SSHError.sessionClosed
            case .hang:
                // "Never returns" as far as any sane budget is concerned;
                // honours cancellation so the test process doesn't linger.
                try await Task.sleep(for: .seconds(60))
                throw SSHError.sessionClosed
            case .deafHang:
                // Awaiting a detached task doesn't pass cancellation on.
                await Task.detached { try? await Task.sleep(for: .seconds(60)) }.value
                throw SSHError.sessionClosed
            }
        }
    }

    nonisolated private static let twoWorkspaces = """
    {"id":"cli:api:snapshot","result":{"snapshot":{
      "workspaces":[{"workspace_id":"w1","label":"~","focused":true},
                    {"workspace_id":"w2","label":"api","focused":false}],
      "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":2}],
      "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":false},
               {"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","focused":true}],
      "layouts":[],
      "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p2"}}}
    """

    /// Let the client's async work drain. Its poll/send hop through `Task`,
    /// so tests need a yield point rather than a fixed sleep — and a busy CI
    /// runner can take longer than a dozen bare yields to schedule that hop
    /// (the dead-socket test went red there once), so each yield also gives
    /// up the actor for a few milliseconds. A caller that can name the effect
    /// it waits for passes it and gets up to two seconds of patience.
    private func settle(_ times: Int = 12, until done: (@MainActor () -> Bool)? = nil) async {
        for _ in 0..<times {
            await Task.yield()
            if let done, done() { return }
            try? await Task.sleep(for: .milliseconds(5))
        }
        guard let done else { return }
        for _ in 0..<200 where !done() {
            try? await Task.sleep(for: .milliseconds(10))
        }
    }

    /// Poll until `done` holds, up to `timeout` — for effects that ride the
    /// client's own timers.
    private func waitUntil(_ timeout: Duration, _ done: @MainActor () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while !done() {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    /// A switch the test flips from another thread.
    final class Switch: @unchecked Sendable {
        private let lock = NSLock()
        private var _on: Bool
        init(_ on: Bool) { _on = on }
        var on: Bool {
            get { lock.withLock { _on } }
            set { lock.withLock { _on = newValue } }
        }
    }

    private func started(_ runner: FakeRunner, customPath: String? = nil) async -> HerdrControlClient {
        let client = HerdrControlClient(runner: runner, customPath: customPath)
        client.start()
        await settle()
        return client
    }

    // MARK: - Reading

    @Test("First poll populates the tree")
    func firstPollPopulates() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        #expect(client.snapshot.sessions.count == 2)
        #expect(client.snapshot.activePaneId == "w1:p2")
        #expect(client.snapshot.isAttached)
        #expect(runner.commands.first?.contains("api snapshot") == true)
    }

    @Test("A custom binary path is used verbatim for control commands too")
    func customPathUsed() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        _ = await started(runner, customPath: "/opt/herdr/bin/herdr")
        #expect(runner.commands.first?.hasPrefix("/opt/herdr/bin/herdr api snapshot") == true)
    }

    @Test("Without a custom path the PATH prefix rides along")
    func defaultPathPrefix() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        _ = await started(runner)
        #expect(runner.commands.first?.contains("$HOME/.local/bin") == true)
    }

    @Test("A failed read keeps the last good tree instead of blanking the UI")
    func failureKeepsLastTree() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        #expect(client.snapshot.sessions.count == 2)

        runner.failNext()
        client.refresh()
        await settle()
        #expect(client.snapshot.sessions.count == 2)   // unchanged
    }

    @Test("Garbage output also keeps the last good tree")
    func garbageKeepsLastTree() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)

        runner.setResponse("bash: herdr: command not found")
        client.refresh()
        await settle()
        #expect(client.snapshot.sessions.count == 2)
    }

    @Test("server_not_running empties the tree — that's real information")
    func serverNotRunningEmpties() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)

        runner.setResponse("""
        {"error":{"code":"server_not_running","message":"no herdr server is running"},"id":"cli:api:snapshot"}
        """)
        client.refresh()
        await settle()
        #expect(client.snapshot.sessions.isEmpty)
        #expect(client.serverNotRunning)
        // …but we remember that we HAD been attached, so the home card shows an
        // empty session list rather than a stuck "Attaching…" spinner.
        #expect(client.snapshot.everAttached)
    }

    /// The regression that shipped: herdr 0.7.3 prints a bare Rust error for a
    /// dead socket instead of the structured `server_not_running` JSON that
    /// 0.8.0 (which the design was read from) prints. Matching on the text
    /// missed it entirely — the tree stayed empty, `serverNotRunning` stayed
    /// false, and the terminal showed a black screen with no empty state and
    /// no error. The exit status is 1 in both versions, so that's what decides.
    /// The poll loop awaited each read, and one that never answered — a
    /// half-open connection, a wedged server — stopped it for good: no tree,
    /// no focus reports, so the frame channel's recovery never ran again.
    @Test("A snapshot read that never answers doesn't stop the reads")
    func hungReadDoesNotStopThePoll() async {
        let hanging = Switch(true)
        let runner = FakeRunner()
        runner.setScript { command in
            guard command.contains("api snapshot") else { return .reply("") }
            return hanging.on ? .hang : .reply(Self.twoWorkspaces)
        }
        let client = HerdrControlClient(runner: runner)
        client.pollReadBudget = .milliseconds(100)
        client.start()
        await settle(until: { !runner.commands.isEmpty })
        // The host answers again: the next read is made, and applied.
        hanging.on = false
        #expect(await waitUntil(.seconds(10)) { client.snapshot.sessions.count == 2 },
                "the poll loop stopped at the read that never answered")
        client.stop()
    }

    /// A protocol mismatch is named in versions before anyone is told, by a
    /// `herdr status` the poll awaited — on a host where that never answers
    /// (a stale client against a server it can't talk to, a wedged link),
    /// the poll loop stopped there, before the mismatch was even said.
    @Test("A version skew whose `herdr status` never answers doesn't hold the read")
    func hungStatusDoesNotHoldTheRead() async {
        let runner = FakeRunner()
        runner.setScript { command in
            command.contains("herdr status") ? .hang : .reply(Self.olderClientMismatch)
        }
        let client = HerdrControlClient(runner: runner, customPath: nil)
        client.statusBudget = .milliseconds(100)

        let finished = await WriteLane.wait(atMost: .seconds(5)) { await client.pollNowForTesting() }
        #expect(finished, "the read waited on a `herdr status` that never answers")
        #expect(client.protocolMismatch?.contains("older than server") == true, "the mismatch went unsaid")
        #expect(client.versionSkew == nil)
    }

    @Test("A dead socket is detected by exit status, not by error wording")
    func deadSocketDetectedByExitCode() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        #expect(!client.serverNotRunning)

        runner.setResponse("""
        Error: Os { code: 2, kind: NotFound, message: "No such file or directory" }
        \(HerdrControlClient.exitCodeMarker)1
        """)
        client.refresh()
        await settle(until: { client.serverNotRunning })
        #expect(client.serverNotRunning)
        #expect(client.snapshot.sessions.isEmpty)
    }

    @Test("A zero exit with junk output is 'no new information', not 'no server'")
    func junkWithSuccessKeepsTree() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)

        runner.setResponse("some unrelated chatter\n\(HerdrControlClient.exitCodeMarker)0")
        client.refresh()
        await settle()
        #expect(!client.serverNotRunning)
        #expect(client.snapshot.sessions.count == 2)   // last good tree kept
    }

    /// The second half of the black-screen report: herdr's direct attach is
    /// exclusive per terminal, so another client attaching with `--takeover`
    /// evicts ours — with the focused pane completely unchanged. Reporting the
    /// focus only on change meant the frame channel could never re-attach and
    /// the app stayed blank forever. Every poll must re-assert it.
    @Test("The focused pane is reported on every poll, not only when it changes")
    func focusReportedEveryPoll() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = HerdrControlClient(runner: runner)
        var reports: [String] = []
        client.onFocusedPaneChanged = { reports.append($0) }
        client.start()
        await settle()
        #expect(reports == ["w1:p2"])

        // Same tree, same focus — still re-asserted, which is what lets the
        // frame channel recover after being evicted.
        client.refresh()
        await settle()
        #expect(reports == ["w1:p2", "w1:p2"])
    }

    // MARK: - Poll cadence

    /// Each poll is a whole SSH exec channel. An idle session — an agent
    /// thinking, a shell at a prompt — is the common case, and paying 30 of
    /// them a minute for an unchanging answer is real battery and cellular
    /// data. herdr's CLI offers no general event subscription to replace it
    /// (`herdr wait` is per-pane and per-status), so the poll eases off
    /// instead.
    @Test("An unchanging tree eases the poll off the fast cadence")
    func idleBackoff() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        #expect(client.pollInterval == HerdrControlClient.fastPollInterval)

        // Drive identical reads until the cadence relaxes. `quicken()` polls
        // and resets, so use it once then let repeated reads accumulate.
        for _ in 0...HerdrControlClient.idlePollThreshold {
            await client.pollNowForTesting()
        }
        #expect(client.pollInterval == HerdrControlClient.idlePollInterval)
    }

    @Test("Any change snaps the cadence back immediately")
    func changeResetsCadence() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        for _ in 0..<6 { await client.pollNowForTesting() }
        #expect(client.pollInterval == HerdrControlClient.idlePollInterval)

        // Focus moved — that's activity.
        runner.setResponse(Self.twoWorkspaces.replacingOccurrences(
            of: #""focused_pane_id":"w1:p2""#, with: #""focused_pane_id":"w1:p1""#))
        await client.pollNowForTesting()
        #expect(client.pollInterval == HerdrControlClient.fastPollInterval)
    }

    @Test("quicken() drops back to fast — the frame channel's recovery rides on it")
    func quickenResetsCadence() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        for _ in 0..<6 { await client.pollNowForTesting() }
        #expect(client.pollInterval == HerdrControlClient.idlePollInterval)

        client.quicken()
        await settle()
        #expect(client.pollInterval == HerdrControlClient.fastPollInterval)
    }

    @Test("Losing the server also returns to the fast cadence")
    func serverLossResetsCadence() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        for _ in 0..<6 { await client.pollNowForTesting() }
        #expect(client.pollInterval == HerdrControlClient.idlePollInterval)

        runner.setResponse("Error: Os { code: 2 }\n\(HerdrControlClient.exitCodeMarker)1")
        await client.pollNowForTesting()
        #expect(client.serverNotRunning)
        #expect(client.pollInterval == HerdrControlClient.fastPollInterval)
    }

    @Test("Every command carries the exit-status trailer")
    func commandsCarryExitTrailer() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        _ = await started(runner)
        #expect(runner.commands.first?.contains(HerdrControlClient.exitCodeMarker) == true)
    }

    @Test("Exit-status parsing splits the trailer off the output")
    func parseExitTrailer() {
        let withCode = HerdrControlClient.parse("payload\n\(HerdrControlClient.exitCodeMarker)7\n")
        #expect(withCode.exitCode == 7)
        #expect(withCode.failed)
        #expect(withCode.output.trimmingCharacters(in: .whitespacesAndNewlines) == "payload")

        let ok = HerdrControlClient.parse("payload\n\(HerdrControlClient.exitCodeMarker)0")
        #expect(ok.exitCode == 0)
        #expect(!ok.failed)

        // No trailer (a shell that died before echoing) → unknown, not failed:
        // blanking the tree on a truncated read would be worse than keeping it.
        let missing = HerdrControlClient.parse("payload only")
        #expect(missing.exitCode == nil)
        #expect(!missing.failed)

        // B193: the trailer only as the last thing printed. An answer cut off
        // inside text that holds the marker (a label, a pane title) is no
        // status; the whole answer behind it is.
        let marker = HerdrControlClient.exitCodeMarker
        let cut = #"{"result":{"workspace":{"label":""# + marker + "0 build"
        #expect(HerdrControlClient.parse(cut).exitCode == nil)
        #expect(HerdrControlClient.parse(cut).output == cut)
        #expect(HerdrControlClient.parse(#"{"title":""# + marker + #"7"}"#).exitCode == nil)
        let whole = HerdrControlClient.parse(cut + "\"}}}\n" + marker + "3\r\n")
        #expect(whole.exitCode == 3)
        #expect(whole.output == cut + "\"}}}\n")
        #expect(HerdrControlClient.parse("x" + marker + "007z").exitCode == nil)
    }

    // MARK: - Commands

    /// Everything the sheets can trigger, and the herdr CLI call it becomes.
    @Test("Sheet actions map to the right herdr subcommands")
    func commandMapping() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)

        func lastCommand(_ action: () -> Void) async -> String {
            let before = runner.commands.count
            action()
            await settle()
            // [command, then the refresh poll that follows it]
            return runner.commands.count > before ? runner.commands[before] : ""
        }

        #expect(await lastCommand { client.selectSession("w2") }.contains("workspace focus 'w2'"))
        #expect(await lastCommand { client.selectWindow("w1:t1") }.contains("tab focus 'w1:t1'"))
        #expect(await lastCommand { client.killSession("w2") }.contains("workspace close 'w2'"))
        #expect(await lastCommand { client.killWindow("w1:t1") }.contains("tab close 'w1:t1'"))
        #expect(await lastCommand { client.killPane("w1:p1") }.contains("pane close 'w1:p1'"))
        #expect(await lastCommand { client.renameSession("w1", to: "api") }
            .contains("workspace rename 'w1' 'api'"))
        #expect(await lastCommand { client.renameWindow("w1:t1", to: "logs") }
            .contains("tab rename 'w1:t1' 'logs'"))
        #expect(await lastCommand { client.newSession(named: "build") }
            .contains("workspace create --focus --label 'build'"))
        #expect(await lastCommand { client.newWindow(named: nil) }
            .contains("tab create --focus"))
        // Splits the FOCUSED pane, taken from the snapshot — `--current` would
        // mean the pane our own SSH channel runs in, which is none of them.
        #expect(await lastCommand { client.newPane() }
            .contains("pane split 'w1:p2' --direction right --focus"))
    }

    @Test("Focusing a pane tries agent focus, with the zoom-bounce fallback in the same exec")
    func selectPaneUsesAgentFocus() async throws {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        let before = runner.commands.count

        client.selectPane("w1:p1")
        await settle()

        // `herdr pane focus` is directional-only in every released version.
        // `agent focus` is the fast path — but current herdr fails it
        // OUTRIGHT on a plain shell pane (the old errors-but-moves behavior
        // is gone), so the same exec carries the fallback: a zoom bounce,
        // which moves focus on --on and keeps it through --off.
        let sent = runner.commands[before]
        #expect(sent.contains("agent focus 'w1:p1' 2>/dev/null || {"))
        #expect(sent.contains("pane zoom --pane 'w1:p1' --on"))
        #expect(sent.contains("pane zoom --pane 'w1:p1' --off"))
        // --on before --off, or the bounce un-zooms panes that were zoomed.
        let on = try #require(sent.range(of: "--on"))
        let off = try #require(sent.range(of: "--off"))
        #expect(on.lowerBound < off.lowerBound)
    }

    // MARK: - Semantic keys

    @Test("Arrow keys ride pane send-keys with no trailing snapshot poll")
    func sendKeyIsQuiet() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        let before = runner.commands.count

        client.sendKey("up", paneId: "w1:p1")
        await settle()

        let sent = Array(runner.commands[before...])
        #expect(sent.contains { $0.contains("pane send-keys 'w1:p1' 'up'") })
        // Keystroke-rate traffic must not drag a snapshot read behind every
        // keypress — a cursor move changes nothing the sheets care about.
        #expect(!sent.contains { $0.contains("api snapshot") })
    }

    @Test("Only a lone arrow sequence maps to a key name — both DECCKM variants")
    func arrowKeyNameMapping() {
        typealias S = SessionHub.ActiveSession
        #expect(S.arrowKeyName(Data([0x1B, 0x5B, 0x41])) == "up")      // ESC[A
        #expect(S.arrowKeyName(Data([0x1B, 0x4F, 0x42])) == "down")    // ESC OB
        #expect(S.arrowKeyName(Data([0x1B, 0x5B, 0x43])) == "right")
        #expect(S.arrowKeyName(Data([0x1B, 0x4F, 0x44])) == "left")
        // Not arrows: plain text, batched sequences, other CSI.
        #expect(S.arrowKeyName(Data("A".utf8)) == nil)
        #expect(S.arrowKeyName(Data([0x1B, 0x5B, 0x41, 0x1B, 0x5B, 0x41])) == nil)
        #expect(S.arrowKeyName(Data([0x1B, 0x5B, 0x48])) == nil)       // Home
    }

    // MARK: - Immersive zoom (the mosh renderer)

    @Test("Immersive mode: picking a pane zooms it — focus and fill in one command")
    func immersiveSelectPaneZooms() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        client.immersiveZoom = true
        let before = runner.commands.count

        client.selectPane("w1:p1")
        await settle()

        // `pane zoom --on` is an idempotent SET that also moves focus
        // (verified live: zooming B while A is zoomed transfers both), so
        // immersive mode never needs `agent focus` and its error noise.
        let sent = Array(runner.commands[before...])
        #expect(sent.contains { $0.contains("pane zoom --pane 'w1:p1' --on 2>/dev/null || true") })
        #expect(!sent.contains { $0.contains("agent focus") })
    }

    @Test("Immersive mode: a tab switch zooms whatever pane the next snapshot names")
    func immersiveWindowSwitchZoomsFocusedPane() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        client.immersiveZoom = true
        let before = runner.commands.count

        client.selectWindow("w1:t1")
        await settle()
        await settle()   // zoom rides the poll AFTER the focus command

        let sent = Array(runner.commands[before...])
        #expect(sent.contains { $0.contains("tab focus 'w1:t1'") })
        // The fixture's focused pane is w1:p2 — the zoom follows the
        // snapshot's answer, not a guess made before the switch landed.
        #expect(sent.contains { $0.contains("pane zoom --pane 'w1:p2' --on") })
    }

    @Test("Without immersive mode, zooms only ever come as a bounce — never left on")
    func nonImmersiveNeverLeavesZoomOn() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        let before = runner.commands.count

        client.selectPane("w1:p1")
        client.selectWindow("w1:t1")
        await settle()
        await settle()

        let sent = Array(runner.commands[before...])
        // The immersive zoom-follow's shape (a lone --on, mosh renderer only)
        // must never appear on the SSH path — it restyles the desktop.
        #expect(!sent.contains { $0.contains("--on 2>/dev/null || true") })
        // The focus fallback's bounce is fine, but every --on must carry its
        // --off in the same exec.
        for command in sent where command.contains("--on") {
            #expect(command.contains("--off"), "a bounce without its --off leaves the desktop zoomed")
        }
    }

    @Test("Focusing a pane retargets the renderer without waiting for the round trip")
    func selectPaneRetargetsBeforeTheExec() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        // Wedge the focus command. If the retarget rode `agent focus` — or the
        // poll queued behind it — nothing below would ever fire, which is
        // exactly the delay this behaviour exists to remove.
        runner.setScript { $0.contains("agent focus") ? .hang : .reply(Self.twoWorkspaces) }

        final class Box: @unchecked Sendable { var seen: [String] = [] }
        let box = Box()
        client.onFocusedPaneChanged = { box.seen.append($0) }

        client.selectPane("w1:p1")

        // Synchronously, on the same turn as the tap.
        #expect(box.seen == ["w1:p1"])
        #expect(client.snapshot.activePaneId == "w1:p1")
    }

    @Test("Every mutation re-reads immediately instead of waiting for the tick")
    func mutationsRefresh() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        let before = runner.commands.count

        client.selectSession("w2")
        await settle()

        #expect(runner.commands.count >= before + 2)
        #expect(runner.commands.last?.contains("api snapshot") == true)
    }

    @Test("Labels from the server are shell-quoted, not interpolated raw")
    func labelsAreQuoted() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        let before = runner.commands.count

        client.renameSession("w1", to: "it's; rm -rf ~")
        await settle()

        let sent = runner.commands[before]
        let quotedLabel = #"'it'\''s; rm -rf ~'"#
        #expect(sent.contains("workspace rename 'w1' " + quotedLabel))
        // The `;` stays INSIDE the quoted argument: the only thing following
        // the label is our own exit-status trailer, so nothing the label
        // contains can start a second command.
        let afterLabel = (sent.components(separatedBy: quotedLabel).last ?? "")
            .trimmingCharacters(in: .whitespaces)
        #expect(afterLabel.hasPrefix("; echo \"\(HerdrControlClient.exitCodeMarker)"))
    }

    @Test("newPane with no known panes sends nothing rather than a bad target")
    func newPaneWithoutTarget() async {
        let runner = FakeRunner(response: "{}")
        let client = await started(runner)
        let before = runner.commands.count

        client.newPane()
        await settle()

        #expect(runner.commands.count == before)
    }

    // MARK: - Agent tasks

    private static let worktreeCreated = """
    {"id":"cli:worktree:create","result":{
      "type":"worktree_created",
      "workspace":{"workspace_id":"w4","label":"fix-scroll","focused":true},
      "root_pane":{"pane_id":"w4:p1","cwd":"/Users/dev/.herdr/worktrees/moshpit/fix-scroll"}}}
    """

    private func taskRequest(prompt: String = "") -> AgentTaskRequest {
        AgentTaskRequest(repoPath: "/Users/dev/code/moshpit", branch: "fix-scroll",
                         agent: "claude", prompt: prompt)
    }

    @Test("A task creates the worktree, then types the agent into its own pane")
    func startAgentTaskChain() async {
        let runner = FakeRunner(response: Self.worktreeCreated)
        let client = await started(runner)
        let before = runner.commands.count

        let outcome = await client.startAgentTask(taskRequest())
        #expect(outcome == .started)

        let sent = Array(runner.commands[before...])
        #expect(sent.contains { $0.contains("worktree create --cwd '/Users/dev/code/moshpit'") })
        #expect(sent.contains { $0.contains("--branch 'fix-scroll'") && $0.contains("--focus") })
        // The agent goes into the pane `worktree create` already put in the
        // checkout — `agent start` would land in the CLI's own directory.
        #expect(sent.contains { $0.contains("pane run 'w4:p1' 'claude'") })
        #expect(!sent.contains { $0.contains("agent start") })
    }

    @Test("An empty prompt sends nothing extra")
    func noPromptNoSend() async {
        let runner = FakeRunner(response: Self.worktreeCreated)
        let client = await started(runner)
        let before = runner.commands.count
        _ = await client.startAgentTask(taskRequest())
        let sent = Array(runner.commands[before...])
        #expect(!sent.contains { $0.contains("agent send") || $0.contains("agent prompt") })
        // Nor asks which herdr is there: only a first message needs to know.
        #expect(!sent.contains { $0.contains("herdr status") })
    }

    @Test("Invalid input never reaches the host")
    func validationShortCircuits() async {
        let runner = FakeRunner(response: Self.worktreeCreated)
        let client = await started(runner)
        let before = runner.commands.count

        var bad = taskRequest(); bad.branch = "two words"
        let outcome = await client.startAgentTask(bad)
        #expect(outcome == .failed(bad.validationError ?? ""))
        #expect(runner.commands.count == before)   // nothing sent
    }

    /// herdr returns a structured error for the things that actually go wrong
    /// (branch already exists, dirty repo). Showing its own words beats
    /// inventing ours.
    @Test("herdr's own error text is what the user sees")
    func surfacesHerdrError() async {
        let runner = FakeRunner(response: Self.worktreeCreated)
        let client = await started(runner)
        runner.setResponse("""
        {"error":{"code":"worktree_create_failed","message":"branch 'fix-scroll' already exists"},"id":"cli:worktree:create"}
        """)
        let outcome = await client.startAgentTask(taskRequest())
        #expect(outcome == .failed("branch 'fix-scroll' already exists"))
    }

    /// `worktree create` that exits 0 printing nothing to read, or whose exit
    /// status never made it back, named no pane: nothing was started in it.
    /// That used to come back `.started`, and the form closed as if the agent
    /// were at work with its first message.
    @Test("A worktree create with no answer to read is a failure, not a started task")
    func worktreeWithoutAnAnswerFails() async {
        let answers = [
            Self.exited(0, ""),                                   // exit 0, nothing printed
            Self.exited(0, "Created worktree fix-scroll"),        // exit 0, no JSON
            "",                                                   // the exit status lost too
        ]
        let ok = Self.exited(0, #"{"id":"cli:pane:run","result":{"type":"ok"}}"#)
        for answer in answers {
            let runner = FakeRunner(response: Self.twoWorkspaces)
            let snapshot = Self.twoWorkspaces
            runner.setScript { command in
                if command.contains("api snapshot") { return .reply(snapshot) }
                if command.contains("worktree create") { return .reply(answer) }
                return .reply(ok)
            }
            let client = quickClient(on: runner)

            let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

            guard case .failed(let message) = outcome else {
                Issue.record("no result from `worktree create` (\(answer.debugDescription)) was reported as \(outcome)")
                continue
            }
            #expect(message.contains("fix-scroll"), "the reason doesn't name the workspace to look for: \(message)")
            #expect(!runner.commands.contains { $0.contains("pane run") || $0.contains("agent prompt") },
                    "something was typed into a pane no answer named")
        }
    }

    // MARK: The first message

    /// `herdr status` as 0.7.3 through 0.9.5 print it, one version on both ends.
    nonisolated private static func status(_ version: String) -> String {
        """
        client:
          version: \(version)
          channel: stable

        server:
          status: running
          version: \(version)

        """
    }

    /// `output` and the exit-status trailer the client appends to every
    /// command — what a real shell sends back.
    private static func exited(_ code: Int, _ output: String) -> String {
        output + "\n" + HerdrControlClient.exitCodeMarker + "\(code)"
    }

    nonisolated private static let prompted = """
    {"id":"cli:agent:prompt","result":{"type":"agent_prompted",\
    "agent":{"terminal_id":"t4","agent":"claude","agent_status":"working"}}}
    """

    /// Tries at the first message, counted across the fake's threads.
    final class Tries: @unchecked Sendable {
        private let lock = NSLock()
        private var _count = 0
        var count: Int { lock.withLock { _count } }
        func next() -> Int { lock.withLock { _count += 1; return _count } }
    }

    /// A host running herdr `version`. It makes the worktree, starts the
    /// agent, and takes a first message through `verb` alone, answering
    /// try n with `answer(n)`; the other spelling is a usage error there,
    /// as it is on a real host.
    private func taskHost(version: String, takes verb: String,
                          answer: (@Sendable (Int) -> String)? = nil,
                          pressed: String? = nil) -> (FakeRunner, Tries) {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let tries = Tries()
        let snapshot = Self.twoWorkspaces
        let status = Self.exited(0, Self.status(version))
        let created = Self.exited(0, Self.worktreeCreated)
        let ran = Self.exited(0, #"{"id":"cli:pane:run","result":{"type":"ok"}}"#)
        let delivered = Self.exited(0, verb == "agent send"
            ? #"{"id":"cli:agent:send","result":{"type":"ok"}}"# : Self.prompted)
        let unknown = Self.exited(2, verb == "agent send"
            ? "usage: herdr agent <list|get|read|send|rename|focus|wait|attach|start|explain>"
            : "error: unrecognized subcommand 'send'")
        let other = verb == "agent send" ? "agent prompt" : "agent send"
        // herdr prints nothing for a key it pressed.
        let enter = pressed ?? Self.exited(0, "")
        runner.setScript { command in
            if command.contains("api snapshot") { return .reply(snapshot) }
            if command.contains("herdr status") { return .reply(status) }
            if command.contains("worktree create") { return .reply(created) }
            if command.contains("pane run") { return .reply(ran) }
            if command.contains("pane send-keys") { return .reply(enter) }
            if command.contains(verb + " ") {
                let n = tries.next()
                return .reply(answer?(n) ?? delivered)
            }
            if command.contains(other + " ") { return .reply(unknown) }
            return .reply("")
        }
        return (runner, tries)
    }

    /// No real waiting: the agent "comes up" as fast as the fake answers.
    private func quickClient(on runner: FakeRunner,
                             patience: Duration = .seconds(5)) -> HerdrControlClient {
        let client = HerdrControlClient(runner: runner)
        client.firstMessageSettle = .zero
        client.firstMessageRetryInterval = .milliseconds(10)
        client.firstMessagePatience = patience
        return client
    }

    /// herdr 0.7.5 replaced `agent send` with `agent prompt`. The task kept
    /// sending `agent send` and dropping the answer — a usage error on every
    /// current host — so the first message typed into the form never reached
    /// the agent, and the form closed as if it had.
    @Test("The first message goes the way the host's herdr takes it",
          arguments: zip(["0.7.3", "0.7.4", "0.7.5", "0.8.0", "0.9.5"],
                         ["agent send", "agent send", "agent prompt", "agent prompt", "agent prompt"]))
    func firstMessagePerServerVersion(version: String, verb: String) async {
        let (runner, tries) = taskHost(version: version, takes: verb)
        let client = quickClient(on: runner)

        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        #expect(outcome == .started)
        #expect(tries.count == 1, "herdr \(version) never got `\(verb)`: \(runner.commands.filter { $0.contains("agent ") })")
        #expect(runner.commands.contains { $0.contains("\(verb) 'w4:p1' 'fix the scroll'") })
        let other = verb == "agent send" ? "agent prompt" : "agent send"
        #expect(!runner.commands.contains { $0.contains(other) })
    }

    /// Before 0.7.5 `agent send` types the text and stops: the first message
    /// sat in the agent's input box, its turn never started, while the form
    /// closed as if it had been sent.
    @Test("On herdr before 0.7.5 the first message is typed, then Return is pressed",
          arguments: ["0.7.3", "0.7.4"])
    func oldHerdrSubmitsTheFirstMessage(version: String) async {
        let (runner, _) = taskHost(version: version, takes: "agent send")
        let client = quickClient(on: runner)

        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        #expect(outcome == .started)
        let typed = runner.commands.firstIndex { $0.contains("agent send 'w4:p1' 'fix the scroll'") }
        let pressed = runner.commands.firstIndex { $0.contains("pane send-keys 'w4:p1' 'Enter'") }
        #expect(typed != nil)
        #expect(pressed != nil, "herdr \(version) typed the first message and nothing pressed Return")
        if let typed, let pressed {
            #expect(typed < pressed, "Return went before the text it was meant to send")
        }
    }

    /// `agent prompt` presses Enter itself; a second one would land in the
    /// agent's composer once its turn is under way.
    @Test("On current herdr nothing presses Return after `agent prompt`",
          arguments: ["0.7.5", "0.9.5"])
    func currentHerdrSubmitsByItself(version: String) async {
        let (runner, _) = taskHost(version: version, takes: "agent prompt")
        let client = quickClient(on: runner)

        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        #expect(outcome == .started)
        #expect(!runner.commands.contains { $0.contains("pane send-keys") })
    }

    @Test("A first message typed but never sent says so, and how to send it")
    func returnThatFailsIsReported() async {
        let refused = Self.exited(1, #"{"id":"cli:request","error":{"code":"pane_not_found","#
            + #""message":"pane w4:p1 not found"}}"#)
        let (runner, _) = taskHost(version: "0.7.4", takes: "agent send", pressed: refused)
        let client = quickClient(on: runner)

        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        guard case .incomplete(let message) = outcome else {
            Issue.record("a first message left unsent was reported as \(outcome)")
            return
        }
        #expect(message.contains("pane w4:p1 not found"))
        #expect(runner.commands.contains { $0.contains("agent send 'w4:p1' 'fix the scroll'") })
    }

    /// An agent waiting at a question (trust this folder?, a permission
    /// prompt) is turned away with `agent_blocked`, and nothing is typed.
    @Test("A first message herdr turns away is said, in herdr's words")
    func refusedFirstMessageIsReported() async {
        let refusal = Self.exited(1, #"{"id":"cli:agent:prompt","error":{"code":"agent_blocked","#
            + #""message":"agent w4:p1 is blocked and requires interactive input"}}"#)
        let (runner, tries) = taskHost(version: "0.9.5", takes: "agent prompt", answer: { _ in refusal })
        let client = quickClient(on: runner)

        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        guard case .incomplete(let message) = outcome else {
            Issue.record("a refused first message was reported as \(outcome)")
            return
        }
        #expect(message.contains("agent w4:p1 is blocked and requires interactive input"))
        #expect(tries.count == 1, "asking again can't answer the agent's question")
    }

    /// `agent prompt` only takes a pane its agent has been detected in, and
    /// detection trails `pane run` — by seconds on a slow host.
    @Test("The first message waits for herdr to see the agent, then goes")
    func firstMessageWaitsForTheAgent() async {
        let notYet = Self.exited(1, #"{"id":"cli:agent:prompt","error":{"code":"agent_not_found","#
            + #""message":"agent target w4:p1 not found"}}"#)
        let delivered = Self.exited(0, Self.prompted)
        let (runner, tries) = taskHost(version: "0.9.5", takes: "agent prompt",
                                       answer: { $0 < 3 ? notYet : delivered })
        let client = quickClient(on: runner)

        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        #expect(outcome == .started)
        #expect(tries.count == 3)
    }

    @Test("An agent that never comes up is reported, not waited on for good")
    func agentThatNeverComesUp() async {
        let notReady = Self.exited(1, #"{"id":"cli:agent:prompt","error":{"code":"agent_not_ready","#
            + #""message":"agent w4:p1 is not an active named agent"}}"#)
        let (runner, tries) = taskHost(version: "0.9.5", takes: "agent prompt", answer: { _ in notReady })
        let client = quickClient(on: runner, patience: .milliseconds(150))

        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        guard case .incomplete = outcome else {
            Issue.record("an agent that never came up was reported as \(outcome)")
            return
        }
        #expect(tries.count > 1)
    }

    /// The agent's own launch can fail too — and with a first message, a
    /// failed launch would read as herdr not finding the agent.
    @Test("An agent that didn't start is said, and no first message is typed")
    func failedLaunchIsReported() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let snapshot = Self.twoWorkspaces
        let created = Self.exited(0, Self.worktreeCreated)
        let refused = Self.exited(1, #"{"id":"cli:pane:run","error":{"code":"pane_not_found","#
            + #""message":"pane w4:p1 not found"}}"#)
        runner.setScript { command in
            if command.contains("api snapshot") { return .reply(snapshot) }
            if command.contains("worktree create") { return .reply(created) }
            if command.contains("pane run") { return .reply(refused) }
            return .reply("")
        }
        let client = quickClient(on: runner)

        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        guard case .incomplete(let message) = outcome else {
            Issue.record("a failed launch was reported as \(outcome)")
            return
        }
        #expect(message.contains("pane w4:p1 not found"))
        #expect(!runner.commands.contains { $0.contains("agent prompt") || $0.contains("agent send") })
    }

    // MARK: An answer cut off before its exit status

    /// The exit status rides a trailer echoed after herdr exits. A channel
    /// that closes before it left no status at all, and that read as 0.
    @Test("A command whose exit status never came back is not taken as done")
    func lostExitStatusIsNotSuccess() {
        typealias Result = HerdrControlClient.CommandResult
        let cutOff = HerdrControlClient.refusal(Result(output: "", exitCode: nil))
        #expect(cutOff != nil, "nothing came back, and that counted as done")
        #expect(HerdrControlClient.refusal(Result(output: "Typing…", exitCode: nil)) != nil)
        // herdr's own result is evidence enough, trailer or not.
        #expect(HerdrControlClient.refusal(Result(output: #"{"id":"cli:pane:run","result":{"type":"ok"}}"#,
                                                  exitCode: nil)) == nil)
        // What was already right stays right.
        #expect(HerdrControlClient.refusal(Result(output: "", exitCode: 0)) == nil)
        #expect(HerdrControlClient.refusal(Result(output: "", exitCode: 2))
                == String(localized: "herdr exited with status \(2)"))
        #expect(HerdrControlClient.refusal(Result(output: "usage: herdr pane", exitCode: 2)) == "usage: herdr pane")
        #expect(HerdrControlClient.refusal(Result(
            output: #"{"error":{"code":"pane_not_found","message":"pane w4:p1 not found"}}"#,
            exitCode: nil)) == "pane w4:p1 not found")
    }

    @Test("A pane run cut off before its exit status is not reported as an agent started")
    func paneRunCutOff() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let snapshot = Self.twoWorkspaces
        let created = Self.exited(0, Self.worktreeCreated)
        runner.setScript { command in
            if command.contains("api snapshot") { return .reply(snapshot) }
            if command.contains("worktree create") { return .reply(created) }
            if command.contains("pane run") { return .reply("") }   // the channel closed first
            return .reply("")
        }
        let client = quickClient(on: runner)

        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        guard case .incomplete(let message) = outcome else {
            Issue.record("a launch nobody confirmed was reported as \(outcome)")
            return
        }
        #expect(message.contains(String(localized: "herdr's answer was cut off")))
        #expect(!runner.commands.contains { $0.contains("agent prompt") || $0.contains("agent send") },
                "a first message was typed after a launch nobody confirmed")
    }

    @Test("A first message cut off before its exit status is not reported as sent")
    func firstMessageCutOff() async {
        let (runner, tries) = taskHost(version: "0.9.5", takes: "agent prompt", answer: { _ in "" })
        let client = quickClient(on: runner)

        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        guard case .incomplete(let message) = outcome else {
            Issue.record("a first message nobody confirmed was reported as \(outcome)")
            return
        }
        #expect(message.contains(String(localized: "herdr's answer was cut off")))
        #expect(tries.count == 1, "an unknown answer is not one to try again on")
    }

    // MARK: A host that stops answering

    /// On a half-open connection no answer ever comes back. The lane stops
    /// waiting for a command after its budget, but the task waited for the
    /// answer itself forever: the New Agent Task form sat on "Starting…",
    /// with Cancel disabled.
    @Test("A worktree create the host never answers fails in bounded time, naming the workspace to look for")
    func worktreeCreateThatNeverAnswers() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let snapshot = Self.twoWorkspaces
        let ran = Self.exited(0, #"{"id":"cli:pane:run","result":{"type":"ok"}}"#)
        runner.setScript { command in
            if command.contains("api snapshot") { return .reply(snapshot) }
            if command.contains("worktree create") { return .hang }
            return .reply(ran)
        }
        let client = quickClient(on: runner)
        client.worktreeCreateBudget = .milliseconds(200)

        let began = ContinuousClock.now
        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        #expect(ContinuousClock.now - began < .seconds(10), "the task waited on a host that never answered")
        guard case .failed(let message) = outcome else {
            Issue.record("a create nobody answered was reported as \(outcome)")
            return
        }
        #expect(message.contains("fix-scroll"), "the reason doesn't name the workspace to look for: \(message)")
        #expect(!runner.commands.contains { $0.contains("pane run") || $0.contains("agent prompt") })
    }

    @Test("A pane run the host never answers is reported, and no first message follows it")
    func paneRunThatNeverAnswers() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let snapshot = Self.twoWorkspaces
        let created = Self.exited(0, Self.worktreeCreated)
        runner.setScript { command in
            if command.contains("api snapshot") { return .reply(snapshot) }
            if command.contains("worktree create") { return .reply(created) }
            if command.contains("pane run") { return .hang }
            return .reply("")
        }
        let client = quickClient(on: runner)
        client.agentLaunchBudget = .milliseconds(200)

        let began = ContinuousClock.now
        let outcome = await client.startAgentTask(taskRequest(prompt: "fix the scroll"))

        #expect(ContinuousClock.now - began < .seconds(10), "the task waited on a host that never answered")
        guard case .incomplete(let message) = outcome else {
            Issue.record("a launch nobody answered was reported as \(outcome)")
            return
        }
        #expect(message.contains("claude"))
        #expect(!runner.commands.contains { $0.contains("agent prompt") || $0.contains("agent send") })
    }

    /// A host whose `worktree create` answers only once the test says so.
    final class GatedCreate: HerdrCommandRunner, @unchecked Sendable {
        private let snapshot: String
        private let created: String
        private let ran: String
        init(snapshot: String, created: String, ran: String) {
            self.snapshot = snapshot
            self.created = created
            self.ran = ran
        }

        private let lock = NSLock()
        private var _commands: [String] = []
        private var waiting: CheckedContinuation<Void, Never>?
        private var released = false
        var commands: [String] { lock.withLock { _commands } }
        var createStarted: Bool { lock.withLock { _commands.contains { $0.contains("worktree create") } } }

        func release() {
            let parked: CheckedContinuation<Void, Never>? = lock.withLock {
                released = true
                defer { waiting = nil }
                return waiting
            }
            parked?.resume()
        }

        func run(_ command: String) async throws -> String {
            lock.withLock { _commands.append(command) }
            if command.contains("api snapshot") { return snapshot }
            if command.contains("worktree create") {
                await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                    let now: Bool = lock.withLock {
                        if released { return true }
                        waiting = continuation
                        return false
                    }
                    if now { continuation.resume() }
                }
                return created
            }
            return ran
        }
    }

    /// Cancel works while the task starts now. The create already sent can't
    /// be taken back; the agent is not typed into its pane after it.
    @Test("A task cancelled while its worktree is made starts no agent in it")
    func cancelledTaskStartsNoAgent() async throws {
        let runner = GatedCreate(snapshot: Self.twoWorkspaces,
                                 created: Self.exited(0, Self.worktreeCreated),
                                 ran: Self.exited(0, #"{"id":"cli:pane:run","result":{"type":"ok"}}"#))
        let client = HerdrControlClient(runner: runner)
        client.firstMessageSettle = .zero

        let task = Task { await client.startAgentTask(taskRequest(prompt: "fix the scroll")) }
        let deadline = ContinuousClock.now + .seconds(5)
        while !runner.createStarted, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        try #require(runner.createStarted)
        task.cancel()
        runner.release()
        let outcome = await task.value

        #expect(outcome == .incomplete(String(localized: "Cancelled. The worktree is ready, but no agent was started in it.")))
        #expect(!runner.commands.contains { $0.contains("pane run") || $0.contains("agent prompt") },
                "an agent was typed into the worktree of a cancelled task")
    }

    private static let panesInRepos = """
    {"id":"x","result":{"snapshot":{
      "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],
      "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":2}],
      "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","foreground_cwd":"/Users/dev/code/moshpit/Moshpit"},
               {"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","cwd":"/Users/dev/code/other"}],
      "layouts":[],"focused_pane_id":"w1:p1"}}}
    """

    /// Two commands ON PURPOSE: the pane resolve is small and reliable, the
    /// `$HOME` walk is neither — fused into one exec (the first version), a
    /// hung walk took the pane repos down with it and the form offered
    /// nothing but "Other…". (Seen for real against the high-RTT host.)
    @Test("Repo discovery: panes' own repos first, host scan appended")
    func gitReposFromPanesAndScan() async {
        let runner = FakeRunner(response: Self.panesInRepos)
        let client = await started(runner)
        let before = runner.commands.count

        runner.setScript { command in
            if command.hasPrefix("for d in") {
                return .reply("/Users/dev/code/moshpit\n/Users/dev/code/other\n")
            }
            return .reply("/Users/dev/code/scanned\n/Users/dev/code/moshpit\n")
        }
        let repos = await client.gitRepos()

        #expect(runner.commands.count == before + 2)
        let resolve = runner.commands.first { $0.hasPrefix("for d in") } ?? ""
        #expect(resolve.contains("rev-parse --show-toplevel"))
        let scan = runner.commands.first { $0.contains("-name .git -print") } ?? ""
        // Follows symlinks — "~/code is a symlink onto the big disk" is a
        // normal dev-host layout and a plain find walks right past it.
        #expect(scan.hasPrefix("find -L"))
        // Hidden directories are pruned or the list fills with .vim/.oh-my-zsh;
        // media libraries are pruned or an iCloud-backed home stalls the walk.
        #expect(scan.contains("! -name .git -prune"))
        #expect(scan.contains("-name Library"))
        #expect(scan.contains("-name Pictures"))
        // Pane repos lead (that's "resume what I was doing"), scan fills in,
        // duplicates collapse.
        #expect(repos == ["/Users/dev/code/moshpit", "/Users/dev/code/other",
                          "/Users/dev/code/scanned"])
    }

    @Test("With no panes to go on, the form still gets the scan — one command")
    func gitReposWithoutPanes() async {
        let runner = FakeRunner(response: "{}")
        let client = await started(runner)
        let before = runner.commands.count
        runner.setResponse("/Users/dev/code/moshpit\n")
        let repos = await client.gitRepos()
        #expect(repos == ["/Users/dev/code/moshpit"])
        #expect(runner.commands.count == before + 1)
    }

    /// The regression this split exists for: the walk hangs (huge or
    /// iCloud-backed home, dying link) and the form must still get the pane
    /// repos — promptly, not after some transport timeout.
    @Test("A hung host scan misses its budget; pane repos arrive anyway")
    func scanBudgetDropsTheSlowHalf() async {
        let runner = FakeRunner(response: Self.panesInRepos)
        let client = await started(runner)
        runner.setScript { command in
            command.hasPrefix("for d in") ? .reply("/Users/dev/code/moshpit\n") : .hang
        }
        let clock = ContinuousClock()
        let startedAt = clock.now
        let repos = await client.gitRepos(scanBudget: .milliseconds(120))
        #expect(repos == ["/Users/dev/code/moshpit"])
        #expect(clock.now - startedAt < .seconds(5))
    }

    @Test("A dead link on the pane resolve falls back to the raw cwd guesses")
    func paneResolveFailureFallsBackToGuesses() async {
        let runner = FakeRunner(response: Self.panesInRepos)
        let client = await started(runner)
        runner.setScript { command in
            command.hasPrefix("for d in") ? .fail : .reply("")
        }
        let repos = await client.gitRepos()
        // Raw cwds, sorted — the server validates the path at worktree-create
        // time, and a tappable guess beats an empty menu on a bad link.
        #expect(repos == ["/Users/dev/code/moshpit/Moshpit", "/Users/dev/code/other"])
    }

    /// On a half-open connection the panes' resolve never answered, and the
    /// New agent task form's "Finding repositories…" never ended.
    @Test("A pane resolve that never answers falls back to the raw directories, in bounded time")
    func paneResolveThatNeverAnswersFallsBack() async {
        let runner = FakeRunner(response: Self.panesInRepos)
        let client = await started(runner)
        runner.setScript { command in
            command.hasPrefix("for d in") ? .deafHang : .reply("")
        }
        client.discoveryBudget = .milliseconds(100)

        var repos: [String]?
        let finished = await WriteLane.wait(atMost: .seconds(10)) { repos = await client.gitRepos() }
        #expect(finished, "the form waited on a resolve that never answers")
        #expect(repos == ["/Users/dev/code/moshpit/Moshpit", "/Users/dev/code/other"])
        client.stop()
    }

    /// The scan was raced against its budget inside a task group — which
    /// waits for every child, cancelled or not. An exec that never notices
    /// cancellation (a half-open connection) held the form anyway.
    @Test("A host scan that never answers, deaf to cancellation, still misses only its budget")
    func deafScanStillMissesItsBudget() async {
        let runner = FakeRunner(response: Self.panesInRepos)
        let client = await started(runner)
        runner.setScript { command in
            command.hasPrefix("for d in") ? .reply("/Users/dev/code/moshpit\n") : .deafHang
        }
        var repos: [String]?
        let finished = await WriteLane.wait(atMost: .seconds(10)) {
            repos = await client.gitRepos(scanBudget: .milliseconds(120))
        }
        #expect(finished, "the scan's budget held nothing up")
        #expect(repos == ["/Users/dev/code/moshpit"])
        client.stop()
    }

    @Test("An agent list the host never answers is an empty list, in bounded time")
    func agentListThatNeverAnswers() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        runner.setScript { command in
            command.contains("agent-manifests") ? .deafHang : .reply(Self.twoWorkspaces)
        }
        client.discoveryBudget = .milliseconds(100)

        var names: [String]?
        let finished = await WriteLane.wait(atMost: .seconds(10)) { names = await client.agentNames() }
        #expect(finished, "the agent list waited on a host that never answers")
        #expect(names == [])
        client.stop()
    }

    @Test("Agent names come from herdr's own manifest list")
    func agentNamesFromManifests() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        runner.setResponse("""
        [{"agent":"pi","active_version":"1"},{"agent":"claude","active_version":"2"},{"agent":"codex"}]
        """)
        let names = await client.agentNames()
        #expect(names == ["claude", "codex", "pi"])
    }

    // MARK: - Worktree removal

    @Test("Only a workspace herdr made from a repo counts as a worktree")
    func worktreeMarking() async {
        let runner = FakeRunner(response: """
        {"result":{"snapshot":{
          "workspaces":[
            {"workspace_id":"w1","label":"~","focused":true},
            {"workspace_id":"w4","label":"fix-scroll",
             "worktree":{"is_linked_worktree":true,"repo_name":"moshpit",
                         "checkout_path":"/Users/x/.herdr/worktrees/moshpit/fix-scroll"}}],
          "tabs":[],"panes":[],"layouts":[]}}}
        """)
        let client = await started(runner)
        #expect(client.worktreeRepo(for: "w4") == "moshpit")
        #expect(client.worktreeRepo(for: "w1") == nil)
    }

    /// The safety property this whole two-step flow exists for: the first
    /// attempt never forces, so a checkout with uncommitted work survives it.
    @Test("A dirty checkout comes back as needsForce, never silently forced")
    func dirtyWorktreeNeedsForce() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        runner.setResponse("""
        {"error":{"code":"dirty_worktree_requires_force","message":"fatal: contains modified or untracked files, use --force to delete it"},"id":"cli:worktree:remove"}
        """)
        let before = runner.commands.count
        let outcome = await client.removeWorktree(sessionId: "w4")
        #expect(outcome == .needsForce)
        #expect(runner.commands[before].contains("worktree remove --workspace 'w4'"))
        #expect(!runner.commands[before].contains("--force"))
    }

    @Test("Forcing is explicit and only happens when asked for")
    func forcedRemoval() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        runner.setResponse("""
        {"id":"cli:worktree:remove","result":{"type":"worktree_removed","forced":true,"workspace_id":"w4"}}
        """)
        let before = runner.commands.count
        let outcome = await client.removeWorktree(sessionId: "w4", force: true)
        #expect(outcome == .removed)
        #expect(runner.commands[before].contains("--force"))
    }

    @Test("Any other failure carries herdr's own message")
    func removalFailureMessage() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        runner.setResponse("""
        {"error":{"code":"not_found","message":"workspace w9 not found"},"id":"cli:worktree:remove"}
        """)
        #expect(await client.removeWorktree(sessionId: "w9") == .failed("workspace w9 not found"))
    }

    /// On a half-open connection the removal's answer never comes; the lane
    /// stopped waiting for it after its budget, but the removal itself waited
    /// forever and the sheet spun for good.
    @Test("A removal the host never answers fails in bounded time")
    func removalThatNeverAnswersFails() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        runner.setScript { $0.contains("worktree remove") ? .hang : .reply(Self.twoWorkspaces) }
        client.worktreeRemoveBudget = .milliseconds(200)

        var outcome: HerdrControlClient.WorktreeRemoval?
        let finished = await WriteLane.wait(atMost: .seconds(10)) {
            outcome = await client.removeWorktree(sessionId: "w4")
        }
        #expect(finished, "the removal waited on a host that never answers")
        #expect(outcome == .failed(String(localized: "The host stopped answering while the worktree was being removed.")))
        client.stop()
    }

    /// The exec channel ended before the exit-status trailer, nothing
    /// printed: no evidence the checkout went, and it read as removed.
    @Test("A removal cut off before its exit status, nothing printed, is not removed")
    func removalCutOffIsNotRemoved() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        runner.setScript { $0.contains("worktree remove") ? .reply("") : .reply(Self.twoWorkspaces) }
        #expect(await client.removeWorktree(sessionId: "w4") == .failed(String(localized: "herdr's answer was cut off")))
        // Evidence is herdr's result, or exit 0.
        let exitedZero = Self.exited(0, ""), exitedOne = Self.exited(1, "{}")
        runner.setScript { $0.contains("worktree remove") ? .reply(exitedZero) : .reply(Self.twoWorkspaces) }
        #expect(await client.removeWorktree(sessionId: "w4") == .removed)
        runner.setScript { $0.contains("worktree remove") ? .reply(exitedOne) : .reply(Self.twoWorkspaces) }
        #expect(await client.removeWorktree(sessionId: "w4") == .failed(String(localized: "Removing the worktree failed")))
        client.stop()
    }

    // MARK: - Lifecycle

    @Test("stop() ends the polling")
    func stopEndsPolling() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        client.stop()
        let after = runner.commands.count
        await settle(20)
        #expect(runner.commands.count == after)
    }

    @Test("start() twice doesn't double up the poller")
    func startIsIdempotent() async {
        let runner = FakeRunner(response: Self.twoWorkspaces)
        let client = await started(runner)
        let after = runner.commands.count
        client.start()
        await settle()
        // A second start must not add a second in-flight poll.
        #expect(runner.commands.count == after)
    }

    // MARK: - Version skew (2026-11-11: stale ~/.local/bin/herdr 0.8.0 on PATH, 0.9.5 server)

    /// `herdr status` from the 0.8.0 binary against a 0.9.5 server, as
    /// captured on the host — plus the exit trailer every `run` appends.
    nonisolated private static let statusOldClient = """
    client:
      version: 0.8.0
      channel: stable
      protocol: 19

    server:
      status: running
      version: 0.9.5
      protocol: 22
      compatible: no
      socket: /Users/u/.config/herdr/herdr.sock

    __moshpit_rc=0
    """

    /// The other direction: the host's binary was upgraded under a server
    /// still running the old one — the case a restart exists to fix.
    nonisolated private static let statusOldServer = """
    client:
      version: 0.9.5
      channel: stable
      protocol: 22

    server:
      status: running
      version: 0.8.0
      endpoint_compatible: no

    __moshpit_rc=0
    """

    nonisolated private static let olderClientMismatch =
        #"{"id":"cli:api:snapshot","error":{"code":"protocol_mismatch","message":"client protocol 19 is older than server protocol 22; upgrade the Herdr client before using this command"}}"#
        + "\n__moshpit_rc=0"

    @Test("Restart refuses — and never sends `server stop` — when the herdr on PATH is the older side")
    func restartRefusesDowngrade() async {
        let runner = FakeRunner()
        runner.setScript { command in
            command.contains("herdr status") ? .reply(Self.statusOldClient) : .reply(Self.twoWorkspaces)
        }
        let client = HerdrControlClient(runner: runner, customPath: nil)

        let outcome = await client.restartServer()

        #expect(outcome == .wouldDowngrade(HerdrVersionReport(client: "0.8.0", server: "0.9.5")))
        // The whole point: stopping the 0.9.5 server to start a 0.8.0 one
        // would exit every pane on the host for a downgrade.
        #expect(!runner.commands.contains { $0.contains("server stop") })
        #expect(!runner.commands.contains { $0.contains("nohup") })
        #expect(client.versionSkew?.clientIsOlder == true)
        #expect(!client.isRestartingServer)
    }

    @Test("Restart still runs when the SERVER is the older side")
    func restartUpgradesOldServer() async {
        let runner = FakeRunner()
        runner.setScript { command in
            command.contains("herdr status") ? .reply(Self.statusOldServer) : .reply(Self.twoWorkspaces)
        }
        let client = HerdrControlClient(runner: runner, customPath: nil)

        let outcome = await client.restartServer()

        #expect(runner.commands.contains { $0.contains("server stop") })
        #expect(outcome == .restarted)
    }

    @Test("An unreadable `herdr status` doesn't block the restart (old herdr without the subcommand)")
    func restartProceedsWithoutReport() async {
        let runner = FakeRunner()
        runner.setScript { command in
            command.contains("herdr status")
                ? .reply("error: unknown command 'status'\n__moshpit_rc=2")
                : .reply(Self.twoWorkspaces)
        }
        let client = HerdrControlClient(runner: runner, customPath: nil)

        _ = await client.restartServer()

        #expect(runner.commands.contains { $0.contains("server stop") })
        #expect(client.versionSkew == nil)
    }

    /// Every restart step waited for good on a link that stopped answering —
    /// `herdr status`, `server stop`, the daemon start, the reads, the first
    /// workspace — and the banner said "Restarting herdr server…" forever.
    /// Nor could it say restarted: no read of the server ever came back.
    /// B192: a workspace created from the empty state of a host whose herdr
    /// server isn't running starts the server, and its first workspace is
    /// the one asked for. The bootstrap made it with no name.
    @Test("Creating on a host with no server names the first workspace as asked")
    func bootstrapNamesTheFirstWorkspace() async {
        let runner = FakeRunner()
        let up = Switch(false)
        let made = Switch(false)
        let empty = """
        {"id":"cli:api:snapshot","result":{"snapshot":{"workspaces":[],"tabs":[],"panes":[],"layouts":[]}}}
        """
        runner.setScript { command in
            if command.contains("herdr server >/dev/null") { up.on = true; return .reply("") }
            if command.contains("workspace create") { made.on = true; return .reply("") }
            guard command.contains("api snapshot") else { return .reply("") }
            guard up.on else {
                return .reply("""
                {"error":{"code":"server_not_running","message":"no herdr server is running"},"id":"cli:api:snapshot"}
                \(HerdrControlClient.exitCodeMarker)1
                """)
            }
            return .reply(made.on ? Self.twoWorkspaces : empty)
        }
        let client = HerdrControlClient(runner: runner, customPath: nil)
        client.bootstrapInterval = .milliseconds(10)
        await client.pollNowForTesting()
        #expect(client.serverNotRunning)

        client.newSession(named: "work")
        #expect(await waitUntil(.seconds(10)) { made.on }, "no workspace was made")
        let creates = runner.commands.filter { $0.contains("workspace create") }
        #expect(creates.count == 1)
        #expect(creates.first?.contains("workspace create --focus --label 'work'") == true,
                "the first workspace lost its name: \(creates)")
    }

    @Test("A restart on a link that stopped answering ends, failed, in bounded time")
    func restartOnADeadLinkEnds() async {
        let runner = FakeRunner()
        runner.setScript { _ in .hang }
        let client = HerdrControlClient(runner: runner, customPath: nil)
        client.statusBudget = .milliseconds(50)
        client.pollReadBudget = .milliseconds(50)
        client.serverCommandBudget = .milliseconds(50)
        client.bootstrapCreateBudget = .milliseconds(50)
        client.restartPause = .milliseconds(10)
        client.bootstrapInterval = .milliseconds(10)

        var outcome: HerdrControlClient.RestartOutcome?
        let finished = await WriteLane.wait(atMost: .seconds(10)) { outcome = await client.restartServer() }
        #expect(finished, "the restart never ended")
        #expect(outcome == .failed, "a restart no read ever confirmed was reported as \(String(describing: outcome))")
        #expect(!client.isRestartingServer)
    }

    @Test("A restart whose reads stopped answering after the stop is not taken for restarted")
    func restartWithoutAReadAfterIsNotRestarted() async {
        let runner = FakeRunner()
        let stopped = Switch(false)
        runner.setScript { command in
            if command.contains("herdr status") { return .reply(Self.statusOldServer) }
            if command.contains("server stop") { stopped.on = true; return .reply("") }
            // The server answered before the stop; nothing does after it.
            return stopped.on ? .hang : .reply(Self.twoWorkspaces)
        }
        let client = HerdrControlClient(runner: runner, customPath: nil)
        await client.pollNowForTesting()
        #expect(client.snapshot.sessions.count == 2)
        client.pollReadBudget = .milliseconds(50)
        client.serverCommandBudget = .milliseconds(50)
        client.restartPause = .milliseconds(10)
        client.bootstrapInterval = .milliseconds(10)

        var outcome: HerdrControlClient.RestartOutcome?
        let finished = await WriteLane.wait(atMost: .seconds(10)) { outcome = await client.restartServer() }
        #expect(finished, "the restart never ended")
        #expect(outcome == .failed, "judged on the tree from before the restart")
    }

    @Test("A protocol mismatch asks `herdr status` once per episode and keeps the versions")
    func mismatchDiagnosesOnce() async {
        let runner = FakeRunner()
        runner.setScript { command in
            command.contains("herdr status") ? .reply(Self.statusOldClient) : .reply(Self.olderClientMismatch)
        }
        let client = HerdrControlClient(runner: runner, customPath: nil)

        await client.pollNowForTesting()
        #expect(client.protocolMismatch?.contains("older than server") == true)
        #expect(client.versionSkew == HerdrVersionReport(client: "0.8.0", server: "0.9.5"))

        await client.pollNowForTesting()
        #expect(runner.commands.filter { $0.contains("herdr status") }.count == 1)

        // Healed (the stale copy got replaced): the skew is forgotten with it.
        runner.setScript { _ in .reply(Self.twoWorkspaces) }
        await client.pollNowForTesting()
        #expect(client.protocolMismatch == nil)
        #expect(client.versionSkew == nil)
    }

    /// B408: the banner's Restart, on a server that already runs the host's
    /// herdr (the skew fixed outside the app since the banner went up), stops
    /// nothing: `server stop` would exit every pane's process. The versions
    /// are read right before the stop.
    @Test("Restart stops nothing when the versions now agree")
    func restartUnneededWhenTheVersionsAgree() async {
        let runner = FakeRunner()
        runner.setScript { command in
            command.contains("herdr status") ? .reply(Self.statusOldServer) : .reply(Self.olderClientMismatch)
        }
        let client = HerdrControlClient(runner: runner, customPath: nil)
        await client.pollNowForTesting()
        #expect(client.versionSkew == HerdrVersionReport(client: "0.9.5", server: "0.8.0"))
        // Upgraded on the host, outside the app.
        runner.setScript { command in
            command.contains("herdr status") ? .reply(Self.statusAgreed) : .reply(Self.twoWorkspaces)
        }
        let outcome = await client.restartServer()
        #expect(outcome == .unneeded)
        #expect(!runner.commands.contains { $0.contains("server stop") })
        #expect(!runner.commands.contains { $0.contains("nohup") })
        #expect(client.versionSkew == nil)
        #expect(!client.isRestartingServer)
    }

    /// B408: the first tree after a mismatch tells the hub the server
    /// answers again (over mosh, nothing else clears its banner); a tree
    /// after a tree doesn't.
    @Test("The server answering again after a mismatch is said once")
    func serverAnswersAgainAfterAMismatch() async {
        let runner = FakeRunner()
        runner.setScript { _ in .reply(Self.twoWorkspaces) }
        let client = HerdrControlClient(runner: runner, customPath: nil)
        var said = 0
        client.onServerAnswersAgain = { said += 1 }
        await client.pollNowForTesting()
        #expect(said == 0)
        runner.setScript { command in
            command.contains("herdr status") ? .reply(Self.statusOldServer) : .reply(Self.olderClientMismatch)
        }
        await client.pollNowForTesting()
        #expect(said == 0)
        runner.setScript { _ in .reply(Self.twoWorkspaces) }
        await client.pollNowForTesting()
        #expect(said == 1)
        await client.pollNowForTesting()
        #expect(said == 1)
    }

    /// Both ends on 0.9.5.
    nonisolated private static let statusAgreed = """
    client:
      version: 0.9.5
      channel: stable
      protocol: 22

    server:
      status: running
      version: 0.9.5

    __moshpit_rc=0
    """
}
