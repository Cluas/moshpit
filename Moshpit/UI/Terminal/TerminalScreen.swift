import SwiftUI
import SwiftTerm
import UIKit

/// Live terminal screen. The UI chrome is visual-only; SwiftTerm, tmux routing,
/// pinch, paste, keyboard shortcuts, and swipe gestures keep their existing
/// service/coordinator paths.
struct TerminalScreen: View {
    /// Mutable (not `let`): tapping the transport pill flips SSH ⇄ Mosh for
    /// this connection in place, so every `connection.xxx` read in this view
    /// (transport pill, breadcrumb, reconnect) picks up the switch immediately.
    @State private var connection: ServerConnection
    let hub: SessionHub

    init(connection: ServerConnection, hub: SessionHub) {
        self._connection = State(initialValue: connection)
        self.hub = hub
    }

    @Environment(AppSettings.self) private var settings
    @Environment(ThemeStore.self) private var themes
    @Environment(SessionMetricsRegistry.self) private var metricsRegistry
    @Environment(ShortcutStore.self) private var shortcutStore
    @Environment(AgentActivityMonitor.self) private var monitor
    @Environment(DeepLinkRouter.self) private var router
    @Environment(ConnectionStoreHolder.self) private var connectionHolder
    @Environment(\.dismiss) private var dismiss

    @State private var active: SessionHub.ActiveSession?
    @State private var showWindowsSheet = false
    @State private var showSessionsSheet = false
    @State private var showPaneSheet = false
    @State private var showError = false
    /// Confirmation dialog before switching SSH ⇄ Mosh — a reconnect isn't
    /// something a stray tap should trigger.
    @State private var showProtocolSwitchConfirm = false
    @State private var showMoshDiagnostics = false
    /// Whether the user has asked for the keyboard yet in this session.
    ///
    /// Opening a session is usually to *read* it, and a keyboard that appears
    /// uninvited covers most of the scrollback you came for — so nothing is
    /// raised until asked. The third state is the load-bearing one: "not asked"
    /// cannot be represented as keyboard-down, because down actively *resigns*
    /// first responder and would undo the tap the user just made on the
    /// terminal to start typing. See ``TerminalFocusPolicy``.
    ///
    /// Starts `.unasked` and is promoted in `.task` when the setting asks for
    /// it — `AppSettings` arrives through the environment, which `init` cannot
    /// read.
    @State private var keyboardIntent: KeyboardIntent = .unasked

    private enum KeyboardIntent {
        /// Screen just opened; nobody has asked either way.
        case unasked
        case up
        case down
    }

    /// How the hosted terminal should treat first responder right now.
    private var focusPolicy: TerminalFocusPolicy {
        // A sheet must never let the pane grab the keyboard back underneath it.
        if anySheetOpen { return .resign }
        switch keyboardIntent {
        case .unasked: return .allow
        case .up: return .take
        case .down: return .resign
        }
    }
    /// Packages to offer in the Install Assist sheet; non-nil presents it.
    @State private var installPackages: [String]?
    /// Pane claimed from `router.paneRequest` for this connection, captured
    /// (and cleared from the router) the moment it's observed — before tmux
    /// may even be attached yet to act on it. Retrying against this local
    /// copy, rather than re-reading the router each time, means a SECOND
    /// request for the same pane still reaches here (the router briefly
    /// returns to nil in between, so it's never hidden by value-equality),
    /// while a claimed-but-not-yet-actionable request can't be replayed by a
    /// later, unrelated manual navigation to this same connection.
    @State private var pendingPaneSelection: String?
    /// Sticky-Ctrl armed: the next typed key goes out as its control code.
    @State private var ctrlArmed = false
    /// URL found in text the user just copied in-terminal — drives the
    /// transient "Open link" chip (plain-text URLs aren't tappable; select +
    /// copy is the natural gesture, so meet the user right after it).
    @State private var copiedURL: URL?
    @State private var copiedURLDismiss: Task<Void, Never>?
    /// Live voice-input session; non-nil while the dictation overlay is up.
    /// Minted on mic tap (not at screen init) so idle terminals never touch
    /// audio plumbing.
    @State private var dictation: VoiceDictationController?

    private var theme: TerminalTheme {
        themes.theme(id: settings.themeId)
    }

    private var metrics: SessionMetrics? { metricsRegistry.metrics[connection.id] }

    /// A single observed signal that advances as the tmux snapshot populates —
    /// attach flips, then panes accumulate. Watched by one `.onChange` so the
    /// deep-link pane selection retries without bloating the view-body type-check.
    private var paneReadinessSignal: Int {
        guard let snapshot = active?.tmuxController?.snapshot, snapshot.isAttached else { return 0 }
        return snapshot.panes.count + 1
    }

    /// Claims a `router.paneRequest` addressed to this connection into
    /// `pendingPaneSelection`, clearing the router immediately — regardless
    /// of whether tmux is even attached yet to act on it. Call before
    /// `selectPendingPaneIfReady()` from every site that might observe a
    /// fresh request (a fresh `TerminalScreen` for a plain card tap has no
    /// memory of prior tokens, so an unclaimed request would look like new
    /// work to it too, replaying a stale/closed pane onto an unrelated visit).
    private func claimPendingPaneRequestIfNeeded() {
        guard let request = router.paneRequest, request.connectionId == connection.id else { return }
        router.clear(token: request.token)
        pendingPaneSelection = request.paneId
    }

    /// Land on the claimed pane once the tmux control plane is attached and
    /// the pane is present in the snapshot. Retried from `.onChange(of:)` as
    /// snapshots arrive, since the pane list populates a few round-trips
    /// after attach. If the pane never shows up, connection-level landing
    /// (the active pane) is an acceptable fallback — nothing forces it, so a
    /// stale/closed pane id is simply dropped.
    private func selectPendingPaneIfReady() {
        guard let paneId = pendingPaneSelection else { return }
        guard let controller = active?.tmuxController, controller.snapshot.isAttached else { return }
        guard controller.snapshot.panes[paneId] != nil else { return }
        pendingPaneSelection = nil
        controller.selectPane(paneId)
    }

    /// Effective cursor colour: amber while roaming (prototype "漫游态自动切换为
    /// amber"), otherwise the user's chosen swatch.
    private var effectiveCursorColorId: String {
        metrics?.isRoaming == true ? "amber" : settings.cursorColorId
    }

    /// Every input `applyAppearance()` reads, in one `Equatable` value.
    ///
    /// This used to be seven separate `.onChange` modifiers; folding them into
    /// one keeps the view's modifier chain inside what the type checker will
    /// solve (adding an eighth tipped it over), and it makes "did we forget to
    /// react to X" answerable by looking at one struct.
    ///
    /// `theme` is here as the resolved theme, not just `settings.themeId`:
    /// editing the custom theme you are currently using changes its colors
    /// while the id stays put, and an open session would otherwise keep the old
    /// palette until the next reconnect.
    private struct AppearanceKey: Equatable {
        let theme: TerminalTheme
        let fontSize: Double
        let fontName: String
        let cursorShape: CursorShape
        let cursorColorId: String
        let cursorBlink: Bool
    }

    private var appearanceKey: AppearanceKey {
        AppearanceKey(theme: theme,
                      fontSize: settings.fontSize,
                      fontName: settings.fontName,
                      cursorShape: settings.cursorShape,
                      cursorColorId: effectiveCursorColorId,
                      cursorBlink: settings.cursorBlink)
    }

    /// Push theme + font + cursor into the tmux controller. The single-pane
    /// path picks these up directly through SwiftTerminalView's bindings.
    private func applyAppearance() {
        active?.tmuxController?.configureAppearance(
            theme: theme, fontSize: settings.fontSize, fontName: settings.fontName,
            cursorShape: settings.cursorShape, cursorColorId: effectiveCursorColorId,
            cursorBlink: settings.cursorBlink)
    }

    /// Multiplexer state from whichever control plane is active — tmux's
    /// in-band -CC controller (SSH), tmux's sidecar -CC controller (mosh), or
    /// herdr's snapshot poller (either transport). Each branch reads a
    /// CONCRETE type, not an existential, so Observation tracks `snapshot`.
    private var tmuxSnapshot: TmuxSnapshot? {
        if let controller = active?.tmuxController { return controller.snapshot }
        if let control = active?.moshControl { return control.snapshot }
        return active?.herdrControl?.snapshot
    }

    /// The agent hooks matching ``tmuxSnapshot`` — same concrete-type dance,
    /// same reason. Feeds the breadcrumb's agent segment; empty when no
    /// control plane (or no hooks) so the crumb falls back to command/number.
    private var multiplexerAgentHooks: [String: AgentHook] {
        if let controller = active?.tmuxController { return controller.agentHooks }
        if let control = active?.moshControl { return control.agentHooks }
        return active?.herdrControl?.agentHooks ?? [:]
    }

    /// True while any tmux navigation sheet is up.
    private var anySheetOpen: Bool {
        showWindowsSheet || showSessionsSheet || showPaneSheet
    }

    /// Collapse the keyboard before presenting a sheet so it doesn't cover
    /// the sheet's content (the window / session / pane lists sit at the
    /// bottom). Paired with `anySheetOpen` gating focus so the terminal
    /// doesn't immediately pop the keyboard back up.
    private func presentSheet(_ open: @escaping () -> Void) {
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        open()
    }

    private var transportKind: TransportPillKind {
        if connection.connectionProtocol == .mosh {
            return metrics?.isRoaming == true ? .moshRoaming : .mosh
        }
        return .ssh
    }

    /// Live connection state for the transport pill — so a dropped/reconnecting
    /// session is visible, not silent.
    private var connState: TransportConnState {
        // One state for the whole automatic reconnect, however many attempts it
        // takes. The status underneath flips connecting → failed → connecting on
        // every keepalive tick, and letting that through made the screen alternate
        // between "opening the pit" and "line dropped" while a modal error card
        // came and went on top — three ways of saying the line is down.
        if active?.viewModel.isAutoReconnectInFlight == true { return .offline }
        switch active?.viewModel.status {
        case .connected: return .live
        case .reconnecting: return .reconnecting
        case .connecting, .none: return .connecting
        case .failed, .disconnected: return .offline
        case .idle: return .connecting
        }
    }

    /// The single top-of-screen banner. The dead-mosh-return-path warning wins
    /// the slot when present (nothing is rendering, so it's the most urgent
    /// thing to say); otherwise roaming (mosh) and degrade (fallback) share it
    /// — and those two are mutually exclusive since a degraded session is plain
    /// SSH, which never roams.
    @ViewBuilder
    private var topBanner: some View {
        if active?.moshReturnPathDead == true {
            MoshReturnPathBannerView(
                onSwitchToSSH: { switchProtocol() },
                onDismiss: {
                    withAnimation(.easeOut(duration: 0.2)) { active?.moshReturnPathDead = false }
                })
                .transition(.move(edge: .top).combined(with: .opacity))
        } else if metrics?.isRoaming == true {
            RoamBanner(metrics: metrics)
                .transition(.move(edge: .top).combined(with: .opacity))
        } else if let notice = active?.degrade {
            HostBannerView(
                notice: notice,
                onInstall: { installPackages = notice.packages },
                onDismiss: {
                    withAnimation(.easeOut(duration: 0.2)) { active?.degrade = nil }
                })
                .transition(.move(edge: .top).combined(with: .opacity))
        }
    }

