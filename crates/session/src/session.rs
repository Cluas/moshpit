//! One session's task: it owns the [`Lifecycle`], carries out its actions
//! (attempts, teardowns, probes, mosh resumes), starts the pane task for
//! each attempt it accepts, routes the panes' input, and turns what happens
//! into [`Event`]s. Everything about a session goes through this one task's
//! mailbox, in order, so no two decisions about it ever race.
//!
//! Input: every pane's byte channel has a small pump that forwards what the
//! view wrote to this mailbox (one queue, in order). Input is sent only
//! while the session is live; typed while it isn't, it is dropped and said
//! so ([`Discarded::NotLive`]: a stray tap during a reconnect must not
//! replay into a prompt afterwards). Live but with its pane not attached
//! yet (a reconnect's tmux attach still discovering) it waits, for that
//! pane only, and goes once the pane is there (B5) — never into the shell
//! underneath. herdr decides for itself (its client's input rule, B185).
//!
//! Content delivered on purpose — a notification's reply, a share
//! ([`Msg::Deliver`]) — is not typing: it waits out a reconnect (bounded,
//! [`DELIVERY_WAIT`]) and is answered once it went out or couldn't.
//!
//! mosh with tmux runs a control plane beside the mosh session: tmux's
//! `-CC` sidecar on a channel of the bootstrap's SSH connection, kept for
//! it ([`Control`]). Its connection dropping is not the session's: mosh
//! carries on and the sidecar is rebuilt on a fresh connection.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use moshpit_install::Multiplexer;
use moshpit_ssh::{
    BoxFuture as SshBoxFuture, DenyAll, HostKey, HostKeyPrompt, LinkFault, Process, PtyRequest,
    SignInQuestion, SignInQuestions, Ssh,
};
use moshpit_stream::{Input, InputEvent, Stream, StreamId};
use moshpit_tmux::boot::SidecarReturn;
use moshpit_tmux::controller::Config as TmuxConfig;
use moshpit_tmux::model::{Selection, Snapshot};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::attempt::{
    Attempt, Established, dial, dial_asking, pay_pin_debt, reap_command, reap_mosh_server,
};
use crate::event::{
    Degrade, Discarded, EndReason, Event, EventKind, Failure, PaneInfo, ResumeOutcome, SHELL_PANE,
    SessionId, SessionState,
};
use crate::herdr::{
    HerdrAction, HerdrCommand, HerdrLeg, HerdrNote, HerdrStart, HerdrView, socket_path_for,
    unanswered,
};
use crate::herdr_mosh::{
    HELD_SHOWN, HerdrMoshCommand, HerdrMoshLeg, HerdrMoshNote, HerdrMoshStart, PlaneConfig,
};
use crate::lifecycle::{Action, Lifecycle, Probe, Teardown, Transport};
use crate::mosh_udp::{self, MoshCommand, MoshLoop, MoshNote, MoshStart, Painted};
use crate::services::Services;
use crate::shell::{ShellCommand, ShellEnd, ShellLeg};
use crate::spec::{ConnectionSpec, Timing};
use crate::tmux_leg::{
    Handoff, SidecarStart, TmuxAction, TmuxCommand, TmuxLeg, TmuxNote, TmuxStart, TmuxView,
};

/// How long content delivered on purpose waits for the session to be live
/// and its pane there (a reconnect after the phone was asleep).
pub const DELIVERY_WAIT: Duration = Duration::from_secs(20);

/// The generation a side connection's sign-in questions go under (B634):
/// no attempt's, so the dial's hold on the person is not asked for.
const SIDE: u64 = u64::MAX;

/// How long a side connection is kept without a borrow (B634): long enough
/// that care, a borrow and a preview in a row ask the person for a second
/// factor once, not each.
const SIDE_IDLE_MS: u64 = 10 * 60 * 1000;

/// How long a disconnect waits for what it owes the host (a retired
/// attach's pins, a mosh-server to stop) before it says it is done.
const OWED_BOUND: Duration = Duration::from_secs(9);

/// What a session's task is told.
pub(crate) enum Msg {
    Connect,
    /// The person closed it: tear down, then the task ends.
    Disconnect(Option<oneshot::Sender<()>>),
    SwitchToSsh,
    Foreground,
    Background,
    /// Resume in place is on (`true`) or off from the next return (B365).
    ResumeInPlace(bool),
    NetworkChanged,
    /// The device was unlocked: a keychain locked before can be read.
    Unlocked,
    /// The tmux server's prefix, as the mosh sidecar read it (B362).
    MoshPrefix(String),
    Visible(bool),
    /// The connection was saved with another address, user or proxy
    /// (B329): the next connect goes there.
    Respec {
        target: moshpit_ssh::Target,
        proxy: Option<moshpit_ssh::Socks5Proxy>,
    },
    AnswerHostKey {
        prompt: u64,
        trust: bool,
    },
    /// The person's answer to a server's sign-in question (`None`:
    /// declined, B563).
    AnswerQuestion {
        prompt: u64,
        answer: Option<String>,
    },
    Tmux(TmuxAction),
    /// The person's herdr intent; `token` names its answer.
    Herdr {
        action: HerdrAction,
        token: u64,
    },
    /// The herdr task asked for under `token` is no longer wanted.
    HerdrCancel {
        token: u64,
    },
    /// A paste into a pane on screen: dropped when not live, like typing.
    Paste {
        pane: String,
        text: String,
    },
    /// Content for one pane on purpose (a notification's reply, a share).
    Deliver {
        pane: String,
        bytes: Vec<u8>,
        paste: bool,
        reply: oneshot::Sender<bool>,
    },
    Attempt {
        generation: u64,
        result: Result<Established, Failure>,
    },
    /// The attempt of `generation` reads its credential — Face ID, the
    /// passphrase card may be up — (`true`), or is done reading (B395).
    OnPerson {
        generation: u64,
        waiting: bool,
    },
    Ask {
        generation: u64,
        host: String,
        port: u16,
        fingerprint: String,
        previous: Option<String>,
        /// The older algorithm the key came by (B564).
        older_algorithm: Option<&'static str>,
        /// An earlier build stored two different keys for the host (B725).
        conflicting: bool,
        reply: oneshot::Sender<bool>,
    },
    /// The attempt of `generation` asks the person what the server asked
    /// at sign-in (B563).
    Question {
        generation: u64,
        host: String,
        question: SignInQuestion,
        reply: oneshot::Sender<Option<String>>,
    },
    /// Probe `probe` of `generation` answered (B394: only the probe out
    /// counts).
    Probe {
        generation: u64,
        probe: u64,
        result: Probe,
    },
    Shell {
        generation: u64,
        end: ShellEnd,
    },
    TmuxNote {
        generation: u64,
        note: TmuxNote,
    },
    Mosh {
        generation: u64,
        note: MoshNote,
    },
    HerdrNote {
        generation: u64,
        note: HerdrNote,
    },
    /// What herdr's control plane beside the mosh leg of `generation` says.
    HerdrMosh {
        generation: u64,
        note: HerdrMoshNote,
    },
    /// A connection for herdr's control plane over mosh (redialled).
    HerdrControlUp {
        epoch: u64,
        result: Result<Ssh, Failure>,
    },
    /// Time to redial herdr's control plane of `epoch`.
    HerdrControlRetry {
        epoch: u64,
    },
    /// What mosh's `-CC` sidecar of control-plane epoch `epoch` says.
    Sidecar {
        epoch: u64,
        note: TmuxNote,
    },
    /// The control plane's connection (redialled) and its terminal.
    ControlUp {
        epoch: u64,
        result: Result<(Ssh, Process), Failure>,
    },
    /// Time to redial the control plane of epoch `epoch`.
    ControlRetry {
        epoch: u64,
    },
    /// The control plane of `epoch` answered the check, or didn't (B401).
    ControlChecked {
        epoch: u64,
        alive: bool,
    },
    /// The tmux commands held since hold `hold` began waited long enough.
    ControlHoldExpired {
        hold: u64,
    },
    /// The tmux commands held for the next attach since hold `hold` began
    /// waited long enough (B483).
    TmuxHoldExpired {
        hold: u64,
    },
    /// Paste `paste` to `dest` is in the tty by mosh-server's ack
    /// (`landed`, B525), or never will be: its loop ended, or herdr's
    /// plane dropped it (B687).
    PasteLanded {
        dest: Dest,
        paste: u64,
        landed: bool,
    },
    /// What a dropped sidecar of `epoch` owed the server, for the next.
    SidecarHandoff {
        epoch: u64,
        handoff: Handoff,
    },
    /// The phone's renderer: mosh-server's child, tmux's client.
    RendererPid {
        generation: u64,
        pid: i64,
    },
    Input {
        stream: StreamId,
        event: InputEvent,
    },
    /// Answer once what was asked of the server so far has been written
    /// (tmux's pins handed back on the way to the background).
    Flush(oneshot::Sender<()>),
    /// A debug build breaks the live connection's link (mosh with tmux:
    /// its control plane's).
    Fault(LinkFault),
    /// A test ends the mosh loop as if its transport had died.
    MoshLoss,
    /// The person's Repaint: mosh-server redraws the whole screen (B338).
    Repaint,
    /// Someone wants an SSH connection to this session's host (SFTP, an
    /// exec): the session's own, or a side connection it keeps.
    BorrowSsh(oneshot::Sender<Result<Ssh, Failure>>),
    /// The side connection a borrow dialled.
    SideSsh {
        result: Result<Ssh, Failure>,
    },
}

/// What the UI can read about a session at any time (after an event says
/// it changed).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionView {
    pub state: SessionState,
    /// The current attach's panes (a session without a multiplexer has one).
    pub attach: u64,
    pub panes: Vec<PaneInfo>,
    /// tmux's tree, when tmux runs (over SSH, or mosh's sidecar).
    pub tmux: Option<Snapshot>,
    /// The tree and what the chrome draws around it, when tmux runs.
    pub tmux_view: Option<TmuxView>,
    /// herdr's tree and what is said around it, when herdr runs.
    pub herdr: Option<HerdrView>,
    pub degrade: Option<Degrade>,
    pub transport: Option<Transport>,
    /// mosh hears nothing back: the banner offering SSH.
    pub return_path_dead: bool,
    pub attach_stalled: bool,
    pub mosh_srtt_ms: Option<f64>,
    /// mosh is roaming, or what was typed has waited over a second for
    /// mosh-server's acknowledgement of what went before it
    /// ([`Self::input_held`]): the pill's amber and "mosh is reconnecting"
    /// (B689).
    pub mosh_roaming: bool,
    /// Typing (a Return behind a paste, a key or typing behind the herdr
    /// plane's barrier) has waited over a second, unbounded, for
    /// mosh-server's acknowledgement of what was sent before it (B689):
    /// shown as mosh's reconnecting state, never a pane that looks fine
    /// while nothing happens.
    pub input_held: bool,
    /// mosh's protocol counters (the pill's long-press popover).
    pub mosh_diagnostics: Option<moshpit_mosh::Diagnostics>,
    /// The mosh-server this session started (for diagnostics and the
    /// bench's cleanup).
    pub mosh_server_pid: Option<i64>,
    /// When the session first went live, in Unix seconds: kept through
    /// reconnects (the card's "3h" counts from it), gone once it ends or
    /// fails. Both platforms kept this themselves (Swift's
    /// `SessionMetrics.connectedAt`, Android's `Conn`).
    pub connected_at: Option<f64>,
    /// When it last stopped being live, in Unix seconds ("Lost 25m ago"):
    /// set on the way out of connected — a reconnect, an end, a failure —
    /// and gone once it is live again. `None` for a session that never went
    /// live (its failure says why) or that is live now.
    pub lost_at: Option<f64>,
}

impl SessionView {
    /// `state` arrives at `unix_now`: [`Self::connected_at`] and
    /// [`Self::lost_at`] follow it.
    pub(crate) fn stamp(&mut self, state: &SessionState, unix_now: f64) {
        let was_live = self.state == SessionState::Connected;
        match state {
            SessionState::Connected => {
                self.connected_at.get_or_insert(unix_now);
                self.lost_at = None;
            }
            SessionState::Ended(_) | SessionState::Failed(_) => {
                self.connected_at = None;
                if was_live {
                    self.lost_at = Some(unix_now);
                }
            }
            SessionState::Connecting | SessionState::Reconnecting => {
                if was_live {
                    self.lost_at = Some(unix_now);
                }
            }
        }
    }

    /// A session's view before anything happened: connecting, nothing
    /// known yet.
    pub fn initial() -> Self {
        Self {
            state: SessionState::Connecting,
            attach: 0,
            panes: Vec::new(),
            tmux: None,
            tmux_view: None,
            herdr: None,
            degrade: None,
            transport: None,
            return_path_dead: false,
            attach_stalled: false,
            mosh_srtt_ms: None,
            mosh_roaming: false,
            input_held: false,
            mosh_diagnostics: None,
            mosh_server_pid: None,
            connected_at: None,
            lost_at: None,
        }
    }
}

/// The session's clock: milliseconds since it was made, plus an offset (the
/// lifecycle's arithmetic never sees 0), with the time the device slept
/// counted (B611). `Instant` doesn't count it on Apple or Linux: a phone
/// locked for minutes slept most of them, and came back "away" a few
/// seconds — no forced reconnect, no fresh mosh flow, a frozen dial looking
/// young. Never goes back.
#[derive(Clone)]
pub(crate) struct Clock {
    epoch: Instant,
    /// The device's sleep so far, ms (the platform's, or a test's).
    slept: fn() -> u64,
    slept_at_epoch: u64,
    /// The last time read: what it says never goes back.
    last: Arc<AtomicU64>,
}

const CLOCK_OFFSET_MS: u64 = 1_000_000;

impl Clock {
    pub fn new() -> Self {
        Self::with_sleep(system_slept_ms)
    }

    pub fn with_sleep(slept: fn() -> u64) -> Self {
        Self {
            epoch: Instant::now(),
            slept,
            slept_at_epoch: slept(),
            last: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn now(&self) -> u64 {
        let slept = (self.slept)().saturating_sub(self.slept_at_epoch);
        let now = self.epoch.elapsed().as_millis() as u64 + slept + CLOCK_OFFSET_MS;
        self.last.fetch_max(now, Ordering::Relaxed).max(now)
    }

    /// When `ms` (this clock) comes, as an `Instant` to sleep until: by how
    /// far off it is now, `Instant` not counting the sleep.
    fn instant(&self, ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms.saturating_sub(self.now()))
    }
}

/// How long the device has slept since it booted, ms: a monotonic clock
/// that counts sleep less the one `Instant` reads, which doesn't (Apple:
/// `CLOCK_MONOTONIC` and `CLOCK_UPTIME_RAW`; Linux and Android:
/// `CLOCK_BOOTTIME` and `CLOCK_MONOTONIC`). The one without sleep is read
/// first: the gap can't come out short.
fn system_slept_ms() -> u64 {
    #[cfg(target_vendor = "apple")]
    let (without, with) = (libc::CLOCK_UPTIME_RAW, libc::CLOCK_MONOTONIC);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let (without, with) = (libc::CLOCK_MONOTONIC, libc::CLOCK_BOOTTIME);
    #[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
    {
        let read = |id| {
            let mut ts = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            // SAFETY: `ts` is a valid, writable timespec.
            if unsafe { libc::clock_gettime(id, &mut ts) } != 0 {
                return None;
            }
            Some(ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64)
        };
        match (read(without), read(with)) {
            (Some(a), Some(b)) => b.saturating_sub(a) / 1_000_000,
            _ => 0,
        }
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
    {
        0
    }
}

/// The in-place return's question (`Timing::resume_in_place`): asked of
/// the transport of `generation` at `asked_at` (the session's clock, ms),
/// after `away_ms` in the background; its answer and when it came.
struct ResumeQuestion {
    generation: u64,
    /// The probe that asks it: only its answer is the question's.
    probe: u64,
    asked_at: u64,
    away_ms: u64,
    through_tmux: bool,
    answer: Option<(Probe, u64)>,
}

enum Leg {
    Shell {
        generation: u64,
        ssh: Ssh,
        shell: ShellLeg,
    },
    Tmux {
        generation: u64,
        ssh: Ssh,
        tmux: TmuxLeg,
    },
    Mosh {
        generation: u64,
        mosh: MoshLoop,
        server_pid: Option<i64>,
        first_contact: bool,
        return_path_reported: bool,
        reaped: bool,
    },
    Herdr {
        generation: u64,
        ssh: Ssh,
        herdr: HerdrLeg,
    },
}

impl Leg {
    fn generation(&self) -> u64 {
        match self {
            Leg::Shell { generation, .. }
            | Leg::Tmux { generation, .. }
            | Leg::Mosh { generation, .. }
            | Leg::Herdr { generation, .. } => *generation,
        }
    }

    fn ssh(&self) -> Option<&Ssh> {
        match self {
            Leg::Shell { ssh, .. } | Leg::Tmux { ssh, .. } | Leg::Herdr { ssh, .. } => Some(ssh),
            Leg::Mosh { .. } => None,
        }
    }
}

/// mosh + tmux's control plane: the `-CC` sidecar on an SSH connection of
/// its own (first the bootstrap's, kept), for the mosh leg of `generation`.
struct Control {
    generation: u64,
    /// Bumped for every sidecar: what an older one says is nobody's.
    epoch: u64,
    ssh: Option<Ssh>,
    sidecar: Option<TmuxLeg>,
    /// A connection or terminal for the next sidecar is on its way.
    dialling: bool,
    /// Redials in a row that didn't bring a sidecar up (the backoff).
    failures: u32,
    /// What the last sidecar knew, for the next.
    preferred: Option<SidecarReturn>,
    handoff: Handoff,
    renderer_pid: Option<i64>,
    /// The mosh grid the renderer's window is pinned to.
    grid: (u16, u16),
    /// The windows before the renderer sized them (B270), for the first
    /// sidecar.
    layouts_before_renderer: Option<String>,
    /// The sidecar can take commands: attached, or its server answered that
    /// it has no session (the first one is made over exec, B228).
    ready: bool,
    /// A check that the connection still answers is out (B401).
    checking: bool,
    /// The person's commands wait for that check: it was asked for on the
    /// foreground return or a path change, not the keepalive's tick.
    holding: bool,
    /// The person's tmux commands while the control plane is in doubt (a
    /// check out, a redial on its way, a sidecar not ready yet), in the
    /// order given: sent once it answers or the next sidecar is attached,
    /// dropped past [`CONTROL_HOLD`].
    held: Vec<TmuxAction>,
    /// Which hold the expiry timer is for.
    hold: u64,
    /// Commands were dropped while the plane was down: said once the next
    /// sidecar is ready.
    dropped: bool,
}

/// How long a check of the control plane's connection may take before it
/// counts as gone: one round trip, with room for a slow path (a false
/// alarm costs a control-plane redial; mosh carries on).
const CONTROL_CHECK: Duration = Duration::from_millis(1_500);

/// How long the person's tmux commands wait for a control plane in doubt.
const CONTROL_HOLD: Duration = Duration::from_secs(5);

/// How long typing waits behind a paste to the same place (B485). A
/// program taking a paste in may drop a key that comes with it: Claude
/// Code, making an image's path an attachment, drops a Return that reaches
/// it with the paste, so the image panel's Send (the path, then Return)
/// only pasted. Measured there: lost back to back, kept 5 ms behind; this
/// leaves room for a slower host.
const PASTE_SETTLE: Duration = Duration::from_millis(150);

/// Where typing goes: the session's shell (plain SSH, mosh) or a
/// multiplexer's pane.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Dest {
    Shell,
    Pane(String),
}

/// Typing held behind the paste just sent to a place (B485).
struct Settling {
    /// The paste's number (an older paste's ack lands nothing of this one).
    paste: u64,
    /// When the paste went (the session clock's ms).
    since: u64,
    /// When the paste has had its time (the session clock's ms).
    until: u64,
    /// The paste has landed: mosh-server acked its state (B525), however
    /// long that takes (B687: a paste state lost on the way goes again as
    /// one diff with what was typed after it, and mosh-server writes them
    /// together; the 2 s this was bounded by let a slow path's Return
    /// join it). Always, elsewhere.
    landed: bool,
    /// What came after it, in order.
    queued: Vec<Queued>,
}

/// One thing waiting behind a paste.
enum Queued {
    /// Typing, with where it ends in the shell's channel (mosh's echo
    /// acknowledgement counts in it).
    Typed(StreamId, Typed, u64),
    /// A size change: it lands between the bytes it came between (V31,
    /// B528).
    Resize(StreamId, u16, u16),
}

/// One thing given while the in-place return waits for its answer.
enum Kept {
    /// Typing, and whether it was the shortcut bar's.
    Typed(StreamId, Typed, bool),
    /// A size change given after typing held for the same stream: it waits
    /// behind it (V31, B529).
    Resize(StreamId, u16, u16),
}

impl Kept {
    fn stream(&self) -> StreamId {
        match self {
            Kept::Typed(stream, ..) | Kept::Resize(stream, ..) => *stream,
        }
    }
}

/// herdr's control plane beside the mosh leg of `generation`: the plane
/// (W2b's client and the raw-attach renderer) on a connection of its own —
/// first the bootstrap's, kept; redialled when it drops, while mosh carries
/// on.
struct HerdrMoshControl {
    generation: u64,
    leg: HerdrMoshLeg,
    ssh: Option<Ssh>,
    /// Bumped for every redial: an older one's connection is nobody's.
    epoch: u64,
    dialling: bool,
    /// Redials in a row that brought no connection (the backoff).
    failures: u32,
}

/// What a view sent a pane, in the order sent: typed bytes, or a paste
/// (B361), which rides the same stream so it keeps its place among the
/// keys and is held, dropped and routed with them.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Typed {
    Bytes(Vec<u8>),
    Paste(String),
}

impl Typed {
    /// Its size, as [`EventKind::InputDiscarded`] counts it.
    fn len(&self) -> usize {
        match self {
            Typed::Bytes(bytes) => bytes.len(),
            Typed::Paste(text) => text.len(),
        }
    }

    /// As bytes for a plain shell or mosh: a paste with the markers, the
    /// text's own taken out (B115).
    fn into_bytes(self) -> Vec<u8> {
        match self {
            Typed::Bytes(bytes) => bytes,
            Typed::Paste(text) => moshpit_herdr::frame::bracketed_paste(&text),
        }
    }

    /// As herdr's client takes it: a paste as one bracketed paste herdr
    /// fits to the pane's real mode (B43).
    fn for_herdr(self, pane: String, stream: StreamId) -> HerdrCommand {
        match self {
            Typed::Bytes(bytes) => HerdrCommand::Input {
                pane,
                stream,
                bytes,
            },
            Typed::Paste(text) => HerdrCommand::Paste { pane, text },
        }
    }
}

/// A multiplexer pane's byte channel and the pump that forwards its input.
struct PaneChannel {
    pane: String,
    attach: u64,
    /// Held, never read: while the core's end lives the view's writes are
    /// taken (a dropped one refuses them), which is what lets input typed
    /// into the last frame wait for its pane.
    #[allow(dead_code)]
    stream: Arc<Stream>,
    pump: JoinHandle<()>,
}

impl Drop for PaneChannel {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

/// The byte channel of a session without a multiplexer: one for the
/// session's whole life, so its terminal keeps its scrollback across
/// reconnects.
struct ShellChannel {
    stream: Arc<Stream>,
    pump: JoinHandle<()>,
    /// A leg has written into it: the next one starts with a CAN.
    used: bool,
    /// Bytes the view has written into it so far: the offsets mosh's echo
    /// acknowledgement is told in.
    written: u64,
}

impl Drop for ShellChannel {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

/// Content waiting for its session to be live and its pane there.
struct Delivery {
    pane: String,
    bytes: Vec<u8>,
    paste: bool,
    reply: oneshot::Sender<bool>,
    /// On the session's clock.
    deadline: u64,
}

pub(crate) struct Actor {
    pub id: SessionId,
    pub spec: ConnectionSpec,
    /// Where the connection was saved to point since this session started
    /// (B329): taken up at the next dial — a reconnect after a drop, or the
    /// person's — never by the transport live now.
    pub saved: Option<(moshpit_ssh::Target, Option<moshpit_ssh::Socks5Proxy>)>,
    pub timing: Timing,
    pub services: Services,
    pub life: Lifecycle,
    pub clock: Clock,
    pub mailbox: mpsc::UnboundedReceiver<Msg>,
    pub me: mpsc::UnboundedSender<Msg>,
    pub events: mpsc::UnboundedSender<Event>,
    pub view: Arc<Mutex<SessionView>>,

    pub attempt: Option<(u64, JoinHandle<()>)>,
    pub prompts: HashMap<u64, (u64, oneshot::Sender<bool>)>,
    /// The server's sign-in questions out with the person, numbered with
    /// the host-key prompts (B563).
    pub questions: HashMap<u64, (u64, oneshot::Sender<Option<String>>)>,
    pub next_prompt: u64,
    leg: Option<Leg>,
    control: Option<Control>,
    herdr_mosh: Option<HerdrMoshControl>,
    next_epoch: u64,
    pub inbound: Arc<AtomicU64>,
    shell: Option<ShellChannel>,
    attach: u64,
    panes: BTreeMap<StreamId, PaneChannel>,
    current: Vec<PaneInfo>,
    /// Input waiting for its pane, by pane, in the order typed.
    pending: BTreeMap<String, Vec<(StreamId, Typed)>>,
    /// Input typed (or pasted) while the in-place return waits for the
    /// live transport's answer ([`Lifecycle::resume_pending`]), in order:
    /// sent once it answered, dropped as typed while not live if it didn't.
    held: Vec<Kept>,
    /// mosh with tmux: the prefix a scroll's copy-mode keys start with
    /// (`C-b` until the sidecar has read the server's), and whether it was
    /// asked (B362).
    mosh_prefix: Vec<u8>,
    mosh_prefix_asked: bool,
    /// mosh with tmux: the copy-mode of the pane on screen last told to the
    /// mosh loop (B614); `None` for a loop told nothing yet.
    mosh_copy_mode_sent: Option<bool>,
    /// The person's last herdr pane pick the server isn't seen on yet: a
    /// connection that died under it (a notification tapped after a long
    /// absence picks before the return finds the socket dead) would lose
    /// it, so the next herdr leg makes it again (B397).
    herdr_pick: Option<String>,
    /// The person's tmux pick made while no tmux leg could carry it (the
    /// connection being redialled): the next attach lands there instead of
    /// where the last one was (B405).
    tmux_pick: Option<Selection>,
    /// tmux commands the person gave while no attach could carry them (the
    /// connection being redialled, or a return still asking whether its
    /// socket lived), for the next attach, in order (B483).
    tmux_held: Vec<TmuxAction>,
    /// Bumped for every hold that starts: an older one's bound is moot.
    tmux_hold: u64,
    /// Commands held past their bound were dropped: said once an attach
    /// carries commands again.
    tmux_dropped: bool,
    /// Typing behind a paste, per place, until the paste has had its time
    /// (B485).
    settling: BTreeMap<Dest, Settling>,
    /// The last paste's number.
    pastes: u64,
    /// herdr's plane holds typing behind an unacknowledged barrier past
    /// [`HELD_SHOWN`] (B689).
    plane_held: bool,
    /// mosh's own roaming flag, as its metrics last said.
    transport_roaming: bool,
    /// The in-place return's question, until it is settled.
    resume_question: Option<ResumeQuestion>,
    deliveries: Vec<Delivery>,
    /// The size each view last reported.
    grids: BTreeMap<StreamId, (u16, u16)>,
    /// The grid the next terminal starts at.
    grid: (u16, u16),
    /// The grid the dial under way opened its terminal at: a size given
    /// since is sent once its leg is up (B616).
    dialed_grid: (u16, u16),
    /// Where a mosh-rendered tmux pane's wheel goes: the cell the scroll
    /// under way began on (B578).
    mosh_wheel_aim: Option<(i64, i64)>,
    handoff: Handoff,
    visible: bool,
    done: Option<oneshot::Sender<()>>,
    /// The side connection borrowers get while the session has no SSH of
    /// its own (plain mosh), and who waits for the one being dialled.
    side: Option<Ssh>,
    side_waiting: Vec<oneshot::Sender<Result<Ssh, Failure>>>,
    /// When the side connection goes for want of a borrow (B634).
    side_idle_until: Option<u64>,
    /// The person said no to a side connection's question: not asked
    /// again in this session, and the side feature fails as it did (B634).
    side_declined: bool,
    /// Borrowers waiting for mosh + tmux's control plane to come up on the
    /// bootstrap's connection, rather than have a side connection dialled
    /// beside it (`borrow_ssh`).
    control_waiting: Vec<oneshot::Sender<Result<Ssh, Failure>>>,
    /// mosh-servers a reconnect left behind (B195): stopped over the next
    /// connection, or at the end.
    orphans: Vec<i64>,
    /// The mosh-server of a mosh + tmux session the person ended, when its
    /// renderer couldn't be taken out of the session at the teardown (the
    /// control plane's connection gone): the pins it owes are paid with the
    /// renderer detached first, and the server stopped after (B270).
    debt_renderer_server: Option<i64>,
    /// What the session owes the host and is doing about it (stopping a
    /// mosh-server, handing pins back): the disconnect waits for them.
    owed: Vec<JoinHandle<()>>,
}

impl Actor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: SessionId,
        spec: ConnectionSpec,
        timing: Timing,
        services: Services,
        foreground: bool,
        mailbox: mpsc::UnboundedReceiver<Msg>,
        me: mpsc::UnboundedSender<Msg>,
        events: mpsc::UnboundedSender<Event>,
        view: Arc<Mutex<SessionView>>,
    ) -> Self {
        let clock = Clock::new();
        let life = Lifecycle::new(timing.clone(), spec.protocol, foreground, clock.now());
        let grid = spec.initial_grid;
        Self {
            id,
            spec,
            saved: None,
            timing,
            services,
            life,
            clock,
            mailbox,
            me,
            events,
            view,
            attempt: None,
            prompts: HashMap::new(),
            questions: HashMap::new(),
            next_prompt: 0,
            leg: None,
            control: None,
            herdr_mosh: None,
            next_epoch: 0,
            inbound: Arc::new(AtomicU64::new(0)),
            shell: None,
            attach: 0,
            panes: BTreeMap::new(),
            current: Vec::new(),
            pending: BTreeMap::new(),
            held: Vec::new(),
            mosh_prefix: vec![0x02],
            mosh_prefix_asked: false,
            mosh_copy_mode_sent: None,
            herdr_pick: None,
            tmux_pick: None,
            tmux_held: Vec::new(),
            tmux_hold: 0,
            tmux_dropped: false,
            settling: BTreeMap::new(),
            pastes: 0,
            plane_held: false,
            transport_roaming: false,
            resume_question: None,
            deliveries: Vec::new(),
            grids: BTreeMap::new(),
            grid,
            dialed_grid: grid,
            mosh_wheel_aim: None,
            handoff: Handoff::default(),
            visible: false,
            done: None,
            side: None,
            side_waiting: Vec::new(),
            side_idle_until: None,
            side_declined: false,
            control_waiting: Vec::new(),
            orphans: Vec::new(),
            debt_renderer_server: None,
            owed: Vec::new(),
        }
    }

