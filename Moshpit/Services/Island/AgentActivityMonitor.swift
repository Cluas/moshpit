import Foundation
import Observation
import UserNotifications
import MoshpitKit
#if canImport(ActivityKit)
import ActivityKit
#endif
#if canImport(WidgetKit)
import WidgetKit
#endif

/// Vibe Island — the platform half.
///
/// What Moshpit makes of its coding agents is decided in Rust
/// (`crates/agents`, through `AgentMachine`): per-pane states from the
/// hooks and the output, the announce grace, the waiting set's 0→1 edge, a
/// long finish's one ring, what the island, the widget and the cards show,
/// and only when it changes. This class feeds it — the multiplexer's hook
/// polls, tmux's output and bells, a sweep every two seconds with the pane
/// on screen — and does what it answers: the Live Activity (ActivityKit),
/// the cards (the notification center), the widget's snapshot (the App
/// Group and WidgetCenter), the announced-episode record (UserDefaults).
///
/// Tapping the island deep-links to the headline agent's pane via
/// `moshpit://connection/<uuid>?pane=<paneId>`.
@MainActor
@Observable
final class AgentActivityMonitor {

    /// What the monitor asks of the notification center.
    struct Notifier {
        var add: (UNNotificationRequest) -> Void
        var withdraw: (_ identifier: String) -> Void
        var requestAuthorization: () -> Void
        /// The identifiers of the notifications it holds, handed to the
        /// completion on any queue.
        var delivered: (_ completion: @escaping @Sendable ([String]) -> Void) -> Void = { $0([]) }

