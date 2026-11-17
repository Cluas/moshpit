//! Round thirty-three against real programs on this machine's loopback,
//! private every one: an sshd of our own (temp keys and config, key auth
//! only) whose sessions get a temp HOME and ZDOTDIR, so the account's login
//! shell — zsh — reads an rc written here and nothing of the user's; a tmux
//! server on a private socket (`-L moshpit-r33-…`, `-f /dev/null`); a herdr
//! 0.9.5 server on a private `--session` under a temp HOME (every herdr
//! command carries it); the mosh-servers the sessions start (stopped by pid).
//!
//! B267: nothing the core starts on the host is a line typed into an
//! interactive login shell that may still be starting. Each path is tried
//! under two such shells:
//!
//! - a 3 s rc that then flushes whatever was typed meanwhile (what
//!   Powerlevel10k's instant prompt does to pending input);
//! - oh-my-zsh with zsh-autosuggestions and zsh-syntax-highlighting: this
//!   machine's own `~/.oh-my-zsh`, read only, its update check off, its
//!   cache and completion dump in the temp HOME.
//!
//! ```sh
//! CARGO_BUILD_JOBS=2 cargo test -p moshpit-session --test live_r33 -- --ignored --nocapture --test-threads 1
//! ```

mod support;

use std::path::{Path, PathBuf};
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

fn ms(since: Instant) -> u128 {
    since.elapsed().as_millis()
}

// MARK: the login shells

/// The account's zsh rc under test.
#[derive(Debug, Clone, Copy)]
enum Rc {
    /// 3 s of start-up, then what was typed meanwhile flushed.
    Flushing,
    /// oh-my-zsh with zsh-autosuggestions and zsh-syntax-highlighting.
    OhMyZsh,
}

impl Rc {
    fn label(self) -> &'static str {
        match self {
            Rc::Flushing => "a 3 s, flushing zsh rc",
            Rc::OhMyZsh => "oh-my-zsh with autosuggestions and syntax highlighting",
        }
    }

    /// The rc's text; `None` when this machine has no `~/.oh-my-zsh`.
    fn text(self) -> Option<String> {
        match self {
            Rc::Flushing => Some(
                "sleep 3\nprint -r -- '[oh-my-zsh] plugins loading'\n\
                 python3 -c 'import termios; termios.tcflush(0, termios.TCIFLUSH)' 2>/dev/null\n\
                 PROMPT='%% '\nunsetopt BEEP\n"
                    .to_owned(),
            ),
            Rc::OhMyZsh => {
                let omz = PathBuf::from(std::env::var("HOME").ok()?).join(".oh-my-zsh");
                if !omz.join("oh-my-zsh.sh").exists() {
                    return None;
                }
                let custom = omz.join("custom/plugins");
                let plugins: Vec<&str> = [
                    "git",
                    "zsh-autosuggestions",
                    "zsh-syntax-highlighting",
                    "zsh-completions",
                ]
                .into_iter()
                .filter(|p| {
                    *p == "git" || custom.join(p).exists() || omz.join("plugins").join(p).exists()
                })
                .collect();
                Some(format!(
                    "export ZSH={}\nDISABLE_AUTO_UPDATE=true\nzstyle ':omz:update' mode disabled\n\
                     ZSH_DISABLE_COMPFIX=true\nZSH_CACHE_DIR=$HOME/.omz-cache\n\
                     ZSH_COMPDUMP=$HOME/.zcompdump\nmkdir -p $ZSH_CACHE_DIR\n\
                     ZSH_THEME=robbyrussell\nplugins=({})\nsource $ZSH/oh-my-zsh.sh\n",
                    omz.display(),
                    plugins.join(" ")
                ))
            }
        }
    }
}

