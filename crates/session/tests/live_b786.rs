//! B786: the tmux server dies under a live SSH session — `kill-server`,
//! `kill -9` of the server (the "abnormal" exit), or a crash (SIGSEGV); and
//! B787, under a mosh session whose phone was away. What the session says and
//! what its view still holds of the dead server: its sessions, the agent
//! stamps Home draws as rows, the panes a tap would open.
//!
//! Ignored by default (they need sshd and tmux): see `live.rs`.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_b786 -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::time::Duration;

use moshpit_session::{Multiplexer, SessionState};
use support::*;

macro_rules! need_sshd {
    () => {
        if !Sshd::available() {
            eprintln!("skipped: no {SSHD}");
            return;
        }
    };
}

fn tmux_spec(port: u16, tmux: &Tmux) -> moshpit_session::ConnectionSpec {
    let mut spec = spec(port);
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    spec
}

/// A session `work` whose pane carries a finished agent's stamp, as the
/// hooks write it.
fn stamped(tmux: &Tmux) {
    tmux.new_session("work");
    for (option, value) in [
        ("@moshpit_state", "done"),
        ("@moshpit_agent", "claude"),
        ("@moshpit_since", "1790000000"),
        ("@moshpit_title", "builder"),
    ] {
        tmux.run(&["set-option", "-p", "-t", "work", option, value]);
    }
}

