import MoshpitKit
import SwiftUI
import UIKit

/// The screens moved from SwiftUI to NSH (`ui/` in the Rust workspace), each
/// behind a runtime switch of its own (docs/nsh/DESIGN.md §3.6): the SwiftUI
/// page stays the default until the NSH one has shipped, and turning the
/// switch off brings the SwiftUI page back.
enum NSHScreen: String, CaseIterable, Sendable {
    /// Settings › Recent Log (`RecentLogNSHPage`).
    case recentLog = "recent-log"
    /// Settings › App Icon (`AppIconNSHPage`).
    case appIcon = "app-icon"
    /// Settings › Accent (`AccentNSHPage`).
    case accent
    /// Settings › Voice Input › Recognition (`VoiceEngineNSHPage`).
    case voiceEngine = "voice-engine"
    /// Settings › Voice Input › Language (`VoiceLanguageNSHPage`).
    case voiceLanguage = "voice-language"
    /// Settings itself, the root of its stack (`SettingsNSHPage`).
    case settings
    /// Settings › Send Feedback (`FeedbackNSHPage`).
    case feedback
    /// Settings › Voice Input › Model (`WhisperModelNSHPage`).
    case whisperModel = "whisper-model"
    /// Settings › Theme (`ThemeGalleryNSHPage`).
    case theme
    /// A custom theme's editor (`ThemeEditorNSHPage`), from either Theme
    /// page.
    case themeEditor = "theme-editor"
    /// A custom accent's editor (`AccentEditorNSHPage`), from either Accent
    /// page.
    case accentEditor = "accent-editor"
    /// Home, the app's first screen (`HomeNSHPage`).
    case home
    /// Home's Add Connection and Edit Connection form
    /// (`ConnectionFormNSHPage`).
    case connectionForm = "connection-form"
    /// Settings › SSH Keys (`SSHKeysNSHPage`).
    case sshKeys = "ssh-keys"
    /// SSH Keys › Add Key, also from Select Key (`AddKeyNSHPage`).
    case addKey = "add-key"
    /// Add Connection › Select Key (`KeyPickerNSHPage`).
    case keyPicker = "key-picker"
    /// Settings › Shortcuts (`ShortcutsNSHPage`).
    case shortcuts
    /// Shortcuts › Add Shortcut or a custom shortcut's editor
    /// (`AddShortcutNSHPage`).
    case addShortcut = "add-shortcut"
    /// Settings › Set up this host (`HostSetupNSHPage`).
    case hostSetup = "host-setup"
    /// A connection's terminal (`TerminalNSHPage`): the chrome around the
    /// session's terminal view.
    case terminal
    /// The terminal's windows, sessions or panes (`MuxPickerNSHPage`), from
    /// the breadcrumb and ⌘K. Drawn by NSH whenever the terminal is.
    case muxPicker = "mux-picker"
    /// The terminal's Install Assist (`InstallAssistNSHPage`). Drawn by NSH
    /// whenever the terminal is.
    case installAssist = "install-assist"
    /// A folder of the host, from a path link or the Files key
    /// (`RemoteFolderNSHSheet`). Drawn by NSH whenever the terminal is.
    case remoteFolder = "remote-folder"
    /// Home › a host's sessions (`ui/src/sessions`): pushed from the NSH
    /// Home, which has no SwiftUI page of its own to fall back to.
    case sessions