    /// Split out of `body` — one giant modifier chain from the ZStack through
    /// every `.onChange` blew the type checker's inference budget (it kept
    /// cascading to a different closure each time one was fixed in place,
    /// since the whole chain is jointly inferred as a single expression).
    /// Breaking the chain into two separately-typed expressions bounds that.
    private var contentWithLifecycle: some View {
        ZStack(alignment: .top) {
            Ink.terminalBG.ignoresSafeArea()

            VStack(spacing: 0) {
                topBar
                terminalBody
            }
            // The pane list populates a few round-trips after attach, so retry
            // the deep-link pane selection as the snapshot fills in (attach,
            // then panes). Attached to this inner view (not the outer modifier
            // chain) to keep the body's type-check tractable.
            .onChange(of: paneReadinessSignal) { selectPendingPaneIfReady() }
            .onChange(of: router.paneRequest) {
                claimPendingPaneRequestIfNeeded()
                selectPendingPaneIfReady()
            }

            topBanner
                .padding(.horizontal, 12)
                .padding(.top, 64)

            // Transient tmux command failures (create/rename/kill) — these were
            // silently dropped before; a quiet capsule beats a modal alert.
            if let notice = active?.tmuxControl?.notice ?? active?.herdrNotice {
                Text(notice)
                    .font(Face.mono(11))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 12).padding(.vertical, 6)
                    .background(Ink.warn.opacity(0.92), in: Capsule())
                    .padding(.top, 112)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }

            if let url = copiedURL {
                VStack {
                    Spacer()
                    Button {
                        UIApplication.shared.open(url)
                        copiedURL = nil
                    } label: {
                        HStack(spacing: 6) {
                            Image(systemName: "safari").font(.system(size: 13, weight: .semibold))
                            Text(url.host() ?? url.absoluteString)
                                .font(Face.mono(12, .semibold)).lineLimit(1)
                            Image(systemName: "arrow.up.right").font(.system(size: 10, weight: .bold))
                        }
                        .foregroundStyle(.black)
                        .padding(.horizontal, 14).padding(.vertical, 9)
                        .background(Ink.accent, in: Capsule())
                    }
                    .buttonStyle(.plain)
                    .padding(.bottom, 12)
                }
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.easeOut(duration: 0.2), value: copiedURL)
        .animation(.easeOut(duration: 0.2), value: active?.tmuxControl?.notice)
        .onReceive(NotificationCenter.default.publisher(
            for: UIPasteboard.changedNotification)) { _ in
            Task { await detectCopiedURL() }
        }
        .onReceive(NotificationCenter.default.publisher(
            for: UIResponder.keyboardWillShowNotification)) { _ in
            // Tapping the terminal raises the keyboard directly — treat that as
            // "I want to type" so the toggle state can't fight it: with the flag
            // stale, the next snapshot-driven re-render force-resigned the
            // keyboard mid-word.
            keyboardIntent = .up
        }
        .navigationBarHidden(true)
        .preferredColorScheme(.dark)
        .safeAreaInset(edge: .bottom, spacing: 0) { bottomAccessory }
        .task {
            if settings.raiseKeyboardOnOpen { keyboardIntent = .up }
            await connectAndAttach()
        }
        .onDisappear {
            // Back to Home keeps the connection alive but stops rendering — hand
            // our phone-grid window pin back so a desktop client sharing those
            // windows isn't stranded at the phone width. Re-pinned in .task on
            // return. (A presented sheet doesn't fire onDisappear, so opening the
            // windows/sessions picker won't release the pin.)
            active?.releaseWindowPinsForBackground()
            if hub.visibleSession === active { hub.visibleSession = nil }
        }
        .onChange(of: appearanceKey) { applyAppearance() }
        .onChange(of: active?.viewModel.errorMessage) { _, message in
            showError = message?.isEmpty == false
        }
    }

    var body: some View {
        contentWithLifecycle
        .moshpitCard(isPresented: $showError) {
            MoshpitNoticeCard(
                icon: "bolt.slash.fill",
                title: "Connection Error",
                message: active?.viewModel.errorMessage ?? ""
            ) {
                showError = false
                active?.viewModel.errorMessage = nil
                dismiss()
            }
        }
        .moshpitCard(
            item: Binding(
                get: { active?.viewModel.hostKeyPrompt },
                set: { if $0 == nil { /* dismissal handled via decide() */ } }
            )
        ) { prompt in
            hostKeyPromptCard(prompt)
        }
        .sheet(isPresented: $showWindowsSheet) {
            // Generic over the concrete controller (not an existential) so
            // Observation tracking on `snapshot` survives — hence the branch.
            if let c = active?.tmuxControl { WindowsSheet(controller: c) }
            else if let h = active?.herdrControl { WindowsSheet(controller: h) }
        }
        .sheet(isPresented: $showSessionsSheet) {
            if let c = active?.tmuxControl { SessionsSheet(controller: c) }
            else if let h = active?.herdrControl { SessionsSheet(controller: h) }
        }
        .sheet(isPresented: $showPaneSheet) {
            if let c = active?.tmuxControl { SelectPaneSheet(controller: c) }
            else if let h = active?.herdrControl { SelectPaneSheet(controller: h) }
        }
        .sheet(item: installTargetBinding) { target in
            if let active {
                InstallAssistView(
                    session: active,
                    packages: target.packages,
                    onReconnect: { reconnect() })
            }
        }
    }

    private var installTargetBinding: Binding<InstallTarget?> {
        Binding(
            get: { installPackages.map { InstallTarget(packages: $0) } },
            set: { installPackages = $0?.packages }
        )
    }

    /// Identifiable wrapper so `.sheet(item:)` can carry the package list.
    private struct InstallTarget: Identifiable {
        let packages: [String]
        var id: String { packages.joined(separator: ",") }
    }

    /// Confirmed via the transport pill's confirmation dialog before
    /// `switchProtocol()` runs — a tap alone no longer reconnects immediately.
    private func switchProtocol() {
        guard active != nil else { return }
        connection.connectionProtocol = connection.connectionProtocol == .mosh ? .ssh : .mosh
        connectionHolder.store.update(connection)
        Haptics.select()
        reconnect()
    }

    /// Tear down and re-run the start flow at full feature — used after the
    /// user installs the missing dependency. The probe re-runs during start,
    /// so a now-present tmux / mosh-server is picked up automatically.
    private func reconnect() {
        guard let active else { return }
        Task {
            await hub.disconnect(connection.id)
            let session = hub.prepare(connection)
            self.active = session
            await hub.start(session, theme: theme, fontSize: settings.fontSize, fontName: settings.fontName,
                        cursorShape: settings.cursorShape, cursorColorId: effectiveCursorColorId,
                        cursorBlink: settings.cursorBlink)
            applyAppearance()
            if let controller = session.tmuxController {
                monitor.track(connection: connection, controller: controller)
            } else if let herdr = session.herdrControl {
                // herdr reports agent status natively — no host-side hooks.
                monitor.track(connection: connection, controller: herdr)
            }
            _ = active
        }
    }

    // MARK: - Top bar

    private var topBar: some View {
        HStack(spacing: 9) {
            Button {
                dismiss()
            } label: {
                Image(systemName: "chevron.left")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(Ink.accent)
                    .frame(width: 34, height: 34)
                    .background(Color.white.opacity(0.045), in: Circle())
                    .overlay(Circle().strokeBorder(Ink.groupBorder, lineWidth: 1))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("terminal-back")

            Button {
                showProtocolSwitchConfirm = true
            } label: {
                TransportPill(kind: transportKind, connState: connState)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("terminal-transport-switch")
            .accessibilityHint(Text("Double tap to switch between SSH and Mosh"))
            // Long-press reveals live protocol counters as a screenshottable
            // overlay — a black-screen report (cursor + input work, no
            // content) needs real packet loss/reordering or a specific
            // remote's shell config to reproduce, neither of which loopback
            // testing exercises, so this is the only practical diagnostic
            // for a session we can't otherwise inspect.
            .simultaneousGesture(
                LongPressGesture(minimumDuration: 0.5).onEnded { _ in
                    if connection.connectionProtocol == .mosh { showMoshDiagnostics = true }
                }
            )
            .popover(isPresented: $showMoshDiagnostics) {
                MoshDiagnosticsView(diagnostics: metrics?.moshDiagnostics)
                    .presentationCompactAdaptation(.popover)
            }
            .confirmationDialog(
                "Switch to \(connection.connectionProtocol == .mosh ? "SSH" : "Mosh")?",
                isPresented: $showProtocolSwitchConfirm,
                titleVisibility: .visible
            ) {
                Button("Switch") { switchProtocol() }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Reconnects this session over \(connection.connectionProtocol == .mosh ? "SSH" : "Mosh").")
            }

            breadcrumb
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .background(Color.white.opacity(0.045), in: Capsule(style: .continuous))
                .overlay(
                    Capsule(style: .continuous)
                        .strokeBorder(Ink.groupBorder, lineWidth: 1)
                )

            Spacer(minLength: 0)
            // The pane-grid button used to live here; it was redundant with the
            // breadcrumb's pane crumb (both open the Select Pane sheet), so it's
            // gone — the rightmost icon the breadcrumb wrapping issue referenced.
        }
        .padding(.horizontal, 14)
        .frame(height: 54)
        .background(Ink.navGlass)
        .background(.ultraThinMaterial)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Ink.hairline).frame(height: 1)
        }
    }

    /// A breadcrumb segment: a small SF Symbol + mono label, in Signal Room citron.
    ///
    /// `text: nil` renders icon-only — the squeezed form the session crumb
    /// takes when the pane crumb is carrying an agent name (three full
    /// segments don't fit the bar; see `BreadcrumbPlan`).
    private func crumbLabel(_ symbol: String, _ text: String?) -> some View {
        HStack(spacing: 3) {
            Image(systemName: symbol)
                .font(.system(size: 9, weight: .semibold))
            // Single line + truncate so a long window name can't wrap the
            // fixed-height top bar onto two rows.
            if let text {
                Text(text).font(Face.mono(11)).lineLimit(1).truncationMode(.tail)
            }
        }
        .foregroundStyle(Ink.accent)
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .background(Ink.accent.opacity(0.08), in: Capsule(style: .continuous))
    }

    /// The pane crumb, which doubles as the agent crumb. With a signal the
    /// state dot replaces the pane glyph (the dot already says "agent lives
    /// here", and it says it in the same palette as the Agents section and
    /// the island), and needs-you re-tints the whole capsule amber — the top
    /// bar's version of the lock screen's exclamation mark.
    private func paneCrumbLabel(_ title: String, signal: AgentSignal?) -> some View {
        let tint = signal == .attention ? Ink.warn : Ink.accent
        return HStack(spacing: 4) {
            if let signal {
                Circle()
                    .fill(signal.color)
                    .frame(width: 5, height: 5)
                    .shadow(color: signal.color.opacity(0.8), radius: 2.5)
            } else {
                Image(systemName: "squareshape.split.2x2")
                    .font(.system(size: 9, weight: .semibold))
            }
            Text(title).font(Face.mono(11)).lineLimit(1).truncationMode(.tail)
        }
        .foregroundStyle(tint)
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .background(tint.opacity(0.08), in: Capsule(style: .continuous))
        // No accessibility modifiers here on purpose: this view is a Button's
        // LABEL, and `.accessibilityElement(children: .combine)` inside a
        // control swallows the control's own label — VoiceOver (and the e2e
        // script, which reads Button AXLabels) then finds a nameless button
        // where "pane 1" used to be. The label belongs on the Button.
    }

    /// What VoiceOver reads for the pane crumb: the title plus what the dot
    /// means, because a colour alone is a signal only some users receive.
    private func paneCrumbAccessibilityLabel(_ title: String, signal: AgentSignal?) -> String {
        signal.map { "\(title), \($0.label)" } ?? title
    }

    private var crumbSep: some View {
        Image(systemName: "chevron.right")
            .font(.system(size: 7, weight: .bold))
            .foregroundStyle(Ink.meta)
    }

