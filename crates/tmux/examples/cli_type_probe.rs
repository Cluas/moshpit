//! Typing into a TUI through the phone's real tmux path, compared with tmux.
//!
//! A private `tmux -L` server runs `/bin/sh`; the real [`Controller`]
//! attaches in control mode with the phone's grid, launches `<cmd>` from the
//! shell (`exec <cmd>`, Return through `view_return_sent` as the view does),
//! then types `QPROBE_WORD` (default `hello`) one key per `send_input`,
//! 400 ms apart. Every `%output` byte goes into a real `moshpit_term`
//! core, and the core's query answers go back as the app sends them
//! (`send-keys -H`), after `QPROBE_DELAY` ms (a phone's round trip; unset:
//! at once; `QPROBE_NOANSWER`: never). At the end it prints tmux's
//! `capture-pane` and the core's rows and cursor: they must match.
//!
//! `QPROBE_SET` takes tmux commands run before the attach, `;`-separated
//! (`set-option -s extended-keys always`). `QPROBE_OUT` (default the temp
//! dir) receives `pane.bin` (the pane's bytes), `cmds.txt` (what the
//! controller sent) and `replies.txt` (the core's answers).
//!
//! ```sh
//! cargo run -p moshpit-tmux --example cli_type_probe -- <cmd> [cols] [rows]
//! ```
//!
//! docs/nsh/CLI-LAB.md (Qoder) has the recipe that reaches Qoder's input.
use moshpit_term::{Options, ScrollOnClear, Terminal};
use moshpit_tmux::controller::{Config, Controller, Notification, Output};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant};
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
}

impl Drop for Live {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

thread_local! {
    /// The core's answers on their way back, each with when it lands.
    static PENDING: std::cell::RefCell<Vec<(Instant, Vec<u8>)>> = std::cell::RefCell::new(Vec::new());
}

/// Run `ms`, feeding the pane's new bytes to the core and sending its
/// replies back as the app does.

fn run_core(
    live: &mut Live,
    t: &mut Terminal,
    pane: &str,
    fed: &mut usize,
    ms: u64,
    answer: bool,
    log: &mut Vec<String>,
) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        live.run(10);
        let bytes = live.panes.get(pane).cloned().unwrap_or_default();
        if bytes.len() > *fed {
            t.feed(&bytes[*fed..]);
            *fed = bytes.len();
        }
        let replies = t.take_replies();
        if !replies.is_empty() {
            log.push(format!("reply {:?}", String::from_utf8_lossy(&replies)));
            if answer {
                let delay: u64 = std::env::var("QPROBE_DELAY")
                    .ok()
                    .and_then(|d| d.parse().ok())
                    .unwrap_or(0);
                PENDING.with(|q| {
                    q.borrow_mut().push((
                        Instant::now() + Duration::from_millis(delay),
                        replies.clone(),
                    ))
                });
            }
        }
        let due: Vec<Vec<u8>> = PENDING.with(|q| {
            let mut q = q.borrow_mut();
            let now = Instant::now();
            let (d, keep): (Vec<_>, Vec<_>) = q.drain(..).partition(|(at, _)| *at <= now);
            *q = keep;
            d.into_iter().map(|(_, b)| b).collect()
        });
        for b in due {
            let p = pane.to_owned();
            log.push(format!("sent {:?}", String::from_utf8_lossy(&b)));
            live.act(|ctl, now| ctl.send_input(&b, &p, now));
        }
    }
}

fn main() {
    let cmd = std::env::args()
        .nth(1)
        .expect("usage: cli_type_probe <cmd> [cols] [rows]");
    let out = std::env::var("QPROBE_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let (c, r): (i64, i64) = (
        std::env::args().nth(2).map_or(80, |s| s.parse().unwrap()),
        std::env::args().nth(3).map_or(24, |s| s.parse().unwrap()),
    );
    let answer = std::env::var("QPROBE_NOANSWER").is_err();
    let server = Server::new("cli-probe");
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "main",
        "-x",
        "80",
        "-y",
        "24",
        "/bin/sh",
    ]);
    if let Ok(sets) = std::env::var("QPROBE_SET") {
        for line in sets.split(';').filter(|l| !l.trim().is_empty()) {
            let words: Vec<&str> = line.split_whitespace().collect();
            println!("set: {line} -> {}", server.run(&words));
        }
    }
    let mut live = Live::attach(&server, "main", Config::default(), c, r);
    let pane = server.run(&["display-message", "-p", "-t", "main:0.0", "#{pane_id}"]);
    let mut t = Terminal::new(
        c as u16,
        r as u16,
        Options {
            scroll_on_clear: ScrollOnClear::Tmux,
            ..Options::default()
        },
    );
    let mut fed = 0usize;
    let mut log = Vec::new();
    run_core(&mut live, &mut t, &pane, &mut fed, 2500, answer, &mut log);
    live.act(|ctl, now| ctl.view_grid(&pane, c, r, now));
    run_core(&mut live, &mut t, &pane, &mut fed, 1000, answer, &mut log);
    // The person launches the CLI from the shell.
    let p = pane.clone();
    let line = format!("exec {cmd}\r");
    live.act(|ctl, now| ctl.send_input(line.as_bytes(), &p, now));
    let p = pane.clone();
    live.act(|ctl, now| ctl.view_return_sent(&p, now));
    run_core(&mut live, &mut t, &pane, &mut fed, 10000, answer, &mut log);
    let mark = fed;
    let word = std::env::var("QPROBE_WORD").unwrap_or("hello".into());
    for ch in word.chars() {
        let p = pane.clone();
        live.act(|ctl, now| ctl.send_input(ch.to_string().as_bytes(), &p, now));
        run_core(&mut live, &mut t, &pane, &mut fed, 400, answer, &mut log);
    }
    run_core(&mut live, &mut t, &pane, &mut fed, 1000, answer, &mut log);
    let bytes = live.panes.get(&pane).cloned().unwrap_or_default();
    std::fs::write(out.join("pane.bin"), &bytes).unwrap();
    std::fs::write(out.join("cmds.txt"), live.commands.join("\n")).unwrap();
    std::fs::write(out.join("replies.txt"), log.join("\n")).unwrap();
    let size = server.run(&[
        "display-message",
        "-p",
        "-t",
        &pane,
        "#{pane_width}x#{pane_height} #{cursor_x},#{cursor_y} flag=#{cursor_flag}",
    ]);
    t.flush_sync();
    println!(
        "tmux pane {size}; typed from {mark} of {}; {} replies",
        bytes.len(),
        log.len()
    );
    println!("--- tmux");
    println!("{}", server.run(&["capture-pane", "-p", "-t", &pane]));
    println!("--- core cursor {:?}", t.cursor());
    for l in 0..r as i32 {
        println!("{}", t.row_text(l).0);
    }
}
