//! The mosh State Synchronization Protocol client as a pure state machine,
//! ported from the socket-free half of `MoshTransport.swift`.
//!
//! Inputs: received datagrams, user input, channel state changes and timer
//! wake-ups, each with the caller's monotonic clock in milliseconds. Outputs
//! queue up as [`Event`]s ([`Transport::poll_event`]): datagrams to send,
//! host output for the terminal, metrics, and decisions the platform layer
//! acts on (restart or rebuild the UDP flow, report a dead return path).
//! [`Transport::next_deadline`] says when to call [`Transport::on_timer`].
//!
//! What it keeps from the Swift actor:
//!
//! - The client→server user stream: one numbered state per keystroke batch
//!   (split to fit an instruction) or resize, sent by mosh's own sender
//!   discipline (`transportsender-impl.h`, see `send_due`): new states at
//!   most once per send interval, from the state the server is assumed to
//!   hold, everything unacked again only once the oldest of it has gone a
//!   retransmission timeout without an ack, in instructions of at most
//!   64 KiB of events (see `send`).
//! - The server→client display state: a diff is applied only when its
//!   `old_num` is the state already applied (the terminal has no framebuffer
//!   of its own, so host bytes compose only in order); a gap or a diff that
//!   does not decode is answered with an ack of the true state, never
//!   skipped over. Output is acked within mosh's 100 ms `ACK_DELAY`.
//! - Keystrokes unacked for 10 s stop being retransmitted (resizes are kept).
//! - A send timer once the channel is ready (see [`Transport::next_deadline`]),
//!   a one-shot return-path watchdog (8 s with nothing received), a liveness
//!   check every 3 s that rebuilds a flow silent for 9 s and reports the
//!   return path dead on the third rebuild in a row, and a new flow one
//!   second after a failure.
//! - A smoothed RTT estimate (and mosh's RTT variance, for the timeout) and
//!   the roaming flag.
//! - An idle client gives the server a new, empty user state every 3 s, as
//!   mosh's client does with its empty acks (see `idle_state`).
//!
//! Where it differed from the Swift actor, each marked `Fix (<id>)` below
//! and listed in `harness/divergences.json` (the Swift transport has carried
//! all four since):
//!
//! - `throwaway`: every instruction carries `throwaway_num`, so mosh-server
//!   can drop the user states it no longer needs.
//! - `ttl-rewrite`: expired keystrokes are dropped only once the server has
//!   provably not received them.
//! - `timestamps`: timestamps and RTT samples follow mosh's `network.cc`.
//! - `full-redraw`: a requested redraw actually makes the server repaint.

use std::collections::VecDeque;

use crate::compress;
use crate::crypto::{Direction, Message, Session};
use crate::ocb::OcbError;
use crate::wire::{
    FRAGMENT_HEADER_LEN, Fragment, FragmentAssembler, HostMessage, Instruction, UserEvent,
    encode_fragment_into, encode_user_message, fragment_ranges,
};

/// Timing and size parameters. [`Default`] gives the app's values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportConfig {
    /// How long an unacked keystroke keeps being retransmitted (10 s).
    pub keystroke_ttl_ms: u64,
    /// No datagram at all this long after the channel became ready means
    /// the return path is dead (8 s).
    pub return_path_deadline_ms: u64,
    /// Liveness check cadence, from the first datagram sent (3 s).
    pub liveness_interval_ms: u64,
    /// Sent but heard nothing for this long: rebuild the flow (9 s).
    pub liveness_deadline_ms: u64,
    /// Restart delay after the channel fails (1 s).
    pub failure_restart_delay_ms: u64,
    /// How long after input is held back (see `ttl-rewrite`) a datagram from
    /// the server proves the held keystrokes never reached it (5 s, mosh's
    /// bound on a plausible round trip).
    pub expiry_proof_delay_ms: u64,
    /// Largest fragment (header included) per datagram (1300).
    pub fragment_mtu: usize,
    /// Most bytes of encoded user events per instruction (64 KiB); input
    /// is split into states that fit (see `Transport::send`).
    pub instruction_budget: usize,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            keystroke_ttl_ms: 10_000,
            return_path_deadline_ms: 8_000,
            liveness_interval_ms: 3_000,
            liveness_deadline_ms: 9_000,
            failure_restart_delay_ms: 1_000,
            expiry_proof_delay_ms: 5_000,
            fragment_mtu: 1_300,
            instruction_budget: 64 * 1024,
        }
    }
}

/// The UDP flow's state as the platform reports it (Network.framework's
/// `NWConnection.State`; a tokio socket only ever reports `Ready`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelState {
    #[default]
    Setup,
    Preparing,
    Waiting,
    Ready,
    Failed,
    Cancelled,
}

/// One applied server diff, as the terminal consumes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFrame {
    /// Terminal output (empty when only the echo ack moved).
    pub bytes: Vec<u8>,
    /// The newest user state ([`Transport::send`]'s number) the server had
    /// received at least 50 ms before this frame. Monotonic.
    pub echo_ack: u64,
}

/// What the state machine asks of its driver, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Send this datagram on the current flow.
    Transmit(Vec<u8>),
    /// Feed this to the terminal.
    HostFrame(HostFrame),
    /// The server acknowledged every user state up to this number
    /// (what `waitUntilDelivered` waits for).
    Acked(u64),
    /// Smoothed RTT (0 until the first sample) and roaming flag, published
    /// where the Swift transport calls `onMetrics`.
    Metrics { srtt_ms: f64, roaming: bool },
    /// Restart the current flow (`NWConnection.restart()`): only ever for
    /// a flow that is waiting, the one state Network.framework restarts.
    RestartFlow,
    /// Cancel the current flow and open a fresh socket, then report its
    /// states; mosh does not care about socket identity. Also what a failed
    /// flow gets: it never comes back. The driver keeps exactly one receive
    /// armed on the current flow, re-armed when it is ready again, never
    /// on a new flow by an old flow's completion.
    RebuildFlow,
    /// The return path looks dead: offer SSH, reap a server that never
    /// heard from us. Can come twice (watchdog, then liveness).
    ReturnPathDead,
    /// mosh-server ended the session (its command exited, or it was told to
    /// stop): its shutdown state arrived and a datagram acking it went out
    /// (the `Transmit` before this). The transport has closed itself;
    /// `Finished` follows. There is nothing to roam back to: report the
    /// end, do not redial. A driver whose sends complete later (an
    /// `NWConnection`) cancels the flow only once they did: cancelling drops
    /// the ack mosh-server quits on, and it then retries 16 times first.
    SessionEnded,
    /// The host stream ended (closed, or the channel was cancelled).
    Finished,
}

