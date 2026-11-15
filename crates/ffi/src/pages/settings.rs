//! Settings (`ui/src/settings.rs`). Its state carries what UniFFI can't
//! (colors, a two-number range), so it crosses as small bridge records, each
//! converted into the screen's own type; its events are read back into a
//! bridge enum whose conversion from the screen's [`Setting`] is
//! exhaustive, so a setting added there doesn't compile here until the
//! host can take it.
//!
//! The screen's small enums (a cursor's shape, a predict mode, an engine,
//! an appearance, the pet's style) cross as their ids: the id is what the
//! setting stores and the app's own enum's raw value, so the host passes it
//! through, and the bridge holds it to the screen's type (an unknown one is
//! an error going out and no event coming back).
//!
//! The names start with `Settings`: the bindings compile into the app's
//! module, which has its own `SettingsHost` and the rest.

use moshpit_ui::app_icon::Icon;
use moshpit_ui::screen::AppearanceChoice;
use moshpit_ui::settings::{
    Anchor, Choice, Count, CursorShape, HostEntry, PredictMode, Setting, SettingsState, Swatch,
    TerminalTheme,
};
use moshpit_ui::theme::AppTheme;
use moshpit_ui::voice_engine::Engine;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{PageError, choice, color};

/// What Settings shows (`SettingsState`), colors written `#RRGGBB`.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SettingsPageState {
    pub accent: SettingsSwatch,
    pub app_icon: SettingsIcon,
    pub fonts: Vec<SettingsChoice>,
    pub font: String,
    pub font_size: f64,
    pub theme: SettingsTerminalTheme,
    /// `block`, `bar` or `underline`.
    pub cursor_shape: String,
    pub cursor_color: String,
    pub cursor_blink: bool,
    pub trail_on_predict: bool,
    pub keyboard_on_open: bool,
    pub remote_clipboard_read: bool,
    pub upload_retention_days: i64,
    pub mosh_by_default: bool,
    /// `adaptive`, `always`, `never` or `experimental`.
    pub predict_mode: String,
    pub udp_range: PortRange,
    pub shortcuts: SettingsCount,
    pub keys: i64,
    pub notifications: bool,
    pub live_activity: bool,
    /// iOS has the app's notifications off (B741).
    pub notifications_blocked: bool,
    /// iOS has the app's Live Activities off (B741).
    pub live_activity_blocked: bool,
    pub alert_sound: bool,
    pub lock_screen_detail: bool,
    pub live_session: bool,
    pub voice_input: bool,
    /// `apple` or `whisper`.
    pub voice_engine: String,
    pub voice_model: Option<String>,
    pub voice_language: String,
    pub version: String,
    pub filing: Option<String>,
    pub app_theme: SettingsAppTheme,
    /// `system`, `light` or `dark`.
    pub appearance: String,
}

/// The accent: its name and color.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsSwatch {
    pub name: String,
    pub color_hex: String,
}

/// The home-screen icon (`app_icon::Icon`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsIcon {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
}

/// One choice of a menu.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsChoice {
    pub id: String,
    pub name: String,
}

/// The terminal's color scheme: its name, background, words' color and
/// eight ANSI colors.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsTerminalTheme {
    pub name: String,
    pub background_hex: String,
    pub foreground_hex: String,
    pub swatches_hex: Vec<String>,
}

/// The theme the app wears now (`theme::AppTheme`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsAppTheme {
    pub accent_hex: String,
    pub accent_pressed_hex: String,
    pub screen_bg_hex: String,
    pub terminal_bg_hex: String,
}

/// How many of something, of how many there can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct SettingsCount {
    pub count: i64,
    pub limit: i64,
}

/// A range of ports, first and last (`[i64; 2]` in the screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PortRange {
    pub start: i64,
    pub end: i64,
}

/// Where the list scrolls to (`{"op": "reveal"}`, the screen's `Anchor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SettingsRevealAnchor {
    Cursor,
    Behavior,
    Mosh,
    Notifications,
    Bottom,
}

/// A saved connection on Set up a host's list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsHostEntry {
    pub id: String,
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: i64,
    pub live: bool,
}

/// What Settings tells the host.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SettingsEvent {
    /// The hub's Done.
    Done,
    /// A page's back.
    Back,
    /// A row asked for page `page` (`ui/src/settings.rs`'s `Page` or `Part`
    /// id: `"udp-range"`, `"feedback"`…).
    Open { page: String },
    /// A setting changed.
    Set { change: SettingsChange },
    /// A connection picked on Set up a host's list.
    Setup { id: String },
}

