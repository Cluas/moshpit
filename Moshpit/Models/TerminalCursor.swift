import SwiftUI
import UIKit

/// The user's CURSOR settings (Settings screen): the swatches' colours here;
/// the core's view takes them as its style (``TerminalLook``).
///
/// The user's choice is AUTHORITATIVE. Remote output carries the same
/// controls — DECSCUSR (`CSI Ps SP q`) for shape/blink, OSC 12 for colour —
/// and vim, zsh plugins, and coding agents emit them freely, so a one-shot
/// apply only survives until the next such sequence ("the cursor style is
/// sometimes right, sometimes wrong"). So the core draws the user's cursor
/// whatever the remote asks for.
enum TerminalCursor {

    /// The colour the cursor is drawn in for the stored id `id` under the
    /// app theme `theme`: the Rust core's mapping (`terminalCursorColor`,
    /// `ui/src/ui/swatch.rs`'s `CursorColor::drawn`), so it is the swatch
    /// Settings shows picked. B318: each id is its own colour — Green the
    /// success green, not the accent; Blue (stored `teal`) the info blue;
    /// Accent the theme's accent; White white at 86% — and a choice hidden
    /// because it looks like the accent draws as Accent. The dark
    /// appearance's values: every screen but the ui gallery is dark until
    /// R18. "amber" is not user-selectable — it's the automatic
    /// roaming-state colour (prototype §3.11: "漫游态自动切换为 amber").
    static func color(forId id: String, theme: AppTheme = AppThemeCatalog.current) -> SwiftUI.Color {
        if id == "amber" { return SwiftUI.Color(hex: "FF9F0A") }
        let rgba = terminalCursorColor(id: id, theme: themeJSON(theme), light: false)
        let channel = { (shift: UInt32) in Double((rgba >> shift) & 0xFF) / 255 }
        return SwiftUI.Color(.sRGB, red: channel(24), green: channel(16), blue: channel(8),
                             opacity: channel(0))
    }

    /// `theme` as the screens' `theme` param (``NSHScreenParams``).
    private static func themeJSON(_ theme: AppTheme) -> String {
        let fields = [
            ("accent", theme.accent), ("accentPressed", theme.accentPressed),
            ("screenBG", theme.screenBG), ("terminalBG", theme.terminalBG),
        ]
        return "{" + fields.map { "\"\($0.0)\":\"\(NSHScreenParams.hex($0.1))\"" }
            .joined(separator: ",") + "}"
    }
}