    fn now(&self) -> u64 {
        self.clock.now()
    }

    /// Something the session owes the host and is doing about it: the
    /// disconnect waits for it (bounded) before it says it is done.
    fn owe(&mut self, work: impl std::future::Future<Output = ()> + Send + 'static) {
        self.owed.retain(|task| !task.is_finished());
        self.owed.push(tokio::spawn(work));
    }

    /// The windows are ours to size: the terminal is on screen with the
    /// app in front.
    fn pinned(&self) -> bool {
        self.visible && self.life.is_foreground()
    }

    pub async fn run(mut self) {
        let mut stopping = false;
        loop {
            let stamp = self.inbound.load(Ordering::Relaxed);
            if stamp > 0 {
                self.life.inbound(stamp);
            }
            if self.resume_question.is_some() && !self.life.resume_pending() {
                self.settle_question();
            }
            if !self.held.is_empty() && !self.life.resume_pending() {
                self.release_held();
            }
            if (!self.tmux_held.is_empty() || self.tmux_dropped) && !self.life.resume_pending() {
                self.release_tmux_held();
            }
            self.act().await;
            if stopping {
                break;
            }
            let next = [
                self.life.next_deadline(),
                self.deliveries.iter().map(|d| d.deadline).min(),
                self.settling
                    .values()
                    .filter(|s| s.landed)
                    .map(|s| s.until)
                    .min(),
                self.held_due(),
                self.side_idle_until,
            ]
            .into_iter()
            .flatten()
            .min();
            let deadline = next.map(|at| self.clock.instant(at));
            tokio::select! {
                message = self.mailbox.recv() => match message {
                    Some(Msg::Disconnect(done)) => {
                        let now = self.now();
                        self.life.disconnect(now);
                        self.done = done;
                        stopping = true;
                    }
                    Some(message) => self.handle(message).await,
                    None => {
                        let now = self.now();
                        self.life.disconnect(now);
                        stopping = true;
                    }
                },
                () = sleep_until(deadline) => {
                    let now = self.now();
                    self.life.tick(now);
                    self.expire_deliveries(now);
                    self.release_settled(now);
                    self.release_idle_side(now);
                }
            }
        }
        self.finish().await;
    }

    /// The session is over: let go of everything, and pay what is owed to
    /// the host — a retired attach's pins (B203), the mosh-servers it
    /// started — BEFORE the disconnect says it is done: the platform holds
    /// the process awake only until then.
    async fn finish(&mut self) {
        if let Some((_, task)) = self.attempt.take() {
            task.abort();
        }
        // A tmux controller retired for a reconnect that never landed on
        // tmux again still owes the server its pins.
        let renderer_server = self.debt_renderer_server.take();
        let (services, spec, timing) = (
            self.services.clone(),
            self.spec.clone(),
            self.timing.clone(),
        );
        if let Some(ledger) = self.handoff.ledger.take()
            && (!ledger.resized_windows.is_empty()
                || !ledger.pristine_layouts.is_empty()
                || ledger.hidden_status.is_some())
        {
            // The renderer out first, the windows back, and only then its
            // server stopped (B270): stopped first or meanwhile, its tmux
            // client was still sizing the windows when they went back.
            self.owe(async move {
                pay_pin_debt(
                    services.clone(),
                    spec.clone(),
                    timing.clone(),
                    ledger,
                    renderer_server,
                )
                .await;
                if let Some(pid) = renderer_server {
                    reap_mosh_server(services, spec, timing, vec![pid]).await;
                }
            });
        } else if let Some(pid) = renderer_server {
            self.owe(reap_mosh_server(services, spec, timing, vec![pid]));
        }
        if !self.orphans.is_empty() {
            let orphans = std::mem::take(&mut self.orphans);
            self.owe(reap_mosh_server(
                self.services.clone(),
                self.spec.clone(),
                self.timing.clone(),
                orphans,
            ));
        }
        for (_, (_, reply)) in self.prompts.drain() {
            let _ = reply.send(false);
        }
        self.discard_pending(Discarded::SessionEnded);
        for kept in std::mem::take(&mut self.held) {
            let Kept::Typed(stream, typed, _) = kept else {
                continue;
            };
            self.emit(EventKind::InputDiscarded {
                stream,
                bytes: typed.len(),
                why: Discarded::SessionEnded,
            });
        }
        for delivery in self.deliveries.drain(..) {
            let _ = delivery.reply.send(false);
        }
        self.panes.clear();
        self.shell = None;
        for waiting in self.side_waiting.drain(..) {
            let _ = waiting.send(Err(Failure::lost("the session was closed")));
        }
        // A side dial's question goes with the session (B634).
        self.withdraw_prompts(SIDE);
        for waiting in self.control_waiting.drain(..) {
            let _ = waiting.send(Err(Failure::lost("the session was closed")));
        }
        if let Some(side) = self.side.take() {
            tokio::spawn(async move { side.close().await });
        }
        let owed = std::mem::take(&mut self.owed);
        let _ = tokio::time::timeout(OWED_BOUND, async {
            for task in owed {
                let _ = task.await;
            }
        })
        .await;
        if let Some(done) = self.done.take() {
            let _ = done.send(());
        }
    }

    fn emit(&self, kind: EventKind) {
        let _ = self.events.send(Event {
            session: self.id,
            kind,
        });
    }