/// How many times each case runs (`R33_ROUNDS`, default 1): a typed line
/// that only sometimes survives is a failure too.
fn rounds() -> usize {
    std::env::var("R33_ROUNDS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
}

/// An sshd whose sessions' HOME and ZDOTDIR are `home`, where `rc` is the
/// zsh rc.
async fn sshd_with(home: &Path, rc: &str) -> Sshd {
    std::fs::write(home.join(".zshenv"), "skip_global_compinit=1\n").unwrap();
    std::fs::write(home.join(".zshrc"), rc).unwrap();
    let home = home.display().to_string();
    Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await
}

// MARK: tmux over SSH

async fn ssh_tmux_under(rc: Rc) {
    let Some(text) = rc.text() else {
        eprintln!("skipped: no ~/.oh-my-zsh");
        return;
    };
    let home = Scratch::new("r33-home");
    let sshd = sshd_with(&home.0, &text).await;
    let tmux = Tmux::named("moshpit-r33");
    tmux.new_session("r33");
    let mut results = Vec::new();
    for round in 0..rounds() {
        let mut bench = Bench::with(sshd.addr(), quick());
        let mut spec = spec(sshd.port);
        spec.multiplexer = Multiplexer::Tmux;
        spec.tmux_path = Some(tmux.prefix());
        // The account's login shell, as in the app.
        spec.pty_command = None;
        let started = Instant::now();
        let id = bench.sessions.connect(spec);
        bench.sessions.set_visible(Some(id));
        bench.connected(id).await;
        let attached = eventually(25, || {
            bench
                .sessions
                .tmux_snapshot(id)
                .is_some_and(|s| s.is_attached)
        })
        .await;
        let attached_ms = ms(started);
        if attached {
            let (_, panes) = bench.panes(id, 10).await;
            let pane = panes.iter().find(|p| p.active).unwrap_or(&panes[0]).clone();
            let mut term = Term::attach(pane.stream);
            term.write(format!("echo r33-{round}-$((6*7))\r").as_bytes());
            assert!(
                term.wait_for(&format!("r33-{round}-42"), 15).await,
                "{}",
                term.text()
            );
        }
        results.push(attached.then_some(attached_ms));
        let _ = bench.sessions.disconnect(id).await;
    }
    eprintln!(
        "tmux over SSH under {}: attached after {results:?} ms",
        rc.label()
    );
    assert!(
        results.iter().all(Option::is_some),
        "tmux over SSH under {}: never attached in some rounds: {results:?} ms",
        rc.label()
    );
}

/// tmux over SSH on the core: `tmux -CC attach` was typed into the login
/// shell the terminal channel opened, and a shell that flushes what was
/// typed while it started never ran it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn ssh_tmux_attaches_under_a_slow_flushing_login_shell() {
    need!();
    ssh_tmux_under(Rc::Flushing).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, tmux and ~/.oh-my-zsh; see the module docs"]
async fn ssh_tmux_attaches_under_oh_my_zsh() {
    need!();
    ssh_tmux_under(Rc::OhMyZsh).await;
}

// MARK: tmux over mosh: the sidecar

async fn mosh_tmux_under(rc: Rc) {
    let Some(text) = rc.text() else {
        eprintln!("skipped: no ~/.oh-my-zsh");
        return;
    };
    let home = Scratch::new("r33-home");
    let sshd = sshd_with(&home.0, &text).await;
    let tmux = Tmux::named("moshpit-r33");
    tmux.new_session("r33");
    let mut results = Vec::new();
    for _ in 0..rounds() {
        let mut bench = Bench::with(sshd.addr(), quick());
        let mut spec = spec(sshd.port);
        spec.protocol = Protocol::Mosh;
        spec.multiplexer = Multiplexer::Tmux;
        spec.tmux_path = Some(tmux.prefix());
        spec.pty_command = None;
        let started = Instant::now();
        let id = bench.sessions.connect(spec);
        bench.sessions.set_visible(Some(id));
        bench.connected(id).await;
        let _reaper = Reaper(bench.sessions.view(id).unwrap().mosh_server_pid);
        // The renderer is mosh-server's program; the tree is the sidecar's.
        let attached = eventually(25, || {
            bench
                .sessions
                .tmux_snapshot(id)
                .is_some_and(|s| s.is_attached && !s.panes.is_empty())
        })
        .await;
        results.push(attached.then_some(ms(started)));
        let _ = bench.sessions.disconnect(id).await;
    }
    eprintln!(
        "tmux over mosh under {}: the sidecar attached after {results:?} ms",
        rc.label()
    );
    assert!(
        results.iter().all(Option::is_some),
        "tmux over mosh under {}: the sidecar never attached in some rounds: {results:?} ms",
        rc.label()
    );
}

/// mosh + tmux on the core: the `-CC` sidecar's attach chain was typed into
/// the login shell its terminal channel opened.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn mosh_tmux_sidecar_attaches_under_a_slow_flushing_login_shell() {
    need!();
    mosh_tmux_under(Rc::Flushing).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server, tmux and ~/.oh-my-zsh; see the module docs"]
async fn mosh_tmux_sidecar_attaches_under_oh_my_zsh() {
    need!();
    mosh_tmux_under(Rc::OhMyZsh).await;
}

// MARK: herdr's push channel

async fn herdr_push_under(rc: Rc, protocol: Protocol) {
    if !PrivateHerdr::available() {
        eprintln!("skipped: no herdr");
        return;
    }
    let Some(text) = rc.text() else {
        eprintln!("skipped: no ~/.oh-my-zsh");
        return;
    };
    let herdr = PrivateHerdr::start();
    std::fs::write(herdr.home.join(".zshrc"), &text).unwrap();
    // The herdr panes' zsh reads the same rc: past its start first.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let home = herdr.home.display().to_string();
    let sshd = Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await;
    let mut results = Vec::new();
    for _ in 0..rounds() {
        let mut bench = Bench::with(sshd.addr(), quick());
        let mut spec = spec(sshd.port);
        spec.protocol = protocol;
        spec.multiplexer = Multiplexer::Herdr;
        spec.herdr_path = Some(herdr.custom_path());
        spec.pty_command = None;
        let started = Instant::now();
        let id = bench.sessions.connect(spec);
        bench.sessions.set_visible(Some(id));
        bench.connected(id).await;
        let _reaper = Reaper(bench.sessions.view(id).and_then(|v| v.mosh_server_pid));
        let active = eventually(25, || {
            bench
                .sessions
                .herdr_view(id)
                .is_some_and(|view| view.push_active)
        })
        .await;
        results.push(active.then_some(ms(started)));
        // And it carries events: a pane made elsewhere reaches the tree.
        if active {
            let before = herdr.panes().len();
            let first = herdr.panes()[0].clone();
            herdr.run(&["pane", "split", &first, "--direction", "right"]);
            assert!(
                eventually(10, || bench
                    .sessions
                    .herdr_view(id)
                    .is_some_and(|view| view.snapshot.panes.len() > before))
                .await,
                "the split never reached the tree"
            );
            let added = herdr.panes().into_iter().find(|p| *p != first).unwrap();
            herdr.run(&["pane", "close", &added]);
        }
        let _ = bench.sessions.disconnect(id).await;
        bench.drain();
    }
    eprintln!(
        "herdr over {protocol:?} under {}: push active after {results:?} ms",
        rc.label()
    );
    assert!(
        results.iter().all(Option::is_some),
        "herdr over {protocol:?} under {}: push never came up in some rounds: {results:?} ms",
        rc.label()
    );
}

/// herdr's push channel on the core: the socket pump's start line was typed
/// into the login shell its terminal channel opened (both transports).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, herdr and python3; see the module docs"]
async fn herdr_push_over_ssh_comes_up_under_a_slow_flushing_login_shell() {
    need!();
    herdr_push_under(Rc::Flushing, Protocol::Ssh).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, herdr, python3 and ~/.oh-my-zsh; see the module docs"]
async fn herdr_push_over_ssh_comes_up_under_oh_my_zsh() {
    need!();
    herdr_push_under(Rc::OhMyZsh, Protocol::Ssh).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server, herdr and python3; see the module docs"]
async fn herdr_push_over_mosh_comes_up_under_a_slow_flushing_login_shell() {
    need!();
    herdr_push_under(Rc::Flushing, Protocol::Mosh).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server, herdr, python3 and ~/.oh-my-zsh; see the module docs"]
async fn herdr_push_over_mosh_comes_up_under_oh_my_zsh() {
    need!();
    herdr_push_under(Rc::OhMyZsh, Protocol::Mosh).await;
}

// MARK: B270: mosh + tmux with the phone as tmux's only client

/// The window's size and layout.
fn layout(tmux: &Tmux, window: &str) -> String {
    tmux.run(&[
        "display-message",
        "-p",
        "-t",
        window,
        "#{window_width}x#{window_height} #{window_layout}",
    ])
}

/// A session at a desktop's size, 200×60, whose first window is split in
/// three and whose second is split in two, with nobody attached: the phone
/// will be its only client.
fn desktop_session(tmux: &Tmux) -> (String, String) {
    let out = tmux
        .command()
        .args([
            "new-session",
            "-d",
            "-s",
            "r33",
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
    tmux.run(&["split-window", "-v", "-t", "r33:0", "/bin/sh"]);
    tmux.run(&["split-window", "-h", "-t", "r33:0", "/bin/sh"]);
    tmux.run(&["new-window", "-t", "r33", "/bin/sh"]);
    tmux.run(&["split-window", "-h", "-t", "r33:1", "/bin/sh"]);
    tmux.run(&["select-window", "-t", "r33:0"]);
    (layout(tmux, "r33:0"), layout(tmux, "r33:1"))
}

/// The phone on mosh, alone on the server, its window pinned to its grid;
/// then `end` takes the session away, and the windows must be back at their
/// size and layout — and stay there once mosh-server and its tmux client
/// are gone.
async fn mosh_tmux_phone_alone(label: &str, end: impl AsyncFnOnce(&mut Bench, u64, &Tmux)) {
    let sshd = Sshd::start().await;
    let tmux = Tmux::named("moshpit-r33");
    let before = desktop_session(&tmux);
    let mut bench = Bench::with(sshd.addr(), quick());
    let mut spec = spec(sshd.port);
    spec.protocol = Protocol::Mosh;
    spec.multiplexer = Multiplexer::Tmux;
    spec.tmux_path = Some(tmux.prefix());
    spec.initial_grid = (60, 20);
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let pid = bench.sessions.view(id).unwrap().mosh_server_pid;
    let _reaper = Reaper(pid);
    let (_, panes) = bench.panes(id, 10).await;
    let term = Term::attach(panes[0].stream);
    term.resize(60, 20);
    assert!(
        eventually(15, || layout(&tmux, "r33:0").starts_with("60x20 ")).await,
        "never pinned to the phone: {}",
        layout(&tmux, "r33:0")
    );
    // The phone looks at the second window too: both are squashed.
    let second = tmux.run(&["display-message", "-p", "-t", "r33:1", "#{window_id}"]);
    bench
        .sessions
        .tmux(id, moshpit_session::TmuxAction::SelectWindow(second))
        .unwrap();
    assert!(
        eventually(15, || layout(&tmux, "r33:1").starts_with("60x20 ")).await,
        "{}",
        layout(&tmux, "r33:1")
    );
    let squashed = (layout(&tmux, "r33:0"), layout(&tmux, "r33:1"));
    end(&mut bench, id, &tmux).await;
    // mosh-server is stopped on the disconnect; its tmux client with it.
    if let Some(pid) = pid {
        assert!(
            eventually(15, || !alive(pid)).await,
            "mosh-server {pid} still runs"
        );
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after = (layout(&tmux, "r33:0"), layout(&tmux, "r33:1"));
    let clients = tmux.run(&["list-clients", "-F", "#{client_name}"]);
    eprintln!(
        "mosh + tmux, the phone alone, {label}:\n  before   {before:?}\n  squashed {squashed:?}\n  after    {after:?}\n  clients left: {clients:?}"
    );
    assert_eq!(
        after, before,
        "{label}: the windows weren't handed back as they were"
    );
}

/// B270: the person disconnects. The sidecar's teardown handed the windows
/// back — sized to their record, the layout replayed, back to automatic
/// sizing — while the phone's renderer (mosh-server's tmux client) was
/// still attached and the only client sizing the session: automatic sizing
/// put them straight back at the phone's grid, and the renderer, stopped
/// right after, left them there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn mosh_tmux_phone_alone_gets_its_windows_back_on_a_disconnect() {
    need!();
    mosh_tmux_phone_alone("disconnected", async |bench, id, _| {
        let _ = bench.sessions.disconnect(id).await;
    })
    .await;
}

/// B270, the dead connection's cleanup: the control plane's connection is
/// gone (its pins owed, the ledger), and the person disconnects: what the
/// ledger owes is paid over a connection of its own while the renderer is
/// still attached.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, mosh-server and tmux; see the module docs"]
async fn mosh_tmux_phone_alone_gets_its_windows_back_after_a_dead_control_plane() {
    need!();
    mosh_tmux_phone_alone(
        "the control plane dead, then disconnected",
        async |bench, id, tmux| {
            // The sidecar's -CC client dies under it (its connection's channel
            // gone): the control plane retires it, owing its pins.
            let sidecar = tmux.run(&["list-clients", "-F", "#{client_control_mode} #{client_pid}"]);
            for line in sidecar.lines() {
                if let Some(pid) = line.strip_prefix("1 ") {
                    kill(pid.parse().unwrap());
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            let _ = bench.sessions.disconnect(id).await;
        },
    )
    .await;
}
