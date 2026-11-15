import MoshpitKit
import SwiftUI
import UIKit

/// A page Settings pushes on its own stack, for a row of the NSH root.
enum SettingsPage: String, Hashable, Identifiable {
    case accent
    case appIcon = "app-icon"
    case theme
    case voiceEngine = "voice-engine"
    case whisperModel = "whisper-model"
    case voiceLanguage = "voice-language"
    case recentLog = "recent-log"

    var id: String { rawValue }

    @ViewBuilder
    var page: some View {
        switch self {
        case .accent: AccentNSHPage()
        case .appIcon: AppIconNSHPage()
        case .theme: ThemeGalleryNSHPage()
        case .voiceEngine: VoiceEngineNSHPage()
        case .whisperModel: WhisperModelNSHPage()
        case .voiceLanguage: VoiceLanguageNSHPage()
        case .recentLog: RecentLogNSHPage()
        }
    }
}

/// A page of Settings the NSH hub opens (`ui/src/settings.rs`'s `Part`): the
/// same screen again, told which page in its params.
enum SettingsPart: String, Hashable, Identifiable, CaseIterable {
    case appearance, terminal, connections, notifications, voice
    /// Appearance › the companion's card (moshpit-design PET-DESIGN §15.6).
    case companion
    /// Set up a host's list of saved connections, pushed for the hub's row
    /// when there is not exactly one (``HostSetupRoute``).
    case hosts
    /// Pages a page of the hub's pushes (R18): Connections › UDP port
    /// range, Notifications › How notifications work.
    case udpRange = "udp-range"
    case notificationInfo = "notification-info"

    var id: String { rawValue }

    #if DEBUG
    /// The page a capture run has the NSH hub push once it is up
    /// (`-MOSHPIT_SEED_SETTINGS_PAGE terminal`, with `-MOSHPIT_SEED_SCREEN
    /// settings`); `-MOSHPIT_SEED_SCROLL` then scrolls that page.
    static var requested: SettingsPart? {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-MOSHPIT_SEED_SETTINGS_PAGE"), i + 1 < args.count else { return nil }
        guard let part = SettingsPart(rawValue: args[i + 1]) else {
            print("[capture] unknown -MOSHPIT_SEED_SETTINGS_PAGE '\(args[i + 1])'; expected one of: "
                  + allCases.map(\.rawValue).joined(separator: ", "))
            return nil
        }
        return part
    }
    #endif
}

/// Settings drawn by NSH (`ui/src/settings.rs`): the hub at the root of
/// Settings' stack, or one of the pages it opens (``SettingsPart``). What
/// it shows comes from the host (``SettingsModel``, from
/// ``SettingsSnapshot``: the settings, the stores' counts, the catalogs'
/// names), again whenever any of it changes; what the user changes comes back
/// and is applied to `AppSettings`.
/// The navigation stays SwiftUI's: a row asks for a page (`onOpen`) and
/// ``SettingsScreen`` pushes it, presents the sheet or opens the link. The
/// version line's Copy menu is UIKit's, on the surface (``NSHContextMenus``).
struct SettingsNSHPage: View {
    @Environment(AppSettings.self) private var settings
    @Environment(ShortcutStore.self) private var shortcutStore
    @Environment(SSHKeyStore.self) private var keyStore
    @Environment(ThemeStore.self) private var themes

    /// The page this is; nil: the hub.
    var part: SettingsPart? = nil
    /// A session is live: Set up this host can inspect it.
    var liveSession: Bool
    /// The storefront wants the APP filing shown.
    var showsFilingNumber: Bool
    /// A row asked for a page, sheet or link (`ui/src/settings.rs`'s `Page`).
    var onOpen: (String) -> Void
    var onDone: () -> Void
    /// The saved connections, for Set up a host's list (the `hosts` part).
    var hosts: [SettingsHost] = []
    /// A connection picked on that list.
    var onSetup: (UUID) -> Void = { _ in }
    /// The hub's screen couldn't run. A page of it that can't says so in
    /// its own place instead, under the system's back button, and its Try
    /// again runs that page again: it used to pop the stack and mark the
    /// hub failed, whose Try again brought back the hub.
    var onFailure: () -> Void = {}

