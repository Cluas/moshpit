//! The terminal's pickers, their host side (`MuxPickerNSHPage.swift`): the
//! breadcrumb's sessions, windows and panes on W11h's shared screen
//! (`ui/src/terminal/picker.rs`), pushed as a page of its own (Android has
//! no sheet). The list is the session's tree — tmux's or herdr's, read the
//! same way — posted again whenever a session event changes it; what the
//! person picks goes to whichever multiplexer the session runs.
//!
//! The screen asks for a new window's or session's name through the host's
//! text prompt (`{"ev": "prompt"}`: the platform's dialog with a field,
//! answered `{"op": "prompted"}`). A row's rename and kill come from a
//! long-press menu that is the host's on iOS (`NSHContextMenus` on the
//! surface); Android has none on an NSH surface yet, so rows are picked and
//! made here, and renamed or killed from Home's tree.

use std::collections::BTreeMap;

use moshpit_seal::foundation::uuid::Uuid;
use moshpit_session::{HerdrAction, TmuxAction};
use moshpit_ui::terminal::model::{Mux, Signal};
use moshpit_ui::terminal::picker::{Kind, Message, Picker, Row};
use serde_json::{Value, json};

use crate::app::sessions::Conn;
use crate::app::{App, Core, PageKind, Question, text};
use crate::platform::Ask;

/// What a picker page keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickerPage {
    pub connection: Uuid,
    pub kind: Kind,
}

/// A session's tree, whichever multiplexer drew it: the lists in the
/// pickers' orders.
struct Tree {
    mux: Mux,
    /// id, name, attached elsewhere.
    sessions: Vec<(String, String, bool)>,
    /// The active session's windows by index: id, index, name, panes.
    windows: Vec<(String, i64, String, i64)>,
    /// Every pane: id, window, index, command, active.
    panes: Vec<(String, String, i64, String, bool)>,
    active_session: Option<String>,
    active_window: Option<String>,
    active_pane: Option<String>,
    /// A pane's agent stamp: its state (`attention`, …) and name.
    hooks: BTreeMap<String, (Option<String>, Option<String>)>,
}

fn tree(conn: &Conn) -> Option<Tree> {
    if let Some(tmux) = &conn.tmux {
        let s = &tmux.snapshot;
        return Some(Tree {
            mux: Mux::Tmux,
            sessions: s
                .sorted_sessions()
                .into_iter()
                .map(|x| (x.id.clone(), x.name.clone(), x.is_attached))
                .collect(),
            windows: s
                .sorted_windows()
                .into_iter()
                .map(|w| (w.id.clone(), w.index, w.name.clone(), w.pane_count))
                .collect(),
            panes: s
                .panes
                .values()
                .map(|p| {
                    let p = p.clone();
                    (p.id, p.window_id, p.index, p.command, p.is_active)
                })
                .collect(),
            active_session: s.active_session_id.clone(),
            active_window: s.active_window_id.clone(),
            active_pane: s.active_pane_id.clone(),
            hooks: tmux
                .agent_hooks
                .iter()
                .map(|(pane, h)| (pane.clone(), (h.state.clone(), h.agent.clone())))
                .collect(),
        });
    }
    let herdr = conn.herdr.as_ref()?;
    let s = &herdr.snapshot;
    let windows = match &s.active_session_id {
        Some(session) => s.windows_in_session(session),
        None => {
            let mut all: Vec<_> = s.windows.values().collect();
            all.sort_by_key(|w| w.index);
            all
        }
    };
    Some(Tree {
        mux: Mux::Herdr,
        sessions: s
            .sessions
            .values()
            .map(|x| (x.id.clone(), x.name.clone(), x.is_attached))
            .collect(),
        windows: windows
            .into_iter()
            .map(|w| (w.id.clone(), w.index, w.name.clone(), w.pane_count))
            .collect(),
        panes: s
            .panes
            .values()
            .map(|p| {
                let p = p.clone();
                (p.id, p.window_id, p.index, p.command, p.is_active)
            })
            .collect(),
        active_session: s.active_session_id.clone(),
        active_window: s.active_window_id.clone(),
        active_pane: s.active_pane_id.clone(),
        hooks: herdr
            .agent_hooks
            .iter()
            .map(|(pane, h)| {
                (
                    pane.clone(),
                    (h.state.map(|s| s.as_str().to_owned()), h.agent.clone()),
                )
            })
            .collect(),
    })
}

