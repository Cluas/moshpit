//! herdr over mosh's plane (`moshpit_session::herdr_mosh::Plane`) against a
//! fake host on a virtual clock: W2b's fake herdr answers the control
//! client's execs (`moshpit_herdr::client::rig::FakeHerdr`, its own delays),
//! a fake raw-attach loop answers the renderer's (a retarget lands a round
//! trip plus the attach later, a plain one past herdr's ~260 ms refusal and
//! the retarget's settle; a watch answers when the attach it was armed on
//! stops), another device can hold or take a pane, and mosh carries the
//! typing one way in `one_way` ms and acknowledges it on the way back (or
//! nothing gets through: UDP silence). Every byte that reaches the tty is
//! recorded with where it landed — a pane's terminal, the loop parked or
//! waiting ("no pane"), or a login shell — at the moment it arrived: never a
//! fake that answers inside the same write (no real link is that fast).
//!
//! Each test is one rule the Swift path learned, held on the core.

#![allow(clippy::type_complexity)]

use std::collections::BTreeMap;

use moshpit_herdr::client::rig::{FakeHerdr, FakePane, Plan, quoted_words};
use moshpit_herdr::client::{Action, Notice, NoticeAction};
use moshpit_herdr::launch;
use moshpit_session::herdr_mosh::{ExecTag, Plane, PlaneConfig, PlaneOut};
use moshpit_session::view_marks;

const KEY: &str = "6F9619FF-8B86-D011-B42D-00C04FC964FF-ABC12345";
const LOGIN_SHELL: &str = "login shell";
const NO_PANE: &str = "no pane";
const A: &str = "w1:p1";
const B: &str = "w1:p2";

fn term(pane: &str) -> String {
    format!("term_{}", pane.replace(':', "_"))
}

/// Where herdr's own TUI puts a byte: the server's focused pane.
fn tui(pane: &str) -> String {
    format!("tui {pane}")
}

fn held_words(terminal: &str) -> String {
    format!(
        "herdr: server shut down: terminal attach failed: terminal {terminal} already has an attached client; retry with --takeover"
    )
}

/// The raw-attach loop in mosh's tty.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Loop {
    /// Not running: a login shell has the tty (its line was swallowed, or
    /// it was killed).
    Unborn,
    Waiting,
    On(String),
    Parked {
        terminal: String,
        status: i64,
        said: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Herdr,
    Boot,
    Retarget,
    Check,
    Watch,
    Keys,
}

fn kind(command: &str) -> Kind {
    if command.contains(launch::RETARGET_LANDED_MARKER) {
        Kind::Retarget
    } else if command.contains(launch::LOOP_UP_MARKER) {
        Kind::Boot
    } else if command.contains(launch::RAW_ATTACH_UP_MARKER) {
        Kind::Check
    } else if command.starts_with("w=; read w") {
        Kind::Watch
    } else if command.contains("pane send-keys") {
        Kind::Keys
    } else {
        Kind::Herdr
    }
}

struct Exec {
    tag: ExecTag,
    command: String,
    kind: Kind,
    /// When it answers; `None`: a watch, answered by what the loop does.
    at: Option<u64>,
    /// A watch: the attach it was armed on.
    attach: u64,
}

/// A keystroke datagram on its way up.
struct Datagram {
    seq: u64,
    bytes: Vec<u8>,
    arrives: Option<u64>,
}

struct Rig {
    now: u64,
    plane: Plane,
    herdr: FakeHerdr,
    state: Loop,
    /// Bumped by every attach the loop starts.
    attach: u64,
    /// Terminals another device's client holds.
    held: Vec<String>,
    /// Terminals our `--takeover` took from another device.
    taken: Vec<String>,
    execs: Vec<Exec>,
    /// Every retarget: (terminal, takeover).
    retargets: Vec<(String, bool)>,
    /// Retargets in flight at once, at most.
    max_retargets: usize,
    one_way: u64,
    udp_down: bool,
    /// mosh shows herdr's own TUI: a byte reaches the server's focused pane
    /// as it arrives.
    tui: bool,
    /// The control connection is up; execs asked while it isn't wait for
    /// the next one, in order, as the leg holds them.
    connected: bool,
    waiting: Vec<(ExecTag, String)>,
    up: Vec<Datagram>,
    seq: u64,
    /// The newest seq the server has, and when the phone hears of it.
    server_seq: u64,
    acks: Vec<(u64, u64)>,
    client_acked: u64,
    barriers: Vec<(u64, u64)>,
    /// Typed copies of the start line a dead loop's shell swallows.
    swallows: u32,
    retypes: Vec<Vec<u8>>,
    /// Bytes by where they landed.
    landed: BTreeMap<String, Vec<u8>>,
    /// Keys herdr typed, by pane.
    keys: Vec<(String, String)>,
    marks: Vec<(u64, u32)>,
    delivered: BTreeMap<u64, (u64, bool)>,
    answers: Vec<u64>,
    dropped: usize,
    /// Pastes said landed or not: (when, token, landed).
    pastes: Vec<(u64, u64, bool)>,
    /// Typing said held or let go: (when, held).
    held_said: Vec<(u64, bool)>,
    /// Screen bytes due: (when).
    paints: Vec<u64>,
    /// When the plane asked the program to redraw its whole screen.
    redraws: Vec<u64>,
    log: Vec<String>,
}

impl Rig {
    fn new() -> Rig {
        Self::with(FakeHerdr::default())
    }

    fn with(herdr: FakeHerdr) -> Rig {
        let config = PlaneConfig::new(KEY, None, Some(KEY.to_owned()), false);
        let mut rig = Rig {
            now: 1_000_000,
            plane: Plane::new(config),
            herdr,
            state: Loop::Waiting,
            attach: 0,
            held: Vec::new(),
            taken: Vec::new(),
            execs: Vec::new(),
            retargets: Vec::new(),
            max_retargets: 0,
            one_way: 30,
            udp_down: false,
            tui: false,
            connected: true,
            waiting: Vec::new(),
            up: Vec::new(),
            seq: 0,
            server_seq: 0,
            acks: Vec::new(),
            client_acked: 0,
            barriers: Vec::new(),
            swallows: 0,
            retypes: Vec::new(),
            landed: BTreeMap::new(),
            keys: Vec::new(),
            marks: Vec::new(),
            delivered: BTreeMap::new(),
            answers: Vec::new(),
            dropped: 0,
            pastes: Vec::new(),
            held_said: Vec::new(),
            paints: Vec::new(),
            redraws: Vec::new(),
            log: Vec::new(),
        };
        rig.herdr.default_delay = 40;
        rig
    }

    fn start(&mut self) {
        let now = self.now;
        self.plane.start(now);
        self.pump();
    }

    /// What the tty shows now: where a byte that arrives goes.
    fn tty(&self) -> String {
        if self.tui {
            return tui(self.herdr.focused.as_deref().unwrap_or("-"));
        }
        match &self.state {
            Loop::On(terminal) => terminal.clone(),
            Loop::Waiting | Loop::Parked { .. } => NO_PANE.to_owned(),
            Loop::Unborn => LOGIN_SHELL.to_owned(),
        }
    }

    fn landed_in(&self, place: &str) -> String {
        String::from_utf8_lossy(self.landed.get(place).map_or(&[][..], |b| b)).into_owned()
    }