    fn update(&self, change: impl FnOnce(&mut SessionView)) {
        let mut view = self.view.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut view);
    }

    // MARK: Messages

    async fn handle(&mut self, message: Msg) {
        let now = self.now();
        match message {
            Msg::Connect => self.life.connect(now),
            Msg::Disconnect(_) => unreachable!("handled by the loop"),
            Msg::SwitchToSsh => self.life.switch_to_ssh(now),
            Msg::Foreground => {
                self.life.foreground(now);
                self.revive_control(true);
                self.revive_herdr_control();
            }
            Msg::Background => self.life.background(now),
            Msg::ResumeInPlace(on) => {
                self.timing.resume_in_place = on;
                self.life.set_resume_in_place(on);
            }
            Msg::NetworkChanged => {
                self.life.network_changed(now);
                // At once, not after the path settles: a command given
                // meanwhile would go to a connection that may be gone (B401).
                // Heard in the background (B447), the return revives it.
                if self.life.is_foreground() {
                    self.revive_control(true);
                }
            }
            Msg::Unlocked => self.life.unlocked(now),
            Msg::MoshPrefix(key) => self.mosh_prefix = prefix_bytes(&key),
            Msg::Visible(visible) => self.set_visible(visible),
            Msg::Respec { target, proxy } => self.saved = Some((target, proxy)),
            Msg::AnswerHostKey { prompt, trust } => {
                if let Some((generation, reply)) = self.prompts.remove(&prompt) {
                    let _ = reply.send(trust);
                    self.life.attempt_waits_on_person(generation, false);
                }
            }
            Msg::AnswerQuestion { prompt, answer } => {
                if let Some((generation, reply)) = self.questions.remove(&prompt) {
                    let _ = reply.send(answer);
                    if generation != SIDE {
                        self.life.attempt_waits_on_person(generation, false);
                    }
                }
            }
            Msg::Tmux(TmuxAction::ScrollLines(lines)) if self.mosh_renders_tmux() => {
                self.mosh_scroll(lines);
            }
            Msg::Tmux(TmuxAction::AimWheel { col, row }) if self.mosh_renders_tmux() => {
                self.mosh_wheel_aim = Some((col, row));
            }
            Msg::Tmux(action) => self.tmux_action(action),
            Msg::Herdr { action, token } => match (&self.leg, &self.herdr_mosh) {
                (Some(Leg::Herdr { herdr, .. }), _) if self.life.state().is_live() => {
                    let pick = herdr_pick_of(&action);
                    let _ = herdr.commands.send(HerdrCommand::Action {
                        action,
                        token: Some(token),
                    });
                    if pick.is_some() {
                        self.herdr_pick = pick;
                    }
                }
                (Some(Leg::Mosh { .. }), Some(plane)) if self.life.state().is_live() => {
                    let _ = plane.leg.commands.send(HerdrMoshCommand::Action {
                        action,
                        token: Some(token),
                    });
                }
                (leg, _) => {
                    // Over SSH, a pick while the connection is redialled is
                    // made on the next herdr leg (B405, as B397's).
                    if !matches!(leg, Some(Leg::Mosh { .. }))
                        && let Some(pick) = herdr_pick_of(&action)
                    {
                        self.herdr_pick = Some(pick);
                    }
                    self.emit(EventKind::HerdrAnswer {
                        token,
                        answer: unanswered(&action),
                    });
                }
            },
            Msg::HerdrCancel { token } => {
                if let Some(Leg::Herdr { herdr, .. }) = &self.leg {
                    let _ = herdr.commands.send(HerdrCommand::CancelTask { token });
                }
                if let Some(plane) = &self.herdr_mosh {
                    let _ = plane
                        .leg
                        .commands
                        .send(HerdrMoshCommand::CancelTask { token });
                }
            }
            Msg::Paste { pane, text } => self.paste(pane, text),
            Msg::Deliver {
                pane,
                bytes,
                paste,
                reply,
            } => {
                if matches!(
                    self.life.state(),
                    SessionState::Ended(_) | SessionState::Failed(_)
                ) || bytes.is_empty()
                {
                    let _ = reply.send(false);
                } else {
                    self.deliveries.push(Delivery {
                        pane,
                        bytes,
                        paste,
                        reply,
                        deadline: now + crate::spec::ms(DELIVERY_WAIT),
                    });
                    // In the background, a transport not heard from lately
                    // is asked first (B524).
                    self.life.check_for_delivery(now);
                    self.try_deliver();
                }
            }
            Msg::Attempt { generation, result } => self.attempt_done(generation, result).await,
            Msg::OnPerson {
                generation,
                waiting,
            } => self.life.attempt_waits_on_person(generation, waiting),
            Msg::Ask {
                generation,
                host,
                port,
                fingerprint,
                previous,
                older_algorithm,
                conflicting,
                reply,
            } => {
                if generation != self.life.generation()
                    || self.attempt.as_ref().map(|(g, _)| *g) != Some(generation)
                {
                    let _ = reply.send(false);
                    return;
                }
                self.next_prompt += 1;
                let prompt = self.next_prompt;
                self.prompts.insert(prompt, (generation, reply));
                // The dial waits on the person's answer (B395).
                self.life.attempt_waits_on_person(generation, true);
                // A new host only: a changed key or two stored ones have
                // their own words (B783).
                let connected_before =
                    self.spec.connected_before && previous.is_none() && !conflicting;
                self.emit(EventKind::HostKeyPrompt {
                    prompt,
                    host,
                    port,
                    fingerprint,
                    previous,
                    older_algorithm: older_algorithm.map(str::to_owned),
                    conflicting,
                    connected_before,
                });
            }
            Msg::Question {
                generation,
                host,
                question,
                reply,
            } => {
                // A side connection's (B634): while its dial is out, unless
                // the person said no to one.
                let side = generation == SIDE;
                let asked = if side {
                    !self.side_waiting.is_empty() && !self.side_declined
                } else {
                    generation == self.life.generation()
                        && self.attempt.as_ref().map(|(g, _)| *g) == Some(generation)
                };
                if !asked {
                    let _ = reply.send(None);
                    return;
                }
                self.next_prompt += 1;
                let prompt = self.next_prompt;
                self.questions.insert(prompt, (generation, reply));
                // The dial waits on the person's answer, as for a host key.
                if !side {
                    self.life.attempt_waits_on_person(generation, true);
                }
                self.emit(EventKind::SignInQuestion {
                    prompt,
                    host,
                    name: question.name,
                    instructions: question.instructions,
                    text: question.prompt,
                    echo: question.echo,
                });
            }
            Msg::Probe {
                generation,
                probe,
                result,
            } => {
                if self.life.resume_pending()
                    && let Some(question) = self.resume_question.as_mut()
                    && question.generation == generation
                    && question.probe == probe
                    && question.answer.is_none()
                {
                    question.answer = Some((result, now));
                }
                self.life.probe_answered(generation, probe, result, now);
                // Deliveries waited for a delivery's question (B524).
                self.try_deliver();
            }
            Msg::Shell { generation, end } => match end {
                ShellEnd::Exited { status } => {
                    self.life
                        .ended(generation, EndReason::ShellExited { status }, now)
                }
                ShellEnd::Lost => self.life.lost(generation, now),
            },
            Msg::TmuxNote { generation, note } => self.tmux_note(generation, note),
            Msg::Mosh { generation, note } => self.mosh_note(generation, note),
            Msg::HerdrNote { generation, note } => self.herdr_note(generation, note),
            Msg::HerdrMosh { generation, note } => self.herdr_mosh_note(generation, note),
            Msg::HerdrControlUp { epoch, result } => {
                self.herdr_control_up(epoch, result);
                self.lend_to_control_waiting();
            }
            Msg::HerdrControlRetry { epoch } => {
                if self.herdr_mosh.as_ref().is_some_and(|c| c.epoch == epoch) {
                    self.redial_herdr_control();
                }
            }
            Msg::Sidecar { epoch, note } => self.sidecar_note(epoch, note),
            Msg::ControlUp { epoch, result } => {
                self.control_up(epoch, result);
                // Up or not, whoever waited for it is answered now.
                self.lend_to_control_waiting();
            }
            Msg::ControlRetry { epoch } => {
                if self.control.as_ref().is_some_and(|c| c.epoch == epoch) {
                    self.redial_control();
                }
            }
            Msg::ControlChecked { epoch, alive } => self.control_checked(epoch, alive),
            Msg::ControlHoldExpired { hold } => self.control_hold_expired(hold),
            Msg::TmuxHoldExpired { hold } => self.tmux_hold_expired(hold),
            Msg::PasteLanded {
                dest,
                paste,
                landed,
            } => {
                let current = self.settling.get(&dest).is_some_and(|s| s.paste == paste);
                if current && !landed {
                    let why = if self.herdr_mosh.is_some() {
                        Discarded::NotAttached
                    } else {
                        Discarded::Reconnected
                    };
                    self.drop_settled(&dest, why);
                } else if current && let Some(settling) = self.settling.get_mut(&dest) {
                    settling.landed = true;
                }
                self.release_settled(now);
            }
            Msg::SidecarHandoff { epoch, handoff } => match self.control.as_mut() {
                // The dropped sidecar's debt goes to the next one; with the
                // control plane gone, to the end of the session.
                Some(control) if control.epoch >= epoch && control.sidecar.is_none() => {
                    control.handoff = handoff;
                }
                _ => {
                    if handoff.ledger.is_some() && self.handoff.ledger.is_none() {
                        self.handoff = handoff;
                    }
                }
            },
            Msg::RendererPid { generation, pid } => {
                if let Some(control) = self.control.as_mut().filter(|c| c.generation == generation)
                {
                    control.renderer_pid = Some(pid);
                    if let Some(sidecar) = &control.sidecar {
                        let _ = sidecar.commands.send(TmuxCommand::RendererPid(pid));
                    }
                }
            }
            Msg::Input { stream, event } => self.input(stream, event),
            Msg::Flush(done) => self.flush(done),
            Msg::Fault(fault) => {
                let ssh = self
                    .leg
                    .as_ref()
                    .and_then(Leg::ssh)
                    .or_else(|| self.control.as_ref().and_then(|c| c.ssh.as_ref()))
                    .or_else(|| self.herdr_mosh.as_ref().and_then(|c| c.ssh.as_ref()));
                if let Some(ssh) = ssh {
                    log::info!("session {}: injected {fault:?}", self.id);
                    ssh.inject_fault(fault);
                }
            }
            Msg::MoshLoss => {
                if let Some(Leg::Mosh { mosh, .. }) = &self.leg {
                    log::info!("session {}: injected the loss of mosh", self.id);
                    let _ = mosh.commands.send(MoshCommand::Close);
                }
            }
            Msg::Repaint => {
                // Only mosh has a server-side screen to redraw from blank.
                if let Some(Leg::Mosh { mosh, .. }) = &self.leg {
                    let _ = mosh.commands.send(MoshCommand::Repaint);
                }
            }
            Msg::BorrowSsh(reply) => self.borrow_ssh(reply),
            Msg::SideSsh { result } => {
                self.withdraw_prompts(SIDE);
                match &result {
                    Ok(ssh) => {
                        self.side = Some(ssh.clone());
                        self.side_idle_until = Some(self.now() + SIDE_IDLE_MS);
                    }
                    // The person said no to its question (B634): the side
                    // feature fails as before, and nothing asks again.
                    Err(failure)
                        if failure.cause
                            == crate::event::Cause::Credentials(
                                crate::event::CredentialsWhy::Cancelled,
                            ) =>
                    {
                        self.side_declined = true;
                    }
                    Err(_) => {}
                }
                for waiting in self.side_waiting.drain(..) {
                    let _ = waiting.send(result.clone());
                }
            }
        }
    }

    /// A paste the person made into a pane on screen: one bracketed paste
    /// the multiplexer fits to the pane's real mode (B43); a plain shell or
    /// mosh gets the markers, the text's own taken out (B115).
    fn paste(&mut self, pane: String, text: String) {
        if text.is_empty() {
            return;
        }
        if !self.life.state().is_live() {
            let stream = self.stream_of_pane(&pane).unwrap_or(0);
            self.emit(EventKind::InputDiscarded {
                stream,
                bytes: text.len(),
                why: Discarded::NotLive,
            });
            return;
        }
        match &self.leg {
            Some(Leg::Tmux { tmux, .. }) => {
                let _ = tmux
                    .commands
                    .send(TmuxCommand::Action(TmuxAction::Paste { pane, text }));
            }
            Some(Leg::Herdr { herdr, .. }) => {
                let _ = herdr.commands.send(HerdrCommand::Paste { pane, text });
            }
            Some(Leg::Shell { shell, .. }) => {
                let payload = moshpit_herdr::frame::bracketed_paste(&text);
                let _ = shell.commands.send(ShellCommand::Input(payload));
            }
            Some(Leg::Mosh { mosh, .. }) => {
                let end = self.shell.as_ref().map_or(0, |shell| shell.written);
                if let Some(plane) = &self.herdr_mosh {
                    // Under the renderer's rule, as typing is.
                    let _ = plane.leg.commands.send(HerdrMoshCommand::Paste {
                        text,
                        end,
                        landed: None,
                    });
                    return;
                }
                let payload = moshpit_herdr::frame::bracketed_paste(&text);
                let _ = mosh.commands.send(MoshCommand::Keys {
                    bytes: payload,
                    end,
                });
            }
            None => {}
        }
    }

    fn stream_of_pane(&self, pane: &str) -> Option<StreamId> {
        if pane == SHELL_PANE {
            return self.shell.as_ref().map(|shell| shell.stream.id());
        }
        self.current
            .iter()
            .find(|info| info.pane == pane)
            .map(|info| info.stream)
    }

    fn flush(&mut self, done: oneshot::Sender<()>) {
        let sidecar = self.control.as_ref().and_then(|c| c.sidecar.as_ref());
        match (&self.leg, sidecar) {
            (Some(Leg::Tmux { tmux, .. }), _) | (Some(Leg::Mosh { .. }), Some(tmux)) => {
                if let Err(error) = tmux.commands.send(TmuxCommand::Flush(done))
                    && let TmuxCommand::Flush(done) = error.0
                {
                    let _ = done.send(());
                }
            }
            (Some(Leg::Herdr { herdr, .. }), _) => {
                if let Err(error) = herdr.commands.send(HerdrCommand::Flush(done))
                    && let HerdrCommand::Flush(done) = error.0
                {
                    let _ = done.send(());
                }
            }
            _ => {
                let _ = done.send(());
            }
        }
    }

    /// The terminal came on screen or left it (back to Home). Leaving hands
    /// tmux's window pins back, so a desktop client sharing the windows gets
    /// its size again; coming back takes them again — what the app's
    /// terminal screen does on disappear and on return. In the background
    /// the lifecycle's own background/foreground handle them.
    fn set_visible(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if !self.life.is_foreground() {
            return;
        }
        let command = || {
            if visible {
                TmuxCommand::Foreground
            } else {
                TmuxCommand::Background
            }
        };
        if let Some(Leg::Tmux { tmux, .. }) = &self.leg {
            let _ = tmux.commands.send(command());
        }
        if let Some(sidecar) = self.control.as_ref().and_then(|c| c.sidecar.as_ref()) {
            let _ = sidecar.commands.send(command());
        }
    }

    /// An SSH connection to the host for someone outside the terminal (SFTP,
    /// an exec): the live leg's own when it has one — what the app's
    /// `fileTransferSSH` rides — mosh's control plane's, or a side
    /// connection dialled on demand and kept (plain mosh has no SSH once
    /// bootstrapped). The host is already trusted: nobody is asked about a
    /// key. Not before the session is live.
    fn borrow_ssh(&mut self, reply: oneshot::Sender<Result<Ssh, Failure>>) {
        if !self.life.state().is_live() {
            let _ = reply.send(Err(Failure::lost("the session is not connected")));
            return;
        }
        if let Some(ssh) = self.leg.as_ref().and_then(Leg::ssh) {
            let _ = reply.send(Ok(ssh.clone()));
            return;
        }
        if let Some(ssh) = self
            .control
            .as_ref()
            .and_then(|c| c.ssh.as_ref())
            .or_else(|| self.herdr_mosh.as_ref().and_then(|c| c.ssh.as_ref()))
            .filter(|ssh| !ssh.is_closed())
        {
            let _ = reply.send(Ok(ssh.clone()));
            return;
        }
        if self
            .control
            .as_ref()
            .is_some_and(|c| c.dialling && c.failures == 0)
            || self
                .herdr_mosh
                .as_ref()
                .is_some_and(|c| c.dialling && c.failures == 0)
        {
            // mosh + tmux, just live: the control plane's terminal is
            // opening on the bootstrap's connection (a round trip). Lend that
            // one once it's up, rather than sign in again beside it — what
            // the app's first borrows (its maintenance, an upload sweep) hit
            // on every connect. A redial after failures is no such wait.
            self.control_waiting.push(reply);
            return;
        }
        if let Some(side) = &self.side
            && !side.is_closed()
        {
            self.side_idle_until = Some(self.now() + SIDE_IDLE_MS);
            let _ = reply.send(Ok(side.clone()));
            return;
        }
        self.side = None;
        self.side_waiting.push(reply);
        if self.side_waiting.len() > 1 {
            return; // a dial is already out
        }
        let services = self.services.clone();
        let spec = self.spec.clone();
        let timing = self.timing.clone();
        let me = self.me.clone();
        // A second factor the host asks is asked of the person, once
        // (B634): the connection is kept for the borrows after.
        let questions: Option<Arc<dyn SignInQuestions>> = (!self.side_declined).then(|| {
            Arc::new(Prompt {
                session: self.me.clone(),
                generation: SIDE,
            }) as Arc<dyn SignInQuestions>
        });
        tokio::spawn(async move {
            let prompt: Arc<dyn HostKeyPrompt> = Arc::new(DenyAll);
            let result = dial_asking(&services, &spec, &timing, prompt, questions).await;
            let _ = me.send(Msg::SideSsh { result });
        });
    }

    /// The side connection, kept for borrows, goes once none came for
    /// [`SIDE_IDLE_MS`] (B634). A borrower still holding it keeps it until
    /// it is done.
    fn release_idle_side(&mut self, now: u64) {
        if self.side_idle_until.is_some_and(|until| now >= until) {
            self.side_idle_until = None;
            self.side = None;
        }
    }

    /// Answers the borrowers that waited for mosh + tmux's control plane:
    /// its connection when it came up, otherwise as a borrow is answered
    /// now (a side connection, or not connected).
    fn lend_to_control_waiting(&mut self) {
        for reply in std::mem::take(&mut self.control_waiting) {
            self.borrow_ssh(reply);
        }
    }

    async fn attempt_done(&mut self, generation: u64, result: Result<Established, Failure>) {
        if self.attempt.as_ref().map(|(g, _)| *g) == Some(generation) {
            self.attempt = None;
        }
        self.withdraw_prompts(generation);
        let now = self.now();
        match result {
            Ok(established) => {
                let transport = match established {
                    Established::Ssh { .. } => Transport::Ssh,
                    Established::Mosh { .. } => Transport::Mosh,
                };
                if self.life.attempt_succeeded(generation, transport, now) {
                    // What the UI reads is in place when Connected goes out,
                    // and Connected goes out before the panes it brings.
                    self.note_established(&established);
                    self.act().await;
                    self.start_leg(generation, established);
                    self.reap_orphans();
                    self.try_deliver();
                } else {
                    // Superseded or given up meanwhile: not ours to use — and
                    // a mosh-server it started is stopped (B194).
                    tokio::spawn(established.close());
                }
            }
            Err(failure) => {
                log::info!(
                    "session {}: attempt {generation} failed: {}",
                    self.id,
                    failure.log
                );
                self.life.attempt_failed(generation, failure, now);
            }
        }
    }

    fn withdraw_prompts(&mut self, generation: u64) {
        let gone: Vec<u64> = self
            .prompts
            .iter()
            .filter(|(_, (g, _))| *g == generation)
            .map(|(prompt, _)| *prompt)
            .collect();
        for prompt in gone {
            if let Some((_, reply)) = self.prompts.remove(&prompt) {
                let _ = reply.send(false);
            }
            self.emit(EventKind::HostKeyPromptWithdrawn { prompt });
        }
        let gone: Vec<u64> = self
            .questions
            .iter()
            .filter(|(_, (g, _))| *g == generation)
            .map(|(prompt, _)| *prompt)
            .collect();
        for prompt in gone {
            if let Some((_, reply)) = self.questions.remove(&prompt) {
                let _ = reply.send(None);
            }
            self.emit(EventKind::SignInQuestionWithdrawn { prompt });
        }
    }

    // MARK: Actions

    async fn act(&mut self) {
        while let Some(action) = self.life.poll_action() {
            match action {
                Action::Dial {
                    generation,
                    protocol,
                    ..
                } => self.dial(generation, protocol),
                Action::Abandon { generation } => {
                    if let Some((g, task)) = self.attempt.take() {
                        if g == generation {
                            // Its drop stops a mosh-server it had started.
                            task.abort();
                        } else {
                            self.attempt = Some((g, task));
                        }
                    }
                    self.withdraw_prompts(generation);
                }
                Action::Teardown { generation, why } => self.teardown(generation, why).await,
                Action::Probe {
                    generation,
                    probe,
                    timeout,
                } => self.probe(generation, probe, timeout),
                Action::ResumeProbe {
                    generation,
                    probe,
                    timeout,
                    away_ms,
                } => self.resume_probe(generation, probe, timeout, away_ms),
                Action::ResumeLost { away_ms } => self.emit(EventKind::ResumeChecked {
                    away_ms,
                    waited_ms: 0,
                    outcome: ResumeOutcome::Dead,
                    through_tmux: false,
                    held: self.held_typing(),
                }),
                Action::ResumeMosh { generation, force } => {
                    if let Some(Leg::Mosh { mosh, .. }) = self.leg_of(generation) {
                        let _ = mosh.commands.send(MoshCommand::Resume { force });
                    }
                    // The keepalive's tick too: the check rides along,
                    // nothing waits for it.
                    self.revive_control(false);
                    self.revive_herdr_control();
                }
                Action::RoamMosh { generation } => {
                    if let Some(Leg::Mosh { mosh, .. }) = self.leg_of(generation) {
                        let _ = mosh.commands.send(MoshCommand::Roam);
                    }
                    self.revive_control(true);
                    self.revive_herdr_control();
                }
                Action::Background { generation } => match self.leg_of(generation) {
                    Some(Leg::Tmux { tmux, .. }) => {
                        let _ = tmux.commands.send(TmuxCommand::Background);
                    }
                    Some(Leg::Herdr { herdr, .. }) => {
                        let _ = herdr.commands.send(HerdrCommand::Background);
                    }
                    Some(Leg::Mosh { .. }) => {
                        if let Some(sidecar) =
                            self.control.as_ref().and_then(|c| c.sidecar.as_ref())
                        {
                            let _ = sidecar.commands.send(TmuxCommand::Background);
                        }
                        if let Some(plane) = &self.herdr_mosh {
                            let _ = plane.leg.commands.send(HerdrMoshCommand::Background);
                        }
                    }
                    _ => {}
                },
                Action::Foreground { generation } => {
                    let visible = self.visible;
                    match self.leg_of(generation) {
                        // Pins only for the terminal on screen.
                        Some(Leg::Tmux { tmux, .. }) if visible => {
                            let _ = tmux.commands.send(TmuxCommand::Foreground);
                        }
                        Some(Leg::Herdr { herdr, .. }) => {
                            let _ = herdr.commands.send(HerdrCommand::Foreground);
                        }
                        Some(Leg::Mosh { .. }) => {
                            if visible
                                && let Some(sidecar) =
                                    self.control.as_ref().and_then(|c| c.sidecar.as_ref())
                            {
                                let _ = sidecar.commands.send(TmuxCommand::Foreground);
                            }
                            if let Some(plane) = &self.herdr_mosh {
                                let _ = plane.leg.commands.send(HerdrMoshCommand::Foreground);
                            }
                        }
                        _ => {}
                    }
                }
                Action::State(state) => {
                    let ended = matches!(state, SessionState::Ended(_));
                    let transport = self.life.live();
                    let unix_now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0.0, |d| d.as_secs_f64());
                    self.update(|view| {
                        view.stamp(&state, unix_now);
                        view.state = state.clone();
                        view.transport = transport;
                        if !matches!(state, SessionState::Connected) {
                            view.attach_stalled = false;
                        }
                        // Live on SSH: nothing of a mosh flow it left stays
                        // (B617) — the card said "roaming" over SSH.
                        if transport == Some(Transport::Ssh) {
                            view.mosh_roaming = false;
                            view.mosh_srtt_ms = None;
                            view.mosh_diagnostics = None;
                        }
                    });
                    self.emit(EventKind::State(state.clone()));
                    if ended {
                        self.discard_pending(Discarded::SessionEnded);
                    }
                    if matches!(state, SessionState::Ended(_) | SessionState::Failed(_)) {
                        for delivery in self.deliveries.drain(..) {
                            let _ = delivery.reply.send(false);
                        }
                    }
                }
            }
        }
    }

    fn leg_of(&self, generation: u64) -> Option<&Leg> {
        self.leg
            .as_ref()
            .filter(|leg| leg.generation() == generation)
    }

    fn dial(&mut self, generation: u64, protocol: crate::spec::Protocol) {
        if let Some((_, task)) = self.attempt.take() {
            task.abort();
        }
        // B329: a connection saved since dials where it was saved to.
        if let Some((target, proxy)) = self.saved.take() {
            self.spec.target = target;
            self.spec.proxy = proxy;
        }
        let prompt = Arc::new(Prompt {
            session: self.me.clone(),
            generation,
        });
        let questions: Arc<dyn SignInQuestions> = prompt.clone();
        let prompt: Arc<dyn HostKeyPrompt> = prompt;
        // The credential read says while it may wait on the person (B395).
        let mut services = self.services.clone();
        services.credentials = Arc::new(ReadingCredentials {
            inner: services.credentials.clone(),
            session: self.me.clone(),
            generation,
        });
        self.dialed_grid = self.grid;
        let attempt = Attempt {
            spec: self.spec.clone(),
            protocol,
            timing: self.timing.clone(),
            services,
            grid: self.grid,
            prompt,
            questions: Some(questions),
        };
        let me = self.me.clone();
        let task = tokio::spawn(async move {
            let result = attempt.run().await;
            let _ = me.send(Msg::Attempt { generation, result });
        });
        self.attempt = Some((generation, task));
    }

    fn probe(&self, generation: u64, probe: u64, timeout: Duration) {
        let Some(ssh) = self.leg_of(generation).and_then(Leg::ssh).cloned() else {
            return;
        };
        let me = self.me.clone();
        tokio::spawn(async move {
            let result = match tokio::time::timeout(timeout, ssh.run("true")).await {
                Ok(Ok(output)) if output.closed => Probe::Alive,
                Ok(_) => Probe::Failed,
                Err(_) => Probe::TimedOut,
            };
            let _ = me.send(Msg::Probe {
                generation,
                probe,
                result,
            });
        });
    }

    /// The in-place return's question (`Timing::resume_in_place`): one
    /// round trip on the leg itself. tmux's control channel when it is
    /// attached — tmux answers in order, so by its answer the output of the
    /// suspension is in the panes — else the SSH keepalive (a global
    /// request sshd answers itself: nothing forked, one round trip).
    fn resume_probe(&mut self, generation: u64, probe: u64, timeout: Duration, away_ms: u64) {
        let leg = self.leg_of(generation);
        let ssh = leg.and_then(Leg::ssh).cloned();
        let tmux = match (leg, &ssh) {
            (Some(Leg::Tmux { tmux, .. }), Some(_)) => {
                let (reply, answer) = oneshot::channel();
                let _ = tmux.commands.send(TmuxCommand::Ping(reply));
                Some(answer)
            }
            _ => None,
        };
        // A question the app left before it was answered goes unreported:
        // this return asks and reports its own (B394).
        self.resume_question = Some(ResumeQuestion {
            generation,
            probe,
            asked_at: self.now(),
            away_ms,
            through_tmux: tmux.is_some(),
            answer: None,
        });
        let Some(ssh) = ssh else {
            // Nothing to ask: dead (reported as such, so the return still
            // says what it found), and the question must not hold input
            // forever.
            let _ = self.me.send(Msg::Probe {
                generation,
                probe,
                result: Probe::Failed,
            });
            return;
        };
        let me = self.me.clone();
        tokio::spawn(async move {
            let started = tokio::time::Instant::now();
            let ask_ssh = |left: Duration| async move {
                if ssh.ping(left).await {
                    Probe::Alive
                } else if ssh.is_closed() {
                    Probe::Failed
                } else {
                    Probe::TimedOut
                }
            };
            let result = match tmux {
                Some(answer) => match tokio::time::timeout(timeout, answer).await {
                    Ok(Ok(Some(true))) => Probe::Alive,
                    Ok(Ok(Some(false))) | Ok(Err(_)) => Probe::Failed,
                    // Nothing attached to ask (an attach that stalled).
                    Ok(Ok(None)) => ask_ssh(timeout.saturating_sub(started.elapsed())).await,
                    Err(_) => Probe::TimedOut,
                },
                None => ask_ssh(timeout).await,
            };
            let _ = me.send(Msg::Probe {
                generation,
                probe,
                result,
            });
        });
    }

    /// The in-place return's question is settled — by its answer, or by
    /// the transport going first (a socket iOS closed fails before anything
    /// can answer): one [`EventKind::ResumeChecked`] either way, with the
    /// time away, so a device's log says how often a suspended socket
    /// survives. Nothing when the person closed the session meanwhile.
    fn settle_question(&mut self) {
        let Some(question) = self.resume_question.take() else {
            return;
        };
        if self.life.is_ended() {
            return;
        }
        let now = self.now();
        let (outcome, at) = match question.answer {
            Some((Probe::Alive, at)) => (ResumeOutcome::Kept, at),
            Some((Probe::TimedOut, at)) => (ResumeOutcome::Silent, at),
            Some((Probe::Failed, at)) => (ResumeOutcome::Dead, at),
            None => (ResumeOutcome::Dead, now),
        };
        let waited_ms = at.saturating_sub(question.asked_at);
        self.emit(EventKind::ResumeChecked {
            away_ms: question.away_ms,
            waited_ms,
            outcome,
            through_tmux: question.through_tmux,
            held: self.held_typing(),
        });
    }

    /// mosh with tmux: mosh renders tmux's own client, and a scroll goes to
    /// that client over mosh (B362). The sidecar is a control client:
    /// copy-mode it drove would never reach that screen, its local route
    /// has no view there, and its wheel (`send-keys`) went into whatever
    /// copy-mode the client was in.
    fn mosh_renders_tmux(&self) -> bool {
        matches!(self.leg, Some(Leg::Mosh { .. }))
            && self.herdr_mosh.is_none()
            && self.spec.multiplexer == Multiplexer::Tmux
    }

    /// Over a pane whose program wants the mouse (as the sidecar last read
    /// it), the program's wheel at the centre of the grid; otherwise a page
    /// of tmux's history in copy-mode. A drag reads the flags as it starts
    /// and its first tick can beat the answer into copy-mode: the wheel
    /// after it leaves copy-mode first, on the same lane.
    fn mosh_scroll(&mut self, lines: i64) {
        if lines == 0 {
            return;
        }
        let (wants_mouse, alt_screen) = {
            let view = self.view.lock().unwrap_or_else(|e| e.into_inner());
            view.tmux_view.as_ref().map_or((false, false), |tmux| {
                (tmux.input.wants_mouse, tmux.input.alt_screen)
            })
        };
        // An alternate-screen pane without the mouse (less, man, git log):
        // its arrow keys, from the sidecar, whose `send-keys` tmux encodes
        // by the pane's real cursor-key mode (it sends nothing until tmux
        // has said the mouse is off). tmux's copy-mode there pages nothing
        // (B500).
        if alt_screen
            && !wants_mouse
            && let Some(sidecar) = self.control.as_ref().and_then(|c| c.sidecar.as_ref())
        {
            let _ = sidecar
                .commands
                .send(TmuxCommand::Action(TmuxAction::ScrollLines(lines)));
            return;
        }
        let command = if wants_mouse {
            let (cols, rows) = self.grid;
            let (col, row) = moshpit_tmux::controller::wheel_cell(
                self.mosh_wheel_aim,
                i64::from(cols),
                i64::from(rows),
            );
            MoshCommand::Mouse(moshpit_tmux::controller::wheel_bytes(lines, col, row))
        } else {
            self.ask_mosh_prefix();
            MoshCommand::CopyScroll {
                lines,
                prefix: self.mosh_prefix.clone(),
            }
        };
        if let Some(Leg::Mosh { mosh, .. }) = &self.leg {
            let _ = mosh.commands.send(command);
        }
    }

    /// mosh with tmux: the copy-mode tmux reports for the pane on screen,
    /// to the mosh loop whenever it changes (B614). The loop's latch was
    /// its only record: the person's own `q` went out as `qq`, a pane
    /// switched to got a `q` first, a pane left in copy-mode never one.
    fn forward_copy_mode(&mut self) {
        let Some(Leg::Mosh { mosh, .. }) = &self.leg else {
            return;
        };
        let in_mode = {
            let view = self.view.lock().unwrap_or_else(|e| e.into_inner());
            view.tmux_view.as_ref().map(|tmux| tmux.input.in_copy_mode)
        };
        let Some(in_mode) = in_mode else {
            return;
        };
        if self.mosh_copy_mode_sent == Some(in_mode) {
            return;
        }
        if mosh.commands.send(MoshCommand::CopyModeIs(in_mode)).is_ok() {
            self.mosh_copy_mode_sent = Some(in_mode);
        }
    }

    /// The server's prefix, asked of the sidecar once (a server with
    /// another prefix enters copy-mode with its own).
    fn ask_mosh_prefix(&mut self) {
        if self.mosh_prefix_asked {
            return;
        }
        let Some(sidecar) = self.control.as_ref().and_then(|c| c.sidecar.as_ref()) else {
            return;
        };
        let (reply, answer) = oneshot::channel();
        if sidecar.commands.send(TmuxCommand::Prefix(reply)).is_err() {
            return;
        }
        self.mosh_prefix_asked = true;
        let me = self.me.clone();
        tokio::spawn(async move {
            if let Ok(key) = answer.await {
                let _ = me.send(Msg::MoshPrefix(key));
            }
        });
    }

    /// The in-place return has its answer: what was typed meanwhile goes
    /// out on the transport that answered, in order. The session no longer
    /// live (the connection was dead and it is reconnecting): a multiplexer
    /// pane's keys wait for the pane on the new attach, a plain shell's are
    /// dropped and said (B396).
    fn release_held(&mut self) {
        let held = std::mem::take(&mut self.held);
        let live = self.life.state().is_live();
        for kept in held {
            let (stream, typed, bar) = match kept {
                Kept::Typed(stream, typed, bar) => (stream, typed, bar),
                // Its turn, after the keys before it: on the transport that
                // answered, or kept for the redial's grid.
                Kept::Resize(stream, cols, rows) => {
                    self.resize_in_turn(stream, cols, rows);
                    continue;
                }
            };
            if live {
                self.route(stream, typed);
            } else if bar {
                // The shortcut bar's tap isn't kept for later (B476, B482):
                // a stray one must not land in an agent's prompt after the
                // reconnect.
                self.emit(EventKind::InputDiscarded {
                    stream,
                    bytes: typed.len(),
                    why: Discarded::NotLive,
                });
            } else if let Some(pane) = self.panes.get(&stream).map(|c| c.pane.clone()) {
                // The connection turned out dead, and the screen had looked
                // live: a multiplexer pane comes back with the redial, and its
                // keys wait for it, as typing into the last frame does (B396,
                // B5).
                self.pending.entry(pane).or_default().push((stream, typed));
            } else {
                // A plain shell: the redial is a new shell. Not sent, and
                // said (B396).
                self.emit(EventKind::InputDiscarded {
                    stream,
                    bytes: typed.len(),
                    why: Discarded::Reconnected,
                });
            }
        }
    }

    /// Writes held for the return's answer (sizes aren't writes).
    fn held_typing(&self) -> usize {
        self.held
            .iter()
            .filter(|kept| matches!(kept, Kept::Typed(..)))
            .count()
    }

    // MARK: Legs

    /// The degrade notice and mosh's server, before the state says
    /// Connected.
    fn note_established(&mut self, established: &Established) {
        let (degrade, server_pid, older_host_key) = match established {
            Established::Ssh {
                degrade,
                older_host_key,
                ..
            } => (*degrade, None, *older_host_key),
            Established::Mosh {
                degrade,
                server_pid,
                older_host_key,
                ..
            } => (*degrade, *server_pid, *older_host_key),
        };
        if let Some(algorithm) = older_host_key {
            self.emit(EventKind::OlderHostKeyAlgorithm {
                algorithm: algorithm.to_owned(),
            });
        }
        self.update(|view| {
            view.degrade = degrade;
            view.mosh_server_pid = server_pid;
            view.return_path_dead = false;
        });
        if let Some(degrade) = degrade {
            self.emit(EventKind::Degraded(degrade));
        }
    }

    fn start_leg(&mut self, generation: u64, established: Established) {
        match established {
            Established::Ssh {
                ssh,
                pty,
                multiplexer,
                push,
                ..
            } => match multiplexer {
                Multiplexer::None => {
                    let stream = self.shell_stream();
                    let me = self.me.clone();
                    let clock = self.clock.clone();
                    let shell = ShellLeg::spawn(
                        pty,
                        stream,
                        self.inbound.clone(),
                        move || clock.now(),
                        Box::new(move |end| {
                            let _ = me.send(Msg::Shell { generation, end });
                        }),
                    );
                    // The terminal was opened at the size of when the dial
                    // began; one given since (the keyboard up, the phone
                    // turned under the veil) goes now (B616).
                    if self.grid != self.dialed_grid {
                        let (cols, rows) = self.grid;
                        let _ = shell.commands.send(ShellCommand::Resize { cols, rows });
                    }
                    self.leg = Some(Leg::Shell {
                        generation,
                        ssh,
                        shell,
                    });
                    self.publish_shell_pane();
                }
                Multiplexer::Tmux => {
                    self.attach += 1;
                    let start = TmuxStart {
                        ssh: ssh.clone(),
                        pty,
                        tmux: self.spec.tmux_command().to_owned(),
                        config: self.tmux_config(),
                        grid: self.grid,
                        // The person's pick, made while redialling (B405).
                        restore_chosen: self.tmux_pick.is_some(),
                        restore: self.next_tmux_restore(),
                        inherited: std::mem::take(&mut self.handoff),
                        attach_stall: self.timing.attach_stall,
                        inbound: self.inbound.clone(),
                        pinned: self.pinned(),
                        sidecar: None,
                    };
                    let me = self.me.clone();
                    let clock = self.clock.clone();
                    let tmux = TmuxLeg::spawn(
                        start,
                        move || clock.now(),
                        Box::new(move |note| {
                            let _ = me.send(Msg::TmuxNote { generation, note });
                        }),
                    );
                    self.current.clear();
                    self.update(|view| view.attach_stalled = false);
                    self.leg = Some(Leg::Tmux {
                        generation,
                        ssh,
                        tmux,
                    });
                }
                Multiplexer::Herdr => {
                    self.attach += 1;
                    self.current.clear();
                    let custom = self.spec.herdr_path.clone();
                    let config = moshpit_herdr::client::Config {
                        socket_path: socket_path_for(custom.as_deref()),
                        custom_path: custom,
                        frame_channel: true,
                        // The attempt opened the channel with the loop as
                        // its program.
                        frame_loop: true,
                        immersive_zoom: false,
                        push,
                        connection_id: self.spec.connection_id.clone(),
                        budgets: Default::default(),
                    };
                    let me = self.me.clone();
                    let clock = self.clock.clone();
                    let herdr = HerdrLeg::spawn(
                        HerdrStart {
                            ssh: ssh.clone(),
                            pty,
                            config,
                            grid: self.grid,
                            inbound: self.inbound.clone(),
                            foreground: self.life.is_foreground(),
                        },
                        move || clock.now(),
                        Box::new(move |note| {
                            let _ = me.send(Msg::HerdrNote { generation, note });
                        }),
                    );
                    // A pick the last connection never carried (B397).
                    if let Some(repick) = self.herdr_repick() {
                        let _ = herdr.commands.send(repick);
                    }
                    self.leg = Some(Leg::Herdr {
                        generation,
                        ssh,
                        herdr,
                    });
                }
            },
            Established::Mosh {
                server,
                key,
                multiplexer,
                control,
                herdr,
                tmux_layouts,
                reaper,
                ..
            } => {
                // The session owns the server now: its teardown stops it.
                let server_pid = reaper.disarm();
                let stream = self.shell_stream();
                let me = self.me.clone();
                let painted = Arc::new(AtomicU64::new(0));
                let clock = self.clock.clone();
                let mosh = mosh_udp::spawn(
                    self.services.udp.clone(),
                    MoshStart {
                        server,
                        key,
                        config: self.timing.mosh.clone(),
                        grid: self.grid,
                        painted: herdr.as_ref().map(|_| Painted {
                            stamp: painted.clone(),
                            clock: Box::new(move || clock.now()),
                        }),
                    },
                    stream.clone(),
                    Box::new(move |note| {
                        let _ = me.send(Msg::Mosh { generation, note });
                    }),
                );
                self.leg = Some(Leg::Mosh {
                    generation,
                    mosh,
                    server_pid,
                    first_contact: false,
                    return_path_reported: false,
                    reaped: false,
                });
                // A new loop knows nothing of a pane left in copy-mode.
                self.mosh_copy_mode_sent = None;
                self.forward_copy_mode();
                self.publish_shell_pane();
                match (multiplexer, control, herdr) {
                    (Multiplexer::Tmux, Some(ssh), _) => {
                        self.start_control(generation, ssh, server_pid, tmux_layouts)
                    }
                    (Multiplexer::Herdr, Some(ssh), Some(herdr)) => {
                        self.start_herdr_control(generation, ssh, herdr, stream, painted)
                    }
                    (_, Some(ssh), _) => {
                        tokio::spawn(async move { ssh.close().await });
                    }
                    _ => {}
                }
            }
        }
    }

    fn tmux_config(&self) -> TmuxConfig {
        let unix_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        TmuxConfig {
            size_owner_label: self.services.device_label.clone(),
            unix_offset_ms: unix_now - self.now() as i64,
            ..TmuxConfig::default()
        }
    }

    /// The session's one shell channel, made on first use. A channel a
    /// previous leg wrote into gets a CAN first: that leg may have been cut
    /// mid escape sequence, and the new shell's first bytes must not be
    /// swallowed by it (the terminal-side `resetInputParser`, in band).
    fn shell_stream(&mut self) -> Arc<Stream> {
        if self.shell.is_none() {
            let (stream, input) = moshpit_stream::open();
            let stream = Arc::new(stream);
            let pump = pump(stream.id(), input, self.me.clone());
            self.shell = Some(ShellChannel {
                stream,
                pump,
                used: false,
                written: 0,
            });
        }
        let shell = self.shell.as_mut().expect("made above");
        if shell.used {
            shell.stream.push(b"\x18");
            // Whatever a previous leg never echoed won't be now.
            shell.stream.mark(moshpit_stream::Mark {
                code: crate::tmux_leg::mark::ECHO_ACK,
                arg: i64::try_from(shell.written).unwrap_or(i64::MAX),
            });
        }
        shell.used = true;
        shell.stream.clone()
    }

    fn publish_shell_pane(&mut self) {
        // A multiplexer's panes (from before a degrade) are done.
        self.panes.clear();
        self.discard_pending(Discarded::PaneGone);
        let Some(shell) = &self.shell else {
            return;
        };
        let panes = vec![PaneInfo {
            pane: SHELL_PANE.to_owned(),
            stream: shell.stream.id(),
            active: true,
        }];
        if panes != self.current {
            self.current = panes.clone();
            self.attach += 1;
            let attach = self.attach;
            self.update(|view| {
                view.attach = attach;
                view.panes = panes.clone();
                view.tmux = None;
                view.tmux_view = None;
                view.herdr = None;
            });
            self.emit(EventKind::Panes { attach, panes });
            self.emit(EventKind::FirstFrame { attach });
        }
    }

    async fn teardown(&mut self, generation: u64, why: Teardown) {
        let Some(leg) = self.leg.take() else {
            return;
        };
        if leg.generation() != generation {
            self.leg = Some(leg);
            return;
        }
        let courtesy = matches!(why, Teardown::Disconnect);
        match leg {
            Leg::Shell { ssh, shell, .. } => {
                shell.stop();
                tokio::spawn(async move { ssh.close().await });
            }
            Leg::Tmux { ssh, tmux, .. } => {
                let (reply, handoff) = oneshot::channel();
                let _ = tmux.commands.send(TmuxCommand::Close { courtesy, reply });
                let bound = if courtesy {
                    Duration::from_secs(5)
                } else {
                    Duration::from_millis(500)
                };
                if let Ok(Ok(handoff)) = tokio::time::timeout(bound, handoff).await {
                    if let Some(selection) = &handoff.selection {
                        self.services
                            .memory
                            .save_tmux_selection(&self.spec.connection_id, selection);
                    }
                    // A replacement settles the debt; with none coming (a
                    // dead channel closed for good) it is paid at the end.
                    if why == Teardown::Reconnect || handoff.ledger.is_some() {
                        self.handoff = handoff;
                    }
                }
                tmux.abort();
                tokio::spawn(async move { ssh.close().await });
                if why != Teardown::Reconnect {
                    // No replacement is coming to paint over them.
                    self.panes.clear();
                    self.discard_pending(Discarded::SessionEnded);
                }
            }
            Leg::Mosh {
                mosh,
                server_pid,
                first_contact,
                reaped,
                ..
            } => {
                // herdr's plane first: nothing it holds reaches mosh after
                // mosh is asked what it never delivered, and what it holds
                // (typing behind a key or a switch, B686) is said with it.
                let held = self.stop_herdr_control().await;
                self.plane_held = false;
                // What waits behind a paste was for this shell: never the
                // next one's, never dropped unsaid (B689).
                let why_dropped = if why == Teardown::Reconnect {
                    Discarded::Reconnected
                } else {
                    Discarded::SessionEnded
                };
                self.drop_settled(&Dest::Shell, why_dropped);
                let now = self.now();
                self.show_held(now);
                // Keys this flow never delivered go with it, said (B618): a
                // reconnect's server is a fresh shell, which mustn't get
                // them, and the person must know they weren't sent.
                let (asked, answer) = oneshot::channel();
                let undelivered = if mosh.commands.send(MoshCommand::Undelivered(asked)).is_ok()
                    && let Ok(Ok(bytes)) =
                        tokio::time::timeout(Duration::from_millis(500), answer).await
                {
                    bytes
                } else {
                    0
                };
                let bytes = undelivered + held;
                if bytes > 0
                    && let Some(stream) = self.shell.as_ref().map(|shell| shell.stream.id())
                {
                    self.emit(EventKind::InputDiscarded {
                        stream,
                        bytes,
                        why: if why == Teardown::Reconnect {
                            Discarded::Reconnected
                        } else {
                            Discarded::SessionEnded
                        },
                    });
                }
                let _ = mosh.commands.send(MoshCommand::Close);
                // The person's end of a mosh + tmux session: the renderer
                // (mosh-server's tmux client) is taken out of the session
                // before the windows go back (B270).
                let renderer_server = server_pid
                    .filter(|_| why == Teardown::Disconnect && !reaped && self.control.is_some());
                // An end (a detach, the server's own end) hands the window
                // pins back at once over the control connection still open,
                // as the person's Disconnect does (B724, B588's hand-back):
                // nothing comes to inherit them, and the app may be killed
                // before the card closes.
                let hand_back = courtesy || why == Teardown::Ended;
                self.stop_control(hand_back, renderer_server).await;
                self.lend_to_control_waiting();
                // A server that never heard from us waits forever; one whose
                // session the person ended (or left for SSH) can never be
                // reattached — this app doesn't keep the key. One a reconnect
                // leaves behind is replaced by the server the redial starts:
                // stopped over the next connection (B195). The server's own
                // end leaves nothing to stop.
                if let Some(pid) = server_pid.filter(|_| !reaped) {
                    let debt = self.debt_renderer_server == Some(pid);
                    match server_at_teardown(why, first_contact, debt) {
                        ServerFate::Stop => self.owe(reap_mosh_server(
                            self.services.clone(),
                            self.spec.clone(),
                            self.timing.clone(),
                            vec![pid],
                        )),
                        ServerFate::Orphan => self.orphans.push(pid),
                        ServerFate::Leave => {}
                    }
                }
            }
            Leg::Herdr { ssh, herdr, .. } => {
                let (done, finished) = oneshot::channel();
                let _ = herdr.commands.send(HerdrCommand::Close { courtesy, done });
                let _ = tokio::time::timeout(Duration::from_secs(3), finished).await;
                herdr.abort();
                tokio::spawn(async move { ssh.close().await });
                if why != Teardown::Reconnect {
                    self.panes.clear();
                    self.discard_pending(Discarded::SessionEnded);
                }
            }
        }
    }

    /// mosh-servers a reconnect left behind, stopped over the connection
    /// just made (or one of their own for plain mosh): whatever the redial
    /// brought, the old server's session is gone for good (B195).
    fn reap_orphans(&mut self) {
        if self.orphans.is_empty() {
            return;
        }
        let pids = std::mem::take(&mut self.orphans);
        let ssh = self
            .leg
            .as_ref()
            .and_then(Leg::ssh)
            .or_else(|| self.control.as_ref().and_then(|c| c.ssh.as_ref()))
            .or_else(|| self.herdr_mosh.as_ref().and_then(|c| c.ssh.as_ref()))
            .cloned();
        let (services, spec, timing) = (
            self.services.clone(),
            self.spec.clone(),
            self.timing.clone(),
        );
        self.owe(async move {
            if let Some(ssh) = ssh {
                let commands: Vec<String> = pids.iter().map(|pid| reap_command(*pid)).collect();
                let ran = tokio::time::timeout(Duration::from_secs(8), async {
                    for command in &commands {
                        if !matches!(ssh.run(command).await, Ok(output) if output.closed) {
                            return false;
                        }
                    }
                    true
                })
                .await;
                if matches!(ran, Ok(true)) {
                    return;
                }
            }
            reap_mosh_server(services, spec, timing, pids).await;
        });
    }

    // MARK: mosh's control plane

    /// The sidecar beside a mosh session with tmux, on `ssh` (the
    /// bootstrap's connection, kept).
    fn start_control(
        &mut self,
        generation: u64,
        ssh: Ssh,
        server_pid: Option<i64>,
        layouts_before_renderer: Option<String>,
    ) {
        self.next_epoch += 1;
        let epoch = self.next_epoch;
        let grid = self.grid;
        self.control = Some(Control {
            generation,
            epoch,
            ssh: None,
            sidecar: None,
            dialling: true,
            failures: 0,
            preferred: None,
            handoff: std::mem::take(&mut self.handoff),
            renderer_pid: None,
            grid,
            layouts_before_renderer,
            ready: false,
            checking: false,
            holding: false,
            held: Vec::new(),
            hold: 0,
            dropped: false,
        });
        // The renderer is mosh's program: mosh-server's child.
        if let Some(pid) = server_pid {
            let me = self.me.clone();
            let probe = ssh.clone();
            tokio::spawn(async move {
                if let Some(child) = renderer_pid(&probe, pid).await {
                    let _ = me.send(Msg::RendererPid {
                        generation,
                        pid: child,
                    });
                }
            });
        }
        let me = self.me.clone();
        let timeout = self.timing.channel_timeout;
        let program = self.sidecar_program(None);
        tokio::spawn(async move {
            let result = sidecar_terminal(&ssh, timeout, program)
                .await
                .map(|pty| (ssh.clone(), pty));
            if result.is_err() {
                tokio::spawn(async move { ssh.close().await });
            }
            let _ = me.send(Msg::ControlUp { epoch, result });
        });
    }

    /// The control plane's connection and terminal are up: the sidecar
    /// starts on them.
    fn control_up(&mut self, epoch: u64, result: Result<(Ssh, Process), Failure>) {
        let mosh_live = matches!(self.leg, Some(Leg::Mosh { .. }));
        let current = self.control.as_ref().is_some_and(|c| c.epoch == epoch);
        let (ssh, pty) = match result {
            Ok((ssh, pty)) if current && mosh_live => (ssh, pty),
            Ok((ssh, _)) => {
                tokio::spawn(async move { ssh.close().await });
                return;
            }
            Err(failure) => {
                if current {
                    log::info!(
                        "session {}: no mosh control plane: {}",
                        self.id,
                        failure.log
                    );
                    if let Some(control) = self.control.as_mut() {
                        control.dialling = false;
                        control.failures += 1;
                    }
                    self.schedule_control_retry();
                }
                return;
            }
        };
        let pinned = self.pinned();
        let restore = self
            .services
            .memory
            .tmux_selection(&self.spec.connection_id);
        let tmux_command = self.spec.tmux_command().to_owned();
        let config = self.tmux_config();
        let attach_stall = self.timing.attach_stall;
        let me = self.me.clone();
        let clock = self.clock.clone();
        let Some(control) = self.control.as_mut() else {
            return;
        };
        control.dialling = false;
        let sidecar = TmuxLeg::spawn(
            TmuxStart {
                ssh: ssh.clone(),
                pty,
                tmux: tmux_command,
                config,
                grid: control.grid,
                restore,
                // Where the app last was, never a pick: the sidecar's.
                restore_chosen: false,
                inherited: std::mem::take(&mut control.handoff),
                attach_stall,
                // The sidecar's chatter is no proof that mosh is alive.
                inbound: Arc::new(AtomicU64::new(0)),
                pinned,
                sidecar: Some(SidecarStart {
                    layouts_before_renderer: control.layouts_before_renderer.take(),
                    renderer_pid: control.renderer_pid,
                    grid: Some(control.grid),
                }),
            },
            move || clock.now(),
            Box::new(move |note| {
                let _ = me.send(Msg::Sidecar { epoch, note });
            }),
        );
        control.ssh = Some(ssh);
        control.sidecar = Some(sidecar);
    }

    /// What the sidecar of `epoch` says.
    fn sidecar_note(&mut self, epoch: u64, note: TmuxNote) {
        if !self.control.as_ref().is_some_and(|c| c.epoch == epoch) {
            return;
        }
        match note {
            TmuxNote::Lost(reason) => {
                // mosh carries on; only the control plane is rebuilt.
                log::info!("session {}: the mosh sidecar dropped: {reason}", self.id);
                let Some(control) = self.control.as_mut() else {
                    return;
                };
                let sidecar = control.sidecar.take();
                let ssh = control.ssh.take();
                control.ready = false;
                control.failures += 1;
                let me = self.me.clone();
                tokio::spawn(async move {
                    if let Some(sidecar) = sidecar {
                        let (reply, handoff) = oneshot::channel();
                        let _ = sidecar.commands.send(TmuxCommand::Close {
                            courtesy: false,
                            reply,
                        });
                        // What it owes the server goes to the next sidecar.
                        if let Ok(Ok(handoff)) =
                            tokio::time::timeout(Duration::from_millis(500), handoff).await
                        {
                            let _ = me.send(Msg::SidecarHandoff { epoch, handoff });
                        }
                        sidecar.abort();
                    }
                    if let Some(ssh) = ssh {
                        ssh.close().await;
                    }
                });
                self.schedule_control_retry();
            }
            TmuxNote::Tree(tmux) => {
                if let Some(control) = self.control.as_mut() {
                    control.preferred =
                        tmux.snapshot
                            .active_session_id
                            .clone()
                            .map(|id| SidecarReturn {
                                name: tmux.snapshot.sessions.get(&id).map(|s| s.name.clone()),
                                id,
                            });
                    control.failures = 0;
                }
                let ready = tmux.snapshot.is_attached || tmux.no_session;
                self.update(|view| {
                    view.tmux = Some(tmux.snapshot.clone());
                    view.tmux_view = Some(*tmux);
                });
                self.forward_copy_mode();
                self.emit(EventKind::TreeChanged);
                self.try_deliver();
                if let Some(control) = self.control.as_mut() {
                    control.ready = ready;
                }
                self.release_control_held();
            }
            TmuxNote::Activity(pane) => self.emit(EventKind::PaneActivity { pane }),
            TmuxNote::Bell(pane) => self.emit(EventKind::PaneBell { pane }),
            TmuxNote::Snapshot { pane, rows } => self.emit(EventKind::PaneSnapshot { pane, rows }),
            TmuxNote::HooksUpdated => self.emit(EventKind::AgentHooksUpdated),
            TmuxNote::AttachStalled => {
                self.update(|view| view.attach_stalled = true);
                self.emit(EventKind::AttachStalled);
            }
            // The sidecar renders nothing: no panes, no frames, no views.
            TmuxNote::Opened { .. }
            | TmuxNote::Panes(_)
            | TmuxNote::View { .. }
            | TmuxNote::FirstFrame => {}
        }
    }

    /// Redial the control plane after the backoff its failures earned.
    fn schedule_control_retry(&mut self) {
        let Some(control) = self.control.as_mut() else {
            return;
        };
        if control.dialling {
            return;
        }
        control.dialling = true;
        let delay = control_retry_delay(control.failures);
        let epoch = control.epoch;
        let me = self.me.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = me.send(Msg::ControlRetry { epoch });
        });
    }

    /// A fresh connection and terminal for the next sidecar.
    fn redial_control(&mut self) {
        let mosh_live = matches!(self.leg, Some(Leg::Mosh { .. })) && self.life.state().is_live();
        let foreground = self.life.is_foreground();
        let Some(control) = self.control.as_mut() else {
            return;
        };
        control.dialling = false;
        if !mosh_live || control.sidecar.is_some() {
            return;
        }
        if !foreground {
            // The foreground return asks again.
            return;
        }
        self.next_epoch += 1;
        control.epoch = self.next_epoch;
        control.dialling = true;
        let epoch = control.epoch;
        let preferred = control.preferred.clone();
        let program = self.sidecar_program(preferred.as_ref());
        let (services, spec, timing, me) = (
            self.services.clone(),
            self.spec.clone(),
            self.timing.clone(),
            self.me.clone(),
        );
        tokio::spawn(async move {
            let prompt: Arc<dyn HostKeyPrompt> = Arc::new(DenyAll);
            let result = match dial(&services, &spec, &timing, prompt).await {
                Ok(ssh) => match sidecar_terminal(&ssh, timing.channel_timeout, program).await {
                    Ok(pty) => Ok((ssh, pty)),
                    Err(failure) => {
                        ssh.close().await;
                        Err(failure)
                    }
                },
                Err(failure) => Err(failure),
            };
            let _ = me.send(Msg::ControlUp { epoch, result });
        });
    }

    /// The app came back, the path moved, or the keepalive ticked: a
    /// control plane that is gone (its connection died while mosh roamed
    /// on) is brought back now. One whose connection is still open is asked
    /// whether it answers (B401): a connection a NAT forgot, or left on the
    /// old path, is never closed — it only goes quiet, and the sidecar's own
    /// watchdog took three silent beats to notice, and only while a command
    /// was pending; what was asked meanwhile was lost with it. `hold`: the
    /// person's commands wait for the answer (there is reason to doubt).
    fn revive_control(&mut self, hold: bool) {
        let me = self.me.clone();
        let Some(control) = self.control.as_mut() else {
            return;
        };
        if control.dialling {
            return;
        }
        if control.checking {
            control.holding |= hold;
            return;
        }
        let Some(ssh) = control.ssh.clone().filter(|ssh| !ssh.is_closed()) else {
            self.rebuild_control();
            return;
        };
        control.checking = true;
        control.holding = hold;
        let epoch = control.epoch;
        tokio::spawn(async move {
            let alive = ssh.ping(CONTROL_CHECK).await;
            let _ = me.send(Msg::ControlChecked { epoch, alive });
        });
    }

    /// The check is answered: commands held for it go out, or a control
    /// plane that stayed silent is replaced now.
    fn control_checked(&mut self, epoch: u64, alive: bool) {
        let Some(control) = self.control.as_mut() else {
            return;
        };
        // No other check can have started meanwhile; a newer epoch means
        // the plane was rebuilt while this one was out: its answer is moot.
        control.checking = false;
        control.holding = false;
        if control.epoch != epoch {
            return;
        }
        if alive {
            self.release_control_held();
            return;
        }
        log::info!(
            "session {}: the mosh control plane stopped answering; a new one now",
            self.id
        );
        self.rebuild_control();
    }

    /// The control plane is gone: the next sidecar, on a fresh connection,
    /// now (no backoff: nothing failed to come up yet).
    fn rebuild_control(&mut self) {
        let Some(control) = self.control.as_mut() else {
            return;
        };
        if let Some(sidecar) = control.sidecar.take() {
            sidecar.abort();
        }
        if let Some(ssh) = control.ssh.take() {
            tokio::spawn(async move { ssh.close().await });
        }
        control.ready = false;
        control.failures = 0;
        self.redial_control();
    }

    /// A tmux command over mosh goes to the sidecar. While the control
    /// plane is in doubt (a check out on the return or a path change, a
    /// redial on its way, a sidecar not ready yet), the person's
    /// commands wait for it, at most
    /// [`CONTROL_HOLD`] (B401): sent meanwhile, they went to a connection
    /// that would never answer, or nowhere. What only matters now (a
    /// scroll's start, a pane's reveal) isn't held.
    fn control_action(&mut self, action: TmuxAction) {
        let me = self.me.clone();
        let Some(control) = self.control.as_mut() else {
            return;
        };
        let in_doubt = control.holding || !control.ready || control.sidecar.is_none();
        if in_doubt && !is_momentary(&action) {
            if control.held.is_empty() {
                control.hold += 1;
                let hold = control.hold;
                tokio::spawn(async move {
                    tokio::time::sleep(CONTROL_HOLD).await;
                    let _ = me.send(Msg::ControlHoldExpired { hold });
                });
            }
            control.held.push(action);
            return;
        }
        if let Some(sidecar) = control.sidecar.as_ref() {
            let _ = sidecar.commands.send(TmuxCommand::Action(action));
        }
    }

    /// The control plane answers again (its check, or the next sidecar
    /// ready): the held commands go out in order; commands dropped while it
    /// was down are said now.
    fn release_control_held(&mut self) {
        let Some(control) = self.control.as_mut() else {
            return;
        };
        if control.holding || !control.ready {
            return;
        }
        let Some(sidecar) = control.sidecar.as_ref() else {
            return;
        };
        // What waited for a leg while the connection was dialled again
        // came first (B483).
        let held = std::mem::take(&mut self.tmux_held)
            .into_iter()
            .chain(std::mem::take(&mut control.held));
        for action in held {
            let _ = sidecar.commands.send(TmuxCommand::Action(action));
        }
        let dropped = std::mem::take(&mut control.dropped) | std::mem::take(&mut self.tmux_dropped);
        if dropped {
            self.emit(EventKind::TmuxActionsDropped);
        }
    }

    /// A tmux command from the person. One no attach can carry now (the
    /// connection is being dialled again, or a return still asks whether
    /// its socket lived) waits for the next, in order, at most
    /// [`CONTROL_HOLD`] (B483): it went into the dead socket or nowhere,
    /// and the window, the rename or the close never happened, unsaid. A
    /// pick made then is where the next attach lands instead (B405); what
    /// only matters now isn't kept.
    fn tmux_action(&mut self, action: TmuxAction) {
        let waits = match &self.leg {
            Some(Leg::Tmux { .. }) => self.life.resume_pending() || !self.tmux_held.is_empty(),
            Some(_) => false,
            None => self.spec.multiplexer == Multiplexer::Tmux,
        };
        let pick = crate::tmux_leg::pick_selection(&action, None);
        if waits && pick.is_none() && kept_for_an_attach(&action) {
            self.hold_tmux(action);
            return;
        }
        match &self.leg {
            Some(Leg::Tmux { tmux, .. }) => {
                let _ = tmux.commands.send(TmuxCommand::Action(action));
            }
            Some(Leg::Mosh { .. }) => self.control_action(action),
            // No leg while the connection is redialled: a notification
            // tapped then picks here. The next attach lands on it (B405).
            _ => {
                if let Some(pick) = pick {
                    self.tmux_pick = Some(pick);
                }
            }
        }
    }

    fn hold_tmux(&mut self, action: TmuxAction) {
        if self.tmux_held.is_empty() {
            self.tmux_hold += 1;
            let hold = self.tmux_hold;
            let me = self.me.clone();
            tokio::spawn(async move {
                tokio::time::sleep(CONTROL_HOLD).await;
                let _ = me.send(Msg::TmuxHoldExpired { hold });
            });
        }
        self.tmux_held.push(action);
    }

    /// The commands held since `hold` waited long enough: dropped rather
    /// than run late, and said once an attach is back (B483).
    fn tmux_hold_expired(&mut self, hold: u64) {
        if self.tmux_hold != hold || self.tmux_held.is_empty() {
            return;
        }
        log::info!(
            "session {}: {} tmux command(s) dropped: no attach carried them in time",
            self.id,
            self.tmux_held.len()
        );
        self.tmux_held.clear();
        self.tmux_dropped = true;
    }

    /// An SSH tmux attach is up on a live session: what waited for one
    /// goes out, in order, and what waited too long is said (B483). mosh's
    /// control plane takes its share when it is ready
    /// ([`Self::release_control_held`]).
    fn release_tmux_held(&mut self) {
        let Some(Leg::Tmux { tmux, .. }) = &self.leg else {
            return;
        };
        if !self.life.state().is_live() || self.current.is_empty() {
            return;
        }
        for action in std::mem::take(&mut self.tmux_held) {
            let _ = tmux.commands.send(TmuxCommand::Action(action));
        }
        if std::mem::take(&mut self.tmux_dropped) {
            self.emit(EventKind::TmuxActionsDropped);
        }
    }

    /// The commands held since `hold` waited long enough: dropped, and said
    /// once the plane is back.
    fn control_hold_expired(&mut self, hold: u64) {
        let Some(control) = self.control.as_mut() else {
            return;
        };
        if control.hold != hold || control.held.is_empty() {
            return;
        }
        log::info!(
            "session {}: {} tmux command(s) dropped: the mosh control plane didn't answer in time",
            self.id,
            control.held.len()
        );
        control.held.clear();
        control.dropped = true;
    }

    /// The sidecar's terminal program: the attach chain back to `preferred`
    /// (a rebuild) or onto the most recent session, then the channel's own
    /// shell.
    fn sidecar_program(&self, preferred: Option<&SidecarReturn>) -> String {
        moshpit_tmux::boot::sidecar_attach_program(
            self.spec.tmux_command(),
            preferred,
            &crate::attempt::shell_after(&self.spec),
        )
    }

    /// The mosh leg is going: its control plane with it, handing back what
    /// the sidecar holds on the server when the person disconnects.
    ///
    /// `renderer_server`: the person ended the session, and this is its
    /// mosh-server, whose tmux client — the phone's renderer — is taken out
    /// of the session first, over the control plane's connection; with that
    /// connection gone, the pins the session still owes are paid that way
    /// at the end (B270).
    async fn stop_control(&mut self, courtesy: bool, renderer_server: Option<i64>) {
        let Some(mut control) = self.control.take() else {
            return;
        };
        if courtesy && let Some(server) = renderer_server {
            let command = moshpit_tmux::boot::renderer_detach_by_server_command(
                self.spec.tmux_command(),
                server,
            );
            let detached = match control.ssh.as_ref().filter(|ssh| !ssh.is_closed()) {
                Some(ssh) => matches!(
                    tokio::time::timeout(Duration::from_secs(3), ssh.run(&command)).await,
                    Ok(Ok(output)) if output.closed
                ),
                None => false,
            };
            if !detached {
                self.debt_renderer_server = Some(server);
            }
        }
        if let Some(sidecar) = control.sidecar.take() {
            let (reply, handoff) = oneshot::channel();
            let _ = sidecar
                .commands
                .send(TmuxCommand::Close { courtesy, reply });
            let bound = if courtesy {
                Duration::from_secs(5)
            } else {
                Duration::from_millis(500)
            };
            if let Ok(Ok(handoff)) = tokio::time::timeout(bound, handoff).await {
                if let Some(selection) = &handoff.selection {
                    self.services
                        .memory
                        .save_tmux_selection(&self.spec.connection_id, selection);
                }
                if handoff.ledger.is_some() {
                    self.handoff = handoff;
                }
            }
            sidecar.abort();
        } else if control.handoff.ledger.is_some() {
            // A sidecar never rebuilt still owes what the last one held.
            self.handoff = std::mem::take(&mut control.handoff);
        }
        if let Some(ssh) = control.ssh.take() {
            tokio::spawn(async move { ssh.close().await });
        }
    }

    // MARK: herdr's control plane over mosh

    /// herdr's control plane beside the mosh leg of `generation`, on `ssh`
    /// (the bootstrap's connection, kept).
    fn start_herdr_control(
        &mut self,
        generation: u64,
        ssh: Ssh,
        herdr: crate::attempt::MoshHerdr,
        shell: Arc<Stream>,
        painted: Arc<AtomicU64>,
    ) {
        let Some(Leg::Mosh { mosh, .. }) = &self.leg else {
            return;
        };
        let config = PlaneConfig::new(
            &self.spec.connection_id,
            self.spec.herdr_path.clone(),
            herdr.renderer_key,
            herdr.push,
        );
        let me = self.me.clone();
        let clock = self.clock.clone();
        let leg = HerdrMoshLeg::spawn(
            HerdrMoshStart {
                ssh: ssh.clone(),
                mosh: mosh.commands.clone(),
                shell,
                painted,
                config,
                foreground: self.life.is_foreground(),
            },
            move || clock.now(),
            Box::new(move |note| {
                let _ = me.send(Msg::HerdrMosh { generation, note });
            }),
        );
        self.next_epoch += 1;
        self.herdr_mosh = Some(HerdrMoshControl {
            generation,
            leg,
            ssh: Some(ssh),
            epoch: self.next_epoch,
            dialling: false,
            failures: 0,
        });
        self.lend_to_control_waiting();
    }

    fn herdr_mosh_note(&mut self, generation: u64, note: HerdrMoshNote) {
        if let HerdrMoshNote::Answer { token, answer } = note {
            // An answer is owed whoever asks: even a retired plane's.
            self.emit(EventKind::HerdrAnswer { token, answer });
            return;
        }
        if !self
            .herdr_mosh
            .as_ref()
            .is_some_and(|c| c.generation == generation)
        {
            return;
        }
        match note {
            HerdrMoshNote::View(herdr) => {
                self.update(|view| view.herdr = Some(*herdr));
                self.emit(EventKind::TreeChanged);
            }
            HerdrMoshNote::AgentHooksUpdated => self.emit(EventKind::AgentHooksUpdated),
            HerdrMoshNote::InputHeld(held) => {
                self.plane_held = held;
                let now = self.now();
                self.show_held(now);
            }
            HerdrMoshNote::Snapshot { pane, rows } => {
                self.emit(EventKind::PaneSnapshot { pane, rows })
            }
            HerdrMoshNote::InputDropped { bytes } => {
                let stream = self.shell.as_ref().map_or(0, |shell| shell.stream.id());
                self.emit(EventKind::InputDiscarded {
                    stream,
                    bytes,
                    why: Discarded::NotAttached,
                });
            }
            HerdrMoshNote::ConnectionLost => {
                // mosh carries on; only the control plane is redialled.
                log::info!(
                    "session {}: herdr's control connection over mosh dropped",
                    self.id
                );
                if let Some(control) = self.herdr_mosh.as_mut() {
                    if let Some(ssh) = control.ssh.take() {
                        tokio::spawn(async move { ssh.close().await });
                    }
                    control.failures += 1;
                }
                self.schedule_herdr_retry();
            }
            HerdrMoshNote::Answer { .. } => unreachable!("handled above"),
        }
    }

    fn schedule_herdr_retry(&mut self) {
        let Some(control) = self.herdr_mosh.as_mut() else {
            return;
        };
        if control.dialling {
            return;
        }
        control.dialling = true;
        let delay = control_retry_delay(control.failures);
        let epoch = control.epoch;
        let me = self.me.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = me.send(Msg::HerdrControlRetry { epoch });
        });
    }

    /// A fresh connection for herdr's control plane.
    fn redial_herdr_control(&mut self) {
        let mosh_live = matches!(self.leg, Some(Leg::Mosh { .. })) && self.life.state().is_live();
        let foreground = self.life.is_foreground();
        let Some(control) = self.herdr_mosh.as_mut() else {
            return;
        };
        control.dialling = false;
        if !mosh_live || control.ssh.is_some() || !foreground {
            // In the background the foreground return asks again.
            return;
        }
        self.next_epoch += 1;
        control.epoch = self.next_epoch;
        control.dialling = true;
        let epoch = control.epoch;
        let (services, spec, timing, me) = (
            self.services.clone(),
            self.spec.clone(),
            self.timing.clone(),
            self.me.clone(),
        );
        tokio::spawn(async move {
            let prompt: Arc<dyn HostKeyPrompt> = Arc::new(DenyAll);
            let result = dial(&services, &spec, &timing, prompt).await;
            let _ = me.send(Msg::HerdrControlUp { epoch, result });
        });
    }

    fn herdr_control_up(&mut self, epoch: u64, result: Result<Ssh, Failure>) {
        let current = self.herdr_mosh.as_ref().is_some_and(|c| c.epoch == epoch)
            && matches!(self.leg, Some(Leg::Mosh { .. }));
        match result {
            Ok(ssh) if current => {
                if let Some(control) = self.herdr_mosh.as_mut() {
                    control.dialling = false;
                    control.failures = 0;
                    control.ssh = Some(ssh.clone());
                    let _ = control.leg.commands.send(HerdrMoshCommand::Connection(ssh));
                }
            }
            Ok(ssh) => {
                tokio::spawn(async move { ssh.close().await });
            }
            Err(failure) => {
                if current {
                    log::info!(
                        "session {}: no herdr control connection: {}",
                        self.id,
                        failure.log
                    );
                    if let Some(control) = self.herdr_mosh.as_mut() {
                        control.dialling = false;
                        control.failures += 1;
                    }
                    self.schedule_herdr_retry();
                }
            }
        }
    }

    /// The app came back, or the path moved: a control connection that is
    /// gone is brought back now.
    fn revive_herdr_control(&mut self) {
        let Some(control) = self.herdr_mosh.as_mut() else {
            return;
        };
        if control.dialling {
            return;
        }
        if let Some(ssh) = &control.ssh {
            if !ssh.is_closed() {
                return;
            }
            let _ = control.ssh.take();
        }
        control.failures = 0;
        self.redial_herdr_control();
    }

    /// The mosh leg is going: its herdr control plane with it. Says how many
    /// bytes of typing the plane held that never reached mosh.
    async fn stop_herdr_control(&mut self) -> usize {
        let Some(control) = self.herdr_mosh.take() else {
            return 0;
        };
        let (done, closed) = oneshot::channel();
        let _ = control.leg.commands.send(HerdrMoshCommand::Close(done));
        let held = tokio::time::timeout(Duration::from_millis(500), closed)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(0);
        control.leg.abort();
        if let Some(ssh) = control.ssh {
            tokio::spawn(async move { ssh.close().await });
        }
        held
    }

    // MARK: What the pane tasks say

    fn tmux_note(&mut self, generation: u64, note: TmuxNote) {
        let current =
            matches!(&self.leg, Some(Leg::Tmux { generation: g, .. }) if *g == generation);
        if let TmuxNote::Lost(reason) = note {
            log::info!("session {}: tmux lost: {reason}", self.id);
            let now = self.now();
            self.life.lost(generation, now);
            return;
        }
        if !current {
            return;
        }
        match note {
            TmuxNote::Opened {
                pane,
                stream,
                input,
            } => self.open_pane(pane, stream, input),
            TmuxNote::Panes(panes) => self.multiplexer_panes(panes),
            TmuxNote::View { stream, action } => self.emit(EventKind::View { stream, action }),
            TmuxNote::Tree(tmux) => {
                self.update(|view| {
                    view.tmux = Some(tmux.snapshot.clone());
                    view.tmux_view = Some(*tmux);
                });
                self.emit(EventKind::TreeChanged);
            }
            TmuxNote::FirstFrame => self.first_frame(),
            TmuxNote::Activity(pane) => self.emit(EventKind::PaneActivity { pane }),
            TmuxNote::Bell(pane) => self.emit(EventKind::PaneBell { pane }),
            TmuxNote::Snapshot { pane, rows } => self.emit(EventKind::PaneSnapshot { pane, rows }),
            TmuxNote::HooksUpdated => self.emit(EventKind::AgentHooksUpdated),
            TmuxNote::AttachStalled => {
                self.update(|view| view.attach_stalled = true);
                self.emit(EventKind::AttachStalled);
            }
            TmuxNote::Lost(_) => unreachable!("handled above"),
        }
    }

    /// Where the next tmux attach lands: the person's pick made while no
    /// leg carried it (B405), what it doesn't name filled from where the
    /// last attach was; else where the last attach was; else where the app
    /// last left this connection.
    fn next_tmux_restore(&mut self) -> Option<Selection> {
        let handed = self.handoff.selection.take();
        let Some(mut pick) = self.tmux_pick.take() else {
            return handed.or_else(|| {
                self.services
                    .memory
                    .tmux_selection(&self.spec.connection_id)
            });
        };
        if let Some(handed) = handed {
            // A pane since closed falls back to the window the person was in.
            if pick.session.is_empty() {
                pick.session = handed.session;
            }
            if pick.pane.is_some() && pick.window.is_none() {
                pick.window = handed.window;
            }
        }
        Some(pick)
    }

    /// The herdr server is seen on the person's pick, the switch landed:
    /// nothing to make again (B397). Seen means a read of the server says
    /// so (B406): the client puts a pick in its own tree at once, and lets
    /// the switch go when its hold runs out or its command fails, so on a
    /// connection that died under it the pick looked made with nothing
    /// sent.
    fn herdr_pick_seen(&mut self, view: &HerdrView) {
        if !view.pane_switch_in_flight
            && view.server_focus.is_some()
            && view.server_focus == self.herdr_pick
        {
            self.herdr_pick = None;
        }
    }

    /// What a new herdr leg is told first: the person's pick the last
    /// connection never carried, once (B397).
    fn herdr_repick(&mut self) -> Option<HerdrCommand> {
        self.herdr_pick.take().map(|pane| HerdrCommand::Action {
            action: HerdrAction::SelectPane(pane),
            token: None,
        })
    }

    fn herdr_note(&mut self, generation: u64, note: HerdrNote) {
        let current =
            matches!(&self.leg, Some(Leg::Herdr { generation: g, .. }) if *g == generation);
        if let HerdrNote::Lost(reason) = note {
            log::info!("session {}: herdr lost: {reason}", self.id);
            let now = self.now();
            self.life.lost(generation, now);
            return;
        }
        if let HerdrNote::Answer { token, answer } = note {
            // An answer is owed whoever asks: even a retired leg's.
            self.emit(EventKind::HerdrAnswer { token, answer });
            return;
        }
        if !current {
            return;
        }
        match note {
            HerdrNote::Opened {
                pane,
                stream,
                input,
            } => self.open_pane(pane, stream, input),
            HerdrNote::Panes(panes) => self.multiplexer_panes(panes),
            HerdrNote::View(herdr) => {
                self.herdr_pick_seen(&herdr);
                self.update(|view| view.herdr = Some(*herdr));
                self.emit(EventKind::TreeChanged);
            }
            HerdrNote::FirstFrame => self.first_frame(),
            HerdrNote::AgentHooksUpdated => self.emit(EventKind::AgentHooksUpdated),
            HerdrNote::Snapshot { pane, rows } => self.emit(EventKind::PaneSnapshot { pane, rows }),
            HerdrNote::InputDropped { stream, bytes } => self.emit(EventKind::InputDiscarded {
                stream,
                bytes,
                why: Discarded::NotAttached,
            }),
            HerdrNote::Lost(_) | HerdrNote::Answer { .. } => unreachable!("handled above"),
        }
    }

    fn open_pane(&mut self, pane: String, stream: Arc<Stream>, input: Input) {
        let id = stream.id();
        let pump = pump(id, input, self.me.clone());
        self.panes.insert(
            id,
            PaneChannel {
                pane,
                attach: self.attach,
                stream,
                pump,
            },
        );
    }

    /// The current attach's panes changed: tell the UI, and send what was
    /// waiting for a pane that is now there. A pane absent once the attach
    /// lists its panes is gone: what waited for it is dropped and said so.
    fn multiplexer_panes(&mut self, panes: Vec<PaneInfo>) {
        self.current = panes.clone();
        let attach = self.attach;
        self.update(|view| {
            view.attach = attach;
            view.panes = panes.clone();
        });
        self.emit(EventKind::Panes {
            attach,
            panes: panes.clone(),
        });
        if panes.is_empty() {
            return;
        }
        let waiting: Vec<String> = self.pending.keys().cloned().collect();
        for pane in waiting {
            let Some(queued) = self.pending.remove(&pane) else {
                continue;
            };
            if panes.iter().any(|info| info.pane == pane) {
                for (stream, typed) in queued {
                    self.send_or_wait(Dest::Pane(pane.clone()), stream, typed, 0);
                }
            } else {
                for (stream, typed) in queued {
                    self.emit(EventKind::InputDiscarded {
                        stream,
                        bytes: typed.len(),
                        why: Discarded::PaneGone,
                    });
                }
            }
        }
        self.try_deliver();
    }

    fn first_frame(&mut self) {
        let attach = self.attach;
        self.emit(EventKind::FirstFrame { attach });
        // The previous attach's terminals have nothing left to stand in
        // for.
        self.panes.retain(|_, channel| channel.attach == attach);
    }

    fn mosh_note(&mut self, generation: u64, note: MoshNote) {
        let now = self.now();
        let Some(Leg::Mosh {
            generation: current,
            server_pid,
            first_contact,
            return_path_reported,
            reaped,
            ..
        }) = &mut self.leg
        else {
            return;
        };
        if *current != generation {
            return;
        }
        match note {
            MoshNote::FirstContact => *first_contact = true,
            MoshNote::ReturnPathDead {
                first_contact: heard,
            } => {
                let reap = (!heard && !*reaped).then_some(*server_pid).flatten();
                if reap.is_some() {
                    *reaped = true;
                }
                let first_report = !*return_path_reported;
                *return_path_reported = true;
                if let Some(pid) = reap {
                    self.owe(reap_mosh_server(
                        self.services.clone(),
                        self.spec.clone(),
                        self.timing.clone(),
                        vec![pid],
                    ));
                    // The flow has no server now: a new one at the next
                    // path change or return (B721).
                    self.life.mosh_server_reaped(generation);
                }
                if first_report {
                    self.update(|view| view.return_path_dead = true);
                    self.emit(EventKind::ReturnPathDead);
                }
            }
            // Heard again (B400): the banner goes, and the next outage is
            // reported — the latch used to stay set for the leg's life, so a
            // banner stayed up on a working session, and after a dismiss a
            // second outage showed none.
            MoshNote::ReturnPathAlive => {
                let reported = std::mem::take(return_path_reported);
                if reported {
                    self.update(|view| view.return_path_dead = false);
                    self.emit(EventKind::ReturnPathAlive);
                }
            }
            MoshNote::SessionEnded => self.life.ended(generation, EndReason::MoshServerEnded, now),
            // The loop stopped (its Finished follows): dial again — a new
            // mosh-server, tmux attached again (B402).
            MoshNote::ServerGone => {
                log::info!("mosh: mosh-server is gone; dialing again");
                self.life.lost(generation, now);
            }
            MoshNote::Metrics { srtt_ms, roaming } => {
                self.transport_roaming = roaming;
                // Typing held past a second shows as roaming too (B689).
                let roaming = roaming
                    || self
                        .view
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .input_held;
                self.update(|view| {
                    view.mosh_srtt_ms = Some(srtt_ms);
                    view.mosh_roaming = roaming;
                });
                self.emit(EventKind::MoshMetrics { srtt_ms, roaming });
            }
            MoshNote::Diagnostics(diagnostics) => {
                self.update(|view| view.mosh_diagnostics = Some(diagnostics.clone()));
                self.emit(EventKind::MoshDiagnostics(diagnostics));
            }
            MoshNote::Finished => {
                // Closed by us, or after the server's end, are both handled
                // where they happen; a loop that stopped on its own is lost.
                if self.life.live() == Some(Transport::Mosh) {
                    self.life.lost(generation, now);
                }
            }
        }
    }

    // MARK: Input

    fn input(&mut self, stream: StreamId, event: InputEvent) {
        let bar = matches!(event, InputEvent::Bar(_));
        let typed = match event {
            InputEvent::Resize { cols, rows } => {
                // Behind keys held for the return's answer, it waits with
                // them (V31, B529).
                if self.held.iter().any(|kept| kept.stream() == stream) {
                    self.held.push(Kept::Resize(stream, cols, rows));
                    return;
                }
                self.resize_in_turn(stream, cols, rows);
                return;
            }
            InputEvent::Bytes(bytes) | InputEvent::Bar(bytes) => {
                if bytes.is_empty() {
                    return;
                }
                if let Some(shell) = self
                    .shell
                    .as_mut()
                    .filter(|shell| shell.stream.id() == stream)
                {
                    shell.written += bytes.len() as u64;
                }
                Typed::Bytes(bytes)
            }
            // Not stream bytes: `written` doesn't count it.
            InputEvent::Paste(text) if text.is_empty() => return,
            InputEvent::Paste(text) => Typed::Paste(text),
        };
        if !self.life.state().is_live() {
            if bar {
                // A tap on the shortcut bar under the veil is not the
                // person's typing to deliver later: a stray one must not
                // land in an agent's prompt after the reconnect (B476).
                self.emit(EventKind::InputDiscarded {
                    stream,
                    bytes: typed.len(),
                    why: Discarded::NotLive,
                });
            } else {
                self.not_live(stream, typed);
            }
            return;
        }
        // Back from the background, the transport not yet heard from: wait
        // for its answer rather than write into a socket that may have died
        // during the suspension — a paste as well, behind the keys typed
        // before it (B361).
        if self.life.resume_pending() {
            self.held.push(Kept::Typed(stream, typed, bar));
            return;
        }
        self.route(stream, typed);
    }

    /// Typed while the session isn't live. Under a reconnect's veil it
    /// follows B396's rule (B476, the rest of B322): a multiplexer pane's
    /// keys and pastes wait for the pane, which comes back with the redial;
    /// a plain shell's are not sent, the redial being a new shell, and that
    /// is said. Before the first connect, or once the session failed or
    /// ended, nothing is sent.
    fn not_live(&mut self, stream: StreamId, typed: Typed) {
        let reconnecting = self.life.state() == SessionState::Reconnecting;
        match self.panes.get(&stream).map(|channel| channel.pane.clone()) {
            Some(pane) if reconnecting => {
                self.pending.entry(pane).or_default().push((stream, typed))
            }
            _ => self.emit(EventKind::InputDiscarded {
                stream,
                bytes: typed.len(),
                why: if reconnecting {
                    Discarded::Reconnected
                } else {
                    Discarded::NotLive
                },
            }),
        }
    }

    /// A size change, in its turn: behind keys that wait behind a paste it
    /// waits with them (V31, B528): the program sees the new size after the
    /// keys typed before it, not before.
    fn resize_in_turn(&mut self, stream: StreamId, cols: u16, rows: u16) {
        if let Some(settling) = self
            .dest_of(stream)
            .and_then(|dest| self.settling.get_mut(&dest))
            .filter(|settling| !settling.queued.is_empty())
        {
            settling.queued.push(Queued::Resize(stream, cols, rows));
            return;
        }
        self.resize(stream, cols, rows);
    }

    /// The place a stream's typing goes to, as its paste settles are kept.
    fn dest_of(&self, stream: StreamId) -> Option<Dest> {
        if self.shell.as_ref().map(|s| s.stream.id()) == Some(stream) {
            return Some(Dest::Shell);
        }
        self.panes
            .get(&stream)
            .map(|channel| Dest::Pane(channel.pane.clone()))
    }

    fn route(&mut self, stream: StreamId, typed: Typed) {
        if let Some(shell) = self.shell.as_ref().filter(|s| s.stream.id() == stream) {
            let end = shell.written;
            self.send_or_wait(Dest::Shell, stream, typed, end);
            return;
        }
        let Some(pane) = self.panes.get(&stream).map(|channel| channel.pane.clone()) else {
            return;
        };
        let attached = self.current.iter().any(|info| info.pane == pane);
        match &self.leg {
            // herdr's client holds a pane's input until its attach paints,
            // and only for the pane the person picked (B185).
            Some(Leg::Herdr { .. }) => self.send_or_wait(Dest::Pane(pane), stream, typed, 0),
            Some(Leg::Tmux { .. }) if attached => {
                self.send_or_wait(Dest::Pane(pane), stream, typed, 0)
            }
            _ => self.pending.entry(pane).or_default().push((stream, typed)),
        }
    }

    /// Typing for a place: behind a paste sent there a moment ago, it
    /// waits for the paste to have had its time, in order (B485).
    fn send_or_wait(&mut self, dest: Dest, stream: StreamId, typed: Typed, end: u64) {
        if let Some(settling) = self.settling.get_mut(&dest) {
            settling.queued.push(Queued::Typed(stream, typed, end));
            return;
        }
        self.send_now(&dest, stream, typed, end);
    }

    /// Sends to a place now; what comes after a paste waits behind it.
    fn send_now(&mut self, dest: &Dest, stream: StreamId, typed: Typed, end: u64) {
        let paste = matches!(typed, Typed::Paste(_));
        // Over herdr and mosh the plane says when the paste is in the tty
        // (B687): it goes through the plane's lane, behind what it holds.
        let through_plane = paste
            && *dest == Dest::Shell
            && matches!(self.leg, Some(Leg::Mosh { .. }))
            && self.herdr_mosh.is_some();
        let (said, landing) = if through_plane {
            let (said, landing) = oneshot::channel();
            (Some(said), Some(landing))
        } else {
            (None, None)
        };
        match dest {
            Dest::Shell => self.send_to_shell(stream, typed, end, said),
            Dest::Pane(pane) => self.send_to_pane(pane, stream, typed),
        }
        if paste {
            let now = self.now();
            self.pastes += 1;
            let paste = self.pastes;
            let landed = match landing {
                Some(landing) => {
                    self.await_paste(dest, paste, async move { landing.await.unwrap_or(false) });
                    false
                }
                None => !self.ask_paste_ack(dest, paste),
            };
            self.settling.insert(
                dest.clone(),
                Settling {
                    paste,
                    since: now,
                    until: now + crate::spec::ms(PASTE_SETTLE),
                    landed,
                    queued: Vec::new(),
                },
            );
        }
    }

    /// Over plain mosh, asks for mosh-server's ack of paste `paste` (B525):
    /// true if it was asked (what follows the paste then waits for it).
    fn ask_paste_ack(&mut self, dest: &Dest, paste: u64) -> bool {
        let (Dest::Shell, Some(Leg::Mosh { mosh, .. }), None) = (dest, &self.leg, &self.herdr_mosh)
        else {
            return false;
        };
        let (reply, acked) = oneshot::channel();
        if mosh.commands.send(MoshCommand::Barrier(reply)).is_err() {
            return false;
        }
        // Acked, or the transport gone without it: the loop's end drops the
        // barrier unanswered.
        self.await_paste(dest, paste, async move { acked.await.is_ok() });
        true
    }

    /// Paste `paste` to `dest` is said landed (or not) once `landed` says.
    fn await_paste(
        &self,
        dest: &Dest,
        paste: u64,
        landed: impl std::future::Future<Output = bool> + Send + 'static,
    ) {
        let me = self.me.clone();
        let dest = dest.clone();
        tokio::spawn(async move {
            let landed = landed.await;
            let _ = me.send(Msg::PasteLanded {
                dest,
                paste,
                landed,
            });
        });
    }

    /// A paste to `dest` never reached the tty: what was sent after it
    /// waiting behind it doesn't go on its own — a Return behind an image
    /// path submitted the prompt without it (B687) — and is said. A size
    /// change among it still goes.
    fn drop_settled(&mut self, dest: &Dest, why: Discarded) {
        let Some(settling) = self.settling.remove(dest) else {
            return;
        };
        for queued in settling.queued {
            match queued {
                Queued::Typed(stream, typed, _) => self.emit(EventKind::InputDiscarded {
                    stream,
                    bytes: typed.len(),
                    why,
                }),
                Queued::Resize(stream, cols, rows) => self.resize(stream, cols, rows),
            }
        }
    }

    /// When typing behind an unacknowledged paste is next due to be shown
    /// as held (B689).
    fn held_due(&self) -> Option<u64> {
        if self
            .view
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .input_held
        {
            return None;
        }
        self.settling
            .values()
            .filter(|s| !s.landed && !s.queued.is_empty())
            .map(|s| s.since + HELD_SHOWN)
            .min()
    }

    /// Typing held behind mosh-server's acknowledgement past [`HELD_SHOWN`]
    /// — behind a paste here, or in herdr's plane — shown as mosh's
    /// reconnecting state, and no longer once it isn't (B689).
    fn show_held(&mut self, now: u64) {
        let settling = self
            .settling
            .values()
            .any(|s| !s.landed && !s.queued.is_empty() && now >= s.since + HELD_SHOWN);
        let held = settling || self.plane_held;
        let (was, srtt) = {
            let view = self.view.lock().unwrap_or_else(|e| e.into_inner());
            (view.input_held, view.mosh_srtt_ms)
        };
        if held == was {
            return;
        }
        let roaming = self.transport_roaming || held;
        self.update(|view| {
            view.input_held = held;
            view.mosh_roaming = roaming;
        });
        // The pill and the banner follow mosh's metrics.
        self.emit(EventKind::MoshMetrics {
            srtt_ms: srtt.unwrap_or(0.0),
            roaming,
        });
    }

    /// The pastes whose time is up: what waited behind each goes out, in
    /// order, up to the next paste, which it then waits behind.
    fn release_settled(&mut self, now: u64) {
        let due: Vec<Dest> = self
            .settling
            .iter()
            .filter(|(_, s)| s.landed && s.until <= now)
            .map(|(dest, _)| dest.clone())
            .collect();
        for dest in due {
            let Some(settling) = self.settling.remove(&dest) else {
                continue;
            };
            let mut queued = settling.queued.into_iter();
            while let Some(next) = queued.next() {
                let (stream, typed, end) = match next {
                    Queued::Typed(stream, typed, end) => (stream, typed, end),
                    Queued::Resize(stream, cols, rows) => {
                        self.resize(stream, cols, rows);
                        continue;
                    }
                };
                let paste = matches!(typed, Typed::Paste(_));
                self.send_now(&dest, stream, typed, end);
                if paste {
                    if let Some(next) = self.settling.get_mut(&dest) {
                        next.queued.extend(queued);
                    }
                    break;
                }
            }
        }
        self.show_held(now);
    }

    /// `landed`, for a paste over herdr and mosh: answered by the plane.
    fn send_to_shell(
        &mut self,
        stream: StreamId,
        typed: Typed,
        end: u64,
        landed: Option<oneshot::Sender<bool>>,
    ) {
        match &self.leg {
            Some(Leg::Shell { shell, .. }) => {
                let _ = shell.commands.send(ShellCommand::Input(typed.into_bytes()));
            }
            Some(Leg::Mosh { mosh, .. }) => match (&self.herdr_mosh, typed) {
                // herdr's renderer decides where typing and pastes may go.
                (Some(plane), Typed::Bytes(bytes)) => {
                    let _ = plane
                        .leg
                        .commands
                        .send(HerdrMoshCommand::Typed { bytes, end });
                }
                (Some(plane), Typed::Paste(text)) => {
                    let _ = plane
                        .leg
                        .commands
                        .send(HerdrMoshCommand::Paste { text, end, landed });
                }
                (None, typed) => {
                    let bytes = typed.into_bytes();
                    let _ = mosh.commands.send(MoshCommand::Keys { bytes, end });
                }
            },
            _ => self.emit(EventKind::InputDiscarded {
                stream,
                bytes: typed.len(),
                why: Discarded::PaneGone,
            }),
        }
    }

    fn send_to_pane(&mut self, pane: &str, stream: StreamId, typed: Typed) {
        match &self.leg {
            Some(Leg::Tmux { tmux, .. }) => {
                let pane = pane.to_owned();
                let _ = tmux.commands.send(match typed {
                    Typed::Bytes(bytes) => TmuxCommand::Input { pane, bytes },
                    // tmux brackets it by the pane's real mode (B43).
                    Typed::Paste(text) => TmuxCommand::Action(TmuxAction::Paste { pane, text }),
                });
            }
            Some(Leg::Herdr { herdr, .. }) => {
                let _ = herdr
                    .commands
                    .send(typed.for_herdr(pane.to_owned(), stream));
            }
            _ => self
                .pending
                .entry(pane.to_owned())
                .or_default()
                .push((stream, typed)),
        }
    }

    fn resize(&mut self, stream: StreamId, cols: u16, rows: u16) {
        self.grids.insert(stream, (cols, rows));
        if self.shell.as_ref().map(|s| s.stream.id()) == Some(stream) {
            self.grid = (cols, rows);
            match &self.leg {
                Some(Leg::Shell { shell, .. }) => {
                    let _ = shell.commands.send(ShellCommand::Resize { cols, rows });
                }
                Some(Leg::Mosh { mosh, .. }) => {
                    let _ = mosh.commands.send(MoshCommand::Resize { cols, rows });
                }
                _ => {}
            }
            if let Some(control) = self.control.as_mut() {
                control.grid = (cols, rows);
                if let Some(sidecar) = &control.sidecar {
                    let _ = sidecar.commands.send(TmuxCommand::MoshGrid { cols, rows });
                }
            }
            return;
        }
        let Some(channel) = self.panes.get(&stream) else {
            return;
        };
        if channel.attach != self.attach {
            return;
        }
        let pane = channel.pane.clone();
        if self
            .current
            .iter()
            .any(|info| info.pane == pane && info.active)
        {
            self.grid = (cols, rows);
        }
        match &self.leg {
            Some(Leg::Tmux { tmux, .. }) => {
                let _ = tmux.commands.send(TmuxCommand::Grid { pane, cols, rows });
            }
            Some(Leg::Herdr { herdr, .. }) => {
                let _ = herdr
                    .commands
                    .send(HerdrCommand::Resize { pane, cols, rows });
            }
            _ => {}
        }
    }

    fn discard_pending(&mut self, why: Discarded) {
        let pending = std::mem::take(&mut self.pending);
        for (stream, typed) in pending.into_values().flatten() {
            self.emit(EventKind::InputDiscarded {
                stream,
                bytes: typed.len(),
                why,
            });
        }
    }

    // MARK: Delivery

    /// Content waiting for the session: what can go now goes.
    fn try_deliver(&mut self) {
        if !self.life.state().is_live()
            || self.deliveries.is_empty()
            || self.life.delivery_check_pending()
        {
            return;
        }
        let waiting = std::mem::take(&mut self.deliveries);
        for delivery in waiting {
            if let Some(kept) = self.deliver(delivery) {
                self.deliveries.push(kept);
            }
        }
    }

    /// One delivery out, or back when its pane isn't there yet.
    fn deliver(&mut self, delivery: Delivery) -> Option<Delivery> {
        let has_sidecar = self
            .control
            .as_ref()
            .is_some_and(|control| control.sidecar.is_some());
        let leg = self.leg.as_ref().map(|leg| match leg {
            Leg::Shell { .. } => LegKind::Shell,
            Leg::Tmux { .. } => LegKind::Tmux,
            Leg::Mosh { .. } if self.herdr_mosh.is_some() => LegKind::MoshHerdr,
            Leg::Mosh { .. } => LegKind::Mosh,
            Leg::Herdr { .. } => LegKind::Herdr,
        });
        let route = delivery_route(leg, &delivery.pane, self.spec.multiplexer, has_sidecar);
        // A host with no tmux: no sidecar is coming to place a tmux pane's
        // reply over mosh.
        let tmux_missing = self.view.lock().unwrap_or_else(|e| e.into_inner()).degrade
            == Some(Degrade::TmuxMissing);
        match route {
            Route::Wait if leg == Some(LegKind::Mosh) && tmux_missing => {
                let _ = delivery.reply.send(false);
                None
            }
            Route::Wait => Some(delivery),
            Route::Refuse => {
                let _ = delivery.reply.send(false);
                None
            }
            Route::HerdrMosh => {
                let Delivery {
                    pane,
                    bytes,
                    paste,
                    reply,
                    deadline,
                } = delivery;
                if let Some(plane) = &self.herdr_mosh
                    && let Err(error) = plane.leg.commands.send(HerdrMoshCommand::Deliver {
                        pane,
                        bytes,
                        paste,
                        deadline,
                        reply,
                    })
                    && let HerdrMoshCommand::Deliver { reply, .. } = error.0
                {
                    let _ = reply.send(false);
                }
                None
            }
            Route::Herdr => {
                let Delivery {
                    pane,
                    bytes,
                    paste,
                    reply,
                    ..
                } = delivery;
                if let Some(Leg::Herdr { herdr, .. }) = &self.leg
                    && let Err(error) = herdr.commands.send(HerdrCommand::Deliver {
                        pane,
                        bytes,
                        paste,
                        reply,
                    })
                    && let HerdrCommand::Deliver { reply, .. } = error.0
                {
                    let _ = reply.send(false);
                }
                None
            }
            Route::Tmux { renders } => {
                let (known, listed) = if renders {
                    (
                        self.current.iter().any(|info| info.pane == delivery.pane),
                        !self.current.is_empty(),
                    )
                } else {
                    let view = self.view.lock().unwrap_or_else(|e| e.into_inner());
                    let tree = view.tmux.as_ref();
                    (
                        tree.is_some_and(|tree| tree.panes.contains_key(&delivery.pane)),
                        // Listed: attached AND its panes read. The sidecar is
                        // attached from the moment it starts reading its tree
                        // (`list-panes -a` comes a round trip later), and a
                        // reply that met that first tree was refused as for a
                        // pane that isn't there.
                        tree.is_some_and(|tree| tree.is_attached && !tree.panes.is_empty()),
                    )
                };
                if !known {
                    if listed {
                        // The attach lists its panes and this isn't one.
                        let _ = delivery.reply.send(false);
                        return None;
                    }
                    return Some(delivery);
                }
                let tmux = match &self.leg {
                    Some(Leg::Tmux { tmux, .. }) => Some(tmux),
                    _ => self.control.as_ref().and_then(|c| c.sidecar.as_ref()),
                };
                let Some(tmux) = tmux else {
                    return Some(delivery);
                };
                let Delivery {
                    pane,
                    bytes,
                    paste,
                    reply,
                    ..
                } = delivery;
                let command = if paste {
                    TmuxCommand::Action(TmuxAction::Paste {
                        pane,
                        text: String::from_utf8_lossy(&bytes).into_owned(),
                    })
                } else {
                    TmuxCommand::Input { pane, bytes }
                };
                let _ = tmux.commands.send(command);
                let (flushed, written) = oneshot::channel();
                let _ = tmux.commands.send(TmuxCommand::Flush(flushed));
                tokio::spawn(async move {
                    let _ = reply.send(written.await.is_ok());
                });
                None
            }
            Route::Shell => {
                let Delivery {
                    bytes,
                    paste,
                    reply,
                    ..
                } = delivery;
                let payload = if paste {
                    moshpit_herdr::frame::bracketed_paste(&String::from_utf8_lossy(&bytes))
                } else {
                    bytes
                };
                let sent = match &self.leg {
                    Some(Leg::Shell { shell, .. }) => {
                        shell.commands.send(ShellCommand::Input(payload)).is_ok()
                    }
                    Some(Leg::Mosh { mosh, .. }) => {
                        // Not the view's typing: its offsets don't move.
                        let end = self.shell.as_ref().map_or(0, |shell| shell.written);
                        mosh.commands
                            .send(MoshCommand::Keys {
                                bytes: payload,
                                end,
                            })
                            .is_ok()
                    }
                    _ => false,
                };
                let _ = reply.send(sent);
                None
            }
        }
    }

    fn expire_deliveries(&mut self, now: u64) {
        let (due, waiting): (Vec<Delivery>, Vec<Delivery>) = std::mem::take(&mut self.deliveries)
            .into_iter()
            .partition(|delivery| delivery.deadline <= now);
        self.deliveries = waiting;
        for delivery in due {
            let _ = delivery.reply.send(false);
        }
    }
}