    private var breadcrumb: some View {
        HStack(spacing: 6) {
            // Read the snapshot through concrete types (not the existential) so
            // SwiftUI Observation tracks updates from either controller/sidecar.
            // The strings and the squeeze rule live in `BreadcrumbPlan` — pure,
            // tested — this just lays the segments out.
            if let snapshot = tmuxSnapshot,
               let plan = BreadcrumbPlan.make(snapshot: snapshot, hooks: multiplexerAgentHooks) {
                Button {
                    presentSheet { showSessionsSheet = true }
                } label: {
                    crumbLabel(connection.multiplexer.vocabulary.sessionIcon,
                               plan.sessionIconOnly ? nil : plan.sessionTitle)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(plan.sessionTitle)

                crumbSep

                Button {
                    presentSheet { showWindowsSheet = true }
                } label: {
                    crumbLabel(connection.multiplexer.vocabulary.windowIcon, plan.windowTitle)
                }
                .buttonStyle(.plain)

                if let paneTitle = plan.paneTitle {
                    crumbSep
                    // The pane (command / agent) segment opens the Select Pane
                    // sheet — switch panes or split a new one. It carries the
                    // name that matters most, so it truncates last.
                    Button {
                        presentSheet { showPaneSheet = true }
                    } label: {
                        paneCrumbLabel(paneTitle, signal: plan.paneSignal)
                    }
                    .buttonStyle(.plain)
                    .layoutPriority(1)
                    .accessibilityLabel(
                        paneCrumbAccessibilityLabel(paneTitle, signal: plan.paneSignal))
                }
            } else {
                // mosh / plain SSH / herdr: no -CC control stream, so no live
                // session/window/pane data to build the rich breadcrumb from.
                // Show the connection identity, plus the multiplexer running
                // inside so it's clear you're multiplexing — navigate it with
                // Ctrl-b, which is the prefix for both tmux and herdr.
                Text("\(connection.username)@\(connection.host)")
                    .font(Face.mono(11))
                    .foregroundStyle(Ink.secondary)
                    .lineLimit(1)
                if connection.multiplexer != .none {
                    Text("·").font(Face.mono(11)).foregroundStyle(Ink.meta)
                    Text(connection.multiplexer.label)
                        .font(Face.mono(11))
                        .foregroundStyle(Ink.mosh)
                }
            }
        }
    }

    // MARK: - Terminal body

    @ViewBuilder
    private var terminalBody: some View {
        if let active {
            if let controller = active.tmuxController {
                if controller.snapshot.isAttached {
                    TmuxPaneSplitView(controller: controller,
                                      focusPolicy: focusPolicy)
                } else {
                    // Unattached. Distinguish the two reasons (the design's
                    // "fix the mis-diagnosis"): tmux genuinely absent → offer
                    // Install; tmux present but the server has no sessions →
                    // offer Create (Moshpit never creates one on its own).
                    MultiplexerEmptyStateView(
                        multiplexer: .tmux,
                        binaryMissing: active.capabilities?.hasTmux == false,
                        connection: connection,
                        connectionState: connState,
                        onCreate: { active.createFirstTmuxSession() },
                        onInstall: { installPackages = ["tmux"] })
                }
            } else if let herdr = active.herdrControl, herdr.serverNotRunning {
                // herdr is installed but nothing is running, so there's no pane
                // for the frame channel to render. Same two honest answers as
                // tmux's unattached state — and creating a workspace is an
                // explicit user action, never something the poller does.
                MultiplexerEmptyStateView(
                    multiplexer: .herdr,
                    binaryMissing: active.capabilities?.hasHerdr == false,
                    connection: connection,
                    connectionState: connState,
                    onCreate: { herdr.newSession(named: nil) },
                    onInstall: { installPackages = ["herdr"] })
            } else {
                ZStack {
                    SwiftTerminalView(
                        theme: theme,
                        fontSize: settings.fontSize,
                        fontName: settings.fontName,
                        cursorShape: settings.cursorShape,
                        cursorColorId: effectiveCursorColorId,
                        cursorBlink: settings.cursorBlink,
                        coordinator: active.coordinator,
                        focusPolicy: focusPolicy)
                        // SwiftTerminalView.updateUIView is deliberately "cosmetic
                        // only" (see its doc) — it never re-wires the underlying
                        // TerminalView's delegate to a NEW coordinator. Reconnecting
                        // onto a brand-new ActiveSession (switchProtocol(), Install
                        // Assist) mints a brand-new coordinator, and without this
                        // `.id()` the view keeps its old identity, so SwiftUI calls
                        // updateUIView (not makeUIView) and the screen goes stale —
                        // frozen on the old session's last frame, deaf to new input,
                        // while the new session's output sits buffered in a
                        // coordinator that never got attached. Keying on the
                        // coordinator's identity forces a fresh makeUIView exactly
                        // when (and only when) it actually changes.
                        .id(ObjectIdentifier(active.coordinator))

                    // The initial handshake would otherwise be a black void
                    // (SwiftTerm is attached but has no output yet). Cover it
                    // with the branded waiting screen and fade out once the
                    // shell is live — kept as an OVERLAY, not a replacement, so
                    // SwiftTerm stays attached and never misses buffered output.
                    // Reconnecting is deliberately excluded: mosh keeps a useful
                    // last frame + predictive echo we shouldn't hide.
                    //
                    // An automatic reconnect keeps it up for the WHOLE cycle,
                    // including the wait between failed attempts. Otherwise the
                    // cover dropped away every time an attempt failed, flashing
                    // the dead terminal underneath for twelve seconds until the
                    // next tick put it back.
                    if active.viewModel.status == .connecting
                        || active.viewModel.isAutoReconnectInFlight {
                        TerminalConnectingView(connection: connection, state: connState)
                            .transition(.opacity)
                    }
                }
                .animation(.easeOut(duration: 0.28), value: active.viewModel.status)
                .animation(.easeOut(duration: 0.28), value: active.viewModel.isAutoReconnectInFlight)
            }
        } else {
            TerminalConnectingView(connection: connection, state: connState)
        }
    }

    // MARK: - Shortcut dispatch

    /// The coordinator receiving keyboard input right now (tmux active pane,
    /// or the single mosh/SSH coordinator).
    private var typingCoordinator: SwiftTerminalView.Coordinator? {
        active?.tmuxController?.activeCoordinator ?? active?.coordinator
    }

    /// Arm/disarm sticky Ctrl on whatever the user is typing into. One-shot:
    /// the coordinator clears itself after folding the next key.
    private func toggleStickyCtrl() {
        guard let coordinator = typingCoordinator else { return }
        if ctrlArmed {
            coordinator.pendingCtrl = false
            ctrlArmed = false
        } else {
            Haptics.tap()
            coordinator.pendingCtrl = true
            coordinator.onPendingCtrlConsumed = { ctrlArmed = false }
            ctrlArmed = true
        }
    }

    /// Sticky-Ctrl only folds into an actual typed key (`Coordinator.send`
    /// consumes it there) — nothing clears it if the very next thing the user
    /// does is tap a *different* shortcut-bar control instead of typing.
    /// Left armed, it silently outlives its own chip: esc/tab/^C/paste/arrow/
    /// scroll would all go out unaffected, but ctrl stayed "on" for whatever
    /// ordinary letter came after, folding it into an unintended control byte.
    /// Any other bar action means "I'm done with ctrl" — disarm it the same
    /// way an explicit second tap on the chip would.
    private func disarmStickyCtrlIfNeeded() {
        guard ctrlArmed, let coordinator = typingCoordinator else { return }
        coordinator.pendingCtrl = false
        ctrlArmed = false
    }

    /// Copying inside the terminal is how users grab dev-server/OAuth links
    /// out of agent output — detect a URL and offer to open it. Same-app
    /// copies don't trigger the system paste banner, but the read is still a
    /// synchronous, potentially-blocking pasteboardd XPC call (see the
    /// `.paste` shortcut's matching comment, which this same pattern hung
    /// on) — off the main thread regardless. Pulled into its own explicitly-
    /// typed method (rather than inlined in the `.onReceive` closure) because
    /// the compound guard chain, inside a `Task`, inside a closure, blew the
    /// type checker's inference budget one clause at a time.
    private func detectCopiedURL() async {
        let text = await Task.detached { UIPasteboard.general.string }.value
        guard let text else { return }
        let detector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.link.rawValue)
        guard let match = detector?.firstMatch(in: text, range: NSRange(text.startIndex..., in: text))
        else { return }
        guard let url = match.url else { return }
        guard let scheme = url.scheme, ["https", "http"].contains(scheme) else { return }
        copiedURL = url
        copiedURLDismiss?.cancel()
        copiedURLDismiss = Task {
            try? await Task.sleep(for: .seconds(8))
            guard !Task.isCancelled else { return }
            copiedURL = nil
        }
    }

    /// Bind the session to the UI BEFORE starting the transport: the
    /// host-key TOFU prompt fires mid-handshake through
    /// `viewModel.hostKeyPrompt`, and the alert can only present once
    /// `active` is set. (Binding after connect deadlocked first-ever
    /// connections: prompt unanswerable → handshake stuck.) Pulled out of
    /// the `.task` modifier into its own explicitly-typed method — inlined,
    /// this closure was one straw too many for the view body's already-huge
    /// modifier-chain expression, blowing the type checker's budget.
    private func connectAndAttach() async {
        let session = hub.prepare(connection)
        active = session
        // Foreground/background transitions re-pin/release for the terminal
        // the user is actually looking at.
        hub.visibleSession = session
        await hub.start(session, theme: theme, fontSize: settings.fontSize, fontName: settings.fontName,
                        cursorShape: settings.cursorShape, cursorColorId: effectiveCursorColorId,
                        cursorBlink: settings.cursorBlink)
        // Returning to an already-alive session (Back→Home doesn't disconnect,
        // and start() is idempotent): onDisappear released our window-size pin
        // so a desktop client could reclaim its width. Re-pin to the phone grid
        // now. No-op on a fresh connect (snapshot not attached yet — the normal
        // flow pins as windows are discovered).
        session.repinForeground()
        // tmux mints its terminals during connect (before cursor settings
        // were threaded in) — re-assert the full appearance now.
        applyAppearance()
        // Pinch-zoom commits persist into Settings — otherwise the next
        // appearance pass snapped the size straight back.
        session.coordinator.onFontSizeCommit = { size in settings.fontSize = size }
        session.tmuxController?.onFontSizeCommitted = { size in settings.fontSize = size }
        if let controller = session.tmuxController {
            monitor.track(connection: connection, controller: controller)
        } else if let herdr = session.herdrControl {
            monitor.track(connection: connection, controller: herdr)
        }
        // Deep-link path: if already attached with the pane present, land on
        // it now; otherwise the .onChange below retries as snapshots arrive.
        claimPendingPaneRequestIfNeeded()
        selectPendingPaneIfReady()
    }

    private func send(shortcut: TerminalShortcut) {
        if shortcut.kind == .ctrl {
            toggleStickyCtrl()
            return
        }
        disarmStickyCtrlIfNeeded()
        if shortcut.kind == .paste {
            // UIPasteboard reads are synchronous and can block for well over a
            // second: a cross-app paste triggers iOS's "Allow Paste?" prompt,
            // and the property getter waits on that (plus the pasteboardd XPC
            // round-trip) on whatever thread calls it. Confirmed via a device
            // hang report — tapping this chip froze the main thread for 1.4s+
            // inside `-[_UIConcretePasteboard string]`, tripping a CA fence
            // hang. Reading off the main thread keeps the UI/render pipeline
            // alive while it waits.
            Task {
                let text = await Task.detached { UIPasteboard.general.string }.value
                guard let text, !text.isEmpty else {
                    // Empty clipboard used to be a silent no-op — at least say "no".
                    UINotificationFeedbackGenerator().notificationOccurred(.warning)
                    return
                }
                // Bracketed-paste-aware: raw bytes into Claude Code executed a
                // multi-line prompt LINE BY LINE.
                active?.sendPaste(text)
            }
            return
        }
        // Plain (unmodified) arrow combos share the D-pad's application-
        // cursor-mode-aware encoding instead of `encodedBytes()`'s fixed CSI —
        // see `sendArrow` for why a hardcoded sequence breaks zsh/bash history
        // search. Modified arrows (⌃/⌥/⇧) keep their own CSI-with-parameter
        // form, which every TUI accepts regardless of cursor-key mode.
        if shortcut.kind == .keyCombo, shortcut.payload.isEmpty, shortcut.modifiers.isEmpty,
           ["up", "down", "left", "right"].contains(shortcut.key.lowercased()) {
            sendArrow(shortcut.key.lowercased())
            return
        }
        // Unified routing handles tmux / mosh / plain SSH (the mosh path
        // previously fell through to a closed SSH session and silently
        // dropped every shortcut).
        let stages = shortcut.encodedStages()
        guard let first = stages.first else { return }
        active?.sendInput(first)
        guard stages.count > 1 else { return }
        let rest = stages.dropFirst()
        Task {
            for stage in rest {
                try? await Task.sleep(for: Self.keySequenceGap)
                active?.sendInput(stage)
            }
        }
    }

    /// Gap between the key presses of a multi-key shortcut (`⇥⏎`).
    ///
    /// Long enough that the remote reads them separately rather than as one
    /// pasted chunk, and long enough for the first key's effect to render —
    /// accepting a completion is a re-render on the other end, and a Return
    /// evaluated against the pre-accept state does nothing. Short enough that
    /// one tap still feels like one action.
    private static let keySequenceGap: Duration = .milliseconds(120)

    /// Send an arrow key from the D-pad as a CSI sequence (ESC [ A/B/C/D in
    /// normal mode, ESC O A/B/C/D in application cursor-key mode — matching
    /// whatever the remote shell/program currently expects; see
    /// `SessionHub.ActiveSession.applicationCursorKeys`).
    private func sendArrow(_ direction: String) {
        disarmStickyCtrlIfNeeded()
        let appMode = active?.applicationCursorKeys ?? false
        let csi: [UInt8]
        switch direction {
        case "up":    csi = appMode ? [0x1B, 0x4F, 0x41] : [0x1B, 0x5B, 0x41]
        case "down":  csi = appMode ? [0x1B, 0x4F, 0x42] : [0x1B, 0x5B, 0x42]
        case "right": csi = appMode ? [0x1B, 0x4F, 0x43] : [0x1B, 0x5B, 0x43]
        case "left":  csi = appMode ? [0x1B, 0x4F, 0x44] : [0x1B, 0x5B, 0x44]
        default: return
        }
        active?.sendInput(Data(csi))
    }

