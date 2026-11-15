//! `home_crosscheck <scenarios.json> [scenario ...]`: one line per scenario,
//! `{"scenario": name, "message": …}`, Home's message as `app::home` builds
//! it from the fixture's session views — the Rust side of
//! `harness/home/crosscheck.py`, against the app's Swift `HomeSnapshot`
//! (`harness/home/build-swift.sh`).

use std::collections::BTreeMap;

use moshpit_app::home::{self, HomeInput, SavedConnection};
use moshpit_herdr::snapshot as herdr;
use moshpit_session::{
    Degrade, EndReason, Failure, HerdrView, SessionState, SessionView, TmuxView,
};
use moshpit_tmux::model as tmux;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
struct Scenario {
    name: String,
    connections: Vec<Saved>,
    version: Option<String>,
    #[serde(default)]
    sidebar: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    id: String,
    name: String,
    user: Option<String>,
    host: Option<String>,
    #[serde(default)]
    mosh: bool,
    udp_port: Option<u32>,
    mux: Option<String>,
    #[serde(default)]
    disconnecting: bool,
    session: Option<Session>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    state: String,
    #[serde(default)]
    stalled: bool,
    connected_at: Option<f64>,
    #[serde(default)]
    degraded: bool,
    #[serde(default)]
    roaming: bool,
    #[serde(default)]
    since: BTreeMap<String, f64>,
    tmux: Option<Tree>,
    herdr: Option<Tree>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Tree {
    #[serde(default)]
    attached: bool,
    #[serde(default)]
    ever_attached: bool,
    #[serde(default)]
    refreshing: bool,
    #[serde(default)]
    sessions: Vec<TreeSession>,
    #[serde(default)]
    windows: Vec<TreeWindow>,
    #[serde(default)]
    panes: Vec<TreePane>,
    active_session: Option<String>,
    active_window: Option<String>,
    active_pane: Option<String>,
    #[serde(default)]
    hooks: Vec<TreeHook>,
    // herdr's
    #[serde(default)]
    cwds: BTreeMap<String, String>,
    #[serde(default)]
    worktrees: BTreeMap<String, String>,
    mismatch: Option<String>,
    skew: Option<Skew>,
    #[serde(default)]
    restarting: bool,
    // tmux's: the server was found with no session (B228).
    #[serde(default)]
    no_session: bool,
}

#[derive(Deserialize)]
struct TreeSession {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    attached: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TreeWindow {
    id: String,
    session: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    index: i64,
    pane_count: Option<i64>,
}

#[derive(Deserialize)]
struct TreePane {
    id: String,
    window: String,
    #[serde(default)]
    index: i64,
    #[serde(default)]
    command: String,
    path: Option<String>,
}

#[derive(Deserialize)]
struct TreeHook {
    pane: String,
    state: Option<String>,
    agent: Option<String>,
    since: Option<f64>,
}

#[derive(Deserialize)]
struct Skew {
    client: String,
    server: String,
}

fn state(name: &str) -> SessionState {
    match name {
        "connecting" => SessionState::Connecting,
        "reconnecting" => SessionState::Reconnecting,
        "connected" => SessionState::Connected,
        "ended" => SessionState::Ended(EndReason::Disconnected),
        "failed" => SessionState::Failed(Failure::lost("failed")),
        other => panic!("state {other}"),
    }
}

fn tmux_view(t: &Tree) -> TmuxView {
    let mut snapshot = tmux::Snapshot::default();
    for s in &t.sessions {
        snapshot.sessions.insert(
            s.id.clone(),
            tmux::SessionInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                is_attached: s.attached,
            },
        );
    }
    for w in &t.windows {
        let mut window = tmux::WindowInfo::new(&w.id, &w.session);
        window.name = w.name.clone();
        window.index = w.index;
        window.pane_count = w.pane_count.unwrap_or(1);
        snapshot.windows.insert(w.id.clone(), window);
    }
    for p in &t.panes {
        let mut pane = tmux::PaneInfo::placeholder(&p.id, &p.window);
        pane.index = p.index;
        pane.command = p.command.clone();
        pane.path = p.path.clone();
        snapshot.panes.insert(p.id.clone(), pane);
    }
    snapshot.active_session_id = t.active_session.clone();
    snapshot.active_window_id = t.active_window.clone();
    snapshot.active_pane_id = t.active_pane.clone();
    snapshot.is_attached = t.attached;
    snapshot.ever_attached = t.ever_attached;
    TmuxView {
        snapshot,
        notice: None,
        refreshing: t.refreshing,
        size_follow: None,
        pane_follow: None,
        agent_hooks: t
            .hooks
            .iter()
            .map(|h| {
                (
                    h.pane.clone(),
                    moshpit_tmux::hooks::AgentHook {
                        state: h.state.clone(),
                        agent: h.agent.clone(),
                        since: h.since,
                        command: None,
                        title: None,
                    },
                )
            })
            .collect(),
        agent_hooks_loaded: true,
        input: Default::default(),
        no_session: t.no_session,
    }
}

fn agent_state(name: &str) -> herdr::AgentState {
    match name {
        "working" => herdr::AgentState::Working,
        "attention" => herdr::AgentState::Attention,
        "done" => herdr::AgentState::Done,
        "idle" => herdr::AgentState::Idle,
        other => panic!("herdr state {other}"),
    }
}

fn herdr_view(t: &Tree) -> HerdrView {
    let mut snapshot = herdr::Snapshot::default();
    for s in &t.sessions {
        snapshot.sessions.insert(
            s.id.clone(),
            herdr::Session {
                id: s.id.clone(),
                name: s.name.clone(),
                is_attached: s.attached,
            },
        );
    }
    for w in &t.windows {
        snapshot.windows.insert(
            w.id.clone(),
            herdr::Window {
                id: w.id.clone(),
                session_id: w.session.clone(),
                name: w.name.clone(),
                index: w.index,
                layout: String::new(),
                is_active: false,
                pane_count: w.pane_count.unwrap_or(1),
                is_zoomed: false,
            },
        );
    }
    for p in &t.panes {
        assert!(p.path.is_none(), "a herdr pane's directory is in cwds");
        snapshot.panes.insert(
            p.id.clone(),
            herdr::Pane {
                id: p.id.clone(),
                window_id: p.window.clone(),
                index: p.index,
                command: p.command.clone(),
                width: 80,
                height: 24,
                is_active: false,
            },
        );
    }
    snapshot.active_session_id = t.active_session.clone();
    snapshot.active_window_id = t.active_window.clone();
    snapshot.active_pane_id = t.active_pane.clone();
    snapshot.is_attached = t.attached;
    snapshot.ever_attached = t.ever_attached;
    HerdrView {
        snapshot,
        agent_hooks: t
            .hooks
            .iter()
            .map(|h| {
                assert!(h.since.is_none(), "herdr's stamps carry no time");
                (
                    h.pane.clone(),
                    herdr::AgentHook {
                        state: h.state.as_deref().map(agent_state),
                        agent: h.agent.clone(),
                        title: None,
                    },
                )
            })
            .collect(),
        agent_hooks_loaded: true,
        notice: None,
        refreshing: t.refreshing,
        restarting: t.restarting,
        server_not_running: false,
        protocol_mismatch: t.mismatch.clone(),
        version_skew: t
            .skew
            .as_ref()
            .map(|s| moshpit_herdr::version::VersionReport {
                client: s.client.clone(),
                server: s.server.clone(),
            }),
        server_version: None,
        push_active: false,
        pane_switch_in_flight: false,
        server_focus: None,
        pane_cwds: t.cwds.clone(),
        worktree_repos: t.worktrees.clone(),
        open_repo_guesses: Vec::new(),
        frame_target: None,
        attached_pane: None,
        retry_wait_ms: None,
    }
}

fn view(s: &Session) -> SessionView {
    let mut view = SessionView::initial();
    view.state = state(&s.state);
    view.attach_stalled = s.stalled;
    view.connected_at = s.connected_at;
    view.degrade = s.degraded.then_some(Degrade::MoshServerMissing);
    view.mosh_roaming = s.roaming;
    view.tmux_view = s.tmux.as_ref().map(tmux_view);
    view.herdr = s.herdr.as_ref().map(herdr_view);
    view
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("scenarios.json");
    let wanted: Vec<String> = args.collect();
    let fixture: Fixture =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("fixture");
    for scenario in fixture.scenarios {
        if !wanted.is_empty() && !wanted.contains(&scenario.name) {
            continue;
        }
        let connections: Vec<SavedConnection> = scenario
            .connections
            .iter()
            .map(|c| SavedConnection {
                id: c.id.clone(),
                name: c.name.clone(),
                fingerprint: None,
                emoji: None,
                user: c.user.clone().unwrap_or_else(|| "cluas".into()),
                host: c.host.clone().unwrap_or_else(|| "studio.local".into()),
                mosh: c.mosh,
                udp_port: c.udp_port.unwrap_or(60001),
                tmux: c.mux.as_deref().unwrap_or("tmux") == "tmux",
                disconnecting: c.disconnecting,
                session: c.session.as_ref().map(view),
            })
            .collect();
        let since: BTreeMap<String, BTreeMap<String, f64>> = scenario
            .connections
            .iter()
            .filter_map(|c| Some((c.id.clone(), c.session.as_ref()?.since.clone())))
            .collect();
        let message = home::message(&HomeInput {
            connections: &connections,
            since: &|conn| since.get(conn).cloned().unwrap_or_default(),
            version: scenario.version.as_deref().unwrap_or("v1.0.9 (1)"),
            sidebar: scenario.sidebar,
            locale: "en",
        });
        let line = serde_json::json!({
            "scenario": scenario.name,
            "message": serde_json::from_str::<serde_json::Value>(&message).expect("json"),
        });
        println!("{line}");
    }
}
