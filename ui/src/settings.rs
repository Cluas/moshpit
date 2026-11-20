//! Settings (`docs/nsh/UI-DESIGN.md` §6.4) in NSH: a hub that fits on one
//! screen and the five pages it opens (Appearance, Terminal, Connections,
//! Notifications, Voice input), on the redesign's components (`ui::row`,
//! `ui::section`, `ui::control`, `ui::swatch`).
//!
//! One screen draws them all: the host runs it once per page of Settings'
//! stack, saying which page in the screen's params (`{"page": "terminal"}`,
//! [`Part`]; none: the hub). The host owns the settings (the app's
//! `AppSettings`, the stores behind the counts, the catalogs behind the
//! names): it posts what the pages show ([`Message`]), `{"op": "settings",
//! …}` ([`SettingsState`]), again whenever any of it changes. A page tells
//! it what the user does, and the host applies it as the SwiftUI page does:
//!
//! - `{"ev": "set", "key": …, "value": …}` ([`Setting`]): a switch, a
//!   segment, a swatch, the font size's steps or a menu's pick;
//! - `{"ev": "open", "page": …}` ([`Page`], [`Part`]): a row that opens a
//!   page, a sheet or a link. The SwiftUI navigation stack stays the
//!   navigation's owner: the host pushes the page (a part of Settings is
//!   this screen again), presents the sheet or opens the link;
//! - `{"ev": "done"}`: the hub's Done; `{"ev": "back"}`: a page's back
//!   button.
//!
//! The hub wears Home's large title ([`crate::home::bar::large_bar`]). Its
//! version row's long-press menu (Copy) is the host's, on the NSH surface,
//! for the view identified `settings-version`. The hub's rows carry no help
//! text;
//! each page's footnote is at most two lines.

use std::{rc::Rc, sync::Arc};

use nsh_core::{color::Rgba, prelude::*};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    app_icon,
    channel::{self, Channel},
    face::{self, Design},
    home::bar as large,
    nav_bar::{self, BarMetrics},
    os::Os,
    screen::{AppearanceChoice, Screen},
    theme::{self, AppTheme},
    ui::{
        button::{IconButtonSize, IconVariant, icon_button},
        control::{option, segmented, select, switch},
        empty::{EmptyVariant, empty},
        field::{field, number_input, text_input},
        icon::{IconSize, Tone, icon},
        page,
        row::{GAP, LEADING, PADDING_X, Row, RowTone, row},
        section::{Inset, section},
        status::Status,
        swatch::{self, CursorColor, cursor_colors},
        tokens::{number, token},
        typography::TextToken,
    },
    voice_engine::Engine,
};

/// A name and its color: the accent the app wears.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Swatch {
    pub name: String,
    #[serde(
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub color: Rgba,
}

/// The terminal's color scheme: its name, what it draws its cells and words
/// in, and its eight ANSI colors, black to white (the row's swatch strip).
/// The Terminal page's preview draws in them, as the terminal does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalTheme {
    pub name: String,
    /// The cells' background.
    #[serde(
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub background: Rgba,
    /// The words' color where nothing sets one.
    #[serde(
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub foreground: Rgba,
    #[serde(with = "colors")]
    pub swatches: Vec<Rgba>,
}

/// A list of colors written `#RRGGBB`.
mod colors {
    use nsh_core::color::Rgba;
    use serde::{Deserialize, Deserializer, Serializer, ser::SerializeSeq};

    pub fn serialize<S: Serializer>(colors: &[Rgba], serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(colors.len()))?;
        for color in colors {
            seq.serialize_element(&crate::theme::hex_string(*color))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Rgba>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .iter()
            .map(|text| {
                crate::theme::parse_hex(text).ok_or_else(|| {
                    serde::de::Error::custom(format!("{text:?} is not #RRGGBB or #RRGGBBAA"))
                })
            })
            .collect()
    }
}

/// One choice of a menu: its id (what the setting stores) and its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub id: String,
    pub name: String,
}

/// The cursor's shape (`CursorShape`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorShape {
    #[default]
    Block,
    Bar,
    Underline,
}

impl CursorShape {
    /// All three, in the segmented control's order.
    pub const ALL: [CursorShape; 3] =
        [CursorShape::Block, CursorShape::Bar, CursorShape::Underline];

    /// Its id, as the setting stores it.
    pub fn id(self) -> &'static str {
        match self {
            CursorShape::Block => "block",
            CursorShape::Bar => "bar",
            CursorShape::Underline => "underline",
        }
    }

    fn title(self) -> String {
        match self {
            CursorShape::Block => t!("Block"),
            CursorShape::Bar => t!("Bar"),
            CursorShape::Underline => t!("Underline"),
        }
    }
}

/// mosh's predictive echo (`PredictMode`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PredictMode {
    #[default]
    Adaptive,
    Always,
    Never,
    Experimental,
}

impl PredictMode {
    /// All four, in the menu's order.
    pub const ALL: [PredictMode; 4] = [
        PredictMode::Adaptive,
        PredictMode::Always,
        PredictMode::Never,
        PredictMode::Experimental,
    ];

    /// Its id, as the setting stores it.
    pub fn id(self) -> &'static str {
        match self {
            PredictMode::Adaptive => "adaptive",
            PredictMode::Always => "always",
            PredictMode::Never => "never",
            PredictMode::Experimental => "experimental",
        }
    }

    /// What the page calls it, in the app's language (the SwiftUI page
    /// shows the English id capitalized in every language).
    pub fn title(self) -> String {
        match self {
            PredictMode::Adaptive => t!("Adaptive"),
            PredictMode::Always => t!("Always"),
            PredictMode::Never => t!("Never"),
            PredictMode::Experimental => t!("Experimental"),
        }
    }
}

/// How many of something there are, of how many there can be.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Count {
    pub count: i64,
    pub limit: i64,
}

/// What the page shows: every setting on it, and the names and counts its
/// rows show beside them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SettingsState {
    /// The accent (Settings › Accent).
    pub accent: Swatch,
    /// The home-screen icon (Settings › App Icon).
    pub app_icon: app_icon::Icon,
    /// The terminal's fonts, in the menu's order.
    pub fonts: Vec<Choice>,
    /// The id of the terminal's font.
    pub font: String,
    /// The terminal's font size, in points (8 to 18).
    pub font_size: f64,
    /// The terminal's color scheme (Display › Theme).
    pub theme: TerminalTheme,
    pub cursor_shape: CursorShape,
    /// `teal`, `green`, `white` or `accent` (another id: no swatch picked,
    /// the preview in teal, as on the SwiftUI page).
    pub cursor_color: String,
    pub cursor_blink: bool,
    pub trail_on_predict: bool,
    pub keyboard_on_open: bool,
    pub remote_clipboard_read: bool,
    /// Days uploads stay on the server: 1, 7, or 0 for ever.
    pub upload_retention_days: i64,
    pub mosh_by_default: bool,
    pub predict_mode: PredictMode,
    /// The UDP port range, first and last.
    pub udp_range: [i64; 2],
    /// Shortcuts on the terminal's bar, of how many fit.
    pub shortcuts: Count,
    /// SSH keys on the device.
    pub keys: i64,
    pub notifications: bool,
    pub live_activity: bool,
    /// iOS itself has the app's notifications off (B741): a "Don't Allow"
    /// on its question, or switched off in its Settings. The app's own
    /// switch can read on while nothing ever shows.
    #[serde(default)]
    pub notifications_blocked: bool,
    /// iOS itself has the app's Live Activities off (B741).
    #[serde(default)]
    pub live_activity_blocked: bool,
    pub alert_sound: bool,
    pub lock_screen_detail: bool,
    /// A session is live, so Set up this host can inspect it.
    pub live_session: bool,
    pub voice_input: bool,
    pub voice_engine: Engine,
    /// The Whisper model a dictation would use (`None`: there is none).
    pub voice_model: Option<String>,
    /// What the dictation language is called.
    pub voice_language: String,
    /// The build's version line (`SettingsScreen.versionLine`).
    pub version: String,
    /// The APP filing number, on the mainland storefront only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filing: Option<String>,
    /// The theme the app wears now: picked on the Accent page while
    /// Settings shows, it reaches this page's own colors too (the screen's
    /// theme is otherwise the one it ran with).
    pub app_theme: AppTheme,
    /// The appearance chosen (Settings › Appearance; the system's when the
    /// host sends none).
    #[serde(default)]
    pub appearance: AppearanceChoice,
}

