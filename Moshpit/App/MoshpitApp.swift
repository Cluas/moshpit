import SwiftUI
import MoshpitKit

@main
struct MoshpitApp: App {
    @State private var store: ConnectionStore
    @State private var settings: AppSettings
    @State private var metrics: SessionMetricsRegistry
    // Every store on the Rust core's store (AppStores).
    @State private var shortcuts = AppStores.shortcuts()
    @State private var sshKeys = AppStores.sshKeys()
    @State private var themes = AppStores.themes()
    @State private var appThemes = AppThemeStore.shared
    @State private var hub: SessionHub
    @State private var monitor: AgentActivityMonitor
    @State private var connectionHolder: ConnectionStoreHolder
    @State private var keychainHolder: KeychainServiceHolder
    @State private var router: DeepLinkRouter
    @State private var notificationHandler: AgentNotificationHandler
    /// Lets the Whisper model go when voice input stops using it, or memory
    /// runs short. Held here only to live as long as the app.
    @State private var whisperResidency: WhisperModelResidency
    /// Remote-push tokens are delivered to a UIApplicationDelegate and nowhere
    /// else — SwiftUI has no equivalent hook, so the app keeps a minimal one
    /// whose only job is forwarding them to PushRegistration.
    @UIApplicationDelegateAdaptor(PushAppDelegate.self) private var pushDelegate
    @Environment(\.scenePhase) private var scenePhase

    private let keychain = KeychainService()

    init() {
        // The device's protected-data state, made here on the main thread
        // so the stores that ask it from their own threads (the
        // sealed-launch rule) never wait on the main queue for it.
        _ = ProtectedDataFlag.app
        // The push pairings through the Rust store, before anything below
        // reads one.
        AppStores.installPushPairings()
        let metrics = SessionMetricsRegistry()
        let settings = AppSettings.shared
        let store = AppStores.connections()
        // Host care pairs only connections this store holds — not one being
        // deleted, or already gone, when a pass comes back from the host —
        // and reaches a host through its session in the hub.
        HostCare.shared.connectionStore = store
        let hub = SessionHub(metrics: metrics)
        HostCare.shared.hub = hub
        // The language Moshpit shows, for the island, the widget and the
        // notification service extension to write in (AppLanguage). iOS
        // relaunches the app whenever it changes, so launch is enough —
        // unless the launch came before the first unlock, when the App
        // Group can't be written safely; then the next time the app comes
        // forward (reloadStoresIfNeeded).
        AppLanguage.recordIfTrusted()
        let monitor = AgentActivityMonitor(settings: settings)
        monitor.savedConnections = { [weak store] in store?.savedIds }
        let router = DeepLinkRouter()
        let notificationHandler = AgentNotificationHandler()
        _metrics = State(initialValue: metrics)
        _settings = State(initialValue: settings)
        _store = State(initialValue: store)
        _hub = State(initialValue: hub)
        _monitor = State(initialValue: monitor)
        _router = State(initialValue: router)
        _notificationHandler = State(initialValue: notificationHandler)
        _whisperResidency = State(initialValue: WhisperModelResidency(settings: settings))
        _connectionHolder = State(initialValue: ConnectionStoreHolder(store: store))
        _keychainHolder = State(initialValue: KeychainServiceHolder(service: KeychainService()))

        // Everything iOS can call into before a view exists.
        //
        // All of this used to live in a `.task` on the root view, and every piece
        // of it had the same defect: iOS launches this app to the BACKGROUND to
        // run a notification action or a LiveActivityIntent, and a WindowGroup's
        // content may never mount there — so the task never ran, the bridge
        // handler stayed nil, and `await handler?(...)` did nothing at all. A
        // lock-screen button produced no keystroke and no error. It looked dead
        // because it was.
        //
        // The notification DELEGATE has it worse: set late, a cold launch from a
        // tapped action can reach iOS's callback before there is anything to
        // receive it.
        //
        // `init()` is the only point before every reader — the same conclusion
        // -MOSHPIT_RESET above arrived at, for the same reason.
        Self.wireSystemEntryPoints(hub: hub, monitor: monitor, router: router,
                                   notificationHandler: notificationHandler)
        Chrome.installAppearance()
    }

