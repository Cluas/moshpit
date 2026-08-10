import SwiftUI

/// A complete app-chrome visual scheme: an accent pair plus a faint background
/// tint so each theme reads as a distinct mood, not just a recolored button.
/// Deliberately narrow surface area — only `accent`/`accentPressed` and the
/// background tones vary; the neutral/semantic tokens (text, warn, success,
/// danger, signal) stay fixed across themes so status colors never collide
/// with whichever accent is active (see `AppThemeCatalog` doc comments).
///
/// That narrowness is what makes a *custom* theme cheap: the user picks one
/// accent and the other three values are derived (``custom(id:name:accentHex:)``).
///
/// The app icon is deliberately **not** part of a theme. iOS can only switch to
/// alternate icons that were bundled and declared at build time, so an icon
/// could never follow a custom accent; pretending otherwise would mean either
/// forbidding custom colors or silently picking an approximate icon. Icons are
/// their own independent choice — see ``AppIconCatalog``.
struct AppTheme: Identifiable, Equatable {
    let id: String
    let name: String
    let accent: Color
    let accentPressed: Color
    /// Subtle per-theme tint applied to the base screen/terminal backgrounds.
    /// Kept intentionally faint — a mood cue, not a different app.
    let screenBG: Color
    let terminalBG: Color
    /// False for user-created themes, which are editable and deletable.
    let isBuiltIn: Bool

    /// The accent as stored hex — the single value a custom theme persists.
    let accentHex: String

    init(id: String, name: String, accentHex: String,
         accentPressedHex: String, screenBGHex: String, terminalBGHex: String,
         isBuiltIn: Bool) {
        self.id = id
        self.name = name
        self.accentHex = Color.normalizedHex(accentHex)
        self.accent = Color(hex: accentHex)
        self.accentPressed = Color(hex: accentPressedHex)
        self.screenBG = Color(hex: screenBGHex)
        self.terminalBG = Color(hex: terminalBGHex)
        self.isBuiltIn = isBuiltIn
    }

    static func == (lhs: AppTheme, rhs: AppTheme) -> Bool {
        // Compares the *values*, not just the id: a custom theme keeps its id
        // while its accent is edited, and callers (notably the root view's
        // rebuild key) have to notice that.
        lhs.id == rhs.id && lhs.accentHex == rhs.accentHex
    }

    /// Build a theme from one accent, deriving the rest — the shape a custom
    /// theme takes. `accentPressed` is the accent darkened for the pressed
    /// state; the two backgrounds are near-black carrying a faint wash of the
    /// accent, which is what gives each theme its mood without turning the app
    /// into a colored block.
    static func custom(id: String, name: String, accentHex: String) -> AppTheme {
        let accent = Color.normalizedHex(accentHex)
        return AppTheme(
            id: id,
            name: name,
            accentHex: accent,
            accentPressedHex: Color.blend(accent, toward: "000000", by: 0.18),
            screenBGHex: Color.blend(accent, toward: "000000", by: 0.955),
            terminalBGHex: Color.blend(accent, toward: "000000", by: 0.982),
            isBuiltIn: false)
    }
}

/// The 4 shipped themes. Each accent is chosen to sit clearly apart from the
/// fixed semantic colors (Ink.signal blue, Ink.warn amber, Ink.success green)
/// so "this is the primary action" never gets confused with a status color —
/// the exact mistake the original "Signal Room" pass made by flooding accent
/// everywhere instead of reserving it for emphasis (HIG: apply color
/// sparingly, reserve for elements that truly benefit from emphasis).
enum AppThemeCatalog {
    static let signalRoom = AppTheme(
        id: "signal-room", name: "Signal Room",
        accentHex: "6C6BEF", accentPressedHex: "5652D6",
        screenBGHex: "06070A", terminalBGHex: "030405", isBuiltIn: true)

    static let moshpitClassic = AppTheme(
        id: "moshpit-classic", name: "Teal Line",
        accentHex: "53DCC9", accentPressedHex: "3EBFA9",
        screenBGHex: "05090A", terminalBGHex: "020505", isBuiltIn: true)

