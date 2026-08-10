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

    /// Whether reaching this screen means opening Settings first.
    var isInsideSettings: Bool { true }

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
        [.settings, .sshKeys, .shortcuts, .shortcutLibrary]
    }
}
#endif
