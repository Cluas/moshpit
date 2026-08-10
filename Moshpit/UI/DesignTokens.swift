import SwiftUI

/// Moshpit Phase 1 visual system.
///
/// Direction: "Signal Room" — a graphite-black iOS shell with citron focus,
/// blue transport signal, ember attention, and compact mechanical surfaces.
/// Existing token names stay in place so Phase 2 can migrate call sites
/// incrementally without breaking the rest of the app.
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
    static let signalSoft = Color(hex: "A9C4FF")
    static let success = Color(hex: "83E58C")
    static let warn = Color(hex: "FFB35C")
    static let danger = Color(hex: "FF6B6B")
    static let mosh = signal

    // MARK: - Surfaces
    static var screenBG: Color { AppThemeCatalog.current.screenBG }
    static let backgroundTop = Color(hex: "11141A")
    static let backgroundMid = Color(hex: "0A0C10")
    static let backgroundBottom = Color(hex: "040507")
    static var terminalBG: Color { AppThemeCatalog.current.terminalBG }
    static let terminalGrid = Color(hex: "26303C").opacity(0.24)
    static let modalBG = Color(hex: "0C0E12")
    static let group = Color(hex: "11141A").opacity(0.94)
    static let groupRaised = Color(hex: "171B22").opacity(0.96)
    static let sheet = Color(hex: "10131A").opacity(0.98)
    static let navGlass = Color(hex: "0B0D12").opacity(0.82)
    static let shortcutBarBG = Color(hex: "0B0D11").opacity(0.98)
    static let shortcutKeyBG = Color(hex: "1B2028").opacity(0.98)
    static var shortcutKeyActiveBG: Color { accent }
    static let hairline = Color.white.opacity(0.075)
    static let cardDivider = Color.white.opacity(0.105)
    static let cardBorder = Color.white.opacity(0.115)
    static let groupBorder = Color.white.opacity(0.075)
    static let insetShadow = Color.black.opacity(0.38)
    // Faint neutral fill on dark chrome (SSH pill, inactive sheet-row icon).
    static let neutralFill = Color.white.opacity(0.045)
    // Subtle neutral element (inactive chip outline, empty meter track).
    static let faintFill = Color.white.opacity(0.08)

    // MARK: - Text
    static let primary = Color(hex: "F4F1E8").opacity(0.96)
    static let secondary = Color(hex: "E8E3D6").opacity(0.72)
    static let tertiary = Color(hex: "DED7C9").opacity(0.48)
    static let meta = Color(hex: "C7C1B5").opacity(0.44)
    // Section headers are structural chrome, not emphasis — HIG favors
    // spacing/tone over color for grouping. Neutral, not accent-tinted.
    static let sectionTitle = Color(hex: "DED7C9").opacity(0.42)
    static let placeholder = Color(hex: "DAD3C3").opacity(0.32)
    static let disabledNav = Color.white.opacity(0.22)
    static let fixedValue = Color(hex: "E8E3D6").opacity(0.66)
    static let termDefault = Color(hex: "E7E2D5").opacity(0.66)
    static let termMuted = Color(hex: "AEB7C5").opacity(0.46)

    // MARK: - Semantic Chips
    static let moshPillBG = signal.opacity(0.14)
    static let moshPillText = signalSoft
    static let moshPillBorder = signal.opacity(0.34)
    static let moshHostPillBG = signal.opacity(0.13)
    static let moshHostPillText = signalSoft
    static let roamPillBG = warn.opacity(0.14)
    static let roamPillText = Color(hex: "FFD39A")
    static let roamPillBorder = warn.opacity(0.36)
    // Neutral, not color-coded: SSH is the default transport, not a special
    // mode worth flagging (unlike Mosh/roaming's tinted pills) — matches the
    // same chrome as the back button and breadcrumb next to it.
    static let sshPillBG = neutralFill
    static let sshPillText = secondary
    static var promptGreen: Color { accent }
    static var cursorGreen: Color { accent }
    static var customChipBG: Color { accent.opacity(0.12) }
    static let customChipText = Color(hex: "EEFFB2")
    static var customChipBorder: Color { accent.opacity(0.28) }
    static var hostChipOnBG: Color { accent.opacity(0.16) }
    static var hostChipOnBorder: Color { accent.opacity(0.38) }
    static let hostChipOnText = Color(hex: "F0FFB8")
    static var modkeyOnBG: Color { accent.opacity(0.20) }
    static var modkeyOnBorder: Color { accent.opacity(0.42) }
    static let modkeyOnText = Color(hex: "F3FFC4")
    static var paneActiveBG: Color { accent.opacity(0.13) }
    static var paneActiveRing: Color { accent.opacity(0.34) }
    static let paneActiveTag = Color(hex: "F0FFB8")
    static var rowActiveBG: Color { accent.opacity(0.12) }
    static let seBadgeBG = success.opacity(0.14)
    static let seBadgeText = Color(hex: "B7F5C0")
    static let seBadgeBorder = success.opacity(0.30)
    static let strengthLabel = Color(hex: "F0FFB8")
    static let fpPreviewText = Color(hex: "DFF89A")
    static let predictOnText = Color(hex: "DFF89A")
    static let warnBoxTitle = Color(hex: "FFD39A")
    static let keyChipBG = Color.white.opacity(0.085)
    static let keyChipText = Color(hex: "E8E3D6").opacity(0.74)
    static let chipNeutralBG = Color(hex: "252B35").opacity(0.96)
    static let keyBG = Color(hex: "2C323C").opacity(0.96)
    static let segTrack = Color(hex: "090B10").opacity(0.70)
    static let segActive = Color(hex: "2A303A").opacity(0.98)
    static let toggleOff = Color(hex: "3A404B").opacity(0.54)
    static let termAmber = warn
    static var predictUnderline: Color { accent.opacity(0.55) }

    // MARK: - Gradients
    // `static var`, not `static let` — these reference the theme-dependent
    // `accent`/`accentPressed`, which are themselves computed. A `static let`
    // here would freeze at whichever value was live on first access (the
    // exact bug that left MoshpitMark's icon stuck on the default theme after
    // a switch) and never update again.
    static var hostIcon: LinearGradient {
        LinearGradient(colors: [accent, signal], startPoint: .topLeading, endPoint: .bottomTrailing)
    }
    static var avatarImported: LinearGradient {
        LinearGradient(colors: [signal, accent], startPoint: .topLeading, endPoint: .bottomTrailing)
    }
    static var avatarSE: LinearGradient {
        LinearGradient(colors: [success, accent], startPoint: .topLeading, endPoint: .bottomTrailing)
    }
    static let avatarYubikey = LinearGradient(
        colors: [warn, Color(hex: "FF7A59")],
        startPoint: .topLeading,
        endPoint: .bottomTrailing
    )
    static let avatarGray = LinearGradient(
        colors: [Color(hex: "454C58"), Color(hex: "20252E")],
        startPoint: .topLeading,
        endPoint: .bottomTrailing
    )
    static var strengthFill: LinearGradient {
        LinearGradient(colors: [accentPressed, accent], startPoint: .leading, endPoint: .trailing)
    }
    static let roamBanner = LinearGradient(
        colors: [warn.opacity(0.20), Color(hex: "22160B").opacity(0.86)],
        startPoint: .topLeading,
        endPoint: .bottomTrailing
    )
    static let chromeSheen = LinearGradient(
        colors: [Color.white.opacity(0.075), Color.white.opacity(0.018), Color.black.opacity(0.20)],
        startPoint: .topLeading,
        endPoint: .bottomTrailing
    )
}