/// Where the list scrolls to (`{"op": "reveal"}`): a group's first row just
/// under the bar, as SwiftUI's `ScrollViewReader.scrollTo(_:anchor: .top)`
/// puts it, or the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Anchor {
    /// Cursor's Shape row.
    Cursor,
    /// Behavior's first switch.
    Behavior,
    /// Mosh by default.
    Mosh,
    /// Notifications' first switch.
    Notifications,
    /// The version line's row at the bottom of the viewport, as SwiftUI's
    /// `scrollTo(_:anchor: .bottom)` puts it.
    Bottom,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "settings", …}`: what the page shows now.
    Settings(Box<SettingsState>),
    /// `{"op": "reveal", "anchor": …}`: scroll the list to `anchor`.
    Reveal(Anchor),
    /// `{"op": "hosts", "hosts": […]}`: the saved connections, for Set up a
    /// host's list ([`Part::Hosts`]).
    Hosts(Vec<HostEntry>),
    /// `{"op": "pet", …}`: the companion (Appearance's group and its card,
    /// [`Part::Companion`]).
    Pet(Box<moshpit_pet::PetView>),
    /// `{"op": "clock", "hour": …}`: the local hour (0–23), which the pet's
    /// card answers a tap by as Home does (B607: the small hours' lines).
    Clock(u8),
}

/// `{"op": "clock"}`'s fields.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Clock {
    hour: u8,
}

/// A saved connection on Set up a host's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostEntry {
    /// The connection's id, which a pick tells (`{"ev": "setup", "id": …}`).
    pub id: String,
    /// Its name (its `user@host` when it has none).
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: i64,
    /// A session of it is connected: its host can be set up now.
    pub live: bool,
}

/// `{"op": "hosts"}`'s fields.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hosts {
    hosts: Vec<HostEntry>,
}

/// `{"op": "reveal"}`'s fields.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reveal {
    anchor: Anchor,
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("settings", json)?;
        let fields = serde_json::Value::Object(fields);
        match op.as_str() {
            "settings" => serde_json::from_value(fields)
                .map(|state| Message::Settings(Box::new(state)))
                .map_err(|e| format!("settings: settings: {e}")),
            "reveal" => serde_json::from_value::<Reveal>(fields)
                .map(|reveal| Message::Reveal(reveal.anchor))
                .map_err(|e| format!("settings: reveal: {e}")),
            "hosts" => serde_json::from_value::<Hosts>(fields)
                .map(|hosts| Message::Hosts(hosts.hosts))
                .map_err(|e| format!("settings: hosts: {e}")),
            "pet" => crate::home::pet::parse(fields).map(|(view, _)| Message::Pet(Box::new(view))),
            "clock" => serde_json::from_value::<Clock>(fields)
                .map(|clock| Message::Clock(clock.hour))
                .map_err(|e| format!("settings: clock: {e}")),
            other => Err(format!("settings: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Settings(state) => {
                let mut value = serde_json::to_value(state).expect("a state serializes");
                value["op"] = json!("settings");
                value.to_string()
            }
            Message::Reveal(anchor) => json!({"op": "reveal", "anchor": anchor}).to_string(),
            Message::Hosts(hosts) => json!({"op": "hosts", "hosts": hosts}).to_string(),
            Message::Pet(view) => {
                let mut value = serde_json::to_value(view).expect("a pet serializes");
                value["op"] = json!("pet");
                value.to_string()
            }
            Message::Clock(hour) => json!({"op": "clock", "hour": hour}).to_string(),
        }
    }
}

/// What a row opens (`{"ev": "open", "page": …}`): a page the host pushes, a
/// sheet it presents, or a link it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Accent,
    AppIcon,
    Theme,
    Shortcuts,
    SshKeys,
    UdpRange,
    NotificationInfo,
    HostSetup,
    VoiceEngine,
    WhisperModel,
    VoiceLanguage,
    RecentLog,
    Feedback,
    GithubIssue,
    Filing,
    /// iOS's own settings for the app, where its notifications and Live
    /// Activities are allowed (B741).
    IosSettings,
}

impl Page {
    /// Every one.
    pub const ALL: [Page; 16] = [
        Page::Accent,
        Page::AppIcon,
        Page::Theme,
        Page::Shortcuts,
        Page::SshKeys,
        Page::UdpRange,
        Page::NotificationInfo,
        Page::HostSetup,
        Page::VoiceEngine,
        Page::WhisperModel,
        Page::VoiceLanguage,
        Page::RecentLog,
        Page::Feedback,
        Page::GithubIssue,
        Page::Filing,
        Page::IosSettings,
    ];

    /// What the host calls it.
    pub fn id(self) -> &'static str {
        match self {
            Page::Accent => "accent",
            Page::AppIcon => "app-icon",
            Page::Theme => "theme",
            Page::Shortcuts => "shortcuts",
            Page::SshKeys => "ssh-keys",
            Page::UdpRange => "udp-range",
            Page::NotificationInfo => "notification-info",
            Page::HostSetup => "host-setup",
            Page::VoiceEngine => "voice-engine",
            Page::WhisperModel => "whisper-model",
            Page::VoiceLanguage => "voice-language",
            Page::RecentLog => "recent-log",
            Page::Feedback => "feedback",
            Page::GithubIssue => "github-issue",
            Page::Filing => "filing",
            Page::IosSettings => "ios-settings",
        }
    }

    /// The page called `id`.
    pub fn named(id: &str) -> Option<Page> {
        Self::ALL.into_iter().find(|page| page.id() == id)
    }

    /// The event that opens it.
    pub fn event(self) -> serde_json::Value {
        json!({"ev": "open", "page": self.id()})
    }
}

/// A setting the user changed (`{"ev": "set", "key": …, "value": …}`): the
/// key is [`SettingsState`]'s field.
#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    Font(String),
    FontSize(f64),
    CursorShape(CursorShape),
    CursorColor(String),
    CursorBlink(bool),
    TrailOnPredict(bool),
    KeyboardOnOpen(bool),
    RemoteClipboardRead(bool),
    UploadRetentionDays(i64),
    MoshByDefault(bool),
    PredictMode(PredictMode),
    Notifications(bool),
    LiveActivity(bool),
    AlertSound(bool),
    LockScreenDetail(bool),
    VoiceInput(bool),
    Appearance(AppearanceChoice),
    /// The UDP ports mosh binds in, from and to ([`udp_ports`]).
    UdpRange([i64; 2]),
    /// The companion's style (`minimal`, `pixel`), and whether it shows.
    Companion(moshpit_pet::Style, bool),
    /// The pet's name (its card).
    PetName(String),
    /// Whether it talks on Home (「让它说话」).
    PetTalk(bool),
}

impl Setting {
    /// The event that tells the host.
    pub fn event(&self) -> serde_json::Value {
        let (key, value) = match self {
            Setting::Font(id) => ("font", json!(id)),
            Setting::FontSize(size) => ("fontSize", json!(size)),
            Setting::CursorShape(shape) => ("cursorShape", json!(shape.id())),
            Setting::CursorColor(id) => ("cursorColor", json!(id)),
            Setting::CursorBlink(on) => ("cursorBlink", json!(on)),
            Setting::TrailOnPredict(on) => ("trailOnPredict", json!(on)),
            Setting::KeyboardOnOpen(on) => ("keyboardOnOpen", json!(on)),
            Setting::RemoteClipboardRead(on) => ("remoteClipboardRead", json!(on)),
            Setting::UploadRetentionDays(days) => ("uploadRetentionDays", json!(days)),
            Setting::MoshByDefault(on) => ("moshByDefault", json!(on)),
            Setting::PredictMode(mode) => ("predictMode", json!(mode.id())),
            Setting::Notifications(on) => ("notifications", json!(on)),
            Setting::LiveActivity(on) => ("liveActivity", json!(on)),
            Setting::AlertSound(on) => ("alertSound", json!(on)),
            Setting::LockScreenDetail(on) => ("lockScreenDetail", json!(on)),
            Setting::VoiceInput(on) => ("voiceInput", json!(on)),
            Setting::Appearance(choice) => ("appearance", json!(choice.id())),
            Setting::UdpRange(range) => ("udpRange", json!(range)),
            Setting::Companion(style, shown) => {
                ("companion", json!({"style": style.id(), "shown": shown}))
            }
            Setting::PetName(name) => ("petName", json!(name)),
            Setting::PetTalk(on) => ("petTalk", json!(on)),
        };
        json!({"ev": "set", "key": key, "value": value})
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// The upload retention's choices, in the segmented control's order.
const RETENTION: [i64; 3] = [1, 7, 0];

/// The terminal font's sizes, in points: the range a pinch on the terminal
/// keeps (B515), so the steps go on from a pinched size.
const FONT_MIN: f64 = 8.0;
const FONT_MAX: f64 = 32.0;

/// A page of Settings: the hub, a page it opens, or a page the host pushes
/// on top: for one of the hub's rows, or one of those pages opens on the
/// stack (`{"page": …}` in the screen's params;
/// `{"ev": "open", "page": …}` asks the host for one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    Hub,
    Appearance,
    Terminal,
    Connections,
    Notifications,
    Voice,
    /// Set up a host: the saved connections, to pick the one to set up
    /// (pushed for the hub's Set up a host when there is not exactly one).
    Hosts,
    /// Connections › UDP port range (UI-DESIGN §6.4: an edit page, two
    /// number fields).
    UdpRange,
    /// Notifications › How notifications work.
    NotificationInfo,
    /// Appearance › the companion's card (moshpit-design PET-DESIGN §15.6).
    Companion,
}

impl Part {
    /// The pages the hub opens, in its order.
    pub const PAGES: [Part; 5] = [
        Part::Appearance,
        Part::Terminal,
        Part::Connections,
        Part::Notifications,
        Part::Voice,
    ];

    /// The pages the host pushes beside [`Self::PAGES`], the screen again on
    /// that page: for a row of the hub's (Set up a host's list) and for the
    /// pages those pages open on the stack (R18).
    pub const PUSHED: [Part; 4] = [
        Part::Hosts,
        Part::UdpRange,
        Part::NotificationInfo,
        Part::Companion,
    ];

    /// What the host calls it (the hub: nothing).
    pub fn id(self) -> &'static str {
        match self {
            Part::Hub => "",
            Part::Appearance => "appearance",
            Part::Terminal => "terminal",
            Part::Connections => "connections",
            Part::Notifications => "notifications",
            Part::Voice => "voice",
            Part::Hosts => "hosts",
            Part::UdpRange => "udp-range",
            Part::NotificationInfo => "notification-info",
            Part::Companion => "companion",
        }
    }

    /// The page of the hub's called `id` ([`Self::PAGES`]); the hub for
    /// none, or for a name no page of the hub's has. What a host routes the
    /// hub's rows by.
    pub fn named(id: Option<&str>) -> Part {
        Self::PAGES
            .into_iter()
            .find(|part| Some(part.id()) == id)
            .unwrap_or(Part::Hub)
    }

    /// The part a screen's params name: a page of the hub's or one the host
    /// pushes ([`Self::PUSHED`]); the hub for none, or for a name no part
    /// has.
    pub fn of(id: Option<&str>) -> Part {
        Self::PAGES
            .into_iter()
            .chain(Self::PUSHED)
            .find(|part| Some(part.id()) == id)
            .unwrap_or(Part::Hub)
    }

    /// Its title, in the app's language.
    pub fn title(self) -> String {
        match self {
            Part::Hub => t!("Settings"),
            Part::Appearance => t!("Appearance"),
            Part::Terminal => t!("Terminal"),
            Part::Connections => t!("Connections"),
            Part::Notifications => t!("Notifications"),
            Part::Voice => t!("Voice input"),
            Part::Hosts => t!("Set up a host"),
            Part::UdpRange => t!("UDP Port Range"),
            Part::NotificationInfo => t!("How notifications work"),
            Part::Companion => t!("Companion"),
        }
    }

    /// The event that opens it.
    pub fn event(self) -> serde_json::Value {
        json!({"ev": "open", "page": self.id()})
    }
}

/// The version a version line names (`Moshpit 1.1.0 (1) · 4db8aeff` →
/// `1.1.0 (1)`); the line whole when it is not one.
pub fn short_version(line: &str) -> String {
    let line = line.strip_prefix("Moshpit ").unwrap_or(line);
    line.split(" · ").next().unwrap_or(line).to_owned()
}

/// What the page reads from the host's state.
#[derive(Clone, Copy)]
struct State(Signal<Option<SettingsState>>);

impl State {
    /// `f` of the state (the default until the host's first post).
    fn read<T: Default>(self, f: impl Fn(&SettingsState) -> T) -> T {
        self.0.with(|s| s.as_ref().map(&f).unwrap_or_default())
    }
}

/// What tells the host.
type Tell = Rc<dyn Fn(serde_json::Value)>;

/// Settings: the hub, or the page the params name.
#[component]
pub fn SettingsScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let params = channel.as_ref().map(|c| c.params()).unwrap_or_default();
    let bar = BarMetrics::or_standard(params.bar);
    let ran_with = params.theme;
    let part = Part::of(params.page.as_deref());
    let state = State(signal(None::<SettingsState>));
    let hosts = signal(Vec::<HostEntry>::new());
    let pet = signal(None::<moshpit_pet::PetView>);
    // Noon until the host says the hour: a host that never does (Android
    // today) gets every line but the small hours'.
    let hour = signal(12_u8);
    let anchors = Anchors {
        cursor: element_ref(),
        behavior: element_ref(),
        mosh: element_ref(),
        notifications: element_ref(),
        bottom: element_ref(),
        bar,
    };
    channel::hear(
        channel.as_ref(),
        Screen::Settings,
        Message::parse,
        move |message| match message {
            Message::Settings(next) => {
                // A theme picked since the screen ran (Settings › Accent).
                let worn = state.0.with_untracked(|s| s.as_ref().map(|s| s.app_theme));
                if worn.unwrap_or(ran_with) != next.app_theme {
                    set_tokens(theme::tokens(&next.app_theme));
                }
                state.0.set(Some(*next));
            }
            Message::Reveal(anchor) => anchors.reveal(anchor),
            Message::Hosts(next) => hosts.set(next),
            Message::Pet(next) => pet.set(Some(*next)),
            Message::Clock(next) => hour.set(next),
        },
    );
    let tell = tell_host(channel);
    let ready = computed(move || state.0.with(Option::is_some));
    let root = view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("settings");
    match part {
        Part::Hub => hub(root, state, tell, anchors, ready),
        part => {
            let back = params.back.clone().unwrap_or_else(|| t!("Settings"));
            subpage(root, part, back, state, hosts, pet, hour, tell, anchors)
        }
    }
}

/// The rows [`Anchor`]s name, marked so the list can be scrolled to them.
#[derive(Clone, Copy)]
struct Anchors {
    cursor: ElementRef,
    behavior: ElementRef,
    mosh: ElementRef,
    notifications: ElementRef,
    /// The list's end.
    bottom: ElementRef,
    bar: BarMetrics,
}

impl Anchors {
    fn get(self, anchor: Anchor) -> Option<ElementRef> {
        match anchor {
            Anchor::Cursor => Some(self.cursor),
            Anchor::Behavior => Some(self.behavior),
            Anchor::Mosh => Some(self.mosh),
            Anchor::Notifications => Some(self.notifications),
            Anchor::Bottom => None,
        }
    }

    /// `group`, marked as `anchor`'s: a point the bar's height above its
    /// top, which scrolled to the list's top puts the group just under the
    /// bar.
    fn mark(self, anchor: Anchor, group: AnyElement) -> AnyElement {
        let Some(marker) = self.get(anchor) else {
            return group;
        };
        let bar = self.bar;
        view()
            .relative()
            .w_full()
            .child(
                view()
                    .absolute()
                    .left(px(0.0))
                    .right(px(0.0))
                    .top(move || px(-nav_bar::bar_space(bar).0))
                    .h(px(1.0))
                    .pointer_events(PointerEvents::None)
                    .element_ref(&marker),
            )
            .child(group)
            .into_element()
    }

    /// Scrolls the list to `anchor`, at once (a page without it: nowhere).
    fn reveal(self, anchor: Anchor) {
        let (marker, alignment) = match self.get(anchor) {
            Some(marker) => (marker, ScrollAlignment::Start),
            None => (self.bottom, ScrollAlignment::End),
        };
        marker.reveal(RevealOptions::default().vertical(alignment).animated(false));
    }
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

/// What opens `page`.
fn open(tell: &Tell, page: Page) -> impl Fn() + 'static {
    let tell = tell.clone();
    move || tell(page.event())
}

/// What opens `part`.
fn go(tell: &Tell, part: Part) -> impl Fn() + 'static {
    let tell = tell.clone();
    move || tell(part.event())
}

/// What tells the host the user set `setting(value)`.
fn set<T: 'static>(tell: &Tell, setting: fn(T) -> Setting) -> impl Fn(T) + 'static {
    let tell = tell.clone();
    move |value| tell(setting(value).event())
}

// --- The list ------------------------------------------------------------------

/// The space over the hub's first group, under its large title (a page's
/// first group sits `page::TOP` under its bar).
const HUB_TOP: f32 = 8.0;

/// The hub's or a page's groups, as one item of the page's list
/// (`settings-column`): `page::BETWEEN` apart, `lead()` over them, the
/// list's end marked `bottom` (what [`Anchor::Bottom`] reveals: the marker
/// reaches over the page's own end, to the list's last point).
fn column(
    lead: impl Fn() -> f32 + 'static,
    groups: Vec<AnyElement>,
    bottom: ElementRef,
) -> AnyElement {
    view()
        .w_full()
        .test_id("settings-column")
        .child(view().h(move || px(lead())))
        .child(view().w_full().gap(px(page::BETWEEN)).children(groups))
        .child(
            view().relative().w_full().child(
                view()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .h(move || px(page::BOTTOM + safe_area().bottom.0))
                    .pointer_events(PointerEvents::None)
                    .element_ref(&bottom),
            ),
        )
        .into_element()
}

// --- The hub -------------------------------------------------------------------

/// The hub: Home's large title, Done at its trailing edge, and the rows
/// that open every page, on one screen.
fn hub(root: View, state: State, tell: Tell, anchors: Anchors, ready: Memo<bool>) -> AnyElement {
    let bar = anchors.bar;
    let offset = animated_value(0.0);
    let pull = animated_value(0.0);
    let done: Rc<dyn Fn()> = {
        let tell = tell.clone();
        Rc::new(move || tell(json!({"ev": "done"})))
    };
    let failed = tell.clone();
    let body = move || {
        let scroller = element_ref();
        // The page's list under the large title: the title's row (what it
        // adds to the bar's space), less what the list puts under a bar, is
        // the column's lead. The large title follows the list's offset and
        // pull (no edge of a bar's own to set off): this `scroll_value`
        // replaces the one the page's `ScrollEdge` bound (a scroll view
        // follows one value).
        let lead = move || large::large_row() + HUB_TOP - page::TOP;
        page::page(
            bar,
            nav_bar::ScrollEdge::new(),
            "settings-list",
            [column(lead, hub_groups(state, &tell), anchors.bottom)],
        )
        .scroll_value(&offset)
        .pull_value(&pull)
        .element_ref(&scroller)
        // §4.18 (c), as Home: let go part way through the large title's
        // collapse, the list settles at the nearer end.
        .on_scroll_end(move |at: &Point<Points>| {
            if let Some(y) = large::LargeTitleMotion::now().settle(at.y.0) {
                scroller.scroll_to(point(px(0.0), px(y)), !reduce_motion());
            }
        })
        .into_element()
    };
    let done_button: large::Build =
        Rc::new(move || nav_bar::close_button(t!("Done"), true, "settings-done", done.clone()));
    root.child(crate::boundary::ready_body(
        move || ready.get(),
        body,
        failed,
    ))
    .child(large::large_bar(
        bar,
        t!("Settings"),
        None,
        vec![done_button],
        offset,
        pull,
    ))
    .into_element()
}

/// The hub's groups (§6.4): each row a tile, a name, the value that
/// matters most and a chevron.
fn hub_groups(state: State, tell: &Tell) -> Vec<AnyElement> {
    let on_off = |on: bool| -> SharedString { if on { t!("On") } else { t!("Off") }.into() };
    // Notifications and Voice input: one group, each where it is wired
    // (B539).
    let shown = rows(Os::current());
    let alerts_and_voice = (shown.notifications || shown.voice).then(|| {
        let mut group = section();
        if shown.notifications {
            group = group.row(
                row(t!("Notifications"))
                    .leading_tile("bell")
                    .value_with(move || {
                        // On in the app, off in iOS: not "On" (B741).
                        match state.read(|s| (s.notifications, s.notifications_blocked)) {
                            (true, true) => t!("Off in iOS").into(),
                            (on, _) => on_off(on),
                        }
                    })
                    .chevron()
                    .on_press(go(tell, Part::Notifications))
                    .test_id("settings-open-notifications"),
            );
        }
        if shown.voice {
            group = group.row(
                row(t!("Voice input"))
                    .leading_tile("mic")
                    .value_with(move || {
                        if state.read(|s| s.voice_input) {
                            state.read(|s| s.voice_engine).name().into()
                        } else {
                            on_off(false)
                        }
                    })
                    .chevron()
                    .on_press(go(tell, Part::Voice))
                    .test_id("settings-open-voice"),
            );
        }
        group.into_element()
    });
    let mut groups = vec![
        section()
            .row(
                row(t!("Appearance"))
                    .leading_tile("paintbrush")
                    .value_with(move || state.read(|s| s.accent.name.clone()).into())
                    .chevron()
                    .on_press(go(tell, Part::Appearance))
                    .test_id("settings-open-appearance"),
            )
            .row(
                row(t!("Terminal"))
                    .leading_tile("terminal")
                    .value_with(move || state.read(|s| s.theme.name.clone()).into())
                    .chevron()
                    .on_press(go(tell, Part::Terminal))
                    .test_id("settings-open-terminal"),
            )
            .into_element(),
        section()
            .row(
                row(t!("Connections"))
                    .leading_tile("network")
                    .chevron()
                    .on_press(go(tell, Part::Connections))
                    .test_id("settings-open-connections"),
            )
            .row(
                row(t!("Shortcuts"))
                    .leading_tile("keyboard")
                    .value_with(move || {
                        let Count { count, limit } = state.read(|s| s.shortcuts);
                        format!("{count}/{limit}").into()
                    })
                    .chevron()
                    .on_press(open(tell, Page::Shortcuts))
                    .test_id("settings-shortcuts"),
            )
            .row(
                row(t!("SSH keys"))
                    .leading_tile("key")
                    .value_with(move || state.read(|s| s.keys).to_string().into())
                    .chevron()
                    .on_press(open(tell, Page::SshKeys))
                    .test_id("settings-ssh-keys"),
            )
            .into_element(),
    ];
    groups.extend(alerts_and_voice);
    groups.extend([
        section()
            .row(
                row(t!("Set up a host"))
                    .leading_tile("wrench.and.screwdriver")
                    .chevron()
                    .on_press(open(tell, Page::HostSetup))
                    .test_id("settings-host-setup"),
            )
            .into_element(),
        section()
            .row(
                row(t!("Send feedback"))
                    .leading_tile("envelope")
                    .chevron()
                    .on_press(open(tell, Page::Feedback))
                    .test_id("settings-feedback"),
            )
            .row(
                row(t!("Recent log"))
                    .leading_tile("doc.text")
                    .chevron()
                    .on_press(open(tell, Page::RecentLog))
                    .test_id("settings-recent-log"),
            )
            .into_element(),
        about(state, tell),
    ]);
    groups
}

/// About: the version row (long-press: Copy, the host's menu), the tagline
/// under it and, on the mainland storefront, the APP filing.
fn about(state: State, tell: &Tell) -> AnyElement {
    let filing = Rc::new(open(tell, Page::Filing));
    let has_filing = computed(move || state.read(|s| s.filing.is_some()));
    let version = row(t!("Version"))
        .leading_tile("info.circle")
        .value_with(move || short_version(&state.read(|s| s.version.clone())).into())
        .test_id("settings-version");
    view()
        .w_full()
        .gap(px(4.0))
        .child(
            section()
                .row(version)
                .footer(t!("The pit never closes."))
                .test_id("settings-about"),
        )
        .child(when(
            move || has_filing.get(),
            move || {
                let filing = filing.clone();
                view().px(px(crate::ui::row::PADDING_X)).child(
                    TextToken::MonoCaption
                        .apply(text(move || {
                            t!(
                                "APP filing %@",
                                arg1 = state.read(|s| s.filing.clone().unwrap_or_default())
                            )
                        }))
                        .text_color(move || token("muted-foreground"))
                        .role(AccessibilityRole::Link)
                        .test_id("settings-icp")
                        .on_press(move || filing()),
                )
            },
        ))
        .into_element()
}

// --- A page --------------------------------------------------------------------

/// A page the hub opens: the bar (back to the hub, the page's title) over
/// its groups.
#[allow(clippy::too_many_arguments)]
fn subpage(
    root: View,
    part: Part,
    back: String,
    state: State,
    hosts: Signal<Vec<HostEntry>>,
    pet: Signal<Option<moshpit_pet::PetView>>,
    hour: Signal<u8>,
    tell: Tell,
    anchors: Anchors,
) -> AnyElement {
    // The body waits for the host's first post.
    let ready = computed(move || state.0.with(Option::is_some));
    let bar = anchors.bar;
    let scrolled = nav_bar::ScrollEdge::new();
    // The UDP range's page edits a draft, which its bar's Save keeps.
    let ports = (part == Part::UdpRange).then(|| Ports::new(state, tell.clone()));
    let body = {
        let tell = tell.clone();
        let ports = ports.clone();
        move || {
            let groups = match part {
                Part::Appearance => appearance(state, &tell, pet),
                Part::Companion => companion_card(pet, hour, &tell),
                Part::Terminal => terminal(state, &tell, anchors),
                Part::Connections => connections(state, &tell, anchors),
                Part::Notifications => notifications(state, &tell, anchors),
                Part::Hosts => host_list(hosts, &tell),
                Part::UdpRange => ports.clone().map(udp_range).unwrap_or_default(),
                Part::NotificationInfo => notification_info(),
                Part::Voice | Part::Hub => voice(state, &tell),
            };
            page::page(
                bar,
                scrolled,
                "settings-list",
                [column(|| 0.0, groups, anchors.bottom)],
            )
            .into_element()
        }
    };
    root.child(crate::boundary::ready_body(
        move || ready.get(),
        body,
        tell.clone(),
    ))
    .child(nav_bar::nav_bar(nav_bar::NavBar {
        metrics: bar,
        back_title: back,
        title: nav_bar::fixed(part.title()),
        leading: None,
        trailing: ports.map(|ports| ports.save_button()).into_iter().collect(),
        on_back: Box::new(move || tell(json!({"ev": "back"}))),
        scrolled,
    }))
    .into_element()
}

/// Set up a host's list: every saved connection, its name over where it
/// reaches, connected or not; a pick tells the host (`{"ev": "setup",
/// "id": …}`), which opens the host's setup or offers to connect first.
fn host_list(hosts: Signal<Vec<HostEntry>>, tell: &Tell) -> Vec<AnyElement> {
    let tell = tell.clone();
    vec![
        dynamic(move || {
            let list = hosts.get();
            let tell = tell.clone();
            untracked(move || {
                let mut group = section().test_id("settings-hosts-group");
                if list.is_empty() {
                    group = group.item(
                        empty(t!("No connections yet"))
                            .variant(EmptyVariant::Inline)
                            .icon("server.rack")
                            .test_id("settings-hosts-none"),
                        Inset::Content,
                    );
                }
                for host in list {
                    let (id, tell) = (host.id.clone(), tell.clone());
                    group = group.row(
                        row(host.name.clone())
                            .subtitle(crate::host_setup::address(&host.user, &host.host, host.port))
                            .mono_subtitle()
                            .value(if host.live {
                                t!("Connected")
                            } else {
                                t!("Not connected")
                            })
                            .value_dot(if host.live {
                                Status::Live
                            } else {
                                Status::Offline
                            })
                            .chevron()
                            .on_press(move || tell(json!({"ev": "setup", "id": id})))
                            .test_id(&format!("settings-host-{}", host.id)),
                    );
                }
                group
                    .footer(t!(
                        "Host setup is automatic. Connecting to a host installs and repairs everything it needs — scripts, hooks registration, push pairing — and asks before its first install. Connect to a host to inspect or remove its setup here."
                    ))
                    .into_element()
            })
        })
        .into_element(),
    ]
}

// --- Connections › UDP port range -----------------------------------------------

/// Which of the range's two ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    From,
    To,
}

