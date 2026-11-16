//! The NSH page hosts' one typed bridge (E6b).
//!
//! A page host (a Swift page that runs an NSH screen) used to talk to its
//! screen in hand-built dictionaries: it spelled every message the screen
//! takes (`{"op": …}`) and read every event the screen tells (`{"ev": …}`)
//! key by key, a second copy of types that already exist once, in the
//! screen's module (`ui/src/<screen>.rs`), as serde types.
//!
//! Here the host builds a [`PageMessage`] and the screen's own serializer
//! writes the JSON it parses ([`page_message`]); what the screen tells is
//! read into a [`PageEvent`] ([`page_event`]). The Swift side is generated:
//! one spelling, in Rust, compile-checked on both ends.
//!
//! How a screen's types cross, one module per page:
//!
//! - A type UniFFI can carry as it is, under a name the app's module doesn't
//!   already use, is declared as a remote type (`#[uniffi::remote]`: the
//!   declaration restates the screen's type and the compiler holds the two
//!   to the same fields). Feedback's state and events cross so.
//! - The rest cross as small bridge records, named for their page, each
//!   converted into the screen's type here: colors as `#RRGGBB` strings (the
//!   app's convention at the FFI), a two-number range as `{ start, end }`,
//!   a fixed array as a list held to its length, a screen's small enum as
//!   its id. A value the screen's type can't hold is a [`PageError`].
//! - Events the screen tells as JSON literals are read here into the page's
//!   event enum; the bridge's tests tell them as the screen does
//!   (`ui/tests` pins the screen's side).
//!
//! Why not Swift `Codable` mirrors of the serde types: every type would be
//! spelled twice again, and drift would show only at run time. Why one
//! envelope rather than a function per screen: one entry point each way,
//! one place a new screen is added, and `NSHSurface.post(_:)` takes any.
//!
//! The JSON is the same as before, field for field; a color the host sent
//! as `#RRGGBB` reads back `#RRGGBBAA`, as the screen writes one.

use moshpit_ui::feedback::{self, FeedbackEvent, FeedbackState};
use moshpit_ui::screen::Screen;
use moshpit_ui::theme::parse_hex;
use nsh_core::color::Rgba;
use serde::de::DeserializeOwned;

mod accent;
mod add_key;
mod feedback_page;
mod install_assist;
mod mux_picker;
mod prompt;
mod remote_folder;
mod settings;
mod shortcuts;
mod ssh_keys;
mod theme_gallery;
mod whisper_model;

pub use accent::*;
pub use add_key::*;
pub use install_assist::*;
pub use mux_picker::*;
pub use prompt::*;
pub use remote_folder::*;
pub use settings::*;
pub use shortcuts::*;
pub use ssh_keys::*;
pub use theme_gallery::*;
pub use whisper_model::*;

// MARK: - The envelope

/// A message for a page's screen: the screen's own state type.
// Its variants are as big as the pages are (Settings' state is the
// largest): UniFFI lowers a record by value, so they can't be boxed, and a
// message is built once and sent.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum PageMessage {
    /// `{"op": "feedback", …}`.
    Feedback { state: FeedbackState },
    /// `{"op": "settings", …}`.
    Settings { state: SettingsPageState },
    /// `{"op": "reveal", …}`: scroll Settings to `anchor`.
    SettingsReveal { anchor: SettingsRevealAnchor },
    /// `{"op": "hosts", …}`: Set up a host's list.
    SettingsHosts { hosts: Vec<SettingsHostEntry> },
    /// `{"op": "clock", …}`: the local hour (0–23), which the pet's card
    /// answers a tap by (B607).
    SettingsClock { hour: u8 },
    /// `{"op": "models", …}`: Voice Input › Model.
    WhisperModel { state: WhisperModelsState },
    /// `{"op": "accents", …}`: Settings › Accent.
    Accent { state: AccentPageState },
    /// `{"op": "themes", …}`: Settings › Theme.
    ThemeGallery { state: ThemeGalleryPageState },
    /// `{"op": "reveal"}`: the user's own themes scrolled to the top (a
    /// capture run's).
    ThemeGalleryReveal,
    /// `{"op": "keys", …}`: Settings › SSH Keys.
    SshKeys { state: SshKeysPageState },
    /// `{"op": "picker", …}`: a multiplexer's list.
    MuxPicker { state: MuxPickerPageState },
    /// `{"op": "act", …}`: a picker row's menu picked `action`.
    MuxPickerAct { action: MuxPickerAction, id: String },
    /// `{"op": "prompted", …}`: the answer to prompt `id` (none: Cancel),
    /// for whichever screen asked.
    Prompted { id: u32, text: Option<String> },
    /// `{"op": "folder", …}`: a folder on the host.
    RemoteFolder { state: RemoteFolderPageState },
    /// `{"op": "act", …}`: an entry's menu picked `action`.
    RemoteFolderAct {
        action: RemoteFolderAction,
        path: String,
    },
    /// `{"op": "add-key", …}`: Add Key's form.
    AddKey { state: AddKeyPageState },
}

/// What a page's screen told its host, read by the screen's own type.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum PageEvent {
    Feedback { event: FeedbackEvent },
    Settings { event: SettingsEvent },
    WhisperModel { event: WhisperModelEvent },
    InstallAssist { event: InstallAssistEvent },
    Accent { event: AccentEvent },
    ThemeGallery { event: ThemeGalleryEvent },
    SshKeys { event: SshKeysEvent },
    Shortcuts { event: ShortcutsEvent },
    AddShortcut { event: AddShortcutEvent },
    MuxPicker { event: MuxPickerEvent },
    RemoteFolder { event: RemoteFolderEvent },
    AddKey { event: AddKeyEvent },
}

