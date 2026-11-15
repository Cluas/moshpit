import Foundation
import Observation
import MoshpitKit

enum CursorShape: String, CaseIterable {
    case block, bar, underline
}

/// The app's appearance (Settings › Appearance, R18): the system's, or
/// always light, or always dark. Read by the NSH screens' params
/// (`NSHScreenParams`); nothing changes a window's style by it until R18's
/// host part.
enum AppAppearance: String, CaseIterable {
    case system, light, dark
}

enum PredictMode: String, CaseIterable {
    case adaptive, always, never, experimental
}

/// User-tunable settings — exactly the controls on the prototype's Settings
/// screen (DISPLAY / CURSOR / BEHAVIOR / MOSH·ROAMING / NOTIFICATIONS /
/// VOICE INPUT). The Rust store (`crates/store/src/settings.rs`, through
/// ``AppSettingsBacking``) reads and writes each — the keys and stored
/// forms every build reads, the sealed-launch rule, the notification
/// service's mirror of its two switches (`PushPrefs`).
///
/// `@AppStorage` cannot be used inside `@Observable`, so properties are
/// computed over the backing with the access/withMutation pattern.
@Observable
final class AppSettings {
    @ObservationIgnored private let defaults: UserDefaults

    /// The preferences the settings live in, for collaborators that persist
    /// their own bookkeeping rather than a user-facing setting —
    /// `AgentActivityMonitor` keeps its "already announced" record here so a
    /// relaunch does not re-notify for prompts that are still standing.
    /// Exposed rather than duplicated so tests that hand in a scratch suite
    /// keep isolating everything.
    @ObservationIgnored var store: UserDefaults { defaults }

    // The app-wide instance, `shared` — so non-View code (design tokens, icon
    // switching) can read the active settings without environment injection —
    // is made with the app's other stores (`AppStores`).

    @ObservationIgnored private let backing: AppSettingsBacking

    /// The Rust store over `defaults`, readable as `readiness` says, with no
    /// notification-service mirror (tests and scratch suites; the app's is
    /// ``shared``).
    convenience init(defaults: UserDefaults = .standard, readiness: DefaultsReadiness = .always) {
        let host = RustPreferencesHost(defaults: defaults, available: readiness.protectedDataAvailable)
        #if DEBUG
        let debugBuild = true
        #else
        let debugBuild = false
        #endif
        self.init(defaults: defaults,
                  backing: RustSettings(preferences: host, mirror: nil, mode: .system, debugBuild: debugBuild))
    }

    /// Settings whose every read and write is `backing`'s, over the
    /// preferences in `defaults` (``store`` hands those out, for the
    /// bookkeeping that isn't a setting).
    init(defaults: UserDefaults = .standard, backing: AppSettingsBacking) {
        self.defaults = defaults
        self.backing = backing
        // Seed the notification service's mirror (PushPrefs) once the
        // preferences can be trusted: a pushed notification can land before
        // the user ever opens Settings, and it must see the values the
        // switches already hold.
        reloadIfNeeded()
    }

    /// Mirror the extension-visible switches once the preferences can be
    /// trusted; a no-op once that has happened.
    func reloadIfNeeded() {
        backing.reloadIfNeeded()
    }

    // MARK: Generic accessors

    private func get<T>(_ key: String, _ fallback: T) -> T {
        backing.value(forKey: key) as? T ?? fallback
    }

    // Store a value under its key, as each setter writes it: one overload
    // per type, each the stored form every build reads.

    private func put(_ value: Bool, _ key: String) { backing.set(value, forKey: key) }
    private func put(_ value: Int, _ key: String) { backing.set(value, forKey: key) }
    private func put(_ value: Double, _ key: String) { backing.set(value, forKey: key) }
    private func put(_ value: String, _ key: String) { backing.set(value, forKey: key) }
    private func put(_ value: [String], _ key: String) { backing.set(value, forKey: key) }

    // MARK: DISPLAY

    var fontSize: Double {
        get { access(keyPath: \.fontSize); return get("moshpit.settings.fontSize", 9.0) }
        set { withMutation(keyPath: \.fontSize) { put(newValue, "moshpit.settings.fontSize") } }
    }

    var themeId: String {
        get { access(keyPath: \.themeId); return get("moshpit.settings.themeId", "github-dark") }
        set { withMutation(keyPath: \.themeId) { put(newValue, "moshpit.settings.themeId") } }
    }

    // MARK: APPEARANCE (app chrome theme, distinct from the terminal `themeId` above)

