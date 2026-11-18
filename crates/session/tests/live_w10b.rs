//! W10b against real programs on this machine's loopback, private every
//! one: an sshd of our own (temp keys and config), mosh-server started
//! through it, a tmux server on a private socket (`-L moshpit-w10-…`), a
//! herdr server on a private session under a temp HOME (`--session
//! mw10b-…`; every herdr command carries it). Everything started
//! here is stopped when the test ends; `~/.ssh`, the machine's sshd and the
//! user's own tmux and herdr are never addressed.
//!
//! - herdr on the Rust session: the tree, the frame channel's first frame,
//!   the input rule (B185), a pick, a delivery, the push channel.
//! - mosh + tmux's `-CC` sidecar: the renderer found by its pid, the window
//!   pinned to the mosh grid, session follow both ways, the status line of
//!   a session Moshpit made, the control plane rebuilt while mosh carries
//!   on, everything handed back on the disconnect.
//! - mosh-servers reaped: a start cut short (B194), a reconnect's old
//!   server (B195).
//! - tmux's pins: paid before the disconnect says done (B203); an attach
//!   nobody watches doesn't pin until shown.
//! - mosh's echo acknowledgement in the channel, after the echo.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_w10b -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::time::{Duration, Instant};

use moshpit_session::{
    Discarded, EventKind, HerdrAction, HerdrAnswer, LinkFault, Multiplexer, PaneInfo, Protocol,
    SHELL_PANE, SessionState, TmuxAction, view_marks,
};
use support::*;

macro_rules! need {
    ($($what:expr),*) => {
        if !Sshd::available() {
            eprintln!("skipped: no {SSHD}");
            return;
        }
        $(if !$what {
            eprintln!("skipped: {}", stringify!($what));
            return;
        })*
    };
}

fn mosh_tmux_spec(port: u16, tmux: &Tmux) -> moshpit_session::ConnectionSpec {
    let mut spec = spec(port);
    spec.protocol = Protocol::Mosh;
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    spec
}

/// Polls `check` every 100 ms for `seconds`.
async fn eventually(seconds: u64, mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    check()
}

async fn mosh_server(bench: &mut Bench, id: u64) -> i64 {
    bench.connected(id).await;
    bench
        .sessions
        .view(id)
        .and_then(|view| view.mosh_server_pid)
        .expect("mosh-server's pid")
}

// MARK: herdr

