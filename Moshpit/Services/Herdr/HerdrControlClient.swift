import Foundation
import Observation

/// The narrow surface ``HerdrControlClient`` needs from a connection: run one
/// shell command on the remote host, hand back its output.
///
/// `SSHSession` conforms; tests plug in an in-memory fake and drive the whole
/// client without a network.
protocol HerdrCommandRunner: Sendable {
    /// Run `command` remotely and return its combined stdout/stderr.
    func run(_ command: String) async throws -> String
}

extension SSHSession: HerdrCommandRunner {
    func run(_ command: String) async throws -> String {
        let data = try await executeCommand(command)
        return String(decoding: data, as: UTF8.self)
    }
}

/// herdr's control plane: polls `herdr api snapshot` over an SSH side-channel
/// and turns the app's session/window/pane commands into `herdr` CLI calls.
///
/// ### Why polling
///
/// herdr's socket API does have an event subscription (`events.subscribe`),
/// but its CLI does not expose one — and a phone can't open the host's unix
/// socket directly. So unlike tmux's `-CC`, which pushes, this pulls on a
/// timer and refreshes immediately after every mutation so user-initiated
/// changes feel instant regardless of where the tick lands.
///
/// ### What it does NOT do
///
/// Rendering. In Phase 0/1 herdr draws its own full-screen TUI in the terminal
/// and Moshpit is only the renderer; this client exists so the native sheets
/// and home tree work against real structure. Per-pane native rendering is
/// Phase 2's frame channel (`herdr terminal session control`).
///
/// Restoring the last selection, either. tmux needs that because each client
/// carries its own view; herdr's focus lives on the server, so re-attaching
/// already lands where the user left off — replaying a stored selection would
/// just yank their desktop client around for no gain.
@MainActor
@Observable
final class HerdrControlClient: MultiplexerControlling {

    nonisolated var multiplexer: Multiplexer { .herdr }

    /// Cadence while anything is happening. Each poll is a whole SSH exec
    /// channel, so this is the expensive end — matched to the rate the tmux
    /// path already polls agent hooks at.
    static let fastPollInterval: Duration = .seconds(2)
    /// Cadence once the tree has been identical for a while. A session sitting
    /// idle is the common case (an agent thinking, a shell at its prompt), and
    /// paying 30 exec channels a minute for an unchanging answer is real
    /// battery and cellular data.
    ///
    /// The ceiling is deliberately low. herdr's CLI exposes no general event
    /// subscription — `herdr wait` is per-pane and per-status, and
    /// `events.subscribe` lives on the unix socket a phone can't reach — so
    /// this poll is also how agent state reaches the Vibe Island. 8s is the
    /// most staleness that felt honest for a "needs you" prompt.
    static let idlePollInterval: Duration = .seconds(8)
    /// Identical polls before easing off. Three keeps a quick back-and-forth
    /// (type, watch, type) at full speed.
    static let idlePollThreshold = 3

    private(set) var snapshot = TmuxSnapshot()
    private(set) var agentHooks: [String: AgentHook] = [:]
    private(set) var isRefreshing = false

    /// True once a poll came back with `server_not_running` — the host has
    /// herdr installed but nothing running. Distinct from "we haven't managed
    /// to read anything yet", which leaves this false.
    private(set) var serverNotRunning = false

    /// Reports the server's focused pane after EVERY successful poll, not only
    /// when it changes.
    ///
    /// The frame channel renders exactly one pane, and it can lose its attach
    /// without the focus moving at all — herdr's direct attach is exclusive
    /// per terminal, so any other client attaching with `--takeover` evicts
    /// ours. Firing only on change left the app permanently blank in exactly
    /// that case (seen live: a second Moshpit on the same host took over, the
    /// first went black and never came back, because "focused pane" never
    /// changed afterwards). Repeating the current pane every poll makes the
    /// channel self-healing; the receiver already ignores a target it is
    /// still rendering.
    @ObservationIgnored var onFocusedPaneChanged: ((String) -> Void)?

