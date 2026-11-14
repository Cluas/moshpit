import Foundation
import Observation
import MoshpitKit

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
class HerdrControlClient: MultiplexerControlling {

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
    /// Cadence once the push channel is carrying events. The poll stops being
    /// the update mechanism at that point — every change arrives as an event
    /// and is answered by an immediate read — so the timer is left as a pure
    /// safety net against a pump that dies quietly. Measured: this is the
    /// difference between ~7 exec channels a minute at idle and about one.
    static let pushedPollInterval: Duration = .seconds(45)

    /// How often this client's agent stamps arrive, in seconds: the poll
    /// once nothing changes, or the safety net while push is up. The agents
    /// machine holds a stamp that long and a margin (B346).
    var stampInterval: TimeInterval {
        let interval = pushActive ? Self.pushedPollInterval : Self.idlePollInterval
        return Double(interval.components.seconds)
    }

    /// Set by the hub when the push channel comes up (and false when it dies).
    var pushActive = false {
        didSet {
            guard pushActive != oldValue else { return }
            if !pushActive { unchangedPolls = 0; pollInterval = Self.fastPollInterval }
        }
    }
    /// Identical polls before easing off. Three keeps a quick back-and-forth
    /// (type, watch, type) at full speed.
    static let idlePollThreshold = 3

    /// When a poll last came back with something we could read. The hub's
    /// keepalive consults this instead of spending its own exec channel: a
    /// control plane that is answering has already proved the connection is
    /// alive, and on this transport its replies never touch the PTY stream
    /// the hub otherwise watches.
    private(set) var lastSuccessfulPoll: Date?

    private(set) var snapshot = TmuxSnapshot()
    private(set) var agentHooks: [String: AgentHook] = [:]
    private(set) var agentHooksLoaded = false
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

    /// Fired by ``selectPane(_:)``, ahead of ``onFocusedPaneChanged``: the
    /// person picked this pane (a tap, a notification they opened, a reply
    /// sent for them), where a read only reports the server's focus. The
    /// frame channel holds what they type at a pane they picked until its
    /// attach is up; a pane the focus moved to without them gets nothing
    /// typed into it before it is on screen.
    @ObservationIgnored var onPanePicked: ((String) -> Void)?

    /// `MultiplexerControlling.onAgentHooksUpdated` — fired after every poll
    /// that produced a tree, since agent state rides the same snapshot.
    @ObservationIgnored var onAgentHooksUpdated: (() -> Void)?

    /// Fired when the server refuses us over version skew (protocol
    /// mismatch) — the hub routes it to the terminal banner, because the fix
    /// (restart/upgrade the herdr server on the host) belongs to the user.
    @ObservationIgnored var onProtocolMismatch: ((String) -> Void)?

    /// Fired when a read comes back a tree again after a mismatch: the
    /// server answers this client now (fixed outside the app, or restarted),
    /// so a banner that said otherwise is stale (B408).
    @ObservationIgnored var onServerAnswersAgain: (() -> Void)?

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
    /// herdr terminal id per pane — what the mosh raw-attach renderer targets
    /// (`terminal attach` refuses pane ids; see HerdrSnapshot.Decoded).
    @ObservationIgnored private(set) var terminalIds: [String: String] = [:]
    /// Workspaces that are git worktrees → the repo they came from.
    private var worktreeRepos: [String: String] = [:]
    /// Consecutive polls that returned exactly what we already had.
    @ObservationIgnored private var unchangedPolls = 0

    init(runner: HerdrCommandRunner, customPath: String? = nil) {
        self.runner = runner
        self.customPath = customPath
    }

    deinit { pollTask?.cancel() }

