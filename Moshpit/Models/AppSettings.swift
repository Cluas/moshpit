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

    /// The same store, for collaborators that persist their own bookkeeping
    /// rather than a user-facing setting — `AgentActivityMonitor` keeps its
    /// "already announced" record here so a relaunch does not re-notify for
    /// prompts that are still standing. Exposed rather than duplicated so tests
    /// that hand in a scratch suite keep isolating everything.
    @ObservationIgnored var store: UserDefaults { defaults }

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

    /// Relay a paired host is pointed at.
    ///
    /// Empty until the user sets one: there is no default that could be right,
    /// and guessing at a host would be worse than asking.
    /// The push relay to pair against. Defaults to the hosted one, because for
    /// an App Store install there is no other that can work: the relay's whole
    /// power is the APNs `.p8` signing key for THIS bundle id, and only ours
    /// holds it. The field stays editable for the two audiences it serves —
    /// development (the simulator harness points it at 127.0.0.1) and people
    /// who build the app from source under their own team, sign with their own
    /// key, and run `push-relay/` themselves. Message content is sealed either
    /// way; a relay, ours included, sees ciphertext and routing hashes.
    /// Connections whose "enable agent notifications on this host?" question the
    /// user answered NO to. Auto-care never asks those again; the Host Setup
    /// sheet remains the door for changing their mind.
    var hostSetupDeclined: Set<String> {
        get {
            access(keyPath: \.hostSetupDeclined)
            return Set(defaults.stringArray(forKey: "moshpit.settings.hostSetupDeclined") ?? [])
        }
        set {
            withMutation(keyPath: \.hostSetupDeclined) {
                defaults.set(Array(newValue).sorted(), forKey: "moshpit.settings.hostSetupDeclined")
            }
        }
    }

    /// The one relay this build can use. HARDCODED, not defaulted: an earlier
    /// version stored a user-editable value and fell back to the constant only
    /// when nothing was stored — so the one time a stray edit dropped the
    /// ".org" suffix, the typo was persisted, the fallback never fired again,
    /// and (once the editing UI was removed) there was no way back at all. A
    /// source builder running their own relay changes THIS line, together with
    /// the signing key that makes their relay real.
    nonisolated static let defaultPushRelay = "https://push.moshpit.cluas.eu.org"

    /// What every pairing and registration uses. In Release this IS the
    /// constant. In DEBUG a stored override is honored so a harness can point a
    /// simulator at a local relay (`simctl spawn <sim> defaults write
    /// com.cluas.moshpit moshpit.settings.pushRelay http://127.0.0.1:PORT`) —
    /// and ONLY for loopback hosts. Not "any valid URL": the developer's own
    /// devices run Debug builds permanently, and the typo that motivated all of
    /// this ("…cluas.eu", org gone) parses as a perfectly valid URL. A harness
    /// needs the loopback; nothing legitimate needs anything else.
    var pushRelayURL: String {
        get {
            access(keyPath: \.pushRelayURL)
            #if DEBUG
            let stored = get("moshpit.settings.pushRelay", "")
            if let url = URL(string: stored), let host = url.host,
               ["127.0.0.1", "localhost", "::1"].contains(host) {
                return stored
            }
            #endif
            return Self.defaultPushRelay
        }
        set { withMutation(keyPath: \.pushRelayURL) { defaults.set(newValue, forKey: "moshpit.settings.pushRelay") } }
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

    /// Raise the software keyboard the moment a terminal opens.
    ///
    /// Off by default: opening a session is usually to read it, and a keyboard
    /// that appears uninvited covers most of the scrollback you came for.
    /// Tapping the terminal raises it, so the cost of leaving it down is one
    /// tap — where the old always-on behavior cost a dismissal every time.
    var raiseKeyboardOnOpen: Bool {
        get { access(keyPath: \.raiseKeyboardOnOpen); return get("moshpit.settings.raiseKeyboardOnOpen", false) }
        set {
            withMutation(keyPath: \.raiseKeyboardOnOpen) {
                defaults.set(newValue, forKey: "moshpit.settings.raiseKeyboardOnOpen")
            }
        }
    }

    /// Answer OSC 52 clipboard READ queries from remote programs with the
    /// phone's clipboard text.
    ///
    /// Off by default, and this is a security posture, not a preference:
    /// with it on, any program on any connected server can silently read
    /// whatever is on the clipboard — passwords, 2FA codes, the works.
    /// Off answers every query with an empty clipboard (the conventional
    /// refusal), so remote programs get an answer instead of a hang, just
    /// never the contents.
    var remoteClipboardReadEnabled: Bool {
        get { access(keyPath: \.remoteClipboardReadEnabled); return get("moshpit.settings.remoteClipboardRead", false) }
        set {
            withMutation(keyPath: \.remoteClipboardReadEnabled) {
                defaults.set(newValue, forKey: "moshpit.settings.remoteClipboardRead")
            }
        }
    }

    /// How long uploaded images stay in the server's `~/.moshpit/uploads/`
    /// before the connect-time sweep deletes them. Days; 0 = never delete.
    ///
    /// Default 7: uploads are working files for an agent, not a photo
    /// backup, and a short retention is the right privacy posture on a
    /// server other people may read. 1 (≈24h) for shared boxes, 0 for
    /// people who treat the folder as their own.
    var uploadRetentionDays: Int {
        get { access(keyPath: \.uploadRetentionDays); return get("moshpit.settings.uploadRetentionDays", 7) }
        set {
            withMutation(keyPath: \.uploadRetentionDays) {
                defaults.set(newValue, forKey: "moshpit.settings.uploadRetentionDays")
            }
        }
    }

    // MARK: VOICE INPUT

    /// Shows the mic key on the terminal shortcut bar. On by default — the
    /// permission prompts only fire on first actual use, so the key costs
    /// nothing until tapped.
    var voiceInputEnabled: Bool {
        get { access(keyPath: \.voiceInputEnabled); return get("moshpit.settings.voiceInput", true) }
        set { withMutation(keyPath: \.voiceInputEnabled) { defaults.set(newValue, forKey: "moshpit.settings.voiceInput") } }
    }

    /// BCP-47 identifier of the dictation language ("en-US", "zh-CN"…).
    /// Empty = Automatic. Applies to the Apple engines only — Whisper has its
    /// own code list and its own setting, because the two catalogs neither
    /// overlap nor use the same identifiers.
    var voiceInputLocaleId: String {
        get { access(keyPath: \.voiceInputLocaleId); return get("moshpit.settings.voiceInputLocale", "") }
        set { withMutation(keyPath: \.voiceInputLocaleId) { defaults.set(newValue, forKey: "moshpit.settings.voiceInputLocale") } }
    }

    /// Which speech engine dictation runs on. Defaults to Apple: it needs no
    /// download and works on a fresh install, whereas Whisper is only usable
    /// once several hundred megabytes have been fetched deliberately.
    var voiceEngine: VoiceEngineKind {
        get {
            access(keyPath: \.voiceEngine)
            return VoiceEngineKind(rawValue: get("moshpit.settings.voiceEngine", VoiceEngineKind.apple.rawValue))
                ?? .apple
        }
        set {
            withMutation(keyPath: \.voiceEngine) {
                defaults.set(newValue.rawValue, forKey: "moshpit.settings.voiceEngine")
            }
        }
    }

    /// Whisper language code ("zh", "en"…). Empty = let Whisper detect it
    /// from the audio, which is a better default than any guess from system
    /// configuration.
    var whisperLanguage: String {
        get { access(keyPath: \.whisperLanguage); return get("moshpit.settings.whisperLanguage", "") }
        set { withMutation(keyPath: \.whisperLanguage) { defaults.set(newValue, forKey: "moshpit.settings.whisperLanguage") } }
    }

    /// WhisperKit model variant. Empty = use whichever supported model is
    /// installed, so deleting the chosen one degrades instead of breaking.
    var whisperModelId: String {
        get { access(keyPath: \.whisperModelId); return get("moshpit.settings.whisperModel", "") }
        set { withMutation(keyPath: \.whisperModelId) { defaults.set(newValue, forKey: "moshpit.settings.whisperModel") } }
    }

    /// Snapshot of everything a dictation session needs, taken when the mic
    /// key is tapped.
    var dictationRequest: DictationRequest {
        DictationRequest(
            engine: voiceEngine,
            appleLocaleId: voiceInputLocaleId,
            whisperLanguage: whisperLanguage,
            whisperModelId: whisperModelId)
    }
}
