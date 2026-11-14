//! The mosh path against real programs (rc1's hunt, B400–B402): a private
//! sshd, mosh-server started through it, a private tmux server
//! (`tmux -L`). What the person sees when the server dies, the return path
//! heals or the control plane goes quiet.
//!
//! Ignored by default (they need sshd, mosh-server and tmux).
//!
//! ```sh
//! cargo test -p moshpit-session --test live_mosh_hunt -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use moshpit_session::{
    EventKind, Multiplexer, Protocol, Services, SessionId, SessionState, Timing, TmuxAction,
};
use support::*;

/// A bench on the system's UDP, no proxy: what the phone's socket hears
/// is what the kernel says, refusals included (the proxy swallows them).
fn direct(sshd: &Sshd) -> Bench {
    let services = Services::new(Arc::new(Keys), trusting(sshd.addr()));
    Bench::new(services, Timing::default())
}

/// Kills the session's mosh-server and waits until it is dialed again,
/// within `seconds` of the kill. The screen's terminal after it (the same
/// stream's if it carries on, the position before the kill to read from),
/// and the new server's reaper.
async fn kill_its_server(
    bench: &mut Bench,
    id: SessionId,
    mut term: Term,
    label: &str,
) -> (Term, usize, Reaper) {
    let first = bench.sessions.view(id).unwrap().mosh_server_pid.unwrap();
    term.read();
    let mark = term.screen.len();
    let killed = Instant::now();
    let _ = std::process::Command::new("kill")
        .args(["-9", &first.to_string()])
        .output();
    bench
        .state(id, 15, |s| *s == SessionState::Reconnecting)
        .await;
    let noticed = killed.elapsed();
    bench.state(id, 30, |s| *s == SessionState::Connected).await;
    let second = bench.sessions.view(id).unwrap().mosh_server_pid.unwrap();
    eprintln!("{label}: dialing again {noticed:?} after the kill");
    assert_ne!(second, first, "{label}: a new mosh-server");
    assert!(noticed < Duration::from_secs(12), "{label}: {noticed:?}");
    let (_, panes) = bench.panes(id, 20).await;
    if panes[0].stream == term.stream {
        (term, mark, Reaper(Some(second)))
    } else {
        drop(term);
        (Term::attach(panes[0].stream), 0, Reaper(Some(second)))
    }
}

/// B402: mosh-server killed under a live session (the host rebooted, the
/// server was OOM-killed or timed out while the phone was away): its port
/// refuses, the session dials again within seconds, a new mosh-server,
/// typing works — with tmux, the same session is rendered again. It used
/// to stay Connected on a frozen screen for good, the return-path banner
/// offering SSH on a network that was fine.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd and mosh-server"]
async fn a_killed_mosh_server_is_dialed_again() {
    let sshd = Sshd::start().await;

    // Plain mosh.
    let mut bench = direct(&sshd);
    let mut wanted = spec(sshd.port);
    wanted.protocol = Protocol::Mosh;
    let id = bench.sessions.connect(wanted);
    bench.connected(id).await;
    let _first = Reaper(bench.sessions.view(id).unwrap().mosh_server_pid);
    let (_, panes) = bench.panes(id, 20).await;
    let mut term = Term::attach(panes[0].stream);
    term.write(b"echo r$((1+1))\r");
    assert!(term.wait_for("r2", 15).await, "{}", term.text());
    let (mut term, mark, _second) = kill_its_server(&mut bench, id, term, "plain mosh").await;
    term.write(b"echo r$((2+2))\r");
    assert!(
        term.wait_for_from(mark, "r4", 15).await.is_some(),
        "typing works: {}",
        term.text()
    );
    let _ = bench.sessions.disconnect(id).await;

    // mosh + tmux: the same tmux session, rendered again.
    let tmux = Tmux::named("rc1-gone");
    tmux.new_session("gone");
    tmux.run(&["send-keys", "-t", "gone", "echo kept-$((40+2))", "Enter"]);
    let mut bench = direct(&sshd);
    let mut wanted = spec(sshd.port);
    wanted.protocol = Protocol::Mosh;
    wanted.multiplexer = Multiplexer::Tmux;
    wanted.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(wanted);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let _first = Reaper(bench.sessions.view(id).unwrap().mosh_server_pid);
    let (_, panes) = bench.panes(id, 20).await;
    let mut term = Term::attach(panes[0].stream);
    term.resize(100, 30);
    assert!(term.wait_for("kept-42", 15).await, "{}", term.text());
    let (mut term, mark, _second) = kill_its_server(&mut bench, id, term, "mosh + tmux").await;
    assert!(
        term.wait_for_from(mark, "kept-42", 15).await.is_some(),
        "tmux's session, rendered again: {}",
        term.text()
    );
    let sessions = tmux.run(&["list-sessions", "-F", "#{session_name}"]);
    assert_eq!(sessions.trim(), "gone", "the same session, no new one");
    let banners = bench
        .seen
        .iter()
        .filter(|e| e.session == id && e.kind == EventKind::ReturnPathDead)
        .count();
    assert_eq!(banners, 0, "no return-path banner");
    let _ = bench.sessions.disconnect(id).await;
}