/// A stamp's state as the pickers light it (`AgentSignal(state)`).
fn signal(state: Option<&str>) -> Option<Signal> {
    match state? {
        "attention" => Some(Signal::Attention),
        "working" => Some(Signal::Working),
        "done" => Some(Signal::Done),
        _ => None,
    }
}

/// A window's panes together (`AgentSignal.aggregate`): attention, else
/// working, else nothing.
fn aggregate(signals: impl IntoIterator<Item = Option<Signal>>) -> Option<Signal> {
    let present: Vec<Signal> = signals.into_iter().flatten().collect();
    if present.contains(&Signal::Attention) {
        Some(Signal::Attention)
    } else if present.contains(&Signal::Working) {
        Some(Signal::Working)
    } else {
        None
    }
}

/// The list of `kind` (`MuxPickerState`), as the SwiftUI sheets read it.
fn picker(tree: &Tree, kind: Kind) -> Picker {
    let state = |pane: &str| tree.hooks.get(pane).and_then(|(s, _)| s.as_deref());
    let rows = match kind {
        Kind::Windows => tree
            .windows
            .iter()
            .map(|(id, index, name, panes)| {
                let mine: Vec<_> = tree.panes.iter().filter(|p| p.1 == *id).collect();
                let command = mine
                    .iter()
                    .find(|p| p.4)
                    .or_else(|| mine.first())
                    .map(|p| p.3.clone())
                    .unwrap_or_default();
                Row {
                    id: id.clone(),
                    index: u32::try_from(*index).ok(),
                    name: name.clone(),
                    panes: u32::try_from(*panes).ok(),
                    command,
                    signal: aggregate(mine.iter().map(|p| signal(state(&p.0)))),
                    ..Row::default()
                }
            })
            .collect(),
        Kind::Sessions => {
            let active_window = tree
                .active_window
                .as_ref()
                .and_then(|id| tree.windows.iter().find(|w| w.0 == *id));
            tree.sessions
                .iter()
                .map(|(id, name, attached)| {
                    let mut row = Row {
                        id: id.clone(),
                        name: name.clone(),
                        attached: *attached,
                        ..Row::default()
                    };
                    if tree.active_session.as_ref() == Some(id) {
                        row.windows = u32::try_from(tree.windows.len()).ok();
                        row.window = active_window.map(|w| format!("{}:{}", w.1, w.2));
                    }
                    row
                })
                .collect()
        }
        Kind::Panes => {
            let mut panes: Vec<_> = tree
                .panes
                .iter()
                .filter(|p| tree.active_window.as_ref() == Some(&p.1))
                .collect();
            panes.sort_by_key(|p| p.2);
            panes
                .into_iter()
                .map(|(id, _, index, command, _)| Row {
                    id: id.clone(),
                    index: u32::try_from(*index).ok(),
                    command: command.clone(),
                    agent: tree.hooks.get(id).and_then(|(_, a)| a.clone()),
                    signal: signal(state(id)),
                    ..Row::default()
                })
                .collect()
        }
    };
    Picker {
        kind,
        mux: tree.mux,
        rows,
        active: match kind {
            Kind::Windows => tree.active_window.clone(),
            Kind::Sessions => tree.active_session.clone(),
            Kind::Panes => tree.active_pane.clone(),
        },
        send: None,
    }
}

