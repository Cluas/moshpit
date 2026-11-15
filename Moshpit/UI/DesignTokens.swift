import SwiftUI
import UIKit

/// Moshpit's visual tokens.
///
/// Direction: "Signal Room" — a graphite-black iOS shell with citron focus,
/// blue transport signal, ember attention, and compact mechanical surfaces.
enum Ink {
    // MARK: - Core Palette
    // Accent, accentPressed, and the two base surface tones come from the
    // active AppTheme (Settings → Appearance) so switching themes recolors
    // every existing call site below without touching them individually.
    // Used sparingly (one primary action or active-state signal per screen)
    // rather than as a base UI color — per HIG "apply color sparingly...
    // reserve for elements that truly benefit from emphasis."
    static var accent: Color { AppThemeCatalog.current.accent }        // primary action / focus
    static var accentPressed: Color { AppThemeCatalog.current.accentPressed }
    static let signal = Color(hex: "7AA7FF")        // links, mosh transport
    static let success = Color(hex: "83E58C")
    static let warn = Color(hex: "FFB35C")
    /// `warn` as words and icons: the tokens' `warning-text` in the light
    /// appearance, where the amber itself reads at 1.8:1 (R18).
    static let warnText = Color(light: Color(hex: "935200"), dark: warn)
    /// The accent as words (a bar's buttons): the accent darkened by the
    /// least that reads at 4.5:1 on the light page and card, as the NSH
    /// pages' `primary-text` ([`InkContrast.readable`]); the accent itself in
    /// the dark appearance, as before (R18).
    static var accentText: Color {
        let accent = UIColor(AppThemeCatalog.current.accent)
        return Color(uiColor: UIColor(light: InkContrast.readable(accent), dark: accent))
    }
    static let danger = Color(hex: "FF6B6B")
    static let mosh = signal

    // MARK: - Surfaces
    static var screenBG: Color { AppThemeCatalog.current.screenBG }
    static var terminalBG: Color { AppThemeCatalog.current.terminalBG }
    /// Behind an NSH page: the tokens' `background`, the paper in the light
    /// appearance (R18) and the theme's screen in the dark one.
    static var pageBG: Color { Color(light: Color(hex: "F3F2EE"), dark: screenBG) }
    static let terminalGrid = Color(hex: "26303C").opacity(0.24)
    // The modal card's colours follow the appearance where it shows (R18):
    // over the NSH Home or terminal it is light in the light appearance.
    static let modalBG = Color(light: Color(hex: "FFFFFF"), dark: Color(hex: "0C0E12"))
    static let modalEdge = Color(light: Color.black.opacity(0.08), dark: Color.white.opacity(0.10))
    static let modalShadow = Color(light: Color.black.opacity(0.18), dark: Color.black.opacity(0.55))
    static let modalButtonFill = Color(light: Color.black.opacity(0.05), dark: Color.white.opacity(0.06))
    static let modalPrimaryText = Color(light: Color(hex: "4E4DEC"), dark: Color(hex: "CFCEFF"))
    static let modalDangerText = Color(light: Color(hex: "BC262C"), dark: Color(hex: "FFC4C4"))
    // A form row's fill: the tokens' `card` in the light appearance (R18).
    static let group = Color(light: Color(hex: "FFFFFF"), dark: Color(hex: "11141A").opacity(0.94))
    static let groupRaised = Color(hex: "171B22").opacity(0.96)
    static let sheet = Color(light: Color(hex: "F3F2EE").opacity(0.98), dark: Color(hex: "10131A").opacity(0.98))
    static let hairline = Color(light: Color.black.opacity(0.08), dark: Color.white.opacity(0.075))
    // Faint neutral fill on dark chrome (SSH pill, inactive sheet-row icon).
    static let neutralFill = Color(light: Color.black.opacity(0.04), dark: Color.white.opacity(0.045))