/// Takes the events of the next `ms`, however quiet they are.
async fn settle(bench: &mut Bench, ms: u64) {
    let until = tokio::time::Instant::now() + Duration::from_millis(ms);
    while let Ok(Some(event)) = tokio::time::timeout_at(until, bench.events.recv()).await {
        bench.seen.push(event);
    }
}

/// mosh + tmux to `tmux`'s server, SSH through `relay`; once the sidecar
/// is attached and has settled.
async fn mosh_tmux_via(relay: &Relay, tmux: &Tmux, timing: Timing) -> (Bench, SessionId, Reaper) {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], relay.port));
    let mut bench = Bench::with(addr, timing);
    let mut wanted = spec(relay.port);
    wanted.protocol = Protocol::Mosh;
    wanted.multiplexer = Multiplexer::Tmux;
    wanted.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(wanted);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let reaper = Reaper(bench.sessions.view(id).unwrap().mosh_server_pid);
    for _ in 0..60 {
        if bench
            .sessions
            .tmux_snapshot(id)
            .is_some_and(|s| s.is_attached)
        {
            break;
        }
        settle(&mut bench, 250).await;
    }
    settle(&mut bench, 1500).await;
    (bench, id, reaper)
}

fn windows(tmux: &Tmux, session: &str) -> usize {
    tmux.run(&["list-windows", "-t", session]).lines().count()
}

fn dropped_notices(bench: &Bench, id: SessionId) -> usize {
    bench
        .seen
        .iter()
        .filter(|e| e.session == id && e.kind == EventKind::TmuxActionsDropped)
        .count()
}

/// B401: mosh + tmux, the control plane's connection goes quiet for good
/// (a NAT that forgot it, the old path after a roam) and the network
/// changes. The check finds it silent and a new control plane is up within
/// seconds; a tmux command given meanwhile waits for it and lands, once.
/// The plane was revived only if its connection had closed, which a quiet
/// one never does: the sidecar's watchdog noticed some 15 s later, and the
/// command, sent into the quiet connection, was lost with it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd, tmux and mosh-server"]
async fn a_quiet_control_plane_is_replaced_at_a_roam_and_a_command_meanwhile_lands() {
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::named("rc1-quiet");
    tmux.new_session("quiet");
    let (mut bench, id, _reaper) = mosh_tmux_via(&relay, &tmux, Timing::default()).await;
    let before = windows(&tmux, "quiet");
    let dialed = relay.connections();
    // Every connection open now goes quiet for good; new ones flow.
    relay.stall();
    relay.heal();
    let roamed = Instant::now();
    bench.sessions.network_changed();
    settle(&mut bench, 200).await;
    bench
        .sessions
        .tmux(id, TmuxAction::NewWindow { name: None })
        .unwrap();
    let mut landed = None;
    for _ in 0..40 {
        settle(&mut bench, 250).await;
        if windows(&tmux, "quiet") > before {
            landed = Some(roamed.elapsed());
            break;
        }
    }
    eprintln!("the new window landed {landed:?} after the roam");
    assert!(relay.connections() > dialed, "a new control connection");
    let landed = landed.expect("the command given meanwhile landed");
    assert!(landed < Duration::from_secs(6), "{landed:?}");
    settle(&mut bench, 1000).await;
    assert_eq!(windows(&tmux, "quiet"), before + 1, "once");
    assert_eq!(dropped_notices(&bench, id), 0);
    let _ = bench.sessions.disconnect(id).await;
}

