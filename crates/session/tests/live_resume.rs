//! The way back from the background, timed: what the person waits for
//! between the app returning to the foreground and a screen that is live
//! again (an echo of what they type), stage by stage, for SSH + tmux -CC,
//! mosh (plain and with tmux) and herdr — on this machine's loopback, with
//! a private sshd, private tmux/herdr servers and the session core as the
//! app runs it (`Timing::default()`).
//!
//! The suspension is the app's: `set_foreground(false)`, the flush the
//! background assertion waits for, then the process stopped (`SIGSTOP`) for
//! the time away — the session core runs in a child process of the test, so
//! nothing of it runs while away (no timer, no read), the kernel keeps its
//! sockets and buffers what the servers send, and on `SIGCONT` the overdue
//! timers and the buffered bytes come first, then the foreground, as on
//! iOS. The parent keeps the link to sshd and decides its fate on the
//! return:
//!
//! - `alive`: the connection survived the suspension (a short switch, or a
//!   long one on a quiet network).
//! - `cut`: every connection through the link closed while away (iOS
//!   reclaimed the socket, the server timed us out): heard at once.
//! - `hole`: the old connections never move again, without a word (a NAT
//!   that forgot us, a Wi-Fi that changed under the suspension); new ones
//!   work.
//!
//! The link adds a one-way delay (`rtt / 2`) to every byte, both ways, in
//! order; the TCP handshake itself is not delayed (add one RTT for it).
//!
//! ```sh
//! RESUME_CASES=tmux:5:50:alive,tmux:30:50:alive \
//!   cargo test -p moshpit-session --test live_resume -- --ignored --nocapture
//! ```
//!
//! `RESUME_IN_PLACE=1` runs them with the prototype's way back
//! (`Timing::resume_in_place`). A case is `kind:away_seconds:rtt_ms:fate[:busy]`; kinds are `tmux`,
//! `mosh`, `moshtmux`, `herdr`, `shell`; `busy` keeps the tmux pane
//! printing through the suspension. Each case prints one `RESULT` line and
//! appends it to `$RESUME_OUT` (default `/tmp/rc1/resume-results.tsv`).

mod support;

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moshpit_session::{
    ConnectionSpec, Event, EventKind, Multiplexer, PaneInfo, Protocol, SessionState, Timing,
};
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

// ---------------------------------------------------------------------------
// The link: a TCP relay with a delay, a suspension and three fates

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LinkState {
    /// Connections accepted before this epoch never move again.
    dead_before: u64,
    epoch: u64,
}

/// One connection's bytes as they were delivered: when, which way, how many.
pub struct Tap {
    pub accepted: Instant,
    pub chunks: Mutex<Vec<(Instant, bool, usize)>>,
}

impl Tap {
    /// Round trips (a turn from upstream to downstream) delivered by `until`.
    fn turns(&self, until: Instant) -> usize {
        let chunks = self.chunks.lock().unwrap();
        let mut turns = 0;
        let mut last_up = false;
        for (at, down, _) in chunks.iter() {
            if *at > until {
                break;
            }
            if *down && last_up {
                turns += 1;
            }
            last_up = !*down;
        }
        turns
    }

    fn first_down(&self) -> Option<Instant> {
        let chunks = self.chunks.lock().unwrap();
        chunks
            .iter()
            .find(|(_, down, _)| *down)
            .map(|(at, _, _)| *at)
    }
}

pub struct Link {
    pub port: u16,
    state: watch::Sender<LinkState>,
    cuts: watch::Sender<u64>,
    pub taps: Arc<Mutex<Vec<Arc<Tap>>>>,
    task: JoinHandle<()>,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.task.abort();
        self.cuts.send_modify(|n| *n += 1);
    }
}