/// Running protocol counters for the diagnostics popover
/// (`MoshDiagnostics`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Diagnostics {
    /// Datagrams received, before decryption.
    pub datagrams_received: u64,
    /// Applicable diffs that did not decode (acked as not applied).
    pub parse_failures: u64,
    /// Diffs that did not extend the applied state.
    pub gap_events: u64,
    /// Highest server state applied.
    pub applied_host_num: u64,
    /// Datagrams sent.
    pub packets_sent: u64,
    /// Newest user state queued.
    pub last_user_num: u64,
    /// Newest user state the server acknowledged.
    pub server_acked_user_num: u64,
    /// Flow rebuilds so far.
    pub flow_rebuilds: u64,
    /// Expired keystrokes are being held back (see `ttl-rewrite`).
    pub input_held: bool,
    pub channel_state: ChannelState,
}

/// mosh's "no timestamp" marker.
const NO_TIMESTAMP: u16 = u16::MAX;
/// A saved server timestamp is echoed only this fresh (mosh `new_packet`).
const TIMESTAMP_HOLD_LIMIT_MS: u64 = 1_000;
/// RTT samples at or above this are ignored (mosh `recv_one`).
const MAX_RTT_SAMPLE_MS: f64 = 5_000.0;
/// mosh's number for the shutdown state (`uint64_t(-1)`): mosh-server sends
/// it once its command exited or it was told to stop, and quits once it is
/// acked.
pub const SHUTDOWN_STATE: u64 = u64::MAX;
/// mosh's `ACK_INTERVAL`: how long after the last new user state an idle
/// client sends another (see `Transport::idle_state`), and how long after
/// its last datagram a client with nothing to say says it anyway.
pub const IDLE_STATE_INTERVAL_MS: u64 = 3_000;
/// mosh's `ACK_DELAY`: output from the server is acked this soon, and a
/// state sent this long past the retransmission timeout without an ack is
/// presumed lost.
pub const ACK_DELAY_MS: u64 = 100;
/// mosh's `ACTIVE_RETRY_TIMEOUT`: unacked input is retried at frame rate
/// only this long after the server was last heard; after that, with every
/// 3 s ack.
pub const ACTIVE_RETRY_TIMEOUT_MS: u64 = 10_000;
/// mosh's frame-rate bounds (`SEND_INTERVAL_MIN`, `SEND_INTERVAL_MAX`).
const SEND_INTERVAL_MIN_MS: f64 = 20.0;
const SEND_INTERVAL_MAX_MS: f64 = 250.0;
/// mosh's retransmission timeout bounds (`MIN_RTO`, `MAX_RTO`).
const MIN_RTO_MS: f64 = 50.0;
const MAX_RTO_MS: f64 = 1_000.0;
/// mosh's estimate before the first RTT sample (`network.cc`).
const INITIAL_SRTT_MS: f64 = 1_000.0;
const INITIAL_RTTVAR_MS: f64 = 500.0;
/// mosh's prophylactic resend (`attempt_prospective_resend_optimization`):
/// everything unacked goes again when that is no longer than the new part
/// alone, or shorter than this and at most `RESEND_SLACK` bytes longer.
const RESEND_SMALL: usize = 1_000;
const RESEND_SLACK: usize = 100;

#[derive(Debug, Clone)]
struct UserState {
    num: u64,
    events: Vec<UserEvent>,
    queued_at_ms: u64,
    /// Went out in at least one instruction, so the server may hold it.
    sent: bool,
    /// When an instruction last carried it (meaningful once `sent`): mosh's
    /// `TimestampedState::timestamp` for a sent state. Never decreases along
    /// the list: an instruction carries every state after the one it starts
    /// from.
    sent_at: u64,
}

impl UserState {
    fn has_keystroke(&self) -> bool {
        self.events.iter().any(UserEvent::is_keystroke)
    }
}

/// The SSP client. See the module documentation.
pub struct Transport {
    config: TransportConfig,
    session: Session,
    events: VecDeque<Event>,

    started: bool,
    closed: bool,
    finished: bool,
    channel_state: ChannelState,

    send_seq: u64,
    /// Outgoing user states; `[0]` is the empty baseline, kept forever.
    user_states: Vec<UserState>,
    server_acked_user_num: u64,
    applied_host_num: u64,
    echo_ack: u64,
    assembler: FragmentAssembler,
    next_instruction_id: u64,

    datagrams_received: u64,
    parse_failures: u64,
    gap_events: u64,
    flow_rebuilds: u64,

    cols: i32,
    rows: i32,

    saved_timestamp: Option<(u16, u64)>,
    expected_receiver_seq: u64,
    srtt: f64,
    /// mosh's `RTTVAR`, for the retransmission timeout.
    rttvar: f64,
    rtt_hit: bool,
    roaming: bool,

    /// The send timer runs: the channel has been ready (see
    /// [`Transport::next_deadline`]).
    ticking: bool,
    /// mosh's `next_ack_time`: a datagram goes out then even with nothing
    /// new to say (the 3 s ack, output acked within `ACK_DELAY_MS`).
    next_ack_at: Option<u64>,
    /// When the server last gave us a new state (mosh's sender
    /// `last_heard`), or the first datagram went out.
    last_heard: Option<u64>,
    watchdog_at: Option<u64>,
    watchdog_armed: bool,
    saw_any_datagram: bool,
    liveness_at: Option<u64>,
    restarts_at: Vec<u64>,
    last_inbound_ms: u64,
    last_send_ms: Option<u64>,
    rebuilds_since_inbound: u32,

    /// When a datagram last carried a user state the server had not been sent
    /// before (see `Transport::idle_state`).
    last_new_state_ms: u64,
    /// When input started being held back (see `ttl-rewrite`).
    held_since: Option<u64>,
    /// A full redraw in progress (see `full-redraw`).
    redraw: Option<Redraw>,
    /// mosh-server ended the session (see [`Event::SessionEnded`]).
    session_ended: bool,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transport")
            .field("diagnostics", &self.diagnostics())
            .finish_non_exhaustive()
    }
}