    /// Static so it cannot accidentally capture `self` — an `App` struct is a
    /// value, and a closure that outlives init() holding a copy of it would be
    /// wiring the system to a snapshot.
    @MainActor
    private static func wireSystemEntryPoints(
        hub: SessionHub, monitor: AgentActivityMonitor, router: DeepLinkRouter,
        notificationHandler: AgentNotificationHandler
    ) {
        // No control handler. The bridge's `handler` used to turn a lock-screen
        // Allow/Deny into a keystroke — through a live session if one happened to
        // be in memory, otherwise sealed to the relay for the host to press. Both
        // halves are gone with the buttons that fed them.
        //
        // `opener` stays, and it is the whole interaction now: a tapped
        // notification opens the pane that wants you.
        AgentControlBridge.shared.opener = { connectionId, paneId in
            router.request(connectionId: connectionId, paneId: paneId)
        }
        AgentControlBridge.shared.drainShareQueue = { [weak hub] in
            guard let hub else { return }
            await ShareQueueDrainer.drain(hub: hub)
        }
        // "Is this exact pane on screen?" — lets willPresent skip the
        // banner + chime when the user is already looking at the prompt.
        AgentControlBridge.shared.isPaneVisible = { connectionId, paneId in
            PaneOnScreen.shows(hub.visibleSession, foreground: hub.isForeground,
                               connectionId: connectionId, paneId: paneId)
        }
        // The same, as the agent machine's sweep asks it: which pane.
        AgentControlBridge.shared.visiblePane = {
            PaneOnScreen.of(hub.visibleSession, foreground: hub.isForeground)
        }
        // A pairing self-test push proves the whole chain works; the
        // sheet that asked for it is waiting on this hop.
        AgentControlBridge.shared.pushSelfTest = { nonce in
            Log.push.info("self-test push arrived: \(nonce, privacy: .public)")
            HostCare.shared.selfTestArrived(nonce: nonce)
        }
        AgentNotifications.configure(delegate: notificationHandler)
        // Ask iOS for a device token when at least one host is paired.
        // No-op otherwise: an unpaired install must not prompt, and has
        // nothing to register.
        HostCare.shared.registerForPush()
    }

    /// Every UserDefaults-backed store re-reads if what it loaded could not be
    /// trusted (a no-op for the ordinary launch). Cheap, so it runs on every
    /// return to the foreground rather than trying to predict the one that
    /// follows an unattended launch.
    private func reloadStoresIfNeeded() {
        AppLanguage.recordIfTrusted()
        store.reloadIfNeeded()
        shortcuts.reloadIfNeeded()
        sshKeys.reloadIfNeeded()
        themes.reloadIfNeeded()
        appThemes.reloadIfNeeded()
        settings.reloadIfNeeded()
        // The agents' announced record a sealed launch couldn't read (B348).
        monitor.reloadIfNeeded()
        // Pairings left by connections deleted before a delete took its
        // pairing along: forgotten once per launch, the first time the saved
        // connections can be trusted (before `.active` asks the relay to
        // mint). A launch before the first unlock can't trust them, and
        // sweeps nothing until it can.
        HostCare.shared.forgetOrphanedPairings()
        // This install's id, if a launch before the first unlock couldn't
        // read it (B443).
        HostCare.shared.syncInstallId()
        // What sessions remembered about connections deleted before a
        // delete took it along (B199): swept once per launch, by the same
        // rule — only once the saved connections can be told.
        let swept = store.sweepSessionMemoryOnce()
        if !swept.isEmpty {
            Log.store.notice("forgot \(swept.count, privacy: .public) session-memory keys of connections no longer saved")
        }
        // Items connections let go of that an earlier launch couldn't drop
        // (B428: its session still read one when the app ended, or the
        // keychain refused) go first, once both stores can be trusted —
        // only what was listed, only what nothing names now. Then private
        // keys earlier builds left in the keychain with no record naming
        // them (B232) — a deleted key's, or one whose record a lost key list
        // took with it: listed in SSH Keys again, once. Recovery deletes
        // nothing, and never offers a listed item back.
        Task { [store, sshKeys, keychain] in
            KeychainReferences.settleReleasesOnce(connections: store, keys: sshKeys, keychain: keychain)
            await KeychainReferences.recoverOrphansOnce(connections: store, keys: sshKeys, keychain: keychain)
        }
    }