    // MARK: - Text
    // Light in the light appearance (R18), as the tokens' `foreground` and
    // `muted-foreground`; the dark pages draw the dark side, as before.
    static let primary = Color(uiColor: InkUI.primary)
    static let secondary = Color(uiColor: InkUI.secondary)
    static let tertiary = Color(light: Color(hex: "5E5C56"), dark: Color(hex: "DED7C9").opacity(0.48))
    static let meta = Color(light: Color(hex: "5E5C56").opacity(0.85), dark: Color(hex: "C7C1B5").opacity(0.44))
    // Section headers are structural chrome, not emphasis — HIG favors
    // spacing/tone over color for grouping. Neutral, not accent-tinted.
    static let sectionTitle = Color(light: Color(hex: "5E5C56"), dark: Color(hex: "DED7C9").opacity(0.42))
    static let placeholder = Color(hex: "DAD3C3").opacity(0.32)
    static let disabledNav = Color(light: Color.black.opacity(0.25), dark: Color.white.opacity(0.22))

    // MARK: - Segmented controls
    // Every segmented control's track and pill (`Chrome.installAppearance`):
    // the tokens' `muted` and `card` in the light appearance (R18).
    static let segTrack = Color(uiColor: InkUI.segTrack)
    static let segActive = Color(uiColor: InkUI.segActive)
}

/// Typography helpers. Display uses SF Rounded for a warmer iOS-native logo
/// voice; dense UI and terminal affordances stay monospaced.
///
/// Sizes are design-time numbers. Each resolves to the nearest system text
/// style, so every label follows Dynamic Type the way the rest of iOS does —
/// a fixed 14pt row label was the single biggest "this is a web view" tell.
/// Only micro labels below the smallest style stay fixed: those are terminal
/// chrome (crumbs, pills) whose dense grid cannot grow.
enum Face {
    static func display(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        font(size, weight, design: .rounded)
    }

    static func text(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        font(size, weight, design: .default)
    }

    static func mono(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        font(size, weight, design: .monospaced)
    }

    private static func font(_ size: CGFloat, _ weight: Font.Weight, design: Font.Design) -> Font {
        guard let style = style(for: size) else {
            return .system(size: size, weight: weight, design: design)
        }
        return .system(style, design: design, weight: weight)
    }

    /// The system text style a design size stands for, by nearest default
    /// point size (caption2 11 · caption 12 · footnote 13 · subheadline 15 ·
    /// callout 16 · body 17 · title3 20 · title2 22 · title 28 · largeTitle
    /// 34). nil below caption2: those labels keep their exact size.
    static func style(for size: CGFloat) -> Font.TextStyle? {
        switch size {
        case ..<9.75: return nil
        case ..<11.75: return .caption2
        case ..<12.75: return .caption
        case ..<14: return .footnote
        case ..<15.5: return .subheadline
        case ..<16.5: return .callout
        case ..<19: return .body
        case ..<21: return .title3
        case ..<25: return .title2
        case ..<31: return .title
        default: return .largeTitle
        }
    }
}

enum Motion {
    static let settle = Animation.snappy(duration: 0.24, extraBounce: 0.08)
}

/// The adaptive inks UIKit draws with (`Chrome.installAppearance`), as
/// `UIColor`s whose provider picks the side: `Ink`'s are made from these.
enum InkUI {
    static let primary = UIColor(light: UIColor(Color(hex: "16171B").opacity(0.96)),
                                 dark: UIColor(Color(hex: "F4F1E8").opacity(0.96)))
    static let secondary = UIColor(light: UIColor(Color(hex: "16171B").opacity(0.72)),
                                   dark: UIColor(Color(hex: "E8E3D6").opacity(0.72)))
    static let segTrack = UIColor(light: UIColor(Color(hex: "ECEBE6")),
                                  dark: UIColor(Color(hex: "090B10").opacity(0.70)))
    static let segActive = UIColor(light: UIColor(Color(hex: "FFFFFF")),
                                   dark: UIColor(Color(hex: "2A303A").opacity(0.98)))
}

extension Color {
    /// `light` where it is drawn in the light appearance and `dark` everywhere
    /// else, resolved by the trait where it is drawn (R18). A SwiftUI page
    /// still forced dark (a fallback on the dark-only palette) draws `dark`,
    /// as it did.
    init(light: Color, dark: Color) {
        self.init(uiColor: UIColor(light: UIColor(light), dark: UIColor(dark)))
    }
}

