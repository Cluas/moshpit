//! Home's host side (`Moshpit/UI/Home/HomeNSHPage.swift`): what the page
//! shows (the saved connections and their sessions, read from the store
//! and the registry) and what its intents do.

use moshpit_seal::foundation::uuid::Uuid;
use moshpit_session::{HerdrAction, SessionState, TmuxAction};
use moshpit_store::connections::ConnectionProtocol;
use moshpit_ui::home::model::{
    Connection, Control, HomeState, Hook, Link, Mux, PaneInfo, SessionInfo, Transport, WindowInfo,
};
use moshpit_ui::home::{Message, TreeState};
use serde_json::Value;

use crate::app::sessions::Conn;
use crate::app::{App, Core, PageKind, Question, form, text, uuid};
use crate::platform::Ask;

/// `{"op": "home", …}`.
pub fn message(app: &App) -> String {
    Message::Home(Box::new(state(app))).to_json()
}

/// What Home shows now.
pub fn state(app: &App) -> HomeState {
    // A mosh card's port is Settings' first, the one its connect asks for
    // (B430).
    let udp_port = u32::try_from(app.settings.udp_range().0).unwrap_or(60001);
    let connections = app
        .connections
        .connections()
        .iter()
        .map(|saved| {
            let name = saved.display_name();
            let conn = app.sessions.conn(saved.id);
            Connection {
                id: saved.id.to_string(),
                initial: name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default(),
                name,
                user: saved.username.clone(),
                host: saved.host.clone(),
                mosh: saved.connection_protocol == ConnectionProtocol::Mosh,
                udp_port,
                link: conn.map_or(Link::Saved, link),
                transport: conn.map_or(Transport::Live, transport),
                degraded: conn.is_some_and(|c| {
                    c.degrade == Some(moshpit_session::Degrade::MoshServerMissing)
                }),
                connected_at: conn.and_then(|c| c.connected_at),
                roaming: conn.is_some_and(|c| c.roaming),
                disconnecting: conn.is_some_and(|c| c.disconnecting),
                control: conn.and_then(control),
                // The shared builder (moshpit_app::home) fills these.
                lost_at: None,
                failure: None,
            }
        })
        .collect();
    HomeState {
        connections,
        version: app.options.version.clone(),
        sidebar: false,
    }
}

/// Where a session is (`moshpit_app::home::link`, which iOS reads).
fn link(conn: &Conn) -> Link {
    match conn.state {
        SessionState::Failed(_) | SessionState::Ended(_) => return Link::Dead,
        SessionState::Connecting => return Link::Connecting,
        SessionState::Reconnecting => return Link::Reconnecting,
        SessionState::Connected => {}
    }
    if conn.attach_stalled {
        return Link::Stalled;
    }
    if let Some(tmux) = &conn.tmux
        && !tmux.snapshot.is_attached
        && !tmux.snapshot.ever_attached
    {
        return Link::Attaching;
    }
    Link::Live
}

fn transport(conn: &Conn) -> Transport {
    match conn.state {
        SessionState::Connecting => Transport::Connecting,
        SessionState::Reconnecting => Transport::Reconnecting,
        SessionState::Failed(_) | SessionState::Ended(_) => Transport::Offline,
        SessionState::Connected => Transport::Live,
    }
}