    fn pump(&mut self) {
        while let Some(output) = self.plane.poll_output() {
            match output {
                PlaneOut::Exec { tag, command } => self.ask(tag, command),
                PlaneOut::Abandon { tag } => {
                    self.execs.retain(|exec| exec.tag != tag);
                    self.waiting.retain(|(waiting, _)| *waiting != tag);
                }
                PlaneOut::OpenPush { .. } => {
                    let now = self.now;
                    self.plane.push_failed(now);
                }
                PlaneOut::PushWrite(_) | PlaneOut::ClosePush => {}
                PlaneOut::Mosh { bytes, .. } => {
                    if bytes.is_empty() {
                        continue;
                    }
                    self.seq += 1;
                    let arrives = (!self.udp_down).then_some(self.now + self.one_way);
                    self.up.push(Datagram {
                        seq: self.seq,
                        bytes,
                        arrives,
                    });
                }
                PlaneOut::Barrier(id) => {
                    if self.client_acked >= self.seq {
                        let now = self.now;
                        self.plane.acked(id, true, now);
                    } else {
                        self.barriers.push((id, self.seq));
                    }
                }
                PlaneOut::Mark(mark) => self.marks.push((self.now, mark.code)),
                PlaneOut::Redraw => self.redraws.push(self.now),
                PlaneOut::Delivered { token, sent } => {
                    self.delivered.insert(token, (self.now, sent));
                }
                PlaneOut::Answer { token, .. } => self.answers.push(token),
                PlaneOut::InputDropped { bytes } => self.dropped += bytes,
                PlaneOut::Landed { token, landed } => self.pastes.push((self.now, token, landed)),
                PlaneOut::InputHeld(held) => self.held_said.push((self.now, held)),
                PlaneOut::AgentHooksUpdated | PlaneOut::Snapshot { .. } => {}
            }
        }
    }

    fn ask(&mut self, tag: ExecTag, command: String) {
        if !self.connected {
            self.waiting.push((tag, command));
            return;
        }
        let kind = kind(&command);
        let at = match kind {
            Kind::Herdr => match self.herdr.plan(&command) {
                Plan::After(delay) => Some(self.now + delay),
                Plan::Never => Some(u64::MAX),
                Plan::Fail => Some(self.now + 5),
            },
            Kind::Boot => Some(
                self.now
                    + if self.state == Loop::Unborn {
                        1_500
                    } else {
                        40
                    },
            ),
            Kind::Retarget => {
                let takeover = !command.contains("rm -f");
                let terminal = quoted_words(&command)[0].clone();
                self.retargets.push((terminal, takeover));
                let out = self
                    .execs
                    .iter()
                    .filter(|exec| exec.kind == Kind::Retarget)
                    .count()
                    + 1;
                self.max_retargets = self.max_retargets.max(out);
                // A round trip, the kill, the next attach's pid; a plain
                // attach is watched past herdr's refusal (12 × 50 ms).
                Some(self.now + if takeover { 180 } else { 780 })
            }
            Kind::Check | Kind::Keys => Some(self.now + 40),
            Kind::Watch => None,
        };
        self.log.push(format!("{} {:?}", self.now, kind));
        self.execs.push(Exec {
            tag,
            command,
            kind,
            at,
            attach: self.attach,
        });
    }

    /// The loop's attach on `terminal` ended on its own.
    fn park(&mut self, terminal: &str, status: i64, said: &str) {
        self.state = Loop::Parked {
            terminal: terminal.to_owned(),
            status,
            said: said.to_owned(),
        };
        self.attach += 1;
    }

    fn stop_record(&self) -> Option<Vec<u8>> {
        match &self.state {
            Loop::Parked {
                terminal,
                status,
                said,
            } => Some(
                format!(
                    "{}\n{status} {terminal}\n{said}\n",
                    launch::RENDERER_STOPPED_MARKER
                )
                .into_bytes(),
            ),
            Loop::Unborn => Some(format!("{}\n", launch::RENDERER_GONE_MARKER).into_bytes()),
            _ => None,
        }
    }

    fn answer(&mut self, exec: Exec) -> Option<Vec<u8>> {
        let command = &exec.command;
        match exec.kind {
            Kind::Herdr => Some(self.herdr.land(self.now, command)),
            Kind::Boot => Some(if self.state == Loop::Unborn {
                Vec::new()
            } else {
                format!("{}\n", launch::LOOP_UP_MARKER).into_bytes()
            }),
            Kind::Retarget => {
                let terminal = quoted_words(command)[0].clone();
                let takeover = !command.contains("rm -f");
                // Numbered anew by a restart, as the fake server lists them.
                let epoch = self.herdr.terminal_epoch;
                let listed = self.herdr.running
                    && self.herdr.panes.iter().any(|pane| {
                        let named = if epoch == 0 {
                            term(&pane.id)
                        } else {
                            format!("{}_r{epoch}", term(&pane.id))
                        };
                        named == terminal
                    });
                if !listed {
                    return Some(format!("{}\n", launch::RETARGET_MISSING_MARKER).into_bytes());
                }
                if self.state == Loop::Unborn {
                    return Some(format!("{}\n", launch::RENDERER_GONE_MARKER).into_bytes());
                }
                if self.held.contains(&terminal) {
                    if takeover {
                        self.held.retain(|held| *held != terminal);
                        self.taken.push(terminal.clone());
                    } else {
                        let words = held_words(&terminal);
                        self.park(&terminal, 1, &words);
                        return self.stop_record();
                    }
                }
                self.state = Loop::On(terminal);
                self.attach += 1;
                // herdr repaints the pane ~250 ms on, over a few frames.
                self.paints.extend([self.now + 250, self.now + 300]);
                Some(format!("{}\n", launch::RETARGET_LANDED_MARKER).into_bytes())
            }
            Kind::Check => {
                let terminal = quoted_words(command)[0].clone();
                if self.state == Loop::On(terminal) {
                    Some(format!("{}\n", launch::RAW_ATTACH_UP_MARKER).into_bytes())
                } else {
                    Some(self.stop_record().unwrap_or_default())
                }
            }
            Kind::Keys => {
                let words = quoted_words(command);
                let pane = words[0].clone();
                for key in &words[1..] {
                    self.keys.push((pane.clone(), key.clone()));
                    self.log.push(format!("{} key {key}", self.now));
                }
                Some(Vec::new())
            }
            Kind::Watch => Some(self.stop_record().unwrap_or_default()),
        }
    }

    /// The next thing that happens, by when.
    fn next_at(&self) -> Option<u64> {
        let execs = self.execs.iter().filter_map(|exec| match exec.kind {
            Kind::Watch => (exec.attach != self.attach).then_some(self.now + 200),
            _ => exec.at,
        });
        let up = self.up.iter().filter_map(|d| d.arrives);
        let acks = self.acks.iter().map(|(at, _)| *at);
        [
            execs.min(),
            up.min(),
            acks.min(),
            self.plane.next_deadline(),
            self.paints.iter().copied().min(),
        ]
        .into_iter()
        .flatten()
        .filter(|at| *at != u64::MAX)
        .min()
    }