/// herdr on the Rust session, over a private sshd onto a private herdr
/// server: the tree and the first frame; a keystroke into the pane once it
/// painted; typing at a pane that isn't reading dropped and said (B185); a
/// pick whose early typing waits for the pane's first frame; a delivery to
/// a pane the frame channel isn't on, switched to and held, then answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and herdr; see the module docs"]
async fn herdr_runs_on_the_rust_session() {
    need!(PrivateHerdr::available());
    let herdr = PrivateHerdr::start();
    let home = herdr.home.display().to_string();
    let sshd = Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.multiplexer = Multiplexer::Herdr;
    spec.herdr_path = Some(herdr.custom_path());
    let started = Instant::now();
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;

    // The tree, read over exec channels.
    assert!(
        eventually(15, || bench.sessions.herdr_view(id).is_some_and(|view| {
            view.snapshot.sessions.values().any(|s| s.name == "live")
        }))
        .await,
        "{:?}",
        bench.sessions.herdr_view(id)
    );
    // The focused pane's frames, into its channel: its panes are listed,
    // then its first frame.
    let (attach, panes) = bench.panes(id, 20).await;
    let painted = |bench: &Bench| {
        bench
            .seen
            .iter()
            .any(|e| e.session == id && e.kind == EventKind::FirstFrame { attach })
    };
    if !painted(&bench) {
        bench
            .until(id, 20, |kind| match kind {
                EventKind::FirstFrame { attach: a } if *a == attach => Some(()),
                _ => None,
            })
            .await;
    }
    let first_frame_ms = started.elapsed().as_millis();
    let order: Vec<&EventKind> = bench
        .seen
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                EventKind::Panes { .. } | EventKind::FirstFrame { .. }
            )
        })
        .map(|e| &e.kind)
        .collect();
    assert!(
        matches!(order.first(), Some(EventKind::Panes { .. })),
        "the panes before the frame they show: {order:?}"
    );
    let focused = panes.iter().find(|p| p.active).unwrap_or(&panes[0]).clone();
    let mut term = Term::attach(focused.stream);
    assert!(term.wait_for("%", 10).await, "the prompt: {}", term.text());
    let typed = Instant::now();
    term.write(b"echo hd-$((6*7))\r");
    assert!(term.wait_for("hd-42", 10).await, "{}", term.text());
    let echo_ms = typed.elapsed().as_millis();
    assert!(herdr.read(&focused.pane).contains("hd-42"));

    // A second pane, made through the client's command lane.
    let before: Vec<String> = bench
        .sessions
        .herdr_view(id)
        .unwrap()
        .snapshot
        .panes
        .keys()
        .cloned()
        .collect();
    let token = bench.sessions.herdr(id, HerdrAction::NewPane);
    let answer = bench
        .until(id, 15, |kind| match kind {
            EventKind::HerdrAnswer { token: t, answer } if *t == token => Some(answer.clone()),
            _ => None,
        })
        .await;
    assert_eq!(answer, HerdrAnswer::Done);
    let mut second = None;
    assert!(
        eventually(15, || {
            second = bench
                .sessions
                .herdr_view(id)
                .unwrap()
                .snapshot
                .panes
                .keys()
                .find(|pane| !before.contains(pane))
                .cloned();
            second.is_some()
        })
        .await
    );
    let second = second.unwrap();

    // Typing at a pane whose attach isn't reading, unpicked: dropped and
    // said (herdr 0.9.5 would drop it unseen).
    let ids_before = bench.sessions.view(id).unwrap().panes;
    if let Some(other) = ids_before.iter().find(|p| p.pane == second && !p.active) {
        let ghost = Term::attach(other.stream);
        ghost.write(b"never-typed");
        bench
            .until(id, 5, |kind| match kind {
                EventKind::InputDiscarded {
                    why: Discarded::NotAttached,
                    ..
                } => Some(()),
                _ => None,
            })
            .await;
    }

    // The person picks the new pane and types at once: held for its first
    // frame, then sent there — not to the pane being left.
    bench
        .sessions
        .herdr(id, HerdrAction::SelectPane(second.clone()));
    let mut picked: Option<PaneInfo> = None;
    assert!(
        eventually(10, || {
            picked = bench
                .sessions
                .view(id)
                .unwrap()
                .panes
                .into_iter()
                .find(|p| p.pane == second);
            picked.is_some()
        })
        .await
    );
    let picked = picked.unwrap();
    let mut pick = Term::attach(picked.stream);
    pick.write(b"echo picked-$((6*7))\r");
    assert!(pick.wait_for("picked-42", 15).await, "{}", pick.text());
    assert!(herdr.read(&second).contains("picked-42"));
    assert!(!herdr.read(&focused.pane).contains("picked-42"));

    // A notification's reply to the first pane: switched to, held for its
    // first frame, answered.
    let delivered = bench.sessions.deliver(
        id,
        &focused.pane,
        b"echo delivered-$((6*7))\r".to_vec(),
        false,
    );
    let delivered = tokio::time::timeout(Duration::from_secs(15), delivered)
        .await
        .expect("answered")
        .expect("answered");
    assert!(delivered);
    assert!(
        eventually(10, || herdr.read(&focused.pane).contains("delivered-42")).await,
        "{}",
        herdr.read(&focused.pane)
    );
    let push = bench.sessions.herdr_view(id).unwrap().push_active;
    eprintln!(
        "herdr: first frame {first_frame_ms} ms after connect, echo {echo_ms} ms, push active: {push}"
    );

    // The disconnect releases the attach; herdr's panes live on.
    let _ = bench.sessions.disconnect(id).await;
    assert!(herdr.read(&focused.pane).contains("delivered-42"));
    assert_eq!(herdr.run(&["api", "snapshot"]).1, 0);
}