    static let terminalGreen = AppTheme(
        id: "terminal-green", name: "Terminal Green",
        accentHex: "2FA871", accentPressedHex: "25865A",
        screenBGHex: "060B08", terminalBGHex: "030503", isBuiltIn: true)

    static let amberConsole = AppTheme(
        id: "amber-console", name: "Amber Console",
        accentHex: "C98A2E", accentPressedHex: "A66F1F",
        screenBGHex: "0A0806", terminalBGHex: "050403", isBuiltIn: true)

    static let builtIns: [AppTheme] = [signalRoom, moshpitClassic, terminalGreen, amberConsole]

    /// Built-ins plus the user's own, which live in ``AppThemeStore``.
    static var all: [AppTheme] { builtIns + AppThemeStore.shared.customThemes }

    static func theme(for id: String) -> AppTheme {
        all.first { $0.id == id } ?? signalRoom
    }

    /// The active theme, read live from `AppSettings.shared`. `Ink`'s
    /// accent-derived properties resolve through this so every existing
    /// `Ink.accent` call site updates without change.
    static var current: AppTheme {
        theme(for: AppSettings.shared.appThemeId)
    }
}

// MARK: - App icons

/// One bundled app icon. Icons are independent of the accent color (see
/// ``AppTheme``) and each bakes in its own colorway, so the gallery offers real
/// alternatives — including two entirely different figures — rather than
/// recolors of one mark.
struct AppIconOption: Identifiable, Equatable {
    /// Stable id used in settings. "default" is the primary icon.
    let id: String
    let name: String
    /// The `CFBundleAlternateIcons` key, or nil for the primary icon —
    /// exactly what `setAlternateIconName` wants.
    let alternateName: String?
    /// Bundled PNG base name, for previewing the real artwork in the picker.
    let previewAsset: String

    var isPrimary: Bool { alternateName == nil }
}

enum AppIconCatalog {
    // An easter-egg gallery: every alternate is its own figure with its own
    // colorway — recolors of one mark all read the same at a glance, which is
    // exactly the feedback that retired the old Teal/Green/Amber/Daylight/Mono
    // set. Each egg carries one accent, so the color range survived.
    static let all: [AppIconOption] = [
        AppIconOption(id: "default", name: "Moshpit", alternateName: nil,
                      previewAsset: "AppIcon60x60"),
        // The pre-rebrand handset — a ringdown circuit needs no dial.
        AppIconOption(id: "ringdown", name: "Ringdown", alternateName: "AppIcon-Ringdown",
                      previewAsset: "AppIcon-Ringdown60x60"),
        AppIconOption(id: "wq", name: ":wq", alternateName: "AppIcon-Wq",
                      previewAsset: "AppIcon-Wq60x60"),
        AppIconOption(id: "prefix", name: "Prefix", alternateName: "AppIcon-Prefix",
                      previewAsset: "AppIcon-Prefix60x60"),
        AppIconOption(id: "localhost", name: "Localhost", alternateName: "AppIcon-Localhost",
                      previewAsset: "AppIcon-Localhost60x60"),
        AppIconOption(id: "nocarrier", name: "No Carrier", alternateName: "AppIcon-NoCarrier",
                      previewAsset: "AppIcon-NoCarrier60x60"),
        AppIconOption(id: "cursor", name: "Cursor", alternateName: "AppIcon-Cursor",
                      previewAsset: "AppIcon-Cursor60x60"),
        AppIconOption(id: "hail", name: "Hail", alternateName: "AppIcon-Hail",
                      previewAsset: "AppIcon-Hail60x60"),
    ]

    static let primary = all[0]

    static func option(for id: String) -> AppIconOption {
        all.first { $0.id == id } ?? primary
    }

    /// The icon a pre-split install should land on. Before icons became their
    /// own setting they were implied by the app theme, so migrate to the
    /// equivalent artwork instead of resetting everyone to the primary icon.
    static func migratedID(fromAppThemeID themeID: String) -> String {
        switch themeID {
        case "moshpit-classic": return "teal"
        case "terminal-green": return "green"
        case "amber-console": return "amber"
        default: return primary.id
        }
    }
}