    /// Runs the host and the plane until `until`.
    fn run_until(&mut self, until: u64) {
        loop {
            self.pump();
            let Some(at) = self.next_at().filter(|at| *at <= until) else {
                self.now = until.max(self.now);
                let now = self.now;
                self.plane.tick(now);
                self.pump();
                return;
            };
            self.now = at.max(self.now);
            let now = self.now;
            // Screen bytes.
            if self.paints.iter().any(|p| *p <= now) {
                self.paints.retain(|p| *p > now);
                self.plane.painted(now);
            }
            // Datagrams up: what arrives reaches whatever the tty shows.
            let (arrived, rest): (Vec<Datagram>, Vec<Datagram>) = std::mem::take(&mut self.up)
                .into_iter()
                .partition(|d| d.arrives.is_some_and(|at| at <= now));
            self.up = rest;
            for datagram in arrived {
                self.arrive(datagram);
            }
            // Acks down.
            let (heard, rest): (Vec<(u64, u64)>, Vec<(u64, u64)>) = std::mem::take(&mut self.acks)
                .into_iter()
                .partition(|(at, _)| *at <= now);
            self.acks = rest;
            for (_, seq) in heard {
                self.client_acked = self.client_acked.max(seq);
            }
            let acked = self.client_acked;
            let (done, waiting): (Vec<(u64, u64)>, Vec<(u64, u64)>) =
                std::mem::take(&mut self.barriers)
                    .into_iter()
                    .partition(|(_, wanted)| *wanted <= acked);
            self.barriers = waiting;
            for (id, _) in done {
                self.plane.acked(id, true, now);
            }
            // Execs.
            let attach = self.attach;
            let due: Vec<usize> = self
                .execs
                .iter()
                .enumerate()
                .filter(|(_, exec)| match exec.kind {
                    Kind::Watch => exec.attach != attach,
                    _ => exec.at.is_some_and(|at| at <= now),
                })
                .map(|(index, _)| index)
                .rev()
                .collect();
            for index in due {
                let exec = self.execs.remove(index);
                let tag = exec.tag;
                let failed = matches!(exec.kind, Kind::Herdr)
                    && self.herdr.plan(&exec.command) == Plan::Fail;
                let output = if failed { None } else { self.answer(exec) };
                self.plane.exec_finished(tag, output, now);
                self.pump();
            }
            self.plane.tick(now);
        }
    }

    fn arrive(&mut self, datagram: Datagram) {
        // Never expired on the same session (B618): kept until acked.
        let place = self.tty();
        let mut bytes = datagram.bytes;
        if place == LOGIN_SHELL && bytes.first() == Some(&0x15) {
            // A typed start line: run, unless the starting shell eats it.
            self.retypes.push(bytes.clone());
            if self.swallows > 0 {
                self.swallows -= 1;
            } else {
                self.state = Loop::Waiting;
            }
            bytes.clear();
        }
        if !bytes.is_empty() {
            self.log.push(format!(
                "{} land {place} {}",
                self.now,
                String::from_utf8_lossy(&bytes)
            ));
            self.landed.entry(place).or_default().extend(bytes);
        }
        self.server_seq = self.server_seq.max(datagram.seq);
        let at = self.now + self.one_way;
        self.acks.push((at, datagram.seq));
    }

    fn run(&mut self, ms: u64) {
        let until = self.now + ms;
        self.run_until(until);
    }

    /// UDP gets through again (a roam landed): mosh sends again what the
    /// server never acknowledged, in order.
    fn path_back(&mut self) {
        self.udp_down = false;
        let at = self.now + self.one_way;
        for datagram in &mut self.up {
            datagram.arrives.get_or_insert(at);
        }
    }

    /// The control connection drops (mosh carries on): what runs on it
    /// fails, what is asked waits for the next one.
    fn drop_connection(&mut self) {
        self.connected = false;
        let now = self.now;
        self.plane.connection_lost(now);
        for exec in std::mem::take(&mut self.execs) {
            self.plane.exec_finished(exec.tag, None, now);
        }
        self.pump();
    }

    /// A control connection again: what waited runs on it, in order.
    fn reconnect(&mut self) {
        self.connected = true;
        for (tag, command) in std::mem::take(&mut self.waiting) {
            self.ask(tag, command);
        }
        let now = self.now;
        self.plane.connection_up(now);
        self.pump();
    }

    /// Where `needle` first landed in the log, by line.
    fn at(&self, needle: &str) -> Option<usize> {
        self.log.iter().position(|line| line.ends_with(needle))
    }

    fn type_text(&mut self, text: &str) {
        let now = self.now;
        self.plane.typed(text.as_bytes(), 0, now);
        self.pump();
    }

    fn act(&mut self, action: Action) {
        let now = self.now;
        self.plane.act(action, None, now);
        self.pump();
    }

    fn deliver(&mut self, token: u64, pane: &str, text: &str) {
        let now = self.now;
        self.plane.deliver(
            token,
            pane,
            text.as_bytes().to_vec(),
            false,
            now + 20_000,
            now,
        );
        self.pump();
    }

    fn on_screen(&self) -> Option<String> {
        self.plane
            .renderer()
            .and_then(|r| r.on_screen())
            .map(str::to_owned)
    }

    fn notice(&self) -> Option<(Notice, Option<NoticeAction>)> {
        self.plane.view().notice
    }

    /// Boots and lands on the focused pane (A).
    fn booted() -> Rig {
        let mut rig = Rig::new();
        rig.start();
        rig.run(3_000);
        assert_eq!(rig.on_screen(), Some(term(A)), "{:?}", rig.log);
        rig
    }
}

// MARK: - The focus, the person's pick

/// B685: a device that boots onto a pane another device holds says so
/// ("open on another device"); its person picks another pane: that pick
/// lands, and typing goes there. Live, the iPad of two devices on one herdr
/// never showed the pane it picked.
#[test]
fn a_pick_from_the_notice_of_a_held_pane_lands() {
    let mut rig = Rig::new();
    rig.held.push(term(A));
    rig.start();
    rig.run(3_000);
    assert_eq!(
        rig.notice(),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere)))
    );
    rig.act(Action::SelectPane(B.to_owned()));
    rig.run(3_000);
    assert_eq!(rig.on_screen(), Some(term(B)), "{:?}", rig.retargets);
    assert_eq!(rig.notice(), None);
    rig.type_text("here");
    rig.run(500);
    assert_eq!(rig.landed_in(&term(B)), "here");
}

/// Right after a switch to mosh, a Claude Code pane showed fragments of
/// earlier frames until the diagnostics' Repaint (Canary 159), and after a
/// tab switch under mosh too (Canary 166): each attach replays the pane
/// onto a fresh tty. Once a pane the loop landed on has painted, its
/// program is asked to redraw the whole screen.
#[test]
fn every_pane_the_loop_lands_on_is_redrawn() {
    let mut rig = Rig::booted();
    assert_eq!(rig.redraws.len(), 1, "{:?}", rig.log);
    rig.held.push(term(B));
    rig.act(Action::SelectPane(B.to_owned()));
    rig.run(3_000);
    assert_eq!(rig.on_screen(), Some(term(B)));
    assert_eq!(rig.redraws.len(), 2);
}

#[test]
fn it_boots_onto_the_focused_pane_with_a_plain_attach() {
    let rig = Rig::booted();
    assert_eq!(rig.retargets, vec![(term(A), false)]);
    assert!(rig.taken.is_empty());
}

