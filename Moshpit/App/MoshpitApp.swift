import SwiftUI

@main
struct MoshpitApp: App {
    @State private var store: ConnectionStore
    @State private var settings: AppSettings
    @State private var metrics: SessionMetricsRegistry
    @State private var shortcuts = ShortcutStore()
    @State private var sshKeys = SSHKeyStore()
    @State private var themes = ThemeStore()
    @State private var appThemes = AppThemeStore.shared
    @State private var hub: SessionHub
    @State private var monitor: AgentActivityMonitor
    @State private var connectionHolder: ConnectionStoreHolder
    @State private var keychainHolder: KeychainServiceHolder
    @State private var router = DeepLinkRouter()
    @State private var notificationHandler = AgentNotificationHandler()
    @Environment(\.scenePhase) private var scenePhase

    private let keychain = KeychainService()

    init() {
        let metrics = SessionMetricsRegistry()
        let settings = AppSettings.shared
        let store = ConnectionStore()
        _metrics = State(initialValue: metrics)
        _settings = State(initialValue: settings)
        _store = State(initialValue: store)
        _hub = State(initialValue: SessionHub(metrics: metrics))
        _monitor = State(initialValue: AgentActivityMonitor(settings: settings))
        _connectionHolder = State(initialValue: ConnectionStoreHolder(store: store))
        _keychainHolder = State(initialValue: KeychainServiceHolder(service: KeychainService()))
    }

