//! The user's settings (`AppSettings`): one preferences key each, read live.
//!
//! Every setting is read the way the app reads it — `object(forKey:) as? T`
//! with the fallback when nothing (or something of another type) is stored
//! ([`crate::plist`] has the bridging rules) — and written as the type the
//! app writes. Reads never write, so a launch before the first unlock heals
//! itself the moment the preferences are readable; the one thing that
//! writes on its own is the mirror of the two switches the notification
//! service reads, which waits for a read that can be trusted.

use std::sync::Arc;

use crate::connections::PredictMode;
use crate::host::Defaults;
use crate::plist::{self, Plist};
use crate::readiness::Readiness;
use crate::swift;

/// The cursor's shape (`CursorShape`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
}

impl CursorShape {
    pub const ALL: [Self; 3] = [Self::Block, Self::Bar, Self::Underline];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Bar => "bar",
            Self::Underline => "underline",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.raw_value() == raw)
    }
}

/// Which speech engine dictation runs on (`VoiceEngineKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoiceEngine {
    Apple,
    Whisper,
}

impl VoiceEngine {
    pub const ALL: [Self; 2] = [Self::Apple, Self::Whisper];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Apple => "apple",
            Self::Whisper => "whisper",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.raw_value() == raw)
    }
}

/// The app's appearance (Settings › Appearance, R18): the system's, or
/// always light, or always dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Appearance {
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.raw_value() == raw)
    }
}

/// The type a setting is stored as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Double,
    String,
    /// An array of strings, sorted, read as a set.
    StringSet,
}

/// A setting's value.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Double(f64),
    String(String),
    StringSet(Vec<String>),
}

