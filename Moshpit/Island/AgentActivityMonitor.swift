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
    /// Attention notifications we have DELIVERED, so leaving the attention
    /// state can revoke them — a stale "needs you" on the lock screen invites
    /// answering a prompt that no longer exists (possibly approving a newer,
    /// more dangerous one).
    @ObservationIgnored private var postedAttention: Set<PaneKey> = []
    @ObservationIgnored private var sweepTimer: Timer?
    @ObservationIgnored private let settings: AppSettings
    #if canImport(ActivityKit)
    @ObservationIgnored private var activity: Activity<AgentActivityAttributes>?
    #endif

    init(settings: AppSettings) {
        self.settings = settings
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

    func untrack(connectionId: UUID) {
        conns[connectionId] = nil
        panes = panes.filter { $0.key.conn != connectionId }
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

        postAttention(connectionId: connectionId, paneId: paneId,
                      location: loc, command: p.command)
        sync()
    }

    /// Post the "agent needs you" local notification. Shared by the BEL path
    /// (``noteBell``) and the hook path (``applyAgentHooks`` when a stamp flips
    /// a pane to `.attention`), so both surface identically.
    private func postAttention(connectionId: UUID, paneId: String,
                               location: String, command: String, detail: String? = nil) {
        guard settings.notificationsEnabled else { return }
        let content = UNMutableNotificationContent()
        content.title = displayCommand(command)
        // Lead with WHAT it's asking when the hook captured it ("Bash: rm -rf …"),
        // so the user can decide from the lock screen; fall back to the generic line.
        content.body = detail.map { "\($0) — \(location)" }
            ?? String(localized: "Agent needs your input — \(location)")
        content.sound = settings.attentionSoundEnabled ? .default : nil
        // Request time-sensitive so an approval prompt breaks through Focus — but
        // this only takes effect where the `time-sensitive` entitlement is
        // provisioned (App Store / paid signing). Moshpit does NOT ship that
        // entitlement (it can't be re-signed by a free Apple ID for sideloading),
        // so on those builds iOS simply delivers this as a normal alert.
        content.interruptionLevel = .timeSensitive
        content.categoryIdentifier = AgentNotifications.Category.attention
        content.userInfo = [
            AgentNotifications.connectionKey: connectionId.uuidString,
            AgentNotifications.paneKey: paneId
        ]
        let request = UNNotificationRequest(
            identifier: "moshpit.attention.\(connectionId.uuidString).\(paneId)",
            content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request)
        postedAttention.insert(PaneKey(conn: connectionId, pane: paneId))
    }

    /// Post the "agent finished" notification. Hook-driven only — a precise
    /// `Stop` stamp flipping a pane to `.done` is a real end-of-turn signal,
    /// whereas the output heuristic's `.done` (4s of quiet) is too fuzzy to ping
    /// on. Carries the Reply action so the user can fire the next instruction
    /// from the lock screen.
    private func postDone(connectionId: UUID, paneId: String,
                          location: String, command: String) {
        guard settings.notificationsEnabled else { return }
        let content = UNMutableNotificationContent()
        content.title = String(localized: "✓ \(displayCommand(command)) finished")
        content.body = location
        // Same switch as the attention ping — agents finishing overnight used
        // to chime regardless of the Alert-sound setting.
        content.sound = settings.attentionSoundEnabled ? .default : nil
        content.categoryIdentifier = AgentNotifications.Category.done
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
                        panes[key] = nil; changed = true
                    }
                case .idle:
                    panes[key] = nil; changed = true
                case .attention:
                    break   // sticks until the human acts / new output flows
                }
            } else if now.timeIntervalSince(p.lastOutput) > promotionDelay + idleAfter {
                // Never promoted and now quiet — drop the bookkeeping.
                panes[key] = nil; changed = true
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
            guard let newState = hookState(hook.state) else { continue }
            let key = PaneKey(conn: connectionId, pane: paneId)
            let cmd = hook.agent ?? command(connectionId: connectionId, paneId: paneId)
            let loc = location(connectionId: connectionId, paneId: paneId)
            let since = hook.since ?? now

            let prevState = panes[key]?.state
            let wasAttention = prevState == .attention
            let wasHookOwned = panes[key].map { hookGoverns($0, now: now) } ?? false

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
            // a BEL would — but only on the transition, and not on the first
            // poll of an already-attention pane the user has presumably seen.
            if newState == .attention, !(wasAttention && wasHookOwned) {
                postAttention(connectionId: connectionId, paneId: paneId,
                              location: loc, command: cmd,
                              detail: settings.lockScreenDetailEnabled ? hook.title : nil)
            }

            // A hook flipping a pane INTO done (from a live working/attention
            // turn) is a real `Stop` — ping once. Skip a first-poll `.done` and
            // repeated done polls (no prior state, or already done).
            if newState == .done, prevState == .working || prevState == .attention {
                postDone(connectionId: connectionId, paneId: paneId,
                         location: loc, command: cmd)
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
        // Revoke delivered attention notifications whose pane has moved on —
        // the prompt they advertise no longer exists.
        let attentionNow = Set(panes.filter { $0.value.state == .attention }.map(\.key))
        let stale = postedAttention.subtracting(attentionNow)
        if !stale.isEmpty {
            let ids = stale.map { "moshpit.attention.\($0.conn.uuidString).\($0.pane)" }
            let center = UNUserNotificationCenter.current()
            center.removeDeliveredNotifications(withIdentifiers: ids)
            center.removePendingNotificationRequests(withIdentifiers: ids)
            postedAttention = postedAttention.intersection(attentionNow)
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
