//! The mosh path, and a tmux session's redial, against real programs
//! (the hunts, B400–B402, B483, B485): a private sshd, mosh-server started
//! through it, a private tmux server (`tmux -L`). What the person sees when
//! the server dies, the return path heals, the control plane goes quiet or
//! the connection is dialled again.
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
    let tmux = Tmux::named("hunt-gone");
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
    let sessions: Vec<&str> = sessions
        .lines()
        .filter(|name| !moshpit_tmux::controller::is_group_name(name))
        .collect();
    assert_eq!(sessions, ["gone"], "the same session, no new one");
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
    tmux_via(relay, tmux, timing, Protocol::Mosh).await
}

/// tmux on `tmux`'s server over `protocol`, SSH through `relay`; once the
/// attach (mosh's sidecar) has settled.
async fn tmux_via(
    relay: &Relay,
    tmux: &Tmux,
    timing: Timing,
    protocol: Protocol,
) -> (Bench, SessionId, Reaper) {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], relay.port));
    tmux_on(Bench::with(addr, timing), relay, tmux, protocol).await
}

/// tmux on `tmux`'s server over `protocol` from `bench`, SSH through
/// `relay`; once the attach (mosh's sidecar) has settled.
async fn tmux_on(
    mut bench: Bench,
    relay: &Relay,
    tmux: &Tmux,
    protocol: Protocol,
) -> (Bench, SessionId, Reaper) {
    let mut wanted = spec(relay.port);
    wanted.protocol = protocol;
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
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::named("hunt-quiet");
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
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::named("hunt-held");
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

/// Waits up to `seconds` for `tmux`'s `session` to have more than `before`
/// windows; how long that took.
async fn window_made(
    bench: &mut Bench,
    tmux: &Tmux,
    session: &str,
    before: usize,
    seconds: u64,
) -> Option<Duration> {
    let start = Instant::now();
    for _ in 0..seconds * 4 {
        settle(bench, 250).await;
        if windows(tmux, session) > before {
            return Some(start.elapsed());
        }
    }
    None
}

/// The connection under a tmux session is cut and every new one hangs
/// until the line heals: the session is redialling, with no leg.
async fn cut_and_stall(bench: &mut Bench, relay: &Relay, id: SessionId) {
    relay.stall();
    relay.cut();
    bench
        .state(id, 15, |s| *s == SessionState::Reconnecting)
        .await;
}

/// The line heals and the redial, out for over a second by now, starts
/// again on it (the path change a phone sees).
async fn heal_and_redial(bench: &mut Bench, relay: &Relay) {
    settle(bench, 1200).await;
    relay.heal();
    bench.sessions.network_changed();
}

/// B483: SSH + tmux, the connection drops and is being redialled; a tmux
/// command given meanwhile (a new window from the chrome, which the
/// reconnect veil leaves usable) waits for the next attach and lands
/// there, once. With no leg to carry it, it was dropped without a word.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd and tmux"]
async fn a_tmux_command_given_while_redialling_lands_on_the_next_attach() {
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::named("hunt-redial");
    tmux.new_session("redial");
    let (mut bench, id, _) = tmux_via(&relay, &tmux, Timing::default(), Protocol::Ssh).await;
    let before = windows(&tmux, "redial");
    cut_and_stall(&mut bench, &relay, id).await;
    bench
        .sessions
        .tmux(id, TmuxAction::NewWindow { name: None })
        .unwrap();
    heal_and_redial(&mut bench, &relay).await;
    let landed = window_made(&mut bench, &tmux, "redial", before, 10).await;
    eprintln!("the new window landed {landed:?} after the line healed");
    assert!(landed.is_some(), "the command given meanwhile landed");
    settle(&mut bench, 1000).await;
    assert_eq!(windows(&tmux, "redial"), before + 1, "once");
    assert_eq!(dropped_notices(&bench, id), 0);
    let _ = bench.sessions.disconnect(id).await;
}

/// B483: the same, the redial taking longer than the bound: the command
/// is dropped rather than run late, and said once the attach is back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd and tmux"]
async fn a_tmux_command_held_past_the_bound_through_a_redial_is_dropped_and_said() {
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::named("hunt-redial-late");
    tmux.new_session("late");
    let (mut bench, id, _) = tmux_via(&relay, &tmux, Timing::default(), Protocol::Ssh).await;
    let before = windows(&tmux, "late");
    cut_and_stall(&mut bench, &relay, id).await;
    bench
        .sessions
        .tmux(id, TmuxAction::NewWindow { name: None })
        .unwrap();
    settle(&mut bench, 6500).await;
    assert_eq!(dropped_notices(&bench, id), 0, "not before the attach");
    heal_and_redial(&mut bench, &relay).await;
    bench
        .until(id, 20, |kind| {
            (*kind == EventKind::TmuxActionsDropped).then_some(())
        })
        .await;
    settle(&mut bench, 1500).await;
    assert_eq!(windows(&tmux, "late"), before, "dropped, not run late");
    assert_eq!(dropped_notices(&bench, id), 1, "said once");
    let _ = bench.sessions.disconnect(id).await;
}

/// B483: SSH + tmux returning in place (`Timing::resume_in_place`) on a
/// connection that died without a word while the app was away: a tmux
/// command given while the return still asks the socket waits for the
/// answer and, the socket found dead, for the next attach, where it lands
/// once. It went into the dead socket.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd and tmux"]
async fn a_tmux_command_given_while_a_return_asks_the_socket_lands_after_it() {
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::named("hunt-return");
    tmux.new_session("back");
    let timing = Timing {
        resume_in_place: true,
        ..Timing::default()
    };
    let (mut bench, id, _) = tmux_via(&relay, &tmux, timing, Protocol::Ssh).await;
    let before = windows(&tmux, "back");
    bench.sessions.set_foreground(false);
    // Silent for good while away; new connections flow.
    relay.stall();
    relay.heal();
    settle(&mut bench, 500).await;
    bench.sessions.set_foreground(true);
    bench
        .sessions
        .tmux(id, TmuxAction::NewWindow { name: None })
        .unwrap();
    let landed = window_made(&mut bench, &tmux, "back", before, 10).await;
    eprintln!("the new window landed {landed:?} after the return");
    assert!(landed.is_some(), "the command given meanwhile landed");
    settle(&mut bench, 1000).await;
    assert_eq!(windows(&tmux, "back"), before + 1, "once");
    assert_eq!(dropped_notices(&bench, id), 0);
    let _ = bench.sessions.disconnect(id).await;
}

/// B483: mosh + tmux, mosh-server gone and the session dialled again (B402):
/// a tmux command given while there is no leg waits for the new control
/// plane and lands, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd, tmux and mosh-server"]
async fn a_tmux_command_given_while_mosh_is_redialled_lands_on_the_new_plane() {
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::named("hunt-redial-mosh");
    tmux.new_session("mosh");
    // The system's UDP: the killed server's port refuses at once (B402);
    // the proxy would swallow that.
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], relay.port));
    let services = Services::new(Arc::new(Keys), trusting(addr));
    let bench = Bench::new(services, Timing::default());
    let (mut bench, id, _first) = tmux_on(bench, &relay, &tmux, Protocol::Mosh).await;
    let before = windows(&tmux, "mosh");
    let server = bench.sessions.view(id).unwrap().mosh_server_pid.unwrap();
    // The redial's SSH hangs until the line heals.
    relay.stall();
    let _ = std::process::Command::new("kill")
        .args(["-9", &server.to_string()])
        .output();
    bench
        .state(id, 15, |s| *s == SessionState::Reconnecting)
        .await;
    bench
        .sessions
        .tmux(id, TmuxAction::NewWindow { name: None })
        .unwrap();
    heal_and_redial(&mut bench, &relay).await;
    let landed = window_made(&mut bench, &tmux, "mosh", before, 10).await;
    let _second = Reaper(bench.sessions.view(id).and_then(|v| v.mosh_server_pid));
    eprintln!("the new window landed {landed:?} after the line healed");
    assert!(landed.is_some(), "the command given meanwhile landed");
    settle(&mut bench, 1000).await;
    assert_eq!(windows(&tmux, "mosh"), before + 1, "once");
    assert_eq!(dropped_notices(&bench, id), 0);
    let _ = bench.sessions.disconnect(id).await;
}