    var body: some Scene {
        WindowGroup {
            #if DEBUG
            // Screenshot seam for the host-setup screen, which cannot otherwise
            // be reached in a simulator: it requires a live SSH session, so
            // Settings disables its row and the sheet renders nothing. Compiled
            // out of Release entirely, like -MOSHPIT_RESET.
            if let scenario = HostSetupDemo.scenario(from: ProcessInfo.processInfo.arguments) {
                HostSetupDemo.view(scenario)
                    .environment(settings)
                    .tint(Ink.accent)
            } else if let screen = CaptureScreen.requested, [.hostSetup, .hostSetupNSH].contains(screen) {
                // The same seam for a capture run, with no setting written.
                HostSetupDemo.view(HostSetupDemo.seededScenario)
                    .environment(settings)
                    .tint(Ink.accent)
            } else {
                mainScene
            }
            #else
            mainScene
            #endif
        }
        // Hardware-keyboard chords for the terminal live on the scene, not in
        // the screen: menu commands fire whether or not anything is focused.
        .commands { TerminalCommands() }
    }

    @ViewBuilder
    private var mainScene: some View {
        Group {
            RootView(
                store: store,
                settings: settings,
                hub: hub,
                keychain: keychain
            )
            // The accent: system controls in it, and every view following
            // it without the root being made again (B293).
            .appThemeRoot()
            // One bundle, injected here and re-injected at every sheet /
            // cover / popover boundary (see AppEnvironment.swift for why).
            .appEnvironment(AppEnvironment(
                settings: settings, metrics: metrics, shortcuts: shortcuts,
                sshKeys: sshKeys, themes: themes, appThemes: appThemes,
                monitor: monitor, connectionHolder: connectionHolder,
                keychainHolder: keychainHolder, router: router))
            .onOpenURL { url in
                router.handle(url)
            }
            // The stores loaded in init(), which may have run before the first
            // unlock after a reboot (prewarming, a push, a Live Activity intent
            // — see DefaultsReadiness). Read again the moment the file can be
            // trusted, and again whenever the app comes forward.
            .onReceive(NotificationCenter.default.publisher(
                for: UIApplication.protectedDataDidBecomeAvailableNotification)) { _ in
                reloadStoresIfNeeded()
            }
            .onChange(of: scenePhase) { _, phase in
                // iOS kills TCP during suspension; run a foreground keepalive
                // while active, and on return force a reconnect if we were away
                // long enough. Handle phases EXPLICITLY: the return path is
                // .background → .inactive → .active, and treating .inactive as
                // "backgrounded" would reset the away-timer to ~0 and defeat the
                // long-background force-reconnect. .inactive (app switcher,
                // notification shade) is transient — ignore it.
                switch phase {
                case .active:
                    reloadStoresIfNeeded()
                    hub.setForeground(true)
                    // Cards whose questions ended while we were away (B433).
                    monitor.sweepDelivered()
                    // Re-announce this phone to its relays. Unconditional: a
                    // relay that lost its registry would otherwise never hear
                    // from us again, and every push it was asked to send would
                    // 401 with nothing anywhere saying why.
                    HostCare.shared.foreground()
                    // Images queued from the share sheet while we were away:
                    // deliver the ones whose sessions are live, leave the
                    // rest for the next drain (session start re-drains too).
                    Task { await ShareQueueDrainer.drain(hub: hub) }
                case .background: hub.setForeground(false)
                case .inactive:   break
                @unknown default: break
                }
            }
        }
    }
}