impl Transport {
    /// - `key`: the 16-byte session key from `MOSH CONNECT`.
    pub fn new(key: &[u8], config: TransportConfig) -> Result<Self, OcbError> {
        Ok(Self {
            config,
            session: Session::new(key)?,
            events: VecDeque::new(),
            started: false,
            closed: false,
            finished: false,
            channel_state: ChannelState::Setup,
            send_seq: 0,
            user_states: vec![UserState {
                num: 0,
                events: Vec::new(),
                queued_at_ms: 0,
                sent: true,
                sent_at: 0,
            }],
            server_acked_user_num: 0,
            applied_host_num: 0,
            echo_ack: 0,
            assembler: FragmentAssembler::new(),
            next_instruction_id: 1,
            datagrams_received: 0,
            parse_failures: 0,
            gap_events: 0,
            flow_rebuilds: 0,
            cols: 80,
            rows: 24,
            saved_timestamp: None,
            expected_receiver_seq: 0,
            srtt: 0.0,
            rttvar: INITIAL_RTTVAR_MS,
            rtt_hit: false,
            roaming: false,
            ticking: false,
            next_ack_at: None,
            last_heard: None,
            watchdog_at: None,
            watchdog_armed: false,
            saw_any_datagram: false,
            liveness_at: None,
            restarts_at: Vec::new(),
            last_inbound_ms: 0,
            last_send_ms: None,
            rebuilds_since_inbound: 0,
            last_new_state_ms: 0,
            held_since: None,
            redraw: None,
            session_ended: false,
        })
    }

    // MARK: Output