/// What is wrong with a UDP port range ([`udp_ports`]), and on which of its
/// fields it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortsError {
    /// Not a whole number.
    NotNumeric(Side),
    /// Outside 1–65535.
    OutOfBounds(Side),
    /// From is past To (shown under To).
    Inverted,
}

impl PortsError {
    /// The field it shows under.
    pub fn side(self) -> Side {
        match self {
            PortsError::NotNumeric(side) | PortsError::OutOfBounds(side) => side,
            PortsError::Inverted => Side::To,
        }
    }

    /// What it says, in the app's language.
    pub fn words(self) -> String {
        match self {
            PortsError::NotNumeric(_) => t!("Enter numeric ports."),
            PortsError::OutOfBounds(_) => t!("Ports must be 1–65535."),
            PortsError::Inverted => t!("From must be ≤ To."),
        }
    }
}

/// The UDP port range `from`–`to` as typed: both whole numbers (spaces
/// around them ignored), each a port (1–65535), from no later than to. The
/// rule the SwiftUI editor keeps (`UDPPortRange.validate`); the first
/// field wrong is the one named.
pub fn udp_ports(from: &str, to: &str) -> Result<[i64; 2], PortsError> {
    let number = |text: &str, side| {
        text.trim()
            .parse::<i64>()
            .map_err(|_| PortsError::NotNumeric(side))
    };
    let (from, to) = (number(from, Side::From)?, number(to, Side::To)?);
    for (port, side) in [(from, Side::From), (to, Side::To)] {
        if !(1..=65535).contains(&port) {
            return Err(PortsError::OutOfBounds(side));
        }
    }
    if from > to {
        return Err(PortsError::Inverted);
    }
    Ok([from, to])
}

