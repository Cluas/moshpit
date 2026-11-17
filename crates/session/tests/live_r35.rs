//! Round thirty-five against real programs on this machine's loopback,
//! private every one: an sshd of our own (temp keys and config, key auth
//! only), a tmux 3.6a server on a private socket (`-L moshpit-r35-…`,
//! `-f /dev/null`), the mosh-servers the sessions start (stopped by pid),
//! and for herdr a private herdr server (`--session …` under a temp HOME)
//! behind a TCP relay of our own.
//!
//! ```sh
//! CARGO_BUILD_JOBS=4 cargo test -p moshpit-session --test live_r35 -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::time::{Duration, Instant};

use moshpit_session::{Multiplexer, Protocol, SessionId};
use support::*;

macro_rules! need {
    () => {
        if !Sshd::available() {
            eprintln!("skipped: no {SSHD}");
            return;
        }
    };
}

/// Polls `check` every 50 ms for `seconds`.
async fn eventually(seconds: u64, mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    check()
}

/// A logger that keeps the core's lines, with when they came.
struct Kept {
    lines: std::sync::Mutex<Vec<String>>,
    start: Instant,
}

impl log::Log for Kept {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        if record.target().starts_with("moshpit") {
            let line = format!(
                "{:>6} ms {} {}",
                self.start.elapsed().as_millis(),
                record.target(),
                record.args()
            );
            self.lines.lock().unwrap().push(line);
        }
    }
    fn flush(&self) {}
}

fn kept() -> &'static Kept {
    static KEPT: std::sync::OnceLock<&'static Kept> = std::sync::OnceLock::new();
    KEPT.get_or_init(|| {
        let kept: &'static Kept = Box::leak(Box::new(Kept {
            lines: std::sync::Mutex::new(Vec::new()),
            start: Instant::now(),
        }));
        let _ = log::set_logger(kept);
        log::set_max_level(log::LevelFilter::Debug);
        kept
    })
}

/// The phone on mosh + tmux at 60×20.
async fn connect_phone(bench: &mut Bench, sshd: &Sshd, tmux: &Tmux) -> (SessionId, Reaper) {
    let mut spec = spec(sshd.port);
    spec.protocol = Protocol::Mosh;
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    spec.initial_grid = (60, 20);
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let pid = bench.sessions.view(id).unwrap().mosh_server_pid;
    (id, Reaper(pid))
}

/// Disconnects, and waits for mosh-server (and its tmux client) to be gone.
async fn hang_up(bench: &mut Bench, id: SessionId, reaper: &Reaper) {
    let _ = bench.sessions.disconnect(id).await;
    if let Some(pid) = reaper.0 {
        assert!(
            eventually(15, || !alive(pid)).await,
            "mosh-server {pid} still runs"
        );
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
}

// MARK: B274. The renderer's pid read over SSH

/// B274: about once in 250 live connects, the SSH exec that reads the
/// renderer's pid (mosh-server's child) didn't give it on the first ask.
/// `R35_CONNECTS` connects (default 30), each disconnected before the next;
/// every ask that isn't exactly one pid is kept with the sshd log of its
/// connect (`LIVE_SSHD_LOGLEVEL=DEBUG3` for the channel-level detail).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn the_renderers_pid_is_read_on_the_first_ask() {
    need!();
    let connects: usize = std::env::var("R35_CONNECTS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(30);
    let kept = kept();
    let sshd = Sshd::start().await;
    // `R35_STARVE=1`: sshd (and every process it starts) at background
    // QoS — a host too busy to run them promptly.
    if std::env::var("R35_STARVE").is_ok() {
        let done = std::process::Command::new("taskpolicy")
            .args(["-b", "-p", &sshd.pid().to_string()])
            .status()
            .unwrap();
        assert!(done.success());
    }
    let tmux = Tmux::named("moshpit-r35");
    tmux.new_session("a");
    // `R35_SPLIT=1`: the session split in four, a second window.
    if std::env::var("R35_SPLIT").is_ok() {
        for args in [
            &["split-window", "-v", "-t", "a:0", "/bin/sh"][..],
            &["split-window", "-h", "-t", "a:0", "/bin/sh"],
            &["split-window", "-h", "-t", "a:0.0", "/bin/sh"],
            &["new-window", "-d", "-t", "a", "/bin/sh"],
        ] {
            tmux.run(args);
        }
    }
    let mut missed = Vec::new();
    let mut asks = std::collections::BTreeMap::<String, usize>::new();
    // `R35_SHARED=1`: one registry for every connect, as the app: the
    // host's capabilities known, re-probed beside the bootstrap.
    let shared = std::env::var("R35_SHARED").is_ok();
    let mut bench = Bench::with(sshd.addr(), quick());
    for round in 0..connects {
        kept.lines.lock().unwrap().clear();
        let sshd_from = sshd.log().len();
        if !shared {
            bench = Bench::with(sshd.addr(), quick());
        }
        let (id, reaper) = connect_phone(&mut bench, &sshd, &tmux).await;
        let read = eventually(15, || {
            kept.lines
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.contains("the renderer is pid"))
        })
        .await;
        let lines = kept.lines.lock().unwrap().clone();
        let ask = lines
            .iter()
            .find_map(|line| line.split("(ask ").nth(1))
            .and_then(|rest| rest.split(')').next())
            .unwrap_or("never")
            .to_owned();
        *asks
            .entry(if read {
                ask.clone()
            } else {
                "never".to_owned()
            })
            .or_default() += 1;
        let failed: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains("pid wasn't read"))
            .collect();
        if !failed.is_empty() || !read || std::env::var("R35_DUMP").is_ok() {
            missed.push(round);
            let log = sshd.log();
            eprintln!(
                "connect {round}: read {read} at ask {ask}\n  core:\n    {}\n  sshd:\n    {}",
                lines.join("\n    "),
                log.get(sshd_from..)
                    .unwrap_or("")
                    .lines()
                    .collect::<Vec<_>>()
                    .join("\n    ")
            );
        }
        hang_up(&mut bench, id, &reaper).await;
    }
    eprintln!("{connects} connects: asks {asks:?}, missed the first ask on {missed:?}");
    assert!(missed.is_empty(), "missed: {missed:?}");
}

