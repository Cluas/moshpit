//! mosh + herdr's raw-attach renderer ([`moshpit_herdr::renderer`]) against
//! a fake host on a virtual clock: the sidecar's execs answer a round trip
//! later, the way the app's own tests model the host
//! (`Tests/MoshpitTests/Services/Herdr/MoshRawAttachRetargetTests.swift`),
//! the loop is attached, parked (its attach ended on its own — another
//! device took the pane, the pane closed, the server stopped) or gone, and
//! every byte sent down mosh is recorded with where it would land.

use std::collections::HashMap;

use moshpit_herdr::launch;
use moshpit_herdr::renderer::{DeliveryId, Down, ExecId, Notice, Output, Renderer};

/// Where a keystroke lands once the loop itself has ended: mosh shows the
/// login shell, which runs what it is given as a command line.
const LOGIN_SHELL: &str = "the login shell";
/// Where it lands while the loop is parked or waiting: the tty, read by no
/// one (and discarded before the next attach).
const NO_PANE: &str = "no pane";
const KEY: &str = "6F9619FF-8B86-D011-B42D-00C04FC964FF-abc12345";
const TAKEN_OVER: &str = "herdr: server shut down: terminal attach taken over";

/// herdr 0.9.5's refusal of a plain attach to a terminal another client
/// holds, measured live.
fn held_words(terminal: &str) -> String {
    format!(
        "herdr: server shut down: terminal attach failed: terminal {terminal} already has an attached client; retry with --takeover"
    )
}

/// The loop under mosh.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Loop {
    /// Its start line was swallowed by a login shell still starting: the
    /// shell has the tty, and no loop runs.
    Unborn,
    /// Booted, no target yet.
    Waiting,
    /// Attached to this terminal.
    On(String),
    /// Its attach ended on its own: the stop record.
    Parked {
        terminal: String,
        status: i64,
        said: String,
    },
    /// The loop shell itself is gone.
    Gone,
}

/// The host end of the sidecar. A retarget lands — the target written, a
/// parked loop woken, the attach bounced — when its exec answers,
/// `delay(terminal)` after it was asked, and says so (the marker) when a
/// loop is there to come round; a gone loop is said instead. A check and a
/// watch answer from the same state; a watch answers once the attach it was
/// armed on stops (100 ms later: its host-side looks) or is replaced.
struct Host {
    state: Loop,
    /// Bumped by every attach: what a watch is armed on.
    attach: u64,
    landed: Vec<String>,
    retargets: usize,
    checks: usize,
    watches: usize,
    in_flight: usize,
    max_in_flight: usize,
    failing: HashMap<String, usize>,
    /// Terminals the server no longer lists.
    missing: Vec<String>,
    /// Terminals another device's client holds: a plain attach to one is
    /// refused (the loop parks on it), one with `--takeover` takes it.
    held: Vec<String>,
    /// Terminals our `--takeover` took from the other device, in order.
    taken_from_others: Vec<String>,
    /// Every retarget, with whether it asked for `--takeover`.
    asked: Vec<(String, bool)>,
    /// Copies of the start line the starting shell swallows (the host's
    /// own first one among them) before one runs.
    boot_swallows: u32,
    /// Copies of the start line typed so far, the host's own first one
    /// included.
    copies: u32,
    /// The retyped start lines, as typed.
    retypes: Vec<String>,
    /// What the host was asked, in order: `boot`, `retype`, `retarget
    /// <terminal>`, `check <terminal>`, `watch`.
    log: Vec<String>,
    delay: Box<dyn Fn(&str) -> u64>,
}

