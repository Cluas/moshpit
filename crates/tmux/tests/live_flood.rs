//! B739 against a real tmux: a Ctrl-C under an output flood. The
//! controller drives a `tmux -C` client of a private server (its own `-L`
//! socket, no config), a shell in the pane runs `yes`, and the phone's ^C
//! goes the way the app sends it (`send-keys -H 03`). Measured: how long the
//! key takes to leave, and how long and how many bytes it takes for the
//! pane's output to stop arriving. Over a pipe, so the SSH window's share
//! (what is already on its way when the key lands) is not in it.
//!
//! ```sh
//! cargo test -p moshpit-tmux --release --test live_flood -- --ignored --nocapture
//! ```

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use moshpit_tmux::controller::{Config, Controller, Output};

struct Server {
    socket: String,
}

impl Server {
    fn start() -> Server {
        let socket = format!("moshpit-b739-{}", std::process::id());
        let server = Server { socket };
        server.tmux(&["new-session", "-d", "-s", "main", "-x", "120", "-y", "40", "sh"]);
        server
    }

    fn tmux(&self, args: &[&str]) -> String {
        let out = Command::new("tmux")
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .args(args)
            .output()
            .expect("tmux runs");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn client(&self) -> (Child, ChildStdin, Receiver<(Instant, Vec<u8>)>) {
        let mut child = Command::new("tmux")
            .args(["-L", &self.socket, "-f", "/dev/null", "-C", "attach", "-t", "main"])
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
                if n == 0 || tx.send((Instant::now(), buf[..n].to_vec())).is_err() {
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

struct Phone {
    controller: Controller,
    _child: Child,
    stdin: ChildStdin,
    rx: Receiver<(Instant, Vec<u8>)>,
    start: Instant,
    /// Bytes of `%output` read, and when the last came.
    output_bytes: usize,
    last_output: Option<Instant>,
    /// When the last `send-keys` went out.
    last_keys: Option<Instant>,
}

impl Phone {
    fn now(&self) -> u64 {
        1_000_000 + self.start.elapsed().as_millis() as u64
    }

    /// One turn of the app's loop: what tmux sent, then the commands.
    fn turn(&mut self) {
        while let Ok((at, bytes)) = self.rx.try_recv() {
            if bytes.windows(8).any(|w| w == b"%output ") {
                self.output_bytes += bytes.len();
                self.last_output = Some(at);
            }
            let now = self.now();
            self.controller.receive(&bytes, now);
        }
        self.write_commands();
    }

    fn write_commands(&mut self) {
        let now = self.now();
        self.controller.tick(now);
        for output in self.controller.drain_outputs() {
            if let Output::Command(command) = output {
                if command.starts_with("send-keys") {
                    self.last_keys = Some(Instant::now());
                }
                let line = format!("{command}\n");
                self.stdin.write_all(line.as_bytes()).expect("tmux's client takes it");
            }
        }
    }

    fn pump_for(&mut self, ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < until {
            self.turn();
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[test]
#[ignore = "needs tmux; see the module docs"]
fn a_ctrl_c_stops_a_flood() {
    let server = Server::start();
    let (child, stdin, rx) = server.client();
    let mut phone = Phone {
        controller: Controller::new(Config::default()),
        _child: child,
        stdin,
        rx,
        start: Instant::now(),
        output_bytes: 0,
        last_output: None,
        last_keys: None,
    };
    let now = phone.now();
    phone.controller.begin_control_mode(now);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(phone.controller.snapshot().is_attached && phone.controller.snapshot().active_pane_id.is_some()) {
        assert!(Instant::now() < deadline, "the phone attached");
        phone.pump_for(20);
    }
    let pane = phone.controller.snapshot().active_pane_id.clone().unwrap();
    phone.pump_for(300);
    let now = phone.now();
    phone.controller.view_input(&pane, b"yes\r", now);
    phone.pump_for(1000);
    let flooding = phone.output_bytes;
    assert!(flooding > 100_000, "no flood: {flooding} bytes");
    // The ^C, from inside a turn, as the bar's key arrives.
    let pressed = Instant::now();
    let before = phone.output_bytes;
    let now = phone.now();
    phone.controller.view_input(&pane, b"\x03", now);
    phone.write_commands();
    let left = phone.last_keys.expect("the key went").duration_since(pressed);
    // Until a quiet 500 ms.
    loop {
        phone.pump_for(50);
        if phone.last_output.is_some_and(|at| at.elapsed() > Duration::from_millis(500)) {
            break;
        }
        assert!(pressed.elapsed() < Duration::from_secs(20), "the flood never stopped");
    }
    let stopped = phone.last_output.unwrap().saturating_duration_since(pressed);
    let after = phone.output_bytes - before;
    eprintln!(
        "flood {:.1} MB/s; the ^C left in {:?}; output stopped {:?} after it, {} KB of %output after it",
        flooding as f64 / 1e6,
        left,
        stopped,
        after / 1024
    );
    assert!(left < Duration::from_millis(5), "the key waited {left:?}");
    assert!(stopped < Duration::from_secs(1), "the flood went on {stopped:?}");
}