#[test]
fn following_the_focus_never_takes_a_pane_and_open_here_does() {
    // Another device holds B; the person picks the tab and the focus lands
    // on B (B683: a focus change they asked for is followed, the desktop's
    // own moves aren't).
    let mut rig = Rig::booted();
    rig.act(Action::SelectWindow("w1:t1".to_owned()));
    rig.run(300);
    rig.held.push(term(B));
    rig.herdr.focused = Some(B.to_owned());
    rig.run(10_000);
    // The follow asked plainly, herdr refused, the loop parked: said, with
    // "Open here"; the other device keeps B.
    assert!(
        rig.retargets.contains(&(term(B), false)),
        "{:?}",
        rig.retargets
    );
    assert!(!rig.retargets.contains(&(term(B), true)), "a follow took B");
    assert!(rig.taken.is_empty());
    assert_eq!(
        rig.notice(),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere)))
    );
    // Typing goes nowhere: dropped and said, not into the parked tty.
    rig.type_text("ls\r");
    rig.run(500);
    assert_eq!(rig.dropped, 3);
    assert!(rig.landed_in(&term(B)).is_empty());
    assert!(rig.landed_in(NO_PANE).is_empty());
    // Later polls repeat B as the focus: never taken.
    rig.run(20_000);
    assert!(rig.taken.is_empty(), "{:?}", rig.retargets);
    // "Open here": the person's word takes it.
    rig.act(Action::NoticeAction);
    rig.run(3_000);
    assert_eq!(rig.taken, vec![term(B)]);
    assert_eq!(rig.on_screen(), Some(term(B)));
    assert_eq!(rig.notice(), None);
    rig.type_text("pwd\r");
    rig.run(500);
    assert_eq!(rig.landed_in(&term(B)), "pwd\r");
}

/// B683: the desktop moves herdr's focus while the phone types over
/// mosh: the loop stays on its pane, and the typing lands there.
#[test]
fn the_desktop_moving_the_focus_leaves_the_loop_on_its_pane() {
    let mut rig = Rig::booted();
    rig.run(1_000);
    assert_eq!(rig.on_screen(), Some(term(A)));
    rig.herdr.focused = Some(B.to_owned());
    rig.run(10_000);
    assert_eq!(rig.on_screen(), Some(term(A)), "{:?}", rig.retargets);
    assert!(!rig.retargets.iter().any(|(t, _)| *t == term(B)));
    rig.type_text("ls\r");
    rig.run(500);
    assert_eq!(rig.landed_in(&term(A)), "ls\r");
    assert_eq!(rig.dropped, 0);
}

#[test]
fn a_pick_takes_the_pane_and_typing_waits_for_the_loops_word() {
    let mut rig = Rig::booted();
    rig.held.push(term(B));
    rig.act(Action::SelectPane(B.to_owned()));
    // Typed during the switch: the pane being left would get it — dropped.
    rig.run(50);
    rig.type_text("early");
    rig.run(3_000);
    assert_eq!(rig.on_screen(), Some(term(B)));
    assert_eq!(rig.taken, vec![term(B)], "the person's pick takes over");
    assert!(rig.retargets.contains(&(term(B), true)));
    assert!(
        rig.landed_in(&term(A)).is_empty(),
        "nothing reached the pane left"
    );
    assert!(rig.landed_in(&term(B)).is_empty());
    assert_eq!(rig.dropped, 5);
    rig.type_text("late");
    rig.run(500);
    assert_eq!(rig.landed_in(&term(B)), "late");
}

#[test]
fn a_switch_storm_keeps_one_retarget_out_and_lands_on_the_last_pick() {
    let mut herdr = FakeHerdr::default();
    for n in 3..=6 {
        herdr.panes.push(FakePane {
            id: format!("w1:p{n}"),
            tab: "w1:t1".into(),
            workspace: "w1".into(),
            cwd: "/".into(),
            agent: None,
        });
    }
    let mut rig = Rig::with(herdr);
    rig.start();
    rig.run(3_000);
    let picks = [
        "w1:p2", "w1:p3", "w1:p4", "w1:p5", "w1:p6", "w1:p3", "w1:p2", "w1:p5",
    ];
    for (index, pane) in picks.iter().enumerate() {
        rig.act(Action::SelectPane((*pane).to_owned()));
        rig.run(37 + 13 * index as u64);
        rig.type_text("x");
    }
    rig.run(8_000);
    assert_eq!(rig.on_screen(), Some(term("w1:p5")));
    assert_eq!(rig.max_retargets, 1, "{:?}", rig.retargets);
    // Whatever typing went out landed on the pane on screen by the loop's
    // word, never one being left.
    for (place, bytes) in &rig.landed {
        assert!(
            bytes.is_empty() || place == &term("w1:p5"),
            "{place}: {bytes:?}"
        );
    }
}

// MARK: - The loop stops

#[test]
fn a_takeover_by_another_device_parks_and_only_the_person_takes_it_back() {
    let mut rig = Rig::booted();
    // Another device takes A.
    rig.held.push(term(A));
    rig.park(
        &term(A),
        0,
        "herdr: server shut down: terminal attach taken over",
    );
    rig.run(1_000);
    assert_eq!(
        rig.notice(),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere)))
    );
    rig.type_text("rm -rf x\r");
    rig.run(20_000);
    assert!(rig.landed_in(LOGIN_SHELL).is_empty());
    assert!(rig.landed_in(NO_PANE).is_empty());
    assert!(rig.landed_in(&term(A)).is_empty());
    // A poll's focus report (still A) never took it back.
    assert!(rig.taken.is_empty(), "{:?}", rig.retargets);
    // A reply for A: not sent, A not taken.
    rig.deliver(7, A, "y\r");
    rig.run(8_000);
    assert_eq!(rig.delivered.get(&7).map(|d| d.1), Some(false));
    assert!(rig.taken.is_empty());
    // The person's tap on A takes it back.
    rig.act(Action::SelectPane(A.to_owned()));
    rig.run(3_000);
    assert_eq!(rig.taken, vec![term(A)]);
    assert_eq!(rig.on_screen(), Some(term(A)));
}

/// B409: "Open here" after the other device moved the shared focus on:
/// the pane is the server's focus again too, as a pick's is, so the next
/// read keeps the loop on it instead of steering it to where the other
/// device went.
#[test]
fn open_here_sticks_after_the_other_device_moved_the_focus() {
    let mut rig = Rig::booted();
    rig.held.push(term(A));
    rig.park(
        &term(A),
        0,
        "herdr: server shut down: terminal attach taken over",
    );
    rig.run(1_000);
    assert_eq!(
        rig.notice(),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere)))
    );
    // The other device moves the focus on, to B.
    rig.herdr.focused = Some(B.to_owned());
    rig.run(10_000);
    rig.act(Action::NoticeAction);
    rig.run(10_000);
    assert_eq!(rig.taken, vec![term(A)]);
    assert_eq!(rig.on_screen(), Some(term(A)), "{:?}", rig.retargets);
    assert_eq!(rig.herdr.focused.as_deref(), Some(A));
    assert_eq!(rig.notice(), None);
}