/// `{"op": "picker", …}` for `page` (an empty list while the tree isn't
/// there).
pub fn message(app: &App, page: &PickerPage) -> String {
    let list = app
        .sessions
        .conn(page.connection)
        .and_then(tree)
        .map(|t| picker(&t, page.kind))
        .unwrap_or(Picker {
            kind: page.kind,
            ..Picker::default()
        });
    let mut value = serde_json::to_value(list).expect("a picker serializes");
    value["op"] = json!("picker");
    let json = value.to_string();
    debug_assert!(Message::parse(&json).is_ok(), "{json}");
    json
}

/// The `n`-th window in the picker's order (⌘1–⌘9 on a keyboard), if the
/// tree has one.
pub fn nth_window(conn: &Conn, n: usize) -> Option<String> {
    let tree = tree(conn)?;
    tree.windows.get(n.checked_sub(1)?).map(|w| w.0.clone())
}

impl App {
    /// The breadcrumb's segment `crumb` (`session`, `window`, `pane`): its
    /// picker, over the live tree only (the remembered one is inert).
    pub(crate) fn open_picker(&mut self, core: &Core, connection: Uuid, crumb: &str) {
        let kind = match crumb {
            "session" => Kind::Sessions,
            "window" => Kind::Windows,
            "pane" => Kind::Panes,
            _ => return,
        };
        let live = self
            .sessions
            .conn(connection)
            .is_some_and(|c| c.state.is_live() && (c.tmux.is_some() || c.herdr.is_some()));
        if !live {
            return;
        }
        self.open(
            core,
            PageKind::MuxPicker(PickerPage { connection, kind }),
            true,
        );
    }

    /// One of the tree's actions on whichever multiplexer `connection`
    /// runs.
    pub(crate) fn mux_action(&self, connection: Uuid, tmux: TmuxAction, herdr: HerdrAction) {
        if self.sessions.is_herdr(connection) {
            self.sessions.herdr(connection, herdr);
        } else {
            self.sessions.tmux(connection, tmux);
        }
    }
}

/// What a picker page told the host.
pub fn on_event(app: &mut App, _core: &Core, surface: u64, ev: &str, event: &Value) {
    let Some(&PageKind::MuxPicker(page)) = app.page_mut(surface).map(|p| &p.kind) else {
        return;
    };
    let c = page.connection;
    let id = text(event, "id");
    let name = event["name"]
        .as_str()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    match ev {
        "select" => {
            let Some(id) = id else { return };
            match page.kind {
                Kind::Windows => app.mux_action(
                    c,
                    TmuxAction::SelectWindow(id.clone()),
                    HerdrAction::SelectWindow(id),
                ),
                Kind::Sessions => app.mux_action(
                    c,
                    TmuxAction::SelectSession(id.clone()),
                    HerdrAction::SelectSession(id),
                ),
                Kind::Panes => app.mux_action(
                    c,
                    TmuxAction::SelectPane(id.clone()),
                    HerdrAction::SelectPane(id),
                ),
            }
            app.close(surface);
        }
        "new" => match page.kind {
            Kind::Windows => {
                app.mux_action(
                    c,
                    TmuxAction::NewWindow { name: name.clone() },
                    HerdrAction::NewWindow { name },
                );
                // The new window becomes active: the person lands on it.
                app.close(surface);
            }
            Kind::Sessions => {
                app.mux_action(
                    c,
                    TmuxAction::NewSession { name: name.clone() },
                    HerdrAction::NewSession { name },
                );
                app.close(surface);
            }
            Kind::Panes => app.mux_action(c, TmuxAction::NewPane, HerdrAction::NewPane),
        },
        "rename" => {
            let (Some(id), Some(name)) = (id, name) else {
                return;
            };
            match page.kind {
                Kind::Windows => app.mux_action(
                    c,
                    TmuxAction::RenameWindow {
                        window: id.clone(),
                        name: name.clone(),
                    },
                    HerdrAction::RenameWindow { window: id, name },
                ),
                Kind::Sessions => app.mux_action(
                    c,
                    TmuxAction::RenameSession {
                        session: id.clone(),
                        name: name.clone(),
                    },
                    HerdrAction::RenameSession { session: id, name },
                ),
                Kind::Panes => {}
            }
        }
        "kill" => {
            let Some(id) = id else { return };
            match page.kind {
                Kind::Windows => app.mux_action(
                    c,
                    TmuxAction::KillWindow(id.clone()),
                    HerdrAction::KillWindow(id),
                ),
                Kind::Sessions => app.mux_action(
                    c,
                    TmuxAction::KillSession(id.clone()),
                    HerdrAction::KillSession(id),
                ),
                Kind::Panes => {}
            }
        }
        // The screen's question with a text field: the platform's dialog.
        "prompt" => {
            let Some(prompt) = event["id"].as_u64() else {
                return;
            };
            let title = text(event, "title").unwrap_or_default();
            let start = text(event, "text").unwrap_or_default();
            let ok = text(event, "confirm").unwrap_or_else(|| "OK".to_owned());
            app.ask(
                Question::ScreenPrompt {
                    surface,
                    id: prompt,
                },
                |token| Ask::Prompt {
                    token,
                    title,
                    text: start,
                    ok,
                    multiline: false,
                },
            );
        }
        "back" | "close" | "done" | "cancel" => app.close(surface),
        _ => {}
    }
}