    /// `MultiplexerControlling.onAgentHooksUpdated` — fired after every poll
    /// that produced a tree, since agent state rides the same snapshot.
    @ObservationIgnored var onAgentHooksUpdated: (() -> Void)?

    /// Fired when the server refuses us over version skew (protocol
    /// mismatch) — the hub routes it to the terminal banner, because the fix
    /// (restart/upgrade the herdr server on the host) belongs to the user.
    @ObservationIgnored var onProtocolMismatch: ((String) -> Void)?

    /// The interval the next tick will wait. Readable so tests can assert the
    /// backoff without waiting on real time.
    private(set) var pollInterval: Duration = HerdrControlClient.fastPollInterval

    @ObservationIgnored private let runner: HerdrCommandRunner
    @ObservationIgnored private let customPath: String?
    @ObservationIgnored private var pollTask: Task<Void, Never>?
    /// Last decoded payload, to tell "nothing happened" from "we couldn't read".
    @ObservationIgnored private var lastDecoded: HerdrSnapshot.Decoded?
    /// Working directory per pane, from the same poll — the raw material for
    /// ``gitRoots()``.
    @ObservationIgnored private var paneCwds: [String: String] = [:]
    /// Workspaces that are git worktrees → the repo they came from.
    private var worktreeRepos: [String: String] = [:]
    /// Consecutive polls that returned exactly what we already had.
    @ObservationIgnored private var unchangedPolls = 0

    init(runner: HerdrCommandRunner, customPath: String? = nil) {
        self.runner = runner
        self.customPath = customPath
    }

    deinit { pollTask?.cancel() }

    // MARK: - Lifecycle