    /// Scroll the visible pane's scrollback from the scroll thumb. Drives the
    /// same local scrollback + output-hold path as the swipe gesture, so it
    /// survives streaming output — but as a dedicated bar control it can't
    /// collide with tap-to-focus / long-press-select on the terminal surface.
    private func scrollHistory(_ direction: String) {
        // Scrolling isn't "typing" either — armed ctrl shouldn't survive it.
        disarmStickyCtrlIfNeeded()
        let lines = 3
        active?.scrollActiveTerminal(lines: direction == "up" ? lines : -lines)
    }

    // MARK: - Voice input

    /// The bottom safe-area stack: dictation overlay (when a session is up)
    /// riding directly above the shortcut bar, both following the keyboard.
    /// Split out of the main modifier chain for the same type-checker-budget
    /// reason as `contentWithLifecycle`.
    @ViewBuilder
    private var bottomAccessory: some View {
        VStack(spacing: 0) {
            if let dictation {
                DictationOverlayView(
                    controller: dictation,
                    onCancel: { cancelDictation() },
                    onInsert: { finishDictation() },
                    onSend: { finishDictation(submit: true) })
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
            ShortcutBarView(
                shortcuts: shortcutStore.toolbar(
                    forHost: connection.displayName,
                    inMultiplexer: active?.tmuxControl != nil || active?.herdrControl != nil),
                onTap: send(shortcut:),
                ctrlArmed: ctrlArmed,
                onArrow: sendArrow,
                onScroll: scrollHistory,
                keyboardDown: keyboardIntent != .up,
                onToggleKeyboard: { keyboardIntent = keyboardIntent == .up ? .down : .up },
                micActive: dictation != nil,
                onMic: settings.voiceInputEnabled ? { toggleDictation() } : nil)
        }
        .animation(.easeOut(duration: 0.2), value: dictation == nil)
    }

    /// Mic key: no session → start one; session live → stop-and-insert (the
    /// natural "I'm done talking" gesture); session failed/preparing → treat
    /// the tap as dismissal.
    private func toggleDictation() {
        disarmStickyCtrlIfNeeded()
        if let dictation {
            if dictation.isListening {
                finishDictation()
            } else {
                cancelDictation()
            }
            return
        }
        Haptics.tap()
        let controller = VoiceDictationController()
        dictation = controller
        Task { await controller.start(settings.dictationRequest) }
    }

    /// Stop the mic, wait for the engine's final words, and type the result
    /// into the live session. `sendPaste` (not raw bytes) so bracketed-paste
    /// apps — Claude Code prompts being the whole point — receive multi-line
    /// dictation as one block instead of executing it line by line.
    ///
    /// `submit` appends the Return that actually sends it, so dictating a
    /// prompt is one tap rather than Insert-then-find-the-Enter-key. It stays a
    /// separate action from Insert on purpose: the same keystroke that submits
    /// a Claude Code prompt EXECUTES a shell command, and a mis-heard word is
    /// exactly what you want the chance to read first. Insert keeps that
    /// chance; Send is for when you have already read it.
    private func finishDictation(submit: Bool = false) {
        guard let dictation else { return }
        Task {
            if let text = await dictation.finish() {
                active?.sendPaste(text)
                if submit {
                    // After the paste, not bundled into it: a bracketed-paste
                    // app treats bytes inside the ESC[200~…ESC[201~ wrapper as
                    // literal text, so a CR in there would be inserted as a
                    // newline instead of submitting.
                    active?.sendInput(Data([0x0D]))
                }
                Haptics.success()
            }
            self.dictation = nil
        }
    }

    private func cancelDictation() {
        dictation?.cancel()
        dictation = nil
    }
}

// MARK: - tmux single-pane rendering

/// Moshpit always shows exactly ONE pane, full-screen (like SSH mode). We never
/// render tmux's split layout — the controller zooms the active pane on the
/// server so it uses the full width, and restores the layout when we switch
/// away (other clients may be viewing the split). Switching panes/windows just
/// swaps which persistent `TerminalView` is shown, keyed by `activePaneId`.
struct TmuxPaneSplitView: View {
    let controller: TmuxSessionController
    /// How the pane should treat first responder — `.resign` while a
    /// navigation sheet is up, so it can't grab the keyboard back over it.
    var focusPolicy: TerminalFocusPolicy = .take

    var body: some View {
        let snapshot = controller.snapshot
        Group {
            if let paneId = snapshot.activePaneId ?? snapshot.activePanes.first?.id {
                PaneTerminalHost(terminal: controller.terminalView(for: paneId),
                                 coordinator: controller.coordinator(for: paneId),
                                 focusPolicy: focusPolicy)
                    // Keyed on the CONTROLLER's identity too, not just paneId:
                    // a reconnect mints a brand-new TmuxSessionController (and
                    // fresh per-pane terminals/coordinators) even when the
                    // server-side pane id is unchanged. PaneTerminalHost's
                    // updateUIView doesn't re-host a new terminal/coordinator
                    // pair onto an already-existing container (same bug class
                    // fixed for the plain SSH/mosh path's SwiftTerminalView —
                    // see its matching `.id()` comment) — without this, the
                    // view would keep showing the OLD controller's pane,
                    // frozen and deaf to new input, after any reconnect.
                    .id("\(ObjectIdentifier(controller))#\(paneId)")
                    // A swipe-driven switch used to be an instant cut — the
                    // outgoing pane's persistent terminal just vanished and the
                    // new one popped in ("太生硬" / too abrupt). Sliding the
                    // incoming pane in from the swipe's own direction (and the
                    // outgoing one out the opposite edge) reads as a single
                    // continuous motion instead. Tap-driven switches (sessions
                    // tree, Windows/Sessions/Pane sheets) reuse whatever
                    // direction the last actual swipe was — see
                    // `lastSwitchForward`'s doc comment.
                    .transition(.asymmetric(
                        insertion: .move(edge: snapshot.lastSwitchForward ? .trailing : .leading),
                        removal: .move(edge: snapshot.lastSwitchForward ? .leading : .trailing)
                    ).combined(with: .opacity))
            } else {
                Color.clear
            }
        }
        .padding(4)
        .background(Ink.terminalBG)
        .clipped()
        // NOT `.animation(_:value:)` here — the mutation that flips
        // `activePaneId` originates from a UIKit gesture-recognizer callback
        // (TerminalScrollGesture -> Coordinator.onSwitch), and empirically
        // (verified frame-by-frame via screen recording) that modifier alone
        // did not pick up changes from that call path — the swap landed as
        // one instant cut, no in-between slide. The controller now brackets
        // the `activePaneId` assignment itself in `withAnimation` (see
        // `selectPane`/`selectWindow`), which is what actually drives this.
    }
}

/// Hosts a persistent SwiftTerm `TerminalView` minted by the controller.
/// The active pane grabs keyboard focus so the system keyboard (with our
/// shortcut bar riding above it) is up as soon as the terminal is usable.
/// The terminal sits inside a ``TerminalHostContainer`` so keyboard
/// transitions can freeze its frame (see that type's doc — this is the
/// keyboard/IME-switch garble fix), same as the single-pane path.
struct PaneTerminalHost: UIViewRepresentable {
    let terminal: TerminalView
    /// The pane's coordinator — hooked to the container so the keyboard
    /// frame-lock covers tmux panes too. nil-safe for previews/tests.
    var coordinator: SwiftTerminalView.Coordinator?
    var focusPolicy: TerminalFocusPolicy = .take

    func makeUIView(context: Context) -> TerminalHostContainer {
        // The scroll/zoom gesture is installed by the coordinator's attach(to:)
        // when the controller mints this pane's terminal, so it's already on by
        // the time we host it here — no need (and idempotent if it weren't).
        if focusPolicy == .take {
            DispatchQueue.main.async { [weak terminal] in
                terminal?.becomeFirstResponder()
            }
        }
        // A pane/window switch re-hosts a *persistent* terminal (its `.id` flips).
        // SwiftTerm only repaints on new output or input, so an idle switched-to
        // pane keeps showing whatever pixels it last drew — stale/garbled from a
        // previous size, and a stale caret colour/shape — until the user types or
        // forces a refresh. Force a full repaint from the (correct) buffer once it
        // has laid out, so the switch lands clean without waiting for output.
        DispatchQueue.main.async { [weak terminal] in
            terminal?.getTerminal().updateFullScreen()
            terminal?.setNeedsDisplay()
        }
        let container = TerminalHostContainer()
        container.host(terminal)
        coordinator?.hostContainer = container
        return container
    }

    func updateUIView(_ uiView: TerminalHostContainer, context: Context) {
        guard let terminal = uiView.terminalView else { return }
        switch focusPolicy {
        case .take:
            if !terminal.isFirstResponder, terminal.window != nil {
                DispatchQueue.main.async { [weak terminal] in
                    terminal?.becomeFirstResponder()
                }
            }
        case .allow:
            // Nobody has asked for the keyboard: leave first responder exactly
            // as the user left it, so a tap on the pane can raise it.
            break
        case .resign:
            if terminal.isFirstResponder {
                // Keyboard put away (toggle) or a sheet is up — drop it.
                DispatchQueue.main.async { [weak terminal] in
                    terminal?.resignFirstResponder()
                }
            }
        }
    }
}

// MARK: - Shortcut bar (36pt strip above keyboard)

/// Measures the chip row's full, unclipped content width so the custom
/// drag-scroll (see `ShortcutBarView.chipRow`) knows how far it's allowed
/// to pan.
private struct ChipRowContentWidthKey: PreferenceKey {
    static var defaultValue: CGFloat = 0
    /// Only the hidden measurement copy in `chipRow`'s `ZStack` ever calls
    /// `.preference(key:value:)` with a real width; its VISIBLE sibling
    /// never touches this key at all, but still implicitly contributes the
    /// untouched `defaultValue` (0) to the reduce pass. A naive
    /// `value = nextValue()` reducer lets whichever sibling is walked last
    /// unconditionally win — which was silently clobbering the real
    /// measurement with that sibling's 0 every single time, regardless of
    /// `.hidden()` vs `.opacity(0)`. Keeping the max (equivalently: only
    /// ever move away from 0) means an untouched sibling can never erase a
    /// genuine measurement.
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

struct ShortcutBarView: View {
    let shortcuts: [TerminalShortcut]
    let onTap: (TerminalShortcut) -> Void
    /// Sticky-Ctrl chip: highlighted while armed. The `.ctrl` shortcut (a
    /// normal, reorderable/removable builtin) drives its own arm/disarm
    /// through `onTap` like any other chip — this only controls the highlight.
    var ctrlArmed: Bool = false
    /// Arrow-key direction from the D-pad: "up"/"down"/"left"/"right".
    var onArrow: ((String) -> Void)?
    /// Scroll the scrollback from the scroll thumb: "up"/"down".
    var onScroll: ((String) -> Void)?
    /// Whether the keyboard is currently collapsed (drives the toggle glyph).
    var keyboardDown: Bool = false
    /// Toggle the system keyboard up/down. nil hides the button entirely.
    var onToggleKeyboard: (() -> Void)?
    /// True while a dictation session is up (tints the mic key active).
    var micActive: Bool = false
    /// Start/stop voice input. nil (voice input disabled in Settings) hides
    /// the mic key entirely.
    var onMic: (() -> Void)?

    /// Committed horizontal pan of the chip row, plus the in-flight drag
    /// delta. A plain `ScrollView(.horizontal)` here measurably has its
    /// viewport correctly bounded (confirmed with the live view debugger)
    /// but its pan gesture recognizer never actually moves content on a real
    /// touch/drag — root cause not identified even at the UIKit gesture-
    /// recognizer level. This hand-rolled drag sidesteps whatever is
    /// swallowing that recognizer, using the same `DragGesture` pattern
    /// already proven to work for `DirectionPad`/`ScrollPad` in this same bar.
    @State private var chipRowOffset: CGFloat = 0
    @GestureState private var chipRowDragTranslation: CGFloat = 0
    @State private var chipRowContentWidth: CGFloat = 0

