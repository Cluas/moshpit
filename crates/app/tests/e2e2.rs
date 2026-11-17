//! E2E-2: the scenario driver. The real session core runs against a private
//! sshd, tmux and mosh (the live tests' bench), and the app's Home model
//! (`moshpit_app::home`, what the NSH tree is drawn from) is rebuilt after
//! EVERY update the core sends. Each scenario is a script of steps; after
//! every update the driver checks the "never" invariants, so a flash (a
//! disconnect, a blank terminal, a stale frame) fails the test even when
//! the end state is right:
//!
//! - NEVER-VEIL: no `Reconnecting`, `Failed` or `Ended` state (the
//!   reconnect veil, the "disconnected" capsule, Home's dead card) unless
//!   the scenario cut the network or expects the end;
//! - NEVER-BLANK: no pane blank for over 2 s while the session is live;
//! - NEVER-STALE: after a tmux server exits, the view holds nothing of it
//!   (sessions, agent stamps, panes) within 4 s, and shows no new pane
//!   until the person makes a session;
//! - EVERY-TAP: a tap (New Session) changes what is shown within 3 s.
//!
//! A violation names the step, the time and the state. Ignored by default
//! (they need sshd, tmux, mosh-server): E2E-1's CI job runs them with
//! `--include-ignored`.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-app --test e2e2 -- --ignored --nocapture --test-threads 1
//! ```

#[path = "../../session/tests/support/mod.rs"]
mod support;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use moshpit_app::home::{self, HomeInput, SavedConnection};
use moshpit_session::{
    ConnectionSpec, EndReason, EventKind, Multiplexer, SessionId, SessionState, SessionView,
    TmuxAction,
};
use moshpit_ui::home::model::Link;
use support::*;

/// A pane may be blank this long after it is shown.
const BLANK: Duration = Duration::from_secs(2);
/// What is left of a dead tmux server may show this long.
const STALE: Duration = Duration::from_secs(4);
/// A tap must change what is shown within this.
const TAP: Duration = Duration::from_secs(3);

macro_rules! need_sshd {
    () => {
        if !Sshd::available() {
            eprintln!("skipped: no {SSHD}");
            return;
        }
    };
}

/// What a terminal would show of `bytes`: the printable ones, outside escape
/// sequences.
fn visible(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b => {
                i += 1;
                match bytes.get(i) {
                    // CSI: up to a final byte.
                    Some(b'[') => {
                        i += 1;
                        while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                            i += 1;
                        }
                    }
                    // OSC / DCS / APC: up to BEL or ST.
                    Some(b']' | b'P' | b'_') => {
                        i += 1;
                        while i < bytes.len() && bytes[i] != 0x07 && bytes[i] != 0x1b {
                            i += 1;
                        }
                        if bytes.get(i) == Some(&0x1b) {
                            i += 1;
                        }
                    }
                    _ => {}
                }
            }
            b @ 0x20..=0x7e => out.push(b as char),
            _ => {}
        }
        i += 1;
    }
    out
}

struct Tap {
    name: String,
    at: Instant,
    shown: String,
}

/// One scenario's run: the session, the Home model built from it after
/// every update, and the invariants over both.
struct Driver {
    bench: Bench,
    id: SessionId,
    started: Instant,
    step: String,
    violations: Vec<String>,
    /// The scenario cut the network: a reconnect is expected.
    net_cut: bool,
    /// The scenario expects the session to end.
    end_ok: bool,
    /// A tmux server died at this time and the view hasn't been seen clean.
    gone_at: Option<Instant>,
    term: Option<Term>,
    shown_at: Instant,
    tap: Option<Tap>,
    /// Every Home link the model took, with the time (the timeline).
    pub timeline: Vec<(Duration, Link)>,
}

impl Driver {
    async fn start(sshd: &Sshd, mut spec: ConnectionSpec, mux: Multiplexer) -> Driver {
        let bench = Bench::with(sshd.addr(), quick());
        spec.multiplexer = mux;
        let id = bench.sessions.connect(spec);
        bench.sessions.set_visible(Some(id));
        Driver {
            bench,
            id,
            started: Instant::now(),
            step: "connect".to_owned(),
            violations: Vec::new(),
            net_cut: false,
            end_ok: false,
            gone_at: None,
            term: None,
            shown_at: Instant::now(),
            tap: None,
            timeline: Vec::new(),
        }
    }