    /// The next queued event, oldest first.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Every queued event, oldest first.
    pub fn drain_events(&mut self) -> std::collections::vec_deque::Drain<'_, Event> {
        self.events.drain(..)
    }

    /// The earliest armed timer, if any: call [`Transport::on_timer`] then.
    /// Once the channel has been ready this includes the send timer: when
    /// mosh's sender would next send (new input at frame rate, a
    /// retransmission, an ack), see `send_deadline`.
    pub fn next_deadline(&self) -> Option<u64> {
        [self.send_timer(), self.watchdog_at, self.liveness_at]
            .into_iter()
            .flatten()
            .chain(self.restarts_at.iter().copied())
            .min()
    }

    fn send_timer(&self) -> Option<u64> {
        if self.ticking && !self.closed {
            self.send_deadline()
        } else {
            None
        }
    }

    // MARK: Lifecycle

    /// Announces the window size as the first user event (mosh-server boots
    /// at 80×24 and only learns the size from a resize). Once only. The
    /// driver then opens its flow and reports [`ChannelState::Ready`].
    pub fn start(&mut self, cols: i32, rows: i32, now: u64) {
        if self.started {
            return;
        }
        self.started = true;
        self.cols = cols;
        self.rows = rows;
        self.append_user_state(
            vec![UserEvent::Resize {
                width: cols,
                height: rows,
            }],
            now,
        );
    }

    /// The flow's state changed.
    pub fn on_channel_state(&mut self, state: ChannelState, now: u64) {
        self.channel_state = state;
        match state {
            ChannelState::Ready => {
                if self.closed {
                    return;
                }
                // The first packet announces our address so the server can
                // push the screen; the send timer keeps NAT and acks alive.
                self.ticking = true;
                self.nudge(now);
                self.start_watchdog(now);
            }
            ChannelState::Cancelled => self.finish(),
            ChannelState::Failed => {
                // A failed UDP "connection" is no reason to end a mosh
                // session: a new flow a beat later, so a persistent failure
                // cannot spin. Not a restart: a failed NWConnection never
                // comes back, and `restart()` is ignored outside the waiting
                // state (nw_connection_restart), which left the flow (and its
                // failed receive) dead until the liveness check rebuilt it 9
                // to 12 s later.
                if self.closed {
                    self.finish();
                } else {
                    self.restarts_at
                        .push(now + self.config.failure_restart_delay_ms);
                }
            }
            ChannelState::Setup | ChannelState::Preparing | ChannelState::Waiting => {}
        }
    }

    /// A better network path appeared (`true`) or went away: the roam
    /// signal. Going roaming nudges a packet out the new path.
    pub fn on_better_path(&mut self, better: bool, now: u64) {
        if self.roaming == better {
            return;
        }
        self.roaming = better;
        self.publish_metrics();
        if better {
            self.nudge(now);
        }
    }

    /// The app returned to the foreground. `force` after a real suspension:
    /// replace the flow without diagnosing it and ask for a repaint.
    /// Otherwise replace a failed flow, restart a waiting one, and send a
    /// datagram now (whatever is due, an ack at least) so the server
    /// re-homes to us.
    pub fn resume(&mut self, force: bool, now: u64) {
        if self.closed {
            return;
        }
        if force {
            self.rebuild_flow();
            self.request_full_redraw(now);
            return;
        }
        match self.channel_state {
            ChannelState::Failed => self.rebuild_flow(),
            ChannelState::Waiting => self.events.push_back(Event::RestartFlow),
            _ => {}
        }
        self.nudge(now);
    }

    /// The return after a long absence (B619): a fresh flow, as
    /// [`Transport::resume`] with `force` makes, without asking for a
    /// repaint. mosh's own sync brings the screen up to date from the acked
    /// state; the repaint sent the remote program two size changes, so a
    /// full-screen one redrew twice, and nothing needed it. The person's
    /// Repaint ([`Transport::request_full_redraw`]) still asks for one.
    pub fn replace_flow(&mut self, now: u64) {
        if self.closed {
            return;
        }
        self.rebuild_flow();
        self.nudge(now);
    }

    /// Stops everything. The driver cancels its flow; waiters are released.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.ticking = false;
        self.next_ack_at = None;
        self.watchdog_at = None;
        self.liveness_at = None;
        self.restarts_at.clear();
        self.finish();
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Whether mosh-server ended the session (see [`Event::SessionEnded`]).
    pub fn session_ended(&self) -> bool {
        self.session_ended
    }

    // MARK: User input

    /// Queues keystrokes as new user states and sends them when their frame
    /// is due (see `next_send_at`). Returns the last
    /// state's number (what the server's echo ack names once it has echoed
    /// them), `None` for empty input.
    ///
    /// mosh-server inflates an instruction into a 4 MiB buffer
    /// (`compressor.cc`) and drops one that does not fit as a "Crypto
    /// exception", unacked; a flush sent the whole unacked stream as one
    /// instruction, so a paste over 4 MiB was refused by every flush until
    /// its keystrokes expired, then silently lost (live, mosh-server 1.4.0:
    /// 4,194,500 bytes, 13 refusals in 14 s, nothing arrived), and one just
    /// under it made mosh-server peak at 700 MiB. Input is split into states
    /// whose event fits the instruction budget, and `send_due` sends them in
    /// instructions under it, chained.
    pub fn send(&mut self, keys: &[u8], now: u64) -> Option<u64> {
        if keys.is_empty() {
            return None;
        }
        for chunk in keys.chunks(keystroke_chunk(self.config.instruction_budget)) {
            self.append_user_state(vec![UserEvent::Keystroke(chunk.to_vec())], now);
        }
        let num = self.newest_user_num();
        self.tick(now);
        Some(num)
    }

    /// Queues a resize (unless the size is unchanged) and sends it when its
    /// frame is due.
    pub fn resize(&mut self, cols: i32, rows: i32, now: u64) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        // A real size change repaints everything anyway.
        self.redraw = None;
        self.cols = cols;
        self.rows = rows;
        self.append_user_state(
            vec![UserEvent::Resize {
                width: cols,
                height: rows,
            }],
            now,
        );
        self.tick(now);
    }

    /// Asks the server to repaint every cell.
    ///
    /// Fix (full-redraw): mosh-server repaints the whole screen only when the
    /// framebuffer it renders differs in size from the one the client has.
    /// The Swift transport sent the narrower width and the real one as two
    /// resizes in one user state; the server applies both before it renders,
    /// sees the old size, and sends nothing (live, mosh-server 1.4.0: zero
    /// bytes). Here the narrower width goes alone; the frame the server
    /// renders at that width is acked but not shown, and only then is the
    /// real width restored, which makes the next frame a complete repaint.
    /// Frames between the two are hidden too (they would draw at the wrong
    /// width), for at most [`REDRAW_HIDE_LIMIT_MS`]. The applied state is
    /// never rewound.
    pub fn request_full_redraw(&mut self, now: u64) {
        if self.closed {
            return;
        }
        let width = if self.cols > 2 {
            self.cols - 1
        } else {
            self.cols + 1
        };
        let size = (width, self.rows);
        self.redraw = Some(Redraw::Narrowing(size));
        self.append_user_state(
            vec![UserEvent::Resize {
                width,
                height: self.rows,
            }],
            now,
        );
        self.tick(now);
    }

    /// Whether an applied frame is shown, advancing a redraw in progress.
    fn redraw_shows(&mut self, resize: Option<(i32, i32)>, now: u64) -> bool {
        match (self.redraw, resize) {
            (Some(Redraw::Narrowing(narrow)), Some(size)) if size == narrow => {
                self.redraw = Some(Redraw::Restoring { since: now });
                self.append_user_state(
                    vec![UserEvent::Resize {
                        width: self.cols,
                        height: self.rows,
                    }],
                    now,
                );
                false
            }
            (Some(Redraw::Restoring { since }), None)
                if now.saturating_sub(since) <= REDRAW_HIDE_LIMIT_MS =>
            {
                false
            }
            (Some(Redraw::Restoring { .. }), _) => {
                // The repaint at the real width (or a size we did not ask
                // for, or a server that never sent one): show from here on.
                self.redraw = None;
                true
            }
            _ => true,
        }
    }

    /// Newest user state queued (0 before any).
    pub fn newest_user_num(&self) -> u64 {
        self.user_states.last().map_or(0, |state| state.num)
    }

    /// Newest user state that carries input (keystrokes, a size): what a
    /// delivery waits for. An idle state carries nothing and isn't waited
    /// for: mosh-server acks it only when it next speaks (B522). The acked
    /// state when nothing newer carries input.
    pub fn newest_input_num(&self) -> u64 {
        self.user_states
            .iter()
            .rev()
            .find(|state| !state.events.is_empty())
            .map_or(self.server_acked_user_num, |state| state.num)
    }

    /// Newest user state the server acknowledged.
    pub fn server_acked_user_num(&self) -> u64 {
        self.server_acked_user_num
    }

    /// Whether `num` has landed: acked by the server, or the transport is
    /// closed (a waiter must never be stranded).
    pub fn is_delivered(&self, num: u64) -> bool {
        self.closed || num <= self.server_acked_user_num
    }

    fn append_user_state(&mut self, events: Vec<UserEvent>, now: u64) {
        let num = self.newest_user_num() + 1;
        self.user_states.push(UserState {
            num,
            events,
            queued_at_ms: now,
            sent: false,
            sent_at: 0,
        });
    }

    // MARK: Timers

    /// Fires every timer due at `now`, earliest first.
    pub fn on_timer(&mut self, now: u64) {
        // A send always moves the send deadline past `now` (see
        // `send_due`); fired once per call all the same, so a broken
        // invariant can never spin the caller.
        let mut sent = false;
        while let Some((timer, at)) = self.due_timer(now, sent) {
            match timer {
                Timer::Send => {
                    sent = true;
                    self.tick(now);
                }
                Timer::Watchdog => {
                    self.watchdog_at = None;
                    if !self.closed && !self.saw_any_datagram {
                        self.events.push_back(Event::ReturnPathDead);
                    }
                }
                Timer::Liveness => {
                    self.liveness_at = Some(now + self.config.liveness_interval_ms);
                    self.check_liveness(now);
                }
                Timer::Restart => {
                    if let Some(index) = self.restarts_at.iter().position(|&due| due == at) {
                        self.restarts_at.remove(index);
                    }
                    // Unless a new flow came since (a forced resume, the
                    // liveness check): that one has not failed.
                    if !self.closed && self.channel_state == ChannelState::Failed {
                        self.rebuild_flow();
                        self.nudge(now);
                    }
                }
            }
        }
    }

    fn due_timer(&self, now: u64, sent: bool) -> Option<(Timer, u64)> {
        [
            (Timer::Send, self.send_timer().filter(|_| !sent)),
            (Timer::Watchdog, self.watchdog_at),
            (Timer::Liveness, self.liveness_at),
            (Timer::Restart, self.restarts_at.iter().copied().min()),
        ]
        .into_iter()
        .filter_map(|(timer, at)| at.filter(|&at| at <= now).map(|at| (timer, at)))
        .min_by_key(|&(_, at)| at)
    }

    /// Arms the one-shot return-path watchdog: once per transport, and not
    /// after anything was heard.
    fn start_watchdog(&mut self, now: u64) {
        if self.saw_any_datagram || self.watchdog_armed {
            return;
        }
        self.watchdog_armed = true;
        self.watchdog_at = Some(now + self.config.return_path_deadline_ms);
    }

    /// The flow dying mid-session: we keep sending, nothing comes back, and
    /// the channel still claims ready. Rebuild it; after three rebuilds with
    /// still nothing inbound, report the return path dead (and keep trying).
    fn check_liveness(&mut self, now: u64) {
        if self.closed || !self.saw_any_datagram {
            return;
        }
        let Some(last_send) = self.last_send_ms else {
            return;
        };
        if now.saturating_sub(self.last_inbound_ms) <= self.config.liveness_deadline_ms
            || last_send <= self.last_inbound_ms
        {
            return;
        }
        self.rebuilds_since_inbound += 1;
        if self.rebuilds_since_inbound == 3 {
            self.events.push_back(Event::ReturnPathDead);
        }
        self.rebuild_flow();
        // Give the fresh flow something to announce itself with.
        self.nudge(now);
    }

    fn rebuild_flow(&mut self) {
        if self.closed {
            return;
        }
        self.flow_rebuilds += 1;
        self.channel_state = ChannelState::Setup;
        self.events.push_back(Event::RebuildFlow);
    }

    fn finish(&mut self) {
        if !self.finished {
            self.finished = true;
            self.events.push_back(Event::Finished);
        }
    }

    // MARK: Send path

    /// Sends whatever mosh's sender would send at `now`
    /// (`TransportSender::tick`): new input once its frame is due, unacked
    /// input again once its retransmission timeout ran out, the ack when
    /// its timer is up. What the send timer does (see
    /// [`Transport::next_deadline`]); input, resizes and received datagrams
    /// call it too.
    pub fn tick(&mut self, now: u64) {
        if self.closed || !self.send_deadline().is_some_and(|at| at <= now) {
            return;
        }
        self.send_due(now);
    }

    /// A datagram now, whatever else is due with it, an ack at least: the
    /// channel became ready, the path changed, the app came back. mosh's
    /// client has no such moment; this is its ack timer brought forward.
    fn nudge(&mut self, now: u64) {
        if self.closed {
            return;
        }
        self.next_ack_at = Some(now);
        self.tick(now);
    }

    /// When the next datagram is due: the earlier of the ack timer and
    /// `next_send_at`.
    fn send_deadline(&self) -> Option<u64> {
        [self.next_ack_at, self.next_send_at()]
            .into_iter()
            .flatten()
            .min()
    }

    /// mosh's `next_send_time` (`TransportSender::calculate_timers`): new
    /// input one send interval after the last datagram (`SEND_MINDELAY` is 0
    /// here: mosh-client's 1 ms gathers the bytes of one burst of reads,
    /// and [`Transport::send`] is handed whole inputs); input the server
    /// has not acked, when the oldest of it has gone a retransmission
    /// timeout plus `ACK_DELAY_MS` since it was last sent (then the server
    /// is no longer assumed to hold it, see `diff_base`), but only while
    /// the server was heard within `ACTIVE_RETRY_TIMEOUT_MS` (otherwise it
    /// goes with the 3 s ack); nothing while input is held.
    ///
    /// Every flush used to send every unacked state, and a flush ran for
    /// each datagram received and every second: a 4 MB paste made the
    /// Swift client send 88.6 MB and this one 33.4 MB (live, mosh-server
    /// 1.4.0), and keystrokes in flight went again with every frame of
    /// output. Unlike mosh, which refreshes only the newest state's time on
    /// a retransmission and so retries every send interval until the ack
    /// comes, a retransmission here restamps every state it carries, so the
    /// next waits a full timeout: one copy per timeout, not two per round
    /// trip.
    fn next_send_at(&self) -> Option<u64> {
        if self.held_since.is_some() {
            return None;
        }
        let acked = self.server_acked_user_num;
        let paced = self
            .last_send_ms
            .map_or(0, |at| at + self.send_interval_ms());
        let mut unacked = self.user_states.iter().filter(|state| state.num > acked);
        if unacked.clone().any(|state| !state.sent) {
            return Some(paced);
        }
        // mosh compares contents: when no unacked state carries an event,
        // the newest is the acked one, and an ack is all it takes.
        if !unacked.clone().any(|state| !state.events.is_empty()) {
            return None;
        }
        let oldest = unacked.next()?.sent_at;
        let at = (oldest + self.retransmit_after_ms()).max(paced);
        let heard = self.last_heard?;
        (heard + ACTIVE_RETRY_TIMEOUT_MS > at).then_some(at)
    }

    /// mosh's `assumed_receiver_state` (`update_assumed_receiver_state`),
    /// then its prophylactic resend (`attempt_prospective_resend_optimization`):
    /// the state to send from. The newest sent state is assumed to have
    /// arrived while every state after the acked one went out within a
    /// retransmission timeout plus `ACK_DELAY_MS`; otherwise the acked state
    /// is the only one the server is known to hold. Everything after the
    /// acked state goes again anyway when that is no longer than the new
    /// part, or small (a few keystrokes: under `RESEND_SMALL` bytes, at
    /// most `RESEND_SLACK` more).
    fn diff_base(&self, now: u64) -> u64 {
        let acked = self.server_acked_user_num;
        let limit = self.retransmit_after_ms();
        let mut assumed = acked;
        for state in self.user_states.iter().filter(|state| state.num > acked) {
            if !state.sent || now.saturating_sub(state.sent_at) >= limit {
                break;
            }
            assumed = state.num;
        }
        if assumed == acked {
            return acked;
        }
        let cost = |from: u64| -> usize {
            self.user_states
                .iter()
                .filter(|state| state.num > from)
                .flat_map(|state| &state.events)
                .map(UserEvent::encoded_len)
                .sum()
        };
        let (resend, incremental) = (cost(acked), cost(assumed));
        if resend <= incremental || (resend < RESEND_SMALL && resend - incremental < RESEND_SLACK) {
            acked
        } else {
            assumed
        }
    }

    /// mosh's retransmission timeout (`Connection::timeout`): SRTT plus four
    /// RTT variances, 50 ms to 1 s; plus `ACK_DELAY_MS`, the time the
    /// server may take to ack.
    fn retransmit_after_ms(&self) -> u64 {
        let (srtt, rttvar) = self.rtt_estimate();
        (srtt + 4.0 * rttvar).ceil().clamp(MIN_RTO_MS, MAX_RTO_MS) as u64 + ACK_DELAY_MS
    }

    /// mosh's frame interval (`TransportSender::send_interval`): half the
    /// SRTT, 20 to 250 ms.
    fn send_interval_ms(&self) -> u64 {
        let (srtt, _) = self.rtt_estimate();
        (srtt / 2.0)
            .ceil()
            .clamp(SEND_INTERVAL_MIN_MS, SEND_INTERVAL_MAX_MS) as u64
    }

    /// SRTT and RTTVAR for the timers: mosh's 1 s and 0.5 s until the first
    /// sample (the published SRTT stays 0 until then).
    fn rtt_estimate(&self) -> (f64, f64) {
        if self.rtt_hit {
            (self.srtt, self.rttvar)
        } else {
            (INITIAL_SRTT_MS, INITIAL_RTTVAR_MS)
        }
    }

    /// Sends from the state the server is assumed to hold (see `diff_base`)
    /// every user state after it (none while input is held), as
    /// instructions of at most `instruction_budget` bytes of events, each
    /// from the state the one before it reached: mosh-server applies them
    /// in turn (one too large for its 4 MiB buffer it drops, see
    /// [`Transport::send`]). A state larger than the budget alone (a resize
    /// under a tiny test budget) goes alone. With nothing to send, a plain
    /// ack: old and new the same state, so the server can only read its
    /// ack (it holds that state, or not the old one), never take the
    /// instruction for a state built without the events in flight.
    fn send_due(&mut self, now: u64) {
        let acked = self.server_acked_user_num;
        let held = self.hold_expired_input(now);
        if !held {
            self.idle_state(now);
        }
        let from = if held { acked } else { self.diff_base(now) };
        // Consecutive states, `budget` bytes of events at most each run.
        let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
        if !held {
            let budget = self.config.instruction_budget;
            let mut size = 0;
            for (index, state) in self.user_states.iter().enumerate() {
                if state.num <= from {
                    continue;
                }
                let cost: usize = state.events.iter().map(UserEvent::encoded_len).sum();
                match runs.last_mut() {
                    Some(run) if size + cost <= budget => {
                        run.end = index + 1;
                        size += cost;
                    }
                    _ => {
                        runs.push(index..index + 1);
                        size = cost;
                    }
                }
            }
        }
        if runs.is_empty() {
            self.send_instruction(from, from, Vec::new(), now);
        }
        let mut old = from;
        for run in runs {
            let mut events = Vec::new();
            for state in &mut self.user_states[run.clone()] {
                events.extend(state.events.iter().cloned());
                if !state.sent {
                    self.last_new_state_ms = now;
                }
                state.sent = true;
                state.sent_at = now;
            }
            let new = self.user_states[run.end - 1].num;
            self.send_instruction(old, new, events, now);
            old = new;
        }
        self.last_send_ms = Some(now);
        self.next_ack_at = Some(now + IDLE_STATE_INTERVAL_MS);
        // mosh's client retries at frame rate only within 10 s of hearing
        // the server, and before it ever has (`last_heard` starts at 0) only
        // every 3 s; here the window opens with the first datagram, so a
        // lost first one goes again within a retransmission timeout, as the
        // old 1 s heartbeat sent it again.
        self.last_heard.get_or_insert(now);
        if self.liveness_at.is_none() {
            self.liveness_at = Some(now + self.config.liveness_interval_ms);
        }
        if self.applied_host_num == SHUTDOWN_STATE {
            self.end_session();
        }
    }

    /// mosh-client's clean exit (`counterparty_shutdown_ack_sent`): once a
    /// datagram acking the server's shutdown state went out, the session is
    /// over. mosh-server quits on that ack (and after 16 tries or 10 s
    /// without it); the client used to apply the state, ack it, and then
    /// hear nothing: a frozen screen, flow rebuilds, and 27 s later the
    /// dead-return-path offer to switch to SSH (live, mosh-server 1.4.0,
    /// the remote command exiting).
    fn end_session(&mut self) {
        if self.session_ended {
            return;
        }
        self.session_ended = true;
        self.events.push_back(Event::SessionEnded);
        self.close();
    }

    /// Output arrived (or a diff we could not apply): ack it within mosh's
    /// `ACK_DELAY_MS`, with whatever else is due by then.
    fn ack_soon(&mut self, now: u64) {
        let at = now + ACK_DELAY_MS;
        self.next_ack_at = Some(self.next_ack_at.map_or(at, |due| due.min(at)));
    }

    /// One instruction from user state `old` to `new`, fragmented and
    /// sealed.
    fn send_instruction(&mut self, old: u64, new: u64, events: Vec<UserEvent>, now: u64) {
        let instruction = Instruction {
            old_num: old,
            new_num: new,
            ack_num: self.applied_host_num,
            // Fix (throwaway): the oldest state we may still diff from. The
            // Swift transport left it 0, so mosh-server never dropped a
            // received user state; past 1024 of them it accepts one new
            // state per 15 seconds and typing lags by up to 15 s. In a run
            // of instructions each names its own old state: applying it,
            // mosh-server holds that state, and lets the run's earlier
            // states go (a long paste is not kept whole in its memory). The
            // same goes for new input sent from the state the server is
            // assumed to hold (`diff_base`): mosh-server reads the throwaway
            // only once it found that state, and a later resend from the
            // acked state it then ignores waits for its ack, which names the
            // newer state, instead of growing its queue.
            throwaway_num: old,
            diff: if events.is_empty() {
                Vec::new()
            } else {
                encode_user_message(&events)
            },
            ..Instruction::default()
        };
        let compressed = compress::compress(&instruction.encode());
        let id = self.next_instruction_id;
        self.next_instruction_id += 1;
        let mut payload = Vec::with_capacity(self.config.fragment_mtu);
        let ranges: Vec<_> = fragment_ranges(compressed.len(), self.config.fragment_mtu).collect();
        let last = ranges.len() - 1;
        for (index, range) in ranges.into_iter().enumerate() {
            payload.clear();
            encode_fragment_into(
                id,
                index as u16,
                index == last,
                &compressed[range],
                &mut payload,
            );
            let timestamp = timestamp16(now);
            let reply = self.take_timestamp_reply(now);
            let packet = self.session.seal(
                self.send_seq,
                Direction::ToServer,
                timestamp,
                reply,
                &payload,
            );
            self.send_seq += 1;
            self.events.push_back(Event::Transmit(packet));
        }
        debug_assert!(FRAGMENT_HEADER_LEN < self.config.fragment_mtu);
    }

    /// mosh's client sends every empty ack as a new user state
    /// (`transportsender-impl.h`, `send_empty_ack`: `new_num = back + 1`, at
    /// least every `ACK_INTERVAL`, 3 s), and mosh-server dates its client by
    /// the newest user state it received (`mosh-server.cc`,
    /// `time_since_remote_state`): `MOSH_SERVER_NETWORK_TMOUT` (or SIGUSR1
    /// with `MOSH_SERVER_SIGNAL_TMOUT`) reaps a session whose client has sent
    /// none for that long, and a frame lost on the way back is retried at
    /// frame rate only within 10 s of one (`ACTIVE_RETRY_TIMEOUT`),
    /// afterwards at the server's 3 s ack tick. Heartbeats and acks only
    /// repeated the acked state (in the Swift transport too), so an idle but
    /// connected session was reaped (live: at 8 s with TMOUT=8) and a lost
    /// frame took 3 s to come back.
    ///
    /// So a datagram that goes out 3 s after the last new state went out
    /// adds an empty one: same events, a new number; idle, that is the 3 s
    /// ack. Not while input is held (its diff from the acked state would
    /// carry the held keystrokes) and not before [`Transport::start`]. (The
    /// acked state is always held, so the number after the newest is free:
    /// an ack is taken only for a state sent, see `receive`.)
    fn idle_state(&mut self, now: u64) {
        let Some(newest) = self.user_states.last() else {
            return;
        };
        if !self.started
            || !newest.sent
            || now.saturating_sub(self.last_new_state_ms) < IDLE_STATE_INTERVAL_MS
        {
            return;
        }
        let num = newest.num + 1;
        self.user_states.push(UserState {
            num,
            events: Vec::new(),
            queued_at_ms: now,
            sent: false,
            sent_at: 0,
        });
    }

    /// Fix (ttl-rewrite): drops what expired and was never sent, and says
    /// whether to hold user input back this flush.
    ///
    /// The Swift transport stripped expired keystrokes out of every unacked
    /// state on each flush. Those states were already sent, so when the
    /// server did receive them and only its acks were lost (a return path
    /// dropping packets for 10 s), the next state no longer extends the one
    /// the server holds. mosh-server diffs user states by position
    /// (`UserStream::diff_from`, whose asserts release builds compile out):
    /// it silently drops as many new events as were stripped, or reads past
    /// the end of the shorter state; builds with asserts abort.
    ///
    /// Here keystrokes past their TTL are dropped at once only from states
    /// never sent. When a sent one expires, flushes carry no user state past
    /// the acked one (they still ack host states and keep the session alive)
    /// until the server answers, see [`Transport::settle_held_input`].
    fn hold_expired_input(&mut self, now: u64) -> bool {
        let acked = self.server_acked_user_num;
        let ttl = self.config.keystroke_ttl_ms;
        let mut sent_expired = false;
        for state in self
            .user_states
            .iter_mut()
            .filter(|state| state.num > acked)
        {
            if now.saturating_sub(state.queued_at_ms) <= ttl || !state.has_keystroke() {
                continue;
            }
            if state.sent {
                sent_expired = true;
            } else {
                state.events.retain(|event| !event.is_keystroke());
            }
        }
        if self.held_since.is_none() && sent_expired {
            self.held_since = Some(now);
        }
        self.held_since.is_some()
    }

    /// Resolves held input on a datagram from the server; true when input
    /// flows again. An ack covering every sent keystroke state means they
    /// arrived after all: nothing is dropped. A datagram arriving
    /// `expiry_proof_delay_ms` or more after holding began, while some are
    /// still unacked, means those never arrived (none has been sent since,
    /// and mosh treats a longer transit as implausible): only then are the
    /// expired keystrokes dropped. State numbers stay, so no state the
    /// server may hold is ever rewritten.
    fn settle_held_input(&mut self, now: u64) -> bool {
        let Some(since) = self.held_since else {
            return false;
        };
        let acked = self.server_acked_user_num;
        let in_doubt = self
            .user_states
            .iter()
            .any(|state| state.num > acked && state.sent && state.has_keystroke());
        if in_doubt {
            if now < since + self.config.expiry_proof_delay_ms {
                return false;
            }
            let ttl = self.config.keystroke_ttl_ms;
            for state in self
                .user_states
                .iter_mut()
                .filter(|state| state.num > acked)
            {
                if now.saturating_sub(state.queued_at_ms) > ttl {
                    state.events.retain(|event| !event.is_keystroke());
                }
            }
        }
        self.held_since = None;
        true
    }

    // MARK: Receive path

    /// Handles one received datagram. After [`Transport::close`] datagrams
    /// are ignored (in Swift their frames went into a finished stream).
    pub fn receive(&mut self, datagram: &[u8], now: u64) {
        if self.closed {
            return;
        }
        self.datagrams_received += 1;
        // The return path is alive the moment anything lands, even a
        // datagram that does not decrypt.
        self.last_inbound_ms = now;
        self.rebuilds_since_inbound = 0;
        if !self.saw_any_datagram {
            self.saw_any_datagram = true;
            self.watchdog_at = None;
        }
        let Ok(message) = self.session.open(datagram) else {
            return;
        };
        if message.direction != Direction::ToClient {
            return;
        }
        // Roaming resolves the moment an authenticated reply lands.
        self.roaming = false;
        self.note_timestamps(&message, now);

        let instruction = Fragment::parse(&message.payload)
            .ok()
            .and_then(|fragment| self.assembler.receive(fragment))
            .and_then(|payload| compress::decompress(&payload).ok())
            .and_then(|bytes| Instruction::decode(&bytes).ok());
        let Some(instruction) = instruction else {
            self.publish_metrics();
            return;
        };

        // Fix (B105): an ack counts only for a state sent and still held,
        // as mosh's `process_acknowledgment_through` ignores one for a state
        // it does not hold. Taken for any number, an ack for a state never
        // sent (only a bad server sends one) became the acked number: every
        // state below it went, the next got a number no greater than it, and
        // input was never sent again while every waiter was told delivered.
        if instruction.ack_num > self.server_acked_user_num
            && self
                .user_states
                .iter()
                .any(|state| state.num == instruction.ack_num && state.sent)
        {
            self.server_acked_user_num = instruction.ack_num;
            let acked = self.server_acked_user_num;
            self.user_states
                .retain(|state| state.num >= acked || state.num == 0);
            self.events.push_back(Event::Acked(acked));
        }
        // The server answered: held input may flow again (see
        // `settle_held_input`); the tick below sends it when it is due.
        self.settle_held_input(now);

        // Output is acked within `ACK_DELAY_MS` (mosh's `set_data_ack`), not
        // by a datagram per frame. So is what does not apply: the server
        // re-diffs from the state we ack, the one state we hold.
        if instruction.new_num > self.applied_host_num
            && instruction.old_num == self.applied_host_num
        {
            if instruction.diff.is_empty() {
                // A pure ack or heartbeat: nothing to apply, safe to advance,
                // nothing to ack early (mosh acks only output early).
                self.applied_host_num = instruction.new_num;
                self.last_heard = Some(now);
            } else if let Ok(host) = HostMessage::decode(&instruction.diff) {
                // A diff that only moves the echo ack still lands: it is
                // what settles pending predictions.
                let echo_moved = host.echo_ack.unwrap_or(0) > self.echo_ack;
                if echo_moved {
                    self.echo_ack = host.echo_ack.unwrap_or(0);
                }
                let bytes = if self.redraw_shows(host.resize, now) {
                    host.host_bytes
                } else {
                    Vec::new()
                };
                if !bytes.is_empty() || echo_moved {
                    self.events.push_back(Event::HostFrame(HostFrame {
                        bytes,
                        echo_ack: self.echo_ack,
                    }));
                }
                self.applied_host_num = instruction.new_num;
                self.last_heard = Some(now);
                self.ack_soon(now);
            } else {
                // Never ack content that did not decode: mosh-server treats
                // an ack as delivered and never resends it. Acking the true
                // (old) state keeps it in the next diff.
                self.parse_failures += 1;
                self.ack_soon(now);
            }
        } else if instruction.new_num > self.applied_host_num {
            // Gap: ack what we have so the server re-diffs from it.
            self.gap_events += 1;
            self.ack_soon(now);
        }
        // mosh acks the shutdown state a frame after its last datagram
        // ("speed up shutdown sequence", `calculate_timers`), then quits.
        if self.applied_host_num == SHUTDOWN_STATE {
            let at = self
                .last_send_ms
                .map_or(now, |last| last + self.send_interval_ms());
            self.next_ack_at = Some(at);
        }
        // Whatever is due now: input a hold released, a redraw's restored
        // size, an ack that was due anyway.
        self.tick(now);
        self.publish_metrics();
    }

    /// Fix (timestamps): mosh's `network.cc` rules. 0xFFFF means "no
    /// timestamp" in both fields; a datagram older than one already seen
    /// (reordered or replayed) feeds neither the RTT nor the echo; the first
    /// sample seeds the estimate whatever its value. The Swift transport
    /// treated 0 as the marker instead, so each 0xFFFF reply (the server's
    /// "nothing to echo", sent whenever it has not heard from us within a
    /// second) became an RTT sample of `now & 0xFFFF + 1` ms whenever the
    /// clock's low 16 bits were under 5000: a bogus sample of up to 5 s in
    /// one 5-second window every 65.5 s.
    fn note_timestamps(&mut self, message: &Message, now: u64) {
        if message.seq < self.expected_receiver_seq {
            return;
        }
        self.expected_receiver_seq = message.seq + 1;
        if message.timestamp != NO_TIMESTAMP {
            self.saved_timestamp = Some((message.timestamp, now));
        }
        if message.timestamp_reply != NO_TIMESTAMP {
            let sample = f64::from(timestamp16(now).wrapping_sub(message.timestamp_reply));
            if sample < MAX_RTT_SAMPLE_MS {
                // RTTVAR from the SRTT before this sample, as mosh orders
                // them; it only feeds the retransmission timeout.
                if self.rtt_hit {
                    self.rttvar = 0.75 * self.rttvar + 0.25 * (self.srtt - sample).abs();
                    self.srtt = 0.875 * self.srtt + 0.125 * sample;
                } else {
                    self.srtt = sample;
                    self.rttvar = sample / 2.0;
                }
                self.rtt_hit = true;
            }
        }
    }

    /// Fix (timestamps): echo the server's last timestamp, advanced by how
    /// long we held it, once and only within a second (mosh
    /// `Connection::new_packet`); otherwise 0xFFFF. The Swift transport
    /// sent 0 before the first reply (a bogus first sample for the server
    /// 7.6% of the time) and then echoed the same timestamp forever.
    fn take_timestamp_reply(&mut self, now: u64) -> u16 {
        match self.saved_timestamp {
            Some((timestamp, at)) if now.saturating_sub(at) < TIMESTAMP_HOLD_LIMIT_MS => {
                self.saved_timestamp = None;
                timestamp.wrapping_add((now - at) as u16)
            }
            _ => NO_TIMESTAMP,
        }
    }

    // MARK: Metrics

    fn publish_metrics(&mut self) {
        self.events.push_back(Event::Metrics {
            srtt_ms: self.srtt,
            roaming: self.roaming,
        });
    }

    /// Smoothed RTT in milliseconds (0 before the first sample).
    pub fn srtt_ms(&self) -> f64 {
        self.srtt
    }

    pub fn roaming(&self) -> bool {
        self.roaming
    }

    pub fn diagnostics(&self) -> Diagnostics {
        Diagnostics {
            datagrams_received: self.datagrams_received,
            parse_failures: self.parse_failures,
            gap_events: self.gap_events,
            applied_host_num: self.applied_host_num,
            packets_sent: self.send_seq,
            last_user_num: self.newest_user_num(),
            server_acked_user_num: self.server_acked_user_num,
            flow_rebuilds: self.flow_rebuilds,
            input_held: self.held_since.is_some(),
            channel_state: self.channel_state,
        }
    }
}

/// The two phases of a full redraw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Redraw {
    /// The narrower size is on its way; waiting for a frame at it.
    Narrowing((i32, i32)),
    /// The real size is on its way; frames are hidden until its repaint.
    Restoring { since: u64 },
}

/// How long a redraw may hide frames while waiting for its repaint.
pub const REDRAW_HIDE_LIMIT_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Timer {
    Send,
    Watchdog,
    Liveness,
    Restart,
}

/// Keystroke bytes per user state: their event encodes in at most `budget`
/// bytes (three nested length-delimited fields, at most 18 bytes of tags and
/// lengths).
pub fn keystroke_chunk(budget: usize) -> usize {
    budget.saturating_sub(32).max(1)
}

/// mosh's 16-bit millisecond clock, which never reads 0xFFFF (that value
/// means "no timestamp"; mosh skips it to 0).
pub fn timestamp16(now: u64) -> u16 {
    match now as u16 {
        NO_TIMESTAMP => 0,
        ts => ts,
    }
}
