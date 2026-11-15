//! The size lease and the layout pin with two Moshpit devices and a desktop
//! on a real tmux (B770–B779): each phone is a controller driving a `tmux -C`
//! client of a private server (its own `-L` socket, no config), the way the
//! app's control channel drives `-CC`; the desktop is a plain client on a
//! pty of a fixed size. The rules held here are the Swift path's:
//!
//! - `@moshpit_size_owner`: the device touched last holds the window's size;
//!   the other follows under a veil until the person touches it (a key, the
//!   veil's tap), and takes a window its holder let go.
//! - The layout pin: a phone's pin squashes a split window's layout; the
//!   layout found is put back exactly once the phone lets go.
//! - A phone's action never moves or sizes a desktop's client: its window,
//!   its session (issue #2).
//!
//! ```sh
//! cargo test -p moshpit-tmux --test live_lease -- --ignored --nocapture --test-threads=1
//! ```

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use moshpit_tmux::controller::{Config, Controller, Output};

struct Server {
    socket: String,
}

impl Server {
    /// A server of its own with one session, `main`, 120×40, one window split
    /// side by side.
    fn start(test: &str) -> Server {
        let socket = format!("moshpit-w15b-lease-{}-{test}", std::process::id());
        let server = Server { socket };
        server.tmux(&["new-session", "-d", "-s", "main", "-x", "120", "-y", "40"]);
        server.tmux(&["split-window", "-h", "-t", "main"]);
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

    /// The command line a hand-back script runs tmux with.
    fn prefix(&self) -> String {
        format!("tmux -L {} -f /dev/null", self.socket)
    }

    fn window(&self, target: &str) -> String {
        self.tmux(&["display-message", "-p", "-t", target, "#{window_id}"])
    }

    fn size_of(&self, window: &str) -> String {
        self.tmux(&[
            "display-message",
            "-p",
            "-t",
            window,
            "#{window_width}x#{window_height}",
        ])
    }

    fn layout_of(&self, window: &str) -> String {
        self.tmux(&["display-message", "-p", "-t", window, "#{window_layout}"])
    }

    /// The lease's holder, by its label ("" when unset).
    fn owner_of(&self, window: &str) -> String {
        let raw = self.tmux(&[
            "display-message",
            "-p",
            "-t",
            window,
            "#{@moshpit_size_owner}",
        ]);
        raw.split('|').nth(1).unwrap_or("").to_owned()
    }

    fn window_size_option(&self, window: &str) -> String {
        self.tmux(&["show-options", "-wqv", "-t", window, "window-size"])
    }

    fn zoomed(&self, window: &str) -> bool {
        self.tmux(&[
            "display-message",
            "-p",
            "-t",
            window,
            "#{window_zoomed_flag}",
        ]) == "1"
    }

    /// Every client: name, control mode, session, window, size.
    fn clients(&self) -> String {
        self.tmux(&[
            "list-clients",
            "-F",
            "#{client_name} cc=#{client_control_mode} #{session_name} #{window_id} #{client_width}x#{client_height}",
        ])
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

    /// A desktop's terminal: a plain client of `target` on a pty of
    /// `cols`×`rows`, attached.
    fn desktop(&self, target: &str, cols: u16, rows: u16) -> Desk {
        const PTY: &str = r#"
import os, sys, pty, fcntl, termios, struct, select
cols, rows = int(sys.argv[1]), int(sys.argv[2])
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))
    os.execvp(sys.argv[3], sys.argv[3:])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))
live = [fd, 0]
while fd in live:
    r, _, _ = select.select(live, [], [])
    if fd in r:
        try:
            if not os.read(fd, 65536):
                break
        except OSError:
            break
    if 0 in r:
        d = os.read(0, 65536)
        if d:
            os.write(fd, d)
        else:
            live.remove(0)