/// The UDP range's page: the draft its two fields edit, and its Save.
#[derive(Clone)]
struct Ports {
    state: State,
    tell: Tell,
    /// What each field holds once edited; the stored port until then.
    from: Signal<Option<String>>,
    to: Signal<Option<String>>,
    /// What Save found wrong, until the field it names is edited.
    error: Signal<Option<PortsError>>,
}

impl Ports {
    fn new(state: State, tell: Tell) -> Ports {
        Ports {
            state,
            tell,
            from: signal(None),
            to: signal(None),
            error: signal(None),
        }
    }

    /// The stored port on `side`.
    fn stored(&self, side: Side) -> i64 {
        let at = side as usize;
        self.state.read(|s| s.udp_range[at])
    }

    /// Keeps the range the fields hold and goes back, or says what is wrong
    /// under the field it names. A range left as it was tells nothing.
    fn save(&self) {
        let typed = |draft: Signal<Option<String>>, side| {
            draft
                .get_untracked()
                .unwrap_or_else(|| self.stored(side).to_string())
        };
        match udp_ports(&typed(self.from, Side::From), &typed(self.to, Side::To)) {
            Ok(range) => {
                if range != [self.stored(Side::From), self.stored(Side::To)] {
                    (self.tell)(Setting::UdpRange(range).event());
                }
                (self.tell)(json!({"ev": "back"}));
            }
            Err(error) => {
                haptic(Haptic::Notification(HapticNotification::Error));
                self.error.set(Some(error));
            }
        }
    }

    /// The bar's Save.
    fn save_button(self) -> nav_bar::BarButton {
        nav_bar::BarButton {
            label: t!("Save"),
            content: nav_bar::BarContent::Confirm,
            enabled: computed(|| true),
            emphasized: true,
            test_id: "settings-udp-save",
            on_press: Box::new(move || self.save()),
            menu: None,
        }
    }

    /// The field for `side`: the stored port to start, the draft as typed,
    /// what Save found wrong with it under it.
    fn field(
        &self,
        side: Side,
        label: String,
        placeholder: &'static str,
        next: Option<ElementRef>,
        me: ElementRef,
    ) -> AnyElement {
        let (draft, error) = (
            match side {
                Side::From => self.from,
                Side::To => self.to,
            },
            self.error,
        );
        let stored = self.stored(side);
        let mut input = number_input()
            .default_value(move || stored.to_string())
            .placeholder(placeholder)
            .max_length(5)
            .element_ref(&me)
            .on_change(move |text: String| {
                draft.set(Some(text));
                if error.get_untracked().is_some_and(|e| e.side() == side) {
                    error.set(None);
                }
            });
        input = match next {
            Some(next) => input.next(&next),
            None => {
                let ports = self.clone();
                input.on_submit(move || ports.save())
            }
        };
        let id = match side {
            Side::From => "settings-udp-from",
            Side::To => "settings-udp-to",
        };
        field(label, input)
            .error(move || {
                error
                    .get()
                    .filter(|e| e.side() == side)
                    .map(|e| e.words().into())
            })
            .test_id(id)
            .into_element()
    }
}

/// Connections › UDP port range: From and To, in a group whose footer says
/// what the range is for.
fn udp_range(ports: Ports) -> Vec<AnyElement> {
    let (from, to) = (element_ref(), element_ref());
    vec![
        section()
            .header(t!("UDP PORT RANGE"))
            .item(
                ports.field(Side::From, t!("From"), "60000", Some(to), from),
                Inset::Content,
            )
            .item(
                ports.field(Side::To, t!("To"), "61000", None, to),
                Inset::Content,
            )
            .footer(t!(
                "mosh binds one UDP port in this range per session. Open it outbound from your iPhone and inbound on the server (default 60000–61000)."
            ))
            .test_id("settings-udp-group")
            .into_element(),
    ]
}

// --- Notifications › How notifications work ----------------------------------------

/// How notifications work: what each part of them does, a symbol in its
/// colour before a title over what it says.
fn notification_info() -> Vec<AnyElement> {
    let mut parts = vec![(
        "bell.fill",
        Tone::Warning,
        t!("Agents stamp their state"),
        t!(
            "Coding agents (Claude Code, Codex, …) report working / needs-you / done through hooks Moshpit installs on your host — precise states, not guesses. The terminal bell still works as a fallback for everything else."
        ),
    )];
    if rows(Os::current()).live_activity {
        parts.push((
            "bubbles.and.sparkles.fill",
            Tone::Primary,
            t!("Live Activity"),
            t!(
                "While a session is attached, the Dynamic Island shows whether the agent is working, idle, or waiting on you. Tapping it deep-links straight back to that pane."
            ),
        ));
    }
    parts.push((
        "paperplane.fill",
        Tone::Info,
        t!("Push, sealed end-to-end"),
        t!(
            "When the app isn’t running, your host sends the alert through Moshpit’s push relay. It is encrypted on your host with a key only this device holds — the relay and Apple carry ciphertext and can read none of it. Delivered even from a locked phone."
        ),
    ));
    parts.push((
        "moon.zzz.fill",
        Tone::Muted,
        t!("Quiet by design"),
        t!(
            "A question must stand for 30 seconds before any phone hears about it — answered at your desk means never announced. All waiting agents share one summary card; only the first rings. A finished turn shows the prompt it answered and only chimes if it ran three minutes or more. Parked agents stay silent."
        ),
    ));
    let mut group = section().test_id("settings-notification-info-group");
    for (n, (symbol, tone, title, words)) in parts.into_iter().enumerate() {
        group = group.item(explained(symbol, tone, title, words, n), Inset::Lead);
    }
    vec![group.into_element()]
}

/// One part of the explainer: its symbol in the rows' leading lane, its
/// title over its words, which wrap.
fn explained(
    symbol: &'static str,
    tone: Tone,
    title: String,
    words: String,
    n: usize,
) -> AnyElement {
    view()
        .w_full()
        .flex_row()
        .items_start()
        .gap(px(GAP))
        .px(px(PADDING_X))
        .py(px(12.0))
        .test_id(&format!("settings-notification-info-{n}"))
        .child(
            view()
                .w(px(LEADING))
                .flex_shrink_0()
                .items_center()
                .accessibility_elements_hidden(true)
                .child(icon(symbol).size(IconSize::Md).tone(tone)),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(4.0))
                .child(
                    TextToken::Headline
                        .apply(text(title))
                        .text_color(move || token("foreground"))
                        .role(AccessibilityRole::Header),
                )
                .child(
                    TextToken::Subhead
                        .apply(text(words))
                        .text_color(move || token("muted-foreground")),
                ),
        )
        .into_element()
}