/// One setting: its name in `AppSettings`, its key, its type and the value
/// read when nothing of that type is stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spec {
    pub name: &'static str,
    pub key: &'static str,
    pub kind: Kind,
    default: Default,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Default {
    Bool(bool),
    Int(i64),
    Double(f64),
    String(&'static str),
    Empty,
}

impl Spec {
    pub fn default_value(&self) -> SettingValue {
        match self.default {
            Default::Bool(flag) => SettingValue::Bool(flag),
            Default::Int(value) => SettingValue::Int(value),
            Default::Double(value) => SettingValue::Double(value),
            Default::String(text) => SettingValue::String(text.to_owned()),
            Default::Empty => SettingValue::StringSet(Vec::new()),
        }
    }
}

const fn spec(name: &'static str, key: &'static str, kind: Kind, default: Default) -> Spec {
    Spec {
        name,
        key,
        kind,
        default,
    }
}

/// "Show detail on lock screen" — mirrored for the notification service.
pub const LOCK_SCREEN_DETAIL_KEY: &str = "moshpit.settings.lockScreenDetail";
/// "Alert sound" — mirrored for the notification service.
pub const ATTENTION_SOUND_KEY: &str = "moshpit.settings.attentionSound";
/// The relay a debug build may be pointed at (loopback only).
pub const PUSH_RELAY_KEY: &str = "moshpit.settings.pushRelay";
/// The one relay a release build uses (`AppSettings.defaultPushRelay`).
pub const DEFAULT_PUSH_RELAY: &str = "https://push.moshpit.cluas.eu.org";

/// Every setting of `AppSettings` at the app's committed `HEAD`, in its
/// order. (Not yet here: `moshpit.settings.showHiddenFiles`, a Bool
/// defaulting to false, in uncommitted work on the main tree.)
pub const ALL: [Spec; 32] = [
    spec(
        "fontSize",
        "moshpit.settings.fontSize",
        Kind::Double,
        Default::Double(9.0),
    ),
    spec(
        "themeId",
        "moshpit.settings.themeId",
        Kind::String,
        Default::String("github-dark"),
    ),
    spec(
        "appThemeId",
        "moshpit.settings.appThemeId",
        Kind::String,
        Default::String("signal-room"),
    ),
    spec(
        "appIconId",
        "moshpit.settings.appIconId",
        Kind::String,
        Default::String("default"),
    ),
    spec(
        "fontName",
        "moshpit.settings.fontName",
        Kind::String,
        Default::String("system"),
    ),
    spec(
        "cursorShape",
        "moshpit.settings.cursorShape",
        Kind::String,
        Default::String("block"),
    ),
    spec(
        "cursorColorId",
        "moshpit.settings.cursorColor",
        Kind::String,
        Default::String("teal"),
    ),
    spec(
        "cursorBlink",
        "moshpit.settings.cursorBlink",
        Kind::Bool,
        Default::Bool(true),
    ),
    spec(
        "trailOnPredict",
        "moshpit.settings.trailOnPredict",
        Kind::Bool,
        Default::Bool(true),
    ),
    spec(
        "keepConnectionsAlive",
        "moshpit.settings.keepAlive",
        Kind::Bool,
        Default::Bool(true),
    ),
    spec(
        "moshByDefault",
        "moshpit.settings.moshByDefault",
        Kind::Bool,
        Default::Bool(true),
    ),
    spec(
        "predictMode",
        "moshpit.settings.predictMode",
        Kind::String,
        Default::String("adaptive"),
    ),
    spec(
        "moshServerBinary",
        "moshpit.settings.moshServerBinary",
        Kind::String,
        Default::String("/opt/homebrew/bin/mosh-server"),
    ),
    spec(
        "udpRangeStart",
        "moshpit.settings.udpRangeStart",
        Kind::Int,
        Default::Int(60000),
    ),
    spec(
        "udpRangeEnd",
        "moshpit.settings.udpRangeEnd",
        Kind::Int,
        Default::Int(61000),
    ),
    spec(
        "notificationsEnabled",
        "moshpit.settings.notifications",
        Kind::Bool,
        Default::Bool(true),
    ),
    spec(
        "hostSetupDeclined",
        "moshpit.settings.hostSetupDeclined",
        Kind::StringSet,
        Default::Empty,
    ),
    spec(
        "pushRelayURL",
        PUSH_RELAY_KEY,
        Kind::String,
        Default::String(DEFAULT_PUSH_RELAY),
    ),
    spec(
        "liveActivityEnabled",
        "moshpit.settings.liveActivity",
        Kind::Bool,
        Default::Bool(true),
    ),
    spec(
        "attentionSoundEnabled",
        ATTENTION_SOUND_KEY,
        Kind::Bool,
        Default::Bool(true),
    ),
    spec(
        "lockScreenDetailEnabled",
        LOCK_SCREEN_DETAIL_KEY,
        Kind::Bool,
        Default::Bool(true),
    ),
    spec(
        "raiseKeyboardOnOpen",
        "moshpit.settings.raiseKeyboardOnOpen",
        Kind::Bool,
        Default::Bool(false),
    ),
    spec(
        "remoteClipboardReadEnabled",
        "moshpit.settings.remoteClipboardRead",
        Kind::Bool,
        Default::Bool(false),
    ),
    spec(
        "uploadRetentionDays",
        "moshpit.settings.uploadRetentionDays",
        Kind::Int,
        Default::Int(7),
    ),
    spec(
        "voiceInputEnabled",
        "moshpit.settings.voiceInput",
        Kind::Bool,
        Default::Bool(true),
    ),
    spec(
        "voiceInputLocaleId",
        "moshpit.settings.voiceInputLocale",
        Kind::String,
        Default::String(""),
    ),
    spec(
        "voiceEngine",
        "moshpit.settings.voiceEngine",
        Kind::String,
        Default::String("apple"),
    ),
    spec(
        "whisperLanguage",
        "moshpit.settings.whisperLanguage",
        Kind::String,
        Default::String(""),
    ),
    // B670: the language Whisper is pinned to, empty for auto-detect. A new
    // key rather than the old `whisperLanguage`, which held the language the
    // app itself preselected the first time Whisper was picked: pinning it
    // measured worse than detection on mixed Chinese–English speech, so
    // every install starts on auto-detect once. The old key stays for 1.0.8.
    spec(
        "whisperLanguagePinned",
        "moshpit.settings.whisperLanguagePinned",
        Kind::String,
        Default::String(""),
    ),
    spec(
        "whisperModelId",
        "moshpit.settings.whisperModel",
        Kind::String,
        Default::String(""),
    ),
    // R18: the system's appearance unless chosen (no migration: an install
    // from before it follows the system too).
    spec(
        "appearance",
        "moshpit.settings.appearance",
        Kind::String,
        Default::String("system"),
    ),
    // B510: the connections the person unpaired in Host Setup, which
    // auto-care leaves unpaired (B371). Missing from this table, the Rust
    // store refused the write and read it back empty, so with the Rust store
    // on an Unpair lasted until the next connect, which paired again.
    spec(
        "hostSetupUnpaired",
        "moshpit.settings.hostSetupUnpaired",
        Kind::StringSet,
        Default::Empty,
    ),
];

/// The settings 1.0.8 did not have: its view of what the app writes
/// leaves them out.
pub const SINCE_108: [&str; 3] = ["appearance", "hostSetupUnpaired", "whisperLanguagePinned"];

/// The setting called `name` in `AppSettings`.
pub fn spec_named(name: &str) -> Option<&'static Spec> {
    ALL.iter().find(|s| s.name == name)
}