    @Environment(\.dismiss) private var dismiss
    /// A page one of this page's rows pushed (Terminal's Theme…): pushed
    /// over this page, not from the stack's root.
    @State private var pushed: SettingsPage?
    /// A page of Settings one of this page's rows pushed (Connections' UDP
    /// range): the screen again, over this page, not from the stack's root.
    @State private var pushedPart: SettingsPart?
    @State private var model = SettingsModel()
    @State private var menus = NSHContextMenus()
    @State private var hostsFeed = SettingsHostsFeed()
    /// This page of the hub couldn't run (the hub's own goes to
    /// `onFailure`).
    @State private var failed = false

    var body: some View {
        if failed {
            NSHPageFailed { failed = false }
        } else {
            page
        }
    }

    @ViewBuilder
    private var page: some View {
        let snapshot = SettingsSnapshot(
            settings: settings, shortcuts: shortcutStore, keys: keyStore, themes: themes,
            liveSession: liveSession, showsFilingNumber: showsFilingNumber)
        NSHPage(
            screen: .settings,
            page: part?.rawValue,
            attach: { surface in
                model.attach(surface, current: snapshot)
                if part == .hosts { hostsFeed.attach(surface, hosts) }
                menus.menus = [SettingsModel.versionID: {
                    [UIAction(title: String(localized: "Copy")) { _ in
                        UIPasteboard.general.string = SettingsScreen.versionLine
                    }]
                }]
                menus.install(on: surface)
            },
            onEvent: { event in
                guard case .settings(let event) = event else { return }
                if case .setup(let id) = event {
                    if let id = UUID(uuidString: id) { onSetup(id) }
                    return
                }
                model.handle(event, settings: settings, onOpen: open, onDone: onDone,
                             onBack: { dismiss() })
            },
            onFailure: {
                if part == nil { onFailure() } else { failed = true }
            })
            .nshPage()
            .onAppear {
                model.update(snapshot)
                model.updatePet(CompanionPet.shared.message)
            }
            .onChange(of: snapshot) { _, new in model.update(new) }
            .onChange(of: CompanionPet.shared.revision) { _, _ in model.updatePet(CompanionPet.shared.message) }
            .onChange(of: hosts) { _, new in hostsFeed.update(new) }
            .navigationDestination(item: $pushed) { page in page.page }
            .navigationDestination(item: $pushedPart) { part in
                SettingsNSHPage(part: part, liveSession: liveSession, showsFilingNumber: showsFilingNumber,
                                onOpen: onOpen, onDone: onDone)
            }
    }

    /// What a row asked for: on a page of the hub, a page of Settings' is
    /// pushed over it here; the rest (the hub's own pages, sheets, links)
    /// goes to ``SettingsScreen``.
    private func open(_ page: String) {
        if part != nil, let pushed = SettingsPage(rawValue: page) {
            self.pushed = pushed
        } else if part != nil, let sub = SettingsPart(rawValue: page) {
            pushedPart = sub
        } else {
            onOpen(page)
        }
    }
}

/// Everything the NSH Settings page shows: the screen's own state
/// (`ui/src/settings.rs`'s `SettingsState`), as the page bridge carries it.
typealias SettingsSnapshot = SettingsPageState

