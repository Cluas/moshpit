//! A host's sessions (`docs/nsh/UI-DESIGN.md` §6.2, R7) on a headless
//! engine: every session is a card headed by itself, its windows under it,
//! a window of several panes folding them out; folding a session or a
//! window tells the host its disclosure; a pane, a window of one pane, the
//! bar's + and the empty page's button tell the host what Home's own rows
//! would; an agent's pane carries its state; past eight sessions only the
//! one in use starts open; disconnected, the page is read only under a
//! banner that reconnects.

mod common;

use common::{hear, next, post, quiet, start};
use moshpit_ui::{
    home::{
        TreeState,
        model::{
            Connection, Control, Hook, Link, Mux, PaneInfo, SessionInfo, Transport, WindowInfo,
        },
    },
    screen::{Params, Screen},
    sessions::Message,
};
use serde_json::json;

const HOST: &str = "0F0E0D0C-0000-4000-8000-000000000001";

fn window(id: &str, session: &str, index: i64, name: &str, panes: i64) -> WindowInfo {
    WindowInfo {
        id: id.into(),
        session: session.into(),
        index,
        name: name.into(),
        pane_count: panes,
    }
}

fn pane(id: &str, window: &str, index: i64, command: &str, path: &str) -> PaneInfo {
    PaneInfo {
        id: id.into(),
        window: window.into(),
        index,
        command: command.into(),
        path: Some(path.into()),
    }
}

/// A tmux host: `work` (in use) with an editor and an agent's window of
/// two panes, the agent waiting on you; `ops` with one shell.
fn control() -> Control {
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
            pane("%1", "@1", 0, "nvim", "/Users/dev/src/api"),
            pane("%2", "@2", 0, "claude", "/Users/dev/src/trainer"),
            pane("%3", "@2", 1, "zsh", "/Users/dev/src/trainer"),
            pane("%4", "@3", 0, "zsh", "/var/log"),
        ],
        active_session: Some("$0".into()),
        active_window: Some("@1".into()),
        active_pane: Some("%1".into()),
        hooks: vec![Hook {
            pane: "%2".into(),
            state: Some("attention".into()),
            agent: Some("claude".into()),
            since: None,
        }],
        ..Control::default()
    }
}

fn host(link: Link, control: Option<Control>) -> Connection {
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
        connection.control = control;
        connection
    })
    .unwrap()
}

fn show(
    app: &mut nsh_testing::Headless,
    channel: &moshpit_ui::channel::Channel,
    connection: Connection,
) {
    post(
        app,
        channel,
        &Message::Sessions(Box::new(connection)).to_json(),
    );
}

fn open() -> (common::App, std::sync::Arc<moshpit_ui::channel::Channel>) {
    let (mut app, channel) = start(Screen::Sessions, Params::default(), "sessions");
    show(&mut app, &channel, host(Link::Live, Some(control())));
    (app, channel)
}

#[test]
fn every_session_is_a_card_headed_by_itself_its_windows_under_it() {
    let (app, channel) = open();
    // Windows tmux named after their program are titled by their
    // folder (§6.1): `claude` by `trainer`, `zsh` by `log`.
    for text in ["devbox", "work", "ops", "api", "trainer", "log", "2 panes"] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(
        app.screen().matches("role:header[/^work/]").unwrap(),
        "a session's head is a heading screen readers jump to"
    );
    assert!(app.screen().shows("Attached"), "the session in use says so");
    assert!(
        app.screen().shows("1 waiting"),
        "and how many agents wait in it"
    );
    assert!(
        app.screen()
            .matches(&format!("id:home-window-{HOST}-@1"))
            .unwrap(),
        "rows carry Home's identifiers, the host's menus find them by"
    );
    channel.close();
}

#[test]
fn a_window_of_several_panes_folds_them_out_and_the_host_keeps_it() {
    let (mut app, channel) = open();
    let told = hear(&channel);
    assert!(
        !app.screen()
            .matches(&format!("id:home-pane-{HOST}-%2"))
            .unwrap()
    );
    app.press(&format!("id:home-window-{HOST}-@2")).unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "tree", "connection": HOST, "collapsed": [], "expanded": ["@2"]})
    );
    assert!(
        app.screen()
            .matches(&format!("id:home-pane-{HOST}-%2"))
            .unwrap()
    );
    assert!(
        app.screen()
            .accessibility_state(&format!("id:home-window-{HOST}-@2"))
            .unwrap()
            .expanded
            == Some(true)
    );
    quiet(&told);
    channel.close();
}

