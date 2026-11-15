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

mod accent;
mod feedback_page;
mod install_assist;
mod settings;
mod theme_gallery;
mod whisper_model;

pub use accent::*;
pub use install_assist::*;
pub use settings::*;
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
    /// `{"op": "models", …}`: Voice Input › Model.
    WhisperModel { state: WhisperModelsState },
    /// `{"op": "accents", …}`: Settings › Accent.
    Accent { state: AccentPageState },
    /// `{"op": "themes", …}`: Settings › Theme.
    ThemeGallery { state: ThemeGalleryPageState },
    /// `{"op": "reveal"}`: the user's own themes scrolled to the top (a
    /// capture run's).
    ThemeGalleryReveal,
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
        _ => None,
    }
}

#[cfg(test)]
mod tests {
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
}