/// B420: the herdr on the host's PATH and its server speak different
/// protocols (the field case: `api snapshot` refused over the protocol,
/// and the raw attach's next handshake hung up). The banner is the skew,
/// in versions, with its remedy, as the control plane names it; the raw
/// attach's "lost connection to server" and a Retry that can only be
/// refused again went over it, and its button retried the attach instead
/// of restarting the server.
#[test]
fn a_refused_attach_under_a_version_skew_shows_the_skew_and_its_restart() {
    let mut rig = Rig::booted();
    // brew upgraded the herdr on PATH; the server is still the old one.
    rig.herdr.mismatch = Some(
        "client protocol 22 is newer than server protocol 19; run herdr server stop".to_owned(),
    );
    rig.herdr.version = "0.8.0".to_owned();
    rig.act(Action::Refresh);
    rig.run(3_000);
    assert!(
        matches!(
            rig.notice(),
            Some((Notice::Skew(_), Some(NoticeAction::RestartServer)))
        ),
        "{:?}",
        rig.notice()
    );
    // The loop's attach ends: the new herdr's handshake, hung up.
    rig.park(
        &term(A),
        1,
        "herdr: lost connection to server: server closed connection",
    );
    rig.run(3_000);
    assert!(
        matches!(
            rig.notice(),
            Some((Notice::Skew(_), Some(NoticeAction::RestartServer)))
        ),
        "{:?}",
        rig.notice()
    );
    // Its button is the restart.
    rig.act(Action::NoticeAction);
    assert_eq!(
        rig.notice().map(|(notice, _)| notice),
        Some(Notice::Restarting)
    );
}

#[test]
fn a_closed_pane_goes_back_to_the_focused_pane_by_itself() {
    let mut rig = Rig::booted();
    rig.act(Action::SelectPane(B.to_owned()));
    rig.run(3_000);
    assert_eq!(rig.on_screen(), Some(term(B)));
    // B closes: herdr ends its attach, the focus goes to A.
    rig.herdr.panes.retain(|pane| pane.id != B);
    rig.herdr.focused = Some(A.to_owned());
    rig.park(
        &term(B),
        1,
        &format!(
            "herdr: server shut down: terminal attach ended: terminal {} not found",
            term(B)
        ),
    );
    rig.run(10_000);
    assert_eq!(rig.on_screen(), Some(term(A)), "{:?}", rig.retargets);
    assert_eq!(rig.notice(), None);
    assert_eq!(
        rig.retargets.last(),
        Some(&(term(A), false)),
        "a follow, not a take"
    );
}

/// B421 (suspected, not so): the pane on screen closes and a read drops it
/// from the tree before the loop says its attach ended. That same read
/// names the new focus, which steers the loop, and a retarget sent leaves
/// nothing on screen until the loop says where it came round to (W10c):
/// typing is dropped and said, so an arrow never goes to another pane as
/// its key (`renderer_pane`'s fallback to the focused pane is never asked).
#[test]
fn an_arrow_typed_once_the_pane_on_screen_left_the_tree_goes_to_no_other_pane() {
    let mut rig = Rig::booted();
    rig.act(Action::SelectPane(B.to_owned()));
    rig.run(3_000);
    assert_eq!(rig.on_screen(), Some(term(B)));
    let keys = rig.keys.len();
    // B closes; a read comes before the loop reports its attach ended.
    rig.herdr.panes.retain(|pane| pane.id != B);
    rig.herdr.focused = Some(A.to_owned());
    rig.act(Action::Refresh);
    for _ in 0..500 {
        if !rig.plane.view().snapshot.panes.contains_key(B) {
            break;
        }
        rig.run(10);
    }
    assert!(!rig.plane.view().snapshot.panes.contains_key(B), "no read");
    assert_eq!(rig.on_screen(), None, "on its way to A: nothing on screen");
    let now = rig.now;
    rig.plane.typed(b"\x1b[A", 0, now);
    rig.pump();
    rig.run(3_000);
    assert!(
        rig.keys[keys..].iter().all(|(pane, _)| pane != A),
        "{:?}",
        &rig.keys[keys..]
    );
    assert!(rig.landed_in(&term(A)).is_empty());
}

#[test]
fn a_herdr_restart_comes_back_on_the_new_focused_pane_by_itself() {
    let mut rig = Rig::booted();
    // The server stops (the loop parks on "shutting down"), then starts
    // with new terminals.
    rig.herdr.running = false;
    rig.park(
        &term(A),
        1,
        "herdr: server shut down: server is shutting down",
    );
    rig.run(3_000);
    rig.herdr.running = true;
    rig.herdr.panes = vec![FakePane {
        id: "w1:p9".into(),
        tab: "w1:t1".into(),
        workspace: "w1".into(),
        cwd: "/".into(),
        agent: None,
    }];
    rig.herdr.focused = Some("w1:p9".into());
    rig.run(15_000);
    assert_eq!(rig.on_screen(), Some(term("w1:p9")), "{:?}", rig.retargets);
    rig.type_text("ok");
    rig.run(500);
    assert_eq!(rig.landed_in(&term("w1:p9")), "ok");
    assert!(rig.landed_in(LOGIN_SHELL).is_empty());
}

/// B684: the server restarts and every device's attach ends with it. The
/// device that had the pane asks for it again at once, plainly; if another
/// device's person picked it in that moment, it stays theirs — said, never
/// taken over.
#[test]
fn a_restart_never_takes_a_pane_another_device_picked_meanwhile() {
    let mut rig = Rig::booted();
    rig.run(1_000);
    rig.herdr.running = false;
    rig.park(
        &term(A),
        1,
        "herdr: server shut down: server is shutting down",
    );
    rig.run(3_000);
    rig.herdr.running = true;
    rig.herdr.terminal_epoch = 1;
    let again = format!("{}_r1", term(A));
    rig.held.push(again.clone());
    let asked = rig.retargets.len();
    rig.run(15_000);
    assert!(
        rig.retargets[asked..].iter().all(|(_, takeover)| !takeover),
        "{:?}",
        rig.retargets
    );
    assert!(rig.taken.is_empty());
    assert_eq!(
        rig.notice(),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere)))
    );
}

/// B684: a device whose loop showed "open on another device" when the
/// server restarted waits a moment before following the focus again — the
/// device that had the pane asks at once and gets it — then asks plainly,
/// and gets it if nobody has.
#[test]
fn a_device_showing_the_notice_waits_its_moment_after_a_restart() {
    let mut rig = Rig::new();
    rig.held.push(term(A));
    rig.start();
    rig.run(3_000);
    assert_eq!(
        rig.notice(),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere)))
    );
    // The restart ends the other device's attach too.
    rig.herdr.running = false;
    rig.run(3_000);
    rig.held.clear();
    rig.herdr.running = true;
    rig.herdr.terminal_epoch = 1;
    let again = format!("{}_r1", term(A));
    let asked = rig.retargets.len();
    rig.run(4_000);
    assert!(
        !rig.retargets[asked..].iter().any(|(t, _)| *t == again),
        "asked at once: {:?}",
        rig.retargets
    );
    assert_eq!(
        rig.notice(),
        Some((Notice::OnAnotherDevice, Some(NoticeAction::OpenHere))),
        "still said while it waits"
    );
    rig.run(8_000);
    assert_eq!(rig.on_screen(), Some(again.clone()), "{:?}", rig.retargets);
    assert!(rig.retargets.contains(&(again, false)), "plainly");
}