impl Link {
    pub async fn start(to: SocketAddr, one_way: Duration) -> Link {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (state, _) = watch::channel(LinkState {
            dead_before: 0,
            epoch: 0,
        });
        let (cuts, _) = watch::channel(0u64);
        let taps: Arc<Mutex<Vec<Arc<Tap>>>> = Arc::default();
        let (states, cut_tx, tap_list) = (state.clone(), cuts.clone(), taps.clone());
        let task = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    return;
                };
                let tap = Arc::new(Tap {
                    accepted: Instant::now(),
                    chunks: Mutex::new(Vec::new()),
                });
                tap_list.lock().unwrap().push(tap.clone());
                let line = states.subscribe();
                let born = line.borrow().epoch;
                let mut cuts = cut_tx.subscribe();
                cuts.borrow_and_update();
                tokio::spawn(async move {
                    let Ok(server) = TcpStream::connect(to).await else {
                        return;
                    };
                    let _ = client.set_nodelay(true);
                    let _ = server.set_nodelay(true);
                    let (cr, cw) = client.into_split();
                    let (sr, sw) = server.into_split();
                    let up = delayed_pipe(cr, sw, one_way, line.clone(), born, tap.clone(), false);
                    let down = delayed_pipe(sr, cw, one_way, line, born, tap, true);
                    let _ = cuts.changed().await;
                    for task in up.into_iter().chain(down) {
                        task.abort();
                    }
                });
            }
        });
        Link {
            port,
            state,
            cuts,
            taps,
            task,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.port))
    }

    /// Every connection there is now goes silent for good.
    pub fn hole(&self) {
        self.state.send_modify(|s| {
            s.epoch += 1;
            s.dead_before = s.epoch;
        });
    }

    /// Every connection there is now is closed.
    pub fn cut(&self) {
        self.cuts.send_modify(|n| *n += 1);
    }

    pub fn connections(&self) -> usize {
        self.taps.lock().unwrap().len()
    }

    pub fn tap(&self, index: usize) -> Option<Arc<Tap>> {
        self.taps.lock().unwrap().get(index).cloned()
    }
}

/// A reader that timestamps what it reads, a writer that delivers it
/// `delay` later, in order, never again once dead.
fn delayed_pipe(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    delay: Duration,
    mut line: watch::Receiver<LinkState>,
    born: u64,
    tap: Arc<Tap>,
    down: bool,
) -> [JoinHandle<()>; 2] {
    let (tx, mut rx) = mpsc::unbounded_channel::<(tokio::time::Instant, Vec<u8>)>();
    let reader = tokio::spawn(async move {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            match from.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    let due = tokio::time::Instant::now() + delay;
                    if tx.send((due, buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
            }
        }
    });
    let writer = tokio::spawn(async move {
        while let Some((due, bytes)) = rx.recv().await {
            tokio::time::sleep_until(due).await;
            if born < line.borrow_and_update().dead_before {
                std::future::pending::<()>().await;
            }
            if to.write_all(&bytes).await.is_err() {
                return;
            }
            tap.chunks
                .lock()
                .unwrap()
                .push((Instant::now(), down, bytes.len()));
        }
        let _ = to.shutdown().await;
    });
    [reader, writer]
}

// ---------------------------------------------------------------------------
// The app's memory of a connection, kept in a map

#[derive(Default)]
struct Memory {
    capabilities: Mutex<std::collections::HashMap<String, moshpit_install::HostCapabilities>>,
    selections: Mutex<std::collections::HashMap<String, moshpit_tmux::model::Selection>>,
}

impl moshpit_session::SessionMemory for Memory {
    fn tmux_selection(&self, id: &str) -> Option<moshpit_tmux::model::Selection> {
        self.selections.lock().unwrap().get(id).cloned()
    }

    fn save_tmux_selection(&self, id: &str, selection: &moshpit_tmux::model::Selection) {
        self.selections
            .lock()
            .unwrap()
            .insert(id.to_owned(), selection.clone());
    }

    fn capabilities(&self, id: &str) -> Option<moshpit_install::HostCapabilities> {
        self.capabilities.lock().unwrap().get(id).cloned()
    }

    fn save_capabilities(&self, id: &str, capabilities: &moshpit_install::HostCapabilities) {
        self.capabilities
            .lock()
            .unwrap()
            .insert(id.to_owned(), capabilities.clone());
    }
}

// ---------------------------------------------------------------------------
// Cases

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Tmux,
    Mosh,
    MoshTmux,
    Herdr,
    Shell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fate {
    Alive,
    Cut,
    Hole,
}

#[derive(Clone, Debug)]
struct Case {
    kind: Kind,
    away: Duration,
    rtt_ms: u64,
    fate: Fate,
    busy: bool,
}

