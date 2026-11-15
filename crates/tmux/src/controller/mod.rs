//! The tmux control-mode controller: a port of
//! `Moshpit/Services/Tmux/TmuxSessionController.swift` as a sans-I/O state
//! machine.
//!
//! Inputs, each with the caller's monotonic clock in milliseconds: bytes off
//! the control channel ([`Controller::receive`], or parser [`Event`]s through
//! [`Controller::handle_event`]), the channel ending, user intents (attach,
//! select, resize, keys, paste, scroll, …), reports from the pane terminal
//! views, and timer ticks ([`Controller::tick`] once
//! [`Controller::next_deadline`] has passed). Outputs queue up as [`Output`]s
//! in order: tmux commands to write (each one line, `\n` appended by the
//! writer, in this order on one channel), bytes for a pane's terminal
//! addressed by the pane's stream id, what the pane's view should do (veil,
//! reveal, …), and notifications for the session and the UI. The controller
//! never owns a terminal view: a pane's view is whatever the host attached to
//! its stream.
//!
//! Ordering is the controller's whole contract, as it was the Swift one's:
//! tmux answers every command with one `%begin … %end` block, in the order
//! the commands were sent, so every command takes a reply slot and the
//! replies are paired by position. Two kinds of block break that count, and
//! neither is paired (B273): a block for a command we never sent (an
//! `after-<command>` hook one of ours fired, the late half of an
//! `if-shell -b`: tmux flags those 0, [`CommandResponse::from_client`]), and
//! the extra blocks of a command that answers more than once (`if-shell`
//! running its command, `source-file`, `a ; b`: [`answers_once`] says no,
//! and a fence follows it — every block up to the fence's is its own).
//! Every repair frame lands on the pane's
//! stream after the bytes that came before it on the control channel; and
//! nothing on the byte path takes an unordered hop.
//!
//! Behaviour is the Swift controller's at `5337db09`, line for line, proven
//! on scripted sessions by the equivalence harness (`harness/controller/`).
//! Where the port differs it says so (`Fix (<id>)`, listed in
//! `harness/controller/divergences.json`).

mod group;
mod input;
mod repair;
mod sidecar;
mod size;
mod timers;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use moshpit_herdr::text::{trim_whitespaces, trim_whitespaces_and_newlines};

use crate::bell::BellDetector;
use crate::control::{CommandResponse, ControlParser, Event};
use crate::format::{
    NAME_REFUSAL, command_and_path, fields, is_valid_id, name_argument, refuses_name, swift_int,
};
use crate::hooks::AgentHook;
use crate::model::{PaneInfo, Selection, SessionInfo, Snapshot, WindowInfo, id_cmp};

pub use group::{GROUP_MARK, GROUP_OF_OPTION, is_group_name};
use group::{GroupState, PhoneGroup};
pub use input::{InputFlags, PixelScroll, ScrollRoute, click_bytes, wheel_bytes, wheel_cell};
pub use sidecar::{OWNED_OPTION, STATUS_HIDDEN_OPTION};
use size::LeaseThen;
pub use size::{
    PRISTINE_PROBE_FORMAT, PinLedger, PinReason, PristineLayout, SIZE_PROBE_FORMAT, SWEEP_FORMAT,
    ZOOM_MARK_OPTION, zoom_command,
};
use timers::{Action, Slot, Timers};

/// A pane's byte stream: the id the host attaches a terminal view to.
pub type StreamId = u64;

/// The pane discovery format, quoted (`paneDiscoveryFormat`): parsed by
/// position, the command and the path last because they may hold spaces
/// ([`crate::format::command_and_path`]).
pub const PANE_DISCOVERY_FORMAT: &str = "'#{pane_id} #{window_id} #{pane_index} #{pane_width} #{pane_height} #{pane_active} #{alternate_on} #{pane_in_mode} #{pane_current_command} #{pane_current_path}'";

const LIST_SESSIONS: &str = "list-sessions -F '#{session_id} #{session_attached} #{session_name}'";
const LIST_WINDOWS: &str = "list-windows -a -F '#{session_id} #{window_id} #{window_index} #{window_layout} #{window_active} #{window_panes} #{window_name}'";

/// How many lines of history a pane's backfill replays (`backfillHistoryLines`).
pub const BACKFILL_HISTORY_LINES: i64 = 400;

/// The controller's knobs; [`Default`] gives the app's values. The timing
/// ones are the Swift controller's injectable properties.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `true`: this controller renders pane content (SSH + tmux). `false`:
    /// control plane only — the mosh sidecar, which never reports a size, so
    /// tmux sends it no `%output` and leaves it out of window sizing.
    pub renders_output: bool,
    /// What our size leases call this device ("iPhone", "iPad").
    pub size_owner_label: String,
    /// Unix time in milliseconds minus the monotonic clock the inputs carry:
    /// the size lease's stamps are wall-clock seconds shared with the other
    /// device.
    pub unix_offset_ms: i64,
    /// Seconds a lease holder may idle before a device attaching or coming
    /// to the front may take its window (`foregroundStealAfter`).
    pub foreground_steal_after: f64,
    /// At most one stamp refresh per this many seconds of typing.
    pub lease_stamp_refresh_interval: f64,
    /// How long the foreground waits for the lease probe before pinning.
    pub foreground_probe_timeout: f64,
    /// Seconds of layout quiet before a war that ended at another client's
    /// width is re-pinned (`foreignQuietRepinDelay`).
    pub foreign_quiet_repin_delay: f64,
    /// How recently a pane must have written to count as streaming.
    pub streaming_window: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            renders_output: true,
            size_owner_label: "iPhone".to_owned(),
            unix_offset_ms: 0,
            foreground_steal_after: 10.0,
            lease_stamp_refresh_interval: 5.0,
            foreground_probe_timeout: 1.5,
            foreign_quiet_repin_delay: 2.0,
            streaming_window: 0.5,
        }
    }
}

/// What the controller asks of its host, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// Write this command, then `\n`, on the control channel.
    Command(String),
    /// Bytes for a pane's terminal: `%output`, a backfill, a repair frame,
    /// a cursor move. In stream order.
    PaneOutput {
        pane: String,
        stream: StreamId,
        bytes: Vec<u8>,
    },
    /// What a pane's view should do.
    View {
        pane: String,
        stream: StreamId,
        action: ViewAction,
    },
    /// A pane got a terminal: the host makes (or keeps) a view for it.
    StreamOpened {
        pane: String,
        stream: StreamId,
    },
    /// A pane's terminal is gone (its window closed).
    StreamClosed {
        pane: String,
        stream: StreamId,
    },
    Notify(Notification),
}

/// What a pane's view should do (the Swift coordinator's methods the
/// controller called).
#[derive(Debug, Clone, PartialEq)]
pub enum ViewAction {
    /// Cover the terminal with its background before a stale buffer shows
    /// (`veilForSwitch`), with the cover's default safety timeout.
    Veil,
    /// Veil unless a cover is up already (`if !isCoverPresented {
    /// veilForSwitch() }`).
    VeilUnlessCovered,
    /// Push an active cover's safety timeout out to `seconds` from now; no
    /// cover, nothing (`extendCoverTimeout(by:)`).
    ExtendCover { seconds: f64 },
    /// Lift the cover: the frame on this pane is the one to look at.
    Reveal,
    /// Stop holding output for a reader scrolled up; replay what was held.
    ReleaseScrollHold,
    /// Page the local scrollback by rows (positive = older).
    ScrollLocal { lines: i64 },
    /// A drag ended: dock onto the bottom if within half a row.
    SettleLocalScroll,
    /// Apply the resize the view held for this repaint
    /// (`applyDeferredResize`), before the frame that follows.
    ApplyDeferredResize,
}

/// Things the session and the UI hear about.
#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    /// The snapshot (or the follow veil, notice, refresh spinner) changed.
    StateChanged,
    /// A pane wrote output (`onPaneActivity`).
    PaneActivity { pane: String },
    /// A pane rang the bell (`onPaneBell`).
    PaneBell { pane: String },
    /// The agent stamps were re-read (`onAgentHooksUpdated`).
    AgentHooksUpdated,
    /// The first pane of this attach was revealed (`onFirstFramePainted`).
    FirstFramePainted,
    /// The control channel is dead: tear the transport down and reconnect
    /// (`onChannelSilent`).
    ChannelDead { reason: String },
    /// Answer to [`Controller::count_non_control_clients`]: -1 for no count.
    ClientCount { request: u64, count: i64 },
    /// Answer to [`Controller::request_pane_path`].
    PanePath { request: u64, path: Option<String> },
    /// Answer to [`Controller::query_prefix_key`].
    PrefixKey { request: u64, key: String },
    /// Everything sent before [`Controller::restore_suppressed_status`]'s
    /// round trip has run, or its 2 s bound passed.
    Flushed { request: u64 },
    /// Answer to [`Controller::ping`]: `alive` once tmux answered (every
    /// byte it sent before the answer has been read by then), `false` when
    /// the channel closed first or nothing was attached to ask.
    Pong { request: u64, alive: bool },
}

/// Whether a pane is on the alternate screen and in copy-mode, from discovery
/// (`ScreenState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScreenState {
    pub alternate: bool,
    pub copy_mode: bool,
}

/// A command waiting for its reply block, and what the reply is for.
#[derive(Debug)]
struct Pending {
    reply: Reply,
    /// Someone awaits this reply: if the channel closes first it gets a
    /// "channel closed" reply instead of never hearing.
    waits: bool,
    /// A command that may answer with more than one block (B273): the text
    /// its fence prints. Every block up to the fence's is this command's;
    /// the first answers it.
    fence: Option<String>,
    /// The first of a fenced command's blocks has answered it.
    answered: bool,
    /// It may move a window's active pane (B587): until it is answered, a
    /// `%window-pane-changed` is ours.
    moves_panes: bool,
}

/// What may move a window's active pane: a `%window-pane-changed` while one
/// of ours is unanswered is our own doing, not another client's (B587).
fn moves_panes(command: &str) -> bool {
    [
        "select-pane",
        "select-window",
        "split-window",
        "kill-pane",
        "kill-window",
        "resize-pane -Z",
        "new-window",
        "swap-pane",
        "break-pane",
        "join-pane",
        "respawn-pane",
        "switch-client",
        "new-session",
        "kill-session",
    ]
    .iter()
    .any(|verb| command.starts_with(verb))
        || !answers_once(command)
}

/// The verbs tmux answers with exactly one block of the command's own when
/// a line holds just that one command: everything the controller sends, and
/// the plain queries and moves a person might. Not here (fenced instead):
/// commands that run other commands (`if-shell`, `source-file`,
/// `run-shell`, the prompts and menus), short aliases, and anything a
/// server's `command-alias` might have made.
const ANSWERS_ONCE: &[&str] = &[
    "capture-pane",
    "clear-history",
    "delete-buffer",
    "detach-client",
    "display-message",
    "has-session",
    "kill-pane",
    "kill-session",
    "kill-window",
    "last-pane",
    "last-window",
    "link-window",
    "list-buffers",
    "list-clients",
    "list-panes",
    "list-sessions",
    "list-windows",
    "move-window",
    "new-session",
    "new-window",
    "next-window",
    "paste-buffer",
    "previous-window",
    "refresh-client",
    "rename-session",
    "rename-window",
    "resize-pane",
    "resize-window",
    "select-layout",
    "select-pane",
    "select-window",
    "send-keys",
    "set-buffer",
    "set-environment",
    "set-option",
    "set-window-option",
    "show-buffer",
    "show-environment",
    "show-options",
    "show-window-options",
    "split-window",
    "swap-pane",
    "swap-window",
    "switch-client",
    "unlink-window",
];

/// Whether tmux answers `command` (one control-mode line) with exactly one
/// block of the command's own (B273). One verb from [`ANSWERS_ONCE`], and no
/// `;` or `{` outside quotes: a separator makes a sequence (`a ; b`, two
/// blocks), a brace a command block. Wrong only towards "no": a command
/// read as answering more than once costs a fence, never a misfiled reply.
pub fn answers_once(command: &str) -> bool {
    let verb = command.split_ascii_whitespace().next().unwrap_or("");
    if !ANSWERS_ONCE.contains(&verb) {
        return false;
    }
    let (mut single, mut double, mut escaped) = (false, false, false);
    for c in command.chars() {
        if escaped {
            // `\;` hands the command a literal `;`, which some split on
            // (`bind-key`, `confirm-before`): counted as a separator too.
            escaped = false;
            if !single && matches!(c, ';' | '{' | '}') {
                return false;
            }
            continue;
        }
        match c {
            '\\' if !single => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            ';' | '{' | '}' if !single && !double => return false,
            _ => {}
        }
    }
    true
}