"#;
        let before = self.clients().lines().count();
        let child = Command::new("python3")
            .args(["-c", PTY, &cols.to_string(), &rows.to_string()])
            .args([
                "tmux",
                "-L",
                &self.socket,
                "-f",
                "/dev/null",
                "attach",
                "-t",
                target,
            ])
            .env("TERM", "xterm-256color")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a pty client starts");
        assert!(
            eventually(10, || self.clients().lines().count() > before),
            "the desktop attached"
        );
        let name = self
            .clients()
            .lines()
            .find(|line| line.contains("cc=0"))
            .and_then(|line| line.split(' ').next())
            .unwrap_or_default()
            .to_owned();
        Desk { child, name }
    }

    /// A Moshpit device calling itself `label`, its view `cols`×`rows`,
    /// attached to `target`.
    fn phone(&self, target: &str, label: &str, cols: i64, rows: i64) -> Phone {
        let (child, stdin, rx) = self.client(target);
        let start = Instant::now();
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let config = Config {
            size_owner_label: label.to_owned(),
            unix_offset_ms: unix_ms - 1_000_000,
            ..Config::default()
        };
        let mut phone = Phone {
            controller: Controller::new(config),
            child,
            stdin,
            rx,
            start,
            label: label.to_owned(),
        };
        phone.controller.set_initial_client_size(cols, rows);
        let now = phone.now();
        phone.controller.begin_control_mode(now);
        let attached = phone.pump_until(15, |c| {
            c.snapshot().is_attached
                && c.snapshot().active_window_id.is_some()
                && c.our_client_name().is_some()
        });
        assert!(attached, "{label}: {:?}", phone.controller.snapshot());
        // The view's first real report.
        let now = phone.now();
        if let Some(pane) = phone.controller.snapshot().active_pane_id.clone() {
            phone.controller.stream_for_pane(&pane);
            phone.controller.view_grid(&pane, cols, rows, now);
        }
        phone.pump(600);
        phone
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
    }
}

struct Desk {
    child: Child,
    name: String,
}

impl Drop for Desk {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Phone {
    controller: Controller,
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Vec<u8>>,
    start: Instant,
    label: String,
}

impl Phone {
    fn now(&self) -> u64 {
        1_000_000 + self.start.elapsed().as_millis() as u64
    }

