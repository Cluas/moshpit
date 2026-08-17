import PhotosUI
import SwiftUI

/// Home command board. Saved hosts stay scannable offline; live hosts expand
/// into the tmux SESSIONS tree without changing the underlying SessionHub flow.
struct AttachHomeView: View {
    @Environment(AppSettings.self) private var settings
    @Environment(ThemeStore.self) private var themes
    @Environment(SessionMetricsRegistry.self) private var metrics
    @Environment(AgentActivityMonitor.self) private var monitor
    @Environment(DeepLinkRouter.self) private var router

    let store: ConnectionStore
    let hub: SessionHub
    let keychain: KeychainService

    #if DEBUG
    // A capture run names the screen it wants (see CaptureScreen); deriving the
    // initial value means the sheet is up from the first render rather than
    // depending on when a .task happens to fire.
    @State private var showSettings = CaptureScreen.requested != nil
    #else
    @State private var showSettings = false
    #endif
    @State private var showAddConnection = false
    @State private var editingConnection: ServerConnection?
    @State private var deletingConnection: ServerConnection?
    /// Cards whose disconnect teardown is in flight — drives the per-card
    /// spinner and guards against double-taps during the ~2s channel flush.
    @State private var disconnectingIds: Set<UUID> = []
    @State private var path: [UUID] = []
    /// Sessions whose handshakes are in flight — a QUEUE, not a single slot:
    /// connecting a second host used to overwrite the first, leaving its
    /// host-key TOFU prompt unobserved → that handshake deadlocked silently.
    /// Prompts and errors surface one at a time, oldest first.
    @State private var connecting: [SessionHub.ActiveSession] = []
    @State private var showConnectError = false

    private var theme: TerminalTheme {
        themes.theme(id: settings.themeId)
    }

    private var liveCount: Int {
        store.connections.filter { hub.session(for: $0) != nil }.count
    }

    /// Live multiplexer sessions across every connection, and what to call
    /// them. Counts BOTH control planes: a herdr workspace is a session too,
    /// and the chip reporting 0 while one was live read as "nothing running".
    /// The label follows whatever is actually connected — mixed hosts fall
    /// back to the neutral word rather than picking a side.
    /// Agents waiting on a human across every live connection — the number
    /// this whole app exists to surface, so it owns the header's status strip.
    private var attentionAgents: (count: Int, firstConnection: ServerConnection?) {
        var count = 0
        var first: ServerConnection?
        for connection in store.connections {
            guard let session = hub.session(for: connection) else { continue }
            let hooks = session.tmuxController?.agentHooks
                ?? session.moshControl?.agentHooks
                ?? session.herdrControl?.agentHooks
                ?? [:]
            let waiting = hooks.filter { AgentSignal($0.value.state) == .attention }.count
            if waiting > 0, first == nil { first = connection }
            count += waiting
        }
        return (count, first)
    }

    private var firstLiveConnection: ServerConnection? {
        store.connections.first { hub.session(for: $0) != nil }
    }

    /// Oldest in-flight session with a pending host-key prompt / error — the
    /// one the (single) alert presents. The next surfaces when this resolves.
    private var promptingSession: SessionHub.ActiveSession? {
        connecting.first { $0.viewModel.hostKeyPrompt != nil }
    }
    private var erroredSession: SessionHub.ActiveSession? {
        connecting.first { $0.viewModel.errorMessage?.isEmpty == false }
    }

    /// A live session for the Settings agent-hooks installer to run against.
    /// Prefer one with an attached tmux control surface (so Run-in-terminal +
    /// Re-check both work); fall back to any connected session, else nil so the
    /// installer shows command + Copy only.
    private var liveSessionForHooks: SessionHub.ActiveSession? {
        let connected = store.connections.compactMap { hub.session(for: $0) }
            .filter { if case .connected = $0.viewModel.status { return true } else { return false } }
        return connected.first { $0.tmuxControl != nil } ?? connected.first
    }

    /// + on Home: adds a new connection. No free-tier cap — every feature is
    /// unlocked for everyone.
    private func handleAdd() {
        showAddConnection = true
    }

    /// Start (or resume) a connection in-place. Binds the session to `connecting`
    /// BEFORE start() so the host-key TOFU prompt — which fires mid-handshake
    /// through `viewModel.hostKeyPrompt` — has an observer and can be answered;
    /// otherwise the handshake deadlocks. The card reads `hub.session(for:)`
    /// to show its in-place spinner / sessions tree as state arrives.
    private func connect(_ connection: ServerConnection) {
        let session = hub.prepare(connection)
        guard !connecting.contains(where: { $0.id == session.id }) else { return }
        connecting.append(session)
        Task {
            await hub.start(session, theme: theme, fontSize: settings.fontSize, fontName: settings.fontName,
                            cursorShape: settings.cursorShape, cursorColorId: settings.cursorColorId,
                            cursorBlink: settings.cursorBlink)
            if let controller = session.tmuxController {
                monitor.track(connection: connection, controller: controller)
            } else if let herdr = session.herdrControl {
                // herdr reports agent status natively — no host-side hooks.
                monitor.track(connection: connection, controller: herdr)
            }
            // Keep failed sessions queued so the error alert (which reads
            // erroredSession) has its message; its OK button dequeues them.
            if session.viewModel.errorMessage == nil {
                connecting.removeAll { $0.id == session.id }
            }
        }
    }

    var body: some View {
        NavigationStack(path: $path) {
            ZStack {
                MoshpitBackground()

                ScrollView {
                    VStack(alignment: .leading, spacing: 18) {
                        HomeHeader(
                            savedCount: store.connections.count,
                            liveCount: liveCount,
                            attentionCount: attentionAgents.count,
                            onStatusTap: {
                                if let target = attentionAgents.firstConnection ?? firstLiveConnection {
                                    Haptics.select()
                                    path.append(target.id)
                                }
                            },
                            onSettings: { showSettings = true },
                            onAdd: { handleAdd() }
                        )

                        if store.connections.isEmpty {
                            HomeSectionLabel(title: "NO HOSTS", trailing: "READY")
                        } else {
                            HomeSectionLabel(title: "CONNECTIONS",
                                             count: store.connections.count)
                        }

                        if store.connections.isEmpty {
                            emptyState
                                .frame(maxWidth: .infinity)
                        } else {
                            LazyVStack(alignment: .leading, spacing: 12) {
                                ForEach(store.connections) { connection in
                                    ConnectionCard(
                                        connection: connection,
                                        active: hub.session(for: connection),
                                        metrics: metrics.metrics[connection.id],
                                        onConnect: { connect(connection) },
                                        onEnter: { path.append(connection.id) },
                                        onEdit: { editingConnection = connection },
                                        isDisconnecting: disconnectingIds.contains(connection.id),
                                        onDisconnect: {
                                            // Teardown flushes the control channel for up to
                                            // ~2s — without feedback the button reads as
                                            // broken and invites a second tap.
                                            guard !disconnectingIds.contains(connection.id) else { return }
                                            disconnectingIds.insert(connection.id)
                                            Haptics.tap()
                                            Task {
                                                monitor.untrack(connectionId: connection.id)
                                                await hub.disconnect(connection.id)
                                                disconnectingIds.remove(connection.id)
                                            }
                                        },
                                        onDelete: { deletingConnection = connection },
                                        onRetryAttach: {
                                            // Same teardown-then-reconnect shape as
                                            // onDisconnect above, chained straight into a
                                            // fresh connect() — a stalled tmux attach
                                            // leaves SSH `.connected`, so a plain retry
                                            // (onConnect alone) would no-op.
                                            guard !disconnectingIds.contains(connection.id) else { return }
                                            disconnectingIds.insert(connection.id)
                                            Haptics.tap()
                                            Task {
                                                monitor.untrack(connectionId: connection.id)
                                                await hub.disconnect(connection.id)
                                                disconnectingIds.remove(connection.id)
                                                connect(connection)
                                            }
                                        }
                                    )
                                }
                            }
                        }

                        // A quiet floor for the page — the scroll used to just
                        // stop dead after the last card. Doubles as the "which
                        // build am I running" answer for sideload feedback.
                        HomeFooter()
                    }
                    .padding(.horizontal, Metrics.homeHPad)
                    .padding(.top, 14)
                    .padding(.bottom, 30)
                    // Two frames rather than one: the first caps the column,
                    // the second hands it the scroll view's full width to be
                    // centred within. Capping alone would just pin the column
                    // to the leading edge and leave the void on one side.
                    .frame(maxWidth: Metrics.homeMaxWidth)
                    .frame(maxWidth: .infinity)
                }
            }
            .toolbar(.hidden, for: .navigationBar)
            .navigationDestination(for: UUID.self) { id in
                if let connection = store.connections.first(where: { $0.id == id }) {
                    // No pane target passed here — TerminalScreen reads the
                    // deep-link pane straight from `router.paneRequest` itself
                    // (see its own `.onChange`), which also lets it react to a
                    // SECOND request naming a different pane on this same
                    // connection while already on screen, when this push is a
                    // no-op (`path` unchanged) and this closure never re-runs.
                    TerminalScreen(connection: connection, hub: hub)
                }
            }
        }
        .preferredColorScheme(.dark)
        .onChange(of: router.paneRequest) { _, request in
            guard let request, store.connections.contains(where: { $0.id == request.connectionId })
            else { return }
            // Only push if we're not already there — reassigning `path` to its
            // current value is a no-op for NavigationStack (this closure won't
            // re-run), which is exactly why TerminalScreen owns the pane
            // selection itself instead of relying on a fresh push here.
            if path != [request.connectionId] {
                // Deep link jumps straight into the terminal; TerminalScreen.task
                // still owns prepare/start + the host-key prompt for that path
                // (idempotent — re-entry from a card tap won't re-handshake).
                path = [request.connectionId]
            }
        }
        .moshpitCard(isPresented: $showConnectError) {
            MoshpitNoticeCard(
                icon: "bolt.slash.fill",
                title: "Connection Error",
                message: erroredSession?.viewModel.errorMessage ?? ""
            ) {
                showConnectError = false
                // Drop the dead session so its card returns to the offline
                // state (tappable to retry); the next queued error (if any)
                // surfaces via onChange.
                if let failed = erroredSession {
                    failed.viewModel.errorMessage = nil
                    let id = failed.id
                    connecting.removeAll { $0.id == id }
                    Task { await hub.disconnect(id) }
                }
            }
        }
        .onChange(of: erroredSession?.viewModel.errorMessage) { _, message in
            showConnectError = message?.isEmpty == false
        }
        .moshpitCard(
            item: Binding(
                get: { promptingSession?.viewModel.hostKeyPrompt },
                set: { if $0 == nil { /* dismissal handled via decide() */ } }
            )
        ) { prompt in
            hostKeyPromptCard(prompt)
        }
        .sheet(isPresented: $showSettings) {
            SettingsScreen(liveSession: liveSessionForHooks)
        }
        .sheet(isPresented: $showAddConnection) {
            AddConnectionView(store: store, keychain: keychain)
        }
        .sheet(item: $editingConnection) { connection in
            AddConnectionView(store: store, keychain: keychain, existing: connection)
        }
        .confirmationDialog(
            "Delete \(deletingConnection?.displayName ?? String(localized: "connection"))?",
            isPresented: Binding(get: { deletingConnection != nil },
                                 set: { if !$0 { deletingConnection = nil } }),
            titleVisibility: .visible,
            presenting: deletingConnection
        ) { connection in
            Button("Delete", role: .destructive) { delete(connection) }
            Button("Cancel", role: .cancel) { deletingConnection = nil }
        } message: { _ in
            Text("Removes the saved server and its stored credentials from this device.")
        }
    }

