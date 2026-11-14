import Foundation
import Testing
import UIKit
@testable import Moshpit

/// The user's cursor colour, as the terminal core's view takes it
/// (`CoreTerminalView.apply(theme:…)` draws ``TerminalCursor/color(forId:theme:)``).
/// Enforcing the user's cursor over the remote's DECSCUSR and OSC 12 is the
/// core's (crates/term); the SwiftTerm adapter's enforcement tests went with
/// it (batch B).
@Suite("TerminalCursor colour")
@MainActor
struct TerminalCursorTests {

    /// `color` as 0xRRGGBBAA, as the terminal draws it.
    private static func rgba(_ color: UIColor) -> UInt32 {
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        color.getRed(&r, green: &g, blue: &b, alpha: &a)
        let byte = { (c: CGFloat) in UInt32((min(max(c, 0), 1) * 255).rounded()) }
        return byte(r) << 24 | byte(g) << 16 | byte(b) << 8 | byte(a)
    }

    /// The colour the core's cursor takes for `id`.
    private static func drawn(_ id: String, under theme: AppTheme) -> UInt32 {
        rgba(UIColor(TerminalCursor.color(forId: id, theme: theme)))
    }

    @Test("B318: the stored green draws the success green, not the accent")
    func greenIsNotTheAccent() {
        let theme = AppThemeCatalog.signalRoom
        #expect(Self.drawn("green", under: theme) == 0x4CC3_8AFF)
        #expect(Self.drawn("accent", under: theme) == 0x6C6B_EFFF, "Accent is the theme's accent")
        #expect(Self.drawn("green", under: theme) != Self.drawn("accent", under: theme))
        #expect(Self.drawn("teal", under: theme) == 0x7AA7_FFFF, "Blue is the info blue")
    }

    @Test("a stored choice the swatches hide draws as the accent")
    func hiddenChoiceDrawsAsAccent() {
        // Terminal Green's accent is a green: Settings hides Green and shows
        // Accent picked, and the cursor is the accent.
        let theme = AppThemeCatalog.terminalGreen
        #expect(Self.drawn("green", under: theme) == Self.drawn("accent", under: theme))
        #expect(Self.drawn("green", under: theme) == 0x2FA8_71FF)
        #expect(Self.drawn("teal", under: theme) != Self.drawn("accent", under: theme))
    }

    @Test("a new app theme is a new look, so an open terminal takes its accent")
    func appThemeIsPartOfTheLook() {
        let signal = TerminalLook(theme: .dracula, cursorColorId: "accent", appTheme: AppThemeCatalog.signalRoom)
        var green = signal
        green.appTheme = AppThemeCatalog.terminalGreen
        #expect(signal != green)
    }
}