    /// Read once, then keep reading every ``pollInterval`` until ``stop()``.
    func start() {
        guard pollTask == nil else { return }
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.poll()
                let wait = self?.pollInterval ?? HerdrControlClient.fastPollInterval
                try? await Task.sleep(for: wait)
            }
        }
    }

    func stop() {
        pollTask?.cancel()
        pollTask = nil
    }

    /// `MultiplexerControlling.pollAgentHooks` — deliberately does nothing.
    ///
    /// tmux needs this: its agent stamps live in `@moshpit_*` options that
    /// only a separate `list-panes` reads, so the Vibe Island's sweep timer
    /// has to ask. herdr carries `agent_status` in the same snapshot this
    /// client is already polling, and `onAgentHooksUpdated` fires after every
    /// one — so answering the sweep would just add a second, uncoordinated
    /// poller on top of ours.
    ///
    /// It did, until this was measured: with the sweep also polling, an idle
    /// session made ~37 SSH exec calls a minute and the backoff below could
    /// never take effect, because the two pollers alternated at 2s regardless
    /// of what the timer decided.
    func pollAgentHooks() {}

    /// Drop back to the fast cadence and read now.
    ///
    /// Called when something happened that the backoff can't know about — the
    /// frame channel losing its attach, say, whose recovery rides on this
    /// poll and shouldn't wait out an idle interval.
    func quicken() {
        unchangedPolls = 0
        pollInterval = Self.fastPollInterval
        Task { [weak self] in await self?.poll() }
    }

    /// One poll, awaited — the cadence tests need to drive reads deterministically
    /// rather than race the timer.
    func pollNowForTesting() async { await poll() }

    /// `MultiplexerControlling.refresh` — a user-initiated re-read, with the
    /// spinner. The timer keeps running underneath; this just doesn't make
    /// them wait for the next tick.
    func refresh() {
        Task { [weak self] in
            guard let self else { return }
            // A user asking for fresh state is also a signal they're engaged,
            // so come off the idle cadence.
            unchangedPolls = 0
            pollInterval = Self.fastPollInterval
            isRefreshing = true
            await poll()
            isRefreshing = false
        }
    }

    // MARK: - Reading

    private func poll() async {
        guard let result = try? await run("api snapshot")
        else { return }   // channel hiccup — keep what we have
        let output = result.output
        if let decoded = HerdrSnapshot.decode(output) {
            serverNotRunning = false
            // Ease off only while the answer keeps coming back identical, and
            // snap straight back the moment anything moves.
            if decoded == lastDecoded {
                unchangedPolls += 1
                if unchangedPolls >= Self.idlePollThreshold {
                    pollInterval = Self.idlePollInterval
                }
            } else {
                unchangedPolls = 0
                pollInterval = Self.fastPollInterval
            }
            lastDecoded = decoded
            apply(decoded)
        } else if let mismatch = HerdrSnapshot.protocolMismatch(output) {
            // The server IS running — it just refuses our client's protocol.
            // Naming that beats the empty state's "create a session" advice,
            // which could never attach. Keep the last good tree: the panes
            // still exist server-side, and blanking them reads as data loss.
            onProtocolMismatch?(mismatch)
            unchangedPolls = 0
            pollInterval = Self.idlePollInterval   // it won't heal until a human restarts it
        } else if result.failed || HerdrSnapshot.isServerNotRunning(output) {
            // Not a failure: the host has herdr but no session yet. Empty the
            // tree so the UI can say so instead of showing a stale one.
            serverNotRunning = true
            // A host that just lost its server is about to get a new one (the
            // empty state's button, or the user's own shell) — stay attentive.
            unchangedPolls = 0
            pollInterval = Self.fastPollInterval
            lastDecoded = nil
            var empty = TmuxSnapshot()
            empty.everAttached = snapshot.everAttached
            snapshot = empty
            agentHooks = [:]
        }
        // Anything else (shell noise, a truncated read) is "no new
        // information" — keep the last good tree rather than blanking it.
    }

    /// Replace state in as few assignments as possible so SwiftUI sees one
    /// coherent change, and preserve the direction the user last switched in
    /// (the snapshot is rebuilt wholesale every poll, but that flag belongs to
    /// the gesture, not the server).
    private func apply(_ decoded: HerdrSnapshot.Decoded) {
        var next = decoded.snapshot
        next.lastSwitchForward = snapshot.lastSwitchForward
        next.everAttached = snapshot.everAttached || next.isAttached
        snapshot = next
        agentHooks = decoded.agentHooks
        paneCwds = decoded.paneCwds
        worktreeRepos = decoded.worktreeRepos
        if let pane = next.activePaneId {
            onFocusedPaneChanged?(pane)
        }
        onAgentHooksUpdated?()
    }

    // MARK: - Running commands

    /// One `herdr` invocation and how it ended.
    struct CommandResult: Equatable {
        var output: String
        /// The command's exit status, or nil if the trailer went missing.
        var exitCode: Int?
        var failed: Bool { (exitCode ?? 0) != 0 }
    }

    /// Marker carrying the remote exit status back to us.
    ///
    /// We can't read it any other way: the SSH layer merges stderr into stdout
    /// and discards the status, and herdr's own failure OUTPUT is not stable
    /// enough to match on — 0.7.3 prints a bare
    /// `Error: Os { code: 2, kind: NotFound … }` when no server is listening,
    /// while 0.8.0 prints a structured `server_not_running` JSON. Matching
    /// either string would break on the other; the exit code is 1 in both.
    static let exitCodeMarker = "__moshpit_rc="

    private func run(_ subcommand: String) async throws -> CommandResult {
        let command = HerdrLaunch.command(subcommand, customPath: customPath)
            + "; echo \"\(Self.exitCodeMarker)$?\""
        return Self.parse(try await runner.run(command))
    }

    /// Split the trailing exit-status marker off a command's output.
    static func parse(_ raw: String) -> CommandResult {
        guard let markerRange = raw.range(of: exitCodeMarker, options: .backwards) else {
            return CommandResult(output: raw, exitCode: nil)
        }
        let digits = raw[markerRange.upperBound...].prefix { $0.isNumber }
        return CommandResult(
            output: String(raw[raw.startIndex..<markerRange.lowerBound]),
            exitCode: Int(digits))
    }

    // MARK: - Agent tasks

    /// Repos the panes are sitting in right now. Instant — no remote call,
    /// just the cwds the last poll already carried — so the form can open
    /// with something in it while the fuller scan runs.
    var openRepoGuesses: [String] {
        var seen: Set<String> = []
        return paneCwds.values.sorted().filter { seen.insert($0).inserted }
    }

    /// Repositories on the host, most recently touched first.
    ///
    /// herdr has no concept of "your repos", and typing `/Users/you/code/thing`
    /// on a phone keyboard is miserable — so this actually goes and looks.
    ///
    /// Two sources with very different reliability, so they are two COMMANDS
    /// run concurrently — the first version fused them into one, and that
    /// shape failed exactly where this app lives: on a real device against
    /// the high-RTT test host the fused command hung (a big, iCloud-backed
    /// `$HOME` can stall `find` for minutes; a lossy link can kill the exec
    /// outright), its `try?` turned the hang into silence, and the form
    /// offered nothing but "Other…".
    ///
    ///  1. Pane cwds resolved through `git rev-parse` — one tiny round trip
    ///     that survives bad links, and is what "resume what I was doing"
    ///     means. These come first in the result.
    ///  2. A `$HOME` scan by mtime — genuinely useful on first run, but
    ///     strictly nice-to-have, so it gets a hard client-side `scanBudget`.
    ///     Missing the budget costs the discovery list, not the form.
    func gitRepos(scanBudget: Duration = .seconds(8)) async -> [String] {
        async let scanned = homeRepoScan(budget: scanBudget)
        let fromPanes = await paneRepos()
        var seen = Set(fromPanes)
        return fromPanes + (await scanned).filter { seen.insert($0).inserted }
    }

    /// Source 1: resolve every open pane's cwd to its repo root. Small enough
    /// to be reliable; a dead link falls back to the raw cwd guesses (the
    /// server validates the path at `worktree create` time anyway).
    private func paneRepos() async -> [String] {
        let openDirs = Set(paneCwds.values).sorted()
        guard !openDirs.isEmpty else { return [] }
        let quoted = openDirs.map { HerdrLaunch.quote($0) }.joined(separator: " ")
        let command = "for d in \(quoted); do git -C \"$d\" rev-parse --show-toplevel 2>/dev/null; done"
        guard let output = try? await runner.run(command) else { return openRepoGuesses }
        return Self.repoLines(output)
    }

    /// Source 2: the bounded `$HOME` walk, raced against `budget`. On budget
    /// exhaustion the result is simply dropped — the remote `find` is left to
    /// finish on its own (an exec channel can't be un-sent), which is why the
    /// command itself stays as cheap as the prune list can make it.
    private func homeRepoScan(budget: Duration) async -> [String] {
        let runner = self.runner
        return await withTaskGroup(of: [String]?.self) { group in
            group.addTask {
                guard let output = try? await runner.run(Self.homeScanCommand) else { return [] }
                return Self.repoLines(output)
            }
            group.addTask {
                try? await Task.sleep(for: budget)
                return nil
            }
            let first = await group.next() ?? nil
            group.cancelAll()
            return first ?? []
        }
    }

    /// The scan is deliberately shallow and prunes the expensive, useless
    /// places: `Library`, `node_modules`, `.build`, `vendor`, the media
    /// libraries (`Music`/`Movies`/`Pictures` — huge, and on iCloud-backed
    /// homes a `stat` there can BLOCK on materialisation), and every hidden
    /// directory except `.git` itself — without that last exclusion the list
    /// fills with `.vim` and `.oh-my-zsh`, which nobody starts an agent task
    /// in. `-L` so the common "~/code is a symlink onto the big disk" layout
    /// is still found. One line on purpose: a multi-line literal's
    /// indentation rules and shell line continuations fight each other.
    static let homeScanCommand =
        "find -L \"$HOME\" -maxdepth 4 "
        + "\\( -type d \\( -name Library -o -name node_modules -o -name .Trash "
        + "-o -name Applications -o -name Music -o -name Movies -o -name Pictures "
        + "-o -name .build -o -name vendor \\) -prune \\) -o "
        + "\\( -type d -name '.?*' ! -name .git -prune \\) -o "
        + "\\( -type d -name .git -print \\) 2>/dev/null "
        + "| while read -r g; do d=\"${g%/.git}\"; "
        + "printf '%s\\t%s\\n' \"$(stat -f %m \"$d\" 2>/dev/null "
        + "|| stat -c %Y \"$d\" 2>/dev/null)\" \"$d\"; "
        + "done | sort -rn | cut -f2 | head -40"

    /// Shared parse: keep absolute paths, first occurrence wins. Everything
    /// else in the merged stdout/stderr (login noise, git complaints) drops.
    /// nonisolated — it runs inside the scan's task-group child, off the actor.
    private nonisolated static func repoLines(_ output: String) -> [String] {
        var seen: Set<String> = []
        return output.split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { $0.hasPrefix("/") && seen.insert($0).inserted }
    }

    /// Agents herdr knows how to detect, e.g. `claude`, `codex`.
    func agentNames() async -> [String] {
        let command = HerdrLaunch.command("server agent-manifests --json", customPath: customPath)
        guard let raw = try? await runner.run(command),
              let start = raw.firstIndex(of: "["),
              let end = raw.lastIndex(of: "]"),
              let data = String(raw[start...end]).data(using: .utf8),
              let list = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]]
        else { return [] }
        return list.compactMap { $0["agent"] as? String }.sorted()
    }

    /// Create an isolated worktree and start an agent in it.
    ///
    /// Returns nil on success, or a message to show the user. The steps are
    /// deliberately ordered so a failure leaves nothing half-built: the
    /// worktree either exists (and is visible in the tree, where it can be
    /// removed) or it doesn't.
    ///
    /// `worktree create` alone yields a workspace whose root pane is already
    /// sitting in the new checkout — so the agent is started by typing into
    /// THAT pane rather than by `agent start`, which does not inherit the
    /// worktree's directory (verified: it lands in the CLI's own cwd).
    func startAgentTask(_ request: AgentTaskRequest) async -> String? {
        if let invalid = request.validationError { return invalid }
        let branch = request.branch.trimmingCharacters(in: .whitespaces)

        var create = "worktree create --cwd \(HerdrLaunch.quote(request.repoPath))"
        create += " --branch \(HerdrLaunch.quote(branch))"
        create += " --label \(HerdrLaunch.quote(branch)) --focus --json"
        guard let result = try? await run(create) else {
            return String(localized: "Couldn't reach the host")
        }
        guard let json = HerdrSnapshot.firstJSONObject(in: result.output) else {
            return result.failed ? String(localized: "Creating the worktree failed") : nil
        }
        if let message = Self.errorMessage(json) { return message }
        guard let payload = json["result"] as? [String: Any],
              let rootPane = (payload["root_pane"] as? [String: Any])?["pane_id"] as? String
        else { return String(localized: "The worktree was created but has no pane") }

        // `pane run` types the command and presses return, which is exactly
        // what the user would have done — and what they see happen.
        _ = try? await run("pane run \(HerdrLaunch.quote(rootPane)) \(HerdrLaunch.quote(request.agent))")

        let prompt = request.prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        if !prompt.isEmpty {
            // Give the agent a moment to be ready for input before typing at
            // it; `agent send` writes literal text with no return of its own.
            try? await Task.sleep(for: .seconds(2))
            _ = try? await run("agent send \(HerdrLaunch.quote(rootPane)) \(HerdrLaunch.quote(prompt))")
        }
        await poll()
        return nil
    }

    /// `MultiplexerControlling.worktreeRepo` — only a workspace herdr created
    /// from a repo answers yes.
    func worktreeRepo(for sessionId: String) -> String? { worktreeRepos[sessionId] }

    /// What happened when we tried to remove a task's worktree.
    enum WorktreeRemoval: Equatable {
        case removed
        /// The checkout has modified or untracked files. Deleting anyway
        /// destroys them, so this comes back to the caller for a second, much
        /// sharper confirmation instead of being forced silently.
        case needsForce
        case failed(String)
    }

    /// Remove a task's worktree — the branch checkout AND its workspace.
    ///
    /// Never forces on the first attempt. herdr refuses a dirty checkout with
    /// `dirty_worktree_requires_force` and leaves every file in place
    /// (verified against 0.7.3), which is exactly the safety net worth
    /// keeping: uncommitted work is the one thing this action can destroy
    /// irrecoverably.
    func removeWorktree(sessionId: String, force: Bool = false) async -> WorktreeRemoval {
        var command = "worktree remove --workspace \(HerdrLaunch.quote(sessionId))"
        if force { command += " --force" }
        guard let result = try? await run(command) else {
            return .failed(String(localized: "Couldn't reach the host"))
        }
        let json = HerdrSnapshot.firstJSONObject(in: result.output)
        if let json, let error = json["error"] as? [String: Any] {
            if (error["code"] as? String) == "dirty_worktree_requires_force" {
                return .needsForce
            }
            return .failed((error["message"] as? String) ?? String(localized: "Removing the worktree failed"))
        }
        if result.failed, json == nil {
            return .failed(String(localized: "Removing the worktree failed"))
        }
        await poll()
        return .removed
    }

    /// herdr's structured error text, if the payload carries one.
    static func errorMessage(_ json: [String: Any]) -> String? {
        guard let error = json["error"] as? [String: Any] else { return nil }
        return (error["message"] as? String) ?? (error["code"] as? String)
    }

    // MARK: - Commands

    /// Run a `herdr` subcommand, then re-read immediately so the UI reflects
    /// it without waiting for the next tick.
    ///
    /// A failure isn't surfaced as an error of its own — the re-read that
    /// follows tells the truth. That matters most for the case that actually
    /// bit a user: tapping "new window" against a host whose herdr server has
    /// stopped used to do nothing at all, silently. Now the same poll that
    /// follows the failed command notices the dead socket and the UI drops to
    /// the empty state, which offers to start one.
    private func send(_ subcommand: String) {
        Task { [weak self] in
            guard let self else { return }
            do {
                let result = try await run(subcommand)
                if result.failed {
                    Log.ssh.error("herdr command failed: \(subcommand, privacy: .public)")
                }
            } catch {
                Log.ssh.error("herdr command could not run: \(subcommand, privacy: .public)")
            }
            await self.poll()
        }
    }

    func selectSession(_ sessionId: String) {
        send("workspace focus \(HerdrLaunch.quote(sessionId))")
    }

    func selectWindow(_ windowId: String) {
        send("tab focus \(HerdrLaunch.quote(windowId))")
    }

    /// Focus one specific pane.
    ///
    /// `herdr agent focus` is the only CLI command that takes a pane id —
    /// `herdr pane focus` is directional only, in both 0.7.3 and 0.8.0, and
    /// the socket API's own `pane.focus` is not exposed by the CLI at all.
    ///
    /// On a pane with no detected agent it prints `agent_not_found` **and
    /// still moves the focus**: the server resolves the target, focuses it,
    /// and only then fails building the agent payload for the response
    /// (`focus_agent_target` in herdr's `src/app/agents.rs` — same order in
    /// both versions). So the error is about the reply, not the action; we
    /// discard it deliberately rather than treating it as a failure.
    func selectPane(_ paneId: String) {
        // Drive the renderer from the id we already hold instead of waiting to
        // rediscover it. Left to the poll, a switch costs two sequential SSH
        // execs before the frame channel even learns its new target — `agent
        // focus`, then the `api snapshot` that reports the focus moved — and
        // the old pane stays painted for all of it (the "I still see the
        // previous agent" report).
        //
        // The frame channel needs neither exec: its start command names the
        // target explicitly and takes it over, so it can be pointed at a pane
        // the server has not focused yet. Retargeting here runs the reattach
        // concurrently with telling the server where focus went.
        //
        // The poll's own retarget stays the self-healing path — it repeats the
        // focused pane every tick and the receiver ignores a target it is
        // already rendering, so the confirmation costs nothing. If `agent
        // focus` fails outright, that same repeat corrects both the snapshot
        // and the channel on the next tick.
        snapshot.activePaneId = paneId
        onFocusedPaneChanged?(paneId)
        send("agent focus \(HerdrLaunch.quote(paneId)) 2>/dev/null || true")
    }

    /// Create a session — or, on a host with no server at all, start one.
    ///
    /// Both come from the same explicit user action (the empty state's button),
    /// which is the only thing that may bring a session into existence. A
    /// freshly started server arrives with its own first workspace, so there's
    /// nothing to create on top of it.
    func newSession(named name: String?) {
        guard !serverNotRunning else {
            Task { [weak self] in await self?.bootstrapServer() }
            return
        }
        var command = "workspace create --focus"
        if let name, !name.isEmpty { command += " --label \(HerdrLaunch.quote(name))" }
        send(command)
    }

    /// Start the headless server, wait for it to answer, then make sure there
    /// is actually a workspace to attach to.
    ///
    /// The second step is not optional. A server started with no persisted
    /// state comes up **empty** — zero workspaces, zero panes. (It looks like
    /// it brings its own only when it restores a previous session from
    /// `~/.config/herdr/session.json`.) Without the create, the user taps
    /// "create a session", a server appears, and the empty state stays put.
    private func bootstrapServer() async {
        _ = try? await runner.run(HerdrLaunch.daemonCommand(customPath: customPath))
        for _ in 0..<10 {
            try? await Task.sleep(for: .milliseconds(500))
            await poll()
            guard !serverNotRunning else { continue }
            if snapshot.sessions.isEmpty {
                _ = try? await runner.run(
                    HerdrLaunch.command("workspace create --focus", customPath: customPath))
                await poll()
            }
            return
        }
    }

    func newWindow(named name: String?) {
        var command = "tab create --focus"
        if let name, !name.isEmpty { command += " --label \(HerdrLaunch.quote(name))" }
        send(command)
    }

    /// Split the focused pane. herdr needs an explicit target, and `--current`
    /// means "the pane this CLI invocation runs in" — which for us is nothing,
    /// since we run over our own SSH channel — so pass the id from the
    /// snapshot instead.
    func newPane() {
        guard let target = snapshot.activePaneId ?? snapshot.activePanes.first?.id else { return }
        send("pane split \(HerdrLaunch.quote(target)) --direction right --focus")
    }

    func renameSession(_ sessionId: String, to name: String) {
        send("workspace rename \(HerdrLaunch.quote(sessionId)) \(HerdrLaunch.quote(name))")
    }

    func killSession(_ sessionId: String) {
        send("workspace close \(HerdrLaunch.quote(sessionId))")
    }

    func renameWindow(_ windowId: String, to name: String) {
        send("tab rename \(HerdrLaunch.quote(windowId)) \(HerdrLaunch.quote(name))")
    }

    func killWindow(_ windowId: String) {
        send("tab close \(HerdrLaunch.quote(windowId))")
    }

    func killPane(_ paneId: String) {
        send("pane close \(HerdrLaunch.quote(paneId))")
    }
}