    /// Tear down any live session, drop the keychain secret, then forget the
    /// connection. Keychain cleanup keeps deleted servers from leaving secrets
    /// behind.
    private func delete(_ connection: ServerConnection) {
        Task {
            monitor.untrack(connectionId: connection.id)
            await hub.disconnect(connection.id)
            // Only drop connection-owned secrets. When the ref points at a
            // managed SSH key (sshKeyId set), the blob belongs to the key —
            // deleting it here would break every other connection using it.
            if let ref = connection.keychainRef, connection.sshKeyId == nil {
                try? await keychain.delete(forRef: ref)
            }
            store.delete(id: connection.id)
            deletingConnection = nil
        }
    }

    private var emptyState: some View {
        VStack(spacing: 12) {
            AppIconMark(size: 52)
                .opacity(0.85)
                .padding(.bottom, 4)
            Text("No connections yet")
                .font(Face.display(18, .semibold))
                .foregroundStyle(Ink.secondary)
            Text("Tap ＋ to add your first server. Moshpit keeps your sessions alive across Wi-Fi / 5G handoff.")
                .font(Face.text(12))
                .foregroundStyle(Ink.tertiary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 46)
        .padding(.horizontal, 22)
        .background(Ink.group, in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                .strokeBorder(Ink.cardBorder, lineWidth: 1)
        )
    }
}

// MARK: - Home header

private struct HomeHeader: View {
    let savedCount: Int
    let liveCount: Int
    let attentionCount: Int
    let onStatusTap: () -> Void
    let onSettings: () -> Void
    let onAdd: () -> Void

    /// The strip only acts when there's somewhere meaningful to go.
    private var stripIsLive: Bool { attentionCount > 0 || liveCount > 0 }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(alignment: .top) {
                HStack(spacing: 12) {
                    AppIconMark(size: 46)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Moshpit")
                            .font(Face.display(38, .bold))
                            .foregroundStyle(Ink.primary)
                        Text("THE PIT NEVER CLOSES")
                            .font(Face.mono(10, .bold))
                            .kerning(1.4)
                            .foregroundStyle(Ink.sectionTitle)
                    }
                }

                Spacer()

                HStack(spacing: 8) {
                    HomeIconButton(systemImage: "gearshape") { onSettings() }
                        .accessibilityIdentifier("home-settings")
                    HomeIconButton(systemImage: "plus") { onAdd() }
                        .accessibilityIdentifier("home-add")
                }
            }

            // One status strip instead of three decorative counters ("又不能
            // 点击") — it says the one thing that matters right now and TAKES
            // YOU THERE: an agent waiting outranks everything, then live
            // connections; all-quiet is just a quiet line.
            Button(action: onStatusTap) {
                HStack(spacing: 8) {
                    Image(systemName: stripIcon)
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(stripTint)
                    Text(verbatim: stripText)
                        .font(Face.mono(11, .semibold))
                        .kerning(0.7)
                        .foregroundStyle(stripIsLive ? Ink.secondary : Ink.meta)
                        .lineLimit(1)
                    Spacer()
                    if stripIsLive {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 10, weight: .bold))
                            .foregroundStyle(stripTint)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(
                    attentionCount > 0 ? Ink.warn.opacity(0.10)
                        : stripIsLive ? Ink.accent.opacity(0.08)
                        : Color.white.opacity(0.045),
                    in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(
                        attentionCount > 0 ? Ink.warn.opacity(0.28)
                            : stripIsLive ? Ink.accent.opacity(0.24)
                            : Color.white.opacity(0.07),
                        lineWidth: 1))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!stripIsLive)
        }
        .padding(EdgeInsets(top: 18, leading: 16, bottom: 16, trailing: 16))
        .background(Ink.groupRaised, in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                .strokeBorder(Ink.cardBorder, lineWidth: 1)
        )
        .shadow(color: .black.opacity(0.26), radius: 24, y: 14)
    }
}

extension HomeHeader {
    private var stripIcon: String {
        if attentionCount > 0 { return "sparkle" }
        if liveCount > 0 { return "bolt.fill" }
        return "moon.zzz"
    }

    private var stripTint: Color {
        if attentionCount > 0 { return Ink.warn }
        if liveCount > 0 { return Ink.accent }
        return Ink.meta
    }

    private var stripText: String {
        if attentionCount > 0 {
            return attentionCount == 1
                ? String(localized: "1 agent needs you")
                : String(localized: "\(attentionCount) agents need you")
        }
        if liveCount > 0 {
            return liveCount == 1
                ? String(localized: "1 live connection · agents quiet")
                : String(localized: "\(liveCount) live connections · agents quiet")
        }
        if savedCount == 0 { return String(localized: "no hosts yet") }
        return savedCount == 1
            ? String(localized: "1 host saved · all quiet")
            : String(localized: "\(savedCount) hosts saved · all quiet")
    }
}

/// Version line anchoring the bottom of the home scroll. Prefers the
/// `MoshpitBuildStamp` written by scripts/build-ipa.sh (git SHA + time, "+"
/// marking uncommitted changes) so a sideloaded build can say exactly what
/// it is; store builds fall back to version (build).
private struct HomeFooter: View {
    private var line: String {
        let info = Bundle.main.infoDictionary
        let version = info?["CFBundleShortVersionString"] as? String ?? "?"
        if let stamp = info?["MoshpitBuildStamp"] as? String, !stamp.isEmpty {
            return "v\(version) · \(stamp)"
        }
        let build = info?["CFBundleVersion"] as? String ?? "?"
        return "v\(version) (\(build))"
    }

    var body: some View {
        Text(line)
            .font(Face.mono(9.5))
            .kerning(0.6)
            .foregroundStyle(Ink.placeholder)
            .frame(maxWidth: .infinity)
            .padding(.top, 22)
    }
}

private struct HomeSectionLabel: View {
    let title: String
    /// Rides right next to the title as a chip — same voice as the AGENTS /
    /// WORKSPACES headers. A count stranded at the trailing edge read as an
    /// orphan with a gulf of nothing between it and its title.
    var count: Int?
    /// A right-aligned STATUS word (the empty state's READY) — status earns
    /// the trailing slot, counts don't.
    var trailing: String?

    var body: some View {
        HStack(spacing: 7) {
            Text(title)
                .font(Face.mono(10, .bold))
                .kerning(1.5)
                .foregroundStyle(Ink.sectionTitle)
            if let count {
                CountBadge(count: count)
            }
            Spacer()
            if let trailing {
                Text(trailing)
                    .font(Face.mono(10, .bold))
                    .kerning(0.9)
                    .foregroundStyle(Ink.meta)
            }
        }
        .padding(.horizontal, 2)
    }
}

// MARK: - Home utility button

struct HomeIconButton: View {
    let systemImage: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(Ink.accent)
                .frame(width: 38, height: 38)
                .background(Ink.shortcutKeyBG, in: Circle())
                .overlay(Circle().strokeBorder(Ink.accent.opacity(0.20), lineWidth: 1))
        }
        .buttonStyle(.plain)
    }
}

// MARK: - Connection card (§3.1)

struct ConnectionCard: View {
    let connection: ServerConnection
    let active: SessionHub.ActiveSession?
    let metrics: SessionMetrics?
    /// Begin connecting in-place (Home runs prepare+start). Called when the
    /// host head of an offline card is tapped.
    let onConnect: () -> Void
    /// Push the live terminal — from a window/pane row in the tree, or the
    /// "open terminal" row of a connected non-tmux session.
    let onEnter: () -> Void
    let onEdit: () -> Void
    var isDisconnecting: Bool = false
    let onDisconnect: () -> Void
    let onDelete: () -> Void
    /// Tear down and reconnect from scratch — offered on the "attach didn't
    /// complete" notice. Plain `onConnect` is a no-op here: SSH is still
    /// `.connected` (only the tmux handshake stalled), and `start()` only
    /// re-runs from `.idle`, so recovering needs a real disconnect first.
    let onRetryAttach: () -> Void

    /// The island's per-pane clock — the Agents section shows the same
    /// "in this state since" the lock-screen timer runs on, rather than
    /// keeping a second, disagreeing stopwatch.
    @Environment(AgentActivityMonitor.self) private var monitor

    @State private var collapsedSessions: Set<String> = []
    /// Windows whose pane level is expanded in the tree. A set, so several
    /// windows can show their panes at once.
    @State private var expandedWindows: Set<String> = []

    /// Dense but touchable SESSIONS tree rows.
    private static let treeRowMinHeight: CGFloat = 34

