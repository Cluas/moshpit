//! A host's sessions (`docs/nsh/UI-DESIGN.md` §6.2): the multiplexer's
//! whole tree, which Home no longer shows, on one pushed page — every
//! session (tmux) or workspace (herdr) as a card headed by itself, its
//! windows (tabs) under it, a window of several panes opening onto them.
//!
//! The host owns the data and the actions, as it does Home's
//! (`ui/src/home`). It posts the host as Home's state carries it
//! (`{"op": "sessions", "connection": {…}}`, [`Connection`]) whenever it
//! changes, and the tree's disclosure Home's model keeps
//! (`{"op": "trees", …}`, [`TreeState`]). The page tells it what the user
//! asks with Home's own intents ([`Intent`]: a window, a pane, a new
//! session, a refresh, reconnecting), so one handler serves both pages,
//! and each change of the disclosure (`{"ev": "tree", "connection": …,
//! "collapsed": […], "expanded": […]}`). Its rows carry Home's row
//! identifiers (`home-session-<connection>-<id>`, …): the host's
//! long-press menus (rename, a new window or pane, close) find them by
//! those.
//!
//! - A session's head folds its windows away; folded, it names its first
//!   three windows. The session in use says so; one where an agent waits
//!   says how many. A git worktree's session names its repository.
//! - A window of one pane opens it; a window of several folds its panes
//!   out. A pane's row is its program and its folder, an agent's its
//!   name, its mark and its state.
//! - Every session starts open; past eight, only the one in use does.
//! - Disconnected, the page is read only under a banner that reconnects.
//! - A session, a window or a pane swipes open on its most used actions
//!   (B499: [`swipe::session_tray`], [`swipe::window_tray`],
//!   [`swipe::pane_tray`]), one row of the page at a time; read only,
//!   none.

pub mod model;

use std::{collections::HashSet, rc::Rc};

use nsh_core::prelude::*;
use serde::Deserialize;
use serde_json::json;

use crate::{
    channel::{self, Channel},
    home::{
        Intent, TreeState,
        model::{AgentState, Connection, Link, Mux},
        swipe,
    },
    nav_bar::{self, BarButton, BarContent, BarMetrics, NavBar},
    screen::Screen,
    ui::{
        button::{Variant, button},
        empty::empty,
        icon::{Tone, icon},
        mark::agent_mark,
        motion::Duration as Motion,
        notice::{BannerTone, banner},
        page::page,
        row::row,
        section::{SectionVariant, section},
        status::Status,
        tokens::token,
    },
};

