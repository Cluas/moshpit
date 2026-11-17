//! Round thirty-four against real programs on this machine's loopback,
//! private every one: an sshd of our own (temp keys and config, key auth
//! only), a tmux 3.6a server on a private socket (`-L moshpit-r34-…`,
//! `-f /dev/null`), the mosh-servers the sessions start (stopped by pid),
//! and — where a desktop is wanted — a tmux client of that private server on
//! a pty of our own at a desktop's size (a python3 `pty.fork`, killed when
//! dropped).
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_r34 -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use moshpit_session::{Multiplexer, Protocol, SessionId, TmuxAction};
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

/// The window's size and layout, as a control client reads it: tmux 3.8
/// writes a plain client's layout as JSON, the active pane in it (B776).
fn layout(tmux: &Tmux, window: &str) -> String {
    tmux.run(&[
        "-C",
        "display-message",
        "-p",
        "-t",
        window,
        "#{window_width}x#{window_height} #{window_layout}",
    ])
    .lines()
    .find(|line| !line.starts_with('%'))
    .unwrap_or_default()
    .to_owned()
}

/// Whether the sidecar pinned the window (`resize-window` sets its
/// `window-size` to manual): it knows the renderer and its window.
fn pinned(tmux: &Tmux, window: &str) -> bool {
    tmux.run(&["show-options", "-wv", "-t", window, "window-size"]) == "manual"
}

fn session_id(tmux: &Tmux, name: &str) -> String {
    tmux.run(&["list-sessions", "-F", "#{session_name} #{session_id}"])
        .lines()
        .find_map(|row| row.strip_prefix(&format!("{name} ")).map(str::to_owned))
        .unwrap_or_default()
}

