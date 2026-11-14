import SwiftTerm
import SwiftUI
import UIKit

/// The user's cursor on SwiftTerm's view: caret colour via the public
/// `caretColor`, shape and blink via ``Terminal/setCursorStyle(_:)``.
extension TerminalCursor {
    /// SwiftTerm caret style for a (shape, blink) pair.
    static func style(shape: CursorShape, blink: Bool) -> CursorStyle {
        switch (shape, blink) {
        case (.block, true): return .blinkBlock
        case (.block, false): return .steadyBlock
        case (.underline, true): return .blinkUnderline
        case (.underline, false): return .steadyUnderline
        case (.bar, true): return .blinkBar
        case (.bar, false): return .steadyBar
        }
    }

    /// Apply colour + shape + blink to one terminal. Idempotent.
    ///
    /// Uses ``Terminal/setCursorStyle(_:)`` rather than feeding a DECSCUSR
    /// escape: a fed escape is parsed as part of the remote stream, so when
    /// the parser happens to sit mid-sequence (a chunk boundary inside an
    /// OSC/CSI) the injected bytes are swallowed or corrupt that sequence —
    /// the setting randomly failed to stick.
    @discardableResult
    static func apply(shape: CursorShape, colorId: String, blink: Bool,
                      theme: AppTheme = AppThemeCatalog.current,
                      to terminal: TerminalView) -> (style: CursorStyle, color: UIColor) {
        let desired = (style: style(shape: shape, blink: blink),
                       color: UIColor(color(forId: colorId, theme: theme)))
        terminal.caretColor = desired.color
        terminal.getTerminal().setCursorStyle(desired.style)
        return desired
    }
}