// MARK: mosh + tmux

/// mosh renders tmux's own client; the `-CC` sidecar beside it on a channel
/// of the kept bootstrap connection: the tree, the renderer found by its
/// pid, its window pinned to the mosh grid, a switch from the tree moving
/// the renderer (and only it), the renderer moving by itself followed, the
/// status line of a session Moshpit made turned off and back, the control
/// plane rebuilt while mosh carries on, and on the disconnect everything
/// handed back and the server stopped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn mosh_tmux_sidecar_tree_pin_and_follow() {
    need!();
    let sshd = Sshd::start().await;
    let tmux = Tmux::new();
    tmux.new_session("alpha");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    tmux.new_session("beta");
    let mut bench = Bench::with(sshd.addr(), quick());
    let started = Instant::now();
    let id = bench.sessions.connect(mosh_tmux_spec(sshd.port, &tmux));
    bench.sessions.set_visible(Some(id));
    let pid = mosh_server(&mut bench, id).await;
    let _reaper = Reaper(Some(pid));
    let (_, panes) = bench.panes(id, 10).await;
    assert_eq!(panes.len(), 1);
    assert_eq!(
        panes[0].pane, SHELL_PANE,
        "mosh paints; the sidecar doesn't"
    );
    let mut term = Term::attach(panes[0].stream);
    term.resize(90, 28);
    // The renderer: tmux's client as mosh's program, on the most recent
    // session (beta, or from tmux 3.8 the phone's own group of it,
    // `beta~mp-…`). Over mosh its status line is off, so tmux says where it is.
    let clients = || tmux.run(&["list-clients", "-F", "#{client_control_mode} #{session_name}"]);
    assert!(
        eventually(15, || clients().lines().any(|l| l.starts_with("0 beta"))).await,
        "{}\n{}",
        clients(),
        term.text()
    );

    // The tree, from the sidecar.
    assert!(
        eventually(15, || bench.sessions.tmux_view(id).is_some_and(|view| {
            view.snapshot.is_attached
                && view.snapshot.sessions.len() == 2
                && view.snapshot.active_session_id.as_deref()
                    == Some(&*tmux.run(&["display-message", "-p", "-t", "beta", "#{session_id}"]))
        }))
        .await,
        "{:?}",
        bench.sessions.tmux_view(id)
    );
    let tree_ms = started.elapsed().as_millis();
    // The renderer's window, pinned to the mosh grid.
    let size = |t: &Tmux, s: &str| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            s,
            "#{window_width}x#{window_height}",
        ])
    };
    assert!(
        eventually(10, || size(&tmux, "beta") == "90x28").await,
        "{}",
        size(&tmux, "beta")
    );
    assert_eq!(
        tmux.run(&["show-options", "-w", "-t", "beta", "-v", "window-size"]),
        "manual"
    );

    // A switch from the tree: the renderer moves (only it: the sidecar's
    // own client follows, nothing else exists to move).
    let alpha = tmux.run(&["display-message", "-p", "-t", "alpha", "#{session_id}"]);
    bench
        .sessions
        .tmux(id, TmuxAction::SelectSession(alpha.clone()))
        .unwrap();
    let at = term.screen.len();
    // tmux repaints only what changed: the status line's "[" stays.
    assert!(
        term.wait_for_from(at, "alpha", 15).await.is_some(),
        "{}",
        term.text()
    );
    assert!(
        eventually(10, || size(&tmux, "alpha") == "90x28").await,
        "{}",
        size(&tmux, "alpha")
    );

    // The renderer moves by itself (prefix s in the mosh TUI): followed.
    let renderer = tmux.run(&[
        "list-clients",
        "-F",
        "#{client_control_mode} #{client_name}",
    ]);
    let renderer = renderer
        .lines()
        .find(|line| line.starts_with("0 "))
        .map(|line| line[2..].to_owned())
        .expect("the renderer");
    let beta = tmux.run(&["display-message", "-p", "-t", "beta", "#{session_id}"]);
    tmux.run(&["switch-client", "-c", &renderer, "-t", "beta"]);
    assert!(
        eventually(10, || bench
            .sessions
            .tmux_view(id)
            .and_then(|view| view.snapshot.active_session_id)
            .as_deref()
            == Some(&*beta))
        .await,
        "the sidecar follows the renderer"
    );

    // A session the person makes from the phone: no status line while on it.
    bench
        .sessions
        .tmux(
            id,
            TmuxAction::NewSession {
                name: Some("mine".into()),
            },
        )
        .unwrap();
    assert!(
        eventually(10, || tmux.viewed_status("mine") == "off").await,
        "status={} sessions={:?}",
        tmux.viewed_status("mine"),
        tmux.sessions()
    );

    // The control plane's connection dies: rebuilt, while mosh carries on.
    let states_before = bench.states(id).len();
    bench.sessions.inject_fault(id, LinkFault::Cut);
    tokio::time::sleep(Duration::from_secs(1)).await;
    tmux.run(&["new-window", "-t", "beta", "-n", "later"]);
    assert!(
        eventually(20, || bench.sessions.tmux_view(id).is_some_and(|view| view
            .snapshot
            .windows
            .values()
            .any(|w| w.name == "later")))
        .await,
        "a rebuilt sidecar reads the tree again"
    );
    bench.drain();
    assert_eq!(
        bench.states(id).len(),
        states_before,
        "the mosh session never left Connected: {:?}",
        bench.states(id)
    );
    let at = term.screen.len();
    term.write(b"echo still-$((6*7))\r");
    assert!(term.wait_for_from(at, "still-42", 10).await.is_some());

    // The disconnect: the pins and the status line handed back, the server
    // stopped.
    let _ = bench.sessions.disconnect(id).await;
    for session in ["alpha", "beta", "mine"] {
        let pinned = tmux.run(&["show-options", "-w", "-t", session, "-v", "window-size"]);
        assert!(
            pinned.is_empty() || pinned == "latest",
            "{session}: {pinned:?}"
        );
    }
    assert_eq!(
        tmux.viewed_status("mine"),
        "",
        "the status line it turned off is back"
    );
    assert!(
        eventually(5, || !alive(pid)).await,
        "the disconnect stops mosh-server"
    );
    eprintln!("mosh + tmux: the sidecar's tree {tree_ms} ms after connect");
}

