import SwiftUI
import UIKit
import SwiftTerm

/// Bridges ``TerminalTheme`` (SwiftUI palette) into the SwiftTerm
/// ``TerminalView`` color model.
///
/// SwiftTerm expects:
///   - `nativeForegroundColor`, `nativeBackgroundColor`, `caretColor` as
///     `UIColor` values, and
///   - a 16-entry ANSI palette via ``TerminalView/installColors(_:)`` where
///     each entry is a SwiftTerm ``Color`` using 16-bit RGB channels.
///
/// ``TerminalTheme`` now supplies a real 16-entry palette: the 8 base colors it
/// declares plus 8 bright ones, either overridden by the theme or derived from
/// the base (``TerminalTheme/brightHex(_:)``). Earlier this bridge duplicated
/// the base colors into the bright slots, which made "bright black" — what many
/// shells and TUIs paint dim text with — identical to black, i.e. invisible on a
/// dark background.
extension TerminalTheme {
    /// Apply the theme to a live ``TerminalView`` instance, touching only
    /// what differs from what the view already has.
    ///
    /// Every colour setter here is a full-screen repaint in SwiftTerm: the
    /// foreground and background setters and `installColors` each drop its
    /// attribute cache and mark every row, whatever the value. Callers apply
    /// the theme far more often than it changes — each time a terminal screen
    /// appears, to every tmux pane, and on a reconnect — so a call that
    /// changes nothing has to cost nothing, or it throws away the partial
    /// repaints SwiftTerm keeps for a typed key.
    ///
    /// Foreground and background are compared with what the view has now, so
    /// a colour the remote set since (OSC 10/11) still gives way to the
    /// theme. The palette cannot be read back from SwiftTerm; it is compared
    /// with the one this method installed last.
    func apply(to terminalView: TerminalView) {
        let foreground = UIColor(self.foreground)
        let background = UIColor(self.background)
        let caret = UIColor(cursor)
        let palette = ansiPalette()

        if !Self.sameColor(terminalView.nativeForegroundColor, foreground) {
            terminalView.nativeForegroundColor = foreground
        }
        if !Self.sameColor(terminalView.nativeBackgroundColor, background) {
            terminalView.nativeBackgroundColor = background
        }
        if !Self.sameColor(terminalView.caretColor, caret) {
            terminalView.caretColor = caret
        }
        let installed = objc_getAssociatedObject(terminalView, &Self.installedPaletteKey) as? InstalledPalette
        if installed?.colors != palette {
            terminalView.installColors(palette)
            objc_setAssociatedObject(terminalView, &Self.installedPaletteKey,
                                     InstalledPalette(palette), .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
        }
    }

    /// The palette ``apply(to:)`` last installed in a view, kept on the view.
    private final class InstalledPalette {
        let colors: [SwiftTerm.Color]
        init(_ colors: [SwiftTerm.Color]) { self.colors = colors }
    }
    private static var installedPaletteKey: UInt8 = 0

    /// The same colour, compared by value: every `UIColor(Color)` is a new
    /// object, and the view hands back whichever one it was given last.
    private static func sameColor(_ a: UIColor, _ b: UIColor) -> Bool {
        if a === b || a.isEqual(b) { return true }
        var ar: CGFloat = 0, ag: CGFloat = 0, ab: CGFloat = 0, aa: CGFloat = 0
        var br: CGFloat = 0, bg: CGFloat = 0, bb: CGFloat = 0, ba: CGFloat = 0
        guard a.getRed(&ar, green: &ag, blue: &ab, alpha: &aa),
              b.getRed(&br, green: &bg, blue: &bb, alpha: &ba) else { return false }
        return (ar, ag, ab, aa) == (br, bg, bb, ba)
    }

    /// 16-entry ANSI palette: base 0–7 followed by bright 8–15.
    private func ansiPalette() -> [SwiftTerm.Color] {
        let slots = TerminalTheme.ANSISlot.allCases
        return (slots.map { ansi($0) } + slots.map { bright($0) })
            .map(Self.swiftTermColor(from:))
    }

    /// Convert a SwiftUI ``Color`` into a SwiftTerm 16-bit ``Color``.
    ///
    /// Goes via ``UIColor`` so we get the resolved RGB even when the source
    /// was constructed from a hex string. Each 0–255 byte is expanded to a
    /// 16-bit channel via `byte << 8 | byte` (i.e. multiplied by 257), which
    /// is the standard 8→16 bit promotion that keeps pure white at 0xFFFF.
    private static func swiftTermColor(from color: SwiftUI.Color) -> SwiftTerm.Color {
        var r: CGFloat = 0
        var g: CGFloat = 0
        var b: CGFloat = 0
        var a: CGFloat = 0
        UIColor(color).getRed(&r, green: &g, blue: &b, alpha: &a)

        return SwiftTerm.Color(
            red: expand8To16(r),
            green: expand8To16(g),
            blue: expand8To16(b)
        )
    }

    /// Clamp a 0–1 channel to 0–255 byte, then promote to 16-bit via
    /// `UInt16(byte) << 8 | UInt16(byte)` (== `byte * 257`).
    private static func expand8To16(_ channel: CGFloat) -> UInt16 {
        let clamped = max(0, min(1, channel))
        let byte = UInt16(clamped * 255.0 + 0.5)   // round-to-nearest
        return (byte << 8) | byte
    }
}