    /// Feed tmux's bytes and write the controller's commands until `ms`
    /// pass quietly.
    fn pump(&mut self, ms: u64) {
        let mut quiet_since = Instant::now();
        while quiet_since.elapsed() < Duration::from_millis(ms) {
            let mut moved = false;
            while let Ok(bytes) = self.rx.try_recv() {
                let now = self.now();
                if trace() {
                    for line in String::from_utf8_lossy(&bytes).lines() {
                        if line.starts_with("%layout-change")
                            || line.starts_with("%session-changed")
                        {
                            eprintln!("{} {} < {line}", self.now(), self.label);
                        }
                    }
                }
                self.controller.receive(&bytes, now);
                moved = true;
            }
            let now = self.now();
            self.controller.tick(now);
            for output in self.controller.drain_outputs() {
                if let Output::Command(command) = output {
                    if trace() {
                        eprintln!("{} {} > {command}", self.now(), self.label);
                    }
                    let line = format!("{command}\n");
                    if self.stdin.write_all(line.as_bytes()).is_err() {
                        return;
                    }
                    moved = true;
                }
            }
            if moved {
                quiet_since = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn pump_until(&mut self, seconds: u64, mut done: impl FnMut(&Controller) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline {
            self.pump(100);
            if done(&self.controller) {
                return true;
            }
        }
        done(&self.controller)
    }

    fn window(&self) -> String {
        self.controller
            .snapshot()
            .active_window_id
            .clone()
            .unwrap_or_default()
    }

    fn pane(&self) -> String {
        self.controller
            .snapshot()
            .active_pane_id
            .clone()
            .unwrap_or_default()
    }

    /// Who the veil says holds the window ("" when not following).
    fn follows(&self) -> String {
        self.controller
            .size_follow()
            .map(|follow| follow.owner_label.clone())
            .unwrap_or_default()
    }

    /// A key typed into the pane on screen.
    fn type_keys(&mut self, bytes: &[u8]) {
        let pane = self.pane();
        let now = self.now();
        self.controller.view_input(&pane, bytes, now);
        self.pump(300);
    }

    /// The veil's tap.
    fn take(&mut self) {
        let now = self.now();
        self.controller.take_size_ownership(now);
        self.pump(300);
    }

    /// The person's end of the session, as the app's courtesy close runs
    /// it (`tmux_leg::Task::close`): the leases, the pins and the layouts
    /// handed back over a connection of their own, then the client gone.
    fn leave(mut self, server: &Server) {
        self.pump(200);
        let leases = self.controller.size_lease_release_commands();
        let prefix = server.prefix();
        let script = self.controller.pin_hand_back_script(&prefix, &[]);
        let _ = self.stdin.write_all(b"detach-client\n");
        let _ = self.child.wait();
        if let Some(script) = script {
            let out = Command::new("sh").args(["-c", &script]).output().unwrap();
            assert!(
                out.status.success(),
                "{}: hand-back script failed",
                self.label
            );
        }
        for command in leases {
            let _ = Command::new("sh")
                .args(["-c", &format!("{prefix} {command}")])
                .output();
        }
    }

    /// The app killed, the network gone: the client dies, nothing handed
    /// back.
    fn die(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `LEASE_TRACE=1`: every command a phone sends and the layout changes it
/// hears, on stderr.
fn trace() -> bool {
    std::env::var_os("LEASE_TRACE").is_some()
}

/// Polls `check` every 50 ms for up to `seconds`.
fn eventually(seconds: u64, mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    check()
}

/// Pumps both phones for `ms`, side by side.
fn pump_both(a: &mut Phone, b: &mut Phone, ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        a.pump(50);
        b.pump(50);
    }
}

// MARK: - Two phones

/// Two phones on one window: the one touched last holds its size, the other
/// follows under a veil naming the holder; a key or the veil's tap takes it
/// back; the holder leaving frees it for the other; both gone, the window and
/// its layout are as they were.
#[test]
#[ignore = "needs tmux; see the module docs"]
fn two_phones_the_last_touched_holds_the_size_and_the_other_follows() {
    let server = Server::start("two");
    let window = server.window("main");
    let layout = server.layout_of(&window);
    eprintln!("before: {} {layout}", server.size_of(&window));

    let mut phone = server.phone("main", "iPhone", 50, 20);
    assert_eq!(phone.window(), window);
    eprintln!(
        "iPhone attached: {}\n{}",
        server.size_of(&window),
        server.clients()
    );
    assert_eq!(server.size_of(&window), "50x20", "pinned to the phone");
    assert_eq!(server.owner_of(&window), "iPhone");

    let mut pad = server.phone("main", "iPad", 100, 30);
    pump_both(&mut phone, &mut pad, 1_500);
    eprintln!(
        "iPad attached: {} owner={} iPhone follows {:?} iPad follows {:?}",
        server.size_of(&window),
        server.owner_of(&window),
        phone.follows(),
        pad.follows()
    );
    // The iPhone holds a lease nobody typed into for less than the steal
    // time: the iPad attaching follows it.
    let holder = server.owner_of(&window);
    match holder.as_str() {
        "iPhone" => {
            assert_eq!(server.size_of(&window), "50x20");
            assert_eq!(pad.follows(), "iPhone");
            assert_eq!(phone.follows(), "");
        }
        "iPad" => {
            assert_eq!(server.size_of(&window), "100x30");
            assert_eq!(phone.follows(), "iPad");
            assert_eq!(pad.follows(), "");
        }
        other => panic!("nobody holds the window: {other:?}"),
    }

    // The iPad's veil tapped: it holds, the iPhone follows.
    pad.take();
    pump_both(&mut phone, &mut pad, 1_500);
    assert_eq!(server.owner_of(&window), "iPad");
    assert_eq!(server.size_of(&window), "100x30");
    assert_eq!(phone.follows(), "iPad", "the iPhone follows the iPad");
    assert_eq!(pad.follows(), "");

    // A key on the iPhone takes it back.
    phone.type_keys(b"x");
    pump_both(&mut phone, &mut pad, 1_500);
    assert_eq!(server.owner_of(&window), "iPhone");
    assert_eq!(server.size_of(&window), "50x20");
    assert_eq!(pad.follows(), "iPhone", "the iPad follows the iPhone");
    assert_eq!(phone.follows(), "");

    // The iPhone leaves: the iPad takes the window it let go.
    phone.leave(&server);
    let freed = pad.pump_until(15, |c| c.size_follow().is_none());
    pad.pump(1_000);
    eprintln!(
        "iPhone left: {} owner={} {}",
        server.size_of(&window),
        server.owner_of(&window),
        server.clients()
    );
    assert!(freed, "the iPad's veil drops");
    assert_eq!(server.owner_of(&window), "iPad");
    assert_eq!(server.size_of(&window), "100x30");

    // The iPad leaves too: the window, its layout and its sizing as found.
    pad.leave(&server);
    assert!(
        eventually(5, || server.layout_of(&window) == layout),
        "the layout put back: {} (was {layout})",
        server.layout_of(&window)
    );
    assert_eq!(server.size_of(&window), "120x40");
    assert_eq!(server.window_size_option(&window), "", "sizing automatic");
    assert_eq!(server.owner_of(&window), "", "no lease left");
    assert!(!server.zoomed(&window), "no zoom of ours left");
}

/// A phone that dies holding the window: the other takes it on its next
/// look (the holder's client is gone), and nobody is left at a dead
/// phone's size.
#[test]
#[ignore = "needs tmux; see the module docs"]
fn a_dead_holders_window_goes_to_the_other_phone() {
    let server = Server::start("dead");
    let window = server.window("main");
    let mut pad = server.phone("main", "iPad", 100, 30);
    eprintln!(
        "iPad on: {} owner={:?}",
        server.size_of(&window),
        server.owner_of(&window)
    );
    let mut phone = server.phone("main", "iPhone", 50, 20);
    pump_both(&mut phone, &mut pad, 1_500);
    eprintln!(
        "iPhone on: {} owner={:?} phone follows {:?} pad follows {:?}",
        server.size_of(&window),
        server.owner_of(&window),
        phone.follows(),
        pad.follows()
    );
    phone.type_keys(b"x");
    pump_both(&mut phone, &mut pad, 1_500);
    eprintln!(
        "iPhone typed: {} owner={:?} phone follows {:?} pad follows {:?}",
        server.size_of(&window),
        server.owner_of(&window),
        phone.follows(),
        pad.follows()
    );
    assert_eq!(server.owner_of(&window), "iPhone");
    assert_eq!(pad.follows(), "iPhone");
    phone.die();
    let freed = pad.pump_until(20, |c| c.size_follow().is_none());
    pad.pump(1_000);
    eprintln!(
        "iPhone died: {} owner={} pad follows {:?}",
        server.size_of(&window),
        server.owner_of(&window),
        pad.follows()
    );
    assert!(freed, "the iPad's veil drops once the holder is gone");
    assert_eq!(server.owner_of(&window), "iPad");
    assert_eq!(server.size_of(&window), "100x30");
}

// MARK: - A desktop

/// A desktop on the session and a phone: the phone's pin, its session
/// switch and its leaving never move the desktop's client — its session,
/// its window — and once the phone has gone the window is the desktop's
/// size again, its layout as found (issue #2, B588, B724).
#[test]
#[ignore = "needs tmux; see the module docs"]
fn a_phone_never_moves_a_desktops_client_and_gives_its_window_back() {
    let server = Server::start("desk");
    server.tmux(&["new-session", "-d", "-s", "dev", "-x", "120", "-y", "40"]);
    let window = server.window("main");
    let desk = server.desktop("main", 120, 40);
    std::thread::sleep(Duration::from_millis(500));
    // Its status line takes a row.
    let desk_size = server.size_of(&window);
    let layout = server.layout_of(&window);
    eprintln!("desktop's window: {desk_size} {layout}");
    let desk_line = |server: &Server| {
        server
            .clients()
            .lines()
            .find(|line| line.starts_with(&format!("{} ", desk.name)))
            .unwrap_or_default()
            .to_owned()
    };
    let before = desk_line(&server);
    eprintln!("desktop: {before}");
    assert!(
        before.contains(" main ") && before.contains(&window),
        "{before}"
    );

    let mut phone = server.phone("main", "iPhone", 50, 20);
    phone.type_keys(b"x");
    eprintln!(
        "phone pinned: {} {}",
        server.size_of(&window),
        server.clients()
    );
    assert_eq!(server.size_of(&window), "50x20", "the phone's pin");
    assert!(
        desk_line(&server).contains(" main "),
        "{}",
        desk_line(&server)
    );

    // The phone goes to another session: the desktop stays where it is.
    let dev = server.tmux(&["display-message", "-p", "-t", "dev", "#{session_id}"]);
    let now = phone.now();
    phone.controller.select_session(&dev, now);
    let moved = phone.pump_until(10, |c| {
        c.snapshot().active_session_id.as_deref() == Some(dev.as_str())
    });
    phone.pump(800);
    eprintln!("phone on dev: {}", server.clients());
    assert!(moved, "the phone on dev");
    let after = desk_line(&server);
    assert!(
        after.contains(" main ") && after.contains(&window),
        "the desktop moved: {after} (was {before})"
    );
    // Left behind, the window is the desktop's again.
    assert!(
        eventually(5, || server.size_of(&window) == desk_size),
        "the window left: {}",
        server.size_of(&window)
    );
    assert_eq!(server.layout_of(&window), layout, "its layout as found");

    // Back to main, then the phone's end.
    let main = server.tmux(&["display-message", "-p", "-t", "main", "#{session_id}"]);
    let now = phone.now();
    phone.controller.select_session(&main, now);
    phone.pump_until(10, |c| {
        c.snapshot().active_session_id.as_deref() == Some(main.as_str())
    });
    phone.pump(800);
    assert_eq!(server.size_of(&window), "50x20", "pinned again");
    phone.leave(&server);
    assert!(
        eventually(5, || server.size_of(&window) == desk_size),
        "handed back: {}",
        server.size_of(&window)
    );
    assert_eq!(server.layout_of(&window), layout, "its layout as found");
    assert_eq!(server.window_size_option(&window), "");
    assert!(!server.zoomed(&window));
    let last = desk_line(&server);
    assert!(last.contains(" main ") && last.contains(&window), "{last}");
    drop(desk);
}
