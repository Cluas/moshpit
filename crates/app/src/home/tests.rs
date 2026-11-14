//! `HomeNSHTests`' builder parts (the link table, the message the screen
//! takes, a stamp's own time first) and the rules the two builders differed
//! on, each with the stricter one.

use std::collections::BTreeMap;

use moshpit_herdr::snapshot as herdr;
use moshpit_session::{
    Cause, Degrade, EndReason, Failure, FailureKind, HerdrView, SessionState, SessionView,
    TmuxView, UnreachableWhy,
};
use moshpit_tmux::model as tmux;
use moshpit_ui::home::Message;
use moshpit_ui::home::model::{FailureReason, Link, Mux, Transport};

use super::*;
use moshpit_ui::ui::mark::Face;

const STUDIO: &str = "C0FFEE00-0000-4000-8000-000000000001";

fn saved(id: &str, name: &str) -> SavedConnection {
    SavedConnection {
        id: id.into(),
        name: name.into(),
        fingerprint: None,
        emoji: None,
        user: "cluas".into(),
        host: "studio.local".into(),
        mosh: false,
        udp_port: 60001,
        tmux: true,
        disconnecting: false,
        session: None,
    }
}

fn view(state: SessionState) -> SessionView {
    let mut view = SessionView::initial();
    view.state = state;
    view
}

fn tmux_tree() -> tmux::Snapshot {
    let mut snapshot = tmux::Snapshot::default();
    for (id, name) in [("$1", "work"), ("$0", "dots")] {
        snapshot.sessions.insert(
            id.into(),
            tmux::SessionInfo {
                id: id.into(),
                name: name.into(),
                is_attached: id == "$1",
            },
        );
    }
    let mut window = tmux::WindowInfo::new("@2", "$1");
    window.name = "api".into();
    window.index = 1;
    window.pane_count = 2;
    snapshot.windows.insert("@2".into(), window);
    for (id, command, path) in [("%3", "claude", Some("/srv/api")), ("%2", "zsh", None)] {
        let mut pane = tmux::PaneInfo::placeholder(id, "@2");
        pane.command = command.into();
        pane.path = path.map(str::to_owned);
        snapshot.panes.insert(id.into(), pane);
    }
    snapshot.active_session_id = Some("$1".into());
    snapshot.active_window_id = Some("@2".into());
    snapshot.active_pane_id = Some("%3".into());
    snapshot.is_attached = true;
    snapshot.ever_attached = true;
    snapshot
}

fn tmux_view(snapshot: tmux::Snapshot) -> TmuxView {
    let mut hooks = BTreeMap::new();
    hooks.insert(
        "%3".to_owned(),
        moshpit_tmux::hooks::AgentHook {
            state: Some("attention".into()),
            agent: Some("claude".into()),
            since: Some(1_000.0),
            command: Some("claude".into()),
            title: None,
        },
    );
    hooks.insert(
        "%2".to_owned(),
        moshpit_tmux::hooks::AgentHook {
            state: Some("working".into()),
            agent: Some("codex".into()),
            since: None,
            command: Some("zsh".into()),
            title: None,
        },
    );
    TmuxView {
        snapshot,
        notice: None,
        refreshing: false,
        size_follow: None,
        agent_hooks: hooks,
        agent_hooks_loaded: true,
        input: Default::default(),
        no_session: false,
    }
}

