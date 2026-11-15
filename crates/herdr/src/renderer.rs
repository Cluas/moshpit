//! mosh + herdr's raw-attach renderer as a sans-I/O state machine: what the
//! mosh+herdr half of `Moshpit/Services/SessionHub.swift` does around
//! [`crate::launch`]'s command lines, on the caller's clock.
//!
//! Under mosh the screen is one pane's raw stream: a loop in the mosh shell
//! keeps `herdr terminal attach <terminal>` pointed at a target file
//! ([`launch::raw_attach_loop_command`]), and the sidecar's execs steer it
//! ([`launch::retarget_command`]). Keystrokes travel the mosh channel into
//! whatever the loop is attached to, so where they land is decided here.
//!
//! **Inputs**, each with the caller's monotonic clock in milliseconds: the
//! focused pane's terminal ([`Renderer::show`], repeated by every poll of the
//! control plane) and the terminals the server lists ([`Renderer::tree`]),
//! the person's pick ([`Renderer::pick`]) and take back
//! ([`Renderer::take_back`]), typing ([`Renderer::typed`]), one pane's input
//! on purpose ([`Renderer::deliver`]), the answer to an exec it asked for
//! ([`Renderer::exec_finished`]) and timer ticks ([`Renderer::tick`] once
//! [`Renderer::next_deadline`] has passed).
//!
//! **Outputs** queue up in order ([`Renderer::poll_output`]): commands for
//! fresh exec channels on the sidecar, bytes for the mosh channel, the
//! answers to deliveries, the switch veil, notices.
//!
//! **The rules**:
//!
//! - The loop's start line is confirmed before the loop is first given a
//!   target ([`launch::raw_attach_loop_up_command`]): a login shell still
//!   starting can swallow a line typed into it, and the loop never started —
//!   the first switch then said the loop was gone after 2 s, and only the
//!   person's Retry typed the line again. Unconfirmed, the line is typed
//!   again ([`Output::Retype`], `^U` first), at most [`Budgets::boot_lines`]
//!   copies in all; the target is published only once the loop runs, so the
//!   loop's read of the tty before its first attach takes any extra copy
//!   away.
//! - A switch counts once the host says the loop came round to it
//!   ([`launch::retarget_landed`]), never on the exec merely coming back
//!   (B208).
//! - Only the person takes a pane from another device: a pick (a tap, a
//!   reply for the pane) and the take back attach with `--takeover`; a
//!   switch that follows the shared focus — the poll's report, the focused
//!   pane after a close or a restart, the loop booted again — attaches
//!   plainly, and herdr refuses that while another client holds the
//!   terminal. Following the focus used to take over every time: a second
//!   device lost its pane whenever the first followed the focus onto it.
//!   The refused follow is said as a takeover is ("This pane is open on
//!   another device", "Open here").
//! - One exec at a time on the retarget lane, and the one that runs points
//!   the loop at whatever is wanted when its turn comes; a step holds the
//!   lane at most [`Budgets::lane`]. A retarget that failed, came back
//!   unconfirmed or ran out its budget is forgotten, so the next report of
//!   the same focus asks again.
//! - After every landing a watch ([`launch::raw_attach_watch_command`])
//!   waits for the loop to stop showing that attach. The loop parks when its
//!   attach ends on its own; then nothing typed has a pane to reach, and it
//!   is dropped and said until the next landing. Who brings it back depends
//!   on why ([`launch::renderer_stop_kind`]): another device took the pane
//!   over → said, and taken back only by the person ([`Renderer::take_back`]
//!   or a pick of it) — a focus report never does, nor a reply; the pane
//!   closed or herdr stopped or restarted → the focused pane, by itself;
//!   anything else herdr refused, or the loop gone → said, on the person's
//!   tap.
//! - Typing during a switch is dropped and said: until the switch lands it
//!   would reach the pane being left.
//! - A delivery goes once the loop is on its pane: after the switch lands,
//!   or — for the pane already on screen — once a check
//!   ([`launch::raw_attach_check_command`]) has seen the loop still on it.
//!   One that can't be placed is answered not sent, with a notice, as soon as
//!   that is known.

use std::collections::{BTreeSet, VecDeque};

use crate::launch::{self, RendererStop, StopKind};

/// How often a terminal another device took is asked for plainly once the
/// focus has left it (B487), in ms.
const PROBE_EVERY: u64 = 10_000;

/// How long a focus change the person asked for is followed (B683), in ms.
const FOLLOW_ASKED_FOCUS: u64 = 10_000;

/// How long a loop that showed "open on another device" waits, once the
/// terminal it was refused is gone (a restart numbers terminals anew),
/// before following the focus again (B684), in ms: the device that had the
/// pane asks for it at once.
const RESTART_GRACE: u64 = 6_000;

/// One exec the renderer asked for ([`Output::Exec`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExecId(pub u64);

/// One delivery ([`Renderer::deliver`]), answered by [`Output::Delivered`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeliveryId(pub u64);

