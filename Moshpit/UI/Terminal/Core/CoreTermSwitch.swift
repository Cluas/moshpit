import Foundation
import MoshpitFFI

/// Whether a new terminal is drawn by the shared Rust terminal core
/// (crates/term, our wezterm-term fork: ``CoreTerminalView``) instead of the
/// SwiftTerm fork's view (docs/nsh/TERMINAL-CORE.md, gate T1).
///
/// A Bool in `UserDefaults.standard`, set on the hidden Core Switches page,
/// or for one launch with the launch argument `-MOSHPIT_TERM_CORE 1` (the
/// argument domain beats the stored value). Off by default: switch off is
/// exactly the app without it. Read once per terminal, when the terminal is
/// made; a terminal keeps the view it was made with.
///
/// What it covers: the session's own terminal (plain SSH, mosh, herdr — on
/// the Swift and the Rust session paths), wherever it shows (the SwiftUI
/// terminal screen, the NSH terminal screen's slot), mosh's predictive echo
/// included (drawn by the core over its frames), and tmux panes on either
/// session path (read when an attach's controller is made).
enum CoreTermSwitch {
    static let switchKey = "MOSHPIT_TERM_CORE"

    /// The library has the core, and the one this app was compiled against.
    static let available = mp_term_abi_version() == UInt32(MP_TERM_ABI_VERSION)

    /// The switch's verdict for a terminal being made now.
    static var isOn: Bool {
        available && UserDefaults.standard.bool(forKey: switchKey)
    }

    /// Set by a launch argument for this launch.
    static var setByLaunchArgument: Bool {
        UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)[switchKey] != nil
    }

    /// What the switch covers, for the debug page.
    static let note = """
        New terminals draw with the shared Rust core (wezterm-term fork): parsing off the \
        main thread, Metal drawing, the core's selection, search and key encoding. Covers the \
        session's own terminal (plain SSH, mosh with its predictive echo, herdr) on either \
        session path, and tmux panes. From the next terminal on.
        """
}
