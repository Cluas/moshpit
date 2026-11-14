#if DEBUG
import Foundation

/// A screen the capture run wants opened at launch, named rather than hunted for.
///
/// The marketing and documentation screenshots used to be taken by driving the
/// real UI with `idb`: dump the accessibility tree, find a row by a substring of
/// its label, tap the middle of its frame, scroll and retry if it was not on
/// screen yet. That works until it doesn't, and when it doesn't it fails
/// *silently* — the tap lands on whatever was under those coordinates and the
/// screenshot is saved and reported as a success. Five capture runs produced a
/// picture of the Settings root filed as "the SSH Keys screen", a healthy home
/// screen filed as "a connection error", and two copies of the terminal filed as
/// "scrollback" and "paste". Each was only caught by a person opening the file.
///
/// Naming the destination removes the guessing: the app is told where to be, and
/// if the name is unknown it says so on the console instead of landing somewhere
/// plausible.
///
/// DEBUG only, and deliberately so — this exists to serve a screenshot run, and
/// a shipping build has no business honouring an argument that opens arbitrary
/// screens. It sits beside the other capture seeds in MoshpitApp, which are
/// compiled out of Release for the same reason.
enum CaptureScreen: String {
    case settings
    /// Settings' NSH page, whatever its switch says (`settings` is the page
    /// the switch picks, `-MOSHPIT_NSH_SETTINGS 1` for NSH).
    case settingsNSH = "settings-nsh"
    case sshKeys = "ssh-keys"
    case shortcuts
    case shortcutLibrary = "shortcut-library"
    /// Pushed inside Settings' navigation stack rather than presented.
    case theme
    case appIcon = "app-icon"
    /// Home's Add Connection sheet, not Settings.
    case addConnection = "add-connection"
    /// Settings › Recent Log, pushed: the SwiftUI page or the NSH one, as
    /// its switch says (`-MOSHPIT_NSH_RECENT_LOG 1` for NSH).
    case recentLog = "recent-log"
    /// Settings › Recent Log's NSH page, whatever the switch says.
    case recentLogNSH = "recent-log-nsh"
    /// Settings › App Icon's NSH page, whatever the switch says (`app-icon`
    /// is the page the switch picks).
    case appIconNSH = "app-icon-nsh"
    /// Settings › Accent, pushed: the page its switch picks
    /// (`-MOSHPIT_NSH_ACCENT 1` for NSH), and its NSH page.
    case accent
    case accentNSH = "accent-nsh"
    /// Settings › Voice Input › Recognition, pushed, and its NSH page.
    case voiceEngine = "voice-engine"
    case voiceEngineNSH = "voice-engine-nsh"
    /// Settings › Voice Input › Language, pushed, and its NSH page.
    case voiceLanguage = "voice-language"
    case voiceLanguageNSH = "voice-language-nsh"
    /// Settings › Send Feedback, presented: the page its switch picks
    /// (`-MOSHPIT_NSH_FEEDBACK 1` for NSH), and its NSH page.
    case feedback
    case feedbackNSH = "feedback-nsh"
    /// Settings › Voice Input › Model, pushed, and its NSH page.
    case whisperModel = "whisper-model"
    case whisperModelNSH = "whisper-model-nsh"
    /// Settings › Theme's NSH page, whatever its switch says (`theme` is the
    /// page the switch picks, `-MOSHPIT_NSH_THEME 1` for NSH).
    case themeNSH = "theme-nsh"
    /// Settings › Theme with its editor up, on the first custom theme (a new
    /// draft when there is none): the editor its switch picks
    /// (`-MOSHPIT_NSH_THEME_EDITOR 1` for NSH), and the NSH editor. The
    /// gallery under it is the one its own switch picks.
    case themeEditor = "theme-editor"
    case themeEditorNSH = "theme-editor-nsh"
    /// Settings › Theme asking to delete the first custom theme: on the page
    /// its switch picks, and on the NSH page.
    case themeDelete = "theme-delete"
    case themeDeleteNSH = "theme-delete-nsh"
    /// Settings › Accent with its editor up, on the first custom accent (a
    /// new draft when there is none): the editor its switch picks
    /// (`-MOSHPIT_NSH_ACCENT_EDITOR 1` for NSH), and the NSH editor.
    case accentEditor = "accent-editor"
    case accentEditorNSH = "accent-editor-nsh"
    /// Settings › Accent asking to delete the first custom accent: on the
    /// page its switch picks, and on the NSH page.
    case accentDelete = "accent-delete"
    case accentDeleteNSH = "accent-delete-nsh"
    /// Home itself, no sheet over it: the page its switch picks
    /// (`-MOSHPIT_NSH_HOME 1` for NSH), and its NSH page whatever the
    /// switch says. Either NSH page reads its record from the core's
    /// sessions, so it shows only with them on (`-MOSHPIT_RUST_SESSION 1`,
    /// a Canary's default).
    case home
    case homeNSH = "home-nsh"
    /// Home's Add Connection sheet drawn by NSH, whatever its switch says
    /// (`add-connection` is the form its switch picks).
    case addConnectionNSH = "add-connection-nsh"