/// Which pane of which connection is on screen, as the notifications ask
/// it: the banner and chime skipped for the pane being looked at, and the
/// agent machine's sweep acknowledging its card (B407). The pane is the
/// focused one of whichever multiplexer the visible session runs: tmux's
/// in-band `-CC` controller (SSH), its sidecar (mosh), or herdr's client
/// (either transport, Swift or Rust path). It read `tmuxControl`, which is
/// nil for herdr: every herdr pane on the connection counted as on screen,
/// so another pane's prompt came without a banner and its card was cleared
/// within a sweep.
///
/// Nothing is on screen while the app isn't in the foreground (B438). The
/// terminal stays presented when the phone locks or the person switches
/// away, and for the seconds the app runs on after that, the sweep took its
/// pane for the one being looked at: a prompt there was "acknowledged" —
/// its card taken down about two seconds after it rang, the push's copy with
/// it — on a locked phone.
@MainActor
enum PaneOnScreen {
    /// The visible session's connection and pane: `""` when its
    /// multiplexer focuses none yet (no card is the one on screen), `nil`
    /// when it runs none (the session on screen is the pane). Nothing when
    /// the app isn't in the `foreground`.
    static func of(_ session: SessionHub.ActiveSession?, foreground: Bool) -> (UUID, String?)? {
        guard foreground, let session else { return nil }
        guard let tree = multiplexerTree(of: session) else { return (session.connection.id, nil) }
        return (session.connection.id, tree.activePaneId ?? "")
    }

    /// Whether `paneId` of `connectionId` is the pane on screen.
    static func shows(_ session: SessionHub.ActiveSession?, foreground: Bool,
                      connectionId: UUID, paneId: String) -> Bool {
        guard let (connection, pane) = of(session, foreground: foreground),
              connection == connectionId else { return false }
        return pane.map { $0 == paneId } ?? true
    }

    /// In ``TerminalScreen``'s order (its `tmuxSnapshot`).
    private static func multiplexerTree(of session: SessionHub.ActiveSession) -> TmuxSnapshot? {
        session.tmuxController?.snapshot ?? session.moshControl?.snapshot ?? session.herdrControl?.snapshot
    }
}

/// moshpit://connection/<uuid>?pane=<paneId> — Vibe Island taps and "need your
/// input" notification taps jump back to the session, and (when they name the
/// most-urgent agent's pane) land on that exact pane rather than just the
/// connection.
@Observable
final class DeepLinkRouter {
    /// One request to land on `connectionId` (and, if named, `paneId`). Carries
    /// a monotonic `token` so two requests for the very same pane are still
    /// distinguishable — e.g. a second "need your input" notification for a
    /// pane the user is already viewing must still re-fire `TerminalScreen`'s
    /// reaction, which plain value-equality on (connectionId, paneId) would
    /// hide (SwiftUI's `.onChange` only fires when the observed value differs).
    struct PaneRequest: Equatable {
        var connectionId: UUID
        var paneId: String?
        var token: Int
    }

    private(set) var paneRequest: PaneRequest?
    private var nextToken = 0

    func request(connectionId: UUID, paneId: String?) {
        nextToken += 1
        paneRequest = PaneRequest(connectionId: connectionId, paneId: paneId, token: nextToken)
        Log.island.info("land: request #\(self.nextToken, privacy: .public) conn=\(connectionId.uuidString, privacy: .public) pane=\(paneId ?? "-", privacy: .public)")
    }

    /// Consumes the request so it can't be replayed — e.g. against a LATER,
    /// unrelated manual navigation to the same connection (a fresh
    /// `TerminalScreen` for a plain card tap has no memory of which token it
    /// already handled, so a request left sitting here forever would look
    /// "new" to it too). Called by whichever `TerminalScreen` claims it,
    /// immediately on observing it — not gated on successfully landing on the
    /// pane, since a stale/closed pane id must not linger either.
    func clear(token: Int) {
        if paneRequest?.token == token { paneRequest = nil }
    }

    func handle(_ url: URL) {
        guard let link = AgentDeepLink.parse(url) else { return }
        request(connectionId: link.connectionId, paneId: link.paneId)
    }
}

/// Routes between the home screen and the smoke-test direct-terminal path.
struct RootView: View {
    let store: ConnectionStore
    let settings: AppSettings
    let hub: SessionHub
    let keychain: KeychainService