impl Case {
    fn parse(text: &str) -> Case {
        let parts: Vec<&str> = text.split(':').collect();
        assert!(parts.len() >= 4, "kind:away:rtt:fate[:busy], got {text}");
        Case {
            kind: match parts[0] {
                "tmux" => Kind::Tmux,
                "mosh" => Kind::Mosh,
                "moshtmux" => Kind::MoshTmux,
                "herdr" => Kind::Herdr,
                "shell" => Kind::Shell,
                other => panic!("kind {other}"),
            },
            away: Duration::from_millis((parts[1].parse::<f64>().unwrap() * 1000.0) as u64),
            rtt_ms: parts[2].parse().unwrap(),
            fate: match parts[3] {
                "alive" => Fate::Alive,
                "cut" => Fate::Cut,
                "hole" => Fate::Hole,
                other => panic!("fate {other}"),
            },
            busy: parts.get(4) == Some(&"busy"),
        }
    }

    fn label(&self) -> String {
        format!(
            "{:?}:{}s:{}ms:{:?}{}",
            self.kind,
            self.away.as_secs_f64(),
            self.rtt_ms,
            self.fate,
            if self.busy { ":busy" } else { "" }
        )
        .to_lowercase()
    }
}

/// What happened after the return, in ms from it.
#[derive(Debug, Default)]
struct Timeline {
    reconnecting: Option<u128>,
    connected: Option<u128>,
    panes: Option<u128>,
    first_frame: Option<u128>,
    /// The echo of what was typed, and on which attach's stream.
    echo: Option<u128>,
    /// A busy pane: its output of the second of the return is on screen.
    caught_up: Option<u128>,
    echo_on_new_stream: bool,
    typed_into_old_lost: bool,
    discarded: usize,
    /// Bytes the old (on screen) stream received after the return, and
    /// whether they erased the screen (a visible repaint).
    old_stream_bytes: usize,
    old_stream_erased: bool,
    views: Vec<(u128, String)>,
    states: Vec<(u128, String)>,
    /// What the in-place return's question found (`ResumeChecked`):
    /// outcome, ms waited, s away.
    checked: Vec<String>,
}

fn ms(since: Instant, at: Instant) -> u128 {
    at.saturating_duration_since(since).as_millis()
}

fn active(panes: &[PaneInfo]) -> PaneInfo {
    panes.iter().find(|p| p.active).unwrap_or(&panes[0]).clone()
}

fn base_spec(port: u16) -> ConnectionSpec {
    let mut spec = spec(port);
    spec.pty_command = Some("/bin/sh".to_owned());
    spec
}

