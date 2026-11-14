//! The control client against a real herdr 0.9.5, on PRIVATE sessions only:
//! every herdr runs as `herdr --session moshpit-w2b-<pid>-<case>` with HOME a
//! fresh directory under /tmp, and the case stops its server and removes the
//! directory when it ends (also on failure). The user's own herdr is never
//! addressed: a guard refuses any command or line that would run herdr
//! without the private session. (A temporary HOME alone is not enough:
//! probed, `HOME=/tmp/x herdr status` finds the user's own default server.)
//!
//!   live_herdr [<case>...]     (no case: all of them)
//!
//! The client is driven in real time as a host would drive it: its execs run
//! through `/bin/sh -c` with stdout and stderr merged, its frame channel and
//! its push channel are interactive zsh shells on PTYs
//! (`harness/client/pty.py`), its timers are ticked. Panes run zsh; `claude`
//! is a copy of /bin/cat, which herdr detects as the claude agent. Each case
//! prints its evidence; `harness/client/live.txt` keeps the last run's.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use moshpit_herdr::client::{
    Action, Client, Config, Event, ExecId, ExecResult, Notice, NoticeAction, Output,
    RestartOutcome, StreamId,
};
use moshpit_herdr::control::{AgentTaskRequest, TaskFailure, TaskOutcome, WorktreeRemoval};
use moshpit_herdr::version::VersionNotice;

const HERDR: &str = "/opt/homebrew/bin/herdr";

type Outcome = Result<(), String>;

