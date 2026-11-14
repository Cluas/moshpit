import MoshpitKit
import PhotosUI
import QuickLook
import SwiftUI
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
    /// What "leave" means here. On an iPhone the screen is pushed and pops;
    /// on an iPad it is the detail column of a split view, where there is
    /// nothing to pop, so Home supplies the action. nil = `dismiss()`.
    let onLeave: (() -> Void)?
    /// Present on an iPad only: the leading button toggles the sidebar
    /// instead of going back — the connection list is beside the terminal,
    /// not behind it.
    let onToggleSidebar: (() -> Void)?

    init(connection: ServerConnection, hub: SessionHub,
         onLeave: (() -> Void)? = nil, onToggleSidebar: (() -> Void)? = nil) {
        self._connection = State(initialValue: connection)
        self.hub = hub
        self.onLeave = onLeave
        self.onToggleSidebar = onToggleSidebar
    }

    @Environment(AppSettings.self) private var settings
    @Environment(ThemeStore.self) private var themes
    @Environment(SessionMetricsRegistry.self) private var metricsRegistry
    @Environment(ShortcutStore.self) private var shortcutStore
    @Environment(AgentActivityMonitor.self) private var monitor
    @Environment(DeepLinkRouter.self) private var router
    @Environment(ConnectionStoreHolder.self) private var connectionHolder
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase

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
    /// When the multiplexer was first seen attached while a claimed pane
    /// waited: its patience counts from there (B468).
    @State private var pendingPaneAttachedAt: Date?
    /// Sticky-Ctrl armed: the next typed key goes out as its control code.
    @State private var ctrlArmed = false
    /// The coordinator it is armed on: one at a time, let go when typing
    /// moves elsewhere (B411).
    @State private var stickyCtrl = StickyCtrl()
    /// URL found in text the user just copied in-terminal — drives the
    /// transient "Open link" chip (plain-text URLs aren't tappable; select +
    /// copy is the natural gesture, so meet the user right after it).
    @State private var copiedURL: URL?
    @State private var copiedURLDismiss: Task<Void, Never>?
    /// Live voice-input session; non-nil while the dictation overlay is up.
    /// Minted on mic tap (not at screen init) so idle terminals never touch
    /// audio plumbing.
    @State private var dictation: VoiceDictationController?
    /// Live image-attachment flow; non-nil while its overlay is up. Minted
    /// when the picker returns (not on chip tap) so cancelling the picker
    /// leaves no state behind.
    @State private var imageAttachment: ImageAttachmentController?
    @State private var photoPickerPresented = false
    @State private var photoSelection: [PhotosPickerItem] = []
    /// Whether the clipboard currently carries an image — drives the chip
    /// menu's "Paste image" entry. Type metadata only (`hasImages`), which is
    /// cheap and never triggers the system paste prompt; the actual data read
    /// happens off-main when the user commits.
    @State private var clipboardHasImage = false
    /// Keeps the connecting cover up for a beat after `connState` reaches
    /// `.live`, so it fades out over a painted terminal instead of over the
    /// black not-yet-attached pane. See the `.onChange(of: connState)`.
    @State private var settleGrace = false
    /// Whether this screen's session has ever been `.live` — the difference
    /// between a first connect (nothing behind the cover but the void, so the
    /// poster is the honest picture) and a reconnect (a frame worth keeping
    /// on screen is right there). Reset when the session object changes,
    /// unless this screen swapped it in itself (``swapCarriesLive``).
    @State private var wasEverLive = false
    /// A manual reconnect (a protocol switch, Install Assist, the ended
    /// card's Reconnect) is swapping in a new session while this screen
    /// shows a reconnect's frame: the new session comes up as a reconnect
    /// too, under the veil, never the first connect's poster (the owner's
    /// rule: a reconnect never shows the poster). A protocol switch always
    /// carries it. Read and cleared when the session changes
    /// (``sessionSwapped()``).
    @State private var swapCarriesLive = false
    /// A protocol switch (mosh ⇄ SSH) is under way, from its confirm until
    /// the new session's first frame is uncovered: nothing moves (the
    /// owner: 「mosh 切换 ssh 有个动画 我们直接去掉」). The veil and its
    /// capsule are there at once, the pane's last picture stays under it
    /// (``switchStill``, the NSH stage's own on that page) and goes at once
    /// with the veil, the pill and the breadcrumb change at once. Ordinary
    /// drops and resumes keep their motion.
    @State private var stillSwitch = false
    /// The SwiftUI chrome's picture of the pane through a protocol switch
    /// (the NSH page's stage holds its own).
    @State private var switchStill: TerminalStill?

    /// Whether this screen has put the cover up for a phase that was not
    /// `.live` — the only licence for a cover to still be up once the
    /// transport reads `.live` (waiting for the first frame, see `settling`).
    ///
    /// A screen that appears onto a session that is ALREADY live (entered
    /// from the home card or the session tree after connecting there) never
    /// shows a cover: the poster would be up for the one frame before the
    /// pane paints and come straight back down in the very update that
    /// resolves `active` and hosts the pane. On iOS 27 that exact sequence
    /// left every SwiftUI control in the top bar deaf to taps for the life
    /// of the screen — breadcrumb, transport pill, back — while the UIKit
    /// terminal beneath kept working (rig, iPhone 17 Pro iOS 27.0
    /// simulator, 2026-09-14; iOS 26.2 was unaffected). Holding the poster a
    /// beat longer avoided it; not putting it up at all is the honest
    /// version, since there is nothing to cover.
    @State private var coverArmed = false
    /// The last state the poster announced before the transport came back:
    /// the label it keeps while the cover waits for the first frame, so a
    /// reconnect does not read "Opening the pit" for its last few hundred ms.
    @State private var lastCoverState: TransportConnState = .connecting
    /// When the live session last dropped, or the screen was entered onto
    /// one that wasn't live (the veil came up): read by the tap that marks
    /// the way back (``connectionFeedback(from:to:droppedAt:now:)``).
    @State private var droppedAt: Date?
    /// Camera capture sheet (an attach-image source; menu entry hidden where
    /// there is no camera, e.g. the simulator).
    @State private var showCamera = false
    /// A fetched host file on its way into QuickLook (a tapped path link).
    @State private var quickLookURL: URL?
    /// A tapped host FOLDER, presented as the folder sheet.
    @State private var remoteFolder: RemoteFolderTarget?
    /// A tapped host video is playing (`RemoteVideoPlayer`, presented
    /// outside SwiftUI — this only tracks it for `anySheetOpen`).
    @State private var remoteVideoOpen = false

    /// iPad landscape trims the app's own chrome (top bar 54→42, shortcut bar
    /// 50→38, pane inset 4→2). With the software keyboard up, an iPad mini has
    /// ~744pt of height and the keyboard takes ~400 of it — the ~24pt clawed
    /// back here is two more terminal rows. Size classes can't express this
    /// (an iPad is regular×regular in both orientations), so it's geometry:
    /// pad + wider-than-tall. Deliberately NOT keyed on the keyboard — a bar
    /// that changes height mid keyboard transition would fight the frame-hold
    /// machinery in TerminalHostContainer.
    @State private var compactChrome = false

    private var theme: TerminalTheme {
        themes.theme(id: settings.themeId)
    }

    private var metrics: SessionMetrics? { metricsRegistry.metrics[connection.id] }

    /// A single observed signal that advances as the multiplexer snapshot
    /// populates — attach flips, then panes accumulate. Watched by one
    /// `.onChange` so the deep-link pane selection retries without bloating the
    /// view-body type-check. Reads ``tmuxSnapshot``, i.e. whichever control
    /// plane is live: over mosh that is the sidecar, which a detached task
    /// wires well after `start()` returns.
    private var paneReadinessSignal: Int {
        guard let snapshot = tmuxSnapshot, snapshot.isAttached else { return 0 }
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
        pendingPaneAttachedAt = nil
        Log.island.info("land: claimed #\(request.token, privacy: .public) pane=\(request.paneId ?? "-", privacy: .public)")
    }

    /// Land on the claimed pane once the tmux control plane is attached and
    /// the pane is present in the snapshot. Retried from `.onChange(of:)` as
    /// snapshots arrive, since the pane list populates a few round-trips
    /// after attach. If the pane never shows up, connection-level landing
    /// (the active pane) is an acceptable fallback — nothing forces it, so a
    /// stale/closed pane id is simply dropped.
    ///
    /// Every control plane, not just SSH's in-band controller. This used to
    /// read `tmuxController` alone, which is nil over mosh — so a tapped
    /// "finished" notification on a mosh+tmux connection opened the terminal
    /// on whatever pane mosh was already showing, and the claimed request sat
    /// here unacted on. Same order as ``tmuxSnapshot``.
    private func selectPendingPaneIfReady() {
        guard let paneId = pendingPaneSelection else { return }
        let attached = tmuxSnapshot?.isAttached == true
        // A drop (a reconnect) starts the patience over at the next attach.
        if !attached { pendingPaneAttachedAt = nil }
        if attached, pendingPaneAttachedAt == nil {
            // Its patience starts now; looked at again once it has run out,
            // whether or not the snapshot changes by then (B468).
            pendingPaneAttachedAt = Date()
            Task { @MainActor in
                try? await Task.sleep(for: .seconds(Self.landingPatience + 0.5))
                selectPendingPaneIfReady()
            }
        }
        let step = Self.landingStep(
            present: tmuxSnapshot?.panes[paneId] != nil, attached: attached,
            attachedFor: pendingPaneAttachedAt.map { Date().timeIntervalSince($0) })
        switch step {
        case .wait:
            Log.island.info("land: waiting pane=\(paneId, privacy: .public) attached=\(attached, privacy: .public) panes=\(tmuxSnapshot?.panes.count ?? -1, privacy: .public)")
            return
        case .drop:
            // A pane the server doesn't have (closed since the notification,
            // a stale id): dropped, as this always meant to. Left waiting,
            // the first pane to take its id later (herdr numbers panes anew
            // after a restart) pulled the screen there out of nowhere.
            pendingPaneSelection = nil
            pendingPaneAttachedAt = nil
            Log.island.notice("land: pane \(paneId, privacy: .public) is not on the server \(Int(Self.landingPatience), privacy: .public) s after attach; dropped")
            return
        case .select:
            break
        }
        guard let snapshot = tmuxSnapshot else { return }
        pendingPaneSelection = nil
        pendingPaneAttachedAt = nil
        Log.island.info("land: selecting pane=\(paneId, privacy: .public) from=\(snapshot.activePaneId ?? "-", privacy: .public) phase=\(String(describing: scenePhase), privacy: .public)")
        if let controller = active?.tmuxController {
            controller.selectPane(paneId)
        } else if let control = active?.moshControl {
            control.selectPane(paneId)
        } else {
            active?.herdrControl?.selectPane(paneId)
        }
    }

    /// How long a claimed pane may be missing from an attached snapshot
    /// before it is dropped (B468): tmux's pane list fills a few round trips
    /// after attach; herdr's tree comes whole.
    nonisolated static let landingPatience: TimeInterval = 8

    /// What a claimed landing does now (B468).
    enum LandingStep: Equatable {
        /// Not attached yet, or the pane may still come.
        case wait
        /// The pane is there: go.
        case select
        /// Attached past ``landingPatience`` without the pane: drop it.
        case drop
    }

    /// The landing's step for a pane `present` in the snapshot or not, the
    /// multiplexer `attached` for `attachedFor` seconds (nil: not seen
    /// attached yet).
    nonisolated static func landingStep(present: Bool, attached: Bool,
                                        attachedFor: TimeInterval?) -> LandingStep {
        guard attached else { return .wait }
        if present { return .select }
        guard let attachedFor, attachedFor >= landingPatience else { return .wait }
        return .drop
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

    /// Whether those hooks have been read once for the current controller —
    /// the breadcrumb holds the previous plan until they have.
    private var multiplexerAgentHooksLoaded: Bool {
        if let controller = active?.tmuxController { return controller.agentHooksLoaded }
        if let control = active?.moshControl { return control.agentHooksLoaded }
        return active?.herdrControl?.agentHooksLoaded ?? false
    }

    /// True while any full-cover modal is up — the tmux navigation sheets and
    /// the camera. Everything here goes through `presentSheet` on the way in,
    /// so the keyboard collapses for the modal and comes back afterwards if it
    /// was up before: the modal borrows the screen, it doesn't get to change
    /// the keyboard's expand/collapse state ("图片插入的时候要保留键盘展开收起
    /// 状态").
    private var anySheetOpen: Bool {
        showWindowsSheet || showSessionsSheet || showPaneSheet || showCamera
            || quickLookURL != nil || remoteFolder != nil || remoteVideoOpen
    }

    /// Whether the pane behind a modal is pinned to the size it already has.
    /// See ``TerminalHostContainer/geometryHeld`` for what this is protecting
    /// against; `presentSheet` sets it and `releaseGeometryHold` clears it.
    @State private var geometryHold = false
    @State private var geometryHoldRelease: Task<Void, Never>?

    /// Collapse the keyboard before presenting a sheet so it doesn't cover
    /// the sheet's content (the window / session / pane lists sit at the
    /// bottom). Paired with `anySheetOpen` gating focus so the terminal
    /// doesn't immediately pop the keyboard back up.
    ///
    /// The keyboard's space is NOT handed to the terminal while this happens.
    /// It is about to be handed straight back — the sheet dismissing restores
    /// focus — and letting the pane grow into it means reflowing the buffer
    /// and resizing the remote twice, for a switch that never changed the
    /// size. Nobody sees the intermediate size either way; the sheet is over
    /// the whole screen.
    private func presentSheet(_ open: @escaping () -> Void) {
        geometryHoldRelease?.cancel()
        geometryHoldRelease = nil
        geometryHold = true
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        open()
    }

    /// Let the pane adopt the container's bounds again, once the sheet is
    /// gone AND the keyboard has had time to come back into the space it
    /// left. If the keyboard isn't coming back (it was down before the
    /// sheet), the release is immediate and costs the one resize that is
    /// genuinely owed.
    private func releaseGeometryHold() {
        guard geometryHold else { return }
        geometryHoldRelease?.cancel()
        guard keyboardIntent == .up else {
            geometryHold = false
            return
        }
        geometryHoldRelease = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(550))
            guard !Task.isCancelled else { return }
            geometryHold = false
        }
    }

    private var transportKind: TransportPillKind {
        if connection.connectionProtocol == .mosh {
            return metrics?.isRoaming == true ? .moshRoaming : .mosh
        }
        return .ssh
    }

    /// Live connection state for the transport pill — so a dropped/reconnecting
    /// session is visible, not silent.
    /// Delegated to the view model so the pill, this screen and the home card's
    /// row cannot disagree — see ``TerminalViewModel/connState``. No session yet
    /// reads as connecting: the screen exists because one is being opened.
    private var connState: TransportConnState {
        active?.viewModel.connState ?? .connecting
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
        Group {
        if usesNSH {
            nshPage
        } else {
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
                VStack(spacing: 6) {
                    Text(notice)
                        .font(Face.mono(11))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 12).padding(.vertical, 6)
                        .background(Ink.warn.opacity(0.92), in: Capsule())
                    // Some notices carry their own remedy (herdr version skew →
                    // restart the server). One tap, but never automatic: a
                    // server restart exits pane processes, and only 0.8+
                    // restores sessions — the skew case is BY DEFINITION an
                    // old server, so the kill is real. The user decides.
                    if let action = active?.herdrNoticeAction {
                        Button {
                            active?.runHerdrNoticeAction()
                        } label: {
                            Text(action)
                                .font(Face.mono(11, .semibold))
                                .foregroundStyle(.black)
                                .padding(.horizontal, 12).padding(.vertical, 6)
                                .background(Ink.accent, in: Capsule())
                        }
                        .buttonStyle(.plain)
                    }
                }
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
        }
        }
        .animation(.easeOut(duration: 0.2), value: copiedURL)
        .animation(.easeOut(duration: 0.2), value: active?.tmuxControl?.notice)
        .onReceive(NotificationCenter.default.publisher(
            for: UIPasteboard.changedNotification)) { _ in pasteboardChanged() }
        .onAppear {
            clipboardHasImage = UIPasteboard.general.hasImages
            HardwareKeys.shared.install(owner: hardwareKeyOwner, hardwareKeyCommands)
        }
        // The change notification doesn't fire for copies made while we were
        // backgrounded — one metadata check per foreground return covers it.
        // Deliberately NOT read per-render: `hasImages` is a synchronous XPC
        // to pasteboardd, and doing it in the bar's render path while Claude
        // Code streams output starved the scrollback swipe of touch events
        // (the reported "can't scroll history" jank).
        .onChange(of: scenePhase) { _, phase in
            if phase == .active {
                clipboardHasImage = UIPasteboard.general.hasImages
                // Foreground return can land with stale keyboard-avoidance
                // geometry (an IME was up when the app left; iOS restores the
                // scene before the keyboard story settles) — reported on
                // device as the terminal missing its bottom chunk. Re-run
                // layout against the CURRENT bounds once things settle. Cheap
                // and idempotent when nothing is stale. No repaint of our own:
                // SwiftTerm redraws the whole view on its way back to the
                // foreground (its pixels may have been dropped) and when a
                // layout changes its size (fork patch 26).
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
                    active?.coordinator.hostContainer?.setNeedsLayout()
                }
            }
        }
        .onChange(of: anySheetOpen) { _, open in
            if !open { releaseGeometryHold() }
        }
        // Another pane or session takes the typing: an armed Ctrl doesn't
        // stay behind on the one left (B411).
        .onChange(of: typingTarget) { _, target in
            stickyCtrl.typingMoved(to: target)
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
        // The NSH terminal takes its colour scheme's appearance on its own
        // surface (R12, `Screen::Terminal`), not the window's: Home under the
        // back swipe keeps the app's. The SwiftUI terminal is dark only.
        .preferredColorScheme(usesNSH ? nil : .dark)
        .onGeometryChange(for: Bool.self) { proxy in
            UIDevice.current.userInterfaceIdiom == .pad
                && proxy.size.width > proxy.size.height
        } action: { compactChrome = $0 }
        .safeAreaInset(edge: .bottom, spacing: 0) { if !usesNSH { bottomAccessory } }
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
            HardwareKeys.shared.uninstall(owner: hardwareKeyOwner)
        }
        .onChange(of: appearanceKey) { applyAppearance() }
        .onChange(of: active?.viewModel.errorMessage) { _, message in
            showError = message?.isEmpty == false
        }
        // The user's own no to the server's key ends the connect quietly
        // (B151): back where they came from, no error card.
        .onChange(of: active?.viewModel.declinedHostKey) { _, declined in
            if declined == true { leave() }
        }
        // The physical version of the transport pill: a tap when the link
        // comes up, a warning buzz when a live session drops. Driven by the
        // state itself, so it fires however the change happened.
        .sensoryFeedback(trigger: connState) { old, new in
            Self.connectionFeedback(from: old, to: new, droppedAt: droppedAt, now: Date())
        }
        // A protocol switch is over once the new session's frame is
        // uncovered: its picture goes, motion comes back.
        .onChange(of: paneCover) { old, cover in
            // What covers the pane, said (B445): a page left in the
            // terminal's colour under its bar reads as a blank terminal.
            Log.ui.notice("terminal: the pane's cover \(String(describing: old), privacy: .public) -> \(String(describing: cover), privacy: .public) (\(String(describing: connState), privacy: .public), settling \(settling, privacy: .public))")
            if stillSwitch, cover == .none, connState == .live { endStillSwitch() }
        }
        .onChange(of: connState) { old, state in
            // When the veil came up for this drop: the tap on the way back
            // is for a drop the person waited through (B324).
            if old == .live, state != .live { droppedAt = Date() }
            // A reconnect used to collapse the keyboard at cycle start: tmux
            // mints a fresh controller per reconnect (see TmuxPaneSplitView's
            // `.id`), so first responder died with the old pane view and the
            // keyboard slid away mid-cycle, then popped back when the new
            // pane took focus. The retired controller now stays hosted until
            // the replacement has painted, so first responder has somewhere
            // to go and the keyboard never moves.
            if state != .live {
                lastCoverState = state
                coverArmed = true
            }
            // `.live` means the TRANSPORT is up, not that anything has
            // painted: on tmux the -CC re-attach and first backfill land a
            // few hundred ms later, and dropping the cover at transport-up
            // revealed a black pane for exactly that window (measured
            // frame-by-frame on the rig). tmux now reports its first painted
            // frame (`ActiveSession.awaitingFirstFrame`); for everything
            // else, hold the cover briefly past `.live`, then fade it out
            // over content instead of over void.
            if state == .live {
                wasEverLive = true
                settleGrace = true
                Task {
                    try? await Task.sleep(for: .milliseconds(500))
                    withAnimation(stillSwitch ? nil : .easeOut(duration: 0.28)) { settleGrace = false }
                }
            }
        }
    }

    @State private var autoCare = HostCare.shared

    /// Identity of the session this screen is showing — a new object (protocol
    /// switch, Install Assist's fresh session) starts from "never live" again.
    private var activeIdentity: ObjectIdentifier? {
        active.map { ObjectIdentifier($0) }
    }
    @State private var hardwareKeyOwner = UUID()
    /// The NSH page (`MOSHPIT_NSH_TERMINAL`, read when the screen is made: a
    /// screen keeps the UI it opened with), its model, and whether NSH
    /// failed to start (the SwiftUI chrome then shows).
    @State private var nshTerminal = NSHScreen.terminal.isOn
    @State private var nshFailed = false
    @State private var nshModel = TerminalNSHModel()

    var body: some View {
        contentWithLifecycle
        .terminalDirector(keyboard: { keyboardIntent = $0 ? .up : .down }, present: { what in
            switch what {
            case "windows": presentSheet { showWindowsSheet = true }
            case "sessions": presentSheet { showSessionsSheet = true }
            case "panes": presentSheet { showPaneSheet = true }
            case "switch": showProtocolSwitchConfirm = true
            case "diagnostics": showMoshDiagnostics = true
            case "install": installPackages = active?.herdrControl != nil ? ["herdr"] : ["tmux"]
            case "folder": active?.openWorkingDirectory()
            default:
                installPackages = nil
                if remoteFolder != nil { remoteFolder = nil }
                showWindowsSheet = false
                showSessionsSheet = false
                showPaneSheet = false
                showProtocolSwitchConfirm = false
                showMoshDiagnostics = false
            }
        })
        // The shell is keyboard-like chrome — 30pt chips, one-line crumbs, a
        // fixed-height bar — and, like the system keyboard, it does not grow
        // with the reading size. Text still scales one step; the terminal's
        // own font has its own setting.
        .dynamicTypeSize(...DynamicTypeSize.xLarge)
        // The ONE question host automation is allowed to ask, once: everything
        // else (pairing, script updates, registration) runs silently, but the
        // first hook install edits the user's agent config and that write needs
        // a yes. "Not Now" asks again next app run; "Don't Ask" never does —
        // the Host Setup sheet stays available for a change of mind.
        .alert(
            String(localized: "Enable agent notifications on \(connection.displayName)?"),
            isPresented: Binding(
                get: { autoCare.needsConsent?.id == connection.id.uuidString },
                set: { if !$0 { autoCare.dismissConsent() } })
        ) {
            Button(String(localized: "Enable")) {
                autoCare.grantConsent(connection)
            }
            Button(String(localized: "Not Now"), role: .cancel) { autoCare.dismissConsent() }
            Button(String(localized: "Don't Ask Again"), role: .destructive) {
                autoCare.decline(connection)
            }
        } message: {
            Text(String(localized: "Moshpit installs its hook scripts in ~/.moshpit and registers them in Claude Code's settings, so agents can reach you when they need you. Everything can be removed from Host Setup."))
        }
        .moshpitCard(isPresented: $showError) {
            if active?.viewModel.endedByServer == true {
                sessionEndedCard
            } else {
                MoshpitNoticeCard(
                    icon: "bolt.slash.fill",
                    title: "Connection Error",
                    message: active?.viewModel.errorMessage ?? ""
                ) {
                    showError = false
                    active?.viewModel.errorMessage = nil
                    leave()
                }
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
        .moshpitCard(
            item: Binding(
                get: { active?.viewModel.keyPassphrasePrompt },
                set: { if $0 == nil { /* dismissal handled via decide() */ } }
            )
        ) { prompt in
            KeyPassphrasePromptCard(prompt: prompt)
        }
        .appSheet(item: installTargetBinding) { target in
            if let active {
                if usesNSH || NSHScreen.installAssist.isOn {
                    InstallAssistNSHPage(session: active, packages: target.packages, onReconnect: { reconnect() })
                } else {
                    InstallAssistView(
                        session: active,
                        packages: target.packages,
                        onReconnect: { reconnect() })
                }
            }
        }
        .photosPicker(isPresented: $photoPickerPresented,
                      selection: $photoSelection,
                      maxSelectionCount: 10,
                      matching: .images)
        .appFullScreenCover(isPresented: $showCamera) {
            // Full screen, not a sheet — the camera IS a full-screen surface,
            // and UIImagePickerController inside a medium sheet misplaces its
            // controls.
            CameraCaptureView { data in
                showCamera = false
                guard let data else { return }
                guard let controller = makeImageAttachmentController() else { return }
                controller.start(pickedData: [data])
            }
            .ignoresSafeArea()
        }
        .onChange(of: photoSelection) { _, items in
            guard !items.isEmpty else { return }
            photoSelection = []
            beginImageAttachment(items)
        }
        .quickLookPreview($quickLookURL)
        .onChange(of: quickLookURL) { _, url in
            if url == nil { dismissRemotePreview() }
        }
        .appSheet(item: $remoteFolder, onDismiss: { dismissRemotePreview() }) { target in
            if usesNSH || NSHScreen.remoteFolder.isOn {
                RemoteFolderNSHSheet(folder: target.folder, files: target.files)
            } else {
                RemoteFolderSheet(folder: target.folder, files: target.files)
            }
        }
        .onChange(of: active?.remotePreview?.phase) { _, phase in
            switch phase {
            case .ready(let url)?:
                presentSheet { quickLookURL = url }
            case .folder(let entry)?:
                guard let files = active?.remotePreview?.files else { return }
                presentSheet { remoteFolder = RemoteFolderTarget(folder: entry, files: files) }
            case .stream(let entry, let source)?:
                guard let files = active?.remotePreview?.files else { return }
                presentSheet {
                    remoteVideoOpen = true
                    RemoteVideoPlayer.present(entry, source: source, files: files) {
                        remoteVideoOpen = false
                        dismissRemotePreview()
                    }
                }
            default:
                break
            }
        }
    }

    /// Identifiable wrapper so `.sheet(item:)` can carry a host folder.
    private struct RemoteFolderTarget: Identifiable {
        let folder: RemoteFileEntry
        let files: RemoteFileAccess
        var id: String { folder.path }
    }

    /// Put away whatever a tapped path link opened.
    private func dismissRemotePreview() {
        active?.remotePreview?.cancel()
        active?.remotePreview = nil
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
        // Nothing moves from here until the new session's first frame is
        // uncovered (``stillSwitch``); the pane's last picture stays.
        stillSwitch = true
        holdStill()
        // The protocol alone, onto the saved record: this screen's copy is
        // from when it opened, and writing it back undid an edit saved
        // since — the iPad's sidebar edits beside it (B429).
        if let switched = connectionHolder.store.switchProtocol(of: connection.id) {
            connection = switched
        }
        Haptics.select()
        reconnect(still: true)
    }

    /// The pane's picture as the switch begins, kept until it ends.
    private func holdStill() {
        if usesNSH {
            nshModel.stage.holdStill()
        } else {
            switchStill = TerminalStill(of: stillSurface)
        }
    }

    /// The terminal the SwiftUI chrome shows alone: a window of one pane's,
    /// or the session's single terminal. A window of several panes side by
    /// side has no one picture: none then.
    private var stillSurface: TerminalSurface? {
        guard let active else { return nil }
        if let controller = active.retiredTmuxController ?? active.tmuxController {
            let panes = controller.snapshot.activePanes
            guard panes.count == 1, let pane = panes.first?.id else { return nil }
            return controller.surface(for: pane)
        }
        return active.coordinator.surface
    }

    /// The switch is over: the new session's frame is uncovered. Its
    /// picture goes at once, and motion is back.
    private func endStillSwitch() {
        stillSwitch = false
        switchStill = nil
        nshModel.stage.releaseStill()
    }

    /// The session this screen shows changed (``activeIdentity``): a new
    /// object starts from "never live", unless this screen swapped it in
    /// over a reconnect's frame (``swapCarriesLive``).
    private func sessionSwapped() {
        wasEverLive = Self.liveAfterSwap(carried: swapCarriesLive)
        swapCarriesLive = false
        // The session resolving while still connecting is the first
        // connect's normal path (`connState` reads `.connecting` both
        // before and after, so the state change above never fires).
        coverArmed = (active?.viewModel.connState ?? .live) != .live
        droppedAt = coverArmed ? Date() : nil
    }

    /// Whether a session this screen swapped in counts as having been live:
    /// it carries the old one's, so it comes up under the veil.
    nonisolated static func liveAfterSwap(carried: Bool) -> Bool {
        carried
    }

    /// The card for a session the server ended — its shell exited, or
    /// mosh-server sent its shutdown. Not an error: it used to arrive as
    /// "Connection Error", whose only button left the screen. It says what
    /// happened, and offers the reconnect nothing automatic will make.
    private var sessionEndedCard: some View {
        let reason = active?.viewModel.errorMessage ?? ""
        return MoshpitModalCard(
            icon: "power",
            tone: .neutral,
            title: "Session ended",
            message: reason.isEmpty ? nil : Text(reason),
            buttons: [
                ModalButton("Done", kind: .secondary) {
                    showError = false
                    active?.viewModel.errorMessage = nil
                    leave()
                },
                ModalButton("Reconnect", kind: .primary) {
                    showError = false
                    active?.viewModel.errorMessage = nil
                    reconnect()
                },
            ]
        ) { EmptyView() }
    }

    /// Tear down and re-run the start flow at full feature — used after the
    /// user installs the missing dependency, and to reconnect a session the
    /// server ended. The probe re-runs during start, so a now-present tmux /
    /// mosh-server is picked up automatically.
    private func reconnect(still: Bool = false) {
        guard let active else { return }
        // A switch moves nothing; any other manual reconnect moves as a
        // reconnect does. Either way the new session is a reconnect when
        // this one was (or always, for a switch): the veil, not the poster.
        if !still, stillSwitch { endStillSwitch() }
        swapCarriesLive = still || isReconnect
        // Manual reconnects (protocol switch, Install Assist) swap in a
        // brand-new session whose connState opens .connecting, so the
        // .reconnecting onChange never fires for them — collapse the keyboard
        // here for the same reason it collapses there.
        keyboardIntent = .down
        Task {
            // A delete under way starts nothing (B432). The saved record,
            // not this screen's copy from when it opened (B429); a debug
            // seed's connection is saved nowhere, and dials as given.
            guard !ConnectionDeletion.isUnderWay(connection.id) else { return }
            connection = connectionHolder.store.saved(connection.id) ?? connection
            await hub.disconnect(connection.id)
            let session = hub.prepare(connection)
            self.active = session
            await hub.start(session, theme: theme, fontSize: settings.fontSize, fontName: settings.fontName,
                        cursorShape: settings.cursorShape, cursorColorId: effectiveCursorColorId,
                        cursorBlink: settings.cursorBlink)
            applyAppearance()
            // When-ready, not right-now: over mosh the control plane is wired
            // by a detached task after start() returns, so a one-shot read
            // here left the island blind on that transport.
            monitor.trackWhenReady(connection: connection, session: session)
            // The buttons the Host Setup sheet used to require, run silently:
            // refresh drifted scripts, mint/repair this device's pairing. Asks
            // (once) before ever touching a host that has no hooks.
            HostCare.shared.tend(connection)
            _ = active
        }
    }

    // MARK: - Leaving, and the hardware keyboard

    private func leave() {
        if let onLeave { onLeave() } else { dismiss() }
    }

    /// What the scene's `TerminalCommands` chords do while this screen is up
    /// (registered on appear; see `HardwareKeys` for why they are not hidden
    /// `.keyboardShortcut` buttons here). SwiftTerm hands ⌘-chords it has no
    /// use for up the chain, so they also work mid-typing. ⌘W leaves the way
    /// the back button does: the session stays alive.
    private var hardwareKeyCommands: [HardwareKeys.Command] {
        // The NSH page decides what a chord does (its keymap gets the
        // presses the menu leaves; the menu's own come here and go there).
        if usesNSH {
            let model = nshModel
            return ["k", "w"].map(Character.init) .map { key in
                HardwareKeys.Command(title: key == "k" ? "Switch Window" : "Back to Home", key: key) {
                    model.post(["op": "key", "stroke": "mod+\(key)"])
                }
            } + (1..<10).map { number in
                HardwareKeys.Command(title: "Window \(number)", key: Character(String(number))) {
                    model.post(["op": "key", "stroke": "mod+\(number)"])
                }
            }
        }
        var list: [HardwareKeys.Command] = [
            .init(title: "Switch Window", key: "k") { presentSheet { showWindowsSheet = true } },
            .init(title: "Back to Home", key: "w") { leave() },
        ]
        for number in 1..<10 {
            list.append(.init(title: "Window \(number)", key: Character(String(number))) {
                selectWindow(number: number)
            })
        }
        return list
    }

    /// ⌘1–⌘9: the n-th window in the picker's order, so the number the
    /// shortcut means is the one the user can read off the Windows sheet.
    private func selectWindow(number: Int) {
        guard let active else { return }
        let control: any MultiplexerControlling
        if let c = active.tmuxControl { control = c } else if let h = active.herdrControl { control = h } else { return }
        let windows = control.snapshot.sortedWindows
        guard windows.indices.contains(number - 1) else { return }
        control.selectWindow(windows[number - 1].id)
    }

    // MARK: - Top bar

    private var topBar: some View {
        HStack(spacing: 8) {
            Button {
                if let onToggleSidebar { onToggleSidebar() } else { leave() }
            } label: {
                Image(systemName: onToggleSidebar != nil ? "sidebar.leading" : "chevron.left")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(Ink.accent)
                    .frame(width: 32, height: 32)
                    .glassChrome(in: Circle())
                    // 44pt hit target around a 32pt glyph disc, like a bar button.
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .hoverEffect(.highlight)
            .accessibilityLabel(onToggleSidebar != nil ? "Toggle Sidebar" : "Back")
            .accessibilityIdentifier(onToggleSidebar != nil ? "terminal-sidebar" : "terminal-back")

            Button {
                showProtocolSwitchConfirm = true
            } label: {
                TransportPill(kind: transportKind, connState: connState, still: stillSwitch)
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
            .appPopover(isPresented: $showMoshDiagnostics) {
                MoshDiagnosticsView(diagnostics: metrics?.moshDiagnostics) {
                    // Repaint = ack state 0 → the server rediffs from blank,
                    // a complete redraw. Doubles as the field diagnostic for
                    // the white-block reports: blocks that VANISH on repaint
                    // are render divergence (SwiftTerm vs mosh-server state),
                    // not lost content.
                    active?.requestRepaint()
                    showMoshDiagnostics = false
                }
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
                .frame(minHeight: 32)
                .glassChrome(in: Capsule(style: .continuous))
                // The pickers hang off the breadcrumb they name: a sheet on the
                // phone, a popover anchored to this capsule on an iPad.
                .adaptivePresentation(isPresented: $showWindowsSheet) {
                    // Generic over the concrete controller (not an existential) so
                    // Observation tracking on `snapshot` survives — hence the branch.
                    if let c = active?.tmuxControl { MuxPicker(controller: c, kind: .windows, nsh: nshPickers) } else if let h = active?.herdrControl { MuxPicker(controller: h, kind: .windows, nsh: nshPickers) }
                }
                .adaptivePresentation(isPresented: $showSessionsSheet) {
                    if let c = active?.tmuxControl { MuxPicker(controller: c, kind: .sessions, nsh: nshPickers) } else if let h = active?.herdrControl { MuxPicker(controller: h, kind: .sessions, nsh: nshPickers) }
                }
                .adaptivePresentation(isPresented: $showPaneSheet) {
                    if let c = active?.tmuxControl { MuxPicker(controller: c, kind: .panes, nsh: nshPickers) } else if let h = active?.herdrControl { MuxPicker(controller: h, kind: .panes, nsh: nshPickers) }
                }

            Spacer(minLength: 0)
            // The pane-grid button used to live here; it was redundant with the
            // breadcrumb's pane crumb (both open the Select Pane sheet), so it's
            // gone — the rightmost icon the breadcrumb wrapping issue referenced.
        }
        .padding(.horizontal, 16)
        // A system navigation bar is 44pt; the shell's bar does not get to be
        // taller than the one it replaces. Same material as a system bar.
        .frame(height: compactChrome ? 40 : 44)
        .background(.bar)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Ink.hairline).frame(height: 1)
        }
    }

    /// A breadcrumb segment: a small SF Symbol + mono label, in Signal Room citron.
    ///
    /// `text: nil` renders icon-only — the squeezed form the session crumb
    /// falls back to when the pane crumb is carrying an agent name and three
    /// full segments do not fit the bar (see `BreadcrumbPlan` and `crumbs`).
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

    /// The last breadcrumb we could build, kept so a reconnect does not throw
    /// the navigation away.
    ///
    /// `BreadcrumbPlan.make` needs an ATTACHED tree, and an automatic
    /// reconnect rebuilds the controller from scratch — so for the seconds it
    /// takes, the bar fell back to `user@host · tmux` and then snapped back to
    /// session/window/pane. Nothing about the user's session actually changed:
    /// the windows are still on the server, we are just not attached to them
    /// this instant. Showing the identity of the machine instead reads as
    /// "your session is gone" at the exact moment the user is worried it
    /// might be.
    @State private var lastBreadcrumb: BreadcrumbPlan?

    /// The plan for the live tree, once it is whole. An attach marks the
    /// tree live before `list-windows` / `list-panes` and the hook poll have
    /// answered; a plan built from that half-filled tree ("0 › —", then the
    /// command without its agent) is what jumped in the bar on every
    /// reconnect (真机 2026-09-16). Until it is complete, `retainedPlan`
    /// keeps the remembered one up.
    private var livePlan: BreadcrumbPlan? {
        guard let snapshot = tmuxSnapshot,
              let plan = BreadcrumbPlan.make(snapshot: snapshot, hooks: multiplexerAgentHooks,
                                             hooksLoaded: multiplexerAgentHooksLoaded,
                                             mux: connection.multiplexer == .herdr ? "herdr" : "tmux"),
              plan.isComplete else { return nil }
        return plan
    }

    /// What to draw while the tree is gone: the last one we had, but only
    /// while the connection is coming back. Once it is genuinely offline the
    /// machine's identity is the honest thing to show again.
    private var retainedPlan: BreadcrumbPlan? {
        switch connState {
        case .connecting, .reconnecting: return lastBreadcrumb
        // Transport back, tree not yet attached and painted — or attached and
        // still filling in: still the same in-between, and the bar flipping
        // to the host name for those few hundred ms was the one piece of
        // chrome that moved during a frozen-frame reconnect.
        case .live: return (settling || tmuxSnapshot?.isAttached == true) ? lastBreadcrumb : nil
        case .offline: return nil
        }
    }

    /// The three segments as one row. `sessionText: nil` is the squeezed
    /// form — session icon only — that `breadcrumb` falls back to when the
    /// full row does not fit.
    private func crumbs(_ plan: BreadcrumbPlan, sessionText: String?) -> some View {
        HStack(spacing: 6) {
            Button {
                presentSheet { showSessionsSheet = true }
            } label: {
                crumbLabel(connection.multiplexer.vocabulary.sessionIcon, sessionText)
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
        }
    }

    private var breadcrumb: some View {
        HStack(spacing: 6) {
            // Read the snapshot through concrete types (not the existential) so
            // SwiftUI Observation tracks updates from either controller/sidecar.
            // The strings and the squeeze rule live in `BreadcrumbPlan` — pure,
            // tested — this just lays the segments out.
            if let plan = livePlan ?? retainedPlan {
                if plan.sessionSqueezable {
                    // The session name first; the icon alone only if the
                    // three full segments overflow the bar. `ViewThatFits`
                    // measures each row's ideal (untruncated) width against
                    // what the bar offers, so a one-character session like
                    // "0" keeps its name beside "11:moshpit › claude" (真机
                    // 2026-09-18) while a long one still yields.
                    ViewThatFits(in: .horizontal) {
                        crumbs(plan, sessionText: plan.sessionTitle)
                        crumbs(plan, sessionText: nil)
                    }
                } else {
                    crumbs(plan, sessionText: plan.sessionTitle)
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
        // Dimmed and inert while it is the remembered tree rather than the
        // live one: it still tells you where you are, without pretending you
        // can navigate a controller that is being rebuilt.
        .opacity(livePlan == nil && retainedPlan != nil ? 0.45 : 1)
        .disabled(livePlan == nil && retainedPlan != nil)
        .animation(stillSwitch ? nil : .easeOut(duration: 0.2), value: livePlan == nil)
        .onChange(of: livePlan) { _, plan in
            if let plan { lastBreadcrumb = plan }
        }
    }

    // MARK: - Terminal body

    /// The terminal area: live content underneath, and ONE hoisted connecting
    /// cover above every branch of it.
    ///
    /// The cover used to live inside the plain-SSH branch only, keyed on
    /// `status == .connecting || isAutoReconnectInFlight`. That left two black
    /// frames on the tmux path's reconnect: `markReconnecting()` sets status
    /// `.reconnecting` (not `.connecting`), and the flag only flips once
    /// `start(automatic:)` runs — AFTER `stop()` has already torn the pane
    /// view out of the tree. In that window the branch below swapped from the
    /// pane to an empty terminal with no cover up yet: the reported one-frame
    /// black flash between the keyboard collapse and the connecting
    /// animation. Hoisted and keyed on `connState` (which folds status and
    /// flag into one value that's already `.reconnecting` BEFORE teardown),
    /// the cover fades in over the still-live pane and the branch swap
    /// happens underneath it, invisibly.
    ///
    /// Kept as an OVERLAY, not a replacement, so SwiftTerm stays attached and
    /// never misses buffered output. Only a FIRST connect gets the poster —
    /// there is nothing behind it but the void. A reconnect NEVER does
    /// (2026-09-18, user ask: the full-screen waiting animation kept
    /// surfacing now and then): it keeps whatever is there — the last frame
    /// (the retired tmux controller, or the persistent single-pane
    /// terminal), or the bare terminal background when the dropped
    /// connection had never painted — under ``ReconnectVeil`` (dim + status
    /// capsule) for the WHOLE cycle, including waits between failed
    /// attempts; both covers hold until the replacement has painted, not
    /// until the transport says so. Mosh roaming is naturally excluded — a
    /// roam never sets `.connecting`/`.reconnecting` (SSP heals itself), so
    /// mosh keeps its useful last frame + predictive echo. `.offline` shows
    /// no poster (nothing is being attempted), only the veil over the dead
    /// frame the person was reading.
    private var terminalBody: some View {
        ZStack {
            terminalContent
            if let switchStill {
                // A protocol switch's last picture, until the new frame.
                TerminalStillView(still: switchStill)
                    .allowsHitTesting(false)
            }
            if posterVisible {
                TerminalConnectingView(connection: connection, state: posterState)
                    // Fades both ways: beneath it is the terminal background
                    // it shares a colour with.
                    .transition(.opacity)
                    // While the cover only waits for paint the session is LIVE
                    // — a swipe in that window belongs to the terminal, not
                    // to a poster. While genuinely connecting, the cover keeps
                    // its touches (there is nothing usable beneath).
                    .allowsHitTesting(connState != .live)
            }
            if veilVisible {
                ReconnectVeil(state: posterState,
                              isMosh: connection.connectionProtocol == .mosh,
                              ended: active?.viewModel.endedByServer == true)
                    .allowsHitTesting(false)
                    .transition(.opacity)
            }
            if let follow = sizeFollow {
                SizeFollowVeil(ownerLabel: follow.ownerLabel) {
                    active?.tmuxController?.takeSizeOwnership()
                }
                .transition(.opacity)
            }
        }
        .animation(Self.coverAnimation(still: stillSwitch), value: transitionKey)
        .onChange(of: activeIdentity) { _, _ in sessionSwapped() }
    }

    /// What the cover says. Once the transport is back the state reads
    /// `.live`, but the cover is still up for the first frame — it keeps
    /// announcing the phase it was covering, not "Opening the pit".
    private var posterState: TransportConnState {
        connState == .live ? lastCoverState : connState
    }

    /// The transport is up but there is nothing to uncover yet: tmux and
    /// herdr until their first frame is painted, everything else for the
    /// beat `settleGrace` holds. Never both — on a fast attach the first
    /// frame lands inside the grace period, and letting the grace outlive
    /// the signal put the poster up for the leftover ~170ms right as the
    /// frozen frame gave way to the new one (a poster flash in the
    /// crossfade, measured on the rig's short-suspension run).
    private var settling: Bool {
        guard let active, connState == .live, coverArmed else { return false }
        let waits = Self.waitsForFirstFrame(transport: active.connection.connectionProtocol,
                                            multiplexer: active.connection.multiplexer,
                                            degraded: active.degrade != nil,
                                            onRustCore: active.rust != nil)
        return waits ? active.awaitingFirstFrame : settleGrace
    }

    /// The veil must have been up this long for the way back to tap: a
    /// redial the person barely saw got a warning buzz and a success tap
    /// within a second of each other (B324).
    nonisolated static let feedbackAfterDrop: TimeInterval = 1

    /// The haptic for a change of the connection's state: a tap when a
    /// first connect comes up, a warning buzz when a live session drops, a
    /// tap when it is back — only after a drop the person waited through
    /// (`feedbackAfterDrop`); back sooner, the buzz was enough. A drop with
    /// no start on record counts as not seen: no tap.
    nonisolated static func connectionFeedback(from old: TransportConnState,
                                               to new: TransportConnState,
                                               droppedAt: Date?,
                                               now: Date) -> SensoryFeedback? {
        switch (old, new) {
        case (.connecting, .live): return .success
        case (.reconnecting, .live):
            guard let droppedAt else { return nil }
            return now.timeIntervalSince(droppedAt) >= feedbackAfterDrop ? .success : nil
        case (.live, .reconnecting), (.live, .offline): return .warning
        default: return nil
        }
    }

    /// Whether the cover waits for the session's first painted frame
    /// (`ActiveSession.awaitingFirstFrame`) rather than the `settleGrace`
    /// beat. tmux's pane hosts report it on either path. herdr over SSH
    /// reports it on the Rust core, where the driver clears the wait on
    /// herdr's first frame (B323): on the grace, the veil lifted 500 ms
    /// after transport-up — over a stale frame whose input herdr was still
    /// dropping, 1.3 s of it at 510 ms RTT. Plain SSH, mosh and a connect
    /// degraded to a plain shell paint into the persistent terminal and
    /// take the grace.
    nonisolated static func waitsForFirstFrame(transport: ConnectionProtocol,
                                               multiplexer: Multiplexer,
                                               degraded: Bool,
                                               onRustCore: Bool) -> Bool {
        guard transport == .ssh, !degraded else { return false }
        switch multiplexer {
        case .tmux: return true
        case .herdr: return onRustCore
        case .none: return false
        }
    }

    /// This is a connection coming BACK, not a first one: the screen has been
    /// live, or it was entered mid-reconnect over a frame kept from before.
    /// The kept frame is enough on its own — a ghost only exists because the
    /// dropped connection had painted, whichever screen was watching.
    private var isReconnect: Bool {
        wasEverLive || active?.retiredTmuxController != nil
    }

    /// Only a FIRST connect gets the poster — a reconnect keeps what is there
    /// and takes the veil instead, even when what is there is only the bare
    /// terminal background (the dropped connection never painted, so no
    /// frame was kept; this path used to show the poster, and that is the
    /// full-screen animation that kept surfacing). It holds until the
    /// replacement has painted, not until the transport says so. `.offline`
    /// shows no poster: nothing is being attempted.
    private var posterVisible: Bool { paneCover == .poster }

    /// The reconnect dim and status capsule — over the kept frame, the
    /// persistent pane, or the bare background; and also over a dropped line
    /// that is no longer being redialled (the frame is still what the person
    /// was reading).
    private var veilVisible: Bool { paneCover == .veil }

    /// What covers the pane now (``paneCover(resolved:reconnect:state:settling:)``).
    private var paneCover: PaneCover {
        Self.paneCover(resolved: active != nil, reconnect: isReconnect, state: connState, settling: settling)
    }

    /// What covers the pane: nothing, the first connect's poster, or a
    /// reconnect's veil.
    enum PaneCover: Equatable {
        case none, poster, veil
    }

    /// The cover for a session `resolved` (or not yet: no session object,
    /// no cover — `active` is a `@State` set on appear, so every screen
    /// renders one frame without it; for a first connect the poster follows
    /// a frame later over the same colour, for a screen entering a live
    /// session it must never come, see `coverArmed`), coming back or not
    /// (`reconnect`), its transport in `state`, `settling` while the
    /// transport is up but nothing has painted. A reconnect — a protocol
    /// switch always is one — gets the veil, never the poster, through the
    /// whole cycle and over a line no longer redialled (`.offline`); a first
    /// connect gets the poster while something is attempted.
    nonisolated static func paneCover(resolved: Bool, reconnect: Bool,
                                      state: TransportConnState, settling: Bool) -> PaneCover {
        guard resolved else { return .none }
        if reconnect {
            switch state {
            case .connecting, .reconnecting, .offline: return .veil
            case .live: return settling ? .veil : .none
            }
        }
        switch state {
        case .connecting, .reconnecting: return .poster
        case .live: return settling ? .poster : .none
        case .offline: return .none
        }
    }

    /// How the cover stack moves: not at all through a protocol switch
    /// (``stillSwitch``), else its 0.3 s ease.
    nonisolated static func coverAnimation(still: Bool) -> Animation? {
        still ? nil : .easeInOut(duration: 0.3)
    }

    /// Another Moshpit device holds the live window's size: the pane is
    /// frozen on its last frame under a veil that names the holder and
    /// offers a tap to take over. Only while nothing else covers the pane —
    /// a reconnect's own veil says what matters then.
    private var sizeFollow: TmuxSessionController.SizeFollow? {
        guard connState == .live, !posterVisible, !veilVisible,
              let controller = active?.tmuxController, active?.retiredTmuxController == nil
        else { return nil }
        return controller.sizeFollow
    }

    /// Everything the cover stack animates on, in one value.
    private struct TransitionKey: Equatable {
        var ghost: Bool
        var poster: Bool
        var veil: Bool
        var follow: Bool
        var state: TransportConnState
    }

    private var transitionKey: TransitionKey {
        TransitionKey(ghost: active?.retiredTmuxController != nil,
                      poster: posterVisible, veil: veilVisible,
                      follow: sizeFollow != nil, state: connState)
    }

    @ViewBuilder
    private var terminalContent: some View {
        if let active {
            // The controller whose panes are on screen: after a drop, the
            // retired one until the replacement has painted (the hub clears
            // it on that signal), otherwise the live one. ONE branch for
            // both on purpose — the retired controller and its pane id are
            // what the hosted view was keyed on before the drop, so with the
            // branch unchanged it is the very same view: nothing is
            // re-parented or re-laid out, the frame stays where it was.
            // (A separate ghost layer moved the TerminalView between two
            // hosts, and it visibly slid into place — measured.) The swap to
            // the new controller is then a keyed identity change inside
            // TmuxPaneSplitView, crossfaded by the animation on the ZStack.
            let ghost = active.retiredTmuxController
            if let controller = ghost ?? active.tmuxController {
                if ghost != nil || controller.snapshot.isAttached {
                    TmuxPaneSplitView(controller: controller,
                                      focusPolicy: focusPolicy,
                                      geometryHeld: geometryHold,
                                      compactChrome: compactChrome,
                                      frozen: ghost != nil,
                                      onPaste: { [weak active] text in active?.sendPaste(text) })
                        // A frozen frame has nobody to send touches to.
                        .allowsHitTesting(ghost == nil)
                } else if isReconnect && (connState != .live || active.awaitingFirstFrame) {
                    // A reconnect with no frame to keep — the dropped
                    // connection never painted, so there is no retired
                    // controller — shows the terminal background under the
                    // veil's capsule and nothing else. The empty state's own
                    // full-screen connecting animation is for a first
                    // connect; here it was the animation that still surfaced
                    // now and then (真机 2026-09-18).
                    Rectangle().fill(Ink.terminalBG)
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
                        quietWait: isReconnect,
                        failure: controller.notice,
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
                    quietWait: isReconnect,
                    onCreate: { herdr.newSession(named: nil) },
                    onInstall: { installPackages = ["herdr"] })
            } else {
                SwiftTerminalView(
                    theme: theme,
                    fontSize: settings.fontSize,
                    fontName: settings.fontName,
                    cursorShape: settings.cursorShape,
                    cursorColorId: effectiveCursorColorId,
                    cursorBlink: settings.cursorBlink,
                    coordinator: active.coordinator,
                    focusPolicy: focusPolicy,
                    geometryHeld: geometryHold)
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
                    // mosh + tmux on a server with no session (B228): the
                    // empty state over the renderer's shell — which stays
                    // mounted, waiting, and attaches the session made here —
                    // instead of a bare shell with nothing to say. Settled at
                    // once: the hub asked the server before this shows.
                    .overlay {
                        if active.awaitingFirstSession {
                            MultiplexerEmptyStateView(
                                multiplexer: .tmux,
                                binaryMissing: false,
                                connection: connection,
                                connectionState: connState,
                                startsSettled: true,
                                failure: active.tmuxControl?.notice,
                                onCreate: { active.createFirstTmuxSession() },
                                onInstall: {})
                        }
                    }
            }
        } else {
            // No session yet (the screen exists because one is being opened) —
            // the hoisted cover in `terminalBody` is showing; keep the ground
            // beneath it the terminal's own colour.
            Ink.terminalBG
        }
    }

    // MARK: - Shortcut dispatch

    /// The coordinator receiving keyboard input right now (tmux active pane,
    /// or the single mosh/SSH coordinator).
    private var typingCoordinator: SwiftTerminalView.Coordinator? {
        active?.tmuxController?.activeCoordinator ?? active?.coordinator
    }

    /// The coordinator typing goes to, by identity (what the armed Ctrl
    /// follows).
    private var typingTarget: ObjectIdentifier? {
        typingCoordinator.map(ObjectIdentifier.init)
    }

    /// Arm/disarm sticky Ctrl on whatever the user is typing into. One-shot:
    /// the coordinator clears itself after folding the next key. Off is off
    /// on the coordinator it was armed on, wherever typing went since.
    private func toggleStickyCtrl() {
        if ctrlArmed {
            stickyCtrl.disarm()
            return
        }
        guard let coordinator = typingCoordinator else { return }
        Haptics.tap()
        stickyCtrl.onChange = { armed in ctrlArmed = armed }
        stickyCtrl.arm(coordinator)
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
        guard ctrlArmed else { return }
        stickyCtrl.disarm()
    }

    /// The pasteboard changed: whether it holds an image, and a link in it
    /// to offer. A method of its own: inline in `body`'s modifier chain,
    /// Xcode 26's type checker gave up on it ("unable to type-check this
    /// expression in reasonable time").
    private func pasteboardChanged() {
        clipboardHasImage = UIPasteboard.general.hasImages
        Task { () async -> Void in await detectCopiedURL() }
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
        // A delete under way starts nothing (B432). The saved record, not
        // this screen's copy from when it opened (B429); a debug seed's
        // connection is saved nowhere, and dials as given.
        guard !ConnectionDeletion.isUnderWay(connection.id) else { return }
        connection = connectionHolder.store.saved(connection.id) ?? connection
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
        // When-ready, not right-now — see the connect-path call for why.
        monitor.trackWhenReady(connection: connection, session: session)
            // The buttons the Host Setup sheet used to require, run silently:
            // refresh drifted scripts, mint/repair this device's pairing. Asks
            // (once) before ever touching a host that has no hooks.
            HostCare.shared.tend(connection)
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
        if shortcut.kind == .image {
            photoPickerPresented = true
            return
        }
        if shortcut.kind == .files {
            active?.openWorkingDirectory()
            return
        }
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
                    // No text — an image-only clipboard routes into the attach
                    // flow instead (copy a screenshot, tap paste: the Ctrl+V
                    // mental model, arriving as an uploaded path). Text keeps
                    // priority when both are present: pasting text is this
                    // chip's core job.
                    let images = await Task.detached { ClipboardImageReader.read() }.value
                    if !images.isEmpty, let controller = makeImageAttachmentController() {
                        controller.start(pickedData: images)
                        return
                    }
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
        // Over herdr these bytes are intercepted in sendInput and rerouted as
        // SEMANTIC keys (`pane send-keys`) — the local DECCKM below is stale
        // there (frames never carry mode changes), and sendInput is the one
        // choke point both this D-pad and a hardware keyboard flow through.
        // SSH+tmux does the same one level down: the controller sends a lone
        // arrow as a tmux key name (`TmuxSessionController.sendInput`).
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
            if let imageAttachment {
                ImageAttachmentOverlayView(
                    controller: imageAttachment,
                    onCancel: { cancelImageAttachment() },
                    onRetry: { imageAttachment.retry() },
                    onInsert: { finishImageAttachment() },
                    onSend: { finishImageAttachment(submit: true) })
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
            if let preview = active?.remotePreview, preview.showsHUD {
                RemotePreviewHUD(model: preview, onDismiss: { dismissRemotePreview() })
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
            ShortcutBarView(
                shortcuts: shortcutStore.toolbar(
                    forHost: connection.displayName,
                    inMultiplexer: active?.tmuxControl != nil || active?.herdrControl != nil),
                onTap: send(shortcut:),
                compact: compactChrome,
                ctrlArmed: ctrlArmed,
                onArrow: sendArrow,
                onScroll: scrollHistory,
                keyboardDown: keyboardIntent != .up,
                onToggleKeyboard: { keyboardIntent = keyboardIntent == .up ? .down : .up },
                micActive: dictation != nil,
                onMic: settings.voiceInputEnabled ? { toggleDictation() } : nil,
                imageHistory: active?.imageUploads.entries ?? [],
                clipboardHasImage: clipboardHasImage,
                onImagePasteboard: { beginClipboardImageAttachment() },
                onImageCamera: CameraCaptureView.isAvailable ? { presentSheet { showCamera = true } } : nil,
                onImageHistory: { insertUploadedImage($0) })
        }
        .animation(.easeOut(duration: 0.2), value: dictation == nil)
        .animation(.easeOut(duration: 0.2), value: imageAttachment == nil)
        .animation(.easeOut(duration: 0.2), value: active?.remotePreview?.showsHUD ?? false)
    }

    // MARK: - Image attachment

    /// Load the picked items' bytes and start the process → upload flow.
    /// `loadTransferable(type: Data.self)` hands over the original file bytes
    /// (HEIC included) without a photo-library permission prompt — PHPicker
    /// runs out of process and only the selection crosses back.
    private func beginImageAttachment(_ items: [PhotosPickerItem]) {
        guard let controller = makeImageAttachmentController() else { return }
        Task {
            var picked: [Data] = []
            for item in items {
                if let data = try? await item.loadTransferable(type: Data.self) {
                    picked.append(data)
                }
            }
            guard !picked.isEmpty else {
                imageAttachment = nil
                UINotificationFeedbackGenerator().notificationOccurred(.warning)
                return
            }
            controller.start(pickedData: picked)
        }
    }

    /// Clipboard route into the same flow — the Ctrl+V mental model from
    /// Claude Code's local image paste, delivered as an uploaded path (its
    /// native [Image #N] cannot exist over SSH; the placeholder only triggers
    /// on a clipboard the Claude Code PROCESS can read, i.e. the server's).
    private func beginClipboardImageAttachment() {
        disarmStickyCtrlIfNeeded()
        guard let controller = makeImageAttachmentController() else { return }
        Task {
            let images = await Task.detached { ClipboardImageReader.read() }.value
            guard !images.isEmpty else {
                imageAttachment = nil
                UINotificationFeedbackGenerator().notificationOccurred(.warning)
                return
            }
            controller.start(pickedData: images)
        }
    }

    /// Shared start: controller + the session-log hookup that assigns #N
    /// numbers the moment every upload lands. The transport resolves lazily
    /// inside the flow — pure mosh dials an SSH connection on demand (no
    /// second Face ID; credentials are cached), and a dial failure surfaces
    /// in the overlay's failed state with a Retry, not a dead chip.
    private func makeImageAttachmentController() -> ImageAttachmentController? {
        guard let active else { return nil }
        let controller = ImageAttachmentController(uploaderProvider: { [weak active] in
            guard let active else { throw SSHError.sessionClosed }
            return try await active.acquireFileTransferSSH()
        })
        controller.onReady = { [weak active] paths in
            guard let active else { return [] }
            return paths.map { active.imageUploads.record(remotePath: $0).number }
        }
        imageAttachment = controller
        return controller
    }

    /// Re-insert a previously uploaded image's path (from the chip's
    /// long-press menu) — no re-upload, the file is already on the server.
    private func insertUploadedImage(_ entry: ImageUploadLog.Entry) {
        disarmStickyCtrlIfNeeded()
        active?.sendPaste(ImageAttachmentPipeline.insertText(
            forRemotePaths: [entry.remotePath], style: activePaneInsertStyle))
        Haptics.tap()
    }

    /// The insert format the ACTIVE pane's agent understands best — the
    /// hook-stamped agent name when Vibe Island hooks are installed, else
    /// the pane's foreground command. Plain SSH (no pane data) reads nil →
    /// the universal bare-path default.
    private var activePaneInsertStyle: ImageInsertStyle {
        guard let control = active?.tmuxControl,
              let paneId = control.snapshot.activePaneId
                ?? control.snapshot.activePanes.first?.id else {
            return .barePath
        }
        let agent = control.agentHooks[paneId]?.agent
            ?? control.snapshot.panes[paneId]?.command
        return .forAgent(agent)
    }

    /// Paste the uploaded paths at the prompt. `sendPaste` (not raw bytes) so
    /// bracketed-paste apps receive them as one block; `submit` appends the
    /// Return AFTER the paste for the same reason dictation does — a CR inside
    /// the ESC[200~…201~ wrapper is a literal newline, not a submit.
    private func finishImageAttachment(submit: Bool = false) {
        guard let controller = imageAttachment,
              let text = controller.insertText(style: activePaneInsertStyle) else { return }
        active?.sendPaste(text)
        if submit {
            active?.sendInput(Data([0x0D]))
        }
        Haptics.success()
        imageAttachment = nil
    }

    private func cancelImageAttachment() {
        imageAttachment?.cancel()
        imageAttachment = nil
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
            // Only this session's panel: Cancel while the engine finished and
            // the mic key straight after start a new session, which this
            // late return must not take off the screen with its mic still on.
            if self.dictation === dictation { self.dictation = nil }
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
/// swaps which persistent pane terminal is shown, keyed by `activePaneId`.
struct TmuxPaneSplitView: View {
    let controller: TmuxSessionController
    /// How the pane should treat first responder — `.resign` while a
    /// navigation sheet is up, so it can't grab the keyboard back over it.
    var focusPolicy: TerminalFocusPolicy = .take
    /// Hold the shown pane at its current size across a modal — see
    /// ``TerminalHostContainer/geometryHeld``.
    var geometryHeld: Bool = false
    /// iPad-landscape chrome trim — see `TerminalScreen.compactChrome`.
    var compactChrome: Bool = false
    /// The controller is a reconnect's retired one, shown for its last frame
    /// only — see `TerminalScreen.terminalContent`. Its pane leaves by fading,
    /// never by sliding: the replacement is not "the next pane over".
    var frozen: Bool = false
    /// Where the pane's own Paste and ⌘V go — see ``PaneTerminalHost/onPaste``.
    var onPaste: ((String) -> Void)?

    var body: some View {
        let snapshot = controller.snapshot
        Group {
            if let paneId = snapshot.activePaneId ?? snapshot.activePanes.first?.id {
                PaneTerminalHost(surface: controller.surface(for: paneId),
                                 coordinator: controller.coordinator(for: paneId),
                                 focusPolicy: focusPolicy,
                                 geometryHeld: geometryHeld,
                                 onPaste: onPaste)
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
                    //
                    // Only once a swipe has happened on this controller, and
                    // never for a frozen frame: the pane a reconnect's
                    // replacement puts up is the SAME pane on a new
                    // connection, and sliding it in from an edge read as the
                    // whole screen lurching sideways mid-crossfade (measured).
                    .transition(snapshot.hasSwipedSinceAttach && !frozen
                        ? .asymmetric(
                            insertion: .move(edge: snapshot.lastSwitchForward ? .trailing : .leading),
                            removal: .move(edge: snapshot.lastSwitchForward ? .leading : .trailing)
                        ).combined(with: .opacity)
                        : .opacity)
            } else {
                Color.clear
            }
        }
        .padding(compactChrome ? 2 : 4)
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

/// Hosts a persistent pane terminal minted by the controller.
/// The active pane grabs keyboard focus so the system keyboard (with our
/// shortcut bar riding above it) is up as soon as the terminal is usable.
/// The terminal sits inside a ``TerminalHostContainer`` so keyboard
/// transitions can freeze its frame (see that type's doc — this is the
/// keyboard/IME-switch garble fix), same as the single-pane path.
struct PaneTerminalHost: UIViewRepresentable {
    let surface: TerminalSurface
    /// The pane's coordinator — hooked to the container so the keyboard
    /// frame-lock covers tmux panes too. nil-safe for previews/tests.
    var coordinator: SwiftTerminalView.Coordinator?
    var focusPolicy: TerminalFocusPolicy = .take
    /// Hold this pane at the size it has — see
    /// ``TerminalHostContainer/geometryHeld``.
    var geometryHeld: Bool = false
    /// Where the pane's own Paste and ⌘V go: the session's paste path, which
    /// asks tmux to bracket (`paste-buffer -p`) from the pane's real mode.
    /// Installed on the pane's coordinator (``SwiftTerminalView/Coordinator/onPaste``);
    /// without it SwiftTerm brackets by the local flag, which a `-CC` attach
    /// never sets — a multi-line paste ran line by line.
    var onPaste: ((String) -> Void)?

    func makeUIView(context: Context) -> TerminalHostContainer {
        // The terminal's gestures came with it when the controller minted it,
        // so they're already on by the time we host it here.
        coordinator?.onPaste = onPaste
        if focusPolicy == .take {
            let terminal = surface.view
            DispatchQueue.main.async { [weak terminal] in
                _ = terminal?.becomeFirstResponder()
            }
        }
        // A pane/window switch re-hosts a *persistent* terminal (its `.id` flips).
        // No repaint is forced here: SwiftTerm repaints a view that comes back
        // into a window, every row of one laid out at a new size, and the rows
        // that changed while it was away (fork patches 26–27); the core's view
        // draws what it holds when it reaches a window. Forcing it on top cost
        // a second whole-screen draw on every switch.
        let container = TerminalHostContainer()
        container.host(surface)
        coordinator?.hostContainer = container
        container.geometryHeld = geometryHeld
        return container
    }

    func updateUIView(_ uiView: TerminalHostContainer, context: Context) {
        guard let terminal = uiView.surface else { return }
        if uiView.geometryHeld != geometryHeld { uiView.geometryHeld = geometryHeld }
        coordinator?.onPaste = onPaste
        SwiftTerminalView.applyFocus(focusPolicy, to: terminal)
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
    /// iPad-landscape chrome trim: the chips are 30pt tall either way, this
    /// only sheds the bar's spare vertical padding. See
    /// `TerminalScreen.compactChrome`.
    var compact: Bool = false
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
    /// Images already uploaded this session — the chip's long-press menu
    /// offers each for re-insertion by number, no re-upload.
    var imageHistory: [ImageUploadLog.Entry] = []
    /// Whether the clipboard currently carries an image (adds the menu's
    /// "Paste image" entry).
    var clipboardHasImage: Bool = false
    /// Upload the clipboard image(s). nil hides the menu entry.
    var onImagePasteboard: (() -> Void)?
    /// Photograph something and attach it. nil (no camera — simulator,
    /// iPod-class hardware) hides the menu entry.
    var onImageCamera: (() -> Void)?
    /// Re-insert a previously uploaded image's path.
    var onImageHistory: ((ImageUploadLog.Entry) -> Void)?

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
    ///
    /// ROUNDED to whole points. The drag's translation is a raw finger
    /// coordinate — fractional — and offsetting the row by it leaves every
    /// chip resting on a half-pixel: each glyph then renders antialiased
    /// across two pixel rows, which on thin strokes (the ^ of ^C) reads as
    /// "the key is blurry" (user report, 2026-08-16). Whole points are crisp
    /// at every display scale; 1pt granularity during the drag itself is
    /// imperceptible. (The blur in that report turned out to be the gradient
    /// mask, not this — see `chipRowFade`. The rounding stays: it costs
    /// nothing.)
    private func clampedChipRowOffset(_ proposed: CGFloat, viewportWidth: CGFloat) -> CGFloat {
        guard chipRowContentWidth > viewportWidth else { return 0 }
        let minOffset = viewportWidth - chipRowContentWidth
        return min(0, max(minOffset, proposed)).rounded()
    }

    /// How far the edge fade reaches into the row. Narrow on purpose: the
    /// default row is one chip wider than a 402pt viewport, so at rest a chip
    /// always straddles the trailing edge, and the fade lands on it.
    private static let chipRowFadeWidth: CGFloat = 20

    /// The chip row's edge fade: the row fades out over an edge that has
    /// chips hidden past it — the standard "there's more this way" cue, and
    /// the only affordance this hand-rolled pan has (user ask, 2026-09-18).
    ///
    /// Painted OVER the row in the bar's own colour, never as a gradient
    /// `.mask`. A gradient mask makes the row an offscreen alpha pass, and on
    /// device (iOS 27, not the simulator) that pass rasterizes TEXT at 1x
    /// while symbols and strokes stay at 3x: esc/tab/^C came out as 1x
    /// bitmaps scaled up — the blur's curvature sat 96% on one pixel phase in
    /// three, the signature of a 3× upscale (lossless device capture,
    /// 2026-10-18). Both blur reports line up with it: 08-16 under the first
    /// fade (26pt, a gradient mask), quiet while the mask was a plain
    /// rectangle, back once this fade returned as a gradient mask. Over an
    /// opaque background a scrim of 1 − a composites to exactly what a mask
    /// of a did, so nothing about the look changes.
    ///
    /// Shaped so a chip under it still reads as a chip: a quadratic curve
    /// that keeps most of its reach near full opacity and drops to clear
    /// only in the last few points at the very edge. Each side grows in over
    /// its own width as content disappears past it, so the fade eases in and
    /// out at the ends of the pan instead of snapping on and off; when
    /// everything fits, both sides are zero-width.
    private func chipRowFade(viewportWidth: CGFloat) -> some View {
        let offset = clampedChipRowOffset(chipRowOffset + chipRowDragTranslation,
                                          viewportWidth: viewportWidth)
        let leading = min(Self.chipRowFadeWidth, max(0, -offset))
        let trailing = min(Self.chipRowFadeWidth,
                           max(0, chipRowContentWidth + offset - viewportWidth))
        return HStack(spacing: 0) {
            chipRowEdgeFade(width: leading, edge: .leading)
            Spacer(minLength: 0)
            chipRowEdgeFade(width: trailing, edge: .trailing)
        }
    }

    /// One edge of `chipRowFade`: clear on the row's side, the bar's colour
    /// at `edge`. How much of the row shows through follows 1 − t², t running
    /// towards the edge — three quarters of the way in it still shows 44%,
    /// and nearly everything inside the first quarter is untouched.
    private func chipRowEdgeFade(width: CGFloat, edge: HorizontalEdge) -> some View {
        let curve: [(CGFloat, Double)] = [(0, 1), (0.25, 0.94), (0.5, 0.75), (0.75, 0.44), (1, 0)]
        let stops = curve.map { location, showsThrough in
            Gradient.Stop(color: Ink.shortcutBarBG.opacity(1 - showsThrough), location: location)
        }
        return LinearGradient(
            stops: stops,
            startPoint: edge == .trailing ? .leading : .trailing,
            endPoint: edge == .trailing ? .trailing : .leading)
            .frame(width: width)
    }

    var body: some View {
        HStack(spacing: 0) {
            chipRow
            if let onToggleKeyboard {
                keyboardToggle(onToggleKeyboard)
            }
        }
        .frame(height: compact ? 38 : 50)
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

    /// The attach-image chip: tap opens the photo picker (the common case,
    /// kept at one tap); long-press opens a menu with the clipboard image
    /// (when there is one) and this session's already-uploaded images for
    /// re-insertion by number — Claude Code's [Image #N] mental model, held
    /// on our side of the wire because over SSH there is no other side.
    private func imageChip(_ shortcut: TerminalShortcut) -> some View {
        Menu {
            if clipboardHasImage, let onImagePasteboard {
                Button(action: onImagePasteboard) {
                    Label(String(localized: "Paste image from clipboard"),
                          systemImage: "doc.on.clipboard")
                }
            }
            if let onImageCamera {
                Button(action: onImageCamera) {
                    Label(String(localized: "Take photo"), systemImage: "camera")
                }
            }
            if !imageHistory.isEmpty, let onImageHistory {
                Section(String(localized: "Sent this session")) {
                    // Newest first: the image you just sent is the one you
                    // most likely want to reference again.
                    ForEach(imageHistory.reversed()) { entry in
                        Button { onImageHistory(entry) } label: {
                            Label("#\(entry.number) · \(entry.filename)", systemImage: "photo")
                        }
                    }
                }
            }
        } label: {
            Image(systemName: "photo")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Ink.primary)
                .frame(width: Metrics.shortcutKeyWidth, height: 30)
                .background(
                    Ink.shortcutKeyBG,
                    in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                        .strokeBorder(Ink.groupBorder, lineWidth: 1)
                )
                .contentShape(Rectangle())
        } primaryAction: {
            onTap(shortcut)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("Attach image"))
        .accessibilityIdentifier("attach-image")
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
            // A mask, not `.clipped()`: clipped() trims only the drawing, and
            // the offset row's hidden tail would still HIT-TEST beyond the
            // viewport — stealing taps under the pinned keyboard toggle. A
            // mask gates hit-testing along with the pixels. A plain
            // rectangle, never a gradient — the edge fade is painted over
            // the row instead; see `chipRowFade` for why.
            .mask(Rectangle())
            .overlay {
                chipRowFade(viewportWidth: viewport.size.width)
                    .allowsHitTesting(false)
            }
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
                } else if shortcut.kind == .image {
                    imageChip(shortcut)
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
                            } else if shortcut.kind == .files {
                                Image(systemName: "folder")
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
                    .accessibilityLabel(shortcut.kind == .paste ? Text("Paste")
                                        : shortcut.kind == .files ? Text("Browse files")
                                        : Text(shortcut.chipLabel))
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
        ("left", "arrow.left", "Move left"),
        ("up", "arrow.up", "Move up"),
        ("down", "arrow.down", "Move down"),
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

/// Shown while mosh re-homes onto a new network path. The path watcher only
/// says *a better path appeared*, not which interfaces it moved between, so the
/// title names the event rather than a direction it can't know.
struct RoamBanner: View {
    let metrics: SessionMetrics?
    @State private var pulsing = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(spacing: 8) {
                Circle()
                    .fill(Ink.warn)
                    .frame(width: 8, height: 8)
                    .shadow(color: Ink.warn.opacity(0.8), radius: 6)
                    .scaleEffect(pulsing ? 1.6 : 1)
                    .opacity(pulsing ? 0.55 : 1)
                Text("Network changed")
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
            guard !reduceMotion else { return }
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
            if i == 0 { path.move(to: CGPoint(x: x, y: y)) } else { path.addLine(to: CGPoint(x: x, y: y)) }
        }
        return path
    }
}

// MARK: - Terminal connecting / idle state

/// The branded waiting screen shown before a session is live for the first
/// time, in place of a black void with a lone spinner. A reconnect never shows
/// it (see `TerminalScreen.terminalBody`).
/// The mark, alive (cursor blinking in the pane you are about to land in), plus
/// the host identity answers "who am I connecting to, and what's happening" —
/// and yields entirely to SwiftTerm
/// the moment there's real output. Prototype: the moshpit OD project's
/// `terminal-idle.html`.
struct TerminalConnectingView: View {
    let connection: ServerConnection
    let state: TransportConnState

    /// Shared with the transport pill and the home card's row — see
    /// ``TransportConnState/transientTint``. This screen recolours everything it
    /// draws (the mark, the pulse dots, the accent pool, the capsule border), so
    /// a state whose colour disagreed with the pill's did not read as a detail:
    /// the whole screen changed hue mid-reconnect.
    private var accent: SwiftUI.Color {
        state.transientTint ?? Ink.accent
    }

    private var statusText: String {
        switch state {
        case .reconnecting:
            // mosh's reconnect really is a handoff — the session is roaming, not
            // being redialled. Plain SSH has no such story: it dropped and we
            // are dialling again, and calling that a handoff overclaims.
            return connection.connectionProtocol == .mosh
                ? String(localized: "Riding the handoff")
                : String(localized: "Reconnecting")
        // No retry is in flight in this state — `connState` routes an automatic
        // reconnect to `.reconnecting` now, so promising "retrying" here would be
        // the screen saying something the app is not doing.
        case .offline: return String(localized: "Line dropped")
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
                PaneLoaderMark(side: 112, accent: accent)

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
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 24)
                    .padding(.bottom, 18)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// The connecting screen's mark, following whichever app icon the user picked.
///
/// The default icon keeps its bespoke choreography — the cursor blinks, the
/// panes beside it pulse — because that animation is authored against the
/// panes geometry itself. None of the alternates HAS a cursor hole or side
/// panes (one is a handset, one is a house, one is the characters `:wq`), so
/// animating them the same way is not merely wrong, it is undefined. They
/// breathe instead: the same tempo as the cursor, carried by the artwork the
/// user chose. (Internal, not private: the NSH page's poster hosts it,
/// `moshpit/pane-loader`.)
struct PaneLoaderMark: View {
    var side: CGFloat
    var accent: SwiftUI.Color
    @Environment(AppSettings.self) private var settings

    var body: some View {
        let option = AppIconCatalog.option(for: settings.appIconId)
        if option.isPrimary {
            PaneLoaderGlyph(side: side, accent: accent)
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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        AppIconThumb(option: option, side: side)
            .scaleEffect(breathing ? 1.04 : 0.96)
            // The glow is the accent, not the artwork's own colours: it is the
            // connection state talking, and it has to read the same whichever
            // icon is underneath.
            .shadow(color: accent.opacity(breathing ? 0.55 : 0.2),
                    radius: side * (breathing ? 0.16 : 0.09))
            // Matches PaneLoaderGlyph's cursor, so the two loaders keep time.
            .animation(reduceMotion ? nil : .easeInOut(duration: 1.15).repeatForever(autoreverses: true),
                       value: breathing)
            .frame(width: side, height: side)
            // Reduce Motion: hold the bright, settled pose instead of pulsing.
            .onAppear { breathing = true }
    }
}

/// The connecting screen's animated mark: same geometry as ``MoshpitGlyph``
/// (all values from its `Metrics`), drawn alive — the cursor blinks in the
/// pane you are about to land in, and the panes beside it pulse in a stagger.
/// The window is warming up. Kept local to the loader; every static placement
/// of the mark stays `MoshpitGlyph`.
private struct PaneLoaderGlyph: View {
    private typealias M = MoshpitGlyph.Metrics

    var side: CGFloat
    var accent: SwiftUI.Color
    @State private var alive = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let k = side / M.tile
        ZStack(alignment: .topLeading) {
            IconTile(accent: accent, side: side)

            // The pane, solid; the cursor is a hole punched by the block below
            // fading in and out — a terminal cursor blinking at 1.15s, the
            // tempo the alternates' breathing keeps time with.
            FocusedPaneShape(cursorOpen: false)
                .fill(.white)
                .frame(width: side, height: side)
                .shadow(color: .black.opacity(M.shadowAlpha),
                        radius: side * M.shadowBlur, y: side * M.shadowDrop)

            RoundedRectangle(cornerRadius: M.cursorCorner * k, style: .continuous)
                .fill(accent)
                .frame(width: M.cursor.width * k, height: M.cursor.height * k)
                .offset(x: M.cursor.minX * k, y: M.cursor.minY * k)
                .opacity(alive ? 1 : 0.15)
                .animation(reduceMotion ? nil : .easeInOut(duration: 1.15).repeatForever(autoreverses: true),
                           value: alive)

            ForEach(Array(M.beside.enumerated()), id: \.offset) { i, r in
                RoundedRectangle(cornerRadius: M.besideCorner * k, style: .continuous)
                    .fill(.white.opacity(alive ? 0.7 : 0.35))
                    .frame(width: r.width * k, height: r.height * k)
                    .offset(x: r.minX * k, y: r.minY * k)
                    .shadow(color: .black.opacity(M.shadowAlpha),
                            radius: side * M.shadowBlur, y: side * M.shadowDrop)
                    .animation(reduceMotion ? nil : .easeInOut(duration: 0.85)
                        .repeatForever(autoreverses: true)
                        .delay(Double(i) * 0.3), value: alive)
            }
        }
        .frame(width: side, height: side)
        .shadow(color: accent.opacity(0.35), radius: side * 0.12, y: side * 0.04)
        .onAppear { alive = true }
    }
}

/// Three mini cursor blocks blinking in a stagger — the crowd itself as the
/// loading indicator, replacing the generic system spinner.
private struct PulseDots: View {
    var color: SwiftUI.Color
    @State private var on = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 4) {
            ForEach(0..<3, id: \.self) { i in
                RoundedRectangle(cornerRadius: 1.5, style: .continuous)
                    .fill(color)
                    .frame(width: 5, height: 9)
                    .opacity(on ? 1 : 0.25)
                    .animation(reduceMotion ? nil : .easeInOut(duration: 0.55)
                        .repeatForever(autoreverses: true)
                        .delay(Double(i) * 0.18), value: on)
            }
        }
        .onAppear { on = true }
    }
}

// MARK: - Reconnect over the frozen frame

/// The reconnect overlay: a dim over the pane that is no longer live and one
/// status capsule at the top, in the transport pill's colour family. Nothing
/// else moves — the frame the person was reading stays put and comes back
/// into focus when the replacement crossfades in over it.
///
/// It also lies over the last frame of a session the server ENDED (`exit`,
/// mosh-server's shutdown), and there it says so rather than "line dropped":
/// nothing dropped, and nothing is coming back on its own.
struct ReconnectVeil: View {
    let state: TransportConnState
    let isMosh: Bool
    /// The server ended the session (`TerminalViewModel.endedByServer`).
    var ended = false

    private var accent: SwiftUI.Color {
        // An end is not a fault: the calm ink, not the dropped line's red.
        ended ? Ink.secondary : (state.transientTint ?? Ink.accent)
    }

    /// What the capsule says.
    static func label(state: TransportConnState, isMosh: Bool, ended: Bool) -> String {
        if ended { return String(localized: "Session ended") }
        switch state {
        case .offline: return String(localized: "Line dropped")
        case .connecting: return String(localized: "Opening the pit")
        default:
            return isMosh
                ? String(localized: "Riding the handoff")
                : String(localized: "Reconnecting")
        }
    }

    private var label: String { Self.label(state: state, isMosh: isMosh, ended: ended) }

    var body: some View {
        ZStack(alignment: .top) {
            Color.black.opacity(0.45)
            HStack(spacing: 10) {
                if state == .offline || ended {
                    Circle()
                        .fill(accent)
                        .frame(width: 8, height: 8)
                        .shadow(color: accent.opacity(0.8), radius: 4)
                } else {
                    PulseDots(color: accent)
                }
                Text(label.uppercased())
                    .font(Face.mono(11, .semibold))
                    .kerning(1.8)
                    .foregroundStyle(Ink.primary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .background(.ultraThinMaterial, in: Capsule())
            .overlay(Capsule().strokeBorder(accent.opacity(0.35), lineWidth: 1))
            .padding(.top, 14)
        }
    }
}

/// The follower's veil: the window on screen is sized for another Moshpit
/// device (the lease in `@moshpit_size_owner`), so the pane keeps the frame
/// the person last saw, dimmed, and one capsule names the holder. A tap
/// anywhere takes the window back — the one thing that ends following.
private struct SizeFollowVeil: View {
    let ownerLabel: String
    let takeOver: () -> Void

    var body: some View {
        ZStack(alignment: .top) {
            Color.black.opacity(0.45)
            VStack(spacing: 6) {
                HStack(spacing: 10) {
                    Image(systemName: ownerLabel == "iPad" ? "ipad" : "iphone")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Ink.accent)
                    Text(String(localized: "Following \(ownerLabel)").uppercased())
                        .font(Face.mono(11, .semibold))
                        .kerning(1.8)
                        .foregroundStyle(Ink.primary)
                }
                Text(String(localized: "Tap to take over"))
                    .font(Face.mono(10, .medium))
                    .foregroundStyle(Ink.secondary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous)
                .strokeBorder(Ink.accent.opacity(0.35), lineWidth: 1))
            .padding(.top, 14)
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: takeOver)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
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
    /// True on a reconnect: the seconds before this view decides anything
    /// show the terminal background (under the reconnect veil), not the
    /// full-screen connecting animation — that is for a first connect only
    /// (see `terminalBody`).
    var quietWait = false
    /// What the creating said when it failed (the controller's notice):
    /// the Create button is back.
    var failure: String?
    let onCreate: () -> Void
    let onInstall: () -> Void
    @State private var settled: Bool
    @State private var creating = false

    init(multiplexer: Multiplexer, binaryMissing: Bool, connection: ServerConnection,
         connectionState: TransportConnState, quietWait: Bool = false,
         startsSettled: Bool = false, failure: String? = nil,
         onCreate: @escaping () -> Void, onInstall: @escaping () -> Void) {
        self.multiplexer = multiplexer
        self.binaryMissing = binaryMissing
        self.connection = connection
        self.connectionState = connectionState
        self.quietWait = quietWait
        self.failure = failure
        self.onCreate = onCreate
        self.onInstall = onInstall
        _settled = State(initialValue: startsSettled)
    }

    var body: some View {
        VStack(spacing: 14) {
            if settled {
                if binaryMissing {
                    missingState
                } else {
                    noSessionsState
                }
            } else if !quietWait {
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
            guard !settled else { return }
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            withAnimation(.easeOut(duration: 0.25)) { settled = true }
        }
        .onChange(of: failure) { _, failure in
            if failure != nil { creating = false }
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
        Text("Moshpit needs \(multiplexer.label) for \(multiplexer.vocabulary.sessionLower) navigation.\nInstall it, then reconnect.")
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
        Text("No \(multiplexer.label) \(multiplexer.vocabulary.sessionPluralLower)")
            .font(Face.display(16, .semibold)).foregroundStyle(Ink.primary)
        Text("This server has no running \(multiplexer.vocabulary.sessionPluralLower).\nMoshpit only attaches — create the first one to start.")
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
    /// Ask the server for a from-scratch repaint (see the popover call site).
    var onRepaint: (() -> Void)?

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
                // Send side — for "typing does nothing": type once, reopen,
                // and compare. sent/user frozen = input never leaves the app;
                // user climbing with ack stuck = it dies on the wire; channel
                // anything but "ready" = the UDP flow itself is wedged.
                row("sent", d.packetsSent)
                row("user", d.lastUserNum)
                row("acked", d.serverAckedUserNum)
                HStack {
                    Text("channel").font(Face.mono(12)).foregroundStyle(Ink.meta)
                    Spacer(minLength: 20)
                    Text(d.channelState).font(Face.mono(12, .bold))
                        .foregroundStyle(d.channelState == "ready" ? Ink.primary : Ink.warn)
                }
                if d.inputHeld { Text("input held until the server answers").font(Face.mono(12, .bold)).foregroundStyle(Ink.warn) }
            } else {
                Text("No datagrams yet.")
                    .font(Face.mono(12))
                    .foregroundStyle(Ink.primary)
            }
        }
        .padding(14)
        .accessibilityIdentifier("mosh-diagnostics-overlay")
        .safeAreaInset(edge: .bottom) {
            if let onRepaint {
                Button(action: onRepaint) {
                    Text("Repaint screen")
                        .font(Face.mono(12, .bold))
                        .foregroundStyle(Ink.accent)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                        .background(Ink.accent.opacity(0.11),
                                    in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
                .buttonStyle(.plain)
                .padding(EdgeInsets(top: 0, leading: 14, bottom: 12, trailing: 14))
            }
        }
    }

    private func row(_ label: String, _ value: UInt64) -> some View {
        HStack {
            Text(label).font(Face.mono(12)).foregroundStyle(Ink.meta)
            Spacer(minLength: 20)
            Text("\(value)").font(Face.mono(12, .bold)).foregroundStyle(Ink.primary)
        }
    }
}

// MARK: - The NSH page (`MOSHPIT_NSH_TERMINAL`)

/// The seam where the screen draws through NSH (``TerminalNSHPage``)
/// instead of its SwiftUI chrome. Everything that is not drawn stays the
/// screen's: the session, the keyboard intent, the presentations, the
/// lifecycle above. What the chrome shows is read here as the SwiftUI
/// body reads it, and what the person asks for runs the same code.
extension TerminalScreen {
    /// Whether this screen draws through NSH: its switch, as it was when the
    /// screen was made, unless NSH could not start.
    private var usesNSH: Bool { nshTerminal && !nshFailed }

    /// Whether the breadcrumb's pickers are NSH's: always with the NSH
    /// terminal, else by their own switch.
    private var nshPickers: Bool { usesNSH || NSHScreen.muxPicker.isOn }

    /// mosh's counters while the page shows them (`{"op": "diagnostics"}`).
    private var nshDiagnostics: TerminalNSHDiagnostics {
        TerminalNSHDiagnostics(shown: showMoshDiagnostics && connection.connectionProtocol == .mosh,
                               counters: metrics?.moshDiagnostics)
    }

    private var nshPage: some View {
        TerminalNSHPage(
            model: nshModel,
            chrome: nshChrome,
            bar: nshBar,
            diagnostics: nshDiagnostics,
            dictation: dictation,
            attachment: imageAttachment,
            preview: active?.remotePreview,
            target: nshTarget,
            focus: focusPolicy,
            geometryHeld: geometryHold,
            appearance: TerminalNSHStage.Appearance(
                theme: theme, fontSize: settings.fontSize, fontName: settings.fontName,
                cursorShape: settings.cursorShape, cursorColorId: effectiveCursorColorId,
                cursorBlink: settings.cursorBlink),
            onPaste: { [weak active] text in active?.sendPaste(text) },
            perform: nshPerform,
            onFailure: {
                Log.ui.error("terminal: NSH did not start; the SwiftUI chrome shows")
                nshFailed = true
            }
        ) {
            nshAnchors
        }
        // What the SwiftUI chrome's own views watch (the pane a deep link
        // asked for, the breadcrumb kept across a reconnect, a new session's
        // first-live bookkeeping), watched here while they are not drawn.
        .onChange(of: paneReadinessSignal) { selectPendingPaneIfReady() }
        .onChange(of: router.paneRequest) {
            claimPendingPaneRequestIfNeeded()
            selectPendingPaneIfReady()
        }
        .onChange(of: livePlan) { _, plan in
            if let plan { lastBreadcrumb = plan }
        }
        .onChange(of: activeIdentity) { _, _ in sessionSwapped() }
    }

    /// What the chrome shows now.
    private var nshChrome: TerminalNSHChrome {
        var chrome = TerminalNSHChrome()
        chrome.name = connection.displayName
        chrome.user = connection.username
        chrome.host = connection.host
        chrome.port = connection.port
        chrome.mosh = connection.connectionProtocol == .mosh
        chrome.mux = connection.multiplexer
        chrome.sidebar = onToggleSidebar != nil
        chrome.pill = transportKind
        chrome.state = connState
        if let plan = livePlan ?? retainedPlan {
            let vocabulary = connection.multiplexer.vocabulary
            chrome.crumbs = .init(plan: plan, sessionIcon: vocabulary.sessionIcon,
                                  windowIcon: vocabulary.windowIcon, live: livePlan != nil)
        }
        chrome.still = stillSwitch
        if posterVisible {
            chrome.cover = .init(kind: .poster, state: posterState, touches: connState != .live)
        } else if veilVisible {
            chrome.cover = .init(kind: .veil, state: posterState,
                                 ended: active?.viewModel.endedByServer == true)
        } else if let follow = sizeFollow {
            chrome.cover = .init(kind: .follow, state: connState, owner: follow.ownerLabel, touches: true)
        }
        (chrome.content, chrome.empty) = nshContent
        if active?.moshReturnPathDead == true {
            chrome.banner = .returnPath
        } else if metrics?.isRoaming == true {
            chrome.banner = .roam(srtt: metrics?.srttMs, history: metrics?.srttHistory ?? [])
        } else if let notice = active?.degrade {
            chrome.banner = .degrade(notice.missing)
        }
        if let text = active?.tmuxControl?.notice ?? active?.herdrNotice {
            chrome.notice = .init(text: text, action: active?.herdrNoticeAction)
        }
        chrome.link = copiedURL.map { $0.host() ?? $0.absoluteString }
        chrome.compact = compactChrome
        return chrome
    }

    /// The shortcut bar (`bottomAccessory`'s `ShortcutBarView`).
    private var nshBar: TerminalNSHBar {
        let toolbar = shortcutStore.toolbar(
            forHost: connection.displayName,
            inMultiplexer: active?.tmuxControl != nil || active?.herdrControl != nil)
        return TerminalNSHBar(
            chips: toolbar.map(TerminalNSHBar.chip),
            keyboardDown: keyboardIntent != .up,
            ctrlArmed: ctrlArmed,
            voice: settings.voiceInputEnabled,
            micActive: dictation != nil,
            clipboardHasImage: clipboardHasImage,
            camera: CameraCaptureView.isAvailable,
            sent: (active?.imageUploads.entries ?? []).map {
                TerminalNSHBar.Sent(id: $0.id.uuidString, number: $0.number, filename: $0.filename)
            })
    }

    /// What the pane area holds (`terminalContent`'s branches).
    private var nshContent: (TerminalNSHChrome.Content, TerminalNSHChrome.Empty?) {
        guard let active else { return (.none, nil) }
        return Self.paneContent(of: active, isReconnect: isReconnect, connState: connState)
    }

    /// ``nshContent`` of `active`: a reconnect's terminal (`isReconnect`),
    /// its transport in `connState`.
    @MainActor
    static func paneContent(of active: SessionHub.ActiveSession, isReconnect: Bool,
                           connState: TransportConnState)
        -> (TerminalNSHChrome.Content, TerminalNSHChrome.Empty?) {
        let ghost = active.retiredTmuxController
        if let controller = ghost ?? active.tmuxController {
            if ghost != nil || controller.snapshot.isAttached { return (.terminal, nil) }
            if isReconnect && (connState != .live || active.awaitingFirstFrame) { return (.blank, nil) }
            // Missing is the host's probe's answer: known. No sessions isn't
            // yet: the attach may only be on its way.
            let missing = active.capabilities?.hasTmux == false
            return (.empty, .init(mux: .tmux, missing: missing, quiet: isReconnect, settled: missing))
        }
        if let herdr = active.herdrControl, herdr.serverNotRunning {
            // The server answered that it isn't running: known.
            return (.empty, .init(mux: .herdr, missing: active.capabilities?.hasHerdr == false,
                                  quiet: isReconnect, settled: true))
        }
        // mosh + tmux on a server with no session (B228): the renderer on
        // screen waits for one in silence. The empty state with its Create
        // goes over it, as the SwiftUI terminal's overlay does; without it
        // the screen stayed black (B337). The server was asked already, so
        // the answer is known: it shows at once (settled), with nothing, not
        // the poster, before it.
        if active.awaitingFirstSession, connState == .live {
            return (.empty, .init(mux: .tmux, missing: false, quiet: true, settled: true))
        }
        return (.terminal, nil)
    }

    /// The terminal the slot shows (`terminalContent`, `TmuxPaneSplitView`).
    private var nshTarget: TerminalNSHStage.Target? {
        guard let active else { return nil }
        let ghost = active.retiredTmuxController
        if let controller = ghost ?? active.tmuxController {
            let snapshot = controller.snapshot
            guard ghost != nil || snapshot.isAttached,
                  let pane = snapshot.activePaneId ?? snapshot.activePanes.first?.id else { return nil }
            let frozen = ghost != nil
            return .init(source: .pane(controller, pane), inset: compactChrome ? 2 : 4, frozen: frozen,
                         slide: snapshot.hasSwipedSinceAttach && !frozen ? snapshot.lastSwitchForward : nil)
        }
        if let herdr = active.herdrControl, herdr.serverNotRunning { return nil }
        return .init(source: .plain(active))
    }

    /// The host's presentations, hung where the screen draws what they
    /// belong to: the pickers from the breadcrumb (a popover on the iPad, a
    /// sheet on the phone), their content NSH's (``MuxPicker``). The pill's
    /// question and mosh's counters are the page's own.
    @ViewBuilder
    private var nshAnchors: some View {
        let anchors = nshModel.anchors
        let crumb = anchors.breadcrumb ?? CGRect(x: 0, y: 0, width: 1, height: 1)
        Color.clear
            .frame(width: crumb.width, height: crumb.height)
            .adaptivePresentation(isPresented: $showWindowsSheet) {
                if let c = active?.tmuxControl { MuxPicker(controller: c, kind: .windows, nsh: nshPickers) } else if let h = active?.herdrControl { MuxPicker(controller: h, kind: .windows, nsh: nshPickers) }
            }
            .adaptivePresentation(isPresented: $showSessionsSheet) {
                if let c = active?.tmuxControl { MuxPicker(controller: c, kind: .sessions, nsh: nshPickers) } else if let h = active?.herdrControl { MuxPicker(controller: h, kind: .sessions, nsh: nshPickers) }
            }
            .adaptivePresentation(isPresented: $showPaneSheet) {
                if let c = active?.tmuxControl { MuxPicker(controller: c, kind: .panes, nsh: nshPickers) } else if let h = active?.herdrControl { MuxPicker(controller: h, kind: .panes, nsh: nshPickers) }
            }
            .position(x: crumb.midX, y: crumb.midY)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
    }

    /// What the NSH screen asked for, done as the SwiftUI chrome does it.
    private func nshPerform(_ intent: TerminalNSHIntent) {
        switch intent {
        case .back:
            leave()
        case .sidebar:
            if let onToggleSidebar { onToggleSidebar() } else { leave() }
        case .transport:
            // Confirmed on the page's own action sheet.
            switchProtocol()
        case .diagnostics:
            if connection.connectionProtocol == .mosh { showMoshDiagnostics = true }
        case .diagnosticsDismiss:
            showMoshDiagnostics = false
        case .repaint:
            // Ack state 0: the server rediffs from blank (MoshDiagnosticsView).
            active?.requestRepaint()
            showMoshDiagnostics = false
        case .crumb(let crumb):
            // The remembered tree is inert: its controller is being rebuilt.
            guard livePlan != nil else { return }
            switch crumb {
            case .session: presentSheet { showSessionsSheet = true }
            case .window: presentSheet { showWindowsSheet = true }
            case .pane: presentSheet { showPaneSheet = true }
            }
        case .window(let number):
            selectWindow(number: number)
        case .takeOver:
            active?.tmuxController?.takeSizeOwnership()
        case .create:
            if let herdr = active?.herdrControl, herdr.serverNotRunning {
                herdr.newSession(named: nil)
            } else {
                active?.createFirstTmuxSession()
            }
        case .install:
            installPackages = active?.herdrControl != nil ? ["herdr"] : ["tmux"]
        case .bannerAction:
            if active?.moshReturnPathDead == true {
                switchProtocol()
            } else if let notice = active?.degrade {
                installPackages = notice.packages
            }
        case .bannerDismiss:
            if active?.moshReturnPathDead == true {
                active?.moshReturnPathDead = false
            } else {
                active?.degrade = nil
            }
        case .noticeAction:
            active?.runHerdrNoticeAction()
        case .openLink:
            if let url = copiedURL {
                UIApplication.shared.open(url)
                copiedURL = nil
            }
        case .chip(let id):
            let toolbar = shortcutStore.toolbar(
                forHost: connection.displayName,
                inMultiplexer: active?.tmuxControl != nil || active?.herdrControl != nil)
            if let shortcut = toolbar.first(where: { $0.id.uuidString == id }) { send(shortcut: shortcut) }
        case .arrow(let direction):
            sendArrow(direction)
        case .scroll(let direction):
            scrollHistory(direction)
        case .keyboard:
            keyboardIntent = keyboardIntent == .up ? .down : .up
        case .mic:
            if settings.voiceInputEnabled { toggleDictation() }
        case .image:
            disarmStickyCtrlIfNeeded()
            photoPickerPresented = true
        case .imagePaste:
            beginClipboardImageAttachment()
        case .imageCamera:
            if CameraCaptureView.isAvailable { presentSheet { showCamera = true } }
        case .imageSent(let id):
            if let entry = active?.imageUploads.entries.first(where: { $0.id.uuidString == id }) {
                insertUploadedImage(entry)
            }
        case .dictation(let action):
            switch action {
            case .cancel: cancelDictation()
            case .insert: finishDictation()
            case .send: finishDictation(submit: true)
            case .settings:
                if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
                cancelDictation()
            default: break
            }
        case .attachment(let action):
            switch action {
            case .cancel: cancelImageAttachment()
            case .retry: imageAttachment?.retry()
            case .insert: finishImageAttachment()
            case .send: finishImageAttachment(submit: true)
            default: break
            }
        case .preview(let action):
            switch action {
            case .cancel: dismissRemotePreview()
            case .confirm:
                Haptics.tap()
                active?.remotePreview?.confirm()
            default: break
            }
        }
    }
}

/// The bar's sticky Ctrl: armed on one coordinator at a time — the one
/// typing went to when it was armed — and let go when typing moves to
/// another pane or session (B411). It used to stay armed on the pane left,
/// for good, while the chip followed the screen: back in that pane, the
/// first letter typed went out as a control code (^C interrupting an
/// agent, ^D ending a shell), and the chip's "off" cleared only the pane on
/// screen.
@MainActor
final class StickyCtrl {
    private(set) weak var coordinator: SwiftTerminalView.Coordinator?
    /// Whether it is armed, for the chip.
    var onChange: ((Bool) -> Void)?

    var isArmed: Bool { coordinator?.pendingCtrl == true }

    /// The next key typed into `coordinator` goes out as its control code.
    func arm(_ coordinator: SwiftTerminalView.Coordinator) {
        if let armed = self.coordinator, armed !== coordinator { armed.pendingCtrl = false }
        self.coordinator = coordinator
        coordinator.pendingCtrl = true
        coordinator.onPendingCtrlConsumed = { [weak self, weak coordinator] in
            guard let self, self.coordinator === coordinator else { return }
            self.coordinator = nil
            self.onChange?(false)
        }
        onChange?(true)
    }

    func disarm() {
        coordinator?.pendingCtrl = false
        coordinator = nil
        onChange?(false)
    }

    /// Typing now goes to `target`'s coordinator (nil: nowhere).
    func typingMoved(to target: ObjectIdentifier?) {
        guard let armed = coordinator, ObjectIdentifier(armed) != target else { return }
        disarm()
    }
}

// MARK: - A protocol switch's still picture (the SwiftUI chrome)

/// The terminal on screen as a protocol switch began, drawn from its own
/// rows (``TerminalSurface/snapshotView()``: a Metal layer's snapshot is not
/// to be trusted), and where it was in the window. The SwiftUI chrome lays
/// it over the pane until the new session's frame is uncovered; the NSH
/// page's stage keeps its own (``TerminalNSHStage/holdStill()``).
struct TerminalStill {
    let picture: UIView
    /// Its frame in the window's coordinates.
    let frame: CGRect

    /// `surface`'s picture, if it is on screen.
    @MainActor
    init?(of surface: TerminalSurface?) {
        guard let surface, let window = surface.view.window,
              let picture = surface.snapshotView() else { return nil }
        picture.isUserInteractionEnabled = false
        self.picture = picture
        frame = surface.view.convert(surface.view.bounds, to: window)
    }
}

/// ``TerminalStill`` where it was in the window, whatever the SwiftUI layout
/// around it does meanwhile.
struct TerminalStillView: UIViewRepresentable {
    let still: TerminalStill

    func makeUIView(context: Context) -> Host {
        let host = Host()
        host.isUserInteractionEnabled = false
        host.clipsToBounds = true
        host.show(still)
        return host
    }

    func updateUIView(_ uiView: Host, context: Context) {
        uiView.show(still)
    }

    final class Host: UIView {
        private var still: TerminalStill?

        func show(_ still: TerminalStill) {
            guard self.still?.picture !== still.picture else { return }
            self.still?.picture.removeFromSuperview()
            self.still = still
            addSubview(still.picture)
            setNeedsLayout()
        }

        override func layoutSubviews() {
            super.layoutSubviews()
            guard let still else { return }
            still.picture.frame = window == nil ? still.frame : convert(still.frame, from: nil)
        }
    }
}
