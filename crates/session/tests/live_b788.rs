//! B788: New Session on a host with no tmux server made nothing: the app
//! flashed a disconnect and was back on New Session, and the host had no
//! server. The control client and `has-session` find a Homebrew tmux on
//! the probed PATH; the exec making the first session didn't, on a host
//! whose non-interactive PATH has no Homebrew (a Mac's, over SSH).
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
