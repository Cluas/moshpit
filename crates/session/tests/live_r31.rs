//! Round thirty-one against real programs on this machine's loopback,
//! private every one: an sshd of our own (temp keys and config), a tmux
//! server on a private socket (`-L moshpit-r31-…`, `-f /dev/null`).
//! Everything started here is stopped when the test ends; `~/.ssh`, the
//! machine's sshd and the user's own tmux are never addressed.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_r31 -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::time::{Duration, Instant};

use moshpit_session::{EventKind, Multiplexer, TmuxAction};
use support::*;

macro_rules! need {
    () => {
        if !Sshd::available() {
            eprintln!("skipped: no {SSHD}");
            return;
        }
    };
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

/// B250: SSH + tmux on a server with no session, the person's "new
/// session" from the empty state with a name holding a line break (a paste
/// accident). The core dropped the name and made a session tmux named
/// ("0"), and said nothing; it refuses it now, before anything is sent, and
/// makes no session. The same "new session" with a name as typed makes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn a_first_session_named_with_a_line_break_is_refused_and_none_is_made() {
    need!();
    let sshd = Sshd::start().await;
    let tmux = Tmux::named("moshpit-r31");
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    // No session on the server: the empty state, not a stalled attach
    // (B228).
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
    assert!(tmux.sessions().is_empty(), "{:?}", tmux.sessions());

    let answer = format!(
        "{:?}",
        bench.sessions.tmux(
            id,
            TmuxAction::NewSession {
                name: Some("two\nlines".to_owned()),
            },
        )
    );
    // The exec that makes a first session answers in well under a second
    // on loopback; three leave it room.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        tmux.sessions().is_empty(),
        "a session was made for a refused name: {:?}",
        tmux.sessions()
    );
    assert_eq!(answer, "Err(NameHasControlCharacters)");

    let answer = format!(
        "{:?}",
        bench.sessions.tmux(
            id,
            TmuxAction::NewSession {
                name: Some(" r31\n".to_owned()),
            },
        )
    );
    assert!(answer == "Ok(())" || answer == "()", "{answer}");
    assert!(
        eventually(10, || tmux.sessions() == vec!["r31".to_owned()]).await,
        "{:?}",
        tmux.sessions()
    );
    let _ = bench.sessions.disconnect(id).await;
}
