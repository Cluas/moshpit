//! Sessions against real programs, on this machine's loopback: a private
//! OpenSSH sshd (as the current user, its own keys and config in a temp
//! directory), mosh-server started through it, a private tmux server — with
//! faults injected: TCP legs cut and stalled, UDP dropped and blackholed,
//! the app sent to the background and back, a clean `exit`. And the echo
//! path measured end to end, keystroke to bytes in the stream, for SSH and
//! mosh.
//!
//! Ignored by default (they need sshd, mosh-server and tmux, and take a
//! while). Nothing outside the temp directories and the private tmux socket
//! is touched; `~/.ssh` and the machine's sshd are left alone.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use moshpit_install::HostCapabilities;
use moshpit_session::{
    Degrade, Discarded, EndReason, EventKind, Multiplexer, PaneInfo, Protocol, SHELL_PANE,
    SessionMemory, SessionState, Timing, Transport,
};
use support::*;

/// The person's sessions: the phone's own group of one (`<name>~mp-…`,
/// made from tmux 3.8 on, B459/B812) is none of theirs.
fn theirs(sessions: Vec<String>) -> Vec<String> {
    sessions
        .into_iter()
        .filter(|name| !moshpit_tmux::controller::is_group_name(name))
        .collect()
}

macro_rules! need_sshd {
    () => {
        if !Sshd::available() {
            eprintln!("skipped: no {SSHD}");
            return;
        }
    };
}

/// The shell pane of a session that has one.
fn shell(panes: &[PaneInfo]) -> PaneInfo {
    let pane = panes.first().expect("a pane").clone();
    assert_eq!(pane.pane, SHELL_PANE);
    pane
}

// MARK: SSH

/// A plain shell: the host's bytes in the pane's channel, the person's keys
/// out; `exit` ENDS the session (B103) and nothing redials it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn ssh_shell_runs_and_exit_ends_the_session() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(spec(sshd.port));
    bench.connected(id).await;
    let (_, panes) = bench.panes(id, 10).await;
    let mut term = Term::attach(shell(&panes).stream);
    assert!(term.write(b"echo w10-$((6*7))\r"));
    assert!(term.wait_for("w10-42", 10).await, "{}", term.text());

    assert!(term.write(b"exit\r"));
    let state = bench
        .state(id, 10, |s| matches!(s, SessionState::Ended(_)))
        .await;
    assert_eq!(
        state,
        SessionState::Ended(EndReason::ShellExited { status: Some(0) })
    );
    // Nothing comes back: no keepalive, return or network change redials.
    bench.sessions.set_foreground(false);
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    bench.sessions.set_foreground(true);
    bench.sessions.network_changed();
    tokio::time::sleep(Duration::from_secs(4)).await;
    bench.drain();
    assert!(
        !bench.states(id).contains(&SessionState::Reconnecting),
        "{:#?}",
        bench.tail()
    );
    assert_eq!(sshd.accepted(), 1, "one sign-in: never redialled");
    let _ = bench.sessions.disconnect(id).await;
}

/// A new host: the card is asked once, the answer trusts it, the session
/// connects; a refusal fails it and nothing asks again on its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn ssh_new_host_is_asked_once_and_a_refusal_is_not_retried() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let known = Arc::new(moshpit_ssh::KnownHosts::in_memory(Default::default()));
    let udp = ProxiedUdp::new();
    let mut services = Bench::services(&udp, sshd.addr());
    services.known_hosts = known.clone();
    let mut bench = Bench::new(services, quick());

    let id = bench.sessions.connect(spec(sshd.port));
    let (prompt, fingerprint) = bench
        .until(id, 15, |kind| match kind {
            EventKind::HostKeyPrompt {
                prompt,
                fingerprint,
                previous: None,
                ..
            } => Some((*prompt, fingerprint.clone())),
            _ => None,
        })
        .await;
    assert_eq!(fingerprint, support::fingerprint("ed25519"));
    // The person reads it for a while: the handshake clock is stopped.
    tokio::time::sleep(Duration::from_secs(9)).await;
    bench.sessions.answer_host_key(id, prompt, false);
    let state = bench
        .state(id, 10, |s| matches!(s, SessionState::Failed(_)))
        .await;
    let SessionState::Failed(failure) = state else {
        unreachable!()
    };
    assert_eq!(failure.kind, moshpit_session::FailureKind::HostKey);
    assert!(!failure.retryable);
    // Ticks, a return, a network change: no new card.
    bench.sessions.set_foreground(false);
    tokio::time::sleep(Duration::from_millis(2_200)).await;
    bench.sessions.set_foreground(true);
    bench.sessions.network_changed();
    tokio::time::sleep(Duration::from_secs(4)).await;
    bench.drain();
    let cards = bench
        .seen
        .iter()
        .filter(|e| matches!(e.kind, EventKind::HostKeyPrompt { .. }))
        .count();
    assert_eq!(cards, 1, "{:#?}", bench.tail());

    // The person tries again and trusts it.
    bench.sessions.reconnect(id);
    let prompt = bench
        .until(id, 15, |kind| match kind {
            EventKind::HostKeyPrompt { prompt, .. } => Some(*prompt),
            _ => None,
        })
        .await;
    bench.sessions.answer_host_key(id, prompt, true);
    bench.connected(id).await;
    assert_eq!(
        known.snapshot().get(&format!("127.0.0.1:{}", sshd.port)),
        Some(&fingerprint)
    );
    let _ = bench.sessions.disconnect(id).await;
}

