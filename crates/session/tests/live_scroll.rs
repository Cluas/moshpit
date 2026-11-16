//! Scrolling on the Rust path against real programs: a private sshd, a
//! private tmux server (`tmux -L`), mosh-server started through it. What a
//! pane's bytes leave in the terminal core's history, and where a scroll
//! tick goes, for each kind of session.
//!
//! Ignored by default (they need sshd, tmux and mosh-server).
//!
//! ```sh
//! cargo test -p moshpit-session --test live_scroll -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::time::Duration;

use moshpit_session::{EventKind, Multiplexer, TmuxAction};
use moshpit_term::{Options, ScrollOnClear, Terminal};
use support::*;

/// A program that turns on what it is told (alternate screen, the mouse,
/// a repaint ten times a second) and writes what it reads to `out`. Run by
/// a copy of bash named `claude`, it stays the pane's foreground process
/// (tmux's `#{pane_current_command}`): it doesn't `exec` its reader.
fn fake_app(dir: &Scratch) -> std::path::PathBuf {
    let path = dir.join("app.sh");
    std::fs::write(
        &path,
        "alt=$1; mouse=$2; spin=$3; out=$4\n\
         [ \"$alt\" = 1 ] && printf '\\033[?1049h'\n\
         [ \"$mouse\" = 1 ] && printf '\\033[?1000h\\033[?1006h'\n\
         printf 'FAKE-APP alt=%s mouse=%s\\r\\n' \"$alt\" \"$mouse\"\n\
         stty raw -echo\n\
         if [ \"$spin\" = 1 ]; then (while :; do printf '\\rspin %s ' \"$(date +%S)\"; sleep 0.1; done) & fi\n\
         cat > \"$out\"\n",
    )
    .unwrap();
    path
}

async fn settle(bench: &mut Bench, ms: u64) {
    let until = tokio::time::Instant::now() + Duration::from_millis(ms);
    while tokio::time::Instant::now() < until {
        if bench.next(1).await.is_none() {
            break;
        }
    }
    bench.drain();
}

/// A pane's bytes, fed to the terminal the app draws a tmux pane with.
fn replay(bytes: &[u8], cols: u16, rows: u16) -> Terminal {
    let mut term = Terminal::new(
        cols,
        rows,
        Options {
            scroll_on_clear: ScrollOnClear::Tmux,
            ..Options::default()
        },
    );
    term.feed(bytes);
    term.flush_sync();
    term
}

/// The terminal's history, trailing blanks trimmed.
fn history_of(term: &Terminal) -> Vec<String> {
    let history = term.history_len() as i32;
    (-history..0).map(|line| term.row_text(line).0).collect()
}

/// B364: a shell pane's local history is tmux's own, line for line, after
/// the attach and whatever repaints follow — a frame after each Enter, a
/// refresh, the return from the background. Each repaint used to push one
/// more copy of the screen into it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd and tmux"]
async fn ssh_tmux_local_history_is_tmuxs() {
    let sshd = Sshd::start().await;
    let tmux = Tmux::named("scroll-sc");
    tmux.new_session("sc");
    tmux.run(&["send-keys", "-t", "sc", "seq 1 300", "Enter"]);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mut bench = Bench::with(sshd.addr(), moshpit_session::Timing::default());
    let mut spec = spec(sshd.port);
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let (attach, panes) = bench.panes(id, 20).await;
    bench
        .until(id, 20, |kind| match kind {
            EventKind::FirstFrame { attach: a } if *a == attach => Some(()),
            _ => None,
        })
        .await;
    let pane = panes.iter().find(|p| p.active).unwrap_or(&panes[0]).clone();
    let mut term = Term::attach(pane.stream);
    term.resize(100, 30);
    settle(&mut bench, 1500).await;
    for i in 0..3 {
        term.write(format!("echo after-{i}-$((40+2))\r").as_bytes());
        // What the view says with every Enter: a repaint once the
        // program's redraw has settled.
        bench
            .sessions
            .tmux(
                id,
                TmuxAction::ReturnSent {
                    pane: pane.pane.clone(),
                },
            )
            .unwrap();
        settle(&mut bench, 1200).await;
    }
    bench.sessions.tmux(id, TmuxAction::Refresh).unwrap();
    settle(&mut bench, 1500).await;
    bench.sessions.set_foreground(false);
    let _ = bench.sessions.flushed().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    bench.sessions.set_foreground(true);
    settle(&mut bench, 2500).await;
    term.read();
    let local = history_of(&replay(&term.screen, 100, 30));
    // tmux's history: everything above its screen.
    let theirs: Vec<String> = tmux
        .run(&[
            "capture-pane",
            "-p",
            "-S",
            "-",
            "-E",
            "-1",
            "-t",
            &pane.pane,
        ])
        .lines()
        .map(|line| line.trim_end().to_owned())
        .collect();
    assert!(
        theirs.len() > 200,
        "tmux kept the history: {}",
        theirs.len()
    );
    assert_eq!(local, theirs, "the pane's local history is tmux's");
    let _ = bench.sessions.disconnect(id).await;
}