        static var system: Notifier { Notifier(
            add: { UNUserNotificationCenter.current().add($0) },
            withdraw: { id in
                let center = UNUserNotificationCenter.current()
                center.removeDeliveredNotifications(withIdentifiers: [id])
                center.removePendingNotificationRequests(withIdentifiers: [id])
            },
            requestAuthorization: {
                UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in }
            },
            delivered: { completion in
                UNUserNotificationCenter.current().getDeliveredNotifications { notes in
                    completion(notes.map(\.request.identifier))
                }
            }) }
    }

    /// The saved connections' ids, nil while they can't be told (before the
    /// first unlock): a card of a connection no longer saved goes with it.
    /// Set by the app; nil until then, which judges no connection gone.
    @ObservationIgnored var savedConnections: () -> Set<UUID>? = { nil }

    private struct ConnRef {
        let connection: ServerConnection
        /// Either multiplexer's control surface: the tree and the hook
        /// stamps are on the protocol; tmux's output and bells are wired in
        /// `track`, because only tmux has them.
        weak var controller: (any MultiplexerControlling)?
        /// The session, for whether its link is up (B662); nil for one
        /// tracked without it (a test's), taken as up.
        weak var session: SessionHub.ActiveSession?
        /// What the machine was last told of the link.
        var linkUp = true
    }

    /// Where the announced-episode record lives.
    static let announcedKey = "island.announcedEpisodes"

    @ObservationIgnored private var conns: [UUID: ConnRef] = [:]
    /// The machine: Home's record reads its clock (`homeMessage`).
    @ObservationIgnored let machine: AgentMachine
    @ObservationIgnored private var sweepTimer: Timer?
    /// The widget's snapshot as last written (B701).
    @ObservationIgnored private var lastWidget: AgentWidgetState?
    @ObservationIgnored private let settings: AppSettings
    @ObservationIgnored private let island: AgentIsland
    @ObservationIgnored private let notifier: Notifier
    @ObservationIgnored private let writeWidget: (AgentWidgetState) -> Void
    /// The clock; nil: the wall's. A test drives the machine on its own.
    @ObservationIgnored private let clock: (() -> Date)?

    /// `standing` and `finishes` are the files the notification service
    /// extension shares (the App Group's; a test's own); the rest are where
    /// the machine's answers go.
    init(settings: AppSettings, standing: URL? = DeviceLocalFiles.url("standing-attention.json"),
         writeWidget: ((AgentWidgetState) -> Void)? = nil, island: AgentIsland? = nil,
         finishes: URL? = DeviceLocalFiles.url("rung-finishes.json"), notifier: Notifier = .system,
         clock: (() -> Date)? = nil, readiness: DefaultsReadiness = .system) {
        self.settings = settings
        self.readiness = readiness
        self.island = island ?? ActivityKitIsland(settings: settings)
        self.notifier = notifier
        self.clock = clock
        self.writeWidget = writeWidget ?? { snapshot in
            AgentWidgetStore.write(snapshot)
            #if canImport(WidgetKit)
            WidgetCenter.shared.reloadAllTimelines()
            #endif
        }
        // Read sealed (a prewarm before the first unlock), the record is
        // nothing: it is read again once it can be believed (B348).
        let trusted = readiness.isReadable(settings.store)
        recordTrusted = trusted
        let record = trusted ? Self.storedRecord(settings.store) : [:]
        machine = AgentMachine(settings: Self.value(of: settings),
                               standingPath: standing?.path,
                               finishesPath: finishes?.path,
                               announced: record,
                               announceGrace: Self.announceGrace,
                               now: (clock?() ?? Date()).timeIntervalSince1970)
        // What a killed process left up: none of its agents is known here
        // yet (B441). The widget's snapshot is not the island's: it stays
        // (B709). The share sheet and Shortcuts offer its agents as the
        // last known, with how old the list is, and the widget turns it
        // stale on its own timeline. B667 emptied it at a launch that
        // connected nowhere, and left them nothing to offer.
        self.island.endLeftovers()
    }

    private var now: Double { (clock?() ?? Date()).timeIntervalSince1970 }

    @ObservationIgnored private let readiness: DefaultsReadiness
    /// Whether the announced record read so far can be believed. Until it
    /// can, nothing is written over the stored one: a launch before the
    /// first unlock read it as empty, and its first save wrote that over
    /// it — every question still standing announced again after a reboot,
    /// its earlier card never withdrawn (B348).
    @ObservationIgnored private var recordTrusted: Bool

    private static func storedRecord(_ defaults: UserDefaults) -> [String: Double] {
        defaults.dictionary(forKey: announcedKey) as? [String: Double] ?? [:]
    }

    /// The widget's latest snapshot while the preferences couldn't be
    /// believed (B603): written once they can.
    @ObservationIgnored private var heldWidget: AgentWidgetState?

    /// The app came forward: the record a sealed launch couldn't read joins
    /// what the machine kept meanwhile, and is saved (B348). A no-op once
    /// the record has been read. The widget's held snapshot, if any, goes
    /// out (B603).
    func reloadIfNeeded() {
        defer { writeHeldWidget() }
        guard !recordTrusted, readiness.isReadable(settings.store) else { return }
        recordTrusted = true
        let stored = Self.storedRecord(settings.store)
        let now = self.now
        drive { $0.adoptAnnounced(announced: stored, now: now) }
    }

    /// How long a prompt must stand before the phone hears about it.
    /// `-MOSHPIT_ANNOUNCE_GRACE <seconds>` shortens it so a device harness
    /// doesn't spend half a minute per phase watching a timer.
    nonisolated static let announceGrace: TimeInterval = {
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: "-MOSHPIT_ANNOUNCE_GRACE"), i + 1 < args.count,
           let v = TimeInterval(args[i + 1]), v >= 0 {
            return v
        }
        return 30
    }()

    // MARK: - Lifecycle

    /// Track a connection's multiplexer, whichever it is. The output and
    /// bell heuristics are tmux-only: herdr reports `agent_status` itself.
    /// Whatever the switches say (B538): a session that connected with
    /// notifications and the Live Activity off was never watched, so
    /// turning either on later did nothing for it until it reconnected, and
    /// Home had no "since" for its agents. The machine decides what reaches
    /// the phone, from the settings as they are then.
    func track<C: MultiplexerControlling>(connection: ServerConnection, controller: C,
                                          session: SessionHub.ActiveSession? = nil) {
        let id = connection.id
        conns[id] = ConnRef(connection: connection, controller: controller,
                            session: session ?? conns[id]?.session)
        if let tmux = controller as? RustTmuxController {
            tmux.onPaneActivity = { [weak self] paneId in
                self?.output(connectionId: id, paneId: paneId)
            }
            tmux.onPaneBell = { [weak self] paneId in
                self?.bell(connectionId: id, paneId: paneId)
            }
            tmux.onPaneInput = { [weak self] paneId in
                self?.input(connectionId: id, paneId: paneId)
            }
        }
        controller.onAgentHooksUpdated = { [weak self] in
            self?.applyAgentHooks(connectionId: id)
        }
        startSweepTimerIfNeeded()
        drive { $0.track(conn: id.uuidString, label: connection.displayName) }
        sendTree(connectionId: id)
        sweepDelivered()
    }

    /// The app left the foreground (B701). The widget is written only when
    /// what it shows changes: each write reloads its timelines, which iOS
    /// budgets by the day. Its timeline turns it stale on its own, a
    /// horizon after the snapshot's date — the last change, maybe long
    /// before. Written once more now, it stays fresh while the app could
    /// still be watching, and goes stale a horizon after the app stopped.
    /// Not with no agents listed: nothing there to go stale.
    func leftForeground() {
        guard let snapshot = Self.widgetRefreshedAtLeaving(lastWidget, at: Date()),
              readiness.isReadable(settings.store)
        else { return }
        lastWidget = snapshot
        writeWidget(snapshot)
    }

    /// The snapshot to write again as the app leaves the foreground: the
    /// last one, when it lists agents; nil otherwise.
    static func widgetRefreshedAtLeaving(_ last: AgentWidgetState?, at now: Date) -> AgentWidgetState? {
        guard var snapshot = last, !snapshot.items.isEmpty else { return nil }
        snapshot.updatedAt = now
        return snapshot
    }

    /// Hand the machine the agent cards the notification center holds, so a
    /// card nothing stands behind any more is taken down (B433): a pushed
    /// "needs you" whose question ended while the app was away. On every
    /// track and every return to the foreground.
    func sweepDelivered() {
        notifier.delivered { [weak self] ids in
            Task { @MainActor in self?.judgeDelivered(ids) }
        }
    }

    private func judgeDelivered(_ ids: [String]) {
        let ours = ids.filter { $0.hasPrefix("moshpit.attention.") || $0.hasPrefix("moshpit.done.") }
        guard !ours.isEmpty else { return }
        let saved = savedConnections().map { $0.map(\.uuidString) }
        let now = self.now
        drive { $0.delivered(ids: ours, saved: saved, now: now) }
    }

    /// Track a connection once its multiplexer control plane exists — and
    /// every controller it ever wires: a reconnect rebuilds it, and the weak
    /// ref `track` keeps goes nil with the old one.
    func trackWhenReady(connection: ServerConnection, session: SessionHub.ActiveSession) {
        session.onMultiplexerControlChanged = { [weak self, weak session] controller in
            self?.track(connection: connection, controller: controller, session: session)
        }
        Task { [weak self, weak session] in
            guard let session, let controller = await session.awaitMultiplexerControl() else { return }
            self?.track(connection: connection, controller: controller, session: session)
        }
    }

    /// Stop watching a connection. `keepWaitingCards`: the user
    /// disconnected (or is reconnecting) and the agents there still wait —
    /// their cards stay up; a deleted connection takes them (the default).
    func untrack(connectionId: UUID, keepWaitingCards: Bool = false) {
        conns[connectionId] = nil
        if conns.isEmpty {
            sweepTimer?.invalidate()
            sweepTimer = nil
        }
        let now = self.now
        drive { $0.untrack(conn: connectionId.uuidString, keepWaitingCards: keepWaitingCards, now: now) }
    }

    /// Cycle the Dynamic Island headline to the next agent: the scripts'
    /// `cycle` step (AgentsScriptTests). The island has no Switch any more.
    func cycleHeadline() {
        let now = self.now
        drive { $0.cycleHeadline(now: now) }
    }


    /// Every tracked pane's state-entry time for one connection, keyed by
    /// pane id.
    func stateSinceByPane(connectionId: UUID) -> [String: Date] {
        machine.stateSinceByPane(conn: connectionId.uuidString)
            .mapValues { Date(timeIntervalSince1970: $0) }
    }

    // MARK: - Signals

    private func output(connectionId: UUID, paneId: String) {
        let place = place(connectionId: connectionId, paneId: paneId)
        let now = self.now
        drive { $0.output(conn: connectionId.uuidString, pane: paneId, place: place, now: now) }
    }

    private func bell(connectionId: UUID, paneId: String) {
        let place = place(connectionId: connectionId, paneId: paneId)
        let now = self.now
        drive { $0.bell(conn: connectionId.uuidString, pane: paneId, place: place, now: now) }
    }

    /// The person typed into a pane (B663): a bell's "needs you" there is
    /// answered.
    private func input(connectionId: UUID, paneId: String) {
        let now = self.now
        drive { $0.input(conn: connectionId.uuidString, pane: paneId, now: now) }
    }

    private func applyAgentHooks(connectionId: UUID) {
        guard let controller = conns[connectionId]?.controller else { return }
        sendTree(connectionId: connectionId)
        let hooks = controller.agentHooks.map { paneId, hook in
            AgentHookInValue(pane: paneId, state: hook.state, agent: hook.agent,
                             since: hook.since?.timeIntervalSince1970,
                             command: hook.command, title: hook.title,
                             place: place(connectionId: connectionId, paneId: paneId))
        }
        let now = self.now
        let interval = Self.stampInterval(of: controller)
        drive { $0.hooks(conn: connectionId.uuidString, hooks: hooks, now: now, interval: interval) }
    }

    // MARK: - Sweep

    /// How often the sweep runs — and so how often tmux's stamps are polled.
    static let sweepInterval: TimeInterval = 2

    /// How often `controller`'s stamps arrive, for how long the machine
    /// holds one (B346): tmux's are polled by this sweep; herdr's by its own
    /// client, at its pace (slower while its push channel is up).
    static func stampInterval(of controller: any MultiplexerControlling) -> TimeInterval {
        if let herdr = controller as? RustHerdrController { return herdr.stampInterval }
        return sweepInterval
    }

    private func startSweepTimerIfNeeded() {
        guard sweepTimer == nil else { return }
        sweepTimer = Timer.scheduledTimer(withTimeInterval: Self.sweepInterval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.sweep() }
        }
    }

    /// Every two seconds: a hook poll for every tracked controller, and the
    /// machine's sweep with the pane on screen.
    func sweep() {
        for (id, ref) in conns {
            // The link (B662): a session reconnecting or offline pauses its
            // agents in the machine, which no longer takes the polls'
            // silence for their stamps going away; live again, they resume.
            let up = ref.session.map { $0.viewModel.connState == .live } ?? true
            if up != ref.linkUp {
                conns[id]?.linkUp = up
                let now = self.now
                drive { $0.link(conn: id.uuidString, up: up, now: now) }
            }
            guard up else { continue }
            ref.controller?.pollAgentHooks()
            sendTree(connectionId: id)
        }
        let visible = AgentControlBridge.shared.visiblePane?()
        let now = self.now
        drive { $0.sweep(visibleConn: visible?.0.uuidString, visiblePane: visible?.1 ?? nil, now: now) }
    }

    /// A test's output and bell, as tmux's callbacks deliver them.
    func outputForScript(connectionId: UUID, paneId: String) { output(connectionId: connectionId, paneId: paneId) }
    func bellForScript(connectionId: UUID, paneId: String) { bell(connectionId: connectionId, paneId: paneId) }

    /// The connection's tree, for what a card says about a pane no poll
    /// has named yet (a prompt the notification service recorded).
    private func sendTree(connectionId: UUID) {
        guard let snapshot = conns[connectionId]?.controller?.snapshot else { return }
        var places: [String: AgentPaneWhereValue] = [:]
        for paneId in snapshot.panes.keys {
            places[paneId] = place(connectionId: connectionId, paneId: paneId)
        }
        machine.tree(conn: connectionId.uuidString, places: places)
    }

    // MARK: - The machine's answers

    /// The snapshot held while the preferences couldn't be believed, now
    /// that they can (B603).
    private func writeHeldWidget() {
        guard let held = heldWidget, readiness.isReadable(settings.store) else { return }
        heldWidget = nil
        writeWidget(held)
    }

    private func drive(_ step: (AgentMachine) -> [AgentEffectValue]) {
        // What a switch changed does first: the island ended when the Live
        // Activity was turned off (B436), within a sweep of it.
        for effect in machine.setSettings(settings: Self.value(of: settings)) {
            apply(effect)
        }
        for effect in step(machine) {
            apply(effect)
        }
    }

    private func apply(_ effect: AgentEffectValue) {
        switch effect {
        case .island(let state):
            var content = Self.content(state)
            // The companion's stills (PET-DESIGN §7.1): the headline's pose,
            // and the sleeping one for a stale island.
            if let pet = CompanionPet.shared.island(state: content.headline?.state.rawValue ?? "idle") {
                content.companion = pet.pose
                content.companionStale = pet.stale
            }
            if island.live() != nil {
                island.update(content)
            } else if island.request(content) == nil {
                // Not shown (B701): the system refused it, or the user
                // swiped it away. It used to count as shown, and wasn't
                // asked for again until the state changed.
                let now = self.now
                drive { $0.islandNotShown(now: now) }
            }
        case .endIsland:
            island.endAll()
        case .widget(let state):
            let snapshot = Self.widget(state, at: Date(timeIntervalSince1970: now))
            // The snapshot is a write into the App Group's preferences,
            // sealed until the first unlock as the app's are: one written
            // then can replace them with this one key, the push switches and
            // the recorded language gone with it (B603, as
            // `AppLanguage.recordIfTrusted`). Held until they can be
            // believed; the latest goes then.
            guard readiness.isReadable(settings.store) else {
                heldWidget = snapshot
                return
            }
            heldWidget = nil
            lastWidget = snapshot
            writeWidget(snapshot)
        case .post(let card):
            notifier.add(Self.request(card))
        case .withdraw(let id):
            notifier.withdraw(id)
        case .saveAnnounced(let record):
            // Kept in the machine until the stored one can be read.
            guard recordTrusted else { return }
            settings.store.set(record, forKey: Self.announcedKey)
        case .requestAuthorization:
            notifier.requestAuthorization()
        case .note(let text):
            Log.island.info("\(text, privacy: .public)")
        case .finished:
            // Every finished turn, for the companion pet (PET-DESIGN §15.4):
            // the egg cracks, the pet counts it.
            CompanionPet.shared.machine.agentFinished()
        }
    }

    /// The machine's switches and language: the app's own localization, so
    /// a card reads in the language the rest of the app does.
    static func value(of settings: AppSettings) -> AgentSettingsValue {
        AgentSettingsValue(notifications: settings.notificationsEnabled,
                           liveActivity: settings.liveActivityEnabled,
                           showDetail: settings.lockScreenDetailEnabled,
                           sound: settings.attentionSoundEnabled,
                           language: AppLanguage.current())
    }

    /// Where a pane lives, from the controller's tree: what runs in it, its
    /// session's and its window's names. Nothing for a pane the tree
    /// doesn't show (the line is then the host alone).
    private func place(connectionId: UUID, paneId: String) -> AgentPaneWhereValue {
        let vocab = (conns[connectionId]?.connection.multiplexer ?? .tmux).vocabulary
        guard let snapshot = conns[connectionId]?.controller?.snapshot,
              let pane = snapshot.panes[paneId] else {
            return AgentPaneWhereValue(command: "", session: nil, window: nil)
        }
        guard let window = snapshot.windows[pane.windowId] else {
            return AgentPaneWhereValue(command: pane.command, session: nil, window: nil)
        }
        let session = snapshot.sessions[window.sessionId]
        let herdr = conns[connectionId]?.connection.multiplexer == .herdr
        // The window's own name, herdr's workspace and the pane's folder:
        // what Home names it from, so the island says it as Home does.
        return AgentPaneWhereValue(command: pane.command,
                                   session: session.map { snapshot.sessionDisplayName($0) },
                                   window: window.displayTitle(vocab),
                                   windowName: window.name,
                                   workspace: herdr ? session?.name : nil,
                                   path: pane.path,
                                   windowIndex: Int64(window.index),
                                   paneIndex: Int64(pane.index))
    }

    static func content(_ state: AgentIslandValue) -> AgentActivityAttributes.ContentState {
        let agents = state.agents.map { row in
            AgentActivityAttributes.Agent(
                id: row.id, connectionId: row.connectionId, paneId: row.paneId,
                command: row.command, location: row.location, place: row.place, detail: row.detail,
                state: AgentActivityAttributes.AgentState(rawValue: row.state) ?? .working,
                startedAt: Date(timeIntervalSince1970: row.startedAt),
                pausedAt: row.pausedAt.map { Date(timeIntervalSince1970: $0) })
        }
        let headline = agents.first { $0.id == state.headlineId }
        return AgentActivityAttributes.ContentState(
            agents: agents,
            workingCount: Int(state.workingCount),
            attentionCount: Int(state.attentionCount),
            headlineDeepLink: headline.map {
                AgentDeepLink.string(connectionId: $0.connectionId, paneId: $0.paneId)
            },
            headlineId: state.headlineId)
    }

    static func widget(_ state: AgentIslandValue, at now: Date) -> AgentWidgetState {
        let content = content(state)
        let items = content.agents.map {
            AgentWidgetState.Item(id: $0.id, command: $0.command, location: $0.location,
                                  detail: $0.detail, state: $0.state.rawValue,
                                  startedAt: $0.startedAt,
                                  deepLink: AgentDeepLink.string(connectionId: $0.connectionId,
                                                                 paneId: $0.paneId),
                                  paused: $0.isPaused ? true : nil)
        }
        // The widget draws the most urgent agent, its first row, and opens
        // that one (B594): it used to open the island's headline, which
        // Switch may have cycled to another agent.
        return AgentWidgetState(
            items: items,
            attentionCount: content.attentionCount,
            workingCount: content.workingCount,
            headlineDeepLink: items.first?.deepLink,
            updatedAt: now)
    }

    static func request(_ card: AgentCardValue) -> UNNotificationRequest {
        let content = UNMutableNotificationContent()
        content.title = card.title
        content.body = card.body
        content.sound = card.sound ? .default : nil
        switch card.level {
        case .passive: content.interruptionLevel = .passive
        case .active: content.interruptionLevel = .active
        case .timeSensitive: content.interruptionLevel = .timeSensitive
        }
        content.userInfo = [
            AgentNotifications.connectionKey: card.connectionId,
            AgentNotifications.paneKey: card.paneId,
        ]
        return UNNotificationRequest(identifier: card.id, content: content, trigger: nil)
    }
}