/// B391: the host hung up while the person read the card — its login
/// grace ran out (OpenSSH's LoginGraceTime, 120 s by default), here the
/// link cut under it. Their yes is kept, and the session connects on a
/// second dial at once: never a "the server closed the connection while
/// setting up SSH" failure in between, which sent people to check the port
/// until the next tick dialled again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn ssh_a_yes_given_after_the_host_hung_up_connects_without_a_failure() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let known = Arc::new(moshpit_ssh::KnownHosts::in_memory(Default::default()));
    let udp = ProxiedUdp::new();
    let mut services = Bench::services(
        &udp,
        std::net::SocketAddr::from(([127, 0, 0, 1], relay.port)),
    );
    services.known_hosts = known.clone();
    let mut bench = Bench::new(services, quick());

    let id = bench.sessions.connect(spec(relay.port));
    let prompt = bench
        .until(id, 15, |kind| match kind {
            EventKind::HostKeyPrompt { prompt, .. } => Some(*prompt),
            _ => None,
        })
        .await;
    // The host hangs up while the card is up; the person says yes after.
    relay.cut();
    tokio::time::sleep(Duration::from_millis(300)).await;
    bench.sessions.answer_host_key(id, prompt, true);
    bench.connected(id).await;
    let failed: Vec<SessionState> = bench
        .states(id)
        .into_iter()
        .filter(|state| matches!(state, SessionState::Failed(_)))
        .collect();
    assert!(failed.is_empty(), "{failed:#?}");
    assert_eq!(
        relay.connections(),
        2,
        "the dial the host hung up on, and one more"
    );
    let cards = bench
        .seen
        .iter()
        .filter(|e| matches!(e.kind, EventKind::HostKeyPrompt { .. }))
        .count();
    assert_eq!(cards, 1, "asked once");
    assert!(
        known
            .snapshot()
            .contains_key(&format!("127.0.0.1:{}", relay.port))
    );
    let _ = bench.sessions.disconnect(id).await;
}

/// B329: the connection was saved pointing elsewhere while its session ran
/// (here, another relay in front of the same sshd: another host:port). The
/// leg live then stays where it is; once it is cut, the redial goes where
/// the connection was saved to point — asking for that host's key, which
/// nothing trusts yet — not where the session started.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn ssh_a_redial_goes_where_the_connection_was_saved_to_point() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let started = Relay::start(sshd.addr()).await;
    let saved = Relay::start(sshd.addr()).await;
    let mut bench = Bench::with(
        std::net::SocketAddr::from(([127, 0, 0, 1], started.port)),
        quick(),
    );
    let first = spec(started.port);
    let id = bench.sessions.connect(first.clone());
    bench.connected(id).await;

    let mut moved = first;
    moved.target.port = saved.port;
    bench.sessions.respec(id, &moved);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(saved.connections(), 0, "the live leg moved");
    assert!(bench.sessions.view(id).unwrap().connected_at.is_some());

    started.cut();
    let (prompt, port) = bench
        .until(id, 15, |kind| match kind {
            EventKind::HostKeyPrompt { prompt, port, .. } => Some((*prompt, *port)),
            _ => None,
        })
        .await;
    assert_eq!(
        port, saved.port,
        "the redial went where the session started"
    );
    bench.sessions.answer_host_key(id, prompt, true);
    bench.connected(id).await;
    assert_eq!(
        started.connections(),
        1,
        "it dialled where it started again"
    );
    assert!(saved.connections() >= 1);
    let _ = bench.sessions.disconnect(id).await;
}

/// The TCP leg killed under a live shell: Reconnecting, a new connection,
/// the same pane channel (its scrollback stays on the view).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn ssh_a_cut_connection_is_redialled_onto_the_same_channel() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let mut bench = Bench::with(
        std::net::SocketAddr::from(([127, 0, 0, 1], relay.port)),
        quick(),
    );
    let id = bench.sessions.connect(spec(relay.port));
    bench.connected(id).await;
    let (_, panes) = bench.panes(id, 10).await;
    let stream = shell(&panes).stream;
    let mut term = Term::attach(stream);
    term.write(b"echo before-cut\r");
    assert!(term.wait_for("before-cut", 10).await);

    let live_since = bench.sessions.view(id).unwrap().connected_at;
    assert!(live_since.is_some(), "a live session says since when");
    let cut_at = Instant::now();
    relay.cut();
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    let lost = bench.sessions.view(id).unwrap().lost_at;
    assert!(
        lost.is_some_and(|at| at >= live_since.unwrap()),
        "the cut is when it was lost"
    );
    bench.connected(id).await;
    eprintln!("cut → connected again in {:?}", cut_at.elapsed());
    assert_eq!(relay.connections(), 2);
    // The same channel: no new panes for a plain shell.
    let view = bench.sessions.view(id).unwrap();
    assert_eq!(
        view.connected_at, live_since,
        "a reconnect keeps when it first went live"
    );
    assert_eq!(view.lost_at, None, "live again");
    assert_eq!(view.panes.len(), 1);
    assert_eq!(view.panes[0].stream, stream);
    let mark = term.screen.len();
    tokio::time::sleep(Duration::from_millis(300)).await;
    term.write(b"echo after-cut\r");
    assert!(
        term.wait_for_from(mark, "after-cut", 10).await.is_some(),
        "{}",
        term.text()
    );
    // The new shell's bytes came after a CAN, so a sequence the old one
    // left open can't swallow them.
    assert!(term.screen[..].contains(&0x18));
    assert!(term.text().contains("before-cut"), "scrollback kept");
    let _ = bench.sessions.disconnect(id).await;
}

/// A link that goes silent without a word (no RST, no FIN): the keepalive
/// probes it, the second silence is a drop, and once the link heals the
/// session is back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn ssh_a_silent_link_is_caught_by_the_keepalive() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let mut bench = Bench::with(
        std::net::SocketAddr::from(([127, 0, 0, 1], relay.port)),
        quick(),
    );
    let id = bench.sessions.connect(spec(relay.port));
    bench.connected(id).await;
    let stalled_at = Instant::now();
    relay.stall();
    bench
        .state(id, 20, |s| *s == SessionState::Reconnecting)
        .await;
    let noticed = stalled_at.elapsed();
    eprintln!("stall noticed after {noticed:?}");
    // Proof-of-life window + two probe timeouts at the quick timings.
    assert!(noticed >= Duration::from_secs(2), "{noticed:?}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    relay.heal();
    bench.connected(id).await;
    let _ = bench.sessions.disconnect(id).await;
}

