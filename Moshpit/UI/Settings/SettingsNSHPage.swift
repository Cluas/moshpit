import MoshpitKit
import SwiftUI
import UIKit

/// A page Settings pushes on its own stack, for a row of the NSH root: each
/// is the page its own switch picks (the NSH one or the SwiftUI one), so the
/// NSH pages chain from the NSH root as they do from the SwiftUI one.
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
        case .accent: AccentPage()
        case .appIcon: AppIconPage()
        case .theme: ThemePage()
        case .voiceEngine: VoiceEnginePage()
        case .whisperModel: WhisperModelPage()
        case .voiceLanguage: VoiceLanguagePage()
        case .recentLog: RecentLogPage()
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
    /// settings-nsh`); `-MOSHPIT_SEED_SCROLL` then scrolls that page.
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
/// and is applied to `AppSettings` as the SwiftUI page's bindings apply it.
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
    var onFailure: () -> Void

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

    var body: some View {
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
            onMessage: { message in
                if message["ev"] as? String == "setup" {
                    if let id = (message["id"] as? String).flatMap(UUID.init(uuidString:)) { onSetup(id) }
                    return
                }
                model.handle(message, settings: settings, onOpen: open, onDone: onDone,
                             onBack: { dismiss() })
            },
            onFailure: onFailure)
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
                                onOpen: onOpen, onDone: onDone, onFailure: onFailure)
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

/// Everything the NSH Settings page shows, as the SwiftUI page reads it.
struct SettingsSnapshot: Equatable {
    var accentName: String
    var accentHex: String
    var icon: AppIconOption
    var font: String
    var fontSize: Double
    var themeName: String
    /// The scheme's background and words' colors (`#RRGGBB`): the Terminal
    /// page's preview draws in them, as the terminal does.
    var themeBackground: String
    var themeForeground: String
    var themeSwatches: [String]
    var cursorShape: CursorShape
    var cursorColor: String
    var cursorBlink: Bool
    var trailOnPredict: Bool
    var keyboardOnOpen: Bool
    var remoteClipboardRead: Bool
    var uploadRetentionDays: Int
    var moshByDefault: Bool
    var predictMode: PredictMode
    var udpRange: [Int]
    var shortcuts: Int
    var keys: Int
    var notifications: Bool
    var liveActivity: Bool
    var alertSound: Bool
    var lockScreenDetail: Bool
    var liveSession: Bool
    var voiceInput: Bool
    var voiceEngine: VoiceEngineKind
    var voiceModel: String?
    var voiceLanguage: String
    var filing: String?
    /// The appearance chosen (Settings › Appearance, R18; its control shows
    /// once R18's host part is in).
    var appearance: AppAppearance
    /// The theme the app wears, as the screen's params carry one
    /// (``NSHScreenParams``): the page's own colors follow a new pick.
    var appTheme: [String: String]

    @MainActor
    init(settings: AppSettings, shortcuts: ShortcutStore, keys: SSHKeyStore, themes: ThemeStore,
         liveSession: Bool, showsFilingNumber: Bool,
         shownIcon: () -> String? = { UIApplication.shared.alternateIconName }) {
        let accent = AppThemeCatalog.theme(for: settings.appThemeId)
        accentName = accent.name
        accentHex = "#" + accent.accentHex
        appTheme = [
            "accent": NSHScreenParams.hex(accent.accent),
            "accentPressed": NSHScreenParams.hex(accent.accentPressed),
            "screenBG": NSHScreenParams.hex(accent.screenBG),
            "terminalBG": NSHScreenParams.hex(accent.terminalBG),
        ]
        // The icon iOS shows, as the NSH App Icon page ticks it (the SwiftUI
        // row names the stored setting, which can differ).
        let shown = shownIcon()
        icon = AppIconCatalog.all.first { $0.alternateName == shown }
            ?? AppIconCatalog.option(for: settings.appIconId)
        font = settings.fontName
        fontSize = settings.fontSize
        let theme = themes.theme(id: settings.themeId)
        themeName = theme.name
        themeBackground = "#" + theme.backgroundHex
        themeForeground = "#" + theme.foregroundHex
        themeSwatches = TerminalTheme.ANSISlot.allCases.map { "#" + theme.ansiHex($0) }
        cursorShape = settings.cursorShape
        cursorColor = settings.cursorColorId
        cursorBlink = settings.cursorBlink
        trailOnPredict = settings.trailOnPredict
        keyboardOnOpen = settings.raiseKeyboardOnOpen
        remoteClipboardRead = settings.remoteClipboardReadEnabled
        uploadRetentionDays = settings.uploadRetentionDays
        moshByDefault = settings.moshByDefault
        predictMode = settings.predictMode
        udpRange = [settings.udpRangeStart, settings.udpRangeEnd]
        self.shortcuts = shortcuts.toolbarCount
        self.keys = keys.keys.count
        notifications = settings.notificationsEnabled
        liveActivity = settings.liveActivityEnabled
        alertSound = settings.attentionSoundEnabled
        lockScreenDetail = settings.lockScreenDetailEnabled
        self.liveSession = liveSession
        voiceInput = settings.voiceInputEnabled
        voiceEngine = settings.voiceEngine
        voiceModel = WhisperModelStore.resolvedVariant(preferring: settings.whisperModelId)
            .map(WhisperModelStore.displayName(for:))
        voiceLanguage = Self.languageName(settings)
        filing = showsFilingNumber ? SettingsScreen.icpFilingNumber : nil
        appearance = settings.appearance
    }

