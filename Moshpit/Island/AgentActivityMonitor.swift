import Foundation
import Observation
import UserNotifications
#if canImport(ActivityKit)
import ActivityKit
#endif
#if canImport(WidgetKit)
import WidgetKit
#endif

/// Vibe Island — iOS adaptation of github.com/Octane0411/open-vibe-island.
///
/// Watches live tmux panes for coding-agent activity and mirrors them into a
/// SINGLE aggregate Dynamic Island Live Activity + local notifications:
///   * pane output flowing (non-shell foreground) → `.working`
///   * terminal bell (BEL)                         → `.attention` + notification
///   * quiet for `idleAfter` after working         → `.done`
///   * `.done` for `doneLinger`                    → removed
///
/// Phase A uses this output-flow heuristic (no remote install). A pane only
/// becomes a row once a non-shell foreground process has been active for
/// `promotionDelay` — so a quick `ls` never flickers a card and idle shells
/// never appear. Phase B will replace the heuristic with hook events stamped
/// onto tmux pane options (see project_vibe_island).
///
/// Tapping the island deep-links to the most-urgent agent's pane via
/// `moshpit://connection/<uuid>?pane=<paneId>`.
@MainActor
@Observable
final class AgentActivityMonitor {

    // MARK: Tunables
    private let idleAfter: TimeInterval = 4      // working → done after this much quiet
    private let promotionDelay: TimeInterval = 2 // sustained non-shell before a row appears
    private let doneLinger: TimeInterval = 25    // keep a finished agent visible this long
    private let maxAgents = 4                    // rows carried in the ContentState
    // Phase B — a hook-owned pane that stops appearing in the poll for longer
    // than this reverts to the output heuristic. The sweep runs every 2s and
    // polls each sweep, so ~2 poll intervals of slack avoids flapping on a
    // single dropped round-trip.
    private let hookGrace: TimeInterval = 5

    /// Agent labels that are Moshpit's own plumbing, never a row on the island.
    ///
    /// A pairing self-test stamps a pane exactly as a real hook would — that is
    /// what makes it a real proof — so without this the act of verifying an
    /// install would flash a phantom agent for the length of a sweep.
    private static let reservedAgents: Set<String> = [PushRemoteNotification.selfTestAgent]

    /// Foreground commands that are NOT agents — never promoted to a row.
    private static let shells: Set<String> = [
        "zsh", "bash", "sh", "fish", "dash", "ksh", "tmux",
        "login", "-zsh", "-bash", "-sh", "ssh", "mosh", ""
    ]

    /// Foreground commands the OUTPUT HEURISTIC may promote to an island row.
    /// Anything else producing output (builds, tails, servers, vim…) stays off
    /// the island — promoting every busy pane buried the actual agents in
    /// meaningless rows. Mirrors the hook installer's agent list (the hook path
    /// is authoritative and unaffected by this filter; a stamped pane always
    /// shows). BELs also bypass this — a bell is an explicit signal.
    private static let agentCommands: Set<String> = [
        "claude", "codex", "gemini", "qwen", "qoder", "factory", "codebuddy",
        "aider", "goose", "opencode",
    ]

    // MARK: State
    private struct ConnRef {
        let connection: ServerConnection
        /// Either multiplexer's control surface. The Island only needs the
        /// tree and the agent stamps, both of which are on the protocol —
        /// tmux's output/bell heuristics are wired separately in `track`
        /// because only tmux has them.
        weak var controller: (any MultiplexerControlling)?
    }
    private struct PaneKey: Hashable { let conn: UUID; let pane: String }
    private struct PaneActivity {
        let conn: UUID
        let paneId: String
        var command: String
        var location: String
        /// What the agent is doing/asking — the hook's `@moshpit_title`. nil on the
        /// output heuristic (no precise content) and once a turn ends (cleared).
        var detail: String? = nil
        var state: AgentActivityAttributes.AgentState
        var stateSince: Date     // entry into the current state — drives the timer
        var lastOutput: Date
        var firstActive: Date    // when sustained non-shell activity began
        var visible: Bool        // promoted into the Live Activity?
        // Phase B — hook bridge. A pane whose `@moshpit_state` is set is governed
        // by the precise hook stamp, not the output heuristic. `hookOwned` makes
        // the hook authoritative; `lastHookSeen` is when the poll last carried a
        // state for it, so the heuristic can resume cleanly when the option is
        // unset (the pane drops out of the poll for ~2 intervals).
        var hookOwned: Bool = false
        var lastHookSeen: Date = .distantPast
    }

    private var conns: [UUID: ConnRef] = [:]
    private var panes: [PaneKey: PaneActivity] = [:]