/// The app in the background longer than the threshold: the return
/// reconnects without asking the socket; a short absence probes and keeps
/// a healthy connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn ssh_the_way_back_from_the_background() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    // The frozen dial below must not be let go by its own timeout: one far
    // longer than anything load can stretch the redial to tells "the
    // network change restarted it" from "it timed out" (T9).
    let frozen = Duration::from_secs(30);
    let mut bench = Bench::with(
        std::net::SocketAddr::from(([127, 0, 0, 1], relay.port)),
        Timing {
            handshake_timeout: frozen,
            channel_timeout: frozen,
            ..quick()
        },
    );
    let id = bench.sessions.connect(spec(relay.port));
    bench.connected(id).await;

    // Short: probed (1 s deadline), alive, kept.
    bench.sessions.set_foreground(false);
    tokio::time::sleep(Duration::from_millis(500)).await;
    bench.sessions.set_foreground(true);
    tokio::time::sleep(Duration::from_secs(2)).await;
    bench.drain();
    assert_eq!(relay.connections(), 1, "{:#?}", bench.tail());

    // Long (past the 2 s threshold): a fresh connection, no probe first.
    bench.sessions.set_foreground(false);
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    bench.sessions.set_foreground(true);
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    bench.connected(id).await;
    assert_eq!(relay.connections(), 2);

    // A dial into a black hole, then the network changes: redialled.
    relay.stall();
    relay.cut();
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    relay.heal();
    let changed = Instant::now();
    bench.sessions.network_changed();
    bench.connected(id).await;
    // Far before the frozen dial's handshake timeout: the network change
    // restarted it.
    assert!(
        changed.elapsed() < frozen / 3,
        "back after {:?}: the frozen dial waited out its {frozen:?}",
        changed.elapsed()
    );
    let _ = bench.sessions.disconnect(id).await;
}

/// B616: a size given while a redial is under way reaches the new shell.
/// The redial opened its terminal at the size of when it began, and no one
/// told the new shell of the size given since (the keyboard up, the phone
/// turned during the veil): vim, htop, an agent drew at the wrong size
/// until the next change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn ssh_a_size_given_during_the_redial_reaches_the_new_shell() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], relay.port));
    let udp = ProxiedUdp::new();
    let mut services = Bench::services(&udp, addr);
    let gate = Arc::new(tokio::sync::Notify::new());
    services.credentials = Arc::new(GatedKeys::new(1, gate.clone()));
    let mut bench = Bench::new(services, quick());
    bench.udp = udp;
    let id = bench.sessions.connect(spec(relay.port));
    bench.connected(id).await;
    let (_, panes) = bench.panes(id, 10).await;
    let mut term = Term::attach(shell(&panes).stream);
    relay.cut();
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    // The redial has begun and waits on its credential read.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(term.resize(90, 28));
    tokio::time::sleep(Duration::from_millis(200)).await;
    gate.notify_one();
    bench.state(id, 15, |s| *s == SessionState::Connected).await;
    assert!(term.write(b"stty size\r"));
    assert!(term.wait_for("28 90", 15).await, "{}", term.text());
    let _ = bench.sessions.disconnect(id).await;
}

/// Typed into a plain shell while the session reconnects: dropped and said
/// so (`Discarded::Reconnected`), never replayed into the new shell the
/// redial opens. The rule for typing while the session isn't live (the
/// session module's, B396, B476, B322):
///
/// - Under a reconnect, a plain shell's keys and pastes are not kept: the
///   redial is a new shell, where a command typed blind would run on its
///   own. Dropped at once, said as `Reconnected`.
/// - Under a reconnect, a multiplexer pane's keys and pastes wait for that
///   pane, which the redial brings back (tmux keeps it on the server), and
///   go to it in order once it is attached again. Dropped and said
///   `PaneGone` when the new attach doesn't have the pane, `SessionEnded`
///   when the session ends or fails first. Nothing else bounds the wait.
/// - A tap on the shortcut bar under the veil, and anything typed before
///   the first connect or once the session failed or ended: dropped, said
///   `NotLive`.
///
/// It said `NotLive` for the plain shell before B396 told the reconnect
/// apart (B851: the expectation had stayed behind; nothing was replayed).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn ssh_input_while_not_live_is_dropped_not_replayed() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let mut bench = Bench::with(
        std::net::SocketAddr::from(([127, 0, 0, 1], relay.port)),
        quick(),
    );
    let id = bench.sessions.connect(spec(relay.port));
    bench.connected(id).await;
    let (_, panes) = bench.panes(id, 10).await;
    let mut term = Term::attach(shell(&panes).stream);
    relay.stall();
    relay.cut();
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    term.write(b"echo typed-blind\r");
    let discarded = bench
        .until(id, 5, |kind| match kind {
            EventKind::InputDiscarded { why, bytes, .. } => Some((*why, *bytes)),
            _ => None,
        })
        .await;
    assert_eq!(discarded, (Discarded::Reconnected, 17));
    relay.heal();
    bench.sessions.network_changed();
    bench.connected(id).await;
    let mark = term.screen.len();
    term.write(b"echo typed-live\r");
    assert!(term.wait_for_from(mark, "typed-live", 10).await.is_some());
    tokio::time::sleep(Duration::from_millis(300)).await;
    term.read();
    assert!(!term.text().contains("typed-blind"), "{}", term.text());
    let _ = bench.sessions.disconnect(id).await;
}

// MARK: tmux

fn tmux_spec(port: u16, tmux: &Tmux) -> moshpit_session::ConnectionSpec {
    let mut spec = spec(port);
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    spec
}

