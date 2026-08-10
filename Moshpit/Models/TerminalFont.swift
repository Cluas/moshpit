import UIKit
import CoreText

/// The monospaced fonts offered in Settings → DISPLAY → Font. "system" is the
/// SF Mono system font; the rest are families guaranteed present on iOS.
enum TerminalFont {
    /// (id, display label). `id == "system"` → `monospacedSystemFont`; the
    /// bundled programmer fonts use their PostScript name as the id. The
    /// JetBrains/Hack/Maple families are shipped in Resources/Fonts and
    /// registered via UIAppFonts.
    static let families: [(id: String, label: String)] = [
        ("system", "SF Mono"),
        ("JetBrainsMono-Regular", "JetBrains Mono"),
        ("MapleMono-Regular", "Maple Mono"),
        ("FiraCode-Regular", "Fira Code"),
        ("SourceCodePro-Regular", "Source Code Pro"),
        ("IBMPlexMono-Regular", "IBM Plex Mono"),
        ("Hack-Regular", "Hack"),
        ("AnonymousPro-Regular", "Anonymous Pro"),
        ("Menlo", "Menlo"),
        ("Courier New", "Courier"),
    ]

    static func label(for id: String) -> String {
        families.first { $0.id == id }?.label ?? "SF Mono"
    }

    /// Build a font at `size` for the given family id, always falling back to
    /// the system monospaced font if the named family fails to load. The result
    /// carries a CJK fallback (see ``withCJKFallback``).
    static func font(id: String, size: CGFloat) -> UIFont {
        let base: UIFont = id == "system"
            ? .monospacedSystemFont(ofSize: size, weight: .regular)
            : (UIFont(name: id, size: size) ?? .monospacedSystemFont(ofSize: size, weight: .regular))
        return withCJKFallback(base, size: size)
    }

    /// Attach a CJK fallback face to a monospace font.
    ///
    /// The bundled programmer fonts (JetBrains Mono, Fira Code, …) carry no CJK
    /// glyphs, so Chinese/Japanese — whether echoed from the remote or shown in
    /// the IME's composing overlay — renders as `？`/tofu. A descriptor cascade
    /// list lets Core Text fall back to PingFang for those code points while
    /// ASCII keeps the chosen monospace face (and its width metrics). CJK stays
    /// double-width, so cell alignment is preserved.
    private static func withCJKFallback(_ base: UIFont, size: CGFloat) -> UIFont {
        // A *single* fallback face (PingFang SC) covers most Simplified-Chinese
        // text, but any code point it lacks — Traditional-only hanzi, Japanese
        // kana, symbol/checkmark/arrow glyphs a coding font also omits — falls
        // straight through to `？`/tofu. That reads as "intermittent": it depends
        // on which characters a given line happens to contain. A multi-face
        // cascade lets Core Text keep trying until something covers the glyph,
        // so coverage is effectively total. Order = priority; ASCII never leaves
        // the base monospace face (cascades only fire for glyphs it can't draw),
        // and CJK stays double-width, so cell metrics are untouched.
        let cjkNames = [
            "PingFangSC-Regular",        // Simplified Chinese (primary)
            "PingFangTC-Regular",        // Traditional Chinese
            "HiraginoSans-W3",           // Japanese kana / kanji
            "AppleSDGothicNeo-Regular",  // Korean
            "AppleSymbols",              // ✓ ✗ arrows, box/technical symbols
            "AppleColorEmoji",           // emoji (🤖 etc.)
        ]
        let cascade = cjkNames.map { UIFontDescriptor(fontAttributes: [.name: $0]) }
        // Disable ligatures. A cell terminal positions one glyph per column by
        // glyph index; a ligating coding font (Fira Code, JetBrains Mono, …)
        // collapses "->", "!=", "==" into a single glyph, so every glyph after
        // it lands one column early → garbled rows. Off keeps glyph-count ==
        // column-count, and is the correct behavior for a terminal anyway.
        let noLigatures: [[UIFontDescriptor.FeatureKey: Int]] = [
            [.type: kLigaturesType, .selector: kCommonLigaturesOffSelector],
            [.type: kLigaturesType, .selector: kContextualLigaturesOffSelector],
        ]
        let descriptor = base.fontDescriptor.addingAttributes([
            .cascadeList: cascade,
            .featureSettings: noLigatures,
        ])
        return UIFont(descriptor: descriptor, size: size)
    }
}

/// Validation for the UDP port-range editor — pure so it can be unit-tested
/// independently of the SwiftUI sheet.
enum UDPPortRange {
    enum ValidationError: Error, Equatable { case nonNumeric, outOfBounds, inverted }

    /// Parse + validate a `(from, to)` pair. Ports must be 1…65535 and
    /// from ≤ to.
    static func validate(from: String, to: String) -> Result<(Int, Int), ValidationError> {
        guard let s = Int(from.trimmingCharacters(in: .whitespaces)),
              let e = Int(to.trimmingCharacters(in: .whitespaces)) else {
            return .failure(.nonNumeric)
        }
        guard (1...65535).contains(s), (1...65535).contains(e) else {
            return .failure(.outOfBounds)
        }
        guard s <= e else { return .failure(.inverted) }
        return .success((s, e))
    }
}