/// B363: a full-screen program already running when the phone attaches
/// (what every reconnect meets) gets the first tick as its wheel — before
/// any drag's probe, and for Claude's full-screen renderer on the primary
/// screen even while it repaints. The flags used to be unread after every
/// attach, so the first ticks paged the pane's local history (old frames);
/// a repainting primary-screen Claude stayed parked there for good.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd and tmux"]
async fn ssh_tmux_a_full_screen_program_across_the_attach_gets_the_first_tick() {
    let sshd = Sshd::start().await;
    let dir = Scratch::new("scroll-across");
    let app = fake_app(&dir);
    let claude = dir.join("claude");
    std::fs::copy("/bin/bash", &claude).unwrap();
    for (label, alt, spin) in [
        ("alt-screen mouse app", 1, 0),
        ("claude on the primary screen, repainting", 0, 1),
    ] {
        let tmux = Tmux::named("scroll-scx");
        tmux.new_session("scx");
        tmux.run(&["send-keys", "-t", "scx", "seq 1 300", "Enter"]);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let out = dir.join("wheel.in");
        let _ = std::fs::remove_file(&out);
        tmux.run(&[
            "respawn-pane",
            "-k",
            "-t",
            "scx",
            &format!(
                "{} {} {alt} 1 {spin} {}",
                claude.display(),
                app.display(),
                out.display()
            ),
        ]);
        tokio::time::sleep(Duration::from_millis(800)).await;
        let mut bench = Bench::with(sshd.addr(), moshpit_session::Timing::default());
        let mut spec = spec(sshd.port);
        spec.multiplexer = Multiplexer::Tmux;
        spec.tmux_path = Some(tmux.prefix());
        let id = bench.sessions.connect(spec);
        bench.sessions.set_visible(Some(id));
        bench.connected(id).await;
        let (attach, panes) = bench.panes(id, 20).await;
        bench
            .until(id, 20, |kind| match kind {
                EventKind::FirstFrame { attach: a } if *a == attach => Some(()),
                _ => None,
            })
            .await;
        let pane = panes.iter().find(|p| p.active).unwrap_or(&panes[0]).clone();
        let term = Term::attach(pane.stream);
        term.resize(100, 30);
        settle(&mut bench, 1200).await;
        let from = bench.seen.len();
        // The first tick, as a swipe's would be: no ScrollBegan first.
        bench.sessions.tmux(id, TmuxAction::ScrollLines(3)).unwrap();
        settle(&mut bench, 800).await;
        let local = bench.seen[from..].iter().any(|e| {
            matches!(
                e.kind,
                EventKind::View {
                    action: moshpit_tmux::controller::ViewAction::ScrollLocal { .. },
                    ..
                }
            )
        });
        let wheel = std::fs::read(&out).unwrap_or_default();
        assert!(!local, "{label}: the first tick paged the local history");
        assert_eq!(
            wheel,
            b"\x1b[<64;50;15M".repeat(3),
            "{label}: the first tick is the program's wheel"
        );
        let _ = bench.sessions.disconnect(id).await;
    }
}