fn herdr_view() -> HerdrView {
    let mut snapshot = herdr::Snapshot::default();
    for id in ["w2", "w1"] {
        snapshot.sessions.insert(
            id.into(),
            herdr::Session {
                id: id.into(),
                name: format!("ws-{id}"),
                is_attached: id == "w1",
            },
        );
    }
    snapshot.windows.insert(
        "w1:t1".into(),
        herdr::Window {
            id: "w1:t1".into(),
            session_id: "w1".into(),
            name: "Tab 1".into(),
            index: 0,
            layout: String::new(),
            is_active: true,
            pane_count: 1,
            is_zoomed: false,
        },
    );
    snapshot.panes.insert(
        "w1:p1".into(),
        herdr::Pane {
            id: "w1:p1".into(),
            window_id: "w1:t1".into(),
            index: 0,
            command: "claude".into(),
            width: 80,
            height: 24,
            is_active: true,
        },
    );
    snapshot.is_attached = true;
    snapshot.ever_attached = true;
    let mut agent_hooks = BTreeMap::new();
    agent_hooks.insert(
        "w1:p1".to_owned(),
        herdr::AgentHook {
            state: Some(herdr::AgentState::Working),
            agent: Some("claude".into()),
            title: None,
        },
    );
    HerdrView {
        snapshot,
        agent_hooks,
        agent_hooks_loaded: true,
        notice: None,
        refreshing: true,
        restarting: false,
        server_not_running: false,
        protocol_mismatch: None,
        version_skew: None,
        server_version: None,
        push_active: false,
        pane_switch_in_flight: false,
        server_focus: None,
        pane_cwds: BTreeMap::from([("w1:p1".to_owned(), "/srv/trainer".to_owned())]),
        worktree_repos: BTreeMap::from([
            ("w2".to_owned(), "trainer".to_owned()),
            // A workspace closed since: no longer in the tree.
            ("w9".to_owned(), "gone".to_owned()),
        ]),
        open_repo_guesses: Vec::new(),
        frame_target: None,
        attached_pane: None,
    }
}

fn no_monitor(_: &str) -> BTreeMap<String, f64> {
    BTreeMap::new()
}

fn build(
    connections: &[SavedConnection],
    since: &dyn Fn(&str) -> BTreeMap<String, f64>,
) -> HomeState {
    state(&HomeInput {
        connections,
        since,
        version: "v1.0.9 (1)",
        sidebar: false,
        locale: "en",
    })
}

// MARK: - HomeNSHTests' builder parts

#[test]
fn a_session_reads_as_the_swiftui_card_reads_it() {
    let mut never = tmux::Snapshot::default();
    let link =
        |state: SessionState, tmux_expected: bool, stalled: bool, snapshot: &tmux::Snapshot| {
            super::link(&state, tmux_expected, Some(snapshot), stalled, false)
        };
    let failed = SessionState::Failed(Failure::lost("x"));
    let ended = SessionState::Ended(EndReason::Disconnected);
    assert_eq!(link(failed.clone(), true, false, &never), Link::Dead);
    assert_eq!(link(ended, false, false, &never), Link::Dead);
    assert_eq!(
        link(failed, true, true, &never),
        Link::Dead,
        "dead wins over stalled"
    );
    assert_eq!(
        link(SessionState::Connecting, true, false, &never),
        Link::Connecting
    );
    assert_eq!(
        link(SessionState::Reconnecting, false, false, &never),
        Link::Reconnecting
    );
    assert_eq!(
        link(SessionState::Connected, true, false, &never),
        Link::Attaching,
        "tmux's first attach"
    );
    assert_eq!(
        link(SessionState::Connected, true, true, &never),
        Link::Stalled
    );
    assert_eq!(
        link(SessionState::Connected, false, false, &never),
        Link::Live
    );
    never.ever_attached = true;
    assert_eq!(
        link(SessionState::Connected, true, false, &never),
        Link::Live,
        "an attach that ended is not attaching again"
    );
    assert_eq!(
        super::link(&SessionState::Connected, true, None, false, false),
        Link::Live,
        "no control plane yet: the open-terminal row"
    );
    assert_eq!(
        transport(&SessionState::Reconnecting),
        Transport::Reconnecting
    );
    assert_eq!(transport(&SessionState::Connected), Transport::Live);
    assert_eq!(
        transport(&SessionState::Failed(Failure::lost("x"))),
        Transport::Offline
    );
}