impl Host {
    fn on_screen(&self) -> String {
        match &self.state {
            Loop::On(terminal) => terminal.clone(),
            Loop::Gone | Loop::Unborn => LOGIN_SHELL.to_owned(),
            Loop::Waiting | Loop::Parked { .. } => NO_PANE.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Boot,
    Retarget,
    Check,
    Watch,
}

/// What an exec asked: a retarget's terminal (`printf '%s' '<id>'`), a
/// check's (`= '<id>' ]`), or a watch.
fn asked(command: &str) -> (Kind, String) {
    let quoted = |after: &str| {
        let start = command
            .find(after)
            .map(|at| at + after.len())
            .expect("a terminal id");
        command[start..].split('\'').next().unwrap().to_owned()
    };
    if command.starts_with("w=; read w") {
        (Kind::Watch, String::new())
    } else if command.contains(&format!("echo {}", launch::LOOP_UP_MARKER)) {
        (Kind::Boot, String::new())
    } else if command.contains(&format!("echo {}", launch::RAW_ATTACH_UP_MARKER)) {
        (Kind::Check, quoted("\" = '"))
    } else {
        (Kind::Retarget, quoted("printf '%s' '"))
    }
}

/// Whether a retarget asks for `--takeover`: it names its terminal in the
/// take file (a plain one removes it), or — the loop before round
/// twenty-eight — it has none, and the loop took over whatever it attached.
fn takes_over(command: &str) -> bool {
    let Some(at) = command.find(".take\"; ") else {
        return true;
    };
    let before = &command[..at];
    let start = before.rfind("; ").map_or(0, |at| at + 2);
    !before[start..].starts_with("rm -f ")
}

/// An exec out: when it answers (a watch: once its attach stops).
struct Pending {
    id: ExecId,
    command: String,
    kind: Kind,
    due: Option<u64>,
    /// A watch's attach.
    armed_on: u64,
}

/// A renderer and its host on one virtual clock.
struct Rig {
    renderer: Renderer,
    host: Host,
    now: u64,
    pending: Vec<Pending>,
    outputs: Vec<(u64, Output)>,
    /// Every byte sent down mosh, and where it landed.
    sent: Vec<(String, Vec<u8>)>,
    /// When mosh-server acknowledges each retyped start line.
    acks: Vec<u64>,
}

impl Rig {
    fn new() -> Self {
        let mut rig = Rig {
            renderer: Renderer::new(KEY, None),
            host: Host {
                state: Loop::Waiting,
                attach: 0,
                landed: Vec::new(),
                retargets: 0,
                checks: 0,
                watches: 0,
                in_flight: 0,
                max_in_flight: 0,
                failing: HashMap::new(),
                missing: Vec::new(),
                held: Vec::new(),
                taken_from_others: Vec::new(),
                asked: Vec::new(),
                boot_swallows: 0,
                copies: 1,
                retypes: Vec::new(),
                log: Vec::new(),
                delay: Box::new(|_| 0),
            },
            now: 0,
            pending: Vec::new(),
            outputs: Vec::new(),
            sent: Vec::new(),
            acks: Vec::new(),
        };
        rig.tree(&["t-1", "t-2", "t-3"]);
        rig
    }

    fn tree(&mut self, listed: &[&str]) {
        self.renderer
            .tree(listed.iter().map(|t| (*t).to_owned()), true, self.now);
        self.drain();
    }

    /// The focus report after a focus change the person asked for (a tab
    /// or workspace picked, B683): followed.
    fn show(&mut self, terminal: &str) {
        self.renderer.follow_focus(self.now);
        self.renderer.show(terminal, self.now);
        self.drain();
    }

    /// The focus report as every poll repeats it — the desktop's moves
    /// included (B683: not followed once the loop is on a pane).
    fn report(&mut self, terminal: &str) {
        self.renderer.show(terminal, self.now);
        self.drain();
    }

    fn pick(&mut self, terminal: &str) {
        self.renderer.pick(terminal, self.now);
        self.drain();
    }

    fn take_back(&mut self) -> Option<String> {
        let line = self.renderer.take_back(self.now);
        self.drain();
        line
    }

    fn type_bytes(&mut self, bytes: &[u8]) -> bool {
        let sent = self.renderer.typed(bytes, self.now);
        self.drain();
        sent
    }

    fn deliver(&mut self, id: u64, terminal: &str, bytes: &[u8]) {
        self.renderer
            .deliver(DeliveryId(id), terminal, bytes.to_vec(), self.now);
        self.drain();
    }

    /// Something on the host ends the attach on its own: the loop parks.
    fn park(&mut self, said: &str) {
        if let Loop::On(terminal) = self.host.state.clone() {
            self.host.state = Loop::Parked {
                terminal,
                status: 1,
                said: said.to_owned(),
            };
            self.wake_watches(|_| true);
        }
    }

    fn taken_nothing(&self) -> bool {
        self.host.taken_from_others.is_empty()
    }

    fn kill_loop(&mut self) {
        self.host.state = Loop::Gone;
        self.wake_watches(|_| true);
    }

    /// Watches waiting on an attach `which` picks answer their next look,
    /// 100 ms on.
    fn wake_watches(&mut self, which: impl Fn(u64) -> bool) {
        for pending in &mut self.pending {
            if pending.kind == Kind::Watch && pending.due.is_none() && which(pending.armed_on) {
                pending.due = Some(self.now + 100);
            }
        }
    }

    /// Carries out what the renderer asked: execs go out, bytes land.
    fn drain(&mut self) {
        while let Some(output) = self.renderer.poll_output() {
            match &output {
                Output::Exec { id, command } => {
                    let (kind, terminal) = asked(command);
                    self.host.log.push(match kind {
                        Kind::Boot => "boot".to_owned(),
                        Kind::Retarget => format!("retarget {terminal}"),
                        Kind::Check => format!("check {terminal}"),
                        Kind::Watch => "watch".to_owned(),
                    });
                    let due = match kind {
                        // The host's looks (up to 1.5 s) end at once here:
                        // the loop runs or the line was swallowed.
                        Kind::Boot => Some(self.now + (self.host.delay)("")),
                        Kind::Retarget => {
                            self.host.retargets += 1;
                            self.host.in_flight += 1;
                            self.host.max_in_flight =
                                self.host.max_in_flight.max(self.host.in_flight);
                            Some(self.now + (self.host.delay)(&terminal))
                        }
                        Kind::Check => {
                            self.host.checks += 1;
                            Some(self.now + (self.host.delay)(&terminal))
                        }
                        Kind::Watch => {
                            self.host.watches += 1;
                            // On an attach: until it stops (or a minute on).
                            // Otherwise its first look says so.
                            match &self.host.state {
                                Loop::On(_) => None,
                                _ => Some(self.now + 100),
                            }
                        }
                    };
                    self.pending.push(Pending {
                        id: *id,
                        command: command.clone(),
                        kind,
                        due,
                        armed_on: self.host.attach,
                    });
                }
                Output::Send(bytes) => self.sent.push((self.host.on_screen(), bytes.clone())),
                Output::Retype(bytes) => {
                    // Typed into the shell: it runs once the shell has
                    // swallowed what it swallows. Acked 30 ms on.
                    self.host.log.push("retype".to_owned());
                    self.host
                        .retypes
                        .push(String::from_utf8_lossy(bytes).into_owned());
                    self.host.copies += 1;
                    if self.host.state == Loop::Unborn && self.host.copies > self.host.boot_swallows
                    {
                        self.host.state = Loop::Waiting;
                    }
                    self.acks.push(self.now + 30);
                }
                _ => {}
            }
            self.outputs.push((self.now, output));
        }
    }

    /// Runs the clock `ms` on: execs answer when due, timers fire.
    fn run_for(&mut self, ms: u64) {
        let end = self.now + ms;
        loop {
            self.drain();
            let exec = self.pending.iter().filter_map(|pending| pending.due).min();
            let timer = self.renderer.next_deadline();
            let ack = self.acks.iter().copied().min();
            let Some(next) = [exec, timer, ack]
                .into_iter()
                .flatten()
                .min()
                .filter(|&at| at <= end)
            else {
                break;
            };
            self.now = self.now.max(next);
            if ack == Some(next) {
                self.acks.retain(|&at| at != next);
                self.renderer.retyped(self.now);
            } else if exec == Some(next) {
                let index = self
                    .pending
                    .iter()
                    .position(|pending| pending.due == Some(next))
                    .unwrap();
                let pending = self.pending.remove(index);
                let output = self.answer(&pending);
                self.renderer
                    .exec_finished(pending.id, output.as_deref(), self.now);
            } else {
                self.renderer.tick(self.now);
            }
        }
        self.now = end;
        self.renderer.tick(self.now);
        self.drain();
    }

    /// The host's answer to `pending`, now.
    fn answer(&mut self, pending: &Pending) -> Option<Vec<u8>> {
        let (kind, terminal) = asked(&pending.command);
        let stopped = match &self.host.state {
            Loop::Parked {
                terminal,
                status,
                said,
            } => Some(format!(
                "MOSHPIT_RENDERER_STOPPED\n{status} {terminal}\n{said}\n"
            )),
            Loop::Gone => Some("MOSHPIT_RENDERER_GONE\n".to_owned()),
            _ => None,
        };
        match kind {
            Kind::Boot => {
                let up = !matches!(self.host.state, Loop::Unborn | Loop::Gone);
                Some(if up {
                    b"MOSHPIT_LOOP_UP\n".to_vec()
                } else {
                    Vec::new()
                })
            }
            Kind::Watch => {
                // Replaced by another attach: nothing to say.
                let replaced = pending.armed_on != self.host.attach;
                let said = if replaced {
                    String::new()
                } else {
                    stopped.unwrap_or_default()
                };
                Some(said.into_bytes())
            }
            Kind::Check => {
                let up = self.host.state == Loop::On(terminal);
                let mut said = if up {
                    "MOSHPIT_ATTACHED\n".to_owned()
                } else {
                    String::new()
                };
                said.push_str(&stopped.unwrap_or_default());
                Some(said.into_bytes())
            }
            Kind::Retarget => {
                self.host.in_flight -= 1;
                if let Some(left) = self
                    .host
                    .failing
                    .get_mut(&terminal)
                    .filter(|left| **left > 0)
                {
                    *left -= 1;
                    return None;
                }
                if self.host.missing.contains(&terminal) {
                    // Refused before the loop is touched.
                    return Some(b"MOSHPIT_RETARGET_MISSING\n".to_vec());
                }
                // A loop that is gone, or never started (its pid file
                // empty when the retarget's 2 s run out).
                if matches!(self.host.state, Loop::Gone | Loop::Unborn) {
                    return Some(b"MOSHPIT_RENDERER_GONE\n".to_vec());
                }
                // The command as the host runs it: the take file names the
                // terminal (`--takeover`), or is removed (a plain attach).
                // A loop without one takes over whatever it attaches.
                let takeover = takes_over(&pending.command);
                self.host.asked.push((terminal.clone(), takeover));
                if let Some(at) = self.host.held.iter().position(|held| *held == terminal) {
                    if takeover {
                        self.host.held.remove(at);
                        self.host.taken_from_others.push(terminal.clone());
                    } else {
                        // herdr refuses the plain attach ~260 ms in; the
                        // loop parks on it, and the retarget's settle
                        // prints the stop record.
                        let said = held_words(&terminal);
                        self.host.state = Loop::Parked {
                            terminal: terminal.clone(),
                            status: 1,
                            said: said.clone(),
                        };
                        self.host.attach += 1;
                        return Some(
                            format!("MOSHPIT_RENDERER_STOPPED\n1 {terminal}\n{said}\n")
                                .into_bytes(),
                        );
                    }
                }
                // Written, a parked loop woken, the attach bounced.
                self.host.landed.push(terminal.clone());
                self.host.state = Loop::On(terminal);
                self.host.attach += 1;
                let attach = self.host.attach;
                self.wake_watches(|armed_on| armed_on != attach);
                Some(b"MOSHPIT_RETARGETED\n".to_vec())
            }
        }
    }

    fn sent_where(&self) -> Vec<String> {
        self.sent.iter().map(|(at, _)| at.clone()).collect()
    }

    fn sent_to(&self, terminal: &str) -> bool {
        self.sent.iter().any(|(at, _)| at == terminal)
    }

    fn said(&self, notice: &Notice) -> bool {
        self.outputs
            .iter()
            .any(|(_, output)| matches!(output, Output::Notice(said) if said == notice))
    }

    fn cleared(&self) -> bool {
        self.outputs
            .iter()
            .any(|(_, output)| matches!(output, Output::ClearNotice))
    }

    fn delivered(&self, id: u64) -> Option<bool> {
        self.outputs.iter().find_map(|(_, output)| match output {
            Output::Delivered { id: answered, sent } if answered.0 == id => Some(*sent),
            _ => None,
        })
    }

    fn delivered_at(&self, id: u64) -> Option<u64> {
        self.outputs.iter().find_map(|(at, output)| match output {
            Output::Delivered { id: answered, .. } if answered.0 == id => Some(*at),
            _ => None,
        })
    }

    fn veils(&self) -> usize {
        self.outputs
            .iter()
            .filter(|(_, output)| matches!(output, Output::Veil))
            .count()
    }
}

// MARK: - B683: the desktop's focus

/// The first report lands the loop on the focused pane; after that, the
/// desktop moving the focus no longer moves it (the phone switched under
/// the person and dropped what they typed); a focus change the person
/// asked for still does, and so does the pane on screen going away.
#[test]
fn the_desktops_focus_moves_the_loop_only_at_first() {
    let mut rig = Rig::new();
    rig.report("t-1");
    rig.run_for(50);
    assert_eq!(rig.host.landed, ["t-1"], "the first report lands");
    for _ in 0..3 {
        rig.report("t-2");
        rig.run_for(100);
    }
    assert_eq!(rig.host.landed, ["t-1"], "the desktop's moves don't");
    assert!(rig.renderer.typed(b"x", rig.now), "typing goes on");
    rig.show("t-3");
    rig.run_for(100);
    assert_eq!(rig.host.landed, ["t-1", "t-3"], "the person's own move does");
}

// MARK: - Round twenty-five (B208)

#[test]
fn two_quick_switches_leave_the_loop_on_the_pane_picked_last() {
    // The first switch's exec is the slow one, as any real link makes it
    // when the second tap comes right after.
    let mut rig = Rig::new();
    rig.host.delay = Box::new(|terminal| if terminal == "t-2" { 150 } else { 0 });
    rig.show("t-1");
    rig.run_for(50);
    assert_eq!(rig.host.landed, ["t-1"]);

    rig.show("t-2");
    rig.run_for(20); // its exec is out
    rig.show("t-3");
    rig.run_for(500);

    assert_eq!(
        rig.host.landed.last().map(String::as_str),
        Some("t-3"),
        "the loop was left on {:?}",
        rig.host.landed
    );
    assert_eq!(rig.host.max_in_flight, 1, "one retarget exec at a time");
}

#[test]
fn a_switch_queued_behind_another_goes_straight_to_the_newest_pane() {
    let mut rig = Rig::new();
    rig.host.delay = Box::new(|terminal| if terminal == "t-1" { 150 } else { 0 });
    rig.show("t-1");
    rig.run_for(20);
    rig.show("t-2"); // queued behind t-1…
    rig.show("t-3"); // …and overtaken before its turn
    rig.run_for(400);
    assert_eq!(
        rig.host.landed,
        ["t-1", "t-3"],
        "no bounce through a pane nobody wants any more"
    );
}

#[test]
fn a_retarget_whose_exec_failed_is_asked_for_again_by_the_next_focus_report() {
    let mut rig = Rig::new();
    rig.host.failing.insert("t-2".to_owned(), 1);
    rig.show("t-1");
    rig.run_for(50);
    rig.show("t-2"); // the exec fails: still on t-1
    rig.run_for(100);
    assert_eq!(rig.host.landed, ["t-1"]);
    // The poll repeats the focus every tick: a retry, not "already showing it".
    rig.show("t-2");
    rig.run_for(100);
    assert_eq!(rig.host.landed.last().map(String::as_str), Some("t-2"));
}

#[test]
fn the_same_focus_reported_again_costs_nothing_once_it_is_on_screen() {
    let mut rig = Rig::new();
    rig.show("t-2");
    rig.run_for(50);
    for _ in 0..3 {
        rig.show("t-2");
    }
    rig.run_for(100);
    assert_eq!(
        rig.host.landed,
        ["t-2"],
        "each repeat would bounce the attach — a visible flicker"
    );
}

#[test]
fn a_lane_step_that_never_answers_gives_way_after_its_budget_and_a_late_landing_is_redone() {
    let mut rig = Rig::new();
    rig.host.delay = Box::new(|terminal| if terminal == "t-2" { 6_000 } else { 0 });
    rig.show("t-1");
    rig.run_for(50);
    rig.show("t-2"); // hangs past the lane's 5 s
    rig.run_for(5_100);
    assert_eq!(rig.renderer.on_screen(), None);
    rig.show("t-3"); // not held up behind it
    rig.run_for(200);
    assert_eq!(rig.renderer.on_screen(), Some("t-3"));
    // t-2's exec lands at 6 s and writes over t-3: the loop is pointed at
    // what is wanted now, once more.
    rig.run_for(1_000);
    assert_eq!(rig.host.landed.last().map(String::as_str), Some("t-3"));
    assert_eq!(rig.renderer.on_screen(), Some("t-3"));
}

#[test]
fn typing_right_after_a_switch_never_reaches_the_pane_being_left() {
    let mut rig = Rig::new();
    rig.host.delay = Box::new(|terminal| if terminal == "t-2" { 300 } else { 0 });
    rig.show("t-1");
    rig.run_for(100);

    rig.show("t-2");
    assert!(
        !rig.type_bytes(b"y"),
        "typed in the same breath, it was sent"
    );
    assert!(!rig.type_bytes(b"\x1b[A"));
    rig.run_for(400);
    assert!(
        !rig.sent_to("t-1"),
        "typed after the switch, it went into the pane being left"
    );
    assert!(
        rig.said(&Notice::NotOnPane),
        "the keys were dropped without a word"
    );

    // Once the loop is on the new terminal, typing reaches it.
    assert!(rig.type_bytes(b"n"));
    assert!(rig.sent_to("t-2"));
}

#[test]
fn a_reply_leaves_only_once_the_loop_is_on_its_panes_terminal() {
    let mut rig = Rig::new();
    rig.host.delay = Box::new(|terminal| if terminal == "t-2" { 300 } else { 0 });
    rig.show("t-1");
    rig.run_for(50);

    rig.deliver(1, "t-2", b"y");
    rig.run_for(500);

    assert_eq!(rig.delivered(1), Some(true));
    assert_eq!(
        rig.sent_where(),
        ["t-2"],
        "the reply went out while the loop was elsewhere"
    );
}

#[test]
fn a_reply_into_a_loop_that_is_gone_is_not_sent_and_says_so_at_once() {
    let mut rig = Rig::new();
    rig.host.delay = Box::new(|_| 60);
    rig.show("t-1");
    rig.run_for(150);
    // The loop shell itself ends: mosh shows the login shell.
    rig.host.state = Loop::Gone;

    rig.deliver(1, "t-2", b"y\r");
    rig.run_for(500);

    assert_eq!(rig.host.retargets, 2, "the switch was asked for");
    assert_eq!(rig.delivered(1), Some(false), "reported as sent");
    assert!(
        rig.sent_where().is_empty(),
        "typed into {:?} as a command line",
        rig.sent_where()
    );
    assert!(rig.said(&Notice::CouldntSwitch), "the failure was silent");
    assert!(
        rig.delivered_at(1).unwrap() < 1_000,
        "answered at the end of the 5 s wait"
    );
}

#[test]
fn a_reply_for_the_pane_on_screen_goes_once_the_loop_is_seen_still_on_it() {
    let mut rig = Rig::new();
    rig.host.delay = Box::new(|_| 60);
    rig.show("t-1");
    rig.run_for(150);

    rig.deliver(1, "t-1", b"y\r");
    rig.run_for(300);

    assert_eq!(rig.delivered(1), Some(true));
    assert_eq!(rig.sent_where(), ["t-1"]);
    assert_eq!(rig.host.checks, 1, "the loop is asked about once");
    assert_eq!(
        rig.host.landed,
        ["t-1"],
        "asked, never re-attached: a retarget would take the pane back"
    );
}

#[test]
fn the_veil_goes_up_for_a_switch_and_not_for_the_first_attach() {
    let mut rig = Rig::new();
    rig.show("t-1");
    rig.run_for(50);
    assert_eq!(rig.veils(), 0, "the first attach has nothing stale to hide");
    rig.show("t-2");
    rig.run_for(50);
    assert_eq!(rig.veils(), 1);
}

// MARK: - A renderer that stopped: who brings it back

/// On t-1, landed and watched.
fn on_first_pane() -> Rig {
    let mut rig = Rig::new();
    rig.host.delay = Box::new(|_| 60);
    rig.show("t-1");
    rig.run_for(200);
    assert_eq!(rig.renderer.on_screen(), Some("t-1"));
    assert_eq!(rig.host.watches, 1, "no watch on the attach");
    rig
}

#[test]
fn a_pane_another_device_takes_over_is_said_and_never_taken_back_on_its_own() {
    let mut rig = on_first_pane();
    rig.park(TAKEN_OVER);
    // The other device keeps it (a plain ask is refused: B487).
    rig.host.held = vec!["t-1".to_owned()];
    rig.run_for(300);

    assert_eq!(
        rig.renderer.down(),
        Some(&Down::TakenOver("t-1".to_owned()))
    );
    assert!(rig.said(&Notice::OnAnotherDevice));
    // Typing and dictation from here: dropped, never into a tty or a shell.
    assert!(!rig.type_bytes(b"rm -rf build\r"));
    // The poll repeats the focus, and the desktop moves it: neither takes a
    // pane from the device that has it.
    for focus in ["t-1", "t-2", "t-1"] {
        rig.show(focus);
        rig.run_for(2_000);
    }
    assert!(
        rig.host.asked.iter().all(|(_, takeover)| !takeover),
        "a focus report took a pane back: {:?}",
        rig.host.asked
    );
    assert!(
        rig.taken_nothing(),
        "taken: {:?}",
        rig.host.taken_from_others
    );
    assert_eq!(
        rig.renderer.down(),
        Some(&Down::TakenOver("t-1".to_owned()))
    );
    assert!(rig.sent.is_empty(), "typed into {:?}", rig.sent_where());

    // One tap: taken back (`--takeover`), and typing reaches it again.
    assert_eq!(rig.take_back(), None);
    rig.run_for(200);
    assert_eq!(rig.host.landed, ["t-1", "t-1"]);
    assert_eq!(rig.host.asked.last(), Some(&("t-1".to_owned(), true)));
    assert_eq!(rig.renderer.down(), None);
    assert!(rig.cleared(), "the notice stayed up");
    assert!(rig.type_bytes(b"y"));
    assert_eq!(rig.sent_where(), ["t-1"]);
}

#[test]
fn a_pick_of_another_pane_brings_a_taken_over_renderer_back_there() {
    let mut rig = on_first_pane();
    rig.park(TAKEN_OVER);
    rig.run_for(300);
    rig.pick("t-2");
    rig.run_for(200);
    assert_eq!(rig.renderer.on_screen(), Some("t-2"));
    assert_eq!(rig.renderer.down(), None);
    assert!(rig.type_bytes(b"y"));
    assert_eq!(rig.sent_where(), ["t-2"]);
}

#[test]
fn a_reply_for_the_pane_another_device_has_is_refused_without_taking_it_back() {
    let mut rig = on_first_pane();
    rig.park(TAKEN_OVER);
    rig.run_for(300);
    rig.deliver(1, "t-1", b"y\r");
    rig.run_for(300);
    assert_eq!(rig.delivered(1), Some(false));
    assert_eq!(rig.host.retargets, 1, "taken back for a reply");
    assert!(rig.sent.is_empty());
}

#[test]
fn a_reply_that_beats_the_watch_to_a_takeover_reads_the_park_and_takes_nothing_back() {
    let mut rig = on_first_pane();
    rig.park(TAKEN_OVER);
    // Before the watch's next look: the check finds the park.
    rig.deliver(1, "t-1", b"y\r");
    rig.run_for(300);
    assert_eq!(rig.delivered(1), Some(false));
    assert_eq!(
        rig.renderer.down(),
        Some(&Down::TakenOver("t-1".to_owned()))
    );
    // The next report of the same focus asks nothing of the loop.
    rig.show("t-1");
    rig.run_for(2_000);
    assert_eq!(
        rig.host.retargets, 1,
        "the pane was taken back from the other device"
    );
}

#[test]
fn a_closed_pane_brings_the_renderer_to_the_focused_pane_by_itself() {
    let mut rig = on_first_pane();
    rig.park("herdr: server shut down: terminal attach ended: terminal t-1 not found");
    rig.run_for(300);
    assert_eq!(rig.renderer.down(), Some(&Down::Gone("t-1".to_owned())));
    assert!(
        rig.outputs
            .iter()
            .any(|(_, output)| *output == Output::RefreshTree)
    );
    // Typed before the focused pane is up: dropped and said.
    assert!(!rig.type_bytes(b"y"));
    assert!(rig.said(&Notice::NotOnPane));
    // The stale report of the closed pane isn't followed; the fresh one is.
    rig.show("t-1");
    rig.run_for(100);
    assert_eq!(rig.host.retargets, 1);
    rig.tree(&["t-2", "t-3"]);
    rig.show("t-2");
    rig.run_for(200);
    assert_eq!(rig.renderer.on_screen(), Some("t-2"));
    assert_eq!(rig.renderer.down(), None);
    assert!(rig.type_bytes(b"n"));
    assert_eq!(rig.sent_where(), ["t-2"]);
}

#[test]
fn a_herdr_restart_brings_the_renderer_to_the_focused_panes_new_terminal() {
    let mut rig = on_first_pane();
    rig.park("herdr: server shut down: server is shutting down");
    rig.run_for(300);
    assert_eq!(rig.renderer.down(), Some(&Down::Gone("t-1".to_owned())));
    // The restarted server: the same panes, new terminal ids.
    rig.tree(&["t-7", "t-8"]);
    rig.show("t-7");
    rig.run_for(200);
    assert_eq!(rig.renderer.on_screen(), Some("t-7"));
    assert!(rig.type_bytes(b"y"));
    assert_eq!(rig.sent_where(), ["t-7"]);
}

#[test]
fn a_pane_taken_over_and_then_closed_is_left_for_the_focused_one() {
    let mut rig = on_first_pane();
    rig.park(TAKEN_OVER);
    // Still theirs: B487's plain ask is refused.
    rig.host.held = vec!["t-1".to_owned()];
    rig.run_for(300);
    rig.show("t-2"); // not followed while taken over…
    rig.run_for(100);
    assert!(
        rig.host
            .asked
            .iter()
            .all(|(terminal, takeover)| terminal != "t-2" && !takeover)
    );
    rig.tree(&["t-2", "t-3"]); // …until its pane is gone
    rig.run_for(200);
    assert_eq!(rig.renderer.on_screen(), Some("t-2"));
    assert!(rig.cleared(), "the takeover notice outlived its pane");
}

#[test]
fn a_loop_that_is_gone_is_said_and_booted_again_on_the_persons_tap() {
    let mut rig = on_first_pane();
    rig.kill_loop();
    rig.run_for(300);
    assert_eq!(rig.renderer.down(), Some(&Down::LoopGone));
    assert!(rig.said(&Notice::LoopGone));
    assert!(!rig.type_bytes(b"ls\r"));
    assert!(rig.sent.is_empty(), "typed into the login shell");

    let line = rig.take_back().expect("a boot line");
    assert!(line.starts_with('\u{15}') && line.ends_with('\r'));
    assert!(
        line.contains(rig.renderer.key()),
        "the new generation's files"
    );
    assert_ne!(rig.renderer.key(), KEY);
    // The host types it; the new loop comes round to the focused pane.
    rig.host.state = Loop::Waiting;
    rig.run_for(300);
    assert_eq!(rig.renderer.on_screen(), Some("t-1"));
    assert_eq!(rig.renderer.down(), None);
}

#[test]
fn a_watch_whose_exec_failed_is_asked_again() {
    let mut rig = on_first_pane();
    let id = rig
        .pending
        .iter()
        .find(|pending| pending.kind == Kind::Watch)
        .unwrap()
        .id;
    rig.pending.retain(|pending| pending.id != id);
    rig.renderer.exec_finished(id, None, rig.now);
    rig.drain();
    rig.run_for(2_500);
    assert_eq!(rig.host.watches, 2);
    rig.park(TAKEN_OVER);
    rig.run_for(300);
    assert_eq!(
        rig.renderer.down(),
        Some(&Down::TakenOver("t-1".to_owned()))
    );
}

#[test]
fn nothing_typed_reaches_a_shell_whatever_ends_the_attach() {
    for said in [
        TAKEN_OVER,
        "herdr: server shut down: terminal attach ended: terminal t-1 not found",
        "herdr: server shut down: server is shutting down",
        "herdr: something new",
    ] {
        let mut rig = on_first_pane();
        rig.park(said);
        // Typed before the watch has looked, and after.
        let _ = rig.type_bytes(b"echo one\r");
        rig.run_for(300);
        let _ = rig.type_bytes(b"echo two\r");
        rig.run_for(300);
        assert!(
            rig.sent.iter().all(|(at, _)| at != LOGIN_SHELL),
            "{said}: typed into the login shell"
        );
        // Only what went before the watch knew reaches the parked loop's
        // tty (discarded before its next attach); nothing after.
        assert!(rig.sent.len() <= 1, "{said}: {:?}", rig.sent_where());
    }
}

// MARK: - A switch to a pane that is gone

#[test]
fn a_switch_to_a_terminal_the_server_no_longer_lists_never_counts_and_leaves_the_pane_on_screen() {
    let mut rig = on_first_pane();
    // t-2's pane closed since the tree was read; the stale tree still has it.
    rig.host.missing.push("t-2".to_owned());
    rig.show("t-2");
    rig.run_for(300);

    assert_eq!(rig.host.landed, ["t-1"], "the attach on screen was bounced");
    assert_eq!(rig.renderer.on_screen(), Some("t-1"));
    assert!(
        !rig.renderer.switching(),
        "a refused switch is still under way"
    );
    assert_eq!(rig.renderer.down(), None, "the renderer went down");
    assert!(
        rig.outputs
            .iter()
            .any(|(_, output)| *output == Output::RefreshTree)
    );
    // What is on screen still takes typing.
    assert!(rig.type_bytes(b"y"));
    assert_eq!(rig.sent_where(), ["t-1"]);
    // The stale report of the closed pane isn't asked again.
    rig.show("t-2");
    rig.run_for(2_000);
    assert_eq!(rig.host.retargets, 2);
}

#[test]
fn a_reply_for_a_pane_that_is_gone_is_answered_not_sent_at_once() {
    let mut rig = on_first_pane();
    rig.host.missing.push("t-2".to_owned());
    rig.deliver(1, "t-2", b"y\r");
    rig.run_for(300);
    assert_eq!(rig.delivered(1), Some(false));
    assert!(
        rig.delivered_at(1).unwrap() < rig.now - 100,
        "waited out its 5 s"
    );
    assert!(rig.sent.is_empty(), "typed into {:?}", rig.sent_where());
    assert_eq!(rig.renderer.on_screen(), Some("t-1"));
}

// MARK: - Before the first switch

#[test]
fn typing_before_the_first_switch_lands_goes_nowhere_not_into_the_first_pane_attached() {
    let mut rig = Rig::new();
    rig.host.delay = Box::new(|_| 60);
    // The loop is booted and shows no pane yet: the tty would keep what is
    // typed for whichever attach comes first.
    assert!(
        !rig.type_bytes(b"y\r"),
        "typed before any pane was on screen"
    );
    assert!(rig.said(&Notice::NotOnPane), "dropped without a word");
    rig.show("t-1");
    assert!(
        !rig.type_bytes(b"y"),
        "typed while the first switch was out"
    );
    rig.run_for(200);
    assert!(rig.sent.is_empty(), "{:?}", rig.sent_where());
    assert!(rig.type_bytes(b"n"));
    assert_eq!(rig.sent_where(), ["t-1"]);
}

/// W10c: a switch back to the pane on screen while a switch away from it is
/// still out. The renderer took the pane it had landed on last for what the
/// screen shows and let typing through, and the retarget in flight then moved
/// the loop: the keys reached the pane switched away to (live on herdr 0.9.5
/// and mosh-server: a switch storm's keystrokes in the other pane, 3 runs of
/// 4).
#[test]
fn switching_back_while_a_switch_away_is_out_types_nothing_until_the_loop_is_back() {
    let mut rig = on_first_pane();
    rig.host.delay = Box::new(|_| 150);
    rig.pick("t-2");
    rig.run_for(20);
    rig.pick("t-1");
    assert!(
        !rig.type_bytes(b"x"),
        "typed while the loop was on its way to t-2"
    );
    rig.run_for(100);
    assert!(!rig.type_bytes(b"x"));
    rig.run_for(2_000);
    assert_eq!(rig.renderer.on_screen(), Some("t-1"));
    assert!(rig.type_bytes(b"y"));
    assert_eq!(rig.sent_where(), ["t-1"]);
}

// MARK: - The switch veil

impl Rig {
    fn paint(&mut self) {
        self.renderer.painted(self.now);
        self.drain();
    }

    fn revealed_at(&self) -> Option<u64> {
        self.outputs
            .iter()
            .find_map(|(at, output)| matches!(output, Output::Reveal).then_some(*at))
    }
}

#[test]
fn the_veil_lifts_on_the_new_panes_repaint_not_on_the_loops_own_noise() {
    let mut rig = on_first_pane();
    rig.host.delay = Box::new(|_| 120);
    let began = rig.now;
    rig.show("t-2");
    // bash's "Killed: 9" ~10 ms after the kill; the pane being left still
    // printing; then quiet until herdr repaints ~250 ms on. The landing's
    // exec comes back at +120.
    rig.run_for(10);
    rig.paint();
    rig.run_for(40);
    rig.paint();
    rig.run_for(200);
    assert_eq!(rig.revealed_at(), None, "lifted on the loop's own noise");
    rig.paint(); // the new pane, +250
    rig.run_for(20);
    rig.paint();
    rig.run_for(500);
    let lifted = rig.revealed_at().expect("never lifted") - began;
    assert_eq!(
        lifted,
        120 + 400,
        "the repaint's last bytes +270, quiet by +420; the landing's grace to +520"
    );
}

#[test]
fn a_repaint_that_beat_the_landing_back_lifts_the_veil_after_the_repaint_grace() {
    let mut rig = on_first_pane();
    rig.host.delay = Box::new(|_| 120);
    let began = rig.now;
    rig.show("t-2");
    rig.run_for(80);
    rig.paint(); // the repaint arrived before the exec's answer
    rig.run_for(1_000);
    let lifted = rig.revealed_at().expect("never lifted") - began;
    assert_eq!(
        lifted,
        120 + 400,
        "the landing at +120, then the grace for bytes that never came"
    );
}

/// W10c: a takeover lands as soon as its attach has a pid; herdr repaints
/// ~250 ms after the attach starts. 150 ms of quiet counted from the landing
/// lifted the veil on the loop's blank screen before the repaint.
#[test]
fn the_veil_waits_for_a_repaint_that_comes_well_after_the_landing() {
    let mut rig = on_first_pane();
    rig.host.delay = Box::new(|_| 120);
    let began = rig.now;
    rig.pick("t-2");
    rig.run_for(120 + 250);
    assert_eq!(rig.revealed_at(), None, "lifted before the repaint");
    rig.paint(); // the repaint, 250 ms after the landing
    rig.run_for(30);
    rig.paint();
    rig.run_for(1_000);
    let lifted = rig.revealed_at().expect("never lifted") - began;
    assert_eq!(lifted, 120 + 250 + 30 + 150);
}

#[test]
fn a_switch_that_ends_without_a_pane_to_show_takes_the_veil_down_at_once() {
    let mut rig = on_first_pane();
    rig.host.failing.insert("t-2".to_owned(), 1);
    rig.show("t-2");
    rig.run_for(100);
    assert_eq!(
        rig.revealed_at(),
        Some(rig.now - 100 + 60),
        "kept up past the failed exec"
    );

    let mut rig = on_first_pane();
    rig.host.missing.push("t-3".to_owned());
    rig.show("t-3");
    rig.run_for(100);
    assert!(rig.revealed_at().is_some(), "kept up over a refused switch");
    assert!(!rig.renderer.veiled());
}

#[test]
fn a_veil_with_nothing_painting_under_it_lifts_at_its_cap() {
    let mut rig = on_first_pane();
    rig.host.delay = Box::new(|_| 10_000);
    let began = rig.now;
    rig.show("t-2");
    rig.run_for(4_000);
    assert_eq!(rig.revealed_at().map(|at| at - began), Some(3_000));
}

// MARK: - Round twenty-eight: following the focus never takes a pane

/// The shared focus moved onto a pane another device holds (that device's
/// person picked it, or the desktop did). Following it took the pane over:
/// the loop ran every attach with `--takeover`, and the other device lost
/// its pane. A follow is a plain attach now; herdr refuses it, and the
/// renderer says so as it says a takeover.
#[test]
fn following_the_focus_onto_a_pane_another_device_holds_takes_nothing_and_says_so() {
    let mut rig = on_first_pane();
    rig.host.held = vec!["t-2".to_owned()];
    rig.show("t-2");
    rig.run_for(300);
    assert!(
        rig.host.taken_from_others.is_empty(),
        "following the focus took the pane from the device that has it: {:?}",
        rig.host.asked
    );
    assert_eq!(rig.host.held, ["t-2"]);
    assert_eq!(rig.host.asked.last(), Some(&("t-2".to_owned(), false)));
    assert_eq!(
        rig.renderer.down(),
        Some(&Down::TakenOver("t-2".to_owned()))
    );
    assert!(rig.said(&Notice::OnAnotherDevice));
    assert!(
        !rig.renderer.veiled(),
        "the veil outlived a switch that won't paint"
    );
    // Nothing typed goes anywhere, and the poll's repeats ask nothing.
    assert!(!rig.type_bytes(b"y\r"));
    assert!(rig.sent.is_empty(), "typed into {:?}", rig.sent_where());
    // Once the focus leaves it, B487 asks for it plainly, refused while
    // theirs: at most once in these six seconds, and never a takeover.
    let asked = rig.host.retargets;
    let before = rig.host.asked.len();
    for focus in ["t-2", "t-1", "t-2"] {
        rig.show(focus);
        rig.run_for(2_000);
    }
    assert!(
        rig.host.retargets <= asked + 1,
        "a focus report retried the pane"
    );
    assert!(rig.host.asked.iter().all(|(_, takeover)| !takeover));
    assert!(
        rig.host.asked[before..]
            .iter()
            .all(|(terminal, _)| terminal != "t-1"),
        "followed the focus: {:?}",
        rig.host.asked
    );
    assert_eq!(
        rig.renderer.down(),
        Some(&Down::TakenOver("t-2".to_owned()))
    );

    // "Open here": the person's word takes it.
    assert_eq!(rig.take_back(), None);
    rig.run_for(200);
    assert_eq!(rig.host.taken_from_others, ["t-2"]);
    assert_eq!(rig.host.asked.last(), Some(&("t-2".to_owned(), true)));
    assert_eq!(rig.renderer.on_screen(), Some("t-2"));
    assert_eq!(rig.renderer.down(), None);
    assert!(rig.cleared(), "the notice stayed up");
    assert!(rig.type_bytes(b"y"));
    assert_eq!(rig.sent_where(), ["t-2"]);
}

/// `--takeover` only on the person's word: a pick, a reply for a pane, the
/// take back. The poll's report, the focused pane after a close, and the
/// focused pane of a loop booted again are follows.
#[test]
fn only_the_persons_word_attaches_with_takeover() {
    let mut rig = Rig::new();
    rig.host.delay = Box::new(|_| 60);
    rig.show("t-1");
    rig.run_for(200);
    rig.pick("t-2");
    rig.run_for(200);
    rig.deliver(1, "t-3", b"y\r");
    rig.run_for(300);
    assert_eq!(rig.delivered(1), Some(true));
    rig.show("t-1");
    rig.run_for(800);
    // The pane on screen closes: the focused one, by itself.
    rig.park("herdr: server shut down: terminal attach ended: terminal t-1 not found");
    rig.run_for(300);
    rig.tree(&["t-2", "t-3"]);
    rig.show("t-2");
    rig.run_for(800);
    // The loop is gone, and the person boots it again: the focused pane.
    rig.kill_loop();
    rig.run_for(300);
    rig.take_back().expect("a boot line");
    rig.host.state = Loop::Waiting;
    rig.run_for(800);
    let asked: Vec<(&str, bool)> = rig
        .host
        .asked
        .iter()
        .map(|(terminal, takeover)| (terminal.as_str(), *takeover))
        .collect();
    assert_eq!(
        asked,
        [
            ("t-1", false),
            ("t-2", true),
            ("t-3", true),
            ("t-1", false),
            ("t-2", false),
            ("t-2", false),
        ]
    );
}

/// The person taps the pane a follow is on its way to, and another device
/// holds it: the follow's plain attach is refused, then the tap's attach
/// takes it.
#[test]
fn a_tap_on_the_pane_a_follow_is_on_its_way_to_takes_it() {
    let mut rig = on_first_pane();
    rig.host.held = vec!["t-2".to_owned()];
    rig.show("t-2");
    rig.run_for(20);
    rig.pick("t-2");
    rig.run_for(400);
    assert_eq!(rig.host.taken_from_others, ["t-2"]);
    assert_eq!(rig.renderer.on_screen(), Some("t-2"));
    assert_eq!(rig.renderer.down(), None);
    assert!(rig.type_bytes(b"y"));
    assert_eq!(rig.sent_where(), ["t-2"]);
}

// MARK: - Round twenty-eight: the start line is typed again

/// The login shell was still starting when the start line was typed, and
/// swallowed it (Powerlevel10k's instant prompt flushes pending input): no
/// loop ran. The first switch then said the loop was gone after 2 s, and
/// only the person's Retry typed the line again.
#[test]
fn a_start_line_a_starting_shell_swallowed_is_typed_again_before_any_target() {
    let mut rig = Rig::new();
    rig.host.state = Loop::Unborn;
    rig.host.boot_swallows = 1;
    rig.show("t-1");
    rig.run_for(500);
    assert_eq!(rig.renderer.on_screen(), Some("t-1"), "{:?}", rig.host.log);
    assert_eq!(rig.renderer.down(), None);
    assert_eq!(rig.host.retypes.len(), 1);
    let line = &rig.host.retypes[0];
    assert!(
        line.starts_with('\u{15}') && line.ends_with('\r'),
        "{line:?}"
    );
    assert!(line.contains(KEY), "the same generation's line");
    // The target goes only once the loop runs: its first read of the tty
    // takes any copy typed since away.
    assert_eq!(
        rig.host.log,
        ["boot", "retype", "boot", "retarget t-1", "watch"]
    );
    assert!(!rig.said(&Notice::LoopGone));
}

/// A line the shell keeps swallowing is typed four times in all; then the
/// loop is said gone, and the person's Retry types it once more — in the
/// same generation, so a copy still waiting in the shell runs it too.
#[test]
fn a_start_line_never_answered_goes_four_times_then_waits_for_the_person() {
    let mut rig = Rig::new();
    rig.host.state = Loop::Unborn;
    rig.host.boot_swallows = 99;
    rig.show("t-1");
    rig.run_for(2_000);
    assert_eq!(rig.host.copies, 4, "{:?}", rig.host.log);
    assert_eq!(rig.renderer.down(), Some(&Down::LoopGone));
    assert!(rig.said(&Notice::LoopGone));
    assert!(
        !rig.host
            .log
            .iter()
            .any(|asked| asked.starts_with("retarget")),
        "a target was published for a loop that never started"
    );
    assert!(!rig.type_bytes(b"ls\r"));
    assert!(rig.sent.is_empty(), "typed into the login shell");
    // Retry: the same key, the line typed again; it runs.
    let line = rig.take_back().expect("a start line");
    assert!(
        line.contains(KEY),
        "a new generation for a loop that never ran"
    );
    rig.host.boot_swallows = 0;
    rig.host.state = Loop::Waiting;
    rig.run_for(500);
    assert_eq!(rig.renderer.on_screen(), Some("t-1"));
    assert_eq!(rig.renderer.down(), None);
}

/// A loop that came up with its first line costs one check.
#[test]
fn a_loop_that_came_up_with_its_first_line_costs_one_check() {
    let mut rig = Rig::new();
    rig.show("t-1");
    rig.run_for(200);
    assert_eq!(rig.renderer.on_screen(), Some("t-1"));
    assert_eq!(rig.host.log, ["boot", "retarget t-1", "watch"]);
    assert!(rig.host.retypes.is_empty());
    rig.show("t-2");
    rig.run_for(200);
    assert_eq!(
        rig.host.log,
        ["boot", "retarget t-1", "watch", "retarget t-2", "watch"],
        "asked again once booted"
    );
}

// MARK: - B471: a pick whose retarget couldn't run

/// The person picked a pane another device holds, and the retarget couldn't
/// run (the control connection dropped under it, mosh carrying on). The
/// next report of the focus — the pick's own, asked again on the new
/// connection (B470) — came as a follow: a plain attach, refused, "on
/// another device". The person's tap took nothing. Now it is still theirs.
#[test]
fn a_pick_whose_retarget_couldnt_run_still_takes_the_pane_on_the_next_report() {
    let mut rig = on_first_pane();
    rig.host.held = vec!["t-2".to_owned()];
    rig.host.failing.insert("t-2".to_owned(), 1);
    rig.pick("t-2");
    rig.run_for(100);
    assert_ne!(rig.renderer.on_screen(), Some("t-2"), "it couldn't run");
    rig.show("t-2");
    rig.run_for(400);
    assert_eq!(
        rig.host.taken_from_others,
        ["t-2"],
        "the person's pick, with --takeover"
    );
    assert_eq!(rig.renderer.on_screen(), Some("t-2"));
    assert_eq!(rig.renderer.down(), None);
}

/// A report of another pane in between ends it: the focus moved on, and
/// the old pick is no one's to take.
#[test]
fn a_lost_pick_is_forgotten_once_the_focus_moves_elsewhere() {
    let mut rig = on_first_pane();
    rig.host.held = vec!["t-2".to_owned()];
    rig.host.failing.insert("t-2".to_owned(), 1);
    rig.pick("t-2");
    rig.run_for(100);
    rig.show("t-3");
    rig.run_for(400);
    rig.show("t-2");
    rig.run_for(400);
    assert!(
        rig.host.taken_from_others.is_empty(),
        "a follow takes nothing"
    );
}

// MARK: - B487: a terminal the other device left comes back by itself

/// Another device took the terminal on screen, then moved on (the focus
/// went with it, and its client let the terminal go). The loop stayed
/// parked under "on another device" until the person's tap. Now a plain
/// retarget — never a takeover — asks for it again when the focus moves
/// away, at most every ten seconds; once it lands, the other device's
/// focus doesn't pull the loop after it.
#[test]
fn a_terminal_the_other_device_left_comes_back_with_a_plain_retarget() {
    let mut rig = on_first_pane();
    rig.park(TAKEN_OVER);
    rig.run_for(300);
    assert_eq!(
        rig.renderer.down(),
        Some(&Down::TakenOver("t-1".to_owned()))
    );
    // The other device is on t-2 now, and holds only that.
    rig.host.held = vec!["t-2".to_owned()];
    rig.show("t-2");
    rig.run_for(500);
    assert_eq!(rig.renderer.on_screen(), Some("t-1"), "ours again");
    assert_eq!(rig.renderer.down(), None);
    assert!(rig.cleared(), "the notice went");
    assert!(rig.taken_nothing());
    assert_eq!(rig.host.asked.last(), Some(&("t-1".to_owned(), false)));
    for _ in 0..5 {
        rig.show("t-2");
        rig.run_for(2_000);
    }
    assert_eq!(
        rig.renderer.on_screen(),
        Some("t-1"),
        "their focus doesn't pull us after it"
    );
    assert!(rig.host.asked.iter().all(|(terminal, _)| terminal != "t-2"));
}
