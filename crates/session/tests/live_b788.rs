//! B788: New Session on a host with no tmux server made nothing: the app
//! flashed a disconnect and was back on New Session, and the host had no
//! server. The control client and `has-session` find a Homebrew tmux on
//! the probed PATH; the exec making the first session didn't, on a host
//! whose non-interactive PATH has no Homebrew (a Mac's, over SSH).
//!
//! B850: the same bare `tmux` in every other exec — the courtesy teardown
//! that hands the windows back (the pins, the size lease, the zoom mark,
//! the status line) — failed on such a host, and what the phone set on the
//! server stayed there after it disconnected.
//!
//! Ignored by default (they need sshd and tmux): see `live.rs`.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_b788 -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use moshpit_session::{Multiplexer, TmuxAction};
use support::*;

macro_rules! need_sshd {
    () => {
        if !Sshd::available() {
            eprintln!("skipped: no {SSHD}");
            return;
        }
    };
}

/// The host's tmux, by its name alone (no `-L`): only where `dir`'s
/// `TMUX_TMPDIR` puts its socket, never anyone else's server.
fn host_tmux(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("tmux")
        .args(args)
        .env_remove("TMUX")
        .env("TMUX_TMPDIR", dir)
        .output()
        .expect("tmux runs")
}

/// An sshd whose sessions' PATH is a Mac's, non-interactive (no Homebrew),
/// over a private `TMUX_TMPDIR`; `None` (skipped) unless this machine's
/// tmux is where Homebrew puts it, which only the probed PATH finds.
async fn limited_host() -> Option<(Sshd, Scratch, Scratch)> {
    let found = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v tmux"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let dir = Path::new(found.trim()).parent().map(Path::to_path_buf);
    if !dir
        .as_deref()
        .is_some_and(|d| [Path::new("/opt/homebrew/bin"), Path::new("/usr/local/bin")].contains(&d))
    {
        eprintln!("skipped: tmux isn't where Homebrew puts it ({found:?})");
        return None;
    }
    let home = Scratch::new("b788-home");
    let sockets = Scratch::new("b788-tmux");
    let (home_text, sockets_text) = (
        home.0.display().to_string(),
        sockets.0.display().to_string(),
    );
    let sshd = Sshd::start_configured(
        &[
            ("HOME", &home_text),
            ("ZDOTDIR", &home_text),
            ("TMUX_TMPDIR", &sockets_text),
            // A Mac's, non-interactive: no Homebrew.
            ("PATH", "/usr/bin:/bin:/usr/sbin:/sbin"),
        ],
        &[],
    )
    .await;
    Some((sshd, home, sockets))
}

/// Polls `check` every 100 ms for `seconds`.
async fn eventually(
    bench: &mut Bench,
    seconds: u64,
    mut check: impl FnMut(&Bench) -> bool,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        while bench.next(0).await.is_some() {}
        if check(bench) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    check(bench)
}

/// B788: tmux where Homebrew puts it, a host whose non-interactive PATH
/// doesn't have it, and no server: New Session "x" makes the session and
/// the phone is in it. The exec that made it ran `tmux` on the bare PATH,
/// found none, and the app attached again to the same empty server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn new_session_is_made_with_tmux_only_on_the_probed_path() {
    need_sshd!();
    let Some((sshd, _home, sockets)) = limited_host().await else {
        return;
    };
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.multiplexer = Multiplexer::Tmux;
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let empty = eventually(&mut bench, 20, |b| {
        b.sessions.tmux_view(id).is_some_and(|t| t.no_session)
    })
    .await;
    assert!(empty, "never read the server as having no session");
    bench
        .sessions
        .tmux(
            id,
            TmuxAction::NewSession {
                name: Some("x".to_owned()),
            },
        )
        .unwrap();
    let attached = eventually(&mut bench, 20, |b| {
        b.sessions.tmux_view(id).is_some_and(|t| {
            t.snapshot.is_attached && t.snapshot.sessions.values().any(|s| s.name == "x")
        })
    })
    .await;
    let made = host_tmux(&sockets.0, &["has-session", "-t", "=x"])
        .status
        .success();
    let listed = String::from_utf8_lossy(&host_tmux(&sockets.0, &["ls"]).stdout).into_owned();
    let _ = bench.sessions.disconnect(id).await;
    let _ = host_tmux(&sockets.0, &["kill-server"]);
    assert!(made, "no session x on the host: {listed:?}");
    assert!(attached, "the phone isn't in x: {listed:?}");
}