/// herdr's tree, as the cards draw it (as `moshpit_app::home` does for
/// iOS): workspaces, tabs and panes, the agents' stamps, the version skew
/// that stops every command, the worktree workspaces.
fn herdr_control(conn: &Conn) -> Option<Control> {
    let herdr = conn.herdr.as_ref()?;
    let snapshot = &herdr.snapshot;
    let mismatch = herdr.protocol_mismatch.as_ref().map(|message| {
        let skew = herdr.version_skew.as_ref();
        moshpit_ui::home::model::Mismatch {
            notice: skew.map_or_else(|| message.clone(), |s| s.notice().english()),
            restart: !skew.map_or_else(
                || moshpit_herdr::snapshot::client_is_older(message),
                |s| s.client_is_older(),
            ),
            restarting: herdr.restarting,
        }
    });
    Some(Control {
        mux: Mux::Herdr,
        attached: snapshot.is_attached,
        refreshing: herdr.refreshing,
        sessions: snapshot
            .sessions
            .values()
            .map(|s| SessionInfo {
                id: s.id.clone(),
                name: s.name.clone(),
            })
            .collect(),
        windows: snapshot
            .windows
            .values()
            .map(|w| WindowInfo {
                id: w.id.clone(),
                session: w.session_id.clone(),
                index: w.index,
                name: w.name.clone(),
                pane_count: w.pane_count,
            })
            .collect(),
        panes: snapshot
            .panes
            .values()
            .map(|p| PaneInfo {
                id: p.id.clone(),
                window: p.window_id.clone(),
                index: p.index,
                command: p.command.clone(),
                path: herdr.pane_cwds.get(&p.id).cloned(),
            })
            .collect(),
        active_session: snapshot.active_session_id.clone(),
        active_window: snapshot.active_window_id.clone(),
        active_pane: snapshot.active_pane_id.clone(),
        hooks: herdr
            .agent_hooks
            .iter()
            .map(|(pane, hook)| Hook {
                pane: pane.clone(),
                state: hook.state.map(|s| s.as_str().to_owned()),
                agent: hook.agent.clone(),
                since: None,
            })
            .collect(),
        mismatch,
        worktrees: herdr
            .worktree_repos
            .iter()
            .map(|(session, repo)| moshpit_ui::home::model::Worktree {
                session: session.clone(),
                repo: repo.clone(),
            })
            .collect(),
    })
}

/// tmux's tree (herdr's, [`herdr_control`]), as the cards draw it.
fn control(conn: &Conn) -> Option<Control> {
    let Some(tmux) = conn.tmux.as_ref() else {
        return herdr_control(conn);
    };
    let snapshot = &tmux.snapshot;
    let mut sessions: Vec<_> = snapshot.sessions.values().collect();
    sessions.sort_by(|a, b| a.id.cmp(&b.id));
    let mut windows: Vec<_> = snapshot.windows.values().collect();
    windows.sort_by(|a, b| a.id.cmp(&b.id));
    let mut panes: Vec<_> = snapshot.panes.values().collect();
    panes.sort_by(|a, b| a.id.cmp(&b.id));
    Some(Control {
        mux: Mux::Tmux,
        attached: snapshot.is_attached,
        refreshing: tmux.refreshing,
        sessions: sessions
            .into_iter()
            .map(|s| SessionInfo {
                id: s.id.clone(),
                name: s.name.clone(),
            })
            .collect(),
        windows: windows
            .into_iter()
            .map(|w| WindowInfo {
                id: w.id.clone(),
                session: w.session_id.clone(),
                index: w.index,
                name: w.name.clone(),
                pane_count: w.pane_count,
            })
            .collect(),
        panes: panes
            .into_iter()
            .map(|p| PaneInfo {
                id: p.id.clone(),
                window: p.window_id.clone(),
                index: p.index,
                command: p.command.clone(),
                path: p.path.clone(),
            })
            .collect(),
        active_session: snapshot.active_session_id.clone(),
        active_window: snapshot.active_window_id.clone(),
        active_pane: snapshot.active_pane_id.clone(),
        hooks: tmux
            .agent_hooks
            .iter()
            .map(|(pane, hook)| Hook {
                pane: pane.clone(),
                state: hook.state.clone(),
                agent: hook.agent.clone(),
                since: hook.since,
            })
            .collect(),
        mismatch: None,
        worktrees: Vec::new(),
    })
}

