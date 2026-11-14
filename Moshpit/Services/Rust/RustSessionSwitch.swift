import Foundation
import MoshpitFFI

/// Whether a connection's terminal runs on the Rust core's session layer
/// (crates/session through crates/ffi, docs/nsh/DESIGN.md §3.6) instead of
/// ``SessionHub``'s Swift transports.
///
/// The switch is a Bool in `UserDefaults.standard`, set on the hidden Core
/// Switches page, or for one launch with the launch argument
/// `-MOSHPIT_RUST_SESSION 1` (the argument domain beats the stored value).
/// Off by default: switch off is exactly the app without it. It is read
/// once per connection, when the connection starts; a running session keeps
/// the path it started on.
///
/// Even with the switch on, some connections stay on the Swift path —
/// ``swiftPathReason(for:predictSetting:)`` says which and why — because
/// the Rust side of what they need isn't wired yet.
enum RustSessionSwitch {
    static let switchKey = "MOSHPIT_RUST_SESSION"

    /// Whether this build can run sessions on the Rust core: the library is
    /// linked and is the one the app was compiled against (the check the
    /// tmux parser's switch makes too).
    static let available = mp_ffi_abi_version() == UInt32(MP_FFI_ABI_VERSION)

    static var isOn: Bool {
        UserDefaults.standard.bool(forKey: switchKey)
    }

    /// Why `connection` stays on the Swift path with the switch on; nil when
    /// it runs on the Rust core. Every kind of connection runs on the core
    /// now (herdr over mosh since W10c: mosh-server runs the raw-attach loop,
    /// the control client steers it from beside mosh); this stays the one
    /// place a future exception would go.
    static func swiftPathReason(for connection: ServerConnection,
                                predictSetting: PredictMode) -> String? {
        nil
    }

    /// The switch's verdict for one connection, at connect time.
    @MainActor
    static func runsOnRust(_ connection: ServerConnection) -> Bool {
        guard available, isOn else { return false }
        return swiftPathReason(for: connection, predictSetting: AppSettings.shared.predictMode) == nil
    }

    /// What stays on the Swift path with the switch on, for the debug page.
    static let swiftPathNote = """
        On: plain SSH, SSH + tmux, SSH + herdr, mosh (predictive echo included), \
        mosh + tmux (tmux's own client on screen, the -CC sidecar for the tree) and \
        mosh + herdr (the raw-attach loop on screen, the control client beside it) connect \
        through the Rust core. Remote previews, \
        file transfers, image uploads, the path probe and host maintenance borrow the Rust \
        session's own connection (no second sign-in; a reconnect under way is waited for, a \
        session that isn't connected is said so); notification replies and shares go \
        through the core, which waits out a reconnect for them.
        """
}

/// Whether a session on the Rust core resumes in place on the way back from
/// the background: a connection that survived the suspension is asked one
/// round trip and kept when it answers — no veil, no reattach, no Face ID —
/// instead of being redialled after 20 s away (`Timing::resume_in_place`,
/// crates/session). The key is the core's own (`RESUME_IN_PLACE_KEY` in
/// crates/ffi): the core reads it from the app's preferences when the
/// registry is made and again around every move to the background and back
/// (B365), so a change takes effect at the next return. It used to be read
/// once per process, and iOS keeps a process for hours: a switch turned off
/// kept resuming in place until a launch the person never saw. Before the
/// first unlock, when the preferences answer with the registered default
/// alone, the core takes it as off until they can be read.
///
/// On by default in a Canary build (``CanaryDefaults``), off otherwise; set
/// on the hidden Core Switches page, or for one launch with the launch
/// argument `-MOSHPIT_RESUME_IN_PLACE 1`.
enum ResumeInPlaceSwitch {
    static let switchKey = "MOSHPIT_RESUME_IN_PLACE"

    /// What decided the core's resume in place, for the log: the
    /// preferences told apart into the person's switch, a launch argument
    /// and the build's default.
    static func words(_ source: ResumeInPlaceSource, defaults: UserDefaults = .standard,
                      bundle: String = Bundle.main.bundleIdentifier ?? "") -> String {
        switch source {
        case .environment:
            return "launch environment"
        case .preferences:
            if defaults.volatileDomain(forName: UserDefaults.argumentDomain)[switchKey] != nil {
                return "launch argument"
            }
            if defaults.persistentDomain(forName: bundle)?[switchKey] != nil {
                return "Core Switches"
            }
            return "build default"
        case .unreadable:
            return "preferences unreadable before the first unlock"
        case .unset:
            return "unset"
        }
    }
}