    /// Read from the environment (rather than passed in) because only the
    /// DEBUG reset seam below needs it; the theme-aware screens resolve their
    /// own copy the same way.
    @Environment(ThemeStore.self) private var themes
    @Environment(ShortcutStore.self) private var shortcuts

    @State private var autoSeededConnection: ServerConnection?
    #if DEBUG
    /// The smoke seed's connection, for this launch only (B200): Home shows
    /// it in place of the saved connections, which stay as they are.
    @State private var smokeSeedStore: ConnectionStore?
    #endif

    var body: some View {
        Group {
            // E2E smoke arg — seeds a localhost connection from a base64 PEM
            // and jumps straight to the Terminal screen. Used by
            // `scripts/verify/smoke-localhost.sh` for self-driven verification.
            if let conn = autoSeededConnection {
                NavigationStack {
                    TerminalScreen(connection: conn, hub: hub)
                }
            } else {
                #if DEBUG
                // `-MOSHPIT_SEED_HOME_FIXTURE`: Home shows a seeded set of
                // connections for this launch, written nowhere (HomeSeed);
                // so does the smoke seed with `-MOSHPIT_SEED_HOME 1`.
                HomeView(store: smokeSeedStore ?? HomeSeed.store ?? store, hub: hub, keychain: keychain)
                #else
                HomeView(store: store, hub: hub, keychain: keychain)
                #endif
            }
        }
        .task {
            // DEBUG-only, compiled out of Release entirely — the smoke-test
            // scripts build Debug. The reset wipes the connection store and
            // the appearance settings (it says so); the smoke seed writes
            // nothing that outlives the launch.
            #if DEBUG
            // Test-only: start from an empty store so UI tests are deterministic.
            if ProcessInfo.processInfo.arguments.contains("-MOSHPIT_RESET") {
                for c in store.connections { store.delete(id: c.id) }
                // Custom themes and the theme selection live in UserDefaults,
                // which outlives an app reinstall in the simulator — a UI test
                // asserting on a fresh install would otherwise inherit the
                // previous run's themes and selection.
                themes.removeAllCustom()
                AppThemeStore.shared.removeAllCustom()
                shortcuts.restoreDefaults()
                settings.appThemeId = AppThemeCatalog.signalRoom.id
                settings.appIconId = AppIconCatalog.primary.id
                settings.themeId = TerminalTheme.fallback.id
            }
            await applySmokeSeedIfRequested()
            #endif
        }
    }

    #if DEBUG
    @MainActor
    private func applySmokeSeedIfRequested() async {
        // W14a's rig: a connection for this launch only, written nowhere.
        if autoSeededConnection == nil, let seeded = await RustSessionSeed.connectionFromLaunchArguments() {
            autoSeededConnection = seeded
            return
        }
        guard autoSeededConnection == nil,
              let seed = await SmokeSeed.prepare(arguments: ProcessInfo.processInfo.arguments)
        else { return }
        smokeSeedStore = seed.store
        if seed.opensTerminal { autoSeededConnection = seed.connection }
    }

    #endif
}

#if DEBUG
/// The smoke seed (`-MOSHPIT_SEED_USER u -MOSHPIT_SEED_KEY_B64 <key>` and the
/// flags below): one connection to a test host, for this launch only. Used by
/// `scripts/verify/smoke-localhost.sh` and the other self-driven checks.
///
/// It used to delete every saved connection, add its own to the store, plant
/// its key in the keychain, trust the host key for good and turn
/// notifications off in the settings: a debug build run on a phone in daily
/// use took the person's connections with it (B200). Now the connection is in
/// a store of its own that is never written
/// (``HomeSeed/launchOnlyStore(_:suite:)``), its key in the connect path's
/// launch-only secrets (keychain ref `smoke.seed.<id>`, never read), the host
/// key trusted for this launch (``LaunchTrust``), and `-MOSHPIT_SEED_QUIET`
/// silences this launch only (``AppSettings/quietForThisLaunch``).
@MainActor
enum SmokeSeed {
    struct Seed {
        let connection: ServerConnection
        /// Home's store for this launch: the seeded connection alone.
        let store: ConnectionStore
        /// Straight to the terminal, unless `-MOSHPIT_SEED_HOME 1` asks to
        /// stay on Home (for the connection-card flow).
        let opensTerminal: Bool
    }