    var body: some Scene {
        WindowGroup {
            RootView(
                store: store,
                settings: settings,
                hub: hub,
                keychain: keychain,
                router: router
            )
            // Brand tint: system controls (toggles, pickers, alerts) render
            // in the active app theme's accent instead of iOS defaults.
            .tint(Ink.accent)
            // Force a full subtree rebuild on theme change. `Ink.accent` etc.
            // are plain computed properties (not tracked by Observation), so
            // views that don't otherwise read `settings` wouldn't repaint on
            // their own; `.id()` guarantees every screen re-evaluates fresh
            // with the new theme instead of only the ones that happen to.
            // `.transition(.opacity)` + the `withAnimation` around the
            // mutation (the accent picker) turn what would otherwise be an
            // instant hard cut into a cross-fade.
            //
            // The key includes the accent VALUE, not just the theme id: editing
            // a custom accent keeps its id, so an id-only key left the whole app
            // painted in the pre-edit color.
            .id(settings.appThemeId + AppThemeCatalog.current.accentHex)
            .transition(.opacity)
            .environment(settings)
            .environment(metrics)
            .environment(shortcuts)
            .environment(sshKeys)
            .environment(themes)
            .environment(appThemes)
            .environment(monitor)
            .environment(connectionHolder)
            .environment(keychainHolder)
            .environment(router)
            .task {
                // Vibe Island control surface — route lock-screen Allow/Deny/
                // Reply (notification actions + Live Activity buttons) and body
                // taps to the live pane via the hub. Set early so a launch-from-
                // action response (app was killed) still resolves once delivered.
                AgentControlBridge.shared.handler = { action, connectionId, paneId, text in
                    // Stale-prompt guard: Allow/Deny are blind keystrokes (Enter/
                    // Esc). If we KNOW the pane has left attention, the prompt this
                    // notification advertised is gone — tapping it could answer a
                    // newer, different question. Only an explicit "no longer
                    // attention" blocks; no record (app relaunched) passes through.
                    if action == .allow || action == .deny,
                       monitor.attentionState(connectionId: connectionId, paneId: paneId) == false {
                        AgentNotifications.postPromptExpired()
                        return
                    }
                    let delivered = await hub.deliverAgentInput(
                        action.bytes(text: text),
                        connectionId: connectionId, paneId: paneId)
                    // A lock-screen Allow/Deny that silently fails is the worst
                    // outcome — the user walks away believing they approved.
                    // Say so, in the same channel they acted from.
                    if !delivered {
                        AgentNotifications.postDeliveryFailure()
                    }
                }
                AgentControlBridge.shared.opener = { connectionId, paneId in
                    router.request(connectionId: connectionId, paneId: paneId)
                }
                AgentControlBridge.shared.cycler = { [weak monitor] in
                    monitor?.cycleHeadline()
                }
                AgentControlBridge.shared.drainShareQueue = { [weak hub] in
                    guard let hub else { return }
                    await ShareQueueDrainer.drain(hub: hub)
                }
                // "Is this exact pane on screen?" — lets willPresent skip the
                // banner + chime when the user is already looking at the prompt.
                AgentControlBridge.shared.isPaneVisible = { connectionId, paneId in
                    guard let session = hub.visibleSession,
                          session.connection.id == connectionId else { return false }
                    if let control = session.tmuxControl {
                        return control.snapshot.activePaneId == paneId
                    }
                    return true   // non-tmux: the session on screen IS the pane
                }
                AgentNotifications.configure(delegate: notificationHandler)
            }
            .onOpenURL { url in
                router.handle(url)
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
                    hub.setForeground(true)
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
        guard url.scheme == "moshpit", url.host == "connection",
              let id = UUID(uuidString: url.lastPathComponent) else { return }
        let pane = URLComponents(url: url, resolvingAgainstBaseURL: false)?
            .queryItems?.first(where: { $0.name == "pane" })?.value
        request(connectionId: id, paneId: pane)
    }
}

/// Routes between the home screen and the smoke-test direct-terminal path.
struct RootView: View {
    let store: ConnectionStore
    let settings: AppSettings
    let hub: SessionHub
    let keychain: KeychainService
    let router: DeepLinkRouter

    /// Read from the environment (rather than passed in) because only the
    /// DEBUG reset seam below needs it; the theme-aware screens resolve their
    /// own copy the same way.
    @Environment(ThemeStore.self) private var themes

    @State private var autoSeededConnection: ServerConnection?

    var body: some View {
        Group {
            // E2E smoke arg — seeds a localhost connection from a base64 PEM
            // and jumps straight to the Terminal screen. Used by
            // `scripts/smoke-localhost.sh` for self-driven verification.
            if let conn = autoSeededConnection {
                NavigationStack {
                    TerminalScreen(connection: conn, hub: hub)
                }
            } else {
                AttachHomeView(store: store, hub: hub, keychain: keychain)
            }
        }
        .task {
            // DEBUG-only: the reset + smoke-seed launch args wipe the connection
            // store and plant a non-biometric private key. That data-wiping /
            // key-planting behavior must never ship in a Release build, so it is
            // compiled out entirely — the smoke-test scripts build Debug.
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
        guard
            let user = launchArg("-MOSHPIT_SEED_USER"),
            let keyB64 = launchArg("-MOSHPIT_SEED_KEY_B64"),
            let keyData = Data(base64Encoded: keyB64),
            let pem = String(data: keyData, encoding: .utf8),
            autoSeededConnection == nil
        else { return }

        let host = launchArg("-MOSHPIT_SEED_HOST") ?? "127.0.0.1"
        let port = Int(launchArg("-MOSHPIT_SEED_PORT") ?? "22") ?? 22

        let id = UUID()
        let ref = id.uuidString
        do {
            try await keychain.savePrivateKey(pem, forRef: ref, requireBiometry: false)
        } catch {
            print("[smoke seed] keychain save failed: \(error)")
            return
        }

        let useMosh = launchArg("-MOSHPIT_SEED_MOSH") == "1"
        let connection = ServerConnection(
            id: id,
            // `-MOSHPIT_SEED_NAME` names the seeded connection. Verification
            // runs keep the default; marketing captures pass a host name a
            // person would actually have, because "smoke-localhost" on a
            // screenshot says "we photographed our test rig".
            name: launchArg("-MOSHPIT_SEED_NAME") ?? "smoke-localhost",
            host: host,
            port: port,
            username: user,
            authMethod: .key,
            connectionProtocol: useMosh ? .mosh : .ssh,
            sshPort: port,
            moshServerPath: launchArg("-MOSHPIT_SEED_MOSH_BIN"),
            keychainRef: ref,
            // `-MOSHPIT_SEED_TMUX 1` still works (existing e2e scripts pass
            // it); `-MOSHPIT_SEED_MUX none|tmux|herdr` picks explicitly and
            // wins when both are present.
            useTmux: launchArg("-MOSHPIT_SEED_TMUX") == "1",
            tmuxPath: launchArg("-MOSHPIT_SEED_TMUX_BIN"),
            herdrPath: launchArg("-MOSHPIT_SEED_HERDR_BIN"),
            multiplexerRaw: launchArg("-MOSHPIT_SEED_MUX")
        )

        for existing in store.connections {
            store.delete(id: existing.id)
        }
        store.add(connection)
        // `-MOSHPIT_SEED_QUIET 1` silences local notifications for a capture
        // run. Simulator banners don't auto-dismiss the way a device's do, so
        // a "needs you" alert sits over the nav bar and ruins every shot after
        // it — and the Vibe Island has its own coverage elsewhere.
        if launchArg("-MOSHPIT_SEED_QUIET") == "1" {
            settings.notificationsEnabled = false
        }
        // `-MOSHPIT_SEED_HOME 1` seeds the connection but stays on the Attach
        // home screen (instead of auto-pushing the terminal), so the home
        // connection-card flow can be exercised/screenshotted.
        if launchArg("-MOSHPIT_SEED_HOME") != "1" {
            autoSeededConnection = connection
        }
    }

    private func launchArg(_ flag: String) -> String? {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: flag), i + 1 < args.count else { return nil }
        return args[i + 1]
    }
    #endif
}