/// What the page told the host.
pub fn on_event(app: &mut App, core: &Core, surface: u64, ev: &str, event: &Value) {
    let connection = uuid(event, "connection");
    match (ev, connection) {
        ("settings", _) => {
            app.open(core, PageKind::Settings, true);
        }
        ("add", _) => {
            let draft = form::FormDraft::new(app);
            app.open(core, PageKind::ConnectionForm(Box::new(draft)), true);
        }
        ("connect", Some(c)) => app.connect(c),
        ("enter", Some(c)) => app.enter(core, c),
        // All of a host's sessions (UI-DESIGN §6.2, R7's page): until it
        // lands, the terminal, whose pickers reach every session.
        ("sessions", Some(c)) => app.enter(core, c),
        ("window", Some(c)) => {
            if let Some(window) = text(event, "window") {
                app.mux_action(
                    c,
                    TmuxAction::SelectWindow(window.clone()),
                    HerdrAction::SelectWindow(window),
                );
            }
            app.enter(core, c);
        }
        ("pane", Some(c)) => {
            if let Some(pane) = text(event, "pane") {
                app.mux_action(
                    c,
                    TmuxAction::SelectPane(pane.clone()),
                    HerdrAction::SelectPane(pane),
                );
            }
            app.enter(core, c);
        }
        ("retry-attach", Some(c)) => app.sessions.reconnect(c),
        ("disconnect", Some(c)) => app.disconnect(core, c),
        ("refresh", Some(c)) => app.mux_action(c, TmuxAction::Refresh, HerdrAction::Refresh),
        ("new-session", Some(c)) => {
            let title = if app.sessions.is_herdr(c) {
                "New herdr workspace"
            } else {
                "New tmux session"
            };
            app.ask(Question::NewSession(c), |token| Ask::Prompt {
                token,
                title: title.to_owned(),
                text: String::new(),
                ok: "Create".to_owned(),
                multiline: false,
            });
        }
        ("edit", Some(c)) => {
            if let Some(saved) = app
                .connections
                .connections()
                .iter()
                .find(|s| s.id == c)
                .cloned()
            {
                let draft = form::FormDraft::editing(app, &saved);
                app.open(core, PageKind::ConnectionForm(Box::new(draft)), true);
            }
        }
        ("delete", Some(c)) => ask_delete(app, c),
        ("rename-session", Some(c)) => {
            if let Some(session) = text(event, "session") {
                app.ask(Question::RenameSession(c, session), |token| Ask::Prompt {
                    token,
                    title: "Rename session".to_owned(),
                    text: String::new(),
                    ok: "Rename".to_owned(),
                    multiline: false,
                });
            }
        }
        ("kill-session", Some(c)) => {
            if let Some(session) = text(event, "session") {
                let (title, ok) = if app.sessions.is_herdr(c) {
                    ("Close this herdr workspace?", "Close")
                } else {
                    ("Kill this tmux session?", "Kill")
                };
                app.ask(Question::KillSession(c, session), |token| Ask::Confirm {
                    token,
                    title: title.to_owned(),
                    message: "Everything running in it stops.".to_owned(),
                    ok: ok.to_owned(),
                    destructive: true,
                });
            }
        }
        ("new-window", Some(c)) => {
            if let Some(session) = text(event, "session") {
                app.mux_action(
                    c,
                    TmuxAction::NewWindowInSession {
                        session: session.clone(),
                        name: None,
                    },
                    HerdrAction::NewWindowInSession {
                        session,
                        name: None,
                    },
                );
            }
        }
        ("rename-window", Some(c)) => {
            if let Some(window) = text(event, "window") {
                app.ask(Question::RenameWindow(c, window), |token| Ask::Prompt {
                    token,
                    title: "Rename window".to_owned(),
                    text: String::new(),
                    ok: "Rename".to_owned(),
                    multiline: false,
                });
            }
        }
        ("kill-window", Some(c)) => {
            if let Some(window) = text(event, "window") {
                app.ask(Question::KillWindow(c, window), |token| Ask::Confirm {
                    token,
                    title: "Kill this window?".to_owned(),
                    message: "Its panes close.".to_owned(),
                    ok: "Kill".to_owned(),
                    destructive: true,
                });
            }
        }
        ("new-pane", Some(c)) => {
            if let Some(window) = text(event, "window") {
                app.mux_action(
                    c,
                    TmuxAction::NewPaneInWindow(window.clone()),
                    HerdrAction::NewPaneInWindow(window),
                );
            }
        }
        ("kill-pane", Some(c)) => {
            if let Some(pane) = text(event, "pane") {
                app.ask(Question::KillPane(c, pane), |token| Ask::Confirm {
                    token,
                    title: "Kill this pane?".to_owned(),
                    message: "What runs in it stops.".to_owned(),
                    ok: "Kill".to_owned(),
                    destructive: true,
                });
            }
        }
        ("tree", Some(c)) => {
            // Each page sends only what it keeps (Home its idle group, the
            // sessions page its sessions and windows): a field the event
            // leaves out keeps its value.
            let strings = |key: &str| -> Option<Vec<String>> {
                event[key].as_array().map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
            };
            let id = c.to_string();
            let mut tree = app
                .trees
                .iter()
                .find(|t| t.connection == id)
                .cloned()
                .unwrap_or(TreeState {
                    connection: id.clone(),
                    ..TreeState::default()
                });
            if let Some(collapsed) = strings("collapsed") {
                tree.collapsed = collapsed;
            }
            if let Some(expanded) = strings("expanded") {
                tree.expanded = expanded;
            }
            if let Some(idle) = event["idle"].as_bool() {
                tree.idle = idle;
            }
            app.trees.retain(|t| t.connection != id);
            app.trees.push(tree);
        }
        ("restart-server", Some(c)) => app.sessions.herdr(c, HerdrAction::RestartServer),
        ("new-agent-task" | "remove-worktree", _) => {
            app.platform
                .toast("herdr's controls are not on Android yet");
        }
        _ => {
            log::info!(target: "moshpit", "home: {ev}: nothing to do");
        }
    }
    let _ = surface;
}

