//! The frame channel (`herdr terminal session control` on the session's
//! terminal channel): which pane it is attached to, how it is pointed at
//! another, and the one ordered path every write into it takes — the herdr
//! half of `SessionHub.swift`, per pane.
//!
//! The channel renders one pane at a time. Pointing it elsewhere is: release
//! the attach that is reading (`terminal.release`), wait for it to let go of
//! the channel (its `terminal.closed`), then name the next pane — one line to
//! the reader loop the boot line started (`<pane> <cols> <rows>`), or the
//! one-shot start line where the loop never announced itself.
//!
//! Every pane has its own terminal (stream). Frames go to the pane whose
//! attach printed them: the pane being released keeps its last frames until
//! its close comes back, and nothing after that is its.
//!
//! Input is addressed to a pane, and goes out only once that pane's attach
//! has painted: never into the pane being left, never into the loop's
//! `read` (B5, B25), and never into an attach that isn't up yet — herdr
//! 0.9.5 drops input that reaches an attach before it is (probed on a PTY:
//! 9 of 10 lines written right behind the attach line were lost, 0 of 10
//! written after its first frame). Input typed at the pane a person just
//! picked waits for that first frame and follows it; input for any other
//! pane is dropped and said ([`super::Event::InputDropped`]). Semantic keys (arrows) go out as `pane
//! send-keys` execs on the same write lane, so typing and keys land in the
//! order they were pressed (B2).
//!
//! Only the person takes a pane from another device: the attach line asks
//! for `--takeover` for a pick (a tap, a reply for the pane) and "Open
//! here", never for a switch that follows the shared focus — herdr refuses
//! that one while another client holds the terminal. Following the focus
//! used to take over every time, and an attach another device's takeover
//! ended was asked for again on the next read, three times in 30 s before a
//! backoff: two devices fought over the pane. Now a pane another device has
//! (taken from us, or held when the focus went there) is said ("This pane
//! is open on another device", "Open here") and left alone until the
//! person's word, or until it closes.

use std::collections::{BTreeMap, VecDeque};

use super::timers::Slot;
use super::{Client, Event, Notice, Purpose, ViewAction};
use crate::control;
use crate::frame::{self, Frame, FrameParser, MouseAction, MouseButton, ScrollSource};
use crate::snapshot;

/// Home, erase display, erase scrollback: what is left on a pane's terminal
/// while its switch is veiled — an empty screen is honest, a stale one is a
/// lie the person acts on.
const CLEAR_SCREEN: &[u8] = b"\x1b[H\x1b[2J\x1b[3J";

/// How long typing waits for the attach of a pane a follow is on its way
/// to (B493), in ms.
const FOLLOW_HOLD: u64 = 2_000;

/// How often a pane another device held is asked for plainly once the
/// focus has left it (B487), in ms.
const PROBE_EVERY: u64 = 10_000;

/// How long a device that showed "open on another device" waits, after the
/// server restarted under that hold, before asking for a pane again (B681,
/// reworked), in ms: the device that had the pane asks at once.
const RESTART_GRACE: u64 = 6_000;

/// The frame channel's state.
#[derive(Debug, Default)]
pub(super) struct Attach {
    /// The channel is up (SSH; never over mosh).
    pub open: bool,
    /// The pane the channel is pointed at (`herdrFrameTarget`).
    pub target: Option<String>,
    /// The pane whose attach line has been written: its attach reads what
    /// is written after it.
    pub attached: Option<String>,
    /// The pane whose release is out: frames until its close are its own.
    pub releasing: Option<String>,
    pub booted: bool,
    pub loop_ready: bool,
    /// Attach lines for the target that went unpainted past their timeout.
    pub unpainted: u32,
    /// The channel is stuck: its loop never said it reads, or its attaches
    /// never paint. Said ("Not attached…", "Retry"); the retry redials.
    pub stuck: bool,
    /// The target's attach has painted.
    pub painted: bool,
    /// The server went away while another device held our pane (B467):
    /// the hold outlived its server.
    pub hold_server_gone: bool,
    /// Until when this device, which showed "open on another device" when
    /// the server restarted, waits before asking for a pane (B681).
    pub restart_grace_until: Option<u64>,
    pub asked_at: Option<u64>,
    /// Releases whose `terminal.closed` hasn't come back, oldest first.
    pub expected: VecDeque<u64>,
    pub next_expected: u64,
    /// A switch waiting for the channel to be free.
    pub switch: Option<Switch>,
    pub release_sent_at: Option<u64>,
    /// Backing off after herdr refused an attach.
    pub retry_after: Option<u64>,
    /// How long that backoff is, for the banner's countdown (B490).
    pub retry_wait: Option<u64>,
    /// The person chose `target` (a pick, a reply, "Open here"): its attach
    /// line asks for `--takeover`. A follow's doesn't.
    pub take_over: bool,
    /// What the attach line written last asked for.
    pub line_took_over: bool,
    /// The pane another device has — it took ours over, or held the one
    /// the focus went to — until the person's word or its closing.
    pub held_elsewhere: Option<String>,
    /// The held pane's terminal when the hold began (B467): a new one is a
    /// server restarted since, which ended every attach.
    pub held_terminal: Option<String>,
    /// When the pane another device held was last asked for plainly
    /// (B487).
    pub probed_at: Option<u64>,
    /// The pane a plain ask got back from another device, and the focus
    /// that device sits on: that focus doesn't pull us after it (B487).
    pub regained: Option<(String, String)>,
    /// The pane whose veil lifts on its first full frame.
    pub awaiting_full: Option<String>,
    /// The last switch's cost, ask to full frame (`herdrLastSwitch`).
    pub last_switch_ms: u64,
    /// Each pane's last grid; the last one reported by any.
    pub grids: BTreeMap<String, (i64, i64)>,
    pub last_grid: Option<(i64, i64)>,
    /// The grid the current attach line carried.
    pub line_grid: Option<(i64, i64)>,
    /// Input for `attached` goes out: its attach has painted.
    pub input_open: bool,
    pub parser: FrameParser,
    pub lane: WriteLane,
    pub held: Held,
    /// What came for a pane right behind a paste to it, until the paste
    /// has had its time (B485).
    pub settling: Option<Settling>,
}