    fn step(&mut self, name: &str) {
        eprintln!("[+{:>6.2}s] step: {name}", self.started.elapsed().as_secs_f32());
        self.step = name.to_owned();
    }

    fn view(&self) -> Option<SessionView> {
        self.bench.sessions.view(self.id)
    }

    /// The Home model the NSH tree is drawn from, for the session's view.
    fn home(&self, view: &SessionView) -> (moshpit_ui::home::model::Connection, String) {
        let saved = [SavedConnection {
            id: "E2E2".to_owned(),
            name: "e2e.example.com".to_owned(),
            fingerprint: Some(fingerprint("ed25519")),
            emoji: None,
            user: "dev".to_owned(),
            host: "192.0.2.1".to_owned(),
            mosh: false,
            udp_port: 60000,
            tmux: true,
            disconnecting: false,
            via_gone: false,
            session: Some(view.clone()),
        }];
        let input = HomeInput {
            connections: &saved,
            since: &|_| BTreeMap::new(),
            version: "e2e",
            sidebar: false,
            locale: "en",
            jump_hosts: 0,
        };
        let state = home::state(&input);
        (state.connections[0].clone(), home::message(&input))
    }

    fn violate(&mut self, invariant: &str, detail: String) {
        let view = self.view();
        let line = format!(
            "[step {:?} at +{:.3}s] {invariant}: {detail}; state {:?}, panes {}, tree {:?}",
            self.step,
            self.started.elapsed().as_secs_f32(),
            view.as_ref().map(|v| &v.state),
            view.as_ref().map_or(0, |v| v.panes.len()),
            view.as_ref().and_then(|v| v.tmux_view.as_ref()).map(|t| (
                t.snapshot.is_attached,
                t.no_session,
                t.snapshot
                    .sessions
                    .values()
                    .map(|s| s.name.clone())
                    .collect::<Vec<_>>()
            )),
        );
        eprintln!("VIOLATION {line}");
        self.violations.push(line);
    }

    /// NEVER-VEIL, on one state the core sent.
    fn check_state(&mut self, state: &SessionState) {
        let bad = match state {
            SessionState::Reconnecting => !self.net_cut,
            SessionState::Failed(_) => !self.net_cut && !self.end_ok,
            SessionState::Ended(_) => !self.end_ok,
            _ => false,
        };
        if bad {
            self.violate("NEVER-VEIL", format!("the session went {state:?}"));
        }
    }

    /// Drains what the core sent, then checks the invariants on the Home
    /// model as it stands.
    fn update(&mut self) {
        // Every state is checked, so one that was gone before the next poll
        // still counts.
        let mut states = Vec::new();
        let id = self.id;
        let mut seen = 0;
        loop {
            let before = self.bench.seen.len();
            let got = futures_now(&mut self.bench);
            if !got {
                break;
            }
            for event in &self.bench.seen[before..] {
                seen += 1;
                if event.session == id
                    && let EventKind::State(state) = &event.kind
                {
                    states.push(state.clone());
                }
            }
        }
        let _ = seen;
        for state in &states {
            self.check_state(state);
        }
        let Some(view) = self.view() else { return };
        let (card, shown) = self.home(&view);
        if self.timeline.last().map(|(_, l)| *l) != Some(card.link) {
            self.timeline.push((self.started.elapsed(), card.link));
        }
        if matches!(card.link, Link::Reconnecting | Link::Dead) {
            let bad = match card.link {
                Link::Reconnecting => !self.net_cut,
                _ => !self.net_cut && !self.end_ok,
            };
            if bad {
                self.violate("NEVER-VEIL", format!("Home draws the card as {:?}", card.link));
            }
        }
        self.check_blank(&view, card.link);
        self.check_stale(&view);
        if let Some(tap) = &self.tap {
            if tap.shown != shown {
                self.tap = None;
            } else if tap.at.elapsed() > TAP {
                let name = tap.name.clone();
                self.tap = None;
                self.violate("EVERY-TAP", format!("{name} changed nothing within {TAP:?}"));
            }
        }
    }