extension SettingsPageState {
    @MainActor
    init(settings: AppSettings, shortcuts: ShortcutStore, keys: SSHKeyStore, themes: ThemeStore,
         liveSession: Bool, showsFilingNumber: Bool,
         shownIcon: () -> String? = { UIApplication.shared.alternateIconName }) {
        let accent = AppThemeCatalog.theme(for: settings.appThemeId)
        // The icon iOS shows, as the NSH App Icon page ticks it (the
        // stored setting can differ).
        let shown = shownIcon()
        let icon = AppIconCatalog.all.first { $0.alternateName == shown }
            ?? AppIconCatalog.option(for: settings.appIconId)
        let theme = themes.theme(id: settings.themeId)
        self.init(
            accent: SettingsSwatch(name: accent.name, colorHex: "#" + accent.accentHex),
            appIcon: SettingsIcon(id: icon.id, name: icon.name, image: AppIconModel.previewURI(icon)),
            fonts: TerminalFont.families.map { SettingsChoice(id: $0.id, name: $0.label) },
            font: settings.fontName,
            fontSize: settings.fontSize,
            theme: SettingsTerminalTheme(
                name: theme.name, backgroundHex: "#" + theme.backgroundHex,
                foregroundHex: "#" + theme.foregroundHex,
                swatchesHex: TerminalTheme.ANSISlot.allCases.map { "#" + theme.ansiHex($0) }),
            cursorShape: settings.cursorShape.rawValue,
            cursorColor: settings.cursorColorId,
            cursorBlink: settings.cursorBlink,
            trailOnPredict: settings.trailOnPredict,
            keyboardOnOpen: settings.raiseKeyboardOnOpen,
            remoteClipboardRead: settings.remoteClipboardReadEnabled,
            uploadRetentionDays: Int64(settings.uploadRetentionDays),
            moshByDefault: settings.moshByDefault,
            predictMode: settings.predictMode.rawValue,
            udpRange: PortRange(start: Int64(settings.udpRangeStart), end: Int64(settings.udpRangeEnd)),
            shortcuts: SettingsCount(count: Int64(shortcuts.toolbarCount), limit: Int64(ShortcutStore.toolbarLimit)),
            keys: Int64(keys.keys.count),
            notifications: settings.notificationsEnabled,
            liveActivity: settings.liveActivityEnabled,
            alertSound: settings.attentionSoundEnabled,
            lockScreenDetail: settings.lockScreenDetailEnabled,
            liveSession: liveSession,
            voiceInput: settings.voiceInputEnabled,
            voiceEngine: settings.voiceEngine.rawValue,
            voiceModel: WhisperModelStore.resolvedVariant(preferring: settings.whisperModelId)
                .map(WhisperModelStore.displayName(for:)),
            voiceLanguage: Self.languageName(settings),
            version: SettingsScreen.versionLine,
            filing: showsFilingNumber ? SettingsScreen.icpFilingNumber : nil,
            // The theme the app wears, as the screen's params carry one
            // (``NSHScreenParams``): the page's own colors follow a new pick.
            appTheme: SettingsAppTheme(
                accentHex: NSHScreenParams.hex(accent.accent),
                accentPressedHex: NSHScreenParams.hex(accent.accentPressed),
                screenBgHex: NSHScreenParams.hex(accent.screenBG),
                terminalBgHex: NSHScreenParams.hex(accent.terminalBG)),
            appearance: settings.appearance.rawValue)
    }

    /// What the dictation language is called, for the engine in use: ""
    /// reads as Automatic or Auto-detect.
    static func languageName(_ settings: AppSettings) -> String {
        if settings.voiceEngine == .whisper {
            return WhisperLanguageCatalog.displayName(for: settings.whisperLanguage)
        }
        let id = settings.voiceInputLocaleId
        guard !id.isEmpty else { return String(localized: "Automatic") }
        return Locale.current.localizedString(forIdentifier: id) ?? id
    }
}

/// What the NSH Settings page shows, owned by the host so a surface made again
/// starts from it (``NSHPage``'s third rule), and what the page's intents do.
@MainActor
final class SettingsModel {
    /// The version line's identifier on the screen (its Copy menu's part).
    nonisolated static let versionID = "settings-version"

    private(set) var snapshot: SettingsSnapshot?
    private weak var surface: NSHSurface?
    /// The companion's last message (`{"op": "pet"}`): Appearance's group
    /// and the pet's card.
    private var pet: String?

