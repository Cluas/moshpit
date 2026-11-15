//! The multiplexer's pickers (`Moshpit/UI/Tmux/TmuxSheets.swift`:
//! `WindowsSheet`, `SessionsSheet`, `SelectPaneSheet`, in `CompactSheet`)
//! in NSH: the breadcrumb's segments open them, ⌘K the windows. One screen
//! for the three lists, in the host's container (a sheet at medium and
//! large heights on a phone, a 400 × 380 popover hung from the breadcrumb
//! on a regular-width iPad, as `AdaptivePresentation` presents them; NSH's
//! modals have neither detents nor popovers).
//!
//! The host owns the multiplexer: it posts the list as the controller has
//! it ([`Message`]), `{"op": "picker", "kind": "windows" | "sessions" |
//! "panes", "mux": "tmux" | "herdr", "rows": [...], "active": id}`, again
//! whenever that changes; the page says each row in the multiplexer's own
//! words ([`Vocab`]) and tells the host what the person picks:
//!
//! - `{"ev": "select", "id"}`: a row (the host switches and closes).
//! - `{"ev": "new", "name": … | null}`: the bar's ＋. A window or a session
//!   asks for its name first (blank lets the program name it); a pane is
//!   split at once (the list stays up).
//! - `{"ev": "rename", "id", "name"}`, `{"ev": "kill", "id"}`: a row's
//!   long-press menu, which is the host's (`NSHContextMenus`: NSH draws no
//!   menu of its own) on the row's `mux-picker-row-<id>`; its pick comes
//!   back as `{"op": "act", "action": "rename" | "kill", "id"}`, and the
//!   page asks for the new name (the host's text prompt, [`super::prompt`])
//!   or confirms the kill (NSH's action sheet) before telling the host.
//!
//! The list opens on the active row (centered once, as `ScrollViewReader`
//! does; a later refresh never moves it).
//!
//! The page is `docs/nsh/UI-DESIGN.md` §6.3's pickers on the `ui` layer
//! (R13): one section of rows, each the level's icon (the accent's on the
//! active row), the name over what runs in it (SF Mono), the agents' state
//! in words in its color (§2.2: never color alone), the active row ticked
//! and selected; the hint and the multiplexer's keys under it.

use std::{rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    folder::symbol,
    model::{Mux, Signal as AgentSignal},
    prompt::{Answer, Prompt, Prompts},
};
use crate::{
    channel::{self, Channel},
    home::model::display_command,
    nav_bar::{self, BarMetrics},
    screen::Screen,
    ui::{
        alert::{AlertDialogOptions, alert_dialog},
        icon::{Tone, icon},
        page::page,
        row::{PADDING_X, row as list_row},
        section::section,
        tokens::{number, token},
        typography::TextToken,
    },
};

/// Which list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Windows,
    Sessions,
    Panes,
}

/// One row, in the host's facts (the page words them).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Row {
    /// What the host knows it by (`@3`, `$1`, `%7`, herdr's `w4`).
    pub id: String,
    /// A window's or a pane's number.
    pub index: Option<u32>,
    /// A window's or a session's name.
    pub name: String,
    /// A window's panes.
    pub panes: Option<u32>,
    /// What runs in it (a window's active pane's command, a pane's).
    pub command: String,
    /// The agent in a pane (its hooks' name); a window's and the active
    /// session's: its active pane's.
    pub agent: Option<String>,
    /// Where a window's active pane works (the active session's: its active
    /// window's): what names a window tmux named after its program.
    pub path: Option<String>,
    /// The agents' signal (a window's: its panes' together).
    pub signal: Option<AgentSignal>,
    /// A session another client is attached to.
    pub attached: bool,
    /// The active session's windows, and its active window
    /// (`index:name`).
    pub windows: Option<u32>,
    pub window: Option<String>,
}

/// `{"op": "picker", …}`: the list as the controller has it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Picker {
    pub kind: Kind,
    pub mux: Mux,
    /// In the order the list shows them.
    pub rows: Vec<Row>,
    /// The row the multiplexer is on.
    pub active: Option<String>,
}