/// B362: on mosh + tmux, a scroll over a shell pane pages tmux's history in
/// copy-mode on the client mosh renders — the screen moves — and the next
/// line typed leaves copy-mode and runs in the shell; over a mouse app it
/// is the program's wheel. On the core a shell's scroll did nothing: the
/// sidecar's local route had no view, the swipe only a local buffer mosh
/// never fills.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd, tmux and mosh-server"]
async fn mosh_tmux_a_scroll_pages_tmuxs_history_and_typing_leaves_it() {
    let sshd = Sshd::start().await;
    let tmux = Tmux::named("scroll-scm");
    tmux.new_session("scm");
    tmux.run(&["send-keys", "-t", "scm", "seq 1 300", "Enter"]);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mut bench = Bench::with(sshd.addr(), moshpit_session::Timing::default());
    let mut spec = spec(sshd.port);
    spec.protocol = moshpit_session::Protocol::Mosh;
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let _reaper = Reaper(bench.sessions.view(id).unwrap().mosh_server_pid);
    let (_, panes) = bench.panes(id, 20).await;
    let mut term = Term::attach(panes[0].stream);
    term.resize(100, 30);
    for _ in 0..40 {
        if bench
            .sessions
            .tmux_snapshot(id)
            .is_some_and(|s| s.is_attached)
        {
            break;
        }
        settle(&mut bench, 250).await;
    }
    settle(&mut bench, 1500).await;
    let pane = tmux.run(&["display-message", "-p", "-t", "scm", "#{pane_id}"]);
    let in_mode =
        |tmux: &Tmux| tmux.run(&["display-message", "-p", "-t", &pane, "#{pane_in_mode}"]);

    term.read();
    let mark = term.screen.len();
    bench.sessions.tmux(id, TmuxAction::ScrollBegan).unwrap();
    settle(&mut bench, 600).await;
    bench.sessions.tmux(id, TmuxAction::ScrollLines(5)).unwrap();
    settle(&mut bench, 1200).await;
    term.read();
    assert_eq!(in_mode(&tmux), "1", "the shell's pane is in copy-mode");
    assert!(term.screen.len() > mark, "the screen moved");
    // Paging down, then a line typed: copy-mode is left first, the line
    // runs in the shell.
    bench
        .sessions
        .tmux(id, TmuxAction::ScrollLines(-5))
        .unwrap();
    settle(&mut bench, 400).await;
    term.write(b"echo r$((40+2))\r");
    settle(&mut bench, 1500).await;
    assert_eq!(in_mode(&tmux), "0", "out of copy-mode");
    let captured = tmux.run(&["capture-pane", "-p", "-t", &pane]);
    assert!(captured.contains("r42"), "the line ran: {captured}");

    // A mouse app: the wheel, not copy-mode.
    let dir = Scratch::new("scroll-mosh");
    let app = fake_app(&dir);
    let out = dir.join("wheel.in");
    term.write(format!("sh {} 1 1 0 {}\r", app.display(), out.display()).as_bytes());
    settle(&mut bench, 1500).await;
    bench.sessions.tmux(id, TmuxAction::ScrollBegan).unwrap();
    settle(&mut bench, 800).await;
    bench.sessions.tmux(id, TmuxAction::ScrollLines(3)).unwrap();
    settle(&mut bench, 1000).await;
    assert_eq!(in_mode(&tmux), "0", "no copy-mode over a mouse app");
    let wheel = std::fs::read(&out).unwrap_or_default();
    assert!(
        wheel.starts_with(b"\x1b[<64;"),
        "the program's wheel: {:?}",
        String::from_utf8_lossy(&wheel)
    );
    let _ = bench.sessions.disconnect(id).await;
}

