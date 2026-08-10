import Foundation
import Observation

enum CursorShape: String, CaseIterable {
    case block, bar, underline
}

enum PredictMode: String, CaseIterable {
    case adaptive, always, never, experimental

    var label: String { rawValue.prefix(1).uppercased() + rawValue.dropFirst() }
}

/// User-tunable settings persisted in UserDefaults — exactly the controls on
/// the prototype's Settings screen (DISPLAY / CURSOR / BEHAVIOR /
/// MOSH·ROAMING / NOTIFICATIONS / VOICE INPUT).
///
/// `@AppStorage` cannot be used inside `@Observable`, so properties are
/// computed over UserDefaults with the access/withMutation pattern.
@Observable
final class AppSettings {
    @ObservationIgnored private let defaults: UserDefaults

    /// App-wide instance so non-View code (design tokens, icon switching)
    /// can read the active settings without environment injection.
    static let shared = AppSettings()

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    // MARK: Generic accessors

    private func get<T>(_ key: String, _ fallback: T) -> T {
        defaults.object(forKey: key) as? T ?? fallback
    }

    // MARK: DISPLAY

    var fontSize: Double {
        get { access(keyPath: \.fontSize); return get("moshpit.settings.fontSize", 9.0) }
        set { withMutation(keyPath: \.fontSize) { defaults.set(newValue, forKey: "moshpit.settings.fontSize") } }
    }

    var themeId: String {
        get { access(keyPath: \.themeId); return get("moshpit.settings.themeId", "github-dark") }
        set { withMutation(keyPath: \.themeId) { defaults.set(newValue, forKey: "moshpit.settings.themeId") } }
    }

    // MARK: APPEARANCE (app chrome theme, distinct from the terminal `themeId` above)

    /// One of `AppThemeCatalog.all[].id` (built-in or custom). Drives `Ink`'s
    /// accent-derived tokens app-wide. The app icon is a separate setting —
    /// see ``appIconId``.
    var appThemeId: String {
        get { access(keyPath: \.appThemeId); return get("moshpit.settings.appThemeId", AppThemeCatalog.signalRoom.id) }
        set { withMutation(keyPath: \.appThemeId) { defaults.set(newValue, forKey: "moshpit.settings.appThemeId") } }
    }

    /// One of `AppIconCatalog.all[].id`. Independent of ``appThemeId``: iOS only
    /// switches between icons bundled at build time, so an icon can't follow a
    /// custom accent.
    ///
    /// Installs that predate the split have no stored value, and their icon was
    /// implied by the app theme — the default reads through
    /// `AppIconCatalog.migratedID(fromAppThemeID:)` so those users keep the icon
    /// they already have on their home screen instead of silently reverting to
    /// the primary one.
    var appIconId: String {
        get {
            access(keyPath: \.appIconId)
            return get("moshpit.settings.appIconId",
                       AppIconCatalog.migratedID(fromAppThemeID: appThemeId))
        }
        set { withMutation(keyPath: \.appIconId) { defaults.set(newValue, forKey: "moshpit.settings.appIconId") } }
    }

    /// Monospaced font family for the terminal. "system" = the SF Mono system
    /// font; otherwise a `UIFont(name:)` family (see TerminalFont.families).
    var fontName: String {
        get { access(keyPath: \.fontName); return get("moshpit.settings.fontName", "system") }
        set { withMutation(keyPath: \.fontName) { defaults.set(newValue, forKey: "moshpit.settings.fontName") } }
    }

    // MARK: CURSOR

    var cursorShape: CursorShape {
        get {
            access(keyPath: \.cursorShape)
            return CursorShape(rawValue: get("moshpit.settings.cursorShape", "block")) ?? .block
        }
        set { withMutation(keyPath: \.cursorShape) { defaults.set(newValue.rawValue, forKey: "moshpit.settings.cursorShape") } }
    }

    /// "teal" | "green" | "white" | "accent"
    var cursorColorId: String {
        get { access(keyPath: \.cursorColorId); return get("moshpit.settings.cursorColor", "teal") }
        set { withMutation(keyPath: \.cursorColorId) { defaults.set(newValue, forKey: "moshpit.settings.cursorColor") } }
    }

    var cursorBlink: Bool {
        get { access(keyPath: \.cursorBlink); return get("moshpit.settings.cursorBlink", true) }
        set { withMutation(keyPath: \.cursorBlink) { defaults.set(newValue, forKey: "moshpit.settings.cursorBlink") } }
    }