extension UIColor {
    /// `light` in the light appearance, `dark` in every other.
    convenience init(light: UIColor, dark: UIColor) {
        self.init { $0.userInterfaceStyle == .light ? light : dark }
    }
}

/// The tokens' rule for the accent as words in the light appearance
/// (`ui/src/ui/tokens.rs`'s `readable_on`), for the SwiftUI pages that draw
/// it: its lightness lowered by the least that reads at 4.5:1 on the page,
/// a card and the accent's wash on both, its hue and saturation kept.
enum InkContrast {
    /// The light page and card (`background`, `card`).
    static let page: (Double, Double, Double) = (0xF3 / 255, 0xF2 / 255, 0xEE / 255)
    static let card: (Double, Double, Double) = (1, 1, 1)

    /// `accent`, readable on the light surfaces.
    static func readable(_ accent: UIColor) -> UIColor {
        var (r, g, b, a): (CGFloat, CGFloat, CGFloat, CGFloat) = (0, 0, 0, 0)
        accent.getRed(&r, green: &g, blue: &b, alpha: &a)
        let rgb = (Double(r), Double(g), Double(b))
        // The accent's wash (12%) over each surface, as `primary-soft`.
        let wash = { (s: (Double, Double, Double)) in
            (s.0 + (rgb.0 - s.0) * 0.12, s.1 + (rgb.1 - s.1) * 0.12, s.2 + (rgb.2 - s.2) * 0.12)
        }
        let surfaces = [page, card, wash(card), wash(page)]
        var (h, sat, l) = hsl(rgb)
        for _ in 0...200 {
            let color = fromHSL(h, sat, l)
            if surfaces.allSatisfy({ contrast(color, $0) >= 4.5 }) {
                return UIColor(red: color.0, green: color.1, blue: color.2, alpha: 1)
            }
            l = max(0, l - 0.005)
        }
        let color = fromHSL(h, sat, l)
        return UIColor(red: color.0, green: color.1, blue: color.2, alpha: 1)
    }

    /// WCAG's relative luminance.
    static func luminance(_ c: (Double, Double, Double)) -> Double {
        func channel(_ v: Double) -> Double { v <= 0.040_45 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
        return 0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2)
    }

    /// WCAG's contrast ratio of two opaque colours.
    static func contrast(_ a: (Double, Double, Double), _ b: (Double, Double, Double)) -> Double {
        let (la, lb) = (luminance(a), luminance(b))
        return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)
    }

    static func hsl(_ c: (Double, Double, Double)) -> (Double, Double, Double) {
        let (maxV, minV) = (max(c.0, c.1, c.2), min(c.0, c.1, c.2))
        let l = (maxV + minV) / 2
        guard maxV != minV else { return (0, 0, l) }
        let d = maxV - minV
        let s = l > 0.5 ? d / (2 - maxV - minV) : d / (maxV + minV)
        var h: Double
        switch maxV {
        case c.0: h = (c.1 - c.2) / d + (c.1 < c.2 ? 6 : 0)
        case c.1: h = (c.2 - c.0) / d + 2
        default: h = (c.0 - c.1) / d + 4
        }
        h /= 6
        return (h, s, l)
    }

    static func fromHSL(_ h: Double, _ s: Double, _ l: Double) -> (Double, Double, Double) {
        guard s > 0 else { return (l, l, l) }
        func hue(_ p: Double, _ q: Double, _ t0: Double) -> Double {
            var t = t0
            if t < 0 { t += 1 }
            if t > 1 { t -= 1 }
            if t < 1 / 6 { return p + (q - p) * 6 * t }
            if t < 1 / 2 { return q }
            if t < 2 / 3 { return p + (q - p) * (2 / 3 - t) * 6 }
            return p
        }
        let q = l < 0.5 ? l * (1 + s) : l + s - l * s
        let p = 2 * l - q
        return (hue(p, q, h + 1 / 3), hue(p, q, h), hue(p, q, h - 1 / 3))
    }
}