/// A live case: its name and what it runs.
type Case = (&'static str, fn() -> Outcome);

fn check(ok: bool, what: &str) -> Outcome {
    if ok { Ok(()) } else { Err(what.to_owned()) }
}

/// A private herdr server and its home, stopped and removed on drop.
struct Private {
    home: PathBuf,
    session: String,
    server: Option<Child>,
}

impl Private {
    fn new(case: &str) -> Result<Self, String> {
        let session = format!("moshpit-w2b-{}-{case}", std::process::id());
        let home = PathBuf::from(format!("/tmp/w2bl-{}-{case}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("bin")).map_err(|e| e.to_string())?;
        std::fs::copy("/bin/cat", home.join("bin/claude")).map_err(|e| e.to_string())?;
        std::fs::write(home.join(".zshrc"), "PROMPT='%% '\nunsetopt BEEP\n")
            .map_err(|e| e.to_string())?;
        let mut private = Self {
            home,
            session,
            server: None,
        };
        // The socket this session resolves to must be under the private
        // home before anything else runs.
        let (status, _) = private.herdr(&["status"]);
        check(
            status.contains(&private.home.display().to_string()),
            &format!("the session's socket is not private: {status}"),
        )?;
        let server = Command::new(HERDR)
            .args(["--session", &private.session, "server"])
            .env_clear()
            .envs(private.env())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|e| e.to_string())?;
        private.server = Some(server);
        for _ in 0..100 {
            if private.herdr(&["api", "snapshot"]).1 == 0 {
                private.herdr(&["workspace", "create", "--focus", "--label", "live"]);
                return Ok(private);
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err("the private server never answered".to_owned())
    }

    fn env(&self) -> Vec<(String, String)> {
        let home = self.home.display().to_string();
        vec![
            ("HOME".to_owned(), home.clone()),
            ("ZDOTDIR".to_owned(), home.clone()),
            (
                "PATH".to_owned(),
                format!("{home}/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"),
            ),
            ("SHELL".to_owned(), "/bin/zsh".to_owned()),
            ("TERM".to_owned(), "xterm-256color".to_owned()),
            ("LANG".to_owned(), "en_US.UTF-8".to_owned()),
        ]
    }

    /// One private herdr command: its merged output and status.
    fn herdr(&self, args: &[&str]) -> (String, i32) {
        let output = Command::new(HERDR)
            .args(["--session", &self.session])
            .args(args)
            .env_clear()
            .envs(self.env())
            .output();
        match output {
            Ok(output) => (
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ),
                output.status.code().unwrap_or(-1),
            ),
            Err(error) => (error.to_string(), -1),
        }
    }

    /// What a pane shows (`pane read`).
    fn read(&self, pane: &str) -> String {
        self.herdr(&["pane", "read", pane, "--format", "text"]).0
    }

    /// The custom path the client runs herdr as.
    fn custom_path(&self) -> String {
        format!("{HERDR} --session {}", self.session)
    }

    fn socket(&self) -> String {
        format!(
            "{}/.config/herdr/sessions/{}/herdr.sock",
            self.home.display(),
            self.session
        )
    }

    fn server_pid(&self) -> Option<String> {
        let output = Command::new("pgrep")
            .args(["-f", &format!("session {} server", self.session)])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .map(str::to_owned)
    }

    fn signal(&self, signal: &str) {
        if let Some(pid) = self.server_pid() {
            let _ = Command::new("kill").args([signal, &pid]).status();
        }
    }

    /// Focus a pane the way a person at the desktop would (a zoom bounce:
    /// `agent focus` refuses a plain shell pane).
    fn focus(&self, pane: &str) {
        self.herdr(&["pane", "zoom", "--pane", pane, "--on"]);
        self.herdr(&["pane", "zoom", "--pane", pane, "--off"]);
    }
}

impl Drop for Private {
    fn drop(&mut self) {
        self.signal("-CONT");
        self.herdr(&["server", "stop"]);
        if let Some(mut server) = self.server.take() {
            for _ in 0..50 {
                if let Ok(Some(_)) = server.try_wait() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            let _ = server.kill();
            let _ = server.wait();
        }
        let (status, _) = self.herdr(&["status"]);
        let stopped = status.contains("not running");
        let _ = std::fs::remove_dir_all(&self.home);
        println!(
            "  private server stopped: {stopped}; {} removed",
            self.home.display()
        );
    }
}

/// Refuse anything that would run herdr outside the private session.
fn guard(text: &str, session: &str) -> Outcome {
    let private = format!(" --session {session}");
    for (index, _) in text.match_indices("herdr") {
        let before = &text[..index];
        let after = &text[index + "herdr".len()..];
        let part_of_a_word = before.ends_with("moshpit-")
            || before.ends_with('.')
            || after.starts_with(".sock")
            || after.starts_with('/')
            // The frame loop's pid file (`herdr-frame-<connection>.pid`).
            || after.starts_with("-frame-");
        let private_binary = before.ends_with('/') && after.starts_with(&private);
        if !part_of_a_word && !private_binary {
            return Err(format!("refused, not the private session: {text}"));
        }
    }
    Ok(())
}

enum Inbound {
    Exec(ExecId, ExecResult),
    Frame(Vec<u8>),
    FrameEnd,
    Push(Vec<u8>),
    PushEnd,
}

/// A shell on a PTY, its output going to `sink`.
struct Shell {
    child: Child,
    stdin: ChildStdin,
}

impl Shell {
    /// An interactive zsh on a PTY, or `program` run by `sh -c` on one
    /// (a channel's own program).
    fn spawn(
        private: &Private,
        program: Option<&str>,
        sink: Sender<Inbound>,
        wrap: fn(Vec<u8>) -> Inbound,
        end: fn() -> Inbound,
    ) -> Self {
        let pty = concat!(env!("CARGO_MANIFEST_DIR"), "/harness/client/pty.py");
        let argv: Vec<&str> = match program {
            Some(program) => vec!["/bin/sh", "-c", program],
            None => vec!["/bin/zsh", "-i"],
        };
        let mut child = Command::new("python3")
            .args(["-I", pty])
            .args(argv)
            .env_clear()
            .envs(private.env())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("python3 runs");
        let stdin = child.stdin.take().expect("piped");
        let mut stdout = child.stdout.take().expect("piped");
        thread::spawn(move || {
            let mut buffer = [0u8; 65536];
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) | Err(_) => {
                        let _ = sink.send(end());
                        return;
                    }
                    Ok(n) => {
                        if sink.send(wrap(buffer[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Self { child, stdin }
    }

    fn write(&mut self, bytes: &[u8]) {
        let _ = self.stdin.write_all(bytes);
        let _ = self.stdin.flush();
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        kill_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// SIGKILL a process group (each exec and shell runs in its own).
fn kill_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-9", &format!("-{pid}")])
        .stderr(Stdio::null())
        .status();
}

/// The client on the private server, in real time.
struct Live<'a> {
    private: &'a Private,
    client: Client,
    start: Instant,
    sink: Sender<Inbound>,
    inbound: Receiver<Inbound>,
    frame: Option<Shell>,
    push: Option<Shell>,
    pids: BTreeMap<u64, u32>,
    events: Vec<(u64, Event)>,
    execs: Vec<(u64, String)>,
    frame_writes: Vec<String>,
    screens: BTreeMap<StreamId, Vec<u8>>,
    refused: Option<String>,
}

impl<'a> Live<'a> {
    fn new(private: &'a Private, config: Config) -> Self {
        let (sink, inbound) = channel();
        let frame = config.frame_channel.then(|| {
            Shell::spawn(private, None, sink.clone(), Inbound::Frame, || {
                Inbound::FrameEnd
            })
        });
        Self {
            private,
            client: Client::new(config),
            start: Instant::now(),
            sink,
            inbound,
            frame,
            push: None,
            pids: BTreeMap::new(),
            events: Vec::new(),
            execs: Vec::new(),
            frame_writes: Vec::new(),
            screens: BTreeMap::new(),
            refused: None,
        }
    }

    fn now(&self) -> u64 {
        self.start.elapsed().as_millis() as u64 + 1
    }

    fn start(&mut self) {
        // Let the PTY's shell print its prompt first, as a login does.
        self.run(700);
        let now = self.now();
        self.client.start(now);
        self.pump();
    }

    fn pump(&mut self) {
        while let Some(output) = self.client.poll_output() {
            let now = self.now();
            match output {
                Output::Exec { id, command } => self.spawn_exec(id, command),
                Output::Abandon { id } => {
                    if let Some(pid) = self.pids.remove(&id.0) {
                        kill_group(pid);
                    }
                }
                Output::FrameWrite(line) => {
                    if let Err(error) = guard(&line, &self.private.session) {
                        self.refused = Some(error);
                        continue;
                    }
                    self.frame_writes.push(line.clone());
                    if let Some(frame) = self.frame.as_mut() {
                        frame.write(format!("{line}\n").as_bytes());
                    }
                }
                Output::OpenPush { program } => {
                    if let Err(error) = guard(&program, &self.private.session) {
                        self.refused = Some(error);
                        continue;
                    }
                    // The pump is the channel's program (B267).
                    let shell = Shell::spawn(
                        self.private,
                        Some(&program),
                        self.sink.clone(),
                        Inbound::Push,
                        || Inbound::PushEnd,
                    );
                    self.push = Some(shell);
                    self.client.push_opened(self.now());
                }
                Output::PushWrite(bytes) => {
                    if let Err(error) =
                        guard(&String::from_utf8_lossy(&bytes), &self.private.session)
                    {
                        self.refused = Some(error);
                        continue;
                    }
                    if let Some(push) = self.push.as_mut() {
                        push.write(&bytes);
                    }
                }
                Output::ClosePush => self.push = None,
                Output::PaneBytes { stream, bytes, .. } => {
                    self.screens.entry(stream).or_default().extend(bytes);
                }
                Output::Event(event) => self.events.push((now, event)),
                Output::StreamOpened { .. } | Output::StreamClosed { .. } | Output::View { .. } => {
                }
            }
        }
    }

    fn spawn_exec(&mut self, id: ExecId, command: String) {
        let now = self.now();
        if let Err(error) = guard(&command, &self.private.session) {
            self.refused = Some(error.clone());
            self.client
                .exec_finished(id, ExecResult::Failed(error), now);
            return;
        }
        self.execs.push((now, command.clone()));
        let child = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("exec 2>&1; {command}"))
            .env_clear()
            .envs(self.private.env())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
        let sink = self.sink.clone();
        match child {
            Err(error) => {
                let _ = sink.send(Inbound::Exec(id, ExecResult::Failed(error.to_string())));
            }
            Ok(mut child) => {
                self.pids.insert(id.0, child.id());
                let mut stdout = child.stdout.take().expect("piped");
                thread::spawn(move || {
                    let mut out = Vec::new();
                    let _ = stdout.read_to_end(&mut out);
                    let _ = child.wait();
                    let _ = sink.send(Inbound::Exec(id, ExecResult::Output(out)));
                });
            }
        }
    }

    fn handle(&mut self, message: Inbound) {
        let now = self.now();
        match message {
            Inbound::Exec(id, result) => {
                self.pids.remove(&id.0);
                self.client.exec_finished(id, result, now);
            }
            Inbound::Frame(bytes) => self.client.frame_bytes(&bytes, now),
            Inbound::FrameEnd => self.client.frame_closed(now),
            Inbound::Push(bytes) => self.client.push_bytes(&bytes, now),
            Inbound::PushEnd => self.client.push_closed(now),
        }
    }

    /// Run for `ms`, in real time.
    fn run(&mut self, ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms);
        loop {
            self.pump();
            let now = Instant::now();
            if now >= until {
                return;
            }
            let mut wait = until - now;
            if let Some(deadline) = self.client.next_deadline() {
                let at = self.start + Duration::from_millis(deadline.saturating_sub(1));
                wait = wait.min(at.saturating_duration_since(now));
            }
            match self.inbound.recv_timeout(wait) {
                Ok(message) => self.handle(message),
                Err(RecvTimeoutError::Timeout) => {
                    let now = self.now();
                    self.client.tick(now);
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Run until `done` holds, at most `ms`.
    fn until(&mut self, ms: u64, what: &str, done: impl Fn(&Self) -> bool) -> Outcome {
        let deadline = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < deadline {
            if done(self) {
                return Ok(());
            }
            self.run(50);
        }
        if done(self) {
            Ok(())
        } else {
            Err(format!("timed out waiting for: {what}"))
        }
    }

    fn act(&mut self, action: Action) -> Option<moshpit_herdr::client::RequestId> {
        let now = self.now();
        let request = self.client.act(action, now);
        self.pump();
        request
    }

    fn type_text(&mut self, pane: &str, text: &str) {
        let now = self.now();
        self.client.input(pane, text.as_bytes(), now);
        self.pump();
    }

    fn type_bytes(&mut self, pane: &str, bytes: &[u8]) {
        let now = self.now();
        self.client.input(pane, bytes, now);
        self.pump();
    }

    fn saw(&self, wanted: impl Fn(&Event) -> bool) -> bool {
        self.events.iter().any(|(_, event)| wanted(event))
    }

    fn sent(&self, fragment: &str) -> usize {
        self.execs
            .iter()
            .filter(|(_, command)| command.contains(fragment))
            .count()
    }

    fn task_outcome(&self) -> Option<TaskOutcome> {
        self.events.iter().find_map(|(_, event)| match event {
            Event::Task { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
    }

    fn finish(&self) -> Outcome {
        match &self.refused {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

impl Drop for Live<'_> {
    fn drop(&mut self) {
        let now = self.now();
        self.client.close(true, now);
        self.pump();
        for pid in self.pids.values() {
            kill_group(*pid);
        }
    }
}

fn config(private: &Private) -> Config {
    Config {
        custom_path: Some(private.custom_path()),
        socket_path: private.socket(),
        ..Config::default()
    }
}

/// The client of one connection (one device's): what its frame loop's pid
/// file is named after.
fn config_as(private: &Private, connection: &str) -> Config {
    Config {
        connection_id: connection.to_owned(),
        ..config(private)
    }
}

/// The client's banner, as its variant's name (`OnAnotherDevice`, …).
fn notice_of(live: &Live) -> String {
    live.client
        .notice()
        .map(|(notice, action)| format!("{notice:?} {action:?}"))
        .unwrap_or_default()
}

/// Both devices' clients, in turns, for `ms`.
fn run_both(one: &mut Live, two: &mut Live, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        one.run(25);
        two.run(25);
    }
}

/// Attach lines a client wrote, and how many asked for `--takeover`.
fn attach_lines(live: &Live) -> (usize, usize) {
    let lines: Vec<&String> = live
        .frame_writes
        .iter()
        .filter(|line| !line.starts_with('{'))
        .collect();
    let taking = lines
        .iter()
        .filter(|line| line.ends_with(" take") || line.ends_with("--takeover"))
        .count();
    (lines.len(), taking)
}

/// Started and painting; the focused pane's id.
fn attached(live: &mut Live) -> Result<String, String> {
    live.start();
    live.until(10_000, "the first frame", |live| {
        live.saw(|event| *event == Event::FirstFrame)
    })?;
    live.client
        .attached_pane()
        .map(str::to_owned)
        .ok_or_else(|| "no attached pane".to_owned())
}

// MARK: - Cases

fn attach_and_type() -> Outcome {
    let private = Private::new("type")?;
    let mut live = Live::new(&private, config(&private));
    let pane = attached(&mut live)?;
    println!(
        "  attached {pane}; boot line: {} write(s)",
        live.frame_writes.len()
    );
    check(live.frame_writes.len() == 1, "one boot line")?;
    check(
        live.frame_writes[0].contains("terminal session control \"$1\""),
        "the boot line runs the reader loop",
    )?;
    live.type_text(&pane, "echo typed-$((40+2))\r");
    live.until(3_000, "typed-42", |live| {
        live.private.read(&pane).contains("typed-42")
    })?;
    // Typing, a semantic arrow, typing: in the order pressed (B2).
    live.type_text(&pane, "cat -v\r");
    live.run(600);
    live.type_text(&pane, "ab");
    live.type_bytes(&pane, b"\x1b[A");
    live.type_text(&pane, "cd\r");
    live.until(3_000, "ab, up, cd", |live| {
        let screen = live.private.read(&pane);
        screen.contains("ab^[[Acd") || screen.contains("ab^[OAcd")
    })?;
    println!(
        "  keys: `pane send-keys` ran {} time(s), between the typing",
        live.sent("pane send-keys")
    );
    live.type_bytes(&pane, b"\x03");
    live.run(500);
    // A paste carrying its own end marker stays one paste (B114): nothing in
    // it runs at zsh's prompt.
    let now = live.now();
    live.client
        .paste(&pane, "x\u{1b}[201~\necho INJ$((40+2))\n", now);
    live.pump();
    live.run(1_500);
    let screen = live.private.read(&pane);
    check(!screen.contains("INJ42"), "the pasted command did not run")?;
    println!("  paste with an inner end marker: not run (INJ42 absent)");
    live.type_bytes(&pane, b"\x03");
    live.run(300);
    // A line herdr turns away on a live attach (B117): written past the
    // client, the way a line meant for a dying attach reaches a live one.
    if let Some(frame) = live.frame.as_mut() {
        frame.write(b"{\"type\":\"terminal.resize\",\"cols\":70000,\"rows\":24}\n");
    }
    live.until(2_000, "input ignored", |live| {
        live.saw(|event| matches!(event, Event::InputIgnored(_)))
    })?;
    check(
        live.client.attached_pane() == Some(pane.as_str()),
        "the attach kept",
    )?;
    check(live.client.notice().is_none(), "no notice, no backoff")?;
    live.type_text(&pane, "echo kept-$((6*7))\r");
    live.until(3_000, "kept-42", |live| {
        live.private.read(&pane).contains("kept-42")
    })?;
    println!("  `input ignored` from herdr: attach kept, no notice, typing goes on");
    live.finish()
}

fn switch_panes() -> Outcome {
    let private = Private::new("switch")?;
    let first = private.herdr(&["api", "snapshot"]).0;
    let p1 = moshpit_herdr::snapshot::decode(&first)
        .and_then(|decoded| decoded.snapshot.active_pane_id)
        .ok_or("no pane")?;
    private.herdr(&["pane", "split", &p1, "--direction", "right"]);
    private.focus(&p1);
    let mut live = Live::new(&private, config(&private));
    let pane = attached(&mut live)?;
    check(pane == p1, "attached to the focused pane")?;
    let p2 = live
        .client
        .snapshot()
        .panes
        .keys()
        .find(|id| **id != p1)
        .cloned()
        .ok_or("no second pane")?;
    live.act(Action::SelectPane(p2.clone()));
    // Typed right after the pick: at the new pane (waits for it), and at
    // the pane being left (goes nowhere, and says so).
    live.type_text(&p2, "echo at-new-$((1+1))\r");
    live.type_text(&p1, "echo at-old-$((2+2))\r");
    let typed = live.until(4_000, "at-new-2 in the new pane", |live| {
        live.private.read(&p2).contains("at-new-2")
    });
    if typed.is_err() {
        for line in &live.frame_writes {
            println!("  write: {}", line.chars().take(100).collect::<String>());
        }
        for (at, event) in &live.events {
            if !matches!(event, Event::StateChanged | Event::AgentHooksUpdated) {
                println!("  {at} {event:?}");
            }
        }
        println!("  p2 shows: {:?}", private.read(&p2));
    }
    typed?;
    live.run(1_000);
    check(
        !private.read(&p1).contains("at-old-4"),
        "nothing reached the pane left",
    )?;
    check(
        live.saw(|event| *event == Event::InputDropped { pane: p1.clone() }),
        "the drop was said",
    )?;
    let releases = live
        .frame_writes
        .iter()
        .filter(|line| line.contains("terminal.release"))
        .count();
    check(releases == 1, "one release")?;
    let focused = moshpit_herdr::snapshot::decode(&private.herdr(&["api", "snapshot"]).0)
        .and_then(|decoded| decoded.snapshot.active_pane_id);
    check(
        focused.as_deref() == Some(p2.as_str()),
        "the server's focus followed",
    )?;
    println!(
        "  {p1} -> {p2}: new pane got its typing, the old one none; 1 release; server focus {p2}"
    );
    // Three quick switches still write one release.
    live.act(Action::SelectPane(p1.clone()));
    live.act(Action::SelectPane(p2.clone()));
    live.act(Action::SelectPane(p1.clone()));
    live.until(4_000, "back on the first pane", |live| {
        live.client.attached_pane() == Some(p1.as_str())
    })?;
    let releases = live
        .frame_writes
        .iter()
        .filter(|line| line.contains("terminal.release"))
        .count();
    println!("  three quick switches: {} release(s) in all", releases);
    check(releases == 2, "one release for the three")?;
    live.type_text(&p1, "echo after-$((3+3))\r");
    live.until(3_000, "after-6", |live| {
        live.private.read(&p1).contains("after-6")
    })?;
    live.finish()
}

fn pipelined_retarget() -> Outcome {
    // The attach line written right behind the release, as an ack that
    // never comes would have it: the dying attach can read it first and
    // says `input ignored` (B117). The attach is not given up, and the
    // switch recovers.
    let private = Private::new("pipe")?;
    let first = private.herdr(&["api", "snapshot"]).0;
    let p1 = moshpit_herdr::snapshot::decode(&first)
        .and_then(|decoded| decoded.snapshot.active_pane_id)
        .ok_or("no pane")?;
    private.herdr(&["pane", "split", &p1, "--direction", "right"]);
    private.focus(&p1);
    let mut config = config(&private);
    config.budgets.release_ack = 1;
    let mut live = Live::new(&private, config);
    attached(&mut live)?;
    let p2 = live
        .client
        .snapshot()
        .panes
        .keys()
        .find(|id| **id != p1)
        .cloned()
        .ok_or("no second pane")?;
    live.act(Action::SelectPane(p2.clone()));
    live.type_text(&p2, "echo piped-$((5+5))\r");
    live.until(
        12_000,
        "attached to the new pane and typing there",
        |live| live.private.read(&p2).contains("piped-10"),
    )?;
    let ignored = live.saw(|event| matches!(event, Event::InputIgnored(_)));
    println!(
        "  input ignored seen: {ignored}; target kept: {:?}; notice: {:?}",
        live.client.frame_target(),
        live.client.notice().map(|(notice, _)| notice.clone())
    );
    check(
        live.client.frame_target() == Some(p2.as_str()),
        "the target is kept",
    )?;
    check(
        !matches!(
            live.client.notice().map(|(notice, _)| notice.clone()),
            Some(Notice::Refused(_))
        ),
        "no refusal notice",
    )?;
    live.finish()
}

fn takeover() -> Outcome {
    let private = Private::new("take")?;
    let mut live = Live::new(&private, config(&private));
    let pane = attached(&mut live)?;
    // Another client takes the pane.
    let mut other = Command::new(HERDR)
        .args([
            "--session",
            &private.session,
            "terminal",
            "session",
            "control",
            &pane,
            "--cols",
            "80",
            "--rows",
            "24",
            "--takeover",
        ])
        .env_clear()
        .envs(private.env())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let lines = live.frame_writes.len();
    // Evicted: said, with "Open here", and never asked for again on the
    // client's own (it used to be, on the next read, `--takeover`).
    live.until(6_000, "the pane said to be on another device", |live| {
        notice_of(live).contains("OnAnotherDevice")
    })?;
    live.run(6_000);
    check(
        live.frame_writes.len() == lines,
        &format!(
            "asked for the pane again on its own: {:?}",
            &live.frame_writes[lines..]
        ),
    )?;
    println!(
        "  evicted by another client's --takeover: {}; nothing asked for 6 s",
        notice_of(&live)
    );
    let _ = other.kill();
    let _ = other.wait();
    // "Open here".
    live.act(Action::NoticeAction);
    live.until(6_000, "attached again", |live| {
        live.client.attached_pane() == Some(pane.as_str())
    })?;
    println!("  Open here: {:?}", &live.frame_writes[lines..]);
    live.type_text(&pane, "echo back-$((7+7))\r");
    live.until(3_000, "back-14", |live| {
        live.private.read(&pane).contains("back-14")
    })?;
    // A pane closed under the attach is not another client.
    // The desktop's split is seen on the next read: up to the idle cadence
    // (8 s) after a quiet while.
    private.herdr(&["pane", "split", &pane, "--direction", "right", "--focus"]);
    live.until(12_000, "on the new pane", |live| {
        live.client
            .attached_pane()
            .is_some_and(|attached| attached != pane)
    })?;
    let doomed = live.client.attached_pane().unwrap_or_default().to_owned();
    private.herdr(&["pane", "close", &doomed]);
    live.until(12_000, "back on the remaining pane", |live| {
        live.client.attached_pane() == Some(pane.as_str())
    })?;
    check(
        !notice_of(&live).contains("OnAnotherDevice"),
        "a closed pane read as another device's",
    )?;
    println!("  re-attached and typing; a closed pane read as gone, not as another device's");
    live.finish()
}

/// Round twenty-eight: two devices on one herdr — this client and another,
/// each with its frame channel on a PTY of its own and a connection of its
/// own. A follow never takes a pane from the device that has it; the person
/// takes it ("Open here"); and nothing is taken back on its own.
fn two_devices() -> Outcome {
    let private = Private::new("two")?;
    let mut one = Live::new(&private, config_as(&private, "DEVICE-ONE"));
    let first = attached(&mut one)?;
    let (split, _) = private.herdr(&["pane", "split", &first, "--direction", "right"]);
    let second = split
        .split('"')
        .skip_while(|word| *word != "pane_id")
        .nth(2)
        .ok_or_else(|| format!("no new pane: {split}"))?
        .to_owned();
    // The other device comes up on the server's focus — the pane this
    // device has.
    let mut two = Live::new(&private, config_as(&private, "DEVICE-TWO"));
    two.start();
    two.until(6_000, "the other device's first attach answered", |two| {
        !notice_of(two).is_empty() || two.client.attached_pane().is_some()
    })?;
    let came_up = notice_of(&two);
    run_both(&mut one, &mut two, 3_000);
    let one_kept = one.client.attached_pane() == Some(first.as_str());
    println!(
        "  the other device came up on {first}: {}; 3 s later this device has it: {one_kept}; \
         attach lines (all, taking over): this device {:?}, the other {:?}; this device says {:?}",
        if came_up.is_empty() {
            "TOOK it".to_owned()
        } else {
            came_up.clone()
        },
        attach_lines(&one),
        attach_lines(&two),
        notice_of(&one)
    );
    // Its person picks the other pane: theirs, `--takeover`; the shared
    // focus moves there.
    two.act(Action::SelectPane(second.clone()));
    two.until(6_000, "the other device on its pane", |two| {
        two.client.attached_pane() == Some(second.as_str())
    })?;
    // This device's poll follows the focus onto it: plain, refused.
    let before = attach_lines(&one);
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline && !notice_of(&one).contains("OnAnotherDevice") {
        run_both(&mut one, &mut two, 200);
    }
    run_both(&mut one, &mut two, 2_000);
    let followed = attach_lines(&one);
    let two_kept = two.client.attached_pane() == Some(second.as_str());
    two.type_text(&second, "echo two-$((20+22))\r");
    two.until(3_000, "two-42", |two| {
        two.private.read(&second).contains("two-42")
    })?;
    println!(
        "  this device followed the focus onto {second}: {} attach line(s), {} taking over; it says {:?}; \
         the other device kept it: {two_kept}, its typing reached it",
        followed.0 - before.0,
        followed.1 - before.1,
        notice_of(&one)
    );
    check(
        two_kept,
        "the follow took the pane from the device that has it",
    )?;
    check(
        notice_of(&one).contains("OnAnotherDevice"),
        "this device didn't say the pane is on another device",
    )?;
    // This device's person: "Open here".
    one.act(Action::NoticeAction);
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline && one.client.attached_pane() != Some(second.as_str()) {
        run_both(&mut one, &mut two, 200);
    }
    check(
        one.client.attached_pane() == Some(second.as_str()),
        "Open here didn't take the pane",
    )?;
    // The other device: said, and never taken back on its own.
    let two_before = attach_lines(&two);
    run_both(&mut one, &mut two, 8_000);
    let two_after = attach_lines(&two);
    one.type_text(&second, "echo one-$((40+2))\r");
    one.until(3_000, "one-42", |one| {
        one.private.read(&second).contains("one-42")
    })?;
    println!(
        "  Open here took {second}; the other device says {:?} and asked {} time(s) in 8 s",
        notice_of(&two),
        two_after.0 - two_before.0
    );
    check(
        two_after.0 == two_before.0,
        "the other device took the pane back on its own",
    )?;
    check(
        one.client.attached_pane() == Some(second.as_str()),
        "this device lost the pane again",
    )?;
    one.finish()?;
    two.finish()
}

/// A connection that dropped without a word leaves its frame loop and its
/// attach on the host, holding the pane, until sshd notices. The next
/// connection's boot line stops this connection's ghost — and only this
/// connection's: another device's attach is no ghost.
fn ghost() -> Outcome {
    let private = Private::new("ghost")?;
    let mut first = Live::new(&private, config_as(&private, "DEVICE-ONE"));
    let pane = attached(&mut first)?;
    // The connection drops: its shell (and the attach in it) stay.
    let ghost = first.frame.take();
    drop(first);
    let mut again = Live::new(&private, config_as(&private, "DEVICE-ONE"));
    let back = attached(&mut again)?;
    check(back == pane, &format!("came back on {back}, not {pane}"))?;
    check(
        !notice_of(&again).contains("OnAnotherDevice"),
        "this device's own ghost was taken for another device",
    )?;
    println!(
        "  reconnected onto {pane}: attached, its ghost stopped ({:?})",
        attach_lines(&again)
    );
    // Another device's connection: it stops no ghost of ours, and is told
    // the pane is open here.
    let mut other = Live::new(&private, config_as(&private, "DEVICE-TWO"));
    other.start();
    other.until(6_000, "the other device's first attach answered", |other| {
        !notice_of(other).is_empty() || other.client.attached_pane().is_some()
    })?;
    run_both(&mut again, &mut other, 1_000);
    check(
        again.client.attached_pane() == Some(pane.as_str()),
        "another device's connection stopped this one's attach",
    )?;
    println!(
        "  another device's connection: {:?}; this one still on {pane}",
        notice_of(&other)
    );
    drop(ghost);
    again.finish()?;
    other.finish()
}

fn agent_task() -> Outcome {
    let private = Private::new("task")?;
    let repo = private.home.join("repo");
    std::fs::create_dir_all(&repo).map_err(|e| e.to_string())?;
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=live",
            "-c",
            "user.email=live@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    ] {
        Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .env_clear()
            .envs(private.env())
            .status()
            .map_err(|e| e.to_string())?;
    }
    let mut live = Live::new(&private, config(&private));
    attached(&mut live)?;
    live.act(Action::StartAgentTask(AgentTaskRequest {
        repo_path: repo.display().to_string(),
        branch: "live-task".to_owned(),
        agent: "claude".to_owned(),
        prompt: "hello from the task".to_owned(),
    }));
    live.until(45_000, "the task's outcome", |live| {
        live.task_outcome().is_some()
    })?;
    let outcome = live.task_outcome();
    println!(
        "  task: {outcome:?}; prompts tried: {}",
        live.sent("agent prompt")
    );
    check(outcome == Some(TaskOutcome::Started), "the task started")?;
    let workspace = live
        .client
        .snapshot()
        .sessions
        .values()
        .find(|session| session.name == "live-task")
        .map(|session| session.id.clone())
        .ok_or("no task workspace")?;
    let pane = live
        .client
        .snapshot()
        .panes
        .values()
        .find(|pane| {
            live.client
                .snapshot()
                .windows
                .get(&pane.window_id)
                .is_some_and(|window| window.session_id == workspace)
        })
        .map(|pane| pane.id.clone())
        .ok_or("no task pane")?;
    live.until(3_000, "the first message in the agent", |live| {
        live.private.read(&pane).contains("hello from the task")
    })?;
    // Dirty the checkout: removal needs force, then removes it.
    let checkout = live
        .client
        .working_directory(&pane)
        .map(str::to_owned)
        .ok_or("no checkout")?;
    std::fs::write(PathBuf::from(&checkout).join("dirty.txt"), "x").map_err(|e| e.to_string())?;
    live.act(Action::RemoveWorktree {
        session: workspace.clone(),
        force: false,
    });
    live.until(10_000, "the first removal", |live| {
        live.saw(|event| matches!(event, Event::Removal { .. }))
    })?;
    check(
        live.saw(|event| {
            matches!(
                event,
                Event::Removal {
                    outcome: WorktreeRemoval::NeedsForce,
                    ..
                }
            )
        }),
        "a dirty checkout needs force",
    )?;
    live.act(Action::RemoveWorktree {
        session: workspace,
        force: true,
    });
    live.until(10_000, "the forced removal", |live| {
        live.saw(|event| {
            matches!(
                event,
                Event::Removal {
                    outcome: WorktreeRemoval::Removed,
                    ..
                }
            )
        })
    })?;
    check(!PathBuf::from(&checkout).exists(), "the checkout is gone")?;
    println!("  first message in the agent; removal: needs force, then removed");
    live.finish()
}

fn half_open() -> Outcome {
    let private = Private::new("halfopen")?;
    let mut config = config(&private);
    config.budgets.poll = 3_000;
    config.budgets.worktree_create = 6_000;
    config.budgets.worktree_remove = 5_000;
    config.budgets.discovery = 4_000;
    let mut live = Live::new(&private, config);
    let pane = attached(&mut live)?;
    // The server stops answering: requests reach its socket, nothing comes
    // back (a wedged server, a half-open link).
    private.signal("-STOP");
    let stopped = Instant::now();
    let reads_before = live.sent("api snapshot");
    live.act(Action::StartAgentTask(AgentTaskRequest {
        repo_path: private.home.display().to_string(),
        branch: "never".to_owned(),
        agent: "claude".to_owned(),
        prompt: String::new(),
    }));
    live.act(Action::RemoveWorktree {
        session: "w9".to_owned(),
        force: false,
    });
    live.act(Action::AgentNames);
    live.until(15_000, "every answer", |live| {
        live.task_outcome().is_some()
            && live.saw(|event| matches!(event, Event::Removal { .. }))
            && live.saw(|event| matches!(event, Event::Agents { .. }))
    })?;
    let took = stopped.elapsed().as_millis();
    println!(
        "  stopped server: task {:?}, removal {:?}, agents answered {:?} — all within {took} ms",
        live.task_outcome(),
        live.events.iter().find_map(|(_, e)| match e {
            Event::Removal { outcome, .. } => Some(outcome.clone()),
            _ => None,
        }),
        live.events.iter().find_map(|(_, e)| match e {
            Event::Agents { answered, .. } => Some(*answered),
            _ => None,
        }),
    );
    check(
        matches!(
            live.task_outcome(),
            Some(TaskOutcome::Failed(
                TaskFailure::HostStoppedAnswering { .. }
            ))
        ),
        "the task says the host stopped answering",
    )?;
    // The reads go on, each given up at its budget.
    live.until(15_000, "two more reads", |live| {
        live.sent("api snapshot") >= reads_before + 2
    })?;
    println!(
        "  reads kept going while stopped: {} in {} ms",
        live.sent("api snapshot") - reads_before,
        stopped.elapsed().as_millis()
    );
    private.signal("-CONT");
    let resumed = live.client.last_inbound_at();
    live.until(8_000, "an answer again", |live| {
        live.client.last_inbound_at() > resumed
    })?;
    live.type_text(&pane, "echo resumed-$((8+8))\r");
    live.until(4_000, "resumed-16", |live| {
        live.private.read(&pane).contains("resumed-16")
    })?;
    let late = live
        .client
        .snapshot()
        .sessions
        .values()
        .any(|session| session.name == "never");
    println!("  resumed; the abandoned create landed anyway after all: {late}");
    live.finish()
}

fn restart() -> Outcome {
    // B408: a server that already runs the host's herdr is left running:
    // the restart is refused with nothing done (`server stop` exits every
    // pane's process). The stop and start themselves: skew_server_older.
    let private = Private::new("restart")?;
    let mut live = Live::new(&private, config(&private));
    let pane = attached(&mut live)?;
    let before = private.server_pid();
    let request = live.act(Action::RestartServer).ok_or("no request")?;
    live.until(30_000, "the restart's outcome", |live| {
        live.saw(|event| matches!(event, Event::Restart { request: r, .. } if *r == request))
    })?;
    let outcome = live.events.iter().find_map(|(_, event)| match event {
        Event::Restart { outcome, .. } => Some(outcome.clone()),
        _ => None,
    });
    let after = private.server_pid();
    println!("  restart: {outcome:?}; server pid {before:?} -> {after:?}");
    check(
        outcome == Some(RestartOutcome::Unneeded),
        "nothing to restart",
    )?;
    check(
        before == after && after.is_some(),
        "the server left running",
    )?;
    check(live.client.notice().is_none(), "no notice")?;
    live.type_text(&pane, "echo still-$((9+9))\r");
    live.until(4_000, "still-18", |live| {
        live.private.read(&pane).contains("still-18")
    })?;
    println!("  still on {pane} and typing");
    live.finish()
}

/// A shim `herdr` that tells the client a version story (as herdr 0.8.0
/// and a mismatched server print them, from the app's captures) and runs
/// the real private herdr for everything else.
fn skew_shim(
    private: &Private,
    client: &str,
    server: &str,
    mismatch: &str,
) -> Result<String, String> {
    let dir = private.home.join("skewbin");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let shim = dir.join("herdr");
    let script = format!(
        "#!/bin/sh\ncase \" $* \" in\n*\" status \"*) printf 'client:\\n  version: {client}\\n  channel: stable\\n\\nserver:\\n  status: running\\n  version: {server}\\n'; exit 0;;\n*\" api snapshot \"*) printf '%s\\n' '{{\"id\":\"cli:api:snapshot\",\"error\":{{\"code\":\"protocol_mismatch\",\"message\":\"{mismatch}\"}}}}'; exit 1;;\nesac\nexec {HERDR} \"$@\"\n"
    );
    std::fs::write(&shim, script).map_err(|e| e.to_string())?;
    Command::new("chmod")
        .args(["+x", &shim.display().to_string()])
        .status()
        .map_err(|e| e.to_string())?;
    Ok(format!("{} --session {}", shim.display(), private.session))
}

fn skew_client_older() -> Outcome {
    let private = Private::new("skewold")?;
    let mut config = config(&private);
    config.custom_path = Some(skew_shim(
        &private,
        "0.8.0",
        "0.9.5",
        "client protocol 19 is older than server protocol 22; upgrade the Herdr client before using this command",
    )?);
    let mut live = Live::new(&private, config);
    live.start();
    live.until(8_000, "the skew named", |live| {
        matches!(live.client.notice(), Some((Notice::Skew(_), _)))
    })?;
    let (notice, action) = live
        .client
        .notice()
        .map(|(notice, action)| (notice.clone(), action))
        .ok_or("no notice")?;
    println!("  notice: {:?} / {:?}", notice.english(), action);
    check(
        notice
            == Notice::Skew(VersionNotice::ClientOlder {
                client: "0.8.0".to_owned(),
                server: "0.9.5".to_owned(),
            }),
        "the client is named older",
    )?;
    check(action.is_none(), "no restart offered")?;
    let pid = private.server_pid();
    live.act(Action::RestartServer);
    live.until(8_000, "the restart refused", |live| {
        live.saw(|event| {
            matches!(
                event,
                Event::Restart {
                    outcome: RestartOutcome::WouldDowngrade(_),
                    ..
                }
            )
        })
    })?;
    check(live.sent("server stop") == 0, "`server stop` never sent")?;
    check(private.server_pid() == pid, "the server untouched")?;
    println!("  restart refused (would downgrade); `server stop` never sent; server pid unchanged");
    live.finish()
}

fn skew_server_older() -> Outcome {
    let private = Private::new("skewnew")?;
    let mut config = config(&private);
    config.custom_path = Some(skew_shim(
        &private,
        "0.9.5",
        "0.8.0",
        "client protocol 22 is newer than server protocol 19; run herdr server stop",
    )?);
    let mut live = Live::new(&private, config);
    live.start();
    live.until(8_000, "the skew named", |live| {
        matches!(live.client.notice(), Some((Notice::Skew(_), _)))
    })?;
    let (notice, action) = live
        .client
        .notice()
        .map(|(notice, action)| (notice.clone(), action))
        .ok_or("no notice")?;
    println!("  notice: {:?} / {:?}", notice.english(), action);
    check(
        action == Some(NoticeAction::RestartServer),
        "restart offered",
    )?;
    let pid = private.server_pid();
    live.act(Action::NoticeAction);
    live.until(30_000, "the restart's outcome", |live| {
        live.saw(|event| matches!(event, Event::Restart { .. }))
    })?;
    let outcome = live.events.iter().find_map(|(_, event)| match event {
        Event::Restart { outcome, .. } => Some(outcome.clone()),
        _ => None,
    });
    let after = private.server_pid();
    println!(
        "  restart: {outcome:?} (the shim still refuses); server pid {pid:?} -> {after:?}; notice {:?}",
        live.client
            .notice()
            .map(|(notice, action)| (notice.english(), action))
    );
    check(
        outcome == Some(RestartOutcome::Failed),
        "the restart didn't take",
    )?;
    check(
        pid != after && after.is_some(),
        "the real server was restarted",
    )?;
    check(
        live.client
            .notice()
            .map(|(notice, action)| (notice.clone(), action))
            == Some((Notice::RestartFailed, Some(NoticeAction::Retry))),
        "says so, with Retry",
    )?;
    live.finish()
}

fn push() -> Outcome {
    let private = Private::new("push")?;
    let mut config = config(&private);
    config.push = true;
    let mut live = Live::new(&private, config);
    attached(&mut live)?;
    live.until(20_000, "push active", |live| live.client.push_active())?;
    // Let the cadence ease off: the push channel carries the news now.
    live.run(9_000);
    let interval = live.client.poll_interval_ms();
    let reads = live.sent("api snapshot");
    let made = Instant::now();
    private.herdr(&["tab", "create", "--label", "pushed"]);
    live.until(3_000, "the new tab in the tree", |live| {
        live.client
            .snapshot()
            .windows
            .values()
            .any(|window| window.name == "pushed")
    })?;
    println!(
        "  push active; cadence {interval} ms; the new tab seen {} ms after it was made ({} read(s))",
        made.elapsed().as_millis(),
        live.sent("api snapshot") - reads
    );
    // The channel dies: the poll is back in charge at once.
    live.push = None;
    live.until(3_000, "push gone", |live| !live.client.push_active())?;
    check(
        live.client.poll_interval_ms() == 2_000,
        "fast cadence again",
    )?;
    println!(
        "  push channel gone: cadence {} ms",
        live.client.poll_interval_ms()
    );
    live.finish()
}

/// A herdr whose `workspace focus` takes `seconds` before it runs: a step
/// that outlasts the command lane's hold (5 s).
fn slow_focus_shim(private: &Private, seconds: u64) -> Result<String, String> {
    let dir = private.home.join("slowbin");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let shim = dir.join("herdr");
    let script = format!(
        "#!/bin/sh\ncase \" $* \" in\n*\" workspace focus \"*) sleep {seconds};;\nesac\nexec {HERDR} \"$@\"\n"
    );
    std::fs::write(&shim, script).map_err(|e| e.to_string())?;
    Command::new("chmod")
        .args(["+x", &shim.display().to_string()])
        .status()
        .map_err(|e| e.to_string())?;
    Ok(format!("{} --session {}", shim.display(), private.session))
}

/// New tab in another workspace is `workspace focus`, then `tab create
/// --focus`; a pane tapped right after queues behind them (B191, B301). On
/// a real server: healthy, the tap runs last and the tapped pane keeps the
/// server's focus, the new tab made. With `workspace focus` outlasting the
/// lane's hold, the tap goes when the hold runs out and the sequence goes no
/// further: no late `tab create --focus` takes the focus from the tapped
/// pane. The held step is abandoned (its exec closed), so here not even the
/// late `workspace focus` lands.
fn new_tab_then_tap() -> Outcome {
    for slow in [false, true] {
        let private = Private::new(if slow { "tabslow" } else { "tab" })?;
        let first = private.herdr(&["api", "snapshot"]).0;
        let p1 = moshpit_herdr::snapshot::decode(&first)
            .and_then(|decoded| decoded.snapshot.active_pane_id)
            .ok_or("no pane")?;
        private.herdr(&["pane", "split", &p1, "--direction", "right"]);
        private.herdr(&["workspace", "create", "--label", "other"]);
        private.focus(&p1);
        let mut config = config(&private);
        if slow {
            config.custom_path = Some(slow_focus_shim(&private, 6)?);
        }
        let mut live = Live::new(&private, config);
        let pane = attached(&mut live)?;
        check(pane == p1, "attached to the focused pane")?;
        let tree = live.client.snapshot();
        let here = tree.active_session_id.clone().ok_or("no workspace")?;
        let other = tree
            .sessions
            .values()
            .find(|session| session.name == "other")
            .map(|session| session.id.clone())
            .ok_or("no other workspace")?;
        let tapped = tree
            .panes
            .keys()
            .find(|id| **id != p1)
            .cloned()
            .ok_or("no second pane")?;
        let tabs_before = tree
            .windows
            .values()
            .filter(|window| window.session_id == other)
            .count();
        live.act(Action::NewWindowInSession {
            session: other.clone(),
            name: Some("logs".to_owned()),
        });
        if slow {
            live.run(700);
        }
        live.act(Action::SelectPane(tapped.clone()));
        live.run(if slow { 15_000 } else { 4_000 });
        let after = moshpit_herdr::snapshot::decode(&private.herdr(&["api", "snapshot"]).0)
            .ok_or("no snapshot")?
            .snapshot;
        let tabs_after = after
            .windows
            .values()
            .filter(|window| window.session_id == other)
            .count();
        let focused = after.active_pane_id.clone().unwrap_or_default();
        let creates = live.sent("tab create");
        println!(
            "  {}: tabs in {other} {tabs_before} -> {tabs_after}; `tab create` sent {creates}; server focus {focused} (tapped {tapped} in {here})",
            if slow {
                "workspace focus held 6 s"
            } else {
                "healthy"
            }
        );
        if slow {
            check(
                creates == 0,
                "the sequence went no further than its held step",
            )?;
            check(tabs_after == tabs_before, "no new tab was made")?;
            check(
                live.sent(&format!("focus {tapped}")) + live.sent(&format!("focus '{tapped}'")) > 0,
                "the tap was sent",
            )?;
        } else {
            check(creates == 1, "the new tab was asked for once")?;
            check(tabs_after == tabs_before + 1, "the new tab was made")?;
            check(focused == tapped, "the tapped pane kept the server's focus")?;
        }
        live.finish()?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let cases: [Case; 13] = [
        ("attach_and_type", attach_and_type),
        ("switch_panes", switch_panes),
        ("pipelined_retarget", pipelined_retarget),
        ("takeover", takeover),
        ("two_devices", two_devices),
        ("ghost", ghost),
        ("agent_task", agent_task),
        ("half_open", half_open),
        ("restart", restart),
        ("skew_client_older", skew_client_older),
        ("skew_server_older", skew_server_older),
        ("push", push),
        ("new_tab_then_tap", new_tab_then_tap),
    ];
    let wanted: Vec<String> = std::env::args().skip(1).collect();
    let version = Command::new(HERDR)
        .arg("--version")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default();
    println!("{version}");
    let mut failed = 0;
    for (name, case) in cases {
        if !wanted.is_empty() && !wanted.iter().any(|w| w == name) {
            continue;
        }
        println!("{name}:");
        let started = Instant::now();
        match case() {
            Ok(()) => println!("  PASS ({} ms)", started.elapsed().as_millis()),
            Err(error) => {
                failed += 1;
                println!("  FAIL: {error}");
            }
        }
    }
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