/// The settings (`AppSettings`).
pub struct Settings {
    defaults: Arc<dyn Defaults>,
    /// Where the notification service reads its two switches (iOS: the App
    /// Group's preferences); `None` when the platform has no such mirror.
    mirror: Option<Arc<dyn Defaults>>,
    readiness: Readiness,
    is_authoritative: bool,
    /// Whether this is a debug build, which honours a loopback relay.
    debug: bool,
}

impl Settings {
    /// Mirrors the two switches straight away when the preferences can be
    /// trusted, as the app's `init` does.
    pub fn new(
        defaults: Arc<dyn Defaults>,
        mirror: Option<Arc<dyn Defaults>>,
        readiness: Readiness,
    ) -> Self {
        let mut settings = Self {
            defaults,
            mirror,
            readiness,
            is_authoritative: false,
            debug: false,
        };
        settings.reload_if_needed();
        settings
    }

    /// Honour a loopback relay stored under [`PUSH_RELAY_KEY`], as a DEBUG
    /// build of the app does.
    pub fn set_debug_build(&mut self, debug: bool) {
        self.debug = debug;
    }

    /// Mirror the switches once the preferences can be trusted; a no-op
    /// once that has happened.
    pub fn reload_if_needed(&mut self) {
        if self.is_authoritative || !self.readiness.is_readable(self.defaults.as_ref()) {
            return;
        }
        self.is_authoritative = true;
        self.readiness.mark_readable(self.defaults.as_ref());
        self.mirror_push_prefs();
    }

    /// The two switches as the notification service should see them.
    pub fn push_prefs(&self) -> moshpit_seal::Prefs {
        let stored = |key: &str| self.defaults.get(key).and_then(|v| v.as_bool());
        moshpit_seal::Prefs::from_stored(
            stored(LOCK_SCREEN_DETAIL_KEY),
            stored(ATTENTION_SOUND_KEY),
        )
    }

    fn mirror_push_prefs(&self) {
        let Some(mirror) = &self.mirror else {
            return;
        };
        let prefs = self.push_prefs();
        mirror.set(
            moshpit_seal::prefs::DETAIL_KEY,
            Some(Plist::Bool(prefs.show_detail)),
        );
        mirror.set(
            moshpit_seal::prefs::SOUND_KEY,
            Some(Plist::Bool(prefs.sound)),
        );
    }

