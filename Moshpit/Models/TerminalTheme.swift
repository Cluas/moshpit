import SwiftUI
import UIKit

/// A terminal color scheme: the three chrome colors plus a full 16-entry ANSI
/// palette.
///
/// Colors are stored as hex **strings**, not `SwiftUI.Color`, for two reasons:
/// a theme has to round-trip through JSON (custom themes are persisted, and
/// exported/imported as text), and `Color` is not meaningfully `Codable` — its
/// components can only be read back by resolving it through `UIColor`, which is
/// lossy for anything but plain sRGB. The `Color` accessors below keep call
/// sites (`theme.background`, `theme.red`, …) reading exactly as before.
///
/// The bright half of the palette (ANSI 8–15) is optional: `nil` means "derive
/// it", which is what every built-in theme does, so authoring a theme only
/// requires the 8 base colors. A custom theme may override any or all of the
/// bright slots — see ``bright(_:)``.
struct TerminalTheme: Identifiable, Codable, Equatable, Hashable {
    var id: String
    var name: String

    var backgroundHex: String
    var foregroundHex: String
    var cursorHex: String

    /// ANSI 0–7, in ``ANSISlot`` order. Always exactly 8 entries; the
    /// initializers guarantee it and ``normalized`` repairs decoded input.
    var ansiHex: [String]

    /// ANSI 8–15, in ``ANSISlot`` order, or `nil` to derive them from
    /// ``ansiHex``. Entries may be individually empty to derive just that slot.
    var brightHex: [String]?

    /// True for the themes that ship with the app. They are never persisted and
    /// cannot be edited or deleted in place — the gallery offers "duplicate"
    /// instead, which produces an editable copy with `isBuiltIn == false`.
    var isBuiltIn: Bool

    // MARK: - Palette slots

    /// The eight ANSI color names, in palette order. Doubles as the key set for
    /// the flat JSON encoding (`black`, …, `brightBlack`, …).
    enum ANSISlot: Int, CaseIterable, Identifiable {
        case black, red, green, yellow, blue, magenta, cyan, white

        var id: Int { rawValue }

        var key: String {
            switch self {
            case .black: return "black"
            case .red: return "red"
            case .green: return "green"
            case .yellow: return "yellow"
            case .blue: return "blue"
            case .magenta: return "magenta"
            case .cyan: return "cyan"
            case .white: return "white"
            }
        }

        var brightKey: String { "bright" + key.prefix(1).uppercased() + key.dropFirst() }

        /// Display label for the editor, e.g. "Black" / "Bright Black".
        var label: String { key.prefix(1).uppercased() + key.dropFirst() }
    }

    // MARK: - Color accessors

    var background: Color { Color(hex: backgroundHex) }
    var foreground: Color { Color(hex: foregroundHex) }
    var cursor: Color { Color(hex: cursorHex) }

    var black: Color { ansi(.black) }
    var red: Color { ansi(.red) }
    var green: Color { ansi(.green) }
    var yellow: Color { ansi(.yellow) }
    var blue: Color { ansi(.blue) }
    var magenta: Color { ansi(.magenta) }
    var cyan: Color { ansi(.cyan) }
    var white: Color { ansi(.white) }

    func ansi(_ slot: ANSISlot) -> Color { Color(hex: ansiHex(slot)) }

    func ansiHex(_ slot: ANSISlot) -> String {
        ansiHex.indices.contains(slot.rawValue) ? ansiHex[slot.rawValue] : "000000"
    }

    /// A bright slot: the explicit override when one is set, otherwise derived
    /// from the base color. Deriving rather than duplicating matters for
    /// legibility — a shell that paints "bright black" for dim text against a
    /// dark background is unreadable when bright black *is* black.
    func brightHex(_ slot: ANSISlot) -> String {
        if let overrides = brightHex, overrides.indices.contains(slot.rawValue) {
            let value = overrides[slot.rawValue]
            if !value.isEmpty { return value }
        }
        return Self.derivedBright(from: ansiHex(slot))
    }

    func bright(_ slot: ANSISlot) -> Color { Color(hex: brightHex(slot)) }

