import SwiftUI
import UIKit
import SwiftTerm

/// Applies the user's CURSOR settings (Settings screen) to a live SwiftTerm
/// ``TerminalView``: caret colour via the public `caretColor`, and shape +
/// blink via ``Terminal/setCursorStyle(_:)``.
///
/// The user's choice is AUTHORITATIVE. Remote output carries the same
/// controls — DECSCUSR (`CSI Ps SP q`) for shape/blink, OSC 12 for colour —
/// and vim, zsh plugins, and coding agents emit them freely, so a one-shot
/// apply only survives until the next such sequence ("the cursor style is
/// sometimes right, sometimes wrong"). Callers install the returned pair as
/// the coordinator's ``SwiftTerminalView/Coordinator/enforcedCursor`` so it
/// is re-asserted after every feed.
enum TerminalCursor {

    /// Resolve the swatch ids to concrete colours. "amber" is not user-
    /// selectable — it's the automatic roaming-state colour (prototype §3.11:
    /// "漫游态自动切换为 amber").
    static func color(forId id: String) -> SwiftUI.Color {
        switch id {
        case "green":  return SwiftUI.Color(hex: "64DC82")
        case "white":  return SwiftUI.Color.white.opacity(0.86)
        case "accent": return SwiftUI.Color(hex: "0A84FF")
        case "amber":  return SwiftUI.Color(hex: "FF9F0A")
        default:       return SwiftUI.Color(hex: "5FE3D8")   // teal
        }
    }

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
                      to terminal: TerminalView) -> (style: CursorStyle, color: UIColor) {
        let desired = (style: style(shape: shape, blink: blink),
                       color: UIColor(color(forId: colorId)))
        terminal.caretColor = desired.color
        terminal.getTerminal().setCursorStyle(desired.style)
        return desired
    }

    /// Convenience: pull the three values straight off AppSettings.
    @discardableResult
    static func apply(_ settings: AppSettings, to terminal: TerminalView) -> (style: CursorStyle, color: UIColor) {
        apply(shape: settings.cursorShape,
              colorId: settings.cursorColorId,
              blink: settings.cursorBlink,
              to: terminal)
    }
}