impl Attach {
    /// The pane input goes to now.
    pub fn input_pane(&self) -> Option<&str> {
        self.attached.as_deref().filter(|_| self.input_open)
    }
}

/// A switch waiting for the channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Switch {
    pub pane: String,
    pub phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    /// The release is out (or queued behind a key): wait for its close.
    Ack,
    /// The login shell's prompt, before a one-shot line.
    Settle,
    /// The loop the channel runs ([`super::Config::frame_loop`]) to say it
    /// reads.
    Loop,
}

/// Every write into the frame channel, and the semantic keys between them
/// (`herdrWriteLane`, `WriteLane`).
#[derive(Debug, Default)]
pub(super) struct WriteLane {
    queue: VecDeque<WriteStep>,
    /// The key run in flight (its exec).
    key: Option<u64>,
}

#[derive(Debug)]
enum WriteStep {
    /// A line; `release`: the switch's ack window opens once it is out.
    Line { line: String, release: bool },
    /// Keys for a pane, sent together when their turn comes. A key that
    /// arrives while the run at the tail hasn't started joins it: a held
    /// arrow repeats ~14 times a second, and each send is a round trip.
    Keys {
        pane: String,
        keys: Vec<&'static str>,
    },
}

/// Input held for the pane being switched to.
#[derive(Debug, Default)]
pub(super) struct Held {
    pub pane: Option<String>,
    steps: Vec<HeldStep>,
}

/// Input for `pane` behind the paste just sent to it (B485).
#[derive(Debug)]
pub(super) struct Settling {
    pane: String,
    steps: Vec<HeldStep>,
}

/// How long what follows a paste waits behind it (B485): a program taking
/// a paste in may drop a key that comes with it (Claude Code, making an
/// image's path an attachment, drops the Return behind it: the image
/// panel's Send only pasted). The session waits as long before it sends;
/// this keeps the two apart where the client held them together.
const PASTE_SETTLE_MS: u64 = 150;

#[derive(Debug)]
enum HeldStep {
    Input(Vec<u8>),
    /// A bracketed paste: what follows it waits behind it (B485).
    Paste(Vec<u8>),
    /// Bytes a request is waiting on ([`super::Action::DeliverInput`]).
    Delivery {
        request: u64,
        bytes: Vec<u8>,
    },
}

impl Client {
    // MARK: - The write lane

    /// Write a line, behind whatever the lane holds.
    fn write_line(&mut self, line: String, release: bool) {
        self.attach
            .lane
            .queue
            .push_back(WriteStep::Line { line, release });
        self.pump_writes();
    }

    /// A semantic key for `pane`, behind whatever the lane holds.
    fn write_key(&mut self, pane: &str, key: &'static str) {
        if let Some(WriteStep::Keys { pane: tail, keys }) = self.attach.lane.queue.back_mut()
            && tail == pane
        {
            keys.push(key);
            return;
        }
        self.attach.lane.queue.push_back(WriteStep::Keys {
            pane: pane.to_owned(),
            keys: vec![key],
        });
        self.pump_writes();
    }

    /// Carry the lane out until it empties or a key is in flight.
    fn pump_writes(&mut self) {
        while self.attach.lane.key.is_none() {
            let Some(step) = self.attach.lane.queue.pop_front() else {
                return;
            };
            match step {
                WriteStep::Line { line, release } => {
                    if !self.attach.open {
                        continue;
                    }
                    self.outputs.push_back(super::Output::FrameWrite(line));
                    if release {
                        self.release_written();
                    }
                }
                WriteStep::Keys { pane, keys } => {
                    let Some(subcommand) = control::send_keys(&keys, &pane) else {
                        continue;
                    };
                    let command = self.herdr(&subcommand);
                    let exec = self.exec(command, Purpose::Key, None);
                    self.attach.lane.key = Some(exec);
                    self.arm(Slot::KeyHold, self.config.budgets.key_hold);
                }
            }
        }
    }

    /// The key run answered: herdr wrote the keys into the pane, so whatever
    /// is written next lands after them.
    pub(super) fn key_answered(&mut self) {
        self.attach.lane.key = None;
        self.timers.cancel(&Slot::KeyHold);
        self.pump_writes();
    }

    /// A key that can't be delivered must not freeze the typing behind it:
    /// past its hold the lane moves on, and the key lands whenever it does.
    pub(super) fn key_hold_expired(&mut self) {
        if let Some(exec) = self.attach.lane.key.take() {
            self.abandon(exec);
        }
        self.pump_writes();
    }

    // MARK: - Pointing the channel