    /// Clamp a proposed pan so the row can't be dragged past its start or
    /// past its last chip. No-ops (returns 0) when everything already fits.
    private func clampedChipRowOffset(_ proposed: CGFloat, viewportWidth: CGFloat) -> CGFloat {
        guard chipRowContentWidth > viewportWidth else { return 0 }
        let minOffset = viewportWidth - chipRowContentWidth
        return min(0, max(minOffset, proposed))
    }

    /// Mask that fades an edge only when content is actually hidden past it.
    ///
    /// Opaque on both sides when everything fits, so the common case keeps
    /// full opacity and — since a SwiftUI mask also gates hit-testing —
    /// undiminished tap targets.
    private func chipRowFade(viewportWidth: CGFloat) -> some View {
        let offset = clampedChipRowOffset(chipRowOffset + chipRowDragTranslation,
                                          viewportWidth: viewportWidth)
        let hiddenLeading = offset < -0.5
        let hiddenTrailing = chipRowContentWidth + offset > viewportWidth + 0.5
        let fade: CGFloat = viewportWidth > 0 ? min(0.09, 26 / viewportWidth) : 0
        return LinearGradient(
            stops: [
                .init(color: .black.opacity(hiddenLeading ? 0 : 1), location: 0),
                .init(color: .black, location: hiddenLeading ? fade : 0),
                .init(color: .black, location: hiddenTrailing ? 1 - fade : 1),
                .init(color: .black.opacity(hiddenTrailing ? 0 : 1), location: 1),
            ],
            startPoint: .leading,
            endPoint: .trailing)
    }

    var body: some View {
        HStack(spacing: 0) {
            chipRow
            if let onToggleKeyboard {
                keyboardToggle(onToggleKeyboard)
            }
        }
        .frame(height: 50)
        .background(Ink.shortcutBarBG)
        .overlay(alignment: .top) {
            Rectangle().fill(Ink.hairline).frame(height: 1)
        }
        .overlay(alignment: .top) {
            LinearGradient(
                colors: [Ink.accent.opacity(0.08), .clear],
                startPoint: .top,
                endPoint: .bottom
            )
            .frame(height: 14)
            .allowsHitTesting(false)
        }
    }

    /// The voice-input chip. Lives inside the scrolling row like any other
    /// shortcut — reorderable, removable, and free to scroll off-screen —
    /// rather than holding a permanent slot at the trailing edge. Accent-filled
    /// while a session is live, mirroring the armed-ctrl treatment.
    private func micChip(_ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: micActive ? "mic.fill" : "mic")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(micActive ? Color(hex: "090B0D") : Ink.primary)
                .frame(width: Metrics.shortcutKeyWidth, height: 30)
                .background(
                    micActive ? AnyShapeStyle(Ink.shortcutKeyActiveBG) : AnyShapeStyle(Ink.shortcutKeyBG),
                    in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                        .strokeBorder(micActive ? Ink.accent.opacity(0.55) : Ink.groupBorder, lineWidth: 1)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(micActive ? "Stop voice input" : "Start voice input"))
        .accessibilityIdentifier("voice-input")
    }