// MARK: mosh-servers

/// A mosh start cut short after the server named its pid but before the
/// session took it (the bootstrap channel still open, as on a slow link):
/// the server is stopped, not left waiting forever (B194).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and mosh-server; see the module docs"]
async fn mosh_a_start_cut_short_stops_its_server() {
    need!();
    let sshd = Sshd::start().await;
    let scratch = Scratch::new("slow-mosh");
    let pids = scratch.join("pids");
    let wrapper = scratch.join("mosh-server");
    // mosh-server, then a channel that stays open 4 s: the attempt has the
    // server's pid in hand and is still reading when it is abandoned.
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nPATH=\"$PATH:/opt/homebrew/bin:/usr/local/bin:/usr/bin\" mosh-server \"$@\" 2>&1 | tee -a '{}'\nsleep 4\n",
            pids.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.protocol = Protocol::Mosh;
    spec.mosh_server_path = Some(wrapper.display().to_string());
    let id = bench.sessions.connect(spec);
    let mut pid = None;
    assert!(
        eventually(15, || {
            pid = std::fs::read_to_string(&pids).ok().and_then(|text| {
                text.split("pid = ")
                    .nth(1)
                    .and_then(|rest| rest.split(']').next())
                    .and_then(|n| n.trim().parse::<i64>().ok())
            });
            pid.is_some()
        })
        .await
    );
    let pid = pid.unwrap();
    let _reaper = Reaper(Some(pid));
    assert!(alive(pid));
    // Still connecting (the channel is open): the person gives up.
    assert_eq!(
        bench.sessions.view(id).unwrap().state,
        SessionState::Connecting
    );
    let closed = Instant::now();
    let _ = bench.sessions.disconnect(id).await;
    assert!(
        eventually(10, || !alive(pid)).await,
        "the abandoned start's server is stopped"
    );
    eprintln!(
        "B194: the cut-short start's mosh-server {pid} gone {} ms after the disconnect",
        closed.elapsed().as_millis()
    );
}