#[test]
fn folding_a_session_tells_the_host_and_names_its_windows() {
    let (mut app, channel) = open();
    let told = hear(&channel);
    app.press(&format!("id:home-session-{HOST}-$1")).unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "tree", "connection": HOST, "collapsed": ["$1"], "expanded": []})
    );
    assert!(
        !app.screen()
            .matches(&format!("id:home-window-{HOST}-@3"))
            .unwrap(),
        "its windows fold away"
    );
    assert!(app.screen().shows("log"), "and the head names them");
    channel.close();
}

#[test]
fn a_pane_a_window_and_the_bar_tell_the_host_what_homes_rows_would() {
    let (mut app, channel) = open();
    let told = hear(&channel);
    app.press(&format!("id:home-window-{HOST}-@1")).unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "window", "connection": HOST, "window": "@1"})
    );
    app.press(&format!("id:home-window-{HOST}-@2")).unwrap();
    let _ = next(&told);
    app.press(&format!("id:home-pane-{HOST}-%3")).unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "pane", "connection": HOST, "pane": "%3"})
    );
    app.press("id:sessions-new").unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "new-session", "connection": HOST})
    );
    channel.close();
}

#[test]
fn an_agents_pane_carries_its_state() {
    let (mut app, channel) = open();
    app.press(&format!("id:home-window-{HOST}-@2")).unwrap();
    assert!(
        app.screen().shows("Needs you"),
        "{:?}",
        app.screen().texts()
    );
    assert!(
        app.screen()
            .matches("label:/^claude, ~?.*trainer, Needs you$/")
            .unwrap()
            || app.screen().matches("label:/claude.*Needs you/").unwrap(),
        "screen readers hear the agent and its state"
    );
    channel.close();
}

/// A host of ten sessions, one window and one pane each, `$3` in use.
fn ten_sessions() -> Control {
    let mut many = control();
    many.sessions = (0..10)
        .map(|i| SessionInfo {
            id: format!("${i}"),
            name: format!("s{i}"),
        })
        .collect();
    many.windows = (0..10)
        .map(|i| window(&format!("@{i}"), &format!("${i}"), 1, &format!("w{i}"), 1))
        .collect();
    many.panes = (0..10)
        .map(|i| pane(&format!("%{i}"), &format!("@{i}"), 0, "zsh", "/tmp"))
        .collect();
    many.active_session = Some("$3".into());
    many.hooks.clear();
    many
}

#[test]
fn past_eight_sessions_only_the_one_in_use_starts_open() {
    let (mut app, channel) = start(Screen::Sessions, Params::default(), "sessions");
    show(&mut app, &channel, host(Link::Live, Some(ten_sessions())));
    assert!(
        app.screen()
            .matches(&format!("id:home-window-{HOST}-@3"))
            .unwrap()
    );
    assert!(
        !app.screen()
            .matches(&format!("id:home-window-{HOST}-@0"))
            .unwrap()
    );
    // Opening one writes the whole disclosure out.
    let told = hear(&channel);
    app.press(&format!("id:home-session-{HOST}-$0")).unwrap();
    let event = next(&told);
    let collapsed = event["collapsed"].as_array().unwrap();
    assert_eq!(collapsed.len(), 8, "{event}");
    assert!(!collapsed.contains(&json!("$0")) && !collapsed.contains(&json!("$3")));
    channel.close();
}

/// B553: Home's idle group opened on a host makes the host keep a tree for
/// it before this page ever told one (`{"ev": "tree", "idle": true}`):
/// that is not a disclosure this page kept, so past eight sessions still
/// only the one in use starts open. Every session opened.
#[test]
fn homes_idle_group_is_not_a_disclosure_this_page_kept() {
    let (mut app, channel) = start(Screen::Sessions, Params::default(), "sessions");
    post(
        &mut app,
        &channel,
        &Message::Trees(vec![TreeState {
            connection: HOST.into(),
            idle: true,
            ..TreeState::default()
        }])
        .to_json(),
    );
    show(&mut app, &channel, host(Link::Live, Some(ten_sessions())));
    assert!(
        app.screen()
            .matches(&format!("id:home-window-{HOST}-@3"))
            .unwrap()
    );
    assert!(
        !app.screen()
            .matches(&format!("id:home-window-{HOST}-@0"))
            .unwrap(),
        "every session opened"
    );
    channel.close();
}