    var trailOnPredict: Bool {
        get { access(keyPath: \.trailOnPredict); return get("moshpit.settings.trailOnPredict", true) }
        set { withMutation(keyPath: \.trailOnPredict) { defaults.set(newValue, forKey: "moshpit.settings.trailOnPredict") } }
    }

    // MARK: BEHAVIOR

    var keepConnectionsAlive: Bool {
        get { access(keyPath: \.keepConnectionsAlive); return get("moshpit.settings.keepAlive", true) }
        set { withMutation(keyPath: \.keepConnectionsAlive) { defaults.set(newValue, forKey: "moshpit.settings.keepAlive") } }
    }

    // MARK: MOSH · ROAMING

    var moshByDefault: Bool {
        get { access(keyPath: \.moshByDefault); return get("moshpit.settings.moshByDefault", true) }
        set { withMutation(keyPath: \.moshByDefault) { defaults.set(newValue, forKey: "moshpit.settings.moshByDefault") } }
    }

    var predictMode: PredictMode {
        get {
            access(keyPath: \.predictMode)
            return PredictMode(rawValue: get("moshpit.settings.predictMode", "adaptive")) ?? .adaptive
        }
        set { withMutation(keyPath: \.predictMode) { defaults.set(newValue.rawValue, forKey: "moshpit.settings.predictMode") } }
    }

    var moshServerBinary: String {
        get { access(keyPath: \.moshServerBinary); return get("moshpit.settings.moshServerBinary", "/opt/homebrew/bin/mosh-server") }
        set { withMutation(keyPath: \.moshServerBinary) { defaults.set(newValue, forKey: "moshpit.settings.moshServerBinary") } }
    }

    var udpRangeStart: Int {
        get { access(keyPath: \.udpRangeStart); return get("moshpit.settings.udpRangeStart", 60000) }
        set { withMutation(keyPath: \.udpRangeStart) { defaults.set(newValue, forKey: "moshpit.settings.udpRangeStart") } }
    }

    var udpRangeEnd: Int {
        get { access(keyPath: \.udpRangeEnd); return get("moshpit.settings.udpRangeEnd", 61000) }
        set { withMutation(keyPath: \.udpRangeEnd) { defaults.set(newValue, forKey: "moshpit.settings.udpRangeEnd") } }
    }

    // MARK: NOTIFICATIONS (Vibe Island)

    var notificationsEnabled: Bool {
        get { access(keyPath: \.notificationsEnabled); return get("moshpit.settings.notifications", true) }
        set { withMutation(keyPath: \.notificationsEnabled) { defaults.set(newValue, forKey: "moshpit.settings.notifications") } }
    }

    var liveActivityEnabled: Bool {
        get { access(keyPath: \.liveActivityEnabled); return get("moshpit.settings.liveActivity", true) }
        set { withMutation(keyPath: \.liveActivityEnabled) { defaults.set(newValue, forKey: "moshpit.settings.liveActivity") } }
    }

    /// Play a sound with the "agent needs you" alert. When off, attention
    /// notifications are silent.
    var attentionSoundEnabled: Bool {
        get { access(keyPath: \.attentionSoundEnabled); return get("moshpit.settings.attentionSound", true) }
        set { withMutation(keyPath: \.attentionSoundEnabled) { defaults.set(newValue, forKey: "moshpit.settings.attentionSound") } }
    }

    /// Show what the agent is doing/asking (the hook `@moshpit_title`, e.g.
    /// "Bash: npm install") on the Vibe Island, widgets, and notifications. These
    /// lock-screen surfaces are visible without unlocking, so turn this off to
    /// keep command/prompt text private (state working/needs-you/done still shows).
    var lockScreenDetailEnabled: Bool {
        get { access(keyPath: \.lockScreenDetailEnabled); return get("moshpit.settings.lockScreenDetail", true) }
        set { withMutation(keyPath: \.lockScreenDetailEnabled) { defaults.set(newValue, forKey: "moshpit.settings.lockScreenDetail") } }
    }

    // MARK: VOICE INPUT (EXPERIMENTAL)

    var voiceInputEnabled: Bool {
        get { access(keyPath: \.voiceInputEnabled); return get("moshpit.settings.voiceInput", false) }
        set { withMutation(keyPath: \.voiceInputEnabled) { defaults.set(newValue, forKey: "moshpit.settings.voiceInput") } }
    }
}