/// A message the bridge couldn't write: a value the screen's type can't
/// hold (a color that isn't `#RRGGBB`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum PageError {
    /// `value` isn't one `field` can hold (a color that isn't `#RRGGBB`,
    /// an id the screen doesn't know).
    Invalid { field: String, value: String },
}

impl std::fmt::Display for PageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid { field, value } => write!(f, "{field}: {value:?} is not one"),
        }
    }
}

impl std::error::Error for PageError {}

/// One of a screen's small enums by its id (`"bar"`), or the error naming
/// `field`.
fn choice<T: DeserializeOwned>(field: &'static str, id: &str) -> Result<T, PageError> {
    serde_json::from_value(serde_json::Value::String(id.to_owned())).map_err(|_| {
        PageError::Invalid {
            field: field.to_owned(),
            value: id.to_owned(),
        }
    })
}

/// Color `hex` (`#RRGGBB` or `#RRGGBBAA`), or the error naming `field`.
fn color(field: &'static str, hex: &str) -> Result<Rgba, PageError> {
    parse_hex(hex).ok_or_else(|| PageError::Invalid {
        field: field.to_owned(),
        value: hex.to_owned(),
    })
}

/// The JSON `message` posts (`NSHSurface.post(json:)`): written by the
/// screen's own serializer, so it is what the screen parses.
#[uniffi::export]
pub fn page_message(message: PageMessage) -> Result<String, PageError> {
    use moshpit_ui::settings::Message as Settings;
    Ok(match message {
        PageMessage::Feedback { state } => feedback::Message::Feedback(state).to_json(),
        PageMessage::Settings { state } => {
            Settings::Settings(Box::new(state.try_into()?)).to_json()
        }
        PageMessage::SettingsReveal { anchor } => Settings::Reveal(anchor.into()).to_json(),
        PageMessage::SettingsHosts { hosts } => {
            Settings::Hosts(hosts.into_iter().map(Into::into).collect()).to_json()
        }
        PageMessage::SettingsClock { hour } => Settings::Clock(hour).to_json(),
        PageMessage::WhisperModel { state } => {
            moshpit_ui::whisper_model::Message::Models(state.into()).to_json()
        }
        PageMessage::Accent { state } => {
            moshpit_ui::accent::Message::Accents(state.try_into()?).to_json()
        }
        PageMessage::ThemeGallery { state } => {
            moshpit_ui::theme_gallery::Message::Themes(state.try_into()?).to_json()
        }
        PageMessage::ThemeGalleryReveal => moshpit_ui::theme_gallery::Message::Reveal.to_json(),
        PageMessage::MuxPicker { state } => mux_picker::picker(state.try_into()?),
        PageMessage::MuxPickerAct { action, id } => mux_picker::act(action, id),
        PageMessage::RemoteFolder { state } => remote_folder::folder(state.into()),
        PageMessage::RemoteFolderAct { action, path } => remote_folder::act(action, path),
        PageMessage::AddKey { state } => {
            moshpit_ui::add_key::Message::AddKey(state.try_into()?).to_json()
        }
        PageMessage::Prompted { id, text } => prompt::answer(id, text),
        PageMessage::SshKeys { state } => {
            moshpit_ui::ssh_keys::Message::Keys(state.into()).to_json()
        }
    })
}