/// The platform's dialog answered a screen's prompt: back to the screen
/// (`text` none when cancelled).
pub fn prompted(app: &mut App, surface: u64, id: u64, text: Option<String>) {
    app.post(
        surface,
        json!({"op": "prompted", "id": id, "text": text}).to_string(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        Tree {
            mux: Mux::Tmux,
            sessions: vec![
                ("$0".into(), "main".into(), false),
                ("$1".into(), "build".into(), true),
            ],
            windows: vec![
                ("@1".into(), 1, "shell".into(), 1),
                ("@2".into(), 2, "agents".into(), 2),
            ],
            panes: vec![
                ("%1".into(), "@1".into(), 0, "zsh".into(), true),
                ("%2".into(), "@2".into(), 1, "claude".into(), false),
                ("%3".into(), "@2".into(), 0, "vim".into(), true),
            ],
            active_session: Some("$0".into()),
            active_window: Some("@2".into()),
            active_pane: Some("%3".into()),
            hooks: [(
                "%2".to_owned(),
                (Some("attention".to_owned()), Some("claude".to_owned())),
            )]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn the_lists_read_as_the_sheets_read_them() {
        let tree = sample();
        let windows = picker(&tree, Kind::Windows);
        assert_eq!(windows.active.as_deref(), Some("@2"));
        assert_eq!(windows.rows[1].command, "vim", "a window's active pane");
        assert_eq!(windows.rows[1].signal, Some(Signal::Attention));
        assert_eq!(windows.rows[0].signal, None);
        let sessions = picker(&tree, Kind::Sessions);
        assert_eq!(sessions.rows[0].windows, Some(2));
        assert_eq!(sessions.rows[0].window.as_deref(), Some("2:agents"));
        assert!(sessions.rows[1].attached && sessions.rows[1].window.is_none());
        let panes = picker(&tree, Kind::Panes);
        let ids: Vec<_> = panes.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["%3", "%2"], "the active window's, by index");
        assert_eq!(panes.rows[1].agent.as_deref(), Some("claude"));
        let json = {
            let mut v = serde_json::to_value(&panes).unwrap();
            v["op"] = json!("picker");
            v.to_string()
        };
        assert!(Message::parse(&json).is_ok(), "{json}");
    }

    #[test]
    fn done_alone_lights_no_window() {
        assert_eq!(aggregate([Some(Signal::Done), None]), None);
        assert_eq!(
            aggregate([Some(Signal::Working), Some(Signal::Attention)]),
            Some(Signal::Attention)
        );
    }
}