/// What the renderer asks of its host, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// Run `command` on a fresh exec channel of the sidecar and answer with
    /// [`Renderer::exec_finished`].
    Exec { id: ExecId, command: String },
    /// Bytes for the mosh channel, in order with everything sent before.
    Send(Vec<u8>),
    /// A delivery's answer: whether its bytes went out.
    Delivered { id: DeliveryId, sent: bool },
    /// Cover the pane being left: a switch is under way.
    Veil,
    /// Take the veil down: the new pane has painted, or no switch is coming
    /// to paint under it.
    Reveal,
    /// Something to tell the person.
    Notice(Notice),
    /// The notice the renderer put up is stale: take it down.
    ClearNotice,
    /// Read the control plane's tree now: a pane closed or herdr restarted,
    /// and the focused pane is what the renderer goes to next.
    RefreshTree,
    /// The loop's start line went unanswered (a login shell still starting
    /// swallowed it): type it again into the mosh channel — `^U` first, `\r`
    /// last — in order with what was sent before, and answer
    /// [`Renderer::retyped`] once mosh-server has acknowledged it: a copy
    /// that comes behind a loop that did start is then in the tty before
    /// the loop's first attach reads the tty away. Unanswered, the renderer
    /// goes on after [`Budgets::boot_ack`].
    Retype(Vec<u8>),
}

/// What the renderer says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Typing went nowhere: no pane is on screen to take it ("Not attached
    /// to the pane yet — input wasn't sent").
    NotOnPane,
    /// A delivery's pane couldn't be put on screen, so nothing was sent
    /// ("Couldn't switch to that pane — nothing was sent").
    CouldntSwitch,
    /// Another device took the pane on screen over, or holds the pane the
    /// focus went to ("This pane is open on another device"), with a take
    /// back ("Open here", [`Renderer::take_back`]). Stays until the next
    /// landing.
    OnAnotherDevice,
    /// herdr refused the attach with these words; retried on the person's
    /// word ("Retry", [`Renderer::take_back`]). Stays until the next landing.
    Refused(String),
    /// The loop itself is gone; booted again on the person's word ("Retry",
    /// [`Renderer::take_back`]). Stays until the next landing.
    LoopGone,
}

/// Why the loop shows no pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Down {
    /// Another device took this terminal's pane over.
    TakenOver(String),
    /// This terminal's pane closed, or herdr stopped or restarted.
    Gone(String),
    /// herdr refused this terminal's attach.
    Refused(String),
    /// The loop itself ended.
    LoopGone,
}

/// Every wait the renderer makes, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budgets {
    /// How long one exec on the retarget lane holds the ones behind it
    /// (`moshRetargetBudget`, 5 s).
    pub lane: u64,
    /// How long a delivery waits for the loop to come round to its pane
    /// (`herdrRetargetWait`, 5 s).
    pub delivery: u64,
    /// The pause before a watch whose exec failed is asked again.
    pub watch_retry: u64,
    /// How long the screen must stay quiet after the new pane's bytes
    /// before the veil lifts (`moshSwitchQuiet`, 150 ms).
    pub cover_quiet: u64,
    /// How long after the landing the veil stays up at least (400 ms):
    /// herdr repaints ~250–330 ms after a takeover's landing (measured on
    /// herdr 0.9.5), and the loop's own wipe of the pane left can arrive
    /// after the landing too.
    pub cover_repaint: u64,
    /// The veil's cap, for a pane that paints nothing (3 s).
    pub cover_cap: u64,
    /// How many copies of the loop's start line a generation gets, the
    /// first (the host's) included (4, as many as the mosh+tmux attach
    /// line).
    pub boot_lines: u32,
    /// How long a retyped start line's acknowledgement is waited for.
    pub boot_ack: u64,
}

impl Default for Budgets {
    fn default() -> Self {
        Budgets {
            lane: 5_000,
            delivery: 5_000,
            watch_retry: 2_000,
            cover_quiet: 150,
            cover_repaint: 400,
            cover_cap: 3_000,
            boot_lines: 4,
            boot_ack: 2_000,
        }
    }
}

/// A step on the retarget lane.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Ask whether the loop the start line was typed for runs.
    Boot,
    /// Point the loop at the target wanted when this step's turn comes.
    Apply,
    /// Ask whether the loop is still on this terminal (reads only).
    Check(String),
}

/// What an exec on the lane was for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Asked {
    Boot,
    Retarget(String),
    Check(String),
}

#[derive(Debug)]
struct Running {
    id: ExecId,
    asked: Asked,
    deadline: u64,
    /// The host holds it back ([`Renderer::exec_held`]): no budget runs.
    held: bool,
    /// A retarget's: the terminal on screen when it went out, back on
    /// screen if it is refused before the loop is touched.
    before: Option<String>,
}

/// What a delivery waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Wait {
    /// The switch to its terminal to land.
    Landing,
    /// The check of the pane on screen (this exec) to answer.
    Check(Option<ExecId>),
}

#[derive(Debug)]
struct Delivery {
    id: DeliveryId,
    terminal: String,
    bytes: Vec<u8>,
    deadline: u64,
    wait: Wait,
}

/// The watch on the attach of one landing.
#[derive(Debug)]
enum Watch {
    /// Its exec is out.
    Out { id: ExecId, epoch: u64 },
    /// Its exec failed; asked again at `at`.
    Retry { at: u64, epoch: u64 },
}

/// The veil over a switch. mosh ships screen diffs, not frames, so the
/// signal that the new pane is up is its repaint: the veil lifts once the
/// bytes stop. Only bytes from the landing on count: the first to come after
/// the kill used to be the loop's own (bash's "Killed: 9" ~10 ms on, its
/// clear) or the pane being left, and the veil lifted on the previous pane
/// (B222). Nor does the landing alone lift it: a takeover lands as soon as
/// its attach has a pid, and herdr repaints ~250 ms after the attach starts
/// — 150 ms of quiet counted from the landing lifted the veil before the
/// repaint, on the loop's blank screen (W10c, the Swift path's
/// `MoshSwitchCover` the same; measured on herdr 0.9.5 and mosh-server on
/// the loopback: the loop's wipe ~85 ms after the landing, herdr's repaint
/// ~325 ms after it). The veil stays up [`Budgets::cover_repaint`] past the
/// landing, and [`Budgets::cover_quiet`] past the last bytes since (a
/// follow's plain attach, watched past herdr's refusal, lands after its
/// repaint: the grace is all it waits).
#[derive(Debug, Clone, Copy)]
struct Cover {
    since: u64,
    landed: Option<u64>,
    /// The last bytes since the landing.
    last: Option<u64>,
}