/// B401: a command given while the control plane is down, which stays
/// down past the bound (its redial hangs), is dropped rather than run
/// late, and once the plane is back the person is told it didn't go
/// through.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd, tmux and mosh-server"]
async fn a_command_held_past_the_bound_is_dropped_and_said_once_the_plane_is_back() {
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::named("rc1-held");
    tmux.new_session("held");
    // A redial into a stalled line fails in 4 s, not 15.
    let timing = Timing {
        handshake_timeout: Duration::from_secs(4),
        ..Timing::default()
    };
    let (mut bench, id, _reaper) = mosh_tmux_via(&relay, &tmux, timing).await;
    let before = windows(&tmux, "held");
    // Everything goes quiet, new connections too, for a while.
    relay.stall();
    bench.sessions.network_changed();
    settle(&mut bench, 200).await;
    bench
        .sessions
        .tmux(id, TmuxAction::NewWindow { name: None })
        .unwrap();
    settle(&mut bench, 6500).await;
    assert_eq!(
        dropped_notices(&bench, id),
        0,
        "not before the plane is back"
    );
    relay.heal();
    let back = Instant::now();
    bench
        .until(id, 20, |kind| {
            (*kind == EventKind::TmuxActionsDropped).then_some(())
        })
        .await;
    eprintln!("the notice came {:?} after the line healed", back.elapsed());
    settle(&mut bench, 1500).await;
    assert_eq!(windows(&tmux, "held"), before, "dropped, not run late");
    assert_eq!(dropped_notices(&bench, id), 1, "said once");
    let _ = bench.sessions.disconnect(id).await;
}

/// B400: mosh's return path dies after contact (a VPN or a proxy that
/// passes UDP out and drops it in, a lift) and the banner offering SSH
/// comes up; the path heals, mosh hears the server again, and the banner
/// goes. The next outage brings it back. It stayed up on a working session,
/// and since the report was latched for the leg's life, after a dismiss a
/// second outage showed none.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd and mosh-server"]
async fn the_return_path_banner_goes_when_mosh_hears_the_server_again() {
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), Timing::default());
    let mut wanted = spec(sshd.port);
    wanted.protocol = Protocol::Mosh;
    let id = bench.sessions.connect(wanted);
    bench.connected(id).await;
    let _reaper = Reaper(bench.sessions.view(id).unwrap().mosh_server_pid);
    let (_, panes) = bench.panes(id, 20).await;
    let mut term = Term::attach(panes[0].stream);
    term.write(b"echo r$((1+1))\r");
    assert!(term.wait_for("r2", 15).await, "{}", term.text());
    for outage in 0..2 {
        bench.udp.faults.blackhole(true);
        bench
            .until(id, 40, |kind| {
                (*kind == EventKind::ReturnPathDead).then_some(())
            })
            .await;
        assert!(bench.sessions.view(id).unwrap().return_path_dead);
        bench.udp.faults.blackhole(false);
        let healed = Instant::now();
        bench
            .until(id, 10, |kind| {
                (*kind == EventKind::ReturnPathAlive).then_some(())
            })
            .await;
        eprintln!(
            "outage {outage}: the banner went {:?} after the path healed",
            healed.elapsed()
        );
        let view = bench.sessions.view(id).unwrap();
        assert!(!view.return_path_dead, "outage {outage}");
        assert_eq!(view.state, SessionState::Connected);
    }
    term.read();
    let mark = term.screen.len();
    term.write(b"echo r$((2+2))\r");
    assert!(
        term.wait_for_from(mark, "r4", 15).await.is_some(),
        "typing works: {}",
        term.text()
    );
    let _ = bench.sessions.disconnect(id).await;
}