    /// The switch: a Bool in `UserDefaults.standard`, off by default, set on
    /// the hidden Core Switches page (Settings: tap the version line seven
    /// times), or for one launch with the launch argument
    /// `-MOSHPIT_NSH_RECENT_LOG 1` — launch arguments land in the argument
    /// domain, which beats the stored value. Read each time the page opens.
    var switchKey: String {
        switch self {
        case .recentLog: "MOSHPIT_NSH_RECENT_LOG"
        case .appIcon: "MOSHPIT_NSH_APP_ICON"
        case .accent: "MOSHPIT_NSH_ACCENT"
        case .voiceEngine: "MOSHPIT_NSH_VOICE_ENGINE"
        case .voiceLanguage: "MOSHPIT_NSH_VOICE_LANGUAGE"
        case .settings: "MOSHPIT_NSH_SETTINGS"
        case .feedback: "MOSHPIT_NSH_FEEDBACK"
        case .whisperModel: "MOSHPIT_NSH_WHISPER_MODEL"
        case .theme: "MOSHPIT_NSH_THEME"
        case .themeEditor: "MOSHPIT_NSH_THEME_EDITOR"
        case .accentEditor: "MOSHPIT_NSH_ACCENT_EDITOR"
        case .home: "MOSHPIT_NSH_HOME"
        case .connectionForm: "MOSHPIT_NSH_CONNECTION_FORM"
        case .sshKeys: "MOSHPIT_NSH_SSH_KEYS"
        case .addKey: "MOSHPIT_NSH_ADD_KEY"
        case .keyPicker: "MOSHPIT_NSH_KEY_PICKER"
        case .shortcuts: "MOSHPIT_NSH_SHORTCUTS"
        case .addShortcut: "MOSHPIT_NSH_ADD_SHORTCUT"
        case .hostSetup: "MOSHPIT_NSH_HOST_SETUP"
        case .terminal: "MOSHPIT_NSH_TERMINAL"
        case .muxPicker: "MOSHPIT_NSH_MUX_PICKER"
        case .installAssist: "MOSHPIT_NSH_INSTALL_ASSIST"
        case .remoteFolder: "MOSHPIT_NSH_REMOTE_FOLDER"
        case .sessions: "MOSHPIT_NSH_SESSIONS"
        }
    }

    /// What the Core Switches page calls its switch.
    var switchTitle: String {
        switch self {
        case .recentLog: "Recent Log in NSH"
        case .appIcon: "App Icon in NSH"
        case .accent: "Accent in NSH"
        case .voiceEngine: "Voice Recognition in NSH"
        case .voiceLanguage: "Voice Language in NSH"
        case .settings: "Settings in NSH"
        case .feedback: "Feedback in NSH"
        case .whisperModel: "Whisper Models in NSH"
        case .theme: "Theme in NSH"
        case .themeEditor: "Theme Editor in NSH"
        case .accentEditor: "Accent Editor in NSH"
        case .home: "Home in NSH (with Rust sessions)"
        case .connectionForm: "Connection Form in NSH"
        case .sshKeys: "SSH Keys in NSH"
        case .addKey: "Add Key in NSH"
        case .keyPicker: "Key Picker in NSH"
        case .shortcuts: "Shortcuts in NSH"
        case .addShortcut: "Shortcut Editor in NSH"
        case .hostSetup: "Host Setup in NSH"
        case .terminal: "Terminal in NSH"
        case .muxPicker: "Window Pickers in NSH"
        case .installAssist: "Install Assist in NSH"
        case .remoteFolder: "Host Folders in NSH"
        case .sessions: "Sessions in NSH"
        }
    }

    /// The page the switch moves, as the Core Switches page says it.
    var page: String {
        switch self {
        case .recentLog: "Settings › Recent Log"
        case .appIcon: "Settings › App Icon"
        case .accent: "Settings › Accent"
        case .voiceEngine: "Settings › Voice Input › Recognition"
        case .voiceLanguage: "Settings › Voice Input › Language"
        case .settings: "Settings"
        case .feedback: "Settings › Send Feedback"
        case .whisperModel: "Settings › Voice Input › Model"
        case .theme: "Settings › Theme"
        case .themeEditor: "Settings › Theme › a theme's editor"
        case .accentEditor: "Settings › Accent › an accent's editor"
        case .home: "Home"
        case .connectionForm: "Home › Add Connection / Edit Connection"
        case .sshKeys: "Settings › SSH Keys"
        case .addKey: "SSH Keys › Add Key"
        case .keyPicker: "Add Connection › Select Key"
        case .shortcuts: "Settings › Shortcuts"
        case .addShortcut: "Shortcuts › Add or Edit Shortcut"
        case .hostSetup: "Settings › Set up this host"
        case .terminal: "A connection's terminal (read when it opens)"
        case .muxPicker: "Terminal › Windows / Sessions / Select Pane (always with the NSH terminal)"
        case .installAssist: "Terminal › Install Assist (always with the NSH terminal)"
        case .remoteFolder: "Terminal › a host folder (always with the NSH terminal)"
        case .sessions: "Home › a host's sessions (always with the NSH Home)"
        }
    }

    /// The name the Rust side runs it by (`Screen::named`).
    var name: String { rawValue }