/// tmux over SSH: the controller attaches the existing session, its panes
/// get byte channels, a keystroke reaches the pane and its output comes
/// back on the pane's channel; tmux's tree reaches the UI.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn tmux_attaches_and_a_pane_echoes() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let tmux = Tmux::new();
    tmux.new_session("w10");
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(tmux_spec(sshd.port, &tmux));
    // The terminal is on screen: its windows are ours to size.
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let (attach, panes) = bench.panes(id, 20).await;
    bench
        .until(id, 20, |kind| match kind {
            EventKind::FirstFrame { attach: a } if *a == attach => Some(()),
            _ => None,
        })
        .await;
    let active = panes.iter().find(|p| p.active).unwrap_or(&panes[0]).clone();
    assert!(active.pane.starts_with('%'), "{active:?}");
    let snapshot = bench.sessions.tmux_snapshot(id).expect("a tree");
    assert!(snapshot.is_attached);
    assert!(snapshot.sessions.values().any(|s| s.name == "w10"));

    let mut term = Term::attach(active.stream);
    term.resize(90, 28);
    term.write(b"echo tmux-$((6*7))\r");
    assert!(term.wait_for("tmux-42", 15).await, "{}", term.text());
    let captured = tmux.run(&["capture-pane", "-p", "-t", &active.pane]);
    assert!(captured.contains("tmux-42"), "{captured}");
    assert_eq!(theirs(tmux.sessions()), vec!["w10".to_owned()]);
    let _ = bench.sessions.disconnect(id).await;
    // The disconnect handed the window back: no size override left.
    let size = tmux.run(&["show-options", "-w", "-t", "w10", "-v", "window-size"]);
    assert!(size.is_empty() || size == "latest", "{size:?}");
}

/// A server without sessions: the session stays up on the empty state
/// (B228: no stalled attach, the person's first session to make) — and no
/// session is created.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn tmux_never_creates_a_session() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let tmux = Tmux::new();
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(tmux_spec(sshd.port, &tmux));
    // The terminal is on screen: its windows are ours to size.
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    while !bench
        .sessions
        .tmux_view(id)
        .is_some_and(|view| view.no_session)
    {
        let stalled = bench
            .until(id, 15, |kind| match kind {
                EventKind::AttachStalled => Some(true),
                EventKind::TreeChanged => Some(false),
                _ => None,
            })
            .await;
        assert!(
            !stalled,
            "a server with no session read as a stalled attach"
        );
    }
    assert!(!bench.sessions.view(id).unwrap().attach_stalled);
    assert!(tmux.sessions().is_empty(), "{:?}", tmux.sessions());
    assert_eq!(
        bench.sessions.view(id).unwrap().state,
        SessionState::Connected
    );
    let _ = bench.sessions.disconnect(id).await;
}

/// The TCP leg under tmux killed: a new attach with fresh channels for the
/// same panes, the old ones retired on its first frame, and what was typed
/// into the last frame after the session was live again but before the
/// pane was back waits for the pane (B5) instead of going into the shell.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn tmux_a_drop_reattaches_and_held_input_reaches_its_pane() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::new();
    tmux.new_session("w10");
    let mut bench = Bench::with(
        std::net::SocketAddr::from(([127, 0, 0, 1], relay.port)),
        quick(),
    );
    let mut spec = tmux_spec(relay.port, &tmux);
    // The shell takes a second before it reads the boot line: every attach
    // lands a second after the session is live, so what the ghost takes in
    // that second has no pane to go to yet.
    spec.pty_command = Some("/bin/sh -c 'sleep 1; exec /bin/sh'".into());
    let id = bench.sessions.connect(spec);
    // The terminal is on screen: its windows are ours to size.
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let (first, panes) = bench.panes(id, 20).await;
    bench
        .until(id, 20, |kind| match kind {
            EventKind::FirstFrame { attach } if *attach == first => Some(()),
            _ => None,
        })
        .await;
    let old = panes.iter().find(|p| p.active).unwrap_or(&panes[0]).clone();
    let ghost = Term::attach(old.stream);

    relay.cut();
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    bench.connected(id).await;
    // Live again, the new attach still on its way: typed into the ghost.
    assert!(
        ghost.write(b"echo held-$((40+2))\r"),
        "the ghost takes input"
    );
    assert!(
        bench.sessions.view(id).unwrap().attach == first,
        "typed before the new attach listed its panes"
    );
    let (second, panes) = bench.panes_after(id, first, 20).await;
    assert!(second > first);
    let new = panes
        .iter()
        .find(|p| p.pane == old.pane)
        .expect("the same pane, re-attached")
        .clone();
    assert_ne!(new.stream, old.stream, "a fresh channel per attach");
    bench
        .until(id, 20, |kind| match kind {
            EventKind::FirstFrame { attach } if *attach == second => Some(()),
            _ => None,
        })
        .await;
    let mut term = Term::attach(new.stream);
    assert!(term.wait_for("held-42", 15).await, "{}", term.text());
    let captured = tmux.run(&["capture-pane", "-p", "-t", &new.pane]);
    assert!(captured.contains("held-42"), "{captured}");
    // The ghost's channel is retired once the new attach painted.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!ghost.write(b"x"), "the old channel refuses input now");
    let _ = bench.sessions.disconnect(id).await;
}

