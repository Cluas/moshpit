import Foundation
import MoshpitFFI

/// A connection's terminal runs on the Rust core's session layer
/// (crates/session through crates/ffi, docs/nsh/DESIGN.md §3.6), not
/// ``SessionHub``'s Swift transports.
///
/// No switch any more (F0): every build runs every connection on the core
/// whenever its library is the one the app was compiled against, as Canary
/// builds did. The Swift transports go with batches E1–E4.
enum RustSessionSwitch {
    /// Whether this build can run sessions on the Rust core: the library is
    /// linked and is the one the app was compiled against.
    static let available = mp_ffi_abi_version() == UInt32(MP_FFI_ABI_VERSION)

    /// Whether `connection` runs on the core: every one, in a build that can.
    @MainActor
    static func runsOnRust(_ connection: ServerConnection) -> Bool {
        available
    }
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
/// On in every build, as Canary builds ran it: registered as the key's
/// default at launch (``registerDefault()``) until the core decides it
/// without the app's preferences (F0b). The Core Switches page that could
/// turn it off is gone.
enum ResumeInPlaceSwitch {
    static let switchKey = "MOSHPIT_RESUME_IN_PLACE"

    /// The key's default, on: before anything makes the core's registry.
    static func registerDefault() {
        UserDefaults.standard.register(defaults: [switchKey: true])
    }

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
                return "stored (an earlier build's Core Switches)"
            }
            return "build default"
        case .unreadable:
            return "preferences unreadable before the first unlock"
        case .unset:
            return "unset"
        }
    }
}