    /// One of `AppThemeCatalog.all[].id` (built-in or custom). Drives `Ink`'s
    /// accent-derived tokens app-wide. The app icon is a separate setting —
    /// see ``appIconId``.
    var appThemeId: String {
        get { access(keyPath: \.appThemeId); return get("moshpit.settings.appThemeId", AppThemeCatalog.signalRoom.id) }
        set { withMutation(keyPath: \.appThemeId) { put(newValue, "moshpit.settings.appThemeId") } }
    }

    /// One of `AppIconCatalog.all[].id`. Independent of ``appThemeId``: iOS only
    /// switches between icons bundled at build time, so an icon can't follow a
    /// custom accent.
    var appIconId: String {
        get { access(keyPath: \.appIconId); return get("moshpit.settings.appIconId", AppIconCatalog.primary.id) }
        set { withMutation(keyPath: \.appIconId) { put(newValue, "moshpit.settings.appIconId") } }
    }

    /// The person picked the icon by hand in the gallery (B668): the pet's
    /// icon card then makes 「先不换」 the default. One picked before this was
    /// kept counts as picked by hand; the card's own switch doesn't.
    var appIconPickedByHand: Bool {
        get {
            access(keyPath: \.appIconPickedByHand)
            return get("moshpit.settings.appIconPickedByHand", appIconId != AppIconCatalog.primary.id)
        }
        set { withMutation(keyPath: \.appIconPickedByHand) { put(newValue, "moshpit.settings.appIconPickedByHand") } }
    }

    /// Monospaced font family for the terminal. "system" = the SF Mono system
    /// font; otherwise a `UIFont(name:)` family (see TerminalFont.families).
    var fontName: String {
        get { access(keyPath: \.fontName); return get("moshpit.settings.fontName", "system") }
        set { withMutation(keyPath: \.fontName) { put(newValue, "moshpit.settings.fontName") } }
    }

    // MARK: CURSOR

    var cursorShape: CursorShape {
        get {
            access(keyPath: \.cursorShape)
            return CursorShape(rawValue: get("moshpit.settings.cursorShape", "block")) ?? .block
        }
        set { withMutation(keyPath: \.cursorShape) { put(newValue.rawValue, "moshpit.settings.cursorShape") } }
    }

    /// "teal" | "green" | "white" | "accent"
    var cursorColorId: String {
        get { access(keyPath: \.cursorColorId); return get("moshpit.settings.cursorColor", "teal") }
        set { withMutation(keyPath: \.cursorColorId) { put(newValue, "moshpit.settings.cursorColor") } }
    }

    var cursorBlink: Bool {
        get { access(keyPath: \.cursorBlink); return get("moshpit.settings.cursorBlink", true) }
        set { withMutation(keyPath: \.cursorBlink) { put(newValue, "moshpit.settings.cursorBlink") } }
    }

    var trailOnPredict: Bool {
        get { access(keyPath: \.trailOnPredict); return get("moshpit.settings.trailOnPredict", true) }
        set {
            withMutation(keyPath: \.trailOnPredict) { put(newValue, "moshpit.settings.trailOnPredict") }
            NotificationCenter.default.post(name: .moshpitPredictionChanged, object: self)
        }
    }

    // MARK: BEHAVIOR

    /// Kept as stored, read by nothing (B516): a live session is probed
    /// every 12 s whatever this says (the probe is how a dead link is
    /// found). Its switch is gone; the key stays, unmigrated, so an older
    /// build still reads it harmlessly.
    var keepConnectionsAlive: Bool {
        get { access(keyPath: \.keepConnectionsAlive); return get("moshpit.settings.keepAlive", true) }
        set { withMutation(keyPath: \.keepConnectionsAlive) { put(newValue, "moshpit.settings.keepAlive") } }
    }

    // MARK: MOSH · ROAMING

    var moshByDefault: Bool {
        get { access(keyPath: \.moshByDefault); return get("moshpit.settings.moshByDefault", true) }
        set { withMutation(keyPath: \.moshByDefault) { put(newValue, "moshpit.settings.moshByDefault") } }
    }

    var predictMode: PredictMode {
        get {
            access(keyPath: \.predictMode)
            return PredictMode(rawValue: get("moshpit.settings.predictMode", "adaptive")) ?? .adaptive
        }
        set {
            withMutation(keyPath: \.predictMode) { put(newValue.rawValue, "moshpit.settings.predictMode") }
            NotificationCenter.default.post(name: .moshpitPredictionChanged, object: self)
        }
    }