/// A tmux session pins the window to the phone's grid; when the
/// connection drops and the redial never lands again before the person
/// closes the session, the pin is still handed back (over a connection of
/// its own) — the desktop gets its window size back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn tmux_pins_are_handed_back_even_when_the_redial_never_landed() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::new();
    tmux.new_session("w10");
    let mut bench = Bench::with(
        std::net::SocketAddr::from(([127, 0, 0, 1], relay.port)),
        quick(),
    );
    let id = bench.sessions.connect(tmux_spec(relay.port, &tmux));
    // The terminal is on screen: its windows are ours to size.
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let (attach, _) = bench.panes(id, 20).await;
    bench
        .until(id, 20, |kind| match kind {
            EventKind::FirstFrame { attach: a } if *a == attach => Some(()),
            _ => None,
        })
        .await;
    let mut pinned = String::new();
    for _ in 0..50 {
        pinned = tmux.run(&["show-options", "-w", "-t", "w10", "-v", "window-size"]);
        if pinned == "manual" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(pinned, "manual", "the attach pinned the window");

    // The link dies; the redial goes into the same black hole.
    relay.stall();
    relay.cut();
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    relay.heal();
    let _ = bench.sessions.disconnect(id).await;
    let mut size = String::from("manual");
    for _ in 0..100 {
        size = tmux.run(&["show-options", "-w", "-t", "w10", "-v", "window-size"]);
        if size != "manual" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(size.is_empty() || size == "latest", "{size:?}");
}

// MARK: mosh

fn mosh_spec(port: u16) -> moshpit_session::ConnectionSpec {
    let mut spec = spec(port);
    spec.protocol = Protocol::Mosh;
    spec
}

async fn mosh_server_pid(bench: &mut Bench, id: u64) -> Reaper {
    bench.connected(id).await;
    let pid = bench.sessions.view(id).unwrap().mosh_server_pid;
    assert!(pid.is_some(), "mosh-server's pid was captured");
    Reaper(pid)
}

/// mosh: bootstrap over SSH, UDP from there, keys and echoes through the
/// transport; the remote shell's `exit` ENDS the session (mosh-server's
/// goodbye, acked) and the server is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and mosh-server; see the module docs"]
async fn mosh_runs_and_exit_ends_the_session() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(mosh_spec(sshd.port));
    let reaper = mosh_server_pid(&mut bench, id).await;
    let pid = reaper.0.unwrap();
    assert_eq!(
        bench.sessions.view(id).unwrap().transport,
        Some(Transport::Mosh)
    );
    let (_, panes) = bench.panes(id, 10).await;
    let mut term = Term::attach(shell(&panes).stream);
    term.write(b"echo mosh-$((6*7))\r");
    assert!(term.wait_for("mosh-42", 15).await, "{}", term.text());
    term.write(b"exit\r");
    let state = bench
        .state(id, 15, |s| matches!(s, SessionState::Ended(_)))
        .await;
    assert_eq!(state, SessionState::Ended(EndReason::MoshServerEnded));
    for _ in 0..50 {
        if !alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!alive(pid), "mosh-server quit on its acked goodbye");
    let _ = bench.sessions.disconnect(id).await;
}

/// No mosh-server on the host (as the cached capabilities say): SSH on the
/// same connection, with the notice.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn mosh_falls_back_to_ssh_without_a_server() {
    need_sshd!();
    struct NoMoshServer;
    impl SessionMemory for NoMoshServer {
        fn capabilities(&self, _: &str) -> Option<HostCapabilities> {
            Some(HostCapabilities {
                has_mosh_server: false,
                ..HostCapabilities::unknown()
            })
        }
    }
    let sshd = Sshd::start().await;
    let udp = ProxiedUdp::new();
    let mut services = Bench::services(&udp, sshd.addr());
    services.memory = Arc::new(NoMoshServer);
    let mut bench = Bench::new(services, quick());
    let id = bench.sessions.connect(mosh_spec(sshd.port));
    let degrade = bench
        .until(id, 15, |kind| match kind {
            EventKind::Degraded(d) => Some(*d),
            _ => None,
        })
        .await;
    assert_eq!(degrade, Degrade::MoshServerMissing);
    bench.connected(id).await;
    let view = bench.sessions.view(id).unwrap();
    assert_eq!(view.transport, Some(Transport::Ssh));
    assert_eq!(view.mosh_server_pid, None);
    let (_, panes) = bench.panes(id, 10).await;
    let mut term = Term::attach(shell(&panes).stream);
    term.write(b"echo fell-back\r");
    assert!(term.wait_for("fell-back", 10).await);
    assert_eq!(sshd.accepted(), 1, "the bootstrap's own connection");
    let _ = bench.sessions.disconnect(id).await;
}

/// The return path dead from the start (a VPN that passes UDP out and
/// drops it in): the watchdog offers SSH, the server that never heard from
/// us is reaped, and the switch to SSH works.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and mosh-server; see the module docs"]
async fn mosh_dead_return_path_offers_ssh_and_reaps_the_unreached_server() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let mut timing = quick();
    timing.mosh.return_path_deadline_ms = 3_000;
    let mut bench = Bench::with(sshd.addr(), timing);
    bench
        .udp
        .faults
        .drop_down
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let id = bench.sessions.connect(mosh_spec(sshd.port));
    let reaper = mosh_server_pid(&mut bench, id).await;
    let pid = reaper.0.unwrap();
    let started = Instant::now();
    bench
        .until(id, 15, |kind| {
            (*kind == EventKind::ReturnPathDead).then_some(())
        })
        .await;
    eprintln!("return path reported dead after {:?}", started.elapsed());
    assert!(bench.sessions.view(id).unwrap().return_path_dead);
    for _ in 0..80 {
        if !alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!alive(pid), "the unreached mosh-server was reaped");

    bench.sessions.switch_to_ssh(id);
    bench
        .state(id, 10, |s| *s == SessionState::Connecting)
        .await;
    bench.connected(id).await;
    assert_eq!(
        bench.sessions.view(id).unwrap().transport,
        Some(Transport::Ssh)
    );
    let _ = bench.sessions.disconnect(id).await;
}

/// The UDP path blackholed mid-session and back: flows rebuilt, the
/// return path reported, the session never leaves Connected and the server
/// it reached is never killed (a silence after contact is not the
/// unreached-server watchdog). Keys typed into the blackhole are kept until
/// mosh-server acks them, however long the silence, as mosh-client keeps
/// them (B618: a live session's keystrokes never expire,
/// `Timing::mosh`): they arrive once it heals, each once and in the order
/// typed — the oldest first.
///
/// It expected keys older than 10 s dropped, the transport's own default
/// before B618 kept them (B852: the expectation had stayed behind).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and mosh-server; see the module docs"]
async fn mosh_survives_a_blackhole_and_never_kills_a_server_it_reached() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(mosh_spec(sshd.port));
    let reaper = mosh_server_pid(&mut bench, id).await;
    let pid = reaper.0.unwrap();
    let (_, panes) = bench.panes(id, 10).await;
    let mut term = Term::attach(shell(&panes).stream);
    term.write(b"echo up-$((1+1))\r");
    assert!(term.wait_for("up-2", 15).await, "{}", term.text());

    bench.udp.faults.blackhole(true);
    tokio::time::sleep(Duration::from_secs(1)).await;
    term.write(b"echo stale-$((2+2))\r");
    // The liveness check's three rebuilds and its report (9 s of silence,
    // then every 3 s).
    bench
        .until(id, 30, |kind| {
            (*kind == EventKind::ReturnPathDead).then_some(())
        })
        .await;
    assert!(
        alive(pid),
        "a server we reached is never killed for silence"
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    term.write(b"echo fresh-$((3+3))\r");
    tokio::time::sleep(Duration::from_secs(2)).await;
    bench.udp.faults.blackhole(false);
    assert!(term.wait_for("fresh-6", 20).await, "{}", term.text());
    // Both ran, once each, the one typed first first.
    let text = term.text();
    assert_eq!(text.matches("\nstale-4").count(), 1, "{text}");
    assert_eq!(text.matches("\nfresh-6").count(), 1, "{text}");
    assert!(text.find("\nstale-4") < text.find("\nfresh-6"), "{text}");
    bench.drain();
    assert_eq!(
        bench.states(id),
        vec![SessionState::Connecting, SessionState::Connected],
        "{:#?}",
        bench.tail()
    );
    assert!(alive(pid));
    let _ = bench.sessions.disconnect(id).await;
}