impl Cover {
    fn lifts_at(&self, budgets: &Budgets) -> u64 {
        let cap = self.since + budgets.cover_cap;
        match self.landed {
            Some(landed) => {
                let quiet = self.last.map_or(0, |last| last + budgets.cover_quiet);
                cap.min((landed + budgets.cover_repaint).max(quiet))
            }
            None => cap,
        }
    }
}

/// The raw-attach renderer's state. See the module docs.
#[derive(Debug)]
pub struct Renderer {
    key: String,
    custom_path: Option<String>,
    budgets: Budgets,
    /// The terminal the loop was last asked to show: the idempotence guard
    /// for [`Renderer::show`] (every poll repeats the focus; only a change
    /// may bounce the attach).
    target: Option<String>,
    /// The person chose `target` (a pick, a reply, the take back): its
    /// attach takes the pane from whoever holds it. A follow's doesn't.
    take_over: bool,
    /// The terminal the person picked whose retarget couldn't run (B471):
    /// the next report of it is still their pick, `--takeover`, not a
    /// follow.
    lost_pick: Option<String>,
    /// Until when the next focus report is followed: a focus change the
    /// person asked for (B683).
    follow_until: Option<u64>,
    /// When the terminal another device took was last asked for plainly
    /// (B487).
    probed_at: Option<u64>,
    /// When the terminal another device held left the listing (B684).
    hold_gone_at: Option<u64>,
    /// The last listing that named any terminal: a restart numbers them all
    /// anew, a pane closing takes one (B684).
    last_listed: BTreeSet<String>,
    /// The terminal a plain ask got back from another device, and the
    /// focus that device sits on: that focus doesn't pull the loop after
    /// it (B487).
    regained: Option<(String, String)>,
    /// The terminal the last retarget pointed the loop at, by the host's
    /// word that the loop came round to it: what is on screen, as far as
    /// anything here knows. None before the first, and after one failed,
    /// came back without that word or ran out its budget.
    applied: Option<String>,
    /// Why the loop shows no pane, until the next landing.
    down: Option<Down>,
    /// Landings so far: a watch armed for an earlier one has nothing to say.
    epoch: u64,
    watch: Option<Watch>,
    /// The focused pane's terminal, by the last report.
    focused: Option<String>,
    /// The last terminal a switch found the server no longer lists: a
    /// report of it predates its closing.
    missing: Option<String>,
    /// The terminals the server lists, and whether it runs.
    listed: BTreeSet<String>,
    server_up: bool,
    lane: VecDeque<Step>,
    running: Option<Running>,
    /// Retargets the lane stopped waiting for, still out: one that lands
    /// late may have written over a newer switch.
    overdue: Vec<ExecId>,
    deliveries: Vec<Delivery>,
    cover: Option<Cover>,
    outputs: VecDeque<Output>,
    next_exec: u64,
    /// The loop of this key was seen running: its start line wasn't
    /// swallowed.
    booted: bool,
    /// Copies of the start line typed for this key, the host's first one
    /// included.
    boot_lines: u32,
    /// A retyped start line's acknowledgement, waited for until then.
    retype_wait: Option<u64>,
}

impl Renderer {
    /// A renderer for the loop booted under `renderer_key`
    /// ([`launch::mosh_renderer_key`]) with the herdr the loop runs.
    pub fn new(renderer_key: impl Into<String>, custom_path: Option<&str>) -> Self {
        Renderer::with_budgets(renderer_key, custom_path, Budgets::default())
    }

    pub fn with_budgets(
        renderer_key: impl Into<String>,
        custom_path: Option<&str>,
        budgets: Budgets,
    ) -> Self {
        Renderer {
            key: renderer_key.into(),
            custom_path: custom_path.map(str::to_owned),
            budgets,
            target: None,
            take_over: false,
            lost_pick: None,
            follow_until: None,
            probed_at: None,
            hold_gone_at: None,
            last_listed: BTreeSet::new(),
            regained: None,
            applied: None,
            down: None,
            epoch: 0,
            watch: None,
            focused: None,
            missing: None,
            listed: BTreeSet::new(),
            server_up: true,
            lane: VecDeque::new(),
            running: None,
            overdue: Vec::new(),
            deliveries: Vec::new(),
            cover: None,
            outputs: VecDeque::new(),
            next_exec: 0,
            booted: false,
            boot_lines: 1,
            retype_wait: None,
        }
    }

    /// Screen bytes arrived on mosh at `now` (the switch veil's only
    /// progress signal). Bytes from before the landing are not the new
    /// pane's, however late they are told.
    pub fn painted(&mut self, now: u64) {
        if let Some(cover) = &mut self.cover
            && cover.landed.is_some_and(|landed| now >= landed)
        {
            cover.last = Some(now);
        }
    }

    /// Whether the switch veil is up.
    pub fn veiled(&self) -> bool {
        self.cover.is_some()
    }

    fn reveal(&mut self) {
        if self.cover.take().is_some() {
            self.outputs.push_back(Output::Reveal);
        }
    }

    /// The terminal the loop was last asked to show.
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    /// The terminal on screen, by the host's word.
    pub fn on_screen(&self) -> Option<&str> {
        self.applied.as_deref()
    }