    /// Point the frame channel at `pane` (`retargetHerdrFrames`). Called on
    /// every read that names a focused pane, where a target already being
    /// rendered costs nothing, and on the person's pick (`user`), whose
    /// typing at the new pane waits for it and whose attach takes the pane
    /// from whichever client holds it (`--takeover`).
    pub(super) fn retarget(&mut self, pane: &str, user: bool) {
        if let Some(at) = self.attach.retry_after {
            if self.now < at {
                return;
            }
            self.attach.retry_after = None;
            self.attach.retry_wait = None;
        }
        if !self.attach.open {
            return;
        }
        if let Some(held) = self.attach.held_elsewhere.clone() {
            // Another device has the pane: a read never takes it back, nor
            // moves on while the notice says so — the person settles it (a
            // pick, "Open here"). Unless its pane is gone: then the focused
            // one, by itself.
            if !user && !self.pane_gone(&held) && !self.hold_outlived(&held) {
                // B487: the focus has moved away from it — the other device
                // has likely moved on, and let the pane go. Asked for
                // plainly, at most every ten seconds: refused (and said
                // again) while it is still theirs, ours again once it isn't.
                // Never a takeover.
                let due = self
                    .attach
                    .probed_at
                    .is_none_or(|at| self.now.saturating_sub(at) >= PROBE_EVERY);
                if pane != held && due {
                    self.attach.probed_at = Some(self.now);
                    self.attach.held_elsewhere = None;
                    self.attach.regained = Some((held.clone(), pane.to_owned()));
                    log::warn!(
                        "herdr: the focus left {held}, held by another device: asking for it plainly"
                    );
                    self.retarget(&held, false);
                }
                return;
            }
            // B681: the hold outlived its server (a restart): the other
            // device's attach ended with it, as ours had. The device that
            // had the pane asks for it again at once, plainly; this one —
            // it only showed the notice — waits a moment first. Asking at
            // once raced it, and an idle device could keep the pane the
            // person was using. Never a takeover either way: a pick on the
            // other device in that moment stays theirs.
            if !user && !self.pane_gone(&held) && self.hold_outlived(&held) {
                let until = *self
                    .attach
                    .restart_grace_until
                    .get_or_insert(self.now + RESTART_GRACE);
                if self.now < until {
                    return;
                }
            }
            self.attach.restart_grace_until = None;
            self.attach.hold_server_gone = false;
            self.attach.held_elsewhere = None;
            if self
                .notice
                .as_ref()
                .is_some_and(|(notice, _)| *notice == Notice::OnAnotherDevice)
            {
                self.clear_notice();
            }
        }
        if user {
            self.attach.regained = None;
        } else if let Some((regained, theirs)) = &self.attach.regained {
            // Back on the pane the plain ask got back: the other device's
            // focus doesn't pull us after it; a new move of the focus does.
            if pane == theirs && self.attach.target.as_deref() == Some(regained.as_str()) {
                return;
            }
            if pane != regained {
                self.attach.regained = None;
            }
        }
        let budgets = &self.config.budgets;
        let reasking = self.attach.target.as_deref() == Some(pane);
        if reasking {
            // Free while it renders. Not free when the attach asked for
            // never painted: a line the shell ate, a boot the host couldn't
            // run. Past the timeout, the repeat is the recovery.
            let fresh = self
                .attach
                .asked_at
                .is_none_or(|at| self.now.saturating_sub(at) <= budgets.attach_timeout);
            if self.attach.painted || fresh {
                if user && !self.attach.painted {
                    // The person picked the pane a follow is on its way to:
                    // theirs, asked again with `--takeover` should the
                    // follow's plain attach be refused.
                    self.attach.take_over = true;
                }
                if user && self.attach.held.pane.is_none() && self.attach.input_pane() != Some(pane)
                {
                    self.hold_for(pane);
                }
                return;
            }
        }
        let had_target = self.attach.target.is_some();
        if self.attach.held.pane.as_deref() != Some(pane) {
            self.drop_all_held(true);
        }
        // A re-ask of the same pane keeps the person's word.
        self.attach.take_over = user || (reasking && self.attach.take_over);
        self.attach.target = Some(pane.to_owned());
        self.attach.painted = false;
        self.attach.asked_at = Some(self.now);
        self.attach.input_open = false;
        self.dirty = true;
        if user {
            self.hold_for(pane);
        }
        if had_target {
            // Cover the pane for the length of the swap: the release, the
            // reattach and the repaint are round trips. What is under the
            // cover is wiped with the new attach's first full frame, not now
            // (B445): an attach refused, taken over or closed before it
            // painted lifted the cover onto a wiped screen, blank until a
            // resize made herdr paint again.
            self.view(pane, ViewAction::Veil);
            let grace = self.cover_grace();
            self.view(pane, ViewAction::ExtendCover { seconds: grace });
            self.attach.awaiting_full = Some(pane.to_owned());
        }
        self.timers.cancel(&Slot::Settle);
        if let Some(live) = self.attach.attached.take() {
            // Release the attach that is reading. Whoever reads the channel
            // next only does once it has exited; a line written before that
            // is consumed by the dying attach.
            self.attach.releasing = Some(live);
            self.expect_close();
            self.attach.switch = Some(Switch {
                pane: pane.to_owned(),
                phase: Phase::Ack,
            });
            self.write_line(frame::RELEASE.to_owned(), true);
        } else if self.attach.releasing.is_some() {
            // One release is out already (a switch it was written for was
            // overtaken): its close frees the channel for this one too. The
            // app wrote a second release, waited for a second close that
            // never comes, and took the next eviction for it.
            self.attach.switch = Some(Switch {
                pane: pane.to_owned(),
                phase: Phase::Ack,
            });
        } else {
            self.attach.switch = Some(Switch {
                pane: pane.to_owned(),
                phase: Phase::Ack,
            });
            self.write_attach_line();
        }
    }

    /// The veil's safety cap, scaled off the last measured switch.
    fn cover_grace(&self) -> f64 {
        let last = self.attach.last_switch_ms as f64 / 1000.0;
        (last * 2.0 + 0.4).clamp(0.8, 6.0)
    }

    fn expect_close(&mut self) {
        self.attach.next_expected += 1;
        let id = self.attach.next_expected;
        self.attach.expected.push_back(id);
        self.arm(Slot::ExpectedClose(id), self.config.budgets.expected_close);
    }

    /// The release left the lane: its close has `release_ack` to come back.
    fn release_written(&mut self) {
        self.attach.release_sent_at = Some(self.now);
        if self
            .attach
            .switch
            .as_ref()
            .is_some_and(|switch| switch.phase == Phase::Ack)
        {
            self.arm(Slot::ReleaseAck, self.config.budgets.release_ack);
        }
    }

    /// No close for the release: the channel may have stopped answering.
    /// The next line goes anyway; input waits for it to paint, as always.
    /// What arrives before the close is still the attach being let go (a
    /// pane streaming output holds its close back behind its frames): its
    /// frames stay its own, never painting the next pane or letting its
    /// held input out before that one is up (B415). The close, or its
    /// budget running out, ends the release.
    pub(super) fn release_ack_timed_out(&mut self) {
        self.channel_free();
    }