/// What a row's menu picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Act {
    Rename,
    Kill,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActFields {
    action: Act,
    id: String,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "picker", …}`.
    Picker(Box<Picker>),
    /// `{"op": "act", "action", "id"}`: a row's menu was picked.
    Act { action: Act, id: String },
    /// `{"op": "prompted", "id", "text"}`: the host's prompt answered.
    Prompted(Answer),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("mux-picker", json)?;
        let fields = serde_json::Value::Object(fields);
        let error = |e: serde_json::Error| format!("mux-picker: {op}: {e}");
        match op.as_str() {
            "picker" => serde_json::from_value(fields)
                .map(|p| Message::Picker(Box::new(p)))
                .map_err(error),
            "act" => serde_json::from_value::<ActFields>(fields)
                .map(|a| Message::Act {
                    action: a.action,
                    id: a.id,
                })
                .map_err(error),
            "prompted" => serde_json::from_value(fields)
                .map(Message::Prompted)
                .map_err(error),
            other => Err(format!("mux-picker: no op {other:?}")),
        }
    }
}

/// A multiplexer's words for its levels and its keys
/// (`MultiplexerVocabulary`): tmux's sessions and windows, herdr's
/// workspaces and tabs.
#[derive(Debug, Clone, PartialEq)]
pub struct Vocab {
    pub session: String,
    pub sessions: String,
    pub session_lower: String,
    pub window: String,
    pub windows: String,
    pub window_lower: String,
    pub windows_lower: String,
    /// What destroys one: tmux kills, herdr closes.
    pub kill: String,
    pub session_keys: [&'static str; 2],
    pub window_keys: [&'static str; 2],
    pub pane_keys: [&'static str; 2],
    pub session_icon: &'static str,
    pub window_icon: &'static str,
}

impl Vocab {
    /// `mux`'s words, in the app's language.
    pub fn of(mux: Mux) -> Vocab {
        match mux {
            Mux::Herdr => Vocab {
                session: t!("Workspace"),
                sessions: t!("Workspaces"),
                session_lower: t!("workspace"),
                window: t!("Tab"),
                windows: t!("Tabs"),
                window_lower: t!("tab"),
                windows_lower: t!("tabs"),
                kill: t!("Close"),
                session_keys: ["⌃", "b w"],
                window_keys: ["⌃", "b n/p"],
                pane_keys: ["⌃", "b v"],
                session_icon: "rectangle.grid.2x2",
                window_icon: "square.on.square",
            },
            // Plain shells never show these; tmux's words are a harmless
            // default.
            Mux::Tmux | Mux::None => Vocab {
                session: t!("Session"),
                sessions: t!("Sessions"),
                session_lower: t!("session"),
                window: t!("Window"),
                windows: t!("Windows"),
                window_lower: t!("window"),
                windows_lower: t!("windows"),
                kill: t!("Kill"),
                session_keys: ["⌃", "b s"],
                window_keys: ["⌃", "1-9"],
                pane_keys: ["⌃", "b q"],
                session_icon: "square.stack.3d.up",
                window_icon: "macwindow",
            },
        }
    }
}

/// What a window row is called: its number and what Home calls the window
/// (UI-DESIGN §6.1): its own name, else its pane's folder, else the agent
/// or the program running in it; the multiplexer's word and its number
/// when nothing says more (herdr names tabs after their number). Never
/// tmux's automatic name: Claude Code's binary is its version, so a window
/// it runs in said "2: 2.1.322".
pub fn window_title(
    index: u32,
    name: &str,
    command: &str,
    path: Option<&str>,
    agent: Option<&str>,
    vocab: &Vocab,
) -> String {
    let name = if name == index.to_string() { "" } else { name };
    let called = super::model::project_name(name, command, path)
        .or_else(|| agent.filter(|a| !a.is_empty()).map(str::to_owned))
        .or_else(|| (!command.is_empty()).then(|| display_command(command)));
    match called {
        Some(called) => format!("{index}: {called}"),
        None => format!("{} {index}", vocab.window),
    }
}

/// `row`'s title when it is a window.
fn window_row_title(row: &Row, vocab: &Vocab) -> String {
    window_title(
        row.index.unwrap_or(0),
        &row.name,
        &row.command,
        row.path.as_deref(),
        row.agent.as_deref(),
        vocab,
    )
}

/// A session's name, with its id after it when another session shares
/// the name (herdr names workspaces after their folder: four at `~`).
pub fn session_title(row: &Row, rows: &[Row]) -> String {
    let twins = rows.iter().filter(|r| r.name == row.name).count();
    if twins > 1 {
        format!("{} · {}", row.name, row.id)
    } else {
        row.name.clone()
    }
}

/// The active session's window as its row's meta names it: the host says
/// `index:name`, and the active pane's program, folder and agent, which
/// name it the way a window row is named.
fn active_window_title(row: &Row, window: &str, vocab: &Vocab) -> String {
    let Some((index, name)) = window
        .split_once(':')
        .and_then(|(i, n)| i.trim().parse::<u32>().ok().map(|i| (i, n)))
    else {
        return window.to_owned();
    };
    window_title(
        index,
        name,
        &row.command,
        row.path.as_deref(),
        row.agent.as_deref(),
        vocab,
    )
}

/// What a signal means, in words (`AgentSignal.label`).
pub fn signal_label(signal: AgentSignal) -> String {
    match signal {
        AgentSignal::Attention => t!("needs you"),
        AgentSignal::Working => t!("working"),
        AgentSignal::Done => t!("done"),
    }
}

/// A row as the list shows it: its icon, its title and its meta line.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub icon: String,
    pub title: String,
    pub meta: String,
}

/// Row `row` of `picker`, in words.
pub fn shown(row: &Row, picker: &Picker, vocab: &Vocab) -> Shown {
    match picker.kind {
        Kind::Windows => {
            let panes = crate::plural!("%lld panes", i64::from(row.panes.unwrap_or(0)));
            Shown {
                icon: vocab.window_icon.to_owned(),
                title: window_row_title(row, vocab),
                meta: if row.command.is_empty() {
                    panes
                } else {
                    format!("{panes} · {}", display_command(&row.command))
                },
            }
        }
        Kind::Sessions => {
            let active = picker.active.as_deref() == Some(row.id.as_str());
            let meta = if active {
                // One window is one window (SwiftUI says "1 windows").
                let count = row.windows.unwrap_or(0);
                let noun = if count == 1 {
                    &vocab.window_lower
                } else {
                    &vocab.windows_lower
                };
                let head = format!("{count} {noun}");
                match &row.window {
                    Some(window) => format!("{head} · {}", active_window_title(row, window, vocab)),
                    None => head,
                }
            } else if row.attached {
                t!("attached")
            } else {
                t!("detached")
            };
            Shown {
                icon: vocab.session_icon.to_owned(),
                title: session_title(row, &picker.rows),
                meta,
            }
        }
        Kind::Panes => {
            let index = row.index.unwrap_or(0);
            // The agent's name first: the process is often a bare version
            // number ("2.1.227"), which goes to the meta line.
            let title = match &row.agent {
                Some(agent) => agent.clone(),
                None if row.command.is_empty() => {
                    t!("pane %lld", arg1 = index.to_string())
                }
                None => row.command.clone(),
            };
            Shown {
                icon: if index <= 50 {
                    format!("{index}.square")
                } else {
                    "number.square".to_owned()
                },
                title,
                meta: if row.agent.is_some() {
                    row.command.clone()
                } else {
                    String::new()
                },
            }
        }
    }
}

/// What the list's title says.
pub fn title(kind: Kind, vocab: &Vocab) -> String {
    match kind {
        Kind::Windows => vocab.windows.clone(),
        Kind::Sessions => vocab.sessions.clone(),
        Kind::Panes => t!("Select Pane"),
    }
}

/// The footer's hint and its keys.
pub fn footer(kind: Kind, vocab: &Vocab) -> (String, [&'static str; 2]) {
    match kind {
        // The terminal's sideways swipe walks the session's panes, window
        // by window, herdr's tabs too; it never leaves the session.
        Kind::Windows | Kind::Sessions => (
            if kind == Kind::Windows {
                t!("Swipe to switch")
            } else {
                t!("Tap to switch")
            },
            if kind == Kind::Windows {
                vocab.window_keys
            } else {
                vocab.session_keys
            },
        ),
        Kind::Panes => (
            t!("Tap to focus · %@", arg1 = t!("＋ splits a new pane")),
            vocab.pane_keys,
        ),
    }
}

/// What screen readers say for a row: its words, what its dot means, and
/// whether it is the active one.
pub fn row_label(shown: &Shown, signal: Option<AgentSignal>, active: bool) -> String {
    let mut parts = vec![shown.title.clone(), shown.meta.clone()];
    if let Some(signal) = signal {
        parts.push(signal_label(signal));
    }
    if active {
        parts.push(t!("active"));
    }
    parts.retain(|p| !p.is_empty());
    parts.join(", ")
}

/// The kill's question and its button (`confirmationDialog`).
pub fn kill_question(kind: Kind, row: &Row, picker: &Picker, vocab: &Vocab) -> AlertDialogOptions {
    let (title, button) = match kind {
        Kind::Sessions => (
            t!(
                "%@ %@ %@? Everything in it dies.",
                arg1 = vocab.kill,
                arg2 = vocab.session_lower,
                arg3 = session_title(row, &picker.rows)
            ),
            t!("%@ %@", arg1 = vocab.kill, arg2 = vocab.session),
        ),
        _ => (
            t!(
                "%@ %@ %@? Every pane in it dies.",
                arg1 = vocab.kill,
                arg2 = vocab.window_lower,
                arg3 = window_row_title(row, vocab)
            ),
            t!("%@ %@", arg1 = vocab.kill, arg2 = vocab.window),
        ),
    };
    alert_dialog()
        .title(title)
        .action_sheet()
        .destructive("kill", button)
        .cancel("cancel", t!("Cancel"))
}

/// The ＋'s question: the new window's or session's name (blank: the
/// program names it).
pub fn new_prompt(kind: Kind, vocab: &Vocab) -> Prompt {
    let noun = if kind == Kind::Sessions {
        &vocab.session
    } else {
        &vocab.window
    };
    Prompt {
        title: t!("New %@", arg1 = noun),
        message: (kind == Kind::Windows).then(|| t!("Leave blank to let the program name it.")),
        placeholder: t!("Name (optional)"),
        text: String::new(),
        confirm: t!("Create"),
        cancel: t!("Cancel"),
    }
}

/// Rename's question, the field holding the name it has.
pub fn rename_prompt(kind: Kind, row: &Row, vocab: &Vocab) -> Prompt {
    let noun = if kind == Kind::Sessions {
        &vocab.session
    } else {
        &vocab.window
    };
    Prompt {
        title: t!("Rename %@", arg1 = noun),
        message: None,
        placeholder: t!("Name"),
        text: row.name.clone(),
        confirm: t!("Rename"),
        cancel: t!("Cancel"),
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

type Tell = Rc<dyn Fn(serde_json::Value)>;

/// The picker.
#[component]
pub fn MuxPickerScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let picker = signal(Picker::default());
    let ready = signal(false);
    let tell = tell_host(channel.clone());
    let prompts = Prompts::default();
    {
        let (tell, prompts) = (tell.clone(), prompts.clone());
        channel::hear(
            channel.as_ref(),
            Screen::MuxPicker,
            Message::parse,
            move |message| match message {
                Message::Picker(next) => {
                    if picker.with_untracked(|p| *p != *next) {
                        picker.set(*next);
                    }
                    ready.set(true);
                }
                Message::Act { action, id } => act(action, &id, picker, &tell, &prompts),
                Message::Prompted(answer) => prompts.answer(answer),
            },
        );
    }
    let scrolled = nav_bar::ScrollEdge::new();
    let kind = computed(move || picker.with(|p| p.kind));
    let mux = computed(move || picker.with(|p| p.mux));
    let list = element_ref();
    // The list opens on the active row once per presentation.
    let revealed = stored(false);
    let body = {
        let tell = tell.clone();
        move || {
            let tell = tell.clone();
            let rows = dynamic(move || {
                let now = picker.get();
                let tell = tell.clone();
                untracked(move || rows_section(&now, list, revealed, tell))
            });
            let foot = dynamic(move || {
                let (kind, mux) = (kind.get(), mux.get());
                untracked(move || footer_view(kind, mux))
            });
            page(
                bar,
                scrolled,
                "mux-picker-list",
                [view()
                    .w_full()
                    .gap(px(8.0))
                    .child(rows)
                    .child(foot)
                    .into_element()],
            )
            .element_ref(&list)
            .into_element()
        }
    };
    let new = {
        let (tell, prompts) = (tell.clone(), prompts.clone());
        move || {
            let (kind, mux) = picker.with_untracked(|p| (p.kind, p.mux));
            match kind {
                Kind::Panes => tell(json!({"ev": "new"})),
                Kind::Windows | Kind::Sessions => {
                    let answer_to = tell.clone();
                    prompts.ask(&*tell, new_prompt(kind, &Vocab::of(mux)), move |text| {
                        let Some(text) = text else { return };
                        let name = text.trim();
                        answer_to(json!({
                            "ev": "new",
                            "name": (!name.is_empty()).then_some(name),
                        }));
                    });
                }
            }
        }
    };
    view()
        .flex_1()
        .relative()
        // The host's sheet is the background: Liquid Glass on iOS 26, the
        // app's sheet fill below.
        .test_id("mux-picker")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(nav_bar::root_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: String::new(),
            title: Box::new(move || title(kind.get(), &Vocab::of(mux.get()))),
            leading: None,
            trailing: vec![nav_bar::BarButton {
                label: t!("New"),
                content: nav_bar::BarContent::Symbol("plus"),
                enabled: computed(|| true),
                emphasized: false,
                test_id: "mux-picker-new",
                on_press: Box::new(new),
                menu: None,
            }],
            on_back: Box::new(|| {}),
            scrolled,
        }))
}

/// A row's menu picked `action` on row `id`: ask, then tell the host.
fn act(action: Act, id: &str, picker: Signal<Picker>, tell: &Tell, prompts: &Prompts) {
    let Some((kind, row, snapshot)) = picker.with_untracked(|p| {
        p.rows
            .iter()
            .find(|r| r.id == id)
            .map(|r| (p.kind, r.clone(), p.clone()))
    }) else {
        return;
    };
    // Panes have no name and are killed from Home's menus.
    if kind == Kind::Panes {
        return;
    }
    let vocab = Vocab::of(snapshot.mux);
    match action {
        Act::Rename => {
            let answer_to = tell.clone();
            let before = row.name.clone();
            let id = row.id.clone();
            prompts.ask(&**tell, rename_prompt(kind, &row, &vocab), move |text| {
                let Some(text) = text else { return };
                // A name cleared by mistake, or left as it was, renames
                // nothing (tmux would take the empty name).
                let name = text.trim();
                if name.is_empty() || name == before {
                    return;
                }
                answer_to(json!({"ev": "rename", "id": id, "name": name}));
            });
        }
        Act::Kill => {
            let answer = kill_question(kind, &row, &snapshot, &vocab).show();
            let tell = tell.clone();
            let id = row.id.clone();
            drop(spawn(async move {
                if answer
                    .await
                    .is_ok_and(|r| r.action.as_deref() == Some("kill"))
                {
                    tell(json!({"ev": "kill", "id": id}));
                }
            }));
        }
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

/// The tone the agents' state is said in (§2.2).
fn signal_tone(signal: AgentSignal) -> Tone {
    match signal {
        AgentSignal::Attention => Tone::Warning,
        AgentSignal::Working => Tone::Success,
        AgentSignal::Done => Tone::Info,
    }
}

/// The picker's rows on one card; the active one opens in view.
fn rows_section(
    picker: &Picker,
    list: ElementRef,
    revealed: Stored<bool>,
    tell: Tell,
) -> AnyElement {
    let vocab = Vocab::of(picker.mux);
    let mut card = section().test_id("mux-picker-rows");
    for row in &picker.rows {
        let active = picker.active.as_deref() == Some(row.id.as_str());
        let shown = shown(row, picker, &vocab);
        let id = row.id.clone();
        let tell = tell.clone();
        let mut item = list_row(shown.title.clone())
            .leading(icon(symbol(&shown.icon)).tone(if active {
                Tone::Primary
            } else {
                Tone::Muted
            }))
            .check(move || active)
            .on_press({
                let id = id.clone();
                move || tell(json!({"ev": "select", "id": id}))
            })
            .test_id(&format!("mux-picker-row-{id}"));
        if !shown.meta.is_empty() {
            item = item.subtitle(shown.meta.clone()).mono_subtitle();
        }
        if let Some(signal) = row.signal {
            item = item
                .value(signal_label(signal))
                .value_tone(signal_tone(signal));
        }
        let inset = item.inset();
        let target = element_ref();
        // Opened on the active row: once, centered, as soon as it is laid
        // out.
        let reveal = move || {
            if !active || revealed.with(|r| *r) {
                return;
            }
            revealed.replace(true);
            drop(spawn(async move {
                let (Some(row), Some(scroll)) = (target.measure().await, list.measure().await)
                else {
                    return;
                };
                let room = scroll.frame.size.height.0;
                let top = row.page.y.0 - scroll.page.y.0;
                let bottom = top + row.frame.size.height.0;
                // Already in view: leave it.
                if top >= 0.0 && bottom <= room {
                    return;
                }
                let y = (top - (room - row.frame.size.height.0) / 2.0).max(0.0);
                list.scroll_to(point(px(0.0), px(y)), false);
            }));
        };
        card = card.item(
            view()
                .w_full()
                .element_ref(&target)
                .on_layout(move |_: &Bounds<Points>| reveal())
                .child(item),
            inset,
        );
    }
    card.into_element()
}

/// Under the card: the hint, and the multiplexer's keys at the trailing
/// edge.
fn footer_view(kind: Kind, mux: Mux) -> AnyElement {
    let vocab = Vocab::of(mux);
    let (hint, keys) = footer(kind, &vocab);
    view()
        .flex_row()
        .items_start()
        .gap(px(12.0))
        .px(px(PADDING_X))
        .test_id("mux-picker-footer")
        .child(
            TextToken::Footnote
                .apply(text(hint))
                .text_color(move || token("muted-foreground"))
                .flex_1()
                .flex_shrink(1.0),
        )
        .child(
            view()
                .flex_row()
                .gap(px(4.0))
                .flex_shrink_0()
                .children(keys.into_iter().map(key_chip)),
        )
        .into_element()
}

/// A key the hint names: SF Mono on `secondary`, `radius-xs`.
fn key_chip(label: &'static str) -> AnyElement {
    view()
        .px(px(6.0))
        .py(px(2.0))
        .rounded(px(number("radius-xs")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || token("secondary"))
        .child(
            TextToken::MonoCaption
                .apply(text(label))
                .number_of_lines(1u32)
                .text_color(move || token("secondary-foreground")),
        )
        .into_element()
}

#[cfg(test)]
mod tests {
    //! What needs no engine; the words in the app's languages, and the page
    //! itself, are `ui/tests/mux_picker.rs`'s.
    use super::*;

    /// tmux's or herdr's words in English, without an engine.
    fn english(mux: Mux) -> Vocab {
        let herdr = mux == Mux::Herdr;
        let w = |tmux: &str, herdr_word: &str| (if herdr { herdr_word } else { tmux }).to_owned();
        Vocab {
            session: w("Session", "Workspace"),
            sessions: w("Sessions", "Workspaces"),
            session_lower: w("session", "workspace"),
            window: w("Window", "Tab"),
            windows: w("Windows", "Tabs"),
            window_lower: w("window", "tab"),
            windows_lower: w("windows", "tabs"),
            kill: w("Kill", "Close"),
            session_keys: ["⌃", "b s"],
            window_keys: ["⌃", "1-9"],
            pane_keys: ["⌃", "b q"],
            session_icon: if herdr {
                "rectangle.grid.2x2"
            } else {
                "square.stack.3d.up"
            },
            window_icon: if herdr {
                "square.on.square"
            } else {
                "macwindow"
            },
        }
    }

    #[test]
    fn messages_parse_and_bad_ones_are_refused() {
        let message = Message::parse(
            r#"{"op":"picker","kind":"windows","mux":"tmux","active":"@1",
                "rows":[{"id":"@1","index":1,"name":"agents","panes":2,"command":"zsh","signal":"working"}]}"#,
        )
        .unwrap();
        let Message::Picker(p) = message else {
            panic!("{message:?}")
        };
        assert_eq!(p.rows[0].panes, Some(2));
        assert_eq!(p.rows[0].signal, Some(AgentSignal::Working));
        assert_eq!(
            Message::parse(r#"{"op":"act","action":"kill","id":"@2"}"#),
            Ok(Message::Act {
                action: Act::Kill,
                id: "@2".into()
            })
        );
        assert_eq!(
            Message::parse(r#"{"op":"prompted","id":3,"text":null}"#),
            Ok(Message::Prompted(Answer { id: 3, text: None }))
        );
        assert!(Message::parse(r#"{"op":"act","action":"zap","id":"@2"}"#).is_err());
        assert!(Message::parse(r#"{"op":"picker","kind":"tabs"}"#).is_err());
        assert!(Message::parse(r#"{"op":"picker","rows":[{"id":"a","colour":1}]}"#).is_err());
        assert!(Message::parse(r#"{"op":"pick"}"#).is_err());
    }

    #[test]
    fn a_window_says_each_thing_once() {
        let herdr = english(Mux::Herdr);
        assert_eq!(window_title(1, "1", "", None, None, &herdr), "Tab 1");
        assert_eq!(window_title(2, "", "", None, None, &herdr), "Tab 2");
        let tmux = english(Mux::Tmux);
        assert_eq!(
            window_title(3, "logs", "tail", None, None, &tmux),
            "3: logs"
        );
    }

    #[test]
    fn a_window_is_named_the_home_way_never_by_a_version() {
        let tmux = english(Mux::Tmux);
        // tmux named it after Claude Code's binary: its folder says more.
        assert_eq!(
            window_title(
                2,
                "2.1.322",
                "2.1.322",
                Some("/home/dev/src/trainer"),
                Some("claude"),
                &tmux
            ),
            "2: trainer"
        );
        // No folder (the home directory): the agent, then the program.
        assert_eq!(
            window_title(
                2,
                "2.1.322",
                "2.1.322",
                Some("/home/dev"),
                Some("claude"),
                &tmux
            ),
            "2: claude"
        );
        assert_eq!(
            window_title(2, "2.1.322", "2.1.322", None, None, &tmux),
            "2: claude"
        );
        assert_eq!(window_title(4, "zsh", "zsh", None, None, &tmux), "4: zsh");
        // A name someone gave it wins over the folder.
        assert_eq!(
            window_title(1, "agents", "zsh", Some("/home/dev/src/api"), None, &tmux),
            "1: agents"
        );
    }

    #[test]
    fn twin_sessions_carry_their_ids() {
        let row = |id: &str, name: &str| Row {
            id: id.into(),
            name: name.into(),
            ..Row::default()
        };
        let rows = vec![row("w1", "~"), row("w4", "~"), row("w5", "api")];
        assert_eq!(session_title(&rows[0], &rows), "~ · w1");
        assert_eq!(session_title(&rows[1], &rows), "~ · w4");
        assert_eq!(session_title(&rows[2], &rows), "api");
    }
}