/// What a reply block is for (the Swift callbacks).
#[derive(Debug, Clone, PartialEq)]
enum Reply {
    Ignore,
    /// The paste-buffer of paste gate `n` (B485): tmux has run it.
    Pasted(u64),
    /// A `list-sessions`: parse it (unless refused).
    Sessions {
        locating_renderer: bool,
    },
    /// The coalesced re-listing behind `%sessions-changed`.
    RelistSessions,
    Windows,
    /// A pane discovery: parse, give the session's panes terminals, then
    /// (`restore`) replay a reconnect's selection; `then` runs whatever the
    /// reply was.
    Panes {
        fresh_attach: bool,
        restore: bool,
        then: Option<Then>,
    },
    /// `zoomed active-pane mark` of a window whose zoom we hand back
    /// (B588).
    ZoomHandBack {
        window: String,
    },
    ZoomProbe {
        window: String,
        /// The zoom epoch it was sent at.
        epoch: u64,
    },
    PristineProbe {
        window: String,
    },
    /// A session's layouts just before (or after) our size request (B150).
    SizeProbe {
        session: String,
        before: bool,
    },
    /// A session's layouts around a switch to it: `earlier` is the read
    /// before it, `None` on that first read.
    SwitchProbe {
        session: String,
        earlier: Option<std::collections::BTreeMap<String, Vec<String>>>,
    },
    /// A window's layout after its record was replayed on leaving it.
    RestoreCheck {
        window: String,
        pristine: PristineLayout,
    },
    BackfillProbe {
        pane: String,
    },
    Backfill {
        pane: String,
        alternate: bool,
    },
    ResyncFrame {
        pane: String,
        reveal: bool,
        sent_at: u64,
    },
    ResyncCursor {
        pane: String,
    },
    NewWindowId,
    NewSessionId {
        name_given: bool,
        /// The name typed, trimmed (empty: none), should tmux's line
        /// lack its own.
        name: String,
    },
    SurfaceError {
        verb: &'static str,
    },
    RefreshDone,
    /// `list-clients -F '#{client_name}'` ahead of a lease read.
    LeaseClients,
    LeaseOwner {
        window: String,
        then: LeaseThen,
    },
    /// Our `#{client_name}` at attach.
    ClientName,
    /// The current window's lease at attach.
    AttachLeaseOwner,
    /// Our `#{client_name}`, ahead of a restore's client listing (B457).
    RestoreSelf,
    /// Who is on the session a restore would move (B457).
    RestoreClients {
        session: String,
    },
    /// Every window's lease, for the dead-pin sweep (B456).
    SweepWindows,
    /// Our group's make: its id (B459).
    GroupMade {
        group: PhoneGroup,
    },
    /// Every session's group mark, for the leftover sweep (B459).
    GroupSweep,
    /// The session another phone's group views (B459).
    ForeignGroup {
        id: String,
        name: String,
    },
    /// The attach's check of who is on the session (B459): our name...
    GroupCheckSelf,
    /// ...the clients...
    GroupCheckClients,
    /// ...and which control clients are Moshpit's; then the decision.
    GroupCheckMarks {
        session: String,
        name: String,
    },
    ClientCount {
        request: u64,
        switches: u64,
    },
    LocateRenderer {
        switches: u64,
    },
    /// `list-clients -F '#{client_name}'` ahead of the owned sessions.
    OwnedClients,
    /// The sessions' ownership marks and status stamps (B86).
    OwnedSessions,
    SidecarSwitch {
        session: String,
        /// The renderer's session and window before the switch: handed
        /// back once the renderer has left them.
        left: Option<String>,
        left_window: Option<String>,
    },
    Flush {
        request: u64,
    },
    /// [`Controller::ping`]'s round trip.
    Ping {
        request: u64,
    },
    MouseFlags {
        pane: String,
    },
    PrefixKey {
        request: u64,
    },
    StaleCopyMode {
        pane: String,
    },
    /// `#{pane_in_mode} #{alternate_on}` after a `%pane-mode-changed`.
    PaneMode {
        pane: String,
    },
    PanePath {
        request: u64,
    },
    AgentHooks,
}

/// What follows a pane discovery's reply.
#[derive(Debug, Clone, PartialEq)]
enum Then {
    /// `createWindow`: land on the window it printed.
    FocusNewWindow,
    /// `relistWindows`: one more listing if another was asked for.
    RelistWindows,
}

/// A coalesced re-listing (`Relist`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Relist {
    in_flight: bool,
    again: bool,
}

/// The watchdog's memory between beats.
#[derive(Debug, Clone, Default)]
struct Watchdog {
    strikes: u32,
    pop_strikes: u32,
    last_seen: Option<u64>,
    last_pops: u64,
}

/// How often a pane's output is noted as activity at most: the agents'
/// machine reads it in seconds (two to show an agent working, four of
/// quiet to call it done).
pub const ACTIVITY_NOTE_MS: u64 = 250;

/// The controller.
#[derive(Debug)]
pub struct Controller {
    config: Config,
    parser: ControlParser,
    snapshot: Snapshot,
    /// Something observable changed during the current input.
    dirty: bool,
    outputs: VecDeque<Output>,
    pending: VecDeque<Pending>,
    /// Commands of ours that may move an active pane, unanswered (B587).
    own_pane_moves: u32,
    /// The pane the phone keeps while another client picked another
    /// (B587).
    pane_follow: Option<crate::lease::PaneFollow>,
    timers: Timers,
    /// Input held behind the paste just sent (its number), until tmux has
    /// run it and it has had its time (B485).
    paste_gate: Option<(u64, Vec<input::Held>)>,
    /// The last paste's number.
    pastes: u64,
    /// The clock of the input (or timer) being handled.
    now: u64,

    streams: BTreeMap<String, StreamId>,
    next_stream: StreamId,

    last_client_size: (i64, i64),
    client_size_confirmed: bool,
    transport_live: bool,

    pending_restore: Option<Selection>,
    /// The restore is the person's own pick (a notification tapped while
    /// redialling, B405): it moves what it names, as a tap does. Otherwise
    /// it is where the app last was, and moves nothing a desktop is on
    /// (B457).
    restore_chosen: bool,
    /// Whether the restore was checked for other clients on its session.
    restore_checked: bool,
    restore_check_in_flight: bool,
    /// Our `#{client_name}`, read for that check.
    restore_self: Option<String>,
    /// How many clients each session had at the last session listing
    /// (`#{session_attached}`).
    session_clients: BTreeMap<String, i64>,
    pending_immersive_zoom: bool,
    pane_screen_state: BTreeMap<String, ScreenState>,
    pending_window_focus: Option<String>,
    sessions_relist: Relist,
    windows_relist: Relist,
    /// The last `%session-window-changed` for our session.
    server_current_window: Option<(String, String)>,
    /// How many sessions each window was linked into at the last listing
    /// (`list-windows -a` names a linked window once per session).
    window_links: BTreeMap<String, usize>,

    backfilled_panes: BTreeSet<String>,
    backfills_in_flight: BTreeSet<String>,
    deferred_resyncs: BTreeMap<String, bool>,
    backfill_reject_streak: BTreeMap<String, u32>,
    resyncs_in_flight: BTreeSet<String>,
    queued_resyncs: BTreeMap<String, bool>,
    resync_reject_streak: BTreeMap<String, u32>,
    /// Seconds, send → frame fed, of the last resync capture.
    last_resync_round_trip: f64,
    convergence_pending: bool,
    convergence_saw_output: bool,
    pane_resize_awaiting_capture: BTreeSet<String>,

    last_pane_output_at: BTreeMap<String, Wall>,
    /// When each pane's output was last noted as activity (B739).
    activity_noted: BTreeMap<String, u64>,
    bell_detectors: BTreeMap<String, BellDetector>,

    has_painted_since_attach: bool,
    attached_at: Option<u64>,

    agent_hooks: BTreeMap<String, AgentHook>,
    agent_hooks_loaded: bool,

    notice: Option<String>,
    is_refreshing: bool,
    next_request: u64,
    /// Requests whose answer (reply or bound) has not been given yet.
    waiting_requests: BTreeSet<u64>,
    refresh_holds: u64,

    /// What tmux said about each pane's program (B363).
    pane_flags: BTreeMap<String, input::PaneFlags>,
    wheel_carry: input::RowCarry,
    /// The cell (0-based, the input pane's) the scroll under way is aimed
    /// at: where its drag began (B578).
    wheel_aim: Option<(i64, i64)>,
    /// Each pane terminal's grid: born at the client size, then what its
    /// view reported.
    pane_grids: BTreeMap<String, (i64, i64)>,
    /// A grid the view holds back until its repaint (`shouldDeferResize`).
    held_grids: BTreeMap<String, (i64, i64)>,

    last_incoming_at: Option<u64>,
    pop_counter: u64,
    /// The clock [`Controller::advance`] is catching up to: the input's
    /// own time, while the timers due before it fire at theirs.
    advancing_to: u64,
    /// The last fence's number (B273).
    fences: u64,
    /// This server flagged a reply to one of our lines (B273): from here
    /// on a block flagged 0 answers nothing of ours. A server that never
    /// does is read the old way, every block a reply.
    flags_seen: bool,
    watchdog: Option<Watchdog>,

    size: size::SizeState,
    sidecar: sidecar::SidecarState,
    group: GroupState,
}