    /// Kept as stored, read by nothing (R18 E): the setting that reaches
    /// mosh-server is each connection's own `moshServerPath`. Its row is gone;
    /// the key stays, unmigrated, so an older build still reads it harmlessly.
    var moshServerBinary: String {
        get { access(keyPath: \.moshServerBinary); return get("moshpit.settings.moshServerBinary", "/opt/homebrew/bin/mosh-server") }
        set { withMutation(keyPath: \.moshServerBinary) { put(newValue, "moshpit.settings.moshServerBinary") } }
    }

    /// mosh-server's UDP ports, for every connection: what a mosh connect
    /// asks for (B430), whatever a connection stored at its last save.
    var udpRange: (low: Int, high: Int) { (udpRangeStart, udpRangeEnd) }

    var udpRangeStart: Int {
        get { access(keyPath: \.udpRangeStart); return get("moshpit.settings.udpRangeStart", 60000) }
        set { withMutation(keyPath: \.udpRangeStart) { put(newValue, "moshpit.settings.udpRangeStart") } }
    }

    var udpRangeEnd: Int {
        get { access(keyPath: \.udpRangeEnd); return get("moshpit.settings.udpRangeEnd", 61000) }
        set { withMutation(keyPath: \.udpRangeEnd) { put(newValue, "moshpit.settings.udpRangeEnd") } }
    }

    // MARK: NOTIFICATIONS (Vibe Island)

    var notificationsEnabled: Bool {
        get {
            access(keyPath: \.notificationsEnabled)
            #if DEBUG
            if Self.quietForThisLaunch { return false }
            #endif
            return get("moshpit.settings.notifications", true)
        }
        set { withMutation(keyPath: \.notificationsEnabled) { put(newValue, "moshpit.settings.notifications") } }
    }