    /// `surface` shows the settings from now on, starting with the current
    /// ones.
    func attach(_ surface: NSHSurface) {
        self.surface = surface
        if let snapshot { surface.post(.settings(state: snapshot)) }
        if let pet { surface.post(json: pet) }
        #if DEBUG
        // Twice: once the page is up, and again once the native controls
        // have reported their sizes and the text settled, so the shot finds
        // the rows where the final layout has them.
        if let anchor = SettingsAnchor.requested {
            for delay in [0.9, 2.0] {
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak surface] in
                    surface?.post(.settingsReveal(anchor: anchor.reveal))
                }
            }
        }
        #endif
    }

    /// ``attach(_:)``, the model filled from `current` (the page's settings as
    /// its body has it now) while it has none (B366): the screen starts from
    /// the page's state whether or not `onAppear` came first. A state the
    /// model has is newer (`onChange` keeps it) and stays.
    func attach(_ surface: NSHSurface, current snapshot: SettingsSnapshot) {
        if self.snapshot == nil {
            self.surface = nil
            update(snapshot)
        }
        attach(surface)
    }

    /// The settings as they are now.
    func update(_ snapshot: SettingsSnapshot) {
        guard snapshot != self.snapshot else { return }
        self.snapshot = snapshot
        surface?.post(.settings(state: snapshot))
    }

    /// The companion as it is now.
    func updatePet(_ message: String) {
        guard message != pet else { return }
        pet = message
        surface?.post(json: message)
    }

    /// What the screen told the host.
    func handle(_ event: SettingsEvent, settings: AppSettings,
                onOpen: (String) -> Void, onDone: () -> Void, onBack: () -> Void = {}) {
        switch event {
        case .done:
            onDone()
        case .back:
            onBack()
        case .open(let page):
            onOpen(page)
        case .set(let change):
            if !Self.apply(change, to: settings) {
                Log.ui.error("settings: refused \(String(describing: change), privacy: .public)")
            }
        case .setup:
            break
        }
    }

    /// Applies `change` to `settings`. false for a value out of what the
    /// setting takes (a font this build doesn't ship, a range that isn't
    /// one): checked here again, as the page checked it.
    @discardableResult
    static func apply(_ change: SettingsChange, to settings: AppSettings) -> Bool {
        switch change {
        case .companion(let style, let shown):
            CompanionPet.shared.choose(style: style, shown: shown)
        case .petName(let name):
            CompanionPet.shared.rename(name)
        case .petTalk(let on):
            CompanionPet.shared.setTalk(on)
        case .font(let id):
            guard TerminalFont.families.contains(where: { $0.id == id }) else { return false }
            settings.fontName = id
        case .fontSize(let size):
            guard size.isFinite else { return false }
            settings.fontSize = min(max(size.rounded(), 8), 32)
        case .cursorShape(let id):
            guard let shape = CursorShape(rawValue: id) else { return false }
            settings.cursorShape = shape
        case .cursorColor(let id):
            guard ["teal", "green", "white", "accent"].contains(id) else { return false }
            settings.cursorColorId = id
        case .uploadRetentionDays(let days):
            guard [0, 1, 7].contains(days) else { return false }
            settings.uploadRetentionDays = Int(days)
        case .predictMode(let id):
            guard let mode = PredictMode(rawValue: id) else { return false }
            settings.predictMode = mode
        case .appearance(let id):
            guard let appearance = AppAppearance(rawValue: id) else { return false }
            settings.appearance = appearance
        case .udpRange(let range):
            // The UDP range's page checked it (`udp_ports`); checked again.
            guard case .success(let (start, end)) = UDPPortRange.validate(
                from: String(range.start), to: String(range.end)) else { return false }
            settings.udpRangeStart = start
            settings.udpRangeEnd = end
        case .cursorBlink(let on): settings.cursorBlink = on
        case .trailOnPredict(let on): settings.trailOnPredict = on
        case .keyboardOnOpen(let on): settings.raiseKeyboardOnOpen = on
        case .remoteClipboardRead(let on): settings.remoteClipboardReadEnabled = on
        case .moshByDefault(let on): settings.moshByDefault = on
        case .notifications(let on): settings.notificationsEnabled = on
        case .liveActivity(let on): settings.liveActivityEnabled = on
        case .alertSound(let on): settings.attentionSoundEnabled = on
        case .lockScreenDetail(let on): settings.lockScreenDetailEnabled = on
        case .voiceInput(let on): settings.voiceInputEnabled = on
        }
        return true
    }
}

