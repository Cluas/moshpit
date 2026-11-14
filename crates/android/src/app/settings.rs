//! Settings' host side (`Moshpit/UI/Settings/SettingsNSHPage.swift`): the
//! page's state from the stores, a changed setting written back, a row's
//! page opened.

use moshpit_store::settings::{SettingValue, spec_named};
use moshpit_store::themes::AnsiSlot;
use moshpit_ui::app_icon::Icon;
use moshpit_ui::screen::AppearanceChoice;
use moshpit_ui::settings::{
    Choice, Count, CursorShape, Message, Part, PredictMode, SettingsState, Swatch, TerminalTheme,
};
use moshpit_ui::voice_engine::Engine;
use serde_json::Value;

use crate::app::looks::{self, rgba};
use crate::app::{App, Core, PageKind};

/// The terminal's fonts, in the menu's order (`TerminalFont.families`).
/// Android draws them all in its monospace face for now.
pub const FONTS: [(&str, &str); 10] = [
    ("system", "SF Mono"),
    ("JetBrainsMono-Regular", "JetBrains Mono"),
    ("MapleMono-Regular", "Maple Mono"),
    ("FiraCode-Regular", "Fira Code"),
    ("SourceCodePro-Regular", "Source Code Pro"),
    ("IBMPlexMono-Regular", "IBM Plex Mono"),
    ("Hack-Regular", "Hack"),
    ("AnonymousPro-Regular", "Anonymous Pro"),
    ("Menlo", "Menlo"),
    ("Courier New", "Courier"),
];

/// The switches, by the page's keys and `AppSettings`' names.
const FLAGS: [(&str, &str); 11] = [
    ("cursorBlink", "cursorBlink"),
    ("trailOnPredict", "trailOnPredict"),
    ("keepAlive", "keepConnectionsAlive"),
    ("keyboardOnOpen", "raiseKeyboardOnOpen"),
    ("remoteClipboardRead", "remoteClipboardReadEnabled"),
    ("moshByDefault", "moshByDefault"),
    ("notifications", "notificationsEnabled"),
    ("liveActivity", "liveActivityEnabled"),
    ("alertSound", "attentionSoundEnabled"),
    ("lockScreenDetail", "lockScreenDetailEnabled"),
    ("voiceInput", "voiceInputEnabled"),
];

impl App {
    fn setting_string(&self, name: &str) -> String {
        match spec_named(name).map(|spec| self.settings.get(spec)) {
            Some(SettingValue::String(text)) => text,
            _ => String::new(),
        }
    }

    fn setting_int(&self, name: &str) -> i64 {
        match spec_named(name).map(|spec| self.settings.get(spec)) {
            Some(SettingValue::Int(n)) => n,
            _ => 0,
        }
    }
}

/// `{"op": "settings", …}`.
pub fn message(app: &App) -> String {
    Message::Settings(Box::new(state(app))).to_json()
}

/// What the page shows now (`SettingsSnapshot`).
pub fn state(app: &App) -> SettingsState {
    let accent = app.accents.theme(&app.settings.app_theme_id());
    let icon = moshpit_store::app_themes::icons::option(&app.settings.app_icon_id());
    let theme = app.themes.theme(&app.settings.theme_id());
    let (low, high) = app.settings.udp_range();
    let live_session = app
        .sessions
        .by_connection
        .values()
        .any(|c| c.state.is_live());
    SettingsState {
        accent: Swatch {
            name: accent.name.clone(),
            color: rgba(&accent.accent_hex),
        },
        app_icon: Icon {
            id: icon.id.to_owned(),
            name: icon.name.to_owned(),
            image: None,
        },
        fonts: FONTS
            .iter()
            .map(|(id, name)| Choice {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
            })
            .collect(),
        font: app.setting_string("fontName"),
        font_size: app.settings.font_size(),
        theme: TerminalTheme {
            name: theme.name.clone(),
            background: rgba(&theme.background_hex),
            foreground: rgba(&theme.foreground_hex),
            swatches: AnsiSlot::ALL
                .iter()
                .map(|slot| rgba(&theme.ansi(*slot)))
                .collect(),
        },
        cursor_shape: match app.settings.cursor_shape().raw_value() {
            "bar" => CursorShape::Bar,
            "underline" => CursorShape::Underline,
            _ => CursorShape::Block,
        },
        cursor_color: app.setting_string("cursorColorId"),
        cursor_blink: app.settings_flag("cursorBlink"),
        trail_on_predict: app.settings_flag("trailOnPredict"),
        keep_alive: app.settings_flag("keepConnectionsAlive"),
        keyboard_on_open: app.settings_flag("raiseKeyboardOnOpen"),
        remote_clipboard_read: app.settings_flag("remoteClipboardReadEnabled"),
        upload_retention_days: app.setting_int("uploadRetentionDays"),
        mosh_by_default: app.settings_flag("moshByDefault"),
        predict_mode: match app.settings.predict_mode().raw_value() {
            "always" => PredictMode::Always,
            "never" => PredictMode::Never,
            "experimental" => PredictMode::Experimental,
            _ => PredictMode::Adaptive,
        },
        mosh_server_binary: app.setting_string("moshServerBinary"),
        udp_range: [low, high],
        shortcuts: Count {
            count: app.shortcuts.toolbar_count() as i64,
            limit: moshpit_store::shortcuts::ShortcutStore::TOOLBAR_LIMIT as i64,
        },
        keys: app.keys.keys().len() as i64,
        notifications: app.settings_flag("notificationsEnabled"),
        live_activity: app.settings_flag("liveActivityEnabled"),
        alert_sound: app.settings_flag("attentionSoundEnabled"),
        lock_screen_detail: app.settings_flag("lockScreenDetailEnabled"),
        live_session,
        voice_input: app.settings_flag("voiceInputEnabled"),
        voice_engine: Engine::Apple,
        voice_model: None,
        voice_language: "Automatic".to_owned(),
        version: app.options.version.clone(),
        filing: None,
        app_theme: looks::app_theme(&accent),
        appearance: appearance_choice(app.settings.appearance()),
    }
}