    /// In-flight rename: the title to show, the current name pre-filled into the
    /// text field, and the closure that applies the new name through the
    /// (generic) controller. Captured here so the alert lives outside the
    /// generic `sessionsSection` helper.
    @State private var renameTarget: RenameTarget?
    @State private var renameText: String = ""
    /// In-flight kill confirmation: a label for the dialog plus the closure
    /// that performs the kill through the controller.
    @State private var killTarget: KillTarget?
    /// In-flight "new session" naming prompt.
    @State private var newSessionTarget: NewSessionTarget?
    /// Pending New agent task sheet. Boxed like `NewSessionTarget` so the
    /// non-generic sheet can call back into the concrete controller.
    @State private var agentTaskTarget: AgentTaskTarget?
    /// The agent row whose photo button was tapped — the picker and the
    /// attachment sheet both address this pane, not "whatever is active".
    @State private var agentImageTarget: AgentImageTarget?
    @State private var agentImageController: ImageAttachmentController?
    @State private var agentPhotoSelection: [PhotosPickerItem] = []
    @State private var agentPickerPresented = false

    /// A specific pane on this card's connection, as an image destination.
    struct AgentImageTarget: Equatable {
        let paneId: String
        let agentName: String
    }
    /// Pending worktree removal. Two stages: the plain confirm, then — only if
    /// the checkout turns out to be dirty — a second one that says so.
    @State private var worktreeTarget: WorktreeTarget?
    @State private var worktreeForceTarget: WorktreeTarget?
    /// Why a removal didn't happen — herdr's own words, not ours.
    @State private var worktreeError: String?
    @State private var newSessionName: String = ""

    /// Captures a pending rename action from a session/window row. `apply`
    /// closes over the concrete controller so the non-generic alert can run it.
    private struct RenameTarget: Identifiable {
        let id = UUID()
        let title: String
        let currentName: String
        let apply: (String) -> Void
    }

    /// Captures a pending kill action from a session/window/pane row.
    private struct KillTarget: Identifiable {
        let id = UUID()
        let label: String
        let confirmTitle: String
        let perform: () -> Void
    }

    /// Captures a pending "new session", applying the typed name on confirm
    /// (nil = let tmux auto-name).
    private struct NewSessionTarget: Identifiable {
        let id = UUID()
        /// What this multiplexer calls the thing being created — the alert
        /// lives outside the generic tree helper, so the word has to travel
        /// with the target rather than be read off a controller.
        let noun: String
        let apply: (String?) -> Void
    }

    struct WorktreeTarget: Identifiable {
        let id = UUID()
        let sessionName: String
        let repo: String
        let remove: (Bool) async -> HerdrControlClient.WorktreeRemoval
    }

    private struct AgentTaskTarget: Identifiable {
        let id = UUID()
        let initialRepos: [String]
        let load: () async -> (repos: [String], agents: [String])
        let start: (AgentTaskRequest) async -> String?
    }

    private var isLive: Bool { active != nil }

    /// The session exists but its transport is gone (connect failed, or a
    /// keepalive reconnect gave up). Rendering these identically to healthy
    /// cards was the "phone says live, host is dead" trap.
    private var isDead: Bool {
        guard let active else { return false }
        switch active.viewModel.status {
        case .failed, .disconnected: return true
        default: return false
        }
    }

    /// True while a live session is still completing its handshake / tmux
    /// attach — the card shows an in-place spinner instead of pushing an
    /// interim "Attaching…" page. Stops once `attachStalled` latches (the
    /// attach timed out) so the card doesn't spin forever with no way out —
    /// see `isAttachStalled`, which takes over at that point.
    private var isConnecting: Bool {
        guard let active else { return false }
        if case .connecting = active.viewModel.status { return true }
        if case .reconnecting = active.viewModel.status { return true }
        // tmux mode: connected over SSH but the -CC control plane hasn't
        // attached YET (first-time discovery). `everAttached` gates this so a
        // session that later ended (last session killed → server exits →
        // isAttached flips back to false) doesn't re-show "Attaching…" forever.
        if connection.multiplexer == .tmux, let c = active.tmuxControl,
           !c.snapshot.isAttached, !c.snapshot.everAttached, !active.attachStalled {
            if case .connected = active.viewModel.status { return true }
        }
        return false
    }

    /// True once a tmux control-mode attach has gone past
    /// `tmuxAttachTimeoutSeconds` with no `%session-changed` confirming it —
    /// `SessionHub` latches `active.attachStalled` at that point. Shows a
    /// dismissible notice in place of `connectingRow` instead of spinning on
    /// "Attaching session…" with no feedback and no way out. SSH itself is
    /// still fine here (only the tmux handshake stalled), so this is
    /// deliberately distinct from `isDead`.
    private var isAttachStalled: Bool {
        active?.attachStalled ?? false
    }

    private var statusTint: Color {
        if isDead { return Ink.danger }
        if isConnecting || isAttachStalled { return Ink.warn }
        if isLive { return Ink.accent }
        return Ink.meta
    }

    private var statusLabel: String {
        if isDead { return String(localized: "offline") }
        if isConnecting { return String(localized: "linking") }
        if isAttachStalled { return String(localized: "stalled") }
        if isLive { return String(localized: "live") }
        return String(localized: "saved")
    }

    /// Live, fully ready, but with no multiplexer control plane behind it
    /// (plain SSH / mosh): there's no sessions tree to show, so the card
    /// offers a direct "open terminal" row.
    private var isLiveWithoutTree: Bool {
        guard let active, !isConnecting else { return false }
        return active.tmuxControl == nil && active.herdrControl == nil
    }

    var body: some View {
        VStack(spacing: 0) {
            hostHead
            if isConnecting {
                connectingRow
            } else if isDead {
                deadRow
            } else if isAttachStalled {
                attachStalledRow
            } else if let c = active?.tmuxControl {
                // tmux mode: always the sessions section. When attached it shows
                // the tree; after the last session was killed it shows an empty
                // SESSIONS list with "+" to create a fresh one (the SSH shell is
                // still alive), rather than a stuck "Attaching…" spinner.
                sessionsSection(control: c)
            } else if let h = active?.herdrControl {
                // Same section, herdr's tree behind it. Branching here rather
                // than passing an existential keeps `sessionsSection` generic
                // over the concrete controller, which is what preserves
                // Observation tracking on `snapshot`.
                agentsSection(control: h)
                sessionsSection(control: h)
            } else if isLiveWithoutTree {
                openTerminalRow
            }
            if isLive && !isDead {
                bottomBar
            }
        }
        .background(Ink.groupRaised)
        .clipShape(RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
        // Status edge-light: an INSET capsule, not a full-height square-ended
        // bar — the old rail ran past the card's 20pt corners and read as a
        // stray line floating outside it. Only drawn when there IS a status;
        // a saved card carries no light, so live ones stand out more.
        .overlay(alignment: .leading) {
            if isLive || isConnecting || isDead || isAttachStalled {
                Capsule()
                    .fill(statusTint)
                    .frame(width: 3)
                    .padding(.vertical, 16)
                    .padding(.leading, 7)
                    .shadow(color: statusTint.opacity(isLive ? 0.55 : 0.3), radius: 4)
            }
        }
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                .strokeBorder(Ink.cardBorder, lineWidth: 1))
        .shadow(color: .black.opacity(0.24), radius: 18, y: 10)
        // Long-press is the only way to edit/delete a saved-but-offline
        // server (the inline Edit/Disconnect bar only appears when live).
        .contextMenu {
            if isLive && !isDead {
                Button { onEnter() } label: { Label("Open", systemImage: "terminal") }
            } else {
                Button { onConnect() } label: { Label("Connect", systemImage: "terminal") }
            }
            Button { onEdit() } label: { Label("Edit", systemImage: "pencil") }
            if isLive {
                Button { onDisconnect() } label: { Label("Disconnect", systemImage: "bolt.slash") }
            }
            Button(role: .destructive) { onDelete() } label: { Label("Delete Connection", systemImage: "trash") }
        }
        // Rename a session / window from its row long-press.
        .moshpitCard(item: $renameTarget) { target in
            MoshpitInputCard(
                icon: "pencil",
                title: "\(target.title)",
                message: nil,
                placeholder: "Name",
                text: $renameText,
                confirmLabel: "Rename",
                onCancel: { renameTarget = nil },
                onConfirm: {
                    target.apply(renameText)
                    renameTarget = nil
                })
        }
        // Confirm killing a session / window / pane from its row long-press.
        .confirmationDialog(
            killTarget?.confirmTitle ?? "",
            isPresented: Binding(get: { killTarget != nil },
                                 set: { if !$0 { killTarget = nil } }),
            titleVisibility: .visible,
            presenting: killTarget
        ) { target in
            Button(target.label, role: .destructive) {
                target.perform()
                killTarget = nil
            }
            Button("Cancel", role: .cancel) { killTarget = nil }
        }
        // Name a new session before creating it (blank = tmux auto-name).
        .sheet(item: $agentTaskTarget) { target in
            NewAgentTaskSheet(initialRepos: target.initialRepos,
                              load: target.load, start: target.start)
        }
        .photosPicker(isPresented: $agentPickerPresented,
                      selection: $agentPhotoSelection,
                      maxSelectionCount: 10,
                      matching: .images)
        .onChange(of: agentPhotoSelection) { _, items in
            guard !items.isEmpty else { return }
            agentPhotoSelection = []
            beginAgentImageAttachment(items)
        }
        .sheet(isPresented: Binding(
            get: { agentImageController != nil },
            set: { if !$0 { cancelAgentImageAttachment() } }
        )) {
            if let controller = agentImageController, let target = agentImageTarget {
                VStack(spacing: 0) {
                    // Same panel as the terminal's, hosted in a sheet: the
                    // destination line is the one thing the terminal version
                    // never needs to say (there, the pane is on screen).
                    HStack(spacing: 6) {
                        Image(systemName: "sparkle")
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(Ink.accent)
                        Text(verbatim: String(localized: "To \(target.agentName) · \(connection.displayName)"))
                            .font(Face.mono(11, .semibold))
                            .foregroundStyle(Ink.secondary)
                            .lineLimit(1)
                        Spacer(minLength: 0)
                    }
                    .padding(EdgeInsets(top: 14, leading: 20, bottom: 2, trailing: 20))
                    ImageAttachmentOverlayView(
                        controller: controller,
                        onCancel: { cancelAgentImageAttachment() },
                        onRetry: { controller.retry() },
                        onInsert: { finishAgentImageAttachment(submit: false) },
                        onSend: { finishAgentImageAttachment(submit: true) })
                }
                .presentationDetents([.height(230)])
                .presentationBackground(Ink.modalBG)
            }
        }
        .modifier(WorktreeRemovalDialogs(
            target: $worktreeTarget, forceTarget: $worktreeForceTarget, error: $worktreeError))
        .moshpitCard(item: $newSessionTarget) { target in
            MoshpitInputCard(
                icon: "plus",
                title: "New \(target.noun)",
                message: Text("Leave blank for an automatic name."),
                placeholder: "Name (optional)",
                text: $newSessionName,
                confirmLabel: "Create",
                onCancel: { newSessionTarget = nil },
                onConfirm: {
                    let trimmed = newSessionName.trimmingCharacters(in: .whitespacesAndNewlines)
                    target.apply(trimmed.isEmpty ? nil : trimmed)
                    newSessionTarget = nil
                })
        }
    }

