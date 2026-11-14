/// A terminal's client-side history, far above SwiftTerm's 500-line default, so
/// long sessions — Claude Code tool output, build logs — stay scrollable on
/// the client. tmux retains even more server-side; this is the client window.
enum TerminalScrollback {
    /// Lines of client-side scrollback to keep per pane. Memory decides it:
    /// a cell costs about 24 bytes, so a full 80-column pane holds ~1.9 KB a
    /// line — 10 000 lines is ~19 MB per pane, 50 000 would be ~96 MB, and a
    /// phone keeps several tmux panes alive at once. tmux keeps the long
    /// history server-side (`history-limit`); this is the phone's window.
    static let lines = 10_000
}