/// A switch row: `title` (and a one-line `subtitle`), the switch on while
/// `read` says so, telling the host `setting`; the switch is `id`.
fn toggle(
    state: State,
    tell: &Tell,
    title: String,
    subtitle: Option<String>,
    read: fn(&SettingsState) -> bool,
    setting: fn(bool) -> Setting,
    id: &'static str,
) -> Row {
    let control = switch(move || state.read(read), set(tell, setting))
        .label(title.clone())
        .test_id(id);
    let row = row(title).trailing(control);
    match subtitle {
        // What the switch does, read in full in any language (B747): one
        // line cut German, French, and English's longer ones.
        Some(subtitle) => row.subtitle(subtitle).wrap_subtitle(),
        None => row,
    }
}

/// A control under its name, on a group's card: a segmented control, the
/// cursor's swatches. Screen readers hear the control's own label.
fn labelled(title: String, control: impl IntoElement) -> AnyElement {
    view()
        .w_full()
        .px(px(crate::ui::row::PADDING_X))
        .py(px(10.0))
        .gap(px(8.0))
        .child(
            TextToken::Body
                .apply(text(title))
                .number_of_lines(1u32)
                .text_color(move || token("foreground"))
                .accessibility_elements_hidden(true),
        )
        .child(control)
        .into_element()
}

/// The rows Settings shows only on some platforms (B258), and the ones a
/// host doesn't read yet (B539): Android's terminal takes the theme and the
/// font size, its connections the UDP range and mosh for new ones; its
/// other controls changed nothing, so they wait until they are wired.
struct Rows {
    /// App Icon: Android has no alternate icons.
    app_icon: bool,
    /// Live Activity: Android has none (nor a Dynamic Island).
    live_activity: bool,
    /// The terminal's font (Android draws its own).
    font: bool,
    /// The cursor's shape, colour, blink and trail.
    cursor: bool,
    /// Keyboard on open, remote clipboard read.
    behavior: bool,
    /// Predictive echo (Android predicts nothing).
    predict: bool,
    /// How long attached images are kept (Android attaches none).
    images: bool,
    /// Notifications' page (Android pairs nothing for push).
    notifications: bool,
    /// Voice input's page.
    voice: bool,
}

fn rows(os: Os) -> Rows {
    let apple = !os.is_android();
    Rows {
        app_icon: apple,
        live_activity: apple,
        font: apple,
        cursor: apple,
        behavior: apple,
        predict: apple,
        images: apple,
        notifications: apple,
        voice: apple,
    }
}

// --- Appearance ----------------------------------------------------------------

/// The appearance choice's words, in its segmented control's order.
fn appearance_title(choice: AppearanceChoice) -> String {
    match choice {
        AppearanceChoice::System => t!("System"),
        AppearanceChoice::Light => t!("Light"),
        AppearanceChoice::Dark => t!("Dark"),
    }
}

/// The appearance's row: the segmented control of system, light, dark.
fn appearance_choice(state: State, tell: &Tell) -> AnyElement {
    let pick = set(tell, Setting::Appearance);
    labelled(
        t!("Appearance"),
        segmented(
            AppearanceChoice::ALL.map(appearance_title),
            move || {
                let picked = state.read(|s| s.appearance);
                AppearanceChoice::ALL.iter().position(|a| *a == picked)
            },
            move |index| pick(AppearanceChoice::ALL[index]),
        )
        .label(t!("Appearance"))
        .test_id("settings-appearance"),
    )
}

/// The accent row's dot and the app icon's thumbnail.
const ACCENT_DOT: f32 = 20.0;
const ICON_THUMB: f32 = 24.0;

fn appearance(
    state: State,
    tell: &Tell,
    pet: Signal<Option<moshpit_pet::PetView>>,
) -> Vec<AnyElement> {
    let mut group = section().row(
        row(t!("Accent"))
            .leading(accent_dot(state))
            .value_with(move || state.read(|s| s.accent.name.clone()).into())
            .chevron()
            .on_press(open(tell, Page::Accent))
            .test_id("settings-accent"),
    );
    // Android has no alternate icons: a launcher shows the one the
    // manifest names, so the row (and its page) is left out there.
    if rows(Os::current()).app_icon {
        let thumbnail = dynamic(move || {
            let icon = state.read(|s| Some(s.app_icon.clone()));
            untracked(move || match icon {
                Some(icon) => app_icon::thumbnail(&icon, ICON_THUMB).into_element(),
                None => view().into_element(),
            })
        });
        group = group.row(
            row(t!("App Icon"))
                .leading(thumbnail)
                .value_with(move || state.read(|s| s.app_icon.name.clone()).into())
                .chevron()
                .on_press(open(tell, Page::AppIcon))
                .test_id("settings-app-icon"),
        );
    }
    // The appearance (R18): the system's, or light, or dark.
    group = group.item(appearance_choice(state, tell), Inset::Content);
    vec![
        group
            .footer(t!(
                "The accent colors buttons and highlights; the terminal's colors are under Terminal."
            ))
            .test_id("settings-appearance-group")
            .into_element(),
        companion_group(pet, tell),
    ]
}

// --- The companion (moshpit-design PET-DESIGN §7.5, §15.6) -------------------

type PetSignal = Signal<Option<moshpit_pet::PetView>>;

/// The companion's figure on its row and card: the egg, or the pet idle.
fn pet_figure(pet: PetSignal) -> impl Fn() -> crate::ui::companion::Figure + 'static {
    use crate::ui::companion::Figure;
    move || {
        pet.with(|p| {
            let p = p.as_ref();
            match p.and_then(|p| p.pet.as_ref()).and_then(|i| i.look()) {
                Some(look) => Figure::Pet {
                    look,
                    pose: moshpit_pet::Pose::Idle,
                },
                None => Figure::Egg {
                    ready: p.and_then(|p| p.egg).is_some_and(|e| e.ready),
                    wobble: false,
                },
            }
        })
    }
}

/// A stage's name.
fn stage_name(stage: u8) -> String {
    match stage {
        0 => t!("Hatchling"),
        1 => t!("Grown"),
        _ => t!("Seasoned"),
    }
}

/// The pet's name: the one it was given, or its species'.
fn pet_name(info: &moshpit_pet::PetInfo) -> String {
    if info.name.is_empty() {
        crate::home::pet::species_name(&info.species)
    } else {
        info.name.clone()
    }
}

/// Appearance's COMPANION group: one switch, Pixel Companion (on: the
/// pixel style with the companion shown; off: today's look, no companion),
/// and the row that opens the pet's card. Only the pixel style is made for
/// now (the owner's call, 2026-11-15); the record keeps room for others.
fn companion_group(pet: PetSignal, tell: &Tell) -> AnyElement {
    use moshpit_pet::Style;
    let on = move || {
        pet.with(|p| {
            p.as_ref()
                .is_none_or(|p| p.shown && p.style == Style::Pixel)
        })
    };
    let toggle = {
        let tell = tell.clone();
        move |on: bool| {
            let style = if on { Style::Pixel } else { Style::Minimal };
            tell(Setting::Companion(style, on).event())
        }
    };
    let title = move || {
        pet.with(|p| match p.as_ref().and_then(|p| p.pet.as_ref()) {
            Some(info) => pet_name(info),
            None => t!("Egg"),
        })
        .into()
    };
    let subtitle = move || {
        pet.with(|p| match p.as_ref().and_then(|p| p.pet.as_ref()) {
            Some(info) => format!(
                "{} · {}",
                crate::home::pet::species_kind(&info.species),
                stage_name(info.stage)
            ),
            None => t!("Tap it three times on Home to hatch it. It also hatches by itself when an agent finishes a task."),
        })
        .into()
    };
    let figure = view()
        .w(px(32.0))
        .h(px(32.0))
        .items_center()
        .justify_center()
        .child(crate::ui::companion::companion(pet_figure(pet), 32.0));
    let talks = move || pet.with(|p| p.as_ref().is_none_or(|p| p.talk));
    let talk = {
        let tell = tell.clone();
        move |on: bool| tell(Setting::PetTalk(on).event())
    };
    section()
        .header(t!("COMPANION"))
        .row(
            row(t!("Pixel Companion"))
                .subtitle(t!("On the Dynamic Island, the lock screen and Home"))
                .wrap_subtitle()
                .trailing(
                    switch(on, toggle)
                        .label(t!("Pixel Companion"))
                        .test_id("settings-companion-shown"),
                ),
        )
        .row(
            row(t!("Let It Talk"))
                .subtitle(t!("A short line on Home now and then"))
                .wrap_subtitle()
                .disabled(move || !on())
                .trailing(
                    switch(talks, talk)
                        .disabled(move || !on())
                        .label(t!("Let It Talk"))
                        .test_id("settings-companion-talk"),
                ),
        )
        .row(
            row(String::new())
                .title_with(title)
                // The egg's is how it hatches: a sentence, whole (B747).
                .subtitle_with(subtitle)
                .wrap_subtitle()
                .leading(figure)
                .chevron()
                .disabled(move || !on())
                .on_press(go(tell, Part::Companion))
                .test_id("settings-companion-card"),
        )
        .footer(t!("Tap it on Home and it reacts. On the Dynamic Island and the lock screen it changes pose but doesn't move. Each server's mark becomes a pixel creature too."))
        .test_id("settings-companion")
        .into_element()
}

/// Days from `from` to `to`, whole.
fn days_between(from: i64, to: i64) -> i64 {
    ((to - from).max(0)) / 86_400
}

/// A trait's id in words.
fn trait_word(id: &str) -> String {
    match id {
        "natural" => t!("Natural"),
        "peach" => t!("Peach"),
        "mint" => t!("Mint"),
        "sky" => t!("Sky"),
        "lilac" => t!("Lilac"),
        "sand" => t!("Sand"),
        "slate" => t!("Slate"),
        "rose" => t!("Rose"),
        "moss" => t!("Moss"),
        "charcoal" => t!("Charcoal"),
        "aurora" => t!("Aurora"),
        "plain" => t!("Plain"),
        "spots" => t!("Spots"),
        "stripes" => t!("Stripes"),
        "patch" => t!("Belly Patch"),
        "round" => t!("Round"),
        "lidded" => t!("Sleepy"),
        "starry" => t!("Starry"),
        "button" => t!("Button"),
        "short" => t!("Short"),
        "standard" => t!("Standard"),
        "tall" => t!("Tall"),
        "sprout" => t!("a sprout"),
        "moss_tuft" => t!("a tuft of moss"),
        "crystal" => t!("a crystal"),
        "freckles" => t!("freckles"),
        "star_mark" => t!("a star"),
        other => other.to_owned(),
    }
}

/// A stage's change in words.
fn change_words(change: &moshpit_pet::view::ChangeView) -> String {
    let what = trait_word(&change.what);
    match change.kind.as_str() {
        "fur" => t!("New fur: %@", arg1 = what),
        "pattern" => t!("New pattern: %@", arg1 = what),
        "grew" => t!("Grew %@", arg1 = what),
        _ => t!("New eyes: %@", arg1 = what),
    }
}

