//! Round thirty-seven against real programs on this machine's loopback,
//! private every one: an sshd of our own (temp keys and config, key auth
//! only), the mosh-servers the sessions start (stopped by pid), and a
//! private herdr server (`--session …` under a temp HOME) behind a TCP
//! relay of our own.
//!
//! ```sh
//! CARGO_BUILD_JOBS=4 cargo test -p moshpit-session --test live_r37 -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::time::{Duration, Instant};

use moshpit_session::{HerdrAction, Multiplexer, Protocol, SessionId};
use support::*;

macro_rules! need {
    () => {
        if !Sshd::available() {
            eprintln!("skipped: no {SSHD}");
            return;
        }
        if !PrivateHerdr::available() {
            eprintln!("skipped: no herdr");
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

fn pings(kept: &Kept) -> usize {
    kept.lines
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains("pinging the control connection"))
        .count()
}

fn view_of(bench: &Bench, id: SessionId) -> Option<moshpit_session::HerdrView> {
    bench.sessions.view(id).and_then(|view| view.herdr)
}

// MARK: B276. A stalled herdr control connection over mosh

/// A phone on mosh + herdr through the relay, push up.
async fn herdr_over_mosh(bench: &mut Bench, relay: &Relay, herdr: &PrivateHerdr) -> SessionId {
    let mut spec = spec(relay.port);
    spec.protocol = Protocol::Mosh;
    spec.multiplexer = Multiplexer::Herdr;
    spec.herdr_path = Some(herdr.custom_path());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    assert!(
        eventually(30, || view_of(bench, id).is_some_and(|v| v.push_active)).await,
        "push never came up"
    );
    id
}

/// B276: the link under herdr's control connection over mosh dies without
/// a word (mosh carries on). Two cases, each timed from the stall to the
/// redial reaching sshd:
///
/// - **asked**: the person asks something of herdr at once (a refresh):
///   the request goes out on the dead link and nothing comes back.
/// - **idle**: nothing is asked (push carries the news, the poll is 45 s
///   apart), so only the leg's own question shows the silence.
///
/// Was 28–29 s either way (20 s of quiet, then an exec with 8 s to answer).
/// Also: in use and healthy, the connection is never pinged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server, herdr and python3; see the module docs"]
async fn a_stalled_herdr_control_connection_over_mosh_is_noticed_in_seconds() {
    need!();
    if !faults_enabled() {
        eprintln!("skipped: set MOSHPIT_E2E_FAULTS=1");
        return;
    }
    let kept = kept();
    let herdr = PrivateHerdr::start();
    let home = herdr.home.display().to_string();
    let sshd = Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await;
    let mut results = Vec::new();
    for case in ["asked", "idle"] {
        kept.lines.lock().unwrap().clear();
        let relay = Relay::start(sshd.addr()).await;
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], relay.port));
        let mut bench = Bench::with(addr, quick());
        let id = herdr_over_mosh(&mut bench, &relay, &herdr).await;
        let reaper = Reaper(bench.sessions.view(id).and_then(|v| v.mosh_server_pid));

        // Healthy and in use: a refresh a second for 10 s, never a ping.
        let in_use = pings(kept);
        for _ in 0..10 {
            bench.sessions.herdr(id, HerdrAction::Refresh);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let pinged_in_use = pings(kept) - in_use;

        let signed_in = sshd.accepted();
        relay.stall();
        relay.heal();
        let stalled = Instant::now();
        if case == "asked" {
            bench.sessions.herdr(id, HerdrAction::Refresh);
        }
        let redialled = eventually(60, || sshd.accepted() > signed_in).await;
        let noticed = stalled.elapsed();
        eprintln!(
            "{case}: redialled {redialled} after {} ms; pings while healthy and in use: {pinged_in_use}",
            noticed.as_millis()
        );
        if !redialled || noticed > Duration::from_secs(16) {
            eprintln!("{}", kept.lines.lock().unwrap().join("\n"));
        }
        let back = eventually(20, || view_of(&bench, id).is_some_and(|v| v.push_active)).await;
        let _ = bench.sessions.disconnect(id).await;
        if let Some(pid) = reaper.0 {
            let _ = eventually(15, || !alive(pid)).await;
        }
        drop(reaper);
        results.push((case, redialled, noticed, pinged_in_use, back));
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    for (case, redialled, noticed, pinged_in_use, back) in results {
        assert!(
            redialled,
            "{case}: the control connection was never redialled"
        );
        // Asked: 2 s of silence after the request, a 4 s ping, the dial.
        // Idle: up to 8 s of quiet, the ping, the dial.
        let bound = match case {
            "asked" => Duration::from_secs(9),
            _ => Duration::from_secs(15),
        };
        assert!(
            noticed <= bound,
            "{case}: noticed after {noticed:?} (bound {bound:?})"
        );
        assert_eq!(
            pinged_in_use, 0,
            "{case}: a healthy connection in use was pinged"
        );
        assert!(back, "{case}: push never came back on the new connection");
    }
}
