//! B459 against a real tmux: the controller drives a `tmux -C` client of a
//! private server (its own `-L` socket, no config), the way the app's
//! control channel drives `-CC`. A desktop's terminal is a plain client on a
//! pty (python's `pty.spawn`). The phone's window switch leaves the
//! desktop's window alone; the phone's group goes when its client does, and
//! the session keeps every window. A desktop's own control client (iTerm2's
//! tmux integration) keeps the phone off a group.
//!
//! ```sh
//! cargo test -p moshpit-tmux --test live_group -- --ignored --nocapture
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
    /// A server of its own: the tests run side by side in one process.
    fn start(test: &str) -> Server {
        let socket = format!("moshpit-b459-{}-{test}", std::process::id());
        let server = Server { socket };
        server.tmux(&["new-session", "-d", "-s", "main", "-x", "120", "-y", "40"]);
        server.tmux(&["new-window", "-t", "main"]);
        server.tmux(&["new-window", "-t", "main"]);
        // Not the lowest window: a group starts on that one.
        server.tmux(&["select-window", "-t", "main:1"]);
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

impl Server {
    /// A desktop's terminal: a plain client of `target` on a pty.
    fn desktop(&self, target: &str) -> Child {
        Command::new("python3")
            .args(["-c", "import pty,sys; pty.spawn(sys.argv[1:])"])
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
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a pty client starts")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
    }
}

struct Phone {
    controller: Controller,
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Vec<u8>>,
    start: Instant,
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
                self.controller.receive(&bytes, now);
                moved = true;
            }
            let now = self.now();
            self.controller.tick(now);
            for output in self.controller.drain_outputs() {
                if let Output::Command(command) = output {
                    let line = format!("{command}\n");
                    if self.stdin.write_all(line.as_bytes()).is_err() {
                        // tmux's client is gone: the test says so.
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

    /// Pump until `done` holds, for up to `seconds` (a loaded machine's
    /// tmux can take longer than any quiet window between two answers).
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

#[test]
#[ignore = "needs tmux; see the module docs"]
fn a_real_tmux_keeps_the_desktop_where_it_is_and_drops_the_group_with_the_phone() {
    let server = Server::start("desktop");
    let mut desk = server.desktop("main");
    assert!(
        eventually(10, || server
            .tmux(&["list-clients", "-F", "#{client_control_mode}"])
            .lines()
            .any(|l| l == "0")),
        "the desktop attached"
    );
    let (child, stdin, rx) = server.client("main");
    let mut phone = Phone {
        controller: Controller::new(Config::default()),
        child,
        stdin,
        rx,
        start: Instant::now(),
    };
    let now = phone.now();
    phone.controller.begin_control_mode(now);
    let window_of =
        |target: &str| server.tmux(&["display-message", "-p", "-t", target, "#{window_id}"]);
    let session_window = window_of("main");
    // The group made and marked, the phone landed where the session is
    // (not on the group's first window).
    let landed = phone.pump_until(15, |c| {
        let Some(name) = c.phone_group() else {
            return false;
        };
        let group_window = window_of(&format!("={name}:"));
        group_window == session_window
            && c.snapshot().active_window_id.as_deref() == Some(session_window.as_str())
    });
    let sessions = server.tmux(&[
        "ls",
        "-F",
        "#{session_id} #{session_name} #{@moshpit_group_of} #{destroy-unattached}",
    ]);
    eprintln!("sessions:\n{sessions}");
    assert!(landed, "{:?}", phone.controller.snapshot());
    let group = sessions
        .lines()
        .find(|line| line.contains("~mp-iPhone-"))
        .expect("the phone made its group")
        .to_owned();
    assert!(group.ends_with("$0 on"), "{group}");
    let snapshot = phone.controller.snapshot();
    assert_eq!(snapshot.active_session_id.as_deref(), Some("$0"));
    assert_eq!(snapshot.sessions.len(), 1, "{:?}", snapshot.sessions);
    let group_id = group.split(' ').next().unwrap().to_owned();

    let windows: Vec<String> = server
        .tmux(&["list-windows", "-t", "main", "-F", "#{window_id}"])
        .lines()
        .map(str::to_owned)
        .collect();
    let desk_window = window_of("main");
    let other = windows.iter().find(|w| **w != desk_window).unwrap().clone();
    let now = phone.now();
    phone.controller.select_window(&other, now);
    let moved = phone.pump_until(15, |c| {
        window_of(&group_id) == other
            && c.snapshot().active_window_id.as_deref() == Some(other.as_str())
    });
    assert!(moved, "the phone's window");
    phone.pump(300);
    assert_eq!(window_of("main"), desk_window, "the desktop's window");

    // The app killed, the link gone: the client dies.
    let _ = phone.child.kill();
    let _ = phone.child.wait();
    assert!(
        eventually(10, || server.tmux(&["ls", "-F", "#{session_name}"])
            == "main"),
        "the group went with the phone: {}",
        server.tmux(&["ls", "-F", "#{session_name}"])
    );
    assert_eq!(
        server
            .tmux(&["list-windows", "-t", "main", "-F", "#{window_id}"])
            .lines()
            .count(),
        windows.len(),
        "the session keeps every window"
    );
    let _ = desk.kill();
    let _ = desk.wait();
}

#[test]
#[ignore = "needs tmux; see the module docs"]
fn a_real_tmux_keeps_the_phone_off_a_group_with_a_desktops_control_client() {
    let server = Server::start("control");
    let (_desk, _desk_in, _desk_rx) = server.client("main");
    let (child, stdin, rx) = server.client("main");
    let mut phone = Phone {
        controller: Controller::new(Config::default()),
        child,
        stdin,
        rx,
        start: Instant::now(),
    };
    let now = phone.now();
    phone.controller.begin_control_mode(now);
    let attached = phone.pump_until(15, |c| {
        c.snapshot().is_attached && c.snapshot().active_window_id.is_some()
    });
    assert!(attached, "{:?}", phone.controller.snapshot());
    phone.pump(500);
    let sessions = server.tmux(&["ls", "-F", "#{session_name}"]);
    assert_eq!(sessions, "main", "no group");
    assert_eq!(phone.controller.phone_group(), None);
    let listed = server.tmux(&["show-options", "-sqv", "@moshpit_cc"]);
    let me = server.tmux(&["list-clients", "-F", "#{client_name}"]);
    assert!(
        me.lines().any(|name| listed.split(' ').any(|n| n == name)),
        "the phone is on Moshpit's list: {listed:?} of {me:?}"
    );
}

impl Server {
    /// The phone's controller on a `-C` client of `target`, attached and
    /// settled in its own group.
    fn phone(&self, target: &str) -> Phone {
        let (child, stdin, rx) = self.client(target);
        let mut phone = Phone {
            controller: Controller::new(Config::default()),
            child,
            stdin,
            rx,
            start: Instant::now(),
        };
        let now = phone.now();
        phone.controller.begin_control_mode(now);
        let attached = phone.pump_until(15, |c| {
            c.phone_group().is_some() && c.snapshot().active_window_id.is_some()
        });
        assert!(attached, "{:?}", phone.controller.snapshot());
        phone.pump(300);
        phone
    }

    /// A desktop's terminal on `target`, attached.
    fn desktop_on(&self, target: &str) -> Child {
        let before = self
            .tmux(&["list-clients", "-F", "#{client_name}"])
            .lines()
            .count();
        let desk = self.desktop(target);
        assert!(
            eventually(10, || self
                .tmux(&["list-clients", "-F", "#{client_name}"])
                .lines()
                .count()
                > before),
            "the desktop attached"
        );
        desk
    }

    fn window_of(&self, target: &str) -> String {
        self.tmux(&["display-message", "-p", "-t", target, "#{window_id}"])
    }
}

/// B580: "New window" on another session's row (the Home tree) made it
/// with `new-window -t '$T:'`, which makes the new window that session's
/// current one: a desktop on that session was switched to it.
#[test]
#[ignore = "needs tmux; see the module docs"]
fn a_new_window_in_another_session_leaves_its_desktop_where_it_is() {
    let server = Server::start("newwin");
    server.tmux(&["new-session", "-d", "-s", "dev", "-x", "120", "-y", "40"]);
    server.tmux(&["new-window", "-t", "dev"]);
    server.tmux(&["select-window", "-t", "dev:0"]);
    let mut desk = server.desktop_on("dev");
    let dev = server.tmux(&["display-message", "-p", "-t", "dev", "#{session_id}"]);
    let desk_window = server.window_of("dev");
    let mut phone = server.phone("main");
    let before: Vec<String> = server
        .tmux(&["list-windows", "-t", "dev", "-F", "#{window_id}"])
        .lines()
        .map(str::to_owned)
        .collect();
    let now = phone.now();
    phone
        .controller
        .new_window_in_session(&dev, Some("made"), now);
    let landed = phone.pump_until(15, |c| {
        c.snapshot().active_session_id.as_deref() == Some(dev.as_str())
            && c.snapshot()
                .active_window_id
                .as_deref()
                .is_some_and(|w| !before.iter().any(|b| b == w))
    });
    assert!(landed, "{:?}", phone.controller.snapshot());
    phone.pump(300);
    assert_eq!(server.window_of("dev"), desk_window, "the desktop's window");
    let _ = desk.kill();
    let _ = desk.wait();
}

impl Server {
    /// Each client's session, `name session` per line.
    fn clients(&self) -> String {
        self.tmux(&["list-clients", "-F", "#{client_name} #{session_name}"])
    }
}

/// B582: tmux detaches a client whose session is destroyed
/// (`detach-on-destroy`, on by default): the phone killing the session it
/// was on — its group with it — ended control mode, and the phone sat on a
/// dead attach though another session was there.
#[test]
#[ignore = "needs tmux; see the module docs"]
fn killing_the_session_on_screen_leaves_the_phone_on_another() {
    let server = Server::start("killown");
    server.tmux(&["new-session", "-d", "-s", "dev", "-x", "80", "-y", "24"]);
    let mut phone = server.phone("main");
    let main = phone
        .controller
        .snapshot()
        .active_session_id
        .clone()
        .unwrap();
    let dev = server.tmux(&["display-message", "-p", "-t", "=dev:", "#{session_id}"]);
    let now = phone.now();
    phone.controller.kill_session(&main, now);
    let moved = phone.pump_until(10, |c| {
        c.phone_group().is_some() && c.snapshot().active_window_id.is_some()
    });
    assert_eq!(
        phone.child.try_wait().unwrap(),
        None,
        "the phone's client exited"
    );
    assert!(moved, "{:?}", phone.controller.snapshot());
    let clients = server.clients();
    assert!(clients.contains(" dev~mp-"), "clients: {clients}");
    assert_eq!(
        phone.controller.snapshot().active_session_id.as_deref(),
        Some(dev.as_str())
    );
    assert!(
        !server
            .tmux(&["ls", "-F", "#{session_name}"])
            .contains("main")
    );
}

/// B582: a desktop killed the session the phone was on; the phone drops
/// its group (it alone would keep the windows alive) — and with it, by
/// tmux's `detach-on-destroy`, its own control client.
#[test]
#[ignore = "needs tmux; see the module docs"]
fn a_session_killed_under_the_phone_leaves_it_on_another() {
    let server = Server::start("killunder");
    server.tmux(&["new-session", "-d", "-s", "dev", "-x", "80", "-y", "24"]);
    let mut phone = server.phone("main");
    server.tmux(&["kill-session", "-t", "=main"]);
    let moved = phone.pump_until(10, |c| {
        c.phone_group().is_some_and(|g| g.starts_with("dev~mp-"))
            && c.snapshot().active_window_id.is_some()
    });
    assert_eq!(
        phone.child.try_wait().unwrap(),
        None,
        "the phone's client exited"
    );
    assert!(moved, "{:?}", phone.controller.snapshot());
    let clients = server.clients();
    assert!(clients.contains(" dev~mp-"), "clients: {clients}");
    let dev = server.tmux(&["display-message", "-p", "-t", "=dev:", "#{session_id}"]);
    assert_eq!(
        phone.controller.snapshot().active_session_id.as_deref(),
        Some(dev.as_str())
    );
    let names = server.tmux(&["ls", "-F", "#{session_name}"]);
    assert!(!names.contains("main"), "the old group went: {names}");
}