    /// What the dictation language is called, for the engine in use: ""
    /// reads as Automatic or Auto-detect (the SwiftUI page's
    /// `voiceLanguageName`).
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
/// starts from it (``NSHPage``'s third rule), the message that carries it
/// (`{"op": "settings", …}`, `ui/src/settings.rs`'s `Message`), and what the
/// page's intents do.
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
        if let snapshot { surface.post(Self.message(snapshot)) }
        if let pet { surface.post(json: pet) }
        #if DEBUG
        // As the SwiftUI page scrolls (``SettingsAnchor/scroll(_:)``), and
        // again once the native controls have reported their sizes and the
        // text settled, so the shot finds the rows where the final layout has
        // them.
        if let anchor = SettingsAnchor.requested {
            for delay in [0.9, 2.0] {
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak surface] in
                    surface?.post(["op": "reveal", "anchor": anchor.rawValue])
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
        surface?.post(Self.message(snapshot))
    }

    /// The companion as it is now.
    func updatePet(_ message: String) {
        guard message != pet else { return }
        pet = message
        surface?.post(json: message)
    }

    /// What the screen told the host.
    func handle(_ message: [String: Any], settings: AppSettings,
                onOpen: (String) -> Void, onDone: () -> Void, onBack: () -> Void = {}) {
        switch message["ev"] as? String {
        case "done":
            onDone()
        case "back":
            onBack()
        case "open":
            if let page = message["page"] as? String { onOpen(page) }
        case "set":
            guard let key = message["key"] as? String,
                  Self.apply(key, message["value"], to: settings)
            else {
                return Log.ui.error("settings: no setting \(String(describing: message["key"]), privacy: .public)")
            }
        default:
            break
        }
    }

    /// Applies setting `key` (`ui/src/settings.rs`'s `Setting`) to
    /// `settings`, as the SwiftUI page's controls set them. false for a key
    /// or value it does not know.
    @discardableResult
    static func apply(_ key: String, _ value: Any?, to settings: AppSettings) -> Bool {
        let flag = value as? Bool
        switch key {
        case "companion":
            guard let choice = value as? [String: Any], let style = choice["style"] as? String,
                  let shown = choice["shown"] as? Bool else { return false }
            CompanionPet.shared.choose(style: style, shown: shown)
        case "petName":
            guard let name = value as? String else { return false }
            CompanionPet.shared.rename(name)
        case "font":
            guard let id = value as? String, TerminalFont.families.contains(where: { $0.id == id }) else { return false }
            settings.fontName = id
        case "fontSize":
            guard let size = (value as? NSNumber)?.doubleValue, size.isFinite else { return false }
            settings.fontSize = min(max(size.rounded(), 8), 32)
        case "cursorShape":
            guard let shape = (value as? String).flatMap(CursorShape.init(rawValue:)) else { return false }
            settings.cursorShape = shape
        case "cursorColor":
            guard let id = value as? String, ["teal", "green", "white", "accent"].contains(id) else { return false }
            settings.cursorColorId = id
        case "uploadRetentionDays":
            guard let days = (value as? NSNumber)?.intValue, [0, 1, 7].contains(days) else { return false }
            settings.uploadRetentionDays = days
        case "predictMode":
            guard let mode = (value as? String).flatMap(PredictMode.init(rawValue:)) else { return false }
            settings.predictMode = mode
        case "appearance":
            guard let appearance = (value as? String).flatMap(AppAppearance.init(rawValue:)) else { return false }
            settings.appearance = appearance
        case "udpRange":
            // The UDP range's page checked it (`udp_ports`); checked again,
            // as the SwiftUI editor checks it.
            guard let range = value as? [Any], range.count == 2,
                  let from = (range[0] as? NSNumber)?.intValue, let to = (range[1] as? NSNumber)?.intValue,
                  case .success(let (start, end)) = UDPPortRange.validate(from: String(from), to: String(to))
            else { return false }
            settings.udpRangeStart = start
            settings.udpRangeEnd = end
        default:
            guard let flag, let setting = Self.flags[key] else { return false }
            settings[keyPath: setting] = flag
        }
        return true
    }

