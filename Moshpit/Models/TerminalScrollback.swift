import SwiftTerm

/// Raises a SwiftTerm terminal's scrollback far above the 500-line default so
/// long sessions — Claude Code tool output, build logs — stay scrollable on
/// the client. tmux retains even more server-side; this is the client window.
enum TerminalScrollback {
    /// Lines of client-side scrollback to keep per pane.
    static let lines = 50_000

    /// Apply before any data is fed. `TerminalView(frame:font:)` builds its
    /// buffer with the default 500; we bump the option and re-run `setup()`
    /// (no data yet, so the reset is harmless) so the buffer is allocated at
    /// the larger size.
    static func enlarge(_ terminalView: TerminalView) {
        let terminal = terminalView.getTerminal()
        guard terminal.options.scrollback < lines else { return }
        terminal.options.scrollback = lines
        terminal.setup(isReset: false)
    }
}
