//! B585 against a real tmux: a pane printing nonstop as the phone attaches
//! keeps every line in the phone's history. The controller drives a
//! `tmux -C` client of a private server (its own `-L` socket, no config),
//! and the bytes it hands the pane go through the terminal the app draws
//! the pane with; its history and screen must read `L1`, `L2`, … without a
//! gap or a repeat.
//!
//! ```sh
//! cargo test -p moshpit-tmux --test live_backfill -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use moshpit_term::{Options, ScrollOnClear, Terminal};
use moshpit_tmux::controller::{Config, Controller, Output};

const COLS: u16 = 60;
const ROWS: u16 = 20;

struct Server {
    socket: String,
}

impl Server {
    fn start(test: &str) -> Server {
        Server {
            socket: format!("moshpit-b585-{}-{test}", std::process::id()),
        }
    }

    fn tmux(&self, args: &[&str]) -> String {
        let out = Command::new("tmux")
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .args(args)
            .output()
            .expect("tmux runs");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn client(&self, target: &str) -> (Child, ChildStdin, Receiver<Vec<u8>>) {
        let mut child = Command::new("tmux")
            .args([
                "-L",
                &self.socket,
                "-f",
                "/dev/null",
                "-C",
                "attach",
                "-t",
                target,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("tmux -C starts");
        let stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        (child, stdin, rx)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
    }
}

/// The phone: the controller pumped against the client, each pane's bytes
/// kept in order.
struct Phone {
    controller: Controller,
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Vec<u8>>,
    start: Instant,
    panes: BTreeMap<String, Vec<u8>>,
}

impl Phone {
    fn now(&self) -> u64 {
        1_000_000 + self.start.elapsed().as_millis() as u64
    }

    fn pump(&mut self, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            while let Ok(bytes) = self.rx.try_recv() {
                let now = self.now();
                self.controller.receive(&bytes, now);
            }
            let now = self.now();
            self.controller.tick(now);
            let mut commands = Vec::new();
            for output in self.controller.drain_outputs() {
                match output {
                    Output::Command(command) => {
                        commands.extend_from_slice(command.as_bytes());
                        commands.push(b'\n');
                    }
                    Output::PaneOutput { pane, bytes, .. } => {
                        self.panes
                            .entry(pane)
                            .or_default()
                            .extend_from_slice(&bytes);
                    }
                    _ => {}
                }
            }
            // Write one turn's backfill + resync commands in a single buffer so
            // tmux -C reads and executes them back-to-back (B585).
            if !commands.is_empty() && self.stdin.write_all(&commands).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Phone {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The `L<n>` rows a pane's bytes leave in the app's terminal, history then
/// screen.
fn printed_rows(bytes: &[u8]) -> Vec<i64> {
    let mut term = Terminal::new(
        COLS,
        ROWS,
        Options {
            scroll_on_clear: ScrollOnClear::Tmux,
            ..Options::default()
        },
    );
    term.feed(bytes);
    term.flush_sync();
    let history = term.history_len() as i32;
    (-history..i32::from(ROWS))
        .filter_map(|line| term.row_text(line).0.strip_prefix('L')?.parse().ok())
        .collect()
}

/// B585: the history dump and the screen frame were two captures a round
/// trip apart; what the pane printed in between scrolled off tmux's screen
/// into neither, and the phone's history lost a line or three on every
/// attach here (far more over a phone's round trip).
#[test]
#[ignore = "needs tmux; see the module docs"]
fn a_pane_printing_through_the_attach_keeps_every_line() {
    for round in 0..3 {
        let server = Server::start(&format!("printing-{round}"));
        server.tmux(&[
            "new-session",
            "-d",
            "-s",
            "log",
            "-x",
            &COLS.to_string(),
            "-y",
            &ROWS.to_string(),
            "sh -c 'i=0; while :; do i=$((i+1)); echo L$i; sleep 0.003; done'",
        ]);
        std::thread::sleep(Duration::from_millis(1200));
        let (child, stdin, rx) = server.client("log");
        let mut phone = Phone {
            controller: Controller::new(Config::default()),
            child,
            stdin,
            rx,
            start: Instant::now(),
            panes: BTreeMap::new(),
        };
        phone
            .controller
            .set_initial_client_size(i64::from(COLS), i64::from(ROWS));
        let now = phone.now();
        phone.controller.begin_control_mode(now);
        let deadline = Instant::now() + Duration::from_secs(10);
        let pane = loop {
            phone.pump(50);
            if let Some(pane) = phone.controller.snapshot().active_pane_id.clone() {
                break pane;
            }
            assert!(Instant::now() < deadline, "attached");
        };
        let now = phone.now();
        phone
            .controller
            .view_grid(&pane, i64::from(COLS), i64::from(ROWS), now);
        phone.pump(2500);
        // The printer stops; what it printed last arrives.
        server.tmux(&["kill-session", "-t", "=log"]);
        phone.pump(300);
        let rows = printed_rows(phone.panes.get(&pane).map_or(&[][..], Vec::as_slice));
        assert!(rows.len() > 100, "round {round}: {} rows", rows.len());
        let first = rows[0];
        let missing: Vec<i64> = (first..=*rows.last().unwrap())
            .filter(|n| !rows.contains(n))
            .collect();
        let expected: Vec<i64> = (first..first + rows.len() as i64).collect();
        assert_eq!(rows, expected, "round {round}: missing {missing:?}");
    }
}
