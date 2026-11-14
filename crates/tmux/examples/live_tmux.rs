//! The Rust controller against a real tmux, on a PRIVATE server:
//! `tmux -L moshpit-w8-<pid>-<case> -f /dev/null`, with `/bin/sh` and no
//! user configuration, killed when the case ends (also on failure). Never
//! the default server, never the user's sessions.
//!
//!   live_tmux [<case>...]     (no case: all of them)
//!
//! A control client runs as `tmux -C attach` over pipes (what the app runs
//! over an SSH pty is `-CC`; the protocol lines are the same). The
//! controller is driven in real time: its commands are written to the
//! client's stdin, every byte tmux writes is fed back, its timers are ticked.
//! Each case checks what tmux did, with one-shot `tmux -L … display` reads,
//! and prints its evidence (`harness/controller/live.txt` keeps the last
//! run's).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant};

use moshpit_tmux::controller::{Config, Controller, Notification, Output, ScrollRoute};

const TMUX: &str = "tmux";

/// A private tmux server, killed on drop.
struct Server {
    socket: String,
}

impl Server {
    fn new(case: &str) -> Self {
        // MOSHPIT_LIVE_SOCKET_PREFIX names the servers (default moshpit-w8).
        let prefix =
            std::env::var("MOSHPIT_LIVE_SOCKET_PREFIX").unwrap_or_else(|_| "moshpit-w8".to_owned());
        let server = Self {
            socket: format!("{prefix}-{}-{case}", std::process::id()),
        };
        server.kill();
        server
    }

    fn command(&self) -> Command {
        let mut command = Command::new(TMUX);
        command
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .env_clear()
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
            .env("TERM", "xterm-256color")
            .env("SHELL", "/bin/sh")
            .env("ENV", "/dev/null")
            .env("LANG", "en_US.UTF-8")
            .env("PS1", "$ ");
        command
    }

    /// A teardown command line as the app runs it over an exec channel:
    /// through `sh -c`, single quotes and all.
    fn shell(&self, line: &str) {
        let _ = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("{TMUX} -L {} -f /dev/null {line}", self.socket))
            .env_clear()
            .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
            .output();
    }

    /// One-shot command; its stdout.
    fn run(&self, args: &[&str]) -> String {
        let out = self.command().args(args).output().expect("tmux runs");
        String::from_utf8_lossy(&out.stdout)
            .trim_end_matches('\n')
            .to_owned()
    }

    fn kill(&self) {
        let _ = self.command().arg("kill-server").output();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.kill();
    }
}

/// A controller on a `tmux -C attach` client.
struct Live {
    controller: Controller,
    child: Child,
    stdin: ChildStdin,
    bytes: Receiver<Vec<u8>>,
    start: Instant,
    commands: Vec<String>,
    panes: BTreeMap<String, Vec<u8>>,
    notes: Vec<Notification>,
    raw: Vec<u8>,
}