impl Controller {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            parser: ControlParser::new(),
            snapshot: Snapshot::default(),
            dirty: false,
            outputs: VecDeque::new(),
            pending: VecDeque::new(),
            own_pane_moves: 0,
            pane_follow: None,
            timers: Timers::default(),
            paste_gate: None,
            pastes: 0,
            now: 0,
            streams: BTreeMap::new(),
            next_stream: 1,
            last_client_size: (80, 24),
            client_size_confirmed: false,
            transport_live: true,
            pending_restore: None,
            restore_chosen: false,
            restore_checked: false,
            restore_check_in_flight: false,
            restore_self: None,
            session_clients: BTreeMap::new(),
            pending_immersive_zoom: false,
            pane_screen_state: BTreeMap::new(),
            pending_window_focus: None,
            sessions_relist: Relist::default(),
            windows_relist: Relist::default(),
            server_current_window: None,
            window_links: BTreeMap::new(),
            backfilled_panes: BTreeSet::new(),
            backfills_in_flight: BTreeSet::new(),
            deferred_resyncs: BTreeMap::new(),
            backfill_reject_streak: BTreeMap::new(),
            resyncs_in_flight: BTreeSet::new(),
            queued_resyncs: BTreeMap::new(),
            resync_reject_streak: BTreeMap::new(),
            last_resync_round_trip: 0.0,
            convergence_pending: false,
            convergence_saw_output: false,
            pane_resize_awaiting_capture: BTreeSet::new(),
            last_pane_output_at: BTreeMap::new(),
            activity_noted: BTreeMap::new(),
            bell_detectors: BTreeMap::new(),
            has_painted_since_attach: false,
            attached_at: None,
            agent_hooks: BTreeMap::new(),
            agent_hooks_loaded: false,
            notice: None,
            is_refreshing: false,
            next_request: 0,
            waiting_requests: BTreeSet::new(),
            refresh_holds: 0,
            pane_flags: BTreeMap::new(),
            wheel_carry: input::RowCarry::default(),
            wheel_aim: None,
            pane_grids: BTreeMap::new(),
            held_grids: BTreeMap::new(),
            last_incoming_at: None,
            pop_counter: 0,
            advancing_to: 0,
            fences: 0,
            flags_seen: false,
            watchdog: None,
            size: size::SizeState::default(),
            sidecar: sidecar::SidecarState::default(),
            group: GroupState::default(),
        }
    }

    // MARK: - Reading the state

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The knobs, for a host (or a test) that changes them while running.
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// The stamps of the last agent-hook poll, by pane id.
    pub fn agent_hooks(&self) -> &BTreeMap<String, AgentHook> {
        &self.agent_hooks
    }

    /// Whether [`agent_hooks`](Self::agent_hooks) has been read (or
    /// inherited): before that, empty means "not known yet".
    pub fn agent_hooks_loaded(&self) -> bool {
        self.agent_hooks_loaded
    }

    /// A tmux command's complaint, shown for 3 s.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// A user-started refresh is on its way (the spinner).
    pub fn is_refreshing(&self) -> bool {
        self.is_refreshing
    }

    /// Whether a pane of this attach has been revealed.
    pub fn has_painted_since_attach(&self) -> bool {
        self.has_painted_since_attach
    }

    /// When the attach landed, on the input clock.
    pub fn attached_at(&self) -> Option<u64> {
        self.attached_at
    }

    /// When the control channel last delivered bytes.
    pub fn last_inbound_at(&self) -> Option<u64> {
        self.last_incoming_at
    }

    /// The pane's stream, if it has a terminal.
    pub fn stream_of(&self, pane: &str) -> Option<StreamId> {
        self.streams.get(pane).copied()
    }

    /// Every pane terminal, by pane id.
    pub fn streams(&self) -> &BTreeMap<String, StreamId> {
        &self.streams
    }

    /// The pane's stream, minting a terminal for it if it has none yet
    /// (`terminalView(for:)`): the same stream for a pane for as long as it
    /// lives.
    pub fn stream_for_pane(&mut self, pane: &str) -> StreamId {
        self.mint_terminal(pane)
    }

    /// The grid tmux is told this client has.
    pub fn client_size(&self) -> (i64, i64) {
        self.last_client_size
    }

    // MARK: - Outputs and time

    /// The next output, in order.
    pub fn poll_output(&mut self) -> Option<Output> {
        self.outputs.pop_front()
    }

    /// Every output queued so far, in order.
    pub fn drain_outputs(&mut self) -> Vec<Output> {
        self.outputs.drain(..).collect()
    }

    /// When [`tick`](Self::tick) has something to do.
    pub fn next_deadline(&self) -> Option<u64> {
        self.timers.next_deadline()
    }

    /// Fire every timer due by `now`, in order.
    pub fn tick(&mut self, now: u64) {
        self.advance(now);
        self.finish_input();
    }

    /// Fire the timers due by `now`, then stand at `now`.
    fn advance(&mut self, now: u64) {
        self.advancing_to = now.max(self.advancing_to);
        while let Some((deadline, action)) = self.timers.pop_due(now) {
            self.now = deadline.max(self.now);
            self.fire(action);
        }
        self.now = now.max(self.now);
    }

    /// End of an input: tell the UI once if anything it draws changed.
    fn finish_input(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.outputs
                .push_back(Output::Notify(Notification::StateChanged));
        }
    }

    fn begin_input(&mut self, now: u64) {
        self.advance(now);
    }

    /// The snapshot, to change it.
    fn snap(&mut self) -> &mut Snapshot {
        self.dirty = true;
        &mut self.snapshot
    }

    fn notify(&mut self, notification: Notification) {
        self.outputs.push_back(Output::Notify(notification));
    }

    fn view(&mut self, pane: &str, action: ViewAction) {
        if let Some(&stream) = self.streams.get(pane) {
            self.outputs.push_back(Output::View {
                pane: pane.to_owned(),
                stream,
                action,
            });
        }
    }

    fn feed(&mut self, pane: &str, bytes: Vec<u8>) {
        if let Some(&stream) = self.streams.get(pane) {
            self.outputs.push_back(Output::PaneOutput {
                pane: pane.to_owned(),
                stream,
                bytes,
            });
        }
    }

    /// The wall clock now (see [`Wall`]).
    fn wall(&self) -> Wall {
        Wall::at(self.now as i64 + self.config.unix_offset_ms)
    }

    /// Unix time in seconds: what a lease stamp is written in and compared
    /// against.
    fn unix_seconds(&self) -> f64 {
        self.wall().unix()
    }

    fn arm(&mut self, slot: Slot, after_ms: u64, action: Action) {
        self.timers.arm(slot, self.now + after_ms, action);
    }

    /// One round trip on the control channel, unbounded (the caller keeps
    /// the clock): answered with [`Notification::Pong`]. tmux answers in
    /// order, so by the answer every byte it queued before it — the output
    /// of a suspension — has been read: the screen has caught up.
    pub fn ping(&mut self, now: u64) -> u64 {
        self.begin_input(now);
        let request = self.request_id();
        self.enqueue(
            "display-message -p ok".to_owned(),
            Reply::Ping { request },
            true,
        );
        self.finish_input();
        request
    }

    fn request_id(&mut self) -> u64 {
        self.next_request += 1;
        self.next_request
    }

    fn fire(&mut self, action: Action) {
        match action {
            Action::CommitClientSize => self.commit_client_size(),
            Action::SettledResync => {
                let grace = self.cover_grace();
                self.extend_active_cover_timeout(grace);
                self.resync_active_pane(true);
            }
            Action::AttachSettledResync => {
                self.extend_active_cover_timeout(2.0);
                self.resync_active_pane(true);
            }
            Action::ForegroundResync => self.resync_active_pane(true),
            Action::ReturnResync(pane) => self.resync_pane(&pane, true),
            Action::ForegroundProbeTimeout(window) => self.foreground_probe_timed_out(&window),
            Action::ForeignQuietRepin(window) => self.foreign_quiet_repin(&window),
            Action::BackfillTimeout(pane) => self.finish_backfill(&pane),
            Action::BackfillRetry(pane) => self.backfill(&pane),
            Action::ResyncTimeout(pane) => self.finish_resync(&pane),
            Action::ResyncRetry { pane, reveal } => self.resync_pane(&pane, reveal),
            Action::Convergence => self.fire_convergence_resync(),
            Action::DeferredResize(pane) => self.release_deferred_resize(&pane),
            Action::ClearNotice => {
                self.notice = None;
                self.dirty = true;
            }
            Action::RefreshDone => {
                self.is_refreshing = false;
                self.dirty = true;
            }
            Action::Watchdog => self.watchdog_beat(),
            Action::PanePathTimeout(request) => self.pane_path_timed_out(request),
            Action::FlushTimeout(request) => self.flush_done(request),
            Action::PasteSettled => self.paste_settled(),
        }
    }

    // MARK: - Lifecycle

    /// Seed the client size before control mode starts
    /// (`setInitialClientSize`): a guess at the view's grid, so tmux's first
    /// `refresh-client -C` is phone-sized. Not a confirmation — the view's
    /// first real report still commits.
    pub fn set_initial_client_size(&mut self, cols: i64, rows: i64) {
        self.last_client_size = (cols, rows);
        self.client_size_confirmed = false;
    }

    /// A selection captured before a reconnect, replayed once discovery has
    /// the ids again (`pendingRestore`).
    pub fn set_pending_restore(&mut self, selection: Option<Selection>) {
        self.pending_restore = selection;
        self.restore_checked = false;
    }

    /// The restore is the person's own pick (B405), not where the app last
    /// was: it selects what it names even with a desktop on that session.
    pub fn set_restore_chosen(&mut self, chosen: bool) {
        self.restore_chosen = chosen;
    }

    pub fn pending_restore(&self) -> Option<&Selection> {
        self.pending_restore.as_ref()
    }

    /// Whether the transport under this controller is live: keystrokes from
    /// the pane views are dropped while it isn't (`transportIsLive`).
    pub fn set_transport_live(&mut self, live: bool) {
        self.transport_live = live;
    }

    /// Start reading the control channel without sending discovery
    /// (`beginControlMode`): tmux is not running yet; the first
    /// `%session-changed` starts the attach.
    pub fn begin_control_mode(&mut self, now: u64) {
        self.begin_input(now);
        self.start_watchdog();
        self.finish_input();
    }

    /// Start reading and run the initial discovery (`attach()`): control
    /// mode is already running.
    pub fn attach(&mut self, now: u64) {
        self.begin_input(now);
        self.start_watchdog();
        self.send_initial_discovery();
        self.finish_input();
    }

    /// Stop reading, reset the parser and forget what the dead connection
    /// knew (`detach()`). Whoever waits on a reply hears that none is coming.
    pub fn detach(&mut self, now: u64) {
        self.begin_input(now);
        self.watchdog = None;
        self.timers.cancel(&Slot::Watchdog);
        self.parser.reset();
        self.snap().is_attached = false;
        self.close_pending_commands();
        // The next connection makes a group of its own (this one tmux
        // destroys with the client).
        self.group = GroupState::default();
        self.backfills_in_flight.clear();
        self.deferred_resyncs.clear();
        let panes: Vec<String> = self.pane_resize_awaiting_capture.iter().cloned().collect();
        for pane in &panes {
            self.timers.cancel(&Slot::DeferredResize(pane.clone()));
        }
        let mut panes = panes;
        panes.sort_by(|a, b| id_cmp(a, b));
        for pane in &panes {
            self.view(pane, ViewAction::ApplyDeferredResize);
            if let Some(grid) = self.held_grids.remove(pane) {
                self.pane_grids.insert(pane.clone(), grid);
            }
        }
        self.pane_resize_awaiting_capture.clear();
        let backfill_panes: Vec<String> = self.timers_for_panes();
        for pane in backfill_panes {
            self.timers.cancel(&Slot::BackfillTimeout(pane.clone()));
            self.timers.cancel(&Slot::BackfillRetry(pane));
        }
        self.backfill_reject_streak.clear();
        self.detach_size_state();
        self.timers.cancel(&Slot::ResyncRetry);
        self.resync_reject_streak.clear();
        self.convergence_pending = false;
        self.convergence_saw_output = false;
        self.timers.cancel(&Slot::ConvergenceDebounce);
        self.timers.cancel(&Slot::ConvergenceDeadline);
        self.finish_input();
    }

    /// Every pane with a backfill timer or retry armed.
    fn timers_for_panes(&self) -> Vec<String> {
        let mut panes: BTreeSet<String> = self.backfilled_panes.clone();
        panes.extend(self.backfills_in_flight.iter().cloned());
        panes.extend(self.backfill_reject_streak.keys().cloned());
        panes
            .into_iter()
            .filter(|pane| {
                self.timers.is_armed(&Slot::BackfillTimeout(pane.clone()))
                    || self.timers.is_armed(&Slot::BackfillRetry(pane.clone()))
            })
            .collect()
    }

    /// The control channel's byte stream ended: a transport death.
    pub fn channel_closed(&mut self, now: u64) {
        self.begin_input(now);
        self.close_pending_commands();
        self.report_channel_dead("stream-ended");
        self.finish_input();
    }

    /// Bytes off the control channel.
    pub fn receive(&mut self, bytes: &[u8], now: u64) {
        self.begin_input(now);
        self.last_incoming_at = Some(self.now);
        let mut events = Vec::new();
        self.parser.feed(bytes, |event| events.push(event));
        for event in events {
            self.dispatch(event);
        }
        self.finish_input();
    }

    /// One parser event, as if it had just been read.
    pub fn handle_event(&mut self, event: Event, now: u64) {
        self.begin_input(now);
        self.dispatch(event);
        self.finish_input();
    }

    /// Every live notification passes here: each id is checked against
    /// tmux's real shape before it is stored or spliced into a command.
    fn dispatch(&mut self, event: Event) {
        match event {
            Event::PaneOutput { pane_id, data } => {
                if is_valid_id(&pane_id) {
                    self.handle_pane_output(&pane_id, data);
                }
            }
            Event::LayoutChange {
                window_id,
                layout,
                zoomed,
            } => {
                if is_valid_id(&window_id) {
                    self.handle_layout_change(&window_id, &layout, zoomed);
                }
            }
            Event::WindowAdd { window_id } => {
                if is_valid_id(&window_id) {
                    self.handle_window_add(&window_id);
                }
            }
            Event::WindowClose { window_id } | Event::UnlinkedWindowClose { window_id } => {
                if is_valid_id(&window_id) {
                    self.handle_window_close(&window_id);
                }
            }
            Event::WindowRenamed { window_id, name }
            | Event::UnlinkedWindowRenamed { window_id, name } => {
                if is_valid_id(&window_id) {
                    self.handle_window_renamed(&window_id, &name);
                }
            }
            Event::SessionChanged { session_id, name } => {
                if is_valid_id(&session_id) {
                    self.handle_session_changed(&session_id, &name);
                }
            }
            Event::SessionRenamed { session_id, name } => {
                if is_valid_id(&session_id) {
                    self.handle_session_renamed(&session_id, &name);
                }
            }
            Event::SessionWindowChanged {
                session_id,
                window_id,
            } => {
                if is_valid_id(&session_id) && is_valid_id(&window_id) {
                    self.handle_session_window_changed(&session_id, &window_id);
                }
            }
            Event::SessionsChanged => self.relist_sessions(),
            Event::UnlinkedWindowAdd { window_id } => {
                if is_valid_id(&window_id) {
                    self.relist_windows();
                }
            }
            Event::ClientSessionChanged {
                client, session_id, ..
            } => {
                if is_valid_id(&session_id) {
                    self.relist_sessions();
                    self.client_session_changed(&client);
                }
            }
            Event::ActivePaneChanged { window_id, pane_id } => {
                if is_valid_id(&window_id) && is_valid_id(&pane_id) {
                    self.handle_active_pane_changed(&window_id, &pane_id);
                }
            }
            Event::Pause { pane_id } => {
                if is_valid_id(&pane_id) {
                    // `on/off/continue/pause` is the vocabulary: the `:+`
                    // this once sent earned an %error and the pane stayed
                    // paused for good.
                    self.send(format!("refresh-client -A {pane_id}:continue"));
                }
            }
            Event::Continue { pane_id } => {
                if is_valid_id(&pane_id) {
                    // tmux dropped this pane's output while it was paused.
                    self.resync_pane(&pane_id, true);
                }
            }
            Event::PaneModeChanged { pane_id } => {
                if is_valid_id(&pane_id) {
                    self.pane_mode_changed(&pane_id);
                }
            }
            Event::ClientDetached { payload } => self.client_detached(&payload),
            Event::Exit { .. } => {
                self.snap().is_attached = false;
                self.close_pending_commands();
            }
            Event::CommandResponse(response) => self.handle_command_response(response),
            Event::ProtocolError(_) => {}
        }
    }

    // MARK: - Commands and replies

    /// A command whose reply is ignored (it still takes a reply slot).
    fn send(&mut self, command: String) {
        self.enqueue(command, Reply::Ignore, false);
    }

    fn send_command(&mut self, command: String, reply: Reply) {
        self.enqueue(command, reply, false);
    }

    /// The one write chokepoint. Before tmux is attached anything written
    /// lands in the SHELL, and its reply slot would never be answered: drop
    /// it, and tell whoever waits that no reply is coming.
    fn enqueue(&mut self, command: String, reply: Reply, waits: bool) {
        if !self.snapshot.is_attached {
            if waits {
                self.answer(reply, &channel_closed());
            }
            return;
        }
        // B139 (the Swift controller the same): one line is one command in
        // control mode; a line break would end this one and run the rest as
        // a command of its own, and every reply after it would go to the
        // wrong slot. Nothing built here holds one; what does is not sent.
        if command.contains(['\n', '\r']) {
            if waits {
                self.answer(reply, &channel_closed());
            }
            return;
        }
        // B273: a command that may answer with more than one block gets a
        // fence right behind it, in the same order on the same channel.
        // tmux runs what a command adds (an `if-shell`'s command, a
        // `source-file`'s) before the next line's, so the fence's block
        // comes after every block of the command's own.
        let fence = (!answers_once(&command)).then(|| {
            self.fences = self.fences.wrapping_add(1);
            format!("moshpit-fence-{}", self.fences)
        });
        let moves_panes = moves_panes(&command);
        if moves_panes {
            self.own_pane_moves = self.own_pane_moves.saturating_add(1);
        }
        self.pending.push_back(Pending {
            reply,
            waits,
            fence: fence.clone(),
            answered: false,
            moves_panes,
        });
        self.outputs.push_back(Output::Command(command));
        if let Some(fence) = fence {
            self.outputs
                .push_back(Output::Command(format!("display-message -p '{fence}'")));
        }
    }

    /// tmux answers in send order: the reply belongs to the oldest command.
    /// A block no command of ours asked for (B273: flags 0 — a hook, an
    /// `-b` command's late half) belongs to none. A fenced command takes
    /// every block up to its fence's (the first answers it).
    fn handle_command_response(&mut self, response: CommandResponse) {
        if response.from_client {
            self.flags_seen = true;
        } else if self.flags_seen {
            return;
        }
        let Some(entry) = self.pending.front_mut() else {
            return;
        };
        self.pop_counter = self.pop_counter.wrapping_add(1);
        if let Some(fence) = &entry.fence {
            let is_fence = !response.is_error
                && response.lines.len() == 1
                && response.lines[0] == *fence;
            if !is_fence {
                if !entry.answered {
                    entry.answered = true;
                    let reply = std::mem::replace(&mut entry.reply, Reply::Ignore);
                    self.answer(reply, &response);
                }
                return;
            }
            let Some(entry) = self.pending.pop_front() else {
                return;
            };
            if entry.moves_panes {
                self.own_pane_moves = self.own_pane_moves.saturating_sub(1);
            }
            if !entry.answered && entry.waits {
                // No block of its own came (it can't: a parse error is one).
                self.answer(entry.reply, &channel_closed());
            }
            return;
        }
        let Some(entry) = self.pending.pop_front() else {
            return;
        };
        if entry.moves_panes {
            self.own_pane_moves = self.own_pane_moves.saturating_sub(1);
        }
        self.answer(entry.reply, &response);
    }

    /// The control channel is gone and nothing queued will be answered
    /// (`closePendingCommands`).
    fn close_pending_commands(&mut self) {
        self.sessions_relist = Relist::default();
        self.windows_relist = Relist::default();
        self.own_pane_moves = 0;
        if self.pending.is_empty() {
            return;
        }
        let waiting: Vec<Reply> = self
            .pending
            .drain(..)
            .filter(|entry| entry.waits)
            .map(|entry| entry.reply)
            .collect();
        self.closed_in_flight();
        for reply in waiting {
            self.answer(reply, &channel_closed());
        }
    }

    fn answer(&mut self, reply: Reply, response: &CommandResponse) {
        match reply {
            Reply::Ignore => {}
            Reply::Pasted(paste) => self.pasted(paste),
            Reply::Sessions { locating_renderer } => {
                if let Some(rows) = listing(response) {
                    self.parse_list_sessions_with(rows, locating_renderer);
                }
            }
            Reply::RelistSessions => {
                self.sessions_relist.in_flight = false;
                if let Some(rows) = listing(response) {
                    self.parse_list_sessions_with(rows, true);
                }
                if self.sessions_relist.again {
                    self.sessions_relist.again = false;
                    self.relist_sessions();
                }
            }
            Reply::Windows => {
                if let Some(rows) = listing(response) {
                    self.parse_list_windows(rows);
                }
            }
            Reply::Panes {
                fresh_attach,
                restore,
                then,
            } => {
                if let Some(rows) = listing(response) {
                    self.parse_list_panes(rows);
                    self.close_streams_of_dead_panes();
                    self.ensure_terminals_for_all_panes(fresh_attach);
                    if restore {
                        self.apply_pending_restore_if_ready();
                    }
                }
                match then {
                    Some(Then::FocusNewWindow) => self.focus_new_window(),
                    Some(Then::RelistWindows) => {
                        self.windows_relist.in_flight = false;
                        if self.windows_relist.again {
                            self.windows_relist.again = false;
                            self.relist_windows();
                        }
                    }
                    None => {}
                }
            }
            Reply::ZoomHandBack { window } => self.zoom_hand_back_answered(&window, response),
            Reply::ZoomProbe { window, epoch } => {
                self.zoom_probe_answered(&window, epoch, response)
            }
            Reply::BackfillProbe { pane } => self.backfill_probe_answered(&pane, response),
            Reply::Backfill { pane, alternate } => {
                self.backfill_answered(&pane, alternate, response)
            }
            Reply::ResyncFrame {
                pane,
                reveal,
                sent_at,
            } => self.resync_frame_answered(&pane, reveal, sent_at, response),
            Reply::ResyncCursor { pane } => self.resync_cursor_answered(&pane, response),
            Reply::NewWindowId => {
                if response.is_error {
                    let message = response
                        .lines
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "new-window failed".to_owned());
                    self.flash_notice(message);
                    return;
                }
                let Some(id) = response.lines.first().map(|line| trim_whitespaces(line)) else {
                    return;
                };
                if is_valid_id(id) {
                    self.pending_window_focus = Some(id.to_owned());
                }
            }
            Reply::NewSessionId { name_given, name } => {
                self.new_session_answered(name_given, &name, response)
            }
            Reply::SurfaceError { verb } => {
                if response.is_error {
                    let detail = response.lines.first().cloned().unwrap_or_default();
                    let message = if detail.is_empty() {
                        format!("{verb} failed")
                    } else {
                        detail
                    };
                    self.flash_notice(message);
                }
            }
            Reply::RefreshDone => {
                // Held a beat so a fast refresh still reads as one.
                self.refresh_holds += 1;
                let slot = Slot::RefreshDone(self.refresh_holds);
                self.arm(slot, 350, Action::RefreshDone);
            }
            Reply::LeaseClients => self.lease_clients_answered(response),
            Reply::LeaseOwner { window, then } => {
                self.lease_owner_answered(&window, then, response)
            }
            Reply::ClientName => self.client_name_answered(response),
            Reply::AttachLeaseOwner => self.attach_lease_answered(response),
            Reply::RestoreSelf => self.restore_self_answered(response),
            Reply::RestoreClients { session } => self.restore_clients_answered(&session, response),
            Reply::SweepWindows => self.sweep_windows_answered(response),
            Reply::GroupMade { group } => self.group_made(group, response),
            Reply::GroupSweep => self.groups_swept(response),
            Reply::ForeignGroup { id, name } => self.foreign_group_answered(&id, &name, response),
            Reply::GroupCheckSelf => self.group_check_self_answered(response),
            Reply::GroupCheckClients => self.group_check_clients_answered(response),
            Reply::GroupCheckMarks { session, name } => {
                self.group_check_marks_answered(&session, &name, response)
            }
            Reply::PristineProbe { window } => self.pristine_probe_answered(&window, response),
            Reply::SizeProbe { before, .. } => self.size_probe_answered(before, response),
            Reply::SwitchProbe { session, earlier } => {
                self.switch_probe_answered(&session, earlier, response)
            }
            Reply::RestoreCheck { window, pristine } => {
                self.restore_check_answered(&window, pristine, response)
            }
            Reply::ClientCount { request, switches } => {
                self.client_count_answered(request, switches, response)
            }
            Reply::LocateRenderer { switches } => self.locate_renderer_answered(switches, response),
            Reply::OwnedClients => self.owned_clients_answered(response),
            Reply::OwnedSessions => self.owned_sessions_answered(response),
            Reply::SidecarSwitch {
                session,
                left,
                left_window,
            } => self.sidecar_switch_answered(&session, left, left_window, response),
            Reply::Flush { request } => self.flush_done(request),
            // A refusal is an answer too; only the synthetic reply of a
            // channel that closed first (`channel_closed`) is not.
            Reply::Ping { request } => self.notify(Notification::Pong {
                request,
                alive: response.command_id >= 0,
            }),
            Reply::MouseFlags { pane } => self.mouse_flags_answered(&pane, response),
            Reply::PrefixKey { request } => self.prefix_key_answered(request, response),
            Reply::StaleCopyMode { pane } => self.stale_copy_mode_answered(&pane, response),
            Reply::PaneMode { pane } => self.pane_mode_answered(&pane, response),
            Reply::PanePath { request } => self.pane_path_answered(request, response),
            Reply::AgentHooks => self.agent_hooks_answered(response),
        }
    }

    // MARK: - Attach and discovery

    fn send_initial_discovery(&mut self) {
        self.snap().is_attached = true;
        self.snap().ever_attached = true;
        if self.attached_at.is_none() {
            self.attached_at = Some(self.now);
        }
        self.send_command(
            LIST_SESSIONS.to_owned(),
            Reply::Sessions {
                locating_renderer: true,
            },
        );
        self.send_command(LIST_WINDOWS.to_owned(), Reply::Windows);
        self.send_command(
            format!("list-panes -a -F {PANE_DISCOVERY_FORMAT}"),
            Reply::Panes {
                fresh_attach: true,
                restore: true,
                then: None,
            },
        );
        if !self.config.renders_output {
            // B86: which sessions Moshpit made, from the server — after the
            // discovery: an attach's first three commands are the
            // discovery's, whoever answers them.
            self.read_owned_sessions();
        }
    }

    /// The first `%session-changed` after the channel comes up IS the
    /// (re)attach; later ones are session switches on a live connection.
    ///
    /// B459: this client views a session through a group of its own. Our
    /// group's change stands for its session; a real session's makes the
    /// group first; another phone's group (an attach without a target lands
    /// on the latest session, which may be one) is left for a group of our
    /// own on the session it views.
    fn handle_session_changed(&mut self, session_id: &str, name: &str) {
        if let Some(session) = self.group_target(session_id, name) {
            self.group.entering = false;
            if let Some(group) = self.group.current.as_mut()
                && group.id.is_none()
            {
                group.id = Some(session_id.to_owned());
            }
            if self.snapshot.is_attached
                && self.group.client_on.as_deref() == Some(session.as_str())
            {
                // The make's own echo: S's change (or the switch) did the work.
                return;
            }
            let name = self
                .snapshot
                .sessions
                .get(&session)
                .map(|info| info.name.clone())
                .unwrap_or_default();
            self.attach_to_session(&session, &name, false);
            return;
        }
        if !is_group_name(name) && self.in_group() {
            // tmux moved this client off our group: it is gone (killed under
            // us). A new one for where we are now.
            self.group.current = None;
            self.group.entering = false;
            self.group.landing = false;
        }
        if self.config.renders_output && !self.group.refused && is_group_name(name) {
            // Control mode is up (tmux said where we are): the lookup goes
            // out, and the attach still counts as fresh when it lands.
            self.group.fresh_pending |= !self.snapshot.is_attached;
            self.snap().is_attached = true;
            self.send_command(
                format!("display-message -p -t '{session_id}' '#{{{GROUP_OF_OPTION}}}'"),
                Reply::ForeignGroup {
                    id: session_id.to_owned(),
                    name: name.to_owned(),
                },
            );
            return;
        }
        let group = self.wants_group_for(session_id);
        self.attach_to_session(session_id, name, group);
    }

    /// Landed on another phone's group: a group of our own on the session it
    /// views; one that names none is taken as a session.
    fn foreign_group_answered(&mut self, id: &str, name: &str, response: &CommandResponse) {
        let session = response
            .lines
            .first()
            .map(|line| trim_whitespaces(line).to_owned())
            .filter(|s| !response.is_error && s.starts_with('$') && is_valid_id(s));
        match session {
            Some(session) => {
                let base = name.split(GROUP_MARK).next().unwrap_or_default().to_owned();
                self.enter_group(&session, &base);
            }
            None => self.attach_to_session(id, name, false),
        }
    }

    /// What a `%session-changed` to `session_id` does: the attach, or a
    /// switch on a live connection; `group`: our group of it made first.
    fn attach_to_session(&mut self, session_id: &str, name: &str, group: bool) {
        self.group.client_on = Some(session_id.to_owned());
        let fresh_attach =
            !self.snapshot.is_attached || std::mem::take(&mut self.group.fresh_pending);
        let snapshot = self.snap();
        snapshot.active_session_id = Some(session_id.to_owned());
        match snapshot.sessions.get_mut(session_id) {
            Some(session) => {
                if !name.is_empty() {
                    session.name = name.to_owned();
                }
                session.is_attached = true;
            }
            None => {
                snapshot.sessions.insert(
                    session_id.to_owned(),
                    SessionInfo {
                        id: session_id.to_owned(),
                        name: name.to_owned(),
                        is_attached: true,
                    },
                );
            }
        }
        snapshot.is_attached = true;
        snapshot.ever_attached = true;
        if self.attached_at.is_none() {
            self.attached_at = Some(self.now);
        }
        if group && fresh_attach {
            // B459: who is on the session first (a desktop's control client
            // keeps the phone off a group); the group, if any, follows.
            self.check_for_foreign_control(session_id, name);
        } else if group {
            // Before anything else, so all that follows runs there.
            self.enter_group(session_id, name);
        }
        if !self.config.renders_output && fresh_attach {
            // A mosh sidecar is a Moshpit control client too.
            self.register_as_moshpit();
        }
        if self.config.renders_output {
            if fresh_attach {
                // B150: what our size is about to squash, read first.
                self.probe_layouts_before_sizing(session_id);
            }
            if fresh_attach && self.size.pins_released {
                // Attaching for a terminal nobody looks at: out of the
                // window's sizing before our size is ever counted.
                self.send("refresh-client -f ignore-size".to_owned());
            }
            // Without a size tmux suppresses %output to a control client.
            let (cols, rows) = self.last_client_size;
            self.send(format!("refresh-client -C {cols}x{rows}"));
            self.probe_layouts_after_sizing();
            self.read_attach_lease();
            if fresh_attach {
                // B456: what a dead phone left pinned, handed back.
                self.sweep_dead_pins();
                // B459: groups a dead phone left behind.
                self.sweep_groups();
            }
        } else {
            // B270: what the renderer's attach squashed, read again now.
            self.probe_layouts_after_renderer();
            if fresh_attach {
                // B86: which sessions Moshpit made, from the server.
                self.read_owned_sessions();
            }
            self.locate_renderer();
        }
        self.pending_immersive_zoom = true;
        self.send_command(
            LIST_SESSIONS.to_owned(),
            Reply::Sessions {
                locating_renderer: false,
            },
        );
        self.refresh_windows_and_panes(fresh_attach, None);
        if fresh_attach {
            self.poll_agent_hooks_now();
        }
    }

    fn handle_session_renamed(&mut self, session_id: &str, name: &str) {
        if self.snapshot.sessions.contains_key(session_id)
            && let Some(session) = self.snap().sessions.get_mut(session_id)
        {
            session.name = name.to_owned();
        }
    }

    fn handle_session_window_changed(&mut self, session_id: &str, window_id: &str) {
        // Broadcast for EVERY session: only the one this client is on is
        // followed — our group, never S's own (a desktop's, B459).
        if self.client_session_id().as_deref() != Some(session_id) {
            return;
        }
        let session = self.snapshot.active_session_id.clone().unwrap_or_default();
        self.server_current_window = Some((session, window_id.to_owned()));
        // The viewed window is local state: tmux's current window is shared
        // by every client of the session, so only adopt it when ours is gone
        // or this is the echo of our own select-window.
        let current = self.snapshot.active_window_id.clone();
        let current_is_ours = current
            .as_deref()
            .and_then(|window| self.snapshot.windows.get(window))
            .map(|window| window.session_id.as_str())
            == self.snapshot.active_session_id.as_deref();
        // Fix (sidecar-current-window, B80): over mosh the screen IS tmux's
        // current window — the renderer is a regular client of the session,
        // and a session's current window is shared by all its clients — so
        // the sidecar always follows it. Swift kept the SSH rule here, and a
        // `prefix n` in the mosh TUI or a desktop's `select-window` left the
        // breadcrumb (and the next pin and zoom) on the old window.
        if self.config.renders_output && current_is_ours && current.as_deref() != Some(window_id) {
            return;
        }
        self.snap().active_window_id = Some(window_id.to_owned());
        self.ensure_immersive_zoom();
    }

    fn handle_active_pane_changed(&mut self, window_id: &str, pane_id: &str) {
        if !self.snapshot.windows.contains_key(window_id) {
            return;
        }
        if self.snapshot.panes.contains_key(pane_id)
            && let Some(pane) = self.snap().panes.get_mut(pane_id)
        {
            pane.is_active = true;
        }
        if self.snapshot.active_window_id.as_deref() != Some(window_id) {
            return;
        }
        // Fix (B587): another client picked another pane in the window we
        // show — tmux keeps one active pane per window, shared — and the
        // phone used to switch to it under the person. It keeps its own,
        // by id, under a veil; a touch takes it back.
        if self.config.renders_output
            && self.own_pane_moves == 0
            && let Some(ours) = self.snapshot.active_pane_id.clone().filter(|ours| {
                ours != pane_id
                    && self
                        .snapshot
                        .panes
                        .get(ours)
                        .is_some_and(|pane| pane.window_id == window_id)
            })
        {
            self.set_pane_follow(Some(crate::lease::PaneFollow {
                window_id: window_id.to_owned(),
                pane_id: ours,
            }));
            return;
        }
        self.snap().active_pane_id = Some(pane_id.to_owned());
        self.set_pane_follow(None);
    }

    /// The pane the phone keeps while another client picked another in the
    /// same window (B587): the view's veil.
    pub fn pane_follow(&self) -> Option<&crate::lease::PaneFollow> {
        self.pane_follow.as_ref()
    }

    fn set_pane_follow(&mut self, follow: Option<crate::lease::PaneFollow>) {
        if self.pane_follow != follow {
            self.pane_follow = follow;
            self.dirty = true;
        }
    }

    /// The person touched the phone (a keystroke, the veil's button): its
    /// own pane again, active and zoomed — the last touch wins (B587).
    /// Whether there was one to take back.
    pub(super) fn retake_pane(&mut self) -> bool {
        let Some(follow) = self.pane_follow.take() else {
            return false;
        };
        self.dirty = true;
        if self.snapshot.active_window_id.as_deref() == Some(follow.window_id.as_str())
            && self.snapshot.active_pane_id.as_deref() == Some(follow.pane_id.as_str())
        {
            self.send(format!("select-pane -t {}", follow.pane_id));
            self.ensure_immersive_zoom();
        }
        true
    }

    /// `list-windows` and `list-panes` (`refreshWindowsAndPanes`).
    fn refresh_windows_and_panes(&mut self, fresh_attach: bool, then: Option<Then>) {
        self.send_command(LIST_WINDOWS.to_owned(), Reply::Windows);
        self.send_command(
            format!("list-panes -a -F {PANE_DISCOVERY_FORMAT}"),
            Reply::Panes {
                fresh_attach,
                restore: true,
                then,
            },
        );
    }

    /// Parse a `list-sessions` reply (public as in Swift, where tests and the
    /// hub's sidecar call it directly).
    pub fn parse_list_sessions<S: AsRef<str>>(&mut self, lines: &[S], now: u64) {
        self.begin_input(now);
        let lines: Vec<String> = lines.iter().map(|line| line.as_ref().to_owned()).collect();
        self.parse_list_sessions_with(&lines, true);
        self.finish_input();
    }

    fn parse_list_sessions_with(&mut self, lines: &[String], locating_renderer: bool) {
        let mut sessions = BTreeMap::new();
        let mut last_attached: Option<String> = None;
        let mut group_ids = BTreeSet::new();
        self.session_clients.clear();
        for line in lines {
            let parts = fields(line, b' ', 2, true);
            if parts.len() < 3 || !is_valid_id(parts[0]) {
                continue;
            }
            if let Some(count) = swift_int(parts[1]) {
                self.session_clients.insert(parts[0].to_owned(), count);
            }
            // B459: a phone's group is a view of a session, never one.
            if is_group_name(parts[2]) {
                group_ids.insert(parts[0].to_owned());
                continue;
            }
            let is_attached = parts[1] != "0";
            sessions.insert(
                parts[0].to_owned(),
                SessionInfo {
                    id: parts[0].to_owned(),
                    name: parts[2].to_owned(),
                    is_attached,
                },
            );
            if is_attached {
                last_attached = Some(parts[0].to_owned());
            }
        }
        self.group.group_ids = group_ids;
        if let Some(session) = self
            .group
            .current
            .as_ref()
            .map(|group| group.session.clone())
            && let Some(info) = sessions.get_mut(&session)
        {
            // The phone is on it, through its group.
            info.is_attached = true;
        }
        let listed: BTreeSet<String> = sessions.keys().cloned().collect();
        let orphan = self.drop_orphaned_group(&listed);
        if self.config.renders_output {
            // Our session is what tmux told US (%session-changed, or our own
            // switch); `session_attached` counts every client, so it only
            // decides when ours is gone or unknown.
            let keep = self
                .snapshot
                .active_session_id
                .as_ref()
                .is_some_and(|current| sessions.contains_key(current));
            let first = sessions.keys().min_by(|a, b| id_cmp(a, b)).cloned();
            let snapshot = self.snap();
            snapshot.sessions = sessions;
            if !keep {
                snapshot.active_session_id = last_attached.or(first);
            }
            if let Some(id) = orphan {
                // Fix (B582): onto the session left on screen first — tmux
                // detaches a client whose session is destroyed
                // (`detach-on-destroy`) — then the group goes.
                if let Some(next) = self.snapshot.active_session_id.clone() {
                    self.select_session_now(&next);
                }
                self.send(format!("kill-session -t '{id}'"));
            }
        } else {
            self.snap().sessions = sessions;
            if locating_renderer {
                self.locate_renderer();
            }
        }
    }

    fn parse_list_windows(&mut self, lines: &[String]) {
        let mut windows = BTreeMap::new();
        let mut links: BTreeMap<String, usize> = BTreeMap::new();
        let mut tmux_active: Option<String> = None;
        // B459: our group's rows are S's, and their current window is the
        // phone's; S's own current window is the desktop's (where the group
        // lands). Another phone's group lists S's windows again: skipped.
        let our_group = self
            .group
            .current
            .as_ref()
            .and_then(|group| Some((group.id.clone()?, group.session.clone())));
        let mut session_active: Option<String> = None;
        let mut from_group: BTreeSet<String> = BTreeSet::new();
        for line in lines {
            let parts = fields(line, b' ', 6, false);
            if parts.len() != 7 || parts[..6].iter().any(|part| part.is_empty()) {
                continue;
            }
            let (listed_session, id) = (parts[0], parts[1]);
            if !is_valid_id(listed_session) || !is_valid_id(id) {
                continue;
            }
            *links.entry(id.to_owned()).or_default() += 1;
            let group_row = our_group
                .as_ref()
                .is_some_and(|(group, _)| group == listed_session);
            if !group_row && self.group.group_ids.contains(listed_session) {
                continue;
            }
            let session_id = match &our_group {
                Some((_, session)) if group_row => session.as_str(),
                _ => listed_session,
            };
            let is_active = parts[4] == "1";
            // `-a` flags every session's current window; only ours counts.
            if is_active && self.snapshot.active_session_id.as_deref() == Some(session_id) {
                if our_group.is_none() || group_row {
                    tmux_active = Some(id.to_owned());
                } else {
                    session_active = Some(id.to_owned());
                }
            }
            if our_group.is_some() && !group_row && from_group.contains(id) {
                continue;
            }
            if group_row {
                from_group.insert(id.to_owned());
            }
            // Fix (linked-window-close, B87): a window linked into several
            // sessions is listed once for each; ours keeps it (the last
            // row won, and the window left our session's tree).
            let ours = self.snapshot.active_session_id.as_deref();
            if ours != Some(session_id)
                && windows
                    .get(id)
                    .is_some_and(|w: &WindowInfo| Some(w.session_id.as_str()) == ours)
            {
                continue;
            }
            windows.insert(
                id.to_owned(),
                WindowInfo {
                    id: id.to_owned(),
                    session_id: session_id.to_owned(),
                    name: parts[6].to_owned(),
                    index: swift_int(parts[2]).unwrap_or(0),
                    layout: parts[3].to_owned(),
                    is_active,
                    pane_count: swift_int(parts[5]).unwrap_or(1),
                    is_zoomed: false,
                },
            );
        }
        self.window_links = links;
        if our_group.is_some()
            && std::mem::take(&mut self.group.landing)
            && let Some(window) = session_active
        {
            // The group starts on S's lowest window: where S is instead.
            if tmux_active.as_deref() != Some(window.as_str()) {
                self.send(format!("select-window -t {window}"));
            }
            tmux_active = Some(window);
        }
        let current_is_ours = self
            .snapshot
            .active_window_id
            .as_deref()
            .and_then(|window| windows.get(window))
            .map(|window: &WindowInfo| window.session_id.clone())
            == self.snapshot.active_session_id;
        // Following tmux's shared current window let another client's switch
        // yank the phone's view: adopt it only when ours is gone. Over mosh
        // the screen is tmux's current window: always (B80, see
        // `handle_session_window_changed`).
        let follow_server = !current_is_ours || !self.config.renders_output;
        let snapshot = self.snap();
        snapshot.windows = windows;
        if follow_server && let Some(active) = tmux_active {
            snapshot.active_window_id = Some(active);
        }
        if self.pending_immersive_zoom {
            self.pending_immersive_zoom = false;
            self.ensure_immersive_zoom();
        }
    }

    fn parse_list_panes(&mut self, lines: &[String]) {
        let mut panes = BTreeMap::new();
        let mut active_pane = self.snapshot.active_pane_id.clone();
        for line in lines {
            let parts = fields(line, b' ', 8, false);
            if parts.len() != 9 || parts[..8].iter().any(|part| part.is_empty()) {
                continue;
            }
            let (id, window_id) = (parts[0], parts[1]);
            if !is_valid_id(id) || !is_valid_id(window_id) {
                continue;
            }
            let is_active = parts[5] == "1";
            self.pane_screen_state.insert(
                id.to_owned(),
                ScreenState {
                    alternate: parts[6] == "1",
                    copy_mode: parts[7] == "1",
                },
            );
            let (command, path) = command_and_path(parts[8]);
            panes.insert(
                id.to_owned(),
                PaneInfo {
                    id: id.to_owned(),
                    window_id: window_id.to_owned(),
                    index: swift_int(parts[2]).unwrap_or(0),
                    command: command.to_owned(),
                    path: path.map(str::to_owned),
                    width: swift_int(parts[3]).unwrap_or(80),
                    height: swift_int(parts[4]).unwrap_or(24),
                    is_active,
                },
            );
            if is_active && self.snapshot.active_window_id.as_deref() == Some(window_id) {
                active_pane = Some(id.to_owned());
            }
        }
        // B587: the pane we keep stays shown while it is there; gone, or
        // active again, the veil goes.
        let kept = self
            .pane_follow
            .as_ref()
            .filter(|follow| {
                self.snapshot.active_window_id.as_deref() == Some(follow.window_id.as_str())
                    && panes
                        .get(&follow.pane_id)
                        .is_some_and(|pane| pane.window_id == follow.window_id && !pane.is_active)
            })
            .map(|follow| follow.pane_id.clone());
        if kept.is_none() {
            self.set_pane_follow(None);
        }
        let snapshot = self.snap();
        snapshot.panes = panes;
        if kept.is_some() {
            snapshot.active_pane_id = kept;
        } else if active_pane.is_some() {
            snapshot.active_pane_id = active_pane;
        }
    }

    /// Give every pane of our session a terminal, and fill the ones on screen
    /// (`ensureTerminalsForAllPanes`). A fresh attach also veils and repaints
    /// the active pane, and pins its window to our grid.
    fn ensure_terminals_for_all_panes(&mut self, fresh_attach: bool) {
        if !self.config.renders_output {
            return;
        }
        let active_session = self.snapshot.active_session_id.clone();
        let session_panes: Vec<String> = self
            .snapshot
            .panes_in_id_order()
            .into_iter()
            .filter(|pane| {
                self.snapshot
                    .windows
                    .get(&pane.window_id)
                    .map(|window| window.session_id.clone())
                    == active_session
            })
            .map(|pane| pane.id.clone())
            .collect();
        for pane in &session_panes {
            if !self.streams.contains_key(pane) {
                self.mint_terminal(pane);
            }
        }
        let active_window = self.snapshot.active_window_id.clone();
        let on_screen: Vec<String> = session_panes
            .iter()
            .filter(|pane| {
                self.snapshot
                    .panes
                    .get(pane.as_str())
                    .map(|info| info.window_id.clone())
                    == active_window
            })
            .cloned()
            .collect();
        let fill = if on_screen.is_empty() {
            session_panes
        } else {
            on_screen
        };
        for pane in &fill {
            self.backfill(pane);
        }
        if fresh_attach && let Some(active) = self.snapshot.active_pane_id.clone() {
            self.view(&active, ViewAction::Veil);
            self.view(&active, ViewAction::ExtendCover { seconds: 2.0 });
            // Unclaimed, `window-size latest` leaves the window at whatever
            // an attached desktop made it: claim it for our grid here.
            if !self.size.pins_released
                && let Some(window) = self.snapshot.active_window_id.clone()
            {
                self.request_pin(&window, PinReason::Attach);
            }
            self.resync_pane(&active, true);
            // The pin makes the program repaint: a second, settled capture.
            self.arm(Slot::SettleResync, 700, Action::AttachSettledResync);
        }
    }

    /// Replay a reconnect's selection once discovery knows the ids
    /// (`applyPendingRestoreIfReady`).
    fn apply_pending_restore_if_ready(&mut self) {
        let Some(restore) = self.pending_restore.clone() else {
            return;
        };
        // B459: not before the attach knows whether it views the session
        // through its own group, and is in it.
        if self.group.deciding {
            return;
        }
        if !self.snapshot.is_attached || self.snapshot.sessions.is_empty() {
            return;
        }
        // B457: a session's current window and its windows' active panes are
        // the session's, not this client's — every client on it shows them.
        // Where the app last was, replayed at every reconnect, moved a
        // desktop on that session to it, mid-typing. Asked first: who else
        // is on the session the restore would move.
        if !self.restore_chosen
            && !self.restore_checked
            && let Some(session) = self.session_a_restore_moves(&restore)
            && self.session_may_have_others(&session)
        {
            if !self.restore_check_in_flight {
                self.restore_check_in_flight = true;
                // The -CC attach read our name already; the sidecar's didn't.
                if self.size.our_client_name.is_none() {
                    self.send_command(
                        "display-message -p '#{client_name}'".to_owned(),
                        Reply::RestoreSelf,
                    );
                }
                self.send_command(
                    format!("list-clients -F '{}'", crate::clients::CLIENT_ROW_FORMAT),
                    Reply::RestoreClients { session },
                );
            }
            return;
        }
        if let Some(pane) = restore
            .pane
            .as_ref()
            .filter(|pane| self.snapshot.panes.contains_key(pane.as_str()))
        {
            self.pending_restore = None;
            self.select_pane_now(&pane.clone());
        } else if let Some(window) = restore
            .window
            .as_ref()
            .filter(|window| self.snapshot.windows.contains_key(window.as_str()))
        {
            self.pending_restore = None;
            self.select_window_now(&window.clone());
        } else if self.snapshot.sessions.contains_key(&restore.session) {
            self.pending_restore = None;
            self.select_session_now(&restore.session);
        } else if !self.snapshot.panes.is_empty() {
            self.pending_restore = None;
        }
    }

    /// The session whose current window or a window's active pane the
    /// restore would change; `None` when it changes neither (or names no
    /// window or pane discovery knows). Through our group (B459) the current
    /// window is ours alone: only a window's active pane is shared.
    fn session_a_restore_moves(&self, restore: &Selection) -> Option<String> {
        let pane = restore
            .pane
            .as_ref()
            .and_then(|pane| self.snapshot.panes.get(pane.as_str()));
        let window_id = pane.map(|pane| pane.window_id.clone()).or_else(|| {
            restore
                .window
                .clone()
                .filter(|window| self.snapshot.windows.contains_key(window.as_str()))
        })?;
        let window = self.snapshot.windows.get(&window_id)?;
        let moves =
            (!self.in_group() && !window.is_active) || pane.is_some_and(|pane| !pane.is_active);
        (moves && !window.session_id.is_empty()).then(|| window.session_id.clone())
    }

    /// Whether `session` may have a client that isn't ours: the session
    /// listing counted more clients there than this one and the phone's mosh
    /// renderer. Not known: maybe. Only then is the listing asked for (one
    /// round trip the usual reconnect, alone on its session, never waits).
    fn session_may_have_others(&self, session: &str) -> bool {
        let Some(&count) = self.session_clients.get(session) else {
            return true;
        };
        // Through our group this client is on the group, not on `session`.
        let mut ours = i64::from(
            !self.in_group() && self.snapshot.active_session_id.as_deref() == Some(session),
        );
        if !self.config.renders_output
            && self.sidecar.renderer_session_id.as_deref() == Some(session)
        {
            ours += 1;
        }
        count > ours
    }

    fn restore_self_answered(&mut self, response: &CommandResponse) {
        self.restore_self = response
            .lines
            .first()
            .map(|line| moshpit_herdr::text::trim_whitespaces(line).to_owned())
            .filter(|name| !name.is_empty() && !response.is_error);
    }

    /// Who else is on the session the restore would move (B457): a client
    /// that is neither this one nor the phone's own mosh renderer keeps its
    /// view — the restore lands on the session's current window instead.
    /// A refused listing reads as nobody else.
    fn restore_clients_answered(&mut self, session: &str, response: &CommandResponse) {
        self.restore_check_in_flight = false;
        self.restore_checked = true;
        let me = self.restore_self.take().or_else(|| self.size.our_client_name.clone());
        let renderer_pid = self.sidecar.renderer_pid;
        let renderer_name = self.sidecar.renderer_client.as_ref().map(|r| r.name.clone());
        let others = !response.is_error
            && response
                .lines
                .iter()
                .filter_map(|line| crate::clients::ClientRow::parse(line))
                .any(|row| {
                    row.session_id == session
                        && Some(&row.name) != me.as_ref()
                        && Some(&row.name) != renderer_name.as_ref()
                        && (row.pid.is_none() || row.pid != renderer_pid)
                });
        let grouped = self.in_group();
        let pane_window = self
            .pending_restore
            .as_ref()
            .and_then(|restore| restore.pane.as_ref())
            .and_then(|pane| self.snapshot.panes.get(pane.as_str()))
            .map(|pane| pane.window_id.clone());
        if others && let Some(restore) = self.pending_restore.as_mut() {
            restore.session = session.to_owned();
            if grouped {
                // B459: the window is ours to pick; its active pane stays.
                restore.window = restore.window.take().or(pane_window);
            } else {
                restore.window = None;
            }
            restore.pane = None;
        }
        self.apply_pending_restore_if_ready();
    }

    // MARK: - Notifications

    fn handle_pane_output(&mut self, pane: &str, data: Vec<u8>) {
        // Once per pane per [`ACTIVITY_NOTE_MS`] (B739): every `%output`
        // line was a note — 2 000 a second from `yes`, each one an event
        // to the app, a call into the agents' machine on the main thread
        // and a state sync there, all ahead of the key that would stop it.
        let now = self.now;
        if self
            .activity_noted
            .get(pane)
            .is_none_or(|at| now.saturating_sub(*at) >= ACTIVITY_NOTE_MS)
        {
            self.activity_noted.insert(pane.to_owned(), now);
            self.notify(Notification::PaneActivity {
                pane: pane.to_owned(),
            });
        }
        let now = self.wall();
        match self.last_pane_output_at.get_mut(pane) {
            Some(at) => *at = now,
            None => {
                self.last_pane_output_at.insert(pane.to_owned(), now);
            }
        }
        if !self.config.renders_output {
            return;
        }
        if self.output_gated(pane) {
            let detector = self.bell_detectors.entry(pane.to_owned()).or_default();
            if detector.contains_bell(&data) {
                self.notify(Notification::PaneBell {
                    pane: pane.to_owned(),
                });
            }
            return;
        }
        // While a convergence capture is due, every byte for the visible
        // window restarts its quiet debounce.
        if self.convergence_pending
            && self
                .snapshot
                .panes
                .get(pane)
                .map(|info| info.window_id.as_str())
                == self.snapshot.active_window_id.as_deref()
        {
            self.convergence_saw_output = true;
            self.schedule_convergence_resync();
        }
        if self.streams.contains_key(pane) {
            self.feed(pane, data);
            return;
        }
        // Output before discovery placed the pane: a terminal on demand, so
        // no byte is dropped.
        if !self.snapshot.panes.contains_key(pane) {
            self.snap()
                .panes
                .insert(pane.to_owned(), PaneInfo::placeholder(pane, ""));
        }
        self.mint_terminal(pane);
        self.feed(pane, data);
    }

    fn handle_window_add(&mut self, window_id: &str) {
        // B459: the make echoes every window of S as added to our group.
        if self.group.entering && self.snapshot.windows.contains_key(window_id) {
            return;
        }
        if !self.snapshot.windows.contains_key(window_id) {
            let session = self.snapshot.active_session_id.clone().unwrap_or_default();
            self.snap()
                .windows
                .insert(window_id.to_owned(), WindowInfo::new(window_id, &session));
        }
        self.send_command(LIST_WINDOWS.to_owned(), Reply::Windows);
        self.send_command(
            format!("list-panes -a -F {PANE_DISCOVERY_FORMAT}"),
            Reply::Panes {
                fresh_attach: false,
                restore: false,
                then: None,
            },
        );
    }

    fn handle_window_close(&mut self, window_id: &str) {
        // Fix (linked-window-close, B87): a close says the window left A
        // session, not that it is gone. tmux 3.6a, a private server: a
        // window linked into A and B, unlinked from B, gives A's control
        // client `%window-close` (the window still in A); unlinked from A,
        // `%unlinked-window-close` (still in B). Swift removed it, panes and
        // terminals with it, until the next listing — the screen leaving a
        // window that was still there. One linked elsewhere is relisted:
        // the listing says whether, and where, it lives on.
        if self.window_links.get(window_id).copied().unwrap_or(0) > 1 {
            self.relist_windows();
            return;
        }
        self.snap().windows.remove(window_id);
        let casualties: Vec<String> = self
            .snapshot
            .panes_in_id_order()
            .into_iter()
            .filter(|pane| pane.window_id == window_id)
            .map(|pane| pane.id.clone())
            .collect();
        for pane in &casualties {
            self.snap().panes.remove(pane);
            self.close_stream(pane);
        }
        if self.snapshot.active_window_id.as_deref() != Some(window_id) {
            return;
        }
        if !self.config.renders_output {
            // Over mosh the screen is tmux's own current window, which tmux
            // has already moved: ask instead of guessing.
            let snapshot = self.snap();
            snapshot.active_window_id = None;
            snapshot.active_pane_id = None;
            self.pending_immersive_zoom = true;
            self.relist_windows();
            return;
        }
        // Land locally, never with select-window (that would yank desktop
        // clients): where tmux moved its current window, else the first.
        let server_choice = self
            .server_current_window
            .as_ref()
            .filter(|(session, window)| {
                self.snapshot.active_session_id.as_deref() == Some(session.as_str())
                    && self.snapshot.windows.contains_key(window)
            })
            .map(|(_, window)| window.clone());
        let fallback =
            server_choice.or_else(|| self.snapshot.sorted_windows().first().map(|w| w.id.clone()));
        self.snap().active_window_id = fallback.clone();
        let pane = fallback.and_then(|window| self.first_pane_of(&window));
        match pane {
            Some(pane) => {
                self.view(&pane, ViewAction::Veil);
                self.view(&pane, ViewAction::ExtendCover { seconds: 2.0 });
                self.snap().active_pane_id = Some(pane.clone());
                self.ensure_immersive_zoom();
                self.resync_pane(&pane, true);
            }
            None => self.snap().active_pane_id = None,
        }
    }

    /// The window's active pane, else its first (in id order).
    fn first_pane_of(&self, window: &str) -> Option<String> {
        let panes = self.snapshot.panes_in_id_order();
        panes
            .iter()
            .find(|pane| pane.window_id == window && pane.is_active)
            .or_else(|| panes.iter().find(|pane| pane.window_id == window))
            .map(|pane| pane.id.clone())
    }

    fn handle_window_renamed(&mut self, window_id: &str, name: &str) {
        if self.snapshot.windows.contains_key(window_id)
            && let Some(window) = self.snap().windows.get_mut(window_id)
        {
            window.name = name.to_owned();
        }
    }

    /// `%sessions-changed`, `%client-session-changed`: one listing on its
    /// way at a time, one more after it if asked meanwhile.
    fn relist_sessions(&mut self) {
        if !self.snapshot.is_attached {
            return;
        }
        if self.sessions_relist.in_flight {
            self.sessions_relist.again = true;
            return;
        }
        self.sessions_relist.in_flight = true;
        self.send_command(LIST_SESSIONS.to_owned(), Reply::RelistSessions);
    }

    /// `%unlinked-window-add` (and a closed window over mosh), coalesced.
    fn relist_windows(&mut self) {
        if !self.snapshot.is_attached {
            return;
        }
        if self.windows_relist.in_flight {
            self.windows_relist.again = true;
            return;
        }
        self.windows_relist.in_flight = true;
        self.refresh_windows_and_panes(false, Some(Then::RelistWindows));
    }

    // MARK: - Terminals

    fn close_stream(&mut self, pane: &str) {
        self.pane_grids.remove(pane);
        self.held_grids.remove(pane);
        if let Some(stream) = self.streams.remove(pane) {
            self.outputs.push_back(Output::StreamClosed {
                pane: pane.to_owned(),
                stream,
            });
        }
    }

    /// Fix (dead-pane-terminals, B87): a pane missing from a whole-server
    /// `list-panes -a` is dead (tmux lists every pane that existed when the
    /// command ran, and never reuses an id); its terminal goes with it.
    /// Swift freed terminals only for a window's `%window-close`, so a killed
    /// session's, or a pane's that exited, lived until the controller did.
    fn close_streams_of_dead_panes(&mut self) {
        let dead: Vec<String> = self
            .streams
            .keys()
            .filter(|pane| !self.snapshot.panes.contains_key(pane.as_str()))
            .cloned()
            .collect();
        for pane in dead {
            self.close_stream(&pane);
        }
    }

    /// The pane's terminal, made on first sight (`mintTerminal`).
    fn mint_terminal(&mut self, pane: &str) -> StreamId {
        if let Some(&stream) = self.streams.get(pane) {
            return stream;
        }
        let stream = self.next_stream;
        self.next_stream += 1;
        self.streams.insert(pane.to_owned(), stream);
        // Born at the client's grid, so a feed before the first layout lands
        // in the shape tmux drew it for.
        self.pane_grids
            .insert(pane.to_owned(), self.last_client_size);
        self.outputs.push_back(Output::StreamOpened {
            pane: pane.to_owned(),
            stream,
        });
        stream
    }

    // MARK: - Navigation

    /// Switch the active window: the snapshot at once, then tmux.
    pub fn select_window(&mut self, window_id: &str, now: u64) {
        self.begin_input(now);
        self.select_window_now(window_id);
        self.finish_input();
    }

    fn select_window_now(&mut self, window_id: &str) {
        // A manual switch beats a reconnect's restore still on its way.
        self.pending_restore = None;
        self.set_pane_follow(None);
        self.release_scroll_holds();
        if let Some(session) = self
            .snapshot
            .windows
            .get(window_id)
            .map(|window| window.session_id.clone())
            .filter(|session| {
                !session.is_empty()
                    && self.snapshot.active_session_id.as_deref() != Some(session.as_str())
            })
        {
            self.select_session_now(&session);
        }
        // Picked: the group (if one was just made) lands here, not where S is.
        self.group.landing = false;
        let old_window = self.snapshot.active_window_id.clone();
        if old_window.as_deref() != Some(window_id) {
            if let Some(old) = old_window {
                self.release_window_pin(&old);
            }
            self.snap().active_window_id = Some(window_id.to_owned());
            // Point the pane at the new window's right now, veiled until its
            // repaint lands.
            if let Some(pane) = self.first_pane_of(window_id) {
                self.view(&pane, ViewAction::Veil);
                let grace = self.cover_grace();
                self.view(&pane, ViewAction::ExtendCover { seconds: grace });
                self.snap().active_pane_id = Some(pane);
            }
        }
        // Our width BEFORE select-window streams the window's output.
        self.request_pin(window_id, PinReason::User);
        if !self.config.renders_output {
            // B270: the select moves the phone's renderer, which sizes the
            // window to the phone at once — ahead of a pin that may wait on
            // the lease: its layout is read before it.
            self.capture_pristine_layout(window_id);
        }
        self.send(format!("select-window -t {window_id}"));
        self.ensure_immersive_zoom();
        self.refresh_active_pane_mouse_now();
        self.ensure_terminals_for_all_panes(false);
        self.resync_active_pane_settling();
    }

    /// Switch the attached client to another session.
    pub fn select_session(&mut self, session_id: &str, now: u64) {
        self.begin_input(now);
        self.select_session_now(session_id);
        self.finish_input();
    }

    fn select_session_now(&mut self, session_id: &str) {
        self.pending_restore = None;
        self.sidecar.session_switches = self.sidecar.session_switches.wrapping_add(1);
        // The sidecar's renderer leaves these (read before the snapshot
        // moves on).
        let left = self.sidecar.renderer_session_id.clone();
        let left_window = self.renderer_window_id();
        if self.snapshot.active_session_id.as_deref() != Some(session_id) {
            self.snap().active_session_id = Some(session_id.to_owned());
        }
        self.sync_status_line();
        if self.config.renders_output {
            // B150: the switch sizes the session's windows by us (its
            // latest client): what it squashes is read on both sides.
            self.probe_switch_layouts(session_id);
            let in_its_group = self
                .group
                .current
                .as_ref()
                .is_some_and(|group| group.session == session_id);
            if in_its_group {
                // Already viewing it.
            } else if self.wants_group_for(session_id) {
                // B459: into a group of its own, in one step — the selects
                // that follow land there, never on the session itself.
                let name = self
                    .snapshot
                    .sessions
                    .get(session_id)
                    .map(|info| info.name.clone())
                    .unwrap_or_default();
                self.enter_group(session_id, &name);
            } else {
                self.send(format!("switch-client -t {session_id}"));
            }
        } else {
            self.sidecar_switch(session_id, left, left_window);
        }
        self.refresh_windows_and_panes(false, None);
    }

    /// Focus a pane and show it full-screen.
    pub fn select_pane(&mut self, pane_id: &str, now: u64) {
        self.begin_input(now);
        self.select_pane_now(pane_id);
        self.finish_input();
    }

    fn select_pane_now(&mut self, pane_id: &str) {
        self.pending_restore = None;
        self.set_pane_follow(None);
        // Known only from its own %output: wait for discovery to place it.
        if let Some(pane) = self.snapshot.panes.get(pane_id)
            && pane.window_id.is_empty()
        {
            self.pending_restore = Some(Selection {
                session: self.snapshot.active_session_id.clone().unwrap_or_default(),
                window: None,
                pane: Some(pane_id.to_owned()),
            });
            self.refresh_windows_and_panes(false, None);
            return;
        }
        self.release_scroll_holds();
        let old_window = self.snapshot.active_window_id.clone();
        let new_window = self
            .snapshot
            .panes
            .get(pane_id)
            .map(|pane| pane.window_id.clone())
            .or_else(|| old_window.clone());
        if let Some(session) = new_window
            .as_ref()
            .and_then(|window| self.snapshot.windows.get(window))
            .map(|window| window.session_id.clone())
            .filter(|session| {
                !session.is_empty()
                    && self.snapshot.active_session_id.as_deref() != Some(session.as_str())
            })
        {
            self.select_session_now(&session);
        }
        self.group.landing = false;
        if let Some(new_window) = new_window
            && Some(&new_window) != old_window.as_ref()
        {
            if let Some(old) = old_window {
                self.release_window_pin(&old);
            }
            self.snap().active_window_id = Some(new_window.clone());
            self.request_pin(&new_window, PinReason::User);
            self.send(format!("select-window -t {new_window}"));
            self.ensure_terminals_for_all_panes(false);
        }
        self.view(pane_id, ViewAction::Veil);
        let grace = self.cover_grace();
        self.view(pane_id, ViewAction::ExtendCover { seconds: grace });
        self.snap().active_pane_id = Some(pane_id.to_owned());
        // -Z: the new pane replaces the old one full-screen in one layout
        // change.
        self.send(format!("select-pane -Z -t {pane_id}"));
        self.ensure_immersive_zoom();
        self.refresh_active_pane_mouse_now();
        self.resync_active_pane_settling();
    }

    /// Swipe: cycle the active window's panes when it has several, else the
    /// session's windows.
    pub fn switch_pane_or_window(&mut self, forward: bool, now: u64) {
        self.begin_input(now);
        if self.snapshot.is_attached
            && let Some(window) = self.snapshot.active_window_id.clone()
        {
            let snapshot = self.snap();
            snapshot.last_switch_forward = forward;
            snapshot.has_swiped_since_attach = true;
            let panes: Vec<String> = self
                .snapshot
                .panes_in_window(&window)
                .iter()
                .map(|pane| pane.id.clone())
                .collect();
            if panes.len() > 1 {
                let from = self
                    .snapshot
                    .active_pane_id
                    .clone()
                    .or_else(|| panes.first().cloned());
                if let Some(next) = crate::format::cycled(&panes, from.as_deref(), forward) {
                    self.select_pane_now(&next);
                }
            } else {
                let windows: Vec<String> = self
                    .snapshot
                    .sorted_windows()
                    .iter()
                    .map(|window| window.id.clone())
                    .collect();
                if let Some(next) = crate::format::cycled(&windows, Some(&window), forward) {
                    self.select_window_now(&next);
                }
            }
        }
        self.finish_input();
    }

    // MARK: - Creating, renaming, killing

    /// A new window in the current session, landed on (`newWindow(named:)`).
    pub fn new_window(&mut self, name: Option<&str>, now: u64) {
        self.begin_input(now);
        self.create_window(None, name);
        self.finish_input();
    }

    /// The Home tree's "New window" on a session row: another session is
    /// named explicitly (`newWindow(inSession:named:)`).
    pub fn new_window_in_session(&mut self, session_id: &str, name: Option<&str>, now: u64) {
        self.begin_input(now);
        let elsewhere = is_valid_id(session_id)
            && self.snapshot.active_session_id.as_deref() != Some(session_id);
        self.create_window(elsewhere.then_some(session_id), name);
        self.finish_input();
    }

    fn create_window(&mut self, session: Option<&str>, name: Option<&str>) {
        let trimmed = name.map(trim_whitespaces_and_newlines).unwrap_or("");
        if refuses_name(trimmed) {
            self.flash_notice(NAME_REFUSAL.to_owned());
            return;
        }
        let name_part = if trimmed.is_empty() {
            String::new()
        } else {
            format!(" -n {}", name_argument(trimmed))
        };
        // Fix (B580): made in another session (the Home tree's row), the
        // window became that session's current one: a desktop on it was
        // switched to it. `-d` makes it without; the phone then picks it in
        // its own view of that session (its group, B459), as a tap does.
        let target_part = session.map(|s| format!(" -d -t {s}:")).unwrap_or_default();
        self.pending_window_focus = None;
        self.send_command(
            format!("new-window{target_part} -P -F '#{{window_id}}'{name_part}"),
            Reply::NewWindowId,
        );
        // Same batch: the id is stashed before the listing's reply runs.
        self.refresh_windows_and_panes(false, Some(Then::FocusNewWindow));
    }

    fn focus_new_window(&mut self) {
        let Some(id) = self.pending_window_focus.take() else {
            return;
        };
        if self.snapshot.windows.contains_key(&id) {
            self.select_window_now(&id);
        }
    }

    /// The Home tree's "New pane" on a window row (`newPane(inWindow:)`).
    pub fn new_pane_in_window(&mut self, window_id: &str, now: u64) {
        self.begin_input(now);
        if self.snapshot.windows.contains_key(window_id)
            && self.snapshot.active_window_id.as_deref() != Some(window_id)
        {
            self.select_window_now(window_id);
        }
        self.new_pane_now();
        self.finish_input();
    }

    /// Split the active window and show the new pane full-screen.
    pub fn new_pane(&mut self, now: u64) {
        self.begin_input(now);
        self.new_pane_now();
        self.finish_input();
    }

    fn new_pane_now(&mut self) {
        let target = self
            .snapshot
            .active_pane_id
            .clone()
            .or_else(|| self.snapshot.active_window_id.clone());
        match target {
            Some(target) => self.send(format!("split-window -t {target}")),
            None => self.send("split-window".to_owned()),
        }
        if let Some(window) = self.snapshot.active_window_id.clone() {
            self.send(format!("resize-pane -Z -t {window}"));
            self.bump_zoom_epoch();
            // Fix (B588): marked as ours, as the probe's zoom is (B458): the
            // window handed back gets it undone, and a sweep after our death
            // tells it from a desktop's own. The new pane is the window's
            // active one: `-F` names it.
            if self.config.renders_output
                && let Some(name) = self.size.our_client_name.clone()
            {
                self.send(format!(
                    "set-option -w -t {window} -F {ZOOM_MARK_OPTION} {}",
                    crate::format::command_string_literal(&format!("{name} #{{pane_id}}"))
                ));
                self.size.zoom_marks.insert(window);
            }
        }
        self.refresh_windows_and_panes(false, None);
    }

    /// A new session, landed on.
    pub fn new_session(&mut self, name: Option<&str>, now: u64) {
        self.begin_input(now);
        let trimmed = name.map(trim_whitespaces_and_newlines).unwrap_or("");
        if refuses_name(trimmed) {
            self.flash_notice(NAME_REFUSAL.to_owned());
            self.finish_input();
            return;
        }
        let name_part = if trimmed.is_empty() {
            String::new()
        } else {
            format!(" -s {}", name_argument(trimmed))
        };
        // Its name with its id (B584): the group the landing makes is
        // named after it, before any listing has it — tmux's own name for
        // one not named, or for a name it changed.
        self.send_command(
            format!("new-session -d -P -F '#{{session_id}} #{{session_name}}'{name_part}"),
            Reply::NewSessionId {
                name_given: !trimmed.is_empty(),
                name: trimmed.to_owned(),
            },
        );
        self.send_command(
            LIST_SESSIONS.to_owned(),
            Reply::Sessions {
                locating_renderer: true,
            },
        );
        self.finish_input();
    }

    fn new_session_answered(&mut self, name_given: bool, name: &str, response: &CommandResponse) {
        if response.is_error {
            let message = response
                .lines
                .first()
                .cloned()
                .unwrap_or_else(|| "new-session failed".to_owned());
            self.flash_notice(message);
            return;
        }
        let Some(line) = response.lines.first().map(|line| trim_whitespaces(line)) else {
            return;
        };
        let (id, listed) = line.split_once(' ').unwrap_or((line, ""));
        let id = id.to_owned();
        if !is_valid_id(&id) {
            return;
        }
        // Fix (B584): known by name before the listing comes back, so the
        // landing's group is named after it, not `s~mp-…`.
        let name = if listed.is_empty() { name } else { listed };
        if !name.is_empty() && !self.snapshot.sessions.contains_key(&id) {
            self.snap().sessions.insert(
                id.clone(),
                SessionInfo {
                    id: id.clone(),
                    name: name.to_owned(),
                    is_attached: false,
                },
            );
        }
        if !self.config.renders_output && name_given {
            self.sidecar.owned_session_ids.insert(id.clone());
            // B86: on the session itself, so a sidecar after this one —
            // the app killed and opened again — still knows.
            self.send(format!("set-option -t '{id}' {OWNED_OPTION} 1"));
        }
        self.select_session_now(&id);
    }

    pub fn rename_session(&mut self, session_id: &str, name: &str, now: u64) {
        self.begin_input(now);
        let trimmed = trim_whitespaces_and_newlines(name);
        // The target by its id, spliced bare: an id or nothing.
        if !trimmed.is_empty() && is_valid_id(session_id) {
            if refuses_name(trimmed) {
                self.flash_notice(NAME_REFUSAL.to_owned());
            } else {
                self.send_surfacing_errors(
                    format!(
                        "rename-session -t {session_id} -- {}",
                        name_argument(trimmed)
                    ),
                    "rename",
                );
                self.send_command(
                    LIST_SESSIONS.to_owned(),
                    Reply::Sessions {
                        locating_renderer: true,
                    },
                );
            }
        }
        self.finish_input();
    }

    /// Kill a session, pruned locally at once (if it was the last one the
    /// server exits and nothing answers).
    pub fn kill_session(&mut self, session_id: &str, now: u64) {
        self.begin_input(now);
        if !is_valid_id(session_id) {
            self.finish_input();
            return;
        }
        // B459: our group would keep the windows alive; it goes with it.
        let group = self
            .group
            .current
            .take_if(|group| group.session == session_id);
        // Fix (B582): tmux detaches a client whose session is destroyed
        // (`detach-on-destroy`, on by default): killing the session on
        // screen ended control mode. The phone moves first, to the session
        // it falls back to (into a group of it), and the kill follows.
        let on_it = self.snapshot.active_session_id.as_deref() == Some(session_id);
        let renderer_on_it = self.sidecar.renderer_session_id.as_deref() == Some(session_id);
        let next = self
            .snapshot
            .sessions
            .keys()
            .filter(|id| id.as_str() != session_id)
            .min_by(|a, b| id_cmp(a, b))
            .cloned();
        if let Some(next) = next {
            if self.config.renders_output {
                if on_it {
                    self.select_session_now(&next);
                }
            } else if on_it || renderer_on_it {
                // Fix (B583): the mosh sidecar's renderer is mosh's own
                // `exec tmux attach`; detached, it ended the mosh session.
                // It moves first, by the name it is known by — the kill
                // goes out right behind, so no listing is waited on — and
                // this client with it.
                if renderer_on_it && let Some(renderer) = self.sidecar.renderer_client.clone() {
                    self.send(format!("switch-client -c {} -t {next}", renderer.name));
                    // Nothing to hand back on a session about to go.
                    self.sidecar.renderer_session_id = Some(next.clone());
                }
                self.send(format!("switch-client -t {next}"));
                self.select_session_now(&next);
            }
        }
        self.send_surfacing_errors(format!("kill-session -t {session_id}"), "kill session");
        if let Some(id) = group.and_then(|group| group.id) {
            self.send(format!("kill-session -t '{id}'"));
        }
        let windows: Vec<String> = self
            .snapshot
            .windows
            .values()
            .filter(|window| window.session_id == session_id)
            .map(|window| window.id.clone())
            .collect();
        let snapshot = self.snap();
        snapshot.sessions.remove(session_id);
        for window in &windows {
            snapshot.windows.remove(window);
            snapshot.panes.retain(|_, pane| &pane.window_id != window);
        }
        if snapshot.active_session_id.as_deref() == Some(session_id) {
            snapshot.active_session_id = snapshot
                .sessions
                .keys()
                .min_by(|a, b| id_cmp(a, b))
                .cloned();
        }
        self.send_command(
            LIST_SESSIONS.to_owned(),
            Reply::Sessions {
                locating_renderer: true,
            },
        );
        self.refresh_windows_and_panes(false, None);
        self.finish_input();
    }

    pub fn rename_window(&mut self, window_id: &str, name: &str, now: u64) {
        self.begin_input(now);
        let trimmed = trim_whitespaces_and_newlines(name);
        if !trimmed.is_empty() && is_valid_id(window_id) {
            if refuses_name(trimmed) {
                self.flash_notice(NAME_REFUSAL.to_owned());
            } else {
                self.send_surfacing_errors(
                    format!("rename-window -t {window_id} -- {}", name_argument(trimmed)),
                    "rename",
                );
                self.refresh_windows_and_panes(false, None);
            }
        }
        self.finish_input();
    }

    pub fn kill_window(&mut self, window_id: &str, now: u64) {
        self.begin_input(now);
        if is_valid_id(window_id) {
            self.send_surfacing_errors(format!("kill-window -t {window_id}"), "kill window");
            self.refresh_windows_and_panes(false, None);
        }
        self.finish_input();
    }

    pub fn kill_pane(&mut self, pane_id: &str, now: u64) {
        self.begin_input(now);
        if is_valid_id(pane_id) {
            self.send_surfacing_errors(format!("kill-pane -t {pane_id}"), "kill pane");
            self.refresh_windows_and_panes(false, None);
        }
        self.finish_input();
    }

    /// Any command, reply ignored (`runCommand`). Dropped until attached.
    /// One that may answer with more than one block (`if-shell`,
    /// `source-file`, `a ; b`, an alias) is fenced (B273): the replies of
    /// every command after it stay where they belong.
    pub fn run_command(&mut self, command: &str, now: u64) {
        self.begin_input(now);
        self.send(command.to_owned());
        self.finish_input();
    }

    /// Re-run discovery and repaint the visible pane (the refresh control).
    pub fn refresh(&mut self, now: u64) {
        self.begin_input(now);
        if self.snapshot.is_attached {
            self.is_refreshing = true;
            self.dirty = true;
            self.send_command(
                LIST_SESSIONS.to_owned(),
                Reply::Sessions {
                    locating_renderer: true,
                },
            );
            self.refresh_windows_and_panes(false, None);
            self.resync_active_pane(true);
            self.send_command("display-message -p done".to_owned(), Reply::RefreshDone);
        }
        self.finish_input();
    }

    /// Re-list sessions, windows and panes, quietly: no spinner, no
    /// repaint (B586). tmux says nothing when a window's place changes
    /// (`swap-window`, `move-window -r`, a renumber after a kill elsewhere
    /// in the order): the tree kept the old numbers and order. The app asks
    /// as Home or the window picker shows, every 30 s while either does,
    /// and the controller itself on the return from the background.
    pub fn relist(&mut self, now: u64) {
        self.begin_input(now);
        self.relist_now();
        self.finish_input();
    }

    pub(super) fn relist_now(&mut self) {
        if !self.snapshot.is_attached {
            return;
        }
        self.send_command(
            LIST_SESSIONS.to_owned(),
            Reply::Sessions {
                locating_renderer: true,
            },
        );
        self.refresh_windows_and_panes(false, None);
    }

    fn send_surfacing_errors(&mut self, command: String, verb: &'static str) {
        self.send_command(command, Reply::SurfaceError { verb });
    }

    fn flash_notice(&mut self, message: String) {
        self.notice = Some(message);
        self.dirty = true;
        self.arm(Slot::NoticeClear, 3000, Action::ClearNotice);
    }

    // MARK: - The view's reports

    /// A pane's view was revealed (its cover's own timeout included):
    /// the first of an attach is its first frame.
    pub fn view_revealed(&mut self, _pane: &str, now: u64) {
        self.begin_input(now);
        self.note_pane_revealed();
        self.finish_input();
    }

    fn note_pane_revealed(&mut self) {
        if self.has_painted_since_attach {
            return;
        }
        self.has_painted_since_attach = true;
        self.notify(Notification::FirstFramePainted);
    }

    /// The view's bell (`coordinator.onBell`).
    pub fn view_bell(&mut self, pane: &str, now: u64) {
        self.begin_input(now);
        self.notify(Notification::PaneBell {
            pane: pane.to_owned(),
        });
        self.finish_input();
    }

    /// Keystrokes typed into a pane's view (`coordinator.onInput`): dropped
    /// while the transport is not live, or they replay after a resume into a
    /// pane nobody saw.
    pub fn view_input(&mut self, pane: &str, bytes: &[u8], now: u64) {
        self.begin_input(now);
        if self.transport_live {
            self.send_input_now(bytes, pane);
        }
        self.finish_input();
    }

    /// The view's layout reporting its grid (`onGridReport`; `onSizeChange`
    /// too): the active pane (or a lone one) sizes the client.
    pub fn view_grid(&mut self, pane: &str, cols: i64, rows: i64, now: u64) {
        self.begin_input(now);
        if self.streams.contains_key(pane) {
            self.pane_grids.insert(pane.to_owned(), (cols, rows));
        }
        if self.is_sizing_pane(pane) {
            self.resize_client_now(rows, cols);
        }
        self.finish_input();
    }

    /// The view is about to resize and asks whether to hold it until the
    /// repaint that answers it (`shouldDeferResize`). Holding means the
    /// client resize is under way; the frame's arrival (or 1.5 s) releases
    /// it with [`ViewAction::ApplyDeferredResize`].
    pub fn view_should_defer_resize(&mut self, pane: &str, cols: i64, rows: i64, now: u64) -> bool {
        self.begin_input(now);
        let defer = self.config.renders_output
            && self.snapshot.is_attached
            && self.is_sizing_pane(pane)
            && self.resize_client_now(rows, cols);
        if self.streams.contains_key(pane) {
            // Held, the view keeps its old grid until the repaint.
            if defer {
                self.held_grids.insert(pane.to_owned(), (cols, rows));
            } else {
                self.pane_grids.insert(pane.to_owned(), (cols, rows));
            }
        }
        if defer {
            self.pane_resize_awaiting_capture.insert(pane.to_owned());
            self.arm(
                Slot::DeferredResize(pane.to_owned()),
                1500,
                Action::DeferredResize(pane.to_owned()),
            );
        }
        self.finish_input();
        defer
    }

    fn is_sizing_pane(&self, pane: &str) -> bool {
        self.snapshot.active_pane_id.as_deref() == Some(pane)
            || self.snapshot.active_panes().len() <= 1
    }

    /// The input method switched (same keyboard height, no resize): converge
    /// with tmux's model.
    pub fn view_input_mode_changed(&mut self, pane: &str, now: u64) {
        self.begin_input(now);
        self.resync_pane(pane, true);
        self.finish_input();
    }

    /// A line was submitted: repaint once the program's redraw has settled.
    pub fn view_return_sent(&mut self, pane: &str, now: u64) {
        self.begin_input(now);
        self.arm(
            Slot::SettleResync,
            450,
            Action::ReturnResync(pane.to_owned()),
        );
        self.finish_input();
    }

    // MARK: - Dead-channel watchdog

    fn start_watchdog(&mut self) {
        self.watchdog = Some(Watchdog::default());
        self.arm(Slot::Watchdog, 4000, Action::Watchdog);
    }

    /// Every 4 s: commands piling up while the channel says nothing — or
    /// bytes flowing while replies stop (a wedged parser) — is death.
    /// Counted in beats, so a suspension can't fake it.
    fn watchdog_beat(&mut self) {
        let Some(mut state) = self.watchdog.take() else {
            return;
        };
        let incoming = self.last_incoming_at;
        let has_pendings = self.snapshot.is_attached && !self.pending.is_empty();
        state.strikes = if has_pendings && incoming == state.last_seen {
            state.strikes + 1
        } else {
            0
        };
        state.pop_strikes = if has_pendings && self.pop_counter == state.last_pops {
            state.pop_strikes + 1
        } else {
            0
        };
        state.last_seen = incoming;
        state.last_pops = self.pop_counter;
        let silence = 3i64.max((self.last_resync_round_trip * 6.0 / 4.0).ceil() as i64);
        if i64::from(state.strikes) >= silence {
            self.report_channel_dead("silent");
            return;
        }
        if state.pop_strikes >= 8 {
            self.report_channel_dead("no-pops");
            return;
        }
        self.watchdog = Some(state);
        // The next beat is 4 s from the input that let this one fire, not
        // from its deadline: after a suspension (iOS runs nothing) the
        // overdue beat fires once, as Swift's repeating `Timer` did. Armed
        // from the deadline, every beat the suspension skipped was replayed
        // back to back before the bytes waiting in the socket were read —
        // three of them with a command unanswered (the pins handed back on
        // the way out) were "silent", and a live channel was torn down on
        // every return after 12 s or more away.
        let from = self.advancing_to.max(self.now);
        self.timers
            .arm(Slot::Watchdog, from + 4000, Action::Watchdog);
    }

    fn report_channel_dead(&mut self, reason: &str) {
        self.watchdog = None;
        self.timers.cancel(&Slot::Watchdog);
        self.notify(Notification::ChannelDead {
            reason: reason.to_owned(),
        });
    }
}

