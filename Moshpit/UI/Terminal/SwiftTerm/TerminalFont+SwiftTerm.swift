import SwiftTerm
import UIKit

extension TerminalFont {
    /// Give `terminal` `font` unless it already has it: the same face at the
    /// same size.
    ///
    /// SwiftTerm's font setter does not compare. Whatever the value, it
    /// drops the attribute cache, repaints the whole screen, tries a resize
    /// and clears the selection — and the font is applied far more often
    /// than it changes: on every SwiftUI pass over a terminal, and to every
    /// tmux pane each time a terminal screen appears and on each reconnect.
    @MainActor
    static func apply(_ font: UIFont, to terminal: TerminalView) {
        let current = terminal.font
        guard current.pointSize != font.pointSize || current.fontName != font.fontName else { return }
        terminal.font = font
    }
}