    /// True when this slot carries an explicit override (drives the editor's
    /// "auto / custom" state without a second stored flag).
    func hasBrightOverride(_ slot: ANSISlot) -> Bool {
        guard let overrides = brightHex, overrides.indices.contains(slot.rawValue) else { return false }
        return !overrides[slot.rawValue].isEmpty
    }

    /// Set one ANSI slot. Goes through here rather than letting callers splice
    /// ``ansiHex`` directly so every write is normalized — a stored `"#RRGGBB"`
    /// would re-encode as `"##RRGGBB"` and stop round-tripping.
    mutating func setAnsi(_ hex: String, for slot: ANSISlot) {
        var slots = ansiHex
        while slots.count <= slot.rawValue { slots.append("000000") }
        slots[slot.rawValue] = Color.normalizedHex(hex)
        ansiHex = slots
    }

    /// Set (or clear, with `nil`) one bright override, materializing the
    /// override array on first write and collapsing it back to `nil` once every
    /// slot is cleared (so a fully-derived theme encodes without a dead array).
    mutating func setBrightOverride(_ hex: String?, for slot: ANSISlot) {
        var overrides = brightHex ?? Array(repeating: "", count: ANSISlot.allCases.count)
        while overrides.count < ANSISlot.allCases.count { overrides.append("") }
        overrides[slot.rawValue] = hex.map(Color.normalizedHex) ?? ""
        brightHex = overrides.allSatisfy(\.isEmpty) ? nil : overrides
    }

    /// Blend 32% toward white. Not xterm's table (which is hand-picked), but a
    /// predictable lift that keeps hue and never darkens — and, unlike copying
    /// the base color into the bright slot, it keeps bright-black distinct from
    /// black.
    static func derivedBright(from hex: String) -> String {
        let (r, g, b) = Color.rgbComponents(hex: hex)
        func lift(_ c: Double) -> Double { c + (1 - c) * 0.32 }
        return Color.hexString(r: lift(r), g: lift(g), b: lift(b))
    }

    // MARK: - Init

    init(id: String,
         name: String,
         background: String,
         foreground: String,
         cursor: String,
         ansi: [String],
         bright: [String]? = nil,
         isBuiltIn: Bool = false) {
        self.id = id
        self.name = name
        self.backgroundHex = Color.normalizedHex(background)
        self.foregroundHex = Color.normalizedHex(foreground)
        self.cursorHex = Color.normalizedHex(cursor)
        var slots = ansi.map(Color.normalizedHex)
        while slots.count < ANSISlot.allCases.count { slots.append("000000") }
        self.ansiHex = Array(slots.prefix(ANSISlot.allCases.count))
        self.brightHex = bright.map { list in
            var normalized = list.map { $0.isEmpty ? "" : Color.normalizedHex($0) }
            while normalized.count < ANSISlot.allCases.count { normalized.append("") }
            return Array(normalized.prefix(ANSISlot.allCases.count))
        }
        self.isBuiltIn = isBuiltIn
    }

    // MARK: - Codable (flat, human-readable keys)

    /// Encoded shape — deliberately flat and named so an exported theme is
    /// something a person can read and hand-edit, and so the importer can
    /// accept palettes pasted from other terminal emulators, which use these
    /// same names:
    ///
    /// ```json
    /// { "id": "…", "name": "…", "background": "#0D1117", "foreground": "…",
    ///   "cursor": "…", "black": "#08080A", …, "brightBlack": "…" }
    /// ```
    ///
    /// `isBuiltIn` is never encoded: it describes where a theme came from in
    /// *this* install, so anything decoded is by definition a custom theme.
    private struct Key: CodingKey {
        var stringValue: String
        var intValue: Int? { nil }
        init(_ value: String) { stringValue = value }
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { return nil }
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Key.self)

        func hex(_ names: [String]) -> String? {
            for name in names {
                if let raw = try? c.decode(String.self, forKey: Key(name)), !raw.isEmpty {
                    return Color.normalizedHex(raw)
                }
            }
            return nil
        }

