import SwiftTerm

extension TerminalScrollback {
    /// Give a freshly minted terminal the larger history.
    /// `TerminalView(frame:font:)` builds its buffer with SwiftTerm's default
    /// 500 lines, and the budget has to reach that BUFFER, not just the
    /// option: `changeScrollback` grows the buffer in place. Setting
    /// `options.scrollback` and re-running `setup(isReset: false)`, as this
    /// once did, only resized the existing buffer — the option read the budget
    /// while the terminal kept 500 lines of history (a device run kept 582
    /// lines, 500 plus the screen, out of 3000).
    static func enlarge(_ terminalView: TerminalView) {
        let terminal = terminalView.getTerminal()
        guard terminal.options.scrollback < lines else { return }
        terminal.changeScrollback(lines)
    }
}