/// The child's half of a case: everything but the link (the parent's).
/// Says `SSHD <port>`, reads `LINK <port>`; says `SUSPEND` once the app is
/// in the background with what it owed the server written (the moment iOS
/// suspends it: the parent stops this process then), reads `RESUMED` once it
/// runs again, and times the way back from there.
async fn run_case(case: &Case, lines: &mut Lines) -> Timeline {
    let herdr = (case.kind == Kind::Herdr).then(PrivateHerdr::start);
    let sshd = match &herdr {
        Some(h) => {
            let home = h.home.display().to_string();
            Sshd::start_with(&[("HOME", &home), ("ZDOTDIR", &home)]).await
        }
        None => Sshd::start().await,
    };
    say(&format!("SSHD {}", sshd.port));
    let link_port: u16 = lines
        .expect("LINK ")
        .await
        .parse()
        .expect("the link's port");
    let link_addr = SocketAddr::from(([127, 0, 0, 1], link_port));
    let tmux = matches!(case.kind, Kind::Tmux | Kind::MoshTmux).then(|| Tmux::named("rc1-rs"));
    if let Some(tmux) = &tmux {
        tmux.new_session("rs");
    }
    let timing = Timing {
        resume_in_place: in_place(),
        ..Timing::default()
    };
    // The app remembers the host's capabilities and the tmux selection: a
    // redial decides on them and re-probes behind (an empty memory would
    // put a probe exec on every redial's path).
    let udp = ProxiedUdp::new();
    let mut services = Bench::services(&udp, link_addr);
    services.memory = Arc::new(Memory::default());
    let mut bench = Bench::new(services, timing);
    bench.udp = udp;
    bench
        .udp
        .faults
        .delay_ms
        .store(case.rtt_ms / 2, Ordering::SeqCst);
    let mut spec = base_spec(link_port);
    match case.kind {
        Kind::Tmux => {
            spec.multiplexer = Multiplexer::Tmux;
            spec.tmux_path = Some(tmux.as_ref().unwrap().prefix());
        }
        Kind::Mosh => spec.protocol = Protocol::Mosh,
        Kind::MoshTmux => {
            spec.protocol = Protocol::Mosh;
            spec.multiplexer = Multiplexer::Tmux;
            spec.tmux_path = Some(tmux.as_ref().unwrap().prefix());
        }
        Kind::Herdr => {
            spec.multiplexer = Multiplexer::Herdr;
            spec.herdr_path = Some(herdr.as_ref().unwrap().custom_path());
        }
        Kind::Shell => {}
    }
    let multiplexed = matches!(case.kind, Kind::Tmux | Kind::Herdr);
    let id = bench.sessions.connect(spec);
    bench.sessions.set_visible(Some(id));
    bench.connected(id).await;
    let _reaper = Reaper(bench.sessions.view(id).unwrap().mosh_server_pid);
    let (attach, panes) = bench.panes(id, 30).await;
    if multiplexed
        && !bench
            .seen
            .iter()
            .any(|e| e.session == id && e.kind == EventKind::FirstFrame { attach })
    {
        bench
            .until(id, 30, |kind| match kind {
                EventKind::FirstFrame { attach: a } if *a == attach => Some(()),
                _ => None,
            })
            .await;
    }
    let pane = active(&panes);
    let mut old = Term::attach(pane.stream);
    let mark = old.screen.len();
    old.write(b"echo r$((1+1))\r");
    assert!(
        old.wait_for_from(mark, "r2", 20).await.is_some(),
        "{}: live before: {}",
        case.label(),
        old.text()
    );
    if case.busy
        && let Some(tmux) = &tmux
    {
        tmux.run(&[
            "send-keys",
            "-t",
            &pane.pane,
            "while :; do date +%s; sleep 0.2; done",
            "Enter",
        ]);
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    bench.drain();
    old.read();

    // Away: what the background assertion waits for, then suspended.
    bench.sessions.set_foreground(false);
    let _ = bench.sessions.flushed().await;
    bench.drain();
    old.read();
    let old_mark = old.screen.len();
    say("SUSPEND");
    lines.expect("RESUMED").await;
    // Running again: overdue timers and buffered bytes are being handled
    // as this line is read, as on iOS before scenePhase turns active.
    let t0 = Instant::now();
    let resumed_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // What a busy pane prints once the screen has caught up with the
    // suspension: this second's stamp, or the next one's.
    let fresh = [resumed_unix.to_string(), (resumed_unix + 1).to_string()];
    let caught_up = |screen: &[u8]| {
        fresh
            .iter()
            .any(|stamp| find(screen, stamp.as_bytes()).is_some())
    };
    bench.sessions.set_foreground(true);
    let mut line = Timeline::default();
    // Typed at once, into what is on screen.
    old.write(b"echo r$((40+2))\r");
    let mut new: Option<(u64, Term)> = None;
    let mut new_typed = false;
    let deadline = t0 + Duration::from_secs(45);
    let mut settled_at: Option<Instant> = None;
    loop {
        while let Ok(event) = bench.events.try_recv() {
            let at = ms(t0, Instant::now());
            note(&mut line, &event, at, id, attach, &old, &new);
            if let EventKind::Panes { attach: a, panes } = &event.kind
                && event.session == id
                && *a > attach
                && !panes.is_empty()
                && new.as_ref().is_none_or(|(n, _)| *n < *a)
            {
                let pane = active(panes);
                if pane.stream != old.stream {
                    new = Some((*a, Term::attach(pane.stream)));
                    new_typed = false;
                }
            }
            bench.seen.push(event);
        }
        old.read();
        if line.echo.is_none() && find(&old.screen[old_mark..], b"r42").is_some() {
            line.echo = Some(ms(t0, Instant::now()));
        }
        if case.busy && line.caught_up.is_none() && caught_up(&old.screen[old_mark..]) {
            line.caught_up = Some(ms(t0, Instant::now()));
        }
        if let Some((a, term)) = new.as_mut() {
            term.read();
            // Typed once its first frame is up, the way a person would.
            if !new_typed && line.first_frame.is_some() {
                let painted = bench
                    .seen
                    .iter()
                    .any(|e| e.session == id && e.kind == EventKind::FirstFrame { attach: *a });
                if painted || !multiplexed {
                    term.write(b"echo r$((50+2))\r");
                    new_typed = true;
                }
            }
            if line.echo.is_none() && find(&term.screen, b"r52").is_some() {
                line.echo = Some(ms(t0, Instant::now()));
                line.echo_on_new_stream = true;
            }
            if case.busy && line.caught_up.is_none() && caught_up(&term.screen) {
                line.caught_up = Some(ms(t0, Instant::now()));
                line.echo_on_new_stream = true;
            }
        }
        // A plain shell keeps its one stream: typed again once back.
        if !multiplexed && line.reconnecting.is_some() && line.connected.is_some() && !new_typed {
            old.write(b"echo r$((50+2))\r");
            new_typed = true;
        }
        if !multiplexed && line.echo.is_none() && find(&old.screen[old_mark..], b"r52").is_some() {
            line.echo = Some(ms(t0, Instant::now()));
        }
        // A busy pane's loop takes what is typed: there it is the screen
        // catching up that counts.
        let done = if case.busy {
            line.caught_up.is_some()
        } else {
            line.echo.is_some()
        };
        if done && settled_at.is_none() {
            settled_at = Some(Instant::now());
        }
        if settled_at.is_some_and(|at| at.elapsed() > Duration::from_millis(1500))
            || Instant::now() > deadline
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(3)).await;
    }
    // Did the line typed into the old screen at the return ever run?
    line.typed_into_old_lost = case.kind == Kind::Tmux
        && !case.busy
        && tmux.as_ref().is_some_and(|t| {
            !t.run(&["capture-pane", "-p", "-S", "-200", "-t", &pane.pane])
                .contains("r42")
        });
    line.old_stream_bytes = old.screen.len() - old_mark;
    line.old_stream_erased = find(&old.screen[old_mark..], b"\x1b[2J").is_some();
    let _ = bench.sessions.disconnect(id).await;
    drop(herdr);
    line
}