impl Live {
    fn attach(server: &Server, target: &str, mut config: Config, cols: i64, rows: i64) -> Self {
        // The lease's stamps are wall-clock seconds.
        let unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        config.unix_offset_ms = unix_ms - 1_000_000;
        let mut child = server
            .command()
            .args(["-C", "attach", "-t", target])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("tmux -C attach");
        let stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        thread::spawn(move || {
            let mut buffer = [0u8; 65536];
            while let Ok(n) = stdout.read(&mut buffer) {
                if n == 0 || tx.send(buffer[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut controller = Controller::new(config);
        controller.set_initial_client_size(cols, rows);
        let mut live = Self {
            controller,
            child,
            stdin,
            bytes: rx,
            start: Instant::now(),
            commands: Vec::new(),
            panes: BTreeMap::new(),
            notes: Vec::new(),
            raw: Vec::new(),
        };
        let now = live.now();
        live.controller.begin_control_mode(now);
        live
    }

    fn now(&self) -> u64 {
        1_000_000 + self.start.elapsed().as_millis() as u64
    }

    /// Write what the controller queued; collect the rest.
    fn pump(&mut self) {
        while let Some(output) = self.controller.poll_output() {
            match output {
                Output::Command(command) => {
                    let _ = self.stdin.write_all(format!("{command}\n").as_bytes());
                    self.commands.push(command);
                }
                Output::PaneOutput { pane, bytes, .. } => self
                    .panes
                    .entry(pane)
                    .or_default()
                    .extend_from_slice(&bytes),
                Output::Notify(Notification::StateChanged) => {}
                Output::Notify(note) => self.notes.push(note),
                _ => {}
            }
        }
        let _ = self.stdin.flush();
    }

    /// Run in real time for `ms`.
    fn run(&mut self, ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms);
        loop {
            self.pump();
            let now = Instant::now();
            if now >= until {
                break;
            }
            let wait = (until - now).min(Duration::from_millis(10));
            match self.bytes.recv_timeout(wait) {
                Ok(chunk) => {
                    self.raw.extend_from_slice(&chunk);
                    let now = self.now();
                    self.controller.receive(&chunk, now);
                }
                Err(_) => {
                    let now = self.now();
                    self.controller.tick(now);
                }
            }
        }
        self.pump();
    }

    fn act(&mut self, f: impl FnOnce(&mut Controller, u64)) {
        let now = self.now();
        f(&mut self.controller, now);
        self.pump();
    }

    fn sent(&self, prefix: &str) -> bool {
        self.commands.iter().any(|c| c.starts_with(prefix))
    }

    fn pane_text(&self, pane: &str) -> String {
        String::from_utf8_lossy(self.panes.get(pane).map_or(&[][..], Vec::as_slice)).into_owned()
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

type Outcome = Result<Vec<String>, String>;
type Case = (&'static str, fn() -> Outcome);

/// A client process, killed and reaped on drop.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn check(ok: bool, what: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what.to_owned()) }
}

fn session(server: &Server, size: &str) {
    let (x, y) = size.split_once('x').unwrap();
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "main",
        "-x",
        x,
        "-y",
        y,
        "/bin/sh",
    ]);
}

/// The pane's id of window 0.
fn first_pane(server: &Server) -> String {
    server.run(&["display-message", "-p", "-t", "main:0.0", "#{pane_id}"])
}

// MARK: - Cases

/// Attach, discovery, backfill, the repair frame, the pin and the passive
/// lease claim, against tmux's real replies.
fn attach() -> Outcome {
    let server = Server::new("attach");
    session(&server, "120x40");
    server.run(&["send-keys", "-t", "main", "seq 1 30", "Enter"]);
    thread::sleep(Duration::from_millis(300));
    let mut live = Live::attach(&server, "main", Config::default(), 60, 20);
    live.run(2500);
    let s = live.controller.snapshot();
    let pane = first_pane(&server);
    check(
        s.is_attached && s.active_session_id.as_deref() == Some("$0"),
        "attached on $0",
    )?;
    check(
        s.active_pane_id.as_deref() == Some(pane.as_str()),
        "active pane",
    )?;
    let text = live.pane_text(&pane);
    check(
        text.contains("29\r\n"),
        "the backfill carried the pane's history",
    )?;
    check(text.contains("\u{1b}[H\u{1b}[2J"), "a repair frame landed")?;
    let size = server.run(&[
        "display-message",
        "-p",
        "-t",
        "main",
        "#{window_width}x#{window_height}",
    ]);
    check(
        size == "60x20",
        &format!("window pinned to the client grid, is {size}"),
    )?;
    let lease = server.run(&["show-options", "-wqv", "-t", "main", "@moshpit_size_owner"]);
    let client = live.controller.our_client_name().unwrap_or("").to_owned();
    check(
        lease == format!("{client}|iPhone|0"),
        &format!("passive claim, is {lease:?}"),
    )?;
    check(
        server.run(&["show-options", "-gv", "history-limit"]) == "2000",
        "history-limit left alone",
    )?;
    Ok(vec![
        format!("window {size} after attach at 60x20 (was 120x40)"),
        format!("lease {lease}"),
        format!("{} commands, history-limit untouched", live.commands.len()),
    ])
}

/// A lone arrow goes as `send-keys Up`: tmux encodes it by the pane's real
/// cursor-key mode.
fn arrow_keys() -> Outcome {
    let server = Server::new("keys");
    session(&server, "80x24");
    let mut evidence = Vec::new();
    for (mode, expected) in [("l", "1b5b41"), ("h", "1b4f41")] {
        let out = format!("/tmp/moshpit-w8-{}-keys-{mode}", std::process::id());
        let _ = std::fs::remove_file(&out);
        let program = format!(
            "printf '\\033[?1{mode}'; stty raw -echo; dd bs=1 count=3 2>/dev/null | od -An -tx1 | tr -d ' \\n' > {out}; stty sane"
        );
        server.run(&["send-keys", "-t", "main", &program, "Enter"]);
        thread::sleep(Duration::from_millis(400));
        let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
        live.run(1200);
        let pane = first_pane(&server);
        live.act(|c, now| c.send_input(b"\x1b[A", &pane, now));
        live.run(800);
        let got = std::fs::read_to_string(&out).unwrap_or_default();
        let _ = std::fs::remove_file(&out);
        check(
            live.sent(&format!("send-keys -t {pane} Up")),
            "sent as a key name",
        )?;
        check(
            got == expected,
            &format!("DECCKM {mode}: pane read {got:?}, expected {expected}"),
        )?;
        evidence.push(format!(
            "DECCKM ?1{mode}: ESC[A sent as `send-keys Up`, the pane read {got}"
        ));
    }
    Ok(evidence)
}

/// A paste: markers out of the text, then bracketed once by tmux.
fn paste() -> Outcome {
    let server = Server::new("paste");
    session(&server, "80x24");
    let out = format!("/tmp/moshpit-w8-{}-paste", std::process::id());
    let _ = std::fs::remove_file(&out);
    let program = format!(
        "printf '\\033[?2004h'; stty raw -echo; dd bs=1 count=15 2>/dev/null | od -An -tx1 | tr -d ' \\n' > {out}; stty sane"
    );
    server.run(&["send-keys", "-t", "main", &program, "Enter"]);
    thread::sleep(Duration::from_millis(400));
    let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
    live.run(1200);
    let pane = first_pane(&server);
    live.act(|c, now| c.send_paste("a\u{1b}[201~b\u{1b}[20\u{1b}[200~1~c", &pane, now));
    live.run(800);
    let got = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    // ESC[200~ a b c ESC[201~ : the markers inside are gone, the outer pair
    // is tmux's.
    let expected = "1b5b3230307e6162631b5b3230317e";
    check(got.starts_with(expected), &format!("pane read {got}"))?;
    Ok(vec![format!(
        "pasted `a⟨201~⟩b⟨20⟨200~⟩1~⟩c`: the pane read {got} (one bracket pair, by tmux)"
    )])
}

/// Two Moshpit devices on one window: the last one touched holds it, the
/// other follows; touching the follower takes it back.
fn lease() -> Outcome {
    let server = Server::new("lease");
    session(&server, "120x40");
    let phone = Config {
        size_owner_label: "iPhone".into(),
        ..Config::default()
    };
    let tablet = Config {
        size_owner_label: "iPad".into(),
        ..Config::default()
    };
    let mut a = Live::attach(&server, "main", phone, 60, 20);
    a.run(1500);
    let mut b = Live::attach(&server, "main", tablet, 100, 30);
    // Both run, interleaved, for a while.
    for _ in 0..40 {
        a.run(50);
        b.run(50);
    }
    let size = server.run(&[
        "display-message",
        "-p",
        "-t",
        "main",
        "#{window_width}x#{window_height}",
    ]);
    let owner = server.run(&["show-options", "-wqv", "-t", "main", "@moshpit_size_owner"]);
    let mut evidence = vec![format!(
        "after the iPad attached: window {size}, lease {owner}, phone follows {:?}",
        a.controller.size_follow().map(|f| &f.owner_label)
    )];
    check(
        size == "100x30" && owner.contains("|iPad|"),
        "the last device to attach holds it",
    )?;
    check(
        a.controller
            .size_follow()
            .is_some_and(|f| f.owner_label == "iPad"),
        "the phone follows the iPad",
    )?;
    // The person types on the phone: it takes the window back.
    let pane = first_pane(&server);
    a.act(|c, now| c.send_input(b"x", &pane, now));
    for _ in 0..40 {
        a.run(50);
        b.run(50);
    }
    let size = server.run(&[
        "display-message",
        "-p",
        "-t",
        "main",
        "#{window_width}x#{window_height}",
    ]);
    let owner = server.run(&["show-options", "-wqv", "-t", "main", "@moshpit_size_owner"]);
    evidence.push(format!(
        "after a keystroke on the phone: window {size}, lease {owner}, iPad follows {:?}",
        b.controller.size_follow().map(|f| &f.owner_label)
    ));
    check(
        size == "60x20" && owner.contains("|iPhone|"),
        "the last device touched holds it",
    )?;
    check(!owner.ends_with("|0"), "an interaction stamps the lease")?;
    check(
        b.controller
            .size_follow()
            .is_some_and(|f| f.owner_label == "iPhone"),
        "the iPad follows",
    )?;
    let wars = a
        .commands
        .iter()
        .filter(|c| c.starts_with("resize-window"))
        .count()
        + b.commands
            .iter()
            .filter(|c| c.starts_with("resize-window"))
            .count();
    evidence.push(format!("{wars} resize-window in all: no war"));
    if wars > 8 {
        for (who, live) in [("iPhone", &a), ("iPad", &b)] {
            for command in &live.commands {
                if command.starts_with("resize-window") || command.starts_with("refresh-client") {
                    evidence.push(format!("{who} {command}"));
                }
            }
        }
        return Err(format!("no resize war: {evidence:#?}"));
    }
    Ok(evidence)
}

/// A desktop's terminal on the session (a control client sized like one).
fn desktop(server: &Server, size: &str) -> Reaped {
    let mut child = server
        .command()
        .args(["-C", "attach", "-t", "main"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(format!("refresh-client -C {size}\n").as_bytes());
    // Keep stdin open (the client lives as long as it does).
    std::mem::forget(stdin);
    thread::sleep(Duration::from_millis(300));
    Reaped(child)
}

/// Pinning a split window squashes its layout; handing it back restores it
/// byte for byte (with a desktop attached), and the zoom came and went.
fn layout() -> Outcome {
    let server = Server::new("layout");
    session(&server, "200x60");
    server.run(&["split-window", "-v", "-t", "main"]);
    server.run(&["split-window", "-h", "-t", "main"]);
    let before = server.run(&["display-message", "-p", "-t", "main", "#{window_layout}"]);
    let desk = desktop(&server, "200x60");
    let mut live = Live::attach(&server, "main", Config::default(), 50, 20);
    live.run(2500);
    let pinned = server.run(&[
        "display-message",
        "-p",
        "-t",
        "main",
        "#{window_layout} #{window_zoomed_flag}",
    ]);
    check(
        pinned.ends_with(" 1") && pinned.contains(",50x20,"),
        &format!("pinned and zoomed: {pinned}"),
    )?;
    live.act(|c, now| c.release_window_pins(now));
    live.run(800);
    let after = server.run(&[
        "display-message",
        "-p",
        "-t",
        "main",
        "#{window_layout} #{window_zoomed_flag}",
    ]);
    drop(desk);
    check(
        after == format!("{before} 0"),
        &format!("restored: {after} vs {before}"),
    )?;
    Ok(vec![
        format!("before {before}"),
        format!("pinned + zoomed {pinned}"),
        format!("released with a 200x60 desktop attached {after}"),
    ])
}

/// The same window with NO other client attached (B150). With the phone as
/// tmux's only (so latest) client, the attach's own `refresh-client -C`
/// resizes the session's windows to the phone grid before discovery, and
/// tmux 3.6a ignores an `ignore-size` client only while another attached
/// client is not flagged. The layouts are read on both sides of that first
/// size report, and the hand-back reports the recorded size as ours: the
/// split window and the session's other window come back as they were.
fn layout_without_desktop() -> Outcome {
    let server = Server::new("layout-alone");
    let (before, other_before) = alone_session(&server);
    let mut live = Live::attach(&server, "main", Config::default(), 50, 20);
    live.run(2500);
    let said = String::from_utf8_lossy(&live.raw).into_owned();
    let first_change = said
        .lines()
        .find(|l| l.starts_with("%layout-change"))
        .unwrap_or("")
        .to_owned();
    check(
        first_change.contains(",50x20,"),
        "the attach's refresh-client -C squashed it",
    )?;
    live.act(|c, now| c.release_window_pins(now));
    live.run(800);
    let after = server.run(&["display-message", "-p", "-t", "main:0", "#{window_layout}"]);
    let other_after = server.run(&["display-message", "-p", "-t", "main:1", "#{window_layout}"]);
    let replayed = live
        .commands
        .iter()
        .find(|c| c.starts_with("select-layout"))
        .cloned()
        .unwrap_or_default();
    check(
        replayed.contains(&before),
        &format!("the record is the pristine layout: {replayed}"),
    )?;
    check(after == before, &format!("restored: {after} vs {before}"))?;
    check(
        other_after == other_before,
        &format!("the other window restored: {other_after} vs {other_before}"),
    )?;
    Ok(vec![
        format!("before {before} | {other_before}"),
        format!("first tmux notification after refresh-client -C 50x20: {first_change}"),
        format!("replayed on release: {replayed}"),
        format!("after release (nobody else attached): {after} | {other_after}"),
    ])
}

/// A split window and a second window, nobody attached: the layouts.
fn alone_session(server: &Server) -> (String, String) {
    session(server, "200x60");
    server.run(&["split-window", "-v", "-t", "main"]);
    server.run(&["split-window", "-h", "-t", "main"]);
    server.run(&["new-window", "-d", "-t", "main", "/bin/sh"]);
    server.run(&["split-window", "-d", "-h", "-t", "main:1", "/bin/sh"]);
    (
        server.run(&["display-message", "-p", "-t", "main:0", "#{window_layout}"]),
        server.run(&["display-message", "-p", "-t", "main:1", "#{window_layout}"]),
    )
}

/// The phone alone again, handing the windows back the way a disconnect
/// does: the teardown commands over a channel of their own (the session
/// crate's courtesy), with our client still attached, then the detach.
fn layout_teardown_alone() -> Outcome {
    let server = Server::new("layout-teardown");
    let (before, other_before) = alone_session(&server);
    let mut live = Live::attach(&server, "main", Config::default(), 50, 20);
    live.run(2500);
    let pinned = server.run(&["display-message", "-p", "-t", "main:0", "#{window_layout}"]);
    check(pinned.contains(",50x20,"), &format!("pinned: {pinned}"))?;
    let windows: Vec<String> = live.controller.resized_windows().iter().cloned().collect();
    let mut commands: Vec<String> = windows
        .iter()
        .map(|w| format!("set-option -u -w -t {w} window-size"))
        .collect();
    commands.extend(live.controller.pristine_layout_restore_commands());
    for command in &commands {
        server.shell(command);
    }
    drop(live);
    thread::sleep(Duration::from_millis(300));
    let after = server.run(&["display-message", "-p", "-t", "main:0", "#{window_layout}"]);
    let other_after = server.run(&["display-message", "-p", "-t", "main:1", "#{window_layout}"]);
    check(after == before, &format!("restored: {after} vs {before}"))?;
    check(
        other_after == other_before,
        &format!("the other window restored: {other_after} vs {other_before}"),
    )?;
    Ok(vec![
        format!("teardown: {}", commands.join(" ; ")),
        format!("after the phone left: {after} | {other_after}"),
    ])
}

/// The ledger's commands (no client of ours left): with nobody attached,
/// each window goes to its record's size, takes the layout, and is handed
/// back to automatic sizing.
fn layout_ledger_alone() -> Outcome {
    let server = Server::new("layout-ledger");
    let (before, _) = alone_session(&server);
    let mut live = Live::attach(&server, "main", Config::default(), 50, 20);
    live.run(2500);
    let ledger = live.controller.take_pin_ledger();
    drop(live);
    thread::sleep(Duration::from_millis(300));
    let squashed = server.run(&["display-message", "-p", "-t", "main:0", "#{window_layout}"]);
    check(
        squashed.contains(",50x20,"),
        &format!("left squashed: {squashed}"),
    )?;
    let mut tally = Controller::new(Config::default());
    tally.inherit_pin_ledger(ledger.clone());
    let mut commands: Vec<String> = ledger
        .resized_windows
        .iter()
        .map(|w| format!("set-option -u -w -t {w} window-size"))
        .collect();
    commands.extend(tally.pristine_layout_restore_commands());
    for command in &commands {
        server.shell(command);
    }
    let after = server.run(&["display-message", "-p", "-t", "main:0", "#{window_layout}"]);
    check(after == before, &format!("restored: {after} vs {before}"))?;
    Ok(vec![
        format!("ledger: {}", commands.join(" ; ")),
        format!("after: {after}"),
    ])
}

/// Only `%session-changed` says which session is ours.
fn session_follow() -> Outcome {
    let server = Server::new("follow");
    session(&server, "80x24");
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "other",
        "-x",
        "80",
        "-y",
        "24",
        "/bin/sh",
    ]);
    let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
    live.run(1500);
    check(
        live.controller.snapshot().active_session_id.as_deref() == Some("$0"),
        "on main",
    )?;
    // Another client attaching elsewhere: re-listed, not followed.
    let other = Reaped(
        server
            .command()
            .args(["-C", "attach", "-t", "other"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    live.run(800);
    let stayed = live.controller.snapshot().active_session_id.clone();
    check(
        stayed.as_deref() == Some("$0"),
        "another client's attach is not ours",
    )?;
    // Our own client switched (by anyone): %session-changed moves us.
    let client = live.controller.our_client_name().unwrap_or("").to_owned();
    server.run(&["switch-client", "-c", &client, "-t", "other"]);
    live.run(1200);
    let moved = live.controller.snapshot().active_session_id.clone();
    drop(other);
    check(
        moved.as_deref() == Some("$1"),
        &format!("followed to $1, is {moved:?}"),
    )?;
    let saw = String::from_utf8_lossy(&live.raw).contains("%session-changed $1 other");
    check(saw, "tmux said %session-changed")?;
    Ok(vec![
        "another client attaching to `other`: we stay on $0 (only %client-session-changed)".into(),
        "our client switched: %session-changed $1 other, the controller follows".into(),
    ])
}

/// The hook poll reads every pane's @moshpit_* stamps.
fn hooks() -> Outcome {
    let server = Server::new("hooks");
    session(&server, "80x24");
    let pane = first_pane(&server);
    for (option, value) in [
        ("@moshpit_state", "attention"),
        ("@moshpit_agent", "claude"),
        ("@moshpit_since", "1758000000"),
        ("@moshpit_title", "needs | you"),
    ] {
        server.run(&["set-option", "-p", "-t", &pane, option, value]);
    }
    let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
    live.run(1500);
    live.act(|c, now| c.poll_agent_hooks(now));
    live.run(500);
    let hook = live
        .controller
        .agent_hooks()
        .get(&pane)
        .cloned()
        .ok_or("no stamp for the pane")?;
    check(hook.state.as_deref() == Some("attention"), "state")?;
    check(
        hook.title.as_deref() == Some("needs | you"),
        "title with a pipe",
    )?;
    check(
        hook.command.as_deref().is_some_and(|c| !c.is_empty()),
        "the running command",
    )?;
    Ok(vec![format!("{pane}: {hook:?}")])
}

/// A directory of the case's own, removed when the case ends.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// UI-DESIGN §10.5: a pane's working directory as tmux has it
/// (`#{pane_current_path}`) at attach, and after a `cd` once the panes are
/// listed again (the refresh control); directory names with spaces.
fn pane_path() -> Outcome {
    let server = Server::new("pane-path");
    let scratch =
        ScratchDir(std::env::temp_dir().join(format!("moshpit pane path {}", std::process::id())));
    let inner = scratch.0.join("inner dir");
    std::fs::create_dir_all(&inner).map_err(|e| e.to_string())?;
    let real = |dir: &std::path::Path| {
        std::fs::canonicalize(dir).map(|p| p.to_string_lossy().into_owned())
    };
    let (start, moved) = (
        real(&scratch.0).map_err(|e| e.to_string())?,
        real(&inner).map_err(|e| e.to_string())?,
    );
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "main",
        "-x",
        "80",
        "-y",
        "24",
        "-c",
        &start,
        "/bin/sh",
    ]);
    let pane = first_pane(&server);
    let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
    live.run(1500);
    let path = |live: &Live| {
        live.controller
            .snapshot()
            .panes
            .get(&pane)
            .and_then(|info| info.path.clone())
    };
    let before = path(&live);
    check(
        before.as_deref() == Some(start.as_str()),
        &format!("the pane's directory at attach, is {before:?}"),
    )?;
    server.run(&["send-keys", "-t", &pane, &format!("cd '{moved}'"), "Enter"]);
    thread::sleep(Duration::from_millis(400));
    live.act(|c, now| c.refresh(now));
    live.run(1000);
    let after = path(&live);
    check(
        after.as_deref() == Some(moved.as_str()),
        &format!("the pane's directory after cd, is {after:?}"),
    )?;
    Ok(vec![format!("{pane}: {before:?} -> {after:?}")])
}

/// A program with the mouse on gets the wheel; a shell its local scrollback.
fn scroll() -> Outcome {
    let server = Server::new("scroll");
    session(&server, "80x24");
    let pane = first_pane(&server);
    let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
    live.run(1500);
    live.act(|c, now| c.refresh_active_pane_mouse(now));
    live.run(300);
    let shell = live.controller.scroll_route().map(|r| r.1);
    check(shell == Some(ScrollRoute::Local), "a shell scrolls locally")?;
    let out = format!("/tmp/moshpit-w8-{}-wheel", std::process::id());
    let _ = std::fs::remove_file(&out);
    let program = format!(
        "printf '\\033[?1000h\\033[?1006h'; stty raw -echo; dd bs=1 count=12 2>/dev/null > {out}; printf '\\033[?1000l'; stty sane"
    );
    server.run(&["send-keys", "-t", &pane, &program, "Enter"]);
    thread::sleep(Duration::from_millis(400));
    live.act(|c, now| c.refresh_active_pane_mouse(now));
    // Quiet for longer than the streaming window: a primary-screen mouse
    // app that is repainting gets the local scrollback instead.
    live.run(800);
    let flags = live.controller.active_input_flags();
    check(flags.wants_mouse, "mouse_any_flag read")?;
    live.act(|c, now| c.scroll_lines(1, now));
    live.run(600);
    let got = std::fs::read(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    check(
        got.starts_with(b"\x1b[<64;40;12M"),
        &format!("wheel read {:?}", String::from_utf8_lossy(&got)),
    )?;
    Ok(vec![
        "shell: local scrollback".into(),
        format!(
            "mouse app: the pane read {:?}",
            String::from_utf8_lossy(&got)
        ),
    ])
}

// MARK: - Evidence for the Swift bugs

/// tmux refuses a capture of a pane that is gone, with its complaint.
fn refused_capture() -> Outcome {
    let server = Server::new("refused");
    session(&server, "80x24");
    let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
    live.run(1200);
    live.act(|c, now| c.run_command("capture-pane -p -e -t %99", now));
    live.run(500);
    let raw = String::from_utf8_lossy(&live.raw).into_owned();
    let refusal = raw
        .lines()
        .skip_while(|l| !l.starts_with("%begin"))
        .collect::<Vec<_>>()
        .windows(2)
        .find(|w| w[1].starts_with("%error"))
        .map(|w| w[0].to_owned())
        .ok_or("no %error reply")?;
    check(refusal.contains("can't find pane"), &refusal)?;
    Ok(vec![format!(
        "capture-pane -t %99 → %error with `{refusal}` as its line"
    )])
}

/// A pane exiting inside a window that lives on: tmux says only
/// %layout-change; the next whole-server listing no longer has the pane.
fn pane_exit_layout_only() -> Outcome {
    let server = Server::new("exit");
    session(&server, "80x24");
    server.run(&["split-window", "-h", "-t", "main"]);
    let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
    live.run(1500);
    let panes = server.run(&["list-panes", "-t", "main", "-F", "#{pane_id}"]);
    let dying = panes.lines().last().unwrap_or("").to_owned();
    check(
        live.controller.stream_of(&dying).is_some(),
        "the pane has a terminal",
    )?;
    let mark = live.raw.len();
    server.run(&["send-keys", "-t", &dying, "exit", "Enter"]);
    live.run(800);
    let said = String::from_utf8_lossy(&live.raw[mark..]).into_owned();
    let notifications: Vec<&str> = said
        .lines()
        .filter(|l| l.starts_with('%') && !l.starts_with("%output"))
        .collect();
    check(
        notifications
            .iter()
            .any(|l| l.starts_with("%layout-change"))
            && !notifications.iter().any(|l| l.contains("window-close")),
        &format!("{notifications:?}"),
    )?;
    live.act(|c, now| c.refresh(now));
    live.run(800);
    check(
        live.controller.stream_of(&dying).is_none(),
        "the dead pane's terminal is freed",
    )?;
    Ok(vec![
        format!("{dying} exited: tmux sent {notifications:?} (no window close)"),
        format!("the next list-panes -a has no {dying}: its stream is closed"),
    ])
}

/// A session's current window is shared by all its clients: one client's
/// select-window moves the other's screen (what the mosh renderer shows).
fn shared_current_window() -> Outcome {
    let server = Server::new("shared");
    session(&server, "80x24");
    server.run(&["new-window", "-t", "main", "/bin/sh"]);
    server.run(&["select-window", "-t", "main:0"]);
    let sidecar = Config {
        renders_output: false,
        ..Config::default()
    };
    let mut live = Live::attach(&server, "main", sidecar, 80, 24);
    live.run(1500);
    // Another client on the session (the renderer's stand-in).
    let renderer = Reaped(
        server
            .command()
            .args(["-C", "attach", "-t", "main"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    thread::sleep(Duration::from_millis(300));
    let clients = server.run(&["list-clients", "-t", "main", "-F", "#{client_name}"]);
    let other = clients.lines().last().unwrap_or("").to_owned();
    server.run(&["select-window", "-t", "main:1"]);
    live.run(1000);
    let seen = server.run(&["display-message", "-p", "-c", &other, "#{window_index}"]);
    let ours = live.controller.snapshot().active_window_id.clone();
    let window1 = server.run(&["display-message", "-p", "-t", "main:1", "#{window_id}"]);
    drop(renderer);
    check(
        seen == "1",
        &format!("the other client shows window {seen}"),
    )?;
    check(
        ours.as_deref() == Some(window1.as_str()),
        &format!("the sidecar is on {ours:?}"),
    )?;
    Ok(vec![
        format!("select-window main:1 from outside: client {other} now shows window 1"),
        format!("the sidecar's breadcrumb follows to {window1}"),
    ])
}

/// In control mode `;` separates commands: an id spliced unchecked into a
/// command can carry another.
fn semicolon_separates() -> Outcome {
    let server = Server::new("semicolon");
    session(&server, "80x24");
    let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
    live.run(1200);
    let before = live.raw.len();
    live.act(|c, now| {
        c.run_command(
            "display-message -p -t %0 ; display-message -p injected",
            now,
        )
    });
    live.run(500);
    let said = String::from_utf8_lossy(&live.raw[before..]).into_owned();
    check(said.contains("\ninjected\n"), &said)?;
    // The controller refuses such an id outright.
    let request = live
        .controller
        .request_pane_path("%0 ; kill-server", live.now());
    live.pump();
    check(
        live.notes.contains(&Notification::PanePath {
            request,
            path: None,
        }),
        "refused",
    )?;
    check(
        server.run(&["has-session", "-t", "main"]).is_empty(),
        "server alive",
    )?;
    Ok(vec![
        "`display-message -p -t %0 ; display-message -p injected` ran both commands".into(),
    ])
}

/// B273: an `after-<command>` hook one of ours fires answers with a block
/// of its own (flags 0) right behind our command's. People set these:
/// the window switch's capture and the new pane's listing must still be
/// theirs.
fn user_hooks() -> Outcome {
    let server = Server::new("hooks-user");
    session(&server, "80x24");
    server.run(&["new-window", "-t", "main", "/bin/sh"]);
    server.run(&["select-window", "-t", "main:0"]);
    server.run(&["set-hook", "-g", "after-select-window", "refresh-client -S"]);
    server.run(&["set-hook", "-g", "after-split-window", "select-layout tiled"]);
    server.run(&["set-hook", "-g", "after-resize-window", "set -g @r35 1"]);
    let window1 = server.run(&["display-message", "-p", "-t", "main:1", "#{window_id}"]);
    let pane1 = server.run(&["display-message", "-p", "-t", "main:1", "#{pane_id}"]);
    server.run(&["send-keys", "-t", &pane1, "echo MARK-$((6*7))", "Enter"]);
    thread::sleep(Duration::from_millis(300));
    let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
    live.run(1500);
    let mark = live.raw.len();
    // The pane's path asked right behind the switch: its reply comes after
    // the hook's block.
    let mut request = 0;
    live.act(|c, now| {
        c.select_window(&window1, now);
        request = c.request_pane_path(&pane1, now);
    });
    live.run(1200);
    let path = live.notes.iter().find_map(|note| match note {
        Notification::PanePath { request: r, path } if *r == request => Some(path.clone()),
        _ => None,
    });
    let real = server.run(&["display-message", "-p", "-t", &pane1, "#{pane_current_path}"]);
    check(
        path == Some(Some(real.clone())),
        &format!("the pane's path read {path:?}, tmux says {real:?}"),
    )?;
    live.act(|c, now| c.new_pane(now));
    live.run(1200);
    live.act(|c, now| c.refresh(now));
    live.run(1200);
    let said = String::from_utf8_lossy(&live.raw[mark..]).into_owned();
    let hook_blocks = said
        .lines()
        .filter(|l| l.starts_with("%begin ") && l.ends_with(" 0"))
        .count();
    check(hook_blocks >= 2, &format!("hook blocks: {hook_blocks}"))?;
    let tmux_panes: Vec<String> = server
        .run(&["list-panes", "-a", "-F", "#{pane_id}"])
        .lines()
        .map(str::to_owned)
        .collect();
    let s = live.controller.snapshot();
    let ours: Vec<String> = s.panes.keys().cloned().collect();
    let mut sorted = tmux_panes.clone();
    sorted.sort();
    check(ours == sorted, &format!("panes {ours:?}, tmux {tmux_panes:?}"))?;
    check(
        s.sessions.get("$0").is_some_and(|session| session.name == "main"),
        &format!("sessions {:?}", s.sessions),
    )?;
    check(
        live.pane_text(&pane1).contains("MARK-42"),
        "the window's pane filled from its own capture",
    )?;
    Ok(vec![
        format!("{hook_blocks} hook blocks (flags 0) between our replies"),
        format!("tree in step: panes {ours:?}; {pane1} shows its own screen; its path {real:?}"),
    ])
}

/// B273: a command that answers more than once, sent through
/// `run_command`, is fenced: the listings right behind it are their own.
fn multi_reply_run() -> Outcome {
    let server = Server::new("multi-reply");
    session(&server, "80x24");
    let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
    live.run(1200);
    let mark = live.raw.len();
    live.act(|c, now| {
        c.run_command("if-shell -F 1 'display-message -p inner'", now);
        c.run_command("display-message -p a ; display-message -p b", now);
        c.refresh(now);
    });
    live.run(1500);
    let said = String::from_utf8_lossy(&live.raw[mark..]).into_owned();
    check(said.contains("\ninner\n") && said.contains("\nb\n"), &said)?;
    let s = live.controller.snapshot();
    check(
        s.sessions.get("$0").is_some_and(|session| session.name == "main")
            && s.panes.len() == 1,
        &format!("sessions {:?} panes {:?}", s.sessions, s.panes.keys()),
    )?;
    let fences = live
        .commands
        .iter()
        .filter(|c| c.starts_with("display-message -p 'moshpit-fence-"))
        .count();
    check(fences == 2, &format!("{fences} fences"))?;
    Ok(vec![
        "if-shell -F 1 (2 blocks) and `a ; b` (2 blocks), each fenced; the refresh behind them in step".into(),
    ])
}

/// B87: a window linked into two sessions. A close notification says it
/// left a session; the window on screen stays while it is still ours.
fn linked_window_close() -> Outcome {
    let server = Server::new("linked");
    session(&server, "80x24");
    server.run(&["new-session", "-d", "-s", "other", "-x", "80", "-y", "24", "/bin/sh"]);
    server.run(&["new-window", "-t", "main", "/bin/sh"]);
    let window = server.run(&["display-message", "-p", "-t", "main:1", "#{window_id}"]);
    let pane = server.run(&["display-message", "-p", "-t", "main:1", "#{pane_id}"]);
    server.run(&["link-window", "-s", "main:1", "-t", "other:5"]);
    server.run(&["select-window", "-t", "main:1"]);
    let mut live = Live::attach(&server, "main", Config::default(), 80, 24);
    live.run(1500);
    let first = live.controller.snapshot().clone();
    check(
        first.windows.get(&window).is_some_and(|w| w.session_id == "$0")
            && first.active_window_id.as_deref() == Some(window.as_str()),
        &format!("linked window on screen in main: {:?}", first.windows.get(&window)),
    )?;
    let mark = live.raw.len();
    server.run(&["unlink-window", "-t", "other:5"]);
    live.run(1000);
    let said = String::from_utf8_lossy(&live.raw[mark..]).into_owned();
    let close = said
        .lines()
        .find(|l| l.contains("window-close"))
        .unwrap_or("")
        .to_owned();
    let s = live.controller.snapshot();
    check(
        s.windows.contains_key(&window)
            && s.active_window_id.as_deref() == Some(window.as_str())
            && live.controller.stream_of(&pane).is_some(),
        &format!("after {close:?}: {:?} on screen, {} kept", s.active_window_id, pane),
    )?;
    // Linked again, then unlinked from main: alive in `other` only.
    server.run(&["link-window", "-s", &window, "-t", "other:5"]);
    live.run(600);
    let mark = live.raw.len();
    server.run(&["unlink-window", "-t", "main:1"]);
    live.run(1000);
    let said = String::from_utf8_lossy(&live.raw[mark..]).into_owned();
    let close2 = said
        .lines()
        .find(|l| l.contains("window-close"))
        .unwrap_or("")
        .to_owned();
    let s = live.controller.snapshot();
    let other = server.run(&["display-message", "-p", "-t", "other", "#{session_id}"]);
    check(
        s.windows.get(&window).is_some_and(|w| w.session_id == other),
        &format!("after {close2:?}: {:?}", s.windows.get(&window)),
    )?;
    Ok(vec![
        format!("unlinked from other: tmux said {close:?}; {window} stayed on screen, {pane}'s terminal kept"),
        format!("unlinked from main: tmux said {close2:?}; {window} listed under {other}"),
    ])
}

fn main() -> ExitCode {
    let cases: Vec<Case> = vec![
        ("attach", attach),
        ("arrow-keys", arrow_keys),
        ("paste", paste),
        ("lease", lease),
        ("layout", layout),
        ("layout-without-desktop", layout_without_desktop),
        ("layout-teardown-alone", layout_teardown_alone),
        ("layout-ledger-alone", layout_ledger_alone),
        ("session-follow", session_follow),
        ("hooks", hooks),
        ("pane-path", pane_path),
        ("scroll", scroll),
        ("refused-capture", refused_capture),
        ("pane-exit-layout-only", pane_exit_layout_only),
        ("shared-current-window", shared_current_window),
        ("semicolon-separates", semicolon_separates),
        ("user-hooks", user_hooks),
        ("multi-reply-run", multi_reply_run),
        ("linked-window-close", linked_window_close),
    ];
    let wanted: Vec<String> = std::env::args().skip(1).collect();
    let version = Command::new(TMUX)
        .arg("-V")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    println!("# {}", version.unwrap_or_default());
    let mut failed = 0;
    for (name, case) in cases {
        if !wanted.is_empty() && !wanted.iter().any(|w| w == name) {
            continue;
        }
        match case() {
            Ok(evidence) => {
                println!("ok   {name}");
                for line in evidence {
                    println!("     {line}");
                }
            }
            Err(why) => {
                failed += 1;
                println!("FAIL {name}: {why}");
            }
        }
    }
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