    /// The previous attach let go: the next line can be written (after the
    /// prompt's settle where there is no reader loop).
    fn channel_free(&mut self) {
        self.timers.cancel(&Slot::ReleaseAck);
        let Some(switch) = self.attach.switch.as_mut() else {
            return;
        };
        if switch.phase != Phase::Ack {
            return;
        }
        if self.attach.loop_ready {
            self.write_attach_line();
        } else {
            switch.phase = Phase::Settle;
            self.arm(Slot::Settle, self.config.budgets.prompt_settle);
        }
    }

    pub(super) fn settled(&mut self) {
        if self
            .attach
            .switch
            .as_ref()
            .is_some_and(|switch| switch.phase == Phase::Settle)
        {
            self.write_attach_line();
        }
    }

    /// Name the switch's pane: one word to the loop, the boot line the first
    /// time, the one-shot line where the loop never came up.
    fn write_attach_line(&mut self) {
        let Some(switch) = self.attach.switch.take() else {
            return;
        };
        let pane = switch.pane;
        let (cols, rows) = self.grid_for(&pane);
        let custom = self.config.custom_path.clone();
        let takeover = self.attach.take_over;
        let line = if self.attach.loop_ready {
            frame::retarget(&pane, cols, rows, takeover)
        } else if self.config.frame_loop {
            // The loop runs but hasn't said it reads: the line would reach
            // it before its `read`, or nothing. Written once it says so.
            self.attach.switch = Some(Switch {
                pane,
                phase: Phase::Loop,
            });
            return;
        } else if !self.attach.booted {
            self.attach.booted = true;
            self.arm(Slot::BootProbe, self.config.budgets.loop_probe);
            frame::boot(
                &pane,
                cols,
                rows,
                custom.as_deref(),
                takeover,
                &self.config.connection_id,
            )
        } else {
            frame::start(&pane, cols, rows, custom.as_deref(), takeover)
        };
        self.attach.line_took_over = takeover;
        self.write_line(line, false);
        self.attach.attached = Some(pane);
        self.attach.line_grid = Some((cols, rows));
        // Not before the attach paints: until it is up, herdr drops what
        // reaches it (and if the line was read by an attach still dying, its
        // pane would get it). The app wrote typing, and a notification's
        // reply, right behind the line.
        self.attach.input_open = false;
        // Recovery for an attach that never paints doesn't wait for the
        // poll's cadence (up to 45 s with push): a read just past the
        // timeout re-asks.
        self.arm(Slot::AttachWatch, self.config.budgets.attach_timeout + 1);
    }

    fn grid_for(&self, pane: &str) -> (i64, i64) {
        self.attach
            .grids
            .get(pane)
            .copied()
            .or(self.attach.last_grid)
            .unwrap_or((80, 24))
    }

    /// The loop never said it is up and nothing painted: this host's shell
    /// couldn't take the boot line's shape. The one-shot line takes over.
    pub(super) fn boot_probe_fired(&mut self) {
        if self.attach.loop_ready || self.attach.painted {
            return;
        }
        if self.config.frame_loop {
            // The channel's own program never said it reads: nothing to
            // type anywhere. Said, never a silent blank terminal.
            self.channel_stuck();
            return;
        }
        let Some(target) = self.attach.target.clone() else {
            return;
        };
        if self.attach.attached.as_deref() != Some(target.as_str()) {
            return;
        }
        let (cols, rows) = self.grid_for(&target);
        let takeover = self.attach.take_over;
        let line = frame::start(
            &target,
            cols,
            rows,
            self.config.custom_path.as_deref(),
            takeover,
        );
        self.attach.line_took_over = takeover;
        self.write_line(line, false);
    }

    pub(super) fn attach_watch_fired(&mut self) {
        if self.attach.target.is_some() && !self.attach.painted {
            self.attach.unpainted += 1;
            if self.config.frame_loop && self.attach.unpainted >= 2 {
                self.channel_stuck();
            }
            self.quicken();
        }
    }

    /// The frame channel shows nothing and won't by itself: said, with a
    /// retry that redials ([`super::Event::FrameChannelLost`]). A notice
    /// herdr itself put up says more and stays.
    fn channel_stuck(&mut self) {
        if self.attach.stuck {
            return;
        }
        self.attach.stuck = true;
        if self.notice.is_none()
            || self
                .notice
                .as_ref()
                .is_some_and(|(notice, _)| *notice == Notice::InputDropped)
        {
            self.set_notice(Notice::InputDropped, Some(super::NoticeAction::Retry));
        }
    }

    /// The channel paints after all: whatever said it was stuck is over.
    fn channel_unstuck(&mut self) {
        self.attach.unpainted = 0;
        if std::mem::take(&mut self.attach.stuck)
            && self.notice == Some((Notice::InputDropped, Some(super::NoticeAction::Retry)))
        {
            self.clear_notice();
        }
    }

    /// The person's Retry on a stuck channel: a fresh connection and loop.
    pub(super) fn retry_stuck_channel(&mut self) -> bool {
        if !self.attach.stuck {
            return false;
        }
        self.clear_notice();
        self.event(Event::FrameChannelLost);
        true
    }

    pub(super) fn expected_close_forgotten(&mut self, id: u64) {
        self.attach.expected.retain(|&expected| expected != id);
        if self.attach.expected.is_empty() {
            // No close is coming for the release: what arrives now is the
            // next attach's (B415).
            self.attach.releasing = None;
            // B680: and a switch waiting for that close goes now.
            self.channel_free();
        }
    }

    // MARK: - What the channel says

    /// Bytes off the frame channel.
    pub fn frame_bytes(&mut self, bytes: &[u8], now: u64) {
        self.begin(now);
        if !bytes.is_empty() {
            self.last_inbound = Some(self.now);
        }
        let frames = self.attach.parser.consume(bytes);
        for frame in frames {
            if !self.attach.open {
                break;
            }
            self.frame(frame);
        }
        self.end();
    }