/// The screens' name for the stored appearance.
pub(crate) fn appearance_choice(stored: moshpit_store::settings::Appearance) -> AppearanceChoice {
    match stored {
        moshpit_store::settings::Appearance::System => AppearanceChoice::System,
        moshpit_store::settings::Appearance::Light => AppearanceChoice::Light,
        moshpit_store::settings::Appearance::Dark => AppearanceChoice::Dark,
    }
}

/// Applies setting `key` (`SettingsModel.apply`); false for a key or value
/// it does not know.
pub fn apply(app: &App, key: &str, value: &Value) -> bool {
    let put = |name: &str, value: SettingValue| {
        spec_named(name).is_some_and(|spec| app.settings.set(spec, value))
    };
    match key {
        "font" => {
            let Some(id) = value
                .as_str()
                .filter(|id| FONTS.iter().any(|(f, _)| f == id))
            else {
                return false;
            };
            put("fontName", SettingValue::String(id.to_owned()))
        }
        "fontSize" => {
            let Some(size) = value.as_f64().filter(|s| s.is_finite()) else {
                return false;
            };
            app.settings.set_font_size(size.round().clamp(8.0, 18.0));
            true
        }
        "cursorShape" => match value.as_str() {
            Some(shape @ ("block" | "bar" | "underline")) => {
                put("cursorShape", SettingValue::String(shape.to_owned()))
            }
            _ => false,
        },
        "cursorColor" => match value.as_str() {
            Some(id @ ("teal" | "green" | "white" | "accent")) => {
                put("cursorColorId", SettingValue::String(id.to_owned()))
            }
            _ => false,
        },
        "uploadRetentionDays" => match value.as_i64() {
            Some(days @ (0 | 1 | 7)) => put("uploadRetentionDays", SettingValue::Int(days)),
            _ => false,
        },
        "predictMode" => match value.as_str() {
            Some(mode @ ("adaptive" | "always" | "never" | "experimental")) => {
                put("predictMode", SettingValue::String(mode.to_owned()))
            }
            _ => false,
        },
        "appearance" => match value
            .as_str()
            .and_then(moshpit_store::settings::Appearance::from_raw)
        {
            Some(appearance) => {
                app.settings.set_appearance(appearance);
                true
            }
            None => false,
        },
        _ => {
            let (Some(flag), Some((_, name))) =
                (value.as_bool(), FLAGS.iter().find(|(k, _)| *k == key))
            else {
                return false;
            };
            put(name, SettingValue::Bool(flag))
        }
    }
}

/// What the page told the host.
pub fn on_event(app: &mut App, core: &Core, surface: u64, ev: &str, event: &Value) {
    match ev {
        "done" | "back" => app.close(surface),
        "set" => {
            let key = event["key"].as_str().unwrap_or_default();
            if apply(app, key, &event["value"]) {
                app.refresh();
            } else {
                log::warn!(target: "moshpit", "settings: no setting {key}");
            }
        }
        "open" => match event["page"].as_str().unwrap_or_default() {
            // A page of Settings' own (Appearance, Terminal…): the screen
            // again, on that page.
            page if Part::named(Some(page)) != Part::Hub => {
                app.open(core, PageKind::SettingsPart(Part::named(Some(page))), true);
            }
            "ssh-keys" => {
                app.open(
                    core,
                    PageKind::SshKeys {
                        deleting: None,
                        notice: None,
                    },
                    true,
                );
            }
            "shortcuts" => {
                app.open(core, PageKind::Shortcuts, true);
            }
            "accent" => {
                let selected = app.settings.app_theme_id();
                app.open(
                    core,
                    PageKind::Accent {
                        selected,
                        deleting: None,
                    },
                    true,
                );
            }
            "theme" => {
                app.open(core, PageKind::Theme { deleting: None }, true);
            }
            "recent-log" => {
                app.open(core, PageKind::RecentLog, true);
            }
            "github-issue" => app
                .platform
                .open_url("https://github.com/Cluas/moshpit/issues/new"),
            "feedback" => app.platform.open_url("mailto:support@moshpit.app"),
            "notification-info" => app
                .platform
                .toast("Notifications from your hosts come with FCM, in a later build"),
            "voice-engine" | "whisper-model" | "voice-language" => {
                app.platform.toast("Voice input is not on Android yet")
            }
            "host-setup" => app.open_host_setup(core),
            // Hidden on Android (the shared page leaves the row out): a
            // launcher shows the one icon the manifest names.
            "app-icon" => app.platform.toast("Android keeps the one icon"),
            other => app.platform.toast(&format!("{other}: not on Android yet")),
        },
        _ => {}
    }
}
