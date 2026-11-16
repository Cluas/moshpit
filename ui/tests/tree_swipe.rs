//! B499: the multiplexer tree's rows swipe open on their most used
//! actions — Home's pane rows (an agent's, another pane's), the sessions
//! page's sessions, windows and panes — the long press keeping the whole
//! list. Each action tells the host the long-press menu's intent and the
//! row it came from (`"row"`), so the rename's prompt and the kill's
//! confirmation are the menu's, the confirmation hanging from that row.
//! Group and link rows have none, and neither has a page that is read
//! only.

mod common;

use common::{hear, home_fixture, next, post, quiet, start, start_observing_with};
use moshpit_ui::{
    home::{
        Intent, Message as HomeMessage,
        model::{
            Connection, Control, HomeState, Link, Mux, PaneInfo, SessionInfo, Transport,
            WindowInfo, Worktree,
        },
        swipe::{self, Label, Renames, Tint},
    },
    screen::{Params, Screen},
    sessions::Message as SessionsMessage,
};
use nsh_core::{Edges, Points, Size, geometry::px, platform::Idiom};
use serde_json::json;

const HOST: &str = "0F0E0D0C-0000-4000-8000-000000000001";

fn labels(actions: &[swipe::TreeAction]) -> Vec<Label> {
    actions.iter().map(|a| a.label).collect()
}

fn tints(actions: &[swipe::TreeAction]) -> Vec<Tint> {
    actions.iter().map(|a| a.tint).collect()
}

#[test]
fn a_panes_tray_is_its_rename_and_its_kill() {
    let tmux = swipe::pane_tray(HOST, "%2", Some(Renames::Window("@2".into())));
    assert_eq!(labels(&tmux), [Label::Rename, Label::KillPane]);
    assert_eq!(tints(&tmux), [Tint::Accent, Tint::Danger]);
    assert_eq!(
        tmux.iter().map(|a| a.intent.clone()).collect::<Vec<_>>(),
        [
            Intent::RenameWindow {
                connection: HOST.into(),
                window: "@2".into()
            },
            Intent::KillPane {
                connection: HOST.into(),
                pane: "%2".into()
            },
        ]
    );
    // herdr: the workspace the title is named after.
    let herdr = swipe::pane_tray("h", "w1:p2", Some(Renames::Session("w1".into())));
    assert_eq!(
        herdr[0].intent,
        Intent::RenameSession {
            connection: "h".into(),
            session: "w1".into()
        }
    );
    // Nothing to rename (the sessions page's panes, a pane the tree
    // doesn't know): the kill alone.
    assert_eq!(
        labels(&swipe::pane_tray(HOST, "%3", None)),
        [Label::KillPane]
    );
}

#[test]
fn a_windows_and_a_sessions_trays_hold_two_or_three() {
    let window = swipe::window_tray(HOST, "@2");
    assert_eq!(
        labels(&window),
        [Label::Rename, Label::NewPane, Label::KillWindow]
    );
    assert_eq!(tints(&window), [Tint::Accent, Tint::Accent, Tint::Danger]);
    assert_eq!(
        labels(&swipe::session_tray(HOST, "$0", false)),
        [Label::Rename, Label::KillSession]
    );
    let worktree = swipe::session_tray("h", "w4", true);
    assert_eq!(
        labels(&worktree),
        [Label::Rename, Label::RemoveWorktree, Label::KillSession]
    );
    assert_eq!(tints(&worktree), [Tint::Accent, Tint::Danger, Tint::Danger]);
    assert_eq!(
        worktree[1].intent,
        Intent::RemoveWorktree {
            connection: "h".into(),
            session: "w4".into()
        }
    );
}

#[test]
fn each_action_tells_its_intent_and_its_row() {
    let pane = swipe::pane_tray(HOST, "%2", Some(Renames::Window("@2".into())));
    let row = format!("home-pane-{HOST}-%2");
    assert_eq!(
        pane[0].event(),
        json!({"ev": "rename-window", "connection": HOST, "window": "@2", "row": row})
    );
    assert_eq!(
        pane[1].event(),
        json!({"ev": "kill-pane", "connection": HOST, "pane": "%2", "row": row})
    );
    let window = swipe::window_tray(HOST, "@2");
    let row = format!("home-window-{HOST}-@2");
    assert_eq!(
        window[1].event(),
        json!({"ev": "new-pane", "connection": HOST, "window": "@2", "row": row})
    );
    assert_eq!(
        window[2].event(),
        json!({"ev": "kill-window", "connection": HOST, "window": "@2", "row": row})
    );
    let session = swipe::session_tray(HOST, "$0", true);
    let row = format!("home-session-{HOST}-$0");
    assert_eq!(
        session[1].event(),
        json!({"ev": "remove-worktree", "connection": HOST, "session": "$0", "row": row})
    );
    assert_eq!(
        session[2].event(),
        json!({"ev": "kill-session", "connection": HOST, "session": "$0", "row": row})
    );
}

