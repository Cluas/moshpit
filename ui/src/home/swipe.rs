//! The swipe tray of Home's rows (`SwipeTrayRow`): a row's actions drawn
//! inside the row, on the card's own surface. A list's own swipe actions
//! slide the whole cell away and put their buttons on the list behind it,
//! which tears a card built from rows in two; here only the row's content
//! moves, and the actions come up in the room it leaves: 44 pt squares in
//! the tokens' soft tints (§6.1: a card's head, a saved host's row).
//!
//! The drag is the platform's own pan, horizontal only
//! ([`Interactive::pan_values`] on x alone: it begins only for a movement
//! more horizontal than vertical, so a vertical drag on a row scrolls the
//! list), moving the content natively with no engine work per frame. Where
//! it ends, and how fast, decides: past halfway open, else closed, on the
//! structural spring (§5.1). One row of the page is open at a time; tapping an open row's
//! content closes it rather than acting.
//!
//! The multiplexer's tree rows have trays too (B499: Home's pane rows,
//! and the sessions page's sessions, windows and panes): their two or
//! three most used actions ([`pane_tray`], [`window_tray`],
//! [`session_tray`]), the long press keeping the whole list. Each action
//! tells the host the intent the long-press menu's item does, with the
//! row it came from (`"row"`: the row's identifier), so the rename's
//! prompt and the kill's confirmation are the menu's, the confirmation
//! hanging from that row.

use std::rc::Rc;

use nsh_core::prelude::*;
use serde_json::json;

use super::{
    Intent, Tell,
    model::{Control, Mux},
};
use crate::ui::{
    icon::{IconSize, Tone, icon},
    motion::structural_spring,
    tokens::{number, token},
};

/// An action's square: 44 pt (the touch target), its symbol at the row's
/// size, the squares 8 pt apart and 12 pt in from the card's edge.
const BUTTON: f32 = 44.0;
const SPACING: f32 = 8.0;
const EDGE: f32 = 12.0;

/// An action's tint: its soft fill and its ink, as the tokens pair them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    Accent,
    Warn,
    Danger,
}

impl Tint {
    fn fill(self) -> &'static str {
        match self {
            Tint::Accent => "primary-soft",
            Tint::Warn => "warning-soft",
            Tint::Danger => "destructive-soft",
        }
    }

    fn tone(self) -> Tone {
        match self {
            Tint::Accent => Tone::Primary,
            Tint::Warn => Tone::Warning,
            Tint::Danger => Tone::Destructive,
        }
    }
}

/// One action of a tray: a tinted square with a symbol, its title what a
/// screen reader says (the long press carries the words).
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub title: String,
    pub symbol: &'static str,
    pub tint: Tint,
    pub intent: Intent,
    /// The identifier of the row it is on (`home-pane-<connection>-<pane>`,
    /// …), told with the intent: what the host hangs the action's
    /// confirmation from (B499). None: the host needs no row.
    pub row: Option<String>,
}

impl Action {
    /// What a press tells the host: the intent's event, and the row.
    pub fn event(&self) -> serde_json::Value {
        let mut event = self.intent.event();
        if let Some(row) = &self.row {
            event["row"] = json!(row);
        }
        event
    }
}

/// The tray's width for `count` actions.
pub(crate) fn tray_width(count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    count as f32 * BUTTON + (count - 1) as f32 * SPACING + 2.0 * EDGE
}

/// How far past open the content may be dragged.
const OVERDRAG: f32 = 14.0;

