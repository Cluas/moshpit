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
    /// The NSH page's pane overview is up: the keyboard is away while it is,
    /// as for a sheet (``anySheetOpen``).
    @State private var nshOverviewOpen = false
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
    /// A one-off said for a moment (B492): the notice the multiplexer
    /// puts up wins over it.
    @State private var flashNotice: String?
    /// Moves once a second while a herdr refusal's backoff counts down
    /// (B490): the banner's "N s" reads it.
    @State private var retryTick = Date()
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
    /// between a first connect (nothing to cover: no cover, B657) and a
    /// reconnect (a frame worth keeping on screen is right there). Reset when the session object changes,
    /// unless this screen swapped it in itself (``swapCarriesLive``).
    @State private var wasEverLive = false
    /// A manual reconnect (a protocol switch, Install Assist, the ended
    /// card's Reconnect) is swapping in a new session while this screen
    /// shows a reconnect's frame: the new session comes up as a reconnect
    /// too, under the veil (the owner's rule: a reconnect keeps its frame,
    /// never a full-screen cover). A protocol switch always
    /// carries it. Read and cleared when the session changes
    /// (``sessionSwapped()``).
    @State private var swapCarriesLive = false
    /// A protocol switch (mosh ⇄ SSH) is under way, from its confirm until
    /// the new session's first frame is uncovered: nothing moves (the
    /// owner: 「mosh 切换 ssh 有个动画 我们直接去掉」). The veil and its
    /// capsule are there at once, the pane's last picture stays under it
    /// (the stage's, ``TerminalNSHStage/holdStill()``) and goes at once
    /// with the veil, the pill and the breadcrumb change at once. Ordinary
    /// drops and resumes keep their motion.
    @State private var stillSwitch = false
    /// The next dial goes over SSH, saving nothing (``reconnectOverSSHOnce``).
    @State private var sshOnce = false

    /// Whether this screen has put the cover up for a phase that was not
    /// `.live` — the only licence for a cover to still be up once the
    /// transport reads `.live` (waiting for the first frame, see `settling`).
    ///
    /// A screen that appears onto a session that is ALREADY live (entered
    /// from the home card or the session tree after connecting there) never
    /// shows a cover: a cover would be up for the one frame before the
    /// pane paints and come straight back down in the very update that
    /// resolves `active` and hosts the pane. On iOS 27 that exact sequence
    /// left every SwiftUI control in the top bar deaf to taps for the life
    /// of the screen — breadcrumb, transport pill, back — while the UIKit
    /// terminal beneath kept working (rig, iPhone 17 Pro iOS 27.0
    /// simulator, 2026-09-14; iOS 26.2 was unaffected). Holding the cover a
    /// beat longer avoided it; not putting it up at all is the honest
    /// version, since there is nothing to cover.
    @State private var coverArmed = false
    /// The last state the veil announced before the transport came back:
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
        // Its terminal stays while the landing waits, listed or not (B736).
        active?.tmuxController?.holdPane(request.paneId)
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
            active?.tmuxController?.holdPane(nil)
            Log.island.notice("land: pane \(paneId, privacy: .public) is not on the server \(Int(Self.landingPatience), privacy: .public) s after attach; dropped")
            // Said, for a moment (B492): landing on another pane without a
            // word read as the notification having taken the person
            // somewhere wrong.
            flash(String(localized: "That pane is closed"))
            return
        case .select:
            break
        }
        guard let snapshot = tmuxSnapshot else { return }
        pendingPaneSelection = nil
        pendingPaneAttachedAt = nil
        active?.tmuxController?.holdPane(nil)
        Log.island.info("land: selecting pane=\(paneId, privacy: .public) from=\(snapshot.activePaneId ?? "-", privacy: .public) phase=\(String(describing: scenePhase), privacy: .public)")
        if let controller = active?.tmuxController {
            controller.selectPane(paneId)
        } else if let control = active?.moshControl {
            control.selectPane(paneId)
        } else {
            active?.herdrControl?.selectPane(paneId)
        }
    }

    /// `text` said for a moment (B492): three seconds, unless another
    /// one-off replaces it.
    private func flash(_ text: String) {
        flashNotice = text
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(Self.flashSeconds))
            if flashNotice == text { flashNotice = nil }
        }
    }

    nonisolated static let flashSeconds: Double = 3

    /// The notice the screen shows: the multiplexer's, else a one-off.
    nonisolated static func shownNotice(multiplexer: String?, flashed: String?) -> String? {
        multiplexer ?? flashed
    }

    /// A herdr notice's text, with the seconds left before what it refused
    /// is asked again, while a backoff runs (B490).
    private func herdrNoticeText(_ text: String) -> String {
        guard active?.tmuxControl?.notice == nil,
              let left = Self.retrySecondsLeft(until: active?.herdrControl?.retryDeadline, now: retryTick)
        else { return text }
        return text + " · " + String(localized: "Retrying in \(left) s")
    }

    /// Whole seconds until `deadline` from `now`, rounded up; nil once it
    /// has passed (or there is none).
    nonisolated static func retrySecondsLeft(until deadline: Date?, now: Date) -> Int? {
        guard let deadline else { return nil }
        let left = deadline.timeIntervalSince(now)
        guard left > 0 else { return nil }
        return Int(left.rounded(.up))
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

    /// Push theme + font + cursor into the tmux controller. The plain path
    /// takes them through the NSH stage (``TerminalNSHStage/apply(appearance:)``).
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
        showWindowsSheet || showSessionsSheet || showPaneSheet || showCamera || nshOverviewOpen
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
        if connection.dialsMosh {
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

    /// Split out of `body` — one giant modifier chain through every
    /// `.onChange` blew the type checker's inference budget (it kept
    /// cascading to a different closure each time one was fixed in place,
    /// since the whole chain is jointly inferred as a single expression).
    /// Breaking the chain into two separately-typed expressions bounds that.
    private var contentWithLifecycle: some View {
        Group {
            if nshFailed {
                // Nothing to draw the terminal with: the page says so, under
                // the system's bar, whose back button leaves.
                NSHPageFailed { nshFailed = false }
            } else {
                nshPage
            }
        }
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
        // The page draws its own bar; only a page that failed shows the
        // system's, for its back button. No `preferredColorScheme`: the
        // terminal takes its colour scheme's appearance on its own surface
        // (R12, `Screen::Terminal`), not the window's, so Home under the
        // back swipe keeps the app's.
        .navigationBarHidden(!nshFailed)
        .onGeometryChange(for: Bool.self) { proxy in
            UIDevice.current.userInterfaceIdiom == .pad
                && proxy.size.width > proxy.size.height
        } action: { compactChrome = $0 }
        .task {
            if settings.raiseKeyboardOnOpen { keyboardIntent = .up }
            await connectAndAttach()
        }
        .onDisappear {
            // Back to Home keeps the connection alive but stops rendering; the
            // core hands the phone-grid window pin back when the pane leaves
            // the screen (``SessionHub/visibleSession``).
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
        .task(id: active?.herdrControl?.retryDeadline) {
            // The banner's countdown (B490): a tick a second until it ends.
            while let deadline = active?.herdrControl?.retryDeadline, deadline > Date() {
                retryTick = Date()
                try? await Task.sleep(for: .seconds(1))
                if Task.isCancelled { return }
            }
            retryTick = Date()
        }
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
            // mints a fresh controller per reconnect, so first responder died
            // with the old pane view and the keyboard slid away mid-cycle,
            // then popped back when the new pane took focus. The retired controller now stays hosted until
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
    /// Whether the NSH page failed to start (the page then says so,
    /// ``NSHPageFailed``), and its model.
    @State private var nshFailed = false
    @State private var nshModel = TerminalNSHModel()

    var body: some View {
        contentWithLifecycle
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
        .alert(active?.viewModel.endedByServer == true
                   ? String(localized: "Session ended") : String(localized: "Connection Error"),
               isPresented: $showError) {
            errorActions
        } message: {
            Text(active?.viewModel.errorMessage ?? "")
        }
        .appSheet(item: installTargetBinding) { target in
            if let active {
                InstallAssistNSHPage(session: active, packages: target.packages, onReconnect: { reconnect() })
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
            RemoteFolderNSHSheet(folder: target.folder, files: target.files)
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

    /// The banner's "this time only": reconnects over SSH, through the jump
    /// host, without saving anything. This screen's copy of the connection
    /// reads as SSH until the next reconnect reloads the saved one.
    private func reconnectOverSSHOnce() {
        guard active != nil else { return }
        stillSwitch = true
        holdStill()
        sshOnce = true
        Haptics.select()
        reconnect(still: true)
    }

    /// The pane's picture as the switch begins, kept until it ends (the
    /// stage's, ``TerminalNSHStage/holdStill()``).
    private func holdStill() {
        nshModel.stage.holdStill()
    }

    /// The switch is over: the new session's frame is uncovered. Its
    /// picture goes at once, and motion is back.
    private func endStillSwitch() {
        stillSwitch = false
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

    /// The alert's buttons. A session the server ended — its shell exited,
    /// or mosh-server sent its shutdown — is not an error: it used to
    /// arrive as "Connection Error", whose only button left the screen. It
    /// says what happened, and offers the reconnect nothing automatic will
    /// make.
    @ViewBuilder
    private var errorActions: some View {
        if active?.viewModel.endedByServer == true {
            Button("Done", role: .cancel) {
                showError = false
                active?.viewModel.errorMessage = nil
                leave()
            }
            Button("Reconnect") {
                showError = false
                active?.viewModel.errorMessage = nil
                reconnect()
            }
        } else {
            Button("OK") {
                showError = false
                active?.viewModel.errorMessage = nil
                leave()
            }
        }
    }

    /// Tear down and re-run the start flow at full feature — used after the
    /// user installs the missing dependency, and to reconnect a session the
    /// server ended. The probe re-runs during start, so a now-present tmux /
    /// mosh-server is picked up automatically.
    private func reconnect(still: Bool = false) {
        guard let active else { return }
        // A switch moves nothing; any other manual reconnect moves as a
        // reconnect does. Either way the new session is a reconnect when
        // this one was (or always, for a switch): the veil.
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
            if sshOnce {
                sshOnce = false
                connection.jumpMoshRoute = JumpMoshRoute.ssh.rawValue
            }
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
        let model = nshModel
        return ["k", "w"].map(Character.init) .map { key in
            HardwareKeys.Command(title: key == "k" ? "Show All Panes" : "Back to Home", key: key) {
                model.post(["op": "key", "stroke": "mod+\(key)"])
            }
        } + (1..<10).map { number in
            HardwareKeys.Command(title: "Window \(number)", key: Character(String(number))) {
                model.post(["op": "key", "stroke": "mod+\(number)"])
            }
        }
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

    // MARK: - Terminal body

    /// What the cover says. Once the transport is back the state reads
    /// `.live`, but the cover is still up for the first frame — it keeps
    /// announcing the phase it was covering, not "Opening the pit".
    private var coverState: TransportConnState {
        connState == .live ? lastCoverState : connState
    }

    /// The transport is up but there is nothing to uncover yet: tmux and
    /// herdr until their first frame is painted, everything else for the
    /// beat `settleGrace` holds. Never both — on a fast attach the first
    /// frame lands inside the grace period, and letting the grace outlive
    /// the signal put a cover up for the leftover ~170ms right as the
    /// frozen frame gave way to the new one (a flash in the crossfade,
    /// measured on the rig's short-suspension run).
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
    /// take the grace — except herdr over mosh on the Rust core, which
    /// waits for the renderer's first reveal: the raw attach replays
    /// fragments until the pane's program redraws, and a protocol switch
    /// keeps its last picture up over them.
    nonisolated static func waitsForFirstFrame(transport: ConnectionProtocol,
                                               multiplexer: Multiplexer,
                                               degraded: Bool,
                                               onRustCore: Bool) -> Bool {
        guard !degraded else { return false }
        // herdr over mosh on the Rust core: its first reveal, after the
        // first pane's redraw (a switch's picture stays until then).
        if transport == .mosh { return multiplexer == .herdr && onRustCore }
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
        guard let active else { return wasEverLive }
        return wasEverLive || active.retiredTmuxController != nil || Self.keepsFrame(active)
    }

    /// The session's own terminal still holds what it showed: herdr, mosh
    /// and plain SSH keep one terminal across a redial. A screen entered
    /// while such a session reconnects is a reconnect too (B657: herdr has
    /// no retired controller to say so, and a fresh screen took the first
    /// connect's path, a full-screen poster saying "Reconnecting…"). tmux's
    /// panes live in their controller; its ghost says it.
    static func keepsFrame(_ active: SessionHub.ActiveSession) -> Bool {
        guard active.tmuxController == nil, active.retiredTmuxController == nil,
              let surface = active.coordinator.surface else { return false }
        return TerminalNSHStage.hasText(surface)
    }

    /// The reconnect dim and status capsule — over the kept frame, the
    /// persistent pane, or the bare background; and also over a dropped line
    /// that is no longer being redialled (the frame is still what the person
    /// was reading).
    private var veilVisible: Bool { paneCover == .veil }

    /// What covers the pane now (``paneCover(resolved:reconnect:state:settling:)``).
    private var paneCover: PaneCover {
        Self.paneCover(resolved: active != nil, reconnect: isReconnect, state: connState, settling: settling)
    }

    /// What covers the pane: nothing (a live pane, and a first connect:
    /// B657, the owner's 「这种动画可以删掉了 不符合我们的产品了」), or a
    /// reconnect's veil.
    enum PaneCover: Equatable {
        case none, veil
    }

    /// The cover for a session `resolved` (or not yet: no session object,
    /// no cover — `active` is a `@State` set on appear, so every screen
    /// renders one frame without it; for a screen entering a live session
    /// it must never come, see `coverArmed`), coming back or not
    /// (`reconnect`), its transport in `state`, `settling` while the
    /// transport is up but nothing has painted. A reconnect — a protocol
    /// switch always is one — gets the veil through the whole cycle and
    /// over a line no longer redialled (`.offline`), until the replacement
    /// has painted. A first connect gets nothing: the pane is the terminal's
    /// background, and the bar's transport pill says it connects.
    nonisolated static func paneCover(resolved: Bool, reconnect: Bool,
                                      state: TransportConnState, settling: Bool) -> PaneCover {
        guard resolved else { return .none }
        if reconnect {
            switch state {
            case .connecting, .reconnecting, .offline: return .veil
            case .live: return settling ? .veil : .none
            }
        }
        return .none
    }

    /// Another Moshpit device holds the live window's size: the pane is
    /// frozen on its last frame under a veil that names the holder and
    /// offers a tap to take over. Only while nothing else covers the pane —
    /// a reconnect's own veil says what matters then.
    private var sizeFollow: RustTmuxController.SizeFollow? {
        guard connState == .live, !veilVisible,
              let controller = active?.tmuxController, active?.retiredTmuxController == nil
        else { return nil }
        return controller.sizeFollow
    }

    /// Another client picked another pane in the window shown: the phone
    /// keeps its own under a veil, a tap taking it back (B587). Under the
    /// same conditions as the size veil, which goes first.
    private var paneFollow: String? {
        guard connState == .live, !veilVisible,
              let controller = active?.tmuxController, active?.retiredTmuxController == nil
        else { return nil }
        return controller.paneFollow
    }

    // MARK: - Shortcut dispatch

    /// The coordinator receiving keyboard input right now (tmux active pane,
    /// or the single mosh/SSH coordinator).
    private var typingCoordinator: CoreTerminalHost.Coordinator? {
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
            // What is being composed goes first, then the paste (B573).
            typingCoordinator?.endComposition(cancel: false)
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
        // An input method composing: Esc takes it away, any other key
        // commits it first (B573). Committed as it stands — the pinyin's
        // letters when no candidate was picked — because that is what iOS
        // itself does when something interrupts a composition, and what
        // the people of other input methods expect kept (kana, Hangul,
        // Zhuyin with a pick are their text). Only Esc, which means "leave"
        // (vim's insert mode), throws it away.
        typingCoordinator?.endComposition(cancel: Self.cancelsComposition(stages))
        active?.sendShortcutInput(first)
        guard stages.count > 1 else { return }
        let rest = stages.dropFirst()
        Task {
            for stage in rest {
                try? await Task.sleep(for: Self.keySequenceGap)
                active?.sendShortcutInput(stage)
            }
        }
    }

    /// Whether a bar key that sends `stages` takes an input method's
    /// composition away rather than committing it (B573): Esc alone, which
    /// leaves vim's insert mode; leaving it with pinyin half typed inserted
    /// the pinyin first.
    nonisolated static func cancelsComposition(_ stages: [Data]) -> Bool {
        stages == [Data([0x1B])]
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
        // What is being composed goes first, then the arrow (B573).
        typingCoordinator?.endComposition(cancel: false)
        // Over herdr these bytes are intercepted in sendInput and rerouted as
        // SEMANTIC keys (`pane send-keys`) — the local DECCKM below is stale
        // there (frames never carry mode changes), and sendInput is the one
        // choke point both this D-pad and a hardware keyboard flow through.
        // SSH+tmux does the same one level down: the controller sends a lone
        // arrow as a tmux key name (crates/tmux, `send-keys` in input.rs).
        let appMode = active?.applicationCursorKeys ?? false
        let csi: [UInt8]
        switch direction {
        case "up":    csi = appMode ? [0x1B, 0x4F, 0x41] : [0x1B, 0x5B, 0x41]
        case "down":  csi = appMode ? [0x1B, 0x4F, 0x42] : [0x1B, 0x5B, 0x42]
        case "right": csi = appMode ? [0x1B, 0x4F, 0x43] : [0x1B, 0x5B, 0x43]
        case "left":  csi = appMode ? [0x1B, 0x4F, 0x44] : [0x1B, 0x5B, 0x44]
        default: return
        }
        active?.sendShortcutInput(Data(csi))
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
        var request = settings.dictationRequest
        request.hotwords = DictationHotwords.harvest(activePaneFacts)
        Task { await controller.start(request) }
    }

    /// What the pane being typed into is about, for the recognizer's
    /// hotwords: its screen as it stands, the agent and command in it, its
    /// working directory and the tmux session and window around it. Plain
    /// SSH has the screen alone.
    private var activePaneFacts: DictationPaneFacts {
        var facts = DictationPaneFacts()
        if let surface = typingCoordinator?.hostContainer?.surface {
            facts.screen = (0..<surface.rows.rows.count).map(surface.rowText)
        }
        if let control = active?.tmuxControl,
           let paneId = control.snapshot.activePaneId ?? control.snapshot.activePanes.first?.id {
            let pane = control.snapshot.panes[paneId]
            facts.agent = control.agentHooks[paneId]?.agent
            facts.command = pane?.displayCommand
            facts.path = pane?.path
            if let window = pane.flatMap({ control.snapshot.windows[$0.windowId] }) {
                facts.windowName = window.name
                facts.sessionName = control.snapshot.sessions[window.sessionId]?.name
            }
        }
        return facts
    }

    /// Stop the mic, wait for the engine's final words, and type the result
    /// into the live session. `sendPaste` (not raw bytes) so bracketed-paste
    /// apps — Claude Code prompts being the whole point — receive the
    /// dictation as one block. It is one line by now (the controller folds a
    /// spoken "new line" into a space, B672): a pane without bracketed paste
    /// would take a line break as Return.
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
            if let commit = await dictation.finish(submit: submit) {
                active?.sendPaste(commit.text)
                // Send presses Return only on the words that were read: a
                // final pass that changed them leaves the text to be read
                // and sent (B676).
                if commit.submit {
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

// MARK: - The roam banner's sparkline

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

// MARK: - The NSH page

/// What the screen draws through NSH (``TerminalNSHPage``). Everything that
/// is not drawn stays the screen's: the session, the keyboard intent, the
/// presentations, the lifecycle above.
extension TerminalScreen {
    /// mosh's counters while the page shows them (`{"op": "diagnostics"}`).
    private var nshDiagnostics: TerminalNSHDiagnostics {
        TerminalNSHDiagnostics(shown: showMoshDiagnostics && connection.dialsMosh,
                               counters: metrics?.moshDiagnostics)
    }

    private var nshPage: some View {
        TerminalNSHPage(
            model: nshModel,
            chrome: nshChrome,
            bar: nshBar,
            switcher: nshSwitcher,
            diagnostics: nshDiagnostics,
            dictation: dictation,
            attachment: imageAttachment,
            preview: active?.remotePreview,
            snapshots: (active?.tmuxControl as (any PaneSnapshotting)?) ?? active?.herdrControl,
            target: nshTarget,
            focus: focusPolicy,
            geometryHeld: geometryHold,
            appearance: TerminalNSHStage.Appearance(
                theme: theme, fontSize: settings.fontSize, fontName: settings.fontName,
                cursorShape: settings.cursorShape, cursorColorId: effectiveCursorColorId,
                cursorBlink: settings.cursorBlink),
            onPaste: { [weak active] text in active?.sendPaste(text) },
            onSelectPane: { pane in selectSwipedPane(pane) },
            perform: nshPerform,
            onFailure: {
                Log.ui.error("terminal: NSH did not start")
                nshFailed = true
            }
        ) {
            nshAnchors
        }
        // What the screen watches: the pane a deep link asked for, the
        // breadcrumb kept across a reconnect, a new session's first-live
        // bookkeeping.
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

    /// The session's panes a sideways swipe walks (`ui/src/terminal/
    /// switcher.rs`): the live controller's whole session, once the
    /// breadcrumb's plan is live. The page decides the order and the
    /// neighbours; the stage moves the terminal.
    private var nshSwitcher: TerminalNSHSwitcher {
        var switcher = TerminalNSHSwitcher(hintSeen: nshModel.hintSeen)
        guard livePlan != nil, let snapshot = tmuxSnapshot else { return switcher }
        switcher.tree = TerminalNSHSwitcher.tree(
            snapshot: snapshot, hooks: multiplexerAgentHooks,
            mux: connection.multiplexer == .herdr ? "herdr" : "tmux")
        return switcher
    }

    /// `act` on the session's multiplexer (tmux's controller, mosh's
    /// sidecar, herdr's client), if it has one.
    private func withMultiplexer(_ act: (any MultiplexerControlling) -> Void) {
        if let control = active?.tmuxControl {
            act(control)
        } else if let herdr = active?.herdrControl {
            act(herdr)
        }
    }

    /// A swipe's (or VoiceOver's step's) pane, the page's neighbour: the
    /// controller selects it, across windows (tmux's select-window first),
    /// herdr retargets its frame channel to it.
    private func selectSwipedPane(_ pane: String) {
        if let control = active?.tmuxControl {
            control.selectPane(pane)
        } else if let herdr = active?.herdrControl {
            herdr.selectPane(pane)
        }
    }

    /// What the chrome shows now.
    private var nshChrome: TerminalNSHChrome {
        var chrome = TerminalNSHChrome()
        chrome.name = connection.displayName
        chrome.user = connection.username
        chrome.host = connection.host
        chrome.port = connection.port
        chrome.mosh = connection.dialsMosh
        // Through a jump host, SSH only unless the connection picked a UDP
        // route for mosh (``ServerConnection/jumpMoshRoute``).
        chrome.jumpMosh = connection.dialsMoshThroughJump
        chrome.jump = connection.viaConnectionId != nil && !chrome.jumpMosh
        // The notice that mosh can use UDP: only while no route was ever
        // picked. A route set to SSH — the pill's switch back, the banner's
        // "this time only", the form — is the person's choice already.
        chrome.jumpSsh = chrome.jump && connection.connectionProtocol == .mosh
            && connection.jumpMoshRoute == nil
        chrome.mux = connection.multiplexer
        chrome.sidebar = onToggleSidebar != nil
        chrome.pill = transportKind
        chrome.state = connState
        if let plan = livePlan ?? retainedPlan {
            let words = connection.multiplexer.words
            chrome.crumbs = .init(plan: plan, sessionIcon: words.sessionIcon,
                                  windowIcon: words.windowIcon, live: livePlan != nil)
        }
        chrome.still = stillSwitch
        if veilVisible {
            chrome.cover = .init(kind: .veil, state: coverState,
                                 ended: active?.viewModel.endedByServer == true)
        } else if let follow = sizeFollow {
            chrome.cover = .init(kind: .follow, state: connState, owner: follow.ownerLabel, touches: true)
        } else if paneFollow != nil {
            chrome.cover = .init(kind: .paneFollow, state: connState, touches: true)
        }
        (chrome.content, chrome.empty) = nshContent
        if active?.moshReturnPathDead == true {
            let udp = AppSettings.shared.udpRange
            chrome.banner = .returnPath(
                typed: active?.moshTypedWaits == true,
                route: connection.dialsMoshThroughJump ? connection.jumpRoute.rawValue : nil,
                ports: "\(udp.low)–\(udp.high)")
        } else if metrics?.isRoaming == true {
            chrome.banner = .roam(srtt: metrics?.srttMs, history: metrics?.srttHistory ?? [])
        } else if let notice = active?.degrade {
            chrome.banner = .degrade(notice.missing)
        }
        let multiplexer = active?.tmuxControl?.notice ?? active?.herdrNotice
        if let text = Self.shownNotice(multiplexer: multiplexer.map(herdrNoticeText), flashed: flashNotice) {
            chrome.notice = .init(text: text, action: multiplexer != nil ? active?.herdrNoticeAction : nil)
        }
        chrome.link = copiedURL.map { $0.host() ?? $0.absoluteString }
        chrome.compact = compactChrome
        // The page's ground follows the grid's (B496).
        chrome.ground = nshModel.ground.map { String(format: "#%06X", $0) }
        return chrome
    }

    /// The shortcut bar.
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

    /// What the pane area holds.
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
            // Missing is the host's probe's answer: known. No sessions is
            // known once the core asked the server (has-session: B228, the
            // control client's exit: B786); until then the attach may only
            // be on its way. It waited out the grace whatever the core knew:
            // seconds of a blank terminal before the answer (B789).
            let missing = active.capabilities?.hasTmux == false
            let known = missing || active.moshServerHadNoSession
            return (.empty, .init(mux: .tmux, missing: missing, settled: known))
        }
        if let herdr = active.herdrControl, herdr.serverNotRunning {
            // The server answered that it isn't running: known.
            return (.empty, .init(mux: .herdr, missing: active.capabilities?.hasHerdr == false,
                                  settled: true))
        }
        // mosh + tmux on a server with no session (B228): the renderer on
        // screen waits for one in silence. The empty state with its Create
        // goes over it, as the SwiftUI terminal's overlay does; without it
        // the screen stayed black (B337). The server was asked already, so
        // the answer is known: it shows at once (settled), with nothing
        // before it.
        if active.awaitingFirstSession, connState == .live {
            return (.empty, .init(mux: .tmux, missing: false, settled: true))
        }
        return (.terminal, nil)
    }

    /// The terminal the slot shows.
    private var nshTarget: TerminalNSHStage.Target? {
        guard let active else { return nil }
        let ghost = active.retiredTmuxController
        if let controller = ghost ?? active.tmuxController {
            let snapshot = controller.snapshot
            guard ghost != nil || snapshot.isAttached,
                  let pane = snapshot.activePaneId ?? snapshot.activePanes.first?.id else { return nil }
            let frozen = ghost != nil
            return .init(source: .pane(controller, pane), inset: Self.nshPaneInset, frozen: frozen,
                         slide: snapshot.hasSwipedSinceAttach && !frozen ? snapshot.lastSwitchForward : nil)
        }
        if let herdr = active.herdrControl, herdr.serverNotRunning { return nil }
        return .init(source: .plain(active))
    }

    /// The room around the tmux pane the NSH page shows (B489): none. The
    /// SwiftUI chrome's 4 pt (2 in iPad landscape) frames panes side by
    /// side; the page shows one at a time, where it framed nothing and
    /// cost about a column each side. The terminal centres its grid in
    /// what it has (B473).
    nonisolated static let nshPaneInset: CGFloat = 0

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
                if let c = active?.tmuxControl { MuxPickerNSHPage(controller: c, kind: .windows) } else if let h = active?.herdrControl { MuxPickerNSHPage(controller: h, kind: .windows) }
            }
            .adaptivePresentation(isPresented: $showSessionsSheet) {
                if let c = active?.tmuxControl { MuxPickerNSHPage(controller: c, kind: .sessions) } else if let h = active?.herdrControl { MuxPickerNSHPage(controller: h, kind: .sessions) }
            }
            .adaptivePresentation(isPresented: $showPaneSheet) {
                if let c = active?.tmuxControl { MuxPickerNSHPage(controller: c, kind: .panes) } else if let h = active?.herdrControl { MuxPickerNSHPage(controller: h, kind: .panes) }
            }
            .position(x: crumb.midX, y: crumb.midY)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
    }

    /// What the NSH screen asked for.
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
            if connection.dialsMosh { showMoshDiagnostics = true }
        case .diagnosticsDismiss:
            showMoshDiagnostics = false
        case .repaint:
            // Ack state 0: the server rediffs from blank.
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
                if connection.dialsMoshThroughJump { reconnectOverSSHOnce() } else { switchProtocol() }
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
        case .jumpRoute(let route):
            guard active != nil else { return }
            stillSwitch = true
            holdStill()
            if let saved = connectionHolder.store.useMoshThroughJump(of: connection.id, route: route) {
                connection = saved
            }
            Haptics.select()
            reconnect(still: true)
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
        case .overview(let open):
            if open {
                if !nshOverviewOpen { presentSheet { nshOverviewOpen = true } }
            } else {
                nshOverviewOpen = false
            }
        case .selectPane(let pane):
            Haptics.select()
            selectSwipedPane(pane)
        case .newPane(let window):
            Haptics.tap()
            withMultiplexer { $0.newPane(inWindow: window) }
        case .renameWindow(let window, let name):
            withMultiplexer { $0.renameWindow(window, to: name) }
        case .killPane(let pane):
            withMultiplexer { $0.killPane(pane) }
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
    private(set) weak var coordinator: CoreTerminalHost.Coordinator?
    /// Whether it is armed, for the chip.
    var onChange: ((Bool) -> Void)?

    var isArmed: Bool { coordinator?.pendingCtrl == true }

    /// The next key typed into `coordinator` goes out as its control code.
    func arm(_ coordinator: CoreTerminalHost.Coordinator) {
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