/// An easter egg found, in words.
fn found_words(id: &str) -> Option<String> {
    match id {
        "night" => Some(t!("Hatched at three in the morning")),
        "roam" => Some(t!("Made it through a network change")),
        "gold" => Some(t!("A thousand tasks together")),
        "sudo" => Some(t!("Named sudo")),
        _ => None,
    }
}

/// The card's big sprite, on a row of its own: a tap pets it and, as on
/// Home, it answers with a line in a bubble on its leading side (B607) —
/// the same lines and rules (Home's [`crate::home::talk::line`]: a tap
/// always answers, never the same line twice in a row), and screen
/// readers hear it. The egg takes no taps here: it hatches on Home. The
/// sides share what the sprite leaves, so the bubble fits beside it, a long
/// line wrapping, and nothing moves when it comes or goes.
fn talking_sprite(pet: PetSignal, hour: Signal<u8>) -> AnyElement {
    use crate::home::pet::{label_of, play_in, say_into};
    use crate::home::talk::{Fill, line};
    use crate::ui::companion::Figure;
    use moshpit_pet::{
        Pose,
        talk::{Chatter, Cue},
    };

    let saying = signal(None::<(u64, String)>);
    let said = Rc::new(std::cell::Cell::new(0_u64));
    let chatter = Rc::new(std::cell::RefCell::new(Chatter::default()));
    let showing = signal(None::<Figure>);
    let base = Rc::new(pet_figure(pet));
    let hatched = computed(move || pet.with(|p| p.as_ref().is_some_and(|p| p.pet.is_some())));
    let press = move || {
        let Some(view) = pet.get_untracked() else {
            return;
        };
        let Some(look) = view.pet.as_ref().and_then(|p| p.look()) else {
            return;
        };
        haptic(Haptic::Impact(HapticImpact::Light));
        play_in(
            showing,
            Figure::Pet {
                look,
                pose: Pose::Petted,
            },
        );
        let now = nsh_core::time::unix_ms() / 1000.0;
        let said_now = line(
            &mut chatter.borrow_mut(),
            Cue::Tap,
            &view,
            hour.get_untracked(),
            Fill::default(),
            now,
        );
        if let Some((name, words)) = said_now {
            say_into(saying, &said, name, words);
        }
    };
    let sprite = move || {
        let base = base.clone();
        view()
            .w(px(120.0))
            .h(px(120.0))
            .rounded(px(number("radius")))
            .bg(move || token("muted"))
            .items_center()
            .justify_center()
            .test_id("settings-pet-sprite")
            .child(crate::ui::companion::companion(
                move || showing.get().unwrap_or_else(|| base()),
                96.0,
            ))
    };
    let sprite = dynamic(move || {
        if hatched.get() {
            let (sprite, press) = (sprite.clone(), press.clone());
            untracked(move || {
                sprite()
                    .role(AccessibilityRole::Button)
                    .label(move || pet.with(|p| p.as_ref().map(label_of).unwrap_or_default()))
                    .hint(|| t!("Tap to pet"))
                    .on_press(press)
                    .into_element()
            })
        } else {
            let sprite = sprite.clone();
            untracked(move || sprite().into_element())
        }
    });
    view()
        .w_full()
        .flex_row()
        .items_center()
        .child(
            view()
                .flex_1()
                .items_end()
                .justify_center()
                .pr(px(6.0))
                .pointer_events(PointerEvents::None)
                .child(crate::ui::bubble::speech(saying)),
        )
        .child(sprite)
        .child(view().flex_1())
        .into_element()
}

/// The pet's card (Appearance › the companion): who it is, how it grows,
/// what it's made of and what changed. Its own: nothing here is shared.
fn companion_card(pet: PetSignal, hour: Signal<u8>, tell: &Tell) -> Vec<AnyElement> {
    let info = move || pet.with(|p| p.as_ref().and_then(|p| p.pet.clone()));
    let hero = view()
        .w_full()
        .items_center()
        .gap(px(4.0))
        .py(px(16.0))
        .child(talking_sprite(pet, hour))
        .child(
            TextToken::Title
                .apply(text(move || {
                    info().map_or_else(|| t!("Egg"), |i| pet_name(&i))
                }))
                .text_color(move || token("foreground"))
                .mt(px(8.0)),
        )
        .child(
            TextToken::Subhead
                .apply(text(move || match info() {
                    Some(i) => {
                        let kind = crate::home::pet::species_kind(&i.species);
                        let shiny = if i.traits.shiny {
                            format!(" · {}", t!("Shiny"))
                        } else {
                            String::new()
                        };
                        format!("{kind} · {}{shiny}", stage_name(i.stage))
                    }
                    None => t!("Tap it three times on Home to hatch it. It also hatches by itself when an agent finishes a task."),
                }))
                .text_color(move || token("muted-foreground")),
        );
    let mut out = vec![
        section()
            .item(hero, Inset::Content)
            .test_id("settings-pet-hero")
            .into_element(),
    ];
    let has_pet = computed(move || pet.with(|p| p.as_ref().is_some_and(|p| p.pet.is_some())));
    let tell = tell.clone();
    out.push(
        when(move || has_pet.get(), move || {
            let draft = Rc::new(std::cell::RefCell::new(None::<String>));
            let keep = {
                let tell = tell.clone();
                let draft = draft.clone();
                move || {
                    if let Some(name) = draft.borrow_mut().take() {
                        tell(Setting::PetName(name).event());
                    }
                }
            };
            let name = field(
                t!("Nickname"),
                text_input()
                    .default_value(move || info().map(|i| i.name).unwrap_or_default())
                    .placeholder(info().map(|i| crate::home::pet::species_name(&i.species)).unwrap_or_default())
                    .max_length(24)
                    .on_change({
                        let draft = draft.clone();
                        move |text| *draft.borrow_mut() = Some(text)
                    })
                    .on_blur(keep.clone())
                    .on_submit(keep),
            )
            .test_id("settings-pet-name");
            let now = (nsh_core::time::unix_ms() / 1000.0) as i64;
            let growing = section()
                .header(t!("GROWING"))
                .row(row(t!("Tasks together")).value_with(move || info().map(|i| i.tasks.to_string()).unwrap_or_default().into()))
                .row(row(t!("Days together")).value_with(move || info().map(|i| days_between(i.hatched_at, now).to_string()).unwrap_or_default().into()))
                .row(
                    row(String::new())
                        .title_with(move || match info().and_then(|i| i.next) {
                            Some(n) => t!("Next: %@", arg1 = stage_name(n.stage)).into(),
                            None => t!("Fully grown").into(),
                        })
                        .subtitle_with(move || match info().and_then(|i| i.next) {
                            Some(n) => {
                                let days = (n.not_before - now).max(0).div_euclid(86_400)
                                    + i64::from((n.not_before - now).rem_euclid(86_400) > 0 && n.not_before > now);
                                let mut parts = Vec::new();
                                if n.tasks_left > 0 {
                                    parts.push(crate::plural!("%lld more tasks", n.tasks_left as i64));
                                }
                                if days > 0 {
                                    parts.push(crate::plural!("%lld more days", days));
                                }
                                if parts.is_empty() {
                                    t!("It grows next time you see it").into()
                                } else {
                                    parts.join(" · ").into()
                                }
                            }
                            None => SharedString::default(),
                        }),
                );
            let traits = section()
                .header(t!("TRAITS"))
                .row(row(t!("Fur")).value_with(move || info().map(|i| trait_word(&i.traits.fur)).unwrap_or_default().into()))
                .row(row(t!("Pattern")).value_with(move || info().map(|i| trait_word(&i.traits.pattern)).unwrap_or_default().into()))
                .row(row(t!("Eyes")).value_with(move || info().map(|i| trait_word(&i.traits.eyes)).unwrap_or_default().into()))
                .row(row(t!("Build")).value_with(move || info().map(|i| trait_word(&i.traits.size)).unwrap_or_default().into()));
            let changes: Vec<Row> = info()
                .map(|i| {
                    let mut rows: Vec<Row> = i
                        .history
                        .iter()
                        .rev()
                        .map(|c| row(t!("Grew up: %@", arg1 = stage_name(c.stage))).subtitle(change_words(c)))
                        .collect();
                    rows.extend(i.found.iter().filter_map(|f| found_words(f)).map(row));
                    let ago = match days_between(i.hatched_at, now) {
                        0 => t!("Today"),
                        days => crate::plural!("%lld days ago", days),
                    };
                    rows.push(row(t!("Hatched")).subtitle(ago));
                    rows
                })
                .unwrap_or_default();
            view()
                .w_full()
                .gap(px(24.0))
                .child(section().item(name, Inset::Content).test_id("settings-pet-name-group"))
                .child(growing)
                .child(traits)
                .child(
                    section()
                        .header(t!("CHANGES"))
                        .rows(changes)
                        .footer(t!("It's yours alone. iCloud keeps it the same on your iPhone and iPad."))
                        .test_id("settings-pet-changes"),
                )
                .into_element()
        })
        .into_element(),
    );
    out
}

/// The accent in a circle, edged.
fn accent_dot(state: State) -> AnyElement {
    view()
        .w(px(ACCENT_DOT))
        .h(px(ACCENT_DOT))
        .flex_shrink_0()
        .rounded_full()
        .bg(move || {
            state
                .read(|s| Some(s.accent.color))
                .map(AdaptiveColor::from)
                .unwrap_or_else(|| token("primary"))
        })
        .border(px(1.0))
        .border_color(move || token("border"))
        .into_element()
}

// --- Terminal ------------------------------------------------------------------

fn terminal(state: State, tell: &Tell, anchors: Anchors) -> Vec<AnyElement> {
    let shape = set(tell, Setting::CursorShape);
    let color = set(tell, Setting::CursorColor);
    let cursor = section()
        .header(t!("CURSOR"))
        .item(
            labelled(
                t!("Shape"),
                segmented(
                    CursorShape::ALL.map(CursorShape::title),
                    move || {
                        let picked = state.read(|s| s.cursor_shape);
                        CursorShape::ALL.iter().position(|s| *s == picked)
                    },
                    move |index| shape(CursorShape::ALL[index]),
                )
                .label(t!("Shape"))
                .test_id("settings-cursor-shape"),
            ),
            Inset::Content,
        )
        .item(
            labelled(
                t!("Color"),
                cursor_colors(
                    move || state.read(|s| s.cursor_color.clone()),
                    move |id| color(id.to_owned()),
                )
                .test_id("settings-cursor-color"),
            ),
            Inset::Content,
        )
        .row(toggle(
            state,
            tell,
            t!("Blink"),
            None,
            |s| s.cursor_blink,
            Setting::CursorBlink,
            "settings-cursor-blink",
        ))
        .row(toggle(
            state,
            tell,
            t!("Trail while predicting"),
            Some(t!("mosh only")),
            |s| s.trail_on_predict,
            Setting::TrailOnPredict,
            "settings-cursor-trail",
        ))
        .footer(t!("Cursor settings apply to every session."))
        .test_id("settings-cursor-group");
    // Each where it is wired (B539).
    let shown = rows(Os::current());
    let mut display = section();
    if shown.font {
        display = display.item(font_select(state, tell), Inset::Content);
    }
    let display = display
        .row(size_row(state, tell))
        .row(
            row(t!("Theme"))
                .value_with(move || state.read(|s| s.theme.name.clone()).into())
                .chevron()
                .on_press(open(tell, Page::Theme))
                .test_id("settings-theme"),
        )
        .test_id("settings-display-group");
    let mut page = vec![preview(state), display.into_element()];
    if shown.cursor {
        page.push(anchors.mark(Anchor::Cursor, cursor.into_element()));
    }
    page
}