    /// Whether the NSH page shows: the switch is on and this build can host
    /// NSH screens. With the switch off nothing of NSH runs.
    var isOn: Bool {
        Self.chosen(switchOn: UserDefaults.standard.bool(forKey: switchKey),
                    available: { NSHSurface.available })
    }

    /// The switch's choice: `available` is only asked when the switch is on.
    static func chosen(switchOn: Bool, available: () -> Bool) -> Bool {
        switchOn && available()
    }

    /// The switch is set for this launch by its launch argument.
    var setByLaunchArgument: Bool {
        UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)[switchKey] != nil
    }
}

/// What every screen is told before it runs: the language the app shows and
/// the colors of the theme the user picked (`ui/src/screen.rs`'s `Params`).
enum NSHScreenParams {
    /// The language the app shows: the localization the app itself picked,
    /// which the system's list of languages may not lead with.
    static var locale: String { Bundle.main.preferredLocalizations.first ?? "en" }

    /// The params now; `barHeight` is the navigation bar's the screen's page
    /// stands in for, as ``NSHSurface/navigationBarHeight`` measured it, and
    /// `backTitle` the page under it (``NSHSurface/backTitle``).
    @MainActor
    static func current(barHeight: CGFloat? = nil, backTitle: String? = nil) -> [String: Any] {
        let theme = AppThemeCatalog.current
        var params: [String: Any] = [
            "locale": locale,
            "theme": [
                "accent": hex(theme.accent),
                "accentPressed": hex(theme.accentPressed),
                "screenBG": hex(theme.screenBG),
                "terminalBG": hex(theme.terminalBG),
            ],
            // Settings › Appearance (R18): what the screens that follow the
            // system take. Until R18's host part the hosts force their
            // windows dark, so the system's (the default) shows dark.
            "appearance": AppSettings.shared.appearance.rawValue,
        ]
        if let barHeight {
            params["bar"] = ["height": barHeight]
        }
        if let backTitle {
            params["back"] = backTitle
        }
        return params
    }

    /// What a screen's colors come from: the theme picked and its accent
    /// (a custom accent edited keeps its id). Read in a view's body, it
    /// follows them (Observation): ``NSHPage`` runs its screen again when
    /// it changes.
    @MainActor
    static var themeKey: String {
        let theme = AppThemeCatalog.current
        return theme.id + hex(theme.accent) + AppSettings.shared.appearance.rawValue
    }

    /// `color` as `#RRGGBB` (sRGB, opaque: a theme's colors are).
    static func hex(_ color: Color) -> String {
        var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
        UIColor(color).getRed(&red, green: &green, blue: &blue, alpha: &alpha)
        let byte = { (c: CGFloat) in Int((min(max(c, 0), 1) * 255).rounded()) }
        return String(format: "#%02X%02X%02X", byte(red), byte(green), byte(blue))
    }
}

/// An NSH screen as a page pushed in a SwiftUI `NavigationStack`, under
/// spike G5's three rules (docs/nsh/DESIGN.md §3.6):
///
/// 1. It ignores every safe area, the keyboard's included (`nshPage()`): the
///    screen lays out the status bar, the home indicator and the keyboard
///    itself. SwiftUI avoiding the keyboard too would lift the screen's
///    bottom a second keyboard height.
/// 2. SwiftUI's navigation bar is hidden (`nshPage()`): the screen draws its
///    own (`ui/src/nav_bar.rs`) and tells the host when its back button is
///    pressed (`{"ev": "back"}`), which pops the stack.
/// 3. A rebuild hands over: what the screen shows lives in the host's model,
///    not in the surface, so a surface SwiftUI makes again (a new identity,
///    the page pushed again) starts from the model's state (`attach`), and a
///    screen that holds the keyboard (the terminal's) tells the host before
///    NSH rebuilds its view. The Recent Log page holds no first responder.
///
/// The screen runs once the surface is in a window: then the navigation bar
/// it stands in for can be measured (56 pt tall in a sheet, 50 or 44 pt
/// elsewhere), and its own bar takes that size. When no surface can be made
/// or the screen will not run (`onFailure`), the page shows its SwiftUI one
/// instead.
///
/// A screen is told the accent's colors when it runs, and a running one
/// can't be told new ones; so when the accent changes (``NSHScreenParams/themeKey``)
/// the page runs its screen again on a new surface, which starts from the
/// host's model as rule 3 has it. Nothing above the page is made again
/// (B293).
struct NSHPage: View {
    let screen: NSHScreen
    /// Which page of a screen of several (`ui/src/screen.rs`'s
    /// `Params.page`: Settings' pages); nil: its first.
    var page: String? = nil
    /// What this screen alone is told when it runs, beside the app's
    /// params (the terminal: its terminal theme's background). A running
    /// screen can't be told new ones either: when they change, so must
    /// `paramsKey`, and the page runs its screen again, as for the accent.
    var params: [String: Any] = [:]
    var paramsKey: String = ""
    var prepare: (NSHSurface) -> Void = { _ in }
    var attach: (NSHSurface) -> Void = { _ in }
    var onMessage: ([String: Any]) -> Void = { _ in }
    var onFailure: () -> Void = {}