/// What the session says and holds, every second for `seconds` (for
/// looking into a failure: `--nocapture`).
#[allow(dead_code)]
async fn watch(bench: &mut Bench, id: moshpit_session::SessionId, seconds: u64, label: &str) {
    for second in 0..seconds {
        while let Some(event) = bench.next(0).await {
            if event.session == id {
                eprintln!("[{label} {second}s] event {:?}", event.kind);
            }
        }
        let view = bench.sessions.view(id).unwrap();
        let tree = bench.sessions.tmux_snapshot(id);
        eprintln!(
            "[{label} {second}s] state {:?} attach {} panes {:?} tmux attached {:?} sessions {:?} hooks {:?} stalled {} no_session {:?}",
            view.state,
            view.attach,
            view.panes
                .iter()
                .map(|p| p.pane.clone())
                .collect::<Vec<_>>(),
            tree.as_ref().map(|t| t.is_attached),
            tree.as_ref().map(|t| t
                .sessions
                .values()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()),
            bench
                .sessions
                .tmux_view(id)
                .map(|v| v.agent_hooks.keys().cloned().collect::<Vec<_>>()),
            view.attach_stalled,
            bench.sessions.tmux_view(id).map(|v| v.no_session),
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Waits up to `seconds` for the session to hold nothing of the dead
/// server: still connected (SSH is fine), the empty state that offers a new
/// session, no session in its tree, no agent's stamp for Home to draw as a
/// row, no pane a tap would open. Says what it holds when it doesn't.
async fn forgets_the_server(bench: &mut Bench, id: moshpit_session::SessionId, seconds: u64) {
    for _ in 0..seconds * 4 {
        while bench.next(0).await.is_some() {}
        let view = bench.sessions.view(id).unwrap();
        let tree = bench.sessions.tmux_view(id);
        let gone = view.state == SessionState::Connected
            && tree.as_ref().is_some_and(|t| {
                t.no_session && t.snapshot.sessions.is_empty() && t.agent_hooks.is_empty()
            })
            && view.panes.is_empty();
        if gone {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let view = bench.sessions.view(id).unwrap();
    let tree = bench.sessions.tmux_view(id);
    panic!(
        "still holds the dead server: state {:?}, no_session {:?}, sessions {:?}, hooks {:?}, panes {:?}",
        view.state,
        tree.as_ref().map(|t| t.no_session),
        tree.as_ref().map(|t| t
            .snapshot
            .sessions
            .values()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()),
        tree.as_ref()
            .map(|t| t.agent_hooks.keys().cloned().collect::<Vec<_>>()),
        view.panes
            .iter()
            .map(|p| p.pane.clone())
            .collect::<Vec<_>>(),
    );
}

/// Connected to `work` with its agent's stamp on Home's side.
async fn attached(tmux: &Tmux) -> (Sshd, Bench, moshpit_session::SessionId) {
    let sshd = Sshd::start().await;
    stamped(tmux);
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(tmux_spec(sshd.port, tmux));
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    for _ in 0..40 {
        while bench.next(0).await.is_some() {}
        if bench
            .sessions
            .tmux_view(id)
            .is_some_and(|t| t.snapshot.is_attached && t.agent_hooks.contains_key("%0"))
        {
            return (sshd, bench, id);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("never attached with the stamp");
}

/// B786: `kill-server` under a live SSH attach. tmux's control client says
/// `%exit` and the shell runs on after it, so nothing read the channel as
/// dead: the session stayed connected with the dead server's session, its
/// agent's "done" row and its pane — Home's live card, "All sessions 1",
/// and a blank terminal behind the row. Now the server is forgotten and
/// the empty state offers a new session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn a_tmux_server_killed_under_the_attach_is_forgotten() {
    need_sshd!();
    let tmux = Tmux::named("moshpit-b786a");
    let (_sshd, mut bench, id) = attached(&tmux).await;
    tmux.run(&["kill-server"]);
    forgets_the_server(&mut bench, id, 10).await;
    let _ = bench.sessions.disconnect(id).await;
}

/// B786, the abnormal exit: the server `kill -9`ed. The same.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn a_tmux_server_that_crashed_is_forgotten() {
    need_sshd!();
    let tmux = Tmux::named("moshpit-b786b");
    let (_sshd, mut bench, id) = attached(&tmux).await;
    let pid = tmux.run(&["display-message", "-p", "#{pid}"]);
    assert!(!pid.is_empty());
    let killed = std::process::Command::new("kill")
        .args(["-9", &pid])
        .status()
        .unwrap();
    assert!(killed.success());
    forgets_the_server(&mut bench, id, 10).await;
    let _ = bench.sessions.disconnect(id).await;
}

/// B786 follow-up (B812's crash, Canary 148): tmux 3.6a died by SIGSEGV
/// on the test Mac. A crash is not a kill: the server doesn't get to say
/// goodbye, and the control client reads only the socket closing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn a_tmux_server_that_segfaulted_is_forgotten() {
    need_sshd!();
    let tmux = Tmux::named("moshpit-b786d");
    let (_sshd, mut bench, id) = attached(&tmux).await;
    let pid = tmux.run(&["display-message", "-p", "#{pid}"]);
    assert!(!pid.is_empty());
    let killed = std::process::Command::new("kill")
        .args(["-SEGV", &pid])
        .status()
        .unwrap();
    assert!(killed.success());
    forgets_the_server(&mut bench, id, 10).await;
    let _ = bench.sessions.disconnect(id).await;
}

/// B786: the control client detached while the server keeps its sessions
/// (`detach-client` from another terminal) is attached again — its tree,
/// stamps and pane back — not left showing a tree it no longer gets news
/// of.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn a_detached_control_client_attaches_again() {
    need_sshd!();
    let tmux = Tmux::named("moshpit-b786c");
    let (_sshd, mut bench, id) = attached(&tmux).await;
    let attach = bench.sessions.view(id).unwrap().attach;
    // Our client sits on a session grouped with `work` (one per device):
    // every client is detached, by name.
    let clients = tmux.run(&["list-clients", "-F", "#{client_name}"]);
    assert!(!clients.is_empty());
    for client in clients.lines() {
        tmux.run(&["detach-client", "-t", client]);
    }
    for _ in 0..80 {
        while bench.next(0).await.is_some() {}
        let view = bench.sessions.view(id).unwrap();
        if view.attach > attach
            && view.state == SessionState::Connected
            && bench
                .sessions
                .tmux_view(id)
                .is_some_and(|t| t.snapshot.is_attached && !t.no_session)
        {
            // `work`, and the grouped session the app attaches through: no
            // session of its own made.
            let sessions = tmux.sessions();
            assert!(
                sessions
                    .iter()
                    .all(|s| s == "work" || s.starts_with("work~")),
                "{sessions:?}"
            );
            let _ = bench.sessions.disconnect(id).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let view = bench.sessions.view(id).unwrap();
    panic!(
        "never attached again: state {:?} attach {} (was {attach}), tree attached {:?}",
        view.state,
        view.attach,
        bench.sessions.tmux_snapshot(id).map(|t| t.is_attached)
    );
}

/// B787 (B786's follow-up; B812's crash on Canary 148): mosh + tmux, the
/// server dies by SIGSEGV while the phone is away, so mosh-server's goodbye
/// (its program, tmux's client, exited with the server) never reaches it,
/// and mosh-server gives up and exits. A network that drops ICMP never
/// says its port refuses either (B402). The session stayed connected on a
/// transport with no server: the last frame frozen, typing going nowhere,
/// the "isn't receiving data" banner. The sidecar knows the server has no
/// session, so the renderer is gone: the session ends, saying tmux on the
/// host stopped (`EndReason::TmuxGone`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn mosh_with_a_tmux_that_crashed_while_away_ends() {
    need_sshd!();
    let tmux = Tmux::named("moshpit-b787a");
    let sshd = Sshd::start().await;
    stamped(&tmux);
    let udp = ProxiedUdp::new();
    let mut bench = Bench::new(Bench::services(&udp, sshd.addr()), quick());
    let mut spec = tmux_spec(sshd.port, &tmux);
    spec.protocol = moshpit_session::Protocol::Mosh;
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    for _ in 0..40 {
        while bench.next(0).await.is_some() {}
        if bench
            .sessions
            .tmux_view(id)
            .is_some_and(|t| t.snapshot.is_attached)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    // The phone away: nothing passes either way when tmux crashes.
    udp.faults.blackhole(true);
    let pid = tmux.run(&["display-message", "-p", "#{pid}"]);
    assert!(!pid.is_empty());
    let killed = std::process::Command::new("kill")
        .args(["-SEGV", &pid])
        .status()
        .unwrap();
    assert!(killed.success());
    for _ in 0..80 {
        while bench.next(0).await.is_some() {}
        if bench.sessions.view(id).unwrap().state
            == SessionState::Ended(moshpit_session::EndReason::TmuxGone)
        {
            udp.faults.blackhole(false);
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let view = bench.sessions.view(id).unwrap();
    panic!(
        "still {:?} on a mosh-server that is gone, no_session {:?}",
        view.state,
        bench.sessions.tmux_view(id).map(|t| t.no_session)
    );
}