    /// Pinned at the trailing edge (outside the scroll) so it's always reachable
    /// — collapses or re-pops the system keyboard. Styled as a dark key-chip so
    /// it matches the shortcut keys rather than standing apart as a bare icon.
    private func keyboardToggle(_ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: keyboardDown ? "keyboard" : "keyboard.chevron.compact.down")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(Ink.primary)
                .frame(minWidth: 42, minHeight: 30)
                .background(
                    Ink.shortcutKeyBG,
                    in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                        .strokeBorder(Ink.groupBorder, lineWidth: 1)
                )
                .contentShape(Rectangle())
                // Swap the glyph instantly — no symbol morph — so it doesn't
                // smear while the bar slides down with the dismissing keyboard.
                .contentTransition(.identity)
                .animation(nil, value: keyboardDown)
                .transaction { $0.animation = nil }
        }
        .buttonStyle(.plain)
        .padding(.leading, 4)
        .padding(.trailing, 8)
        .accessibilityLabel(Text(keyboardDown ? "Show keyboard" : "Hide keyboard"))
        .accessibilityIdentifier("keyboard-toggle")
    }

    private var chipRow: some View {
        GeometryReader { viewport in
            ZStack(alignment: .leading) {
                // Measures the row's true, unclamped ideal width in complete
                // isolation from the visible copy below. `.hidden()` here
                // measurably reports the right size once (confirmed via
                // onAppear) but then never fires `.onPreferenceChange` again —
                // hidden subtrees appear to drop out of later preference-
                // propagation passes. `.opacity(0)` (+ explicit hit-testing/
                // accessibility suppression) participates in every normal
                // update pass, so the preference keeps flowing.
                chipRowContent
                    .fixedSize()
                    .opacity(0)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
                    .background(
                        GeometryReader { inner in
                            Color.clear.preference(key: ChipRowContentWidthKey.self, value: inner.size.width)
                        }
                    )

                chipRowContent
                    .offset(x: clampedChipRowOffset(chipRowOffset + chipRowDragTranslation,
                                                     viewportWidth: viewport.size.width))
            }
            .frame(width: viewport.size.width, height: viewport.size.height, alignment: .leading)
            .clipped()
            // Soften whichever edge has content hidden past it. A hard clip
            // makes a chip that merely scrolled off look broken — the row is
            // allowed twelve chips, so overflow is the normal state, not a
            // layout bug — while a fade is the standard "there's more this
            // way" cue and is the only affordance this hand-rolled pan has.
            .mask(chipRowFade(viewportWidth: viewport.size.width))
            .contentShape(Rectangle())
            .gesture(
                // minimumDistance keeps a plain tap from ever engaging this
                // gesture, so it falls through to the chip Buttons untouched.
                DragGesture(minimumDistance: 10)
                    .updating($chipRowDragTranslation) { value, state, _ in
                        state = value.translation.width
                    }
                    .onEnded { value in
                        chipRowOffset = clampedChipRowOffset(
                            chipRowOffset + value.translation.width,
                            viewportWidth: viewport.size.width)
                    }
            )
            .onPreferenceChange(ChipRowContentWidthKey.self) { chipRowContentWidth = $0 }
        }
        .frame(maxWidth: .infinity)
    }

    private var chipRowContent: some View {
        HStack(spacing: 6) {
            ForEach(shortcuts) { shortcut in
                // Both arrow controls are shortcuts too — rendered wherever the
                // user positioned them. The cluster is the default; the
                // joystick stays available for anyone who preferred pushing it.
                if shortcut.kind == .arrows {
                    if let onArrow { ArrowCluster(onArrow: onArrow) }
                } else if shortcut.kind == .dpad {
                    if let onArrow { DirectionPad(onArrow: onArrow, keyboardDown: keyboardDown) }
                } else if shortcut.kind == .scroll {
                    if let onScroll { ScrollPad(onScroll: onScroll) }
                } else if shortcut.kind == .mic {
                    // nil when voice input is off in Settings — the chip
                    // disappears rather than sitting there doing nothing.
                    if let onMic { micChip(onMic) }
                } else if shortcut.kind == .ctrl {
                    // Highlightable while armed — the only chip whose look
                    // depends on live state rather than just its label.
                    Button { onTap(shortcut) } label: {
                        Text("ctrl")
                            .font(Face.mono(12, .semibold))
                            .foregroundStyle(ctrlArmed ? Color(hex: "090B0D") : Ink.primary)
                            .frame(width: Metrics.shortcutKeyWidth, height: 30)
                            .background(
                                ctrlArmed ? AnyShapeStyle(Ink.shortcutKeyActiveBG) : AnyShapeStyle(Ink.shortcutKeyBG),
                                in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                            .overlay(
                                RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                                    .strokeBorder(ctrlArmed ? Ink.accent.opacity(0.55) : Ink.groupBorder, lineWidth: 1)
                            )
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(ctrlArmed ? "Control armed" : "Control")
                } else if shortcut.kind == .keyCombo {
                    // Single keys (⌫, esc, ^C, …) auto-repeat while held.
                    HoldRepeatChip(shortcut: shortcut, onTap: onTap)
                } else {
                    Button {
                        onTap(shortcut)
                    } label: {
                        // One uniform key style — built-in and custom keys
                        // both render as the dark-grey pill (no accent-tinted
                        // custom variant), matching the keyboard toggle.
                        // Paste shows the clipboard glyph instead of a label.
                        Group {
                            if shortcut.kind == .paste {
                                Image(systemName: "doc.on.clipboard")
                                    .font(.system(size: 13, weight: .semibold))
                            } else {
                                Text(shortcut.chipLabel)
                                    .font(Face.mono(11, .semibold))
                            }
                        }
                        .foregroundStyle(Ink.primary)
                        .frame(width: Metrics.shortcutKeyWidth, height: 30)
                        .background(
                            Ink.shortcutKeyBG,
                            in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                        .overlay(
                            RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                                .strokeBorder(Ink.groupBorder, lineWidth: 1)
                        )
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(shortcut.kind == .paste
                                        ? Text("Paste") : Text(shortcut.chipLabel))
                }
            }
        }
        .padding(.horizontal, 8)
    }
}

/// The press-then-push gesture the bar's two drag chips (``DirectionPad``,
/// ``ScrollPad``) engage on.
///
/// ### Why a push has to be told apart from a swipe passing through
///
/// These chips sit at the bottom of the screen — with the keyboard collapsed
/// the bar is at its lowest, and the chip row's underside is a finger's width
/// (~44pt) above the home indicator. That is close enough that iOS's
/// swipe-up-to-background can BEGIN on a chip rather than below it, and a bare
/// `DragGesture(minimumDistance: 0)` fires the instant such a swipe clears the
/// dead-zone: swiping the app away typed a real arrow key into whatever was on
/// screen. Reported as "Claude Code jumps to its session switcher by itself
/// when I background the app" — `←` is exactly that key, and the joystick
/// resolves a push to its dominant axis, so a thumb leaving the bottom edge
/// with any sideways lean sends `←` rather than `↑`. The keyboard being up hid
/// it: the bar rides above the keyboard, nowhere near the swipe.
///
/// Position can't tell the two apart (they begin on the same pixels), but
/// *settling* can: a push against a 42×30 stick lands and stays — it never has
/// to travel further than the dead-zone plus a nudge — while a swipe on its way
/// off the screen is already moving. So the stick only engages once the touch
/// has held still briefly, which is `LongPressGesture`'s exact contract:
/// `maximumDistance` fails it for a moving touch, and a failed gesture sends
/// nothing at all. The dwell is deliberately short — long enough that no swipe
/// survives it (anything iOS reads as a system swipe covers `engageSlop` well
/// inside `engageDelay`), short enough that a human press-then-push doesn't
/// notice it.
/// **The dwell is only owed when the keyboard is down.** Everything above
/// describes the bar at its lowest, a finger's width above the home indicator.
/// With the keyboard up the bar rides above it, nowhere near where a system
/// swipe can begin — which is exactly why the bug only ever reproduced with the
/// keyboard collapsed. Charging every push an 80ms settle in the state where the
/// hazard cannot occur is pure tax, and that state is most of typing. So callers
/// pass the dwell they actually owe: `engageDelay` with the keyboard down, none
/// with it up, where the stick engages on touch-down like a plain drag.
/// **The two constants are one number.** `LongPressGesture` fails as soon as the
/// touch travels past `engageSlop` before `engageDelay` elapses, so what the gate
/// actually enforces is a *speed*: every swipe faster than `engageSlop /
/// engageDelay` is rejected, and anything slower slips through. Shortening the
/// dwell alone therefore makes the gate weaker, not just quicker — 12pt/0.08s
/// rejects everything above 150pt/s, but 12pt/0.05s only catches 240pt/s and up.
/// Trim both together to keep the ratio.
///
/// The ratio has margin: a real swipe-up-to-background covers several hundred
/// points in a couple of tenths, i.e. ~1000pt/s and up, so 150pt/s was already
/// an order of magnitude conservative. The floor is `engageSlop`, not the ratio —
/// a thumb landing rolls onto the glass and its centroid can wander several
/// points before it settles, and a slop tighter than that turns an honest press
/// into a key that didn't respond.
private enum StickGesture {
    /// How long the touch must hold still before the stick engages, in the one
    /// state that needs it (keyboard down — see the note above).
    ///
    /// 0.05s, down from 0.08. Paired with the slop trim below it holds the
    /// rejection speed at ~160pt/s — the same order of conservatism as before —
    /// while cutting what a thumb waits through by a third.
    static let engageDelay: Double = 0.05
    /// How far it may drift during that dwell without failing (SwiftUI's own
    /// "did the finger stay put" rule). 8pt: enough for a thumb's roll-on,
    /// little enough to hold the ratio at the shorter dwell.
    static let engageSlop: CGFloat = 8

    typealias Value = SequenceGesture<LongPressGesture, DragGesture>.Value

    /// - Parameter dwell: seconds the touch must settle before the stick
    ///   engages. `0` engages immediately; only safe where a system
    ///   swipe-to-background cannot start on the control.
    static func gesture(dwell: Double = engageDelay) -> SequenceGesture<LongPressGesture, DragGesture> {
        LongPressGesture(minimumDuration: dwell, maximumDistance: engageSlop)
            .sequenced(before: DragGesture(minimumDistance: 0))
    }

    /// The push so far, or `.zero` while still in the dwell. Also the value the
    /// chips watch for cancellation: `@GestureState` resets it to `.zero` even
    /// when the system steals the touch without ever calling `.onEnded`.
    static func translation(of value: Value) -> CGSize {
        if case .second(_, let drag) = value, let drag { return drag.translation }
        return .zero
    }
}

/// Aggregated arrow-key control — a single key (same footprint as a shortcut
/// chip) you drag like a tiny joystick/thumbstick: push toward a direction to
/// send that arrow, hold to key-repeat, release to stop. Replaces both the
/// 4-way cross and the separate ↑/↓ chips. VoiceOver gets explicit per-axis
/// actions so the drag isn't the only way in.
struct DirectionPad: View {
    let onArrow: (String) -> Void
    /// Whether the software keyboard is collapsed. Drives the engage dwell:
    /// only a collapsed keyboard puts this control within reach of iOS's
    /// swipe-up-to-background, so only then is the settle owed. See
    /// ``StickGesture``.
    var keyboardDown: Bool = true

    @StateObject private var repeater = JoystickRepeater()
    @GestureState private var drag: CGSize = .zero
    @Environment(\.scenePhase) private var scenePhase

    /// Travel before a push registers as a direction (keeps a tap inert).
    ///
    /// 7pt, down from 11. The larger number was chosen while the dwell was
    /// unconditional and the two costs compounded — settle, *then* travel far
    /// enough to be sure. With the dwell gone whenever the keyboard is up, the
    /// dead-zone only has to beat a still thumb's jitter, and 7pt does that while
    /// cutting the distance a push has to cover by a third.
    ///
    /// `static` so the tests can assert against the shipping value instead of
    /// copying the number behind a "matches DirectionPad" comment — the comment
    /// stayed at 11 through this change, and a duplicated constant that lies is
    /// how a dead-zone test quietly stops testing the dead zone.
    static let dragThreshold: CGFloat = 7

    var body: some View {
        // Same footprint/skin as a shortcut chip — it IS one, just dragged.
        Image(systemName: "arrow.up.and.down.and.arrow.left.and.right")
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(repeater.direction == nil ? Ink.primary : Ink.accent)
            .offset(knobOffset(for: drag))
            .animation(.interactiveSpring(duration: 0.12), value: drag)
            .padding(.horizontal, 8)
            .frame(minWidth: 42, minHeight: 30)
            .background(Ink.shortcutKeyBG, in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                    .strokeBorder(repeater.direction == nil ? Ink.groupBorder : Ink.accent.opacity(0.42), lineWidth: 1)
            )
            .contentShape(Rectangle())
            .gesture(
                // Press-then-push ONLY while the keyboard is down, which is the
                // only state where iOS's swipe-to-background can begin on this
                // chip and type an arrow key on its way past. Keyboard up, the
                // push registers on touch-down — see ``StickGesture``.
                StickGesture.gesture(dwell: keyboardDown ? StickGesture.engageDelay : 0)
                    .updating($drag) { value, state, _ in
                        state = StickGesture.translation(of: value)
                    }
                    .onChanged { value in
                        repeater.set(direction(for: StickGesture.translation(of: value)))
                    }
                    .onEnded { _ in repeater.stop() }
            )
            // `.onEnded` doesn't reliably fire when the SYSTEM cancels the drag
            // (an incoming call/notification banner stealing the touch, the app
            // backgrounding mid-hold) — the reported "reconnect leaves the D-pad
            // stuck highlighted, spamming arrow keys" bug. `@GestureState` DOES
            // reliably auto-reset to its initial value on cancellation even
            // then, so watching for that catches what `.onEnded` misses.
            // `scenePhase`/`onDisappear` are additional, cheap safety nets for
            // the same failure mode.
            .onChange(of: drag) { _, newValue in
                if newValue == .zero { repeater.stop() }
            }
            .onChange(of: scenePhase) { _, phase in
                if phase != .active { repeater.stop() }
            }
            .onDisappear { repeater.stop() }
            .onAppear { repeater.onFire = onArrow }
            .accessibilityElement()
            .accessibilityLabel(Text("Arrow keys"))
            .accessibilityHint(Text("Drag toward a direction to send an arrow key"))
            .accessibilityAction(named: Text("Move up")) { onArrow("up") }
            .accessibilityAction(named: Text("Move down")) { onArrow("down") }
            .accessibilityAction(named: Text("Move left")) { onArrow("left") }
            .accessibilityAction(named: Text("Move right")) { onArrow("right") }
            .accessibilityIdentifier("dpad-joystick")
    }

    /// Dominant-axis direction for the current push, or nil inside the
    /// dead-zone (so a stray tap sends nothing).
    private func direction(for t: CGSize) -> String? {
        JoystickRepeater.direction(dx: t.width, dy: t.height, threshold: Self.dragThreshold)
    }

    /// Nudge the glyph toward the push for tactile feedback.
    private func knobOffset(for t: CGSize) -> CGSize {
        guard let dir = direction(for: t) else { return .zero }
        let d: CGFloat = 3
        switch dir {
        case "up":    return CGSize(width: 0, height: -d)
        case "down":  return CGSize(width: 0, height: d)
        case "left":  return CGSize(width: -d, height: 0)
        default:      return CGSize(width: d, height: 0)
        }
    }
}

/// The default arrow control: four tappable zones in one chip-height pill.
/// A tap sends exactly one arrow, a hold repeats it — a hardware keyboard's
/// cadence, and the thing ``DirectionPad`` structurally cannot offer. Reading a
/// direction out of a *push* means the cheapest possible single arrow is
/// press → dwell → drag 11pt → release, which is the reported "I have to hold
/// it just to nudge the cursor one column".
///
/// It gets there without reopening the bug the joystick's dwell defends against
/// (iOS's swipe-up-to-background starting on the chip and typing a real `←` on
/// its way off screen — see ``StickGesture``). The dwell was only ever needed
/// because the joystick fires on movement; here movement fails ``KeyRepeatGesture``
/// outright, which both keeps a system swipe from typing and leaves a horizontal
/// drag to the shortcut bar's own pan.
///
/// Four zones cost ~140pt against a single chip's 46pt. The bar pans, so the
/// width is affordable; a tap that does nothing is not.
struct ArrowCluster: View {
    /// "up" / "down" / "left" / "right", per tap and per repeat tick.
    let onArrow: (String) -> Void

    /// Reading order flattened onto one row. ←→ sit at the outer edges with the
    /// vertical pair between them, so the two directions used for *editing* a
    /// line are the two easiest to hit without looking.
    private static let zones: [(direction: String, symbol: String, label: String)] = [
        ("left",  "arrow.left",  "Move left"),
        ("up",    "arrow.up",    "Move up"),
        ("down",  "arrow.down",  "Move down"),
        ("right", "arrow.right", "Move right"),
    ]

    /// Narrower than a full chip (46pt) so four zones plus their hairlines come
    /// to 131pt — about 2.5 slots rather than four. The full 30pt chip height is
    /// kept: that is the axis a thumb misses on, and every other key in the bar
    /// is already 30pt tall.
    ///
    /// The exact number is load-bearing, and `ShortcutStore` has the test that
    /// says so: at the cluster's default third slot, 32pt zones put its right
    /// edge inside even a 375pt phone's chip viewport. Widening these without
    /// re-checking that clips `→` at rest on small phones.
    static let zoneWidth: CGFloat = 32

    /// Total laid-out width: four zones plus the three hairlines between them.
    /// Mirrored by the toolbar-width test, which can't measure a live view.
    static var totalWidth: CGFloat { zoneWidth * CGFloat(zones.count) + CGFloat(zones.count - 1) }

    var body: some View {
        HStack(spacing: 0) {
            ForEach(Array(Self.zones.enumerated()), id: \.element.direction) { index, zone in
                if index > 0 {
                    // Hairline between zones: four arrows in one pill need to
                    // read as four keys, not one wide key with four glyphs.
                    Rectangle()
                        .fill(Ink.groupBorder)
                        .frame(width: 1, height: 18)
                }
                ArrowZone(direction: zone.direction,
                          symbol: zone.symbol,
                          label: zone.label,
                          width: Self.zoneWidth,
                          onArrow: onArrow)
            }
        }
        .frame(height: 30)
        .background(Ink.shortcutKeyBG, in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                .strokeBorder(Ink.groupBorder, lineWidth: 1)
        )
        // `.contain`, not `.combine`: each zone is its own VoiceOver element, so
        // the four arrows stay four separately-activatable keys.
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("arrow-cluster")
    }
}

/// One arrow inside ``ArrowCluster``. Same tap-once / hold-to-repeat contract as
/// an ordinary chip — it *is* one, minus the pill (the cluster draws that once
/// around all four).
private struct ArrowZone: View {
    let direction: String
    let symbol: String
    let label: String
    let width: CGFloat
    let onArrow: (String) -> Void

    @StateObject private var repeater = HoldRepeater()
    @GestureState private var isPressing = false
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(isPressing ? Ink.accent : Ink.primary)
            .frame(width: width, height: 30)
            .contentShape(Rectangle())
            .gesture(
                KeyRepeatGesture.gesture
                    .updating($isPressing) { value, state, _ in
                        state = KeyRepeatGesture.isRepeatPhase(value)
                    }
                    .onEnded { value in
                        if KeyRepeatGesture.isTap(value) { onArrow(direction) }
                        repeater.stop()
                    }
            )
            // Same stuck-repeat guard as HoldRepeatChip/DirectionPad: `.onEnded`
            // can miss a system-cancelled gesture, but `@GestureState` still
            // auto-resets `isPressing`, so watching it stops a repeat that would
            // otherwise hold an arrow key down indefinitely. Doubles as the
            // repeat-START signal — it flips true exactly when the long press
            // clears its minimum duration.
            .onChange(of: isPressing) { _, pressing in
                if pressing { repeater.start { onArrow(direction) } } else { repeater.stop() }
            }
            .onChange(of: scenePhase) { _, phase in
                if phase != .active { repeater.stop() }
            }
            .onDisappear { repeater.stop() }
            .accessibilityElement()
            .accessibilityLabel(Text(label))
            .accessibilityIdentifier("arrow-\(direction)")
    }
}

/// A chip-sized drag thumb that scrolls the terminal's scrollback — push up for
/// older lines, down for newer, hold to keep scrolling. A conflict-free
/// alternative to the on-terminal swipe: it's its own control, so it never
/// competes with tap-to-focus or long-press-select on the terminal surface.
/// Reuses ``JoystickRepeater`` for the fire-on-enter / hold-to-repeat cadence,
/// acting only on the vertical axis.
struct ScrollPad: View {
    /// "up" / "down", emitted on press and repeated while held.
    let onScroll: (String) -> Void

    @StateObject private var repeater = JoystickRepeater()
    @GestureState private var drag: CGSize = .zero
    @Environment(\.scenePhase) private var scenePhase

    private let threshold: CGFloat = 8

    var body: some View {
        Image(systemName: "arrow.up.and.down")
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(repeater.direction == nil ? Ink.primary : Ink.accent)
            .offset(y: knobOffsetY(for: drag))
            .animation(.interactiveSpring(duration: 0.12), value: drag)
            .padding(.horizontal, 8)
            .frame(minWidth: 42, minHeight: 30)
            .background(Ink.shortcutKeyBG, in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                    .strokeBorder(repeater.direction == nil ? Ink.groupBorder : Ink.accent.opacity(0.42), lineWidth: 1)
            )
            .contentShape(Rectangle())
            .gesture(
                // Same press-then-push gate as the D-pad: a swipe-to-background
                // starting on this chip used to page the scrollback (and put a
                // tmux pane into copy-mode) on its way off screen.
                // Keeps the unconditional dwell: unlike the arrows, scrolling
                // isn't something you reach for mid-sentence with the keyboard
                // up, so there's no friction worth trading a guarantee for.
                StickGesture.gesture()
                    .updating($drag) { value, state, _ in
                        state = StickGesture.translation(of: value)
                    }
                    .onChanged { value in
                        repeater.set(verticalDirection(for: StickGesture.translation(of: value)))
                    }
                    .onEnded { _ in repeater.stop() }
            )
            // See DirectionPad's matching modifiers — same stuck-repeat bug
            // when the system cancels the drag without firing `.onEnded`.
            .onChange(of: drag) { _, newValue in
                if newValue == .zero { repeater.stop() }
            }
            .onChange(of: scenePhase) { _, phase in
                if phase != .active { repeater.stop() }
            }
            .onDisappear { repeater.stop() }
            .onAppear { repeater.onFire = onScroll }
            .accessibilityElement()
            .accessibilityLabel(Text("Scroll history"))
            .accessibilityHint(Text("Drag up or down to scroll the terminal scrollback"))
            .accessibilityAction(named: Text("Scroll up")) { onScroll("up") }
            .accessibilityAction(named: Text("Scroll down")) { onScroll("down") }
            .accessibilityIdentifier("scroll-thumb")
    }

    /// Vertical-only direction (ignores horizontal travel), or nil in the
    /// dead-zone so a tap scrolls nothing.
    private func verticalDirection(for t: CGSize) -> String? {
        guard abs(t.height) >= threshold else { return nil }
        return t.height < 0 ? "up" : "down"
    }

    private func knobOffsetY(for t: CGSize) -> CGFloat {
        guard let dir = verticalDirection(for: t) else { return 0 }
        return dir == "up" ? -3 : 3
    }
}

/// The tap-once / hold-to-repeat recognizer every *tappable* key in the bar
/// uses — ordinary chips (``HoldRepeatChip``) and the arrow cluster's zones
/// (``ArrowCluster``) alike.
///
/// A separate `.onTapGesture` alongside a `.gesture(...)` used to compete for
/// the same touch uncoordinated — no priority/failure relationship between
/// them, so a quick tap could occasionally get swallowed by neither recognizer
/// (the reported "this key sometimes doesn't respond" feel on ordinary chips
/// like Tab/Esc/Ctrl/Backspace). `.exclusively` makes them one recognizer with
/// a defined resolution instead of two independent ones.
///
/// The long press supplies the initial delay before auto-repeat; its
/// `maximumDistance` fails it for a *moving* touch, and that single property
/// buys two things at once:
///
///   - a horizontal drag is left to the shortcut bar's own pan instead of being
///     swallowed by the key it started on, and
///   - iOS's swipe-up-to-background can't type into the bar on its way past.
///     That is the same bug ``StickGesture`` exists to prevent, but a tap key
///     needs no dwell to be safe from it: a swipe is movement, movement fails
///     both halves here, and a failed gesture sends nothing. (A drag-resolved
///     control like ``DirectionPad`` cannot use this rule — movement is how it
///     reads a direction in the first place, which is exactly why a single
///     arrow used to cost a press-and-push.)
private enum KeyRepeatGesture {
    /// How long a stationary press must hold before auto-repeat starts.
    static let holdDelay: Double = 0.3

    typealias Value = ExclusiveGesture<TapGesture, SequenceGesture<LongPressGesture, DragGesture>>.Value

    /// The trailing `DragGesture` isn't there to track a drag — it's there to
    /// give the hold a release event to stop on.
    static var gesture: ExclusiveGesture<TapGesture, SequenceGesture<LongPressGesture, DragGesture>> {
        TapGesture()
            .exclusively(before: LongPressGesture(minimumDuration: holdDelay)
                .sequenced(before: DragGesture(minimumDistance: 0)))
    }

    /// Whether the long-press has cleared its minimum duration and the
    /// trailing drag is now tracking — the signal to (keep) firing repeats.
    static func isRepeatPhase(_ value: Value) -> Bool {
        if case .second(.second(true, _)) = value { return true }
        return false
    }

    /// Whether this ended as a plain tap (fire exactly once) rather than a hold.
    static func isTap(_ value: Value) -> Bool {
        if case .first = value { return true }
        return false
    }
}

/// A single-key shortcut chip that auto-repeats while held — so holding ⌫
/// deletes continuously (the reported "have to tap one by one" bug), like a
/// hardware keyboard. A quick tap still sends exactly once. See
/// ``KeyRepeatGesture`` for why the repeat is gated behind a long press.
private struct HoldRepeatChip: View {
    let shortcut: TerminalShortcut
    let onTap: (TerminalShortcut) -> Void
    @StateObject private var repeater = HoldRepeater()
    @GestureState private var isPressing = false
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Text(shortcut.chipLabel)
            .font(Face.mono(11, .semibold))
            .foregroundStyle(Ink.primary)
            .frame(width: Metrics.shortcutKeyWidth, height: 30)
            .background(Ink.shortcutKeyBG, in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                    .strokeBorder(Ink.groupBorder, lineWidth: 1)
            )
            .contentShape(Rectangle())
            .gesture(
                KeyRepeatGesture.gesture
                    // `.onChanged` needs `Value: Equatable`, which this
                    // combined gesture's Value can't be (it carries
                    // TapGesture's Void) — driving the repeat start off the
                    // `isPressing` GestureState change below instead.
                    .updating($isPressing) { value, state, _ in
                        state = KeyRepeatGesture.isRepeatPhase(value)
                    }
                    .onEnded { value in
                        if KeyRepeatGesture.isTap(value) { onTap(shortcut) }
                        repeater.stop()
                    }
            )
            // Same stuck-repeat bug as DirectionPad/ScrollPad: `.onEnded` can
            // miss a system-cancelled gesture, but `@GestureState` still
            // auto-resets `isPressing` to false — watching that (plus
            // scenePhase/onDisappear as extra safety nets) stops the repeat
            // even then, instead of holding ⌫ (etc.) hostage indefinitely.
            // Also doubles as the repeat-START signal: `isPressing` flips
            // true exactly when the long-press clears its minimum duration.
            .onChange(of: isPressing) { _, pressing in
                if pressing {
                    repeater.start { onTap(shortcut) }
                } else {
                    repeater.stop()
                }
            }
            .onChange(of: scenePhase) { _, phase in
                if phase != .active { repeater.stop() }
            }
            .onDisappear { repeater.stop() }
            .accessibilityLabel(Text(shortcut.chipLabel))
    }
}

/// Drives the joystick's fire-on-enter + hold-to-repeat cadence. Firing once
/// when a direction is first acquired, then auto-repeating after a short
/// initial delay, mirrors hardware key-repeat.
@MainActor
final class JoystickRepeater: ObservableObject {
    /// Sends a direction ("up"/"down"/"left"/"right") to the PTY.
    var onFire: ((String) -> Void)?
    /// Currently-held direction, or nil at center. Published for the glyph tint.
    @Published private(set) var direction: String?

    private var timer: Timer?
    private var ticks = 0
    private let tick: TimeInterval = 0.07
    private let initialDelayTicks = 5   // ~0.35s before auto-repeat kicks in

    /// Dominant-axis direction for a push of `(dx, dy)`, or nil inside the
    /// dead-zone. Pure + nonisolated so it's unit-testable without a view.
    nonisolated static func direction(dx: CGFloat, dy: CGFloat, threshold: CGFloat) -> String? {
        if abs(dx) < threshold && abs(dy) < threshold { return nil }
        if abs(dx) > abs(dy) { return dx > 0 ? "right" : "left" }
        return dy > 0 ? "down" : "up"
    }

    /// Update the held direction. Fires immediately on a *new* direction, then
    /// lets the timer auto-repeat while it's held.
    func set(_ dir: String?) {
        guard dir != direction else { return }
        direction = dir
        guard let dir else { stop(); return }
        onFire?(dir)
        ticks = 0
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: tick, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, let dir = self.direction else { return }
                // `scenePhase`'s onChange races app suspension — it can lose,
                // leaving this timer un-invalidated. iOS never RUNS the timer
                // while suspended, but a tick already in flight the instant
                // suspension hits can still fire once on resume, replaying a
                // stale direction (reported: an arrow key firing into a
                // freshly-foregrounded session, e.g. recalling shell history
                // in whatever's reading it) — refuse to fire unless the app
                // is verifiably active right now, not just "wasn't told to stop".
                guard UIApplication.shared.applicationState == .active else { self.stop(); return }
                self.ticks += 1
                if self.ticks >= self.initialDelayTicks { self.onFire?(dir) }
            }
        }
    }

    func stop() {
        timer?.invalidate()
        timer = nil
        ticks = 0
        direction = nil
    }
}