    /// How long one snapshot read may take before it is given up, and the
    /// next one scheduled. The poll loop awaits each read, and a read that
    /// never answered — a half-open connection, a wedged server — stopped it
    /// for good: no tree, no focus reports, so the frame channel's recovery
    /// (the repeated focus report) never ran again (B189). Per instance so
    /// tests can shorten it, like the budgets below.
    @ObservationIgnored var pollReadBudget: Duration = .seconds(10)
    /// How long `herdr status` may take — the skew diagnosis inside the poll,
    /// and the restart's check. Its hang held the poll the same way.
    @ObservationIgnored var statusBudget: Duration = .seconds(5)
    /// `server stop` and the daemon start: each given up after this. On a
    /// link that stopped answering neither returned, and a restart said
    /// "Restarting…" for good (B190).
    @ObservationIgnored var serverCommandBudget: Duration = .seconds(10)
    /// The bootstrap's first workspace, waited for at most this.
    @ObservationIgnored var bootstrapCreateBudget: Duration = .seconds(10)
    /// Between `server stop` and the start, and before each of the
    /// bootstrap's ten reads.
    @ObservationIgnored var restartPause: Duration = .seconds(1)
    @ObservationIgnored var bootstrapInterval: Duration = .milliseconds(500)
    /// Reads applied so far: a restart is judged on the server it read after
    /// it began, never on what was known before.
    @ObservationIgnored private var readsApplied = 0

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
        let epoch = commandEpoch
        let startedQuiet = commandsQueued == 0
        guard let result = await read("api snapshot", within: pollReadBudget)
        else { return }   // channel hiccup, or no answer in time — keep what we have
        let output = result.output
        lastSuccessfulPoll = Date()   // the channel answered — see the property
        // A command was queued, or still on its way, while this read was out:
        // the server may have answered before the command landed, so the focus
        // and tree it reports are the ones the user just moved away from.
        // Applying them pulled the app back to the pane it had left until the
        // next tick put it right. The re-read after the last command follows.
        guard startedQuiet, epoch == commandEpoch else { return }
        readsApplied &+= 1
        if let decoded = HerdrSnapshot.decode(output) {
            let wasRefused = lastPollMismatched || protocolMismatch != nil || serverNotRunning
            serverNotRunning = false
            lastPollMismatched = false
            protocolMismatch = nil
            versionSkew = nil
            if wasRefused { onServerAnswersAgain?() }
            // Ease off only while the answer keeps coming back identical, and
            // snap straight back the moment anything moves.
            if decoded == lastDecoded {
                unchangedPolls += 1
                if unchangedPolls >= Self.idlePollThreshold {
                    pollInterval = pushActive ? Self.pushedPollInterval : Self.idlePollInterval
                }
            } else {
                unchangedPolls = 0
                pollInterval = Self.fastPollInterval
            }
            lastDecoded = decoded
            apply(decoded)
            // A tab/workspace switch (or the initial attach) lands here with
            // the target's own focused pane finally named — zoom it now.
            if zoomAfterNextApply, immersiveZoom, let paneId = snapshot.activePaneId {
                zoomAfterNextApply = false
                sendImmersiveZoom(paneId)
            }
        } else if let mismatch = HerdrSnapshot.protocolMismatch(output) {
            // The server IS running — it just refuses our client's protocol.
            // Naming that beats the empty state's "create a session" advice,
            // which could never attach. Keep the last good tree: the panes
            // still exist server-side, and blanking them reads as data loss.
            lastPollMismatched = true
            // Name the versions once per episode, before anyone is told —
            // which side is older decides whether "restart" is a remedy at all.
            if protocolMismatch == nil { await diagnoseVersionSkew() }
            protocolMismatch = mismatch
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
            agentHooksLoaded = true
            // What the tree said of its panes goes with it. The terminal ids
            // stayed: a pick or a reply for a pane of the gone tree pointed
            // the mosh renderer at a terminal no server has (a restarted one
            // numbers them anew), and the frame channel's checks read panes
            // that weren't there.
            terminalIds = [:]
            paneCwds = [:]
            worktreeRepos = [:]
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
        next.hasSwipedSinceAttach = snapshot.hasSwipedSinceAttach
        next.everAttached = snapshot.everAttached || next.isAttached
        snapshot = next
        agentHooks = decoded.agentHooks
        agentHooksLoaded = true
        paneCwds = decoded.paneCwds
        terminalIds = decoded.terminalIds
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

    /// A read, given up after `budget`: nil for no answer in time, as for one
    /// that couldn't run. The exec is left to finish on its own (one on a
    /// half-open connection may never; nothing waits for it).
    private func read(_ subcommand: String, within budget: Duration) async -> CommandResult? {
        var result: CommandResult?
        let answered = await WriteLane.wait(atMost: budget) { [weak self] in
            result = try? await self?.run(subcommand)
        }
        return answered ? result : nil
    }

    /// Split the trailing exit-status marker off a command's output.
    static func parse(_ raw: String) -> CommandResult {
        guard let markerRange = raw.range(of: exitCodeMarker, options: .backwards) else {
            return CommandResult(output: raw, exitCode: nil)
        }
        // The trailer counts only as the last thing printed: the marker, its
        // digits, then nothing but whitespace (B193). An answer cut off
        // before the real trailer, inside herdr's own text holding the
        // marker (a label, a pane title, a cwd), read as that status.
        let tail = raw[markerRange.upperBound...]
        let digits = tail.prefix { $0.isASCII && $0.isNumber }
        guard !digits.isEmpty, tail.dropFirst(digits.count).allSatisfy(\.isWhitespace),
              let code = Int(digits) else {
            return CommandResult(output: raw, exitCode: nil)
        }
        return CommandResult(output: String(raw[raw.startIndex..<markerRange.lowerBound]), exitCode: code)
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
        // Bounded (B120): one that never answers falls back like a dead link.
        guard let output = await runRaw(command, within: discoveryBudget) else { return openRepoGuesses }
        return Self.repoLines(output)
    }

    /// Source 2: the bounded `$HOME` walk. Past `budget` the result is
    /// simply dropped — the remote `find` is left to finish on its own (an
    /// exec channel can't be un-sent), which is why the command itself stays
    /// as cheap as the prune list can make it.
    private func homeRepoScan(budget: Duration) async -> [String] {
        guard let output = await runRaw(Self.homeScanCommand, within: budget) else { return [] }
        return Self.repoLines(output)
    }

    /// How long the panes' repo resolve and the agent list may take. On a
    /// half-open connection neither answered: the New agent task form's
    /// "Finding repositories…" never ended, and its agent list stayed empty
    /// for good (B120). Per instance so tests can shorten it.
    @ObservationIgnored var discoveryBudget: Duration = .seconds(10)

    /// A command run as given (no herdr, no trailer), given up after
    /// `budget`: nil for no answer in time, as for one that couldn't run.
    /// Nothing waits for the exec past that — not even a task group, which
    /// waits for every child, and an exec on a half-open connection may never
    /// notice it was cancelled: the scan raced against its budget that way
    /// still held the form until the walk gave up on its own.
    private func runRaw(_ command: String, within budget: Duration) async -> String? {
        let runner = self.runner
        var output: String?
        let answered = await WriteLane.wait(atMost: budget) {
            output = try? await runner.run(command)
        }
        return answered ? output : nil
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
        // Bounded (B120): a list that never answers is an empty one.
        guard let raw = await runRaw(command, within: discoveryBudget) else { return [] }
        return Self.agentNames(inManifests: raw)
    }

    /// The agent names in a `server agent-manifests --json` reply, sorted:
    /// the JSON array from its first `[` to its last `]`, whatever login
    /// noise the merged output carries around it. No list for anything else
    /// — including a last `]` that comes BEFORE the first `[` (a banner with
    /// a `]` in it, then a reply cut short): slicing that range trapped, and
    /// opening the New agent task form crashed the app. The brackets are
    /// found as bytes, the way herdr writes them.
    nonisolated static func agentNames(inManifests raw: String) -> [String] {
        let bytes = Array(raw.utf8)
        guard let start = bytes.firstIndex(of: UInt8(ascii: "[")),
              let end = bytes.lastIndex(of: UInt8(ascii: "]")), start < end,
              let list = (try? JSONSerialization.jsonObject(with: Data(bytes[start...end]))) as? [[String: Any]]
        else { return [] }
        return list.compactMap { $0["agent"] as? String }.sorted()
    }

    /// Create an isolated worktree and start an agent in it.
    ///
    /// The steps are deliberately ordered so a failure leaves nothing
    /// half-built: the worktree either exists (and is visible in the tree,
    /// where it can be removed) or it doesn't. Past the worktree, what didn't
    /// land comes back as ``AgentTaskOutcome/incomplete(_:)`` — the task is
    /// there, so the form must not offer to make it again.
    ///
    /// `worktree create` alone yields a workspace whose root pane is already
    /// sitting in the new checkout — so the agent is started by typing into
    /// THAT pane rather than by `agent start`, which does not inherit the
    /// worktree's directory (verified: it lands in the CLI's own cwd).
    func startAgentTask(_ request: AgentTaskRequest) async -> AgentTaskOutcome {
        if let invalid = request.validationError { return .failed(invalid) }
        let branch = request.branch.trimmingCharacters(in: .whitespaces)

        var create = "worktree create --cwd \(HerdrLaunch.quote(request.repoPath))"
        create += " --branch \(HerdrLaunch.quote(branch))"
        create += " --label \(HerdrLaunch.quote(branch)) --focus --json"
        // Every step takes its turn on the command lane. `--focus` moves the
        // server's focus the way a tap does: run beside the lane, a pane
        // picked while the worktree was being made was overridden when the
        // create landed after it, and a poll read in between pulled the app
        // back to a pane the server was about to leave.
        //
        // Bounded: the lane stops waiting for a command after its budget,
        // but this answer used to be waited for forever, and on a half-open
        // connection none comes — the form sat on "Starting…" for good.
        // A create still queued when the budget runs out is never sent.
        let (createAnswered, created) = await runOnLane(create, within: worktreeCreateBudget)
        guard createAnswered else {
            Log.ssh.error("herdr worktree create: no answer from the host")
            return .failed(String(localized: "The host stopped answering while the worktree was being made, so no agent was started. Look for a “\(branch)” workspace before trying again."))
        }
        guard let result = created else {
            return .failed(String(localized: "Couldn't reach the host"))
        }
        guard let json = HerdrSnapshot.firstJSONObject(in: result.output) else {
            if result.failed { return .failed(String(localized: "Creating the worktree failed")) }
            // No answer to read, yet no failure either: exit 0 with no JSON,
            // or the exit status lost on the way back. Either way herdr named
            // no pane, so no agent was started and no first message sent —
            // and this used to report the task started, closing the form over
            // nothing. Whether the worktree exists is anyone's guess; the
            // re-read shows it in the tree if it does.
            let status = result.exitCode.map { "exit \($0)" } ?? "no exit status"
            Log.ssh.error("herdr worktree create: no result (\(status, privacy: .public)): \(String(result.output.prefix(200)), privacy: .public)")
            await poll()
            return .failed(String(localized: "herdr didn't confirm the worktree, so no agent was started. Look for a “\(branch)” workspace before trying again."))
        }
        if let message = Self.errorMessage(json) { return .failed(message) }
        guard let payload = json["result"] as? [String: Any],
              let rootPane = (payload["root_pane"] as? [String: Any])?["pane_id"] as? String
        else { return .incomplete(String(localized: "The worktree was created but has no pane")) }

        // The form was cancelled while the worktree was being made: it is
        // there (the tree shows it), and nothing more is done in it.
        if Task.isCancelled {
            await poll()
            return .incomplete(String(localized: "Cancelled. The worktree is ready, but no agent was started in it."))
        }

        // `pane run` types the command and presses return, which is exactly
        // what the user would have done — and what they see happen. Bounded
        // like the create.
        let launchCommand = "pane run \(HerdrLaunch.quote(rootPane)) \(HerdrLaunch.quote(request.agent))"
        let (launchAnswered, launch) = await runOnLane(launchCommand, within: agentLaunchBudget)
        guard launchAnswered else {
            Log.ssh.error("herdr pane run: no answer from the host")
            return .incomplete(String(localized: "The worktree is ready, but the host stopped answering before \(request.agent) started."))
        }
        if let reason = Self.refusal(launch) {
            Log.ssh.error("herdr pane run failed: \(reason, privacy: .public)")
            await poll()
            return .incomplete(String(localized: "The worktree is ready, but \(request.agent) didn't start: \(reason)"))
        }

        let prompt = request.prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        var outcome = AgentTaskOutcome.started
        if !prompt.isEmpty {
            outcome = await sendFirstMessage(prompt, to: rootPane, agent: request.agent)
        }
        await poll()
        return outcome
    }

    // MARK: The first message

    /// The first herdr whose CLI has `agent prompt` — the same release took
    /// `agent send` away ("`agent send` was replaced by `agent send-keys`",
    /// 0.7.5's changelog) and moved the protocol from 16 to 17. A client and
    /// server that accept each other are always on the same side of it.
    static let agentPromptVersion = "0.7.5"

    /// The command that hands `text` to the agent in `paneId`, for a server
    /// running `serverVersion` — nil when that couldn't be read, which means
    /// today's herdr.
    ///
    /// From 0.7.5 that is `agent prompt`: it types the text, waits a beat,
    /// and presses Enter — the turn starts. Before it, `agent send`, which
    /// types the text and stops there; it is all those versions have, so
    /// the Enter is ours to press (``submitCommand(paneId:)``). Sending
    /// `agent send` to a 0.7.5+ herdr is a usage error that never reaches
    /// the server — the first message was lost that way on every current
    /// host, and nothing said so.
    static func firstMessageCommand(_ text: String, paneId: String, serverVersion: String?) -> String {
        let pane = HerdrLaunch.quote(paneId)
        if typesWithoutSubmitting(serverVersion: serverVersion) {
            return "agent send \(pane) \(HerdrLaunch.quote(text))"
        }
        return "agent prompt \(pane) \(HerdrLaunch.quote(text))"
    }

    /// Whether a server running `serverVersion` takes the first message as
    /// `agent send` — text typed, no Enter — so that the turn only starts
    /// once we press it. Every herdr before 0.7.5.
    static func typesWithoutSubmitting(serverVersion: String?) -> Bool {
        guard let serverVersion else { return false }
        return HerdrVersionReport.compare(serverVersion, agentPromptVersion) == .orderedAscending
    }

    /// The Enter under a first message `agent send` typed: `pane send-keys`,
    /// which every herdr here has, encoded by herdr against the pane's own
    /// terminal modes — the key `pane run` and, from 0.7.5, `agent prompt`
    /// press themselves.
    static func submitCommand(paneId: String) -> String {
        "pane send-keys \(HerdrLaunch.quote(paneId)) \(HerdrLaunch.quote("Enter"))"
    }

    /// How long a task waits for `worktree create` to answer. Generous — a
    /// big repository takes a while to check out — but bounded: on a
    /// half-open connection no answer ever comes, and the New Agent Task form
    /// sat on "Starting…" for good. Per instance so tests can shorten it,
    /// like the timings below.
    @ObservationIgnored var worktreeCreateBudget: Duration = .seconds(60)
    /// How long a task waits for `pane run` — it only types the agent's
    /// command into the new pane.
    @ObservationIgnored var agentLaunchBudget: Duration = .seconds(20)

    /// The pause before the first try: `pane run` has only typed the agent's
    /// command, and an agent takes a moment to come up. Per instance so tests
    /// can shorten it, like the timings below.
    @ObservationIgnored var firstMessageSettle: Duration = .seconds(2)
    /// How long to keep trying while herdr can't see the agent in the pane
    /// yet. `agent prompt` only takes a pane its agent is detected in, and
    /// detection trails the process starting — on a slow host by seconds.
    /// 30s is what herdr's own `agent start` gives an agent to be ready.
    @ObservationIgnored var firstMessagePatience: Duration = .seconds(30)
    /// Between tries.
    @ObservationIgnored var firstMessageRetryInterval: Duration = .seconds(1)
    /// Between a first message `agent send` typed and the Enter we press
    /// under it: the beat herdr's own `agent prompt` waits, so the agent has
    /// taken the text as typed before Enter arrives rather than swallowing
    /// both as one paste.
    @ObservationIgnored var firstMessageSubmitDelay: Duration = .milliseconds(300)

    /// Hand the first message to the agent the way the host's herdr takes
    /// it, on the command lane, and say what happened.
    private func sendFirstMessage(_ text: String, to paneId: String, agent: String) async -> AgentTaskOutcome {
        // Which command that is depends on the herdr on the host. Ask while
        // the agent comes up, so knowing costs no time.
        let versions = Task { await self.readVersions() }
        try? await Task.sleep(for: firstMessageSettle)
        let serverVersion = await versions.value?.server
        let command = Self.firstMessageCommand(text, paneId: paneId, serverVersion: serverVersion)
        let deadline = ContinuousClock.now.advanced(by: firstMessagePatience)
        while true {
            // Bounded, so a host that stops answering can't leave the form
            // spinning; the try that ran out may still land, and the message
            // says the host stopped answering — not that nothing arrived.
            let remaining = ContinuousClock.now.duration(to: deadline)
            let (answered, answer) = await runOnLane(command, within: max(remaining, commandBudget))
            guard answered, let answer else {
                Log.ssh.error("herdr first message: no answer from the host")
                return .incomplete(String(localized: "The task is running, but the host stopped answering before the first message was sent."))
            }
            if let reason = Self.refusal(answer) {
                guard Self.agentNotUpYet(answer) else {
                    Log.ssh.error("herdr first message refused: \(reason, privacy: .public)")
                    return .incomplete(String(localized: "The task is running, but \(agent) didn't get the first message: \(reason)"))
                }
                guard ContinuousClock.now < deadline else {
                    Log.ssh.error("herdr first message: agent never came up: \(reason, privacy: .public)")
                    return .incomplete(String(localized: "The task is running, but \(agent) wasn't ready for the first message in time."))
                }
                try? await Task.sleep(for: firstMessageRetryInterval)
                continue
            }
            guard Self.typesWithoutSubmitting(serverVersion: serverVersion) else { return .started }
            return await submitFirstMessage(in: paneId, agent: agent)
        }
    }

    /// Press Enter under a first message `agent send` typed, as `agent prompt`
    /// does by itself from 0.7.5. On an older herdr the text used to stop
    /// there: the message sat in the agent's input box, its turn never
    /// started, and the form closed as though it had been sent.
    private func submitFirstMessage(in paneId: String, agent: String) async -> AgentTaskOutcome {
        try? await Task.sleep(for: firstMessageSubmitDelay)
        // Bounded like the message itself: past the budget it counts as the
        // host not answering.
        let (_, pressed) = await runOnLane(Self.submitCommand(paneId: paneId), within: commandBudget)
        if let reason = Self.refusal(pressed) {
            Log.ssh.error("herdr first message typed, but Enter failed: \(reason, privacy: .public)")
            return .incomplete(String(localized: "The task is running, but the first message is still in \(agent)'s input: pressing Return failed (\(reason)). Press Return in the pane to send it."))
        }
        return .started
    }

    /// What `herdr status` says each end runs — nil when it can't be read
    /// within ``commandBudget``. The same report the version-skew diagnosis
    /// reads (``diagnoseVersionSkew()``), taken here whether or not the two
    /// ends agree.
    private func readVersions() async -> HerdrVersionReport? {
        var report: HerdrVersionReport?
        await WriteLane.wait(atMost: commandBudget) { [weak self] in
            guard let self, let result = try? await run("status") else { return }
            report = HerdrVersionReport.parse(result.output)
        }
        return report
    }

    /// Why a command didn't do its job, or nil when it did: herdr's own
    /// error text when it gave one, else the first thing it printed (a
    /// usage error is plain text), else that it couldn't run at all.
    ///
    /// Done takes evidence: exit status 0, or herdr's own result printed.
    /// The exit status rides a trailer echoed AFTER herdr exits
    /// (``exitCodeMarker``), and a channel that closes before it arrives
    /// left no status at all — which read as 0: `pane run` and
    /// `agent prompt` cut off that way reported the agent started and the
    /// message sent (as `worktree create` did before round twelve). With
    /// neither the status nor herdr's result, how it went is unknown, and
    /// unknown is not done.
    static func refusal(_ result: CommandResult?) -> String? {
        guard let result else { return String(localized: "Couldn't reach the host") }
        let json = HerdrSnapshot.firstJSONObject(in: result.output)
        if let json, let message = errorMessage(json) {
            return message
        }
        guard let exitCode = result.exitCode else {
            // herdr printed its result before the trailer was lost: it ran.
            if json?["result"] != nil { return nil }
            return String(localized: "herdr's answer was cut off")
        }
        guard exitCode != 0 else { return nil }
        let printed = result.output.split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty }
        return printed.map { String($0.prefix(200)) }
            ?? String(localized: "herdr exited with status \(exitCode)")
    }

