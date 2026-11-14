import Foundation

/// A Canary build (compiled with `MOSHPIT_CANARY`) turns every migration
/// switch on by default: the Rust core for sessions, stores and the tmux
/// parser, every screen drawn by NSH, and the terminal drawn by the shared
/// Rust terminal core (``CoreTermSwitch``, from 1.0.8 (105): the Canary week
/// is the last item of gate T1), and the way back from the background that
/// keeps a connection which survived it (``ResumeInPlaceSwitch``: what the
/// Canary phones' logs measure). Registered defaults only — nothing
/// is written — so a switch the user flips on the hidden Core Switches page
/// (Settings: tap the version line seven times) wins, and a build without the
/// flag is exactly the app without it.
///
/// Called before anything reads a switch: first thing in the app's `init()`,
/// and from ``RustStoreSwitch/isOn``, which property initialisers can reach
/// before `init()` runs.
enum CanaryDefaults {
    static func register() {
        _ = registered
    }

    private static let registered: Void = {
        #if MOSHPIT_CANARY
        var defaults: [String: Any] = [
            RustStoreSwitch.switchKey: true,
            RustSessionSwitch.switchKey: true,
            "MOSHPIT_RUST_TMUX_PARSER": true,
            CoreTermSwitch.switchKey: true,
            ResumeInPlaceSwitch.switchKey: true,
        ]
        for screen in NSHScreen.allCases {
            defaults[screen.switchKey] = true
        }
        UserDefaults.standard.register(defaults: defaults)
        #endif
    }()
}