    /// Why the loop shows no pane, if it doesn't.
    pub fn down(&self) -> Option<&Down> {
        self.down.as_ref()
    }

    /// The renderer's own key: what the loop's files are named after.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// A switch has been asked for and hasn't landed: typing would still
    /// reach the pane being left.
    pub fn switching(&self) -> bool {
        matches!(&self.target, Some(target) if self.applied.as_ref() != Some(target))
    }

    /// The control plane's tree: the terminals the server lists, and
    /// whether it runs at all. A pane another device took over that has
    /// since closed is gone: the renderer goes to the focused pane.
    pub fn tree(&mut self, listed: impl IntoIterator<Item = String>, server_up: bool, now: u64) {
        self.listed = listed.into_iter().collect();
        self.server_up = server_up;
        // A restart numbers every terminal anew: none of the last listing's
        // is left. A pane closing leaves the others.
        let restarted = !self.listed.is_empty()
            && !self.last_listed.is_empty()
            && self.last_listed.is_disjoint(&self.listed);
        if !self.listed.is_empty() {
            self.last_listed = self.listed.clone();
        }
        if let Some(Down::TakenOver(terminal)) = &self.down
            && server_up
            && !self.listed.contains(terminal)
            && (restarted || self.hold_gone_at.is_some())
        {
            // Fix (B684): the terminal another device held is gone — the
            // server restarted (terminals numbered anew), or its pane
            // closed. Every device's attach ended with a restart; the one
            // that had the pane asks for it again at once, plainly. This
            // one only showed the notice: it waits a moment first, the
            // notice still up — following at once raced the device in use
            // and could keep its pane. Never a takeover.
            let terminal = terminal.clone();
            self.hold_gone(terminal, now);
            return;
        }
        if let Some(Down::TakenOver(terminal) | Down::Refused(terminal)) = &self.down
            && server_up
            && !self.listed.contains(terminal)
        {
            let terminal = terminal.clone();
            self.went(terminal, now);
        }
    }

    /// The terminal another device held left the listing at `hold_gone_at`
    /// (B684): followed past its moment.
    fn hold_gone(&mut self, terminal: String, now: u64) {
        let since = *self.hold_gone_at.get_or_insert(now);
        if now.saturating_sub(since) >= RESTART_GRACE {
            self.went(terminal, now);
        }
    }

    /// The focused pane's terminal, as the control plane reports it (every
    /// poll repeats it): the loop follows it, free once it is on screen —
    /// except while it is down for a reason only the person settles.
    pub fn show(&mut self, terminal: &str, now: u64) {
        self.focused = Some(terminal.to_owned());
        if self.missing.as_deref() == Some(terminal) {
            return;
        }
        match &self.down {
            Some(Down::TakenOver(held)) if self.hold_gone_at.is_some() => {
                // B684: its terminal is gone; the moment runs out here too.
                let held = held.clone();
                self.hold_gone(held, now);
                return;
            }
            Some(Down::TakenOver(held)) => {
                // B487: the focus has left the terminal another device took
                // — that device has likely moved on and let it go. Asked
                // for plainly, at most every ten seconds: refused (and said
                // again) while it is still theirs, ours again once it isn't.
                // Never a takeover.
                let held = held.clone();
                let due = self
                    .probed_at
                    .is_none_or(|at| now.saturating_sub(at) >= PROBE_EVERY);
                if terminal != held && due {
                    self.probed_at = Some(now);
                    self.regained = Some((held.clone(), terminal.to_owned()));
                    self.target = None;
                    self.applied = None;
                    self.steer(&held, false, now);
                }
                return;
            }
            Some(Down::Refused(_) | Down::LoopGone) => return,
            // A report of the pane that just closed predates its closing.
            Some(Down::Gone(closed)) if closed == terminal => return,
            _ => {}
        }
        if let Some((regained, theirs)) = &self.regained {
            // Back on the terminal the plain ask got back: the other
            // device's focus doesn't pull the loop after it; a new move of
            // the focus does.
            if terminal == theirs && self.target.as_deref() == Some(regained.as_str()) {
                return;
            }
            if terminal != regained {
                self.regained = None;
            }
        }
        if let Some(picked) = self.lost_pick.take()
            && picked == terminal
        {
            // The person's pick whose retarget never ran, reported again:
            // still theirs (B471).
            self.steer(terminal, true, now);
            return;
        }
        // Fix (B683): once the loop is on a pane, the server's focus moving
        // elsewhere (the desktop) no longer moves it — the phone switched
        // under the person and dropped what they typed. Followed still: the
        // first landing, a pane gone (by `went`), a focus change the person
        // asked for, B487's ask above.
        if let Some(ours) = self.target.clone()
            && ours != terminal
            && (self.listed.is_empty() || self.listed.contains(&ours))
            && !self.follow_until.take().is_some_and(|until| now < until)
        {
            return;
        }
        // Following the focus never takes a pane from another device.
        self.steer(terminal, false, now);
    }

    /// The person asked for a focus change the server makes (a workspace or
    /// tab picked, a pane, tab or workspace made): the next focus report is
    /// followed (B683).
    pub fn follow_focus(&mut self, now: u64) {
        self.follow_until = Some(now + FOLLOW_ASKED_FOCUS);
    }