/// A dialog's answer (`ok`).
pub fn answer(app: &mut App, core: &Core, question: Question, text: Option<String>) {
    let name = text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    match question {
        Question::DeleteConnection(c) => delete_connection(app, core, c),
        Question::NewSession(c) => app.mux_action(
            c,
            TmuxAction::NewSession { name: name.clone() },
            HerdrAction::NewSession { name },
        ),
        Question::RenameSession(c, session) => {
            if let Some(name) = name {
                app.mux_action(
                    c,
                    TmuxAction::RenameSession {
                        session: session.clone(),
                        name: name.clone(),
                    },
                    HerdrAction::RenameSession { session, name },
                );
            }
        }
        Question::KillSession(c, session) => app.mux_action(
            c,
            TmuxAction::KillSession(session.clone()),
            HerdrAction::KillSession(session),
        ),
        Question::RenameWindow(c, window) => {
            if let Some(name) = name {
                app.mux_action(
                    c,
                    TmuxAction::RenameWindow {
                        window: window.clone(),
                        name: name.clone(),
                    },
                    HerdrAction::RenameWindow { window, name },
                );
            }
        }
        Question::KillWindow(c, window) => app.mux_action(
            c,
            TmuxAction::KillWindow(window.clone()),
            HerdrAction::KillWindow(window),
        ),
        Question::KillPane(c, pane) => app.mux_action(
            c,
            TmuxAction::KillPane(pane.clone()),
            HerdrAction::KillPane(pane),
        ),
        Question::TrustHostKey(c, prompt) => app.answer_host_key(c, prompt, true),
        // The controller's own (`App::answer`).
        Question::ImportThemes | Question::ScreenPrompt { .. } => {}
    }
}

/// Asks whether to delete the saved connection `c` (Home's menu, the
/// connection form's Delete connection…).
pub(crate) fn ask_delete(app: &mut App, c: Uuid) {
    let name = app
        .connections
        .connections()
        .iter()
        .find(|s| s.id == c)
        .map(|s| s.display_name())
        .unwrap_or_default();
    app.ask(Question::DeleteConnection(c), |token| Ask::Confirm {
        token,
        title: format!("Delete {name}?"),
        message: "Its saved password or pasted key goes with it.".to_owned(),
        ok: "Delete".to_owned(),
        destructive: true,
    });
}

/// Deletes a saved connection: its session first, then the record, then
/// the keychain item it owns (a managed key's item belongs to the key).
fn delete_connection(app: &mut App, core: &Core, connection: Uuid) {
    if app.sessions.conn(connection).is_some() {
        app.disconnect(core, connection);
    }
    let Some(saved) = app
        .connections
        .connections()
        .iter()
        .find(|s| s.id == connection)
        .cloned()
    else {
        return;
    };
    let _ = saved;
    let keychain = app.keychain.clone();
    let fate = moshpit_store::secrets::delete_connection(
        connection,
        &mut app.connections,
        &mut app.keys,
        keychain.as_ref(),
    );
    log::info!(target: "moshpit", "connection deleted; its keychain item: {fate:?}");
    app.trees.retain(|t| t.connection != connection.to_string());
    app.refresh();
}
