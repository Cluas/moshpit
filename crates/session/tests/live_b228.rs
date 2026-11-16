//! B228 against real programs on this machine's loopback, private every
//! one: an sshd of our own (temp keys and config), mosh-server started
//! through it, a tmux server on a private socket (`-L moshpit-b228-…`,
//! `-f /dev/null`) that has no session. Everything started here is stopped
//! when the test ends; `~/.ssh`, the machine's sshd and the user's own tmux
//! are never addressed.
//!
//! A host whose tmux has no session is live: it waits for the person's
//! first session, made from the empty state. The Rust session path read it
//! as an attach under way, then a stalled one (its notice and Retry); the
//! Swift path asked the server and showed the empty tree (`SessionHub`'s
//! `awaitingFirstSession`, mosh + tmux).
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_b228 -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::time::{Duration, Instant};

use moshpit_session::{EventKind, Multiplexer, Protocol, SessionId, TmuxAction};
use moshpit_tmux::controller::is_group_name;
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

/// Waits until `id`'s tmux view says its server has no session; a stalled
/// attach on the way fails.
async fn reads_no_session(bench: &mut Bench, id: SessionId) {
    while !bench
        .sessions
        .tmux_view(id)
        .is_some_and(|view| view.no_session)
    {
        let stalled = bench
            .until(id, 20, |kind| match kind {
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
    let view = bench.sessions.view(id).unwrap();
    assert!(!view.attach_stalled);
    let tree = view.tmux_view.unwrap();
    assert!(!tree.snapshot.is_attached, "nothing to attach");
    assert!(tree.snapshot.sessions.is_empty(), "{:?}", tree.snapshot);
}

/// The person's first session, from the empty state: once it is attached,
/// the view no longer says there is none.
async fn first_session_ends_it(bench: &mut Bench, id: SessionId, tmux: &Tmux) {
    bench
        .sessions
        .tmux(
            id,
            TmuxAction::NewSession {
                name: Some("b228".to_owned()),
            },
        )
        .unwrap();
    let attached = eventually(20, || {
        bench
            .sessions
            .tmux_view(id)
            .is_some_and(|view| view.snapshot.is_attached && !view.no_session)
    })
    .await;
    assert!(
        attached,
        "{:?}",
        bench
            .sessions
            .tmux_view(id)
            .map(|view| (view.snapshot.is_attached, view.no_session))
    );
    // The phone's own group of it (`b228~mp-…`, B459) is no session of the
    // person's: only theirs is counted.
    let sessions: Vec<String> = tmux
        .sessions()
        .into_iter()
        .filter(|name| !is_group_name(name))
        .collect();
    assert_eq!(sessions, vec!["b228".to_owned()]);
    assert!(!bench.sessions.view(id).unwrap().attach_stalled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn ssh_tmux_with_no_session_waits_for_the_first_instead_of_stalling() {
    need!();
    let sshd = Sshd::start().await;
    let tmux = Tmux::named("moshpit-b228");
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    reads_no_session(&mut bench, id).await;
    first_session_ends_it(&mut bench, id, &tmux).await;
    let _ = bench.sessions.disconnect(id).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn mosh_tmux_with_no_session_waits_for_the_first_instead_of_stalling() {
    need!();
    let sshd = Sshd::start().await;
    let tmux = Tmux::named("moshpit-b228m");
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.protocol = Protocol::Mosh;
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    reads_no_session(&mut bench, id).await;
    first_session_ends_it(&mut bench, id, &tmux).await;
    let _ = bench.sessions.disconnect(id).await;
}