/// mosh's loop dies and the session reconnects with a new server: the old
/// one, contacted, is stopped over the new connection instead of being left
/// running for good (B195).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and mosh-server; see the module docs"]
async fn mosh_a_reconnect_stops_the_server_it_left() {
    need!();
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.protocol = Protocol::Mosh;
    let id = bench.sessions.connect(spec);
    let old = mosh_server(&mut bench, id).await;
    let _old_reaper = Reaper(Some(old));
    let (_, panes) = bench.panes(id, 10).await;
    let mut term = Term::attach(panes[0].stream);
    term.write(b"echo first-$((6*7))\r");
    assert!(term.wait_for("first-42", 15).await, "contacted");

    let lost = Instant::now();
    bench.sessions.inject_mosh_loss(id);
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    bench.connected(id).await;
    let new = bench.sessions.view(id).unwrap().mosh_server_pid.unwrap();
    let _new_reaper = Reaper(Some(new));
    assert_ne!(old, new, "a new server");
    assert!(
        eventually(10, || !alive(old)).await,
        "the old server is stopped, not orphaned"
    );
    eprintln!(
        "B195: reconnected onto mosh-server {new}; the old one, {old}, gone {} ms after the loss",
        lost.elapsed().as_millis()
    );
    assert!(alive(new));
    let at = term.screen.len();
    term.write(b"echo second-$((6*7))\r");
    assert!(term.wait_for_from(at, "second-42", 15).await.is_some());
    let _ = bench.sessions.disconnect(id).await;
    assert!(eventually(5, || !alive(new)).await);
}

// MARK: tmux's pins

/// The connection drops and the redial never lands; the person closes the
/// session: the retired attach's pin is handed back BEFORE the disconnect
/// says it is done — the platform holds the process awake only until then
/// (B203).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn tmux_the_pin_debt_is_paid_before_the_disconnect_is_done() {
    need!();
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::new();
    tmux.new_session("w10b");
    let mut bench = Bench::with(
        std::net::SocketAddr::from(([127, 0, 0, 1], relay.port)),
        quick(),
    );
    let mut spec = spec(relay.port);
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let size = || tmux.run(&["show-options", "-w", "-t", "w10b", "-v", "window-size"]);
    assert!(eventually(10, || size() == "manual").await, "pinned");
    relay.stall();
    relay.cut();
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    relay.heal();
    let asked = Instant::now();
    let done = bench.sessions.disconnect(id);
    tokio::time::timeout(Duration::from_secs(12), done)
        .await
        .expect("done in bounded time")
        .expect("done");
    let at_done = size();
    assert!(
        at_done.is_empty() || at_done == "latest",
        "handed back by the time the disconnect is done: {at_done:?}"
    );
    eprintln!(
        "B203: the disconnect was done {} ms after it was asked, the pin already back",
        asked.elapsed().as_millis()
    );
}