    /// The person picked `terminal`'s pane (a tap, a reply for it): shown,
    /// whatever stopped the loop, and taken from whichever client holds it
    /// (`--takeover`) — on the very pane another device has, that is the
    /// person taking it back.
    pub fn pick(&mut self, terminal: &str, now: u64) {
        self.lost_pick = None;
        self.regained = None;
        match &self.down {
            Some(Down::TakenOver(stopped) | Down::Refused(stopped)) if stopped == terminal => {
                // The same pane, wanted again: not "already showing it".
                self.target = None;
                self.applied = None;
            }
            Some(Down::TakenOver(_) | Down::Refused(_)) => {
                // Fix (B685): another pane picked while the notice of one
                // another device holds (or herdr refused) was up: the person
                // moved on from it. Its hold went on: B487's ask for it,
                // due at the next focus report (the pick's own), pulled the
                // loop back off the pane just picked — refused, parked,
                // nothing on screen (live: two devices on one herdr, the
                // iPad never showed the pane it picked). The frame
                // channel's pick drops the hold the same way.
                self.down = None;
                self.probed_at = None;
                self.hold_gone_at = None;
                self.outputs.push_back(Output::ClearNotice);
            }
            Some(Down::LoopGone) => return,
            _ => {}
        }
        self.steer(terminal, true, now);
    }

    /// The notice's button: the pane the loop stopped on, taken over from
    /// whoever has it (`--takeover`) — or the loop booted again, when it is
    /// gone: the line to type into mosh comes back for the host to send.
    pub fn take_back(&mut self, now: u64) -> Option<String> {
        match self.down.clone() {
            Some(Down::TakenOver(terminal) | Down::Refused(terminal)) => {
                if self.server_up && !self.listed.is_empty() && !self.listed.contains(&terminal) {
                    // Its pane is gone meanwhile: the focused one.
                    self.went(terminal, now);
                } else {
                    self.pick(&terminal, now);
                }
                None
            }
            Some(Down::LoopGone) => Some(self.reboot(now)),
            Some(Down::Gone(_)) | None => None,
        }
    }

    /// The loop is gone: a new generation's line (as at connect, `^U`
    /// first to clear whatever half a line the shell holds), and the
    /// focused pane next. A loop that never started keeps its key: a copy
    /// of its line still waiting in the shell runs the same generation.
    fn reboot(&mut self, now: u64) -> String {
        if self.booted {
            self.key = format!("{}-r{}", self.key, self.epoch);
        }
        self.booted = false;
        self.boot_lines = 1;
        self.retype_wait = None;
        self.down = None;
        self.target = None;
        self.applied = None;
        self.lane.clear();
        self.outputs.push_back(Output::ClearNotice);
        let line = format!(
            "\u{15}{}\r",
            launch::raw_attach_loop_command(&self.key, self.custom_path.as_deref())
        );
        // The focused pane: a follow, not the person's pick.
        if let Some(focused) = self.focused.clone() {
            self.steer(&focused, false, now);
        }
        line
    }

    /// Typing: down the mosh channel when the pane on screen is the one
    /// asked for (true); dropped and said while a switch is under way, the
    /// loop shows no pane, or none is on screen yet by the host's word —
    /// before the first switch landed, typing reached whichever pane the
    /// loop attached first.
    pub fn typed(&mut self, bytes: &[u8], _now: u64) -> bool {
        if bytes.is_empty() {
            return false;
        }
        if self.switching() || self.down.is_some() || self.applied.is_none() {
            // A notice that says more stays up.
            if !matches!(
                self.down,
                Some(Down::TakenOver(_) | Down::Refused(_) | Down::LoopGone)
            ) {
                self.outputs.push_back(Output::Notice(Notice::NotOnPane));
            }
            return false;
        }
        self.outputs.push_back(Output::Send(bytes.to_vec()));
        true
    }

    /// `bytes` for `terminal`'s pane on purpose — a notification's reply, a
    /// share — once the server has the pane focused (the caller's part).
    /// Answered by [`Output::Delivered`].
    pub fn deliver(&mut self, id: DeliveryId, terminal: &str, bytes: Vec<u8>, now: u64) {
        // The loop stopped on this pane, or is gone: nothing reaches it now,
        // and a reply is no reason to take a pane back from another device.
        let stopped_here = match &self.down {
            Some(Down::TakenOver(t) | Down::Refused(t) | Down::Gone(t)) => t == terminal,
            Some(Down::LoopGone) => true,
            None => false,
        };
        if stopped_here {
            self.outputs
                .push_back(Output::Notice(Notice::CouldntSwitch));
            self.outputs
                .push_back(Output::Delivered { id, sent: false });
            return;
        }
        let on_screen =
            self.applied.as_deref() == Some(terminal) && self.target.as_deref() == Some(terminal);
        // The person's reply, for this pane: a pick, as a tap would be.
        self.pick(terminal, now);
        let wait = if on_screen {
            // No retarget vouched for it just now, and the loop may have
            // stopped under it since: asked, not re-attached.
            self.lane.push_back(Step::Check(terminal.to_owned()));
            Wait::Check(None)
        } else {
            Wait::Landing
        };
        self.deliveries.push(Delivery {
            id,
            terminal: terminal.to_owned(),
            bytes,
            deadline: now + self.budgets.delivery,
            wait,
        });
        self.pump(now);
        self.settle_deliveries(now);
    }