/// Connects over mosh to `tmux`'s server, the screen at 100×30, and waits
/// until the sidecar is attached and has settled.
async fn mosh_tmux(
    bench: &mut Bench,
    port: u16,
    tmux: &Tmux,
) -> (moshpit_session::SessionId, Term, Reaper) {
    let mut spec = spec(port);
    spec.protocol = moshpit_session::Protocol::Mosh;
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let reaper = Reaper(bench.sessions.view(id).unwrap().mosh_server_pid);
    let (_, panes) = bench.panes(id, 20).await;
    let term = Term::attach(panes[0].stream);
    term.resize(100, 30);
    for _ in 0..40 {
        if bench
            .sessions
            .tmux_snapshot(id)
            .is_some_and(|s| s.is_attached)
        {
            break;
        }
        settle(bench, 250).await;
    }
    settle(bench, 1500).await;
    (id, term, reaper)
}

/// B362, a mouse app on mosh + tmux: its wheel goes over mosh, out of any
/// copy-mode first. The sidecar read the flags only as a drag began, so a
/// program already running at the attach got its first ticks as copy-mode;
/// and once a tick had entered copy-mode (a program started after the
/// flags were read, the drag's first tick beating their answer), the
/// sidecar's wheel (`send-keys`) went into copy-mode, never the program.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd, tmux and mosh-server"]
async fn mosh_tmux_a_mouse_app_gets_its_wheel_not_copy_mode() {
    let sshd = Sshd::start().await;
    let dir = Scratch::new("wheel-mosh");
    let app = fake_app(&dir);
    let wheel_of = |out: &std::path::Path| std::fs::read(out).unwrap_or_default();

    // Running at the attach: the first tick, no drag's probe first.
    let tmux = Tmux::named("scroll-whm");
    tmux.new_session("whm");
    let out = dir.join("at-attach.in");
    tmux.run(&[
        "respawn-pane",
        "-k",
        "-t",
        "whm",
        &format!("sh {} 1 1 0 {}", app.display(), out.display()),
    ]);
    tokio::time::sleep(Duration::from_millis(800)).await;
    let pane = tmux.run(&["display-message", "-p", "-t", "whm", "#{pane_id}"]);
    let in_mode = |tmux: &Tmux, pane: &str| {
        tmux.run(&["display-message", "-p", "-t", pane, "#{pane_in_mode}"])
    };
    let mut bench = Bench::with(sshd.addr(), moshpit_session::Timing::default());
    let (id, _term, _reaper) = mosh_tmux(&mut bench, sshd.port, &tmux).await;
    bench.sessions.tmux(id, TmuxAction::ScrollLines(3)).unwrap();
    settle(&mut bench, 1000).await;
    assert_eq!(in_mode(&tmux, &pane), "0", "at the attach: no copy-mode");
    assert!(
        wheel_of(&out).starts_with(b"\x1b[<64;"),
        "at the attach: the program's wheel: {:?}",
        String::from_utf8_lossy(&wheel_of(&out))
    );
    let _ = bench.sessions.disconnect(id).await;
    drop(tmux);

    // Started after the attach: the first tick is copy-mode (the flags
    // say shell), the drag's probe answers, the next tick leaves it.
    let tmux = Tmux::named("scroll-whm2");
    tmux.new_session("whm2");
    let pane = tmux.run(&["display-message", "-p", "-t", "whm2", "#{pane_id}"]);
    let mut bench = Bench::with(sshd.addr(), moshpit_session::Timing::default());
    let (id, term, _reaper) = mosh_tmux(&mut bench, sshd.port, &tmux).await;
    let out = dir.join("after-attach.in");
    term.write(format!("sh {} 1 1 0 {}\r", app.display(), out.display()).as_bytes());
    settle(&mut bench, 1500).await;
    bench.sessions.tmux(id, TmuxAction::ScrollLines(3)).unwrap();
    settle(&mut bench, 600).await;
    assert_eq!(
        in_mode(&tmux, &pane),
        "1",
        "the raced tick entered copy-mode"
    );
    bench.sessions.tmux(id, TmuxAction::ScrollBegan).unwrap();
    settle(&mut bench, 800).await;
    bench.sessions.tmux(id, TmuxAction::ScrollLines(3)).unwrap();
    settle(&mut bench, 1000).await;
    assert_eq!(in_mode(&tmux, &pane), "0", "the wheel left copy-mode");
    assert!(
        wheel_of(&out).starts_with(b"\x1b[<64;"),
        "the program's wheel, nothing of copy-mode's `q`: {:?}",
        String::from_utf8_lossy(&wheel_of(&out))
    );
    let _ = bench.sessions.disconnect(id).await;
}

