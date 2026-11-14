//! R5's acceptance fixture (UI-DESIGN §8): a herdr host with 13 agents
//! over 12 workspaces (1 waiting, 2 working, 1 done, 9 idle), a tmux host
//! with 3 agents and other panes, an offline host and a saved one. Home's
//! model tests read it, and so do Home's screen tests (R6).

use moshpit_ui::home::model::{
    Connection, Control, Hook, Link, Mux, PaneInfo, SessionInfo, WindowInfo,
};

pub const AGENT: &str = "agent";

pub fn session(id: &str, name: &str) -> SessionInfo {
    SessionInfo {
        id: id.into(),
        name: name.into(),
    }
}

pub fn window(id: &str, session: &str, index: i64, name: &str) -> WindowInfo {
    WindowInfo {
        id: id.into(),
        session: session.into(),
        index,
        name: name.into(),
        pane_count: 1,
    }
}

pub fn pane(id: &str, window: &str, index: i64, command: &str, path: Option<&str>) -> PaneInfo {
    PaneInfo {
        id: id.into(),
        window: window.into(),
        index,
        command: command.into(),
        path: path.map(str::to_owned),
    }
}

pub fn hook(pane: &str, state: &str, agent: Option<&str>, since: Option<f64>) -> Hook {
    Hook {
        pane: pane.into(),
        state: Some(state.into()),
        agent: agent.map(str::to_owned),
        since,
    }
}

pub fn live(id: &str, name: &str, control: Control) -> Connection {
    Connection {
        id: id.into(),
        name: name.into(),
        link: Link::Live,
        control: Some(control),
        ..Connection::default()
    }
}

/// gpu-01: herdr, 12 workspaces (`~` three times, the rest projects), each
/// with one tab herdr called `Tab 1`; 13 agents (w1 has two). One waits
/// (trainer, w3), two work (api, w1; web, w2), one is done (docs, w5), nine
/// sit idle.
pub fn herdr_host() -> Connection {
    let names = [
        "api", "web", "trainer", "~", "docs", "infra", "~", "ml", "site", "bot", "cli", "~",
    ];
    let mut control = Control {
        mux: Mux::Herdr,
        attached: true,
        ..Control::default()
    };
    for (n, name) in names.iter().enumerate() {
        let w = format!("w{}", n + 1);
        control.sessions.push(session(&w, name));
        control
            .windows
            .push(window(&format!("{w}:t1"), &w, 0, "Tab 1"));
        let folder = if *name == "~" {
            format!("/home/dev/src/proj-{}", n + 1)
        } else {
            format!("/home/dev/src/{name}")
        };
        control.panes.push(pane(
            &format!("{w}:p1"),
            &format!("{w}:t1"),
            0,
            "2.1.322",
            Some(&folder),
        ));
    }
    control.panes.push(pane(
        "w1:p2",
        "w1:t1",
        1,
        "codex",
        Some("/home/dev/src/api"),
    ));
    for p in &control.panes {
        let (state, since) = match p.id.as_str() {
            "w3:p1" => ("attention", Some(1_000.0)),
            "w1:p1" => ("working", Some(2_000.0)),
            "w2:p1" => ("working", Some(2_500.0)),
            "w5:p1" => ("done", Some(3_000.0)),
            _ => ("idle", None),
        };
        let agent = if p.command == "codex" {
            "codex"
        } else {
            "claude"
        };
        control.hooks.push(hook(&p.id, state, Some(agent), since));
    }
    live("c-gpu", "gpu-01", control)
}

/// build-box: tmux, one session. Claude Code runs as `2.1.322` in a window
/// tmux named after it (trainer, working); claude idles in a window the user
/// named `frontend`; aider is done in `cli`. Five panes run no agent: a
/// shell at home, nvim, a log the user named, htop, another shell.
pub fn tmux_host() -> Connection {
    let me = "/Users/me";
    let control = Control {
        mux: Mux::Tmux,
        attached: true,
        sessions: vec![session("$0", "work")],
        windows: vec![
            window("@1", "$0", 1, "2.1.322"),
            window("@2", "$0", 2, "frontend"),
            window("@3", "$0", 3, "aider"),
            window("@4", "$0", 4, "zsh"),
            window("@5", "$0", 5, "nvim"),
            window("@6", "$0", 6, "logs"),
            window("@7", "$0", 7, "htop"),
            window("@8", "$0", 8, "bash"),
        ],
        panes: vec![
            pane("%1", "@1", 0, "2.1.322", Some(&format!("{me}/src/trainer"))),
            pane("%2", "@2", 0, "claude", Some(&format!("{me}/src/api"))),
            pane("%3", "@3", 0, "aider", Some(&format!("{me}/src/cli"))),
            pane("%4", "@4", 0, "zsh", Some(me)),
            pane("%5", "@5", 0, "nvim", Some(&format!("{me}/src/api"))),
            pane("%6", "@6", 0, "tail", Some("/var/log")),
            pane("%7", "@7", 0, "htop", Some(me)),
            pane("%8", "@8", 0, "bash", Some("/tmp")),
        ],
        hooks: vec![
            hook("%1", "working", None, Some(4_000.0)),
            hook("%2", "idle", Some("claude"), None),
            hook("%3", "done", None, Some(4_500.0)),
        ],
        ..Control::default()
    };
    live("c-box", "build-box", control)
}

/// The acceptance fixture: herdr 13 + tmux 3, 1 waiting, 1 offline, 1 saved.
pub fn fixture() -> Vec<Connection> {
    vec![
        herdr_host(),
        tmux_host(),
        Connection {
            id: "c-edge".into(),
            name: "edge".into(),
            link: Link::Dead,
            ..Connection::default()
        },
        Connection {
            id: "c-lab".into(),
            name: "lab".into(),
            ..Connection::default()
        },
    ]
}