/// The words are the long-press menu's: tmux kills, herdr closes (seen on
/// a page, where the language is: the pages below press them by label).
#[test]
fn herdrs_trays_close() {
    let mut control = tmux();
    control.mux = Mux::Herdr;
    let (mut app, channel) = sessions(Link::Live, control);
    app.drag(
        &format!("id:home-window-{HOST}-@1"),
        nsh_core::geometry::point(px(LEFT), px(0.0)),
    )
    .unwrap();
    app.settle();
    assert!(
        app.screen()
            .id(&format!("id:home-tray-window-{HOST}-@1 >> label:Close Tab"))
            .is_ok()
    );
    assert!(
        app.screen()
            .id(&format!(
                "id:home-tray-window-{HOST}-@1 >> label:Kill Window"
            ))
            .is_err()
    );
    channel.close();
}

fn window(id: &str, session: &str, index: i64, name: &str, panes: i64) -> WindowInfo {
    WindowInfo {
        id: id.into(),
        session: session.into(),
        index,
        name: name.into(),
        pane_count: panes,
    }
}

fn pane(id: &str, window: &str, index: i64, command: &str) -> PaneInfo {
    PaneInfo {
        id: id.into(),
        window: window.into(),
        index,
        command: command.into(),
        path: Some("/Users/dev/src/api".into()),
    }
}

/// A tmux host: `work` (in use) with an editor and an agent's window of
/// two panes; `ops` with one shell.
fn tmux() -> Control {
    Control {
        mux: Mux::Tmux,
        attached: true,
        sessions: vec![
            SessionInfo {
                id: "$0".into(),
                name: "work".into(),
            },
            SessionInfo {
                id: "$1".into(),
                name: "ops".into(),
            },
        ],
        windows: vec![
            window("@1", "$0", 1, "api", 1),
            window("@2", "$0", 2, "claude", 2),
            window("@3", "$1", 1, "zsh", 1),
        ],
        panes: vec![
            pane("%1", "@1", 0, "nvim"),
            pane("%2", "@2", 0, "claude"),
            pane("%3", "@2", 1, "zsh"),
            pane("%4", "@3", 0, "zsh"),
        ],
        active_session: Some("$0".into()),
        active_window: Some("@1".into()),
        active_pane: Some("%1".into()),
        ..Control::default()
    }
}

#[test]
fn a_panes_rename_is_where_its_title_comes_from() {
    assert_eq!(
        swipe::pane_renames(&tmux(), "%3"),
        Some(Renames::Window("@2".into()))
    );
    assert_eq!(
        swipe::pane_renames(&tmux(), "%9"),
        None,
        "a pane the tree doesn't know"
    );
    let herdr = Control {
        mux: Mux::Herdr,
        sessions: vec![SessionInfo {
            id: "w1".into(),
            name: "trainer".into(),
        }],
        windows: vec![window("w1:t1", "w1", 1, "1", 1)],
        panes: vec![pane("w1:p1", "w1:t1", 0, "claude")],
        ..Control::default()
    };
    assert_eq!(
        swipe::pane_renames(&herdr, "w1:p1"),
        Some(Renames::Session("w1".into()))
    );
}

fn host(link: Link, control: Control) -> Connection {
    serde_json::from_value(json!({
        "id": HOST,
        "name": "devbox",
        "initial": "D",
        "user": "dev",
        "host": "devbox.example.com",
        "mosh": false,
        "udpPort": 60000,
        "link": link,
        "transport": Transport::default(),
    }))
    .map(|mut connection: Connection| {
        connection.control = Some(control);
        connection
    })
    .unwrap()
}

fn sessions(
    link: Link,
    control: Control,
) -> (common::App, std::sync::Arc<moshpit_ui::channel::Channel>) {
    let (mut app, channel) = start(Screen::Sessions, Params::default(), "sessions");
    post(
        &mut app,
        &channel,
        &SessionsMessage::Sessions(Box::new(host(link, control))).to_json(),
    );
    (app, channel)
}

const LEFT: f32 = -220.0;