    /// The answer to an exec: its output, or None when it failed.
    pub fn exec_finished(&mut self, id: ExecId, output: Option<&[u8]>, now: u64) {
        if let Some(Watch::Out {
            id: watching,
            epoch,
        }) = self.watch
            && watching == id
        {
            self.watch = None;
            if epoch != self.epoch {
                return;
            }
            match output {
                None => {
                    self.watch = Some(Watch::Retry {
                        at: now + self.budgets.watch_retry,
                        epoch,
                    });
                }
                Some(output) => match launch::renderer_stop(output) {
                    Some(stop) => self.stopped(stop, now),
                    // A minute with nothing to say, or a switch moved the
                    // loop on (its landing arms the next watch).
                    None => self.arm_watch(now),
                },
            }
            self.settle_deliveries(now);
            return;
        }
        if let Some(index) = self.overdue.iter().position(|&overdue| overdue == id) {
            self.overdue.remove(index);
            // Landed after the lane gave up on it: a newer switch may have
            // run in the meantime, and this one just wrote over it. Point
            // the loop at whatever is wanted now, once more.
            if output.is_some_and(launch::retarget_landed) {
                self.applied = None;
                self.lane.push_back(Step::Apply);
                self.pump(now);
            }
            self.settle_deliveries(now);
            return;
        }
        let Some(running) = self.running.take_if(|running| running.id == id) else {
            return;
        };
        match running.asked {
            Asked::Boot => self.boot_answered(output.is_some_and(launch::loop_up), now),
            Asked::Retarget(terminal) => {
                // Landed on the host's word that the loop came round to the
                // target, never on the exec merely coming back (B208).
                if output.is_some_and(launch::retarget_landed) {
                    // The veil's switch landed — unless a newer one is
                    // queued, whose landing is the one that counts.
                    if self.target.as_ref() == Some(&terminal)
                        && let Some(cover) = &mut self.cover
                    {
                        cover.landed = Some(now);
                    }
                    self.applied = Some(terminal);
                    self.landed(now);
                } else if output.is_some_and(launch::retarget_missing) {
                    // The server no longer lists it, and the loop wasn't
                    // touched: what was on screen still is. Its stale
                    // reports aren't followed; the tree is read again.
                    if self.applied.is_none() {
                        self.applied = running.before;
                    }
                    if self.target.as_ref() == Some(&terminal) {
                        self.target = self.applied.clone();
                    }
                    self.missing = Some(terminal);
                    self.outputs.push_back(Output::RefreshTree);
                    if !self.switching() {
                        self.reveal();
                    }
                } else if let Some(stop @ RendererStop::Parked { .. }) =
                    output.and_then(launch::renderer_stop)
                {
                    // A follow's plain attach that herdr refused — another
                    // client holds the pane — and the loop parked on it:
                    // said as the watch says a takeover, taken by nobody
                    // but the person.
                    self.stopped(stop, now);
                } else {
                    if output.is_none() && self.take_over && self.target.as_ref() == Some(&terminal)
                    {
                        // It never ran (the control connection dropped
                        // under it): the person's pick stands (B471).
                        self.lost_pick = Some(terminal.clone());
                    }
                    self.retarget_failed(&terminal);
                    if output.and_then(launch::renderer_stop) == Some(RendererStop::LoopGone) {
                        self.stopped(RendererStop::LoopGone, now);
                    }
                }
            }
            Asked::Check(terminal) => {
                let up = output.is_some_and(launch::raw_attach_up);
                let stop = if up {
                    None
                } else {
                    output.and_then(launch::renderer_stop)
                };
                self.checked(id, &terminal, up, stop, now);
            }
        }
        self.pump(now);
        self.settle_deliveries(now);
    }

    /// The retyped start line ([`Output::Retype`]) was acknowledged: the
    /// loop is asked about again.
    pub fn retyped(&mut self, now: u64) {
        if self.retype_wait.take().is_some() {
            self.pump(now);
            self.settle_deliveries(now);
        }
    }

    /// The host holds exec `id` back — a retarget behind typing still on
    /// its way through mosh: the switch waits for it, however long (B686).
    /// Its budget doesn't run until [`Renderer::exec_sent`]: a roam longer
    /// than the budget had the switch given up while it still waited.
    pub fn exec_held(&mut self, id: ExecId) {
        if let Some(running) = self.running.as_mut().filter(|running| running.id == id) {
            running.held = true;
        }
    }

    /// Exec `id`, held back, went out now: its budget counts from here.
    pub fn exec_sent(&mut self, id: ExecId, now: u64) {
        if let Some(running) = self.running.as_mut().filter(|running| running.id == id) {
            running.held = false;
            running.deadline = now + self.budgets.lane;
        }
    }

    /// Timers: the lane's budget, the deliveries' waits, a watch to ask
    /// again.
    pub fn tick(&mut self, now: u64) {
        if let Some(running) = self
            .running
            .take_if(|running| !running.held && now >= running.deadline)
        {
            match running.asked {
                Asked::Boot => self.boot_answered(false, now),
                Asked::Retarget(terminal) => {
                    self.overdue.push(running.id);
                    self.retarget_failed(&terminal);
                }
                Asked::Check(terminal) => self.checked(running.id, &terminal, false, None, now),
            }
            self.pump(now);
        }
        if self.retype_wait.is_some_and(|at| now >= at) {
            self.retype_wait = None;
            self.pump(now);
        }
        if let Some(cover) = self.cover
            && now >= cover.lifts_at(&self.budgets)
        {
            self.reveal();
        }
        if let Some(Watch::Retry { at, epoch }) = self.watch
            && now >= at
        {
            self.watch = None;
            if epoch == self.epoch {
                self.arm_watch(now);
            }
        }
        self.settle_deliveries(now);
    }

