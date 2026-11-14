import SwiftUI
import UIKit

/// The agent-state colours, fixed by SEMANTICS — one value per state, the
/// same value on every surface that renders one.
///
/// Lives in MoshpitKit because the app and the widget both draw these: the
/// widget can't import the app's DesignTokens, and duplicating the constants
/// is exactly how the island's "working" spent a release as teal while the
/// app's was indigo.
///
/// Deliberately NOT the theme accent: `Ink.accent` follows the user's
/// Appearance choice, and a user who picks an amber-ish accent would watch
/// "working" and "needs you" become the same colour. State colours carry
/// meaning, so they don't move with taste.
public enum AgentPalette {
    /// Actively doing something — teal, matching the Live Activity timer.
    public static let working = Color(red: 94 / 255, green: 227 / 255, blue: 217 / 255)
    /// Waiting on a human — amber. Same value as `Ink.warn`, asserted by test.
    public static let attention = Color(red: 255 / 255, green: 179 / 255, blue: 92 / 255)
    /// Finished, not yet looked at — green. Same value as `Ink.success`.
    public static let done = Color(red: 131 / 255, green: 229 / 255, blue: 140 / 255)
}

extension AgentPalette {
    /// The states on a light lock screen or Home Screen (R18 D): the same
    /// hues deep enough to read on white, as the app's light text tones
    /// (its tokens' `warning-text` and `success-text`, a teal as deep).
    public static let workingOnLight = Color(red: 0x0F / 255, green: 0x76 / 255, blue: 0x6E / 255)
    public static let attentionOnLight = Color(red: 0x93 / 255, green: 0x52 / 255, blue: 0x00 / 255)
    public static let doneOnLight = Color(red: 0x17 / 255, green: 0x73 / 255, blue: 0x46 / 255)

    /// Each state's colour for the appearance it is drawn in: the light
    /// tone in the light one, the colour above in every other (the Dynamic
    /// Island, always dark, draws that).
    public static let workingAdaptive = adaptive(light: workingOnLight, dark: working)
    public static let attentionAdaptive = adaptive(light: attentionOnLight, dark: attention)
    public static let doneAdaptive = adaptive(light: doneOnLight, dark: done)

    /// `light` where it is drawn in the light appearance, `dark` elsewhere.
    public static func adaptive(light: Color, dark: Color) -> Color {
        let (light, dark) = (UIColor(light), UIColor(dark))
        return Color(uiColor: UIColor { $0.userInterfaceStyle == .light ? light : dark })
    }
}