fn note(
    line: &mut Timeline,
    event: &Event,
    at: u128,
    id: u64,
    attach: u64,
    old: &Term,
    new: &Option<(u64, Term)>,
) {
    if event.session != id {
        return;
    }
    match &event.kind {
        EventKind::State(state) => {
            line.states.push((at, format!("{state:?}")));
            match state {
                SessionState::Reconnecting if line.reconnecting.is_none() => {
                    line.reconnecting = Some(at)
                }
                SessionState::Connected if line.reconnecting.is_some() => {
                    line.connected.get_or_insert(at);
                }
                _ => {}
            }
        }
        EventKind::Panes { attach: a, .. } if *a > attach => {
            line.panes.get_or_insert(at);
        }
        EventKind::FirstFrame { attach: a } if *a > attach => {
            line.first_frame.get_or_insert(at);
        }
        EventKind::InputDiscarded { .. } => line.discarded += 1,
        EventKind::ResumeChecked {
            away_ms,
            waited_ms,
            outcome,
            through_tmux,
            held,
        } => line.checked.push(format!(
            "{outcome:?}/{waited_ms}ms/{}s/{}/{held}",
            away_ms / 1000,
            if *through_tmux { "tmux" } else { "ssh" }
        )),
        EventKind::View { stream, action } => {
            let whose = if *stream == old.stream {
                "old"
            } else if new.as_ref().is_some_and(|(_, t)| t.stream == *stream) {
                "new"
            } else {
                "other"
            };
            line.views.push((at, format!("{whose}:{action:?}")));
        }
        _ => {}
    }
}

fn report(case: &Case, line: &Timeline) -> String {
    let f = |v: Option<u128>| v.map_or("-".to_owned(), |v| v.to_string());
    let views: Vec<String> = line
        .views
        .iter()
        .map(|(at, what)| format!("{at}:{what}"))
        .collect();
    let states: Vec<String> = line
        .states
        .iter()
        .map(|(at, what)| format!("{at}:{what}"))
        .collect();
    format!(
        "{}\t{}\treconnecting={}\tconnected={}\tpanes={}\tfirst_frame={}\techo={}\tcaught_up={}\techo_new={}\tlost_typed={}\tdiscarded={}\told_bytes={}\told_erased={}\tchecked={}\tstates=[{}]\tviews=[{}]",
        case.label(),
        if in_place() { "in-place" } else { "today" },
        f(line.reconnecting),
        f(line.connected),
        f(line.panes),
        f(line.first_frame),
        f(line.echo),
        f(line.caught_up),
        line.echo_on_new_stream,
        line.typed_into_old_lost,
        line.discarded,
        line.old_stream_bytes,
        line.old_stream_erased,
        if line.checked.is_empty() {
            "-".to_owned()
        } else {
            line.checked.join(",")
        },
        states.join(" "),
        views.join(" "),
    )
}