#[test]
fn every_state_is_a_message_the_screen_takes() {
    let mut studio = saved(STUDIO, "studio");
    let mut live = view(SessionState::Connected);
    live.tmux_view = Some(tmux_view(tmux_tree()));
    live.connected_at = Some(1_762_800_000.0);
    studio.session = Some(live);
    let pi = saved("C0FFEE00-0000-4000-8000-000000000002", "pi");
    let connections = [studio, pi];
    let input = HomeInput {
        connections: &connections,
        since: &|_| BTreeMap::from([("%3".to_owned(), 5.0), ("%2".to_owned(), 7.0)]),
        version: "v1.0.9 (1)",
        sidebar: true,
        locale: "en",
    };
    let json = message(&input);
    let Message::Home(parsed) = Message::parse(&json).expect("the screen takes it") else {
        panic!("not a home message");
    };
    assert!(parsed.sidebar, "the iPad's sidebar");
    let control = parsed.connections[0].control.as_ref().unwrap();
    let hooks: Vec<(&str, Option<f64>)> = control
        .hooks
        .iter()
        .map(|h| (h.pane.as_str(), h.since))
        .collect();
    assert_eq!(
        hooks,
        [("%2", Some(7.0)), ("%3", Some(1_000.0))],
        "the stamp's own time first, the monitor's where it has none"
    );
    assert!(
        parsed.connections[1].control.is_none(),
        "a saved host has no tree"
    );
    assert_eq!(parsed.connections[1].link, Link::Saved);
    assert_eq!(parsed.connections[0].connected_at, Some(1_762_800_000.0));
    let empty = message(&HomeInput {
        connections: &[],
        since: &no_monitor,
        version: "v",
        sidebar: false,
        locale: "en",
    });
    assert!(Message::parse(&empty).is_ok());
}

/// B228: a server with no session waits for the person's first. Live,
/// with its empty tree and its +: not an attach under way, and not a
/// stalled one even when the attach gave up.
#[test]
fn a_server_with_no_session_is_live_with_its_empty_tree() {
    let mut studio = saved(STUDIO, "studio");
    studio.mosh = true;
    let mut tree = tmux_view(tmux::Snapshot::default());
    tree.agent_hooks.clear();
    tree.no_session = true;
    for stalled in [false, true] {
        let mut live = view(SessionState::Connected);
        live.tmux_view = Some(tree.clone());
        live.attach_stalled = stalled;
        studio.session = Some(live);
        let home = build(std::slice::from_ref(&studio), &no_monitor);
        let card = &home.connections[0];
        assert_eq!(card.link, Link::Live, "stalled: {stalled}");
        let control = card.control.as_ref().expect("the empty tree, with its +");
        assert_eq!(control.mux, Mux::Tmux);
        assert!(!control.attached && control.sessions.is_empty());
    }
    let mut dead = view(SessionState::Failed(Failure::lost("x")));
    dead.tmux_view = Some(tree);
    studio.session = Some(dead);
    assert_eq!(
        build(std::slice::from_ref(&studio), &no_monitor).connections[0].link,
        Link::Dead,
        "dead wins"
    );
}

// MARK: - Where the builders differed

#[test]
fn every_list_is_in_id_order_and_every_pane_says_where_it_runs() {
    let mut studio = saved(STUDIO, "studio");
    let mut live = view(SessionState::Connected);
    live.tmux_view = Some(tmux_view(tmux_tree()));
    studio.session = Some(live);
    let home = build(&[studio], &no_monitor);
    let control = home.connections[0].control.as_ref().unwrap();
    assert_eq!(control.mux, Mux::Tmux);
    assert_eq!(
        control
            .sessions
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["$0", "$1"]
    );
    assert_eq!(
        control
            .panes
            .iter()
            .map(|p| (p.id.as_str(), p.path.as_deref()))
            .collect::<Vec<_>>(),
        [("%2", None), ("%3", Some("/srv/api"))]
    );
    assert_eq!(control.active_pane.as_deref(), Some("%3"));
}