#[test]
fn a_switch_to_a_pane_gone_meanwhile_leaves_the_screen_as_it_was() {
    let mut rig = Rig::booted();
    // B closed since the last read; the person taps it from a stale tree.
    rig.herdr.panes.retain(|pane| pane.id != B);
    rig.act(Action::SelectPane(B.to_owned()));
    rig.run(3_000);
    assert_eq!(rig.on_screen(), Some(term(A)));
    rig.type_text("still");
    rig.run(500);
    assert_eq!(rig.landed_in(&term(A)), "still");
}

// MARK: - The start line

#[test]
fn a_swallowed_start_line_is_typed_again_kill_line_first_and_bounded() {
    // A loop killed under a login shell: Retry boots it again; the starting
    // shell swallows the first two copies.
    let mut rig = Rig::booted();
    rig.state = Loop::Unborn;
    rig.attach += 1;
    rig.run(2_000);
    assert_eq!(
        rig.notice(),
        Some((Notice::InputDropped, Some(NoticeAction::Retry)))
    );
    rig.swallows = 2;
    rig.act(Action::NoticeAction);
    rig.run(20_000);
    assert!(rig.retypes.len() >= 3, "{}", rig.retypes.len());
    assert!(rig.retypes.iter().all(|line| line.first() == Some(&0x15)));
    assert!(rig.retypes.len() <= 4);
    assert!(matches!(rig.state, Loop::On(_)), "{:?}", rig.state);
    // Nothing else reached the login shell meanwhile.
    rig.type_text("q");
    assert!(rig.landed_in(LOGIN_SHELL).is_empty());

    // A shell that eats every copy: four in all, then said gone.
    let mut rig = Rig::booted();
    rig.state = Loop::Unborn;
    rig.attach += 1;
    rig.run(2_000);
    rig.swallows = 99;
    rig.retypes.clear();
    rig.act(Action::NoticeAction);
    rig.run(30_000);
    assert_eq!(rig.retypes.len(), 4);
    assert_eq!(
        rig.notice(),
        Some((Notice::InputDropped, Some(NoticeAction::Retry)))
    );
}

// MARK: - mosh's order

#[test]
fn a_switch_waits_for_the_typing_before_it_to_reach_the_tty() {
    let mut rig = Rig::booted();
    rig.one_way = 400;
    rig.type_text("for-a");
    rig.act(Action::SelectPane(B.to_owned()));
    rig.run(5_000);
    assert_eq!(rig.on_screen(), Some(term(B)));
    assert_eq!(rig.landed_in(&term(A)), "for-a", "{:?}", rig.landed);
    assert!(rig.landed_in(&term(B)).is_empty());
}

/// B686: a switch waits for the typing before it however long mosh takes
/// to acknowledge it: on a path slower than the old 1.5 s bound (a 4 s
/// round trip), the retarget went anyway and the keystrokes still on their
/// way landed in the pane switched to.
#[test]
fn a_switch_on_a_slow_path_never_takes_the_typing_before_it() {
    let mut rig = Rig::booted();
    rig.one_way = 2_000;
    rig.type_text("for-a");
    rig.act(Action::SelectPane(B.to_owned()));
    rig.run(15_000);
    assert_eq!(rig.landed_in(&term(A)), "for-a", "{:?}", rig.log);
    assert!(rig.landed_in(&term(B)).is_empty(), "{:?}", rig.log);
    assert_eq!(rig.on_screen(), Some(term(B)), "{:?}", rig.log);
}

/// B686: across a roam (UDP gone for 8 s, then back), the typing before a
/// switch lands in the pane it was typed for, and the switch lands after
/// it — the renderer's budget for the switch counts from when it went, not
/// from when it was asked.
#[test]
fn a_switch_across_a_roaming_gap_lands_after_the_typing_before_it() {
    let mut rig = Rig::booted();
    rig.udp_down = true;
    rig.type_text("for-a");
    rig.act(Action::SelectPane(B.to_owned()));
    rig.run(8_000);
    assert!(rig.landed.values().all(|bytes| bytes.is_empty()));
    rig.path_back();
    rig.run(10_000);
    assert_eq!(rig.landed_in(&term(A)), "for-a", "{:?}", rig.log);
    assert!(rig.landed_in(&term(B)).is_empty(), "{:?}", rig.log);
    assert_eq!(rig.on_screen(), Some(term(B)), "{:?}", rig.log);
    rig.type_text("for-b");
    rig.run(2_000);
    assert_eq!(rig.landed_in(&term(B)), "for-b", "{:?}", rig.log);
}

/// B686: an arrow waits for the typing before it however slow the path:
/// past the old 1.5 s bound herdr typed the key ahead of it.
#[test]
fn an_arrow_on_a_slow_path_waits_for_the_typing_before_it() {
    let mut rig = Rig::booted();
    rig.one_way = 2_000;
    rig.type_text("ab");
    rig.type_text("\x1b[A");
    rig.type_text("cd");
    rig.run(15_000);
    assert_eq!(rig.keys, vec![(A.to_owned(), "up".to_owned())]);
    assert_eq!(rig.landed_in(&term(A)), "abcd");
    let (ab, up, cd) = (rig.at(" ab"), rig.at("key up"), rig.at(" cd"));
    assert!(
        ab.is_some() && up.is_some() && cd.is_some(),
        "{:?}",
        rig.log
    );
    assert!(ab < up && up < cd, "{:?}", rig.log);
}

/// B686: an arrow pressed while the control connection is being redialled
/// never reaches the pane after the typing behind it. Down then Return in a
/// menu picked the default: the Return went once the key's 3 s were up,
/// and the key ran on the next connection. Past its bound the key and the
/// typing behind it are dropped and said, and the key never runs late.
#[test]
fn an_arrow_while_the_control_connection_is_down_never_lands_after_what_follows() {
    let mut rig = Rig::booted();
    rig.drop_connection();
    rig.type_text("\x1b[B");
    rig.type_text("\r");
    rig.run(5_000);
    rig.reconnect();
    rig.run(3_000);
    assert!(rig.keys.is_empty(), "the key ran late: {:?}", rig.log);
    assert!(rig.landed_in(&term(A)).is_empty(), "{:?}", rig.log);
    assert_eq!(rig.dropped, 4, "said, with the Return behind it");
    // What is typed next goes, in order.
    rig.type_text("x");
    rig.run(1_000);
    assert_eq!(rig.landed_in(&term(A)), "x", "{:?}", rig.log);
}

/// B686: a connection back within the key's bound: the key and the typing
/// behind it go, in order.
#[test]
fn an_arrow_waits_out_a_quick_redial_in_order() {
    let mut rig = Rig::booted();
    rig.drop_connection();
    rig.type_text("\x1b[B");
    rig.type_text("\r");
    rig.run(1_000);
    assert!(rig.landed_in(&term(A)).is_empty(), "{:?}", rig.log);
    rig.reconnect();
    rig.run(3_000);
    assert_eq!(rig.keys, vec![(A.to_owned(), "down".to_owned())]);
    assert_eq!(rig.landed_in(&term(A)), "\r");
    assert!(rig.at("key down") < rig.at(" \r"), "{:?}", rig.log);
    assert_eq!(rig.dropped, 0);
}