/// Font: the system's menu of the terminal's fonts.
fn font_select(state: State, tell: &Tell) -> AnyElement {
    let pick: Rc<dyn Fn(String)> = Rc::new(set(tell, Setting::Font));
    dynamic(move || {
        let fonts = state.read(|s| s.fonts.clone());
        let pick = pick.clone();
        untracked(move || {
            select(
                t!("Font"),
                fonts.iter().map(|f| option(f.id.clone(), f.name.clone())),
                move || state.read(|s| s.font.clone()).into(),
                move |id: SharedString| pick(id.to_string()),
            )
            .test_id("settings-font")
            .into_element()
        })
    })
    .into_element()
}

/// Size: the font's size and its steps (`−`/`+`, a point each: a slider is
/// hard to stop on one point on a phone).
fn size_row(state: State, tell: &Tell) -> Row {
    let change: Rc<dyn Fn(f64)> = Rc::new(set(tell, Setting::FontSize));
    let size = move || state.read(|s| s.font_size).round();
    let step = move |delta: f64| {
        let change = change.clone();
        move || {
            let next = (size() + delta).clamp(FONT_MIN, FONT_MAX);
            if next != size() {
                change(next);
            }
        }
    };
    let steps = view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(
            icon_button("minus", t!("Smaller"))
                .variant(IconVariant::Tinted)
                .size(IconButtonSize::Sm)
                .disabled(move || size() <= FONT_MIN)
                .on_press(step(-1.0))
                .test_id("settings-font-smaller"),
        )
        .child(
            icon_button("plus", t!("Larger"))
                .variant(IconVariant::Tinted)
                .size(IconButtonSize::Sm)
                .disabled(move || size() >= FONT_MAX)
                .on_press(step(1.0))
                .test_id("settings-font-larger"),
        );
    row(t!("Size"))
        .value_with(move || t!("%lld pt", arg1 = size() as i64).into())
        .trailing(steps)
        .test_id("settings-font-size")
}

/// Whether the terminal's scheme is a light one: the terminal takes its
/// scheme's appearance (R12, [`crate::screen::terminal_appearance`]), not
/// the app's, and so does the preview.
fn scheme_is_light(state: State) -> bool {
    state
        .read(|s| Some(s.theme.background))
        .is_some_and(crate::screen::is_light_background)
}

/// `color`, a token's, as the terminal draws it: its side for the scheme's
/// appearance, whichever Settings is in.
fn as_terminal(state: State, color: AdaptiveColor) -> AdaptiveColor {
    let side = if scheme_is_light(state) {
        color.light
    } else {
        color.dark
    };
    AdaptiveColor {
        light: side,
        dark: side,
    }
}

/// The preview: a few lines of a made-up session in the terminal's theme,
/// font and size, ending at the cursor as the settings draw it. It is the
/// terminal's picture, in the scheme's own colors: a dark scheme's stays
/// dark on a light page.
fn preview(state: State) -> AnyElement {
    let scheme = move || state.read(|s| Some((s.theme.background, s.theme.foreground)));
    // An ANSI color, or the words' own (`None`).
    let ink = move |index: Option<usize>| -> AdaptiveColor {
        index
            .and_then(|index| state.read(|s| s.theme.swatches.get(index).copied()))
            .or_else(|| scheme().map(|(_, foreground)| foreground))
            .map(AdaptiveColor::from)
            .unwrap_or_else(|| as_terminal(state, token("foreground")))
    };
    let lines = dynamic(move || {
        let (font, size, shape) =
            state.read(|s| (s.font.clone(), s.font_size as f32, s.cursor_shape));
        untracked(move || {
            let family = if font == "system" || font.is_empty() {
                "monospace".to_owned()
            } else {
                format!("{font}, monospace")
            };
            let mono = move |words: &'static str, index: Option<usize>| -> AnyElement {
                face::face(text(words), Design::Monospaced, size, FontWeight::NORMAL)
                    .font_family(family.clone())
                    .number_of_lines(1u32)
                    .text_color(move || ink(index))
                    .into_element()
            };
            let line = |parts: Vec<AnyElement>| view().flex_row().items_center().children(parts);
            view()
                .gap(px(size * 0.3))
                .child(line(vec![
                    mono("➜  ", Some(2)),
                    mono("api ", Some(6)),
                    mono("git:(", Some(4)),
                    mono("main", Some(1)),
                    mono(") ", Some(4)),
                    mono("ls", None),
                ]))
                .child(line(vec![mono("Cargo.toml  README.md  src", None)]))
                .child(line(vec![
                    mono("➜  ", Some(2)),
                    mono("api ", Some(6)),
                    preview_cursor(state, size, shape),
                ]))
                .into_element()
        })
    });
    view()
        .w_full()
        .p(px(16.0))
        .bg(move || {
            scheme()
                .map(|(background, _)| AdaptiveColor::from(background))
                .unwrap_or_else(|| as_terminal(state, token("terminal-bg")))
        })
        .rounded(px(number("radius-lg")))
        .border_curve(BorderCurve::Continuous)
        .border(px(1.0))
        .border_color(move || token("border"))
        .overflow_hidden()
        .accessibility_elements_hidden(true)
        .test_id("settings-terminal-preview")
        .child(lines)
        .into_element()
}

/// The preview's cursor: the picked shape in the picked color, blinking
/// while Blink is on (never with reduced motion), after the trail while
/// Trail while predicting is on.
fn preview_cursor(state: State, size: f32, shape: CursorShape) -> AnyElement {
    let blink = computed(move || state.read(|s| s.cursor_blink) && !reduce_motion());
    // As the terminal draws it (B318): a choice the theme hides draws as
    // Accent, as its swatch shows picked.
    let paint = move || {
        let color = CursorColor::drawn(
            &state.read(|s| s.cursor_color.clone()),
            &swatch::live_palette,
        )
        .color();
        as_terminal(state, color)
    };
    let cell = (size * 0.6, size * 1.2);
    let (w, h) = match shape {
        CursorShape::Block => cell,
        CursorShape::Bar => (2.0, cell.1),
        CursorShape::Underline => (cell.0, 2.0),
    };
    // On and off, 0.55 s each way, natively.
    let opacity = animated_value(1.0);
    let blinking = Animation::sequence([
        Animation::timing(&opacity, 0.2, Timing::new(550.0, Easing::EaseInOut)),
        Animation::timing(&opacity, 1.0, Timing::new(550.0, Easing::EaseInOut)),
    ])
    .looped(None);
    effect(move || {
        if blink.get() {
            drop(blinking.start());
        } else {
            blinking.stop();
            opacity.set(1.0);
        }
    });
    let trail = computed(move || state.read(|s| s.trail_on_predict));
    view()
        .h(px(cell.1))
        .flex_row()
        .items_end()
        .test_id("settings-preview-cursor")
        .child(when(
            move || trail.get(),
            move || {
                dynamic(move || {
                    let tint: Hsla = nsh_core::prelude::color(paint());
                    untracked(move || {
                        view()
                            .w(px(cell.0 * 3.0))
                            .h(px(h))
                            .bg_linear_to_r()
                            .gradient_from(Hsla { a: 0.0, ..tint })
                            .gradient_to(Hsla {
                                a: tint.a * 0.55,
                                ..tint
                            })
                    })
                })
            },
        ))
        .child(view().w(px(w)).h(px(h)).bg(paint).bind_opacity(&opacity))
        .into_element()
}

// --- Connections ---------------------------------------------------------------

fn connections(state: State, tell: &Tell, anchors: Anchors) -> Vec<AnyElement> {
    let pick = set(tell, Setting::PredictMode);
    let retention = set(tell, Setting::UploadRetentionDays);
    let behavior = section()
        .row(toggle(
            state,
            tell,
            t!("Keyboard on open"),
            None,
            |s| s.keyboard_on_open,
            Setting::KeyboardOnOpen,
            "settings-keyboard-on-open",
        ))
        .row(toggle(
            state,
            tell,
            t!("Remote clipboard read"),
            Some(t!("OSC 52; off answers with an empty clipboard")),
            |s| s.remote_clipboard_read,
            Setting::RemoteClipboardRead,
            "settings-remote-clipboard",
        ))
        .test_id("settings-behavior-group");
    // Each where it is wired (B539).
    let shown = rows(Os::current());
    let mosh = section().header(t!("MOSH")).row(toggle(
        state,
        tell,
        t!("Mosh for new connections"),
        None,
        |s| s.mosh_by_default,
        Setting::MoshByDefault,
        "settings-mosh-default",
    ));
    let mosh = if !shown.predict {
        mosh
    } else {
        mosh.item(
            select(
                t!("Predictive echo"),
                PredictMode::ALL.map(|mode| option(mode.id(), mode.title())),
                move || state.read(|s| s.predict_mode).id().into(),
                move |id: SharedString| {
                    if let Some(mode) = PredictMode::ALL.into_iter().find(|m| m.id() == &*id) {
                        pick(mode);
                    }
                },
            )
            .test_id("settings-predict"),
            Inset::Content,
        )
    };
    let mosh = mosh
        .row(
            row(t!("UDP ports"))
                .value_with(move || {
                    let [from, to] = state.read(|s| s.udp_range);
                    format!("{from}–{to}").into()
                })
                .mono_value()
                .chevron()
                .on_press(open(tell, Page::UdpRange))
                .test_id("settings-udp-range"),
        )
        .footer(t!("Open the server's UDP ports in this range."))
        .test_id("settings-mosh-group");
    let images = section()
        .header(t!("ATTACHED IMAGES"))
        .item(
            labelled(
                t!("Keep"),
                segmented(
                    [t!("24 hours"), t!("7 days"), t!("Forever")],
                    move || {
                        let days = state.read(|s| s.upload_retention_days);
                        RETENTION.iter().position(|d| *d == days)
                    },
                    move |index| retention(RETENTION[index]),
                )
                .label(t!("Keep"))
                .test_id("settings-upload-retention"),
            ),
            Inset::Content,
        )
        .footer(t!(
            "Images upload to the host's ~/.moshpit/uploads; each connect clears expired ones."
        ))
        .test_id("settings-images-group");
    let mut page = Vec::new();
    if shown.behavior {
        page.push(anchors.mark(Anchor::Behavior, behavior.into_element()));
    }
    page.push(anchors.mark(Anchor::Mosh, mosh.into_element()));
    if shown.images {
        page.push(images.into_element());
    }
    page
}

// --- Notifications -------------------------------------------------------------