/// Android sent herdr's stamps with no time; Swift took the monitor's.
#[test]
fn herdrs_stamps_have_the_monitors_time_and_its_panes_their_directory() {
    let mut box_ = saved(STUDIO, "trainer");
    box_.tmux = false;
    let mut live = view(SessionState::Connected);
    live.herdr = Some(herdr_view());
    box_.session = Some(live);
    let home = build(&[box_], &|conn| {
        assert_eq!(conn, STUDIO, "the monitor is asked by the connection's id");
        BTreeMap::from([("w1:p1".to_owned(), 42.0)])
    });
    let control = home.connections[0].control.as_ref().unwrap();
    assert_eq!(control.mux, Mux::Herdr);
    assert!(control.refreshing);
    assert_eq!(control.hooks[0].state.as_deref(), Some("working"));
    assert_eq!(control.hooks[0].since, Some(42.0));
    assert_eq!(control.panes[0].path.as_deref(), Some("/srv/trainer"));
    assert_eq!(
        control
            .sessions
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["w1", "w2"],
        "herdr's workspaces in id order too"
    );
    assert_eq!(
        control.worktrees.len(),
        1,
        "only the workspaces in the tree"
    );
    assert_eq!(control.worktrees[0].repo, "trainer");
    assert!(control.mismatch.is_none());
}

#[test]
fn a_herdr_version_skew_is_said_in_the_apps_language_with_the_right_remedy() {
    let mut box_ = saved(STUDIO, "trainer");
    box_.tmux = false;
    let mut herdr = herdr_view();
    herdr.protocol_mismatch = Some("protocol 3, server speaks 2".into());
    herdr.version_skew = Some(moshpit_herdr::version::VersionReport {
        client: "0.9.4".into(),
        server: "0.9.5".into(),
    });
    let mut live = view(SessionState::Connected);
    live.herdr = Some(herdr.clone());
    box_.session = Some(live);
    let input = |connections: &[SavedConnection], locale: &str| -> HomeState {
        state(&HomeInput {
            connections,
            since: &no_monitor,
            version: "v",
            sidebar: false,
            locale,
        })
    };
    let en = input(std::slice::from_ref(&box_), "en");
    let mismatch = en.connections[0]
        .control
        .as_ref()
        .unwrap()
        .mismatch
        .clone()
        .unwrap();
    assert!(
        mismatch
            .notice
            .starts_with("herdr on the host's PATH is 0.9.4")
    );
    assert!(
        !mismatch.restart,
        "an older client is updated, not the server restarted"
    );
    let zh = input(std::slice::from_ref(&box_), "zh-Hans");
    let notice = &zh.connections[0]
        .control
        .as_ref()
        .unwrap()
        .mismatch
        .as_ref()
        .unwrap()
        .notice;
    assert!(!notice.contains("PATH is"), "{notice}");
    // No report: the message as herdr said it.
    herdr.version_skew = None;
    let mut live = view(SessionState::Connected);
    live.herdr = Some(herdr);
    box_.session = Some(live);
    let plain = input(std::slice::from_ref(&box_), "zh-Hans");
    assert_eq!(
        plain.connections[0]
            .control
            .as_ref()
            .unwrap()
            .mismatch
            .as_ref()
            .unwrap()
            .notice,
        "protocol 3, server speaks 2"
    );
}

#[test]
fn an_initial_is_a_character_as_a_person_counts_them() {
    assert_eq!(initial("studio"), "S");
    assert_eq!(
        initial("e\u{301}cole"),
        "E\u{301}",
        "the accent comes along"
    );
    assert_eq!(initial("ßeta"), "SS");
    assert_eq!(initial("👩‍💻 box"), "👩‍💻");
    assert_eq!(initial(""), "");
}

// MARK: - What R5 reads

