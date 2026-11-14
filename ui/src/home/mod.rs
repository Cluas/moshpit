//! Home (`docs/nsh/UI-DESIGN.md` §6.1): the app's first screen, which says
//! at a glance which agent waits on you and takes you to it in one tap.
//!
//! Top to bottom, under the large-title bar ([`bar`], R4):
//!
//! - NEEDS YOU, across hosts, only while someone waits: one row per waiting
//!   agent, its host in its subtitle. Nobody waits: a quiet line of facts
//!   ("Nothing needs you · 2 working · 9 idle"), and nothing at all while
//!   no host is up.
//! - CONNECTIONS: a card per host with a session, in the order they were
//!   saved. Its head (the host's mark, name, transport, `user@host · 3h`,
//!   a "1 waiting" badge, ⋯) over what it has: its agents by state, the
//!   idle ones folded into one row, a few other panes, the way to all its
//!   sessions; or one row saying it connects, is lost, failed, or stalled.
//! - SAVED: one card, a row per host that is not connected.
//!
//! No version line, no tagline, no grid on a Home with hosts; the empty
//! Home keeps the grid and the app's mark (§10, decision 1).
//!
//! Every row is one target: an agent's opens the terminal on its pane, a
//! card's head opens (or connects, or reconnects) the host, ⋯ on the head
//! is the only second button. A row's secondary actions are the host's
//! long-press menus (by the row's identifier: `home-head-<connection>`,
//! `home-pane-<connection>-<pane>`) and, on a head or a saved row, the
//! swipe tray.
//!
//! The host owns the data and the navigation. It posts what Home shows
//! ([`Message`]): `{"op": "home", …}` ([`model::HomeState`]) whenever any
//! of it changes; the page derives what to draw ([`model`]: R5) and tells
//! the host what the user asks ([`Intent`]: `{"ev": "pane", …}`). The one
//! thing only the page knows is which hosts have their idle group open: it
//! tells the host each change (`{"ev": "tree", …, "idle": true}`) and gets
//! it back when it is made again (`{"op": "trees", …}`).

pub mod bar;
mod card;
pub mod model;
pub mod pet;
mod rows;
mod swipe;
pub mod words;

