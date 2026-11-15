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
    case sshKeys = "ssh-keys"
    case shortcuts
    case shortcutLibrary = "shortcut-library"
    /// Settings › Theme, pushed inside Settings' navigation stack rather
    /// than presented.
    case theme
    /// Settings › App Icon, pushed.
    case appIcon = "app-icon"
    /// Home's Add Connection sheet, not Settings.
    case addConnection = "add-connection"
    /// Settings › Recent Log, pushed.
    case recentLog = "recent-log"
    /// Settings › Accent, pushed.
    case accent
    /// Settings › Voice Input › Recognition, pushed.
    case voiceEngine = "voice-engine"
    /// Settings › Voice Input › Language, pushed.
    case voiceLanguage = "voice-language"
    /// Settings › Send Feedback, presented.
    case feedback
    /// Settings › Voice Input › Model, pushed.
    case whisperModel = "whisper-model"
    /// Settings › Theme with its editor up, on the first custom theme (a new
    /// draft when there is none).
    case themeEditor = "theme-editor"
    /// Settings › Theme asking to delete the first custom theme.
    case themeDelete = "theme-delete"
    /// Settings › Accent with its editor up, on the first custom accent (a
    /// new draft when there is none).
    case accentEditor = "accent-editor"
    /// Settings › Accent asking to delete the first custom accent.
    case accentDelete = "accent-delete"
    /// Home itself, no sheet over it.
    case home

    /// The screens that open on Home itself rather than in Settings' sheet.
    static let onHome: Set<CaptureScreen> = [.home, .addConnection]
    /// SSH Keys asking to delete the first imported key. (With any of the
    /// keys' screens, `-MOSHPIT_SEED_KEYS demo` shows the capture's keys, in
    /// memory and never written, instead of the device's.)
    case sshKeyDelete = "ssh-key-delete"
    /// SSH Keys with Add Key up over it, on Generate or on Import.
    case addKey = "add-key"
    case addKeyImport = "add-key-import"
    /// SSH Keys with Select Key up over it (its keys, the second picked).
    case keyPicker = "key-picker"
    /// Shortcuts with Add Shortcut up over it, or the editor of a capture's
    /// custom shortcut (in memory: Save is not pressed).
    case addShortcut = "add-shortcut"
    case editShortcut = "edit-shortcut"
    /// Set up this host on a canned host (`-MOSHPIT_SEED_HOSTSETUP
    /// <scenario>`, HostSetupDemo's scenarios, fresh by default), as the
    /// window's root: the page its switch picks, and the NSH page. Unlike
    /// -MOSHPIT_HOSTSETUP_DEMO it writes no setting.
    case hostSetup = "host-setup"
    case hostSetupNSH = "host-setup-nsh"

    /// The screens that open Settings › SSH Keys first.
    static let opensSSHKeys: Set<CaptureScreen> = [
        .sshKeys, .sshKeyDelete, .addKey, .addKeyImport, .keyPicker,
    ]

    /// The screens that open Settings › Shortcuts first.
    static let opensShortcuts: Set<CaptureScreen> = [
        .shortcuts, .shortcutLibrary, .addShortcut, .editShortcut,
    ]

    /// The pages pushed in Settings' own navigation stack (a capture run
    /// opens Settings, then pushes one of these once its sheet is up).
    static let pushedInSettings: Set<CaptureScreen> = [
        .appIcon, .recentLog, .accent, .voiceEngine, .voiceLanguage, .whisperModel,
        .themeEditor, .themeDelete, .accentEditor, .accentDelete,
    ]

    /// The editor a capture run opens over its gallery: true the theme
    /// gallery's, false the accent gallery's.
    static var editorRequested: Bool? {
        switch requested {
        case .themeEditor: true
        case .accentEditor: false
        default: nil
        }
    }

    /// The gallery asked to show its delete card for its first custom entry:
    /// true the theme gallery's, false the accent gallery's.
    static var deleteRequested: Bool? {
        switch requested {
        case .themeDelete: true
        case .accentDelete: false
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
        [.settings, .sshKeys, .shortcuts, .shortcutLibrary, .theme, .appIcon, .addConnection,
         .recentLog, .accent, .voiceEngine, .voiceLanguage, .feedback, .whisperModel,
         .themeEditor, .themeDelete, .accentEditor, .accentDelete,
         .home,
         .sshKeyDelete, .addKey, .addKeyImport, .keyPicker,
         .addShortcut, .editShortcut, .hostSetup, .hostSetupNSH]
    }
}
#endif