#[test]
fn a_dead_connection_says_when_it_was_lost_and_why_it_failed() {
    let failure = |kind, cause| Failure {
        kind,
        cause,
        log: "said".into(),
        retryable: false,
    };
    let cases = [
        (
            failure(FailureKind::Credentials, Cause::Plain),
            FailureReason::Auth,
        ),
        (
            failure(
                FailureKind::HostKey,
                Cause::HostKey {
                    host: "h".into(),
                    port: 22,
                    changed: true,
                },
            ),
            FailureReason::HostKeyChanged,
        ),
        (
            failure(
                FailureKind::HostKey,
                Cause::HostKey {
                    host: "h".into(),
                    port: 22,
                    changed: false,
                },
            ),
            FailureReason::HostKey,
        ),
        (
            failure(
                FailureKind::Unreachable,
                Cause::Unreachable {
                    host: "h".into(),
                    port: 22,
                    why: UnreachableWhy::TimedOut,
                },
            ),
            FailureReason::Timeout,
        ),
        (
            failure(
                FailureKind::Unreachable,
                Cause::Unreachable {
                    host: "h".into(),
                    port: 22,
                    why: UnreachableWhy::Refused,
                },
            ),
            FailureReason::Unreachable,
        ),
        (
            failure(
                FailureKind::Handshake,
                Cause::Handshake {
                    host: "h".into(),
                    port: 22,
                    timed_out: true,
                },
            ),
            FailureReason::Timeout,
        ),
        (
            failure(FailureKind::Lost, Cause::Plain),
            FailureReason::Other,
        ),
    ];
    for (failure, reason) in cases {
        let mut dead = saved(STUDIO, "studio");
        let mut v = view(SessionState::Failed(failure.clone()));
        v.lost_at = Some(300.0);
        dead.session = Some(v);
        let home = build(&[dead], &no_monitor);
        let card = &home.connections[0];
        assert_eq!(card.link, Link::Dead);
        assert_eq!(card.lost_at, Some(300.0));
        let said = card.failure.as_ref().unwrap();
        assert_eq!(said.reason, reason, "{:?}", failure.kind);
        assert_eq!(said.detail, "said");
    }
    let mut ended = saved(STUDIO, "studio");
    ended.session = Some(view(SessionState::Ended(EndReason::Disconnected)));
    let home = build(&[ended], &no_monitor);
    assert_eq!(home.connections[0].link, Link::Dead);
    assert!(
        home.connections[0].failure.is_none(),
        "an end is not a failure"
    );
}

#[test]
fn the_card_flags_come_from_the_view_and_the_host() {
    let mut c = saved(STUDIO, "studio");
    c.mosh = true;
    c.udp_port = 60100;
    c.disconnecting = true;
    let mut v = view(SessionState::Connected);
    v.degrade = Some(Degrade::MoshServerMissing);
    v.mosh_roaming = true;
    c.session = Some(v);
    let card = &build(&[c.clone()], &no_monitor).connections[0];
    assert!(card.degraded && card.roaming && card.disconnecting && card.mosh);
    assert_eq!(card.udp_port, 60100);
    let mut v = view(SessionState::Connected);
    v.degrade = Some(Degrade::TmuxMissing);
    c.session = Some(v);
    assert!(
        !build(&[c.clone()], &no_monitor).connections[0].degraded,
        "only mosh's fall back to SSH is a degraded transport"
    );
    c.session = None;
    let card = &build(&[c], &no_monitor).connections[0];
    assert!(
        card.disconnecting,
        "a disconnect under way, its session already gone"
    );
    assert_eq!(card.transport, Transport::Live);
    assert!(!card.degraded && card.connected_at.is_none());
}

// MARK: - A host's face (§4.13)

/// A trusted key's fingerprint as the known hosts keep it.
const KEY: &str = "SHA256:nThbg6kXUpJWGl7E1IGOCspRomTxdCARLviKw6E5SY8";