    /// The seed `arguments` ask for, made ready to connect; nil when they
    /// ask for none.
    static func prepare(arguments: [String]) async -> Seed? {
        func arg(_ flag: String) -> String? {
            guard let i = arguments.firstIndex(of: flag), i + 1 < arguments.count else { return nil }
            return arguments[i + 1]
        }
        guard let user = arg("-MOSHPIT_SEED_USER"),
              let keyB64 = arg("-MOSHPIT_SEED_KEY_B64"),
              let keyData = Data(base64Encoded: keyB64),
              let pem = String(data: keyData, encoding: .utf8)
        else { return nil }

        let host = arg("-MOSHPIT_SEED_HOST") ?? "127.0.0.1"
        let port = Int(arg("-MOSHPIT_SEED_PORT") ?? "22") ?? 22
        // `-MOSHPIT_SEED_TRUST_FP SHA256:…` pre-answers the New Host card, for
        // rigs with no way to tap it (Xcode 27 ships no Simulator.app, so
        // idb's HID path is gone).
        if let fingerprint = arg("-MOSHPIT_SEED_TRUST_FP") {
            LaunchTrust.trust(host: host, port: port, fingerprint: fingerprint)
        }

        // `-MOSHPIT_SEED_ID` fixes the connection's id so a harness can address
        // it afterwards — `moshpit://connection/<id>?pane=%N` is the only way to
        // drive a window switch on a real device, where XCUITest (the only thing
        // that can tap) cannot be signed for the test runner's bundle id.
        // Random when absent, as before.
        let id = arg("-MOSHPIT_SEED_ID").flatMap(UUID.init(uuidString:)) ?? UUID()
        await SSHService.shared.seedSecretForThisLaunch(connectionId: id, secret: pem)
        // What an earlier seeded run's sessions remembered about this id.
        RustSessionSeed.forget(id)

        let useMosh = arg("-MOSHPIT_SEED_MOSH") == "1"
        let connection = ServerConnection(
            id: id,
            // `-MOSHPIT_SEED_NAME` names the seeded connection. Verification
            // runs keep the default; marketing captures pass a host name a
            // person would actually have, because "smoke-localhost" on a
            // screenshot says "we photographed our test rig".
            name: arg("-MOSHPIT_SEED_NAME") ?? "smoke-localhost",
            host: host,
            port: port,
            username: user,
            authMethod: .key,
            connectionProtocol: useMosh ? .mosh : .ssh,
            sshPort: port,
            moshServerPath: arg("-MOSHPIT_SEED_MOSH_BIN"),
            keychainRef: "smoke.seed.\(id.uuidString)",
            // `-MOSHPIT_SEED_TMUX 1` still works (existing e2e scripts pass
            // it); `-MOSHPIT_SEED_MUX none|tmux|herdr` picks explicitly and
            // wins when both are present.
            useTmux: arg("-MOSHPIT_SEED_TMUX") == "1",
            tmuxPath: arg("-MOSHPIT_SEED_TMUX_BIN"),
            herdrPath: arg("-MOSHPIT_SEED_HERDR_BIN"),
            multiplexerRaw: arg("-MOSHPIT_SEED_MUX")
        )
        // `-MOSHPIT_SEED_QUIET 1` silences local notifications for a capture
        // run (``AppSettings/quietForThisLaunch``). Simulator banners don't
        // auto-dismiss the way a device's do, so a "needs you" alert sits over
        // the nav bar and ruins every shot after it — and the Vibe Island has
        // its own coverage elsewhere.
        return Seed(connection: connection,
                    store: HomeSeed.launchOnlyStore([connection], suite: "moshpit.smoke-seed"),
                    opensTerminal: arg("-MOSHPIT_SEED_HOME") != "1")
    }
}
#endif