/// What screen `screen` (its name, `"feedback"`) told, read by its own
/// event type; `None` for a screen with no typed events yet, or an event
/// it doesn't know (the host logs it).
#[uniffi::export]
pub fn page_event(screen: String, json: String) -> Option<PageEvent> {
    match Screen::named(&screen)? {
        Screen::Feedback => serde_json::from_str(&json)
            .ok()
            .map(|event| PageEvent::Feedback { event }),
        Screen::Settings => SettingsEvent::read(&json).map(|event| PageEvent::Settings { event }),
        Screen::WhisperModel => {
            WhisperModelEvent::read(&json).map(|event| PageEvent::WhisperModel { event })
        }
        Screen::InstallAssist => {
            InstallAssistEvent::read(&json).map(|event| PageEvent::InstallAssist { event })
        }
        Screen::Accent => AccentEvent::read(&json).map(|event| PageEvent::Accent { event }),
        Screen::Theme => {
            ThemeGalleryEvent::read(&json).map(|event| PageEvent::ThemeGallery { event })
        }
        Screen::SshKeys => SshKeysEvent::read(&json).map(|event| PageEvent::SshKeys { event }),
        Screen::Shortcuts => {
            ShortcutsEvent::read(&json).map(|event| PageEvent::Shortcuts { event })
        }
        Screen::AddShortcut => {
            AddShortcutEvent::read(&json).map(|event| PageEvent::AddShortcut { event })
        }
        Screen::MuxPicker => {
            MuxPickerEvent::read(&json).map(|event| PageEvent::MuxPicker { event })
        }
        Screen::RemoteFolder => {
            RemoteFolderEvent::read(&json).map(|event| PageEvent::RemoteFolder { event })
        }
        Screen::AddKey => AddKeyEvent::read(&json).map(|event| PageEvent::AddKey { event }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    /// What the Settings host saves a UDP range by (E7c: Swift's
    /// UDPPortRange was a copy of this rule).
    #[test]
    fn a_udp_range_is_saved_by_the_pages_rule() {
        use super::settings::{PortRange, settings_udp_ports};
        let ports = |a: &str, b: &str| settings_udp_ports(a.into(), b.into());
        assert_eq!(ports(" 60000 ", "61000"), Some(PortRange { start: 60000, end: 61000 }));
        assert_eq!(ports("5000", "5000"), Some(PortRange { start: 5000, end: 5000 }));
        assert_eq!(ports("abc", "61000"), None);
        assert_eq!(ports("0", "61000"), None);
        assert_eq!(ports("1", "70000"), None);
        assert_eq!(ports("61000", "60000"), None);
    }

    use moshpit_ui::feedback::FeedbackSwitch;
    use moshpit_ui::settings::{self as screen, Setting};
    use serde_json::{Value, json};

    use super::*;

    /// The envelope writes what the screen parses, and reads what the
    /// screen tells, unchanged.
    #[test]
    fn feedback_crosses_as_the_screen_spells_it() {
        let state = FeedbackState {
            message: Some("hi".into()),
            environment: true,
            log: false,
            preparing: false,
            show_address: true,
            copied: false,
            recipient: "support@example.com".into(),
        };
        let json = page_message(PageMessage::Feedback {
            state: state.clone(),
        })
        .unwrap();
        assert_eq!(
            feedback::Message::parse(&json),
            Ok(feedback::Message::Feedback(state))
        );
        for (told, event) in [
            (r#"{"ev":"cancel"}"#, FeedbackEvent::Cancel),
            (
                r#"{"ev":"set","key":"log","value":true}"#,
                FeedbackEvent::Set {
                    key: FeedbackSwitch::Log,
                    value: true,
                },
            ),
            (
                r#"{"ev":"send","message":"x"}"#,
                FeedbackEvent::Send {
                    message: "x".into(),
                },
            ),
            (r#"{"ev":"copy-address"}"#, FeedbackEvent::CopyAddress),
            (r#"{"ev":"github-issue"}"#, FeedbackEvent::GithubIssue),
        ] {
            assert_eq!(
                page_event("feedback".into(), told.into()),
                Some(PageEvent::Feedback {
                    event: event.clone()
                }),
                "{told}"
            );
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(told).unwrap(),
                event.json()
            );
        }
        assert_eq!(
            page_event("feedback".into(), r#"{"ev":"jump"}"#.into()),
            None
        );
        assert_eq!(
            page_event("nowhere".into(), r#"{"ev":"cancel"}"#.into()),
            None
        );
    }

    fn settings_state() -> SettingsPageState {
        SettingsPageState {
            accent: SettingsSwatch {
                name: "Signal Room".into(),
                color_hex: "#6C6BEF".into(),
            },
            app_icon: SettingsIcon {
                id: "default".into(),
                name: "Pit".into(),
                image: Some("asset:/IconPreview-Pit@3x.png".into()),
            },
            fonts: vec![SettingsChoice {
                id: "system".into(),
                name: "SF Mono".into(),
            }],
            font: "system".into(),
            font_size: 13.0,
            theme: SettingsTerminalTheme {
                name: "GitHub Dark".into(),
                background_hex: "#0D1117".into(),
                foreground_hex: "#E6EDF3".into(),
                swatches_hex: vec!["#484F58".into(), "#FF7B72".into()],
            },
            cursor_shape: "bar".into(),
            cursor_color: "teal".into(),
            cursor_blink: true,
            trail_on_predict: false,
            keyboard_on_open: true,
            remote_clipboard_read: false,
            upload_retention_days: 7,
            mosh_by_default: true,
            predict_mode: "never".into(),
            udp_range: PortRange {
                start: 60000,
                end: 61000,
            },
            shortcuts: SettingsCount {
                count: 8,
                limit: 12,
            },
            keys: 2,
            notifications: true,
            live_activity: false,
            notifications_blocked: true,
            live_activity_blocked: false,
            alert_sound: true,
            lock_screen_detail: false,
            live_session: true,
            voice_input: true,
            voice_engine: "whisper".into(),
            voice_model: Some("base".into()),
            voice_language: "English".into(),
            version: "Moshpit 1.0.9 (1) · dev".into(),
            filing: None,
            app_theme: SettingsAppTheme {
                accent_hex: "#6C6BEF".into(),
                accent_pressed_hex: "#5652D6".into(),
                screen_bg_hex: "#06070A".into(),
                terminal_bg_hex: "#030405".into(),
            },
            appearance: "dark".into(),
        }
    }

    /// Settings' state crosses into the screen's own type, which writes the
    /// JSON the screen parses: every field where the host put it, colors as
    /// the screen writes them (`#RRGGBBAA`), the range as the screen's pair.
    #[test]
    fn settings_crosses_into_the_screens_own_type() {
        let json = page_message(PageMessage::Settings {
            state: settings_state(),
        })
        .unwrap();
        let screen::Message::Settings(parsed) = screen::Message::parse(&json).unwrap() else {
            panic!("not settings: {json}");
        };
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["udpRange"], json!([60000, 61000]));
        assert_eq!(value["cursorShape"], "bar");
        assert_eq!(value["predictMode"], "never");
        assert_eq!(value["voiceEngine"], "whisper");
        assert_eq!(value["appearance"], "dark");
        assert_eq!(value["theme"]["background"], "#0D1117FF");
        assert_eq!(
            value["theme"]["swatches"],
            json!(["#484F58FF", "#FF7B72FF"])
        );
        assert_eq!(value["appTheme"]["screenBG"], "#06070AFF");
        assert_eq!(value["shortcuts"], json!({"count": 8, "limit": 12}));
        assert_eq!(parsed.voice_model.as_deref(), Some("base"));
        assert_eq!(
            parsed.app_icon.image.as_deref(),
            Some("asset:/IconPreview-Pit@3x.png")
        );
        assert!(value.get("filing").is_none(), "none: left out, as before");
        // What iOS has off crosses too (B741).
        assert_eq!(value["notificationsBlocked"], true);
        assert_eq!(value["liveActivityBlocked"], false);

        let mut bad = settings_state();
        bad.theme.swatches_hex.push("teal".into());
        assert_eq!(
            page_message(PageMessage::Settings { state: bad }),
            Err(PageError::Invalid {
                field: "theme.swatches".into(),
                value: "teal".into()
            })
        );

        let reveal = page_message(PageMessage::SettingsReveal {
            anchor: SettingsRevealAnchor::Notifications,
        })
        .unwrap();
        assert_eq!(
            screen::Message::parse(&reveal),
            Ok(screen::Message::Reveal(screen::Anchor::Notifications))
        );
        let hosts = page_message(PageMessage::SettingsHosts {
            hosts: vec![SettingsHostEntry {
                id: "C0FFEE00-0000-4000-8000-00000000F001".into(),
                name: "studio".into(),
                user: "cluas".into(),
                host: "studio.local".into(),
                port: 22,
                live: true,
            }],
        })
        .unwrap();
        let screen::Message::Hosts(hosts) = screen::Message::parse(&hosts).unwrap() else {
            panic!("not hosts");
        };
        assert_eq!(hosts[0].name, "studio");
        let clock = page_message(PageMessage::SettingsClock { hour: 3 }).unwrap();
        assert_eq!(
            screen::Message::parse(&clock),
            Ok(screen::Message::Clock(3))
        );
    }

    /// What Settings tells is read as the screen wrote it: every setting
    /// its `Setting::event` writes comes back as the bridge's change, and
    /// the page's own events.
    #[test]
    fn settings_events_read_as_the_screen_writes_them() {
        let settings = [
            Setting::Font("Menlo".into()),
            Setting::FontSize(12.0),
            Setting::CursorShape(screen::CursorShape::Underline),
            Setting::CursorColor("accent".into()),
            Setting::CursorBlink(false),
            Setting::TrailOnPredict(true),
            Setting::KeyboardOnOpen(true),
            Setting::RemoteClipboardRead(true),
            Setting::UploadRetentionDays(0),
            Setting::MoshByDefault(false),
            Setting::PredictMode(screen::PredictMode::Experimental),
            Setting::Notifications(false),
            Setting::LiveActivity(true),
            Setting::AlertSound(false),
            Setting::LockScreenDetail(true),
            Setting::VoiceInput(false),
            Setting::Appearance(moshpit_ui::screen::AppearanceChoice::Light),
            Setting::UdpRange([60001, 60999]),
            Setting::Companion(moshpit_pet::Style::Minimal, false),
            Setting::PetName("Tun".into()),
            Setting::PetTalk(true),
        ];
        for setting in settings {
            let told = setting.event().to_string();
            assert_eq!(
                page_event("settings".into(), told.clone()),
                Some(PageEvent::Settings {
                    event: SettingsEvent::Set {
                        change: setting.into()
                    }
                }),
                "{told}"
            );
        }
        let page = screen::Page::Feedback.event().to_string();
        assert_eq!(
            page_event("settings".into(), page),
            Some(PageEvent::Settings {
                event: SettingsEvent::Open {
                    page: "feedback".into()
                }
            })
        );
        for (told, event) in [
            (json!({"ev": "done"}), SettingsEvent::Done),
            (json!({"ev": "back"}), SettingsEvent::Back),
            (
                json!({"ev": "setup", "id": "C0FFEE00"}),
                SettingsEvent::Setup {
                    id: "C0FFEE00".into(),
                },
            ),
        ] {
            assert_eq!(
                page_event("settings".into(), told.to_string()),
                Some(PageEvent::Settings { event })
            );
        }
        assert_eq!(
            page_event(
                "settings".into(),
                json!({"ev": "set", "key": "fontSize", "value": "big"}).to_string()
            ),
            None,
            "a value of the wrong type"
        );
    }

    /// The Model page's models cross into the screen's own, and its events
    /// read back as the page writes them.
    #[test]
    fn whisper_models_cross_both_ways() {
        let row = WhisperModelRow {
            id: "openai_whisper-base".into(),
            name: "Base".into(),
            summary: "Fast and light.".into(),
            approximate: "145 MB".into(),
            installed: None,
            partial: Some("48 MB".into()),
            progress: Some(0.42),
            selected: false,
        };
        let json = page_message(PageMessage::WhisperModel {
            state: WhisperModelsState {
                models: vec![row],
                failure: Some("The network connection was lost.".into()),
                storage: None,
            },
        })
        .unwrap();
        let moshpit_ui::whisper_model::Message::Models(models) =
            moshpit_ui::whisper_model::Message::parse(&json).unwrap();
        assert_eq!(models.models[0].partial.as_deref(), Some("48 MB"));
        assert_eq!(models.models[0].progress, Some(0.42));
        assert!(models.storage.is_none());
        for (told, event) in [
            (json!({"ev": "back"}), WhisperModelEvent::Back),
            (
                json!({"ev": "select", "id": "a"}),
                WhisperModelEvent::Select { id: "a".into() },
            ),
            (
                json!({"ev": "download", "id": "a"}),
                WhisperModelEvent::Download { id: "a".into() },
            ),
            (
                json!({"ev": "pause", "id": "a"}),
                WhisperModelEvent::Pause { id: "a".into() },
            ),
            (
                json!({"ev": "discard", "id": "a"}),
                WhisperModelEvent::Discard { id: "a".into() },
            ),
            (
                json!({"ev": "remove", "id": "a"}),
                WhisperModelEvent::Remove { id: "a".into() },
            ),
        ] {
            assert_eq!(
                page_event("whisper-model".into(), told.to_string()),
                Some(PageEvent::WhisperModel { event }),
                "{told}"
            );
        }
        assert_eq!(
            page_event("whisper-model".into(), json!({"ev": "select"}).to_string()),
            None,
            "a select names its model"
        );
    }
    /// Install Assist's events read as the sheet tells them.
    #[test]
    fn install_assist_events_read_as_the_sheet_tells_them() {
        for (told, event) in [
            ("done", InstallAssistEvent::Done),
            ("run", InstallAssistEvent::Run),
            ("copy", InstallAssistEvent::Copy),
            ("recheck", InstallAssistEvent::Recheck),
            ("reconnect", InstallAssistEvent::Reconnect),
        ] {
            assert_eq!(
                page_event("install-assist".into(), json!({"ev": told}).to_string()),
                Some(PageEvent::InstallAssist { event }),
                "{told}"
            );
        }
        assert_eq!(
            page_event(
                "install-assist".into(),
                json!({"ev": "install"}).to_string()
            ),
            None
        );
    }

    fn accent(id: &str, accent: &str) -> AccentEntry {
        AccentEntry {
            id: id.into(),
            name: id.to_uppercase(),
            accent_hex: accent.into(),
            screen_bg_hex: "#06070A".into(),
        }
    }

    /// The Accent page's accents cross into the screen's own, colors as it
    /// writes them; its events read back as it tells them.
    #[test]
    fn accents_cross_both_ways() {
        let state = AccentPageState {
            built_in: vec![accent("signal", "#6C6BEF")],
            custom: vec![accent("mine", "#FF8800")],
            selected: "mine".into(),
            deleting: Some("mine".into()),
        };
        let json = page_message(PageMessage::Accent { state }).unwrap();
        let moshpit_ui::accent::Message::Accents(accents) =
            moshpit_ui::accent::Message::parse(&json).unwrap();
        assert_eq!(accents.custom[0].id, "mine");
        assert_eq!(accents.deleting.as_deref(), Some("mine"));
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["builtIn"][0]["accent"], "#6C6BEFFF");
        assert_eq!(value["custom"][0]["screenBG"], "#06070AFF");

        let bad = AccentPageState {
            built_in: vec![],
            custom: vec![accent("mine", "orange")],
            selected: "mine".into(),
            deleting: None,
        };
        assert_eq!(
            page_message(PageMessage::Accent { state: bad }),
            Err(PageError::Invalid {
                field: "accent".into(),
                value: "orange".into()
            })
        );

        for (told, event) in [
            (json!({"ev": "back"}), AccentEvent::Back),
            (
                json!({"ev": "select", "id": "mine"}),
                AccentEvent::Select { id: "mine".into() },
            ),
            (json!({"ev": "add"}), AccentEvent::Add),
            (
                json!({"ev": "delete", "id": "mine"}),
                AccentEvent::Delete { id: "mine".into() },
            ),
            (json!({"ev": "cancel-delete"}), AccentEvent::CancelDelete),
        ] {
            assert_eq!(
                page_event("accent".into(), told.to_string()),
                Some(PageEvent::Accent { event }),
                "{told}"
            );
        }
        assert_eq!(
            page_event("accent".into(), json!({"ev": "delete"}).to_string()),
            None,
            "a delete names its accent"
        );
    }

    fn palette(id: &str, ansi: usize) -> ThemePaletteEntry {
        ThemePaletteEntry {
            id: id.into(),
            name: id.to_uppercase(),
            background_hex: "#0D1117".into(),
            foreground_hex: "#E6EDF3".into(),
            cursor_hex: "#58A6FF".into(),
            ansi_hex: (0..ansi).map(|i| format!("#0000{i:02X}")).collect(),
        }
    }

    /// The Theme page's themes cross into the screen's own, eight ANSI
    /// colors and no other count; its events read back as it tells them.
    #[test]
    fn themes_cross_both_ways() {
        let state = ThemeGalleryPageState {
            built_in: vec![palette("github-dark", 8)],
            custom: vec![palette("mine", 8)],
            selected: "github-dark".into(),
            deleting: None,
        };
        let json = page_message(PageMessage::ThemeGallery { state }).unwrap();
        let moshpit_ui::theme_gallery::Message::Themes(themes) =
            moshpit_ui::theme_gallery::Message::parse(&json).unwrap()
        else {
            panic!("not themes: {json}");
        };
        assert_eq!(themes.built_in[0].id, "github-dark");
        assert_eq!(themes.custom[0].ansi[7], parse_hex("#000007").unwrap());
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["builtIn"][0]["cursor"], "#58A6FFFF");
        assert!(value.get("deleting").is_none());

        let short = ThemeGalleryPageState {
            built_in: vec![palette("short", 7)],
            custom: vec![],
            selected: "short".into(),
            deleting: None,
        };
        assert_eq!(
            page_message(PageMessage::ThemeGallery { state: short }),
            Err(PageError::Invalid {
                field: "ansi".into(),
                value: "7 colors, not 8".into()
            })
        );
        assert_eq!(
            moshpit_ui::theme_gallery::Message::parse(
                &page_message(PageMessage::ThemeGalleryReveal).unwrap()
            ),
            Ok(moshpit_ui::theme_gallery::Message::Reveal)
        );

        for (told, event) in [
            (json!({"ev": "back"}), ThemeGalleryEvent::Back),
            (
                json!({"ev": "select", "id": "mine"}),
                ThemeGalleryEvent::Select { id: "mine".into() },
            ),
            (json!({"ev": "new"}), ThemeGalleryEvent::New),
            (json!({"ev": "import"}), ThemeGalleryEvent::Import),
            (
                json!({"ev": "delete", "id": "mine"}),
                ThemeGalleryEvent::Delete { id: "mine".into() },
            ),
            (
                json!({"ev": "cancel-delete"}),
                ThemeGalleryEvent::CancelDelete,
            ),
        ] {
            assert_eq!(
                page_event("theme".into(), told.to_string()),
                Some(PageEvent::ThemeGallery { event }),
                "{told}"
            );
        }
    }
    fn ssh_key(id: &str) -> SshKeyRow {
        SshKeyRow {
            id: id.into(),
            name: "laptop".into(),
            badge: "ED25519".into(),
            avatar: SshKeyAvatar::Gray,
            stale: true,
            fingerprint: "SHA256:abc".into(),
            hosts: vec!["nas".into()],
            unused_since: Some("Mar 2026".into()),
            has_public_key: true,
            refused: None,
        }
    }

    /// The SSH Keys page's metadata crosses into the screen's own, and its
    /// events read back as the page tells them.
    #[test]
    fn ssh_keys_cross_both_ways() {
        let state = SshKeysPageState {
            device: vec![],
            imported: vec![ssh_key("A1")],
            missing: vec![ssh_key("B2")],
            deleting: Some("A1".into()),
            notice: Some(SshKeysNotice {
                title: "Key kept".into(),
                message: "nas still signs in with it".into(),
            }),
        };
        let json = page_message(PageMessage::SshKeys { state }).unwrap();
        let moshpit_ui::ssh_keys::Message::Keys(keys) =
            moshpit_ui::ssh_keys::Message::parse(&json).unwrap();
        assert_eq!(keys.imported[0].avatar, moshpit_ui::ssh_keys::Avatar::Gray);
        assert_eq!(keys.imported[0].unused_since.as_deref(), Some("Mar 2026"));
        assert_eq!(keys.deleting.as_deref(), Some("A1"));
        assert_eq!(
            keys.missing[0].id, "B2",
            "B781: the keys not on this device"
        );
        assert_eq!(keys.notice.unwrap().title, "Key kept");
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["imported"][0]["avatar"], "gray");
        assert_eq!(value["imported"][0]["hasPublicKey"], true);

        for (told, event) in [
            (json!({"ev": "done"}), SshKeysEvent::Done),
            (
                json!({"ev": "add"}),
                SshKeysEvent::Add {
                    method: SshKeyAddMethod::Generate,
                },
            ),
            (
                json!({"ev": "add", "method": "import"}),
                SshKeysEvent::Add {
                    method: SshKeyAddMethod::Import,
                },
            ),
            (
                json!({"ev": "copy-public-key", "id": "A1"}),
                SshKeysEvent::CopyPublicKey { id: "A1".into() },
            ),
            (
                json!({"ev": "copy-fingerprint", "id": "A1"}),
                SshKeysEvent::CopyFingerprint { id: "A1".into() },
            ),
            (
                json!({"ev": "share-public-key", "id": "A1"}),
                SshKeysEvent::SharePublicKey { id: "A1".into() },
            ),
            (
                json!({"ev": "confirm-delete", "id": "A1"}),
                SshKeysEvent::ConfirmDelete { id: "A1".into() },
            ),
            (
                json!({"ev": "import-again", "id": "B2"}),
                SshKeysEvent::ImportAgain { id: "B2".into() },
            ),
            (json!({"ev": "cancel-delete"}), SshKeysEvent::CancelDelete),
            (
                json!({"ev": "delete", "id": "A1"}),
                SshKeysEvent::Delete { id: "A1".into() },
            ),
            (json!({"ev": "dismiss-notice"}), SshKeysEvent::DismissNotice),
        ] {
            assert_eq!(
                page_event("ssh-keys".into(), told.to_string()),
                Some(PageEvent::SshKeys { event }),
                "{told}"
            );
        }
        assert_eq!(
            page_event(
                "ssh-keys".into(),
                json!({"ev": "add", "method": "hardware"}).to_string()
            ),
            None,
            "a method Add Key doesn't open on"
        );
    }

    /// What the Shortcuts pages tell reads back as they tell it, and a list
    /// event goes back to the shared model as told.
    #[test]
    fn shortcuts_events_read_as_the_pages_tell_them() {
        for told in [
            json!({"ev": "done"}),
            json!({"ev": "add"}),
            json!({"ev": "edit", "id": "5C00"}),
            json!({"ev": "delete", "id": "5C00"}),
            json!({"ev": "remove-from-toolbar", "id": "5C00"}),
            json!({"ev": "add-to-toolbar", "id": "5C00"}),
            json!({"ev": "move", "from": 1, "to": 3}),
        ] {
            let Some(PageEvent::Shortcuts { event }) =
                page_event("shortcuts".into(), told.to_string())
            else {
                panic!("not read: {told}");
            };
            assert_eq!(event.told(), told);
        }
        assert_eq!(
            page_event(
                "shortcuts".into(),
                json!({"ev": "move", "from": -1, "to": 3}).to_string()
            ),
            None
        );
        let draft = json!({"kind": "text", "payload": "ls", "chipLabel": "ls"});
        assert_eq!(
            page_event(
                "add-shortcut".into(),
                json!({"ev": "change", "draft": draft}).to_string()
            ),
            Some(PageEvent::AddShortcut {
                event: AddShortcutEvent::Change {
                    draft_json: draft.to_string()
                }
            })
        );
        for (told, event) in [
            ("save", AddShortcutEvent::Save),
            ("cancel", AddShortcutEvent::Cancel),
        ] {
            assert_eq!(
                page_event("add-shortcut".into(), json!({"ev": told}).to_string()),
                Some(PageEvent::AddShortcut { event })
            );
        }
        assert_eq!(
            page_event("add-shortcut".into(), json!({"ev": "change"}).to_string()),
            None,
            "a change carries the draft"
        );
    }
    /// The picker's list crosses into the screen's own, its small enums by
    /// id; a row's menu and a prompt's answer are messages it takes; what
    /// it tells reads back, a prompt with its question.
    #[test]
    fn mux_picker_crosses_both_ways() {
        use moshpit_ui::terminal::picker::{self, Act};
        use moshpit_ui::terminal::prompt::{self as asked, Prompt};

        let state = MuxPickerPageState {
            kind: "windows".into(),
            mux: "tmux".into(),
            rows: vec![MuxPickerRow {
                id: "@1".into(),
                index: Some(1),
                name: "build".into(),
                panes: Some(2),
                command: "cargo".into(),
                signal: Some("attention".into()),
                ..MuxPickerRow::default()
            }],
            active: Some("@1".into()),
            send: None,
        };
        let json = page_message(PageMessage::MuxPicker {
            state: state.clone(),
        })
        .unwrap();
        let picker::Message::Picker(parsed) = picker::Message::parse(&json).unwrap() else {
            panic!("not a picker: {json}");
        };
        assert_eq!(parsed.kind, picker::Kind::Windows);
        assert_eq!(parsed.rows[0].panes, Some(2));
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["rows"][0]["signal"], "attention");
        assert_eq!(value["mux"], "tmux");

        // A held share's picker (B622): its agent crosses, each pane's
        // place with it.
        let mut sending = state.clone();
        sending.kind = "panes".into();
        sending.active = None;
        sending.send = Some("claude".into());
        sending.rows[0].window = Some("work · review".into());
        let json = page_message(PageMessage::MuxPicker { state: sending }).unwrap();
        let picker::Message::Picker(parsed) = picker::Message::parse(&json).unwrap() else {
            panic!("not a picker: {json}");
        };
        assert_eq!(parsed.send.as_deref(), Some("claude"));
        assert_eq!(parsed.rows[0].window.as_deref(), Some("work · review"));

        let mut tabs = state;
        tabs.kind = "tabs".into();
        assert_eq!(
            page_message(PageMessage::MuxPicker { state: tabs }),
            Err(PageError::Invalid {
                field: "kind".into(),
                value: "tabs".into()
            })
        );
        assert_eq!(
            picker::Message::parse(
                &page_message(PageMessage::MuxPickerAct {
                    action: MuxPickerAction::Kill,
                    id: "$0".into()
                })
                .unwrap()
            ),
            Ok(picker::Message::Act {
                action: Act::Kill,
                id: "$0".into()
            })
        );
        for text in [Some("build".to_owned()), None] {
            let answer = page_message(PageMessage::Prompted {
                id: 3,
                text: text.clone(),
            })
            .unwrap();
            assert_eq!(
                picker::Message::parse(&answer),
                Ok(picker::Message::Prompted(asked::Answer { id: 3, text }))
            );
        }

        let prompt = Prompt {
            title: "Rename window".into(),
            message: None,
            placeholder: "Name".into(),
            text: "build".into(),
            confirm: "Rename".into(),
            cancel: "Cancel".into(),
        };
        assert_eq!(
            page_event("mux-picker".into(), asked::event(7, &prompt).to_string()),
            Some(PageEvent::MuxPicker {
                event: MuxPickerEvent::Prompt {
                    question: TextPromptQuestion {
                        id: 7,
                        title: "Rename window".into(),
                        message: None,
                        placeholder: "Name".into(),
                        text: "build".into(),
                        confirm: "Rename".into(),
                        cancel: "Cancel".into(),
                    }
                }
            })
        );
        for (told, event) in [
            (
                json!({"ev": "select", "id": "@2"}),
                MuxPickerEvent::Select { id: "@2".into() },
            ),
            (
                json!({"ev": "new", "name": null}),
                MuxPickerEvent::New { name: None },
            ),
            (
                json!({"ev": "new", "name": "logs"}),
                MuxPickerEvent::New {
                    name: Some("logs".into()),
                },
            ),
            (
                json!({"ev": "rename", "id": "@2", "name": "web"}),
                MuxPickerEvent::Rename {
                    id: "@2".into(),
                    name: "web".into(),
                },
            ),
            (
                json!({"ev": "kill", "id": "@2"}),
                MuxPickerEvent::Kill { id: "@2".into() },
            ),
        ] {
            assert_eq!(
                page_event("mux-picker".into(), told.to_string()),
                Some(PageEvent::MuxPicker { event }),
                "{told}"
            );
        }
        assert_eq!(
            page_event(
                "mux-picker".into(),
                json!({"ev": "rename", "id": "@2"}).to_string()
            ),
            None,
            "a rename names the new name"
        );
    }
    /// A folder crosses into the screen's own; an entry's menu and a
    /// prompt's answer are messages it takes; what it tells reads back.
    #[test]
    fn remote_folder_crosses_both_ways() {
        use moshpit_ui::terminal::folder::{self, Act};

        let state = RemoteFolderPageState {
            name: "shots".into(),
            path: "/home/me/shots".into(),
            parents: vec!["/home/me".into(), "/home".into(), "/".into()],
            pushed: true,
            uploading: Some(0.5),
            media: vec![RemoteFolderItem {
                path: "/home/me/shots/a.mov".into(),
                name: "a.mov".into(),
                video: true,
                symbol: "film".into(),
                duration: Some("1:05".into()),
                ..RemoteFolderItem::default()
            }],
            edit_failure: Some(RemoteFolderFailure {
                title: "Can't rename".into(),
                message: "Permission denied".into(),
            }),
            edit_failure_id: 4,
            ..RemoteFolderPageState::default()
        };
        let json = page_message(PageMessage::RemoteFolder { state }).unwrap();
        let folder::Message::Folder(parsed) = folder::Message::parse(&json).unwrap() else {
            panic!("not a folder: {json}");
        };
        assert_eq!(parsed.media[0].duration.as_deref(), Some("1:05"));
        assert_eq!(parsed.edit_failure.unwrap().title, "Can't rename");
        assert_eq!(parsed.edit_failure_id, 4);
        assert_eq!(parsed.uploading, Some(0.5));
        assert_eq!(
            folder::Message::parse(
                &page_message(PageMessage::RemoteFolderAct {
                    action: RemoteFolderAction::Share,
                    path: "/a".into()
                })
                .unwrap()
            ),
            Ok(folder::Message::Act {
                action: Act::Share,
                path: "/a".into()
            })
        );
        assert!(matches!(
            folder::Message::parse(
                &page_message(PageMessage::Prompted { id: 2, text: None }).unwrap()
            ),
            Ok(folder::Message::Prompted(_))
        ));

        for (told, event) in [
            (
                json!({"ev": "open", "path": "/a"}),
                RemoteFolderEvent::Open { path: "/a".into() },
            ),
            (
                json!({"ev": "share", "path": "/a"}),
                RemoteFolderEvent::Share { path: "/a".into() },
            ),
            (json!({"ev": "back"}), RemoteFolderEvent::Back),
            (
                json!({"ev": "climb", "path": "/"}),
                RemoteFolderEvent::Climb { path: "/".into() },
            ),
            (
                json!({"ev": "add", "item": "photos"}),
                RemoteFolderEvent::Add {
                    item: RemoteFolderUpload::Photos,
                },
            ),
            (
                json!({"ev": "add", "item": "files"}),
                RemoteFolderEvent::Add {
                    item: RemoteFolderUpload::Files,
                },
            ),
            (
                json!({"ev": "new-folder", "name": "new"}),
                RemoteFolderEvent::NewFolder { name: "new".into() },
            ),
            (
                json!({"ev": "rename", "path": "/a", "name": "b"}),
                RemoteFolderEvent::Rename {
                    path: "/a".into(),
                    name: "b".into(),
                },
            ),
            (
                json!({"ev": "delete", "path": "/a"}),
                RemoteFolderEvent::Delete { path: "/a".into() },
            ),
            (json!({"ev": "refresh"}), RemoteFolderEvent::Refresh),
            (
                json!({"ev": "dismiss", "failure": "edit"}),
                RemoteFolderEvent::Dismiss {
                    failure: RemoteFolderAlert::Edit,
                },
            ),
        ] {
            assert_eq!(
                page_event("remote-folder".into(), told.to_string()),
                Some(PageEvent::RemoteFolder { event }),
                "{told}"
            );
        }
        for bad in [
            json!({"ev": "add", "item": "camera"}),
            json!({"ev": "dismiss", "failure": "upload"}),
            json!({"ev": "prompt", "id": 1}),
        ] {
            assert_eq!(
                page_event("remote-folder".into(), bad.to_string()),
                None,
                "{bad}"
            );
        }
    }
    /// Add Key's form crosses into the screen's own, never a key; its
    /// events read back as it tells them.
    #[test]
    fn add_key_crosses_both_ways() {
        use moshpit_ui::add_key::{self as screen, Algorithm, Method};

        let state = AddKeyPageState {
            method: "import".into(),
            algorithm: "rsa".into(),
            require_face_id: false,
            secure_enclave: false,
            pem: AddKeyPem {
                empty: false,
                encrypted: true,
            },
            name: "work".into(),
            comment: String::new(),
            public_line: "ssh-ed25519 AAAA work".into(),
            fields: 3,
            can_submit: false,
            working: false,
            fingerprint: None,
            error: Some("Wrong passphrase".into()),
            again: false,
        };
        let json = page_message(PageMessage::AddKey {
            state: state.clone(),
        })
        .unwrap();
        let screen::Message::AddKey(form) = screen::Message::parse(&json).unwrap();
        assert_eq!(form.method, Method::Import);
        assert_eq!(form.algorithm, Algorithm::Rsa);
        assert!(form.pem.encrypted);
        assert_eq!(form.fields, 3);
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["requireFaceID"], false);
        assert!(value.get("fingerprint").is_none());
        let mut dsa = state;
        dsa.algorithm = "dsa".into();
        assert_eq!(
            page_message(PageMessage::AddKey { state: dsa }),
            Err(PageError::Invalid {
                field: "algorithm".into(),
                value: "dsa".into()
            })
        );

        for (told, event) in [
            (
                json!({"ev": "method", "value": "import"}),
                AddKeyEvent::Method {
                    id: "import".into(),
                },
            ),
            (
                json!({"ev": "algorithm", "value": "rsa"}),
                AddKeyEvent::Algorithm { id: "rsa".into() },
            ),
            (
                json!({"ev": "set", "key": "requireFaceID", "value": false}),
                AddKeyEvent::Set {
                    key: AddKeySwitch::RequireFaceId,
                    on: false,
                },
            ),
            (
                json!({"ev": "set", "key": "secureEnclave", "value": true}),
                AddKeyEvent::Set {
                    key: AddKeySwitch::SecureEnclave,
                    on: true,
                },
            ),
            (
                json!({"ev": "name", "value": "work"}),
                AddKeyEvent::Name {
                    text: "work".into(),
                },
            ),
            (
                json!({"ev": "comment", "value": "me@laptop"}),
                AddKeyEvent::Comment {
                    text: "me@laptop".into(),
                },
            ),
            (
                json!({"ev": "public-line", "value": "ssh-ed25519 AAAA"}),
                AddKeyEvent::PublicLine {
                    text: "ssh-ed25519 AAAA".into(),
                },
            ),
            (
                json!({"ev": "import-file", "target": "public"}),
                AddKeyEvent::ImportFile {
                    target: AddKeyFile::PublicKey,
                },
            ),
            (json!({"ev": "submit"}), AddKeyEvent::Submit),
            (json!({"ev": "cancel"}), AddKeyEvent::Cancel),
            (json!({"ev": "dismiss-error"}), AddKeyEvent::DismissError),
        ] {
            assert_eq!(
                page_event("add-key".into(), told.to_string()),
                Some(PageEvent::AddKey { event }),
                "{told}"
            );
        }
        for bad in [
            json!({"ev": "algorithm", "value": "dsa"}),
            json!({"ev": "set", "key": "pin", "value": true}),
            json!({"ev": "import-file", "target": "clipboard"}),
            json!({"ev": "name"}),
            // B602: the host chips are gone.
            json!({"ev": "host", "name": "nas"}),
        ] {
            assert_eq!(page_event("add-key".into(), bad.to_string()), None, "{bad}");
        }
    }
}