    fn frame(&mut self, frame: Frame) {
        match frame {
            Frame::Screen { full, bytes, .. } => self.screen(full, bytes),
            Frame::LoopReady => {
                self.attach.loop_ready = true;
                self.timers.cancel(&Slot::BootProbe);
                if self
                    .attach
                    .switch
                    .as_ref()
                    .is_some_and(|switch| switch.phase == Phase::Loop)
                {
                    self.write_attach_line();
                }
            }
            Frame::Closed { reason } => self.closed(reason),
            Frame::Fault { message } => self.fault(message),
        }
    }

    fn screen(&mut self, full: bool, bytes: Vec<u8>) {
        let Some(pane) = self
            .attach
            .releasing
            .clone()
            .or_else(|| self.attach.attached.clone())
        else {
            return;
        };
        if full
            && self.attach.releasing.is_none()
            && self.attach.target.as_deref() == Some(pane.as_str())
            && self.attach.awaiting_full.as_deref() == Some(pane.as_str())
        {
            // The switch's own frame is here: what was under its cover goes
            // now (the history with it), the new pane's picture right after.
            self.pane_bytes(&pane, CLEAR_SCREEN.to_vec());
        }
        self.pane_bytes(&pane, bytes);
        if self.attach.releasing.is_some() || self.attach.target.as_deref() != Some(pane.as_str()) {
            // The attach being let go: its last frames are its own, and say
            // nothing about the one asked for.
            return;
        }
        // Painting means this attach holds the pane: any warning is stale.
        if self
            .notice
            .as_ref()
            // A one-off goes on its own clock (B491).
            .is_some_and(|(notice, _)| !matches!(notice, Notice::Restarting | Notice::SwitchFailed))
        {
            self.clear_notice();
        }
        if !self.attach.painted {
            self.attach.painted = true;
            self.dirty = true;
            self.channel_unstuck();
            if !self.attach.input_open {
                // The attach is up and reading: input can go, what was held
                // for it first. Its size may have moved since its line.
                self.attach.input_open = true;
                if let Some(grid) = self.attach.grids.get(&pane).copied()
                    && Some(grid) != self.attach.line_grid
                {
                    self.attach.line_grid = Some(grid);
                    self.write_line(frame::resize(grid.0, grid.1), false);
                }
                self.flush_held(&pane);
            }
        }
        if !self.first_frame {
            self.first_frame = true;
            self.event(Event::FirstFrame);
        }
        if full && self.attach.awaiting_full.as_deref() == Some(pane.as_str()) {
            self.attach.awaiting_full = None;
            if let Some(asked) = self.attach.asked_at {
                self.attach.last_switch_ms = self.now.saturating_sub(asked);
            }
            self.view(&pane, ViewAction::Reveal);
        }
    }

    /// `terminal.closed`: our own release coming back, or the attach ended
    /// by someone else.
    fn closed(&mut self, reason: Option<String>) {
        if let Some(id) = self.attach.expected.pop_front() {
            self.timers.cancel(&Slot::ExpectedClose(id));
            self.attach.releasing = None;
            self.attach.release_sent_at = None;
            self.channel_free();
            return;
        }
        let Some(pane) = self.attach.attached.take() else {
            // Nothing of ours is attached: a close from no attach we know.
            return;
        };
        let wanted = self.attach.take_over;
        let asked = self.attach.line_took_over;
        self.attach.input_open = false;
        self.attach.target = None;
        self.attach.take_over = false;
        self.dirty = true;
        let settling = self.attach.settling.take();
        self.drop_all_held(true);
        self.hold_settling(settling);
        let reason = reason.unwrap_or_default();
        if self.attach.awaiting_full.take().is_some() {
            log::warn!(
                "herdr: the attach to {pane} closed before its first full frame ({reason}): its cover lifts over what the pane showed"
            );
            self.view(&pane, ViewAction::Reveal);
        } else {
            log::warn!("herdr: the attach to {pane} closed ({reason})");
        }
        if reason.contains("taken over") || crate::launch::held_elsewhere(&reason) {
            if wanted && !asked {
                // The person picked the pane a follow's plain attach was
                // refused for: theirs, asked again with `--takeover`.
                self.retarget(&pane, true);
                return;
            }
            // Another device has the pane: said, and never taken back
            // without the person. It used to be asked for again on the next
            // read (`--takeover`), three times in 30 s before a backoff.
            log::warn!(
                "herdr: {pane} is held by another device; not taken back without the person"
            );
            self.hold(pane);
            return;
        }
        // A pane that doesn't exist (closed, or a stale focus), or anything
        // else: the focused pane, by itself.
        self.quicken();
    }

    /// Whether the server, running and read since, no longer has `pane`.
    pub(super) fn pane_gone(&self, pane: &str) -> bool {
        !self.poll.server_not_running
            && !self.poll.snapshot.panes.is_empty()
            && !self.poll.snapshot.panes.contains_key(pane)
    }

    /// "Open here": the pane another device has, taken from it
    /// (`--takeover`) — or, gone meanwhile, the focused one. A pick in
    /// full: the server's focus goes there too (B409), else the next read
    /// follows the focus to where the other device moved it.
    pub(super) fn open_here(&mut self) {
        let Some(pane) = self.attach.held_elsewhere.clone() else {
            return;
        };
        if self.pane_gone(&pane) {
            self.attach.held_elsewhere = None;
            self.clear_notice();
            self.quicken();
            return;
        }
        self.select_pane(&pane);
    }