/// A setting the user changed (`Setting`); an enum's value is its id.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SettingsChange {
    Font {
        id: String,
    },
    FontSize {
        points: f64,
    },
    CursorShape {
        id: String,
    },
    CursorColor {
        id: String,
    },
    CursorBlink {
        on: bool,
    },
    TrailOnPredict {
        on: bool,
    },
    KeyboardOnOpen {
        on: bool,
    },
    RemoteClipboardRead {
        on: bool,
    },
    UploadRetentionDays {
        days: i64,
    },
    MoshByDefault {
        on: bool,
    },
    PredictMode {
        id: String,
    },
    Notifications {
        on: bool,
    },
    LiveActivity {
        on: bool,
    },
    AlertSound {
        on: bool,
    },
    LockScreenDetail {
        on: bool,
    },
    VoiceInput {
        on: bool,
    },
    Appearance {
        id: String,
    },
    UdpRange {
        range: PortRange,
    },
    /// The companion's style id (`minimal`, `pixel`) and whether it shows.
    Companion {
        style: String,
        shown: bool,
    },
    PetName {
        name: String,
    },
    PetTalk {
        on: bool,
    },
}

// MARK: - Into the screen's types

impl TryFrom<SettingsPageState> for SettingsState {
    type Error = PageError;

    fn try_from(s: SettingsPageState) -> Result<Self, PageError> {
        Ok(SettingsState {
            accent: Swatch {
                name: s.accent.name,
                color: color("accent", &s.accent.color_hex)?,
            },
            app_icon: Icon {
                id: s.app_icon.id,
                name: s.app_icon.name,
                image: s.app_icon.image,
            },
            fonts: s
                .fonts
                .into_iter()
                .map(|c| Choice {
                    id: c.id,
                    name: c.name,
                })
                .collect(),
            font: s.font,
            font_size: s.font_size,
            theme: TerminalTheme {
                name: s.theme.name,
                background: color("theme.background", &s.theme.background_hex)?,
                foreground: color("theme.foreground", &s.theme.foreground_hex)?,
                swatches: s
                    .theme
                    .swatches_hex
                    .iter()
                    .map(|hex| color("theme.swatches", hex))
                    .collect::<Result<_, _>>()?,
            },
            cursor_shape: choice::<CursorShape>("cursorShape", &s.cursor_shape)?,
            cursor_color: s.cursor_color,
            cursor_blink: s.cursor_blink,
            trail_on_predict: s.trail_on_predict,
            keyboard_on_open: s.keyboard_on_open,
            remote_clipboard_read: s.remote_clipboard_read,
            upload_retention_days: s.upload_retention_days,
            mosh_by_default: s.mosh_by_default,
            predict_mode: choice::<PredictMode>("predictMode", &s.predict_mode)?,
            udp_range: [s.udp_range.start, s.udp_range.end],
            shortcuts: Count {
                count: s.shortcuts.count,
                limit: s.shortcuts.limit,
            },
            keys: s.keys,
            notifications: s.notifications,
            live_activity: s.live_activity,
            notifications_blocked: s.notifications_blocked,
            live_activity_blocked: s.live_activity_blocked,
            alert_sound: s.alert_sound,
            lock_screen_detail: s.lock_screen_detail,
            live_session: s.live_session,
            voice_input: s.voice_input,
            voice_engine: choice::<Engine>("voiceEngine", &s.voice_engine)?,
            voice_model: s.voice_model,
            voice_language: s.voice_language,
            version: s.version,
            filing: s.filing,
            app_theme: AppTheme {
                accent: color("appTheme.accent", &s.app_theme.accent_hex)?,
                accent_pressed: color("appTheme.accentPressed", &s.app_theme.accent_pressed_hex)?,
                screen_bg: color("appTheme.screenBG", &s.app_theme.screen_bg_hex)?,
                terminal_bg: color("appTheme.terminalBG", &s.app_theme.terminal_bg_hex)?,
            },
            appearance: choice::<AppearanceChoice>("appearance", &s.appearance)?,
        })
    }
}

impl From<SettingsRevealAnchor> for Anchor {
    fn from(anchor: SettingsRevealAnchor) -> Self {
        match anchor {
            SettingsRevealAnchor::Cursor => Anchor::Cursor,
            SettingsRevealAnchor::Behavior => Anchor::Behavior,
            SettingsRevealAnchor::Mosh => Anchor::Mosh,
            SettingsRevealAnchor::Notifications => Anchor::Notifications,
            SettingsRevealAnchor::Bottom => Anchor::Bottom,
        }
    }
}

impl From<SettingsHostEntry> for HostEntry {
    fn from(h: SettingsHostEntry) -> Self {
        HostEntry {
            id: h.id,
            name: h.name,
            user: h.user,
            host: h.host,
            port: h.port,
            live: h.live,
        }
    }
}

