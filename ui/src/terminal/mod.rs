//! The terminal screen (`Moshpit/UI/Terminal/TerminalScreen.swift`) in NSH:
//! the screen people live in. NSH draws the chrome; the terminal itself is
//! the host's native view (`moshpit/terminal`, SwiftTerm on iOS), placed as
//! a sibling under the chrome and never inside a pressable NSH container
//! (an ancestor's press would fire together with the terminal's own tap:
//! `docs/nsh/DESIGN.md` §3.2).
//!
//! - The top bar ([`bar`]): back (or the iPad's sidebar toggle), the
//!   transport pill, the breadcrumb on glass with its session segment that
//!   gives up its name only when three full segments do not fit.
//! - The pane area ([`cover`]): the terminal slot, the multiplexer's empty
//!   state, and over them a reconnect's veil with its capsule (never a
//!   full-screen cover, a first connect none at all: the old frame stays
//!   until the new one has painted, and the two cross-fade, which the
//!   host's view does), or the size-lease follower's veil.
//! - Under the bar, over the pane ([`chrome`]): the one banner (mosh's dead
//!   return path, roaming, a degraded session), the multiplexer's notice
//!   with its remedy, the chip that opens a link just copied.
//! - At the bottom ([`keys`], [`panels`]): the shortcut bar riding the
//!   keyboard (the column's bottom margin is the keyboard's height, moving
//!   on the keyboard's curve), and over it the voice, image and preview
//!   panels.
//!
//! The host owns the session and posts what to draw ([`model`], [`Message`]);
//! the page tells it what the person does ([`Intent`]). The keyboard is the
//! terminal's business: the host holds the person's intent (asked for,
//! put away, or not asked yet) and the first responder; the page only
//! rides the keyboard's height.
//!
//! The shell does not grow with the reading size past extra large
//! (`.dynamicTypeSize(...xLarge)` in SwiftUI): keyboard-like chrome, like
//! the system keyboard ([`size`]).

mod bar;
mod chrome;
mod cover;
mod diagnostics;
pub mod folder;
pub mod install;
mod keys;
pub mod model;
pub mod overview;
mod panels;
pub mod picker;
pub mod prompt;
pub mod shortcuts;
pub mod switcher;

