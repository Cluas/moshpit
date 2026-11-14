//! The screens an NSH surface runs, by name, and the NSH root of each.
//!
//! A host runs a screen by its name (`"recent-log"`) with [`Params`]: what
//! the screen cannot know by itself — the language the app shows and the
//! colors of the theme the user picked. Each root draws in the app's
//! tokens ([`crate::theme`]), dark only, with the app's translations
//! ([`crate::strings`]), and finds its surface's [`Channel`] by the id its
//! props carry.

use nsh_core::{element::Root, platform::Appearance};
use serde::{Deserialize, Serialize};

use crate::{
    accent, accent_editor, add_key, add_shortcut, app_icon,
    channel::{Channel, SurfaceId},
    connection_form, feedback, home, host_setup, key_picker, recent_log, sessions, settings,
    shortcuts, ssh_keys, strings, terminal,
    theme::{self, AppTheme},
    theme_editor, theme_gallery, ui, voice_engine, voice_language, whisper_model,
};

/// A screen moved from SwiftUI to NSH.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    /// Settings › Recent Log ([`recent_log`]).
    RecentLog,
    /// Settings › App Icon ([`app_icon`]).
    AppIcon,
    /// Settings › Accent ([`accent`]).
    Accent,
    /// Settings › Voice Input › Recognition ([`voice_engine`]).
    VoiceEngine,
    /// Settings › Voice Input › Language ([`voice_language`]).
    VoiceLanguage,
    /// Settings, the root of its stack ([`settings`]).
    Settings,
    /// Settings › Send Feedback ([`feedback`]).
    Feedback,
    /// Settings › Voice Input › Model ([`whisper_model`]).
    WhisperModel,
    /// Settings › Theme ([`theme_gallery`]).
    Theme,
    /// Settings › Theme › a theme's editor ([`theme_editor`]).
    ThemeEditor,
    /// Settings › Accent › an accent's editor ([`accent_editor`]).
    AccentEditor,
    /// Home, the app's first screen ([`home`]).
    Home,
    /// Home › Add Connection and Edit Connection ([`connection_form`]).
    ConnectionForm,
    /// Settings › SSH Keys ([`ssh_keys`]).
    SshKeys,
    /// Add Connection › Select Key ([`key_picker`]).
    KeyPicker,
    /// SSH Keys › Add Key ([`add_key`]).
    AddKey,
    /// Settings › Shortcuts ([`shortcuts`]).
    Shortcuts,
    /// Shortcuts › Add Shortcut, or a custom shortcut's editor
    /// ([`add_shortcut`]).
    AddShortcut,
    /// Settings › Set up this host ([`host_setup`]).
    HostSetup,
    /// A connection's terminal ([`terminal`]).
    Terminal,
    /// The terminal's windows, sessions or panes ([`terminal::picker`]).
    MuxPicker,
    /// The terminal's Install Assist ([`terminal::install`]).
    InstallAssist,
    /// A folder of the host, from a path link or the Files key
    /// ([`terminal::folder`]).
    RemoteFolder,
    /// Home › a host's sessions ([`sessions`]).
    Sessions,
    /// The `ui` layer's gallery, a debug screen ([`ui::gallery`]).
    UiGallery,
}

impl Screen {
    /// Every screen.
    pub const ALL: [Screen; 25] = [
        Screen::RecentLog,
        Screen::AppIcon,
        Screen::Accent,
        Screen::VoiceEngine,
        Screen::VoiceLanguage,
        Screen::Settings,
        Screen::Feedback,
        Screen::WhisperModel,
        Screen::Theme,
        Screen::ThemeEditor,
        Screen::AccentEditor,
        Screen::Home,
        Screen::ConnectionForm,
        Screen::SshKeys,
        Screen::KeyPicker,
        Screen::AddKey,
        Screen::Shortcuts,
        Screen::AddShortcut,
        Screen::HostSetup,
        Screen::Terminal,
        Screen::MuxPicker,
        Screen::InstallAssist,
        Screen::RemoteFolder,
        Screen::Sessions,
        Screen::UiGallery,
    ];

    /// The screen called `name`, as a host names it.
    pub fn named(name: &str) -> Option<Screen> {
        Self::ALL.into_iter().find(|screen| screen.name() == name)
    }