// MARK: B210. herdr over mosh: push after the control connection is redialled

fn view_of(bench: &Bench, id: SessionId) -> Option<moshpit_session::HerdrView> {
    bench.sessions.view(id).and_then(|view| view.herdr)
}

/// B210: herdr over mosh, push up on the control connection. The link
/// under that connection dies without a word (a stall: nothing closes,
/// the leg's probe finds it after its quiet time); mosh carries on and the
/// connection is redialled. Push rode the old connection: the client must
/// stop counting on it at once and open it again on the new one — the old
/// leg dropped the channel without telling the client, which went on
/// believing push up (the poll at its 45 s safety net, push never tried
/// again): a pane made on the host showed up to 45 s later.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server, herdr and python3; see the module docs"]
async fn herdr_push_comes_back_after_the_control_connection_is_redialled() {
    need!();
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    if !PrivateHerdr::available() {
        eprintln!("skipped: no herdr");
        return;
    }
    let kept = kept();
    kept.lines.lock().unwrap().clear();
    let herdr = PrivateHerdr::start();
    let home = herdr.home.display().to_string();
    let sshd = Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await;
    let relay = Relay::start(sshd.addr()).await;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], relay.port));
    let mut bench = Bench::with(addr, quick());
    let mut spec = spec(relay.port);
    spec.protocol = Protocol::Mosh;
    spec.multiplexer = Multiplexer::Herdr;
    spec.herdr_path = Some(herdr.custom_path());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let reaper = Reaper(bench.sessions.view(id).and_then(|v| v.mosh_server_pid));
    assert!(
        eventually(30, || view_of(&bench, id).is_some_and(|v| v.push_active)).await,
        "push never came up: {:?}\n{}",
        view_of(&bench, id).map(|v| (v.push_active, v.notice, v.snapshot.panes.len())),
        kept.lines.lock().unwrap().join("\n")
    );
    // The link under the control connection dies without a word; new
    // connections go through.
    let signed_in = sshd.accepted();
    relay.stall();
    relay.heal();
    let stalled = Instant::now();
    let mut went_down = false;
    let redialled = eventually(60, || {
        went_down |= view_of(&bench, id).is_some_and(|v| !v.push_active);
        sshd.accepted() > signed_in
    })
    .await;
    assert!(redialled, "the control connection was never redialled");
    let redial_ms = stalled.elapsed().as_millis();
    let back = eventually(20, || {
        went_down |= view_of(&bench, id).is_some_and(|v| !v.push_active);
        went_down && view_of(&bench, id).is_some_and(|v| v.push_active)
    })
    .await;
    // A change on the host now reaches the phone by push, not by the
    // 45 s poll.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let before: Vec<String> = view_of(&bench, id)
        .map(|v| v.snapshot.panes.keys().cloned().collect())
        .unwrap_or_default();
    let first = herdr.panes()[0].clone();
    herdr.run(&["pane", "split", &first, "--direction", "right"]);
    let made = Instant::now();
    let seen = eventually(5, || {
        view_of(&bench, id).is_some_and(|v| v.snapshot.panes.len() > before.len())
    })
    .await;
    let seen_ms = made.elapsed().as_millis();
    eprintln!(
        "redialled {redial_ms} ms after the stall; push went down {went_down}, back {back}; a new pane seen {seen} after {seen_ms} ms"
    );
    let _ = bench.sessions.disconnect(id).await;
    if let Some(pid) = reaper.0 {
        let _ = eventually(15, || !alive(pid)).await;
    }
    assert!(went_down, "the client went on believing push up");
    assert!(back, "push never came back on the new connection");
    assert!(seen, "the new pane took more than 5 s to show");
}