use std::{collections::HashMap, rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde_json::json;

use crate::{
    channel::{self, Channel},
    face::{self, TextStyle},
    screen::Screen,
    theme,
};
pub use model::{Attachment, Bar, Chrome, Counters, Diagnostics, Dictation, Preview, Switcher};

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "chrome", …}`.
    Chrome(Box<Chrome>),
    /// `{"op": "bar", …}`.
    Bar(Box<Bar>),
    /// `{"op": "dictation", "panel": {…} | null}`.
    Dictation(Option<Box<Dictation>>),
    /// `{"op": "level", "level": 0.4}`: the voice meter, 0…1.
    Level(f32),
    /// `{"op": "attachment", "panel": {…} | null}`.
    Attachment(Option<Box<Attachment>>),
    /// `{"op": "preview", "panel": {…} | null}`.
    Preview(Option<Box<Preview>>),
    /// `{"op": "rekey"}`: make the terminal's element again (the handover
    /// test: the host keeps the terminal, the keyboard stays up).
    Rekey,
    /// `{"op": "diagnostics", "shown": bool, "counters": {…} | null}`:
    /// mosh's live counters, while the host shows them.
    Diagnostics(Box<Diagnostics>),
    /// `{"op": "key", "stroke": "mod+k"}`: one of the host's key commands
    /// (the scene's menu took the press before NSH's keymap could).
    Key(String),
    /// `{"op": "switcher", "tree": {…} | null, "hintSeen": bool}`: the
    /// session's panes a sideways swipe walks ([`switcher`]).
    Switcher(Box<Switcher>),
    /// `{"op": "swipe", "active": bool}`: a sideways swipe began (the
    /// terminal follows the finger) or settled.
    Swipe(bool),
    /// `{"op": "overview", "open": bool}`: the host opens (or closes) the
    /// pane overview: ⌘K's twin, VoiceOver's Show all panes.
    Overview(bool),
    /// `{"op": "overview-act", "action", "pane"}`: a pane's long-press menu
    /// (the host's) picked `action` (`open`, `new-pane`, `rename`, `kill`).
    OverviewAct { action: String, pane: String },
    /// `{"op": "prompted", "id", "text"}`: the host's answer to a prompt.
    Prompted(prompt::Answer),
    /// `{"op": "pictures", "panes": {"%1": ["row", …]}}`: the screens of
    /// the panes the overview asked for, as text rows, read from the
    /// terminals the session already keeps (B658). A pane left out has no
    /// picture.
    Pictures(HashMap<String, Vec<String>>),
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SwipeFields {
    active: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFields {
    stroke: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Panel<T> {
    panel: Option<T>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LevelFields {
    level: f32,
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("terminal", json)?;
        let fields = serde_json::Value::Object(fields);
        let error = |e: serde_json::Error| format!("terminal: {op}: {e}");
        match op.as_str() {
            "chrome" => serde_json::from_value(fields)
                .map(|c| Message::Chrome(Box::new(c)))
                .map_err(error),
            "bar" => serde_json::from_value(fields)
                .map(|b| Message::Bar(Box::new(b)))
                .map_err(error),
            "dictation" => serde_json::from_value::<Panel<Dictation>>(fields)
                .map(|p| Message::Dictation(p.panel.map(Box::new)))
                .map_err(error),
            "level" => serde_json::from_value::<LevelFields>(fields)
                .map(|l| Message::Level(l.level.clamp(0.0, 1.0)))
                .map_err(error),
            "attachment" => serde_json::from_value::<Panel<Attachment>>(fields)
                .map(|p| Message::Attachment(p.panel.map(Box::new)))
                .map_err(error),
            "preview" => serde_json::from_value::<Panel<Preview>>(fields)
                .map(|p| Message::Preview(p.panel.map(Box::new)))
                .map_err(error),
            "key" => serde_json::from_value::<KeyFields>(fields)
                .map(|k| Message::Key(k.stroke))
                .map_err(error),
            "diagnostics" => serde_json::from_value(fields)
                .map(|d| Message::Diagnostics(Box::new(d)))
                .map_err(error),
            "switcher" => serde_json::from_value(fields)
                .map(|s| Message::Switcher(Box::new(s)))
                .map_err(error),
            "swipe" => serde_json::from_value::<SwipeFields>(fields)
                .map(|f| Message::Swipe(f.active))
                .map_err(error),
            "overview" => serde_json::from_value::<overview::OpenFields>(fields)
                .map(|f| Message::Overview(f.open))
                .map_err(error),
            "overview-act" => serde_json::from_value::<overview::ActFields>(fields)
                .map(|f| Message::OverviewAct {
                    action: f.action,
                    pane: f.pane,
                })
                .map_err(error),
            "prompted" => serde_json::from_value(fields)
                .map(Message::Prompted)
                .map_err(error),
            "pictures" => serde_json::from_value::<overview::PicturesFields>(fields)
                .map(|f| Message::Pictures(f.panes))
                .map_err(error),
            "rekey" if fields.as_object().is_some_and(|f| f.is_empty()) => Ok(Message::Rekey),
            "rekey" => Err("terminal: rekey: takes no fields".to_owned()),
            other => Err(format!("terminal: no op {other:?}")),
        }
    }
}

/// A breadcrumb segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crumb {
    Session,
    Window,
    Pane,
}

/// An arrow or scroll direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub fn name(self) -> &'static str {
        match self {
            Dir::Up => "up",
            Dir::Down => "down",
            Dir::Left => "left",
            Dir::Right => "right",
        }
    }
}

/// A panel's buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelAction {
    Cancel,
    Insert,
    Send,
    /// The voice panel's Open Settings (a permission was denied).
    Settings,
    /// The image panel's Retry.
    Retry,
    /// The preview panel's Download.
    Confirm,
}

impl PanelAction {
    fn name(self) -> &'static str {
        match self {
            PanelAction::Cancel => "cancel",
            PanelAction::Insert => "insert",
            PanelAction::Send => "send",
            PanelAction::Settings => "settings",
            PanelAction::Retry => "retry",
            PanelAction::Confirm => "confirm",
        }
    }
}

/// What the person asks the host for (`{"ev": …}`).
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    /// The leading button: back (or, on the iPad, the sidebar).
    Back,
    Sidebar,
    /// The transport pill, confirmed on the page's action sheet: switch
    /// SSH ⇄ mosh now.
    Transport,
    /// The page is about to show an action sheet that belongs to the piece
    /// of the bar named so (`"pill"`): on the iPad the host hangs NSH's
    /// popover there (NSH hangs it from the screen's bottom middle).
    DialogAnchor(&'static str),
    /// The pill held: mosh's live counters (the host shows them).
    Diagnostics,
    /// The counters' popover closed (a press outside it).
    DiagnosticsDismiss,
    /// The counters' Repaint screen: the server redraws from blank.
    Repaint,
    /// A breadcrumb segment: its sheet (a popover on the iPad).
    Crumb(Crumb),
    /// ⌘1–⌘9: the n-th window in the picker's order.
    Window(u8),
    /// The follower's veil: take the window's size back.
    TakeOver,
    /// The empty state: create the first session, or install the binary.
    Create,
    Install,
    /// The banner's action (switch to SSH, install) and its ×.
    BannerAction,
    BannerDismiss,
    /// The notice's remedy.
    NoticeAction,
    /// Through a jump host over SSH: mosh from now on, its UDP on `route`
    /// (saved on the connection, which reconnects).
    JumpRoute(crate::connection_form::JumpRoute),
    /// The copied link's chip.
    OpenLink,
    /// A shortcut chip, by its id (a held key repeats).
    Chip(String),
    Arrow(Dir),
    Scroll(Dir),
    /// The keyboard toggle.
    Keyboard,
    /// The mic chip.
    Mic,
    /// The image chip's tap: the photo picker.
    Image,
    /// The image chip's menu: the clipboard's image, a photo, a sent one.
    ImagePaste,
    ImageCamera,
    ImageSent(String),
    /// A button of the voice panel.
    Dictation(PanelAction),
    /// A button of the image panel.
    Attachment(PanelAction),
    /// A button of the preview panel.
    Preview(PanelAction),
    /// The terminal is about to move to a new element: the host keeps it
    /// in the window, so the keyboard stays where it is.
    Handover,
    /// Where a piece of the bar is, in the surface's points: what the host
    /// hangs its popovers from (the breadcrumb's sheets on the iPad, the
    /// pill's confirmation and counters).
    Anchor {
        name: &'static str,
        rect: [f32; 4],
    },
    /// The panes either side of the one shown, by id ([`switcher`]): what
    /// the host's swipe rubber-bands against and switches to. None either
    /// side with nowhere to go (one pane, a cover over it).
    Neighbours {
        previous: Option<String>,
        next: Option<String>,
    },
    /// The one-time swipe hint is showing: the host remembers it on this
    /// device and nudges the terminal toward the pane it would go to
    /// (`forward`: the next one).
    SwipeHint {
        forward: bool,
    },
    /// The pane overview opened (closed): the host puts the keyboard away
    /// while it is up, and back after.
    Overview(bool),
    /// The overview picked pane `id`: the host switches to it.
    SelectPane(String),
    /// The overview asks for the pictures of `panes` (`current`: the pane
    /// shown, the only one a mosh or herdr session has a picture of).
    Pictures {
        panes: Vec<String>,
        current: Option<String>,
    },
    /// Where pane `pane`'s cell is in the overview (the surface's points):
    /// the one shown, which the live terminal shrinks into and grows back
    /// from (`picked` false), or the one just picked, which the pane's
    /// terminal grows from (`picked` true) (B658).
    OverviewAnchor {
        pane: String,
        rect: [f32; 4],
        picked: bool,
    },
    /// Split window `id` (`newPane(inWindow:)`).
    NewPane(String),
    RenameWindow {
        id: String,
        name: String,
    },
    KillPane(String),
    /// Ask the host's text prompt (`prompt::event`'s message).
    Prompt(serde_json::Value),
}

impl Intent {
    /// The event that tells the host.
    pub fn event(&self) -> serde_json::Value {
        let ev = |name: &str| json!({ "ev": name });
        match self {
            Intent::Back => ev("back"),
            Intent::Sidebar => ev("sidebar"),
            Intent::Transport => ev("switch-transport"),
            Intent::DialogAnchor(name) => json!({"ev": "dialog-anchor", "name": name}),
            Intent::Diagnostics => ev("diagnostics"),
            Intent::DiagnosticsDismiss => ev("diagnostics-dismiss"),
            Intent::Repaint => ev("repaint"),
            Intent::Crumb(crumb) => json!({"ev": "crumb", "crumb": match crumb {
                Crumb::Session => "session",
                Crumb::Window => "window",
                Crumb::Pane => "pane",
            }}),
            Intent::Window(number) => json!({"ev": "window", "number": number}),
            Intent::TakeOver => ev("take-over"),
            Intent::Create => ev("create"),
            Intent::Install => ev("install"),
            Intent::BannerAction => ev("banner-action"),
            Intent::BannerDismiss => ev("banner-dismiss"),
            Intent::NoticeAction => ev("notice-action"),
            Intent::JumpRoute(route) => json!({"ev": "jump-route", "route": route.id()}),
            Intent::OpenLink => ev("open-link"),
            Intent::Chip(id) => json!({"ev": "chip", "id": id}),
            Intent::Arrow(dir) => json!({"ev": "arrow", "dir": dir.name()}),
            Intent::Scroll(dir) => json!({"ev": "scroll", "dir": dir.name()}),
            Intent::Keyboard => ev("keyboard"),
            Intent::Mic => ev("mic"),
            Intent::Image => ev("image"),
            Intent::ImagePaste => ev("image-paste"),
            Intent::ImageCamera => ev("image-camera"),
            Intent::ImageSent(id) => json!({"ev": "image-sent", "id": id}),
            Intent::Dictation(action) => json!({"ev": "dictation", "action": action.name()}),
            Intent::Attachment(action) => json!({"ev": "attachment", "action": action.name()}),
            Intent::Preview(action) => json!({"ev": "preview", "action": action.name()}),
            Intent::Handover => ev("handover"),
            Intent::Anchor { name, rect } => json!({"ev": "anchor", "name": name, "rect": rect}),
            Intent::Neighbours { previous, next } => {
                json!({"ev": "neighbours", "previous": previous, "next": next})
            }
            Intent::SwipeHint { forward } => json!({"ev": "swipe-hint", "forward": forward}),
            Intent::Overview(open) => json!({"ev": "overview", "open": open}),
            Intent::SelectPane(id) => json!({"ev": "select-pane", "id": id}),
            Intent::Pictures { panes, current } => {
                json!({"ev": "pictures", "panes": panes, "current": current})
            }
            Intent::OverviewAnchor { pane, rect, picked } => {
                json!({"ev": "overview-anchor", "pane": pane, "rect": rect, "picked": picked})
            }
            Intent::NewPane(window) => json!({"ev": "new-pane", "window": window}),
            Intent::RenameWindow { id, name } => {
                json!({"ev": "rename-window", "id": id, "name": name})
            }
            Intent::KillPane(id) => json!({"ev": "kill-pane", "id": id}),
            Intent::Prompt(event) => event.clone(),
        }
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// What tells the host.
pub(crate) type Tell = Rc<dyn Fn(Intent)>;

/// What the page's parts share: the host's state, and the host.
#[derive(Clone)]
pub(crate) struct Ctx {
    pub tell: Tell,
    pub chrome: Signal<Chrome>,
    pub bar: Signal<Bar>,
    pub dictation: Signal<Option<Dictation>>,
    pub level: Signal<f32>,
    pub attachment: Signal<Option<Attachment>>,
    pub preview: Signal<Option<Preview>>,
    /// Bumped by a rekey: the terminal's element is made again.
    pub generation: Signal<u32>,
    /// mosh's counters, while the host shows them.
    pub diagnostics: Signal<Diagnostics>,
    /// The transport pill: what its popover hangs from, and where it is.
    pub pill: ElementRef,
    pub pill_rect: Signal<Option<Bounds<Points>>>,
    /// The session's panes a sideways swipe walks.
    pub switcher: Signal<Switcher>,
    /// A sideways swipe is under way (the host's).
    pub swiping: Signal<bool>,
    /// The pane overview is open.
    pub overview: Signal<bool>,
    /// The overview's pane pictures, by pane id, while it is open.
    pub pictures: Signal<HashMap<String, Vec<String>>>,
}

impl Ctx {
    /// Whether a protocol switch is under way: nothing moves
    /// ([`Chrome::still`]). Read untracked by what decides a motion as it
    /// starts.
    pub(crate) fn still(&self) -> bool {
        self.chrome.with(|c| c.still)
    }

    /// Tells the host `intent`.
    pub(crate) fn ask(&self, intent: Intent) {
        (self.tell)(intent);
    }

    /// What `intent` asks, as a press handler.
    pub(crate) fn asking(&self, intent: Intent) -> impl Fn() + 'static {
        let tell = self.tell.clone();
        move || tell(intent.clone())
    }

    /// iPad landscape's trimmed chrome.
    pub(crate) fn compact(&self) -> bool {
        self.chrome.with(|c| c.compact)
    }
}

/// The largest reading size the shell's text follows: extra large (the
/// SwiftUI screen's `.dynamicTypeSize(...DynamicTypeSize.xLarge)`).
pub(crate) const X_LARGE: usize = 4;

/// The size `Face` draws design size `size` at on this screen: its text
/// style's at the user's reading size, no larger than at extra large; a
/// size below the smallest style stays as it is.
pub(crate) fn size(size: f32) -> f32 {
    face::style_for(size).map_or(size, |style| style_size(style, font_scale()))
}

fn style_size(style: TextStyle, scale: f32) -> f32 {
    face::point_size_clamped(style, scale, 0, X_LARGE)
}

/// `text` in `Face`'s `design` at design size `points` and `weight`,
/// capped at extra large ([`size`]).
pub(crate) fn styled(text: Text, design: face::Design, points: f32, weight: FontWeight) -> Text {
    let text = text
        .text_size(move || px(size(points)))
        .line_height(face::line_height())
        .font_weight(weight)
        .allow_font_scaling(false);
    match design {
        face::Design::Default => text,
        face::Design::Monospaced => text.font_family("monospace"),
        face::Design::Rounded => text.font_family("ui-rounded"),
    }
}

/// [`styled`] on one line at SwiftUI's height for its size: the line at
/// the bottom of its frame, its top on the device pixel at or below where
/// it falls (SwiftUI rounds it down the screen; the pill's and the crumbs'
/// words sat half a point high at Large, where it falls on 0.75 of a
/// pixel).
pub(crate) fn line(
    content: impl IntoReactive<SharedString>,
    design: face::Design,
    points: f32,
    weight: FontWeight,
) -> Text {
    let frame = move || face::frame(size(points), pixel_ratio());
    let slack = move || {
        let ratio = pixel_ratio().max(1.0);
        ((frame() - face::line(size(points))) * ratio - 1e-3)
            .ceil()
            .max(0.0)
            / ratio
    };
    styled(text(content), design, points, weight)
        .number_of_lines(1u32)
        .h(move || px(frame()))
        .pt(move || px(slack()))
}

/// An SF Symbol at a fixed size (`.font(.system(size:weight:))`), in
/// `color` (a theme's color is read when the symbol is made: callers in a
/// `dynamic` follow a theme change).
pub(crate) fn symbol(name: &str, points: f32, weight: &'static str, color: Rgba) -> NativeView {
    native("moshpit/symbol")
        .flex_shrink_0()
        .prop("name", json!(name))
        .prop("pointSize", json!(points))
        .prop("weight", json!(weight))
        .prop("color", json!(theme::hex_string(color)))
}

/// The terminal page's tokens (B496): its ground (`terminal-bg`) the
/// grid's `ground` when the host says one, else the terminal theme's
/// background (B488), else the app theme's.
pub fn ground_tokens(
    params: &crate::screen::Params,
    ground: Option<&str>,
) -> nsh_core::element::Tokens {
    let colour = ground
        .or(params.terminal.as_deref())
        .and_then(crate::theme::parse_hex);
    crate::theme::tokens_on(&params.theme, colour)
}

/// The terminal screen.
#[component]
pub fn TerminalScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    // What the page ran with: the app theme and the terminal theme's
    // background, for the ground's tokens (B496).
    let params = channel.as_ref().map(|c| c.params()).unwrap_or_default();
    let keys = shortcuts::Shortcuts::default();
    let ctx = Ctx {
        tell: tell_host(channel.clone()),
        chrome: signal(Chrome::default()),
        bar: signal(Bar::default()),
        dictation: signal(None),
        level: signal(0.0),
        attachment: signal(None),
        preview: signal(None),
        generation: signal(0),
        diagnostics: signal(Diagnostics::default()),
        pill: element_ref(),
        pill_rect: signal(None),
        switcher: signal(Switcher::default()),
        swiping: signal(false),
        overview: signal(false),
        pictures: signal(HashMap::new()),
    };
    let prompts = prompt::Prompts::default();
    shortcuts::bind(&ctx, &keys);
    let order = switcher::order(&ctx);
    switcher::tell_neighbours(&ctx, order);
    let panes = overview::model(&ctx);
    overview::tell_open(&ctx);
    overview::ask_pictures(&ctx, panes);
    {
        let ctx = ctx.clone();
        let prompts = prompts.clone();
        channel::hear(
            channel.as_ref(),
            Screen::Terminal,
            Message::parse,
            move |message| match message {
                Message::Chrome(next) => {
                    // The grid's ground changed (herdr painting its own):
                    // the page's tokens follow, no re-run (B496).
                    if ctx.chrome.with_untracked(|c| c.ground != next.ground) {
                        set_tokens(ground_tokens(&params, next.ground.as_deref()));
                    }
                    if ctx.chrome.with_untracked(|c| *c != *next) {
                        ctx.chrome.set(*next);
                    }
                }
                Message::Bar(next) => {
                    if ctx.bar.with_untracked(|b| *b != *next) {
                        ctx.bar.set(*next);
                    }
                }
                Message::Dictation(next) => ctx.dictation.set(next.map(|d| *d)),
                Message::Level(level) => ctx.level.set(level),
                Message::Attachment(next) => ctx.attachment.set(next.map(|a| *a)),
                Message::Preview(next) => ctx.preview.set(next.map(|p| *p)),
                Message::Diagnostics(next) => {
                    if ctx.diagnostics.with_untracked(|d| *d != *next) {
                        ctx.diagnostics.set(*next);
                    }
                }
                Message::Key(stroke) => keys.run(&ctx, &stroke, shortcuts::Via::Host),
                Message::Switcher(next) => {
                    if ctx.switcher.with_untracked(|s| *s != *next) {
                        ctx.switcher.set(*next);
                    }
                }
                Message::Swipe(active) => {
                    if ctx.swiping.get_untracked() != active {
                        ctx.swiping.set(active);
                    }
                }
                Message::Overview(open) => {
                    // Only with a live tree to show (⌘K's rule).
                    let live = ctx.switcher.with_untracked(|s| s.tree.is_some());
                    ctx.overview.set(open && live);
                }
                Message::OverviewAct { action, pane } => {
                    overview::hear_act(&ctx, &prompts, panes, &action, &pane)
                }
                Message::Prompted(answer) => prompts.answer(answer),
                Message::Pictures(pictures) => {
                    // Only while it is open (one that comes after a close
                    // is dropped with the rest).
                    if ctx.overview.get_untracked() {
                        ctx.pictures.update(|p| p.extend(pictures));
                    }
                }
                Message::Rekey => {
                    // The host hears this before the batch that takes the
                    // old element out of the window: it moves the terminal
                    // out first, so UIKit never resigns it.
                    ctx.ask(Intent::Handover);
                    ctx.generation.update(|g| *g += 1);
                }
            },
        );
    }
    view()
        .flex_1()
        .relative()
        // The terminal's ground in the appearance the screen draws in (light
        // on a light terminal theme): what shows around the panels.
        .bg(move || crate::ui::tokens::token("terminal-bg"))
        .test_id("terminal-screen")
        .child(
            view()
                .flex_1()
                .flex_col()
                // The keyboard's height: the column ends where the keyboard
                // starts, moving with it in the commit that hears it.
                .mb_keyboard()
                .child(bar::top_bar(ctx.clone()))
                .child(cover::pane_area(ctx.clone(), order))
                .child(panels::panels(ctx.clone()))
                .child(keys::shortcut_bar(ctx.clone())),
        )
        .child(chrome::over_pane(ctx.clone(), order))
        .child(diagnostics::popover(ctx.clone()))
        // Over everything while it is open.
        .child(overview::overview(ctx, panes, prompts))
}

/// What tells the host `intent`, if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |intent: Intent| {
        if let Some(channel) = &channel {
            channel.tell(&intent.event());
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_parse_and_bad_ones_are_refused() {
        assert!(matches!(
            Message::parse(r#"{"op":"chrome","name":"studio"}"#),
            Ok(Message::Chrome(_))
        ));
        assert!(matches!(
            Message::parse(r#"{"op":"bar","keyboardDown":true}"#),
            Ok(Message::Bar(b)) if b.keyboard_down
        ));
        assert_eq!(
            Message::parse(r#"{"op":"dictation","panel":null}"#),
            Ok(Message::Dictation(None))
        );
        assert_eq!(
            Message::parse(r#"{"op":"level","level":3}"#),
            Ok(Message::Level(1.0)),
            "a level is clamped"
        );
        assert_eq!(Message::parse(r#"{"op":"rekey"}"#), Ok(Message::Rekey));
        assert_eq!(
            Message::parse(r#"{"op":"key","stroke":"mod+k"}"#),
            Ok(Message::Key("mod+k".into()))
        );
        assert!(Message::parse(r#"{"op":"key"}"#).is_err());
        assert!(matches!(
            Message::parse(r#"{"op":"diagnostics","shown":true,"counters":null}"#),
            Ok(Message::Diagnostics(d)) if d.shown && d.counters.is_none()
        ));
        assert!(Message::parse(r#"{"op":"rekey","now":1}"#).is_err());
        assert!(Message::parse(r#"{"op":"focus"}"#).is_err());
        assert!(Message::parse(r#"{"op":"chrome","colour":1}"#).is_err());
        assert!(Message::parse("[]").is_err());
    }

    #[test]
    fn intents_name_what_they_ask() {
        assert_eq!(Intent::Back.event(), json!({"ev": "back"}));
        assert_eq!(Intent::Transport.event(), json!({"ev": "switch-transport"}));
        assert_eq!(
            Intent::DialogAnchor("pill").event(),
            json!({"ev": "dialog-anchor", "name": "pill"})
        );
        assert_eq!(
            Intent::Crumb(Crumb::Pane).event(),
            json!({"ev": "crumb", "crumb": "pane"})
        );
        assert_eq!(
            Intent::Chip("esc".into()).event(),
            json!({"ev": "chip", "id": "esc"})
        );
        assert_eq!(
            Intent::Arrow(Dir::Left).event(),
            json!({"ev": "arrow", "dir": "left"})
        );
        assert_eq!(
            Intent::Dictation(PanelAction::Send).event(),
            json!({"ev": "dictation", "action": "send"})
        );
    }

    #[test]
    fn the_shell_stops_growing_at_extra_large() {
        // xxxLarge: caption2 would be 17; the shell keeps extra large's 13.
        let scale = 23.0 / 17.0;
        assert_eq!(style_size(TextStyle::Caption2, scale), 13.0);
        assert_eq!(style_size(TextStyle::Caption2, 21.0 / 17.0), 13.0);
        assert_eq!(style_size(TextStyle::Caption2, 19.0 / 17.0), 13.0);
        assert_eq!(style_size(TextStyle::Caption2, 1.0), 11.0);
        assert_eq!(style_size(TextStyle::Caption2, 14.0 / 17.0), 11.0);
        assert_eq!(style_size(TextStyle::Body, scale), 19.0);
    }
}