    /// Whether it draws in the light appearance when a device test asks
    /// (`Params::light`): the `ui` layer's gallery and the screens the
    /// redesign rebuilt on it. One list, so the pages adding themselves do
    /// not each rewrite the appearance line.
    pub fn draws_light(self) -> bool {
        matches!(
            self,
            Screen::UiGallery
                | Screen::Home
                | Screen::ConnectionForm
                | Screen::Theme
                | Screen::ThemeEditor
                | Screen::Accent
                | Screen::AccentEditor
                | Screen::AppIcon
                | Screen::RemoteFolder
                | Screen::MuxPicker
                | Screen::SshKeys
                | Screen::AddKey
                | Screen::KeyPicker
                | Screen::Shortcuts
                | Screen::AddShortcut
                | Screen::VoiceEngine
                | Screen::VoiceLanguage
                | Screen::WhisperModel
                | Screen::Feedback
                | Screen::RecentLog
                | Screen::HostSetup
                | Screen::Sessions
                | Screen::InstallAssist
                | Screen::Settings
        )
    }

    /// Whether it follows the system's appearance (R18, UI-DESIGN §10.6):
    /// light when the system is, as the surface's trait collection says,
    /// switching as it switches. The screens on it draw with nothing the
    /// light appearance cannot have: no colour of the old dark palette (a
    /// literal, the old helpers that made one) in their files or in what
    /// they draw with (`ui/tests/ui_light_guard.rs` holds every screen
    /// listed here to that), so none ships half light. A screen joins once
    /// its page is clean (R15, R16's pages, Settings the last; the terminal,
    /// R12, whose appearance is its colour scheme's:
    /// [`terminal_appearance`]). Every screen is on it now; a new one stays
    /// off until its page is clean.
    ///
    /// The app forces every page dark until R18's host part lets the
    /// system's appearance through (`.preferredColorScheme(.dark)` on the
    /// hosts): until then this list changes nothing on screen.
    pub fn follows_system(self) -> bool {
        matches!(
            self,
            Screen::UiGallery
                | Screen::Home
                | Screen::ConnectionForm
                | Screen::Theme
                | Screen::ThemeEditor
                | Screen::Accent
                | Screen::AccentEditor
                | Screen::AppIcon
                | Screen::RemoteFolder
                | Screen::MuxPicker
                | Screen::SshKeys
                | Screen::AddKey
                | Screen::KeyPicker
                | Screen::Shortcuts
                | Screen::AddShortcut
                | Screen::Terminal
                | Screen::InstallAssist
                | Screen::Sessions
                | Screen::VoiceEngine
                | Screen::VoiceLanguage
                | Screen::WhisperModel
                | Screen::Feedback
                | Screen::RecentLog
                | Screen::HostSetup
                | Screen::Settings
        )
    }

    /// The appearance the screen asks the platform for:
    ///
    /// - what a device test forces (`Params::light`): light on a screen that
    ///   draws it ([`Self::draws_light`]), dark otherwise;
    /// - on a screen that follows the system ([`Self::follows_system`]),
    ///   what the user chose in Settings › Appearance (`Params::appearance`):
    ///   the system's unless they chose light or dark;
    /// - dark on every other;
    /// - the terminal's colour scheme's on the terminal, whatever the
    ///   others ([`Params::terminal_is_light`]).
    pub fn appearance(self, params: &Params) -> Appearance {
        // The terminal's chrome takes its terminal theme's appearance
        // (UI-DESIGN §6.3, R12: `Params::terminal`), not the system's nor
        // what a device test forces.
        if self == Screen::Terminal {
            return if params.terminal_is_light() {
                Appearance::Light
            } else {
                Appearance::Dark
            };
        }
        match params.light {
            Some(true) if self.draws_light() => Appearance::Light,
            Some(_) => Appearance::Dark,
            None if !self.follows_system() => Appearance::Dark,
            // What the user chose in Settings › Appearance.
            None => match params.appearance.unwrap_or_default() {
                AppearanceChoice::System => Appearance::System,
                AppearanceChoice::Light => Appearance::Light,
                AppearanceChoice::Dark => Appearance::Dark,
            },
        }
    }