/// The leg a delivery would ride.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegKind {
    Shell,
    Tmux,
    Mosh,
    /// mosh rendering herdr, its control plane beside it.
    MoshHerdr,
    Herdr,
}

/// Where a delivery goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Herdr,
    /// herdr over mosh's control plane: it waits for the pane in its tree,
    /// focuses it, puts the renderer on it and sends through mosh.
    HerdrMosh,
    Tmux {
        renders: bool,
    },
    Shell,
    Wait,
    /// Nowhere it could be placed: answered not sent.
    Refuse,
}

/// Where a delivery for `pane` goes on `leg`, in a session whose person
/// asked for `wants`, with mosh + tmux's `-CC` sidecar up or not.
///
/// A herdr pane's reply on a session that runs no herdr client — a host
/// where herdr went missing: a plain shell, the leg's own — was typed into
/// that shell as a command line and said sent: the B208 shape the app's
/// raw-attach renderer refuses (round twenty-five). Refused here too: nothing
/// places it. Over mosh with herdr, the control plane places it
/// ([`Route::HerdrMosh`]); what is addressed to the shell there would be
/// typed at whatever the loop shows, and is refused.
///
/// A tmux pane's reply over mosh before the `-CC` sidecar is up (the first
/// second or two of every connect, and while it is rebuilt) went down the
/// mosh shell too, and was said sent: that shell is tmux's own client (or
/// the login shell before it attaches), so the reply landed in tmux's
/// ACTIVE pane — whichever the desktop left focused — or ran as a command
/// line. It waits for the sidecar now, which sends it to its pane by id
/// once its tree lists it (the delivery's own wait, [`DELIVERY_WAIT`],
/// bounds that); a host with no tmux has no pane to place it in
/// ([`Actor::deliver`] refuses at once), and neither does a plain shell
/// where tmux was asked for.
fn delivery_route(
    leg: Option<LegKind>,
    pane: &str,
    wants: Multiplexer,
    has_sidecar: bool,
) -> Route {
    let addressed = pane != SHELL_PANE;
    match leg {
        Some(LegKind::Herdr) => Route::Herdr,
        Some(LegKind::MoshHerdr) if addressed => Route::HerdrMosh,
        Some(LegKind::MoshHerdr) => Route::Refuse,
        Some(LegKind::Tmux) if addressed => Route::Tmux { renders: true },
        Some(LegKind::Mosh) if addressed && has_sidecar => Route::Tmux { renders: false },
        Some(LegKind::Mosh) if addressed && wants == Multiplexer::Tmux => Route::Wait,
        Some(LegKind::Shell) if addressed && wants == Multiplexer::Tmux => Route::Refuse,
        Some(LegKind::Shell | LegKind::Mosh) if addressed && wants == Multiplexer::Herdr => {
            Route::Refuse
        }
        Some(LegKind::Shell | LegKind::Mosh) => Route::Shell,
        _ => Route::Wait,
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// How long the control plane waits before its next redial, after
/// `failures` in a row: 1, 2, 4, 8, then 16 s — a host that went away is
/// not hammered, one that blinked is back within a second.
pub(crate) fn control_retry_delay(failures: u32) -> Duration {
    Duration::from_secs(1 << failures.clamp(1, 5).saturating_sub(1))
}

/// The sidecar's terminal: `-CC` wants a controlling terminal; its size
/// is never reported to tmux. Its program is the attach chain
/// (`sidecar_attach_program`), never a line typed into a login shell that
/// may still be starting (B267).
async fn sidecar_terminal(
    ssh: &Ssh,
    timeout: Duration,
    program: String,
) -> Result<Process, Failure> {
    let mut request = PtyRequest::new(80, 24);
    request.command = Some(program);
    match tokio::time::timeout(timeout, ssh.terminal(&request)).await {
        Ok(Ok(pty)) => Ok(pty),
        Ok(Err(error)) => Err(Failure::from(&error)),
        Err(_) => Err(Failure::lost("the sidecar's terminal didn't open in time")),
    }
}

/// mosh-server `server`'s child: the program mosh runs, which with tmux
/// is tmux's own client (`exec`ed) — the phone's renderer. Asked a few
/// times: the child comes a moment after the server reports. An exec that
/// failed is asked again too (round thirty-four: once in some 250 live
/// connects the first ask failed, and with it the renderer was never
/// known), unless the connection itself is gone (its generation is over).
///
/// B274, looked for and not found: 890 live connects (private sshd at
/// DEBUG1, tmux 3.6a, mosh-server; one registry per connect and one for
/// all of them as the app keeps it, a four-pane session, sshd and every
/// process it starts at background QoS) read the pid on the first ask
/// every time. sshd's log shows at most two session channels open on the
/// connection at that moment — this exec and the sidecar's terminal,
/// interleaving cleanly — against `MaxSessions` 10, and russh answers a
/// channel's close at once, so no slot is held. Every ask that doesn't give
/// exactly one pid logs why (the error, or the exit and output), so the
/// next one names its cause.
async fn renderer_pid(ssh: &Ssh, server: i64) -> Option<i64> {
    let command = format!(
        "pgrep -P {server} 2>/dev/null || ps -A -o pid= -o ppid= 2>/dev/null | awk '$2 == {server} {{ print $1 }}'"
    );
    for ask in 0..10 {
        match tokio::time::timeout(Duration::from_secs(5), ssh.run(&command)).await {
            Ok(Ok(output)) => {
                let text = String::from_utf8_lossy(&output.stdout()).into_owned();
                let mut pids = text
                    .split_ascii_whitespace()
                    .filter_map(|word| word.parse::<i64>().ok())
                    .filter(|pid| *pid > 0);
                if let (Some(pid), None) = (pids.next(), pids.next()) {
                    log::debug!("mosh+tmux: the renderer is pid {pid} (ask {ask})");
                    return Some(pid);
                }
                log::info!(
                    "mosh+tmux: the renderer's pid wasn't read (ask {ask}): exit {:?}, closed {}, stdout {:?}, stderr {:?}",
                    output.exit,
                    output.closed,
                    text,
                    String::from_utf8_lossy(&output.stderr())
                );
            }
            Ok(Err(error)) => {
                log::info!("mosh+tmux: the renderer's pid wasn't read (ask {ask}): {error:?}");
                if ssh.is_closed() {
                    return None;
                }
            }
            Err(_) => log::info!("mosh+tmux: the renderer's pid wasn't read in time (ask {ask})"),
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    None
}

/// Forwards what a view writes into its byte channel to the session, in
/// order.
fn pump(stream: StreamId, mut input: Input, session: mpsc::UnboundedSender<Msg>) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = input.next().await {
            if session.send(Msg::Input { stream, event }).is_err() {
                break;
            }
        }
    })
}

/// The pane a herdr action picks, if it picks one.
fn herdr_pick_of(action: &HerdrAction) -> Option<String> {
    match action {
        HerdrAction::SelectPane(pane) => Some(pane.clone()),
        _ => None,
    }
}

/// An attempt's credentials, saying while they are read (B395): Face ID or
/// the passphrase card may be up, and a return must not restart the dial
/// under the person. The session's other connections (control planes, side
/// connections) read the credentials as they are.
struct ReadingCredentials {
    inner: Arc<dyn crate::services::Credentials>,
    session: mpsc::UnboundedSender<Msg>,
    generation: u64,
}

impl crate::services::Credentials for ReadingCredentials {
    fn auth<'a>(
        &'a self,
        connection_id: &'a str,
    ) -> crate::mosh_udp::BoxFuture<'a, Result<moshpit_ssh::Auth, crate::CredentialError>> {
        Box::pin(async move {
            let say = |waiting| {
                let _ = self.session.send(Msg::OnPerson {
                    generation: self.generation,
                    waiting,
                });
            };
            say(true);
            let auth = self.inner.auth(connection_id).await;
            say(false);
            auth
        })
    }
}