    /// One setting as `AppSettings` reads it: what is stored, if it is of the
    /// setting's type, else the default. (An enum-backed setting reads its
    /// stored raw string here; the typed getters check it.)
    pub fn get(&self, spec: &Spec) -> SettingValue {
        let stored = self.defaults.get(spec.key);
        let value = match spec.kind {
            Kind::Bool => stored.and_then(|v| v.as_bool()).map(SettingValue::Bool),
            Kind::Int => stored.and_then(|v| v.as_int()).map(SettingValue::Int),
            Kind::Double => stored.and_then(|v| v.as_double()).map(SettingValue::Double),
            Kind::String => stored
                .and_then(|v| v.as_str().map(str::to_owned))
                .map(SettingValue::String),
            // Fix (B159): the strings of an array that holds something else
            // too. `stringArray(forKey:)` read it as nothing, and the next
            // write (one more host declined) stored that nothing plus the one.
            // The write stores the strings only, so every build reads the
            // array again.
            Kind::StringSet => Some(SettingValue::StringSet(swift::dedup(
                plist::strings_in(stored.as_ref()).unwrap_or_default(),
            ))),
        };
        value.unwrap_or_else(|| spec.default_value())
    }

    /// One setting as its setter writes it. A value of another type than the
    /// setting's is refused.
    pub fn set(&self, spec: &Spec, value: SettingValue) -> bool {
        let stored = match (spec.kind, value) {
            (Kind::Bool, SettingValue::Bool(flag)) => Plist::Bool(flag),
            (Kind::Int, SettingValue::Int(number)) => Plist::Int(number),
            (Kind::Double, SettingValue::Double(number)) => Plist::Double(number),
            (Kind::String, SettingValue::String(text)) => Plist::String(text),
            (Kind::StringSet, SettingValue::StringSet(strings)) => {
                // `Array(set).sorted()`.
                let mut strings = swift::dedup(strings);
                strings.sort_by(|a, b| swift::cmp(a, b));
                Plist::Array(strings.into_iter().map(Plist::String).collect())
            }
            _ => return false,
        };
        self.defaults.set(spec.key, Some(stored));
        if spec.key == LOCK_SCREEN_DETAIL_KEY || spec.key == ATTENTION_SOUND_KEY {
            self.mirror_push_prefs();
        }
        true
    }

    fn string(&self, name: &str) -> String {
        match self.get(spec_named(name).expect("a known setting")) {
            SettingValue::String(text) => text,
            _ => unreachable!("{name} is a string"),
        }
    }

    fn bool(&self, name: &str) -> bool {
        match self.get(spec_named(name).expect("a known setting")) {
            SettingValue::Bool(flag) => flag,
            _ => unreachable!("{name} is a Boolean"),
        }
    }

    fn int(&self, name: &str) -> i64 {
        match self.get(spec_named(name).expect("a known setting")) {
            SettingValue::Int(number) => number,
            _ => unreachable!("{name} is an Int"),
        }
    }

    fn put(&self, name: &str, value: SettingValue) {
        self.set(spec_named(name).expect("a known setting"), value);
    }

    pub fn font_size(&self) -> f64 {
        match self.get(spec_named("fontSize").expect("known")) {
            SettingValue::Double(size) => size,
            _ => unreachable!(),
        }
    }

    pub fn set_font_size(&self, size: f64) {
        self.put("fontSize", SettingValue::Double(size));
    }

    pub fn theme_id(&self) -> String {
        self.string("themeId")
    }

    pub fn app_theme_id(&self) -> String {
        self.string("appThemeId")
    }

    /// The icon's id; the primary's when none was chosen (B13: not derived
    /// from the accent theme any more).
    pub fn app_icon_id(&self) -> String {
        self.string("appIconId")
    }

    pub fn cursor_shape(&self) -> CursorShape {
        CursorShape::from_raw(&self.string("cursorShape")).unwrap_or(CursorShape::Block)
    }

    pub fn set_cursor_shape(&self, shape: CursorShape) {
        self.put(
            "cursorShape",
            SettingValue::String(shape.raw_value().to_owned()),
        );
    }