    /// The switches, by their keys.
    static let flags: [String: ReferenceWritableKeyPath<AppSettings, Bool>] = [
        "cursorBlink": \.cursorBlink,
        "trailOnPredict": \.trailOnPredict,
        "keyboardOnOpen": \.raiseKeyboardOnOpen,
        "remoteClipboardRead": \.remoteClipboardReadEnabled,
        "moshByDefault": \.moshByDefault,
        "notifications": \.notificationsEnabled,
        "liveActivity": \.liveActivityEnabled,
        "alertSound": \.attentionSoundEnabled,
        "lockScreenDetail": \.lockScreenDetailEnabled,
        "voiceInput": \.voiceInputEnabled,
    ]

    /// The screen's message for `snapshot`.
    nonisolated static func message(_ s: SettingsSnapshot) -> [String: Any] {
        var message: [String: Any] = [
            "op": "settings",
            "accent": ["name": s.accentName, "color": s.accentHex],
            "appIcon": AppIconModel.icon(s.icon),
            "fonts": TerminalFont.families.map { ["id": $0.id, "name": $0.label] },
            "font": s.font,
            "fontSize": s.fontSize,
            "theme": ["name": s.themeName, "background": s.themeBackground,
                      "foreground": s.themeForeground, "swatches": s.themeSwatches],
            "cursorShape": s.cursorShape.rawValue,
            "cursorColor": s.cursorColor,
            "cursorBlink": s.cursorBlink,
            "trailOnPredict": s.trailOnPredict,
            "keyboardOnOpen": s.keyboardOnOpen,
            "remoteClipboardRead": s.remoteClipboardRead,
            "uploadRetentionDays": s.uploadRetentionDays,
            "moshByDefault": s.moshByDefault,
            "predictMode": s.predictMode.rawValue,
            "udpRange": s.udpRange,
            "shortcuts": ["count": s.shortcuts, "limit": ShortcutStore.toolbarLimit],
            "keys": s.keys,
            "notifications": s.notifications,
            "liveActivity": s.liveActivity,
            "alertSound": s.alertSound,
            "lockScreenDetail": s.lockScreenDetail,
            "liveSession": s.liveSession,
            "voiceInput": s.voiceInput,
            "voiceEngine": s.voiceEngine.rawValue,
            "voiceLanguage": s.voiceLanguage,
            "version": SettingsScreen.versionLine,
            "appTheme": s.appTheme,
            "appearance": s.appearance.rawValue,
        ]
        if let model = s.voiceModel { message["voiceModel"] = model }
        if let filing = s.filing { message["filing"] = filing }
        return message
    }
}

/// Where a capture run has Settings scrolled to (`-MOSHPIT_SEED_SCROLL
/// <anchor>`), on the SwiftUI page and the NSH one alike, so both are
/// pictured at the same rows: a group's first row at the top, or the end.
enum SettingsAnchor: String, CaseIterable {
    case cursor, behavior, mosh, notifications, bottom

    #if DEBUG
    static var requested: SettingsAnchor? {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-MOSHPIT_SEED_SCROLL"), i + 1 < args.count else { return nil }
        return SettingsAnchor(rawValue: args[i + 1])
    }

    /// The SwiftUI page's half: scrolls its list to the anchor asked for, a
    /// moment after it appears (the NSH page's is ``SettingsModel/attach(_:)``).
    @MainActor
    static func scroll(_ proxy: ScrollViewProxy) async {
        guard let anchor = requested else { return }
        for delay in [900, 1100] {
            try? await Task.sleep(for: .milliseconds(delay))
            proxy.scrollTo(anchor, anchor: anchor == .bottom ? .bottom : .top)
        }
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

    /// `{"op": "hosts", …}` for `hosts`, in their order.
    static func message(_ hosts: [SettingsHost]) -> [String: Any] {
        ["op": "hosts",
         "hosts": hosts.map {
             ["id": $0.id.uuidString, "name": $0.name, "user": $0.user, "host": $0.host,
              "port": $0.port, "live": $0.live] as [String: Any]
         }]
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
