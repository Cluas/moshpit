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
//! for the view identified `settings-version`; seven quick taps on the row
//! open the hidden Core Switches page. The hub's rows carry no help text;
//! each page's footnote is at most two lines.

use std::{
    cell::Cell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

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
        page,
        row::{Row, row},
        section::{Inset, section},
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
    pub keep_alive: bool,
    pub keyboard_on_open: bool,
    pub remote_clipboard_read: bool,
    /// Days uploads stay on the server: 1, 7, or 0 for ever.
    pub upload_retention_days: i64,
    pub mosh_by_default: bool,
    pub predict_mode: PredictMode,
    pub mosh_server_binary: String,
    /// The UDP port range, first and last.
    pub udp_range: [i64; 2],
    /// Shortcuts on the terminal's bar, of how many fit.
    pub shortcuts: Count,
    /// SSH keys on the device.
    pub keys: i64,
    pub notifications: bool,
    pub live_activity: bool,
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
    ServerBinary,
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
    CoreSwitches,
}

impl Page {
    /// Every one.
    pub const ALL: [Page; 17] = [
        Page::Accent,
        Page::AppIcon,
        Page::Theme,
        Page::Shortcuts,
        Page::SshKeys,
        Page::ServerBinary,
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
        Page::CoreSwitches,
    ];

    /// What the host calls it.
    pub fn id(self) -> &'static str {
        match self {
            Page::Accent => "accent",
            Page::AppIcon => "app-icon",
            Page::Theme => "theme",
            Page::Shortcuts => "shortcuts",
            Page::SshKeys => "ssh-keys",
            Page::ServerBinary => "server-binary",
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
            Page::CoreSwitches => "core-switches",
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
    KeepAlive(bool),
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
            Setting::KeepAlive(on) => ("keepAlive", json!(on)),
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

/// The terminal font's sizes, in points.
const FONT_MIN: f64 = 8.0;
const FONT_MAX: f64 = 18.0;

/// A page of Settings: the hub, or a page it opens (`{"page": …}` in the
/// screen's params; `{"ev": "open", "page": …}` asks the host for one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    Hub,
    Appearance,
    Terminal,
    Connections,
    Notifications,
    Voice,
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

    /// What the host calls it (the hub: nothing).
    pub fn id(self) -> &'static str {
        match self {
            Part::Hub => "",
            Part::Appearance => "appearance",
            Part::Terminal => "terminal",
            Part::Connections => "connections",
            Part::Notifications => "notifications",
            Part::Voice => "voice",
        }
    }

    /// The part a screen's params name: the hub for none, or for a name no
    /// page has.
    pub fn named(id: Option<&str>) -> Part {
        Self::PAGES
            .into_iter()
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
        }
    }

    /// The event that opens it.
    pub fn event(self) -> serde_json::Value {
        json!({"ev": "open", "page": self.id()})
    }
}

/// The version a version line names (`Moshpit 1.0.9 (1) · 4db8aeff` →
/// `1.0.9 (1)`); the line whole when it is not one.
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
    let part = Part::named(params.page.as_deref());
    let state = State(signal(None::<SettingsState>));
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
            subpage(root, part, back, state, tell, anchors, ready)
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
        Rc::new(move || nav_bar::text_button(t!("Done"), true, "settings-done", done.clone()));
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
    vec![
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
        section()
            .row(
                row(t!("Notifications"))
                    .leading_tile("bell")
                    .value_with(move || on_off(state.read(|s| s.notifications)))
                    .chevron()
                    .on_press(go(tell, Part::Notifications))
                    .test_id("settings-open-notifications"),
            )
            .row(
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
            )
            .into_element(),
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
    ]
}

/// Taps on the version row that open Core Switches, and how far apart they
/// may come (SwiftUI's `onTapGesture(count: 7)`).
const SECRET_TAPS: u32 = 7;
const TAP_GAP: Duration = Duration::from_millis(500);