    pub fn predict_mode(&self) -> PredictMode {
        PredictMode::from_raw(&self.string("predictMode")).unwrap_or(PredictMode::Adaptive)
    }

    pub fn set_predict_mode(&self, mode: PredictMode) {
        self.put(
            "predictMode",
            SettingValue::String(mode.raw_value().to_owned()),
        );
    }

    pub fn voice_engine(&self) -> VoiceEngine {
        VoiceEngine::from_raw(&self.string("voiceEngine")).unwrap_or(VoiceEngine::Apple)
    }

    pub fn set_voice_engine(&self, engine: VoiceEngine) {
        self.put(
            "voiceEngine",
            SettingValue::String(engine.raw_value().to_owned()),
        );
    }

    /// The appearance chosen; the system's when nothing (or nothing it
    /// knows) is stored.
    pub fn appearance(&self) -> Appearance {
        Appearance::from_raw(&self.string("appearance")).unwrap_or(Appearance::System)
    }

    pub fn set_appearance(&self, appearance: Appearance) {
        self.put(
            "appearance",
            SettingValue::String(appearance.raw_value().to_owned()),
        );
    }

    pub fn udp_range(&self) -> (i64, i64) {
        (self.int("udpRangeStart"), self.int("udpRangeEnd"))
    }

    pub fn lock_screen_detail_enabled(&self) -> bool {
        self.bool("lockScreenDetailEnabled")
    }

    pub fn attention_sound_enabled(&self) -> bool {
        self.bool("attentionSoundEnabled")
    }

    /// Connections whose "agent notifications on this host?" the user
    /// answered no to.
    pub fn host_setup_declined(&self) -> Vec<String> {
        match self.get(spec_named("hostSetupDeclined").expect("known")) {
            SettingValue::StringSet(ids) => ids,
            _ => unreachable!(),
        }
    }

    pub fn set_host_setup_declined(&self, ids: Vec<String>) {
        self.put("hostSetupDeclined", SettingValue::StringSet(ids));
    }

    /// Connections the person unpaired in Host Setup (B371, B510).
    pub fn host_setup_unpaired(&self) -> Vec<String> {
        match self.get(spec_named("hostSetupUnpaired").expect("known")) {
            SettingValue::StringSet(ids) => ids,
            _ => unreachable!(),
        }
    }

    pub fn set_host_setup_unpaired(&self, ids: Vec<String>) {
        self.put("hostSetupUnpaired", SettingValue::StringSet(ids));
    }

    /// The relay every pairing uses: the constant, or in a debug build a
    /// stored loopback URL (`127.0.0.1`, `localhost`, `::1`).
    pub fn push_relay_url(&self) -> String {
        if self.debug
            && let SettingValue::String(stored) =
                self.get(spec_named("pushRelayURL").expect("known"))
            && url_host(&stored)
                .is_some_and(|host| ["127.0.0.1", "localhost", "::1"].contains(&host))
        {
            return stored;
        }
        DEFAULT_PUSH_RELAY.to_owned()
    }
}