/// `content` with a tray of `actions` behind its trailing edge. `key`
/// names the row among the page's (`open` holds the open one's).
pub(crate) fn tray_row(
    key: String,
    open: Signal<Option<String>>,
    actions: Vec<Action>,
    tell: Tell,
    content: AnyElement,
) -> AnyElement {
    if actions.is_empty() {
        return content;
    }
    let tray = tray_width(actions.len());
    let offset = animated_value(0.0);
    // The content: never right of its place, at most a little past open.
    let shown = offset.interpolate(&[-(tray + OVERDRAG), 0.0], &[-(tray + OVERDRAG), 0.0]);
    // The tray's window opens from the trailing edge as the content moves;
    // the buttons stay put inside it.
    let window = offset.interpolate(&[-tray, 0.0], &[0.0, tray]);
    let keep = offset.interpolate(&[-tray, 0.0], &[0.0, -tray]);
    let fade = offset.interpolate(&[-tray, 0.0], &[1.0, 0.35]);
    let grow = offset.interpolate(&[-tray, 0.0], &[1.0, 0.88]);
    let is_open = computed({
        let key = key.clone();
        move || open.with(|o| o.as_deref() == Some(key.as_str()))
    });
    // Another row opened, or the page closed this one: settle closed.
    effect(move || {
        if !is_open.get() {
            drop(offset.spring(0.0, settle()));
        }
    });
    let close: Rc<dyn Fn()> = {
        let key = key.clone();
        Rc::new(move || {
            if open.with_untracked(|o| o.as_deref() == Some(key.as_str())) {
                open.set(None);
            }
        })
    };
    let on_end = {
        let key = key.clone();
        move |event: &PanEvent| {
            let at = event.value_x.unwrap_or(0.0);
            // Where the content would come to rest if the finger's speed
            // carried it a little further: past halfway is open.
            let projected = at + event.velocity_x * 0.1;
            let opens = -projected > tray / 2.0 && !event.cancelled;
            if opens {
                if open.with_untracked(|o| o.as_deref() != Some(key.as_str())) {
                    haptic(Haptic::Selection);
                }
                open.set(Some(key.clone()));
                drop(offset.spring(-tray, settle()));
            } else {
                if open.with_untracked(|o| o.as_deref() == Some(key.as_str())) {
                    open.set(None);
                }
                drop(offset.spring(0.0, settle()));
            }
        }
    };
    let on_start = {
        let key = key.clone();
        move |_: &PanEvent| {
            // Dragging one row closes any other.
            if open.with_untracked(|o| o.as_ref().is_some_and(|o| *o != key)) {
                open.set(None);
            }
        }
    };
    let buttons = actions
        .into_iter()
        .map(|action| button(action, close.clone(), tell.clone()))
        .collect::<Vec<_>>();
    let tray_view = view()
        .absolute()
        .top(px(0.0))
        .bottom(px(0.0))
        .right(px(0.0))
        .w(px(tray))
        .overflow_hidden()
        // Shut, the tray takes no touches: what is under it is the row's.
        .pointer_events(move || {
            if is_open.get() {
                PointerEvents::Auto
            } else {
                PointerEvents::None
            }
        })
        .bind_translate_x(&window)
        .child(
            view()
                .absolute()
                .inset_0()
                .flex_row()
                .items_center()
                .justify_end()
                .px(px(EDGE))
                .gap(px(SPACING))
                .bind_translate_x(&keep)
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(SPACING))
                        .origin_right()
                        .bind_opacity(&fade)
                        .bind_scale(&grow)
                        .children(buttons),
                ),
        );
    let closer = when(
        move || is_open.get(),
        move || {
            let close = close.clone();
            view().absolute().inset_0().on_press(move || close())
        },
    );
    view()
        .relative()
        .w_full()
        .overflow_hidden()
        .test_id(&format!("home-tray-{}", key.replace(':', "-")))
        .child(
            view()
                .w_full()
                .pan_values(Some(&offset), None)
                .on_pan_start(on_start)
                .on_pan_end(on_end)
                .bind_translate_x(&shown)
                .child(content)
                .child(closer),
        )
        .child(tray_view)
        .into_element()
}

/// The spring a tray settles on (§5.1: the structural spring, carrying
/// the finger's speed; it lands at once with reduced motion).
fn settle() -> Spring {
    structural_spring(None).unwrap_or_else(|| Spring::new(1.0).damping_ratio(1.0))
}

/// One action's square: its symbol in its tint's ink on the tint's soft
/// fill.
fn button(action: Action, close: Rc<dyn Fn()>, tell: Tell) -> AnyElement {
    let event = action.event();
    let Action {
        title,
        symbol,
        tint,
        ..
    } = action;
    view()
        .w(px(BUTTON))
        .h(px(BUTTON))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(px(number("radius")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || token(tint.fill()))
        .role(AccessibilityRole::Button)
        .label(title)
        .active(|s| s.opacity(0.75))
        .on_press(move || {
            close();
            tell(event.clone());
        })
        .child(icon(symbol).size(IconSize::Md).tone(tint.tone()))
        .into_element()
}

// MARK: The tree's trays (B499)

/// What a tree action is called, worded at its tray (the long-press
/// menu's words: tmux kills, herdr closes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Rename,
    NewPane,
    RemoveWorktree,
    KillPane,
    KillWindow,
    KillSession,
}

impl Label {
    /// Its words on `mux`.
    pub fn words(self, mux: Mux) -> String {
        let herdr = mux == Mux::Herdr;
        let kill = if herdr { t!("Close") } else { t!("Kill") };
        match self {
            Label::Rename => t!("Rename"),
            Label::NewPane => t!("New Pane"),
            Label::RemoveWorktree => t!("Remove Worktree"),
            Label::KillPane => t!("%@ %@", arg1 = kill, arg2 = t!("Pane")),
            Label::KillWindow => {
                t!(
                    "%@ %@",
                    arg1 = kill,
                    arg2 = if herdr { t!("Tab") } else { t!("Window") }
                )
            }
            Label::KillSession => t!(
                "%@ %@",
                arg1 = kill,
                arg2 = if herdr {
                    t!("Workspace")
                } else {
                    t!("Session")
                }
            ),
        }
    }
}