    /// When each pane's most recently ANNOUNCED attention episode began.
    ///
    /// This exists because the thing it replaced was not a record of anything.
    /// The old test was `!(wasAttention && wasHookOwned)`, and `wasHookOwned` is
    /// `hookGoverns` — a five-second liveness window. Any gap longer than that in
    /// hook polling (a reconnect, a spell in the background, a slow host) made the
    /// sweep drop `hookOwned`, while `.attention` deliberately sticks. The next
    /// poll then saw an unchanged prompt as a new one and announced it again, once
    /// per reconnect, for as long as the agent kept waiting.
    /// Persisted, because an app RELAUNCH has the same problem a reconnect does.
    /// A device log caught four notifications added in one millisecond right after
    /// a fresh launch — every pane still standing in `attention`, re-announced,
    /// one of them re-added two seconds after being withdrawn. In-memory alone
    /// would have fixed the reconnect the user reported and left that.
    private var announcedEpisode: [PaneKey: Date] = [:]

    private static let announcedKey = "island.announcedEpisodes"

    /// Entries older than this are dropped on load: a pane whose prompt has stood
    /// untouched for a day is not one this record needs to keep suppressing, and
    /// unbounded growth in UserDefaults is its own bug.
    private static let announcedTTL: TimeInterval = 24 * 60 * 60

    /// `"<uuid>|<pane>" -> epoch seconds`, flat so it survives as a plist.
    static func encodeAnnounced(_ v: [String: Date]) -> [String: Double] {
        v.mapValues { $0.timeIntervalSince1970 }
    }

    static func decodeAnnounced(_ raw: [String: Double], now: Date,
                                ttl: TimeInterval) -> [String: Date] {
        raw.compactMapValues { seconds in
            let date = Date(timeIntervalSince1970: seconds)
            // A future timestamp means a clock moved, not a real episode; drop it
            // rather than let it suppress notifications until the clock catches up.
            guard date <= now.addingTimeInterval(60),
                  now.timeIntervalSince(date) <= ttl else { return nil }
            return date
        }
    }

    private func loadAnnounced() {
        let raw = settings.store.dictionary(forKey: Self.announcedKey) as? [String: Double] ?? [:]
        let kept = Self.decodeAnnounced(raw, now: Date(), ttl: Self.announcedTTL)
        announcedEpisode = kept.reduce(into: [:]) { out, pair in
            let parts = pair.key.split(separator: "|", maxSplits: 1)
            guard parts.count == 2, let conn = UUID(uuidString: String(parts[0])) else { return }
            out[PaneKey(conn: conn, pane: String(parts[1]))] = pair.value
        }
    }

    private func saveAnnounced() {
        let flat = announcedEpisode.reduce(into: [String: Date]()) { out, pair in
            out["\(pair.key.conn.uuidString)|\(pair.key.pane)"] = pair.value
        }
        settings.store.set(Self.encodeAnnounced(flat), forKey: Self.announcedKey)
    }
    /// Attention notifications we have DELIVERED, so leaving the attention
    /// state can revoke them — a stale "needs you" on the lock screen invites
    /// answering a prompt that no longer exists (possibly approving a newer,
    /// more dangerous one).
    @ObservationIgnored private var postedAttention: Set<PaneKey> = []