/// A program that asks for bracketed paste and logs each read it gets,
/// with when (seconds, monotonic), until ^D.
const READ_LOGGER: &str = r#"
import os, sys, time, tty, termios
fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)
sys.stdout.write("\x1b[?2004h")
sys.stdout.flush()
log = open(sys.argv[1], "a", buffering=1)
log.write("ready\n")
try:
    while True:
        b = os.read(fd, 4096)
        if not b or b == b"\x04":
            break
        log.write("%.6f %r\n" % (time.monotonic(), b))
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
"#;

/// The reads [`READ_LOGGER`] wrote to `log`: when, and what (as Python
/// printed it).
fn reads(log: &std::path::Path) -> Vec<(f64, String)> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (at, what) = line.split_once(' ')?;
            Some((at.parse().ok()?, what.to_owned()))
        })
        .collect()
}

/// Gives the pane on `term` a paste and a Return at once, as the image
/// panel's Send does, and reads back how they reached the program: the
/// time from the read that ended the paste to the one with the Return.
async fn paste_then_return(term: &Term, log: &std::path::Path) -> f64 {
    // The pane takes typing now: a key goes through and is read first
    // (an attach still settling holds typing and lets it go at once).
    assert!(term.write(b"."));
    let mut through = false;
    for _ in 0..100 {
        if reads(log).iter().any(|(_, b)| b == "b'.'") {
            through = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(through, "a key never reached the pane: {:?}", reads(log));
    assert!(moshpit_stream::view::paste(
        term.stream,
        " /tmp/IMG-20261114-120000-a3f2.jpg "
    ));
    assert!(term.write(b"\r"));
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let got = reads(log);
        let pasted = got.iter().position(|(_, b)| b.contains(r"\x1b[201~"));
        let entered = got.iter().rposition(|(_, b)| b.contains(r"\r"));
        if let (Some(p), Some(e)) = (pasted, entered) {
            eprintln!("reads: {got:?}");
            // One read with both: no time apart at all.
            return if e == p { 0.0 } else { got[e].0 - got[p].0 };
        }
    }
    panic!("the paste and the Return never came: {:?}", reads(log));
}

/// How far apart, at least, the program reads the paste and the Return:
/// sent 150 ms apart, a loaded host can write the paste late and the Return
/// on time (84 ms seen with builds running); Claude Code needs about 5 ms,
/// and the old code had them together or 0.2 ms apart.
const APART: f64 = 0.05;

/// Waits for [`READ_LOGGER`] to say it runs.
async fn logger_ready(log: &std::path::Path) {
    for _ in 0..100 {
        if std::fs::read_to_string(log).is_ok_and(|s| s.starts_with("ready")) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the read logger never started");
}

/// B485: the image panel's Send is a paste and a Return given at once; the
/// Return reaches the program apart from the paste, 150 ms behind it.
/// Claude Code drops a Return that comes with the paste of an image's path
/// (it is still taking the paste in): it went out right behind the paste,
/// both reached the host together, and Send only pasted, as Insert does.
/// SSH + tmux, and a plain SSH shell.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd, tmux and python3"]
async fn a_return_given_with_a_paste_reaches_the_program_apart() {
    let sshd = Sshd::start().await;
    let scratch = Scratch::new("hunt-paste");
    let script = scratch.join("reads.py");
    std::fs::write(&script, READ_LOGGER).unwrap();

    // SSH + tmux: the pane runs the logger.
    let log = scratch.join("tmux.log");
    let tmux = Tmux::named("hunt-paste");
    tmux.new_session("paste");
    let program = format!("python3 -I {} {}", script.display(), log.display());
    tmux.run(&["respawn-pane", "-k", "-t", "paste", &program]);
    logger_ready(&log).await;
    let mut bench = direct(&sshd);
    let mut wanted = spec(sshd.port);
    wanted.multiplexer = Multiplexer::Tmux;
    wanted.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(wanted);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let (_, panes) = bench.panes(id, 20).await;
    let term = Term::attach(panes[0].stream);
    settle(&mut bench, 1500).await;
    let apart = paste_then_return(&term, &log).await;
    eprintln!("tmux: the Return came {apart:.3} s after the paste");
    assert!(apart >= APART, "tmux: {apart:.3} s");
    drop(term);
    let _ = bench.sessions.disconnect(id).await;

    // A plain SSH shell: the logger started by typing.
    let log = scratch.join("shell.log");
    let mut bench = direct(&sshd);
    let id = bench.sessions.connect(spec(sshd.port));
    bench.connected(id).await;
    let (_, panes) = bench.panes(id, 20).await;
    let term = Term::attach(panes[0].stream);
    let program = format!("exec python3 -I {} {}\r", script.display(), log.display());
    assert!(term.write(program.as_bytes()));
    logger_ready(&log).await;
    let apart = paste_then_return(&term, &log).await;
    eprintln!("shell: the Return came {apart:.3} s after the paste");
    assert!(apart >= APART, "shell: {apart:.3} s");
    drop(term);
    let _ = bench.sessions.disconnect(id).await;
}

/// B485, on herdr (the user's own pane is herdr's): the paste goes as one
/// bracketed paste with nothing after it (Insert submits nothing), and the
/// Return given with it reaches the program apart, 150 ms behind it, over
/// herdr's frame channel as over tmux.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd, herdr and python3"]
async fn a_return_given_with_a_paste_reaches_a_herdr_pane_apart() {
    if !PrivateHerdr::available() {
        eprintln!("skipped: no {HERDR}");
        return;
    }
    let scratch = Scratch::new("hunt-paste-herdr");
    let script = scratch.join("reads.py");
    std::fs::write(&script, READ_LOGGER).unwrap();
    let log = scratch.join("herdr.log");
    let herdr = PrivateHerdr::start();
    let pane = herdr.panes().first().cloned().expect("a pane");
    let program = format!(
        "exec /usr/bin/python3 -I {} {}",
        script.display(),
        log.display()
    );
    herdr.run(&["pane", "run", &pane, &program]);
    logger_ready(&log).await;
    let home = herdr.home.display().to_string();
    let sshd = Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await;
    let mut bench = Bench::with(sshd.addr(), Timing::default());
    let mut wanted = spec(sshd.port);
    wanted.multiplexer = Multiplexer::Herdr;
    wanted.herdr_path = Some(herdr.custom_path());
    wanted.pty_command = None;
    let id = bench.sessions.connect(wanted);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let (_, panes) = bench.panes(id, 20).await;
    let shown = panes.iter().find(|p| p.active).unwrap_or(&panes[0]);
    assert_eq!(shown.pane, pane, "{panes:?}");
    let term = Term::attach(shown.stream);
    settle(&mut bench, 2000).await;
    let apart = paste_then_return(&term, &log).await;
    eprintln!("herdr: the Return came {apart:.3} s after the paste");
    let got = reads(&log);
    let paste = got
        .iter()
        .find(|(_, b)| b.contains(r"\x1b[201~"))
        .map(|(_, b)| b.clone())
        .unwrap();
    assert!(
        paste.starts_with(r"b'\x1b[200~ /tmp/IMG-")
            && (apart == 0.0 || paste.ends_with(r"\x1b[201~'")),
        "one bracketed paste, nothing after it: {paste}"
    );
    assert!(apart >= APART, "herdr: {apart:.3} s");
    drop(term);
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
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
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