fn notifications(state: State, tell: &Tell, anchors: Anchors) -> Vec<AnyElement> {
    let mut group = section().row(toggle(
        state,
        tell,
        t!("Notifications"),
        Some(t!("Alert when an agent needs you")),
        |s| s.notifications,
        Setting::Notifications,
        "settings-notifications",
    ));
    // No Live Activities, no Dynamic Island on Android.
    if rows(Os::current()).live_activity {
        group = group.row(toggle(
            state,
            tell,
            t!("Live Activity"),
            Some(t!("Status in the Dynamic Island")),
            |s| s.live_activity,
            Setting::LiveActivity,
            "settings-live-activity",
        ));
    }
    let group = group
        .row(toggle(
            state,
            tell,
            t!("Alert sound"),
            Some(t!("Play a sound when the agent needs you")),
            |s| s.alert_sound,
            Setting::AlertSound,
            "settings-alert-sound",
        ))
        .row(toggle(
            state,
            tell,
            t!("Show detail on lock screen"),
            Some(t!("Off keeps it private")),
            |s| s.lock_screen_detail,
            Setting::LockScreenDetail,
            "settings-lock-screen-detail",
        ))
        .row(
            row(t!("How notifications work"))
                .chevron()
                .on_press(open(tell, Page::NotificationInfo))
                .test_id("settings-notification-info"),
        )
        .test_id("settings-notifications-group");
    vec![
        ios_off(state, tell),
        anchors.mark(Anchor::Notifications, group.into_element()),
    ]
}

/// What iOS itself has off of what the app's switches have on (B741), and
/// the way to iOS's settings for it; nothing while iOS allows them all.
fn ios_off(state: State, tell: &Tell) -> AnyElement {
    let live_activity = rows(Os::current()).live_activity;
    let off = move || {
        state.read(|s| {
            (
                s.notifications && s.notifications_blocked,
                live_activity && s.live_activity && s.live_activity_blocked,
            )
        })
    };
    let tell = tell.clone();
    when(
        move || off() != (false, false),
        move || {
            section()
                .row(
                    row(t!("Allow in iOS Settings"))
                        .tone(RowTone::Primary)
                        .subtitle_with(move || {
                            match off() {
                                (true, true) => {
                                    t!("iOS has notifications and Live Activities off for Moshpit.")
                                }
                                (true, false) => t!(
                                    "iOS has notifications off for Moshpit, so no alert reaches you."
                                ),
                                _ => t!(
                                    "iOS has Live Activities off for Moshpit, so the Dynamic Island stays empty."
                                ),
                            }
                            .into()
                        })
                        .wrap_subtitle()
                        .chevron()
                        .on_press(open(&tell, Page::IosSettings))
                        .test_id("settings-ios-off"),
                )
                .test_id("settings-ios-off-group")
        },
    )
    .into_element()
}

// --- Voice input ---------------------------------------------------------------

fn voice(state: State, tell: &Tell) -> Vec<AnyElement> {
    let tell = tell.clone();
    vec![
        dynamic(move || {
            let (enabled, whisper) =
                state.read(|s| (s.voice_input, s.voice_engine == Engine::Whisper));
            let tell = tell.clone();
            untracked(move || {
                let mut group = section().row(toggle(
                    state,
                    &tell,
                    t!("Voice input"),
                    Some(t!("Adds a mic key to the shortcut bar")),
                    |s| s.voice_input,
                    Setting::VoiceInput,
                    "settings-voice-input",
                ));
                if enabled {
                    group = group
                        .row(
                            row(t!("Recognition"))
                                .value_with(move || state.read(|s| s.voice_engine).name().into())
                                .chevron()
                                .on_press(open(&tell, Page::VoiceEngine))
                                .test_id("settings-voice-engine"),
                        )
                        .row(
                            row(t!("Language"))
                                .value_with(move || state.read(|s| s.voice_language.clone()).into())
                                .chevron()
                                .on_press(open(&tell, Page::VoiceLanguage))
                                .test_id("settings-voice-language"),
                        );
                    if whisper {
                        group = group.row(
                            row(t!("Model"))
                                .value_with(move || {
                                    state
                                        .read(|s| s.voice_model.clone())
                                        .unwrap_or_else(|| t!("None"))
                                        .into()
                                })
                                .chevron()
                                .on_press(open(&tell, Page::WhisperModel))
                                .test_id("settings-voice-model"),
                        );
                    }
                }
                group
                    .footer(t!(
                        "Speech is transcribed on this device and never leaves it."
                    ))
                    .test_id("settings-voice-group")
            })
        })
        .into_element(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_leaves_out_the_app_icon_and_live_activity_rows() {
        assert!(!rows(Os::Android).app_icon && !rows(Os::Android).live_activity);
        assert!(rows(Os::Apple).app_icon && rows(Os::Apple).live_activity);
    }

    /// B539: Android shows only what it reads; the font, the cursor, the
    /// behaviour switches, predictive echo, attached images, notifications
    /// and voice input wait until they are wired. Every one shows on Apple.
    #[test]
    fn android_leaves_out_what_it_does_not_read_yet() {
        let shown = |r: Rows| {
            [
                r.font,
                r.cursor,
                r.behavior,
                r.predict,
                r.images,
                r.notifications,
                r.voice,
            ]
        };
        assert_eq!(shown(rows(Os::Android)), [false; 7]);
        assert_eq!(shown(rows(Os::Apple)), [true; 7]);
    }

    /// A state as the host posts it.
    pub fn sample() -> SettingsState {
        SettingsState {
            accent: Swatch {
                name: "Signal Room".into(),
                color: nsh_core::color::rgb(0x6C6BEF),
            },
            app_icon: app_icon::Icon {
                id: "default".into(),
                name: "Default".into(),
                image: Some("asset:/IconPreview-Default@3x.png".into()),
            },
            fonts: vec![
                Choice {
                    id: "system".into(),
                    name: "SF Mono".into(),
                },
                Choice {
                    id: "Menlo".into(),
                    name: "Menlo".into(),
                },
            ],
            font: "system".into(),
            font_size: 9.0,
            theme: TerminalTheme {
                name: "GitHub Dark".into(),
                background: nsh_core::color::rgb(0x0D1117),
                foreground: nsh_core::color::rgb(0xC9D1D9),
                swatches: (0..8).map(|i| nsh_core::color::rgb(0x101010 * i)).collect(),
            },
            cursor_shape: CursorShape::Block,
            cursor_color: "teal".into(),
            cursor_blink: true,
            trail_on_predict: true,
            keyboard_on_open: false,
            remote_clipboard_read: false,
            upload_retention_days: 7,
            mosh_by_default: true,
            predict_mode: PredictMode::Adaptive,
            udp_range: [60000, 61000],
            shortcuts: Count {
                count: 9,
                limit: 12,
            },
            keys: 1,
            notifications: true,
            live_activity: true,
            notifications_blocked: false,
            live_activity_blocked: false,
            alert_sound: true,
            lock_screen_detail: true,
            live_session: false,
            voice_input: true,
            voice_engine: Engine::Whisper,
            voice_model: None,
            voice_language: "Auto-detect".into(),
            version: "Moshpit 1.0.9 (1) · dev".into(),
            filing: None,
            app_theme: AppTheme::SIGNAL_ROOM,
            appearance: AppearanceChoice::System,
        }
    }

    #[test]
    fn messages_round_trip() {
        let mut state = sample();
        let message = Message::Settings(Box::new(state.clone()));
        assert_eq!(Message::parse(&message.to_json()), Ok(message));
        state.filing = Some("浙ICP备2025212603号-2A".into());
        state.voice_model = Some("Small".into());
        state.cursor_shape = CursorShape::Underline;
        state.predict_mode = PredictMode::Experimental;
        let message = Message::Settings(Box::new(state));
        assert_eq!(Message::parse(&message.to_json()), Ok(message));
        for anchor in [Anchor::Cursor, Anchor::Mosh, Anchor::Bottom] {
            let message = Message::Reveal(anchor);
            assert_eq!(Message::parse(&message.to_json()), Ok(message));
        }
        assert_eq!(
            Message::parse(r#"{"op":"reveal","anchor":"notifications"}"#),
            Ok(Message::Reveal(Anchor::Notifications))
        );
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        let good: serde_json::Value =
            serde_json::from_str(&Message::Settings(Box::new(sample())).to_json()).unwrap();
        let with = |key: &str, value: serde_json::Value| {
            let mut bad = good.clone();
            bad[key] = value;
            bad.to_string()
        };
        let without = |key: &str| {
            let mut bad = good.clone();
            bad.as_object_mut().unwrap().remove(key);
            bad.to_string()
        };
        for (json, says) in [
            ("{".to_owned(), "not JSON".to_owned()),
            (
                r#"{"op":"engine"}"#.to_owned(),
                "no op \"engine\"".to_owned(),
            ),
            (with("cursorShape", json!("beam")), "beam".to_owned()),
            (
                with("predictMode", json!("sometimes")),
                "sometimes".to_owned(),
            ),
            (with("voiceEngine", json!("siri")), "siri".to_owned()),
            (
                with("accent", json!({"name": "A", "color": "teal"})),
                "teal".to_owned(),
            ),
            (with("colour", json!(1)), "colour".to_owned()),
            (without("keyboardOnOpen"), "keyboardOnOpen".to_owned()),
            (without("version"), "version".to_owned()),
            (
                r#"{"op":"reveal","anchor":"top"}"#.to_owned(),
                "top".to_owned(),
            ),
            (r#"{"op":"reveal"}"#.to_owned(), "anchor".to_owned()),
        ] {
            let error = Message::parse(&json).unwrap_err();
            assert!(error.contains(&says), "{json}: {error}");
        }
    }

    #[test]
    fn pages_and_settings_as_the_host_reads_them() {
        for page in Page::ALL {
            assert_eq!(Page::named(page.id()), Some(page));
        }
        assert_eq!(
            Page::WhisperModel.event(),
            json!({"ev": "open", "page": "whisper-model"})
        );
        assert_eq!(
            Setting::KeyboardOnOpen(false).event(),
            json!({"ev": "set", "key": "keyboardOnOpen", "value": false})
        );
        assert_eq!(
            Setting::CursorShape(CursorShape::Bar).event(),
            json!({"ev": "set", "key": "cursorShape", "value": "bar"})
        );
        assert_eq!(
            Setting::PredictMode(PredictMode::Never).event(),
            json!({"ev": "set", "key": "predictMode", "value": "never"})
        );
        assert_eq!(
            Setting::FontSize(12.0).event(),
            json!({"ev": "set", "key": "fontSize", "value": 12.0})
        );
        assert_eq!(
            Setting::UploadRetentionDays(0).event(),
            json!({"ev": "set", "key": "uploadRetentionDays", "value": 0})
        );
    }

    #[test]
    fn parts_as_the_host_names_them() {
        assert_eq!(Part::named(None), Part::Hub);
        assert_eq!(Part::named(Some("mystery")), Part::Hub);
        for part in Part::PAGES {
            assert_eq!(Part::named(Some(part.id())), part);
            assert_eq!(part.event(), json!({"ev": "open", "page": part.id()}));
            assert!(
                Page::named(part.id()).is_none(),
                "{part:?} is not one of the host's own pages"
            );
        }
    }

    #[test]
    fn the_version_row_shows_the_version_and_build() {
        assert_eq!(short_version("Moshpit 1.0.9 (1) · 4db8aeff+"), "1.0.9 (1)");
        assert_eq!(short_version("Moshpit 1.0.9 (1) · dev"), "1.0.9 (1)");
        assert_eq!(short_version("something else"), "something else");
    }
}