    /// The screens that open on Home itself rather than in Settings' sheet.
    static let onHome: Set<CaptureScreen> = [.home, .homeNSH, .addConnection, .addConnectionNSH]
    /// Settings › SSH Keys' NSH page, whatever its switch says (`ssh-keys` is
    /// the page the switch picks). Add `-MOSHPIT_SEED_KEYS demo` for the
    /// capture's keys (in memory, never written) instead of the device's.
    case sshKeysNSH = "ssh-keys-nsh"
    /// SSH Keys asking to delete the first imported key: on the page its
    /// switch picks, and on the NSH page.
    case sshKeyDelete = "ssh-key-delete"
    case sshKeyDeleteNSH = "ssh-key-delete-nsh"
    /// SSH Keys with Add Key up over it, on Generate or on Import: the Add
    /// Key page its switch picks (`-MOSHPIT_NSH_ADD_KEY 1` for NSH), and the
    /// NSH page.
    case addKey = "add-key"
    case addKeyNSH = "add-key-nsh"
    case addKeyImport = "add-key-import"
    case addKeyImportNSH = "add-key-import-nsh"
    /// SSH Keys with Select Key up over it (its keys, the second picked):
    /// the picker its switch picks, and the NSH one.
    case keyPicker = "key-picker"
    case keyPickerNSH = "key-picker-nsh"
    /// Settings › Shortcuts' NSH page, whatever its switch says.
    case shortcutsNSH = "shortcuts-nsh"
    /// Shortcuts with Add Shortcut up over it, or the editor of a capture's
    /// custom shortcut (in memory: Save is not pressed): the editor its
    /// switch picks (`-MOSHPIT_NSH_ADD_SHORTCUT 1` for NSH), and the NSH one.
    case addShortcut = "add-shortcut"
    case addShortcutNSH = "add-shortcut-nsh"
    case editShortcut = "edit-shortcut"
    case editShortcutNSH = "edit-shortcut-nsh"
    /// Set up this host on a canned host (`-MOSHPIT_SEED_HOSTSETUP
    /// <scenario>`, HostSetupDemo's scenarios, fresh by default), as the
    /// window's root: the page its switch picks, and the NSH page. Unlike
    /// -MOSHPIT_HOSTSETUP_DEMO it writes no setting.
    case hostSetup = "host-setup"
    case hostSetupNSH = "host-setup-nsh"

    /// The screens that open Settings › SSH Keys first.
    static let opensSSHKeys: Set<CaptureScreen> = [
        .sshKeys, .sshKeysNSH, .sshKeyDelete, .sshKeyDeleteNSH, .addKey, .addKeyNSH,
        .addKeyImport, .addKeyImportNSH, .keyPicker, .keyPickerNSH,
    ]

    /// The screens that open Settings › Shortcuts first.
    static let opensShortcuts: Set<CaptureScreen> = [
        .shortcuts, .shortcutLibrary, .shortcutsNSH, .addShortcut, .addShortcutNSH,
        .editShortcut, .editShortcutNSH,
    ]

    /// The pages pushed in Settings' own navigation stack (a capture run
    /// opens Settings, then pushes one of these once its sheet is up).
    static let pushedInSettings: Set<CaptureScreen> = [
        .appIcon, .appIconNSH, .recentLog, .recentLogNSH, .accent, .accentNSH,
        .voiceEngine, .voiceEngineNSH, .voiceLanguage, .voiceLanguageNSH,
        .whisperModel, .whisperModelNSH, .themeNSH, .themeEditor, .themeEditorNSH, .themeDelete,
        .themeDeleteNSH, .accentEditor, .accentEditorNSH, .accentDelete, .accentDeleteNSH,
    ]

    /// The theme or accent editor a capture run opens over its gallery, and
    /// whether it is the NSH one whatever the switch says.
    static var editorRequested: (theme: Bool, forcedNSH: Bool)? {
        switch requested {
        case .themeEditor: (true, false)
        case .themeEditorNSH: (true, true)
        case .accentEditor: (false, false)
        case .accentEditorNSH: (false, true)
        default: nil
        }
    }

    /// The gallery asked to show its delete card for its first custom entry
    /// (the theme gallery's, else the accent gallery's).
    static var deleteRequested: (theme: Bool, nsh: Bool)? {
        switch requested {
        case .themeDelete: (true, false)
        case .themeDeleteNSH: (true, true)
        case .accentDelete: (false, false)
        case .accentDeleteNSH: (false, true)
        default: nil
        }
    }

    /// The screen named by `-MOSHPIT_SEED_SCREEN`, if any.
    static var requested: CaptureScreen? {
        let args = ProcessInfo.processInfo.arguments
        guard
            let i = args.firstIndex(of: "-MOSHPIT_SEED_SCREEN"),
            i + 1 < args.count
        else { return nil }
        let raw = args[i + 1]
        guard let screen = CaptureScreen(rawValue: raw) else {
            // Loud, because the alternative is a capture that looks fine and is
            // of the wrong screen — the exact failure this type exists to end.
            print("[capture] unknown -MOSHPIT_SEED_SCREEN '\(raw)'; expected one of: "
                  + allCases.map(\.rawValue).joined(separator: ", "))
            return nil
        }
        return screen
    }

    static var allCases: [CaptureScreen] {
        [.settings, .settingsNSH, .sshKeys, .shortcuts, .shortcutLibrary, .theme, .appIcon, .addConnection,
         .recentLog, .recentLogNSH, .appIconNSH, .accent, .accentNSH, .voiceEngine, .voiceEngineNSH,
         .voiceLanguage, .voiceLanguageNSH, .feedback, .feedbackNSH, .whisperModel, .whisperModelNSH,
         .themeNSH, .themeEditor, .themeEditorNSH, .themeDelete, .themeDeleteNSH, .accentEditor,
         .accentEditorNSH, .accentDelete, .accentDeleteNSH,
         .home, .homeNSH, .addConnectionNSH,
         .sshKeysNSH, .sshKeyDelete, .sshKeyDeleteNSH,
         .addKey, .addKeyNSH, .addKeyImport, .addKeyImportNSH, .keyPicker, .keyPickerNSH, .shortcutsNSH,
         .addShortcut, .addShortcutNSH, .editShortcut, .editShortcutNSH, .hostSetup, .hostSetupNSH]
    }
}
#endif