        let name = try c.decodeIfPresent(String.self, forKey: Key("name")) ?? "Imported"
        // A theme pasted from elsewhere has no id of ours; mint one so it can
        // still be stored and selected.
        let id = try c.decodeIfPresent(String.self, forKey: Key("id"))
            ?? "custom-\(UUID().uuidString.prefix(8).lowercased())"

        // `bg`/`fg` are common shorthands in hand-written palettes.
        let background = hex(["background", "bg"]) ?? "000000"
        let foreground = hex(["foreground", "fg", "text"]) ?? "FFFFFF"
        let cursor = hex(["cursor", "cursorColor", "caret"]) ?? foreground

        var ansi: [String] = []
        var bright: [String] = []
        for slot in ANSISlot.allCases {
            ansi.append(hex([slot.key, "ansi\(slot.rawValue)"]) ?? "000000")
            bright.append(hex([slot.brightKey, "ansi\(slot.rawValue + 8)"]) ?? "")
        }

        self.init(id: id, name: name,
                  background: background, foreground: foreground, cursor: cursor,
                  ansi: ansi,
                  bright: bright.allSatisfy(\.isEmpty) ? nil : bright,
                  isBuiltIn: false)
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Key.self)
        try c.encode(id, forKey: Key("id"))
        try c.encode(name, forKey: Key("name"))
        try c.encode("#" + backgroundHex, forKey: Key("background"))
        try c.encode("#" + foregroundHex, forKey: Key("foreground"))
        try c.encode("#" + cursorHex, forKey: Key("cursor"))
        for slot in ANSISlot.allCases {
            try c.encode("#" + ansiHex(slot), forKey: Key(slot.key))
            if hasBrightOverride(slot) {
                try c.encode("#" + brightHex(slot), forKey: Key(slot.brightKey))
            }
        }
    }

    // MARK: - Built-in themes

    static let bioluminescent = TerminalTheme(
        id: "bioluminescent", name: "Bioluminescent",
        background: "0D1117", foreground: "C9D1D9", cursor: "97C459",
        ansi: ["08080A", "F85149", "56D364", "EF9F27", "58A6FF", "C9B4F0", "58A6FF", "C9D1D9"],
        isBuiltIn: true)

    static let dracula = TerminalTheme(
        id: "dracula", name: "Dracula",
        background: "282A36", foreground: "F8F8F2", cursor: "F8F8F2",
        ansi: ["21222C", "FF5555", "50FA7B", "F1FA8C", "BD93F9", "FF79C6", "8BE9FD", "F8F8F2"],
        isBuiltIn: true)

    static let nord = TerminalTheme(
        id: "nord", name: "Nord",
        background: "2E3440", foreground: "D8DEE9", cursor: "D8DEE9",
        ansi: ["3B4252", "BF616A", "A3BE8C", "EBCB8B", "81A1C1", "B48EAD", "88C0D0", "E5E9F0"],
        isBuiltIn: true)

    static let solarizedDark = TerminalTheme(
        id: "solarized-dark", name: "Solarized Dark",
        background: "002B36", foreground: "839496", cursor: "839496",
        ansi: ["073642", "DC322F", "859900", "B58900", "268BD2", "D33682", "2AA198", "EEE8D5"],
        isBuiltIn: true)

    static let monokai = TerminalTheme(
        id: "monokai", name: "Monokai",
        background: "272822", foreground: "F8F8F2", cursor: "F8F8F0",
        ansi: ["272822", "F92672", "A6E22E", "F4BF75", "66D9EF", "AE81FF", "A1EFE4", "F8F8F2"],
        isBuiltIn: true)

    static let tokyoNight = TerminalTheme(
        id: "tokyo-night", name: "Tokyo Night",
        background: "1A1B26", foreground: "C0CAF5", cursor: "C0CAF5",
        ansi: ["15161E", "F7768E", "9ECE6A", "E0AF68", "7AA2F7", "BB9AF7", "7DCFFF", "A9B1D6"],
        isBuiltIn: true)

    /// Liquid Glass — iOS 26 deep-glass palette with a cyan / violet / amber
    /// neon family. Background sits a touch above pure black so the SwiftUI
    /// glass surfaces layered above still read; foreground is a cool near-white.
    static let liquidGlass = TerminalTheme(
        id: "liquid-glass", name: "Liquid Glass",
        background: "0E1019", foreground: "E8EAF0", cursor: "5BD5F0",
        ansi: ["0A0C13", "E68988", "7DD49E", "E4B656", "5BD5F0", "B289F5", "5BD5F0", "F8F8FB"],
        isBuiltIn: true)

    /// GitHub Dark — the app's default theme.
    static let githubDark = TerminalTheme(
        id: "github-dark", name: "GitHub Dark",
        background: "050507", foreground: "C9D1D9", cursor: "5FE3D8",
        ansi: ["484F58", "FF7B72", "3FB950", "D29922", "58A6FF", "BC8CFF", "39C5CF", "B1BAC4"],
        isBuiltIn: true)

    static let builtIns: [TerminalTheme] = [
        .githubDark, .liquidGlass, .bioluminescent, .dracula, .nord, .solarizedDark, .monokai, .tokyoNight
    ]

    /// The theme used when a stored `themeId` resolves to nothing.
    static let fallback = TerminalTheme.githubDark
}

