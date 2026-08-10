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
    /// Apply the theme to a live ``TerminalView`` instance.
    ///
    /// Safe to call repeatedly (idempotent); used both during
    /// ``UIViewRepresentable/makeUIView(context:)`` and on every
    /// ``UIViewRepresentable/updateUIView(_:context:)`` re-render.
    func apply(to terminalView: TerminalView) {
        terminalView.nativeForegroundColor = UIColor(foreground)
        terminalView.nativeBackgroundColor = UIColor(background)
        terminalView.caretColor = UIColor(cursor)

        terminalView.installColors(ansiPalette())
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
            red:   expand8To16(r),
            green: expand8To16(g),
            blue:  expand8To16(b)
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