/// Hold-to-repeat for a single key chip. The caller gates `start` behind a long
/// press (which supplies the initial delay), so we fire once immediately and
/// then repeat at a steady cadence until `stop`.
@MainActor
final class HoldRepeater: ObservableObject {
    private var timer: Timer?
    private var running = false
    private let tick: TimeInterval = 0.07   // ~14/s, matches the joystick repeat

    func start(_ fire: @escaping () -> Void) {
        guard !running else { return }   // .onChanged fires repeatedly; arm once
        running = true
        fire()
        timer = Timer.scheduledTimer(withTimeInterval: tick, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, self.running else { return }
                // See JoystickRepeater's identical guard: `scenePhase`'s
                // onChange can lose its race with app suspension, leaving
                // this timer un-invalidated for one stale tick on resume.
                guard UIApplication.shared.applicationState == .active else { self.stop(); return }
                fire()
            }
        }
    }

    func stop() {
        timer?.invalidate()
        timer = nil
        running = false
    }
}

// MARK: - Roam banner (screen 5)

struct RoamBanner: View {
    let metrics: SessionMetrics?
    @State private var pulsing = false

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(spacing: 8) {
                Circle()
                    .fill(Ink.warn)
                    .frame(width: 8, height: 8)
                    .shadow(color: Ink.warn.opacity(0.8), radius: 6)
                    .scaleEffect(pulsing ? 1.6 : 1)
                    .opacity(pulsing ? 0.55 : 1)
                Text("Wi-Fi → 5G")
                    .font(Face.display(13, .semibold))
                    .foregroundStyle(Ink.primary)
                Spacer()
                Text("roaming…")
                    .font(Face.mono(10, .bold))
                    .kerning(0.6)
                    .foregroundStyle(Ink.warn)
            }
            HStack(spacing: 8) {
                Text("SRTT").font(Face.mono(10)).foregroundStyle(Ink.secondary)
                if let srtt = metrics?.srttMs {
                    Text("\(Int(srtt))ms")
                        .font(Face.mono(10))
                        .foregroundStyle(Ink.primary)
                }
                Sparkline(samples: metrics?.srttHistory ?? [])
                    .stroke(Ink.warn, lineWidth: 1.4)
                    .frame(width: 64, height: 18)
                Spacer()
                HStack(spacing: 5) {
                    Circle().fill(Ink.mosh).frame(width: 5, height: 5)
                        .shadow(color: Ink.mosh.opacity(0.7), radius: 3)
                    Text("Predict ON")
                        .font(Face.mono(10))
                        .foregroundStyle(Ink.predictOnText)
                }
            }
        }
        .padding(EdgeInsets(top: 12, leading: 14, bottom: 12, trailing: 14))
        .background(Ink.roamBanner)
        .background(.ultraThinMaterial)
        .clipShape(RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                .strokeBorder(Ink.warn.opacity(0.34), lineWidth: 1))
        .shadow(color: .black.opacity(0.42), radius: 19, y: 9)
        .onAppear {
            withAnimation(Motion.roamPulse) {
                pulsing = true
            }
        }
    }
}

struct Sparkline: Shape {
    let samples: [Double]

    func path(in rect: CGRect) -> Path {
        var path = Path()
        guard samples.count >= 2,
              let minValue = samples.min(),
              let maxValue = samples.max()
        else { return path }
        let span = max(maxValue - minValue, 1)
        let step = rect.width / CGFloat(samples.count - 1)
        for (i, sample) in samples.enumerated() {
            let x = CGFloat(i) * step
            let y = rect.height - (CGFloat((sample - minValue) / span) * rect.height)
            if i == 0 { path.move(to: CGPoint(x: x, y: y)) }
            else { path.addLine(to: CGPoint(x: x, y: y)) }
        }
        return path
    }
}

// MARK: - Terminal connecting / idle state