    #if DEBUG
    /// `-MOSHPIT_SEED_QUIET 1`: local notifications off for this launch, the
    /// stored switch left as it is. The smoke seed used to turn the switch
    /// itself off, for good (B200).
    static let quietForThisLaunch: Bool = {
        let args = ProcessInfo.processInfo.arguments
        guard let index = args.firstIndex(of: "-MOSHPIT_SEED_QUIET"), index + 1 < args.count else { return false }
        return args[index + 1] == "1"
    }()
    #endif

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
            let key = "moshpit.settings.hostSetupDeclined"
            // The strings of an array that holds something else too (B159):
            // `stringArray(forKey:)` read it as no array at all, and the next
            // host declined stored that nothing plus the one. The setter
            // writes the strings only, so every build reads it again.
            let stored = backing.value(forKey: key)
            return Set((stored as? [Any])?.compactMap { $0 as? String } ?? [])
        }
        set {
            withMutation(keyPath: \.hostSetupDeclined) {
                put(Array(newValue).sorted(), "moshpit.settings.hostSetupDeclined")
            }
        }
    }

    /// Connections the user unpaired in Host Setup. Auto-care pairs nothing
    /// for them until the user pairs again there (B371); the core says when
    /// to add or drop one.
    var hostSetupUnpaired: Set<String> {
        get {
            access(keyPath: \.hostSetupUnpaired)
            let key = "moshpit.settings.hostSetupUnpaired"
            let stored = backing.value(forKey: key)
            return Set((stored as? [Any])?.compactMap { $0 as? String } ?? [])
        }
        set {
            withMutation(keyPath: \.hostSetupUnpaired) {
                put(Array(newValue).sorted(), "moshpit.settings.hostSetupUnpaired")
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
        set { withMutation(keyPath: \.pushRelayURL) { put(newValue, "moshpit.settings.pushRelay") } }
    }

    var liveActivityEnabled: Bool {
        get { access(keyPath: \.liveActivityEnabled); return get("moshpit.settings.liveActivity", true) }
        set { withMutation(keyPath: \.liveActivityEnabled) { put(newValue, "moshpit.settings.liveActivity") } }
    }

    /// Play a sound with the "agent needs you" alert. When off, attention
    /// notifications are silent.
    var attentionSoundEnabled: Bool {
        get { access(keyPath: \.attentionSoundEnabled); return get("moshpit.settings.attentionSound", true) }
        set {
            withMutation(keyPath: \.attentionSoundEnabled) { put(newValue, "moshpit.settings.attentionSound") }
        }
    }

    /// Show what the agent is doing/asking (the hook `@moshpit_title`, e.g.
    /// "Bash: npm install") on the Vibe Island, widgets, and notifications. These
    /// lock-screen surfaces are visible without unlocking, so turn this off to
    /// keep command/prompt text private (state working/needs-you/done still shows).
    var lockScreenDetailEnabled: Bool {
        get { access(keyPath: \.lockScreenDetailEnabled); return get("moshpit.settings.lockScreenDetail", true) }
        set {
            withMutation(keyPath: \.lockScreenDetailEnabled) { put(newValue, "moshpit.settings.lockScreenDetail") }
        }
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
                put(newValue, "moshpit.settings.raiseKeyboardOnOpen")
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
                put(newValue, "moshpit.settings.remoteClipboardRead")
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
                put(newValue, "moshpit.settings.uploadRetentionDays")
            }
        }
    }

    // MARK: VOICE INPUT

    /// Shows the mic key on the terminal shortcut bar. On by default — the
    /// permission prompts only fire on first actual use, so the key costs
    /// nothing until tapped.
    var voiceInputEnabled: Bool {
        get { access(keyPath: \.voiceInputEnabled); return get("moshpit.settings.voiceInput", true) }
        set { withMutation(keyPath: \.voiceInputEnabled) { put(newValue, "moshpit.settings.voiceInput") } }
    }

    /// BCP-47 identifier of the dictation language ("en-US", "zh-CN"…).
    /// Empty = Automatic. Applies to the Apple engines only — Whisper has its
    /// own code list and its own setting, because the two catalogs neither
    /// overlap nor use the same identifiers.
    var voiceInputLocaleId: String {
        get { access(keyPath: \.voiceInputLocaleId); return get("moshpit.settings.voiceInputLocale", "") }
        set { withMutation(keyPath: \.voiceInputLocaleId) { put(newValue, "moshpit.settings.voiceInputLocale") } }
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
                put(newValue.rawValue, "moshpit.settings.voiceEngine")
            }
        }
    }

    /// Whisper language code ("zh", "en"…) the user pinned. Empty = let
    /// Whisper detect it from the audio, which beat pinning even on mixed
    /// Chinese–English speech (B670: 16.1% against 30.2% error on 50 real
    /// clips, docs/nsh/ASR-2026-11.md).
    ///
    /// Stored under a new key. The old `moshpit.settings.whisperLanguage` held
    /// the language the app preselected the first time Whisper was picked,
    /// indistinguishable from one the user chose, so every install starts on
    /// auto-detect once; 1.0.8 still reads the old key.
    var whisperLanguage: String {
        get { access(keyPath: \.whisperLanguage); return get("moshpit.settings.whisperLanguagePinned", "") }
        set { withMutation(keyPath: \.whisperLanguage) { put(newValue, "moshpit.settings.whisperLanguagePinned") } }
    }

    /// WhisperKit model variant. Empty = use whichever supported model is
    /// installed, so deleting the chosen one degrades instead of breaking.
    var whisperModelId: String {
        get { access(keyPath: \.whisperModelId); return get("moshpit.settings.whisperModel", "") }
        set { withMutation(keyPath: \.whisperModelId) { put(newValue, "moshpit.settings.whisperModel") } }
    }

    /// The app's appearance; the system's when nothing (or nothing known) is
    /// stored. No migration: an install from before R18 follows the system.
    var appearance: AppAppearance {
        get {
            access(keyPath: \.appearance)
            return AppAppearance(rawValue: get("moshpit.settings.appearance", AppAppearance.system.rawValue))
                ?? .system
        }
        set { withMutation(keyPath: \.appearance) { put(newValue.rawValue, "moshpit.settings.appearance") } }
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

/// What ``AppSettings`` reads and writes through: the Rust store
/// (crates/store, through crates/ffi; ``RustSettings``), or a test's
/// stand-in. By preferences key, as the settings' getters and setters go: a
/// value read is what is stored if it is of the setting's type, else the
/// setting's default (an enum-backed one's raw string, which its getter
/// checks); a value written is the setter's — and the notification
/// service's mirror follows its two switches.
protocol AppSettingsBacking: AnyObject {
    /// `Bool`, `Int`, `Double`, `String` or `[String]`; nil for a key that
    /// is no setting's.
    func value(forKey key: String) -> Any?
    func set(_ value: Any, forKey key: String)
    /// Mirror the switches once the preferences can be trusted.
    func reloadIfNeeded()
}

extension Notification.Name {
    /// Settings' predictive echo or its trail changed (B533): a live mosh
    /// session takes it at once, not at its next connect.
    static let moshpitPredictionChanged = Notification.Name("moshpit.predictionChanged")
}
