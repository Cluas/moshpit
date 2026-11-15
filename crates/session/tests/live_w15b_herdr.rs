//! W15b's herdr hunt against real programs on this machine's loopback: an
//! sshd of our own and a herdr server on a private session under a temp
//! HOME (`--session …`; every herdr command carries it). Nothing touches
//! `~/.ssh`, the machine's sshd or the user's own herdr.
//!
//! - B683: the desktop moves herdr's focus while the phone types: the phone
//!   keeps its pane — on screen, as the active one — and nothing typed is
//!   dropped or lands elsewhere.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_w15b_herdr -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::time::{Duration, Instant};

use moshpit_session::{EventKind, HerdrAction, HerdrAnswer, Multiplexer, SessionId};
use support::*;

async fn eventually(seconds: u64, mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    check()
}

fn on_screen(bench: &Bench, id: SessionId) -> Option<String> {
    bench
        .sessions
        .view(id)
        .and_then(|v| v.herdr)
        .and_then(|v| v.attached_pane)
}

fn stream_of(bench: &Bench, id: SessionId, pane: &str) -> Option<u64> {
    bench
        .sessions
        .view(id)?
        .panes
        .into_iter()
        .find(|p| p.pane == pane)
        .map(|p| p.stream)
}

/// B683: the phone followed herdr's shared focus wherever the desktop moved
/// it, switching under the person and dropping what they were typing. It
/// keeps its pane now; herdr's frame channel stays on an unfocused pane.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and herdr; see the module docs"]
async fn the_desktop_moving_herdrs_focus_leaves_the_phone_on_its_pane() {
    if !PrivateHerdr::available() || !Sshd::available() {
        eprintln!("skipped: no herdr or sshd");
        return;
    }
    let herdr = PrivateHerdr::start();
    let home = herdr.home.display().to_string();
    let sshd = Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.multiplexer = Multiplexer::Herdr;
    spec.herdr_path = Some(herdr.custom_path());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let _ = bench.panes(id, 20).await;
    let a = herdr.focused().expect("a focused pane");
    assert!(eventually(15, || on_screen(&bench, id).as_deref() == Some(a.as_str())).await);

    // A second pane; the phone goes back to A (its own pick).
    let before: Vec<String> = herdr.panes();
    let token = bench.sessions.herdr(id, HerdrAction::NewPane);
    let answer = bench
        .until(id, 15, |kind| match kind {
            EventKind::HerdrAnswer { token: t, answer } if *t == token => Some(answer.clone()),
            _ => None,
        })
        .await;
    assert_eq!(answer, HerdrAnswer::Done);
    let mut b = None;
    assert!(
        eventually(15, || {
            b = herdr.panes().into_iter().find(|p| !before.contains(p));
            b.is_some()
        })
        .await
    );
    let b = b.unwrap();
    bench.sessions.herdr(id, HerdrAction::SelectPane(a.clone()));
    assert!(eventually(15, || on_screen(&bench, id).as_deref() == Some(a.as_str())).await);
    let typing = Term::attach(stream_of(&bench, id, &a).expect("A's terminal"));
    tokio::time::sleep(Duration::from_millis(800)).await;

    // The desktop moves herdr's focus to B, and keeps typing… on the phone.
    herdr.run(&["pane", "zoom", "--pane", &b, "--on"]);
    herdr.run(&["pane", "zoom", "--pane", &b, "--off"]);
    assert!(
        eventually(10, || herdr.focused().as_deref() == Some(b.as_str())).await,
        "the server's focus moved"
    );
    for i in 0..5 {
        typing.write(format!("echo own{i}-$((6*7))\r").as_bytes());
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
    assert!(
        eventually(10, || (0..5).all(|i| herdr.read(&a).contains(&format!("own{i}-42"))))
            .await,
        "every line ran in A: {}",
        herdr.read(&a)
    );
    assert!(!herdr.read(&b).contains("own"), "nothing in B");
    assert_eq!(on_screen(&bench, id).as_deref(), Some(a.as_str()), "the phone kept A");
    let shown = bench
        .sessions
        .view(id)
        .unwrap()
        .panes
        .into_iter()
        .find(|p| p.active)
        .map(|p| p.pane);
    assert_eq!(shown.as_deref(), Some(a.as_str()), "A is the active one");
    let _ = bench.sessions.disconnect(id).await;
}
