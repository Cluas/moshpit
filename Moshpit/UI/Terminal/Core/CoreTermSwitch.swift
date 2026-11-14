import Foundation
import MoshpitFFI

/// New terminals are drawn by the shared Rust terminal core (crates/term,
/// our wezterm-term fork: ``CoreTerminalView``), not the SwiftTerm fork's
/// view (docs/nsh/TERMINAL-CORE.md, gate T1).
///
/// No switch any more (F0): every build draws with the core whenever its
/// library is the one the app was compiled against, as Canary builds did.
/// The SwiftTerm adapter goes with batch B.
enum CoreTermSwitch {
    /// The library has the core, and the one this app was compiled against.
    static let available = mp_term_abi_version() == UInt32(MP_TERM_ABI_VERSION)

    /// Whether a terminal being made now is the core's: in a build that can.
    static var isOn: Bool { available }
}