/// mosh with tmux: tmux's own client is the mosh session's program, on the
/// existing session; keys reach its pane. With no session on the server it
/// waits for one — and none is created until the person asks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn mosh_with_tmux_renders_the_existing_session_and_never_creates_one() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let tmux = Tmux::new();
    tmux.new_session("w10");
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = mosh_spec(sshd.port);
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(spec.clone());
    let _reaper = mosh_server_pid(&mut bench, id).await;
    let (_, panes) = bench.panes(id, 10).await;
    let mut term = Term::attach(shell(&panes).stream);
    // tmux's status line names the session: `[w10]`, or from tmux 3.8 the
    // phone's own group of it, `[w10~mp-…` (B773, B812).
    assert!(term.wait_for("[w10", 15).await, "{}", term.text());
    term.write(b"echo mt-$((6*7))\r");
    assert!(term.wait_for("mt-42", 15).await, "{}", term.text());
    let pane = tmux.run(&["display-message", "-p", "-t", "w10", "#{pane_id}"]);
    let captured = tmux.run(&["capture-pane", "-p", "-t", &pane]);
    assert!(captured.contains("mt-42"), "{captured}");
    let _ = bench.sessions.disconnect(id).await;

    let empty = Tmux::new();
    spec.tmux_path = Some(empty.prefix());
    let id = bench.sessions.connect(spec);
    // The terminal is on screen: its windows are ours to size.
    bench.sessions.set_visible(Some(id));
    let _reaper = mosh_server_pid(&mut bench, id).await;
    let (_, panes) = bench.panes(id, 10).await;
    let _term = Term::attach(shell(&panes).stream);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(empty.sessions().is_empty(), "{:?}", empty.sessions());
    let _ = bench.sessions.disconnect(id).await;
}

/// mosh with tmux, no session on the server: the person's "new session"
/// from the empty state makes one, and the renderer, waiting, attaches it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn mosh_with_tmux_makes_the_first_session_when_asked() {
    // B788 over mosh: no session is made yet (public/main 0ab623bf, tmux
    // 3.6a and 3.8). `MOSHPIT_LIVE_B788=1` runs it; its fix un-skips it.
    if std::env::var_os("MOSHPIT_LIVE_B788").is_none() {
        eprintln!("skipped: B788 over mosh, until its fix");
        return;
    }
    need_sshd!();
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = mosh_spec(sshd.port);
    spec.multiplexer = Multiplexer::Tmux;
    let empty = Tmux::new();
    spec.tmux_path = Some(empty.prefix());
    let id = bench.sessions.connect(spec);
    // The terminal is on screen: its windows are ours to size.
    bench.sessions.set_visible(Some(id));
    let _reaper = mosh_server_pid(&mut bench, id).await;
    let (_, panes) = bench.panes(id, 10).await;
    let mut term = Term::attach(shell(&panes).stream);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(empty.sessions().is_empty(), "{:?}", empty.sessions());
    bench
        .sessions
        .tmux(id, moshpit_session::TmuxAction::NewSession { name: None })
        .unwrap();
    // Made on the host, the person's one session (the phone's own group of
    // it aside, `~mp-…` from tmux 3.8); then the renderer shows it: `[0]`,
    // or `[0~mp-…` (B773, B812, B853).
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while theirs(empty.sessions()).is_empty() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(theirs(empty.sessions()), ["0"], "{:?}", empty.sessions());
    assert!(term.wait_for("[0", 15).await, "{}", term.text());
    let _ = bench.sessions.disconnect(id).await;
}

/// The person disconnects a mosh session: its server is stopped (nothing
/// could ever reattach to it).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and mosh-server; see the module docs"]
async fn mosh_disconnect_stops_the_server() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(mosh_spec(sshd.port));
    let reaper = mosh_server_pid(&mut bench, id).await;
    let pid = reaper.0.unwrap();
    let (_, panes) = bench.panes(id, 10).await;
    let mut term = Term::attach(shell(&panes).stream);
    term.write(b"echo hi\r");
    assert!(term.wait_for("hi", 15).await);
    let _ = bench.sessions.disconnect(id).await;
    for _ in 0..100 {
        if !alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!alive(pid), "mosh-server stopped on disconnect");
}

/// The app in the background past the threshold: mosh replaces its flow and
/// repaints instead of reconnecting; the session stays the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and mosh-server; see the module docs"]
async fn mosh_the_way_back_replaces_the_flow() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(mosh_spec(sshd.port));
    let _reaper = mosh_server_pid(&mut bench, id).await;
    let (_, panes) = bench.panes(id, 10).await;
    let mut term = Term::attach(shell(&panes).stream);
    term.write(b"echo one\r");
    assert!(term.wait_for("one", 15).await);
    bench.sessions.set_foreground(false);
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    bench.sessions.set_foreground(true);
    let mark = term.screen.len();
    term.write(b"echo two\r");
    assert!(term.wait_for_from(mark, "two", 15).await.is_some());
    bench.drain();
    assert_eq!(
        bench.states(id),
        vec![SessionState::Connecting, SessionState::Connected]
    );
    let _ = bench.sessions.disconnect(id).await;
}

// MARK: herdr's seam

// MARK: Echo