    /// Open the rename alert pre-filled with `currentName`, applying through
    /// `apply` on confirm.
    private func beginRename(title: String, currentName: String, apply: @escaping (String) -> Void) {
        renameText = currentName
        renameTarget = RenameTarget(title: title, currentName: currentName, apply: apply)
    }

    // MARK: Host head

    /// Tapping the host head connects an offline card (loading shows in-place);
    /// once live it expands the tree (tmux) or offers the open-terminal row.
    private func headTapped() {
        // A dead card's actions (retry / dismiss) live on its dead row —
        // the head stays inert so a stray tap can't look like a fix.
        if isDead { return }
        if isLive {
            // The trailing chevron promises navigation — honor it. Enters on
            // the active pane, exactly like tapping that pane's tree row.
            Haptics.select()
            onEnter()
            return
        }
        onConnect()
    }

    private var hostHead: some View {
        Button(action: headTapped) {
            HStack(spacing: 13) {
                ZStack(alignment: .bottomTrailing) {
                    RoundedRectangle(cornerRadius: 7, style: .continuous)
                        .fill(Ink.hostIcon)
                        .frame(width: 42, height: 42)
                        .overlay(
                            RoundedRectangle(cornerRadius: 7, style: .continuous)
                                .strokeBorder(Color.white.opacity(0.22), lineWidth: 1)
                        )
                        .overlay(
                            Text(String(connection.displayName.prefix(1)).uppercased())
                                .font(Face.mono(16, .black))
                                .foregroundStyle(Color(hex: "090B0D"))
                        )

                    Circle()
                        .fill(statusTint)
                        .frame(width: 10, height: 10)
                        .overlay(Circle().strokeBorder(Ink.groupRaised, lineWidth: 2))
                        .shadow(color: statusTint.opacity(0.65), radius: 5)
                        .offset(x: 2, y: 2)
                }

                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 7) {
                        Text(connection.displayName)
                            .font(Face.display(18, .semibold))
                            .foregroundStyle(Ink.primary)
                            .lineLimit(1)
                        if connection.connectionProtocol == .mosh {
                            moshHostPill
                        } else {
                            sshHostPill
                        }
                    }

                    HStack(spacing: 7) {
                        Text(statusLabel.uppercased())
                            .font(Face.mono(9, .bold))
                            .kerning(0.8)
                            .foregroundStyle(statusTint)
                        Text(subtitle)
                            .font(Face.mono(11))
                            .foregroundStyle(Ink.meta)
                            .lineLimit(1)
                    }
                }
                Spacer()
                if !isLive {
                    Image(systemName: "power")
                        .font(.system(size: 13, weight: .bold))
                        .foregroundStyle(Ink.accent)
                        .frame(width: 32, height: 32)
                        .background(Ink.accent.opacity(0.10), in: Circle())
                        .overlay(Circle().strokeBorder(Ink.accent.opacity(0.22), lineWidth: 1))
                } else if !isDead {
                    // Navigation affordance — headTapped answers it with
                    // onEnter(). A dead card shows nothing here; promising
                    // a way in on a dead line was the lie.
                    MiniChevron(color: Ink.meta)
                }
            }
            .padding(EdgeInsets(top: 15, leading: 15, bottom: 14, trailing: 14))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("connection-card-\(connection.displayName)")
    }

    @ViewBuilder
    private var moshHostPill: some View {
        if metrics?.moshDegraded == true {
            // Host had no mosh-server: this session fell back to plain SSH.
            // Grey pill + ⚠ so the card honestly says roaming isn't active.
            HStack(spacing: 4) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 7, weight: .bold))
                Text("MOSH")
                    .font(Face.mono(9.5, .bold))
                    .kerning(0.57)
            }
            .foregroundStyle(Ink.meta)
            .padding(EdgeInsets(top: 3, leading: 7, bottom: 3, trailing: 7))
            .background(Color.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 5, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 5, style: .continuous).strokeBorder(Color.white.opacity(0.12), lineWidth: 1))
            .accessibilityIdentifier("mosh-pill-degraded")
        } else {
            HStack(spacing: 4) {
                Circle().fill(Ink.mosh).frame(width: 5, height: 5)
                Text("MOSH")
                    .font(Face.mono(9.5, .bold))
                    .kerning(0.57)
            }
            .foregroundStyle(Ink.moshHostPillText)
            .padding(EdgeInsets(top: 3, leading: 7, bottom: 3, trailing: 7))
            .background(Ink.moshHostPillBG, in: RoundedRectangle(cornerRadius: 5, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 5, style: .continuous).strokeBorder(Ink.mosh.opacity(0.24), lineWidth: 1))
        }
    }

    /// SSH counterpart of `moshHostPill`, so every card names its protocol —
    /// neutral grey (vs mosh's signal blue) keeps the roaming-capable
    /// transport visually distinct without leaving SSH cards unlabeled.
    private var sshHostPill: some View {
        HStack(spacing: 4) {
            Circle().fill(Ink.meta).frame(width: 5, height: 5)
            Text("SSH")
                .font(Face.mono(9.5, .bold))
                .kerning(0.57)
        }
        .foregroundStyle(Ink.secondary)
        .padding(EdgeInsets(top: 3, leading: 7, bottom: 3, trailing: 7))
        .background(Color.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 5, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 5, style: .continuous).strokeBorder(Color.white.opacity(0.12), lineWidth: 1))
    }

    private var subtitle: String {
        var parts = ["\(connection.username)@\(connection.host)"]
        if connection.connectionProtocol == .mosh {
            parts.append("UDP \(connection.moshPortRangeStart)")
        }
        if let connectedAt = metrics?.connectedAt {
            let hours = max(1, Int(Date().timeIntervalSince(connectedAt) / 3600))
            parts.append(metrics?.isRoaming == true
                ? String(localized: "\(hours)h roamed")
                : String(localized: "\(hours)h up"))
        }
        return parts.joined(separator: " · ")
    }

    // MARK: Agents

    /// One row per agent that is doing something, above the tree.
    ///
    /// This is a PROJECTION of the same snapshot the tree and the Vibe Island
    /// read — no extra polling, no second store. It exists because the tree
    /// answers "what is running where" while the question people actually
    /// arrive with is "who is waiting on me", and the tree makes that a
    /// four-level scavenger hunt.
    ///
    /// Hidden entirely when nothing is happening, so a herdr connection with
    /// no agents looks exactly like it did before this shipped.
    ///
    /// herdr only, for now: its `agent_status` is a protocol field that covers
    /// every pane. tmux's equivalent is only as complete as the hooks the user
    /// installed, and a list that silently omits half your agents is worse
    /// than no list.
    @ViewBuilder
    private func agentsSection<C: MultiplexerControlling>(control: C) -> some View {
        let entries = Self.agentEntries(
            snapshot: control.snapshot,
            hooks: control.agentHooks,
            since: monitor.stateSinceByPane(connectionId: connection.id))
        // The header stays even with nothing running, because it carries the
        // `+` — hiding the section outright made the app's most valuable
        // action impossible to reach until an agent already existed.
        if control.snapshot.isAttached {
            VStack(alignment: .leading, spacing: 0) {
                Rectangle().fill(Ink.cardDivider).frame(height: 1)
                    .padding(.bottom, 10)

                HStack {
                    HStack(spacing: 7) {
                        Text("AGENTS")
                            .font(Face.mono(10, .bold))
                            .kerning(1.7)
                            .foregroundStyle(Ink.sectionTitle)
                        CountBadge(count: entries.count)
                    }
                    Spacer()
                    HStack(spacing: 8) {
                        // "NEEDS YOU" — with the count once there's more than
                        // one, because "2 NEED YOU" and "one of several needs
                        // you" are different amounts of trouble.
                        let waiting = entries.filter { $0.signal == .attention }.count
                        // A tinted capsule, not bare text — this is the single
                        // most important signal on the screen and it was
                        // dressing like a section label.
                        if waiting >= 1 {
                            Text(verbatim: waiting == 1
                                ? AgentSignal.attention.label.uppercased()
                                : String(localized: "\(waiting) NEED YOU"))
                                .font(Face.mono(10, .bold))
                                .kerning(1.2)
                                .foregroundStyle(Ink.warn)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(Ink.warn.opacity(0.12), in: Capsule())
                                .overlay(Capsule().strokeBorder(Ink.warn.opacity(0.28), lineWidth: 1))
                        }
                        newAgentTaskButton(control: control)
                    }
                }
                .padding(.bottom, 6)

                if entries.isEmpty {
                    Text("Nothing running — start a task to isolate one")
                        .font(Face.mono(11))
                        .foregroundStyle(Ink.meta)
                        .padding(.vertical, 6)
                }

                ForEach(entries) { entry in
                    agentRow(entry, isActive: entry.paneId == control.snapshot.activePaneId) {
                        // Exactly what a tree pane row does — same selection
                        // call, same destination. One way in.
                        Haptics.select()
                        control.selectPane(entry.paneId)
                        onEnter()
                    }
                }
            }
            .padding(EdgeInsets(top: 10, leading: 14, bottom: 0, trailing: 14))
        }
    }

    // MARK: Per-agent image attach (the "send it a picture" entry that
    // never opens the terminal — upload over this card's channel, deliver
    // the path to THAT pane, in that agent's dialect).

    private func beginAgentImageAttachment(_ items: [PhotosPickerItem]) {
        guard let active, agentImageTarget != nil else { return }
        let controller = ImageAttachmentController(uploaderProvider: { [weak active] in
            guard let active else { throw SSHError.sessionClosed }
            return try await active.acquireFileTransferSSH()
        })
        controller.onReady = { [weak active] paths in
            guard let active else { return [] }
            return paths.map { active.imageUploads.record(remotePath: $0).number }
        }
        agentImageController = controller
        Task {
            var picked: [Data] = []
            for item in items {
                if let data = try? await item.loadTransferable(type: Data.self) {
                    picked.append(data)
                }
            }
            guard !picked.isEmpty else {
                agentImageController = nil
                return
            }
            controller.start(pickedData: picked)
        }
    }

    private func finishAgentImageAttachment(submit: Bool) {
        guard let active, let target = agentImageTarget,
              let controller = agentImageController,
              let text = controller.insertText(style: .forAgent(target.agentName))
        else { return }
        Task {
            let delivered = await active.deliverPaste(text, toPane: target.paneId)
            if delivered, submit {
                _ = await active.deliverInput(Data([0x0D]), toPane: target.paneId)
            }
            if delivered { Haptics.success() }
        }
        agentImageController = nil
        agentImageTarget = nil
    }

    private func cancelAgentImageAttachment() {
        agentImageController?.cancel()
        agentImageController = nil
        agentImageTarget = nil
    }

    /// One agent, in the same row grammar as the tree below: identity +
    /// status in the middle, an explicit enter affordance on the right.
    /// needs-you is the only badge; every other state is a dot and the meta
    /// line. The amber OPEN button and the edge-light make the row the card's
    /// loudest element exactly when it should be.
    private func agentRow(_ entry: AgentEntry, isActive: Bool,
                          action: @escaping () -> Void) -> some View {
        let attention = entry.signal == .attention
        return Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: "sparkle")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(attention ? Ink.warn : Ink.secondary)
                    .frame(width: 30, height: 30)
                    .background(attention ? Ink.warn.opacity(0.12) : Ink.neutralFill,
                                in: RoundedRectangle(cornerRadius: 7, style: .continuous))

                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(entry.name)
                            .font(Face.text(14, .semibold))
                            .foregroundStyle(Ink.primary)
                            .lineLimit(1)
                        if attention { needsYouMini }
                    }
                    Text(verbatim: agentMeta(entry))
                        .font(Face.mono(10.5))
                        .foregroundStyle(Ink.meta)
                        .lineLimit(1)
                }

                Spacer()

                // Hand this agent a picture without opening the terminal —
                // picker → upload over this card's channel → path delivered
                // to THIS pane. Nested button; the row's own tap still enters.
                Button {
                    Haptics.tap()
                    agentImageTarget = AgentImageTarget(paneId: entry.paneId,
                                                        agentName: entry.name)
                    agentPickerPresented = true
                } label: {
                    Image(systemName: "photo")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Ink.secondary)
                        .frame(width: 26, height: 26)
                        .background(Ink.neutralFill,
                                    in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text(verbatim: String(localized: "Send image to \(entry.name)")))

                if attention {
                    Text("OPEN")
                        .font(Face.mono(11, .bold))
                        .kerning(0.6)
                        .foregroundStyle(Ink.warn)
                        .padding(.horizontal, 11)
                        .padding(.vertical, 6)
                        .background(Ink.warn.opacity(0.10),
                                    in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 7, style: .continuous)
                            .strokeBorder(Ink.warn.opacity(0.30), lineWidth: 1))
                } else {
                    if let signal = entry.signal {
                        Circle()
                            .fill(signal.color)
                            .frame(width: 7, height: 7)
                            .shadow(color: signal.color.opacity(0.7), radius: 3.5)
                    }
                    enterGlyph(active: isActive)
                }
            }
            .padding(EdgeInsets(top: 7, leading: 8, bottom: 7, trailing: 8))
            .frame(minHeight: 48)
            .background(isActive ? Ink.rowActiveBG : .clear,
                        in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
            .overlay(alignment: .leading) {
                if attention {
                    Capsule().fill(Ink.warn).frame(width: 3).padding(.vertical, 9)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(verbatim: [
            entry.name,
            entry.signal?.label ?? String(localized: "idle"),
            entry.location,
        ].filter { !$0.isEmpty }.joined(separator: ", ")))
    }

    /// "work · api · 2m" — where the agent lives, then for how long.
    private func agentMeta(_ entry: AgentEntry) -> String {
        var parts = entry.location.isEmpty ? [] : [entry.location]
        if entry.signal == nil {
            parts.append(String(localized: "idle"))
        } else if let elapsed = Self.elapsedLabel(since: entry.since) {
            parts.append(elapsed)
        }
        return parts.joined(separator: " · ")
    }

    /// `+` on the AGENTS header — the same affordance the tree's header uses
    /// for "new session", one level up in meaning: a whole isolated task.
    private func newAgentTaskButton<C: MultiplexerControlling>(control: C) -> some View {
        Button {
            Haptics.tap()
            guard let herdr = control as? HerdrControlClient else { return }
            // Present immediately with the directories the panes are already
            // in; the sheet scans for the rest itself. Waiting ~1s on a dead
            // button before anything appears is worse than a list that fills in.
            agentTaskTarget = AgentTaskTarget(
                initialRepos: herdr.openRepoGuesses,
                load: { await (repos: herdr.gitRepos(), agents: herdr.agentNames()) },
                start: { request in await herdr.startAgentTask(request) })
        } label: {
            Image(systemName: "plus")
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(Ink.accent)
                .frame(width: 26, height: 26)
                .background(Ink.accent.opacity(0.12),
                            in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("new-agent-task")
    }

    /// A pane with an agent in it, ready to render.
    struct AgentEntry: Identifiable, Equatable {
        let paneId: String
        let name: String
        let location: String
        /// nil = idle: an agent the multiplexer knows BY NAME, sitting at its
        /// prompt. No dot, no urgency — but a row, because "no agents" and
        /// "an agent waiting for work" are different answers.
        let signal: AgentSignal?
        /// When the agent entered its current state — the island's clock, so
        /// the row and the lock screen never disagree about "for how long".
        let since: Date?
        var id: String { paneId }
    }

    /// Pure so the ordering — the whole point of the section — is testable
    /// without a live connection.
    ///
    /// A row needs a REASON: a live signal, or an explicit `idle` stamp with
    /// an agent name on it (herdr 0.8+ reports those; 0.7.3 reports neither
    /// field, so it simply never grows idle rows). An anonymous idle pane is
    /// nothing — rendering every quiet shell would drown the section.
    static func agentEntries(snapshot: TmuxSnapshot,
                             hooks: [String: AgentHook],
                             since: [String: Date] = [:]) -> [AgentEntry] {
        hooks.compactMap { paneId, hook -> AgentEntry? in
            guard let pane = snapshot.panes[paneId] else { return nil }
            let signal = AgentSignal(hook.state)
            let namedAgent = hook.agent.flatMap { $0.isEmpty ? nil : $0 }
            guard signal != nil || (hook.state == "idle" && namedAgent != nil) else { return nil }
            let window = snapshot.windows[pane.windowId]
            let session = window.flatMap { snapshot.sessions[$0.sessionId] }
            // herdr names the agent; tmux hooks may too. Falling back to the
            // foreground command keeps the row meaningful either way, and
            // herdr 0.7.3 reports neither for a pane it hasn't detected.
            let name = namedAgent
                ?? (pane.command.isEmpty
                    ? String(localized: "agent")
                    : normalizedCommand(pane.command))
            let location = [session?.name, window?.name]
                .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
            return AgentEntry(paneId: paneId, name: name, location: location,
                              signal: signal,
                              // The hook's own stamp (tmux writes one) beats
                              // the monitor's first-seen; idle rows show no
                              // duration at all rather than a made-up one.
                              since: signal == nil ? nil : (hook.since ?? since[paneId]))
        }
        // Whoever is waiting on a human comes first, idle agents last; ties
        // broken by name so the list doesn't reshuffle itself between polls.
        .sorted { (rank($0.signal), $0.name, $0.paneId) < (rank($1.signal), $1.name, $1.paneId) }
    }

    private static func rank(_ signal: AgentSignal?) -> Int {
        signal?.rank ?? Int.max
    }

    /// The most urgent agent signal anywhere inside `sessionId` — what a
    /// collapsed session's summary line leads with.
    private static func strongestSignal(inSession sessionId: String,
                                        snapshot: TmuxSnapshot,
                                        hooks: [String: AgentHook]) -> AgentSignal? {
        hooks.compactMap { paneId, hook -> AgentSignal? in
            guard let pane = snapshot.panes[paneId],
                  let window = snapshot.windows[pane.windowId],
                  window.sessionId == sessionId else { return nil }
            return AgentSignal(hook.state)
        }
        .min { $0.rank < $1.rank }
    }

    /// True when some pane of `windowId` has an agent waiting on a human —
    /// the tree row echoes the AGENTS section's amber so the two sections
    /// point at the same trouble.
    private static func attentionInWindow(_ windowId: String,
                                          snapshot: TmuxSnapshot,
                                          hooks: [String: AgentHook]) -> Bool {
        hooks.contains { paneId, hook in
            guard let pane = snapshot.panes[paneId],
                  pane.windowId == windowId else { return false }
            return AgentSignal(hook.state) == .attention
        }
    }

    /// The one element allowed to shout: a small amber capsule reading
    /// NEEDS YOU. Everything else on these rows is a dot and a word.
    private var needsYouMini: some View {
        Text(verbatim: AgentSignal.attention.label.uppercased())
            .font(Face.mono(8.5, .bold))
            .kerning(0.8)
            .foregroundStyle(Ink.warn)
            .padding(.horizontal, 6)
            .padding(.vertical, 2.5)
            .background(Ink.warn.opacity(0.12), in: Capsule())
            .overlay(Capsule().strokeBorder(Ink.warn.opacity(0.28), lineWidth: 1))
    }

    /// The explicit "this enters the terminal" affordance — a quiet icon
    /// button, identical on every enterable row, so entry stops being an
    /// invisible whole-row behavior.
    private func enterGlyph(active: Bool) -> some View {
        Image(systemName: "arrow.up.right")
            .font(.system(size: 10, weight: .bold))
            .foregroundStyle(active ? Ink.accent : Ink.meta)
            .frame(width: 26, height: 26)
            .background(active ? Ink.accent.opacity(0.10) : Color.white.opacity(0.045),
                        in: RoundedRectangle(cornerRadius: 7, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 7, style: .continuous)
                .strokeBorder(active ? Ink.accent.opacity(0.28) : Ink.hairline, lineWidth: 1))
    }

    /// Disambiguated session names arrive as "name · id" (several unlabeled
    /// herdr workspaces are all called "~", so the id is appended). Rendered
    /// as-is that's a run of glyphs — "~ · w9" — posing as a title. Split it:
    /// the ID is the real identity and becomes the title; the name becomes a
    /// proper label chip, and the placeholder "~" (zero information when
    /// every unlabeled workspace carries it) is dropped outright.
    private static func splitDisplayName(_ display: String) -> (title: String, label: String?) {
        guard let range = display.range(of: " · ", options: .backwards),
              range.upperBound < display.endIndex else { return (display, nil) }
        let name = String(display[..<range.lowerBound])
        let id = String(display[range.upperBound...])
        let label = (name.isEmpty || name == "~") ? nil : name
        return (id, label)
    }

    /// tmux reports the foreground process's comm name, and Claude Code sets
    /// a process title that STARTS WITH ITS VERSION — the comm arrives as
    /// "2.1.222", which no icon map and no human recognizes. Normalize before
    /// icon lookup and display: anything containing "claude" is claude, and a
    /// bare semver comm is claude too (nothing else in a terminal names its
    /// process a version string).
    private static func normalizedCommand(_ command: String) -> String {
        let lowered = command.lowercased()
        if lowered.contains("claude") { return "claude" }
        if lowered.range(of: #"^\d+\.\d+\.\d+$"#, options: .regularExpression) != nil {
            return "claude"
        }
        return command
    }

    /// A pane's leading icon — what's RUNNING, at a glance. Three shells and
    /// a vim all read as bare words without this; the tile also gives every
    /// pane row the same front anchor the agent rows have.
    private static func paneIcon(for command: String) -> String {
        switch command.lowercased() {
        case "claude", "codex", "aider", "agy": return "sparkle"
        case "vim", "nvim", "vi", "nano", "emacs", "hx": return "square.and.pencil"
        case "git", "lazygit", "tig": return "arrow.triangle.branch"
        case "ssh", "mosh", "mosh-client": return "network"
        case "top", "htop", "btop": return "chart.bar"
        case "docker", "kubectl", "podman": return "shippingbox"
        case "tail", "less", "bat", "more": return "doc.text"
        case "node", "python", "python3", "ruby", "cargo", "go",
             "swift", "make", "npm", "pnpm", "bun", "yarn": return "hammer"
        default: return "terminal"
        }
    }

    /// Collapsed-session summary chip: a tab name (capped) or a "+N" tail.
    private func tabChip(_ text: String) -> some View {
        Text(text)
            .font(Face.mono(9.5, .medium))
            .foregroundStyle(Ink.tertiary)
            .lineLimit(1)
            // No flexible frame: `.frame(maxWidth:)` GREEDILY expands to its
            // cap, which blew "p1" up to an 88pt slab. The chip hugs its
            // text; long names truncate under HStack compression instead.
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(Color.white.opacity(0.05),
                        in: RoundedRectangle(cornerRadius: 5, style: .continuous))
    }

    /// "2m" / "1h 12m", minutes at the finest — the poll behind this data is
    /// up to 8s coarse, and printing seconds would claim a precision the
    /// number doesn't have. Under a minute reads "now".
    static func elapsedLabel(since: Date?, now: Date = Date()) -> String? {
        guard let since else { return nil }
        let minutes = Int(now.timeIntervalSince(since)) / 60
        guard minutes >= 1 else { return String(localized: "now") }
        guard minutes >= 60 else { return "\(minutes)m" }
        return "\(minutes / 60)h \(minutes % 60)m"
    }

    // MARK: Sessions tree

    private func sessionsSection<C: MultiplexerControlling>(control: C) -> some View {
        let snapshot = control.snapshot
        let sessions = snapshot.sessions.values.sorted { $0.id < $1.id }

        return VStack(alignment: .leading, spacing: 0) {
            Rectangle().fill(Ink.cardDivider).frame(height: 1)
                .padding(.bottom, 10)

            HStack {
                HStack(spacing: 7) {
                    Text(verbatim: control.multiplexer.vocabulary.sessionPlural.uppercased())
                        .font(Face.mono(10, .bold))
                        .kerning(1.7)
                        .foregroundStyle(Ink.sectionTitle)
                    CountBadge(count: sessions.count)
                }
                Spacer()
                HStack(spacing: 8) {
                    Button {
                        Haptics.tap()
                        newSessionName = ""
                        newSessionTarget = NewSessionTarget(noun: control.multiplexer.vocabulary.session) { control.newSession(named: $0) }
                    } label: {
                        Image(systemName: "plus")
                            .font(.system(size: 12, weight: .bold))
                            .foregroundStyle(Ink.accent)
                            .frame(width: 26, height: 26)
                            .background(Ink.accent.opacity(0.10), in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                    }
                    Button { control.refresh() } label: {
                        Group {
                            if control.isRefreshing {
                                ProgressView()
                                    .controlSize(.mini)
                                    .tint(Ink.accent)
                            } else {
                                Image(systemName: "arrow.clockwise")
                                    .font(.system(size: 11, weight: .bold))
                                    .foregroundStyle(Ink.accent)
                            }
                        }
                        .frame(width: 26, height: 26)
                        .background(Color.white.opacity(0.055), in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                    }
                    .disabled(control.isRefreshing)     // no duplicate in-flight refreshes
                    .sensoryFeedback(trigger: control.isRefreshing) { _, now in
                        now ? .impact(weight: .light) : nil   // a tick when it starts
                    }
                }
                .buttonStyle(.plain)
            }
            .padding(.bottom, 8)

            VStack(alignment: .leading, spacing: 4) {
                if let herdr = control as? HerdrControlClient,
                   let mismatch = herdr.protocolMismatch {
                    // Version skew: every command is being refused, so "No
                    // workspaces yet" would be a lie and the + button a trap
                    // (its create fails silently). Name the problem and offer
                    // the one remedy, same as the terminal banner.
                    herdrMismatchRow(mismatch, client: herdr)
                } else if sessions.isEmpty {
                    sessionsEmptyRow(control.multiplexer.vocabulary)
                } else {
                    ForEach(sessions) { session in
                        sessionRow(session, snapshot: snapshot, control: control)
                        if !collapsedSessions.contains(session.id) {
                            ForEach(windows(in: session, snapshot: snapshot)) { window in
                                windowRow(window, snapshot: snapshot, control: control)
                                if expandedWindows.contains(window.id) {
                                    ForEach(snapshot.panes(inWindow: window.id)) { pane in
                                        paneRow(pane, snapshot: snapshot, control: control)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        .padding(EdgeInsets(top: 0, leading: 14, bottom: 12, trailing: 14))
    }

    /// The last tmux-only string on this screen: a herdr connection with an
    /// empty tree was telling the user "No tmux sessions", which names the
    /// wrong program AND the wrong noun (herdr has workspaces).
    /// herdr refused every command over client/server version skew — the row
    /// says so and carries the one-tap remedy (same consent story as the
    /// terminal banner: restarting exits pane processes, so it always asks).
    private func herdrMismatchRow(_ message: String, client: HerdrControlClient) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(Ink.warn)
            Text(message)
                .font(Face.mono(10))
                .foregroundStyle(Ink.meta)
                .lineLimit(3)
            Spacer(minLength: 8)
            Button {
                Task { await client.restartServer() }
            } label: {
                Group {
                    if client.isRestartingServer {
                        ProgressView().controlSize(.mini).tint(Ink.accent)
                    } else {
                        Text("Restart server")
                            .font(Face.mono(10, .semibold))
                            .foregroundStyle(Ink.accent)
                    }
                }
                .padding(.horizontal, 10).padding(.vertical, 6)
                .background(Ink.accent.opacity(0.12), in: Capsule())
            }
            .buttonStyle(.plain)
            .disabled(client.isRestartingServer)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(Color.white.opacity(0.035), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
    }

    private func sessionsEmptyRow(_ vocab: MultiplexerVocabulary) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "square.stack.3d.up.slash")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(Ink.meta)
            Text(verbatim: String(localized: "No \(vocab.sessionPlural.lowercased()) yet"))
                .font(Face.mono(11, .semibold))
                .foregroundStyle(Ink.meta)
            Spacer()
        }
        .padding(.horizontal, 10)
        .frame(minHeight: 34)
        .background(Color.white.opacity(0.035), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
    }

    private func windows(in session: SessionInfo, snapshot: TmuxSessionController.Snapshot) -> [WindowInfo] {
        // Discovery loads every session's windows (`list-windows -a`), so each
        // session shows its own — several can be expanded at once.
        snapshot.windows(inSession: session.id)
    }

    private func sessionRow<C: MultiplexerControlling>(_ session: SessionInfo,
                            snapshot: TmuxSnapshot,
                            control: C) -> some View {
        let isActive = session.id == snapshot.activeSessionId
        let expanded = !collapsedSessions.contains(session.id)
        let windowCount = snapshot.windows(inSession: session.id).count

        return Button {
            // Tapping any session expands/collapses its windows; several can be
            // open at once. Switching the attached session happens when you
            // enter one of its windows/panes.
            if expanded { collapsedSessions.insert(session.id) }
            else { collapsedSessions.remove(session.id) }
        } label: {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 8) {
                    // The chevron is the row's ONLY leading symbol — the old
                    // status dot next to it was redundant with the violet name
                    // and the OPEN button, and together with herdr's "~ ·"
                    // path prefix the row opened on four glyphs in a row.
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(isActive ? Ink.accent : Ink.meta)
                        .frame(width: 16, height: 16)
                    let parts = Self.splitDisplayName(snapshot.sessionDisplayName(session))
                    Text(verbatim: parts.title)
                        .font(Face.mono(12, .semibold))
                        .foregroundStyle(isActive ? Ink.accent : Ink.primary)
                        .lineLimit(1)
                    if let label = parts.label {
                        tabChip(label)
                    }
                    if windowCount > 0 {
                        let vocab = control.multiplexer.vocabulary
                        let noun = windowCount == 1 ? vocab.window : vocab.windowPlural
                        Text(verbatim: "\(windowCount) \(noun.lowercased())")
                            .font(Face.mono(10, .medium))
                            .foregroundStyle(Ink.meta)
                            .padding(.horizontal, 4)
                            .padding(.vertical, 2)
                            .background(Color.white.opacity(0.05),
                                        in: RoundedRectangle(cornerRadius: 4, style: .continuous))
                    }
                    Spacer()
                    // The attached session carries the card's one violet text
                    // button — the shortest path back into the live terminal.
                    if isActive {
                        Button {
                            Haptics.select()
                            onEnter()
                        } label: {
                            Text("OPEN")
                                .font(Face.mono(11, .bold))
                                .kerning(0.6)
                                .foregroundStyle(Ink.accent)
                                .padding(.horizontal, 11)
                                .padding(.vertical, 5)
                                .background(Ink.accent.opacity(0.12),
                                            in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                                .overlay(RoundedRectangle(cornerRadius: 7, style: .continuous)
                                    .strokeBorder(Ink.accent.opacity(0.32), lineWidth: 1))
                        }
                        .buttonStyle(.plain)
                    }
                }
                .frame(minHeight: Self.treeRowMinHeight)

                // Collapsed ≠ hidden: the row keeps a one-line summary — up to
                // three tab names, an overflow tail, and the loudest agent
                // signal — so folding a tall tree never folds away trouble.
                if !expanded, windowCount > 0 {
                    let names = windows(in: session, snapshot: snapshot)
                        .map { $0.displayTitle(control.multiplexer.vocabulary) }
                    HStack(spacing: 5) {
                        ForEach(Array(names.prefix(3).enumerated()), id: \.offset) { _, name in
                            tabChip(name)
                        }
                        if names.count > 3 { tabChip("+\(names.count - 3)") }
                        if Self.strongestSignal(inSession: session.id, snapshot: snapshot,
                                                hooks: control.agentHooks) == .attention {
                            needsYouMini
                        }
                        Spacer()
                    }
                    .padding(.leading, 24)
                    .padding(.bottom, 9)
                }
            }
            .padding(.horizontal, 9)
            // Parent rows sit visibly heavier than their children — the tree's
            // levels shouldn't need the indent alone to read.
            .background(
                isActive ? Ink.rowActiveBG : Color.white.opacity(0.05),
                in: RoundedRectangle(cornerRadius: 7, style: .continuous))
            .overlay {
                if isActive {
                    RoundedRectangle(cornerRadius: 7, style: .continuous)
                        .strokeBorder(Ink.accent.opacity(0.24), lineWidth: 1)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            Button {
                beginRename(title: String(localized: "Rename \(control.multiplexer.vocabulary.session)"),
                            currentName: session.name) { control.renameSession(session.id, to: $0) }
            } label: { Label("Rename", systemImage: "pencil") }
            if let repo = control.worktreeRepo(for: session.id),
               let herdr = control as? HerdrControlClient {
                // Only a session that IS a checkout can be removed as a task;
                // an ordinary workspace has no directory to delete.
                Button(role: .destructive) {
                    worktreeTarget = WorktreeTarget(
                        sessionName: session.name, repo: repo,
                        remove: { force in
                            await herdr.removeWorktree(sessionId: session.id, force: force)
                        })
                } label: { Label("Remove Worktree", systemImage: "trash.slash") }
            }
            Button(role: .destructive) {
                let vocab = control.multiplexer.vocabulary
                killTarget = KillTarget(
                    label: "\(vocab.killVerb) \(vocab.session)",
                    confirmTitle: String(localized: "\(vocab.killVerb) \(vocab.session.lowercased()) \"\(snapshot.sessionDisplayName(session))\"?")
                ) { control.killSession(session.id) }
            } label: {
                Label("\(control.multiplexer.vocabulary.killVerb) \(control.multiplexer.vocabulary.session)",
                      systemImage: "xmark.circle")
            }
        }
    }

    private func windowRow<C: MultiplexerControlling>(_ window: WindowInfo,
                           snapshot: TmuxSnapshot,
                           control: C) -> some View {
        let isActive = window.id == snapshot.activeWindowId
        let panes = window.paneCount
        let expanded = expandedWindows.contains(window.id)

        return HStack(spacing: 4) {
            // Hierarchy is pure indentation — the old rail-and-tick glyphs
            // read as ASCII art (├—) no matter how thinly they were drawn.
            Color.clear.frame(width: 26)

            // Disclosure for the pane level — only multi-pane windows expand.
            if panes > 1 {
                Button {
                    if expanded { expandedWindows.remove(window.id) }
                    else { expandedWindows.insert(window.id) }
                } label: {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(Ink.meta)
                        .frame(width: 20, height: Self.treeRowMinHeight)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            } else {
                Color.clear.frame(width: 20, height: Self.treeRowMinHeight)
            }

            Button {
                Haptics.select()
                control.selectWindow(window.id)
                onEnter()
            } label: {
                HStack(spacing: 5) {
                    // No leading marker: the rail's L-tick already points at
                    // the row, activity is the violet name + enter glyph.
                    Text(verbatim: window.displayTitle(control.multiplexer.vocabulary))
                        .font(Face.mono(12, .medium))
                        .foregroundStyle(isActive ? Ink.accent : Ink.primary)
                        .lineLimit(1)
                    Text(verbatim: "\(panes)p")
                        .font(Face.mono(10, .bold))
                        .foregroundStyle(Ink.meta)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 2)
                        .background(Color.white.opacity(0.05), in: RoundedRectangle(cornerRadius: 4, style: .continuous))
                    // Echo the AGENTS section's amber on the exact window that
                    // holds the waiting agent — the two sections point at the
                    // same trouble.
                    if Self.attentionInWindow(window.id, snapshot: snapshot,
                                              hooks: control.agentHooks) {
                        needsYouMini
                    }
                    Spacer()
                    enterGlyph(active: isActive)
                }
                .padding(.horizontal, 8)
                .frame(minHeight: Self.treeRowMinHeight)
                .background(
                    isActive ? Ink.rowActiveBG : .clear,
                    in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .contextMenu {
            Button {
                beginRename(title: String(localized: "Rename \(control.multiplexer.vocabulary.window)"),
                            currentName: window.name) { control.renameWindow(window.id, to: $0) }
            } label: { Label("Rename", systemImage: "pencil") }
            Button(role: .destructive) {
                let vocab = control.multiplexer.vocabulary
                killTarget = KillTarget(
                    label: "\(vocab.killVerb) \(vocab.window)",
                    confirmTitle: String(localized: "\(vocab.killVerb) \(vocab.window.lowercased()) \"\(window.displayTitle(vocab))\"?")
                ) { control.killWindow(window.id) }
            } label: {
                Label("\(control.multiplexer.vocabulary.killVerb) \(control.multiplexer.vocabulary.window)",
                      systemImage: "xmark.circle")
            }
            // Pane level: tmux panes have no name, so only Kill is offered.
            // The active pane of this window is the one Moshpit shows full-screen.
            if let pane = activePane(of: window, snapshot: snapshot) {
                Divider()
                Button(role: .destructive) {
                    killTarget = KillTarget(
                        label: "\(control.multiplexer.vocabulary.killVerb) \(String(localized: "Pane"))",
                        confirmTitle: String(localized: "\(control.multiplexer.vocabulary.killVerb) pane \(pane.index)?")
                    ) { control.killPane(pane.id) }
                } label: { Label("\(control.multiplexer.vocabulary.killVerb) \(String(localized: "Pane"))", systemImage: "rectangle.split.2x1") }
            }
        }
    }

    /// A pane row — the third tree level, shown when its window is expanded.
    /// Tap selects the pane and enters the terminal on it.
    private func paneRow<C: MultiplexerControlling>(_ pane: PaneInfo,
                         snapshot: TmuxSnapshot,
                         control: C) -> some View {
        let isActive = pane.id == snapshot.activePaneId
        let label = pane.command.isEmpty ? "shell" : Self.normalizedCommand(pane.command)

        return Button {
            Haptics.select()
            control.selectPane(pane.id)
            onEnter()
        } label: {
            HStack(spacing: 8) {
                // Third level: indentation only.
                Color.clear.frame(width: 46)
                // Order and identity live at the FRONT: icon tile (what's
                // running) then the index chip — a row of same-named shells
                // was unreadable with the number trailing at ragged x.
                Image(systemName: Self.paneIcon(for: label))
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(isActive ? Ink.accent : Ink.secondary)
                    .frame(width: 24, height: 24)
                    .background(isActive ? Ink.accent.opacity(0.12) : Ink.neutralFill,
                                in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                tabChip("p\(pane.index)")
                Text(verbatim: label)
                    .font(Face.mono(12, .medium))
                    .foregroundStyle(isActive ? Ink.accent : Ink.primary)
                    .lineLimit(1)
                Spacer()
                enterGlyph(active: isActive)
            }
            .padding(.trailing, 8)
            .frame(minHeight: Self.treeRowMinHeight)
            .background(
                isActive ? Ink.rowActiveBG : .clear,
                in: RoundedRectangle(cornerRadius: 7, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            Button(role: .destructive) {
                killTarget = KillTarget(
                    label: "\(control.multiplexer.vocabulary.killVerb) \(String(localized: "Pane"))",
                    confirmTitle: String(localized: "Kill pane \(pane.index)?")
                ) { control.killPane(pane.id) }
            } label: { Label("\(control.multiplexer.vocabulary.killVerb) \(String(localized: "Pane"))", systemImage: "rectangle.split.2x1") }
        }
    }

    /// The active pane within `window` (the one Moshpit renders full-screen),
    /// used to offer a pane-level Kill in the window long-press menu.
    private func activePane(of window: WindowInfo, snapshot: TmuxSnapshot) -> PaneInfo? {
        let panes = snapshot.panes.values.filter { $0.windowId == window.id }
        // Prefer the active pane; otherwise the lowest-indexed one. Sort by the
        // numeric index, not the "%N" id string (which orders %0 < %10 < %2).
        return panes.first(where: { $0.isActive }) ?? panes.sorted { $0.index < $1.index }.first
    }

    // MARK: In-place loading row

    /// Shown on the card while the session is mid-handshake / tmux attach —
    /// this replaces the old pushed "Connecting…" / "Attaching tmux…" page.
    private var connectingLabel: String {
        if case .reconnecting = active?.viewModel.status {
            return String(localized: "Reconnecting…")
        }
        // While the SSH handshake is still in flight, say so — "Attaching
        // session…" here made an unreachable host read as a tmux problem
        // (the tmux attach only begins once status is .connected).
        if case .connecting = active?.viewModel.status {
            return String(localized: "Connecting…")
        }
        // Only tmux has an attach to wait on — its -CC control plane confirms
        // with %session-changed. herdr renders its own TUI with no control
        // plane yet, so there's nothing to attach and "Connecting…" is honest.
        return connection.multiplexer == .tmux
            ? String(localized: "Attaching session…")
            : String(localized: "Connecting…")
    }

    /// The same colour the Terminal screen and the transport pill give this
    /// state — see ``TransportConnState/transientTint``. This row used to be
    /// amber whatever was happening, which made a reconnect read as a third
    /// colour for an event the other two surfaces were already disagreeing about.
    private var connectingTint: Color {
        active?.viewModel.connState.transientTint ?? Ink.accent
    }

    private var connectingRow: some View {
        VStack(alignment: .leading, spacing: 0) {
            Rectangle().fill(Ink.cardDivider).frame(height: 1)
                .padding(.bottom, 12)
            HStack(spacing: 9) {
                ProgressView().tint(connectingTint).controlSize(.small)
                Text(connectingLabel)
                    .font(Face.mono(11, .semibold))
                    .foregroundStyle(Ink.secondary)
                Spacer()
                Text("WAIT")
                    .font(Face.mono(9, .bold))
                    .kerning(0.8)
                    .foregroundStyle(connectingTint)
            }
            .padding(.horizontal, 10)
            .frame(minHeight: 36)
            .background(connectingTint.opacity(0.07), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
        }
        .padding(EdgeInsets(top: 0, leading: 14, bottom: 12, trailing: 14))
        .accessibilityIdentifier("card-connecting-\(connection.displayName)")
    }

    // MARK: Attach-timeout notice

    /// Replaces `connectingRow` once `isAttachStalled` latches — the tmux
    /// attach never confirmed within `tmuxAttachTimeoutSeconds`. SSH is still
    /// live, so this isn't `deadRow`; it names the actual failure and offers
    /// Retry (tear down + reconnect) or dismiss (give up — same teardown,
    /// via `onDisconnect`, returning the card to its normal offline state
    /// instead of leaving `attachStalled` set with nothing watching it).
    private var attachStalledRow: some View {
        VStack(alignment: .leading, spacing: 0) {
            Rectangle().fill(Ink.cardDivider).frame(height: 1)
                .padding(.bottom, 12)
            HStack(alignment: .top, spacing: 9) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(Ink.warn)
                    .padding(.top, 2)
                VStack(alignment: .leading, spacing: 6) {
                    Text("Attach didn't complete — tmux never confirmed.")
                        .font(Face.mono(11, .semibold))
                        .foregroundStyle(Ink.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Button(action: onRetryAttach) {
                        Text("RETRY")
                            .font(Face.mono(10, .bold))
                            .kerning(0.8)
                            .foregroundStyle(Ink.accent)
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("card-attach-stalled-retry-\(connection.displayName)")
                }
                Spacer(minLength: 4)
                Button(action: onDisconnect) {
                    Image(systemName: "xmark")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(Ink.secondary)
                        .frame(width: 26, height: 26)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("card-attach-stalled-dismiss-\(connection.displayName)")
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(Ink.warn.opacity(0.07), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
        }
        .padding(EdgeInsets(top: 0, leading: 14, bottom: 12, trailing: 14))
        .accessibilityIdentifier("card-attach-stalled-\(connection.displayName)")
    }

    // MARK: Open-terminal row (live, non-tmux)

    /// Connected plain-SSH / mosh sessions have no tmux tree to expand, so the
    /// card offers a single row that pushes the terminal directly.
    private var openTerminalRow: some View {
        Button(action: onEnter) {
            VStack(alignment: .leading, spacing: 0) {
                Rectangle().fill(Ink.cardDivider).frame(height: 1)
                    .padding(.bottom, 12)
                HStack(spacing: 8) {
                    Image(systemName: "terminal")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Ink.accent)
                    Text("Open terminal")
                        .font(Face.mono(12, .semibold))
                        .foregroundStyle(Ink.primary)
                    Spacer()
                    Image(systemName: "arrow.up.right")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(Ink.meta)
                }
                .padding(.horizontal, 10)
                .frame(minHeight: 36)
                .background(Color.white.opacity(0.035), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
            }
            .padding(EdgeInsets(top: 0, leading: 14, bottom: 12, trailing: 14))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("card-open-terminal-\(connection.displayName)")
    }

    /// Honest state for a dead session: one amber row, tap to reconnect.
    private var deadRow: some View {
        Button {
            onConnect()
        } label: {
            HStack(spacing: 8) {
                Image(systemName: "bolt.slash.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(Ink.warn)
                Text("Connection lost — tap to reconnect")
                    .font(Face.text(13, .semibold))
                    .foregroundStyle(Ink.warn)
                Spacer()
            }
            .padding(EdgeInsets(top: 12, leading: 14, bottom: 14, trailing: 14))
            .background(Ink.warn.opacity(0.08), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
            .padding(.horizontal, 14)
            .padding(.bottom, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    // MARK: Bottom bar

    private var bottomBar: some View {
        VStack(spacing: 0) {
            Rectangle().fill(Ink.cardBorder).frame(height: 1)
            HStack {
                CardActionButton(title: "Edit", systemImage: "slider.horizontal.3", action: onEdit)
                Spacer()
                if isDisconnecting {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.mini).tint(Ink.meta)
                        Text("Disconnecting…").foregroundStyle(Ink.meta)
                    }
                } else {
                    CardActionButton(title: "Disconnect", systemImage: "bolt.slash", action: onDisconnect)
                }
            }
            .buttonStyle(.plain)
            .padding(EdgeInsets(top: 10, leading: 14, bottom: 12, trailing: 14))
        }
    }
}

private struct CardActionButton: View {
    let title: LocalizedStringKey
    let systemImage: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: systemImage)
                    .font(.system(size: 11, weight: .bold))
                Text(title)
                    .font(Face.text(13, .semibold))
            }
            .foregroundStyle(Ink.accent)
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(Ink.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .strokeBorder(Ink.accent.opacity(0.18), lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
    }
}

/// The two-stage worktree removal confirmation, split out of `ConnectionCard`
/// because inlining it pushed that view's modifier chain past what the type
/// checker will solve.
///
/// Two stages on purpose: the first asks about removing a task; the second
/// appears ONLY when herdr reports the checkout is dirty — the one case where
/// this action destroys work that exists nowhere else. herdr refuses a dirty
/// removal on its own and leaves every file in place, so the force flag is
/// never set without the user seeing that sentence.
private struct WorktreeRemovalDialogs: ViewModifier {
    @Binding var target: ConnectionCard.WorktreeTarget?
    @Binding var forceTarget: ConnectionCard.WorktreeTarget?
    @Binding var error: String?

    func body(content: Content) -> some View {
        content
            .confirmationDialog(
                "Remove the worktree for \"\(target?.sessionName ?? "")\"?",
                isPresented: Binding(get: { target != nil }, set: { if !$0 { target = nil } }),
                titleVisibility: .visible,
                presenting: target
            ) { item in
                Button("Remove", role: .destructive) { remove(item, force: false) }
                Button("Cancel", role: .cancel) { target = nil }
            } message: { item in
                Text("Deletes the branch checkout under ~/.herdr/worktrees. \(item.repo) itself is untouched.")
            }
            .confirmationDialog(
                "\"\(forceTarget?.sessionName ?? "")\" has uncommitted changes",
                isPresented: Binding(get: { forceTarget != nil }, set: { if !$0 { forceTarget = nil } }),
                titleVisibility: .visible,
                presenting: forceTarget
            ) { item in
                Button("Delete anyway", role: .destructive) { remove(item, force: true) }
                Button("Keep it", role: .cancel) { forceTarget = nil }
            } message: { _ in
                Text("Those changes exist nowhere else. Removing the worktree throws them away.")
            }
            .moshpitCard(isPresented: Binding(
                get: { error != nil }, set: { if !$0 { error = nil } }
            )) {
                MoshpitNoticeCard(
                    icon: "trash.slash.fill",
                    title: "Couldn't remove the worktree",
                    message: error ?? ""
                ) { error = nil }
            }
    }

    private func remove(_ item: ConnectionCard.WorktreeTarget, force: Bool) {
        target = nil
        forceTarget = nil
        Task {
            switch await item.remove(force) {
            case .removed:
                break
            case .needsForce:
                forceTarget = item
            case .failed(let message):
                error = message
            }
        }
    }
}