/// The host-key card, through the session: the attempt waits for the
/// person's answer (its handshake clock stopped), and gets "no" if the
/// attempt is given up first.
struct Prompt {
    session: mpsc::UnboundedSender<Msg>,
    generation: u64,
}

impl Prompt {
    fn ask(
        &self,
        host: &str,
        port: u16,
        key: &HostKey,
        previous: Option<&str>,
        conflicting: bool,
    ) -> SshBoxFuture<'static, bool> {
        let (reply, answer) = oneshot::channel();
        let asked = self
            .session
            .send(Msg::Ask {
                generation: self.generation,
                host: host.to_owned(),
                port,
                fingerprint: key.fingerprint(),
                previous: previous.map(str::to_owned),
                older_algorithm: key.older_algorithm(),
                conflicting,
                reply,
            })
            .is_ok();
        Box::pin(async move { asked && answer.await.unwrap_or(false) })
    }
}

/// The server's sign-in questions, through the session like the host-key
/// card (B563): `None` — declined — when the attempt is given up first.
impl SignInQuestions for Prompt {
    fn ask<'a>(
        &'a self,
        host: &'a str,
        question: &'a SignInQuestion,
    ) -> SshBoxFuture<'a, Option<String>> {
        let (reply, answer) = oneshot::channel();
        let asked = self
            .session
            .send(Msg::Question {
                generation: self.generation,
                host: host.to_owned(),
                question: question.clone(),
                reply,
            })
            .is_ok();
        Box::pin(async move {
            if !asked {
                return None;
            }
            answer.await.ok().flatten()
        })
    }
}

impl HostKeyPrompt for Prompt {
    fn trust_new_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
    ) -> SshBoxFuture<'a, bool> {
        self.ask(host, port, key, None, false)
    }

    fn accept_changed_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
        expected: &'a str,
    ) -> SshBoxFuture<'a, bool> {
        self.ask(host, port, key, Some(expected), false)
    }

    fn accept_conflicting_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
    ) -> SshBoxFuture<'a, bool> {
        self.ask(host, port, key, None, true)
    }
}