/// Where a capture run has Settings scrolled to (`-MOSHPIT_SEED_SCROLL
/// <anchor>`): a group's first row at the top, or the end. The page reveals
/// it (``SettingsModel/attach(_:)``).
enum SettingsAnchor: String, CaseIterable {
    case cursor, behavior, mosh, notifications, bottom

    /// As the bridge carries it.
    var reveal: SettingsRevealAnchor {
        switch self {
        case .cursor: .cursor
        case .behavior: .behavior
        case .mosh: .mosh
        case .notifications: .notifications
        case .bottom: .bottom
        }
    }

    #if DEBUG
    static var requested: SettingsAnchor? {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-MOSHPIT_SEED_SCROLL"), i + 1 < args.count else { return nil }
        return SettingsAnchor(rawValue: args[i + 1])
    }
    #endif
}

// MARK: - Set up a host

/// A saved connection on Settings › Set up a host's list
/// (`ui/src/settings.rs`'s `HostEntry`).
struct SettingsHost: Equatable, Identifiable {
    let id: UUID
    /// Its name (its `user@host` when it has none).
    let name: String
    let user: String
    let host: String
    let port: Int
    /// A session of it is connected: its host can be set up now.
    let live: Bool

    init(id: UUID, name: String, user: String, host: String, port: Int, live: Bool) {
        self.id = id
        self.name = name
        self.user = user
        self.host = host
        self.port = port
        self.live = live
    }

    init(_ connection: ServerConnection, live: Bool) {
        self.init(id: connection.id, name: connection.displayName, user: connection.username,
                  host: connection.host, port: connection.port, live: live)
    }

    /// Set up a host's list for `hosts`, in their order.
    static func message(_ hosts: [SettingsHost]) -> PageMessage {
        .settingsHosts(hosts: hosts.map {
            SettingsHostEntry(id: $0.id.uuidString, name: $0.name, user: $0.user, host: $0.host,
                              port: Int64($0.port), live: $0.live)
        })
    }
}

/// Where Settings › Set up a host goes, from the hub's row or a pick on the
/// list. Setup runs over a connection's live session, so a connection that
/// isn't connected is offered Connect first.
enum HostSetupRoute: Equatable {
    /// The saved connections, to pick the one to set up.
    case list
    /// The connection's Host Setup.
    case setup(UUID)
    /// Ask to connect it first.
    case connect(UUID)

    /// The hub's row: straight in with exactly one saved connection, the
    /// list with none or several.
    static func entering(_ hosts: [SettingsHost]) -> HostSetupRoute {
        guard hosts.count == 1, let only = hosts.first else { return .list }
        return only.live ? .setup(only.id) : .connect(only.id)
    }

    /// A pick on the list; nil for a connection no longer saved.
    static func choosing(_ id: UUID, in hosts: [SettingsHost]) -> HostSetupRoute? {
        guard let host = hosts.first(where: { $0.id == id }) else { return nil }
        return host.live ? .setup(id) : .connect(id)
    }
}

/// What Set up a host's list was last told, posted again to a surface made
/// again, and as connections come and go.
@MainActor
final class SettingsHostsFeed {
    private weak var surface: NSHSurface?
    private var hosts: [SettingsHost]?

    func attach(_ surface: NSHSurface, _ hosts: [SettingsHost]) {
        self.surface = surface
        self.hosts = hosts
        _ = surface.post(SettingsHost.message(hosts))
    }

    func update(_ hosts: [SettingsHost]) {
        guard hosts != self.hosts else { return }
        self.hosts = hosts
        _ = surface?.post(SettingsHost.message(hosts))
    }
}
