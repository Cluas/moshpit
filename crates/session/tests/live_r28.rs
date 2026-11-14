//! Round twenty-eight against real programs on this machine's loopback,
//! private every one: an sshd of our own (temp keys and config), mosh-server
//! started through it, a tmux server on a private socket (`-L
//! moshpit-r28-…`, `-f /dev/null`). Everything started here is stopped when
//! the test ends; `~/.ssh`, the machine's sshd and the user's own tmux are
//! never addressed.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_r28 -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::time::{Duration, Instant};

use moshpit_session::{Multiplexer, Protocol};
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

/// mosh + tmux, a notification's reply for a pane that isn't tmux's active
/// one, sent the moment the session is connected: before the `-CC` sidecar
/// has its tree. Two clients of one private tmux — the phone's renderer
/// (mosh's program is tmux's own client) and the desktop, which left the
/// other pane active. The reply went down the mosh shell, said sent, and
/// landed in the active pane (or ran in the login shell before tmux
/// attached); it waits for the sidecar now, which sends it to its pane.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn a_reply_before_the_sidecar_is_up_reaches_its_pane_never_the_active_one() {
    need!();
    let sshd = Sshd::start().await;
    let tmux = Tmux::named("moshpit-r28");
    tmux.new_session("work");
    // The desktop splits and keeps the first pane active.
    tmux.run(&["split-window", "-d", "-t", "work", "/bin/sh"]);
    let listing = tmux.run(&[
        "list-panes",
        "-t",
        "work",
        "-F",
        "#{pane_id} #{pane_active}",
    ]);
    let pane = |active: &str| {
        listing
            .lines()
            .find(|line| line.ends_with(active))
            .and_then(|line| line.split(' ').next())
            .expect("a pane")
            .to_owned()
    };
    let (active, target) = (pane(" 1"), pane(" 0"));

    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.protocol = Protocol::Mosh;
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let pid = bench
        .sessions
        .view(id)
        .and_then(|view| view.mosh_server_pid)
        .expect("mosh-server's pid");
    let _reaper = Reaper(Some(pid));

    // At once: the sidecar has no tree yet.
    let knew = bench
        .sessions
        .tmux_view(id)
        .is_some_and(|view| view.snapshot.panes.contains_key(&target));
    let sent_at = Instant::now();
    let answer = bench
        .sessions
        .deliver(id, &target, b"echo reply-$((6*7))\r".to_vec(), false);
    let sent = tokio::time::timeout(Duration::from_secs(25), answer)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false);
    let answered = sent_at.elapsed().as_millis();
    let capture = |pane: &str| tmux.run(&["capture-pane", "-p", "-t", pane]);
    eventually(5, || {
        capture(&target).contains("reply-42") || capture(&active).contains("reply-42")
    })
    .await;
    let in_target = capture(&target).contains("reply-42");
    let in_active = capture(&active).contains("reply-42");
    println!(
        "the sidecar knew {target} when the reply was sent: {knew}; answered {sent} after {answered} ms; \
         in its pane {target}: {in_target}; in the active pane {active}: {in_active}"
    );
    assert!(
        !knew,
        "the sidecar was up already: the window this test needs is gone"
    );
    assert!(
        !in_active,
        "the reply landed in the active pane {active}, said sent: {sent}"
    );
    assert!(sent, "never answered sent");
    assert!(in_target, "the reply never reached {target}");

    let _ = tokio::time::timeout(Duration::from_secs(10), bench.sessions.disconnect(id)).await;
}