/// What becomes of the mosh-server a torn-down leg started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerFate {
    /// Stopped now, over a connection of its own.
    Stop,
    /// Stopped over the next connection the redial makes (B195).
    Orphan,
    /// Left alone: it ended the session itself, or the pins the renderer
    /// holds are paid first and it is stopped then (`finish`, B270).
    Leave,
}

/// A mosh-server at its leg's teardown (`why`), whether it was ever heard
/// from (`contacted`), whether the renderer's pins are still owed on it
/// (`debt`): one that never heard from us waits forever, so it goes; one
/// whose session the person ended or left for SSH can never be reattached
/// (this app doesn't keep the key), so it goes; one a reconnect leaves
/// behind is replaced by the server the redial starts and goes over that
/// connection; one that ended the session is gone already.
fn server_at_teardown(why: Teardown, contacted: bool, debt: bool) -> ServerFate {
    match why {
        Teardown::Disconnect if debt => ServerFate::Leave,
        Teardown::Disconnect | Teardown::SwitchToSsh => ServerFate::Stop,
        Teardown::Reconnect if !contacted => ServerFate::Stop,
        Teardown::Reconnect => ServerFate::Orphan,
        Teardown::Ended => ServerFate::Leave,
    }
}

/// What waits for the next tmux attach when none can carry it (B483): not
/// what only matters now, and not a refresh (an attach is one).
fn kept_for_an_attach(action: &TmuxAction) -> bool {
    !is_momentary(action) && !matches!(action, TmuxAction::Refresh)
}

/// A tmux action that only matters the moment it is given (B401): a
/// scroll, a tap, a view's report. Not held for a control plane in doubt:
/// late, it would act on what the screen no longer shows.
fn is_momentary(action: &TmuxAction) -> bool {
    matches!(
        action,
        TmuxAction::ScrollLines(_)
            | TmuxAction::AimWheel { .. }
            | TmuxAction::Click { .. }
            | TmuxAction::ScrollBegan
            | TmuxAction::ViewRevealed { .. }
            | TmuxAction::ReturnSent { .. }
            | TmuxAction::InputModeChanged { .. }
            | TmuxAction::DeferResize { .. }
            | TmuxAction::PollAgentHooks
            | TmuxAction::Relist
            | TmuxAction::SnapshotPanes(_)
            | TmuxAction::StopSnapshots
    )
}

