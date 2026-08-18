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
        enum Scripted { case reply(String), hang, fail }

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
            }
        }
    }

    private static let twoWorkspaces = """
    {"id":"cli:api:snapshot","result":{"snapshot":{
      "workspaces":[{"workspace_id":"w1","label":"~","focused":true},
                    {"workspace_id":"w2","label":"api","focused":false}],
      "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":2}],
      "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":false},
               {"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","focused":true}],
      "layouts":[],
      "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p2"}}}
    """

    /// Let the client's async work drain. Its poll/send hop through `Task`, so
    /// tests need a yield point rather than a sleep.
    private func settle(_ times: Int = 12) async {
        for _ in 0..<times { await Task.yield() }
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
        await settle()
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
      "root_pane":{"pane_id":"w4:p1","cwd":"/Users/cluas/.herdr/worktrees/moshi/fix-scroll"}}}
    """

    private func taskRequest(prompt: String = "") -> AgentTaskRequest {
        AgentTaskRequest(repoPath: "/Users/cluas/code/moshi", branch: "fix-scroll",
                         agent: "claude", prompt: prompt)
    }

    @Test("A task creates the worktree, then types the agent into its own pane")
    func startAgentTaskChain() async {
        let runner = FakeRunner(response: Self.worktreeCreated)
        let client = await started(runner)
        let before = runner.commands.count

        let error = await client.startAgentTask(taskRequest())
        #expect(error == nil)

        let sent = Array(runner.commands[before...])
        #expect(sent.contains { $0.contains("worktree create --cwd '/Users/cluas/code/moshi'") })
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
        #expect(!Array(runner.commands[before...]).contains { $0.contains("agent send") })
    }

    @Test("Invalid input never reaches the host")
    func validationShortCircuits() async {
        let runner = FakeRunner(response: Self.worktreeCreated)
        let client = await started(runner)
        let before = runner.commands.count

        var bad = taskRequest(); bad.branch = "two words"
        let error = await client.startAgentTask(bad)
        #expect(error != nil)
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
        let error = await client.startAgentTask(taskRequest())
        #expect(error == "branch 'fix-scroll' already exists")
    }

    private static let panesInRepos = """
    {"id":"x","result":{"snapshot":{
      "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],
      "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":2}],
      "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","foreground_cwd":"/Users/cluas/code/moshi/Moshpit"},
               {"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","cwd":"/Users/cluas/code/other"}],
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
                return .reply("/Users/cluas/code/moshi\n/Users/cluas/code/other\n")
            }
            return .reply("/Users/cluas/code/scanned\n/Users/cluas/code/moshi\n")
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
        #expect(repos == ["/Users/cluas/code/moshi", "/Users/cluas/code/other",
                          "/Users/cluas/code/scanned"])
    }

    @Test("With no panes to go on, the form still gets the scan — one command")
    func gitReposWithoutPanes() async {
        let runner = FakeRunner(response: "{}")
        let client = await started(runner)
        let before = runner.commands.count
        runner.setResponse("/Users/cluas/code/moshi\n")
        let repos = await client.gitRepos()
        #expect(repos == ["/Users/cluas/code/moshi"])
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
            command.hasPrefix("for d in") ? .reply("/Users/cluas/code/moshi\n") : .hang
        }
        let clock = ContinuousClock()
        let startedAt = clock.now
        let repos = await client.gitRepos(scanBudget: .milliseconds(120))
        #expect(repos == ["/Users/cluas/code/moshi"])
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
        #expect(repos == ["/Users/cluas/code/moshi/Moshpit", "/Users/cluas/code/other"])
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
             "worktree":{"is_linked_worktree":true,"repo_name":"moshi",
                         "checkout_path":"/Users/x/.herdr/worktrees/moshi/fix-scroll"}}],
          "tabs":[],"panes":[],"layouts":[]}}}
        """)
        let client = await started(runner)
        #expect(client.worktreeRepo(for: "w4") == "moshi")
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
}