/// B689: typing held behind mosh-server's acknowledgement is never
/// invisible: past a second the plane says so, and says when it goes.
#[test]
fn typing_held_past_a_second_is_said_and_so_is_its_going() {
    let mut rig = Rig::booted();
    rig.udp_down = true;
    let start = rig.now;
    rig.type_text("ab");
    rig.type_text("\x1b[A");
    rig.type_text("\r");
    rig.run(900);
    assert!(rig.held_said.is_empty(), "not before a second");
    rig.run(300);
    assert_eq!(rig.held_said, vec![(start + 1_000, true)]);
    rig.path_back();
    rig.run(3_000);
    assert_eq!(
        rig.held_said.iter().map(|h| h.1).collect::<Vec<_>>(),
        [true, false]
    );
    assert_eq!(rig.keys, vec![(A.to_owned(), "up".to_owned())]);
    assert_eq!(rig.landed_in(&term(A)), "ab\r");
}

/// B689: closing the plane while typing is held says how much never
/// reached mosh, and that nothing waits any more.
#[test]
fn closing_while_typing_is_held_lets_it_go_said() {
    let mut rig = Rig::booted();
    rig.udp_down = true;
    rig.type_text("ab");
    rig.type_text("\x1b[A");
    rig.type_text("\r");
    rig.run(1_500);
    assert_eq!(rig.held_said.last().map(|h| h.1), Some(true));
    let now = rig.now;
    assert_eq!(rig.plane.close(now), 4, "the key and the Return");
    rig.pump();
    assert_eq!(rig.held_said.last().map(|h| h.1), Some(false));
}

/// B686: what the lane still holds when the plane closes (the mosh leg
/// going: a reconnect, the end) is said, never dropped in silence.
#[test]
fn typing_held_in_the_lane_when_the_plane_closes_is_said() {
    let mut rig = Rig::booted();
    rig.udp_down = true;
    rig.type_text("ab");
    rig.type_text("\x1b[A");
    rig.type_text("cd");
    rig.run(2_000);
    let now = rig.now;
    assert_eq!(rig.plane.close(now), 5, "the key and what's behind it");
}

#[test]
fn arrows_go_as_keys_herdr_encodes_in_order_with_the_typing() {
    let mut rig = Rig::booted();
    rig.one_way = 120;
    rig.type_text("ab");
    let now = rig.now;
    rig.plane.typed(b"\x1b[A", 0, now);
    rig.plane.typed(b"\x1bOA", 0, now);
    rig.pump();
    rig.type_text("cd");
    rig.run(3_000);
    assert_eq!(
        rig.keys,
        vec![
            (A.to_owned(), "up".to_owned()),
            (A.to_owned(), "up".to_owned())
        ]
    );
    assert_eq!(rig.landed_in(&term(A)), "abcd");
    // In the pane, in the order pressed: "ab" in the tty before herdr typed
    // the keys, "cd" after.
    let at = |needle: &str| {
        rig.log
            .iter()
            .position(|line| line.ends_with(needle))
            .unwrap_or_else(|| panic!("{needle}: {:?}", rig.log))
    };
    assert!(at(" ab") < at("key up"), "{:?}", rig.log);
    assert!(at("key up") < at(" cd"), "{:?}", rig.log);
}

/// B360: the shell's view hands over what was written since its last read
/// as one chunk; its arrows still go as keys, in order with the text.
#[test]
fn arrows_that_arrive_together_still_go_as_keys_in_order() {
    let mut rig = Rig::booted();
    rig.one_way = 120;
    rig.type_text("ab\x1b[A\x1b[Acd\x1bOB");
    rig.run(3_000);
    assert_eq!(
        rig.keys,
        vec![
            (A.to_owned(), "up".to_owned()),
            (A.to_owned(), "up".to_owned()),
            (A.to_owned(), "down".to_owned())
        ]
    );
    assert_eq!(rig.landed_in(&term(A)), "abcd");
    let at = |needle: &str| {
        rig.log
            .iter()
            .position(|line| line.ends_with(needle))
            .unwrap_or_else(|| panic!("{needle}: {:?}", rig.log))
    };
    assert!(at(" ab") < at("key up"), "{:?}", rig.log);
    assert!(at("key up") < at(" cd"), "{:?}", rig.log);
    assert!(at(" cd") < at("key down"), "{:?}", rig.log);
}

/// An arrow inside a paste is pasted text, not a key.
#[test]
fn an_arrow_inside_a_paste_is_pasted_not_pressed() {
    let mut rig = Rig::booted();
    let now = rig.now;
    rig.plane.paste("a\x1b[Ab", 0, now);
    rig.pump();
    rig.run(2_000);
    assert!(rig.keys.is_empty(), "{:?}", rig.keys);
    // A paste carries no ESC (B738): what lands is the text around it.
    assert_eq!(rig.landed_in(&term(A)), "\x1b[200~a[Ab\x1b[201~");
}

/// B687: a paste the session waits on is said landed once mosh-server has
/// it, not when it leaves: what follows (an image path's Return) must not
/// join its retransmitted diff.
#[test]
fn a_paste_is_said_landed_once_mosh_server_has_it() {
    let mut rig = Rig::booted();
    rig.one_way = 2_000;
    let start = rig.now;
    rig.plane.paste_then(" /tmp/a.jpg ", 0, 7, start);
    rig.pump();
    rig.run(1_000);
    assert!(rig.pastes.is_empty(), "not on its way out");
    rig.run(9_000);
    assert_eq!(rig.pastes.len(), 1, "{:?}", rig.pastes);
    let (at, token, landed) = rig.pastes[0];
    assert_eq!((token, landed), (7, true));
    assert!(at >= start + 4_000, "acknowledged: {at} {start}");
    assert_eq!(rig.landed_in(&term(A)), "\x1b[200~ /tmp/a.jpg \x1b[201~");
}

/// B687: a paste dropped mid-switch is said not landed at once, so the
/// Return sent after it doesn't go on its own.
#[test]
fn a_paste_dropped_mid_switch_is_said_not_landed() {
    let mut rig = Rig::booted();
    rig.act(Action::SelectPane(B.to_owned()));
    let now = rig.now;
    rig.plane.paste_then("p", 0, 8, now);
    rig.pump();
    assert_eq!(rig.pastes, vec![(now, 8, false)]);
    assert!(rig.dropped > 0);
}

/// B687: a paste queued behind a key that never went is dropped with it,
/// and said not landed.
#[test]
fn a_paste_behind_a_key_that_never_went_is_said_not_landed() {
    let mut rig = Rig::booted();
    rig.drop_connection();
    rig.type_text("\x1b[B");
    let now = rig.now;
    rig.plane.paste_then("p", 0, 9, now);
    rig.pump();
    rig.run(5_000);
    assert_eq!(
        rig.pastes.iter().map(|p| (p.1, p.2)).collect::<Vec<_>>(),
        vec![(9, false)]
    );
    assert!(rig.landed_in(&term(A)).is_empty(), "{:?}", rig.log);
}

#[test]
fn typing_mid_switch_is_dropped_and_said_for_a_moment() {
    let mut rig = Rig::booted();
    rig.act(Action::SelectPane(B.to_owned()));
    rig.type_text("lost");
    assert_eq!(rig.dropped, 4);
    assert_eq!(rig.notice(), Some((Notice::InputDropped, None)));
    rig.run(5_000);
    assert_eq!(rig.notice(), None, "a one-off notice stays");
    assert!(rig.landed.values().all(|bytes| bytes.is_empty()));
}