    /// herdr said something on the channel that isn't a frame.
    fn fault(&mut self, message: String) {
        // B117: herdr turned away one line on an attach that is still alive
        // (`terminal session control input ignored: …`). The app took it for
        // a refused attach: dropped the pane and backed off for 30 s while the
        // attach kept painting.
        if message.contains("input ignored") {
            self.event(Event::InputIgnored(message));
            return;
        }
        // B466: the attach's CLI says its last words on the channel when the
        // attach ends on the host's side (`herdr: server shut down: …`). The
        // server stopping or restarting and the pane closing are no refusal,
        // nor is another device taking the pane: each was said as one and
        // backed off from for 30 s, the stale frame on screen.
        if message.contains("shutting down")
            || (message.contains("terminal attach") && message.contains("not found"))
        {
            return self.attach_gone(&message);
        }
        if message.contains("taken over") || crate::launch::held_elsewhere(&message) {
            return self.attach_taken(&message);
        }
        let refused = message.clone();
        if snapshot::looks_like_version_skew(&message) {
            self.offer_restart(message);
        } else {
            // Said with how long until it is asked again, and a button
            // that asks now (B490).
            self.set_notice(
                Notice::Refused(message.clone()),
                Some(super::NoticeAction::Retry),
            );
            if frame::is_handshake_drop(&message) {
                // A hang-up mid-handshake names no versions: ask.
                let command = self.herdr(control::STATUS);
                self.exec(
                    command,
                    Purpose::Status(super::StatusFor::Explain(message)),
                    Some(self.config.budgets.status),
                );
            }
        }
        // Stop claiming the pane, and back off: a refusal doesn't heal by
        // asking again on every read.
        self.attach.attached = None;
        self.attach.target = None;
        self.attach.input_open = false;
        self.attach.switch = None;
        self.timers.cancel(&Slot::Settle);
        self.timers.cancel(&Slot::ReleaseAck);
        self.drop_all_held(true);
        self.attach.retry_after = Some(self.now + self.config.budgets.refusal_backoff);
        self.attach.retry_wait = Some(self.config.budgets.refusal_backoff);
        self.dirty = true;
        let awaiting = self.attach.awaiting_full.take();
        log::warn!(
            "herdr: an attach was refused ({refused}); asked again in {} ms{}",
            self.config.budgets.refusal_backoff,
            awaiting.as_ref().map_or(String::new(), |pane| format!(
                ", the cover over {pane} lifts without its frame"
            ))
        );
        if let Some(awaiting) = awaiting {
            self.view(&awaiting, ViewAction::Reveal);
        }
    }

    /// The attach let go of the channel on the host's side: nothing of ours
    /// reads it, and nothing typed for it goes anywhere now.
    fn attach_ended(&mut self) {
        // Fix (B680): the attach a release was out for ended on the host's
        // side (its last words: the server shut down, the pane closed):
        // it let go of the channel, and no close is coming for the release
        // (the reader loop takes the release's line for nothing). The next
        // attach waited for that close anyway, then for nobody: the
        // switch's line was never written.
        if self.attach.releasing.take().is_some() {
            for id in self.attach.expected.drain(..) {
                self.timers.cancel(&Slot::ExpectedClose(id));
            }
            self.attach.release_sent_at = None;
        }
        self.attach.attached = None;
        self.attach.target = None;
        self.attach.input_open = false;
        self.attach.switch = None;
        self.timers.cancel(&Slot::Settle);
        self.timers.cancel(&Slot::ReleaseAck);
        let settling = self.attach.settling.take();
        self.drop_all_held(true);
        self.hold_settling(settling);
        self.dirty = true;
        if let Some(awaiting) = self.attach.awaiting_full.take() {
            self.view(&awaiting, ViewAction::Reveal);
        }
    }

    /// The attach ended because its pane closed or the server stopped or
    /// restarted (B466): the next read names the pane to follow, or says
    /// there is no server; asked for now.
    fn attach_gone(&mut self, said: &str) {
        log::warn!(
            "herdr: the attach to {} ended ({said}): following the focus",
            self.attach.attached.as_deref().unwrap_or("-")
        );
        self.attach_ended();
        self.quicken();
    }

    /// Another device took the pane, said by the CLI (B466): the pane is
    /// theirs until the person's word, as when the close says so.
    fn attach_taken(&mut self, said: &str) {
        let Some(pane) = self
            .attach
            .attached
            .clone()
            .or_else(|| self.attach.target.clone())
        else {
            return self.attach_gone(said);
        };
        log::warn!(
            "herdr: {pane} was taken by another device ({said}); not taken back without the person"
        );
        self.attach_ended();
        self.hold(pane);
    }

    /// Another device has `pane`: said with its take-back, and its terminal
    /// noted (B467).
    fn hold(&mut self, pane: String) {
        // A plain ask refused: still theirs, nothing got back.
        self.attach.regained = None;
        self.attach.held_terminal = self.poll.terminal_ids.get(&pane).cloned();
        self.attach.held_elsewhere = Some(pane);
        self.attach.hold_server_gone = false;
        self.attach.restart_grace_until = None;
        self.set_notice(Notice::OnAnotherDevice, Some(super::NoticeAction::OpenHere));
    }

    /// Whether the server restarted since the hold on `pane` began: its
    /// terminal is a new one (B467). The other device's attach ended with
    /// the old server.
    fn hold_outlived(&self, pane: &str) -> bool {
        self.attach.hold_server_gone
            || matches!(
                (self.attach.held_terminal.as_deref(), self.poll.terminal_ids.get(pane)),
                (Some(then), Some(now)) if then != now
            )
    }

    /// What was backed off from is asked for now (B490): the banner's
    /// Retry, the app's return, a server of another version. A refusal's
    /// banner goes with it; a skew's stays until the server answers.
    pub(super) fn retry_now(&mut self) {
        if self.attach.retry_after.take().is_none() {
            return;
        }
        self.attach.retry_wait = None;
        if self
            .notice
            .as_ref()
            .is_some_and(|(notice, _)| matches!(notice, Notice::Refused(_)))
        {
            self.clear_notice();
        }
        self.dirty = true;
        self.quicken();
    }