/// `face`'s rows, `#` filled and `.` not.
fn drawn(face: &Face) -> Vec<String> {
    let side = usize::from(face.side);
    (0..side)
        .map(|row| {
            (0..side)
                .map(|column| if face.filled(row, column) { '#' } else { '.' })
                .collect()
        })
        .collect()
}

/// The same fingerprint, the same face: pinned, so a change to how a face
/// is made shows here (every host's face would change with it).
#[test]
fn a_fingerprint_makes_a_pinned_face() {
    let made = face(KEY);
    assert_eq!(made, face(KEY), "the same key, the same face");
    assert_eq!(made.side, 5);
    assert_eq!(made.cells.len(), 25);
    assert_eq!(
        drawn(&made),
        [".#.#.", "..#..", "#...#", "..#..", "#...#"],
        "{made:?}"
    );
    assert_eq!(made.hue, 5, "blue");
}

/// Today's faces are mirrored left to right, every one; none is empty.
#[test]
fn a_face_is_mirror_symmetric() {
    for n in 0..64 {
        let face = face(&format!("SHA256:key-{n}"));
        let side = usize::from(face.side);
        assert_eq!(face.cells.len(), side * side);
        for row in 0..side {
            for column in 0..side {
                assert_eq!(
                    face.filled(row, column),
                    face.filled(row, side - 1 - column),
                    "key {n}: row {row}, column {column}"
                );
            }
        }
        assert!(face.cells.iter().any(|c| *c), "key {n}: an empty face");
        assert!(usize::from(face.hue) < moshpit_ui::ui::tokens::FACES.len());
    }
}

/// Different keys look different: across a sample of a thousand keys, the
/// faces are all but all distinct, and every hue is used.
#[test]
fn different_keys_make_different_faces() {
    let faces: Vec<_> = (0..1000)
        .map(|n| face(&format!("SHA256:host-{n}")))
        .collect();
    let distinct: std::collections::BTreeSet<_> =
        faces.iter().map(|f| (f.cells.clone(), f.hue)).collect();
    assert!(distinct.len() >= 990, "{} distinct of 1000", distinct.len());
    let hues: std::collections::BTreeSet<_> = faces.iter().map(|f| f.hue).collect();
    assert_eq!(hues.len(), moshpit_ui::ui::tokens::FACES.len());
    assert_ne!(face("SHA256:a"), face("SHA256:b"));
}

/// A card's mark: the emoji picked, else the trusted key's face, else (no
/// key trusted yet, a host never connected) only the initial.
#[test]
fn a_card_has_its_keys_face_its_emoji_or_neither() {
    let mut keyed = saved(STUDIO, "studio");
    keyed.fingerprint = Some(KEY.into());
    let mut whale = saved("C0FFEE00-0000-4000-8000-000000000002", "whale");
    whale.fingerprint = Some(KEY.into());
    whale.emoji = Some("🐳".into());
    let fresh = saved("C0FFEE00-0000-4000-8000-000000000003", "fresh");
    let home = build(&[keyed, whale, fresh], &|_| BTreeMap::new());
    let [keyed, whale, fresh] = &home.connections[..] else {
        panic!("three cards");
    };
    assert_eq!(keyed.face.as_ref(), Some(&face(KEY)));
    assert_eq!(keyed.emoji, None);
    assert_eq!(whale.emoji.as_deref(), Some("🐳"));
    assert_eq!(
        whale.face.as_ref(),
        Some(&face(KEY)),
        "the face is kept under the emoji"
    );
    assert_eq!(fresh.face, None, "no key trusted: no face");
    assert_eq!(fresh.initial, "F");
    // The screen takes them.
    let json = message(&HomeInput {
        connections: &[saved(STUDIO, "studio")],
        since: &|_| BTreeMap::new(),
        version: "v1.0.9 (1)",
        sidebar: false,
        locale: "en",
    });
    assert!(Message::parse(&json).is_ok());
}