use model::{Disclosure, Group, PaneItem, WindowItem, Words};

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "sessions", "connection": {…}}`: the host and its
    /// multiplexer as they are now.
    Sessions(Box<Connection>),
    /// `{"op": "trees", "trees": […]}`: the disclosure the pages last
    /// told, for a page made again.
    Trees(Vec<TreeState>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionsFields {
    connection: Connection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TreesFields {
    trees: Vec<TreeState>,
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("sessions", json)?;
        let fields = serde_json::Value::Object(fields);
        match op.as_str() {
            "sessions" => serde_json::from_value::<SessionsFields>(fields)
                .map(|f| Message::Sessions(Box::new(f.connection)))
                .map_err(|e| format!("sessions: sessions: {e}")),
            "trees" => serde_json::from_value::<TreesFields>(fields)
                .map(|f| Message::Trees(f.trees))
                .map_err(|e| format!("sessions: trees: {e}")),
            other => Err(format!("sessions: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Sessions(connection) => {
                json!({"op": "sessions", "connection": connection}).to_string()
            }
            Message::Trees(trees) => json!({"op": "trees", "trees": trees}).to_string(),
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
type Tell = Rc<dyn Fn(serde_json::Value)>;

/// The sessions page.
#[component]
pub fn SessionsScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let params = channel.as_ref().map(|c| c.params()).unwrap_or_default();
    let bar = BarMetrics::or_standard(params.bar);
    let back = params.back.clone().unwrap_or_else(|| t!("Moshpit"));
    let connection = signal(None::<Connection>);
    let disclosure = signal(Disclosure::default());
    // The row whose swipe tray is open (one at a time on the page).
    let open = signal(None::<String>);
    // The disclosure the host kept, which may come before the host does.
    let kept = stored(Vec::<TreeState>::new());
    let take = move |id: &str| {
        if disclosure.with_untracked(|d| d.told) {
            return;
        }
        // Only what this page told: a tree Home's idle group made is not
        // one (B553).
        let tree = kept.with(|trees| trees.iter().find(|t| t.connection == id && t.told).cloned());
        if let Some(tree) = tree {
            disclosure.set(Disclosure::told(&tree));
        }
    };
    channel::hear(
        channel.as_ref(),
        Screen::Sessions,
        Message::parse,
        move |message| match message {
            Message::Sessions(next) => {
                // A session or window come or gone moves the rows into
                // place (the user's own new or close).
                let moved = connection.with_untracked(|now| {
                    now.as_ref()
                        .is_some_and(|now| model::shape(now) != model::shape(&next))
                });
                if moved && !reduce_motion() {
                    animate_layout(nsh_core::LayoutAnimation::all(
                        nsh_core::LayoutTransition::new(
                            nsh_core::LayoutCurve::EaseInEaseOut,
                            Motion::Base.nominal_ms(),
                        ),
                    ));
                }
                let id = next.id.clone();
                connection.set(Some(*next));
                take(&id);
            }
            Message::Trees(trees) => {
                kept.replace(trees);
                disclosure.update(|d| d.told = false);
                if let Some(id) = connection.with_untracked(|c| c.as_ref().map(|c| c.id.clone())) {
                    take(&id);
                }
            }
        },
    );
    let tell: Tell = {
        let channel = channel.clone();
        Rc::new(move |event| {
            if let Some(channel) = &channel {
                channel.tell(&event);
            }
        })
    };
    let scrolled = nav_bar::ScrollEdge::new();
    let id = move || connection.with(|c| c.as_ref().map(|c| c.id.clone()));
    let live = computed(move || {
        connection.with(|c| {
            c.as_ref()
                .is_some_and(|c| c.link == Link::Live && c.control.is_some())
        })
    });
    let refreshing = computed(move || {
        connection.with(|c| {
            c.as_ref()
                .and_then(|c| c.control.as_ref())
                .is_some_and(|control| control.refreshing)
        })
    });
    let title = move || connection.with(|c| c.as_ref().map(|c| c.name.clone()).unwrap_or_default());
    let new_session = {
        let tell = tell.clone();
        move || {
            if let Some(id) = id() {
                tell(Intent::NewSession(id).event());
            }
        }
    };
    let refresh = {
        let tell = tell.clone();
        move || {
            if let Some(id) = untracked(id) {
                tell(Intent::Refresh(id).event());
            }
        }
    };
    let body = {
        let tell = tell.clone();
        move || body(connection, disclosure, live, open, tell.clone())
    };
    let list = page(
        bar,
        scrolled,
        "sessions-list",
        crate::boundary::guarded(move || dynamic(body.clone()), tell.clone()),
    )
    .refreshing(move || refreshing.get())
    .on_refresh(move |_: &()| refresh());
    let trailing = vec![BarButton {
        label: t!("New Session"),
        content: BarContent::Symbol("plus"),
        enabled: computed(move || live.get()),
        emphasized: false,
        test_id: "sessions-new",
        on_press: Box::new(new_session),
        menu: None,
    }];
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("sessions")
        .child(list)
        .child(nav_bar::nav_bar(NavBar {
            metrics: bar,
            back_title: back,
            title: Box::new(title),
            leading: None,
            trailing,
            on_back: Box::new(move || tell(json!({"ev": "back"}))),
            scrolled,
        }))
}

/// What the page shows under its bar: the banners, then the sessions or
/// the empty state.
fn body(
    connection: Signal<Option<Connection>>,
    disclosure: Signal<Disclosure>,
    live: Memo<bool>,
    open: Signal<Option<String>>,
    tell: Tell,
) -> Vec<AnyElement> {
    let Some(host) = connection.get() else {
        return Vec::new();
    };
    let words = Words::of(host.control.as_ref().map_or(Mux::Tmux, |c| c.mux));
    let mut parts = Vec::new();
    if !live.get() {
        let id = host.id.clone();
        let tell = tell.clone();
        parts.push(
            banner(BannerTone::Warning, || t!("Disconnected").into())
                .icon("bolt.slash")
                .action(t!("Reconnect"), move || {
                    tell(Intent::Connect(id.clone()).event())
                })
                .action_test_id("sessions-reconnect")
                .test_id("sessions-offline")
                .into_element(),
        );
    }
    let Some(control) = host.control.clone() else {
        return parts;
    };
    if let Some(mismatch) = control.mismatch.clone() {
        let mut notice = banner(BannerTone::Warning, move || mismatch.notice.clone().into())
            .test_id("sessions-mismatch");
        if mismatch.restart && !mismatch.restarting {
            let id = host.id.clone();
            let tell = tell.clone();
            notice = notice
                .action(t!("Restart server"), move || {
                    tell(Intent::RestartServer(id.clone()).event())
                })
                .action_test_id("sessions-restart");
        }
        parts.push(notice.into_element());
    }
    let groups = model::groups(&control, &disclosure.get(), &words);
    if groups.is_empty() {
        let id = host.id.clone();
        let tell = tell.clone();
        let live = live.get();
        let mut state = empty(words.none.clone())
            .icon("square.stack")
            .test_id("sessions-empty");
        if live {
            state = state.action(
                button(words.new.clone())
                    .variant(Variant::Primary)
                    .on_press(move || tell(Intent::NewSession(id.clone()).event()))
                    .test_id("sessions-empty-new"),
            );
        }
        parts.push(state.into_element());
        return parts;
    }
    let read_only = !live.get();
    // The trays (B499): none on a page that is read only.
    let trays = (!read_only).then_some(Trays {
        mux: control.mux,
        open,
    });
    for group in groups {
        parts.push(session_card(
            &host.id,
            group,
            &words,
            read_only,
            trays,
            disclosure,
            tell.clone(),
        ));
    }
    parts
}

/// The page's swipe trays (B499): the multiplexer the actions are
/// worded for, and the row whose tray is open.
#[derive(Clone, Copy)]
struct Trays {
    mux: Mux,
    open: Signal<Option<String>>,
}

impl Trays {
    /// `line` in a tray of `actions`, named `key` among the page's rows.
    fn wrap(
        self,
        line: crate::ui::row::Row,
        key: String,
        actions: Vec<swipe::Action>,
        tell: &Tell,
    ) -> crate::ui::row::Row {
        let (open, tell) = (self.open, tell.clone());
        line.wrapped(move |content| swipe::tray_row(key, open, actions, tell, content))
    }
}

/// A session's card: its head, then (open) its windows and their panes.
fn session_card(
    connection: &str,
    group: Group,
    words: &Words,
    read_only: bool,
    trays: Option<Trays>,
    disclosure: Signal<Disclosure>,
    tell: Tell,
) -> AnyElement {
    let session = group.id.clone();
    let mut head = row(group.title.clone())
        .heading()
        .disclosure({
            let open = group.expanded;
            move || open
        })
        .test_id(&format!("home-session-{connection}-{session}"));
    let subtitle = group.subtitle();
    if !subtitle.is_empty() {
        head = head.subtitle(subtitle);
    }
    if group.waiting > 0 {
        head = head
            .value(crate::plural!("%lld waiting", group.waiting as i64))
            .value_tone(Tone::Warning)
            .value_dot(Status::Attention);
    }
    if let Some(trays) = trays {
        let actions = swipe::worded(
            swipe::session_tray(connection, &session, group.repo.is_some()),
            trays.mux,
        );
        head = trays.wrap(
            head,
            format!("session:{connection}:{session}"),
            actions,
            &tell,
        );
    }
    {
        let (connection, session, tell) = (connection.to_owned(), session.clone(), tell.clone());
        let sessions = group.all_sessions.clone();
        let active = group.active_session.clone();
        head = head.on_press(move || {
            toggle_session(
                &connection,
                &session,
                &sessions,
                active.as_deref(),
                disclosure,
                &tell,
            )
        });
    }
    let mut card = section()
        .variant(SectionVariant::Card)
        .head(head)
        .test_id(&format!("sessions-card-{session}"));
    if group.expanded {
        for window in group.windows {
            card = card.row(window_row(
                connection, &window, read_only, trays, disclosure, &tell,
            ));
            if window.expanded {
                for pane in &window.panes {
                    card = card.row(pane_row(connection, pane, words, read_only, trays, &tell));
                }
            }
        }
        // The open session's last row: a new window (herdr: tab) in it,
        // named first by the host's prompt. Not on a page that is read
        // only.
        if !read_only {
            card = card.row(new_window_row(connection, &session, words, &tell));
        }
    }
    card.into_element()
}

/// "New window" (herdr: "New tab") at the end of session `session`'s
/// open card: quiet as the tree's rows, a muted + and the words.
fn new_window_row(
    connection: &str,
    session: &str,
    words: &Words,
    tell: &Tell,
) -> crate::ui::row::Row {
    let intent = Intent::NewWindow {
        connection: connection.to_owned(),
        session: session.to_owned(),
    };
    let tell = tell.clone();
    row(words.new_window.clone())
        .leading(icon("plus").tone(Tone::Muted))
        .on_press(move || tell(intent.event()))
        .test_id(&format!("sessions-new-window-{connection}-{session}"))
}

/// A window's row: one pane opens it; several fold out.
fn window_row(
    connection: &str,
    window: &WindowItem,
    read_only: bool,
    trays: Option<Trays>,
    disclosure: Signal<Disclosure>,
    tell: &Tell,
) -> crate::ui::row::Row {
    let mut line =
        row(window.title.clone()).test_id(&format!("home-window-{connection}-{}", window.id));
    if let Some(trays) = trays {
        let actions = swipe::worded(swipe::window_tray(connection, &window.id), trays.mux);
        line = trays.wrap(
            line,
            format!("window:{connection}:{}", window.id),
            actions,
            tell,
        );
    }
    if !window.subtitle.is_empty() {
        line = line.subtitle(window.subtitle.clone());
    }
    line = match window.agent {
        Some(state) => line
            .leading(agent_mark(status_of(state)))
            .value(state_words(state))
            .value_tone(tone_of(state)),
        None => line.leading_icon(window.icon),
    };
    if window.panes.len() > 1 {
        let open = window.expanded;
        let (connection, id, tell) = (connection.to_owned(), window.id.clone(), tell.clone());
        line = line
            .disclosure(move || open)
            .on_press(move || toggle_window(&connection, &id, disclosure, &tell));
    } else if !read_only {
        let (connection, id, tell) = (connection.to_owned(), window.id.clone(), tell.clone());
        line = line.chevron().on_press(move || {
            tell(
                Intent::Window {
                    connection: connection.clone(),
                    window: id.clone(),
                }
                .event(),
            )
        });
    }
    line
}

/// A pane's row: its program and folder, or an agent's name and state.
fn pane_row(
    connection: &str,
    pane: &PaneItem,
    words: &Words,
    read_only: bool,
    trays: Option<Trays>,
    tell: &Tell,
) -> crate::ui::row::Row {
    let title = if pane.title.is_empty() {
        words.shell.clone()
    } else {
        pane.title.clone()
    };
    let mut line = row(title).test_id(&format!("home-pane-{connection}-{}", pane.id));
    if let Some(trays) = trays {
        // The window above names it: no Rename here, the kill only.
        let actions = swipe::worded(swipe::pane_tray(connection, &pane.id, None), trays.mux);
        line = trays.wrap(
            line,
            format!("pane:{connection}:{}", pane.id),
            actions,
            tell,
        );
    }
    if let Some(path) = &pane.path {
        line = line.subtitle(path.clone()).mono_subtitle();
    }
    line = match pane.agent {
        Some(state) => line
            .leading(agent_mark(status_of(state)))
            .value(state_words(state))
            .value_tone(tone_of(state)),
        None => line.leading_icon(pane.icon),
    };
    if pane.active {
        line = line.selected(|| true);
    }
    if !read_only {
        let (connection, id, tell) = (connection.to_owned(), pane.id.clone(), tell.clone());
        line = line.chevron().on_press(move || {
            tell(
                Intent::Pane {
                    connection: connection.clone(),
                    pane: id.clone(),
                }
                .event(),
            )
        });
    }
    line
}

/// Folds session `session` away, or out, and tells the host.
fn toggle_session(
    connection: &str,
    session: &str,
    sessions: &[String],
    active: Option<&str>,
    disclosure: Signal<Disclosure>,
    tell: &Tell,
) {
    let mut next = disclosure.get_untracked().settled(sessions, active);
    if !next.collapsed.remove(session) {
        next.collapsed.insert(session.to_owned());
    }
    disclose(connection, next, disclosure, tell);
}

/// Folds window `window`'s panes out, or away, and tells the host.
fn toggle_window(connection: &str, window: &str, disclosure: Signal<Disclosure>, tell: &Tell) {
    let mut next = disclosure.get_untracked();
    next.told = true;
    if !next.expanded.remove(window) {
        next.expanded.insert(window.to_owned());
    }
    disclose(connection, next, disclosure, tell);
}

/// `next` is the disclosure now: the rows move into place (`base`), and
/// the host keeps it.
fn disclose(connection: &str, next: Disclosure, disclosure: Signal<Disclosure>, tell: &Tell) {
    if !reduce_motion() {
        animate_layout(nsh_core::LayoutAnimation::all(
            nsh_core::LayoutTransition::new(
                nsh_core::LayoutCurve::EaseInEaseOut,
                Motion::Base.nominal_ms(),
            ),
        ));
    }
    let sorted = |set: &HashSet<String>| {
        let mut ids: Vec<&String> = set.iter().collect();
        ids.sort();
        ids.into_iter().cloned().collect::<Vec<_>>()
    };
    tell(json!({
        "ev": "tree",
        "connection": connection,
        "collapsed": sorted(&next.collapsed),
        "expanded": sorted(&next.expanded),
    }));
    disclosure.set(next);
}

/// An agent's state as a dot's.
fn status_of(state: AgentState) -> Status {
    match state {
        AgentState::NeedsYou => Status::Attention,
        AgentState::Working => Status::Working,
        AgentState::Done => Status::Done,
        AgentState::Idle => Status::Idle,
    }
}

/// An agent's state in words.
fn state_words(state: AgentState) -> String {
    match state {
        AgentState::NeedsYou => t!("Needs you"),
        AgentState::Working => t!("Working"),
        AgentState::Done => t!("Finished"),
        AgentState::Idle => t!("Idle"),
    }
}

/// The ink of an agent's state: amber while it waits on you.
fn tone_of(state: AgentState) -> Tone {
    match state {
        AgentState::NeedsYou => Tone::Warning,
        _ => Tone::Muted,
    }
}