#[test]
fn a_herdr_without_terminal_attach_runs_its_own_tui_and_zooms_server_side() {
    // mosh-server runs herdr's TUI: no renderer, the server's focus is what
    // mosh shows (zoomed), typing waits out a switch.
    let config = PlaneConfig::new(KEY, None, None, false);
    let mut rig = Rig::new();
    rig.plane = Plane::new(config);
    rig.state = Loop::On("tui".into());
    rig.start();
    rig.run(2_000);
    rig.act(Action::SelectPane(B.to_owned()));
    rig.type_text("mid");
    assert_eq!(rig.dropped, 3, "typed while the zoom hadn't landed");
    rig.run(2_000);
    assert_eq!(rig.herdr.focused.as_deref(), Some(B));
    assert!(
        rig.herdr.landed.iter().any(|l| l == &format!("zoom {B}")),
        "{:?}",
        rig.herdr.landed
    );
    rig.type_text("ok");
    rig.run(500);
    assert_eq!(rig.landed_in("tui"), "ok");
    // A reply for A: focused, then typed, said sent once mosh has it.
    rig.deliver(1, A, "r\r");
    rig.run(8_000);
    assert_eq!(rig.delivered.get(&1).map(|d| d.1), Some(true));
    assert_eq!(rig.herdr.focused.as_deref(), Some(A));
    assert_eq!(rig.landed_in("tui"), "okr\r");
    assert!(rig.retargets.is_empty());
}

/// herdr's own TUI on mosh, its focus the server's, booted on A.
fn tui_rig() -> Rig {
    let mut rig = Rig::new();
    rig.plane = Plane::new(PlaneConfig::new(KEY, None, None, false));
    rig.state = Loop::On("tui".into());
    rig.tui = true;
    rig.start();
    rig.run(2_000);
    assert_eq!(rig.herdr.focused.as_deref(), Some(A));
    rig
}

/// B688: herdr's own TUI types into the server's focused pane, so a switch
/// the person asks for waits for the typing before it to reach the tty, as
/// the raw-attach loop's does (B686): the focus moved on the server while
/// the keystrokes were still on their way, and they landed in the pane
/// switched to.
#[test]
fn the_full_tuis_switch_waits_for_the_typing_before_it() {
    let mut rig = tui_rig();
    rig.one_way = 400;
    rig.type_text("for-a");
    rig.act(Action::SelectPane(B.to_owned()));
    rig.type_text("mid");
    assert_eq!(rig.dropped, 3, "typed while the switch hadn't landed");
    rig.run(5_000);
    assert_eq!(rig.herdr.focused.as_deref(), Some(B));
    assert_eq!(rig.landed_in(&tui(A)), "for-a", "{:?}", rig.log);
    assert!(rig.landed_in(&tui(B)).is_empty(), "{:?}", rig.log);
    rig.type_text("for-b");
    rig.run(2_000);
    assert_eq!(rig.landed_in(&tui(B)), "for-b", "{:?}", rig.log);
}

/// B688: a reply to another pane under herdr's own TUI: the typing before
/// it lands where it was typed, the reply in its pane.
#[test]
fn the_full_tuis_reply_to_another_pane_waits_for_the_typing_before_it() {
    let mut rig = tui_rig();
    rig.one_way = 400;
    rig.type_text("for-a");
    rig.deliver(1, B, "r\r");
    rig.run(10_000);
    assert_eq!(rig.delivered.get(&1).map(|d| d.1), Some(true));
    assert_eq!(rig.landed_in(&tui(A)), "for-a", "{:?}", rig.log);
    assert_eq!(rig.landed_in(&tui(B)), "r\r", "{:?}", rig.log);
}

// MARK: - Replies

#[test]
fn a_reply_to_an_unfocused_pane_is_switched_to_and_said_sent_once_mosh_has_it() {
    let mut rig = Rig::booted();
    rig.deliver(1, B, "yes\r");
    rig.run(10_000);
    assert_eq!(rig.delivered.get(&1).map(|d| d.1), Some(true));
    assert_eq!(rig.landed_in(&term(B)), "yes\r");
    assert!(rig.landed_in(&term(A)).is_empty());
    // Not before mosh-server acknowledged it.
    let (said_at, _) = rig.delivered[&1];
    assert!(said_at >= rig.now.saturating_sub(10_000));
    assert!(rig.herdr.focused.as_deref() == Some(B));
}

#[test]
fn a_reply_waits_for_the_tree_and_is_refused_for_a_pane_it_lacks() {
    let mut rig = Rig::new();
    // Before the control plane has read anything.
    rig.deliver(1, B, "1\r");
    rig.deliver(2, "w9:p9", "2\r");
    rig.start();
    rig.run(10_000);
    assert_eq!(rig.delivered.get(&1).map(|d| d.1), Some(true));
    assert_eq!(rig.delivered.get(&2).map(|d| d.1), Some(false));
    assert_eq!(rig.landed_in(&term(B)), "1\r");
    assert!(!rig.landed.values().any(|bytes| bytes.ends_with(b"2\r")));
}

#[test]
fn a_reply_lost_on_a_silent_path_is_never_said_sent() {
    let mut rig = Rig::booted();
    rig.udp_down = true;
    rig.deliver(1, B, "allow\r");
    rig.run(9_000);
    assert_eq!(rig.delivered.get(&1).map(|d| d.1), Some(false));
}

#[test]
fn a_reply_to_the_pane_on_screen_is_checked_not_reattached() {
    let mut rig = Rig::booted();
    let before = rig.retargets.len();
    rig.deliver(1, A, "go\r");
    rig.run(5_000);
    assert_eq!(rig.delivered.get(&1).map(|d| d.1), Some(true));
    assert_eq!(rig.retargets.len(), before, "a check, not a bounce");
    assert_eq!(rig.landed_in(&term(A)), "go\r");
}

// MARK: - The veil

#[test]
fn the_switch_veil_lifts_after_the_new_panes_repaint() {
    let mut rig = Rig::booted();
    rig.marks.clear();
    rig.act(Action::SelectPane(B.to_owned()));
    rig.run(3_000);
    let veiled = rig
        .marks
        .iter()
        .find(|(_, code)| *code == view_marks::VEIL)
        .map(|(at, _)| *at)
        .expect("veiled");
    let revealed = rig
        .marks
        .iter()
        .find(|(_, code)| *code == view_marks::REVEAL)
        .map(|(at, _)| *at)
        .expect("revealed");
    assert!(
        rig.marks
            .iter()
            .any(|(_, code)| *code == view_marks::EXTEND_COVER),
        "{:?}",
        rig.marks
    );
    // The retarget lands 180 ms on, the repaint ends 300 ms after that:
    // revealed only once that has been quiet.
    assert!(revealed >= veiled + 180 + 300, "{veiled} → {revealed}");
}

#[test]
fn switching_back_while_a_switch_away_is_out_never_types_into_the_pane_switched_away_to() {
    // Live, before the fix: a switch storm's keystrokes in the other pane.
    let mut rig = Rig::booted();
    rig.act(Action::SelectPane(B.to_owned()));
    rig.run(60);
    rig.act(Action::SelectPane(A.to_owned()));
    rig.type_text("mid");
    rig.run(5_000);
    assert_eq!(rig.on_screen(), Some(term(A)));
    assert!(rig.landed_in(&term(B)).is_empty(), "{:?}", rig.landed);
    rig.type_text("after");
    rig.run(500);
    assert_eq!(rig.landed_in(&term(A)), "after");
}
