//! B738 against a real tmux: a paste goes out the way the app sends it
//! (the controller's `set-buffer` and `paste-buffer -p` on a `tmux -C`
//! client of a private server: its own `-L` socket, no config), into a pane
//! whose program asked for bracketed paste and writes every byte it reads
//! to a file (`stty raw -echo; cat`). The bytes in the file are what a
//! program would read: compared with what the person pasted.
//!
//! ```sh
//! cargo test -p moshpit-tmux --test live_paste -- --ignored --nocapture
//! ```

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use moshpit_tmux::controller::{Config, Controller, Output};

struct Server {
    socket: String,
    out: PathBuf,
}

impl Server {
    fn start(test: &str) -> Server {
        let socket = format!("moshpit-b738-{}-{test}", std::process::id());
        let out = std::env::temp_dir().join(format!("{socket}.bytes"));
        let _ = std::fs::remove_file(&out);
        let server = Server { socket, out };
        let program = format!(
            "printf '\\033[?2004h'; stty raw -echo; exec cat > '{}'",
            server.out.display()
        );
        server.tmux(&[
            "new-session",
            "-d",
            "-s",
            "main",
            "-x",
            "120",
            "-y",
            "40",
            "sh",
            "-c",
            &program,
        ]);
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

    fn client(&self) -> (Child, ChildStdin, Receiver<Vec<u8>>) {
        let mut child = Command::new("tmux")
            .args([
                "-L",
                &self.socket,
                "-f",
                "/dev/null",
                "-C",
                "attach",
                "-t",
                "main",
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

    fn received(&self) -> Vec<u8> {
        std::fs::read(&self.out).unwrap_or_default()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        let _ = std::fs::remove_file(&self.out);
    }
}

struct Phone {
    controller: Controller,
    _child: Child,
    stdin: ChildStdin,
    rx: Receiver<Vec<u8>>,
    start: Instant,
}

impl Phone {
    fn now(&self) -> u64 {
        1_000_000 + self.start.elapsed().as_millis() as u64
    }

    /// Feeds tmux's bytes and writes the controller's commands, once.
    fn turn(&mut self) {
        while let Ok(bytes) = self.rx.try_recv() {
            let now = self.now();
            self.controller.receive(&bytes, now);
        }
        let now = self.now();
        self.controller.tick(now);
        for output in self.controller.drain_outputs() {
            if let Output::Command(command) = output {
                let line = format!("{command}\n");
                self.stdin
                    .write_all(line.as_bytes())
                    .expect("tmux's client takes it");
            }
        }
    }

    fn pump_until(&mut self, seconds: u64, mut done: impl FnMut(&Controller) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline {
            self.turn();
            if done(&self.controller) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        done(&self.controller)
    }
}

/// What tmux's `paste-buffer` makes of a buffer: each line feed a carriage
/// return (no `-r`).
fn as_tmux_pastes(text: &str) -> Vec<u8> {
    text.replace('\n', "\r").into_bytes()
}

/// Lines of CJK, emoji (ZWJ and skin tones), tabs, LF and CRLF, to about
/// `bytes`.
fn mixed_text(bytes: usize) -> String {
    let pieces = [
        "第一行：中文与 English 混排\tTAB\n",
        "emoji 🎉👨‍👩‍👧👍🏽 and ❤️ end\r\n",
        "路径 /tmp/首页.png，见 https://example.com/a?b=c&d=$HOME ~root \"quote\" back\\slash\n",
        "    indented\tcode { let x = 1; }\r\n",
        "日本語のテキスト、한국어 텍스트\n",
    ];
    let mut text = String::new();
    let mut i = 0;
    while text.len() < bytes {
        text.push_str(&format!("{i:06} "));
        text.push_str(pieces[i % pieces.len()]);
        i += 1;
    }
    text
}

/// The first place two byte strings differ, for a readable failure.
fn first_difference(a: &[u8], b: &[u8]) -> String {
    let at = a
        .iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let around = |s: &[u8]| {
        String::from_utf8_lossy(&s[at.saturating_sub(20)..(at + 20).min(s.len())]).into_owned()
    };
    format!(
        "lengths {} and {}; first difference at {at}: {:?} vs {:?}",
        a.len(),
        b.len(),
        around(a),
        around(b)
    )
}

#[test]
#[ignore = "needs tmux; see the module docs"]
fn a_big_paste_arrives_whole_in_order_and_safe() {
    let server = Server::start("big");
    let (child, stdin, rx) = server.client();
    let mut phone = Phone {
        controller: Controller::new(Config::default()),
        _child: child,
        stdin,
        rx,
        start: Instant::now(),
    };
    let now = phone.now();
    phone.controller.begin_control_mode(now);
    assert!(
        phone.pump_until(10, |c| c.snapshot().is_attached
            && c.snapshot().active_pane_id.is_some()),
        "the phone attached"
    );
    let pane = phone.controller.snapshot().active_pane_id.clone().unwrap();
    // The pane's program asked for bracketed paste before the paste goes:
    // its file is opened after the request is printed.
    assert!(
        (0..250).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            server.out.exists()
        }),
        "the pane's program never started"
    );
    // An injection first: an end marker, a command, an interrupt. Then 200
    // KB of text.
    let injection = "\u{1b}[201~echo INJECTED\u{3}\r\n";
    let body = mixed_text(200 * 1024);
    let pasted = format!("{injection}{body}");
    let now = phone.now();
    phone.controller.send_paste(&pasted, &pane, now);
    let mut expected = b"\x1b[200~".to_vec();
    expected.extend(as_tmux_pastes("echo INJECTED\n"));
    expected.extend(as_tmux_pastes(&body.replace("\r\n", "\n")));
    expected.extend(b"\x1b[201~");
    let arrived = phone.pump_until(30, |_| server.received().len() >= expected.len());
    std::thread::sleep(Duration::from_millis(300));
    let got = server.received();
    assert!(
        arrived,
        "only {} of {} bytes arrived",
        got.len(),
        expected.len()
    );
    assert!(got == expected, "{}", first_difference(&got, &expected));
    assert!(
        std::str::from_utf8(&got).is_ok(),
        "a UTF-8 sequence split or mangled"
    );
}