/// Typography helpers. Display uses SF Rounded for a warmer iOS-native logo
/// voice; dense UI and terminal affordances stay monospaced.
enum Face {
    static func display(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight, design: .rounded)
    }

    static func text(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight, design: .default)
    }

    static func mono(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight, design: .monospaced)
    }
}

enum Metrics {
    // Generous, continuous-corner radii (Apple Fitness/Health register) —
    // previous 7-8pt values on every card/control/group read as boxy/flat.
    // `.continuous` (squircle) corner style was already used everywhere
    // these feed into; only the radius magnitude needed to change.
    static let groupRadius: CGFloat = 20
    static let cardRadius: CGFloat = 20
    static let sheetRadius: CGFloat = 32
    static let controlRadius: CGFloat = 14
    static let groupInset: CGFloat = 14
    /// Fixed (not minimum) width for every shortcut-bar keycap — "ctrl"/"esc"
    /// used to hug their own label while "^End"/"⇧Tab" stretched wider,
    /// so the row read as uneven. One width for all of them, sized to the
    /// longest label ("⇧Tab"/"^End" at 11pt mono semibold) plus padding.
    static let shortcutKeyWidth: CGFloat = 46
    static let cellMinHeight: CGFloat = 46
    static let pageHPad: CGFloat = 16
    static let homeHPad: CGFloat = 18
    /// Ceiling on the home column's width. Only iPad and iPhone landscape ever
    /// reach it — an iPhone in portrait is far narrower — and without it the
    /// cards stretch the full 1032pt of a 13" iPad, which reads as an iPhone
    /// layout nobody got around to adapting. Sized between the iPhone width the
    /// screen grew up at and the ~540pt form sheet that iOS already gives
    /// Settings and the galleries on iPad, so the root screen and the sheets
    /// presented over it look like they belong to the same app.
    static let homeMaxWidth: CGFloat = 640
    static let sheetMaxFraction: CGFloat = 0.68
    static let paneBoardHeight: CGFloat = 168
}

enum Motion {
    static let quick = Animation.easeOut(duration: 0.16)
    static let settle = Animation.snappy(duration: 0.24, extraBounce: 0.08)
    static let roamPulse = Animation.easeInOut(duration: 0.75).repeatForever(autoreverses: true)
}