/// `RESUME_IN_PLACE=1`: the prototype's way back (`Timing::resume_in_place`).
fn in_place() -> bool {
    std::env::var("RESUME_IN_PLACE").is_ok_and(|v| v == "1")
}

// ---------------------------------------------------------------------------
// The two processes

/// One line to the parent, on its own (the harness prints around it).
fn say(line: &str) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "@@{line}");
    let _ = out.flush();
}

/// The parent's lines, on the child's stdin (read on a thread of its own).
struct Lines(mpsc::UnboundedReceiver<String>);

impl Lines {
    fn new() -> Lines {
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { return };
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        Lines(rx)
    }

    /// The rest of the next line that starts with `prefix`.
    async fn expect(&mut self, prefix: &str) -> String {
        loop {
            let line = self.0.recv().await.expect("the parent went away");
            if let Some(rest) = line.strip_prefix(prefix) {
                return rest.trim().to_owned();
            }
        }
    }
}

/// The child: one case, the parent driving the suspension.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run by resume_timeline"]
async fn resume_child() {
    let Ok(text) = std::env::var("RESUME_CHILD_CASE") else {
        return;
    };
    let case = Case::parse(&text);
    let mut lines = Lines::new();
    let line = run_case(&case, &mut lines).await;
    say(&format!("RESULT\t{}", report(&case, &line)));
}

/// One case: the child process runs the session core; this one runs the
/// link, stops the child for the time away, applies the link's fate and
/// lets it go on.
async fn drive(case: &Case, in_place: bool) -> String {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "resume_child", "--nocapture"])
        .env("RESUME_CHILD_CASE", text_of(case))
        .env("RESUME_IN_PLACE", if in_place { "1" } else { "0" })
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let mut link: Option<Link> = None;
    let mut before = 0;
    let mut resumed_at = Instant::now();
    let mut log = Vec::new();
    loop {
        let next = tokio::time::timeout(
            Duration::from_secs(case.away.as_secs() + 180),
            out.next_line(),
        )
        .await;
        let Ok(Ok(Some(line))) = next else {
            return format!("{}\tFAILED\t{}", case.label(), log.join(" | "));
        };
        let Some(said) = line.strip_prefix("@@") else {
            if line.contains("panicked") || line.contains("FAILED") || line.contains("error") {
                log.push(line);
            }
            continue;
        };
        if let Some(port) = said.strip_prefix("SSHD ") {
            let sshd = SocketAddr::from(([127, 0, 0, 1], port.parse::<u16>().unwrap()));
            let started = Link::start(sshd, Duration::from_millis(case.rtt_ms / 2)).await;
            stdin
                .write_all(format!("LINK {}\n", started.port).as_bytes())
                .await
                .unwrap();
            link = Some(started);
        } else if said == "SUSPEND" {
            let link = link.as_ref().unwrap();
            signal(pid, "-STOP");
            tokio::time::sleep(case.away).await;
            match case.fate {
                Fate::Alive => {}
                Fate::Cut => link.cut(),
                Fate::Hole => link.hole(),
            }
            before = link.connections();
            resumed_at = Instant::now();
            signal(pid, "-CONT");
            stdin.write_all(b"RESUMED\n").await.unwrap();
        } else if let Some(result) = said.strip_prefix("RESULT\t") {
            let link = link.as_ref().unwrap();
            let after = link.connections();
            let mut extra = format!("conns={before}->{after}");
            if after > before
                && let Some(tap) = link.tap(before)
            {
                let connected = result
                    .split('\t')
                    .find_map(|f| f.strip_prefix("connected="))
                    .and_then(|v| v.parse::<u64>().ok());
                extra.push_str(&format!(
                    "\tdial={}\tbanner={}\tturns_to_connected={}",
                    ms(resumed_at, tap.accepted),
                    tap.first_down()
                        .map_or("-".into(), |at| ms(resumed_at, at).to_string()),
                    connected.map_or("-".into(), |c| tap
                        .turns(resumed_at + Duration::from_millis(c))
                        .to_string()),
                ));
            }
            let _ = child.wait().await;
            return format!("{result}\t{extra}");
        }
    }
}

fn signal(pid: u32, which: &str) {
    let _ = std::process::Command::new("kill")
        .args([which, &pid.to_string()])
        .output();
}