/// The branded waiting screen shown before a session is live (and while a
/// dropped one is coming back), in place of a black void with a lone spinner.
/// A large, dimmed crowd-surf watermark plus the host identity answers "who am
/// I connecting to, and what's happening" — and yields entirely to SwiftTerm
/// the moment there's real output. Prototype: the moshpit OD project's
/// `terminal-idle.html`.
struct TerminalConnectingView: View {
    let connection: ServerConnection
    let state: TransportConnState

    private var accent: SwiftUI.Color {
        switch state {
        case .reconnecting: return Ink.signal   // mosh roam — transport blue
        case .offline: return Ink.danger
        default: return Ink.accent
        }
    }

    private var statusText: String {
        switch state {
        case .reconnecting: return String(localized: "Riding the handoff")
        case .offline: return String(localized: "Line dropped — retrying")
        default: return String(localized: "Opening the pit")
        }
    }

    private var flavor: String {
        connection.connectionProtocol == .mosh
            ? String(localized: "mosh keeps the line up · sessions survive the handoff")
            : String(localized: "the pit never closes · your sessions wait for you")
    }

    var body: some View {
        ZStack {
            Ink.terminalBG.ignoresSafeArea()

            // The same grid the home screen stands on — a stage, not a void.
            SignalGrid()
                .stroke(Ink.terminalGrid, lineWidth: 0.6)
                .ignoresSafeArea()

            // Accent pool under the mark, bright enough to actually read.
            RadialGradient(
                colors: [accent.opacity(0.13), .clear],
                center: UnitPoint(x: 0.5, y: 0.38),
                startRadius: 0, endRadius: 300)
                .ignoresSafeArea()

            VStack(spacing: 0) {
                PitLoaderMark(side: 112, accent: accent)

                Text(connection.displayName)
                    .font(Face.display(24, .bold))
                    .foregroundStyle(Ink.primary)
                    .padding(.top, 28)

                // `String(port)`, not interpolation: Text reads its argument as a
                // LocalizedStringKey and puts a grouping separator in an Int, so
                // port 2222 rendered as "2,222".
                Text(verbatim: "\(connection.username)@\(connection.host):\(String(connection.port))")
                    .font(Face.mono(12.5))
                    .foregroundStyle(Ink.tertiary)
                    .padding(.top, 6)

                // Status capsule — same family as the breadcrumb's transport
                // pill, so "what's happening" wears the app's own uniform.
                HStack(spacing: 10) {
                    if state == .offline {
                        Circle().fill(accent).frame(width: 8, height: 8)
                            .shadow(color: accent.opacity(0.8), radius: 4)
                    } else {
                        PulseDots(color: accent)
                    }
                    Text(statusText.uppercased())
                        .font(Face.mono(11, .semibold))
                        .kerning(1.8)
                        .foregroundStyle(Ink.secondary)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 9)
                .background(Color.white.opacity(0.04), in: Capsule())
                .overlay(Capsule().strokeBorder(accent.opacity(0.22), lineWidth: 1))
                .padding(.top, 34)
            }
            .offset(y: -14)

            VStack {
                Spacer()
                Text(flavor)
                    .font(Face.mono(10.5))
                    .kerning(0.6)
                    .foregroundStyle(Ink.meta)
                    .padding(.bottom, 18)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// The connecting screen's mark, following whichever app icon the user picked.
///
/// The default icon keeps its bespoke choreography — the surfer bobs, the crowd
/// pulses — because that animation is authored against the crowd-surf geometry
/// itself. None of the alternates HAS a surfer or a crowd (one is a handset,
/// one is a house, one is the characters `:wq`), so animating them the same way
/// is not merely wrong, it is undefined. They breathe instead: the same tempo
/// as the surfer, carried by the artwork the user chose.
private struct PitLoaderMark: View {
    var side: CGFloat
    var accent: SwiftUI.Color
    @Environment(AppSettings.self) private var settings

    var body: some View {
        let option = AppIconCatalog.option(for: settings.appIconId)
        if option.isPrimary {
            PitLoaderGlyph(side: side, accent: accent)
        } else {
            BreathingIconMark(option: option, side: side, accent: accent)
        }
    }
}

/// An alternate icon, alive while the session connects.
private struct BreathingIconMark: View {
    let option: AppIconOption
    var side: CGFloat
    var accent: SwiftUI.Color
    @State private var breathing = false

    var body: some View {
        AppIconThumb(option: option, side: side)
            .scaleEffect(breathing ? 1.04 : 0.96)
            // The glow is the accent, not the artwork's own colours: it is the
            // connection state talking, and it has to read the same whichever
            // icon is underneath.
            .shadow(color: accent.opacity(breathing ? 0.55 : 0.2),
                    radius: side * (breathing ? 0.16 : 0.09))
            // Matches PitLoaderGlyph's surfer, so the two loaders keep time.
            .animation(.easeInOut(duration: 1.15).repeatForever(autoreverses: true),
                       value: breathing)
            .frame(width: side, height: side)
            .onAppear { breathing = true }
    }
}

/// The connecting screen's animated mark: same geometry as ``MoshpitGlyph``
/// (all values from its `Metrics`), drawn alive — the surfer bobs mid-air and
/// the crowd pulses in a stagger underneath. The pit is warming up. Kept local
/// to the loader; every static placement of the mark stays `MoshpitGlyph`.
private struct PitLoaderGlyph: View {
    private typealias M = MoshpitGlyph.Metrics

    var side: CGFloat
    var accent: SwiftUI.Color
    @State private var bob = false

    var body: some View {
        let k = side / 24
        ZStack {
            ForEach(Array(M.crowdXs.enumerated()), id: \.offset) { i, x in
                RoundedRectangle(cornerRadius: M.crowdCorner * k, style: .continuous)
                    .fill(Ink.primary.opacity(M.crowdAlpha))
                    .frame(width: M.crowdSize.width * k,
                           height: M.crowdSize.height * k)
                    .offset(x: (x + M.crowdSize.width / 2 - 12) * k,
                            y: (M.crowdY + M.crowdSize.height / 2 - 12) * k
                                + (bob ? -0.5 * k : 0.4 * k))
                    .animation(.easeInOut(duration: 0.85)
                        .repeatForever(autoreverses: true)
                        .delay(Double(i) * 0.14), value: bob)
            }

            RoundedRectangle(cornerRadius: M.surferCorner * k, style: .continuous)
                .fill(accent)
                .frame(width: M.surfer.width * k, height: M.surfer.height * k)
                .rotationEffect(.degrees(-M.surferTilt + (bob ? 3 : -3)))
                .offset(x: (M.surfer.midX - 12) * k,
                        y: (M.surfer.midY - 12) * k + (bob ? -1.6 * k : 1.2 * k))
                .shadow(color: accent.opacity(0.8), radius: 2.6 * k)
                .animation(.easeInOut(duration: 1.15)
                    .repeatForever(autoreverses: true), value: bob)
        }
        .frame(width: side, height: side)
        .onAppear { bob = true }
    }
}

/// Three mini cursor blocks blinking in a stagger — the crowd itself as the
/// loading indicator, replacing the generic system spinner.
private struct PulseDots: View {
    var color: SwiftUI.Color
    @State private var on = false

    var body: some View {
        HStack(spacing: 4) {
            ForEach(0..<3, id: \.self) { i in
                RoundedRectangle(cornerRadius: 1.5, style: .continuous)
                    .fill(color)
                    .frame(width: 5, height: 9)
                    .opacity(on ? 1 : 0.25)
                    .animation(.easeInOut(duration: 0.55)
                        .repeatForever(autoreverses: true)
                        .delay(Double(i) * 0.18), value: on)
            }
        }
        .onAppear { on = true }
    }
}

// MARK: - Multiplexer empty state (server has no sessions)

/// Shown while a multiplexer session is expected but absent. The first few
/// seconds are the normal attach handshake, so a spinner shows; after the
/// grace period the honest state is one of two things, no longer conflated:
///   - `binaryMissing` → "not installed on this host" + an Install action
///     (the design's mis-diagnosis fix).
///   - otherwise → "No sessions" + a Create action (Moshpit never creates
///     a session behind the user's back).
///
/// Shared by tmux (unattached `-CC`) and herdr (`server_not_running` from the
/// snapshot poller) — the situation and the two honest answers are identical,
/// only the name differs.
private struct MultiplexerEmptyStateView: View {
    let multiplexer: Multiplexer
    let binaryMissing: Bool
    /// Identity for the shared connecting view, so the wait before this screen
    /// decides anything looks like the SAME wait the connection already showed.
    let connection: ServerConnection
    let connectionState: TransportConnState
    let onCreate: () -> Void
    let onInstall: () -> Void
    @State private var settled = false
    @State private var creating = false

    var body: some View {
        VStack(spacing: 14) {
            if settled {
                if binaryMissing {
                    missingState
                } else {
                    noSessionsState
                }
            } else {
                // The SAME view the connection itself showed, not a second
                // spinner with its own label. Attaching is a phase of opening
                // the session, not a new thing to wait for, and two
                // back-to-back loading states read as one stall that restarted.
                TerminalConnectingView(connection: connection, state: connectionState)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(settled ? 24 : 0)
        .background(Ink.terminalBG)
        .task {
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            withAnimation(.easeOut(duration: 0.25)) { settled = true }
        }
    }

    @ViewBuilder
    private var missingState: some View {
        Image(systemName: "shippingbox")
            .font(.system(size: 28, weight: .light))
            .foregroundStyle(Ink.meta)
        Text("\(multiplexer.label) not installed on this host")
            .font(Face.display(16, .semibold)).foregroundStyle(Ink.primary)
            .multilineTextAlignment(.center)
        Text("Moshpit needs \(multiplexer.label) for \(multiplexer.vocabulary.session.lowercased()) navigation.\nInstall it, then reconnect.")
            .font(Face.text(12)).foregroundStyle(Ink.meta)
            .multilineTextAlignment(.center)
        Button(action: onInstall) {
            Text("Install \(multiplexer.label)")
                .font(Face.text(14).weight(.semibold))
                .foregroundStyle(Color(hex: "090B0D"))
                .padding(.horizontal, 18).padding(.vertical, 9)
                .background(Ink.accent, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("multiplexer-install")
    }

    @ViewBuilder
    private var noSessionsState: some View {
        Image(systemName: "square.stack.3d.up.slash")
            .font(.system(size: 28, weight: .light))
            .foregroundStyle(Ink.meta)
        Text("No \(multiplexer.label) \(multiplexer.vocabulary.sessionPlural.lowercased())")
            .font(Face.display(16, .semibold)).foregroundStyle(Ink.primary)
        Text("This server has no running \(multiplexer.vocabulary.sessionPlural.lowercased()).\nMoshpit only attaches — create the first one to start.")
            .font(Face.text(12)).foregroundStyle(Ink.meta)
            .multilineTextAlignment(.center)
        Button {
            creating = true
            onCreate()
        } label: {
            Text(creating ? "Creating…" : "Create \(multiplexer.vocabulary.session)")
                .font(Face.text(14).weight(.semibold))
                .foregroundStyle(Color(hex: "090B0D"))
                .padding(.horizontal, 18).padding(.vertical, 9)
                .background(Ink.accent, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
        }
        .buttonStyle(.plain)
        .disabled(creating)
        .accessibilityIdentifier("multiplexer-create-first-session")
    }
}

/// Plain-text live counters from `MoshTransport`, shown on long-pressing the
/// terminal's protocol pill. See `MoshDiagnostics` for what each value means
/// and why this exists instead of a proper debugger/log pull — it's meant to
/// be screenshotted and sent back, not read as a polished UI.
struct MoshDiagnosticsView: View {
    let diagnostics: MoshDiagnostics?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("MOSH DIAGNOSTICS")
                .font(Face.mono(10, .bold))
                .kerning(0.6)
                .foregroundStyle(Ink.meta)
            if let d = diagnostics {
                row("datagrams", d.datagramsReceived)
                row("applied", d.appliedHostNum)
                row("parse fails", d.parseFailures)
                row("gaps", d.gapEvents)
            } else {
                Text("No datagrams yet.")
                    .font(Face.mono(12))
                    .foregroundStyle(Ink.primary)
            }
        }
        .padding(14)
        .accessibilityIdentifier("mosh-diagnostics-overlay")
    }

    private func row(_ label: String, _ value: UInt64) -> some View {
        HStack {
            Text(label).font(Face.mono(12)).foregroundStyle(Ink.meta)
            Spacer(minLength: 20)
            Text("\(value)").font(Face.mono(12, .bold)).foregroundStyle(Ink.primary)
        }
    }
}
