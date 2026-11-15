//! B754: a host that takes the TCP connection and then never answers the
//! SSH handshake (an sshd throttling the address, as OpenSSH's
//! `PerSourcePenalties` did to dry run 6's staged host; a black hole past
//! the TCP leg) must never make the UI wait. Everything the app's main
//! thread calls on the registry while a session connects (its view, its
//! tmux and herdr trees, an intent, which session shows) answers at once
//! through the whole stall, and the stall ends as a failure in so many
//! words when the handshake's bound passes.

mod support;

use std::time::{Duration, Instant};

use moshpit_session::{SessionState, Timing, TmuxAction};
use support::*;

/// A frame at 60 Hz: what one of the UI's calls may take, at most.
const FRAME: Duration = Duration::from_millis(16);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_handshake_never_holds_the_uis_calls() {
    // Takes every connection and never writes a byte: no banner, no KEX.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let silent = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let bound = Duration::from_secs(3);
    let timing = Timing {
        handshake_timeout: bound,
        ..quick()
    };
    let mut bench = Bench::with(addr, timing);
    let started = Instant::now();
    let id = bench.sessions.connect(spec(addr.port()));

    // The app's main thread, as it draws while the session connects: every
    // read and intent it makes, every 4 ms, past the handshake's bound.
    let sessions = bench.sessions.clone();
    let ui = std::thread::spawn(move || {
        let mut worst = (Duration::ZERO, "");
        let mut calls = 0u32;
        let end = Instant::now() + bound + Duration::from_secs(1);
        let mut timed = |name: &'static str, call: &dyn Fn()| {
            let at = Instant::now();
            call();
            let took = at.elapsed();
            if took > worst.0 {
                worst = (took, name);
            }
            calls += 1;
        };
        while Instant::now() < end {
            timed("view", &|| drop(sessions.view(id)));
            timed("tmux_view", &|| drop(sessions.tmux_view(id)));
            timed("herdr_view", &|| drop(sessions.herdr_view(id)));
            timed("ids", &|| drop(sessions.ids()));
            timed("set_visible", &|| sessions.set_visible(Some(id)));
            timed("tmux", &|| {
                let _ = sessions.tmux(id, TmuxAction::Relist);
            });
            std::thread::sleep(Duration::from_millis(4));
        }
        (worst, calls)
    });

    // The stall ends as a failure, within the bound and a little.
    let state = bench
        .state(id, 10, |s| matches!(s, SessionState::Failed(_)))
        .await;
    let failed_after = started.elapsed();
    let ((worst, which), calls) = ui.join().unwrap();
    silent.abort();
    let SessionState::Failed(failure) = state else {
        unreachable!()
    };
    println!(
        "handshake stall: failed after {failed_after:?} ({:?}); {calls} UI calls, the slowest {which} {worst:?}",
        failure.kind
    );
    assert!(
        failed_after < bound + Duration::from_secs(2),
        "the stall outlived its bound: {failed_after:?}"
    );
    assert!(
        calls > 100,
        "the UI's thread ran through the stall: {calls}"
    );
    assert!(
        worst < FRAME,
        "{which} held the UI's thread {worst:?} during the stall"
    );
}