    var body: some View {
        NSHPageSurface(screen: screen, page: page, params: params, prepare: prepare, attach: attach,
                       onMessage: onMessage, onFailure: onFailure)
            .id(NSHScreenParams.themeKey + paramsKey)
    }
}

/// ``NSHPage``'s surface, for one theme.
private struct NSHPageSurface: UIViewRepresentable {
    let screen: NSHScreen
    var page: String?
    /// ``NSHPage/params``.
    var params: [String: Any] = [:]
    /// The surface was made, the screen does not run yet: register the
    /// page's own native components.
    /// The surface was made and the screen is about to run on it: register
    /// the page's own native components (those bound to its model, such as
    /// a field whose text must stay in the host).
    var prepare: (NSHSurface) -> Void = { _ in }
    /// The surface was made and runs the screen: post it the current state.
    /// The latest one the page gave is called (``Coordinator``), so it can
    /// hand the model the state the body has now.
    var attach: (NSHSurface) -> Void = { _ in }
    /// What the screen tells the host, on the main thread.
    var onMessage: ([String: Any]) -> Void = { _ in }
    /// No surface could be made, or the screen would not run.
    var onFailure: () -> Void = {}

    /// The page's latest `attach`, its state as the page's body last had
    /// it: a screen run (or run again) starts from the page's current state
    /// whatever `onAppear` did or didn't do before it (B366).
    final class Coordinator {
        var attach: (NSHSurface) -> Void

        init(attach: @escaping (NSHSurface) -> Void) {
            self.attach = attach
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(attach: attach)
    }

    func makeUIView(context: Context) -> UIView {
        guard let surface = NSHSurface.make() else {
            DispatchQueue.main.async(execute: onFailure)
            return UIView()
        }
        surface.onMessage = onMessage
        prepare(surface)
        let (screen, page, extra, onFailure) = (screen, page, params, onFailure)
        let coordinator = context.coordinator
        let start: (NSHSurface) -> Void = { surface in
            let barHeight = surface.navigationBarHeight
            var params = NSHScreenParams.current(barHeight: barHeight, backTitle: surface.backTitle)
                .merging(extra) { _, own in own }
            if let page { params["page"] = page }
            guard surface.run(screen: screen.name, params: params) else {
                NSHDiagnostics.error("NSH screen \(screen.name) did not run")
                // Not inside UIKit's window callback, where SwiftUI state
                // must not change.
                return DispatchQueue.main.async(execute: onFailure)
            }
            Log.ui.info("NSH screen \(screen.name, privacy: .public) runs, bar \(barHeight.map { "\($0) pt" } ?? "standard", privacy: .public)")
            coordinator.attach(surface)
            NSHWatch.watch(surface)
        }
        surface.onWindow = start
        // Retry under a body that failed: the screen again, on this surface,
        // from the page's state (B366).
        surface.onRetry = { [weak surface] in
            guard let surface else { return }
            start(surface)
        }
        return surface
    }

    func updateUIView(_ uiView: UIView, context: Context) {
        (uiView as? NSHSurface)?.onMessage = onMessage
        context.coordinator.attach = attach
    }
}

extension View {
    /// Rules 1 and 2 of ``NSHPage``: no safe area (the keyboard's neither),
    /// no SwiftUI navigation bar.
    func nshPage() -> some View {
        ignoresSafeArea(.all)
            .toolbar(.hidden, for: .navigationBar)
    }
}