/// The screen's setting as the host hears it. Exhaustive: a setting the
/// screen adds doesn't compile here until the bridge carries it.
impl From<Setting> for SettingsChange {
    fn from(setting: Setting) -> Self {
        match setting {
            Setting::Font(id) => SettingsChange::Font { id },
            Setting::FontSize(points) => SettingsChange::FontSize { points },
            Setting::CursorShape(shape) => SettingsChange::CursorShape {
                id: shape.id().to_owned(),
            },
            Setting::CursorColor(id) => SettingsChange::CursorColor { id },
            Setting::CursorBlink(on) => SettingsChange::CursorBlink { on },
            Setting::TrailOnPredict(on) => SettingsChange::TrailOnPredict { on },
            Setting::KeyboardOnOpen(on) => SettingsChange::KeyboardOnOpen { on },
            Setting::RemoteClipboardRead(on) => SettingsChange::RemoteClipboardRead { on },
            Setting::UploadRetentionDays(days) => SettingsChange::UploadRetentionDays { days },
            Setting::MoshByDefault(on) => SettingsChange::MoshByDefault { on },
            Setting::PredictMode(mode) => SettingsChange::PredictMode {
                id: mode.id().to_owned(),
            },
            Setting::Notifications(on) => SettingsChange::Notifications { on },
            Setting::LiveActivity(on) => SettingsChange::LiveActivity { on },
            Setting::AlertSound(on) => SettingsChange::AlertSound { on },
            Setting::LockScreenDetail(on) => SettingsChange::LockScreenDetail { on },
            Setting::VoiceInput(on) => SettingsChange::VoiceInput { on },
            Setting::Appearance(choice) => SettingsChange::Appearance {
                id: choice.id().to_owned(),
            },
            Setting::UdpRange([start, end]) => SettingsChange::UdpRange {
                range: PortRange { start, end },
            },
            Setting::Companion(style, shown) => SettingsChange::Companion {
                style: style.id().to_owned(),
                shown,
            },
            Setting::PetName(name) => SettingsChange::PetName { name },
            Setting::PetTalk(on) => SettingsChange::PetTalk { on },
        }
    }
}

// MARK: - Reading what the screen told

impl SettingsEvent {
    /// What Settings told (`{"ev": …}`), read; `None` for anything else.
    pub(super) fn read(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        let text = |key: &str| value[key].as_str().map(str::to_owned);
        match value["ev"].as_str()? {
            "done" => Some(SettingsEvent::Done),
            "back" => Some(SettingsEvent::Back),
            "open" => Some(SettingsEvent::Open {
                page: text("page")?,
            }),
            "setup" => Some(SettingsEvent::Setup { id: text("id")? }),
            "set" => change(value["key"].as_str()?, &value["value"])
                .map(|change| SettingsEvent::Set { change }),
            _ => None,
        }
    }
}

/// `value` when it is an id of `T`.
fn known<T: DeserializeOwned>(value: &Value) -> Option<String> {
    serde_json::from_value::<T>(value.clone()).ok()?;
    value.as_str().map(str::to_owned)
}

/// A `set` event's `key` and `value`, read as the screen's
/// [`Setting::event`] writes them.
fn change(key: &str, value: &Value) -> Option<SettingsChange> {
    let on = || value.as_bool();
    let text = || value.as_str().map(str::to_owned);
    Some(match key {
        "font" => SettingsChange::Font { id: text()? },
        "fontSize" => SettingsChange::FontSize {
            points: value.as_f64()?,
        },
        "cursorShape" => SettingsChange::CursorShape {
            id: known::<CursorShape>(value)?,
        },
        "cursorColor" => SettingsChange::CursorColor { id: text()? },
        "cursorBlink" => SettingsChange::CursorBlink { on: on()? },
        "trailOnPredict" => SettingsChange::TrailOnPredict { on: on()? },
        "keyboardOnOpen" => SettingsChange::KeyboardOnOpen { on: on()? },
        "remoteClipboardRead" => SettingsChange::RemoteClipboardRead { on: on()? },
        "uploadRetentionDays" => SettingsChange::UploadRetentionDays {
            days: value.as_i64()?,
        },
        "moshByDefault" => SettingsChange::MoshByDefault { on: on()? },
        "predictMode" => SettingsChange::PredictMode {
            id: known::<PredictMode>(value)?,
        },
        "notifications" => SettingsChange::Notifications { on: on()? },
        "liveActivity" => SettingsChange::LiveActivity { on: on()? },
        "alertSound" => SettingsChange::AlertSound { on: on()? },
        "lockScreenDetail" => SettingsChange::LockScreenDetail { on: on()? },
        "voiceInput" => SettingsChange::VoiceInput { on: on()? },
        "appearance" => SettingsChange::Appearance {
            id: known::<AppearanceChoice>(value)?,
        },
        "udpRange" => {
            let [start, end] = serde_json::from_value::<[i64; 2]>(value.clone()).ok()?;
            SettingsChange::UdpRange {
                range: PortRange { start, end },
            }
        }
        "companion" => SettingsChange::Companion {
            style: known::<moshpit_pet::Style>(&value["style"])?,
            shown: value["shown"].as_bool()?,
        },
        "petName" => SettingsChange::PetName { name: text()? },
        "petTalk" => SettingsChange::PetTalk { on: on()? },
        _ => return None,
    })
}