    /// No server: whatever another device held, it holds no more (B467) —
    /// once the server is back and this device has waited its moment
    /// (B681): the device that had the pane asks for it first.
    pub(super) fn server_gone_ends_hold(&mut self) {
        if self.attach.held_elsewhere.is_some() {
            log::warn!("herdr: the server went away: the pane another device held is free");
            self.attach.hold_server_gone = true;
        }
    }

    /// The frame channel ended (its shell exited, the connection dropped).
    pub fn frame_closed(&mut self, now: u64) {
        self.begin(now);
        if self.attach.open {
            self.attach.open = false;
            self.attach.target = None;
            self.attach.attached = None;
            self.attach.releasing = None;
            self.attach.input_open = false;
            self.attach.switch = None;
            self.attach.held_elsewhere = None;
            self.attach.lane.queue.clear();
            self.drop_all_held(false);
            self.dirty = true;
            self.event(Event::FrameChannelLost);
        }
        self.end();
    }

    // MARK: - Input

    /// Keys or text typed at `pane`'s terminal.
    pub fn input(&mut self, pane: &str, bytes: &[u8], now: u64) {
        self.begin(now);
        if !bytes.is_empty() && self.attach.open && !self.closed {
            self.route_input(pane, HeldStep::Input(bytes.to_vec()));
        }
        self.end();
    }

    /// Text pasted into `pane`'s terminal: one bracketed paste, whatever the
    /// local terminal thinks, for herdr to fit to the pane's real mode (B43),
    /// with any marker inside taken out (B114).
    pub fn paste(&mut self, pane: &str, text: &str, now: u64) {
        self.begin(now);
        if !text.is_empty() && self.attach.open && !self.closed {
            self.route_input(pane, HeldStep::Paste(frame::bracketed_paste(text)));
        }
        self.end();
    }

    /// `pane`'s terminal has this grid.
    pub fn resize(&mut self, pane: &str, cols: i64, rows: i64, now: u64) {
        self.begin(now);
        self.attach.grids.insert(pane.to_owned(), (cols, rows));
        self.attach.last_grid = Some((cols, rows));
        if self.attach.open && self.attach.input_pane() == Some(pane) {
            self.attach.line_grid = Some((cols, rows));
            self.write_line(frame::resize(cols, rows), false);
        }
        self.end();
    }

    /// Input for `pane`: out now if its attach is reading, held if it is
    /// the pane being switched to on the person's pick, else dropped.
    fn route_input(&mut self, pane: &str, step: HeldStep) {
        if self.attach.input_pane() == Some(pane) {
            match self.attach.settling.as_mut() {
                Some(settling) if settling.pane == pane => settling.steps.push(step),
                _ => self.send_step(pane, step),
            }
        } else if self.attach.held.pane.as_deref() == Some(pane) {
            self.attach.held.steps.push(step);
        } else if !matches!(step, HeldStep::Delivery { .. })
            && self.attach.held.pane.is_none()
            && self.attach.target.as_deref() == Some(pane)
        {
            // B493: the pane a follow is on its way to (the desktop switched
            // to it), typed at before its attach is up: held for it, as a
            // pick's typing is, for two seconds at most, then sent once it
            // paints. It was dropped and said.
            self.attach.held.pane = Some(pane.to_owned());
            self.arm(Slot::HeldInput, FOLLOW_HOLD);
            self.attach.held.steps.push(step);
        } else {
            self.not_sent(pane, step);
        }
    }

    /// One step out to `pane`, whose attach reads: a paste starts the wait
    /// of what follows it (B485).
    fn send_step(&mut self, pane: &str, step: HeldStep) {
        match step {
            HeldStep::Input(bytes) => self.send_input(pane, &bytes),
            HeldStep::Paste(bytes) => {
                self.send_input(pane, &bytes);
                self.attach.settling = Some(Settling {
                    pane: pane.to_owned(),
                    steps: Vec::new(),
                });
                self.arm(Slot::PasteSettle, PASTE_SETTLE_MS);
            }
            HeldStep::Delivery { request, bytes } => {
                self.send_input(pane, &bytes);
                self.event(Event::Delivered {
                    request: super::RequestId(request),
                    delivered: true,
                });
            }
        }
    }

    /// A step that can't reach `pane`: typing is said, a delivery answered.
    fn not_sent(&mut self, pane: &str, step: HeldStep) {
        match step {
            HeldStep::Delivery { request, .. } => self.event(Event::Delivered {
                request: super::RequestId(request),
                delivered: false,
            }),
            HeldStep::Input(_) | HeldStep::Paste(_) => self.input_dropped(pane),
        }
    }

    /// `steps` out to `pane` in order, up to a paste, which the rest then
    /// waits behind (B485).
    fn release_steps(&mut self, pane: &str, steps: Vec<HeldStep>) {
        let mut steps = steps.into_iter();
        while let Some(step) = steps.next() {
            let paste = matches!(step, HeldStep::Paste(_));
            self.send_step(pane, step);
            if paste {
                if let Some(settling) = self.attach.settling.as_mut() {
                    settling.steps.extend(steps);
                }
                return;
            }
        }
    }

    /// A paste has had its time: what followed it goes, if its pane is
    /// still the one typed into (herdr's input line goes to whatever
    /// terminal is attached: never into another pane).
    pub(super) fn paste_settled(&mut self) {
        let Some(Settling { pane, steps }) = self.attach.settling.take() else {
            return;
        };
        if self.attach.input_pane() == Some(pane.as_str()) {
            self.release_steps(&pane, steps);
        } else {
            self.hold_settling(Some(Settling { pane, steps }));
        }
    }