/// B614, mosh + tmux: after a scroll's copy-mode, the person's own `q`
/// leaves it — and nothing more reaches the shell. The mosh loop's latch
/// put its own `q` in front: `qq`, the second one typed into the shell (a
/// `qecho …` here; Claude's prompt or a vim macro in life). Then a scroll
/// in one window and a switch to another: the first line typed there runs
/// as typed (it got a `q` in front), and the first window is still in
/// copy-mode, as tmux keeps it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs sshd, tmux and mosh-server"]
async fn mosh_tmux_the_persons_own_q_leaves_copy_mode_and_nothing_more() {
    let sshd = Sshd::start().await;
    let tmux = Tmux::named("scroll-qq");
    tmux.new_session("qq");
    tmux.run(&["send-keys", "-t", "qq", "seq 1 300", "Enter"]);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mut bench = Bench::with(sshd.addr(), moshpit_session::Timing::default());
    let (id, term, _reaper) = mosh_tmux(&mut bench, sshd.port, &tmux).await;
    let pane = tmux.run(&["display-message", "-p", "-t", "qq", "#{pane_id}"]);
    let in_mode = |tmux: &Tmux, pane: &str| {
        tmux.run(&["display-message", "-p", "-t", pane, "#{pane_in_mode}"])
    };

    bench.sessions.tmux(id, TmuxAction::ScrollBegan).unwrap();
    settle(&mut bench, 600).await;
    bench.sessions.tmux(id, TmuxAction::ScrollLines(5)).unwrap();
    settle(&mut bench, 1500).await;
    assert_eq!(in_mode(&tmux, &pane), "1", "in copy-mode");
    term.write(b"q");
    settle(&mut bench, 1000).await;
    assert_eq!(in_mode(&tmux, &pane), "0", "the q left copy-mode");
    term.write(b"echo r$((40+2))\r");
    settle(&mut bench, 1500).await;
    let captured = tmux.run(&["capture-pane", "-p", "-t", &pane]);
    assert!(
        captured.contains("r42") && !captured.contains("qecho"),
        "the line ran as typed: {captured}"
    );

    // A scroll in this window, then another window.
    tmux.run(&["new-window", "-d", "-t", "qq"]);
    settle(&mut bench, 1500).await;
    bench.sessions.tmux(id, TmuxAction::ScrollBegan).unwrap();
    settle(&mut bench, 600).await;
    bench.sessions.tmux(id, TmuxAction::ScrollLines(5)).unwrap();
    settle(&mut bench, 1500).await;
    assert_eq!(in_mode(&tmux, &pane), "1", "in copy-mode again");
    let other = tmux.run(&["display-message", "-p", "-t", "qq:1", "#{window_id}"]);
    let other_pane = tmux.run(&["display-message", "-p", "-t", "qq:1", "#{pane_id}"]);
    bench
        .sessions
        .tmux(id, TmuxAction::SelectWindow(other.clone()))
        .unwrap();
    settle(&mut bench, 2000).await;
    term.write(b"echo s$((40+3))\r");
    settle(&mut bench, 1500).await;
    let there = tmux.run(&["capture-pane", "-p", "-t", &other_pane]);
    assert!(
        there.contains("s43") && !there.contains("qecho"),
        "the line ran as typed in the other window: {there}"
    );
    assert_eq!(
        in_mode(&tmux, &pane),
        "1",
        "the first window keeps its copy-mode"
    );
    let _ = bench.sessions.disconnect(id).await;
}
