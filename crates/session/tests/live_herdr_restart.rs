//! Herdr checks against real programs on this machine's loopback,
//! on the shared rig (`support`'s `PrivateHerdr` and `Sshd`): an sshd of our
//! own and a herdr server on a private session under a temp HOME. Nothing
//! touches `~/.ssh`, the machine's sshd or the user's own herdr.
//!
//! - B708: a notification sent before a herdr restart, tapped after it,
//!   lands on the pane that sent it.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_herdr_restart -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::time::{Duration, Instant};

use moshpit_session::{HerdrAction, Multiplexer, SessionId};
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

/// B708: a push names a herdr pane by the `HERDR_PANE_ID` its agent's hook
/// runs with (the sender's `pane`), and the app's agents machine keys herdr
/// panes by the same id. A restart numbers herdr's terminals anew; were
/// that the id, a tap after it would land on another pane or nowhere. Its
/// panes keep their ids (herdr 0.9.5, probed): the phone connected before
/// the restart, a notification from before it lands, after it, on the pane
/// that asked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and herdr; see the module docs"]
async fn a_notification_from_before_a_herdr_restart_lands_on_its_pane() {
    if !PrivateHerdr::available() || !Sshd::available() {
        eprintln!("skipped: no herdr or sshd");
        return;
    }
    let mut herdr = PrivateHerdr::start();
    let first = herdr.focused().expect("a focused pane");
    herdr.run(&["pane", "split", "--pane", &first, "--direction", "right"]);
    let asked = herdr
        .panes()
        .into_iter()
        .find(|pane| *pane != first)
        .expect("a second pane");
    // What the hook in that pane sees, and the sender pushes.
    let seen = herdr.home.join("asked");
    herdr.run(&[
        "pane",
        "run",
        &asked,
        &format!("echo \"$HERDR_PANE_ID\" > {}", seen.display()),
    ]);
    let mut pushed = String::new();
    assert!(
        eventually(10, || {
            pushed = std::fs::read_to_string(&seen)
                .unwrap_or_default()
                .trim()
                .to_owned();
            !pushed.is_empty()
        })
        .await,
        "the pane's hook environment"
    );
    assert_eq!(pushed, asked, "the push names the pane, not its terminal");

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
    assert!(eventually(15, || on_screen(&bench, id).is_some()).await);

    // herdr restarts between the push and the tap.
    let terminal = herdr.terminal(&asked);
    herdr.restart();
    assert!(
        herdr.panes().contains(&pushed),
        "the pane kept its id: {:?}",
        herdr.panes()
    );
    assert_ne!(
        herdr.terminal(&asked),
        terminal,
        "its terminal is numbered anew"
    );

    // The tap: the app selects the pane the notification names.
    bench
        .sessions
        .herdr(id, HerdrAction::SelectPane(pushed.clone()));
    assert!(
        eventually(30, || on_screen(&bench, id).as_deref()
            == Some(pushed.as_str()))
        .await,
        "on screen: {:?}",
        on_screen(&bench, id)
    );
    let _ = bench.sessions.disconnect(id).await;
}