    /// What waited behind a paste, its pane's attach gone before it went
    /// (the attach ended or closed: herdr re-attaching it, the server
    /// restarted): it waits for the pane's next attach as a follow's typing
    /// does (B493), ahead of what was typed since, and goes once that
    /// paints — the round trips of the re-attach being the rest of the
    /// paste's time. It was dropped and said: the image panel's Send lost
    /// its Return (B527). An attach to another pane drops it, said, as any
    /// hold; never into another pane.
    fn hold_settling(&mut self, settling: Option<Settling>) {
        let Some(Settling { pane, steps }) = settling else {
            return;
        };
        self.timers.cancel(&Slot::PasteSettle);
        if steps.is_empty() {
            return;
        }
        let ours = match self.attach.held.pane.as_deref() {
            Some(held) => held == pane,
            None => self
                .attach
                .target
                .as_deref()
                .is_none_or(|target| target == pane),
        };
        if !ours {
            for step in steps {
                self.not_sent(&pane, step);
            }
            return;
        }
        if self.attach.held.pane.is_none() {
            self.attach.held.pane = Some(pane);
            self.arm(Slot::HeldInput, FOLLOW_HOLD);
        }
        let since = std::mem::replace(&mut self.attach.held.steps, steps);
        self.attach.held.steps.extend(since);
    }

    fn send_input(&mut self, pane: &str, bytes: &[u8]) {
        // A lone arrow goes as a semantic key, for herdr to encode against
        // the pane's real cursor-key mode (the local terminal never learns
        // it from repaint frames) — arrows that arrive together too (B360).
        for piece in frame::split_arrows(bytes) {
            match piece {
                frame::Piece::Arrow(key) => self.write_key(pane, key),
                frame::Piece::Bytes(run) => self.write_line(frame::input(run), false),
            }
        }
    }

    /// Say that input went nowhere, and hurry the re-attach.
    fn input_dropped(&mut self, pane: &str) {
        self.event(Event::InputDropped {
            pane: pane.to_owned(),
        });
        if self.notice.is_none() {
            self.set_notice(Notice::InputDropped, None);
            self.quicken();
        }
    }

    /// Hold input for `pane` until its attach reads.
    fn hold_for(&mut self, pane: &str) {
        if self.attach.held.pane.as_deref() == Some(pane) {
            return;
        }
        self.drop_all_held(true);
        self.attach.held.pane = Some(pane.to_owned());
        self.arm(Slot::HeldInput, self.config.budgets.held_input);
    }

    /// `pane`'s attach reads now: what was held for it follows, in order.
    fn flush_held(&mut self, pane: &str) {
        if self.attach.held.pane.as_deref() != Some(pane) {
            return;
        }
        self.timers.cancel(&Slot::HeldInput);
        self.attach.held.pane = None;
        let steps = std::mem::take(&mut self.attach.held.steps);
        self.release_steps(pane, steps);
    }

    /// Drop what is held: said for typing (`tell`), answered for deliveries.
    pub(super) fn drop_all_held(&mut self, tell: bool) {
        self.timers.cancel(&Slot::HeldInput);
        // What waited behind a paste goes with the hold (B485).
        self.timers.cancel(&Slot::PasteSettle);
        if let Some(Settling { pane, steps }) = self.attach.settling.take() {
            let mut typed = false;
            for step in steps {
                match step {
                    HeldStep::Input(_) | HeldStep::Paste(_) => typed = true,
                    HeldStep::Delivery { request, .. } => self.event(Event::Delivered {
                        request: super::RequestId(request),
                        delivered: false,
                    }),
                }
            }
            if typed && tell {
                self.input_dropped(&pane);
            }
        }
        let Some(pane) = self.attach.held.pane.take() else {
            return;
        };
        let mut typed = false;
        for step in std::mem::take(&mut self.attach.held.steps) {
            match step {
                HeldStep::Input(_) | HeldStep::Paste(_) => typed = true,
                HeldStep::Delivery { request, .. } => self.event(Event::Delivered {
                    request: super::RequestId(request),
                    delivered: false,
                }),
            }
        }
        if typed && tell {
            self.input_dropped(&pane);
        }
    }

    pub(super) fn held_input_expired(&mut self) {
        self.drop_all_held(true);
    }

    /// Bytes for one pane, wherever the channel is (`deliverInput`): switch
    /// there if it isn't, then send once its attach reads.
    pub(super) fn deliver(&mut self, pane: &str, bytes: Vec<u8>) -> super::RequestId {
        let request = self.request_id();
        // Another device has the pane: nothing reaches it, and a reply is no
        // reason to take it from that device ("Open here" is the person's).
        // A reply for another pane is the person's pick of it.
        if !self.attach.open
            || bytes.is_empty()
            || self.attach.held_elsewhere.as_deref() == Some(pane)
        {
            self.event(Event::Delivered {
                request,
                delivered: false,
            });
            return request;
        }
        if self.attach.input_pane() != Some(pane) {
            if self.attach.target.as_deref() != Some(pane) {
                self.select_pane(pane);
            } else {
                self.hold_for(pane);
            }
            if self.attach.held.pane.as_deref() == Some(pane) {
                // The delivery's own wait, not the typing's.
                self.arm(Slot::HeldInput, self.config.budgets.delivery);
            }
        }
        self.route_input(
            pane,
            HeldStep::Delivery {
                request: request.0,
                bytes,
            },
        );
        request
    }

    /// A drag or wheel (or the paging keys) over `pane`: herdr's server
    /// decides between a mouse report and scrollback.
    pub(super) fn scroll(&mut self, pane: &str, lines: i64, source: ScrollSource) {
        if lines != 0 && self.attach.open && self.attach.input_pane() == Some(pane) {
            self.write_line(frame::scroll(lines > 0, lines.abs(), source), false);
        }
    }

    /// A pointer event over `pane`, for a server that takes them (≥ 0.9.2).
    pub(super) fn mouse(
        &mut self,
        pane: &str,
        action: MouseAction,
        button: MouseButton,
        column: i64,
        row: i64,
        modifiers: u8,
    ) {
        let takes = self
            .server_version
            .as_deref()
            .is_some_and(frame::takes_mouse);
        if takes && self.attach.open && self.attach.input_pane() == Some(pane) {
            self.write_line(frame::mouse(action, button, column, row, modifiers), false);
        }
    }
}