// MARK: - Color hex conversion

extension Color {
    init(hex: String) {
        let (r, g, b) = Color.rgbComponents(hex: hex)
        self.init(red: r, green: g, blue: b)
    }

    /// Parse `#RGB`, `#RRGGBB`, `RRGGBB`, or `0xRRGGBB` into 0–1 components.
    /// Anything unparseable is black — a theme with one typo'd color should
    /// still load and be fixable in the editor, not fail to decode.
    static func rgbComponents(hex: String) -> (Double, Double, Double) {
        let cleaned = normalizedHex(hex)
        var int: UInt64 = 0
        Scanner(string: cleaned).scanHexInt64(&int)
        return (Double((int >> 16) & 0xFF) / 255.0,
                Double((int >> 8) & 0xFF) / 255.0,
                Double(int & 0xFF) / 255.0)
    }

    /// Strip `#`/`0x`/whitespace, expand 3-digit shorthand, and uppercase —
    /// producing the bare 6-digit form used for storage and comparison.
    static func normalizedHex(_ hex: String) -> String {
        var s = hex.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        if s.hasPrefix("#") { s.removeFirst() }
        if s.hasPrefix("0X") { s.removeFirst(2) }
        s = s.filter(\.isHexDigit)
        if s.count == 3 {
            s = s.map { "\($0)\($0)" }.joined()
        }
        if s.count == 8 { s = String(s.prefix(6)) }   // #RRGGBBAA → drop alpha
        guard s.count == 6 else { return "000000" }
        return s
    }

    /// Blend one hex toward another, returning bare hex. Used for derived
    /// values — a theme's pressed accent, the faint background wash, a bright
    /// ANSI slot — so those relationships live in one place.
    static func blend(_ hex: String, toward target: String, by t: Double) -> String {
        let (r1, g1, b1) = rgbComponents(hex: hex)
        let (r2, g2, b2) = rgbComponents(hex: target)
        let k = max(0, min(1, t))
        return hexString(r: r1 + (r2 - r1) * k,
                         g: g1 + (g2 - g1) * k,
                         b: b1 + (b2 - b1) * k)
    }

    /// 0–1 components → bare 6-digit uppercase hex.
    static func hexString(r: Double, g: Double, b: Double) -> String {
        func byte(_ c: Double) -> Int { Int((max(0, min(1, c)) * 255).rounded()) }
        return String(format: "%02X%02X%02X", byte(r), byte(g), byte(b))
    }

    /// Resolve this color through `UIColor` and return its bare hex — the write
    /// path for `ColorPicker` bindings in the theme editor.
    var hexString: String {
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        UIColor(self).getRed(&r, green: &g, blue: &b, alpha: &a)
        return Color.hexString(r: Double(r), g: Double(g), b: Double(b))
    }
}