/// One action of a tree row's tray, before its words: what it is called,
/// its symbol and tint, the menu's intent, and the row it is on.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeAction {
    pub label: Label,
    pub symbol: &'static str,
    pub tint: Tint,
    pub intent: Intent,
    pub row: String,
}

impl TreeAction {
    /// The tray's action, worded for `mux`.
    pub fn action(self, mux: Mux) -> Action {
        Action {
            title: self.label.words(mux),
            symbol: self.symbol,
            tint: self.tint,
            intent: self.intent,
            row: Some(self.row),
        }
    }

    /// What a press tells the host ([`Action::event`]).
    pub fn event(&self) -> serde_json::Value {
        let mut event = self.intent.event();
        event["row"] = json!(self.row);
        event
    }
}

/// `actions` worded for `mux`: a tray's.
pub fn worded(actions: Vec<TreeAction>, mux: Mux) -> Vec<Action> {
    actions.into_iter().map(|a| a.action(mux)).collect()
}

/// What a pane row's Rename renames: what its title is named after —
/// tmux's window, herdr's workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Renames {
    Window(String),
    Session(String),
}

/// Pane `pane`'s Rename on `control`'s tree, if it has one (a pane the
/// tree doesn't know has none).
pub fn pane_renames(control: &Control, pane: &str) -> Option<Renames> {
    let window = &control.panes.iter().find(|p| p.id == pane)?.window;
    match control.mux {
        Mux::Tmux => Some(Renames::Window(window.clone())),
        Mux::Herdr => {
            let session = &control.windows.iter().find(|w| w.id == *window)?.session;
            Some(Renames::Session(session.clone()))
        }
    }
}

fn tree_action(
    label: Label,
    symbol: &'static str,
    tint: Tint,
    intent: Intent,
    row: &str,
) -> TreeAction {
    TreeAction {
        label,
        symbol,
        tint,
        intent,
        row: row.to_owned(),
    }
}

/// A pane row's tray (Home's agents and other panes, the sessions page's
/// panes): Rename (where its title comes from: [`pane_renames`]), and the
/// kill.
pub fn pane_tray(connection: &str, pane: &str, renames: Option<Renames>) -> Vec<TreeAction> {
    let row = format!("home-pane-{connection}-{pane}");
    let connection = connection.to_owned();
    let mut actions = Vec::new();
    let renaming = match renames {
        Some(Renames::Window(window)) => Some(Intent::RenameWindow {
            connection: connection.clone(),
            window,
        }),
        Some(Renames::Session(session)) => Some(Intent::RenameSession {
            connection: connection.clone(),
            session,
        }),
        None => None,
    };
    if let Some(intent) = renaming {
        actions.push(tree_action(
            Label::Rename,
            "pencil",
            Tint::Accent,
            intent,
            &row,
        ));
    }
    actions.push(tree_action(
        Label::KillPane,
        "xmark.circle",
        Tint::Danger,
        Intent::KillPane {
            connection,
            pane: pane.to_owned(),
        },
        &row,
    ));
    actions
}

/// A window's (a tab's) tray on the sessions page: Rename, New Pane, the
/// kill.
pub fn window_tray(connection: &str, window: &str) -> Vec<TreeAction> {
    let row = format!("home-window-{connection}-{window}");
    let (connection, window) = (connection.to_owned(), window.to_owned());
    vec![
        tree_action(
            Label::Rename,
            "pencil",
            Tint::Accent,
            Intent::RenameWindow {
                connection: connection.clone(),
                window: window.clone(),
            },
            &row,
        ),
        tree_action(
            Label::NewPane,
            "rectangle.split.2x1",
            Tint::Accent,
            Intent::NewPane {
                connection: connection.clone(),
                window: window.clone(),
            },
            &row,
        ),
        tree_action(
            Label::KillWindow,
            "xmark.circle",
            Tint::Danger,
            Intent::KillWindow { connection, window },
            &row,
        ),
    ]
}

/// A session's (a workspace's) tray on the sessions page: Rename, Remove
/// Worktree for a worktree's, the kill.
pub fn session_tray(connection: &str, session: &str, worktree: bool) -> Vec<TreeAction> {
    let row = format!("home-session-{connection}-{session}");
    let (connection, session) = (connection.to_owned(), session.to_owned());
    let mut actions = vec![tree_action(
        Label::Rename,
        "pencil",
        Tint::Accent,
        Intent::RenameSession {
            connection: connection.clone(),
            session: session.clone(),
        },
        &row,
    )];
    if worktree {
        actions.push(tree_action(
            Label::RemoveWorktree,
            "trash.slash",
            Tint::Danger,
            Intent::RemoveWorktree {
                connection: connection.clone(),
                session: session.clone(),
            },
            &row,
        ));
    }
    actions.push(tree_action(
        Label::KillSession,
        "xmark.circle",
        Tint::Danger,
        Intent::KillSession {
            connection,
            session,
        },
        &row,
    ));
    actions
}