    /// How long a prompt must STAND before the phone hears about it. A question
    /// answered at the desk inside this window never interrupts anyone — which
    /// is most of them, most days. The cost is honesty about the tail: when the
    /// user really is away, they learn about the wait this much later.
    ///
    /// `-MOSHPIT_ANNOUNCE_GRACE <seconds>` overrides it so the device harness
    /// does not spend half a minute per phase watching a timer.
    nonisolated static let announceGrace: TimeInterval = {
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: "-MOSHPIT_ANNOUNCE_GRACE"), i + 1 < args.count,
           let v = TimeInterval(args[i + 1]), v >= 0 {
            return v
        }
        return 30
    }()

    private struct PendingAnnounce { var episode: Date; var task: Task<Void, Never> }
    @ObservationIgnored private var pendingAnnounce: [PaneKey: PendingAnnounce] = [:]

    /// Wait out the grace, re-check that the SAME question is still standing,
    /// and only then announce. The re-check is the point: a prompt answered at
    /// the desk leaves `attention` (or starts a new episode) inside the window,
    /// and the scheduled announcement evaporates without a trace.
    private func scheduleAnnounce(key: PaneKey, episode: Date,
                                  location: String, command: String, detail: String?) {
        if let pending = pendingAnnounce[key], pending.episode == episode { return }
        pendingAnnounce[key]?.task.cancel()
        Log.island.info("announce scheduled: pane \(key.pane, privacy: .public) grace \(Self.announceGrace, privacy: .public)s")
        let task = Task { [weak self] in
            try? await Task.sleep(for: .seconds(Self.announceGrace))
            guard !Task.isCancelled, let self else { return }
            self.pendingAnnounce[key] = nil
            guard let p = self.panes[key], p.state == .attention,
                  p.stateSince == episode else {
                // The window did its job: the question this was scheduled for is
                // gone (answered at the desk) or superseded. Logged, because a
                // silently swallowed announcement and a scheduling bug look
                // identical from a phone.
                Log.island.info("announce cancelled: pane \(key.pane, privacy: .public) left the episode within the grace")
                return
            }
            self.announcedEpisode[key] = episode
            self.saveAnnounced()
            self.postAttention(connectionId: key.conn, paneId: key.pane,
                               location: p.location, command: p.command,
                               detail: self.settings.lockScreenDetailEnabled ? p.detail : nil,
                               episode: episode)
        }
        pendingAnnounce[key] = PendingAnnounce(episode: episode, task: task)
    }
    @ObservationIgnored private var sweepTimer: Timer?
    @ObservationIgnored private let settings: AppSettings
    #if canImport(ActivityKit)
    @ObservationIgnored private var activity: Activity<AgentActivityAttributes>?
    #endif

    init(settings: AppSettings) {
        self.settings = settings
        loadAnnounced()
    }

    // MARK: - Lifecycle

    /// Track a connection's multiplexer, whichever it is.
    ///
    /// The output/bell heuristics are tmux-only, and deliberately so: they
    /// exist to GUESS at agent state from a pane's byte stream when no hook is
    /// installed. herdr reports `agent_status` itself, so on herdr the guess
    /// would be a strictly worse second opinion competing with the truth.
    func track<C: MultiplexerControlling>(connection: ServerConnection, controller: C) {
        guard settings.liveActivityEnabled || settings.notificationsEnabled else { return }

        conns[connection.id] = ConnRef(connection: connection, controller: controller)
        if let tmux = controller as? TmuxSessionController {
            tmux.onPaneActivity = { [weak self] paneId in
                self?.noteOutput(connectionId: connection.id, paneId: paneId)
            }
            tmux.onPaneBell = { [weak self] paneId in
                self?.noteBell(connectionId: connection.id, paneId: paneId)
            }
        }
        // Re-sync the island the instant fresh agent state lands, instead of
        // waiting for the next sweep tick.
        controller.onAgentHooksUpdated = { [weak self] in
            self?.applyAgentHooks(connectionId: connection.id)
        }
        startSweepTimerIfNeeded()
        requestNotificationAuthorizationIfNeeded()
    }

    /// Track a connection once its multiplexer control plane exists.
    ///
    /// Views used to sample `session.tmuxController` right after
    /// `hub.start()` returned and wire nothing when it was nil. Over mosh
    /// that is always: both control planes come up in detached tasks seconds
    /// later, so mosh+tmux never fed the island at all — no Live Activity,
    /// no lock-screen alerts, an empty Agents section — while the same
    /// connection over SSH lit up fine. Waiting on the session makes the
    /// wiring transport-agnostic; `track` still applies the tmux-vs-herdr
    /// distinction (heuristics vs native `agent_status`) once the concrete
    /// controller is known.
    func trackWhenReady(connection: ServerConnection, session: SessionHub.ActiveSession) {
        guard settings.liveActivityEnabled || settings.notificationsEnabled else { return }
        // Subscribe to every controller this session will ever wire, not just
        // the first: a mid-session reconnect rebuilds the controller, and the
        // weak ref `track` stores went nil with the old one — hook polling
        // silently stopped and no prompt after a reconnect ever announced.
        // `track` is idempotent (it overwrites the ref and rewires callbacks),
        // so re-tracking the same controller is harmless.
        session.onMultiplexerControlChanged = { [weak self] controller in
            self?.track(connection: connection, controller: controller)
        }
        Task { [weak self] in
            guard let controller = await session.awaitMultiplexerControl() else { return }
            self?.track(connection: connection, controller: controller)
        }
    }

    func untrack(connectionId: UUID) {
        conns[connectionId] = nil
        panes = panes.filter { $0.key.conn != connectionId }
        for (key, pending) in pendingAnnounce where key.conn == connectionId {
            pending.task.cancel()
            pendingAnnounce[key] = nil
        }
        if conns.isEmpty {
            sweepTimer?.invalidate()
            sweepTimer = nil
        }
        sync()
    }

    // MARK: - Signals

    private func noteOutput(connectionId: UUID, paneId: String) {
        let key = PaneKey(conn: connectionId, pane: paneId)
        let now = Date()

        // Phase B precedence — a pane the hook currently governs is authoritative;
        // the output heuristic must not fight it. Track lastOutput for the
        // fallback path but never touch its state/visibility.
        if var p = panes[key], hookGoverns(p, now: now) {
            p.lastOutput = now
            panes[key] = p
            return
        }

        let cmd = command(connectionId: connectionId, paneId: paneId)
        let loc = location(connectionId: connectionId, paneId: paneId)

        // Shell foreground = not an agent. If a visible agent just dropped back
        // to its shell, it has finished its turn → mark done; otherwise ignore.
        if Self.shells.contains(cmd) {
            if var existing = panes[key], existing.visible, existing.state == .working {
                existing.state = .done
                existing.stateSince = now
                panes[key] = existing
                sync()
            }
            return
        }

        let isKnownAgent = Self.agentCommands.contains(cmd)
        if var p = panes[key] {
            p.command = cmd
            p.location = loc
            p.lastOutput = now
            if !p.visible {
                // Promotion is agent-only; other busy panes (builds, tails,
                // servers) never earn a row from mere output.
                if isKnownAgent, now.timeIntervalSince(p.firstActive) >= promotionDelay {
                    p.visible = true
                    p.state = .working
                    p.stateSince = now
                }
            } else if p.state != .working {
                p.state = .working
                p.stateSince = now
            }
            panes[key] = p
        } else if isKnownAgent {
            panes[key] = PaneActivity(
                conn: connectionId, paneId: paneId,
                command: cmd, location: loc,
                state: .working, stateSince: now,
                lastOutput: now, firstActive: now, visible: false)
        } else {
            return   // not an agent, nothing visible to age out — no bookkeeping
        }
        sync()
    }

    private func noteBell(connectionId: UUID, paneId: String) {
        let key = PaneKey(conn: connectionId, pane: paneId)
        let now = Date()

        // Phase B precedence — if a hook governs this pane, its precise state
        // (e.g. still `working`) wins over a stray BEL. The hook's own
        // `.attention` path posts the notification instead.
        if let p = panes[key], hookGoverns(p, now: now) { return }

        let cmd = command(connectionId: connectionId, paneId: paneId)
        let loc = location(connectionId: connectionId, paneId: paneId)

        // A bell always matters — surface immediately, even for a shell prompt.
        var p = panes[key] ?? PaneActivity(
            conn: connectionId, paneId: paneId,
            command: cmd, location: loc,
            state: .attention, stateSince: now,
            lastOutput: now, firstActive: now, visible: true)
        p.command = Self.shells.contains(cmd) ? p.command : cmd
        p.location = loc
        p.state = .attention
        p.stateSince = now
        p.visible = true
        panes[key] = p

        // Through the same grace and bookkeeping as the hook path. A bell sets
        // a fresh `stateSince`, so a genuinely new bell still announces; the
        // grace re-check quietly swallows one the user handled at the desk.
        if Self.shouldAnnounce(state: .attention, episode: p.stateSince,
                               lastAnnounced: announcedEpisode[key]) {
            scheduleAnnounce(key: key, episode: p.stateSince,
                             location: loc, command: p.command, detail: nil)
        }
        sync()
    }

    /// Announce a standing prompt — as ONE summary per connection, not one card
    /// per pane. Shared by the BEL path (``noteBell``) and the hook path, both
    /// arriving through ``scheduleAnnounce``'s grace window.
    ///
    /// The sound and the Focus breach belong to the 0→1 EDGE alone: the moment
    /// "nobody is waiting on you" becomes "someone is". A second agent joining
    /// the wait updates the summary's count silently at `.passive`. The edge is
    /// read from ``PushStanding`` — the App Group set the notification service
    /// extension shares — so the push and the local path cannot both ring for
    /// one prompt: whichever lands first takes the edge.
    private func postAttention(connectionId: UUID, paneId: String,
                               location: String, command: String,
                               detail: String? = nil, episode: Date = Date()) {
        // Logged before the authorization gate, so "we decided to announce" and
        // "iOS delivered something" stay separable (the reconnect bug was only
        // diagnosable because of this line).
        Log.island.info("announcing attention: pane \(paneId, privacy: .public) on \(location, privacy: .public)")
        guard settings.notificationsEnabled else { return }
        let conn = connectionId.uuidString
        let edge = PushStanding.noteStanding(conn: conn, entry: .init(
            pane: paneId, agent: displayCommand(command), title: detail,
            location: location, since: Int(episode.timeIntervalSince1970),
            recordedAt: Date()))
        postedAttention.insert(PaneKey(conn: connectionId, pane: paneId))
        renderAttentionDigest(connectionId: connectionId, edge: edge)
    }

    /// Draw (or withdraw) the one summary card a connection's waiting prompts
    /// share. `edge` decides sound and interruption level; everything else is
    /// read from the standing set, newest first.
    private func renderAttentionDigest(connectionId: UUID, edge: Bool) {
        let conn = connectionId.uuidString
        let id = "moshpit.attention.\(conn)"
        let standing = PushStanding.standing(conn: conn)
        let center = UNUserNotificationCenter.current()
        guard let newest = standing.first else {
            center.removeDeliveredNotifications(withIdentifiers: [id])
            center.removePendingNotificationRequests(withIdentifiers: [id])
            return
        }
        let content = UNMutableNotificationContent()
        let who = newest.agent?.isEmpty == false ? newest.agent! : "agent"
        // "+N" rather than a sentence, for parity with the notification service
        // extension, which renders the pushed copy of this same card and has no
        // localization catalog of its own.
        content.title = standing.count > 1 ? "\(who) +\(standing.count - 1)" : who
        if let title = newest.title, !title.isEmpty {
            content.body = "\(title) — \(newest.location ?? "")"
        } else {
            content.body = newest.location ?? ""
        }
        content.sound = (edge && settings.attentionSoundEnabled) ? .default : nil
        // The edge is ELIGIBLE to break through Focus (see the entitlement notes
        // in docs/PUSH.md — each Focus still decides for itself). Updates are
        // `.passive`: they change a count on a card, they wake no one.
        content.interruptionLevel = edge ? .timeSensitive : .passive
        content.userInfo = [
            AgentNotifications.connectionKey: conn,
            AgentNotifications.paneKey: newest.pane
        ]
        center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil))
    }

    /// Post the "agent finished" notification. Hook-driven only — a precise
    /// `Stop` stamp flipping a pane to `.done` is a real end-of-turn signal,
    /// whereas the output heuristic's `.done` (4s of quiet) is too fuzzy to ping
    /// on. Carries the Reply action so the user can fire the next instruction
    /// from the lock screen.
    private func postDone(connectionId: UUID, paneId: String,
                          location: String, command: String,
                          duration: TimeInterval = 0) {
        guard settings.notificationsEnabled else { return }
        let content = UNMutableNotificationContent()
        content.title = String(localized: "✓ \(displayCommand(command)) finished")
        content.body = location
        // Only a LONG turn's finish makes a sound — the user walked away from a
        // build; tell them it is over. A twenty-second answer finishing is
        // information, not an interruption: silent, `.passive`, in the list for
        // whenever they next look. Same threshold as the pushed copy
        // (PushRemoteNotification.doneSoundThreshold).
        let isLong = Int(duration) >= PushRemoteNotification.doneSoundThreshold
        content.interruptionLevel = isLong ? .active : .passive
        content.sound = (isLong && settings.attentionSoundEnabled) ? .default : nil
        content.userInfo = [
            AgentNotifications.connectionKey: connectionId.uuidString,
            AgentNotifications.paneKey: paneId
        ]
        let request = UNNotificationRequest(
            identifier: "moshpit.done.\(connectionId.uuidString).\(paneId)",
            content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request)
    }

    /// True when a pane is currently governed by a fresh hook stamp — the
    /// heuristic must defer to it. `hookOwned` flips on in ``applyAgentHooks``;
    /// `lastHookSeen` keeps it authoritative for ``hookGrace`` after the last
    /// poll that carried a state, so a single dropped round-trip never lets the
    /// heuristic clobber a live agent.
    /// Whether a pane in `state`, whose current episode began at `episode`,
    /// should announce itself given what was last announced for it.
    ///
    /// Deliberately static and free of the state machine: the version this
    /// replaced was three terms inline in `applyAgentHooks`, unreachable from a
    /// test without a live tmux controller, and wrong for two years' worth of
    /// reconnects. An `episode` is a pane's `stateSince` — stable while a prompt
    /// stands, different when a new one begins — so "have I said this already"
    /// becomes a comparison rather than a guess about liveness.
    static func shouldAnnounce(state: AgentActivityAttributes.AgentState,
                               episode: Date,
                               lastAnnounced: Date?) -> Bool {
        guard state == .attention else { return false }
        return lastAnnounced != episode
    }

    private func hookGoverns(_ p: PaneActivity, now: Date) -> Bool {
        p.hookOwned && now.timeIntervalSince(p.lastHookSeen) <= hookGrace
    }

    // MARK: - Sweep

    private func startSweepTimerIfNeeded() {
        guard sweepTimer == nil else { return }
        sweepTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.sweep() }
        }
    }

    private func sweep() {
        let now = Date()

        // Phase B — drive a hook poll for every tracked controller on the same
        // cadence as the heuristic sweep. Each completion calls back into
        // applyAgentHooks (via onAgentHooksUpdated) which re-syncs the island.
        for ref in conns.values {
            ref.controller?.pollAgentHooks()
        }

        // Looking at a prompt acknowledges it. The island keeps showing the
        // factual state (the agent IS still waiting until it is answered), but
        // the lock-screen card's job — get you to the pane — is done the moment
        // you are there. Clear the standing entry and let the summary re-render
        // around whoever you have NOT seen, or come down entirely.
        //
        // `announcedEpisode` is untouched, so this never re-arms a re-announce:
        // an acknowledged prompt stays acknowledged for its whole episode.
        for key in postedAttention {
            if AgentControlBridge.shared.isPaneVisible?(key.conn, key.pane) == true {
                // Logged for the same reason the announce decision is: an
                // acknowledgment that silently fails and one that never ran look
                // identical from a lock screen that still shows the card.
                Log.island.info("acknowledged: pane \(key.pane, privacy: .public) viewed — standing cleared")
                PushStanding.clear(conn: key.conn.uuidString, pane: key.pane)
                postedAttention.remove(key)
                renderAttentionDigest(connectionId: key.conn, edge: false)
            }
        }

        var changed = false
        for (key, var p) in panes {
            // Fallback restoration — a hook-owned pane whose stamp has gone
            // stale (option unset, dropped from the poll past hookGrace) returns
            // to heuristic governance. Re-arm its bookkeeping from now so the
            // heuristic doesn't immediately mark it done off an ancient
            // lastOutput, then let the normal heuristic branches below run.
            if p.hookOwned && now.timeIntervalSince(p.lastHookSeen) > hookGrace {
                p.hookOwned = false
                p.lastOutput = now
                p.firstActive = now
                panes[key] = p
                changed = true
                continue
            }
            // A live hook-owned pane is fully governed by applyAgentHooks; the
            // heuristic's idle/done/linger transitions must not touch it.
            if hookGoverns(p, now: now) { continue }
            if p.visible {
                switch p.state {
                case .working:
                    if now.timeIntervalSince(p.lastOutput) > idleAfter {
                        p.state = .done; p.stateSince = now; panes[key] = p; changed = true
                    }
                case .done:
                    if now.timeIntervalSince(p.stateSince) > doneLinger {
                        panes[key] = nil; announcedEpisode[key] = nil; saveAnnounced(); changed = true
                    }
                case .idle:
                    panes[key] = nil; announcedEpisode[key] = nil; saveAnnounced(); changed = true
                case .attention:
                    break   // sticks until the human acts / new output flows
                }
            } else if now.timeIntervalSince(p.lastOutput) > promotionDelay + idleAfter {
                // Never promoted and now quiet — drop the bookkeeping.
                panes[key] = nil; announcedEpisode[key] = nil; saveAnnounced(); changed = true
            }
        }
        if changed { sync() }
    }

    // MARK: - Hook bridge (Phase B)

    /// Map the controller's string-typed hook state onto the aggregate schema's
    /// `AgentState`. Only the three hook-emitted phases are valid; a `nil`/other
    /// value means "no precise hook data" and the pane stays on the heuristic
    /// (so this never returns `.idle`, which hooks never emit).
    private func hookState(_ raw: String?) -> AgentActivityAttributes.AgentState? {
        switch raw {
        case "working":   return .working
        case "attention": return .attention
        case "done":      return .done
        default:          return nil
        }
    }

    /// Reclassify a stamp the host got wrong (or that time made wrong).
    ///
    /// Two fossils this heals, both seen standing on a real phone for a DAY:
    ///
    /// - An `attention` whose title is Claude's idle reminder ("Claude is
    ///   waiting for your input"). Older stamp scripts recorded idle nags as
    ///   attention; the current one refuses them, which also means it never
    ///   overwrites the old ones — the pane would show NEEDS YOU forever. An
    ///   idle reminder means the agent is at its prompt: that is `done`.
    ///
    /// - Any hook state on a pane whose FOREGROUND is back to a shell. The
    ///   stamp only ever gets written by hooks, and a killed agent fires no
    ///   final hook — `attention` freezes at the moment of death. What is
    ///   actually running in the pane is the ground truth the options cannot
    ///   provide. (Same insight the reply-path waiter used, for the same
    ///   reason.)
    static func reclassify(state: AgentActivityAttributes.AgentState,
                           title: String?,
                           foregroundCommand: String?) -> AgentActivityAttributes.AgentState {
        if let cmd = foregroundCommand, shells.contains(cmd) {
            return .done
        }
        if state == .attention, let title,
           title.localizedCaseInsensitiveContains("waiting for your input") {
            return .done
        }
        return state
    }

    /// Merge one controller's freshly-polled `@moshpit_*` stamps into the per-pane
    /// model. For each pane carrying a precise state, the hook is AUTHORITATIVE:
    /// it upserts a visible row (no `promotionDelay`/shell filtering — a stamp is
    /// an explicit "this is an agent" signal), sets state/stateSince from the
    /// stamp, and flags the pane `hookOwned` so the heuristic defers to it.
    /// Panes WITHOUT a precise state are left untouched here — `sweep()` reverts
    /// any that were previously hook-owned once their stamp goes stale, and the
    /// output heuristic governs them in the meantime.
    private func applyAgentHooks(connectionId: UUID) {
        guard let controller = conns[connectionId]?.controller else { return }
        let now = Date()
        var changed = false

        for (paneId, hook) in controller.agentHooks {
            guard let mapped = hookState(hook.state) else { continue }
            // The foreground command comes from the hook poll itself (fresh every
            // round), NOT from the discovery snapshot — that copy can be minutes
            // stale, and a stale "zsh" would suppress a just-launched agent.
            let newState = Self.reclassify(state: mapped, title: hook.title,
                                           foregroundCommand: hook.command)
            // Moshpit's own plumbing stamps a pane for real — that is what makes
            // an install self-test a proof rather than a claim — so it has to be
            // filtered here rather than by not stamping.
            if let agent = hook.agent, Self.reservedAgents.contains(agent) { continue }
            let key = PaneKey(conn: connectionId, pane: paneId)
            let cmd = hook.agent ?? command(connectionId: connectionId, paneId: paneId)
            let loc = location(connectionId: connectionId, paneId: paneId)
            let since = hook.since ?? now

            let prevState = panes[key]?.state
            let prevSince = panes[key]?.stateSince

            if var p = panes[key] {
                // A state flip (or first stamp) resets the timer to the stamp's
                // `since`; an unchanged state keeps its original start so the
                // live timer doesn't restart every 2s poll. agent/title may
                // refine the label each poll.
                let stateChanged = p.state != newState || !p.hookOwned
                p.command = cmd
                p.location = loc
                p.detail = hook.title
                p.state = newState
                p.stateSince = stateChanged ? since : p.stateSince
                p.visible = true
                p.hookOwned = true
                p.lastHookSeen = now
                p.lastOutput = now
                panes[key] = p
            } else {
                panes[key] = PaneActivity(
                    conn: connectionId, paneId: paneId,
                    command: cmd, location: loc,
                    detail: hook.title,
                    state: newState, stateSince: since,
                    lastOutput: now, firstActive: now, visible: true,
                    hookOwned: true, lastHookSeen: now)
            }
            changed = true

            // A hook flipping a pane INTO attention fires the same notification
            // a BEL would — once per episode, never once per poll and never
            // again on reconnect. `stateSince` is read back from the pane after
            // the update above, so it is the resolved episode start (the host's
            // `@moshpit_since` when the stamp carried one) rather than `now`,
            // which would differ on every poll and announce on every one.
            let episode = panes[key]?.stateSince ?? since
            if Self.shouldAnnounce(state: newState, episode: episode,
                                   lastAnnounced: announcedEpisode[key]) {
                // Through the grace window, not straight to the lock screen: a
                // question answered at the desk within `announceGrace` never
                // reaches a phone. The record is written when the announcement
                // FIRES — an episode that evaporates in the window was never
                // announced, so its next occurrence starts clean.
                scheduleAnnounce(key: key, episode: episode,
                                 location: loc, command: cmd,
                                 detail: settings.lockScreenDetailEnabled ? hook.title : nil)
            }

            // A hook flipping a pane INTO done (from a live working/attention
            // turn) is a real `Stop` — ping once. Skip a first-poll `.done` and
            // repeated done polls (no prior state, or already done).
            // `mapped == .done` and not merely `newState == .done`: a
            // reclassified fossil (a day-old idle-nag `attention` healed to
            // done, or a killed agent's frozen stamp) is bookkeeping, not a
            // finish — healing two of them must not chime twice, and their
            // "durations" are how long the fossil stood, not how long anything
            // ran. Only the host explicitly saying `done` is a turn ending.
            if newState == .done, mapped == .done,
               prevState == .working || prevState == .attention {
                // How long the closing episode ran decides whether this finish
                // is worth a sound. `prevSince` is the episode the pane is
                // leaving — since the host stopped rewriting `@moshpit_since`
                // on same-state stamps, that is "time since the last human
                // interaction", which is exactly the walked-away measure.
                let duration = prevSince.map { since.timeIntervalSince($0) } ?? 0
                postDone(connectionId: connectionId, paneId: paneId,
                         location: loc, command: cmd, duration: duration)
            }
        }

        if changed { sync() }
    }

    // MARK: - Snapshot helpers

    private func command(connectionId: UUID, paneId: String) -> String {
        guard let snapshot = conns[connectionId]?.controller?.snapshot,
              let pane = snapshot.panes[paneId] else { return "" }
        return pane.command
    }

    /// Where an agent lives, as one line for the lock screen, the island and
    /// the notification body.
    ///
    /// This is the only place that formats a window without going through
    /// `displayTitle`, and on herdr that showed. herdr labels a tab after its
    /// own number, so `"\(index):\(name)"` rendered `1:1`; worse, the session
    /// was missing entirely, so two agents in different workspaces produced a
    /// byte-identical location — on a card whose buttons are Allow and Deny.
    private func location(connectionId: UUID, paneId: String) -> String {
        let host = conns[connectionId]?.connection.displayName ?? ""
        let vocab = (conns[connectionId]?.connection.multiplexer ?? .tmux).vocabulary
        guard let snapshot = conns[connectionId]?.controller?.snapshot,
              let pane = snapshot.panes[paneId],
              let window = snapshot.windows[pane.windowId]
        else { return host }
        let session = snapshot.sessions[window.sessionId].map { snapshot.sessionDisplayName($0) }
        let place = [session, window.displayTitle(vocab)]
            .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
        return place.isEmpty ? host : "\(host) · \(place)"
    }

    /// Clean a foreground command for display (drop a leading login "-").
    private func displayCommand(_ cmd: String) -> String {
        cmd.hasPrefix("-") ? String(cmd.dropFirst()) : cmd
    }

    /// Whether a pane is still awaiting approval. nil = we have no record
    /// (e.g. the app was relaunched) — callers should NOT block on nil, only
    /// on an explicit "no longer attention".
    func attentionState(connectionId: UUID, paneId: String) -> Bool? {
        panes[PaneKey(conn: connectionId, pane: paneId)].map { $0.state == .attention }
    }

    /// When a pane entered its current state — the Agents section's "waiting
    /// for how long" answer, off the same clock the island's timer runs on.
    /// nil = no record (relaunch, or the pane never registered); the row just
    /// shows no duration rather than inventing one.
    func stateSince(connectionId: UUID, paneId: String) -> Date? {
        panes[PaneKey(conn: connectionId, pane: paneId)]?.stateSince
    }

    /// Every tracked pane's state-entry time for one connection, keyed by pane
    /// id — the Agents section reads the whole map once per rebuild instead of
    /// probing pane by pane.
    func stateSinceByPane(connectionId: UUID) -> [String: Date] {
        var result: [String: Date] = [:]
        for (key, pane) in panes where key.conn == connectionId {
            result[key.pane] = pane.stateSince
        }
        return result
    }

    // MARK: - Live Activity sync

    private func buildState() -> AgentActivityAttributes.ContentState {
        let visible = panes.values.filter(\.visible).sorted {
            if $0.state.rank != $1.state.rank { return $0.state.rank < $1.state.rank }
            return $0.stateSince > $1.stateSince
        }
        let agents = visible.prefix(maxAgents).map { p in
            AgentActivityAttributes.Agent(
                id: "\(p.conn.uuidString):\(p.paneId)",
                connectionId: p.conn.uuidString,
                paneId: p.paneId,
                command: displayCommand(p.command),
                location: p.location,
                // Privacy: the title (what the agent runs/asks) shows on the lock
                // screen, so honor the user's opt-out. Gating here covers BOTH the
                // Live Activity and the widget (writeWidgetSnapshot reads this).
                detail: settings.lockScreenDetailEnabled ? p.detail : nil,
                state: p.state,
                startedAt: p.stateSince)
        }
        let ordered = Array(agents)
        // Which agent the pill shows: the most-urgent (first) by default, or the
        // user-cycled one (the "switch agent" control rotates through the list).
        let headline = ordered.isEmpty
            ? nil
            : ordered[((headlineOffset % ordered.count) + ordered.count) % ordered.count]
        let deepLink = headline.map {
            "moshpit://connection/\($0.connectionId)?pane=\($0.paneId)"
        }
        return AgentActivityAttributes.ContentState(
            agents: ordered,
            workingCount: visible.filter { $0.state == .working }.count,
            attentionCount: visible.filter { $0.state == .attention }.count,
            headlineDeepLink: deepLink,
            headlineId: headline?.id)
    }

    /// Rotation chosen by the "switch agent" control — which agent the Dynamic
    /// Island pill shows. Advanced by ``cycleHeadline()``; harmlessly wraps as
    /// the agent set changes.
    @ObservationIgnored private var headlineOffset = 0

    /// Cycle the Dynamic Island headline to the next agent and push an update.
    func cycleHeadline() {
        headlineOffset += 1
        sync()
    }

    private func sync() {
        // Panes that moved on leave the standing set, and each connection's
        // summary card follows: re-rendered silently around whoever is still
        // waiting, withdrawn outright when nobody is. (This used to remove
        // per-pane cards; there is one card per connection now.)
        let attentionNow = Set(panes.filter { $0.value.state == .attention }.map(\.key))
        let stale = postedAttention.subtracting(attentionNow)
        if !stale.isEmpty {
            for key in stale {
                PushStanding.clear(conn: key.conn.uuidString, pane: key.pane)
            }
            postedAttention = postedAttention.intersection(attentionNow)
            for conn in Set(stale.map(\.conn)) {
                renderAttentionDigest(connectionId: conn, edge: false)
            }
        }

        let state = buildState()
        // Mirror to the App Group for the home/lock-screen Widget (it PULLs on
        // the OS schedule and can't read the pushed Live Activity state).
        writeWidgetSnapshot(state)

        #if canImport(ActivityKit)
        guard !state.agents.isEmpty else {
            if let activity {
                Task { await activity.end(nil, dismissalPolicy: .immediate) }
                self.activity = nil
            }
            return
        }

        guard settings.liveActivityEnabled,
              ActivityAuthorizationInfo().areActivitiesEnabled else { return }

        // staleDate: the island's data is pushed by OUR 2s poll, which stops the
        // moment iOS suspends the app — the island would silently show frozen
        // "working" forever. Marking content stale after 2 missed sweeps lets
        // the views render an honest "paused" hint (context.isStale) instead.
        let content = ActivityContent(state: state, staleDate: Date().addingTimeInterval(120))
        if let activity {
            Task { await activity.update(content) }
        } else {
            let attributes = AgentActivityAttributes()
            activity = try? Activity.request(attributes: attributes, content: content)
        }
        #endif
    }

    /// Mirror the current agent list into the App Group for the home/lock-screen
    /// Widget, then nudge WidgetKit to refresh (the widget otherwise only pulls
    /// on the OS's slow schedule).
    private func writeWidgetSnapshot(_ state: AgentActivityAttributes.ContentState) {
        let items = state.agents.map {
            AgentWidgetState.Item(id: $0.id, command: $0.command, location: $0.location,
                                  detail: $0.detail,
                                  state: $0.state.rawValue, startedAt: $0.startedAt,
                                  deepLink: "moshpit://connection/\($0.connectionId)?pane=\($0.paneId)")
        }
        AgentWidgetStore.write(AgentWidgetState(
            items: items,
            attentionCount: state.attentionCount,
            workingCount: state.workingCount,
            headlineDeepLink: state.headlineDeepLink,
            updatedAt: Date()))
        #if canImport(WidgetKit)
        WidgetCenter.shared.reloadAllTimelines()
        #endif
    }

    private func requestNotificationAuthorizationIfNeeded() {
        guard settings.notificationsEnabled else { return }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in }
    }
}