/// A reconnect while the terminal isn't on screen: the new attach doesn't
/// squash the window to the phone; shown again, it pins.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn tmux_a_reconnect_off_screen_pins_only_once_shown() {
    need!();
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let sshd = Sshd::start().await;
    let relay = Relay::start(sshd.addr()).await;
    let tmux = Tmux::new();
    tmux.new_session("w10b");
    let mut bench = Bench::with(
        std::net::SocketAddr::from(([127, 0, 0, 1], relay.port)),
        quick(),
    );
    let mut spec = spec(relay.port);
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let (first, _) = bench.panes(id, 20).await;
    let size = || tmux.run(&["show-options", "-w", "-t", "w10b", "-v", "window-size"]);
    assert!(eventually(10, || size() == "manual").await, "pinned");
    // Back to Home: handed back.
    bench.sessions.set_visible(None);
    assert!(eventually(10, || size() != "manual").await);
    // A drop while on Home: the reconnect attaches without the pin.
    relay.cut();
    bench
        .state(id, 10, |s| *s == SessionState::Reconnecting)
        .await;
    bench.connected(id).await;
    let (second, _) = bench.panes_after(id, first, 20).await;
    assert!(second > first);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_ne!(
        size(),
        "manual",
        "nobody looks: the window stays the desktop's"
    );
    // Shown: pinned.
    bench.sessions.set_visible(Some(id));
    assert!(
        eventually(10, || size() == "manual").await,
        "pinned once shown"
    );
    let _ = bench.sessions.disconnect(id).await;
}

// MARK: mosh's echo acknowledgement

/// Predictive echo's evidence: after the frame that echoed it, a mark in
/// the shell's channel says how much of the input written into it the
/// server has echoed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and mosh-server; see the module docs"]
async fn mosh_the_echo_ack_follows_the_echo() {
    need!();
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.protocol = Protocol::Mosh;
    spec.pty_command = Some("/bin/cat".into());
    let id = bench.sessions.connect(spec);
    let pid = mosh_server(&mut bench, id).await;
    let _reaper = Reaper(Some(pid));
    let (_, panes) = bench.panes(id, 10).await;
    let stream = panes[0].stream;
    let rung = std::sync::Arc::new(tokio::sync::Notify::new());
    let bell = rung.clone();
    assert!(moshpit_stream::view::attach(stream, move || bell.notify_one()));
    let mut bytes = Vec::new();
    let mut acks: Vec<(usize, i64)> = Vec::new();
    let mut buf = [0u8; 4096];
    let mut read = |bytes: &mut Vec<u8>, acks: &mut Vec<(usize, i64)>| loop {
        let (n, mark) = moshpit_stream::view::read_marked(stream, &mut buf);
        bytes.extend_from_slice(&buf[..n]);
        if let Some(mark) = mark {
            if mark.code == view_marks::ECHO_ACK {
                acks.push((bytes.len(), mark.arg));
            }
            continue;
        }
        if n < buf.len() {
            return;
        }
    };
    let mut written = 0;
    let mut samples = Vec::new();
    for word in ["alpha", "beta", "gamma"] {
        let typed = Instant::now();
        assert!(moshpit_stream::view::write(stream, word.as_bytes()));
        written += word.len() as i64;
        let deadline = Instant::now() + Duration::from_secs(10);
        while acks.last().map(|(_, arg)| *arg) != Some(written) && Instant::now() < deadline {
            let _ = tokio::time::timeout(Duration::from_millis(200), rung.notified()).await;
            read(&mut bytes, &mut acks);
        }
        samples.push(typed.elapsed().as_micros());
        assert_eq!(acks.last().map(|(_, arg)| *arg), Some(written), "{acks:?}");
        let (at, _) = *acks.last().unwrap();
        let text = String::from_utf8_lossy(&bytes[..at]).into_owned();
        assert!(
            text.contains(word),
            "the ack comes after its echo: {text:?}"
        );
    }
    moshpit_stream::view::detach(stream);
    eprintln!("mosh echo ack: keystroke to ack {samples:?} µs");
    let _ = bench.sessions.disconnect(id).await;
}