/// About: the version row (long-press: Copy, the host's menu; seven taps:
/// Core Switches), the tagline under it and, on the mainland storefront,
/// the APP filing.
fn about(state: State, tell: &Tell) -> AnyElement {
    let taps = Rc::new(Cell::new((0_u32, None::<Instant>)));
    let core_switches = open(tell, Page::CoreSwitches);
    let filing = Rc::new(open(tell, Page::Filing));
    let has_filing = computed(move || state.read(|s| s.filing.is_some()));
    let version = row(t!("Version"))
        .leading_tile("info.circle")
        .value_with(move || short_version(&state.read(|s| s.version.clone())).into())
        .on_press(move || {
            // The engine's clock: the system's on a device, the test's in a
            // headless run (a manual clock's taps are as quick as the test
            // says, however loaded the machine).
            let now = nsh_core::time::now();
            let (count, last) = taps.get();
            let count = match last {
                Some(last) if now.duration_since(last) <= TAP_GAP => count + 1,
                _ => 1,
            };
            if count >= SECRET_TAPS {
                taps.set((0, None));
                core_switches();
            } else {
                taps.set((count, Some(now)));
            }
        })
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
fn subpage(
    root: View,
    part: Part,
    back: String,
    state: State,
    tell: Tell,
    anchors: Anchors,
    ready: Memo<bool>,
) -> AnyElement {
    let bar = anchors.bar;
    let scrolled = nav_bar::ScrollEdge::new();
    let body = {
        let tell = tell.clone();
        move || {
            let groups = match part {
                Part::Appearance => appearance(state, &tell),
                Part::Terminal => terminal(state, &tell, anchors),
                Part::Connections => connections(state, &tell, anchors),
                Part::Notifications => notifications(state, &tell, anchors),
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
        trailing: Vec::new(),
        on_back: Box::new(move || tell(json!({"ev": "back"}))),
        scrolled,
    }))
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
        Some(subtitle) => row.subtitle(subtitle),
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

/// The rows Settings shows only on some platforms (B258).
struct Rows {
    /// App Icon: Android has no alternate icons.
    app_icon: bool,
    /// Live Activity: Android has none (nor a Dynamic Island).
    live_activity: bool,
}

fn rows(os: Os) -> Rows {
    Rows {
        app_icon: !os.is_android(),
        live_activity: !os.is_android(),
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

fn appearance(state: State, tell: &Tell) -> Vec<AnyElement> {
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
    ]
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
    vec![
        preview(state),
        section()
            .item(font_select(state, tell), Inset::Content)
            .row(size_row(state, tell))
            .row(
                row(t!("Theme"))
                    .value_with(move || state.read(|s| s.theme.name.clone()).into())
                    .chevron()
                    .on_press(open(tell, Page::Theme))
                    .test_id("settings-theme"),
            )
            .test_id("settings-display-group")
            .into_element(),
        anchors.mark(Anchor::Cursor, cursor.into_element()),
    ]
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
            t!("Keep connections alive"),
            Some(t!("Sends a keepalive so idle connections aren't dropped.")),
            |s| s.keep_alive,
            Setting::KeepAlive,
            "settings-keep-alive",
        ))
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
    let mosh = section()
        .header(t!("MOSH"))
        .row(toggle(
            state,
            tell,
            t!("Mosh for new connections"),
            None,
            |s| s.mosh_by_default,
            Setting::MoshByDefault,
            "settings-mosh-default",
        ))
        .item(
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
        .row(
            row(t!("mosh-server path"))
                .value_with(move || {
                    let path = state.read(|s| s.mosh_server_binary.clone());
                    if path.is_empty() {
                        t!("Automatic").into()
                    } else {
                        path.into()
                    }
                })
                .mono_value()
                .stacked(|| true)
                .chevron()
                .on_press(open(tell, Page::ServerBinary))
                .test_id("settings-server-binary"),
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
    vec![
        anchors.mark(Anchor::Behavior, behavior.into_element()),
        anchors.mark(Anchor::Mosh, mosh.into_element()),
        images.into_element(),
    ]
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
    vec![anchors.mark(Anchor::Notifications, group.into_element())]
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
            keep_alive: true,
            keyboard_on_open: false,
            remote_clipboard_read: false,
            upload_retention_days: 7,
            mosh_by_default: true,
            predict_mode: PredictMode::Adaptive,
            mosh_server_binary: "/opt/homebrew/bin/mosh-server".into(),
            udp_range: [60000, 61000],
            shortcuts: Count {
                count: 9,
                limit: 12,
            },
            keys: 1,
            notifications: true,
            live_activity: true,
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
            (without("keepAlive"), "keepAlive".to_owned()),
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
            Setting::KeepAlive(false).event(),
            json!({"ev": "set", "key": "keepAlive", "value": false})
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