    /// When [`Renderer::tick`] is next due.
    pub fn next_deadline(&self) -> Option<u64> {
        let lane = self
            .running
            .as_ref()
            .filter(|running| !running.held)
            .map(|running| running.deadline);
        let deliveries = self
            .deliveries
            .iter()
            .map(|delivery| delivery.deadline)
            .min();
        let watch = match self.watch {
            Some(Watch::Retry { at, .. }) => Some(at),
            _ => None,
        };
        let cover = self.cover.map(|cover| cover.lifts_at(&self.budgets));
        [lane, deliveries, watch, cover, self.retype_wait]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn poll_output(&mut self) -> Option<Output> {
        self.outputs.pop_front()
    }

    // MARK: steering

    /// Point the loop at `terminal`; `takeover`: the person chose it.
    fn steer(&mut self, terminal: &str, takeover: bool, now: u64) {
        if self.target.as_deref() == Some(terminal) {
            // The person chose the pane a follow is on its way to: theirs
            // now, asked again with `--takeover` unless the follow has it
            // on screen already (its turn on the lane skips it then).
            if takeover && !self.take_over {
                self.take_over = true;
                if self.applied.as_deref() != Some(terminal) {
                    self.lane.push_back(Step::Apply);
                    self.pump(now);
                }
            }
            return;
        }
        let had_target = self.target.is_some();
        self.target = Some(terminal.to_owned());
        self.take_over = takeover;
        // The exec, the kill, the re-attach and the repaint are all round
        // trips; until the last one the screen is the pane being left.
        if had_target {
            self.outputs.push_back(Output::Veil);
            self.cover = Some(Cover {
                since: now,
                landed: None,
                last: None,
            });
        }
        self.lane.push_back(Step::Apply);
        self.pump(now);
        self.settle_deliveries(now);
    }

    /// The loop came round to the pane asked for: whatever stopped it is
    /// over, and the watch goes on the attach it is on now.
    fn landed(&mut self, now: u64) {
        self.epoch += 1;
        if let Some(down) = self.down.take()
            && !matches!(down, Down::Gone(_))
        {
            self.outputs.push_back(Output::ClearNotice);
        }
        self.arm_watch(now);
    }

    fn arm_watch(&mut self, _now: u64) {
        let id = self.exec(launch::raw_attach_watch_command(&self.key));
        self.watch = Some(Watch::Out {
            id,
            epoch: self.epoch,
        });
    }

    /// The loop stopped showing its pane: why decides who brings it back.
    fn stopped(&mut self, stop: RendererStop, now: u64) {
        // Nothing is coming to paint under a veil that is up.
        self.reveal();
        match stop {
            RendererStop::LoopGone => {
                self.applied = None;
                self.down = Some(Down::LoopGone);
                self.outputs.push_back(Output::Notice(Notice::LoopGone));
            }
            RendererStop::Parked {
                status,
                terminal,
                said,
            } => {
                if self.applied.as_ref() == Some(&terminal) {
                    self.applied = None;
                }
                let listed = self.listed.contains(&terminal);
                match launch::renderer_stop_kind(status, &said, listed, self.server_up) {
                    StopKind::TakenOver => {
                        // A plain ask refused: still theirs.
                        self.regained = None;
                        self.hold_gone_at = None;
                        self.down = Some(Down::TakenOver(terminal));
                        self.outputs
                            .push_back(Output::Notice(Notice::OnAnotherDevice));
                    }
                    StopKind::Refused => {
                        self.down = Some(Down::Refused(terminal));
                        self.outputs
                            .push_back(Output::Notice(Notice::Refused(said)));
                    }
                    StopKind::Gone => self.went(terminal, now),
                }
            }
        }
        self.settle_deliveries(now);
    }

    /// `terminal`'s pane closed, or herdr stopped or restarted: the focused
    /// pane next, by itself — not this one again.
    fn went(&mut self, terminal: String, now: u64) {
        self.hold_gone_at = None;
        if matches!(
            self.down,
            Some(Down::TakenOver(_) | Down::Refused(_) | Down::LoopGone)
        ) {
            self.outputs.push_back(Output::ClearNotice);
        }
        if self.target.as_ref() == Some(&terminal) {
            self.target = None;
        }
        if self.applied.as_ref() == Some(&terminal) {
            self.applied = None;
        }
        self.down = Some(Down::Gone(terminal.clone()));
        self.outputs.push_back(Output::RefreshTree);
        if let Some(focused) = self.focused.clone()
            && focused != terminal
        {
            self.steer(&focused, false, now);
        }
    }

    // MARK: the lane

    fn exec(&mut self, command: String) -> ExecId {
        let id = ExecId(self.next_exec);
        self.next_exec += 1;
        self.outputs.push_back(Output::Exec { id, command });
        id
    }

    fn pump(&mut self, now: u64) {
        while self.running.is_none() && self.retype_wait.is_none() {
            // The loop is confirmed before anything is asked of it: a
            // target published for a loop that hasn't started would be
            // taken up the moment it does, before a copy of its line typed
            // since has reached the tty for its first read to take away.
            if !self.booted && !self.lane.is_empty() && self.lane.front() != Some(&Step::Boot) {
                self.lane.push_front(Step::Boot);
            }
            let Some(step) = self.lane.pop_front() else {
                return;
            };
            let asked = match step {
                Step::Boot if self.booted => continue,
                Step::Boot => Asked::Boot,
                Step::Apply => match &self.target {
                    Some(target) if self.applied.as_ref() != Some(target) => {
                        Asked::Retarget(target.clone())
                    }
                    _ => continue,
                },
                Step::Check(terminal) => Asked::Check(terminal),
            };
            let before = self.applied.clone();
            if let Asked::Retarget(terminal) = &asked
                && self.applied.as_ref() != Some(terminal)
            {
                // The loop is on its way elsewhere: nothing is on screen by
                // the host's word until it says where it came round to. A
                // switch back to the pane landed last, made while this one
                // was out, used to find that pane "on screen" and let typing
                // through — and the retarget in flight moved the loop under
                // it, into the pane switched away to (W10c, live: a switch
                // storm's keystrokes in the other pane).
                self.applied = None;
            }
            let command = match &asked {
                Asked::Boot => launch::raw_attach_loop_up_command(&self.key),
                Asked::Retarget(terminal) => launch::retarget_command(
                    terminal,
                    &self.key,
                    self.custom_path.as_deref(),
                    self.take_over,
                ),
                Asked::Check(terminal) => launch::raw_attach_check_command(terminal, &self.key),
            };
            let id = self.exec(command);
            if let Asked::Check(terminal) = &asked {
                for delivery in &mut self.deliveries {
                    if delivery.terminal == *terminal && delivery.wait == Wait::Check(None) {
                        delivery.wait = Wait::Check(Some(id));
                    }
                }
            }
            self.running = Some(Running {
                id,
                asked,
                deadline: now + self.budgets.lane,
                held: false,
                before,
            });
        }
    }

    /// The loop-up check came back: `up`, or not (in time). Not up, the
    /// start line is typed again while copies are left; the last one
    /// unanswered, the loop is gone — said, and booted again on the
    /// person's word.
    fn boot_answered(&mut self, up: bool, now: u64) {
        if up {
            self.booted = true;
            return;
        }
        if self.boot_lines < self.budgets.boot_lines {
            self.boot_lines += 1;
            self.lane.push_front(Step::Boot);
            self.retype_wait = Some(now + self.budgets.boot_ack);
            let line = format!(
                "\u{15}{}\r",
                launch::raw_attach_loop_command(&self.key, self.custom_path.as_deref())
            );
            self.outputs.push_back(Output::Retype(line.into_bytes()));
            return;
        }
        // Never started: the switch waiting on it is over.
        self.lane
            .retain(|step| !matches!(step, Step::Boot | Step::Apply));
        self.target = None;
        self.stopped(RendererStop::LoopGone, now);
    }

    /// A retarget failed, came back unconfirmed or ran out its budget: what
    /// the loop shows is unknown. Unless a newer target is already queued,
    /// this one is forgotten too, so the next report of the same focus asks
    /// again rather than being taken for the screen that is up.
    fn retarget_failed(&mut self, terminal: &str) {
        self.applied = None;
        if self.target.as_deref() == Some(terminal) {
            self.target = None;
            // The switch is over without a pane to show.
            self.reveal();
        }
    }

    /// A check came back: `up` when the loop is still on `terminal`; when it
    /// isn't, the park (or the loop's end) it found.
    fn checked(
        &mut self,
        id: ExecId,
        terminal: &str,
        up: bool,
        stop: Option<RendererStop>,
        now: u64,
    ) {
        if !up {
            match stop {
                // Parked under it before the watch said so: taken for what
                // it is — never by forgetting the target, which had the next
                // report take the pane back from the device that has it.
                Some(stop) => self.stopped(stop, now),
                None => {
                    // As a retarget that never landed: the next report asks
                    // again.
                    if self.applied.as_deref() == Some(terminal) {
                        self.applied = None;
                    }
                    if self.target.as_deref() == Some(terminal) {
                        self.target = None;
                    }
                }
            }
        }
        let blocked = self.switching() || self.down.is_some();
        let mut index = 0;
        while index < self.deliveries.len() {
            if self.deliveries[index].wait != Wait::Check(Some(id)) {
                index += 1;
                continue;
            }
            let delivery = self.deliveries.remove(index);
            // A switch started meanwhile: the bytes would reach the pane
            // being left.
            let sent = up && !blocked;
            self.answer(delivery, sent);
        }
    }

    // MARK: deliveries

    fn settle_deliveries(&mut self, now: u64) {
        let mut index = 0;
        while index < self.deliveries.len() {
            let delivery = &self.deliveries[index];
            let verdict = match delivery.wait {
                Wait::Landing => {
                    let terminal = delivery.terminal.as_str();
                    let wanted = self.target.as_deref() == Some(terminal);
                    // The loop went down on this very pane, or altogether.
                    let down_here = match &self.down {
                        Some(Down::TakenOver(t) | Down::Refused(t) | Down::Gone(t)) => {
                            t == terminal
                        }
                        Some(Down::LoopGone) => true,
                        None => false,
                    };
                    if wanted && self.down.is_none() && self.applied.as_deref() == Some(terminal) {
                        Some(true)
                    } else if !wanted || down_here || now >= delivery.deadline {
                        // The switch ended without landing (failed, came
                        // back unconfirmed, a newer pane was picked), the
                        // loop went down on it, or the wait ran out:
                        // nothing reaches this pane now.
                        Some(false)
                    } else {
                        None
                    }
                }
                // The check's own lane budget bounds it; the delivery's wait
                // covers the time spent queued behind the lane.
                Wait::Check(_) => (now >= delivery.deadline).then_some(false),
            };
            match verdict {
                Some(sent) => {
                    let delivery = self.deliveries.remove(index);
                    self.answer(delivery, sent);
                }
                None => index += 1,
            }
        }
    }

    fn answer(&mut self, delivery: Delivery, sent: bool) {
        if sent {
            self.outputs.push_back(Output::Send(delivery.bytes));
        } else {
            self.outputs
                .push_back(Output::Notice(Notice::CouldntSwitch));
        }
        self.outputs.push_back(Output::Delivered {
            id: delivery.id,
            sent,
        });
    }
}