    /// NEVER-BLANK: a live session's shown pane has painted within 2 s.
    fn check_blank(&mut self, view: &SessionView, link: Link) {
        let live = view.state == SessionState::Connected && link == Link::Live;
        let pane = view.panes.iter().find(|p| p.active).or(view.panes.first());
        let Some(pane) = pane.filter(|_| live) else {
            self.term = None;
            return;
        };
        if self.term.as_ref().map(|t| t.stream) != Some(pane.stream) {
            self.term = Some(Term::attach(pane.stream));
            self.shown_at = Instant::now();
        }
        let term = self.term.as_mut().unwrap();
        term.read();
        if visible(&term.screen).trim().is_empty() && self.shown_at.elapsed() > BLANK {
            self.shown_at = Instant::now() + Duration::from_secs(3600);
            self.violate(
                "NEVER-BLANK",
                format!("pane {} blank for over {BLANK:?}", pane.pane),
            );
        }
    }

    /// NEVER-STALE: once a server is gone, the view lets go of it.
    fn check_stale(&mut self, view: &SessionView) {
        let Some(at) = self.gone_at else { return };
        let tree = view.tmux_view.as_ref();
        let clean = tree.is_some_and(|t| {
            t.snapshot.sessions.is_empty() && t.agent_hooks.is_empty() && t.no_session
        }) && view.panes.is_empty();
        let ended = matches!(view.state, SessionState::Ended(_));
        if clean || ended {
            self.gone_at = None;
        } else if at.elapsed() > STALE {
            self.gone_at = None;
            let hooks = tree.map(|t| t.agent_hooks.keys().cloned().collect::<Vec<_>>());
            self.violate(
                "NEVER-STALE",
                format!("the view still holds the dead server (stamps {hooks:?}) after {STALE:?}"),
            );
        }
    }