fn text_of(case: &Case) -> String {
    format!(
        "{}:{}:{}:{}{}",
        match case.kind {
            Kind::Tmux => "tmux",
            Kind::Mosh => "mosh",
            Kind::MoshTmux => "moshtmux",
            Kind::Herdr => "herdr",
            Kind::Shell => "shell",
        },
        case.away.as_secs_f64(),
        case.rtt_ms,
        match case.fate {
            Fate::Alive => "alive",
            Fate::Cut => "cut",
            Fate::Hole => "hole",
        },
        if case.busy { ":busy" } else { "" }
    )
}

/// Every case in `$RESUME_CASES`, one after another.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd, tmux, mosh-server and herdr; see the module docs"]
async fn resume_timeline() {
    if !Sshd::available() {
        eprintln!("skipped: no {SSHD}");
        return;
    }
    let cases = std::env::var("RESUME_CASES").unwrap_or_else(|_| "tmux:5:0:alive".to_owned());
    let out = std::env::var("RESUME_OUT").unwrap_or_else(|_| "/tmp/rc1/resume-results.tsv".into());
    for text in cases.split(',').filter(|c| !c.is_empty()) {
        let case = Case::parse(text);
        let row = format!("RESULT\t{}", drive(&case, in_place()).await);
        eprintln!("{row}");
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&out)
        {
            let _ = writeln!(file, "{row}");
        }
    }
}

/// A field of a result row.
fn field<'a>(row: &'a str, name: &str) -> Option<&'a str> {
    row.split('\t')
        .find_map(|f| f.strip_prefix(name).and_then(|rest| rest.strip_prefix('=')))
}

fn number(row: &str, name: &str) -> Option<u64> {
    field(row, name).and_then(|v| v.parse().ok())
}

/// The prototype's promise, on a suspended core (`Timing::resume_in_place`):
/// a tmux connection that survived 25 s away is kept — no reconnect, no new
/// connection, the key typed on the return echoed within a few round trips
/// — and one that died without a word is redialled once the 1.5 s question
/// goes unanswered, the key typed meanwhile reported as not sent (never
/// written into the dead socket); one closed while away is redialled at
/// once. Each return says what it found, once, with the time away.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs sshd and tmux; see the module docs"]
async fn in_place_keeps_what_survived_and_redials_what_did_not() {
    if !Sshd::available() {
        eprintln!("skipped: no {SSHD}");
        return;
    }
    let kept = drive(&Case::parse("tmux:25:50:alive"), true).await;
    eprintln!("{kept}");
    assert_eq!(field(&kept, "reconnecting"), Some("-"), "{kept}");
    assert_eq!(field(&kept, "conns"), Some("1->1"), "{kept}");
    assert_eq!(field(&kept, "discarded"), Some("0"), "{kept}");
    let echo = number(&kept, "echo").expect("echoed");
    assert!(echo < 1_000, "{kept}");
    // One outcome per return, with the time away, through tmux's channel;
    // the key typed on the return was held for the answer.
    let checked = field(&kept, "checked").unwrap_or("-");
    assert!(checked.starts_with("Kept/"), "{kept}");
    assert!(checked.ends_with("s/tmux/1"), "{kept}");
    assert!(
        checked.contains("/25s/") || checked.contains("/26s/"),
        "the time away: {kept}"
    );

    let redialled = drive(&Case::parse("tmux:25:50:hole"), true).await;
    eprintln!("{redialled}");
    let reconnecting = number(&redialled, "reconnecting").expect("reconnected");
    assert!(
        (1_400..3_000).contains(&reconnecting),
        "after the 1.5 s question: {redialled}"
    );
    assert_eq!(field(&redialled, "discarded"), Some("1"), "{redialled}");
    assert!(number(&redialled, "first_frame").is_some(), "{redialled}");
    assert_eq!(field(&redialled, "echo_new"), Some("true"), "{redialled}");
    assert!(
        field(&redialled, "checked").is_some_and(|c| c.starts_with("Silent/")),
        "{redialled}"
    );

    // A socket closed while away fails before anything can answer: still
    // one outcome, "dead", and the redial starts at once.
    let closed = drive(&Case::parse("tmux:25:50:cut"), true).await;
    eprintln!("{closed}");
    assert!(
        field(&closed, "checked").is_some_and(|c| c.starts_with("Dead/")),
        "{closed}"
    );
    assert!(
        number(&closed, "reconnecting").is_some_and(|at| at < 1_000),
        "{closed}"
    );
}