/// The host of an absolute URL (`URL(string:)?.host`, for the loopback
/// check): between `scheme://` (after any `user@`) and the port, path,
/// query or fragment; an IPv6 address without its brackets.
fn url_host(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if let Some(bracketed) = host_port.strip_prefix('[') {
        return bracketed.split_once(']').map(|(host, _)| host);
    }
    let host = host_port.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::MemoryDefaults;

    fn settings() -> (Arc<MemoryDefaults>, Settings) {
        let defaults = Arc::new(MemoryDefaults::new());
        let settings = Settings::new(defaults.clone(), None, Readiness::Always);
        (defaults, settings)
    }

    /// `AppSettingsTests`.
    #[test]
    fn defaults_match_the_prototype() {
        let (_, s) = settings();
        assert_eq!(s.font_size(), 9.0);
        assert_eq!(s.theme_id(), "github-dark");
        assert_eq!(s.cursor_shape(), CursorShape::Block);
        assert_eq!(
            s.get(spec_named("cursorColorId").unwrap()),
            SettingValue::String("teal".into())
        );
        assert!(s.bool("cursorBlink") && s.bool("trailOnPredict") && s.bool("moshByDefault"));
        assert_eq!(s.predict_mode(), PredictMode::Adaptive);
        assert_eq!(s.udp_range(), (60000, 61000));
        assert_eq!(s.app_icon_id(), "default");
        assert_eq!(s.push_relay_url(), DEFAULT_PUSH_RELAY);
        assert_eq!(s.voice_engine(), VoiceEngine::Apple);
    }

    #[test]
    fn values_persist_as_the_app_writes_them() {
        let (defaults, s) = settings();
        s.set_font_size(18.0);
        s.put("themeId", SettingValue::String("solarized-dark".into()));
        s.put("uploadRetentionDays", SettingValue::Int(0));
        s.set_cursor_shape(CursorShape::Bar);
        assert_eq!(
            defaults.get("moshpit.settings.fontSize"),
            Some(Plist::Double(18.0))
        );
        assert_eq!(
            defaults.get("moshpit.settings.uploadRetentionDays"),
            Some(Plist::Int(0))
        );
        assert_eq!(
            defaults.get("moshpit.settings.cursorShape"),
            Some(Plist::String("bar".into()))
        );
        let reader = Settings::new(defaults.clone(), None, Readiness::Always);
        assert_eq!(reader.font_size(), 18.0);
        assert_eq!(reader.theme_id(), "solarized-dark");
        assert!(!s.set(spec_named("fontSize").unwrap(), SettingValue::Int(3)));
    }

    #[test]
    fn a_value_of_another_type_reads_as_the_default_or_bridges() {
        let (defaults, s) = settings();
        defaults.set("moshpit.settings.fontSize", Some(Plist::Int(12)));
        assert_eq!(s.font_size(), 12.0);
        defaults.set(
            "moshpit.settings.fontSize",
            Some(Plist::String("12".into())),
        );
        assert_eq!(s.font_size(), 9.0);
        defaults.set("moshpit.settings.cursorBlink", Some(Plist::Int(0)));
        assert!(!s.bool("cursorBlink"));
        defaults.set("moshpit.settings.cursorBlink", Some(Plist::Int(2)));
        assert!(s.bool("cursorBlink"));
        defaults.set(
            "moshpit.settings.udpRangeStart",
            Some(Plist::Double(60001.0)),
        );
        assert_eq!(s.udp_range().0, 60001);
        defaults.set(
            "moshpit.settings.cursorShape",
            Some(Plist::String("BLOCK".into())),
        );
        assert_eq!(s.cursor_shape(), CursorShape::Block);
        defaults.set(
            "moshpit.settings.predictMode",
            Some(Plist::String("never".into())),
        );
        assert_eq!(s.predict_mode(), PredictMode::Never);
    }

    #[test]
    fn declined_hosts_are_a_sorted_set() {
        let (defaults, s) = settings();
        s.set_host_setup_declined(vec![
            "b".into(),
            "a".into(),
            "b".into(),
            "e\u{301}".into(),
            "\u{e9}".into(),
        ]);
        assert_eq!(
            defaults.get("moshpit.settings.hostSetupDeclined"),
            Some(Plist::Array(vec![
                Plist::String("a".into()),
                Plist::String("b".into()),
                Plist::String("e\u{301}".into()),
            ]))
        );
    }

    /// B159: one item that isn't a string (another build's, a damaged
    /// write) no longer reads the whole set as empty, and the next host
    /// declined lands beside the ones there, not over them.
    #[test]
    fn declined_hosts_survive_an_item_that_is_not_a_string() {
        let (defaults, s) = settings();
        defaults.set(
            "moshpit.settings.hostSetupDeclined",
            Some(Plist::Array(vec![
                Plist::String("b".into()),
                Plist::Int(1),
                Plist::String("a".into()),
            ])),
        );
        assert_eq!(
            s.host_setup_declined(),
            vec!["b".to_owned(), "a".to_owned()]
        );
        let mut declined = s.host_setup_declined();
        declined.push("c".into());
        s.set_host_setup_declined(declined);
        assert_eq!(
            defaults.get("moshpit.settings.hostSetupDeclined"),
            Some(Plist::Array(vec![
                Plist::String("a".into()),
                Plist::String("b".into()),
                Plist::String("c".into()),
            ]))
        );
        // Not an array at all: the default, as before.
        defaults.set(
            "moshpit.settings.hostSetupDeclined",
            Some(Plist::String("a".into())),
        );
        assert!(s.host_setup_declined().is_empty());
    }

    /// B510: an Unpair in Host Setup is kept. The app writes the set by
    /// its key (`AppSettings.hostSetupUnpaired`); the store takes it and
    /// reads it back, as it does the declined hosts.
    #[test]
    fn an_unpair_in_host_setup_is_kept() {
        let (defaults, s) = settings();
        let spec = spec_named("hostSetupUnpaired").expect("a setting the app writes");
        assert_eq!(spec.key, "moshpit.settings.hostSetupUnpaired");
        assert_eq!(s.get(spec), SettingValue::StringSet(Vec::new()));
        assert!(s.set(spec, SettingValue::StringSet(vec!["B".into(), "A".into()])));
        assert_eq!(
            defaults.get("moshpit.settings.hostSetupUnpaired"),
            Some(Plist::Array(vec![
                Plist::String("A".into()),
                Plist::String("B".into())
            ]))
        );
        assert_eq!(
            s.get(spec),
            SettingValue::StringSet(vec!["A".to_owned(), "B".to_owned()])
        );
    }

    #[test]
    fn the_notification_switches_are_mirrored_once_trusted_and_on_every_change() {
        let defaults = Arc::new(MemoryDefaults::new());
        defaults.set_protected_data_available(false);
        defaults.set(LOCK_SCREEN_DETAIL_KEY, Some(Plist::Bool(false)));
        let mirror = Arc::new(MemoryDefaults::new());
        let mut s = Settings::new(defaults.clone(), Some(mirror.clone()), Readiness::System);
        assert_eq!(mirror.get(moshpit_seal::prefs::DETAIL_KEY), None);
        assert_eq!(defaults.get(crate::readiness::SENTINEL_KEY), None);
        defaults.set_protected_data_available(true);
        s.reload_if_needed();
        assert_eq!(
            mirror.get(moshpit_seal::prefs::DETAIL_KEY),
            Some(Plist::Bool(false))
        );
        assert_eq!(
            mirror.get(moshpit_seal::prefs::SOUND_KEY),
            Some(Plist::Bool(true))
        );
        s.put("attentionSoundEnabled", SettingValue::Bool(false));
        assert_eq!(
            mirror.get(moshpit_seal::prefs::SOUND_KEY),
            Some(Plist::Bool(false))
        );
        assert!(!s.push_prefs().sound);
    }

    #[test]
    fn a_debug_build_takes_a_loopback_relay_only() {
        let (defaults, mut s) = settings();
        defaults.set(
            PUSH_RELAY_KEY,
            Some(Plist::String("http://127.0.0.1:8080".into())),
        );
        assert_eq!(s.push_relay_url(), DEFAULT_PUSH_RELAY);
        s.set_debug_build(true);
        assert_eq!(s.push_relay_url(), "http://127.0.0.1:8080");
        defaults.set(
            PUSH_RELAY_KEY,
            Some(Plist::String("http://[::1]:9/x".into())),
        );
        assert_eq!(s.push_relay_url(), "http://[::1]:9/x");
        defaults.set(
            PUSH_RELAY_KEY,
            Some(Plist::String("https://push.moshpit.cluas.eu".into())),
        );
        assert_eq!(s.push_relay_url(), DEFAULT_PUSH_RELAY);
    }
}