/// Wall-clock time the way Foundation keeps a `Date`: seconds since
/// 2001-01-01 in an f64. Intervals between two of them come out exactly as
/// the Swift controller's `Date().timeIntervalSince(…)` did — boundaries
/// included (a 300 ms gap reads as a hair over 0.3 s from some starting
/// points), which keeps the two controllers' decisions identical on every
/// clock reading; the monotonic input clock drives the timers.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub(crate) struct Wall(f64);

impl Wall {
    const UNIX_TO_REFERENCE: f64 = 978_307_200.0;

    fn at(unix_ms: i64) -> Self {
        Wall(unix_ms as f64 / 1000.0 - Self::UNIX_TO_REFERENCE)
    }

    /// Seconds from `earlier` to this.
    pub(crate) fn since(self, earlier: Wall) -> f64 {
        self.0 - earlier.0
    }

    pub(crate) fn unix(self) -> f64 {
        self.0 + Self::UNIX_TO_REFERENCE
    }

    pub(crate) fn adding(self, seconds: f64) -> Wall {
        Wall(self.0 + seconds)
    }
}

/// A listing's rows — or `None` when tmux refused it: a refusal says nothing
/// about what the server holds, so the snapshot keeps what it had.
fn listing(response: &CommandResponse) -> Option<&[String]> {
    (!response.is_error).then_some(response.lines.as_slice())
}

/// What a command still waiting gets when the channel closes first: never a
/// reply tmux sends (its ids are not negative).
fn channel_closed() -> CommandResponse {
    CommandResponse {
        command_id: -1,
        command_num: -1,
        is_error: true,
        lines: Vec::new(),
        from_client: true,
    }
}