#[test]
fn the_sessions_pages_rows_swipe_open_and_tell_the_host() {
    let mut control = tmux();
    control.worktrees = vec![Worktree {
        session: "$1".into(),
        repo: "/Users/dev/src/api".into(),
    }];
    let (mut app, channel) = sessions(Link::Live, control);
    let told = hear(&channel);
    // A window: Rename, New Pane, Kill Window.
    app.drag(
        &format!("id:home-window-{HOST}-@1"),
        nsh_core::geometry::point(px(LEFT), px(0.0)),
    )
    .unwrap();
    app.settle();
    app.press(&format!("id:home-tray-window-{HOST}-@1 >> label:New Pane"))
        .unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "new-pane", "connection": HOST, "window": "@1", "row": format!("home-window-{HOST}-@1")})
    );
    // A session that is a worktree's: Remove Worktree among them.
    app.drag(
        &format!("id:home-session-{HOST}-$1"),
        nsh_core::geometry::point(px(LEFT), px(0.0)),
    )
    .unwrap();
    app.settle();
    app.press(&format!(
        "id:home-tray-session-{HOST}-$1 >> label:Remove Worktree"
    ))
    .unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "remove-worktree", "connection": HOST, "session": "$1", "row": format!("home-session-{HOST}-$1")})
    );
    // A pane, once its window has folded its panes out: the kill.
    app.press(&format!("id:home-window-{HOST}-@2")).unwrap();
    let _ = next(&told);
    app.drag(
        &format!("id:home-pane-{HOST}-%3"),
        nsh_core::geometry::point(px(LEFT), px(0.0)),
    )
    .unwrap();
    app.settle();
    app.press(&format!("id:home-tray-pane-{HOST}-%3 >> label:Kill Pane"))
        .unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "kill-pane", "connection": HOST, "pane": "%3", "row": format!("home-pane-{HOST}-%3")})
    );
    // A tap on a row still opens it.
    app.press(&format!("id:home-window-{HOST}-@1")).unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "window", "connection": HOST, "window": "@1"})
    );
    quiet(&told);
    channel.close();
}

#[test]
fn a_read_only_page_has_no_trays() {
    let (mut app, channel) = sessions(Link::Dead, tmux());
    app.settle();
    let screen = app.screen();
    assert!(
        screen.id(&format!("id:home-window-{HOST}-@1")).is_ok(),
        "the rows are there"
    );
    assert!(
        screen
            .id(&format!("id:home-tray-window-{HOST}-@1"))
            .is_err(),
        "read only: no tray"
    );
    channel.close();
}

/// Home on the owner's iPhone with R5's fixture.
fn home() -> (common::App, std::sync::Arc<moshpit_ui::channel::Channel>) {
    let (mut app, channel, _) =
        start_observing_with(Screen::Home, Params::default(), "home", |config| {
            config.viewport = Size::new(Points(402.0), Points(874.0));
            config.environment.idiom = Idiom::Phone;
            config.environment.safe_area = Edges {
                top: Points(62.0),
                bottom: Points(34.0),
                left: Points(0.0),
                right: Points(0.0),
            };
        });
    let state = HomeState {
        connections: home_fixture::fixture(),
        version: "v1.0.9 (1)".into(),
        sidebar: false,
        jump_hosts: 0,
    };
    post(
        &mut app,
        &channel,
        &HomeMessage::Home(Box::new(state)).to_json(),
    );
    (app, channel)
}

#[test]
fn homes_pane_rows_swipe_open_on_rename_and_kill() {
    let (mut app, channel) = home();
    let told = hear(&channel);
    // build-box (tmux): its trainer agent, in window @1.
    let agent = "id:home-pane-c-box-%1";
    app.wait_for(agent).unwrap();
    app.drag(agent, nsh_core::geometry::point(px(LEFT), px(0.0)))
        .unwrap();
    app.settle();
    app.press("id:home-tray-pane-c-box-%1 >> label:Rename")
        .unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "rename-window", "connection": "c-box", "window": "@1", "row": "home-pane-c-box-%1"})
    );
    app.drag(agent, nsh_core::geometry::point(px(LEFT), px(0.0)))
        .unwrap();
    app.settle();
    app.press("id:home-tray-pane-c-box-%1 >> label:Kill Pane")
        .unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "kill-pane", "connection": "c-box", "pane": "%1", "row": "home-pane-c-box-%1"})
    );
    // The group and link rows have none.
    let screen = app.screen();
    assert!(screen.id("id:home-idle-c-gpu").is_ok());
    assert!(screen.id("id:home-tray-idle-c-gpu").is_err());
    assert!(screen.id("id:home-tray-all-c-box").is_err());
    channel.close();
}