/// What the phone leaves on window `window`: its pin (`window-size
/// manual`), its size lease and its zoom mark.
fn marks(sockets: &Path, window: &str) -> Vec<String> {
    let shown = |option: &str| {
        String::from_utf8_lossy(
            &host_tmux(sockets, &["show-options", "-wqv", "-t", window, option]).stdout,
        )
        .trim()
        .to_owned()
    };
    let mut marks = Vec::new();
    if shown("window-size") == "manual" {
        marks.push("window-size manual".to_owned());
    }
    for option in ["@moshpit_size_owner", "@moshpit_zoomed"] {
        let value = shown(option);
        if !value.is_empty() {
            marks.push(format!("{option} {value}"));
        }
    }
    marks
}

/// B850: what the phone set on a window wider than it — its pin, its size
/// lease — goes when it disconnects, on a host whose non-interactive PATH
/// has no Homebrew. The courtesy teardown ran `tmux` on the bare PATH,
/// found none, and the window stayed at the phone's size.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn a_disconnect_hands_the_window_back_with_tmux_only_on_the_probed_path() {
    need_sshd!();
    let Some((sshd, _home, sockets)) = limited_host().await else {
        return;
    };
    assert!(
        host_tmux(
            &sockets.0,
            &["new-session", "-d", "-s", "work", "-x", "200", "-y", "60"]
        )
        .status
        .success()
    );
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.multiplexer = Multiplexer::Tmux;
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let pinned = eventually(&mut bench, 20, |_| !marks(&sockets.0, "work:").is_empty()).await;
    let set = marks(&sockets.0, "work:");
    let _ = bench.sessions.disconnect(id).await;
    let mut left = marks(&sockets.0, "work:");
    for _ in 0..40 {
        if left.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        left = marks(&sockets.0, "work:");
    }
    let _ = host_tmux(&sockets.0, &["kill-server"]);
    assert!(pinned, "the phone never pinned the window");
    assert_eq!(
        left,
        Vec::<String>::new(),
        "left on the window (set: {set:?})"
    );
}

/// B855: the first session made from the empty state attaches without the
/// session reconnecting. The leg reported a drop to attach again, and the
/// lifecycle went Connected, Reconnecting, Connected: the disconnect the
/// person saw flash by after New Session (Canary 151, after kill-server).
/// Now the control client attaches on a new channel of the same
/// connection: the session never leaves Connected, and the phone is in
/// the new session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn new_session_after_kill_server_never_reconnects() {
    need_sshd!();
    let tmux = Tmux::named("moshpit-b855");
    tmux.new_session("work");
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let attached = eventually(&mut bench, 20, |b| {
        b.sessions
            .tmux_view(id)
            .is_some_and(|t| t.snapshot.is_attached)
    })
    .await;
    assert!(attached, "never attached to work");
    tmux.run(&["kill-server"]);
    let empty = eventually(&mut bench, 20, |b| {
        b.sessions.tmux_view(id).is_some_and(|t| t.no_session)
    })
    .await;
    assert!(empty, "never read the server as gone");
    bench.drain();
    let before = bench.states(id).len();
    bench
        .sessions
        .tmux(
            id,
            TmuxAction::NewSession {
                name: Some("x".to_owned()),
            },
        )
        .unwrap();
    let made = eventually(&mut bench, 20, |b| {
        b.sessions.tmux_view(id).is_some_and(|t| {
            t.snapshot.is_attached && t.snapshot.sessions.values().any(|s| s.name == "x")
        })
    })
    .await;
    bench.drain();
    let since: Vec<_> = bench.states(id).into_iter().skip(before).collect();
    let _ = bench.sessions.disconnect(id).await;
    assert!(made, "the phone isn't in x: {:?}", tmux.sessions());
    assert_eq!(
        since,
        Vec::<moshpit_session::SessionState>::new(),
        "the session left Connected for New Session"
    );
}