    /// Runs the model for `seconds` or until `done`. A timeout is a violation
    /// of the step, so the failure names where the script stopped.
    async fn until(&mut self, what: &str, seconds: u64, done: impl Fn(&Driver) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        loop {
            self.update();
            if done(self) {
                return true;
            }
            if Instant::now() > deadline {
                self.violate("STEP-TIMEOUT", format!("never {what} in {seconds}s"));
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Keeps checking for `seconds` (the flash is in what follows a step).
    async fn watch(&mut self, seconds: f32) {
        let end = Instant::now() + Duration::from_secs_f32(seconds);
        while Instant::now() < end {
            self.update();
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn tree(&self) -> Option<moshpit_session::TmuxView> {
        self.bench.sessions.tmux_view(self.id)
    }

    async fn connect_and_attach(&mut self) {
        self.step("connect and attach");
        let id = self.id;
        self.until("connected", 30, |d| {
            d.bench
                .sessions
                .view(id)
                .is_some_and(|v| v.state == SessionState::Connected)
        })
        .await;
        self.until("attached", 20, |d| {
            d.tree().is_some_and(|t| t.snapshot.is_attached)
        })
        .await;
    }

    async fn new_session(&mut self, name: &str) {
        self.step(&format!("New Session {name:?}"));
        self.gone_at = None;
        let shown = self.view().map(|v| self.home(&v).1).unwrap_or_default();
        self.tap = Some(Tap {
            name: format!("New Session {name}"),
            at: Instant::now(),
            shown,
        });
        self.bench
            .sessions
            .tmux(
                self.id,
                TmuxAction::NewSession {
                    name: Some(name.to_owned()),
                },
            )
            .unwrap();
        let name = name.to_owned();
        self.until("in the new session", 20, move |d| {
            d.tree().is_some_and(|t| {
                t.snapshot.is_attached && t.snapshot.sessions.values().any(|s| s.name == name)
            })
        })
        .await;
        // The flash after a New Session comes after the tree says it is made.
        self.watch(3.0).await;
    }

    /// Types `line` in the shown pane; its echo (`marker`) must come back.
    async fn type_and_see(&mut self, line: &str, marker: &str) {
        self.step(&format!("type {line:?}"));
        self.until("a pane to type in", 10, |d| d.term.is_some()).await;
        let start = self.term.as_ref().map_or(0, |t| t.screen.len());
        self.term.as_ref().expect("a pane").write(line.as_bytes());
        let marker = marker.to_owned();
        self.until("the output back", 10, move |d| {
            d.term.as_ref().is_some_and(|t| {
                visible(&t.screen[start.min(t.screen.len())..]).contains(&marker)
            })
        })
        .await;
    }

    /// Disconnects (the person closes it) and fails the scenario if any
    /// invariant broke on the way.
    async fn finish(mut self) {
        self.step("close");
        self.end_ok = true;
        let _ = self.bench.sessions.disconnect(self.id).await;
        self.watch(0.5).await;
        eprintln!("timeline of Home's card: {:?}", self.timeline);
        assert!(
            self.violations.is_empty(),
            "\n{} invariant violation(s):\n{}\n",
            self.violations.len(),
            self.violations.join("\n")
        );
    }
}

/// Reads one event now, if the core has sent one.
fn futures_now(bench: &mut Bench) -> bool {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};
    let mut cx = Context::from_waker(Waker::noop());
    let mut next = std::pin::pin!(bench.events.recv());
    match next.as_mut().poll(&mut cx) {
        Poll::Ready(Some(event)) => {
            bench.seen.push(event);
            true
        }
        _ => false,
    }
}

fn tmux_spec(port: u16, tmux: &Tmux) -> ConnectionSpec {
    let mut spec = spec(port);
    spec.tmux_path = Some(tmux.prefix());
    spec
}

fn stamped(tmux: &Tmux) {
    tmux.new_session("work");
    for (option, value) in [
        ("@moshpit_state", "done"),
        ("@moshpit_agent", "claude"),
        ("@moshpit_since", "1790000000"),
        ("@moshpit_title", "builder"),
    ] {
        tmux.run(&["set-option", "-p", "-t", "work", option, value]);
    }
}

/// Happy path: connect, open the session, type, close. Nothing flashes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn happy_path_connect_open_type_close() {
    need_sshd!();
    let tmux = Tmux::named("moshpit-e2e2-happy");
    tmux.new_session("work");
    let sshd = Sshd::start().await;
    let mut d = Driver::start(&sshd, tmux_spec(sshd.port, &tmux), Multiplexer::Tmux).await;
    d.connect_and_attach().await;
    d.watch(2.5).await;
    d.type_and_see("echo e2e-$((20+22))\r", "e2e-42").await;
    d.finish().await;
}

/// B855: the tmux server is gone (the empty state), then New Session. The
/// session is Connected the whole time: no Reconnecting, no dead card.
/// Red on 9c46a0a2 (the parent of its merge 5b08c5e9), green on 029a3186.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn b855_new_session_after_the_server_went_away_never_disconnects() {
    need_sshd!();
    let tmux = Tmux::named("moshpit-e2e2-b855");
    tmux.new_session("work");
    let sshd = Sshd::start().await;
    let mut d = Driver::start(&sshd, tmux_spec(sshd.port, &tmux), Multiplexer::Tmux).await;
    d.connect_and_attach().await;
    d.step("kill the tmux server");
    tmux.run(&["kill-server"]);
    d.gone_at = Some(Instant::now());
    d.until("the empty state", 20, |d| {
        d.tree().is_some_and(|t| t.no_session)
    })
    .await;
    d.new_session("x").await;
    d.type_and_see("echo e2e-$((20+22))\r", "e2e-42").await;
    d.finish().await;
}

/// B786/B787 over SSH: the server dies (`kill-server`, `kill -9`, SIGSEGV)
/// under a live attach, an agent's stamp on its pane. The view lets go of
/// it (no stale frame: no session, stamp or pane of the dead server), and
/// the session stays connected, ready for a new session.
async fn server_dies(label: &str, how: &[&str]) {
    need_sshd!();
    let tmux = Tmux::named(&format!("moshpit-e2e2-{label}"));
    stamped(&tmux);
    let sshd = Sshd::start().await;
    let mut d = Driver::start(&sshd, tmux_spec(sshd.port, &tmux), Multiplexer::Tmux).await;
    d.connect_and_attach().await;
    d.until("the stamp", 10, |d| {
        d.tree().is_some_and(|t| t.agent_hooks.contains_key("%0"))
    })
    .await;
    d.watch(2.5).await;
    d.step(&format!("the server dies ({label})"));
    if how == ["kill-server"] {
        tmux.run(&["kill-server"]);
    } else {
        let pid = tmux.run(&["display-message", "-p", "#{pid}"]);
        assert!(!pid.is_empty());
        let status = std::process::Command::new("kill")
            .args([how[0], &pid])
            .status()
            .unwrap();
        assert!(status.success());
    }
    d.gone_at = Some(Instant::now());
    d.until("the server forgotten", 10, |d| d.gone_at.is_none()).await;
    d.watch(2.0).await;
    d.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn b786_kill_server_leaves_no_stale_frame() {
    server_dies("b786a", &["kill-server"]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn b786_kill_9_leaves_no_stale_frame() {
    server_dies("b786b", &["-9"]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn b786_a_segfault_leaves_no_stale_frame() {
    server_dies("b786d", &["-SEGV"]).await;
}

/// B787: mosh + tmux, the server crashes while the phone is away. The
/// session ends (tmux gone) and nothing of the dead server stays shown.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn b787_mosh_tmux_crash_while_away_ends_with_no_stale_frame() {
    need_sshd!();
    let tmux = Tmux::named("moshpit-e2e2-b787");
    stamped(&tmux);
    let sshd = Sshd::start().await;
    let mut spec = tmux_spec(sshd.port, &tmux);
    spec.protocol = moshpit_session::Protocol::Mosh;
    let mut d = Driver::start(&sshd, spec, Multiplexer::Tmux).await;
    d.connect_and_attach().await;
    d.watch(2.0).await;
    d.step("the phone is away and the server segfaults");
    d.bench.udp.faults.blackhole(true);
    let pid = tmux.run(&["display-message", "-p", "#{pid}"]);
    assert!(
        std::process::Command::new("kill")
            .args(["-SEGV", &pid])
            .status()
            .unwrap()
            .success()
    );
    d.end_ok = true;
    d.gone_at = Some(Instant::now());
    d.until("the session to end as tmux gone", 25, |d| {
        d.view()
            .is_some_and(|v| v.state == SessionState::Ended(EndReason::TmuxGone))
    })
    .await;
    d.bench.udp.faults.blackhole(false);
    d.finish().await;
}

/// B788/B850: a host whose non-interactive PATH has no Homebrew, no server:
/// New Session makes the session and the phone is in it (no disconnect, no
/// blank pane), and closing hands the window back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn b788_new_session_on_a_host_with_a_bare_path() {
    need_sshd!();
    let found = String::from_utf8(
        std::process::Command::new("sh")
            .args(["-c", "command -v tmux"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let dir = std::path::Path::new(found.trim()).parent().map(|p| p.to_path_buf());
    if !dir.as_deref().is_some_and(|d| {
        [
            std::path::Path::new("/opt/homebrew/bin"),
            std::path::Path::new("/usr/local/bin"),
        ]
        .contains(&d)
    }) {
        eprintln!("skipped: tmux isn't where Homebrew puts it ({found:?})");
        return;
    }
    // Darwin caps Unix domain sockets (`TMUX_TMPDIR/tmux-<uid>/default`) at
    // 104 bytes; `env::temp_dir()` under `/var/folders/...` overflows it.
    struct ShortDir(std::path::PathBuf);
    impl Drop for ShortDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let sockets_path = std::path::PathBuf::from("/tmp").join(unique("mp-b788"));
    let _ = std::fs::remove_dir_all(&sockets_path);
    std::fs::create_dir_all(&sockets_path).unwrap();
    let sockets = ShortDir(sockets_path);
    let home = Scratch::new("e2e2-b788-home");
    let (home_text, sockets_text) = (home.0.display().to_string(), sockets.0.display().to_string());
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
    let mut d = Driver::start(&sshd, spec(sshd.port), Multiplexer::Tmux).await;
    d.step("connect to a host with no server");
    d.until("the empty state", 30, |d| d.tree().is_some_and(|t| t.no_session)).await;
    d.new_session("x").await;
    d.type_and_see("echo e2e-$((20+22))\r", "e2e-42").await;
    let made = std::process::Command::new("tmux")
        .args(["has-session", "-t", "=x"])
        .env_remove("TMUX")
        .env("TMUX_TMPDIR", &sockets.0)
        .status()
        .unwrap()
        .success();
    d.finish().await;
    let _ = std::process::Command::new("tmux")
        .arg("kill-server")
        .env_remove("TMUX")
        .env("TMUX_TMPDIR", &sockets.0)
        .status();
    assert!(made, "no session x on the host");
}