#[test]
fn a_disclosure_the_host_kept_comes_back() {
    let (mut app, channel) = open();
    post(
        &mut app,
        &channel,
        &Message::Trees(vec![TreeState {
            connection: HOST.into(),
            collapsed: vec!["$0".into()],
            told: true,
            ..TreeState::default()
        }])
        .to_json(),
    );
    assert!(
        !app.screen()
            .matches(&format!("id:home-window-{HOST}-@1"))
            .unwrap()
    );
    assert!(
        app.screen()
            .matches(&format!("id:home-window-{HOST}-@3"))
            .unwrap()
    );
    channel.close();
}

#[test]
fn a_disclosure_kept_before_the_host_came_waits_for_it() {
    let (mut app, channel) = start(Screen::Sessions, Params::default(), "sessions");
    post(
        &mut app,
        &channel,
        &Message::Trees(vec![TreeState {
            connection: HOST.into(),
            collapsed: vec!["$1".into()],
            told: true,
            ..TreeState::default()
        }])
        .to_json(),
    );
    show(&mut app, &channel, host(Link::Live, Some(control())));
    assert!(
        app.screen()
            .matches(&format!("id:home-window-{HOST}-@1"))
            .unwrap()
    );
    assert!(
        !app.screen()
            .matches(&format!("id:home-window-{HOST}-@3"))
            .unwrap()
    );
    channel.close();
}

#[test]
fn disconnected_the_page_is_read_only_under_a_banner_that_reconnects() {
    let (mut app, channel) = start(Screen::Sessions, Params::default(), "sessions");
    show(&mut app, &channel, host(Link::Dead, Some(control())));
    let told = hear(&channel);
    assert!(app.screen().matches("id:sessions-offline").unwrap());
    assert!(
        !app.screen().enabled("id:sessions-new").unwrap(),
        "no new session"
    );
    let _ = app.press(&format!("id:home-window-{HOST}-@1"));
    quiet(&told);
    app.press("id:sessions-reconnect").unwrap();
    assert_eq!(next(&told), json!({"ev": "connect", "connection": HOST}));
    channel.close();
}

#[test]
fn a_host_with_no_sessions_offers_one() {
    let (mut app, channel) = start(Screen::Sessions, Params::default(), "sessions");
    let mut none = control();
    none.sessions.clear();
    none.windows.clear();
    none.panes.clear();
    none.hooks.clear();
    show(&mut app, &channel, host(Link::Live, Some(none)));
    assert!(app.screen().shows("No tmux sessions on this host"));
    let told = hear(&channel);
    app.press("id:sessions-empty-new").unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "new-session", "connection": HOST})
    );
    channel.close();
}

#[test]
fn herdr_says_workspaces() {
    let (mut app, channel) = start(Screen::Sessions, Params::default(), "sessions");
    let mut none = control();
    none.mux = Mux::Herdr;
    none.sessions.clear();
    none.windows.clear();
    none.panes.clear();
    none.hooks.clear();
    show(&mut app, &channel, host(Link::Live, Some(none)));
    assert!(app.screen().shows("No herdr workspaces on this host"));
    channel.close();
}

/// B555: two herdr workspaces with one name (two agents in one repo) are
/// told apart by their ids, the name kept: "api · w4", as the terminal's
/// breadcrumb says. The cards said "w4" and "w5", the name nowhere.
#[test]
fn herdr_workspaces_of_one_name_keep_it() {
    let (mut app, channel) = start(Screen::Sessions, Params::default(), "sessions");
    let mut twins = control();
    twins.mux = Mux::Herdr;
    twins.sessions = ["w4", "w5"]
        .map(|id| SessionInfo {
            id: id.into(),
            name: "api".into(),
        })
        .to_vec();
    twins.windows = vec![
        window("w4:t1", "w4", 1, "Tab 1", 1),
        window("w5:t1", "w5", 1, "Tab 1", 1),
    ];
    twins.panes = vec![
        pane("w4:p1", "w4:t1", 0, "zsh", "/src/api"),
        pane("w5:p1", "w5:t1", 0, "zsh", "/src/api"),
    ];
    twins.active_session = Some("w4".into());
    twins.hooks.clear();
    show(&mut app, &channel, host(Link::Live, Some(twins)));
    for title in ["api · w4", "api · w5"] {
        assert!(
            app.screen().shows(title),
            "{title}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}

#[test]
fn the_page_speaks_chinese_in_chinese() {
    let (mut app, channel) = start(Screen::Sessions, common::zh_hans(), "sessions");
    show(&mut app, &channel, host(Link::Live, Some(control())));
    assert!(
        app.screen().shows("1 个在等你"),
        "{:?}",
        app.screen().texts()
    );
    channel.close();
}