/// Keystroke → host → bytes in the pane's channel, on the loopback, for
/// SSH and mosh: one printable byte at a time into `cat` (raw tty, so the
/// echo is the pty's), each timed from the view's write to the view reading
/// it back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and mosh-server; see the module docs"]
async fn echo_latency_ssh_and_mosh() {
    need_sshd!();
    const KEYS: usize = 300;
    let sshd = Sshd::start().await;
    for protocol in [Protocol::Ssh, Protocol::Mosh] {
        let mut bench = Bench::with(sshd.addr(), quick());
        let mut spec = spec(sshd.port);
        spec.protocol = protocol;
        spec.pty_command = Some("/bin/sh -c 'stty raw -echo; echo READY; exec cat'".into());
        let id = bench.sessions.connect(spec);
        bench.connected(id).await;
        let _reaper = Reaper(bench.sessions.view(id).unwrap().mosh_server_pid);
        let (_, panes) = bench.panes(id, 10).await;
        let mut term = Term::attach(shell(&panes).stream);
        assert!(term.wait_for("READY", 15).await, "{}", term.text());
        tokio::time::sleep(Duration::from_millis(500)).await;
        term.read();
        let mut samples = Vec::with_capacity(KEYS);
        for i in 0..KEYS {
            let key = b'a' + (i % 26) as u8;
            let mark = term.screen.len();
            let sent = Instant::now();
            assert!(term.write(&[key]));
            let found = term
                .wait_for_from(mark, std::str::from_utf8(&[key]).unwrap(), 5)
                .await;
            assert!(found.is_some(), "{protocol:?} key {i} never echoed");
            samples.push(sent.elapsed().as_micros());
            // mosh paces new input at its frame interval; space keys like a
            // person typing fast.
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        let slowest = samples
            .iter()
            .enumerate()
            .max_by_key(|(_, us)| **us)
            .map(|(i, _)| i)
            .unwrap();
        eprintln!(
            "{protocol:?}: the first keys took {:?} µs; the slowest was key {slowest}",
            &samples[..5]
        );
        let (p50, p95, p99, max) = percentiles(&mut samples);
        eprintln!(
            "echo {protocol:?} over the loopback, {KEYS} keys: p50 {:.2} ms, p95 {:.2} ms, p99 {:.2} ms, max {:.2} ms",
            p50 as f64 / 1000.0,
            p95 as f64 / 1000.0,
            p99 as f64 / 1000.0,
            max as f64 / 1000.0
        );
        assert!(p50 < 100_000, "{protocol:?} p50 {p50} µs");
        let _ = bench.sessions.disconnect(id).await;
    }
}

// MARK: What the app's wiring needs (W14a)

/// What a marked reader of a pane's channel saw: bytes, and marks where
/// they fell among them.
#[derive(Debug, Default)]
struct Marked {
    bytes: Vec<u8>,
    /// Each mark, with how many bytes had been read before it.
    marks: Vec<(usize, moshpit_session::Mark)>,
}

impl Marked {
    fn read(&mut self, stream: u64) {
        let mut buf = [0u8; 8 * 1024];
        loop {
            let (n, mark) = moshpit_stream::view::read_marked(stream, &mut buf);
            self.bytes.extend_from_slice(&buf[..n]);
            match mark {
                Some(mark) => self.marks.push((self.bytes.len(), mark)),
                None if n < buf.len() => return,
                None => {}
            }
        }
    }

    fn codes(&self) -> Vec<u32> {
        self.marks.iter().map(|(_, mark)| mark.code).collect()
    }
}

/// tmux over SSH as the app's terminal chrome reads it: the tree with what
/// it draws around it (the agents' stamps once polled, the active pane's
/// flags), the views' covers and reveals as marks in each pane's channel
/// after the repaint they belong to, leaving the terminal (back to Home)
/// handing the window's pin back and returning taking it again, and a
/// flush that answers once the release went out.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn tmux_the_chrome_reads_a_view_marks_and_pins_follow_the_screen() {
    use moshpit_session::{TmuxAction, view_marks};
    need_sshd!();
    let sshd = Sshd::start().await;
    let tmux = Tmux::new();
    tmux.new_session("w14");
    tmux.run(&["send-keys", "-t", "w14", "echo before-$((6*7))", "Enter"]);
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(tmux_spec(sshd.port, &tmux));
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let (attach, panes) = bench.panes(id, 20).await;
    let active = panes.iter().find(|p| p.active).unwrap_or(&panes[0]).clone();
    // Attached before anything reads the channel: everything the
    // controller put in it is still queued, marks and all.
    let rung = Arc::new(tokio::sync::Notify::new());
    let bell = rung.clone();
    assert!(moshpit_stream::view::attach(active.stream, move || bell.notify_one()));
    let mut seen = Marked::default();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !seen.codes().contains(&view_marks::REVEAL) && Instant::now() < deadline {
        seen.read(active.stream);
        let _ = tokio::time::timeout(Duration::from_millis(500), rung.notified()).await;
    }
    let reveal = seen
        .marks
        .iter()
        .find(|(_, mark)| mark.code == view_marks::REVEAL)
        .map(|(at, _)| *at)
        .unwrap_or_else(|| panic!("a reveal mark; saw {:?}", seen.codes()));
    let text = String::from_utf8_lossy(&seen.bytes[..reveal]).into_owned();
    assert!(
        text.contains("before-42"),
        "the reveal comes after the repaint it uncovers: {text:?}"
    );
    // The view's reveal is what makes the first frame.
    bench
        .sessions
        .tmux(
            id,
            TmuxAction::ViewRevealed {
                pane: active.pane.clone(),
            },
        )
        .unwrap();
    bench
        .until(id, 20, |kind| match kind {
            EventKind::FirstFrame { attach: a } if *a == attach => Some(()),
            _ => None,
        })
        .await;

    let view = bench.sessions.tmux_view(id).expect("tmux's view");
    assert!(view.snapshot.is_attached);
    assert!(view.snapshot.sessions.values().any(|s| s.name == "w14"));
    bench.sessions.tmux(id, TmuxAction::PollAgentHooks).unwrap();
    bench
        .until(id, 10, |kind| {
            (*kind == EventKind::AgentHooksUpdated).then_some(())
        })
        .await;
    assert!(bench.sessions.tmux_view(id).unwrap().agent_hooks_loaded);

    let size = |tmux: &Tmux| tmux.run(&["show-options", "-w", "-t", "w14", "-v", "window-size"]);
    let settle = |tmux: &Tmux, want: bool| {
        let tmux_size = size(tmux);
        (tmux_size == "manual") == want
    };
    for _ in 0..50 {
        if settle(&tmux, true) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(size(&tmux), "manual", "the attach pinned the window");
    // Back to Home: the pin goes back, and the flush answers once it went.
    bench.sessions.set_visible(None);
    let flushed = bench.sessions.flushed();
    tokio::time::timeout(Duration::from_secs(5), flushed)
        .await
        .expect("the flush answers")
        .expect("and says so");
    for _ in 0..50 {
        if settle(&tmux, false) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_ne!(
        size(&tmux),
        "manual",
        "leaving the terminal hands the pin back"
    );
    // On screen again: taken again.
    bench.sessions.set_visible(Some(id));
    for _ in 0..50 {
        if settle(&tmux, true) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        size(&tmux),
        "manual",
        "the terminal on screen pins it again"
    );
    moshpit_stream::view::detach(active.stream);
    let _ = bench.sessions.disconnect(id).await;
}

/// The debug fault hooks, on a session's own connection: a cut is a drop
/// the session redials onto the same channel; a stall is silence the
/// keepalive catches. Neither ends the session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn ssh_injected_faults_are_redialled() {
    use moshpit_session::LinkFault;
    need_sshd!();
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(spec(sshd.port));
    bench.connected(id).await;
    let (_, panes) = bench.panes(id, 10).await;
    let stream = shell(&panes).stream;
    let mut term = Term::attach(stream);

    let cut_at = Instant::now();
    bench.sessions.inject_fault(id, LinkFault::Cut);
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    bench.connected(id).await;
    eprintln!("injected cut → connected again in {:?}", cut_at.elapsed());
    let mark = term.screen.len();
    tokio::time::sleep(Duration::from_millis(300)).await;
    term.write(b"echo after-cut-$((6*7))\r");
    assert!(
        term.wait_for_from(mark, "after-cut-42", 10).await.is_some(),
        "{}",
        term.text()
    );

    let stalled_at = Instant::now();
    bench.sessions.inject_fault(id, LinkFault::Stall);
    bench
        .state(id, 20, |s| *s == SessionState::Reconnecting)
        .await;
    bench.connected(id).await;
    eprintln!(
        "injected stall → connected again in {:?}",
        stalled_at.elapsed()
    );
    let mark = term.screen.len();
    tokio::time::sleep(Duration::from_millis(300)).await;
    term.write(b"echo after-stall-$((6*7))\r");
    assert!(
        term.wait_for_from(mark, "after-stall-42", 10)
            .await
            .is_some(),
        "{}",
        term.text()
    );
    assert!(
        !bench
            .states(id)
            .iter()
            .any(|s| matches!(s, SessionState::Ended(_) | SessionState::Failed(_))),
        "{:?}",
        bench.states(id)
    );
    let _ = bench.sessions.disconnect(id).await;
}

/// Borrowing the session's SSH: an SSH session lends its own connection; a
/// mosh session, which has none once bootstrapped, dials one side
/// connection and lends that, the same one each time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and mosh-server; see the module docs"]
async fn borrowing_the_sessions_ssh() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(spec(sshd.port));
    bench.connected(id).await;
    let ssh = bench
        .sessions
        .borrow_ssh(id)
        .await
        .unwrap()
        .expect("its connection");
    let out = ssh.run("echo borrowed-$((6*7))").await.unwrap();
    assert!(String::from_utf8_lossy(&out.merged()).contains("borrowed-42"));
    assert_eq!(
        sshd.accepted(),
        1,
        "no second connection for an SSH session"
    );
    let _ = bench.sessions.disconnect(id).await;
    assert!(
        bench.sessions.borrow_ssh(id).await.unwrap().is_err(),
        "a closed session lends nothing"
    );

    let id = bench.sessions.connect(mosh_spec(sshd.port));
    let _reaper = mosh_server_pid(&mut bench, id).await;
    let before = sshd.accepted();
    let first = bench
        .sessions
        .borrow_ssh(id)
        .await
        .unwrap()
        .expect("a side connection");
    let second = bench
        .sessions
        .borrow_ssh(id)
        .await
        .unwrap()
        .expect("the same one");
    let out = second.run("echo side-$((6*7))").await.unwrap();
    assert!(String::from_utf8_lossy(&out.merged()).contains("side-42"));
    assert_eq!(
        sshd.accepted(),
        before + 1,
        "one side connection, lent twice"
    );
    drop(first);
    let _ = bench.sessions.disconnect(id).await;
}

/// B870: dropping the private test `Sshd` with an active PTY session must
/// reap detached `setsid` session children still holding `sshd.log` open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd; see the module docs"]
async fn sshd_drop_reaps_detached_session_children() {
    need_sshd!();
    let sshd = Sshd::start().await;
    let listener = sshd.pid();
    let log_path = sshd.log_path();
    let mut bench = Bench::with(sshd.addr(), quick());
    let id = bench.sessions.connect(spec(sshd.port));
    bench.connected(id).await;
    let (_, panes) = bench.panes(id, 10).await;
    let _term = Term::attach(shell(&panes).stream);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let workers: Vec<u32> = pids_holding_file(&log_path)
        .into_iter()
        .filter(|&pid| pid != listener)
        .collect();
    assert!(
        !workers.is_empty(),
        "expected detached session worker holding {}",
        log_path.display()
    );
    drop(sshd);
    let survivors: Vec<u32> = workers
        .into_iter()
        .filter(|&pid| alive(i64::from(pid)))
        .collect();
    if !survivors.is_empty() {
        for &pid in &survivors {
            let _ = std::process::Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .output();
        }
        panic!("B870: Sshd workers survived Drop: {survivors:?}");
    }
}