    /// herdr turned the message away because it doesn't see the agent in
    /// that pane yet — `agent_not_found` until detection names it,
    /// `agent_not_ready` while it is still starting. Anything else (an
    /// agent waiting on a question, a server that's gone) won't change by
    /// asking again.
    static func agentNotUpYet(_ result: CommandResult) -> Bool {
        guard let json = HerdrSnapshot.firstJSONObject(in: result.output),
              let code = (json["error"] as? [String: Any])?["code"] as? String else { return false }
        return code == "agent_not_found" || code == "agent_not_ready"
    }

    /// `MultiplexerControlling.worktreeRepo` — only a workspace herdr created
    /// from a repo answers yes.
    func worktreeRepo(for sessionId: String) -> String? { worktreeRepos[sessionId] }

    /// Where the pane's foreground program runs, from the last snapshot —
    /// what an agent's relative paths are relative to.
    func workingDirectory(ofPane paneId: String) -> String? { paneCwds[paneId] }

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
        // On the lane like any command that changes the tree: closing the
        // workspace can move the focus too. Bounded (B119): the lane stops
        // waiting for a command after its budget, but this answer was waited
        // for forever, and on a half-open connection none comes — the sheet
        // spun for good.
        let (answered, answer) = await runOnLane(command, within: worktreeRemoveBudget)
        guard answered else {
            Log.ssh.error("herdr worktree remove: no answer from the host")
            return .failed(String(localized: "The host stopped answering while the worktree was being removed."))
        }
        guard let result = answer else {
            return .failed(String(localized: "Couldn't reach the host"))
        }
        let json = HerdrSnapshot.firstJSONObject(in: result.output)
        if let json, let error = json["error"] as? [String: Any] {
            if (error["code"] as? String) == "dirty_worktree_requires_force" {
                return .needsForce
            }
            return .failed((error["message"] as? String) ?? String(localized: "Removing the worktree failed"))
        }
        // Removed takes evidence: herdr's result printed, or exit status 0.
        // An answer cut off before its exit status, with nothing printed, says
        // nothing about the checkout, and read as removed (B119, the B90
        // class): a worktree reported gone that may still be there.
        if json?["result"] == nil {
            guard let exitCode = result.exitCode else {
                Log.ssh.error("herdr worktree remove: answer cut off: \(String(result.output.prefix(200)), privacy: .public)")
                return .failed(String(localized: "herdr's answer was cut off"))
            }
            guard exitCode == 0 else {
                return .failed(String(localized: "Removing the worktree failed"))
            }
        }
        await poll()
        return .removed
    }

    /// How long a removal waits for `worktree remove` to answer. Generous —
    /// a big checkout takes a while to delete — but bounded. Per instance so
    /// tests can shorten it.
    @ObservationIgnored var worktreeRemoveBudget: Duration = .seconds(60)

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
    ///
    /// Queued on ``commandLane``, so commands land in the order they were
    /// issued — see ``sendSequence(_:landed:)``. `landed` runs once the lane
    /// is done with the command: it finished, or its budget ran out.
    private func send(_ subcommand: String, landed: (@MainActor () -> Void)? = nil) {
        sendSequence([subcommand], landed: landed)
    }

    /// Control commands for this client, one at a time, in the order issued.
    ///
    /// Each command used to be its own Task — its own exec, racing every other
    /// one. Tapping pane A then pane B could land B's focus first and A's
    /// after it: the server stayed on A, and the next poll pulled the app back
    /// to A. One lane per client fixes the order on the server.
    @ObservationIgnored private let commandLane = WriteLane()

    /// How long one command may hold up the commands queued behind it. A
    /// healthy command is one exec; this only bites when the control channel
    /// stopped answering, and the taps after it must not wait for good. Past
    /// it the lane moves on, the late command lands whenever it does, and the
    /// poll's repeat of the server's focus heals whatever that reorders.
    static let defaultCommandBudget: Duration = .seconds(5)
    /// Per instance so tests can shorten it.
    @ObservationIgnored var commandBudget = HerdrControlClient.defaultCommandBudget

    /// Commands queued and not yet finished. The re-read after a command
    /// waits for the last one: a read between two of them reports the state
    /// the next is about to change.
    @ObservationIgnored private var commandsQueued = 0

    /// Bumped as each command is queued. A poll that was out across a bump,
    /// or began while a command was still queued, may have read the server
    /// before that command landed, and is discarded (see `poll()`).
    @ObservationIgnored private var commandEpoch = 0

    /// Wait for every command queued so far — tests drive the lane with this.
    func drainCommandsForTesting() async { await commandLane.drain() }

    // MARK: Semantic keys

    /// How long one semantic key may hold up the input queued behind it. A
    /// healthy `pane send-keys` is one exec — a channel open, a fork, one
    /// request — so this only bites when the control channel has stopped
    /// answering (a half-open socket, a sidecar that died in a roam). Typing
    /// must not freeze behind that: past the budget the lane moves on.
    static let defaultKeySendBudget: Duration = .seconds(3)
    /// Per instance so tests can shorten it.
    @ObservationIgnored var keySendBudget = HerdrControlClient.defaultKeySendBudget

    /// The lane keys ride when the caller has none to share — still one
    /// exec at a time, in order, so arrows can't overtake each other.
    @ObservationIgnored private let ownKeyLane = WriteLane()

    /// Send a SEMANTIC key to a pane — herdr encodes it server-side against
    /// the pane's REAL terminal modes. This exists because the client cannot
    /// know those modes over herdr: arrow variants (`ESC[A` vs `ESC OA`,
    /// DECCKM) are chosen from the LOCAL emulator's state everywhere else,
    /// but the frame channel feeds that emulator absolutely-positioned
    /// repaints that never carry mode changes — so the flag never flips, the
    /// app keeps sending the normal-mode variant, and zsh widgets bound via
    /// terminfo (`kcuu1` IS the application-mode sequence) never fire: the
    /// "no up/down history under herdr" report. `pane send-keys up` recalls
    /// history against the live server (verified, both directions), because
    /// the encoder that honors application_cursor lives where the truth is.
    ///
    /// The key is an exec while the typing around it rides the terminal
    /// channel, so ORDER is the caller's lane: pass the lane this session's
    /// typed bytes go down and the key lands after everything already on it
    /// and before anything added later. herdr has no semantic-key message
    /// on the frame channel (`terminal session control` takes input, resize,
    /// scroll, mouse and release — herdr 0.9.5), so one channel can't do it.
    /// Each key used to be its own unstructured exec: quick ↑↓ reordered
    /// among themselves, and ↑ then fast typing could land the text first.
    ///
    /// `landed`, when given, runs before the exec — for a lane whose writes
    /// are only SENT when their step finishes, not yet applied (mosh: the
    /// datagram left, the server may not have it).
    func sendKey(_ key: String, paneId: String, on lane: WriteLane? = nil,
                 after landed: (@MainActor () async -> Void)? = nil) {
        let queue = lane ?? ownKeyLane
        queue.enqueueKey(key, paneId: paneId, holdingAtMost: keySendBudget) { [weak self] keys, paneId in
            await landed?()
            await self?.sendKeys(keys, paneId: paneId)
        }
    }

    /// `pane send-keys` for a run of keys, in one exec. Returns once herdr has
    /// answered — by then it has written the keys into the pane, so whatever
    /// is sent next lands after them. No trailing snapshot poll: a poll per
    /// keypress is churn with nothing to learn, since a cursor move changes
    /// no tree state the sheets care about.
    func sendKeys(_ keys: [String], paneId: String) async {
        guard !keys.isEmpty else { return }
        let names = keys.map(HerdrLaunch.quote).joined(separator: " ")
        _ = try? await run("pane send-keys \(HerdrLaunch.quote(paneId)) \(names)")
    }

    // MARK: Immersive zoom (mosh renderer)

    /// mosh+herdr's answer to mosh+tmux's `ensureImmersiveZoom`. Over mosh
    /// the renderer is herdr's own TUI, one shared screen — the phone can't
    /// swap views the way the SSH frame channel does, so "one pane,
    /// full-screen" has to be produced SERVER-side, exactly like the tmux
    /// path zooms the selected pane. `pane zoom --pane <id> --on` is a
    /// better primitive than tmux's: it is an idempotent SET (no blind
    /// toggle), it moves focus in the same call (verified live: zooming B
    /// while A is zoomed transfers both in one step), and it sidesteps
    /// `agent focus`'s agent_not_found noise entirely.
    ///
    /// Same shared-state tradeoff as mosh+tmux: zoom is server state, so a
    /// desktop TUI on the same workspace sees the zoom too (`prefix z`
    /// brings the splits back, there as here). Never set on the SSH path —
    /// native frame rendering swaps views client-side and needs no zoom.
    var immersiveZoom = false

    /// Zoom whatever pane the NEXT successful snapshot says is focused.
    /// Covers the moves whose target pane this client doesn't know yet: the
    /// initial attach, and tab/workspace switches (herdr focuses that tab's
    /// own focused pane, which only the follow-up snapshot names).
    private var zoomAfterNextApply = false

    /// Arm the initial zoom — called by the hub once the mosh renderer has
    /// herdr's TUI on screen, so the first thing the user sees is the
    /// focused pane full-screen rather than the stacked mobile layout.
    func requestImmersiveZoom() {
        zoomAfterNextApply = true
        quicken()
    }

    private func sendImmersiveZoom(_ paneId: String, landed: (@MainActor () -> Void)? = nil) {
        // stderr dropped and rc forced for the same reason `agent focus`
        // does it: a pane that vanished mid-flight is a stale tap, not a
        // broken channel, and the next poll self-heals the tree anyway.
        send("pane zoom --pane \(HerdrLaunch.quote(paneId)) --on 2>/dev/null || true", landed: landed)
    }

    /// A pane switch this client sent that may not have reached the server
    /// yet: from ``selectPane(_:)`` until its focus command has finished, or
    /// the lane has stopped waiting for it (``commandBudget``). Until then the
    /// server's focus — where typing into herdr's own TUI goes — can still be
    /// the pane being left. Polls and other commands never set it.
    @ObservationIgnored private(set) var paneSwitchInFlight = false
    /// Which `selectPane` the flag stands for: only the newest one's command
    /// lowers it.
    @ObservationIgnored private var paneSwitchGeneration = 0

    func selectSession(_ sessionId: String) {
        if immersiveZoom { zoomAfterNextApply = true }
        send("workspace focus \(HerdrLaunch.quote(sessionId))")
    }

    func selectWindow(_ windowId: String) {
        if immersiveZoom { zoomAfterNextApply = true }
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
        paneSwitchGeneration &+= 1
        let generation = paneSwitchGeneration
        paneSwitchInFlight = true
        let landed: @MainActor () -> Void = { [weak self] in
            guard let self, paneSwitchGeneration == generation else { return }
            paneSwitchInFlight = false
        }
        snapshot.activePaneId = paneId
        onPanePicked?(paneId)
        onFocusedPaneChanged?(paneId)
        if immersiveZoom {
            // Focus AND fill the mosh screen in one idempotent command —
            // see `immersiveZoom` for why this replaces `agent focus` here.
            sendImmersiveZoom(paneId, landed: landed)
        } else {
            // Two rungs in one exec. `agent focus` is the fast path — but on
            // current herdr it FAILS OUTRIGHT on a plain shell pane: the old
            // "prints agent_not_found but still moves the focus" behavior is
            // GONE (verified live: focused_pane_id does not move), which is
            // exactly the "two panes in a tab, can't switch to the shell one"
            // report. Rung two is a zoom bounce: `pane zoom --on` moves focus
            // as part of zooming and `--off` restores the layout while the
            // focus STAYS on the target (both verified live). The socket's
            // id-based `pane.focus` was considered and rejected: on 0.8.0's
            // headless server it acts but never replies, so every call would
            // eat a full client timeout.
            let target = HerdrLaunch.quote(paneId)
            let herdr = HerdrLaunch.attachCommand(customPath: customPath)
            send("agent focus \(target) 2>/dev/null || { "
                + "\(herdr) pane zoom --pane \(target) --on >/dev/null 2>&1; "
                + "\(herdr) pane zoom --pane \(target) --off >/dev/null 2>&1; }",
                 landed: landed)
        }
    }

    /// The pane the SERVER has focused, read once every command queued so
    /// far has landed. ``selectPane(_:)`` writes its target into ``snapshot``
    /// the moment it is called — that is what makes a tap feel instant — so
    /// the snapshot can't confirm a switch; this can. nil when the read fails
    /// or doesn't come back within `budget`. Nothing is applied from it: the
    /// re-read after the commands does that.
    func confirmedFocus(within budget: Duration) async -> String? {
        let (answered, result) = await runOnLane("api snapshot", mutation: false, within: budget)
        guard answered, let result, let decoded = HerdrSnapshot.decode(result.output) else { return nil }
        return decoded.snapshot.activePaneId
    }

    /// Create a session — or, on a host with no server at all, start one.
    ///
    /// Both come from the same explicit user action (the empty state's button),
    /// which is the only thing that may bring a session into existence. A
    /// freshly started server arrives with its own first workspace, so there's
    /// nothing to create on top of it.
    func newSession(named name: String?) {
        guard !serverNotRunning else {
            // The server's first workspace is the one asked for, name and all
            // (B192: the bootstrap made it unnamed).
            Task { [weak self] in await self?.bootstrapServer(label: name) }
            return
        }
        send(Self.workspaceCreate(label: name))
    }

    /// `workspace create --focus`, with the label when there is one.
    static func workspaceCreate(label: String?) -> String {
        var command = "workspace create --focus"
        if let label, !label.isEmpty { command += " --label \(HerdrLaunch.quote(label))" }
        return command
    }

    /// The protocol_mismatch remedy, on the user's explicit tap — never
    /// automatic, because `server stop` exits pane processes and only 0.8+
    /// restores sessions (the skew case is by definition an OLD server, so
    /// the kill is real; herdr's own error text prescribes exactly this).
    /// `server stop` is exempt from the version gate — the same message that
    /// reports the mismatch instructs running it.
    ///
    /// Asks `herdr status` first, because the restart runs whatever `herdr`
    /// our commands resolve to — and when THAT is the older side, "restart"
    /// means downgrading the server and exiting every pane for nothing (a
    /// stale `~/.local/bin/herdr` 0.8.0 shadowing a 0.9.5 server, 2026-11-11).
    /// A report that can't be read (no `status` on an old herdr) doesn't
    /// block: that was the behaviour before the check existed.
    ///
    /// Every step is bounded (B190): on a link that stopped answering none of
    /// them returned, and the banner said "Restarting herdr server…" for good.
    func restartServer() async -> RestartOutcome {
        isRestartingServer = true
        defer { isRestartingServer = false }
        // Read now, right before `server stop`, not taken from an earlier
        // read. An unreadable report (an old herdr without `status`, or a
        // stopped server, which names no version) doesn't block.
        if let report = await readSkew() {
            if report.clientIsOlder { return .wouldDowngrade(report) }
            // Already as new as the host's herdr (fixed outside the app since
            // the banner went up): stopping it would only exit every pane's
            // process (B408).
            if !report.mismatched { return .unneeded }
        }
        let began = readsApplied
        await runServerCommand(HerdrLaunch.command("server stop", customPath: customPath))
        try? await Task.sleep(for: restartPause)
        await bootstrapServer()
        // Restarted takes evidence: a read of the server since the restart
        // began. Without one, all there is to go by is the state from before.
        guard readsApplied > began else { return .failed }
        return !serverNotRunning && !lastPollMismatched ? .restarted : .failed
    }

    /// A server command (`server stop`, the daemon start), waited for at most
    /// ``serverCommandBudget``.
    private func runServerCommand(_ command: String) async {
        let runner = self.runner
        await WriteLane.wait(atMost: serverCommandBudget) {
            _ = try? await runner.run(command)
        }
    }

    enum RestartOutcome: Equatable {
        case restarted
        case failed
        /// Refused: the `herdr` we would restart with is older than the
        /// running server. The remedy is upgrading the host's herdr.
        case wouldDowngrade(HerdrVersionReport)
        /// Nothing done: the server already runs the host's herdr (B408).
        case unneeded
    }

    /// Which `herdr` our commands run and which server is up, per
    /// `herdr status` — kept only while the two disagree. Observable so the
    /// Home row can drop its restart button when restarting would downgrade.
    private(set) var versionSkew: HerdrVersionReport?

    /// Ask the host. nil when the versions match or can't be read.
    @discardableResult
    func diagnoseVersionSkew() async -> HerdrVersionReport? {
        guard await readSkew() != nil else { return nil }
        return versionSkew
    }

    /// `herdr status`'s two versions, matched or not (nil when it can't be
    /// read within ``statusBudget``), noted in ``versionSkew``.
    private func readSkew() async -> HerdrVersionReport? {
        guard let result = await read("status", within: statusBudget),
              let report = HerdrVersionReport.parse(result.output) else { return nil }
        versionSkew = report.mismatched ? report : nil
        return report
    }

    /// Whether the most recent poll came back as a protocol mismatch —
    /// restartServer()'s success check.
    @ObservationIgnored private var lastPollMismatched = false

    /// The mismatch as OBSERVABLE state, for surfaces with no terminal
    /// banner — the Home card's workspace tree said "No workspaces yet"
    /// while every command was being refused over version skew, and its
    /// New Workspace sheet failed silently (user report, 2026-08-17).
    /// nil while healthy; the message while refused.
    private(set) var protocolMismatch: String?

    /// True while restartServer() is in flight — drives the Home row's
    /// spinner so a tap doesn't look ignored.
    private(set) var isRestartingServer = false

    /// Start the headless server, wait for it to answer, then make sure there
    /// is actually a workspace to attach to.
    ///
    /// The second step is not optional. A server started with no persisted
    /// state comes up **empty** — zero workspaces, zero panes. (It looks like
    /// it brings its own only when it restores a previous session from
    /// `~/.config/herdr/session.json`.) Without the create, the user taps
    /// "create a session", a server appears, and the empty state stays put.
    ///
    /// The create takes its turn on the command lane like every command that
    /// moves the focus. Run beside it, a tap made while the workspace was
    /// being made landed first and was overridden by the create's `--focus`,
    /// and a poll read while it was out was applied as if nothing were
    /// pending — the empty state coming back over the new workspace.
    private func bootstrapServer(label: String? = nil) async {
        let began = readsApplied
        await runServerCommand(HerdrLaunch.daemonCommand(customPath: customPath))
        for _ in 0..<10 {
            try? await Task.sleep(for: bootstrapInterval)
            await poll()
            // A read that never answered says nothing about the new server.
            guard readsApplied > began, !serverNotRunning else { continue }
            if snapshot.sessions.isEmpty {
                // Bounded like the steps before it: the lane stops holding
                // the create after its budget, and its answer was waited for
                // forever.
                _ = await runOnLane(Self.workspaceCreate(label: label), within: bootstrapCreateBudget)
                // Read once the lane has let go of the create — a read that
                // starts while it still counts as queued is thrown away — so
                // the answer is in before the caller (`restartServer`)
                // judges the outcome by it.
                await commandLane.drain()
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

    /// Session-row "New tab" on Home: focus `sessionId`'s workspace, then
    /// create — one lane step, so a failed focus stops the create.
    func newWindow(inSession sessionId: String, named name: String?) {
        var create = "tab create --focus"
        if let name, !name.isEmpty { create += " --label \(HerdrLaunch.quote(name))" }
        var steps: [String] = []
        if sessionId != snapshot.activeSessionId {
            steps.append("workspace focus \(HerdrLaunch.quote(sessionId))")
        }
        steps.append(create)
        sendSequence(steps)
    }

    /// Window-row "New pane" on Home: `pane split` takes a pane id, so the
    /// target tab's own active pane is split whether or not it is on screen;
    /// focusing its workspace and tab first is what lands the user there
    /// (the frame channel follows focus).
    func newPane(inWindow windowId: String) {
        let panes = snapshot.panes(inWindow: windowId)
        guard let window = snapshot.windows[windowId],
              let target = panes.first(where: \.isActive) ?? panes.first else { return }
        var steps: [String] = []
        if window.sessionId != snapshot.activeSessionId {
            steps.append("workspace focus \(HerdrLaunch.quote(window.sessionId))")
        }
        if windowId != snapshot.activeWindowId {
            steps.append("tab focus \(HerdrLaunch.quote(windowId))")
        }
        steps.append("pane split \(HerdrLaunch.quote(target.id)) --direction right --focus")
        sendSequence(steps)
    }

    /// `send` for several commands that must run in this order: one step on
    /// ``commandLane``, stopping at the first step not known done, then one
    /// re-read once nothing else is queued behind it.
    private func sendSequence(_ subcommands: [String], landed: (@MainActor () -> Void)? = nil) {
        enqueueCommand({ [weak self] hold in await self?.runInOrder(subcommands, hold: hold) }, landed: landed)
    }

    /// The lane's hold on the step it is running: released once the lane
    /// stops waiting for it — the step finished, or ``commandBudget`` ran
    /// out and the lane moved on to the steps queued behind it.
    @MainActor final class LaneHold {
        fileprivate(set) var released = false
    }

    /// A caller's claim on a command it queued on the lane and waits for:
    /// abandoned once the caller stops waiting, and a command still queued
    /// then is never sent (B191).
    @MainActor final class LaneTicket {
        fileprivate(set) var abandoned = false
    }

    /// Queue `step` on ``commandLane`` with what every command that changes
    /// the server carries: a read out across it is dropped (`commandEpoch`),
    /// the lane waits for it at most ``commandBudget``, and one re-read
    /// follows once nothing else is queued. `landed` runs when the lane is
    /// done waiting for it. The step is handed the lane's hold on it, so
    /// that it can tell once the lane has let it go.
    private func enqueueCommand(_ step: @escaping @MainActor (LaneHold) async -> Void,
                                landed: (@MainActor () -> Void)? = nil) {
        commandEpoch &+= 1
        commandsQueued += 1
        commandLane.enqueue { [weak self] in
            let hold = LaneHold()
            guard let self else { await step(hold); hold.released = true; landed?(); return }
            await WriteLane.wait(atMost: commandBudget) { await step(hold) }
            hold.released = true
            landed?()
            commandsQueued -= 1
            guard commandsQueued == 0 else { return }
            // Off the lane: the re-read is for the UI, and a tap made while it
            // is out must not wait behind it. If one is made, this read is
            // stale before it lands and `poll()` drops it.
            Task { [weak self] in await self?.poll() }
        }
    }

    /// One command on ``commandLane``, its result handed back — for a caller
    /// that needs the answer and not just the re-read (`worktree create`
    /// names the pane the agent goes into). `mutation: false` is a read that
    /// rides the lane only for its place in line: no epoch bump, no re-read.
    /// The lane waits for it at most ``commandBudget``; the caller waits for
    /// the answer at most `patience`.
    ///
    /// `answered` is false once the patience ran out. A command still queued
    /// then is taken off the lane: it is never sent (B191). It used to run
    /// when its turn came anyway — a worktree made, focus and all, after the
    /// form had said none was. One already sent can't be unsent; its answer
    /// is dropped. `result` is nil when the command couldn't run.
    private func runOnLane(_ subcommand: String, mutation: Bool = true,
                           within patience: Duration) async -> (answered: Bool, result: CommandResult?) {
        let ticket = LaneTicket()
        var result: CommandResult?
        let answered = await WriteLane.wait(atMost: patience) { [weak self] in
            result = await self?.queueOnLane(subcommand, mutation: mutation, ticket: ticket)
        }
        if !answered { ticket.abandoned = true }
        return (answered, answered ? result : nil)
    }

    /// ``runOnLane(_:mutation:within:)``'s step: nil, and nothing sent, for
    /// a ticket abandoned before the command's turn came.
    private func queueOnLane(_ subcommand: String, mutation: Bool,
                             ticket: LaneTicket) async -> CommandResult? {
        await withCheckedContinuation { (continuation: CheckedContinuation<CommandResult?, Never>) in
            let step: @MainActor () async -> Void = { [weak self] in
                guard !ticket.abandoned else {
                    Log.ssh.info("herdr command dropped, its caller gave up: \(subcommand, privacy: .public)")
                    continuation.resume(returning: nil)
                    return
                }
                continuation.resume(returning: try? await self?.run(subcommand))
            }
            if mutation {
                enqueueCommand({ _ in await step() })
            } else {
                commandLane.enqueue { [weak self] in
                    await WriteLane.wait(atMost: self?.commandBudget ?? HerdrControlClient.defaultCommandBudget,
                                         for: step)
                }
            }
        }
    }

    /// Each command once the one before it is known done (B121: a lost
    /// exit-status trailer with no herdr result is not done, and used to
    /// count as success — `workspace focus` cut off, then `tab create
    /// --focus` in whichever workspace was focused). Nothing more once the
    /// lane has let the sequence go (B191): the rest would land after the
    /// commands queued behind it — a new tab taking the focus from the pane
    /// the user tapped after asking for it.
    private func runInOrder(_ subcommands: [String], hold: LaneHold) async {
        for subcommand in subcommands {
            guard !hold.released else {
                Log.ssh.error("herdr sequence stopped, the lane moved on before: \(subcommand, privacy: .public)")
                return
            }
            let result = try? await run(subcommand)
            if let reason = Self.refusal(result) {
                Log.ssh.error("herdr command failed: \(subcommand, privacy: .public): \(reason, privacy: .public)")
                return
            }
        }
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

    // MARK: - Mirrored from the Rust core

    /// herdr's state as the Rust core read it (``RustHerdrController``,
    /// which runs no poll of its own), where the chrome reads it. Only what
    /// changed is written: every write is an Observation change.
    func mirrorRust(snapshot new: TmuxSnapshot, agentHooks: [String: AgentHook],
                    agentHooksLoaded: Bool, isRefreshing: Bool, serverNotRunning: Bool,
                    protocolMismatch: String?, versionSkew: HerdrVersionReport?,
                    isRestartingServer: Bool, pushActive: Bool, paneSwitchInFlight: Bool,
                    paneCwds: [String: String], worktreeRepos: [String: String]) {
        if snapshot != new { snapshot = new }
        if self.agentHooks != agentHooks { self.agentHooks = agentHooks }
        if self.agentHooksLoaded != agentHooksLoaded { self.agentHooksLoaded = agentHooksLoaded }
        if self.isRefreshing != isRefreshing { self.isRefreshing = isRefreshing }
        if self.serverNotRunning != serverNotRunning { self.serverNotRunning = serverNotRunning }
        if self.protocolMismatch != protocolMismatch { self.protocolMismatch = protocolMismatch }
        if self.versionSkew != versionSkew { self.versionSkew = versionSkew }
        if self.isRestartingServer != isRestartingServer { self.isRestartingServer = isRestartingServer }
        if self.pushActive != pushActive { self.pushActive = pushActive }
        self.paneSwitchInFlight = paneSwitchInFlight
        self.paneCwds = paneCwds
        if self.worktreeRepos != worktreeRepos { self.worktreeRepos = worktreeRepos }
    }
}
