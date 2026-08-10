import SwiftUI

/// The agent-state colours, fixed by SEMANTICS — one value per state, the
/// same value on every surface that renders one.
///
/// Lives in Island/ because it is compiled into BOTH targets (see the
/// MoshpitIsland sources list in project.yml): the widget can't import the
/// app's DesignTokens, and duplicating the constants is exactly how the
/// island's "working" spent a release as teal while the app's was indigo.
///
/// Deliberately NOT the theme accent: `Ink.accent` follows the user's
/// Appearance choice, and a user who picks an amber-ish accent would watch
/// "working" and "needs you" become the same colour. State colours carry
/// meaning, so they don't move with taste.
enum AgentPalette {
    /// Actively doing something — teal, matching the Live Activity timer.
    static let working = Color(red: 94 / 255, green: 227 / 255, blue: 217 / 255)
    /// Waiting on a human — amber. Same value as `Ink.warn`, asserted by test.
    static let attention = Color(red: 255 / 255, green: 179 / 255, blue: 92 / 255)
    /// Finished, not yet looked at — green. Same value as `Ink.success`.
    static let done = Color(red: 131 / 255, green: 229 / 255, blue: 140 / 255)
}