    /// The tokens the screen draws from: the app theme's. On the terminal,
    /// its ground (`terminal-bg`: around the slot, under the bar, the
    /// panels' strip) is the terminal theme's background, in both
    /// appearances (B488): the app theme's near black beside a grey
    /// terminal read as a black frame.
    pub fn tokens(self, params: &Params) -> nsh_core::element::Tokens {
        let terminal = if self == Screen::Terminal {
            params.terminal.as_deref().and_then(theme::parse_hex)
        } else {
            None
        };
        theme::tokens_on(&params.theme, terminal)
    }

    /// What a host calls it.
    pub fn name(self) -> &'static str {
        match self {
            Screen::RecentLog => "recent-log",
            Screen::AppIcon => "app-icon",
            Screen::Accent => "accent",
            Screen::VoiceEngine => "voice-engine",
            Screen::VoiceLanguage => "voice-language",
            Screen::Settings => "settings",
            Screen::Feedback => "feedback",
            Screen::WhisperModel => "whisper-model",
            Screen::Theme => "theme",
            Screen::ThemeEditor => "theme-editor",
            Screen::AccentEditor => "accent-editor",
            Screen::Home => "home",
            Screen::ConnectionForm => "connection-form",
            Screen::SshKeys => "ssh-keys",
            Screen::KeyPicker => "key-picker",
            Screen::AddKey => "add-key",
            Screen::Shortcuts => "shortcuts",
            Screen::AddShortcut => "add-shortcut",
            Screen::HostSetup => "host-setup",
            Screen::Terminal => "terminal",
            Screen::MuxPicker => "mux-picker",
            Screen::InstallAssist => "install-assist",
            Screen::RemoteFolder => "remote-folder",
            Screen::Sessions => "sessions",
            Screen::UiGallery => "ui-gallery",
        }
    }

    /// Whether `message` (a host's post, JSON) is one of this screen's.
    pub fn check(self, message: &str) -> Result<(), String> {
        match self {
            Screen::RecentLog => recent_log::Message::parse(message).map(drop),
            Screen::AppIcon => app_icon::Message::parse(message).map(drop),
            Screen::Accent => accent::Message::parse(message).map(drop),
            Screen::VoiceEngine => voice_engine::Message::parse(message).map(drop),
            Screen::VoiceLanguage => voice_language::Message::parse(message).map(drop),
            Screen::Settings => settings::Message::parse(message).map(drop),
            Screen::Feedback => feedback::Message::parse(message).map(drop),
            Screen::WhisperModel => whisper_model::Message::parse(message).map(drop),
            Screen::Theme => theme_gallery::Message::parse(message).map(drop),
            Screen::ThemeEditor => theme_editor::Message::parse(message).map(drop),
            Screen::AccentEditor => accent_editor::Message::parse(message).map(drop),
            Screen::Home => home::Message::parse(message).map(drop),
            Screen::ConnectionForm => connection_form::Message::parse(message).map(drop),
            Screen::SshKeys => ssh_keys::Message::parse(message).map(drop),
            Screen::KeyPicker => key_picker::Message::parse(message).map(drop),
            Screen::AddKey => add_key::Message::parse(message).map(drop),
            Screen::Shortcuts => shortcuts::Message::parse(message).map(drop),
            Screen::AddShortcut => add_shortcut::Message::parse(message).map(drop),
            Screen::HostSetup => host_setup::Message::parse(message).map(drop),
            Screen::Terminal => terminal::Message::parse(message).map(drop),
            Screen::MuxPicker => terminal::picker::Message::parse(message).map(drop),
            Screen::InstallAssist => terminal::install::Message::parse(message).map(drop),
            Screen::RemoteFolder => terminal::folder::Message::parse(message).map(drop),
            Screen::Sessions => sessions::Message::parse(message).map(drop),
            Screen::UiGallery => Err("the gallery takes no messages".to_owned()),
        }
    }

    /// The root that runs this screen on `channel`'s surface, as `params`
    /// say. The channel starts taking the screen's messages.
    pub fn root(self, channel: &Channel, params: &Params) -> Root {
        channel.start(self, params.clone());
        let surface: SurfaceId = channel.id();
        let root = match self {
            Screen::RecentLog => {
                Root::new::<recent_log::RecentLogScreen>(move || recent_log::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::AppIcon => {
                Root::new::<app_icon::AppIconScreen>(move || app_icon::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::Accent => Root::new::<accent::AccentScreen>(move || accent::ScreenProps {
                surface: surface.into(),
            }),
            Screen::VoiceEngine => {
                Root::new::<voice_engine::VoiceEngineScreen>(move || voice_engine::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::VoiceLanguage => Root::new::<voice_language::VoiceLanguageScreen>(move || {
                voice_language::ScreenProps {
                    surface: surface.into(),
                }
            }),
            Screen::Settings => {
                Root::new::<settings::SettingsScreen>(move || settings::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::Feedback => {
                Root::new::<feedback::FeedbackScreen>(move || feedback::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::WhisperModel => {
                Root::new::<whisper_model::WhisperModelScreen>(move || whisper_model::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::Theme => {
                Root::new::<theme_gallery::ThemeGalleryScreen>(move || theme_gallery::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::ThemeEditor => {
                Root::new::<theme_editor::ThemeEditorScreen>(move || theme_editor::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::AccentEditor => {
                Root::new::<accent_editor::AccentEditorScreen>(move || accent_editor::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::Home => Root::new::<home::HomeScreen>(move || home::ScreenProps {
                surface: surface.into(),
            }),
            Screen::ConnectionForm => {
                Root::new::<connection_form::ConnectionFormScreen>(move || {
                    connection_form::ScreenProps {
                        surface: surface.into(),
                    }
                })
            }
            Screen::SshKeys => {
                Root::new::<ssh_keys::SshKeysScreen>(move || ssh_keys::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::KeyPicker => {
                Root::new::<key_picker::KeyPickerScreen>(move || key_picker::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::AddKey => Root::new::<add_key::AddKeyScreen>(move || add_key::ScreenProps {
                surface: surface.into(),
            }),
            Screen::Shortcuts => {
                Root::new::<shortcuts::ShortcutsScreen>(move || shortcuts::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::AddShortcut => {
                Root::new::<add_shortcut::AddShortcutScreen>(move || add_shortcut::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::HostSetup => {
                Root::new::<host_setup::HostSetupScreen>(move || host_setup::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::Terminal => {
                Root::new::<terminal::TerminalScreen>(move || terminal::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::MuxPicker => Root::new::<terminal::picker::MuxPickerScreen>(move || {
                terminal::picker::ScreenProps {
                    surface: surface.into(),
                }
            }),
            Screen::InstallAssist => {
                Root::new::<terminal::install::InstallAssistScreen>(move || {
                    terminal::install::ScreenProps {
                        surface: surface.into(),
                    }
                })
            }
            Screen::RemoteFolder => Root::new::<terminal::folder::RemoteFolderScreen>(move || {
                terminal::folder::ScreenProps {
                    surface: surface.into(),
                }
            }),
            Screen::Sessions => {
                Root::new::<sessions::SessionsScreen>(move || sessions::ScreenProps {
                    surface: surface.into(),
                })
            }
            Screen::UiGallery => {
                Root::new::<ui::gallery::GalleryScreen>(move || ui::gallery::ScreenProps {
                    surface: surface.into(),
                })
            }
        };
        root.tokens(self.tokens(params))
            .appearance(self.appearance(params))
            .translations(strings::translations())
    }
}

/// The appearance chosen in Settings › Appearance (R18; the host keeps it
/// as `moshpit.settings.appearance`): the system's, or always light, or
/// always dark.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppearanceChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl AppearanceChoice {
    /// All three, in Settings' segmented control's order.
    pub const ALL: [AppearanceChoice; 3] = [
        AppearanceChoice::System,
        AppearanceChoice::Light,
        AppearanceChoice::Dark,
    ];

    /// Its id, as the setting stores it.
    pub fn id(self) -> &'static str {
        match self {
            AppearanceChoice::System => "system",
            AppearanceChoice::Light => "light",
            AppearanceChoice::Dark => "dark",
        }
    }
}

/// The relative luminance (WCAG's) past which a terminal's background is
/// light: where black and white words contrast with it alike, so above it
/// dark words read better (the light appearance), below it white ones.
pub const LIGHT_BACKGROUND: f32 = 0.179;

/// Whether a terminal background of `background` is light
/// ([`LIGHT_BACKGROUND`]): Solarized Light and a white scheme are,
/// GitHub Dark and Solarized Dark aren't.
pub fn is_light_background(background: nsh_core::color::Rgba) -> bool {
    crate::ui::tokens::luminance(background) > LIGHT_BACKGROUND
}

/// The appearance of a terminal whose colour scheme's background is
/// `background` (UI-DESIGN §2.2, §6.3): the one rule
/// ([`is_light_background`]). The terminal's chrome follows its scheme,
/// not the system: a white bar over a black terminal would be the one
/// thing on screen in the other appearance.
pub fn terminal_appearance(background: nsh_core::color::Rgba) -> Appearance {
    if is_light_background(background) {
        Appearance::Light
    } else {
        Appearance::Dark
    }
}

/// What a host says about a screen before it runs it (JSON, all optional):
/// `{"locale": "zh-Hans", "theme": {"accent": "#6C6BEF", …}, "bar":
/// {"height": 56}}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// The language the app shows (`Bundle.main.preferredLocalizations`):
    /// the screen's text follows the app's, not the system's list, which
    /// can differ (a language the app has no translations for, a per-app
    /// language setting). `None`: the system's.
    pub locale: Option<String>,
    /// The theme the user picked (Settings › Appearance).
    pub theme: AppTheme,
    /// The navigation bar the screen's page stands in for, as the host
    /// measured it (`None`: the platform's standard bar).
    pub bar: Option<crate::nav_bar::BarMetrics>,
    /// The title of the page under this one, which its back button shows,
    /// as the host's navigation has it (`None`: the screen's own guess). A
    /// page pushed from more than one place needs it.
    pub back: Option<String>,
    /// Draw in this appearance whatever the system's (`true`: light, on a
    /// screen that can, [`Screen::draws_light`]; `false`: dark): a device
    /// test's, for its pictures in both. The app never sets it; `None`
    /// leaves the screen its own ([`Screen::appearance`]).
    pub light: Option<bool>,
    /// The appearance chosen in Settings › Appearance, as the host keeps it
    /// (`"system"`, `"light"`, `"dark"`): what the screens that follow the
    /// system take ([`Screen::appearance`]). `None`: the system's.
    pub appearance: Option<AppearanceChoice>,
    /// The text size relative to the default (1.0), in place of the
    /// system's Dynamic Type: a device test's (Home at AX3). The app never
    /// sets it; `None` follows the system.
    #[serde(rename = "fontScale")]
    pub font_scale: Option<f32>,
    /// Which page of a screen of several the host runs it for (Settings'
    /// pages: `"terminal"`, [`crate::settings::Part`]); `None`: its first.
    pub page: Option<String>,
    /// The terminal theme's background (`#RRGGBB`): the terminal screen's
    /// chrome is light on a light one, dark otherwise
    /// ([`Params::terminal_is_light`]).
    pub terminal: Option<String>,
}

impl Params {
    /// The params of `json` (`None` or empty: the defaults).
    pub fn parse(json: Option<&str>) -> Result<Params, String> {
        let json = match json.map(str::trim) {
            None | Some("") => return Ok(Params::default()),
            Some(json) => json,
        };
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("screen params: {e}"))?;
        // An object: serde would also read a struct from an array.
        if !value.is_object() {
            return Err("screen params: a JSON object".to_owned());
        }
        serde_json::from_value(value).map_err(|e| format!("screen params: {e}"))
    }

    /// Whether the terminal theme's background is light: dark ink reads
    /// better on it than white does (its luminance is past the point where
    /// the two contrast with it alike, WCAG's 0.179). None or unreadable:
    /// dark, as every terminal theme was.
    pub fn terminal_is_light(&self) -> bool {
        self.terminal
            .as_deref()
            .and_then(crate::theme::parse_hex)
            .is_some_and(is_light_background)
    }

    /// [`Self::locale`] as NSH's locale (`None` when absent or not a
    /// language tag).
    pub fn nsh_locale(&self) -> Option<nsh_core::platform::Locale> {
        self.locale
            .as_deref()
            .and_then(nsh_core::platform::Locale::parse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screens_are_found_by_their_names() {
        for screen in Screen::ALL {
            assert_eq!(Screen::named(screen.name()), Some(screen));
        }
        assert_eq!(Screen::named("recent-log"), Some(Screen::RecentLog));
        assert_eq!(Screen::named("app-icon"), Some(Screen::AppIcon));
        assert_eq!(Screen::named("accent"), Some(Screen::Accent));
        assert_eq!(Screen::named("voice-engine"), Some(Screen::VoiceEngine));
        assert_eq!(Screen::named("voice-language"), Some(Screen::VoiceLanguage));
        assert_eq!(Screen::named("settings"), Some(Screen::Settings));
        assert_eq!(Screen::named("feedback"), Some(Screen::Feedback));
        assert_eq!(Screen::named("whisper-model"), Some(Screen::WhisperModel));
        assert_eq!(Screen::named("theme"), Some(Screen::Theme));
        assert_eq!(Screen::named("theme-editor"), Some(Screen::ThemeEditor));
        assert_eq!(Screen::named("accent-editor"), Some(Screen::AccentEditor));
        assert_eq!(Screen::named("home"), Some(Screen::Home));
        assert_eq!(
            Screen::named("connection-form"),
            Some(Screen::ConnectionForm)
        );
        assert_eq!(Screen::named("ssh-keys"), Some(Screen::SshKeys));
        assert_eq!(Screen::named("key-picker"), Some(Screen::KeyPicker));
        assert_eq!(Screen::named("add-key"), Some(Screen::AddKey));
        assert_eq!(Screen::named("shortcuts"), Some(Screen::Shortcuts));
        assert_eq!(Screen::named("add-shortcut"), Some(Screen::AddShortcut));
        assert_eq!(Screen::named("host-setup"), Some(Screen::HostSetup));
        assert_eq!(Screen::named("terminal"), Some(Screen::Terminal));
        assert_eq!(Screen::named("mux-picker"), Some(Screen::MuxPicker));
        assert_eq!(Screen::named("install-assist"), Some(Screen::InstallAssist));
        assert_eq!(Screen::named("remote-folder"), Some(Screen::RemoteFolder));
        assert_eq!(Screen::named("ui-gallery"), Some(Screen::UiGallery));
        assert_eq!(Screen::named("terminal-screen"), None);
    }

    #[test]
    fn params_default_and_parse() {
        assert_eq!(Params::parse(None), Ok(Params::default()));
        assert_eq!(Params::parse(Some(" ")), Ok(Params::default()));
        let params = Params::parse(Some(
            r##"{"locale":"zh-Hans","theme":{"accent":"#53DCC9","accentPressed":"#3EBFA9","screenBG":"#05090A","terminalBG":"#020505"}}"##,
        ))
        .unwrap();
        assert_eq!(params.locale.as_deref(), Some("zh-Hans"));
        assert_eq!(
            params.nsh_locale().map(|l| l.to_string()),
            Some("zh-Hans".to_owned())
        );
        assert_ne!(params.theme, AppTheme::default());
        assert_eq!(params.bar, None);
        let measured = Params::parse(Some(r#"{"bar":{"height":56}}"#)).unwrap();
        assert_eq!(measured.bar.map(|b| b.height), Some(56.0));
        assert_eq!(measured.back, None);
        let solarized = Params::parse(Some(r##"{"terminal":"#FDF6E3"}"##)).unwrap();
        assert!(solarized.terminal_is_light(), "a light terminal theme");
        for dark in [
            r##"{"terminal":"#1E1E1E"}"##,
            r##"{"terminal":"#5A5A5A"}"##,
            "{}",
        ] {
            assert!(
                !Params::parse(Some(dark)).unwrap().terminal_is_light(),
                "{dark}"
            );
        }
        assert!(
            !Params::parse(Some(r#"{"terminal":"tan"}"#))
                .unwrap()
                .terminal_is_light()
        );
        let pushed = Params::parse(Some(r#"{"back":"Recognition"}"#)).unwrap();
        assert_eq!(pushed.back.as_deref(), Some("Recognition"));
        assert!(Params::parse(Some(r#"{"bar":{"tall":true}}"#)).is_err());
        assert!(
            Params::parse(Some(r#"{"colour":1}"#)).is_err(),
            "unknown keys are refused"
        );
        assert!(Params::parse(Some("[]")).is_err());
    }
}