/// The byte a tmux `prefix` option names (`C-b`, `C-a`, …): Ctrl and a
/// letter or one of `@[\]^_`; `C-b` (0x02) for anything else.
fn prefix_bytes(option: &str) -> Vec<u8> {
    let option = option.trim();
    if let Some(rest) = option.strip_prefix("C-")
        && let [key] = rest.as_bytes()
    {
        let upper = key.to_ascii_uppercase();
        if (b'@'..=b'_').contains(&upper) {
            return vec![upper - b'@'];
        }
    }
    vec![0x02]
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A return whose question finds nothing to ask (no leg for its
    /// generation) still reports one outcome, dead, and lets input go.
    #[test]
    fn a_return_with_nothing_to_ask_reports_it_dead() {
        struct NoKeys;
        impl crate::services::Credentials for NoKeys {
            fn auth<'a>(
                &'a self,
                _: &'a str,
            ) -> crate::mosh_udp::BoxFuture<'a, Result<moshpit_ssh::Auth, crate::CredentialError>>
            {
                Box::pin(async { Err(crate::CredentialError::Cancelled) })
            }
        }
        let services = Services::new(
            Arc::new(NoKeys),
            Arc::new(moshpit_ssh::KnownHosts::in_memory(BTreeMap::new())),
        );
        let spec = ConnectionSpec::ssh(
            "0F2E2C4A-6E1B-4D8A-9C5B-2A7D3E9F1B11",
            moshpit_ssh::Target::new("me", "192.0.2.1", 22),
        );
        let (me, mailbox) = mpsc::unbounded_channel();
        let (events, mut seen) = mpsc::unbounded_channel();
        let timing = Timing {
            resume_in_place: true,
            ..Timing::default()
        };
        let mut actor = Actor::new(
            7,
            spec,
            timing,
            services,
            true,
            mailbox,
            me,
            events,
            Arc::new(Mutex::new(SessionView::initial())),
        );
        actor.resume_probe(1, 1, Duration::from_millis(1_500), 25_000);
        assert!(
            matches!(
                actor.mailbox.try_recv(),
                Ok(Msg::Probe {
                    generation: 1,
                    probe: 1,
                    result: Probe::Failed
                })
            ),
            "the question answers itself: input isn't held"
        );
        actor.settle_question();
        let checked: Vec<EventKind> = std::iter::from_fn(|| seen.try_recv().ok())
            .map(|event| event.kind)
            .filter(|kind| matches!(kind, EventKind::ResumeChecked { .. }))
            .collect();
        assert!(
            matches!(
                checked.as_slice(),
                [EventKind::ResumeChecked {
                    away_ms: 25_000,
                    waited_ms: 0..=5,
                    outcome: ResumeOutcome::Dead,
                    through_tmux: false,
                    held: 0,
                }]
            ),
            "{checked:?}"
        );
    }

    /// B365: the registry's word that resume in place was switched reaches
    /// a session already running: its next return follows it.
    #[tokio::test]
    async fn a_running_session_follows_resume_in_place_switched_off() {
        struct NoKeys;
        impl crate::services::Credentials for NoKeys {
            fn auth<'a>(
                &'a self,
                _: &'a str,
            ) -> crate::mosh_udp::BoxFuture<'a, Result<moshpit_ssh::Auth, crate::CredentialError>>
            {
                Box::pin(async { Err(crate::CredentialError::Cancelled) })
            }
        }
        let services = Services::new(
            Arc::new(NoKeys),
            Arc::new(moshpit_ssh::KnownHosts::in_memory(BTreeMap::new())),
        );
        let spec = ConnectionSpec::ssh(
            "0F2E2C4A-6E1B-4D8A-9C5B-2A7D3E9F1B11",
            moshpit_ssh::Target::new("me", "192.0.2.1", 22),
        );
        let (me, mailbox) = mpsc::unbounded_channel();
        let (events, _seen) = mpsc::unbounded_channel();
        let timing = Timing {
            resume_in_place: true,
            ..Timing::default()
        };
        let mut actor = Actor::new(
            7,
            spec,
            timing,
            services,
            true,
            mailbox,
            me,
            events,
            Arc::new(Mutex::new(SessionView::initial())),
        );
        let now = actor.now();
        actor.life.connect(now);
        while actor.life.poll_action().is_some() {}
        let generation = actor.life.generation();
        assert!(
            actor
                .life
                .attempt_succeeded(generation, Transport::Ssh, now)
        );
        while actor.life.poll_action().is_some() {}

        actor.handle(Msg::ResumeInPlace(false)).await;
        assert!(!actor.timing.resume_in_place);
        actor.life.background(now + 1_000);
        while actor.life.poll_action().is_some() {}
        actor.life.foreground(now + 31_000);
        let asked = std::iter::from_fn(|| actor.life.poll_action())
            .any(|action| matches!(action, Action::ResumeProbe { .. }));
        assert!(!asked, "asked in place with the switch off");
        assert!(!actor.life.resume_pending());
    }

    /// Home's "3h" and "Lost 25m ago": when a session went live, kept
    /// through a reconnect, and when it stopped being live.
    #[test]
    fn a_session_says_when_it_went_live_and_when_it_was_lost() {
        let failed = SessionState::Failed(Failure::lost("cut"));
        let mut view = SessionView::initial();
        view.stamp(&SessionState::Connecting, 100.0);
        view.state = SessionState::Connecting;
        assert_eq!((view.connected_at, view.lost_at), (None, None));
        view.stamp(&SessionState::Connected, 110.0);
        view.state = SessionState::Connected;
        assert_eq!((view.connected_at, view.lost_at), (Some(110.0), None));
        view.stamp(&SessionState::Reconnecting, 200.0);
        view.state = SessionState::Reconnecting;
        assert_eq!(
            (view.connected_at, view.lost_at),
            (Some(110.0), Some(200.0)),
            "a reconnect keeps when it first went live"
        );
        view.stamp(&SessionState::Connecting, 205.0);
        view.state = SessionState::Connecting;
        assert_eq!(view.lost_at, Some(200.0), "a redial is not another loss");
        view.stamp(&SessionState::Connected, 210.0);
        view.state = SessionState::Connected;
        assert_eq!((view.connected_at, view.lost_at), (Some(110.0), None));
        view.stamp(&failed, 300.0);
        view.state = failed.clone();
        assert_eq!((view.connected_at, view.lost_at), (None, Some(300.0)));
    }

    #[test]
    fn a_session_that_never_went_live_was_never_lost() {
        let mut view = SessionView::initial();
        view.stamp(&SessionState::Failed(Failure::lost("refused")), 50.0);
        assert_eq!((view.connected_at, view.lost_at), (None, None));
    }

    #[test]
    fn a_herdr_panes_reply_on_a_session_without_herdr_is_refused_not_typed_into_the_shell() {
        // herdr missing on the host leaves a plain shell (SSH or mosh): a
        // notification's reply for a herdr pane went into that shell as a
        // command line, said sent.
        for leg in [LegKind::Mosh, LegKind::Shell] {
            assert_eq!(
                delivery_route(Some(leg), "w1:p2", Multiplexer::Herdr, false),
                Route::Refuse,
                "{leg:?}"
            );
            // The shell itself, addressed as such, still takes its input.
            assert_eq!(
                delivery_route(Some(leg), SHELL_PANE, Multiplexer::Herdr, false),
                Route::Shell
            );
        }
        // herdr over mosh: its control plane places a pane's reply; the
        // loop's tty, addressed as the shell, takes nothing.
        assert_eq!(
            delivery_route(Some(LegKind::MoshHerdr), "w1:p2", Multiplexer::Herdr, false),
            Route::HerdrMosh
        );
        assert_eq!(
            delivery_route(
                Some(LegKind::MoshHerdr),
                SHELL_PANE,
                Multiplexer::Herdr,
                false
            ),
            Route::Refuse
        );
        // Everything else as it was.
        assert_eq!(
            delivery_route(Some(LegKind::Herdr), "w1:p2", Multiplexer::Herdr, false),
            Route::Herdr
        );
        assert_eq!(
            delivery_route(None, "w1:p2", Multiplexer::Herdr, false),
            Route::Wait
        );
        assert_eq!(
            delivery_route(Some(LegKind::Mosh), "%3", Multiplexer::Tmux, true),
            Route::Tmux { renders: false }
        );
        assert_eq!(
            delivery_route(Some(LegKind::Tmux), "%3", Multiplexer::Tmux, false),
            Route::Tmux { renders: true }
        );
        assert_eq!(
            delivery_route(Some(LegKind::Shell), "x", Multiplexer::None, false),
            Route::Shell
        );
    }

    #[test]
    fn a_tmux_panes_reply_over_mosh_waits_for_the_sidecar_never_into_the_active_pane() {
        // Before the `-CC` sidecar is up (a connect's first second or two, a
        // rebuild), mosh's shell is tmux's own client: a reply for `%3` went
        // down it into tmux's ACTIVE pane, said sent.
        assert_eq!(
            delivery_route(Some(LegKind::Mosh), "%3", Multiplexer::Tmux, false),
            Route::Wait
        );
        // Once it is up, the sidecar places it by id.
        assert_eq!(
            delivery_route(Some(LegKind::Mosh), "%3", Multiplexer::Tmux, true),
            Route::Tmux { renders: false }
        );
        // tmux asked for and missing: a plain shell has no `%3`.
        assert_eq!(
            delivery_route(Some(LegKind::Shell), "%3", Multiplexer::Tmux, false),
            Route::Refuse
        );
        // The shell itself, addressed as such, still takes its input.
        assert_eq!(
            delivery_route(Some(LegKind::Mosh), SHELL_PANE, Multiplexer::Tmux, false),
            Route::Shell
        );
    }

    #[test]
    fn the_control_plane_backs_off_but_comes_back_soon_after_a_blink() {
        let delays: Vec<u64> = (0..8).map(|n| control_retry_delay(n).as_secs()).collect();
        assert_eq!(delays, [1, 1, 2, 4, 8, 16, 16, 16]);
    }

    // MARK: Pastes ride the stream (B361)

    struct NoCredentials;
    impl crate::services::Credentials for NoCredentials {
        fn auth<'a>(
            &'a self,
            _: &'a str,
        ) -> crate::mosh_udp::BoxFuture<'a, Result<moshpit_ssh::Auth, crate::CredentialError>>
        {
            Box::pin(async { Err(crate::CredentialError::Cancelled) })
        }
    }

    /// A session actor on its own, no leg: live on SSH (in place when
    /// `in_place`), its events kept.
    fn live_actor(in_place: bool) -> (Actor, mpsc::UnboundedReceiver<Event>) {
        let (mut actor, seen) = dialing_actor(in_place);
        let now = actor.now();
        let generation = actor.life.generation();
        assert!(
            actor
                .life
                .attempt_succeeded(generation, Transport::Ssh, now)
        );
        while actor.life.poll_action().is_some() {}
        (actor, seen)
    }

    /// A session actor on its own, no leg: its first dial out (not run).
    fn dialing_actor(in_place: bool) -> (Actor, mpsc::UnboundedReceiver<Event>) {
        let services = Services::new(
            Arc::new(NoCredentials),
            Arc::new(moshpit_ssh::KnownHosts::in_memory(BTreeMap::new())),
        );
        let spec = ConnectionSpec::ssh(
            "0F2E2C4A-6E1B-4D8A-9C5B-2A7D3E9F1B11",
            moshpit_ssh::Target::new("me", "192.0.2.1", 22),
        );
        let (me, mailbox) = mpsc::unbounded_channel();
        let (events, seen) = mpsc::unbounded_channel();
        let timing = Timing {
            resume_in_place: in_place,
            ..Timing::default()
        };
        let mut actor = Actor::new(
            7,
            spec,
            timing,
            services,
            true,
            mailbox,
            me,
            events,
            Arc::new(Mutex::new(SessionView::initial())),
        );
        let now = actor.now();
        actor.life.connect(now);
        while actor.life.poll_action().is_some() {}
        (actor, seen)
    }

    /// B395: the attempt's credential read says while it may wait on the
    /// person (Face ID, the passphrase card), and so does a host-key
    /// question until it is answered; meanwhile a return leaves the dial
    /// alone, where it used to restart it and replace the card.
    #[tokio::test]
    async fn a_dial_waiting_on_the_person_survives_a_return() {
        // The credentials say so around the read.
        let (me, mut mailbox) = mpsc::unbounded_channel();
        let reading = ReadingCredentials {
            inner: Arc::new(NoCredentials),
            session: me,
            generation: 4,
        };
        let _ = crate::services::Credentials::auth(&reading, "c").await;
        let said: Vec<bool> = std::iter::from_fn(|| mailbox.try_recv().ok())
            .filter_map(|message| match message {
                Msg::OnPerson {
                    generation: 4,
                    waiting,
                } => Some(waiting),
                _ => None,
            })
            .collect();
        assert_eq!(said, [true, false]);

        // While the credential is read, a return leaves the dial alone.
        let (mut actor, _events) = dialing_actor(false);
        let generation = actor.life.generation();
        let now = actor.now();
        actor
            .handle(Msg::OnPerson {
                generation,
                waiting: true,
            })
            .await;
        actor.life.background(now + 30_000);
        actor.life.foreground(now + 40_000);
        let restarted = std::iter::from_fn(|| actor.life.poll_action())
            .any(|action| matches!(action, Action::Abandon { .. }));
        assert!(!restarted, "the passphrase card's dial was restarted");
        actor
            .handle(Msg::OnPerson {
                generation,
                waiting: false,
            })
            .await;

        // A host-key question holds it too, until answered.
        actor.attempt = Some((generation, tokio::spawn(async {})));
        let (reply, _answer) = oneshot::channel();
        actor
            .handle(Msg::Ask {
                generation,
                host: "box".into(),
                port: 22,
                fingerprint: "SHA256:x".into(),
                previous: None,
                older_algorithm: None,
                conflicting: false,
                reply,
            })
            .await;
        actor.life.background(now + 50_000);
        actor.life.foreground(now + 60_000);
        let restarted = std::iter::from_fn(|| actor.life.poll_action())
            .any(|action| matches!(action, Action::Abandon { .. }));
        assert!(!restarted, "the host-key card's dial was restarted");
        let prompt = actor.next_prompt;
        actor
            .handle(Msg::AnswerHostKey {
                prompt,
                trust: true,
            })
            .await;
        actor.life.background(now + 70_000);
        actor.life.foreground(now + 75_000);
        let restarted = std::iter::from_fn(|| actor.life.poll_action())
            .any(|action| matches!(action, Action::Abandon { .. }));
        assert!(restarted, "answered, a stalled dial is restarted as before");
    }

    /// B564: a host key that came by `ssh-rsa` says so on its card.
    #[tokio::test]
    async fn a_host_key_by_the_older_algorithm_says_so_on_its_card() {
        let (mut actor, mut events) = dialing_actor(false);
        let generation = actor.life.generation();
        actor.attempt = Some((generation, tokio::spawn(async {})));
        for older in [Some("ssh-rsa"), None] {
            let (reply, _answer) = oneshot::channel();
            actor
                .handle(Msg::Ask {
                    generation,
                    host: "box".into(),
                    port: 22,
                    fingerprint: "SHA256:x".into(),
                    previous: None,
                    older_algorithm: older,
                    conflicting: false,
                    reply,
                })
                .await;
            let said =
                std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event.kind {
                    EventKind::HostKeyPrompt {
                        older_algorithm, ..
                    } => Some(older_algorithm),
                    _ => None,
                });
            assert_eq!(said, Some(older.map(str::to_owned)));
        }
    }

    /// B783: a new host on a connection used before — its trusted key gone
    /// with a restore — has its card say so; a changed key and two stored
    /// ones keep their own words, and a connection never used reads as a
    /// first connection.
    #[tokio::test]
    async fn a_new_host_on_a_connection_used_before_says_so_on_its_card() {
        let said = |events: &mut mpsc::UnboundedReceiver<Event>| {
            std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event.kind {
                EventKind::HostKeyPrompt {
                    connected_before, ..
                } => Some(connected_before),
                _ => None,
            })
        };
        for (used, previous, conflicting, expected) in [
            (true, None, false, true),
            (true, Some("SHA256:old"), false, false),
            (true, None, true, false),
            (false, None, false, false),
        ] {
            let (mut actor, mut events) = dialing_actor(false);
            actor.spec.connected_before = used;
            let generation = actor.life.generation();
            actor.attempt = Some((generation, tokio::spawn(async {})));
            let (reply, _answer) = oneshot::channel();
            actor
                .handle(Msg::Ask {
                    generation,
                    host: "box".into(),
                    port: 22,
                    fingerprint: "SHA256:x".into(),
                    previous: previous.map(str::to_owned),
                    older_algorithm: None,
                    conflicting,
                    reply,
                })
                .await;
            assert_eq!(
                said(&mut events),
                Some(expected),
                "used {used}, previous {previous:?}, conflicting {conflicting}"
            );
        }
    }

    /// B634: a side connection's sign-in question (a host behind a second
    /// factor, plain mosh having no SSH of its own) is asked of the person
    /// while its dial is out, and its answer goes back. A no is kept: the
    /// side feature fails as it did, and no later side dial asks again.
    #[tokio::test]
    async fn a_side_connections_question_is_asked_and_a_no_is_kept() {
        let (mut actor, mut events) = dialing_actor(false);
        let question = || SignInQuestion {
            name: String::new(),
            instructions: String::new(),
            prompt: "Verification code: ".into(),
            echo: false,
        };
        let ask = |actor: &mut Actor| {
            let (reply, answer) = oneshot::channel();
            let message = Msg::Question {
                generation: SIDE,
                host: "box".into(),
                question: question(),
                reply,
            };
            (message, answer, actor.next_prompt)
        };

        // No side dial out: nothing to ask for.
        let (message, answer, _) = ask(&mut actor);
        actor.handle(message).await;
        assert_eq!(answer.await.unwrap(), None);

        // A borrow waits on a side dial: its question is the person's.
        let (borrower, _borrowed) = oneshot::channel();
        actor.side_waiting.push(borrower);
        let (message, answer, _) = ask(&mut actor);
        actor.handle(message).await;
        let prompt = actor.next_prompt;
        let shown = std::iter::from_fn(|| events.try_recv().ok()).any(|event| {
            matches!(event.kind, EventKind::SignInQuestion { prompt: p, .. } if p == prompt)
        });
        assert!(shown, "the side connection's question wasn't shown");
        actor
            .handle(Msg::AnswerQuestion {
                prompt,
                answer: Some("424242".into()),
            })
            .await;
        assert_eq!(answer.await.unwrap().as_deref(), Some("424242"));

        // Asked again, and the side dial ends first: the card goes.
        let (message, answer, _) = ask(&mut actor);
        actor.handle(message).await;
        let prompt = actor.next_prompt;
        actor
            .handle(Msg::SideSsh {
                result: Err(Failure::credentials(
                    crate::event::CredentialsWhy::Cancelled,
                    "signing in was declined",
                )),
            })
            .await;
        assert_eq!(answer.await.unwrap(), None);
        let withdrawn = std::iter::from_fn(|| events.try_recv().ok()).any(|event| {
            matches!(event.kind, EventKind::SignInQuestionWithdrawn { prompt: p } if p == prompt)
        });
        assert!(withdrawn, "the side connection's card was left up");

        // The person said no: a later side dial asks nothing.
        assert!(actor.side_declined);
        let (borrower, _borrowed) = oneshot::channel();
        actor.side_waiting.push(borrower);
        let (message, answer, _) = ask(&mut actor);
        actor.handle(message).await;
        assert_eq!(answer.await.unwrap(), None, "asked again after a no");
    }

    /// B634: the side connection is let go once no borrow came for ten
    /// minutes, not before.
    #[tokio::test]
    async fn an_idle_side_connection_is_let_go() {
        let (mut actor, _events) = dialing_actor(false);
        actor.side_idle_until = Some(1_000 + SIDE_IDLE_MS);
        actor.release_idle_side(1_000 + SIDE_IDLE_MS - 1);
        assert!(actor.side_idle_until.is_some(), "let go early");
        actor.release_idle_side(1_000 + SIDE_IDLE_MS);
        assert!(actor.side_idle_until.is_none());
        assert!(actor.side.is_none());
    }

    /// B563: the server's sign-in question goes to the person with its own
    /// words, holds the dial like the host-key card, takes their answer
    /// back; a question the attempt outlived is withdrawn as declined.
    #[tokio::test]
    async fn a_servers_sign_in_question_is_the_persons_to_answer() {
        let (mut actor, mut events) = dialing_actor(false);
        let generation = actor.life.generation();
        let now = actor.now();
        actor.attempt = Some((generation, tokio::spawn(async {})));
        let question = SignInQuestion {
            name: String::new(),
            instructions: String::new(),
            prompt: "Verification code: ".into(),
            echo: false,
        };
        let (reply, answer) = oneshot::channel();
        actor
            .handle(Msg::Question {
                generation,
                host: "box".into(),
                question: question.clone(),
                reply,
            })
            .await;
        let prompt = actor.next_prompt;
        let asked = std::iter::from_fn(|| events.try_recv().ok()).any(|event| {
            matches!(
                event.kind,
                EventKind::SignInQuestion { prompt: p, ref host, ref text, echo: false, .. }
                    if p == prompt && host == "box" && text == "Verification code: "
            )
        });
        assert!(asked, "the question wasn't shown");
        actor.life.background(now + 50_000);
        actor.life.foreground(now + 60_000);
        let restarted = std::iter::from_fn(|| actor.life.poll_action())
            .any(|action| matches!(action, Action::Abandon { .. }));
        assert!(!restarted, "the question's dial was restarted");
        actor
            .handle(Msg::AnswerQuestion {
                prompt,
                answer: Some("424242".into()),
            })
            .await;
        assert_eq!(answer.await.unwrap().as_deref(), Some("424242"));

        // Asked again, then the attempt is given up: the card closes and
        // the dial reads a no.
        let (reply, answer) = oneshot::channel();
        actor
            .handle(Msg::Question {
                generation,
                host: "box".into(),
                question,
                reply,
            })
            .await;
        let prompt = actor.next_prompt;
        actor.withdraw_prompts(generation);
        assert_eq!(answer.await.unwrap(), None);
        let withdrawn = std::iter::from_fn(|| events.try_recv().ok()).any(|event| {
            matches!(event.kind, EventKind::SignInQuestionWithdrawn { prompt: p } if p == prompt)
        });
        assert!(withdrawn, "the question's card was left up");

        // A question from an attempt already gone is declined at once.
        let (reply, answer) = oneshot::channel();
        actor.attempt = None;
        actor
            .handle(Msg::Question {
                generation,
                host: "box".into(),
                question: SignInQuestion {
                    name: String::new(),
                    instructions: String::new(),
                    prompt: "Token: ".into(),
                    echo: false,
                },
                reply,
            })
            .await;
        assert_eq!(answer.await.unwrap(), None);
    }

    /// A tmux pane's channel on `actor`, its pump forwarding to the actor's
    /// mailbox; the pane isn't attached (there is no leg), so what is
    /// routed to it waits in `pending`, in order.
    fn tmux_pane(actor: &mut Actor, pane: &str) -> StreamId {
        let (stream, input) = moshpit_stream::open();
        let id = stream.id();
        let pump = pump(id, input, actor.me.clone());
        actor.panes.insert(
            id,
            PaneChannel {
                pane: pane.to_owned(),
                attach: 1,
                stream: Arc::new(stream),
                pump,
            },
        );
        id
    }

    /// What the pump forwarded, `n` messages, handled as the actor would.
    async fn take_input(actor: &mut Actor, n: usize) {
        for _ in 0..n {
            let message = tokio::time::timeout(Duration::from_secs(5), actor.mailbox.recv())
                .await
                .expect("the pump forwards")
                .expect("the mailbox is open");
            assert!(matches!(message, Msg::Input { .. }));
            actor.handle(message).await;
        }
    }

    /// Drains the lifecycle's actions: the number of the return's question
    /// among them.
    fn asked(actor: &mut Actor) -> u64 {
        let asked: Vec<u64> = std::iter::from_fn(|| actor.life.poll_action())
            .filter_map(|action| match action {
                Action::ResumeProbe { probe, .. } => Some(probe),
                _ => None,
            })
            .collect();
        *asked.last().expect("the return asked")
    }

    fn keys(bytes: &[u8]) -> Typed {
        Typed::Bytes(bytes.to_vec())
    }

    fn paste(text: &str) -> Typed {
        Typed::Paste(text.to_owned())
    }

    /// B394: the return's question takes only its own answer. A keepalive's
    /// from before the suspension used to become the question's, and the
    /// log said "kept" of a round trip older than the suspension.
    #[tokio::test]
    async fn the_returns_question_takes_only_its_own_answer() {
        let (mut actor, _events) = live_actor(true);
        let generation = actor.life.generation();
        let now = actor.now();
        actor.life.background(now);
        actor.life.foreground(now + 30_000);
        let question = asked(&mut actor);
        // As the actor asks it (there is no leg here to ask on).
        actor.resume_question = Some(ResumeQuestion {
            generation,
            probe: question,
            asked_at: now + 30_000,
            away_ms: 30_000,
            through_tmux: false,
            answer: None,
        });
        actor
            .handle(Msg::Probe {
                generation,
                probe: question + 1,
                result: Probe::Alive,
            })
            .await;
        let stale = actor.resume_question.as_ref().and_then(|q| q.answer);
        assert_eq!(stale, None, "another probe's answer");
        assert!(actor.life.resume_pending());
        actor
            .handle(Msg::Probe {
                generation,
                probe: question,
                result: Probe::Alive,
            })
            .await;
        let own = actor.resume_question.as_ref().and_then(|q| q.answer);
        assert_eq!(own.map(|(result, _)| result), Some(Probe::Alive));
        assert!(!actor.life.resume_pending());
    }

    /// Back from the background in place, before the transport answered:
    /// a paste made after some typing waits with that typing and goes out
    /// behind it, in the order made — it neither overtakes the held keys
    /// nor goes into the socket the hold distrusts (red before: a paste
    /// went straight to the leg).
    #[tokio::test]
    async fn a_paste_during_the_return_waits_behind_the_keys_typed_before_it() {
        let (mut actor, _events) = live_actor(true);
        let generation = actor.life.generation();
        let now = actor.now();
        actor.life.background(now);
        actor.life.foreground(now + 30_000);
        let question = asked(&mut actor);
        assert!(actor.life.resume_pending());
        let id = tmux_pane(&mut actor, "%1");
        // One main-thread turn's worth: keys, a paste, keys.
        assert!(moshpit_stream::view::write(id, b"ab"));
        assert!(moshpit_stream::view::paste(id, "x\ny"));
        assert!(moshpit_stream::view::write(id, b"cd"));
        take_input(&mut actor, 3).await;
        let made = vec![(id, keys(b"ab")), (id, paste("x\ny")), (id, keys(b"cd"))];
        let held: Vec<(StreamId, Typed)> = actor
            .held
            .iter()
            .filter_map(|kept| match kept {
                Kept::Typed(stream, typed, _) => Some((*stream, typed.clone())),
                Kept::Resize(..) => None,
            })
            .collect();
        assert_eq!(held, made, "all of it waits for the answer");
        assert!(actor.pending.is_empty(), "nothing went ahead");
        actor
            .life
            .probe_answered(generation, question, Probe::Alive, now + 30_080);
        assert!(!actor.life.resume_pending());
        actor.release_held();
        assert!(actor.held.is_empty());
        let routed: Vec<(StreamId, Typed)> = actor.pending.remove("%1").expect("routed to %1");
        assert_eq!(routed, made, "in the order made");
    }

    /// B397: a herdr pane the person picked — a notification tapped after a
    /// long absence picks before the return finds the socket dead — is
    /// picked again on the next herdr leg, once; one the server is seen on
    /// is not, nor is anything else a herdr action does.
    #[test]
    fn a_herdr_pick_the_connection_never_carried_is_made_again() {
        let view = |active: Option<&str>, in_flight: bool| HerdrView {
            snapshot: moshpit_herdr::snapshot::Snapshot {
                active_pane_id: active.map(str::to_owned),
                ..Default::default()
            },
            agent_hooks: BTreeMap::new(),
            agent_hooks_loaded: false,
            notice: None,
            refreshing: false,
            restarting: false,
            server_not_running: false,
            protocol_mismatch: None,
            version_skew: None,
            server_version: None,
            push_active: false,
            pane_switch_in_flight: in_flight,
            server_focus: active.map(str::to_owned),
            pane_cwds: BTreeMap::new(),
            worktree_repos: BTreeMap::new(),
            open_repo_guesses: Vec::new(),
            frame_target: None,
            attached_pane: None,
            retry_wait_ms: None,
        };
        let repicks = |actor: &mut Actor| match actor.herdr_repick() {
            Some(HerdrCommand::Action {
                action: HerdrAction::SelectPane(pane),
                token: None,
            }) => Some(pane),
            Some(_) => panic!("not a pick"),
            None => None,
        };
        let (mut actor, _events) = dialing_actor(false);
        assert_eq!(
            herdr_pick_of(&HerdrAction::SelectPane("w1:p2".into())),
            Some("w1:p2".into())
        );
        assert_eq!(herdr_pick_of(&HerdrAction::NewPane), None);

        // Picked; the dead connection's last views show the server elsewhere,
        // or the switch still out.
        actor.herdr_pick = Some("w1:p2".into());
        actor.herdr_pick_seen(&view(Some("w1:p1"), false));
        actor.herdr_pick_seen(&view(Some("w1:p2"), true));
        assert_eq!(repicks(&mut actor), Some("w1:p2".into()), "made again");
        assert_eq!(repicks(&mut actor), None, "once");

        // Seen on it, the switch landed: nothing to make again.
        actor.herdr_pick = Some("w1:p3".into());
        actor.herdr_pick_seen(&view(Some("w1:p3"), false));
        assert_eq!(repicks(&mut actor), None);
    }

    /// B406: a herdr pick counts as seen only on the server's word. The
    /// client puts the pick in its own tree at once and lets the switch go
    /// when its hold runs out; on a link that died under it nothing was
    /// sent, and the pick must still be made on the next leg.
    #[test]
    fn a_herdr_pick_is_seen_only_on_the_servers_word() {
        use moshpit_herdr::client::rig::{Fate, Rig, Rule};
        let mut rig = Rig::new(moshpit_herdr::client::Config::default());
        rig.start();
        rig.run(500);
        // The link dies: nothing answers any more.
        rig.host.rules.push(Rule {
            pattern: "herdr".into(),
            delay: None,
            fate: Fate::Hang,
        });
        rig.act(HerdrAction::SelectPane("w1:p2".into()));
        rig.run(6_000);
        let view = HerdrView::of(&rig.client);
        assert_eq!(view.snapshot.active_pane_id.as_deref(), Some("w1:p2"));
        assert!(!view.pane_switch_in_flight, "the hold ran out");
        let (mut actor, _events) = dialing_actor(false);
        actor.herdr_pick = Some("w1:p2".into());
        actor.herdr_pick_seen(&view);
        assert_eq!(
            actor.herdr_pick.as_deref(),
            Some("w1:p2"),
            "seen with nothing said"
        );
        // A live link: the read after the switch says the server is on it.
        rig.host.rules.clear();
        rig.act(HerdrAction::SelectPane("w1:p2".into()));
        rig.run(20_000);
        actor.herdr_pick_seen(&HerdrView::of(&rig.client));
        assert_eq!(actor.herdr_pick, None);
    }

    /// B405: a tmux pick made while no leg carries it — a notification
    /// tapped while the connection is redialled — is where the next attach
    /// lands, not where the last one was; what it doesn't name comes from
    /// there. Anything but a pick isn't remembered.
    #[tokio::test]
    async fn a_tmux_pick_made_while_redialling_is_where_the_next_attach_lands() {
        let place = |session: &str, window: Option<&str>, pane: Option<&str>| Selection {
            session: session.into(),
            window: window.map(str::to_owned),
            pane: pane.map(str::to_owned),
        };
        let (mut actor, _events) = dialing_actor(false);
        assert!(actor.leg.is_none());
        actor.handoff.selection = Some(place("$0", Some("@1"), Some("%1")));
        actor
            .handle(Msg::Tmux(TmuxAction::SelectPane("%5".into())))
            .await;
        actor.handle(Msg::Tmux(TmuxAction::Refresh)).await;
        assert_eq!(
            actor.next_tmux_restore(),
            Some(place("$0", Some("@1"), Some("%5"))),
            "the pick, in the last place"
        );
        assert_eq!(actor.next_tmux_restore(), None, "once");
        // A window picked: its own, not the last pane.
        actor.handoff.selection = Some(place("$0", Some("@1"), Some("%1")));
        actor
            .handle(Msg::Tmux(TmuxAction::SelectWindow("@3".into())))
            .await;
        assert_eq!(
            actor.next_tmux_restore(),
            Some(place("$0", Some("@3"), None))
        );
        // No pick: where the last attach was.
        actor.handoff.selection = Some(place("$0", Some("@1"), Some("%1")));
        assert_eq!(
            actor.next_tmux_restore(),
            Some(place("$0", Some("@1"), Some("%1")))
        );
    }

    /// B485: typing behind a paste to the same place waits for the paste to
    /// have had its time, in order, then goes; a paste among it starts the
    /// wait again; another place doesn't wait.
    #[tokio::test]
    async fn typing_behind_a_paste_waits_for_it_in_order() {
        let (mut actor, _events) = dialing_actor(false);
        let pane = |p: &str| Dest::Pane(p.to_owned());
        let sent = |actor: &Actor, p: &str| -> Vec<Typed> {
            actor
                .pending
                .get(p)
                .map(|q| q.iter().map(|(_, t)| t.clone()).collect())
                .unwrap_or_default()
        };
        let start = actor.now();
        actor.send_or_wait(pane("%1"), 1, Typed::Paste(" /tmp/a.jpg ".into()), 0);
        actor.send_or_wait(pane("%1"), 1, Typed::Bytes(b"\r".to_vec()), 0);
        actor.send_or_wait(pane("%1"), 1, Typed::Paste("b".into()), 0);
        actor.send_or_wait(pane("%1"), 1, Typed::Bytes(b"c".to_vec()), 0);
        actor.send_or_wait(pane("%2"), 2, Typed::Bytes(b"d".to_vec()), 0);
        assert_eq!(
            sent(&actor, "%1"),
            vec![Typed::Paste(" /tmp/a.jpg ".into())],
            "the Return waits"
        );
        assert_eq!(sent(&actor, "%2"), vec![Typed::Bytes(b"d".to_vec())]);
        let until = actor.settling[&pane("%1")].until;
        assert!(until >= start + 150, "{until} {start}");
        actor.release_settled(until - 1);
        assert_eq!(sent(&actor, "%1").len(), 1, "not before its time");
        actor.release_settled(until);
        assert_eq!(
            sent(&actor, "%1"),
            vec![
                Typed::Paste(" /tmp/a.jpg ".into()),
                Typed::Bytes(b"\r".to_vec()),
                Typed::Paste("b".into()),
            ],
            "up to the next paste, which waits in turn"
        );
        let next = actor.settling[&pane("%1")].until;
        actor.release_settled(next);
        assert_eq!(
            sent(&actor, "%1").last(),
            Some(&Typed::Bytes(b"c".to_vec()))
        );
        assert!(
            !actor.settling.contains_key(&pane("%1")),
            "nothing left waiting"
        );
    }

    /// What a torn-down leg's mosh-server comes to (the Swift client's
    /// MoshServerTeardownTests and their rule, kept when it went, E2): the
    /// person's end or SSH stops it, a reconnect stops one never heard from
    /// and leaves a contacted one to the next connection (B195), and a
    /// server that ended the session itself is never stopped.
    #[test]
    fn what_a_torn_down_legs_mosh_server_comes_to() {
        use ServerFate::*;
        for contacted in [false, true] {
            assert_eq!(
                server_at_teardown(Teardown::Disconnect, contacted, false),
                Stop
            );
            assert_eq!(
                server_at_teardown(Teardown::SwitchToSsh, contacted, false),
                Stop
            );
            assert_eq!(server_at_teardown(Teardown::Ended, contacted, false), Leave);
            assert_eq!(
                server_at_teardown(Teardown::Disconnect, contacted, true),
                Leave,
                "the pins first"
            );
        }
        assert_eq!(server_at_teardown(Teardown::Reconnect, false, false), Stop);
        assert_eq!(server_at_teardown(Teardown::Reconnect, true, false), Orphan);
    }

    /// B483: with no leg, a tmux command (a new window, a close) waits for
    /// the next attach instead of going nowhere, in order; a pick still goes
    /// where that attach lands (B405), and what only matters now, or a
    /// refresh, isn't kept. Held past the bound, they are dropped, to be
    /// said once an attach is back. Without tmux nothing waits.
    #[tokio::test]
    async fn tmux_commands_given_while_redialling_wait_for_the_next_attach() {
        let (mut actor, _events) = dialing_actor(false);
        assert!(actor.leg.is_none());
        actor
            .handle(Msg::Tmux(TmuxAction::NewWindow { name: None }))
            .await;
        assert!(actor.tmux_held.is_empty(), "a plain shell has no attach");
        actor.spec.multiplexer = Multiplexer::Tmux;
        for action in [
            TmuxAction::NewWindow { name: None },
            TmuxAction::SelectPane("%5".into()),
            TmuxAction::ScrollLines(3),
            TmuxAction::Refresh,
            TmuxAction::KillPane("%2".into()),
        ] {
            actor.handle(Msg::Tmux(action)).await;
        }
        assert_eq!(
            actor.tmux_held,
            vec![
                TmuxAction::NewWindow { name: None },
                TmuxAction::KillPane("%2".into()),
            ]
        );
        let picked = actor.tmux_pick.as_ref().and_then(|pick| pick.pane.clone());
        assert_eq!(picked.as_deref(), Some("%5"));
        let hold = actor.tmux_hold;
        actor.handle(Msg::TmuxHoldExpired { hold: hold - 1 }).await;
        assert_eq!(actor.tmux_held.len(), 2, "an older hold's bound is moot");
        assert!(!actor.tmux_dropped);
        actor.handle(Msg::TmuxHoldExpired { hold }).await;
        assert!(actor.tmux_held.is_empty(), "dropped, not run late");
        assert!(actor.tmux_dropped, "and said at the next attach");
    }

    /// B405: a dead tmux leg's pick carries its window and session from the
    /// tree it last had.
    #[test]
    fn a_pick_names_the_window_and_session_its_pane_is_in() {
        let mut tree = Snapshot::default();
        tree.windows.insert(
            "@2".into(),
            moshpit_tmux::model::WindowInfo::new("@2", "$1"),
        );
        tree.panes.insert(
            "%5".into(),
            moshpit_tmux::model::PaneInfo::placeholder("%5", "@2"),
        );
        let pick = |action| crate::tmux_leg::pick_selection(&action, Some(&tree));
        assert_eq!(
            pick(TmuxAction::SelectPane("%5".into())),
            Some(Selection {
                session: "$1".into(),
                window: Some("@2".into()),
                pane: Some("%5".into()),
            })
        );
        assert_eq!(
            pick(TmuxAction::SelectPane("%9".into())),
            Some(Selection {
                session: String::new(),
                window: None,
                pane: Some("%9".into()),
            })
        );
        assert_eq!(pick(TmuxAction::NewPane), None);
    }

    /// B405: over SSH, a herdr pick while the connection is redialled is
    /// made on the next herdr leg (as B397's), and still answered.
    #[tokio::test]
    async fn a_herdr_pick_made_while_redialling_is_made_on_the_next_leg() {
        let (mut actor, mut events) = dialing_actor(false);
        assert!(!actor.life.state().is_live());
        actor
            .handle(Msg::Herdr {
                action: HerdrAction::SelectPane("w1:p2".into()),
                token: 9,
            })
            .await;
        assert_eq!(actor.herdr_pick.as_deref(), Some("w1:p2"));
        let mut answered = false;
        while let Ok(event) = events.try_recv() {
            answered |= matches!(event.kind, EventKind::HerdrAnswer { token: 9, .. });
        }
        assert!(answered, "answered");
        actor
            .handle(Msg::Herdr {
                action: HerdrAction::Refresh,
                token: 10,
            })
            .await;
        assert_eq!(
            actor.herdr_pick.as_deref(),
            Some("w1:p2"),
            "a refresh is no pick"
        );
    }

    /// B482: in the return's confirmation window (the screen looks live)
    /// the shortcut bar's taps wait with the keys; if the connection proves
    /// dead they are not kept for the pane as the keys are (B476's rule: a
    /// stray tap must not land in an agent's prompt after the reconnect).
    /// If it answers, they go, in order.
    #[tokio::test]
    async fn bar_taps_held_through_a_dead_return_are_not_kept_for_the_pane() {
        let (mut actor, mut events) = live_actor(true);
        let generation = actor.life.generation();
        let now = actor.now();
        actor.life.background(now);
        actor.life.foreground(now + 30_000);
        let question = asked(&mut actor);
        let id = tmux_pane(&mut actor, "%6");
        assert!(moshpit_stream::view::write(id, b"ls"));
        assert!(moshpit_stream::view::bar(id, b"\r"));
        take_input(&mut actor, 2).await;
        assert_eq!(actor.held.len(), 2, "both wait for the answer");
        while events.try_recv().is_ok() {}
        actor
            .life
            .probe_answered(generation, question, Probe::TimedOut, now + 31_500);
        actor.release_held();
        assert_eq!(
            actor.pending.remove("%6"),
            Some(vec![(id, keys(b"ls"))]),
            "the keys wait for the pane, the tap doesn't"
        );
        assert_eq!(
            discarded(&mut events),
            [EventKind::InputDiscarded {
                stream: id,
                bytes: 1,
                why: Discarded::NotLive,
            }]
        );
    }

    /// B396: keys typed while the return asked a connection that turned out
    /// dead — the screen looked live — wait for their pane on a multiplexer
    /// (the same pane comes back with the redial), where they used to be
    /// dropped unsaid.
    #[tokio::test]
    async fn keys_typed_during_a_dead_return_wait_for_their_pane() {
        let (mut actor, mut events) = live_actor(true);
        let generation = actor.life.generation();
        let now = actor.now();
        actor.life.background(now);
        actor.life.foreground(now + 30_000);
        let question = asked(&mut actor);
        let id = tmux_pane(&mut actor, "%1");
        assert!(moshpit_stream::view::write(id, b"ls\r"));
        take_input(&mut actor, 1).await;
        assert_eq!(actor.held.len(), 1);
        while events.try_recv().is_ok() {}
        actor
            .life
            .probe_answered(generation, question, Probe::TimedOut, now + 31_500);
        assert!(!actor.life.state().is_live(), "reconnecting");
        actor.release_held();
        assert_eq!(
            actor.pending.remove("%1"),
            Some(vec![(id, keys(b"ls\r"))]),
            "they wait for the pane"
        );
        let dropped = std::iter::from_fn(|| events.try_recv().ok())
            .filter(|event| matches!(event.kind, EventKind::InputDiscarded { .. }))
            .count();
        assert_eq!(dropped, 0);
    }

    /// B396: a plain shell's keys typed then are dropped — the redial is a
    /// new shell — and said, as their own reason.
    #[tokio::test]
    async fn a_plain_shells_keys_typed_during_a_dead_return_are_said_dropped() {
        let (mut actor, mut events) = live_actor(true);
        let generation = actor.life.generation();
        let now = actor.now();
        let (stream, input) = moshpit_stream::open();
        let id = stream.id();
        actor.shell = Some(ShellChannel {
            stream: Arc::new(stream),
            pump: pump(id, input, actor.me.clone()),
            used: false,
            written: 0,
        });
        actor.life.background(now);
        actor.life.foreground(now + 30_000);
        let question = asked(&mut actor);
        assert!(moshpit_stream::view::write(id, b"rm x\r"));
        take_input(&mut actor, 1).await;
        assert_eq!(actor.held.len(), 1);
        while events.try_recv().is_ok() {}
        actor
            .life
            .probe_answered(generation, question, Probe::Failed, now + 30_010);
        actor.release_held();
        let dropped: Vec<EventKind> = std::iter::from_fn(|| events.try_recv().ok())
            .map(|event| event.kind)
            .filter(|kind| matches!(kind, EventKind::InputDiscarded { .. }))
            .collect();
        assert_eq!(
            dropped,
            [EventKind::InputDiscarded {
                stream: id,
                bytes: 5,
                why: Discarded::Reconnected,
            }]
        );
    }

    /// Live, keys then a paste in one turn reach the pane in that order (the
    /// pump's hop no longer lets a paste overtake them); a paste under the
    /// reconnect's veil waits for its pane, as typing does (B322).
    #[tokio::test]
    async fn keys_then_a_paste_go_to_the_pane_in_the_order_made() {
        let (mut actor, mut events) = live_actor(false);
        let id = tmux_pane(&mut actor, "%2");
        assert!(moshpit_stream::view::write(id, b"ls "));
        assert!(moshpit_stream::view::paste(id, "a b"));
        assert!(moshpit_stream::view::write(id, b"\r"));
        take_input(&mut actor, 3).await;
        assert_eq!(
            actor.pending.remove("%2").expect("routed to %2"),
            vec![(id, keys(b"ls ")), (id, paste("a b")), (id, keys(b"\r"))]
        );
        let generation = actor.life.generation();
        let now = actor.now();
        actor.life.lost(generation, now);
        while actor.life.poll_action().is_some() {}
        while events.try_recv().is_ok() {}
        assert!(moshpit_stream::view::paste(id, "late"));
        take_input(&mut actor, 1).await;
        assert_eq!(
            actor.pending.remove("%2"),
            Some(vec![(id, paste("late"))]),
            "it waits for the pane"
        );
        let discarded = std::iter::from_fn(|| events.try_recv().ok())
            .filter(|event| matches!(event.kind, EventKind::InputDiscarded { .. }))
            .count();
        assert_eq!(discarded, 0);
    }

    /// A plain shell with its stream, on `actor`.
    fn plain_shell(actor: &mut Actor) -> StreamId {
        let (stream, input) = moshpit_stream::open();
        let id = stream.id();
        actor.shell = Some(ShellChannel {
            stream: Arc::new(stream),
            pump: pump(id, input, actor.me.clone()),
            used: false,
            written: 0,
        });
        id
    }

    /// Handles what the mailbox brings up to a paste's landing, that too.
    async fn paste_landed(actor: &mut Actor) {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(5), actor.mailbox.recv())
                .await
                .expect("the paste's landing is said")
                .expect("the mailbox is open");
            let landed = matches!(message, Msg::PasteLanded { .. });
            actor.handle(message).await;
            if landed {
                return;
            }
        }
    }

    fn discarded(events: &mut mpsc::UnboundedReceiver<Event>) -> Vec<EventKind> {
        std::iter::from_fn(|| events.try_recv().ok())
            .map(|event| event.kind)
            .filter(|kind| matches!(kind, EventKind::InputDiscarded { .. }))
            .collect()
    }

    /// B322: keys typed under the reconnect's veil follow B396's rule. A
    /// multiplexer pane's wait for the pane, which comes back with the
    /// redial; a plain shell's are not sent (the redial is a new shell) and
    /// that is said, so the app tells the person once it is back. They were
    /// dropped before the core, unsaid.
    #[tokio::test]
    async fn keys_typed_under_the_reconnect_veil_wait_for_their_pane_or_are_said_dropped() {
        let (mut actor, mut events) = live_actor(false);
        let pane = tmux_pane(&mut actor, "%3");
        let shell = plain_shell(&mut actor);
        let generation = actor.life.generation();
        let now = actor.now();
        actor.life.lost(generation, now);
        while actor.life.poll_action().is_some() {}
        assert_eq!(actor.life.state(), SessionState::Reconnecting);
        while events.try_recv().is_ok() {}
        assert!(moshpit_stream::view::write(pane, b"git st"));
        assert!(moshpit_stream::view::write(shell, b"rm x\r"));
        take_input(&mut actor, 2).await;
        assert_eq!(
            actor.pending.remove("%3"),
            Some(vec![(pane, keys(b"git st"))]),
            "the pane's keys wait for it"
        );
        assert_eq!(
            discarded(&mut events),
            [EventKind::InputDiscarded {
                stream: shell,
                bytes: 5,
                why: Discarded::Reconnected,
            }],
            "the shell's are said dropped"
        );
    }

    /// B476: the shortcut bar's keys under the reconnect's veil are not the
    /// person's typing to deliver later (a stray tap must not land in an
    /// agent's prompt after the reconnect): not sent, and not kept, while
    /// the keyboard's keys around them wait for the pane in order.
    #[tokio::test]
    async fn the_bars_keys_under_the_reconnect_veil_are_not_kept() {
        let (mut actor, mut events) = live_actor(false);
        let pane = tmux_pane(&mut actor, "%4");
        let generation = actor.life.generation();
        let now = actor.now();
        actor.life.lost(generation, now);
        while actor.life.poll_action().is_some() {}
        while events.try_recv().is_ok() {}
        assert!(moshpit_stream::view::write(pane, b"ls"));
        assert!(moshpit_stream::view::bar(pane, b"\x03"));
        assert!(moshpit_stream::view::write(pane, b" -l"));
        take_input(&mut actor, 3).await;
        assert_eq!(
            actor.pending.remove("%4"),
            Some(vec![(pane, keys(b"ls")), (pane, keys(b" -l"))]),
            "the keyboard's keys wait for the pane, the bar's are not among them"
        );
        assert_eq!(
            discarded(&mut events),
            [EventKind::InputDiscarded {
                stream: pane,
                bytes: 1,
                why: Discarded::NotLive,
            }]
        );
        // Live, the bar's keys are keystrokes like any.
        let mut actor = live_actor(false).0;
        let pane = tmux_pane(&mut actor, "%5");
        assert!(moshpit_stream::view::bar(pane, b"\x1b[A"));
        take_input(&mut actor, 1).await;
        assert_eq!(
            actor.pending.remove("%5"),
            Some(vec![(pane, keys(b"\x1b[A"))])
        );
    }

    /// Before the first connect (nothing to come back to), typing is not
    /// sent, as before.
    #[tokio::test]
    async fn keys_typed_before_the_first_connect_are_not_sent() {
        let (mut actor, mut events) = dialing_actor(false);
        assert_eq!(actor.life.state(), SessionState::Connecting);
        let shell = plain_shell(&mut actor);
        assert!(moshpit_stream::view::write(shell, b"ls"));
        take_input(&mut actor, 1).await;
        assert_eq!(
            discarded(&mut events),
            [EventKind::InputDiscarded {
                stream: shell,
                bytes: 2,
                why: Discarded::NotLive,
            }]
        );
    }

    /// mosh + tmux, a swipe over an alternate-screen pane without the mouse
    /// (less, man): its ticks go to the sidecar, which presses the arrow
    /// keys (B500); over the primary screen, tmux's copy-mode down mosh as
    /// before. It went to copy-mode, which pages nothing there.
    #[tokio::test]
    async fn a_swipe_over_a_pager_under_mosh_tmux_goes_to_the_sidecar() {
        let (mut actor, _events) = live_actor(false);
        actor.spec.multiplexer = Multiplexer::Tmux;
        let (commands, mut sent) = mpsc::unbounded_channel();
        actor.leg = Some(Leg::Mosh {
            generation: actor.life.generation(),
            mosh: MoshLoop {
                commands,
                task: tokio::spawn(async {}),
            },
            server_pid: None,
            first_contact: true,
            return_path_reported: false,
            reaped: false,
        });
        let (to_sidecar, mut sidecar) = mpsc::unbounded_channel();
        actor.control = Some(Control {
            generation: actor.life.generation(),
            epoch: 1,
            ssh: None,
            sidecar: Some(TmuxLeg::for_tests(to_sidecar)),
            dialling: false,
            failures: 0,
            preferred: None,
            handoff: Handoff::default(),
            renderer_pid: None,
            grid: (80, 24),
            layouts_before_renderer: None,
            ready: true,
            checking: false,
            holding: false,
            held: Vec::new(),
            hold: 0,
            dropped: false,
        });
        let view = |alt_screen: bool| TmuxView {
            snapshot: Snapshot::default(),
            notice: None,
            refreshing: false,
            size_follow: None,
            pane_follow: None,
            agent_hooks: BTreeMap::new(),
            agent_hooks_loaded: false,
            input: moshpit_tmux::controller::InputFlags {
                alt_screen,
                ..Default::default()
            },
            no_session: false,
        };
        actor.update(|v| v.tmux_view = Some(view(true)));
        actor.handle(Msg::Tmux(TmuxAction::ScrollLines(3))).await;
        assert!(matches!(
            sidecar.try_recv(),
            Ok(TmuxCommand::Action(TmuxAction::ScrollLines(3)))
        ));
        assert!(sent.try_recv().is_err(), "nothing down mosh");
        // The primary screen: tmux's copy-mode, down mosh.
        actor.update(|v| v.tmux_view = Some(view(false)));
        actor.handle(Msg::Tmux(TmuxAction::ScrollLines(3))).await;
        assert!(matches!(
            sent.try_recv(),
            Ok(MoshCommand::CopyScroll { lines: 3, .. })
        ));
        let to_sidecar: Vec<TmuxCommand> = std::iter::from_fn(|| sidecar.try_recv().ok()).collect();
        assert!(
            !to_sidecar
                .iter()
                .any(|c| matches!(c, TmuxCommand::Action(TmuxAction::ScrollLines(_)))),
            "not the sidecar's"
        );
    }

    /// B614: mosh + tmux, the copy-mode tmux reports for the pane on screen
    /// goes to the mosh loop each time it changes — its latch follows it.
    #[tokio::test]
    async fn tmuxs_copy_mode_under_mosh_goes_to_the_loop_when_it_changes() {
        let (mut actor, _events) = live_actor(false);
        actor.spec.multiplexer = Multiplexer::Tmux;
        let (commands, mut sent) = mpsc::unbounded_channel();
        actor.leg = Some(Leg::Mosh {
            generation: actor.life.generation(),
            mosh: MoshLoop {
                commands,
                task: tokio::spawn(async {}),
            },
            server_pid: None,
            first_contact: true,
            return_path_reported: false,
            reaped: false,
        });
        let (to_sidecar, _sidecar) = mpsc::unbounded_channel();
        actor.control = Some(Control {
            generation: actor.life.generation(),
            epoch: 1,
            ssh: None,
            sidecar: Some(TmuxLeg::for_tests(to_sidecar)),
            dialling: false,
            failures: 0,
            preferred: None,
            handoff: Handoff::default(),
            renderer_pid: None,
            grid: (80, 24),
            layouts_before_renderer: None,
            ready: true,
            checking: false,
            holding: false,
            held: Vec::new(),
            hold: 0,
            dropped: false,
        });
        let tree = |in_copy_mode: bool| {
            TmuxNote::Tree(Box::new(TmuxView {
                snapshot: Snapshot::default(),
                notice: None,
                refreshing: false,
                size_follow: None,
                pane_follow: None,
                agent_hooks: BTreeMap::new(),
                agent_hooks_loaded: false,
                input: moshpit_tmux::controller::InputFlags {
                    in_copy_mode,
                    ..Default::default()
                },
                no_session: false,
            }))
        };
        let told = |sent: &mut mpsc::UnboundedReceiver<MoshCommand>| -> Vec<bool> {
            std::iter::from_fn(|| sent.try_recv().ok())
                .filter_map(|c| match c {
                    MoshCommand::CopyModeIs(on) => Some(on),
                    _ => None,
                })
                .collect()
        };
        actor.sidecar_note(1, tree(true));
        assert_eq!(told(&mut sent), [true]);
        actor.sidecar_note(1, tree(true));
        assert_eq!(told(&mut sent), Vec::<bool>::new(), "only a change");
        actor.sidecar_note(1, tree(false));
        assert_eq!(told(&mut sent), [false]);
    }

    /// B524: a reply for a pane asked for in the background, on a live SSH
    /// session not heard from lately, waits for the transport's answer to
    /// a question first. It went out at once, into a socket iOS may have
    /// killed while the app slept, and was said sent: a notification's
    /// Allow or reply that reached nothing.
    #[tokio::test(start_paused = true)]
    async fn a_delivery_in_the_background_waits_for_the_transports_answer() {
        let (mut actor, _events) = live_actor(false);
        let now = actor.now();
        actor.life.background(now);
        while actor.life.poll_action().is_some() {}
        // Half a minute without a byte.
        tokio::time::advance(Duration::from_secs(30)).await;
        let (reply, _answer) = oneshot::channel();
        actor
            .handle(Msg::Deliver {
                pane: SHELL_PANE.to_owned(),
                bytes: b"y\r".to_vec(),
                paste: false,
                reply,
            })
            .await;
        let asked: Vec<Action> = std::iter::from_fn(|| actor.life.poll_action()).collect();
        assert!(
            asked.iter().any(|a| matches!(a, Action::Probe { .. })),
            "the transport is asked: {asked:?}"
        );
        assert_eq!(actor.deliveries.len(), 1, "the reply waits");
    }

    /// B525: over plain mosh, what follows a paste waits for mosh-server's
    /// ack of the paste's state, then the paste's time: a paste state lost
    /// on the way goes again as one diff with the Return typed after it
    /// (mosh retransmits from the acked state to the newest), mosh-server
    /// writes them together, and Claude Code drops the Return behind an
    /// image path's paste. B687: however long the ack takes — it went
    /// after 2 s, into a slow path's retransmitted diff — and when the loop
    /// ends without it, what follows never goes (a reconnect's shell is a
    /// fresh one) and is said.
    #[tokio::test(start_paused = true)]
    async fn over_mosh_what_follows_a_paste_waits_for_its_ack() {
        let (mut actor, mut events) = live_actor(false);
        let (commands, mut sent) = mpsc::unbounded_channel();
        actor.leg = Some(Leg::Mosh {
            generation: actor.life.generation(),
            mosh: MoshLoop {
                commands,
                task: tokio::spawn(async {}),
            },
            server_pid: None,
            first_contact: true,
            return_path_reported: false,
            reaped: false,
        });
        let keys = |sent: &mut mpsc::UnboundedReceiver<MoshCommand>| -> (Vec<Vec<u8>>, Vec<oneshot::Sender<()>>) {
            let (mut keys, mut barriers) = (Vec::new(), Vec::new());
            while let Ok(command) = sent.try_recv() {
                match command {
                    MoshCommand::Keys { bytes, .. } => keys.push(bytes),
                    MoshCommand::Barrier(reply) => barriers.push(reply),
                    _ => {}
                }
            }
            (keys, barriers)
        };
        actor.send_or_wait(Dest::Shell, 1, Typed::Paste("/tmp/a.jpg".into()), 0);
        actor.send_or_wait(Dest::Shell, 1, Typed::Bytes(b"\r".to_vec()), 1);
        let (pasted, mut barriers) = keys(&mut sent);
        assert_eq!(pasted.len(), 1, "the paste");
        assert_eq!(barriers.len(), 1, "the paste's ack is asked for");
        // Its time is up, its ack not here: the Return waits.
        let later = actor.now() + 500;
        actor.release_settled(later);
        assert!(keys(&mut sent).0.is_empty(), "no ack yet");
        // The ack.
        let _ = barriers.remove(0).send(());
        let landed = actor.mailbox.recv().await.unwrap();
        actor.handle(landed).await;
        let past = actor.now() + 200;
        actor.release_settled(past);
        assert_eq!(keys(&mut sent).0, vec![b"\r".to_vec()], "then the Return");

        // No ack, however long: the key waits.
        actor.send_or_wait(Dest::Shell, 1, Typed::Paste("x".into()), 2);
        actor.send_or_wait(Dest::Shell, 1, Typed::Bytes(b"y".to_vec()), 3);
        let (_, mut barriers) = keys(&mut sent);
        let _ = discarded(&mut events);
        let start = actor.now();
        actor.release_settled(start + 60_000);
        assert!(keys(&mut sent).0.is_empty(), "no ack, a minute on");
        // The loop ends without it: the key never goes, and is said.
        drop(barriers.remove(0));
        paste_landed(&mut actor).await;
        actor.release_settled(start + 120_000);
        assert!(keys(&mut sent).0.is_empty(), "never into a fresh shell");
        assert_eq!(
            discarded(&mut events),
            [EventKind::InputDiscarded {
                stream: 1,
                bytes: 1,
                why: Discarded::Reconnected,
            }]
        );
        assert!(actor.settling.is_empty());
    }

    /// B689: an unbounded wait is never invisible. A Return held behind a
    /// paste mosh-server hasn't acknowledged shows, past a second, as
    /// mosh's reconnecting state (the amber pill, "mosh is reconnecting");
    /// herdr's plane holding typing shows the same; and the person closing
    /// the session says the Return wasn't sent, with its count.
    #[tokio::test(start_paused = true)]
    async fn typing_held_past_a_second_is_shown_and_said_when_the_session_closes() {
        let (mut actor, mut events) = live_actor(false);
        let generation = actor.life.generation();
        let (commands, _sent) = mpsc::unbounded_channel();
        actor.leg = Some(Leg::Mosh {
            generation,
            mosh: MoshLoop {
                commands,
                task: tokio::spawn(async {}),
            },
            server_pid: None,
            first_contact: true,
            return_path_reported: false,
            reaped: false,
        });
        let held = |actor: &Actor| {
            let view = actor.view.lock().unwrap();
            (view.input_held, view.mosh_roaming)
        };
        let roaming = |events: &mut mpsc::UnboundedReceiver<Event>| -> Vec<bool> {
            std::iter::from_fn(|| events.try_recv().ok())
                .filter_map(|event| match event.kind {
                    EventKind::MoshMetrics { roaming, .. } => Some(roaming),
                    _ => None,
                })
                .collect()
        };
        actor.send_or_wait(Dest::Shell, 1, Typed::Paste("/tmp/a.jpg".into()), 0);
        actor.send_or_wait(Dest::Shell, 1, Typed::Bytes(b"\r".to_vec()), 1);
        let since = actor.settling[&Dest::Shell].since;
        assert_eq!(
            actor.held_due(),
            Some(since + HELD_SHOWN),
            "a wake-up is due"
        );
        actor.release_settled(since + 900);
        assert_eq!(held(&actor), (false, false));
        actor.release_settled(since + HELD_SHOWN);
        assert_eq!(held(&actor), (true, true), "shown past a second");
        assert_eq!(
            roaming(&mut events),
            [true],
            "the pill and the banner hear it"
        );
        assert_eq!(actor.held_due(), None, "said once");

        // herdr's plane saying it holds typing shows the same.
        let (plane, _asked) = mpsc::unbounded_channel();
        actor.herdr_mosh = Some(HerdrMoshControl {
            generation,
            leg: HerdrMoshLeg::fake(plane),
            ssh: None,
            epoch: 1,
            dialling: false,
            failures: 0,
        });
        actor.herdr_mosh_note(generation, HerdrMoshNote::InputHeld(true));
        assert!(actor.plane_held);
        assert_eq!(held(&actor), (true, true));

        // The person closes the session: the Return is said, not sent, and
        // nothing waits any more.
        actor.teardown(generation, Teardown::Disconnect).await;
        assert_eq!(held(&actor), (false, false));
        let said: Vec<EventKind> = std::iter::from_fn(|| events.try_recv().ok())
            .map(|event| event.kind)
            .filter(|kind| {
                matches!(
                    kind,
                    EventKind::InputDiscarded { .. } | EventKind::MoshMetrics { .. }
                )
            })
            .collect();
        assert!(
            said.contains(&EventKind::InputDiscarded {
                stream: 1,
                bytes: 1,
                why: Discarded::SessionEnded,
            }),
            "{said:?}"
        );
        assert!(
            said.iter()
                .any(|kind| matches!(kind, EventKind::MoshMetrics { roaming: false, .. })),
            "{said:?}"
        );
        assert!(actor.settling.is_empty());
    }

    /// B687: over herdr and mosh, what follows a paste waits for herdr's
    /// plane to say the paste is in the tty (mosh-server acknowledged it
    /// behind whatever the plane's lane held before it), as over plain mosh
    /// (B525) — it waited only the paste's 150 ms, and an image path's
    /// Return could join its retransmitted diff. A paste the plane dropped
    /// (typing not going now) takes what follows it with it, said: a bare
    /// Return submitted the prompt without the image.
    #[tokio::test(start_paused = true)]
    async fn over_herdr_and_mosh_what_follows_a_paste_waits_for_the_plane_to_land_it() {
        let (mut actor, mut events) = live_actor(false);
        let generation = actor.life.generation();
        let (commands, _sent) = mpsc::unbounded_channel();
        actor.leg = Some(Leg::Mosh {
            generation,
            mosh: MoshLoop {
                commands,
                task: tokio::spawn(async {}),
            },
            server_pid: None,
            first_contact: true,
            return_path_reported: false,
            reaped: false,
        });
        let (plane, mut asked) = mpsc::unbounded_channel();
        actor.herdr_mosh = Some(HerdrMoshControl {
            generation,
            leg: HerdrMoshLeg::fake(plane),
            ssh: None,
            epoch: 1,
            dialling: false,
            failures: 0,
        });
        // What reached the plane: typing, and each paste's question.
        let take = |asked: &mut mpsc::UnboundedReceiver<HerdrMoshCommand>| {
            let (mut typed, mut pastes) = (Vec::new(), Vec::new());
            while let Ok(command) = asked.try_recv() {
                match command {
                    HerdrMoshCommand::Typed { bytes, .. } => typed.push(bytes),
                    HerdrMoshCommand::Paste { text, landed, .. } => {
                        pastes.push((text, landed.expect("the paste's landing is asked")))
                    }
                    _ => {}
                }
            }
            (typed, pastes)
        };
        actor.send_or_wait(Dest::Shell, 1, Typed::Paste(" /tmp/a.jpg ".into()), 0);
        actor.send_or_wait(Dest::Shell, 1, Typed::Bytes(b"\r".to_vec()), 12);
        let (typed, mut pastes) = take(&mut asked);
        assert!(typed.is_empty());
        assert_eq!(pastes.len(), 1);
        let start = actor.now();
        actor.release_settled(start + 60_000);
        assert!(
            take(&mut asked).0.is_empty(),
            "the Return waits for the plane"
        );
        let _ = pastes.remove(0).1.send(true);
        paste_landed(&mut actor).await;
        actor.release_settled(start + 60_200);
        assert_eq!(take(&mut asked).0, vec![b"\r".to_vec()], "then the Return");

        // Dropped by the plane: the Return behind it never goes, said.
        let _ = discarded(&mut events);
        actor.send_or_wait(Dest::Shell, 1, Typed::Paste(" /tmp/b.jpg ".into()), 13);
        actor.send_or_wait(Dest::Shell, 1, Typed::Bytes(b"\r".to_vec()), 25);
        let (_, mut pastes) = take(&mut asked);
        let _ = pastes.remove(0).1.send(false);
        paste_landed(&mut actor).await;
        actor.release_settled(start + 120_000);
        assert!(take(&mut asked).0.is_empty(), "no bare Return");
        assert_eq!(
            discarded(&mut events),
            [EventKind::InputDiscarded {
                stream: 1,
                bytes: 1,
                why: Discarded::NotAttached,
            }]
        );
    }

    thread_local! {
        /// The device's sleep, as a test says it.
        static SLEPT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }

    fn test_slept() -> u64 {
        SLEPT.with(|slept| slept.get())
    }

    /// B611: the time the device slept counts. `Instant` stops while a
    /// phone sleeps (Apple, Linux): a lock of minutes came back as a few
    /// seconds away, under the 20 s that forces a reconnect, and keys typed
    /// in the next 4 s went into a dead socket. A deadline after a sleep
    /// sleeps for what is left of it, not the slept time again.
    #[tokio::test(start_paused = true)]
    async fn a_return_counts_the_time_the_device_slept() {
        SLEPT.with(|slept| slept.set(0));
        let clock = Clock::with_sleep(test_slept);
        let t0 = clock.now();
        tokio::time::advance(Duration::from_secs(1)).await;
        SLEPT.with(|slept| slept.set(60_000));
        assert!(clock.now() - t0 >= 61_000, "{}", clock.now() - t0);
        let soon = clock.now() + 200;
        assert_eq!(
            clock.instant(soon) - Instant::now(),
            Duration::from_millis(200)
        );

        SLEPT.with(|slept| slept.set(0));
        let (mut actor, _events) = live_actor(false);
        actor.clock = Clock::with_sleep(test_slept);
        let now = actor.now();
        actor.life.background(now);
        while actor.life.poll_action().is_some() {}
        tokio::time::advance(Duration::from_secs(1)).await;
        SLEPT.with(|slept| slept.set(5 * 60_000));
        let now = actor.now();
        actor.life.foreground(now);
        let actions: Vec<Action> = std::iter::from_fn(|| actor.life.poll_action()).collect();
        assert!(
            actions.iter().any(|a| matches!(
                a,
                Action::Teardown {
                    why: Teardown::Reconnect,
                    ..
                }
            )),
            "five minutes asleep is a long absence: {actions:?}"
        );
    }

    /// B617: a session on SSH shows nothing of mosh's — not "roaming", no
    /// round trip, no counters. They stayed from the mosh flow it left (a
    /// switch to SSH is made exactly while mosh roams with no replies), and
    /// the Home card kept saying "roaming" over an SSH session.
    #[tokio::test]
    async fn an_ssh_session_shows_nothing_of_moshs_flow() {
        let (mut actor, _events) = live_actor(false);
        actor.update(|view| {
            view.mosh_roaming = true;
            view.mosh_srtt_ms = Some(900.0);
            view.mosh_diagnostics = Some(moshpit_mosh::Diagnostics::default());
        });
        let now = actor.now();
        actor.life.lost(actor.life.generation(), now);
        let generation = actor.life.generation();
        assert!(
            actor
                .life
                .attempt_succeeded(generation, Transport::Ssh, now + 1)
        );
        actor.act().await;
        let view = actor.view.lock().unwrap().clone();
        assert_eq!(view.state, SessionState::Connected);
        assert!(!view.mosh_roaming, "not roaming");
        assert_eq!(view.mosh_srtt_ms, None);
        assert!(view.mosh_diagnostics.is_none());
    }

    /// B618: keys a mosh flow never delivered go with it when another server
    /// replaces it (a reconnect is a fresh shell), and that is said with how
    /// many: never dropped in silence.
    #[tokio::test]
    async fn a_replaced_mosh_flow_says_what_it_never_delivered() {
        let (mut actor, mut events) = live_actor(false);
        let generation = actor.life.generation();
        let (commands, mut sent) = mpsc::unbounded_channel();
        actor.leg = Some(Leg::Mosh {
            generation,
            mosh: MoshLoop {
                commands,
                task: tokio::spawn(async {}),
            },
            server_pid: None,
            first_contact: true,
            return_path_reported: false,
            reaped: false,
        });
        let (stream, input) = moshpit_stream::open();
        let id = stream.id();
        actor.shell = Some(ShellChannel {
            stream: Arc::new(stream),
            pump: pump(id, input, actor.me.clone()),
            used: false,
            written: 0,
        });
        tokio::spawn(async move {
            while let Some(command) = sent.recv().await {
                if let MoshCommand::Undelivered(reply) = command {
                    let _ = reply.send(3);
                }
            }
        });
        actor.teardown(generation, Teardown::Reconnect).await;
        assert_eq!(
            discarded(&mut events),
            [EventKind::InputDiscarded {
                stream: id,
                bytes: 3,
                why: Discarded::Reconnected,
            }]
        );
    }

    /// B529: back from the background in place, before the transport
    /// answered: a size change given after keys held for the answer waits
    /// with them and goes out in order (V31). It went at once, ahead of the
    /// keys typed before it. With nothing held for its stream it still goes
    /// at once.
    #[tokio::test]
    async fn a_resize_during_the_return_waits_behind_the_keys_typed_before_it() {
        let (mut actor, _events) = live_actor(true);
        let generation = actor.life.generation();
        let (commands, mut sent) = mpsc::unbounded_channel();
        actor.leg = Some(Leg::Mosh {
            generation,
            mosh: MoshLoop {
                commands,
                task: tokio::spawn(async {}),
            },
            server_pid: None,
            first_contact: true,
            return_path_reported: false,
            reaped: false,
        });
        let (stream, input) = moshpit_stream::open();
        let id = stream.id();
        actor.shell = Some(ShellChannel {
            stream: Arc::new(stream),
            pump: pump(id, input, actor.me.clone()),
            used: false,
            written: 0,
        });
        let seen = |sent: &mut mpsc::UnboundedReceiver<MoshCommand>| -> Vec<String> {
            std::iter::from_fn(|| sent.try_recv().ok())
                .filter_map(|command| match command {
                    MoshCommand::Keys { bytes, .. } => {
                        Some(String::from_utf8_lossy(&bytes).into_owned())
                    }
                    MoshCommand::Resize { cols, rows } => Some(format!("{cols}x{rows}")),
                    _ => None,
                })
                .collect()
        };
        let now = actor.now();
        actor.life.background(now);
        actor.life.foreground(now + 30_000);
        let question = asked(&mut actor);
        assert!(actor.life.resume_pending());
        // Nothing held yet: the size goes now.
        actor.input(id, InputEvent::Resize { cols: 90, rows: 20 });
        assert_eq!(seen(&mut sent), ["90x20"]);
        actor.input(id, InputEvent::Bytes(b"a".to_vec()));
        actor.input(
            id,
            InputEvent::Resize {
                cols: 100,
                rows: 30,
            },
        );
        actor.input(id, InputEvent::Bytes(b"b".to_vec()));
        assert_eq!(seen(&mut sent), Vec::<String>::new(), "all three wait");
        actor
            .life
            .probe_answered(generation, question, Probe::Alive, now + 30_080);
        assert!(!actor.life.resume_pending());
        actor.release_held();
        assert_eq!(seen(&mut sent), ["a", "100x30", "b"], "in the order given");
    }

    /// B528: a size change given after keys that wait behind a paste waits
    /// with them, in order (V31: a size lands between the bytes it came
    /// between). It overtook them: the program saw the new size before the
    /// keys typed ahead of it.
    #[tokio::test]
    async fn a_resize_behind_a_paste_waits_with_the_keys_in_order() {
        let (mut actor, _events) = live_actor(false);
        let (commands, mut sent) = mpsc::unbounded_channel();
        actor.leg = Some(Leg::Mosh {
            generation: actor.life.generation(),
            mosh: MoshLoop {
                commands,
                task: tokio::spawn(async {}),
            },
            server_pid: None,
            first_contact: true,
            return_path_reported: false,
            reaped: false,
        });
        let (stream, input) = moshpit_stream::open();
        let id = stream.id();
        actor.shell = Some(ShellChannel {
            stream: Arc::new(stream),
            pump: pump(id, input, actor.me.clone()),
            used: false,
            written: 0,
        });
        // mosh-server acknowledges the paste as it is asked (B687: what
        // follows waits for that, however long).
        let seen = |sent: &mut mpsc::UnboundedReceiver<MoshCommand>| -> Vec<String> {
            std::iter::from_fn(|| sent.try_recv().ok())
                .filter_map(|command| match command {
                    MoshCommand::Keys { bytes, .. } => {
                        Some(String::from_utf8_lossy(&bytes).into_owned())
                    }
                    MoshCommand::Resize { cols, rows } => Some(format!("{cols}x{rows}")),
                    MoshCommand::Barrier(reply) => {
                        let _ = reply.send(());
                        None
                    }
                    _ => None,
                })
                .collect()
        };
        actor.input(id, InputEvent::Paste("p".into()));
        // Nothing waits yet: the size goes now, after the paste.
        actor.input(id, InputEvent::Resize { cols: 90, rows: 20 });
        assert_eq!(seen(&mut sent), ["\u{1b}[200~p\u{1b}[201~", "90x20"]);
        actor.input(id, InputEvent::Bytes(b"a".to_vec()));
        actor.input(
            id,
            InputEvent::Resize {
                cols: 100,
                rows: 30,
            },
        );
        actor.input(id, InputEvent::Bytes(b"b".to_vec()));
        assert_eq!(seen(&mut sent), Vec::<String>::new(), "all three wait");
        paste_landed(&mut actor).await;
        let past = actor.now() + 10_000;
        actor.release_settled(past);
        assert_eq!(seen(&mut sent), ["a", "100x30", "b"], "in the order given");
    }

    /// A live actor on a mosh leg whose mosh-server is `pid`; what it asks
    /// of the mosh loop.
    fn on_mosh(
        pid: i64,
    ) -> (
        Actor,
        mpsc::UnboundedReceiver<Event>,
        mpsc::UnboundedReceiver<MoshCommand>,
    ) {
        let (mut actor, events) = live_actor(false);
        let (commands, sent) = mpsc::unbounded_channel();
        actor.leg = Some(Leg::Mosh {
            generation: actor.life.generation(),
            mosh: MoshLoop {
                commands,
                task: tokio::spawn(async {}),
            },
            server_pid: Some(pid),
            first_contact: false,
            return_path_reported: false,
            reaped: false,
        });
        (actor, events, sent)
    }

    fn reaped(actor: &Actor) -> bool {
        matches!(actor.leg, Some(Leg::Mosh { reaped: true, .. }))
    }

    /// A reply for a pane, given to the session; its answer.
    async fn deliver_to(actor: &mut Actor, pane: &str) -> oneshot::Receiver<bool> {
        let (reply, answer) = oneshot::channel();
        actor
            .handle(Msg::Deliver {
                pane: pane.to_owned(),
                bytes: b"y\r".to_vec(),
                paste: false,
                reply,
            })
            .await;
        answer
    }

    fn went_down_mosh(sent: &mut mpsc::UnboundedReceiver<MoshCommand>) -> bool {
        std::iter::from_fn(|| sent.try_recv().ok()).any(|c| matches!(c, MoshCommand::Keys { .. }))
    }

    /// A reply for a tmux pane over mosh (the Swift client's
    /// MoshTmuxDeliveryTests, kept when it went, E2). With no sidecar to
    /// place it, it waits, and past the wait it is answered "not sent";
    /// with tmux missing on the host it is refused at once; a pane the
    /// sidecar's tree doesn't list is refused at once, one it lists goes to
    /// the sidecar. Never down mosh: mosh's shell is tmux's own client,
    /// whose active pane it would land in.
    #[tokio::test]
    async fn a_reply_for_a_tmux_pane_over_mosh_waits_for_the_sidecar_or_is_refused() {
        // No sidecar: it waits, then "not sent".
        let (mut actor, _events, mut sent) = on_mosh(4242);
        actor.spec.multiplexer = Multiplexer::Tmux;
        let mut answer = deliver_to(&mut actor, "%1").await;
        assert!(answer.try_recv().is_err(), "it waits for the sidecar");
        let past = actor.now() + crate::spec::ms(DELIVERY_WAIT);
        actor.expire_deliveries(past);
        assert_eq!(answer.await, Ok(false));
        assert!(!went_down_mosh(&mut sent));

        // tmux missing on the host: refused at once.
        let (mut actor, _events, mut sent) = on_mosh(4242);
        actor.spec.multiplexer = Multiplexer::Tmux;
        actor.update(|view| view.degrade = Some(Degrade::TmuxMissing));
        assert_eq!(deliver_to(&mut actor, "%1").await.await, Ok(false));
        assert!(!went_down_mosh(&mut sent));

        // The sidecar lists %0 and %1: %9 is refused, %1 goes to it.
        let (mut actor, _events, mut sent) = on_mosh(4242);
        actor.spec.multiplexer = Multiplexer::Tmux;
        let (commands, mut sidecar) = mpsc::unbounded_channel();
        actor.control = Some(Control {
            generation: actor.life.generation(),
            epoch: 1,
            ssh: None,
            sidecar: Some(TmuxLeg::for_tests(commands)),
            dialling: false,
            failures: 0,
            preferred: None,
            handoff: Handoff::default(),
            renderer_pid: None,
            grid: (80, 24),
            layouts_before_renderer: None,
            ready: true,
            checking: false,
            holding: false,
            held: Vec::new(),
            hold: 0,
            dropped: false,
        });
        let mut tree = Snapshot {
            is_attached: true,
            ..Snapshot::default()
        };
        for pane in ["%0", "%1"] {
            tree.panes.insert(
                pane.into(),
                moshpit_tmux::model::PaneInfo::placeholder(pane, "@1"),
            );
        }
        actor.update(|view| view.tmux = Some(tree));
        assert_eq!(
            deliver_to(&mut actor, "%9").await.await,
            Ok(false),
            "not listed"
        );
        let _answer = deliver_to(&mut actor, "%1").await;
        let to_sidecar: Vec<TmuxCommand> = std::iter::from_fn(|| sidecar.try_recv().ok()).collect();
        assert!(
            to_sidecar
                .iter()
                .any(|c| matches!(c, TmuxCommand::Input { pane, .. } if pane == "%1")),
            "by its id"
        );
        assert!(!went_down_mosh(&mut sent));
    }

    /// The return path dies (the Swift client's MoshReturnPathReapTests,
    /// kept when it went, E2): a mosh-server never heard from waits forever
    /// and is stopped; one that answered keeps the session, which stays
    /// connected with the banner up. Said once either way.
    #[tokio::test]
    async fn a_dead_return_path_stops_only_a_server_never_heard_from() {
        for heard in [false, true] {
            let (mut actor, mut events, _sent) = on_mosh(4242);
            let generation = actor.life.generation();
            for _ in 0..2 {
                actor
                    .handle(Msg::Mosh {
                        generation,
                        note: MoshNote::ReturnPathDead {
                            first_contact: heard,
                        },
                    })
                    .await;
            }
            assert_eq!(reaped(&actor), !heard, "heard: {heard}");
            assert_eq!(
                actor.owed.len(),
                usize::from(!heard),
                "one stop, heard: {heard}"
            );
            assert_eq!(actor.life.state(), SessionState::Connected);
            assert!(actor.view.lock().unwrap().return_path_dead);
            let banners = std::iter::from_fn(|| events.try_recv().ok())
                .filter(|e| e.kind == EventKind::ReturnPathDead)
                .count();
            assert_eq!(banners, 1, "said once, heard: {heard}");
        }
    }

    /// B721: the session's reap of a mosh-server never heard from tells the
    /// lifecycle, which dials a new server at the return; one that answered
    /// keeps its flow.
    #[tokio::test]
    async fn a_reaped_server_is_replaced_at_the_return() {
        for heard in [false, true] {
            let (mut actor, _events) = dialing_actor(false);
            let generation = actor.life.generation();
            let now = actor.now();
            assert!(
                actor
                    .life
                    .attempt_succeeded(generation, Transport::Mosh, now)
            );
            while actor.life.poll_action().is_some() {}
            let (commands, _sent) = mpsc::unbounded_channel();
            actor.leg = Some(Leg::Mosh {
                generation,
                mosh: MoshLoop {
                    commands,
                    task: tokio::spawn(async {}),
                },
                server_pid: Some(4242),
                first_contact: heard,
                return_path_reported: false,
                reaped: false,
            });
            actor
                .handle(Msg::Mosh {
                    generation,
                    note: MoshNote::ReturnPathDead {
                        first_contact: heard,
                    },
                })
                .await;
            let now = actor.now();
            actor.life.background(now);
            while actor.life.poll_action().is_some() {}
            actor.life.foreground(now + 2_000);
            let redial = std::iter::from_fn(|| actor.life.poll_action()).any(|a| {
                matches!(
                    a,
                    Action::Teardown {
                        why: Teardown::Reconnect,
                        ..
                    }
                )
            });
            assert_eq!(redial, !heard, "a new server at the return, heard: {heard}");
        }
    }

    /// B724: a mosh + tmux session that ends (a detach, the server's own end)
    /// hands the window pins back at once, over the control connection
    /// still open, as the person's Disconnect does (B588's hand-back). The
    /// debt waited for the card to close or a reconnect: killed first, the
    /// desktop's windows stayed at the phone's size for good.
    #[tokio::test]
    async fn an_ended_mosh_tmux_session_hands_the_pins_back_at_once() {
        for (why, courteous) in [
            (Teardown::Ended, true),
            (Teardown::Disconnect, true),
            (Teardown::Reconnect, false),
        ] {
            let (mut actor, _events, _sent) = on_mosh(4242);
            let generation = actor.life.generation();
            let (to_sidecar, mut sidecar) = mpsc::unbounded_channel();
            actor.control = Some(Control {
                generation,
                epoch: 1,
                ssh: None,
                sidecar: Some(TmuxLeg::for_tests(to_sidecar)),
                dialling: false,
                failures: 0,
                preferred: None,
                handoff: Handoff::default(),
                renderer_pid: None,
                grid: (80, 24),
                layouts_before_renderer: None,
                ready: true,
                checking: false,
                holding: false,
                held: Vec::new(),
                hold: 0,
                dropped: false,
            });
            let closing = tokio::spawn(async move {
                while let Some(command) = sidecar.recv().await {
                    if let TmuxCommand::Close { courtesy, reply } = command {
                        let _ = reply.send(Handoff::default());
                        return Some(courtesy);
                    }
                }
                None
            });
            actor.teardown(generation, why).await;
            assert_eq!(
                closing.await.unwrap(),
                Some(courteous),
                "{why:?}: the pins handed back"
            );
        }
    }

    /// On mosh a paste from the stream goes as one bracketed paste among the
    /// keys, at the offset the keys before it end at: it isn't stream bytes.
    #[tokio::test]
    async fn on_mosh_a_paste_goes_bracketed_between_the_keys() {
        let (mut actor, _events) = live_actor(false);
        let (commands, mut sent) = mpsc::unbounded_channel();
        actor.leg = Some(Leg::Mosh {
            generation: actor.life.generation(),
            mosh: MoshLoop {
                commands,
                task: tokio::spawn(async {}),
            },
            server_pid: None,
            first_contact: false,
            return_path_reported: false,
            reaped: false,
        });
        let (stream, input) = moshpit_stream::open();
        let id = stream.id();
        actor.shell = Some(ShellChannel {
            stream: Arc::new(stream),
            pump: pump(id, input, actor.me.clone()),
            used: false,
            written: 0,
        });
        assert!(moshpit_stream::view::write(id, b"ab"));
        assert!(moshpit_stream::view::paste(id, "p\x1b[201~q"));
        assert!(moshpit_stream::view::write(id, b"c"));
        take_input(&mut actor, 3).await;
        // The key after the paste waits behind it (B485), for the paste's
        // ack (B525), ending where it did when typed.
        let mut went: Vec<(Vec<u8>, u64)> = Vec::new();
        let take = |sent: &mut mpsc::UnboundedReceiver<MoshCommand>,
                    went: &mut Vec<(Vec<u8>, u64)>| {
            while let Ok(command) = sent.try_recv() {
                match command {
                    MoshCommand::Keys { bytes, end } => went.push((bytes, end)),
                    MoshCommand::Barrier(reply) => {
                        let _ = reply.send(());
                    }
                    _ => {}
                }
            }
        };
        take(&mut sent, &mut went);
        paste_landed(&mut actor).await;
        let until = actor.settling[&Dest::Shell].until;
        actor.release_settled(until);
        take(&mut sent, &mut went);
        assert_eq!(
            went,
            [
                (b"ab".to_vec(), 2),
                (b"\x1b[200~pq\x1b[201~".to_vec(), 2),
                (b"c".to_vec(), 3),
            ]
        );
    }

    #[test]
    fn a_tmux_prefix_is_its_control_byte() {
        assert_eq!(prefix_bytes("C-b"), [0x02]);
        assert_eq!(prefix_bytes("C-a"), [0x01]);
        assert_eq!(
            prefix_bytes(" C-Space "),
            [0x02],
            "not one key: the default"
        );
        assert_eq!(prefix_bytes("C-]"), [0x1d]);
        assert_eq!(prefix_bytes("M-a"), [0x02]);
        assert_eq!(prefix_bytes(""), [0x02]);
    }
}