use std::{collections::HashMap, rc::Rc, sync::Arc, time::Duration};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    nav_bar::BarMetrics,
    screen::Screen,
    theme,
    ui::{
        button::{Variant, button},
        empty::empty,
        motion,
        section::section,
        status::{Badge, BadgeVariant, Status, badge, share_breath, status_dot},
        tokens::{number, token},
        typography::{TextToken, eyebrow},
    },
};
pub use model::HomeState;

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "home", …}`: what Home shows now.
    Home(Box<HomeState>),
    /// `{"op": "trees", "trees": […]}`: what the page last said of each
    /// host's lists, for a page made again.
    Trees(Vec<TreeState>),
    /// `{"op": "pet", …}`: the companion, and the count of its moments
    /// ([`pet`]).
    Pet(Box<moshpit_pet::PetView>, u64),
}

/// What the page says of one host's lists: its idle group open or not;
/// and, for the sessions page (R7), the sessions collapsed and the windows
/// showing their panes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeState {
    pub connection: String,
    #[serde(default)]
    pub collapsed: Vec<String>,
    #[serde(default)]
    pub expanded: Vec<String>,
    /// Home's idle group is open on this host's card.
    #[serde(default)]
    pub idle: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TreesFields {
    trees: Vec<TreeState>,
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("home", json)?;
        let fields = serde_json::Value::Object(fields);
        match op.as_str() {
            "home" => serde_json::from_value(fields)
                .map(|state| Message::Home(Box::new(state)))
                .map_err(|e| format!("home: home: {e}")),
            "trees" => serde_json::from_value::<TreesFields>(fields)
                .map(|f| Message::Trees(f.trees))
                .map_err(|e| format!("home: trees: {e}")),
            "pet" => pet::parse(fields).map(|(view, seq)| Message::Pet(Box::new(view), seq)),
            other => Err(format!("home: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Home(state) => {
                let mut value = serde_json::to_value(state).expect("a state serializes");
                value["op"] = json!("home");
                value.to_string()
            }
            Message::Trees(trees) => json!({"op": "trees", "trees": trees}).to_string(),
            Message::Pet(view, seq) => {
                let mut value = serde_json::to_value(view).expect("a pet serializes");
                value["op"] = json!("pet");
                value["seq"] = json!(seq);
                value.to_string()
            }
        }
    }
}

/// What the user asks the host for (`{"ev": …}`), each naming the
/// connection it is about (by id) and, for a pane, the pane. The session,
/// window and pane actions past [`Intent::Sessions`] are the long-press
/// menus' and the sessions page's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// The bar's Settings.
    Settings,
    /// The bar's +, or the empty state's Add connection.
    Add,
    /// The iPad sidebar's button: hide or show the sidebar.
    ToggleSidebar,
    /// Connect a saved host (its row), or one whose session is gone (its
    /// head, Reconnect).
    Connect(String),
    /// Open the terminal where it was: a live head, Open terminal.
    Enter(String),
    /// Open the terminal on a window (`selectWindow`).
    Window {
        connection: String,
        window: String,
    },
    /// Open the terminal on a pane (`selectPane`): an agent's row, another
    /// pane's.
    Pane {
        connection: String,
        pane: String,
    },
    /// All of a host's sessions or workspaces (§6.2: the sessions page).
    Sessions(String),
    /// A stalled attach: tear down and connect afresh.
    RetryAttach(String),
    /// Disconnect (⋯, the head's swipe and long press).
    Disconnect(String),
    /// Read the host's sessions again.
    Refresh(String),
    /// Name a new session.
    NewSession(String),
    /// A new agent task (herdr).
    NewAgentTask(String),
    /// Restart herdr's server over a version skew.
    RestartServer(String),
    /// Edit the connection.
    Edit(String),
    /// Delete the connection (the host asks first).
    Delete(String),
    RenameSession {
        connection: String,
        session: String,
    },
    KillSession {
        connection: String,
        session: String,
    },
    NewWindow {
        connection: String,
        session: String,
    },
    RemoveWorktree {
        connection: String,
        session: String,
    },
    RenameWindow {
        connection: String,
        window: String,
    },
    KillWindow {
        connection: String,
        window: String,
    },
    NewPane {
        connection: String,
        window: String,
    },
    KillPane {
        connection: String,
        pane: String,
    },
}

impl Intent {
    /// The event that tells the host.
    pub fn event(&self) -> serde_json::Value {
        let on = |ev: &str, connection: &str| json!({"ev": ev, "connection": connection});
        let at = |ev: &str, connection: &str, level: &str, id: &str| json!({"ev": ev, "connection": connection, level: id});
        match self {
            Intent::Settings => json!({"ev": "settings"}),
            Intent::Add => json!({"ev": "add"}),
            Intent::ToggleSidebar => json!({"ev": "toggle-sidebar"}),
            Intent::Connect(c) => on("connect", c),
            Intent::Enter(c) => on("enter", c),
            Intent::Window { connection, window } => at("window", connection, "window", window),
            Intent::Pane { connection, pane } => at("pane", connection, "pane", pane),
            Intent::Sessions(c) => on("sessions", c),
            Intent::RetryAttach(c) => on("retry-attach", c),
            Intent::Disconnect(c) => on("disconnect", c),
            Intent::Refresh(c) => on("refresh", c),
            Intent::NewSession(c) => on("new-session", c),
            Intent::NewAgentTask(c) => on("new-agent-task", c),
            Intent::RestartServer(c) => on("restart-server", c),
            Intent::Edit(c) => on("edit", c),
            Intent::Delete(c) => on("delete", c),
            Intent::RenameSession {
                connection,
                session,
            } => at("rename-session", connection, "session", session),
            Intent::KillSession {
                connection,
                session,
            } => at("kill-session", connection, "session", session),
            Intent::NewWindow {
                connection,
                session,
            } => at("new-window", connection, "session", session),
            Intent::RemoveWorktree {
                connection,
                session,
            } => at("remove-worktree", connection, "session", session),
            Intent::RenameWindow { connection, window } => {
                at("rename-window", connection, "window", window)
            }
            Intent::KillWindow { connection, window } => {
                at("kill-window", connection, "window", window)
            }
            Intent::NewPane { connection, window } => at("new-pane", connection, "window", window),
            Intent::KillPane { connection, pane } => at("kill-pane", connection, "pane", pane),
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
pub(crate) type Tell = Rc<dyn Fn(serde_json::Value)>;

/// What the rows of the page share: the host, what the page says of each
/// host's lists, the clock their durations read, the open swipe tray.
#[derive(Clone)]
pub(crate) struct Ctx {
    tell: Tell,
    /// By connection: what the page last said of its lists.
    trees: Signal<HashMap<String, TreeState>>,
    /// Unix seconds, ticking every half minute.
    pub(crate) now: Signal<f64>,
    /// The row whose swipe tray is open (one at a time on the page).
    pub(crate) open: Signal<Option<String>>,
    /// The companion ([`pet`]).
    pub(crate) pet: pet::PetCtx,
    /// Where it perches and how, from what Home shows.
    pub(crate) perch: Memo<Option<(pet::Perch, moshpit_pet::Pose)>>,
    /// Hosts' marks are their pixel creatures ([`pet::creatures`]).
    pub(crate) creatures: Memo<bool>,
}

impl Ctx {
    /// Tells the host `intent`.
    pub(crate) fn ask(&self, intent: Intent) {
        (self.tell)(intent.event());
    }

    /// What `intent` asks, as a press handler.
    pub(crate) fn asking(&self, intent: Intent) -> impl Fn() + 'static {
        let tell = self.tell.clone();
        move || tell(intent.event())
    }

    /// Whether connection `id`'s idle group is open (tracked).
    pub(crate) fn idle_open(&self, id: &str) -> bool {
        self.trees
            .with(|trees| trees.get(id).is_some_and(|t| t.idle))
    }

    /// Opens or closes connection `id`'s idle group, its rows coming and
    /// going with the layout (§6.1 motion: `base`; at once with reduced
    /// motion), and tells the host.
    pub(crate) fn toggle_idle(&self, id: &str) {
        let mut next = self
            .trees
            .with_untracked(|trees| trees.get(id).cloned())
            .unwrap_or_else(|| TreeState {
                connection: id.to_owned(),
                ..TreeState::default()
            });
        next.idle = !next.idle;
        animate_rows();
        // Only what Home keeps: the sessions page (R7) sends the sessions
        // and windows it opens for the same host, and the host updates the
        // fields a page sends.
        (self.tell)(json!({
            "ev": "tree",
            "connection": id,
            "idle": next.idle,
        }));
        let id = id.to_owned();
        self.trees.update(move |trees| {
            trees.insert(id, next);
        });
    }
}

/// Animates the layout change the current update makes, as Home's lists
/// change (§6.1 motion: rows that come fade in, rows that go fade out, the
/// rest move, over `base`); nothing with reduced motion.
pub(crate) fn animate_rows() {
    if reduce_motion() {
        return;
    }
    animate_layout(nsh_core::LayoutAnimation::all(
        nsh_core::LayoutTransition::new(
            nsh_core::LayoutCurve::EaseInEaseOut,
            motion::Duration::Base.ms(),
        ),
    ));
}

/// The agent name a pane falls back to when its hook names none and its
/// program is no agent Home knows ("agent").
pub(crate) fn agent_word() -> String {
    t!("agent")
}

/// Home.
#[component]
pub fn HomeScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    // Home is a root, never in a sheet: its inline row is the platform's
    // standard bar.
    let bar = BarMetrics::standard(idiom());
    let state = signal(None::<HomeState>);
    let trees = signal(HashMap::<String, TreeState>::new());
    let pet_ctx = pet::PetCtx::new();
    let (pet_view, pet_seq) = (pet_ctx.view, pet_ctx.seq);
    channel::hear(
        channel.as_ref(),
        Screen::Home,
        Message::parse,
        move |message| match message {
            Message::Home(next) => state.set(Some(*next)),
            Message::Trees(next) => trees.set(
                next.into_iter()
                    .map(|t| (t.connection.clone(), t))
                    .collect(),
            ),
            Message::Pet(view, seq) => {
                pet_view.set(Some(*view));
                pet_seq.set(seq);
            }
        },
    );
    // One breath for every working dot on the page (§4.12).
    share_breath();
    let now = signal(unix_now());
    let _ticker = set_interval(Duration::from_secs(30), move || now.set(unix_now()));
    let perch = computed(move || {
        state.with(|s| {
            s.as_ref()
                .and_then(|s| pet::perch(&s.connections, &agent_word()))
        })
    });
    let creatures = computed(move || pet_view.with(|v| pet::creatures(v.as_ref())));
    let ctx = Ctx {
        tell: tell_host(channel),
        trees,
        now,
        open: signal(None),
        pet: pet_ctx,
        perch,
        creatures,
    };
    let _seen = pet::seen(ctx.tell.clone());
    let offset = animated_value(0.0);
    let pull = animated_value(0.0);
    let ready = computed(move || state.with(Option::is_some));
    let empty =
        computed(move || state.with(|s| s.as_ref().is_some_and(|s| s.connections.is_empty())));
    let list = {
        let ctx = ctx.clone();
        move || list(state, bar, ctx.clone(), offset, pull)
    };
    let trailing = vec![
        bar::BarButton {
            symbol: "gearshape",
            label: t!("Settings"),
            test_id: "home-settings",
            on_press: Rc::new(ctx.asking(Intent::Settings)),
        },
        bar::BarButton {
            symbol: "plus",
            label: t!("Add Connection"),
            test_id: "home-add",
            on_press: Rc::new(ctx.asking(Intent::Add)),
        },
    ];
    let add: Rc<dyn Fn()> = Rc::new(ctx.asking(Intent::Add));
    let toggle: Rc<dyn Fn()> = Rc::new(ctx.asking(Intent::ToggleSidebar));
    let tell = ctx.tell.clone();
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("home")
        .child(crate::boundary::ready_body(
            move || ready.get() && !empty.get(),
            list,
            tell.clone(),
        ))
        .child(crate::boundary::ready_body(
            move || empty.get(),
            move || empty_page(bar, add.clone()),
            tell,
        ))
        .child(dynamic(move || {
            let sidebar = state.with(|s| s.as_ref().is_some_and(|s| s.sidebar));
            let (trailing, toggle) = (trailing.clone(), toggle.clone());
            untracked(move || {
                let leading = sidebar.then(|| bar::BarButton {
                    symbol: "sidebar.left",
                    label: t!("Hide Sidebar"),
                    test_id: "home-sidebar",
                    on_press: toggle,
                });
                bar::bar(bar, t!("Moshpit"), leading, trailing, offset, pull)
            })
        }))
}

/// The time now, in Unix seconds, on the engine's clock: the system's on a
/// device, the one a headless test sets in a test.
fn unix_now() -> f64 {
    nsh_core::time::unix_ms() / 1000.0
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

/// From the large title to the first group, and between groups (§6.1
/// structure 1, §3.2: `space-6`).
pub(crate) const FIRST_GAP: f32 = 8.0;
pub(crate) const GROUP_GAP: f32 = 24.0;
/// Under the last group, above the bottom's safe area.
const BOTTOM: f32 = 32.0;

/// The page's margin: where the cards' edge and the large title are.
pub(crate) fn margin() -> f32 {
    number("space-page")
}

/// The list: the top group (NEEDS YOU or the summary), CONNECTIONS, SAVED,
/// under the bar's large title; at most `content-max-width` wide, centered.
fn list(
    state: Signal<Option<HomeState>>,
    bar: BarMetrics,
    ctx: Ctx,
    offset: AnimatedValue,
    pull: AnimatedValue,
) -> AnyElement {
    let connections = computed(move || {
        state.with(|s| {
            s.as_ref()
                .map(|s| s.connections.clone())
                .unwrap_or_default()
        })
    });
    let summary = computed(move || connections.with(|c| model::summary(c, &agent_word())));
    let waiting = computed(move || {
        connections.with(|c| {
            model::needs_you(c, &agent_word())
                .into_iter()
                .map(|w| {
                    // The login, for the path's `~`.
                    let user = c
                        .iter()
                        .find(|c| c.id == w.row.connection)
                        .map(|c| c.user.clone())
                        .unwrap_or_default();
                    (w, user)
                })
                .collect::<Vec<_>>()
        })
    });
    let hosts = computed(move || {
        connections.with(|c| {
            c.iter()
                .filter(|c| c.link != model::Link::Saved)
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    let saved = computed(move || {
        connections.with(|c| {
            c.iter()
                .filter(|c| c.link == model::Link::Saved)
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    // What shows, in order, so each group knows whether one is above it.
    let top = computed(move || summary.get() != model::Summary::Nothing);
    let has_hosts = computed(move || hosts.with(|h| !h.is_empty()));
    let has_saved = computed(move || saved.with(|s| !s.is_empty()));
    let gap = |above: bool| px(if above { GROUP_GAP } else { FIRST_GAP });

    let scroller = element_ref();
    let content = view()
        .w_full()
        .px(move || px(margin()))
        .child(view().h(move || bar::space(bar)))
        .child(when(move || top.get(), {
            let ctx = ctx.clone();
            move || {
                view()
                    .w_full()
                    .mt(px(FIRST_GAP))
                    .child(top_group(summary, waiting, ctx.clone()))
            }
        }))
        .child(when(move || has_hosts.get(), {
            let ctx = ctx.clone();
            move || {
                view()
                    .w_full()
                    .mt(move || gap(top.get()))
                    .child(connections_group(hosts, ctx.clone()))
            }
        }))
        .child(when(move || has_saved.get(), {
            let ctx = ctx.clone();
            move || {
                view()
                    .w_full()
                    .mt(move || gap(top.get() || has_hosts.get()))
                    .child(saved_group(saved, ctx.clone()))
            }
        }))
        .child(view().h(move || px(BOTTOM + safe_area().bottom.0)));
    let scroll = scroll()
        .w_full()
        .h_full()
        .max_w(px(number("content-max-width")))
        .test_id("home-list")
        .scroll_value(&offset)
        .pull_value(&pull)
        .element_ref(&scroller)
        // UI-DESIGN §4.18 (c): let go part way through the large title's
        // collapse, the list settles at the nearer end; at once with
        // reduced motion.
        .on_scroll_end(move |at: &Point<Points>| {
            if let Some(y) = bar::LargeTitleMotion::now().settle(at.y.0) {
                scroller.scroll_to(point(px(0.0), px(y)), !reduce_motion());
            }
        })
        .child(content);
    view()
        .absolute()
        .inset_0()
        .items_center()
        .child(scroll)
        .into_element()
}

/// The top group: NEEDS YOU while someone waits, else the line of facts.
fn top_group(
    summary: Memo<model::Summary>,
    waiting: Memo<Vec<(model::WaitingRow, String)>>,
    ctx: Ctx,
) -> AnyElement {
    let needs = computed(move || matches!(summary.get(), model::Summary::NeedsYou { .. }));
    // NEEDS YOU's first coming fades in with the layout (§6.1 motion).
    effect(move || {
        if needs.get() {
            animate_rows();
        }
    });
    dynamic(move || {
        if needs.get() {
            let ctx = ctx.clone();
            untracked(move || needs_you(waiting, ctx))
        } else {
            untracked(move || summary_line(summary))
        }
    })
}

/// A group's title (an eyebrow: a heading to screen readers) with a badge
/// beside it, on the cards' text line (32 pt in: the page's margin and the
/// card's padding).
fn group_header(title: String, badge: Option<Badge>) -> AnyElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .ps(px(crate::ui::row::PADDING_X))
        .min_h(px(20.0))
        .child(eyebrow(text(title)).role(AccessibilityRole::Header))
        .children(badge)
        .into_element()
}

/// NEEDS YOU: its title and count, then a card with a row per waiting
/// agent, the one waiting longest first (R5's order).
fn needs_you(waiting: Memo<Vec<(model::WaitingRow, String)>>, ctx: Ctx) -> AnyElement {
    let count = computed(move || waiting.with(Vec::len));
    // The rows' identity (who waits): the card is made again when it
    // changes, a new row rising in (§6.1 motion), not when a duration
    // ticks.
    let who = computed(move || {
        waiting.with(|w| {
            w.iter()
                .map(|(w, _)| (w.row.connection.clone(), w.row.pane.clone()))
                .collect::<Vec<_>>()
        })
    });
    view()
        .w_full()
        .gap(px(8.0))
        .test_id("home-needs-you")
        .child(dynamic(move || {
            let n = count.get();
            untracked(move || {
                group_header(
                    t!("Needs you"),
                    Some(
                        badge(n.to_string())
                            .variant(BadgeVariant::Warning)
                            .test_id("home-needs-you-count"),
                    ),
                )
            })
        }))
        .child(dynamic(move || {
            let _ = who.get();
            animate_rows();
            let rows = waiting.get_untracked();
            let ctx = ctx.clone();
            untracked(move || {
                let card = section()
                    .rows(
                        rows.into_iter()
                            .map(|(w, user)| rows::waiting_row(w, user, &ctx)),
                    )
                    .into_element();
                pet::on(ctx.clone(), pet::Perch::NeedsYou, card)
            })
        }))
        .into_element()
}

/// The line of facts when nobody waits (§6.1 structure 2): not a button,
/// not in a card; a green dot while something works, a hollow ring
/// otherwise; its words cross-fade as they change.
fn summary_line(summary: Memo<model::Summary>) -> AnyElement {
    let words = computed(move || summary_words(summary.get()));
    let working = computed(
        move || matches!(summary.get(), model::Summary::Agents { working, .. } if working > 0),
    );
    view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(crate::ui::row::PADDING_X))
        .min_h(px(20.0))
        .role(AccessibilityRole::Header)
        .label(move || words.get())
        .test_id("home-summary")
        .child(dynamic(move || {
            let status = if working.get() {
                Status::Live
            } else {
                Status::Idle
            };
            untracked(move || status_dot(status))
        }))
        .child(dynamic(move || {
            let words = words.get();
            animate_words();
            untracked(move || {
                TextToken::Subhead
                    .apply(text(words))
                    .number_of_lines(1u32)
                    .flex_shrink(1.0)
                    .text_color(move || token("muted-foreground"))
                    .accessibility_elements_hidden(true)
            })
        }))
        .into_element()
}

/// The summary's words: the facts, joined.
pub(crate) fn summary_words(summary: model::Summary) -> String {
    match summary {
        model::Summary::Agents {
            working,
            done,
            idle,
        } => {
            let mut parts = vec![t!("Nothing needs you")];
            if working > 0 {
                parts.push(t!("%lld working", arg1 = working));
            }
            if done > 0 {
                parts.push(crate::plural!("%lld done", done as i64));
            }
            if idle > 0 {
                parts.push(crate::plural!("%lld idle", idle as i64));
            }
            parts.join(" · ")
        }
        model::Summary::Online { connections } => {
            crate::plural!("%lld connections online", connections as i64)
        }
        model::Summary::NeedsYou { waiting } => {
            crate::plural!("%lld agents need you", waiting as i64)
        }
        model::Summary::Nothing => String::new(),
    }
}

/// New words cross-fade in (§6.1 motion: `base`); at once with reduced
/// motion.
fn animate_words() {
    if reduce_motion() {
        return;
    }
    let fade = nsh_core::LayoutTransition::new(
        nsh_core::LayoutCurve::EaseInEaseOut,
        motion::Duration::Base.ms(),
    );
    animate_layout(nsh_core::LayoutAnimation {
        create: Some(fade),
        update: None,
        delete: Some(fade),
    });
}

/// CONNECTIONS: its title, then a card per host with a session, in the
/// order they were saved, 24 pt apart.
fn connections_group(hosts: Memo<Vec<model::Connection>>, ctx: Ctx) -> AnyElement {
    view()
        .w_full()
        .gap(px(8.0))
        .test_id("home-connections")
        .child(group_header(t!("Connections"), None))
        .child(view().w_full().gap(px(GROUP_GAP)).child(each(
            hosts,
            |c: &model::Connection| c.id.clone(),
            move |connection, _| {
                let id = connection.with_untracked(|c| c.id.clone());
                pet::on(
                    ctx.clone(),
                    pet::Perch::Host(id),
                    card::card(connection, ctx.clone()),
                )
            },
        )))
        .into_element()
}

/// SAVED: one card, a row per host that is not connected. Rebuilt when a
/// row's host comes or goes or what a row shows changes (an edit, a key
/// trusted since: B512), not on every change to the hosts.
fn saved_group(saved: Memo<Vec<model::Connection>>, ctx: Ctx) -> AnyElement {
    let shown = computed(move || {
        saved.with(|s| {
            s.iter()
                .map(|c| {
                    (
                        c.id.clone(),
                        c.name.clone(),
                        c.initial.clone(),
                        c.face.clone(),
                        c.creature.clone(),
                        c.emoji.clone(),
                        c.user.clone(),
                        c.host.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
    });
    view()
        .w_full()
        .gap(px(8.0))
        .test_id("home-saved")
        .child(group_header(t!("Saved"), None))
        .child(dynamic(move || {
            shown.with(|_| ());
            // The marks: key faces or creatures.
            let _ = ctx.creatures.get();
            let hosts = saved.get_untracked();
            let ctx = ctx.clone();
            untracked(move || {
                pet::on(
                    ctx.clone(),
                    pet::Perch::Saved,
                    rows::saved_card(hosts, &ctx),
                )
            })
        }))
        .into_element()
}

/// No connections yet (§6.1 states): the grid, the app's mark, what Home
/// will show, and Add connection.
fn empty_page(bar: BarMetrics, add: Rc<dyn Fn()>) -> AnyElement {
    view()
        .absolute()
        .inset_0()
        .child(stage())
        .child(
            view()
                .absolute()
                .left(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .top(move || bar::space(bar))
                .pb_safe_area()
                .items_center()
                .justify_center()
                .child(
                    view()
                        .w_full()
                        .max_w(px(number("content-max-width")))
                        .items_center()
                        .child(
                            empty(t!("No connections yet"))
                                .mark()
                                .message(t!(
                                    "Add a machine you can reach over SSH. Agents running there show up here."
                                ))
                                .action(
                                    button(t!("Add Connection"))
                                        .variant(Variant::Primary)
                                        .on_press(move || add())
                                        .test_id("home-add-empty"),
                                )
                                .test_id("home-empty"),
                        )
                        .child(
                            eyebrow(text(t!("The pit never closes")))
                                .accessibility_elements_hidden(true)
                                .mt(px(8.0)),
                        ),
                ),
        )
        .into_element()
}

/// The brand's grid under the empty page (`HomeStage`'s, the host's
/// `moshpit/signal-grid`), in token `grid-line` for the appearance now.
fn stage() -> AnyElement {
    native("moshpit/signal-grid")
        .absolute()
        .inset_0()
        .pointer_events(PointerEvents::None)
        .prop("color", || {
            json!(theme::hex_string(nsh_core::color::Rgba::from(color(
                token("grid-line")
            ))))
        })
        .prop("lineWidth", json!(0.6))
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HomeState {
        HomeState {
            connections: vec![model::Connection {
                id: "C0FFEE00-0000-4000-8000-000000000001".into(),
                name: "studio".into(),
                initial: "S".into(),
                user: "cluas".into(),
                host: "studio.local".into(),
                link: model::Link::Live,
                connected_at: Some(1.0),
                control: Some(model::Control {
                    mux: model::Mux::Tmux,
                    attached: true,
                    sessions: vec![model::SessionInfo {
                        id: "$0".into(),
                        name: "work".into(),
                    }],
                    ..model::Control::default()
                }),
                ..model::Connection::default()
            }],
            version: "v1.0.9 (1)".into(),
            sidebar: false,
        }
    }

    #[test]
    fn messages_round_trip() {
        let message = Message::Home(Box::new(sample()));
        assert_eq!(Message::parse(&message.to_json()), Ok(message));
        let trees = Message::Trees(vec![TreeState {
            connection: "a".into(),
            collapsed: vec!["$1".into()],
            expanded: vec!["@2".into()],
            idle: true,
        }]);
        assert_eq!(Message::parse(&trees.to_json()), Ok(trees));
        // A host that keeps no idle flag yet: the group is shut.
        assert_eq!(
            Message::parse(
                r#"{"op":"trees","trees":[{"connection":"a","collapsed":[],"expanded":[]}]}"#
            ),
            Ok(Message::Trees(vec![TreeState {
                connection: "a".into(),
                ..TreeState::default()
            }]))
        );
        assert_eq!(
            Message::parse(r#"{"op":"home","connections":[],"version":""}"#),
            Ok(Message::Home(Box::default()))
        );
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        let good: serde_json::Value =
            serde_json::from_str(&Message::Home(Box::new(sample())).to_json()).unwrap();
        let mut bad = good.clone();
        bad["connections"][0]["link"] = json!("asleep");
        assert!(
            Message::parse(&bad.to_string())
                .unwrap_err()
                .contains("asleep")
        );
        let mut bad = good.clone();
        bad["connections"][0]["control"]["mux"] = json!("screen");
        assert!(
            Message::parse(&bad.to_string())
                .unwrap_err()
                .contains("screen")
        );
        let mut bad = good.clone();
        bad["colour"] = json!(1);
        assert!(
            Message::parse(&bad.to_string())
                .unwrap_err()
                .contains("colour")
        );
        let mut bad = good;
        bad.as_object_mut().unwrap().remove("version");
        assert!(
            Message::parse(&bad.to_string())
                .unwrap_err()
                .contains("version")
        );
        assert!(Message::parse(r#"{"op":"settings"}"#).is_err());
        assert!(
            Message::parse(r#"{"op":"trees"}"#)
                .unwrap_err()
                .contains("trees")
        );
    }

    #[test]
    fn intents_as_the_host_reads_them() {
        assert_eq!(Intent::Settings.event(), json!({"ev": "settings"}));
        assert_eq!(
            Intent::Enter("a".into()).event(),
            json!({"ev": "enter", "connection": "a"})
        );
        assert_eq!(
            Intent::Pane {
                connection: "a".into(),
                pane: "%3".into()
            }
            .event(),
            json!({"ev": "pane", "connection": "a", "pane": "%3"})
        );
        assert_eq!(
            Intent::Sessions("a".into()).event(),
            json!({"ev": "sessions", "connection": "a"})
        );
        assert_eq!(
            Intent::KillWindow {
                connection: "a".into(),
                window: "@1".into()
            }
            .event(),
            json!({"ev": "kill-window", "connection": "a", "window": "@1"})
        );
    }
}