/// A session at a desktop's size, 200×60, with nobody attached: its first
/// window split in four, unevenly, its second in three. Uneven splits, so a
/// round trip through the phone's grid can't land back on them by chance.
fn desktop_session(tmux: &Tmux, name: &str) {
    let out = tmux
        .command()
        .args([
            "new-session",
            "-d",
            "-s",
            name,
            "-x",
            "200",
            "-y",
            "60",
            "/bin/sh",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let w0 = format!("{name}:0");
    let w1 = format!("{name}:1");
    tmux.run(&["split-window", "-v", "-l", "13", "-t", &w0, "/bin/sh"]);
    tmux.run(&["split-window", "-h", "-l", "37", "-t", &w0, "/bin/sh"]);
    tmux.run(&["select-pane", "-t", &format!("{w0}.0")]);
    tmux.run(&["split-window", "-h", "-l", "71", "-t", &w0, "/bin/sh"]);
    tmux.run(&["new-window", "-t", name, "/bin/sh"]);
    tmux.run(&["split-window", "-h", "-l", "43", "-t", &w1, "/bin/sh"]);
    tmux.run(&["split-window", "-v", "-l", "17", "-t", &w1, "/bin/sh"]);
    tmux.run(&["select-window", "-t", &w0]);
}

fn layouts(tmux: &Tmux, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .flat_map(|name| {
            [
                layout(tmux, &format!("{name}:0")),
                layout(tmux, &format!("{name}:1")),
            ]
        })
        .collect()
}

/// A desktop's terminal: a client of the private tmux server on a pty of
/// our own, `cols`×`rows`, attached to `session`. Killed when dropped (the
/// pty's master goes with python, and tmux's client with it).
struct Desktop {
    child: Child,
    pid: Option<i64>,
}

impl Desktop {
    fn attach(tmux: &Tmux, session: &str, cols: u16, rows: u16) -> Desktop {
        const PTY: &str = "import os, pty, sys, fcntl, termios, struct\n\
            cols, rows = int(sys.argv[1]), int(sys.argv[2])\n\
            pid, fd = pty.fork()\n\
            if pid == 0:\n\
            \x20   fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))\n\
            \x20   os.execv(sys.argv[3], sys.argv[3:])\n\
            print(pid, flush=True)\n\
            try:\n\
            \x20   while os.read(fd, 65536):\n\
            \x20       pass\n\
            except OSError:\n\
            \x20   pass\n";
        let target = format!("={session}");
        let child = Command::new("python3")
            .args([
                "-I",
                "-c",
                PTY,
                &cols.to_string(),
                &rows.to_string(),
                &tmux.binary,
                "-L",
                &tmux.socket,
                "-f",
                "/dev/null",
                "attach",
                "-t",
                &target,
            ])
            .env_remove("TMUX")
            .env_remove("TMUX_TMPDIR")
            .env("TERM", "xterm-256color")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("python3");
        let mut desktop = Desktop { child, pid: None };
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let rows = tmux.run(&["list-clients", "-F", "#{client_pid} #{client_session}"]);
            if let Some(pid) = rows
                .lines()
                .find(|row| row.ends_with(&format!(" {session}")))
                .and_then(|row| row.split(' ').next())
                .and_then(|pid| pid.parse().ok())
            {
                desktop.pid = Some(pid);
                return desktop;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(desktop);
        panic!("the desktop never attached");
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(pid) = self.pid {
            kill(pid);
        }
    }
}

/// The non-control client tmux lists that isn't the desktop: the phone's
/// renderer, `(pid, session name)` — the session it views, through the
/// phone's own group of it from tmux 3.8 on (`a~mp-…`, B773, B812).
fn renderer(tmux: &Tmux, desktop: Option<&Desktop>) -> Option<(i64, String)> {
    let rows = tmux.run(&[
        "list-clients",
        "-F",
        "#{client_control_mode} #{client_pid} #{client_session}",
    ]);
    rows.lines().find_map(|row| {
        let mut words = row.splitn(3, ' ');
        let control = words.next()?;
        let pid: i64 = words.next()?.parse().ok()?;
        let session = words.next()?;
        let session = session
            .split(moshpit_tmux::controller::GROUP_MARK)
            .next()?
            .to_owned();
        (control == "0" && desktop.and_then(|d| d.pid) != Some(pid)).then_some((pid, session))
    })
}

/// The phone on mosh + tmux at 60×20.
async fn connect_phone(bench: &mut Bench, sshd: &Sshd, tmux: &Tmux) -> (SessionId, Reaper) {
    connect_over(bench, sshd, tmux, Protocol::Mosh).await
}

/// The phone on `protocol` + tmux at 60×20.
async fn connect_over(
    bench: &mut Bench,
    sshd: &Sshd,
    tmux: &Tmux,
    protocol: Protocol,
) -> (SessionId, Reaper) {
    let mut spec = spec(sshd.port);
    spec.protocol = protocol;
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
    tokio::time::sleep(Duration::from_millis(500)).await;
}

// MARK: 1. The sidecar's session switch squashes the session it lands on

/// Round thirty-four, 1: the phone switches sessions over mosh. The
/// sidecar moves the renderer (mosh's tmux client) to the other session,
/// which tmux then sizes by it — with the phone alone, every window of it at
/// the phone's grid; with a desktop there, its current window — and nothing
/// recorded those windows' layouts first: the disconnect left them squashed
/// (alone), or refitted from the squash at the desktop's size (with a
/// desktop).
async fn sidecar_switch(with_desktop: bool) {
    let sshd = Sshd::start().await;
    let tmux = Tmux::named("moshpit-r34");
    desktop_session(&tmux, "b");
    let desktop = with_desktop.then(|| Desktop::attach(&tmux, "b", 200, 60));
    tokio::time::sleep(Duration::from_millis(1500)).await;
    // The most recent session: where mosh's tmux client attaches.
    desktop_session(&tmux, "a");
    let before = layouts(&tmux, &["a", "b"]);
    let mut bench = Bench::with(sshd.addr(), quick());
    let (id, reaper) = connect_phone(&mut bench, &sshd, &tmux).await;
    assert!(
        eventually(15, || renderer(&tmux, desktop.as_ref())
            .is_some_and(|(_, session)| session == "a"))
        .await,
        "the renderer isn't on a: {:?}\n{}\n{}",
        renderer(&tmux, desktop.as_ref()),
        tmux.run(&[
            "list-clients",
            "-F",
            "#{client_control_mode} #{client_pid} #{client_session}"
        ]),
        tmux.run(&[
            "list-sessions",
            "-F",
            "#{session_id} #{session_name} #{@moshpit_group_of}"
        ])
    );
    assert!(
        eventually(15, || pinned(&tmux, "a:0")).await,
        "never pinned: {}",
        layout(&tmux, "a:0")
    );
    let b = session_id(&tmux, "b");
    bench
        .sessions
        .tmux(id, TmuxAction::SelectSession(b))
        .unwrap();
    assert!(
        eventually(15, || renderer(&tmux, desktop.as_ref())
            .is_some_and(|(_, session)| session == "b"))
        .await,
        "the renderer never moved: {:?} {:?} {:?}",
        tmux.run(&[
            "list-clients",
            "-F",
            "#{client_control_mode} #{client_pid} #{client_name} #{client_session}"
        ]),
        bench.sessions.tmux_snapshot(id).map(|s| (
            s.active_session_id.clone(),
            s.sessions.keys().cloned().collect::<Vec<_>>()
        )),
        session_id(&tmux, "b")
    );
    assert!(
        eventually(15, || layout(&tmux, "b:0").starts_with("60x20 ")).await,
        "{}",
        layout(&tmux, "b:0")
    );
    let squashed = layouts(&tmux, &["a", "b"]);
    // The session the renderer left is nobody's of the phone's: handed back
    // now, not on the disconnect (a desktop attaching to it meanwhile found
    // its window pinned at the phone's grid).
    let left_back = eventually(5, || {
        layouts(&tmux, &["a"]) == before[..2] && !pinned(&tmux, "a:0")
    })
    .await;
    hang_up(&mut bench, id, &reaper).await;
    let after = layouts(&tmux, &["a", "b"]);
    let label = if with_desktop {
        "with a desktop on b"
    } else {
        "the phone alone"
    };
    eprintln!(
        "sidecar session switch, {label}:\n  before   {before:#?}\n  squashed {squashed:#?}\n  after    {after:#?}"
    );
    drop(desktop);
    assert!(
        left_back,
        "{label}: the session left wasn't handed back at the switch"
    );
    assert_eq!(after, before, "{label}: not handed back as they were");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn a_sidecar_session_switch_hands_both_sessions_back_with_the_phone_alone() {
    need!();
    sidecar_switch(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server, tmux and python3; see the module docs"]
async fn a_sidecar_session_switch_hands_both_sessions_back_with_a_desktop() {
    need!();
    sidecar_switch(true).await;
}

// MARK: 2. The renderer identified late

/// A logger that keeps the core's lines, with when they came, for a run
/// that goes wrong.
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

/// Round thirty-four, 2: about one live connect in five, the sidecar still
/// hadn't identified the phone's renderer ~2 s after the session was up,
/// so no window was pinned (and a session switch would move no client).
/// `R34_CONNECTS` connects (default 30), each disconnected before the next;
/// a window is "pinned" when its `window-size` is manual — only the
/// sidecar's pin sets it, and only once it knows the renderer's window.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn the_renderer_is_identified_on_every_connect() {
    need!();
    let connects: usize = std::env::var("R34_CONNECTS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(30);
    let kept = kept();
    let sshd = Sshd::start().await;
    let tmux = Tmux::named("moshpit-r34");
    desktop_session(&tmux, "a");
    let mut late = Vec::new();
    let mut never = Vec::new();
    let mut times = Vec::new();
    for round in 0..connects {
        kept.lines.lock().unwrap().clear();
        let mut bench = Bench::with(sshd.addr(), quick());
        let (id, reaper) = connect_phone(&mut bench, &sshd, &tmux).await;
        let up = Instant::now();
        let in_time = eventually(2, || pinned(&tmux, "a:0")).await;
        let at_all = in_time || eventually(10, || pinned(&tmux, "a:0")).await;
        let took = up.elapsed().as_millis();
        if !in_time {
            late.push(round);
            eprintln!(
                "connect {round}: not pinned 2 s after it was up (at all: {at_all}, {took} ms)\n  clients: {:?}\n  log:\n    {}",
                tmux.run(&[
                    "list-clients",
                    "-F",
                    "#{client_control_mode} #{client_pid} #{client_name} #{client_session}"
                ]),
                kept.lines.lock().unwrap().join("\n    ")
            );
        } else {
            times.push(took);
        }
        if !at_all {
            never.push(round);
        }
        hang_up(&mut bench, id, &reaper).await;
        // The window back to automatic for the next connect.
        let _ = tmux.run(&["set-option", "-u", "-w", "-t", "a:0", "window-size"]);
    }
    times.sort_unstable();
    eprintln!(
        "{connects} connects: pinned within 2 s {}, late {late:?}, never {never:?}; p50 {} ms, max {} ms",
        times.len(),
        times.get(times.len() / 2).copied().unwrap_or(0),
        times.last().copied().unwrap_or(0)
    );
    assert!(late.is_empty(), "late: {late:?}, never: {never:?}");
}

// MARK: 5. Sessions Moshpit made, known from the server (B86)

fn session_status(tmux: &Tmux, name: &str) -> String {
    tmux.run(&["show-options", "-t", &format!("{name}:"), "-v", "status"])
}

/// The status line on what the phone shows of `name`: the session's own,
/// or from tmux 3.8 its group's (`name~mp-…`, B773), off when either is.
fn viewed_status(tmux: &Tmux, name: &str) -> String {
    let group = format!("{name}{}", moshpit_tmux::controller::GROUP_MARK);
    tmux.run(&["list-sessions", "-F", "#{session_name}"])
        .lines()
        .filter(|session| session.starts_with(&group))
        .map(|session| session_status(tmux, session))
        .find(|status| status == "off")
        .unwrap_or_else(|| session_status(tmux, name))
}

/// B86: the phone over mosh makes a session ("mine"); the sidecar turns its
/// status line off while the phone is on it. The app is killed there: none
/// of its code runs again (its runtime stopped, its connections cut, its
/// mosh-server gone). Next launch, the phone lands back on "mine" and then
/// goes to the user's own session: the old sidecar knew Moshpit's sessions
/// only in memory, so "mine" kept its status line off for good.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn a_killed_app_does_not_leave_its_sessions_status_line_off() {
    need!();
    let tmux = Tmux::named("moshpit-r34");
    tmux.new_session("work");
    let first_sshd = Sshd::start().await;
    let (addr, port, prefix) = (first_sshd.addr(), first_sshd.port, tmux.prefix());
    // The first launch, on a runtime of its own.
    let first = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let server = runtime.block_on(async move {
            let mut bench = Bench::with(addr, quick());
            let mut spec = spec(port);
            spec.protocol = Protocol::Mosh;
            spec.multiplexer = Multiplexer::Tmux;
            spec.tmux_path = Some(prefix);
            spec.initial_grid = (60, 20);
            let id = bench.sessions.connect(spec);
            bench.sessions.set_visible(Some(id));
            bench.connected(id).await;
            let server = bench.sessions.view(id).unwrap().mosh_server_pid;
            // Wait for the sidecar to be up, then make "mine".
            tokio::time::sleep(Duration::from_secs(2)).await;
            bench
                .sessions
                .tmux(
                    id,
                    TmuxAction::NewSession {
                        name: Some("mine".into()),
                    },
                )
                .unwrap();
            // Killed here: nothing of this launch runs again.
            std::mem::forget(bench);
            server
        });
        (runtime, server)
    });
    let (runtime, server) = first.join().unwrap();
    let _reaper = Reaper(server);
    assert!(
        eventually(15, || viewed_status(&tmux, "mine") == "off").await,
        "the first launch never turned mine's status line off: {:?}",
        viewed_status(&tmux, "mine")
    );
    let marks = tmux.run(&[
        "list-sessions",
        "-F",
        "#{session_name} owned=#{@moshpit_owned} hidden=#{@moshpit_status_hidden}",
    ]);
    runtime.shutdown_background();
    if let Some(pid) = server {
        kill(pid);
    }
    drop(first_sshd);
    assert!(
        eventually(15, || tmux
            .run(&["list-clients", "-F", "#{client_name}"])
            .is_empty())
        .await,
        "the killed app's clients are still attached: {}",
        tmux.run(&["list-clients", "-F", "#{client_name}"])
    );
    let left = session_status(&tmux, "mine");
    // The next launch.
    let sshd = Sshd::start().await;
    let mut bench = Bench::with(sshd.addr(), quick());
    let (id, reaper) = connect_phone(&mut bench, &sshd, &tmux).await;
    assert!(
        eventually(15, || renderer(&tmux, None).is_some()).await,
        "no renderer"
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let landed = renderer(&tmux, None).map(|(_, session)| session);
    let work = session_id(&tmux, "work");
    bench
        .sessions
        .tmux(id, TmuxAction::SelectSession(work))
        .unwrap();
    let back = eventually(15, || session_status(&tmux, "mine").is_empty()).await;
    let after = session_status(&tmux, "mine");
    hang_up(&mut bench, id, &reaper).await;
    eprintln!(
        "B86: marks after the first launch {marks:?}; mine's status after the kill {left:?}; \
         the next launch landed on {landed:?}; mine's status on work {after:?}"
    );
    assert!(back, "mine's status line stayed off: {after:?}");
}

// MARK: 3. Only over a layout of ours (B270's trade-off)

/// The window's panes as `left,top,width,height` each.
fn panes(tmux: &Tmux, window: &str) -> Vec<String> {
    tmux.run(&[
        "list-panes",
        "-t",
        window,
        "-F",
        "#{pane_left},#{pane_top},#{pane_width},#{pane_height}",
    ])
    .lines()
    .map(str::to_owned)
    .collect()
}

/// Round thirty-four, 3: a desktop client on the session, the phone's pin
/// on its window (4 panes, unevenly split). With `rearrange` the desktop
/// stacks the panes (`select-layout even-vertical`, the same count) while
/// the phone is there; then the phone disconnects. The disconnect replayed
/// the layout from before the phone came over the desktop's arrangement.
/// Now the window is handed back as the desktop left it; without a
/// rearrangement, the layout from before comes back byte for byte as ever.
async fn desktop_rearranges(protocol: Protocol, rearrange: bool) {
    let sshd = Sshd::start().await;
    let tmux = Tmux::named("moshpit-r34");
    desktop_session(&tmux, "a");
    let desktop = Desktop::attach(&tmux, "a", 200, 60);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = layout(&tmux, "a:0");
    let mut bench = Bench::with(sshd.addr(), quick());
    let (id, reaper) = connect_over(&mut bench, &sshd, &tmux, protocol).await;
    assert!(
        eventually(15, || pinned(&tmux, "a:0")
            && layout(&tmux, "a:0").starts_with("60x"))
        .await,
        "never pinned: {}",
        layout(&tmux, "a:0")
    );
    let squashed = layout(&tmux, "a:0");
    if rearrange {
        tmux.run(&["select-layout", "-t", "a:0", "even-vertical"]);
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    let rearranged = layout(&tmux, "a:0");
    hang_up(&mut bench, id, &reaper).await;
    assert!(
        eventually(10, || !pinned(&tmux, "a:0")).await,
        "still pinned"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = layout(&tmux, "a:0");
    let stacked = panes(&tmux, "a:0");
    let label = format!(
        "{protocol:?}, {}",
        if rearrange {
            "the desktop rearranged"
        } else {
            "nothing rearranged"
        }
    );
    eprintln!(
        "{label}:\n  before     {before}\n  pinned     {squashed}\n  at the end {rearranged}\n  after      {after}\n  panes      {stacked:?}"
    );
    drop(desktop);
    if rearrange {
        assert_ne!(
            after, before,
            "{label}: the desktop's arrangement was replaced"
        );
        assert!(
            stacked.len() == 4
                && stacked
                    .iter()
                    .all(|pane| pane.starts_with("0,") && pane.split(',').nth(2) == Some("200")),
            "{label}: not the desktop's stack: {stacked:?}"
        );
    } else {
        assert_eq!(after, before, "{label}: not handed back as it was");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, tmux and python3; see the module docs"]
async fn over_ssh_a_desktops_rearrangement_is_left_alone() {
    need!();
    desktop_rearranges(Protocol::Ssh, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, tmux and python3; see the module docs"]
async fn over_ssh_with_a_desktop_the_layout_still_comes_back() {
    need!();
    desktop_rearranges(Protocol::Ssh, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server, tmux and python3; see the module docs"]
async fn over_mosh_a_desktops_rearrangement_is_left_alone() {
    need!();
    desktop_rearranges(Protocol::Mosh, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server, tmux and python3; see the module docs"]
async fn over_mosh_with_a_desktop_the_layout_still_comes_back() {
    need!();
    desktop_rearranges(Protocol::Mosh, false).await;
}
