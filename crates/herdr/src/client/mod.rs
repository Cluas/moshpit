//! The herdr control client as a sans-I/O state machine (W2b): what
//! `Moshpit/Services/Herdr/HerdrControlClient.swift`, the herdr half of
//! `Moshpit/Services/SessionHub.swift` (the frame channel's attach, retarget
//! and input, the notices), `HerdrSocketClient.swift` and
//! `HerdrPushDriver.swift` do around the pure pieces in the rest of this
//! crate, on the caller's clock.
//!
//! **Inputs**, each with the caller's monotonic clock in milliseconds: the
//! answer to an exec the client asked for ([`Client::exec_finished`]), bytes
//! off the frame channel ([`Client::frame_bytes`]) and its end, bytes off the
//! push channel and its life, the person's intents ([`Client::act`] and the
//! input methods), and timer ticks ([`Client::tick`] once
//! [`Client::next_deadline`] has passed).
//!
//! **Outputs** queue up in order ([`Client::poll_output`]): commands to run
//! on fresh exec channels, lines to write on the frame channel, the push
//! channel's life and writes, bytes for a pane's terminal (by stream id),
//! what a pane's view should do, and [`Event`]s for the session and the UI.
//!
//! **The contract**, which every test holds it to:
//!
//! - Order. Commands that change the server run one at a time in the order
//!   issued, on the command lane (B7, B26, B44); a step holds the ones
//!   behind it at most [`Budgets::command_hold`]. Frame-channel lines and
//!   semantic keys share the write lane (B2): a key waits for the text
//!   written before it, the text after it waits for the key. The host
//!   carries outputs out in order, and a [`Output::FrameWrite`] is handed to
//!   the channel before anything after it is done.
//! - A definite outcome for everything asked, in bounded time. Every exec
//!   has a budget, every request ([`RequestId`]) gets exactly one answering
//!   event, whatever the connection does: a half-open channel costs a
//!   timeout, never a hang (B89, B119, B120 and the unbounded waits listed
//!   in `harness/client/divergences.json`).
//! - Evidence. A command is done when herdr said so: exit status 0 behind
//!   the trailer, or its `result` printed. A missing trailer is never
//!   success (B72, B90, B119, B121), and the trailer only counts at the end
//!   of the output ([`read_result`]).
//! - Input reaches only the pane it was typed at, and only once that pane's
//!   attach is the one reading (B5, B25): never the pane being left, never
//!   the frame loop's `read`. What can't be sent is said
//!   ([`Event::InputDropped`]), never swallowed.
//! - A fault is not a refusal: `input ignored` from a live attach leaves the
//!   attach alone (B117).
//!
//! How the session drives it (`crates/session`'s herdr leg) is written down
//! at the end of this file's [`Client`] docs.

mod attach;
mod exec;
mod push;
#[doc(hidden)]
pub mod rig;
#[doc(hidden)]
pub mod scenario;
mod task;
mod timers;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub use exec::{ExecResult, read_result};
pub use push::pump_boot_line_for;
pub use task::RestartOutcome;

use task::Answer;

use crate::control::{
    self, AgentTaskRequest, PollEffect, PollReading, PollState, Refusal, TaskOutcome,
    WorktreeRemoval,
};
use crate::frame::{MouseAction, MouseButton, ScrollSource};
use crate::launch;
use crate::snapshot::{self, AgentHook, Snapshot};
use crate::version::{VersionNotice, VersionReport};
use timers::{Slot, Timers};

/// A pane's byte stream: the id the host attaches a terminal view to.
pub type StreamId = u64;

/// One exec the client asked for ([`Output::Exec`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExecId(pub u64);

/// One request whose answer comes back as an [`Event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub u64);

/// Where the push pump finds herdr's socket by default: the default
/// session's, as `HerdrPushBoot` hardcodes it.
pub const DEFAULT_SOCKET_PATH: &str = "~/.config/herdr/herdr.sock";

/// Every wait the client makes, in milliseconds. [`Default`] is the app's
/// values where the app has one; where it waited forever, the bound chosen
/// here says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budgets {
    /// How long one command-lane step holds the steps behind it
    /// (`commandBudget`, 5 s).
    pub command_hold: u64,
    /// How long one semantic key holds the input behind it
    /// (`keySendBudget`, 3 s).
    pub key_hold: u64,
    /// A snapshot read. The app waited forever, and its poll loop stopped
    /// with it; past this the read is given up and the next one scheduled.
    pub poll: u64,
    /// `herdr status`: the first message's read (`commandBudget`) and the
    /// skew diagnosis (forever in the app, inside the poll).
    pub status: u64,
    /// `worktree create` (`worktreeCreateBudget`, 60 s).
    pub worktree_create: u64,
    /// `pane run` (`agentLaunchBudget`, 20 s).
    pub agent_launch: u64,
    /// `worktree remove` (forever in the app, B119).
    pub worktree_remove: u64,
    /// The panes' repo resolve and the agent list (forever, B120).
    pub discovery: u64,
    /// The `$HOME` scan (`scanBudget`, 8 s).
    pub home_scan: u64,
    /// `firstMessageSettle`, `firstMessagePatience`,
    /// `firstMessageRetryInterval`, `firstMessageSubmitDelay`.
    pub first_message_settle: u64,
    pub first_message_patience: u64,
    pub first_message_retry: u64,
    pub first_message_submit_delay: u64,
    /// `server stop` and the daemon start (forever in the app).
    pub server_command: u64,
    /// The bootstrap's first workspace (forever in the app).
    pub bootstrap_create: u64,
    /// Between `server stop` and the start (1 s).
    pub restart_pause: u64,
    /// Before each of the bootstrap's reads (500 ms, ten of them).
    pub bootstrap_interval: u64,
    pub bootstrap_rounds: u32,
    /// The release's `terminal.closed` (1.5 s).
    pub release_ack: u64,
    /// The login shell's prompt before a one-shot attach line (400 ms).
    pub prompt_settle: u64,
    /// The reader loop's announcement after the boot line (2.5 s).
    pub loop_probe: u64,
    /// An attach that never painted may be asked for again (3 s).
    pub attach_timeout: u64,
    /// herdr refused an attach and said why (version skew, a refused
    /// handshake): attaching backs off this long (30 s) — a refusal doesn't
    /// heal by asking again on every read.
    pub refusal_backoff: u64,
    /// An expected `terminal.closed` that never came is forgotten after
    /// this (the app never forgot one).
    pub expected_close: u64,
    /// Input typed at the pane being switched to waits at most this.
    pub held_input: u64,
    /// A delivery to one pane (a notification's reply) waits at most this
    /// for its pane's attach (5 s).
    pub delivery: u64,
    /// The push pump's ready marker and the subscribe's reply (15 s, 10 s).
    pub pump_ready: u64,
    pub subscribe: u64,
    /// After an active push channel dies, try once more after this (the
    /// app never retried, and its poll stayed on the 45 s cadence).
    pub push_retry: u64,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            command_hold: 5_000,
            key_hold: 3_000,
            poll: 10_000,
            status: 5_000,
            worktree_create: 60_000,
            agent_launch: 20_000,
            worktree_remove: 60_000,
            discovery: 10_000,
            home_scan: 8_000,
            first_message_settle: 2_000,
            first_message_patience: 30_000,
            first_message_retry: 1_000,
            first_message_submit_delay: 300,
            server_command: 10_000,
            bootstrap_create: 10_000,
            restart_pause: 1_000,
            bootstrap_interval: 500,
            bootstrap_rounds: 10,
            release_ack: 1_500,
            prompt_settle: 400,
            loop_probe: 2_500,
            attach_timeout: 3_000,
            refusal_backoff: 30_000,
            expected_close: 5_000,
            held_input: 5_000,
            delivery: 5_000,
            pump_ready: 15_000,
            subscribe: 10_000,
            push_retry: 10_000,
        }
    }
}

/// The client's knobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The connection's herdr binary when the user named one, run verbatim
    /// (`connection.herdrPath`).
    pub custom_path: Option<String>,
    /// The session's terminal channel speaks herdr's frame protocol (SSH).
    /// `false` over mosh: mosh renders, this is the control plane only.
    pub frame_channel: bool,
    /// The frame channel's program is the reader loop itself
    /// ([`crate::frame::loop_program`]), started with the channel: nothing
    /// is ever typed into a login shell — no boot line, no one-shot line —
    /// and the first target waits for the loop to say it reads. Typed into
    /// an interactive login shell, the boot line met shells still starting
    /// (a flushed type-ahead, a plugin that re-highlights every character,
    /// a startup prompt reading stdin), and every re-ask typed a release
    /// and a one-shot line into the same shell: the loop never came up, the
    /// shell chewed on the app's lines, and the terminal showed a blinking
    /// cursor forever.
    pub frame_loop: bool,
    /// mosh showing herdr's full TUI: one pane fills the shared screen by
    /// zooming it server-side (`immersiveZoom`).
    pub immersive_zoom: bool,
    /// Bring up the push channel (the host has python3).
    pub push: bool,
    /// The socket the push pump connects to.
    pub socket_path: String,
    /// The connection's id: what the frame loop's pid file is named after
    /// ([`crate::frame::loop_pid_path`]), so the next connection's boot line
    /// can stop the attach a dropped one left behind.
    pub connection_id: String,
    pub budgets: Budgets,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            custom_path: None,
            frame_channel: true,
            frame_loop: false,
            immersive_zoom: false,
            push: false,
            socket_path: DEFAULT_SOCKET_PATH.to_owned(),
            connection_id: String::new(),
            budgets: Budgets::default(),
        }
    }
}

/// What the client asks of its host, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// Run `command` on a fresh exec channel (through the account's shell,
    /// no PTY, nothing on stdin), then answer [`Client::exec_finished`].
    Exec {
        id: ExecId,
        command: String,
    },
    /// Nobody waits for this exec any more: its channel may be closed. An
    /// answer that still comes is ignored.
    Abandon {
        id: ExecId,
    },
    /// Write this line, then `\n`, on the frame channel.
    FrameWrite(String),
    /// Open the push channel: a terminal channel (80×24) whose program is
    /// `program`, the socket pump ([`pump_boot_line_for`]) — never a login
    /// shell to type it into; then [`Client::push_opened`] or
    /// [`Client::push_failed`].
    OpenPush {
        program: String,
    },
    /// Write these bytes on the push channel.
    PushWrite(Vec<u8>),
    /// Close the push channel; nothing more comes from it.
    ClosePush,
    /// A pane got a terminal: the host makes (or keeps) a view for it.
    StreamOpened {
        pane: String,
        stream: StreamId,
    },
    /// A pane's terminal is gone (the pane closed).
    StreamClosed {
        pane: String,
        stream: StreamId,
    },
    /// Bytes for a pane's terminal, in order.
    PaneBytes {
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
    Event(Event),
}

/// What a pane's view should do (the coordinator calls of SessionHub).
#[derive(Debug, Clone, PartialEq)]
pub enum ViewAction {
    /// Cover the terminal before its stale screen shows (`veilForSwitch`).
    Veil,
    /// Keep an active cover up `seconds` more (`extendCoverTimeout`).
    ExtendCover { seconds: f64 },
    /// Lift the cover: what is on the terminal is the pane's.
    Reveal,
}

/// What the session and the UI hear about.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The tree, the agent stamps, a notice, the cadence or a flag changed:
    /// read them again through the accessors.
    StateChanged,
    /// The agent stamps were re-read (`onAgentHooksUpdated`).
    AgentHooksUpdated,
    /// The server's focused pane, after every tree (`onFocusedPaneChanged`).
    FocusedPane(String),
    /// The first frame of this connection painted.
    FirstFrame,
    /// The frame channel ended: the session redials.
    FrameChannelLost,
    /// Input typed at `pane` was not sent: its attach isn't the one
    /// reading. The notice says so too.
    InputDropped { pane: String },
    /// A command the person issued failed, in herdr's words (the app only
    /// re-read; the tree tells the rest).
    CommandFailed { command: String, reason: Refusal },
    /// herdr's frame channel turned one of our lines away on a live attach
    /// (`input ignored: …`). Nothing else happens (B117).
    InputIgnored(String),
    /// The push channel came up (true) or went (false).
    PushActive(bool),
    Task {
        request: RequestId,
        outcome: TaskOutcome,
    },
    Removal {
        request: RequestId,
        outcome: WorktreeRemoval,
    },
    /// The repositories for the New agent task form: the panes' own first,
    /// then the scan's.
    Repos {
        request: RequestId,
        repos: Vec<String>,
    },
    /// The agents herdr detects. `answered`: herdr's list came back (an
    /// empty list then means none).
    Agents {
        request: RequestId,
        names: Vec<String>,
        answered: bool,
    },
    Restart {
        request: RequestId,
        outcome: RestartOutcome,
    },
    /// The pane the server has focused once every command queued before the
    /// request has landed (`confirmedFocus`), `None` if it couldn't be read.
    FocusConfirmed {
        request: RequestId,
        pane: Option<String>,
    },
    /// Input for one pane ([`Action::DeliverInput`]) went out (true), or
    /// couldn't reach that pane in time (false).
    Delivered { request: RequestId, delivered: bool },
}

/// The banner over the terminal (`herdrNotice`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// herdr refused the attach and said why.
    Refused(String),
    /// The server refuses this client's protocol; which side is older is not
    /// known (yet).
    Mismatch(String),
    /// The skew, in versions.
    Skew(VersionNotice),
    /// Another device took the pane on screen over, or holds the pane the
    /// focus went to: never taken without the person ("Open here",
    /// [`NoticeAction::OpenHere`]).
    OnAnotherDevice,
    /// Input was typed where no pane could take it.
    InputDropped,
    /// Content for one pane (a notification's reply, a share) couldn't be
    /// put on its pane, so nothing was sent (mosh's raw-attach renderer:
    /// [`crate::renderer::Notice::CouldntSwitch`]).
    CouldntSwitch,
    Restarting,
    RestartFailed,
}

impl Notice {
    /// The app's English text (its string catalog's source).
    pub fn english(&self) -> String {
        match self {
            Notice::Refused(message) | Notice::Mismatch(message) => message.clone(),
            Notice::Skew(notice) => notice.english(),
            Notice::OnAnotherDevice => "This pane is open on another device".to_owned(),
            Notice::InputDropped => "Not attached to the pane yet — input wasn't sent".to_owned(),
            Notice::CouldntSwitch => "Couldn't switch to that pane — nothing was sent".to_owned(),
            Notice::Restarting => "Restarting herdr server…".to_owned(),
            Notice::RestartFailed => {
                "Restart didn't take — on the host, run: herdr server stop && herdr".to_owned()
            }
        }
    }
}

/// The banner's one-tap remedy (`herdrNoticeAction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeAction {
    /// "Restart herdr server": only offered when restarting would not
    /// downgrade the server.
    RestartServer,
    /// "Retry", after a restart that didn't take.
    Retry,
    /// "Open here": take the pane another device has (`--takeover`).
    OpenHere,
}

impl NoticeAction {
    pub fn english(self) -> &'static str {
        match self {
            NoticeAction::RestartServer => "Restart herdr server",
            NoticeAction::Retry => "Retry",
            NoticeAction::OpenHere => "Open here",
        }
    }
}

/// What the person (or the UI) asks of herdr. [`Client::act`] takes one;
/// the ones with an answer return its [`RequestId`].
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SelectSession(String),
    SelectWindow(String),
    SelectPane(String),
    /// The empty state's "create": a workspace, or a server when there is
    /// none (the one way either comes into existence).
    NewSession {
        name: Option<String>,
    },
    NewWindow {
        name: Option<String>,
    },
    NewPane,
    NewWindowInSession {
        session: String,
        name: Option<String>,
    },
    NewPaneInWindow(String),
    RenameSession {
        session: String,
        name: String,
    },
    KillSession(String),
    RenameWindow {
        window: String,
        name: String,
    },
    KillWindow(String),
    KillPane(String),
    /// The refresh control: read now, with the spinner.
    Refresh,
    StartAgentTask(AgentTaskRequest),
    /// The New agent task form went away while its task was starting.
    CancelAgentTask(RequestId),
    RemoveWorktree {
        session: String,
        force: bool,
    },
    DiscoverRepos,
    AgentNames,
    /// Restart the host's herdr server (the banner's button).
    RestartServer,
    /// The banner's button, whatever it offers.
    NoticeAction,
    /// The server's focus once the commands queued so far have landed.
    ConfirmFocus {
        budget_ms: u64,
    },
    /// Bytes for one pane, wherever the frame channel is (a notification's
    /// reply): switch there first, then send.
    DeliverInput {
        pane: String,
        bytes: Vec<u8>,
    },
    /// The same, as a bracketed paste.
    DeliverPaste {
        pane: String,
        text: String,
    },
    Scroll {
        pane: String,
        lines: i64,
        source: ScrollSource,
    },
    Mouse {
        pane: String,
        action: MouseAction,
        button: MouseButton,
        column: i64,
        row: i64,
        modifiers: u8,
    },
}

/// Why an exec was run.
#[derive(Debug, Clone)]
enum Purpose {
    Poll,
    /// A command-lane step's command.
    Lane {
        step: u64,
    },
    /// The write lane's key run.
    Key,
    Status(StatusFor),
    PaneRepos(u64),
    HomeScan(u64),
    Agents(u64),
    ServerStop,
    Daemon,
}

#[derive(Debug, Clone)]
enum StatusFor {
    /// A protocol mismatch, before it is reported.
    Skew,
    /// A hang-up mid-handshake: this notice, in versions.
    Explain(String),
    /// A task's first message: which command the server takes.
    Task(u64),
    /// A restart: would it downgrade the server?
    Restart,
}

/// A poll on its way.
#[derive(Debug, Clone, Copy)]
struct PollRead {
    exec: u64,
    /// The lane's epoch when the read went out.
    epoch: u64,
    /// No mutation was queued when it went out.
    quiet: bool,
}

/// Someone waiting for the next poll to finish (the restart).
#[derive(Debug, Clone, Copy)]
struct PollWaiter {
    /// The poll count that has to be reached.
    after: u64,
    then: AfterPoll,
}

#[derive(Debug, Clone, Copy)]
enum AfterPoll {
    Bootstrap,
}

/// The command lane: `HerdrControlClient.commandLane` with
/// `commandsQueued` and `commandEpoch`.
#[derive(Debug, Default)]
struct Lane {
    steps: VecDeque<Step>,
    running: Option<Running>,
    /// Mutations queued and not yet let go (`commandsQueued`).
    queued: u32,
    /// Bumped as each mutation is queued (`commandEpoch`).
    epoch: u64,
    next_id: u64,
}

#[derive(Debug)]
struct Step {
    id: u64,
    mutation: bool,
    body: Body,
    landed: Option<Landed>,
}

#[derive(Debug)]
enum Body {
    /// herdr subcommands run in order, stopping at the first that fails;
    /// nobody waits for the answers (`send`, `sendSequence`).
    Commands {
        subcommands: Vec<String>,
        next: usize,
    },
    /// One command whose answer its owner waits for (`runOnLane`); the
    /// owner is in `Client::owned`.
    Owned { command: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    TaskCreate(u64),
    TaskLaunch(u64),
    TaskPrompt(u64),
    TaskSubmit(u64),
    Removal(u64),
    ConfirmFocus(u64),
    Bootstrap,
}

#[derive(Debug, Clone, Copy)]
enum Landed {
    /// Lowers `pane_switch_in_flight` if this is still the newest switch.
    PaneSwitch(u64),
}

#[derive(Debug)]
struct Running {
    step: Step,
    exec: Option<u64>,
}

/// An owned step's owner, waiting.
#[derive(Debug, Clone, Copy)]
struct OwnedWait {
    owner: Owner,
    exec: Option<u64>,
    /// The lane has let go of the step (answered, or its hold ran out).
    released: bool,
}

/// The herdr control client. See the module docs for the contract.
///
/// # Driving it (the session's herdr leg)
///
/// `crates/session`'s herdr leg (`crates/session/src/herdr.rs`, W10b) owns
/// a [`Client`] on one task and maps the session onto it, the way its tmux
/// leg drives the tmux controller:
///
/// | session                                   | client |
/// |-------------------------------------------|--------|
/// | the leg starts (SSH, a terminal channel)  | [`Client::start`] |
/// | the terminal channel's reads, its end     | [`Client::frame_bytes`], [`Client::frame_closed`] |
/// | keys typed at a pane's byte channel       | [`Client::input`] |
/// | `Sessions::paste`                         | [`Client::paste`] |
/// | a pane view's size                        | [`Client::resize`] |
/// | `Sessions::herdr` (the UI's intents)      | [`Client::act`] (its [`RequestId`] answered under the session's token) |
/// | `Sessions::deliver` (replies, shares)     | [`Action::DeliverInput`], [`Action::DeliverPaste`] |
/// | the app's foreground and background       | [`Client::foreground`], [`Client::background`] |
/// | the teardown (`courtesy` on a disconnect) | [`Client::close`], then what it queued is written |
/// | timers                                    | [`Client::next_deadline`], [`Client::tick`] |
///
/// and carries out the outputs in order: [`Output::Exec`] is `ssh.run` on
/// a task of its own (at most four at once), answered with the merged
/// output ([`ExecResult::Output`]) or the failure to start it, and
/// cancelled on [`Output::Abandon`]; an [`Output::FrameWrite`] goes to the
/// terminal channel's writer (one write at a time, in order, before the
/// next output); [`Output::OpenPush`] is a second terminal channel (80×24,
/// the pump its program) on the same connection, fed back through
/// [`Client::push_opened`], [`Client::push_bytes`] and
/// [`Client::push_closed`]; [`Output::StreamOpened`] mints the pane's byte
/// channel, [`Output::PaneBytes`] is a push into it and [`Output::View`] a
/// mark in it; [`Event::FirstFrame`] goes to the session after the panes
/// it painted ([`Client::panes`], read after each batch);
/// [`Event::FrameChannelLost`] is a lost transport, which the session
/// redials.
#[derive(Debug)]
pub struct Client {
    config: Config,
    now: u64,
    outputs: VecDeque<Output>,
    timers: Timers,
    dirty: bool,
    started: bool,
    closed: bool,
    next_exec: u64,
    execs: BTreeMap<u64, Purpose>,
    next_request: u64,

    // Reading the tree.
    poll: PollState,
    polling: Option<PollRead>,
    poll_again: bool,
    polls_done: u64,
    poll_waiters: Vec<PollWaiter>,
    refreshing: bool,
    /// The server's version, from the snapshot or `herdr status`.
    server_version: Option<String>,
    /// A skew diagnosis is out; the mismatch it holds back.
    diagnosing: bool,
    held_mismatch: Option<String>,
    last_inbound: Option<u64>,
    /// A read was applied since the restart (or bootstrap) began: what the
    /// tree says is the new server's.
    heard_since_restart: bool,
    /// The focused pane as the server last said it (a read applied), not
    /// as a pick here already put it in the snapshot (B406).
    server_focus: Option<String>,

    // The command lane.
    lane: Lane,
    owned: BTreeMap<u64, OwnedWait>,
    pane_switch_generation: u64,
    pane_switch_in_flight: bool,
    /// The newest pick's focus was asked for again after it couldn't run
    /// (B470): once a pick.
    pick_asked_again: bool,

    // What the person asked for and waits on.
    tasks: BTreeMap<u64, task::AgentTask>,
    removals: BTreeSet<u64>,
    discoveries: BTreeMap<u64, task::Discovery>,
    agent_lists: BTreeSet<u64>,
    restart: Option<task::Restart>,
    confirms: BTreeSet<u64>,

    // The frame channel.
    attach: attach::Attach,
    streams: BTreeMap<String, StreamId>,
    next_stream: StreamId,
    first_frame: bool,

    // The push channel.
    push: push::Push,

    notice: Option<(Notice, Option<NoticeAction>)>,
}

impl Client {
    pub fn new(config: Config) -> Self {
        let mut poll = PollState::new();
        poll.immersive_zoom = config.immersive_zoom;
        Self {
            config,
            now: 0,
            outputs: VecDeque::new(),
            timers: Timers::default(),
            dirty: false,
            started: false,
            closed: false,
            next_exec: 0,
            execs: BTreeMap::new(),
            next_request: 0,
            poll,
            polling: None,
            poll_again: false,
            polls_done: 0,
            poll_waiters: Vec::new(),
            refreshing: false,
            server_version: None,
            diagnosing: false,
            held_mismatch: None,
            last_inbound: None,
            heard_since_restart: false,
            server_focus: None,
            lane: Lane::default(),
            owned: BTreeMap::new(),
            pane_switch_generation: 0,
            pane_switch_in_flight: false,
            pick_asked_again: false,
            tasks: BTreeMap::new(),
            removals: BTreeSet::new(),
            discoveries: BTreeMap::new(),
            agent_lists: BTreeSet::new(),
            restart: None,
            confirms: BTreeSet::new(),
            attach: attach::Attach::default(),
            streams: BTreeMap::new(),
            next_stream: 1,
            first_frame: false,
            push: push::Push::default(),
            notice: None,
        }
    }

    // MARK: - Reading the state

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The tree as last read (with a pane picked here already in it).
    pub fn snapshot(&self) -> &Snapshot {
        &self.poll.snapshot
    }

    /// The focused pane the server itself last reported: a pick made here
    /// is in [`Self::snapshot`] at once, and here only once a read says the
    /// server is on it (B406).
    pub fn server_focus(&self) -> Option<&str> {
        self.server_focus.as_deref()
    }

    pub fn agent_hooks(&self) -> &BTreeMap<String, AgentHook> {
        &self.poll.agent_hooks
    }

    /// Whether [`agent_hooks`](Self::agent_hooks) has been read.
    pub fn agent_hooks_loaded(&self) -> bool {
        self.poll.agent_hooks_loaded
    }

    /// The host has herdr but no server: the empty state.
    pub fn server_not_running(&self) -> bool {
        self.poll.server_not_running
    }

    /// The server refuses this client's protocol (the message).
    pub fn protocol_mismatch(&self) -> Option<&str> {
        self.poll.protocol_mismatch.as_deref()
    }

    /// Which side is behind, while the two disagree.
    pub fn version_skew(&self) -> Option<&VersionReport> {
        self.poll.version_skew.as_ref()
    }

    /// The server's version, once a snapshot or `herdr status` named it.
    pub fn server_version(&self) -> Option<&str> {
        self.server_version.as_deref()
    }

    /// The banner and its button.
    pub fn notice(&self) -> Option<(&Notice, Option<NoticeAction>)> {
        self.notice
            .as_ref()
            .map(|(notice, action)| (notice, *action))
    }

    /// A user-started refresh is out (the spinner).
    pub fn is_refreshing(&self) -> bool {
        self.refreshing
    }

    /// A restart is under way (the Home row's spinner).
    pub fn is_restarting(&self) -> bool {
        self.restart.is_some()
    }

    /// The interval the next scheduled read waits.
    pub fn poll_interval_ms(&self) -> u64 {
        self.poll.interval.as_millis() as u64
    }

    /// A pane switch that may not have reached the server yet
    /// (`paneSwitchInFlight`): until its command landed or the lane stopped
    /// waiting for it.
    pub fn pane_switch_in_flight(&self) -> bool {
        self.pane_switch_in_flight
    }

    /// The push channel carries events.
    pub fn push_active(&self) -> bool {
        self.push.active()
    }

    /// When the host last answered anything: an exec's output or a frame
    /// channel byte. The session's keepalive reads this instead of spending
    /// an exec of its own.
    pub fn last_inbound_at(&self) -> Option<u64> {
        self.last_inbound
    }

    /// Where the pane's program runs (`workingDirectory(ofPane:)`).
    pub fn working_directory(&self, pane: &str) -> Option<&str> {
        self.poll.pane_cwds.get(pane).map(String::as_str)
    }

    /// The repository a worktree workspace came from (`worktreeRepo`).
    pub fn worktree_repo(&self, session: &str) -> Option<&str> {
        self.poll.worktree_repos.get(session).map(String::as_str)
    }

    /// herdr's terminal id per pane (the mosh raw-attach renderer's target).
    pub fn terminal_ids(&self) -> &BTreeMap<String, String> {
        &self.poll.terminal_ids
    }

    /// The panes' directories, sorted, each once: what the New agent task
    /// form offers before the repositories are resolved.
    pub fn open_repo_guesses(&self) -> Vec<String> {
        let cwds: Vec<&str> = self.poll.pane_cwds.values().map(String::as_str).collect();
        control::open_repo_guesses(&cwds)
    }

    /// The pane terminals, by pane id.
    pub fn streams(&self) -> &BTreeMap<String, StreamId> {
        &self.streams
    }

    /// The panes with terminals, and whether each is the server's focus:
    /// what `HerdrNotes::panes` reports.
    pub fn panes(&self) -> Vec<(String, StreamId, bool)> {
        let active = self.poll.snapshot.active_pane_id.as_deref();
        self.streams
            .iter()
            .map(|(pane, &stream)| (pane.clone(), stream, active == Some(pane.as_str())))
            .collect()
    }

    /// The pane the frame channel is pointed at (`herdrFrameTarget`).
    pub fn frame_target(&self) -> Option<&str> {
        self.attach.target.as_deref()
    }

    /// The pane whose attach is reading the frame channel now, and takes
    /// input.
    pub fn attached_pane(&self) -> Option<&str> {
        self.attach.input_pane()
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
        self.begin(now);
        self.end();
    }

    fn begin(&mut self, now: u64) {
        while let Some((deadline, slot)) = self.timers.pop_due(now) {
            self.now = deadline.max(self.now);
            self.fire(slot);
        }
        self.now = now.max(self.now);
    }

    fn end(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.event(Event::StateChanged);
        }
    }

    fn event(&mut self, event: Event) {
        self.outputs.push_back(Output::Event(event));
    }

    fn arm(&mut self, slot: Slot, after: u64) {
        self.timers.arm(slot, self.now + after);
    }

    fn request_id(&mut self) -> RequestId {
        self.next_request += 1;
        RequestId(self.next_request)
    }

    fn fire(&mut self, slot: Slot) {
        match slot {
            Slot::Poll => self.poll_now(),
            Slot::Exec(id) => self.exec_timed_out(id),
            Slot::LaneHold(step) => self.lane_hold_expired(step),
            Slot::Patience(step) => self.patience_expired(step),
            Slot::KeyHold => self.key_hold_expired(),
            Slot::ReleaseAck => self.release_ack_timed_out(),
            Slot::Settle => self.settled(),
            Slot::BootProbe => self.boot_probe_fired(),
            Slot::AttachWatch => self.attach_watch_fired(),
            Slot::ExpectedClose(id) => self.expected_close_forgotten(id),
            Slot::HeldInput => self.held_input_expired(),
            Slot::Task(id) => self.task_timer(id),
            Slot::Restart => self.restart_timer(),
            Slot::PumpReady => self.pump_ready_timed_out(),
            Slot::Subscribe => self.subscribe_timed_out(),
            Slot::Debounce => self.debounce_fired(),
            Slot::PushRetry => self.push_retry(),
        }
    }

    // MARK: - Execs

    /// The command line for a herdr subcommand, with the exit-status trailer.
    fn herdr(&self, subcommand: &str) -> String {
        control::command_line(subcommand, self.config.custom_path.as_deref())
    }

    /// The command line for a herdr subcommand, without the trailer.
    fn herdr_bare(&self, subcommand: &str) -> String {
        launch::command(subcommand, self.config.custom_path.as_deref())
    }

    /// Ask for an exec. `timeout`: when nobody waits for it any more
    /// (`None` for the lanes, whose holds and patience bound theirs).
    fn exec(&mut self, command: String, purpose: Purpose, timeout: Option<u64>) -> u64 {
        self.next_exec += 1;
        let id = self.next_exec;
        self.execs.insert(id, purpose);
        if let Some(timeout) = timeout {
            self.arm(Slot::Exec(id), timeout);
        }
        self.outputs.push_back(Output::Exec {
            id: ExecId(id),
            command,
        });
        id
    }

    /// Stop waiting for an exec that is still out.
    fn abandon(&mut self, id: u64) {
        if self.execs.remove(&id).is_some() {
            self.timers.cancel(&Slot::Exec(id));
            self.outputs.push_back(Output::Abandon { id: ExecId(id) });
        }
    }

    /// An exec the client asked for came back.
    pub fn exec_finished(&mut self, id: ExecId, result: ExecResult, now: u64) {
        self.begin(now);
        if let ExecResult::Output(_) = result {
            self.last_inbound = Some(self.now);
        }
        if let Some(purpose) = self.execs.remove(&id.0) {
            self.timers.cancel(&Slot::Exec(id.0));
            self.exec_answered(id.0, purpose, Some(result));
        }
        self.end();
    }

    fn exec_timed_out(&mut self, id: u64) {
        if let Some(purpose) = self.execs.remove(&id) {
            self.outputs.push_back(Output::Abandon { id: ExecId(id) });
            self.exec_answered(id, purpose, None);
        }
    }

    /// `result`: what came back, `None` for no answer in time.
    fn exec_answered(&mut self, id: u64, purpose: Purpose, result: Option<ExecResult>) {
        match purpose {
            Purpose::Poll => self.poll_answered(id, result),
            Purpose::Lane { step } => self.lane_answered(step, result),
            Purpose::Key => self.key_answered(),
            Purpose::Status(why) => {
                let report = match result {
                    Some(ExecResult::Output(bytes)) => {
                        VersionReport::parse(&read_result(&bytes).output)
                    }
                    _ => None,
                };
                if let Some(report) = &report {
                    self.server_version = Some(report.server.clone());
                }
                self.status_answered(why, report);
            }
            Purpose::PaneRepos(request) => self.pane_repos_answered(request, result),
            Purpose::HomeScan(request) => self.home_scan_answered(request, result),
            Purpose::Agents(request) => self.agents_answered(request, result),
            Purpose::ServerStop => self.server_stop_answered(),
            Purpose::Daemon => self.daemon_answered(),
        }
    }

    // MARK: - Lifecycle

    /// Start: the first read now, then the cadence; the push channel if
    /// configured. The frame channel stays quiet until the first read names
    /// a focused pane.
    pub fn start(&mut self, now: u64) {
        self.begin(now);
        if !self.started && !self.closed {
            self.started = true;
            self.attach.open = self.config.frame_channel;
            if self.attach.open && self.config.frame_loop {
                // The loop runs from the start; it says so, or this says
                // the channel never came up.
                self.attach.booted = true;
                self.arm(Slot::BootProbe, self.config.budgets.loop_probe);
            }
            self.poll_now();
            self.start_push();
        }
        self.end();
    }

    /// Back in the foreground: read now at the fast cadence.
    pub fn foreground(&mut self, now: u64) {
        self.begin(now);
        // What was backed off from is asked again, not after the rest of
        // the wait (B490).
        self.retry_now();
        self.quicken();
        self.end();
    }

    /// How long the backoff after a refusal is, while one runs: the
    /// banner counts it down (B490).
    pub fn retry_wait(&self) -> Option<u64> {
        self.attach.retry_after.and(self.attach.retry_wait)
    }

    /// Read the tree now, at the fast cadence, without the refresh
    /// control's spinner (`quicken`): what mosh's raw-attach renderer asks
    /// for when a pane closed or herdr restarted under it.
    pub fn read_now(&mut self, now: u64) {
        self.begin(now);
        self.quicken();
        self.end();
    }

    /// Into the background: nothing to hand back (herdr's focus and sizes
    /// are the server's), so nothing to do.
    pub fn background(&mut self, now: u64) {
        self.begin(now);
        self.end();
    }

    /// Stop. `courtesy`: release the attach first (a full disconnect).
    /// Everything still waited on gets its answer now; nothing more is asked
    /// of the host.
    pub fn close(&mut self, courtesy: bool, now: u64) {
        self.begin(now);
        if self.closed {
            self.end();
            return;
        }
        if courtesy && self.attach.open && self.attach.attached.is_some() {
            self.outputs
                .push_back(Output::FrameWrite(crate::frame::RELEASE.to_owned()));
        }
        self.closed = true;
        self.shut_down();
        self.end();
    }

    /// Answer everything waited on, abandon every exec, stop every timer.
    fn shut_down(&mut self) {
        for (id, task) in std::mem::take(&mut self.tasks) {
            self.event(Event::Task {
                request: RequestId(id),
                outcome: task.interrupted(),
            });
        }
        for id in std::mem::take(&mut self.removals) {
            self.event(Event::Removal {
                request: RequestId(id),
                outcome: WorktreeRemoval::Failed(control::RemovalFailure::Unreachable),
            });
        }
        for (id, discovery) in std::mem::take(&mut self.discoveries) {
            self.event(Event::Repos {
                request: RequestId(id),
                repos: discovery.so_far(),
            });
        }
        for id in std::mem::take(&mut self.agent_lists) {
            self.event(Event::Agents {
                request: RequestId(id),
                names: Vec::new(),
                answered: false,
            });
        }
        if let Some(request) = self.restart.take().and_then(|restart| restart.request) {
            self.event(Event::Restart {
                request,
                outcome: RestartOutcome::Failed,
            });
        }
        for id in std::mem::take(&mut self.confirms) {
            self.event(Event::FocusConfirmed {
                request: RequestId(id),
                pane: None,
            });
        }
        self.drop_all_held(false);
        self.push_shut_down();
        let ids: Vec<u64> = self.execs.keys().copied().collect();
        for id in ids {
            self.abandon(id);
        }
        self.lane = Lane::default();
        self.owned.clear();
        self.polling = None;
        self.poll_waiters.clear();
        self.timers = Timers::default();
        self.attach.open = false;
    }

    // MARK: - Reading the tree

    /// Read now; a read already out is followed by another as soon as it
    /// is back (it may have left before whatever asked for this).
    fn poll_now(&mut self) {
        if !self.started || self.closed {
            return;
        }
        if self.polling.is_some() {
            self.poll_again = true;
            return;
        }
        self.timers.cancel(&Slot::Poll);
        let command = self.herdr(control::SNAPSHOT);
        let exec = self.exec(command, Purpose::Poll, Some(self.config.budgets.poll));
        self.polling = Some(PollRead {
            exec,
            epoch: self.lane.epoch,
            quiet: self.lane.queued == 0,
        });
    }

    /// Back to the fast cadence, and read now (`quicken`).
    fn quicken(&mut self) {
        self.poll.quicken();
        self.poll_now();
    }

    fn poll_answered(&mut self, exec: u64, result: Option<ExecResult>) {
        let Some(read) = self.polling.filter(|read| read.exec == exec) else {
            return;
        };
        self.polling = None;
        // A command was queued, or still on its way, while this read was
        // out: the server may have answered before it landed, and the focus
        // it reports is the one the person just left. The re-read after the
        // last command follows; whoever waits for a read waits for that one.
        let current = read.quiet && read.epoch == self.lane.epoch;
        if current || result.is_none() {
            self.polls_done += 1;
        }
        if current && let Some(text) = result.as_ref().and_then(ExecResult::text) {
            self.heard_since_restart = true;
            let answer = read_result(text.as_bytes());
            let reading = control::read_poll(&answer);
            let tree = matches!(reading, PollReading::Tree(_));
            if matches!(reading, PollReading::NoServer) {
                self.server_gone_ends_hold();
            }
            let server_was_down = self.poll.server_not_running;
            if tree {
                // What the latest tree says, or unknown: a server that
                // stopped naming its version was replaced.
                let version = snapshot_version(&answer.output);
                if self.server_version.is_some()
                    && version.is_some()
                    && version != self.server_version
                {
                    // Another server answers (upgraded on the host): what
                    // the old one refused is asked of this one now (B490).
                    self.retry_now();
                }
                self.server_version = version;
            }
            let effects = self.poll.apply(reading);
            self.dirty = true;
            if tree {
                self.server_focus = self.poll.snapshot.active_pane_id.clone();
                self.prune_streams();
                if server_was_down {
                    self.revive_push();
                }
                if !self.config.frame_channel {
                    self.server_answers_again();
                }
            }
            for effect in effects {
                self.poll_effect(effect);
            }
        }
        if self.refreshing {
            self.refreshing = false;
            self.dirty = true;
        }
        let done = self.polls_done;
        let (due, waiting): (Vec<PollWaiter>, Vec<PollWaiter>) =
            std::mem::take(&mut self.poll_waiters)
                .into_iter()
                .partition(|waiter| waiter.after <= done);
        self.poll_waiters = waiting;
        if std::mem::take(&mut self.poll_again) {
            self.poll_now();
        } else {
            let interval = self.poll_interval_ms();
            self.arm(Slot::Poll, interval);
        }
        for waiter in due {
            match waiter.then {
                AfterPoll::Bootstrap => self.bootstrap_read(),
            }
        }
    }

    /// Wait for the next read that leaves from now on and counts (not one
    /// dropped for a command queued across it), reading now.
    fn after_next_poll(&mut self, then: AfterPoll) {
        let after = self.polls_done + if self.polling.is_some() { 2 } else { 1 };
        self.poll_waiters.push(PollWaiter { after, then });
        self.poll_now();
    }

    fn poll_effect(&mut self, effect: PollEffect) {
        match effect {
            PollEffect::DiagnoseSkew => {
                if !self.diagnosing {
                    self.diagnosing = true;
                    let command = self.herdr(control::STATUS);
                    self.exec(
                        command,
                        Purpose::Status(StatusFor::Skew),
                        Some(self.config.budgets.status),
                    );
                }
            }
            PollEffect::Mismatch(message) => {
                if self.diagnosing {
                    // Named once the versions are in (or their read gave up):
                    // which side is older decides the remedy.
                    self.held_mismatch = Some(message);
                } else {
                    self.offer_restart(message);
                }
            }
            PollEffect::FocusedPane(pane) => {
                self.event(Event::FocusedPane(pane.clone()));
                self.retarget(&pane, false);
            }
            PollEffect::HooksUpdated => self.event(Event::AgentHooksUpdated),
            PollEffect::Zoom(pane) => self.send(&control::immersive_zoom(&pane), None),
        }
    }

    fn status_answered(&mut self, why: StatusFor, report: Option<VersionReport>) {
        match why {
            StatusFor::Skew => {
                self.diagnosing = false;
                self.poll.note_versions(report);
                self.dirty = true;
                if let Some(message) = self.held_mismatch.take() {
                    self.offer_restart(message);
                }
            }
            StatusFor::Explain(message) => {
                self.poll.note_versions(report);
                if let Some(skew) = self.poll.version_skew.clone()
                    && self
                        .notice
                        .as_ref()
                        .is_some_and(|(notice, _)| *notice == Notice::Refused(message.clone()))
                {
                    self.show_skew(&skew);
                }
            }
            StatusFor::Task(id) => self.task_versions(id, report),
            StatusFor::Restart => self.restart_versions(report),
        }
    }

    /// Put the skew up with its remedy (`offerHerdrServerRestart`). Never
    /// runs it: `server stop` exits pane processes.
    fn offer_restart(&mut self, message: String) {
        if let Some(skew) = self.poll.version_skew.clone() {
            self.show_skew(&skew);
            return;
        }
        let action = (!snapshot::client_is_older(&message)).then_some(NoticeAction::RestartServer);
        self.set_notice(Notice::Mismatch(message), action);
    }

    /// The skew in versions: restart offered only when it is an upgrade.
    fn show_skew(&mut self, report: &VersionReport) {
        let action = (!report.client_is_older()).then_some(NoticeAction::RestartServer);
        self.set_notice(Notice::Skew(report.notice()), action);
    }

    fn set_notice(&mut self, notice: Notice, action: Option<NoticeAction>) {
        let next = Some((notice, action));
        if self.notice != next {
            self.notice = next;
            self.dirty = true;
        }
    }

    fn clear_notice(&mut self) {
        if self.notice.take().is_some() {
            self.dirty = true;
        }
    }

    /// A read came back a tree: the server answers this client, so a skew,
    /// a mismatch or a failed restart said earlier is over (B408). Over SSH
    /// the next painted frame says so; without a frame channel (mosh, whose
    /// screen is the raw attach's) nothing else would, and the banner's
    /// Restart would stop a healthy server, exiting every pane's process.
    fn server_answers_again(&mut self) {
        self.held_mismatch = None;
        if self.restart.is_none()
            && self.notice.as_ref().is_some_and(|(notice, _)| {
                matches!(
                    notice,
                    Notice::Mismatch(_) | Notice::Skew(_) | Notice::RestartFailed
                )
            })
        {
            self.clear_notice();
        }
    }

    // MARK: - The command lane

    /// Queue a step. A mutation bumps the epoch (a read out across it is
    /// dropped) and counts toward the re-read that follows the last one.
    fn enqueue(&mut self, body: Body, mutation: bool, landed: Option<Landed>) -> u64 {
        self.lane.next_id += 1;
        let id = self.lane.next_id;
        if mutation {
            self.lane.epoch += 1;
            self.lane.queued += 1;
        }
        self.lane.steps.push_back(Step {
            id,
            mutation,
            body,
            landed,
        });
        self.lane_advance();
        id
    }

    /// A mutation at the head of the queue: it runs next, ahead of what
    /// waits (B470's focus asked again, in its failed step's place).
    fn enqueue_front(&mut self, body: Body, landed: Option<Landed>) {
        self.lane.next_id += 1;
        let id = self.lane.next_id;
        self.lane.epoch += 1;
        self.lane.queued += 1;
        self.lane.steps.push_front(Step {
            id,
            mutation: true,
            body,
            landed,
        });
        self.lane_advance();
    }

    /// Queue a command whose answer `owner` waits for, at most `patience`
    /// from now (the queue wait included, as `WriteLane.wait` counted it).
    fn enqueue_owned(
        &mut self,
        command: String,
        owner: Owner,
        mutation: bool,
        patience: u64,
    ) -> u64 {
        // Armed before the step can start: a step that starts (and is
        // answered) at once must find its owner waiting.
        let id = self.lane.next_id + 1;
        self.owned.insert(
            id,
            OwnedWait {
                owner,
                exec: None,
                released: false,
            },
        );
        self.arm(Slot::Patience(id), patience);
        let queued = self.enqueue(Body::Owned { command }, mutation, None);
        debug_assert_eq!(queued, id);
        id
    }

    fn lane_advance(&mut self) {
        while self.lane.running.is_none() {
            let Some(mut step) = self.lane.steps.pop_front() else {
                return;
            };
            self.arm(Slot::LaneHold(step.id), self.config.budgets.command_hold);
            let command = match &mut step.body {
                Body::Commands { subcommands, next } => {
                    let subcommand = subcommands[*next].clone();
                    *next += 1;
                    self.herdr(&subcommand)
                }
                Body::Owned { command, .. } => command.clone(),
            };
            let exec = self.exec(command, Purpose::Lane { step: step.id }, None);
            if let Some(wait) = self.owned.get_mut(&step.id) {
                wait.exec = Some(exec);
            }
            self.lane.running = Some(Running {
                step,
                exec: Some(exec),
            });
        }
    }

    fn lane_answered(&mut self, step_id: u64, result: Option<ExecResult>) {
        let answer = match result {
            Some(ExecResult::Output(bytes)) => Answer::Printed(read_result(&bytes)),
            Some(ExecResult::Failed(_)) => Answer::Unreachable,
            None => Answer::Unanswered,
        };
        // An owned step's answer goes to its owner, whether or not the lane
        // still holds the step.
        if let Some(wait) = self.owned.remove(&step_id) {
            self.timers.cancel(&Slot::Patience(step_id));
            self.owner_answered(wait.owner, answer.clone());
        }
        let Some(running) = self.lane.running.as_mut() else {
            return;
        };
        if running.step.id != step_id {
            return;
        }
        running.exec = None;
        let follow = match &mut running.step.body {
            Body::Commands { subcommands, next } => {
                let printed = match &answer {
                    Answer::Printed(result) => Some(result),
                    _ => None,
                };
                // B121: the next command goes only once this one is known
                // done; a lost trailer stops the sequence.
                match control::refusal(printed) {
                    Some(reason) => Err((subcommands[*next - 1].clone(), reason)),
                    None if *next < subcommands.len() => {
                        *next += 1;
                        Ok(Some(subcommands[*next - 1].clone()))
                    }
                    None => Ok(None),
                }
            }
            Body::Owned { .. } => Ok(None),
        };
        match follow {
            Err((command, reason)) => {
                let again = self.pick_to_ask_again(&reason);
                self.event(Event::CommandFailed { command, reason });
                let again = again.map(|(subcommands, landed)| {
                    // In the failed step's place, ahead of what was queued
                    // behind it (a reply's focus check), and queued before
                    // the step lets go, so the lane's re-read doesn't run
                    // between the two and report the focus where the person
                    // left it (B470).
                    self.enqueue_front(
                        Body::Commands {
                            subcommands,
                            next: 0,
                        },
                        Some(landed),
                    );
                });
                self.lane_release();
                if again.is_some() {
                    // Still on its way: the step let go lowered it.
                    self.pane_switch_in_flight = true;
                }
            }
            Ok(Some(subcommand)) => {
                let command = self.herdr(&subcommand);
                let exec = self.exec(command, Purpose::Lane { step: step_id }, None);
                if let Some(running) = self.lane.running.as_mut() {
                    running.exec = Some(exec);
                }
            }
            Ok(None) => self.lane_release(),
        }
    }

    /// The running step's hold ran out: the lane moves on. A sequence sends
    /// nothing more (the rest would land among the steps after it); an
    /// owned command's owner keeps waiting until its patience ends.
    fn lane_hold_expired(&mut self, step_id: u64) {
        let Some(running) = self.lane.running.as_ref() else {
            return;
        };
        if running.step.id != step_id {
            return;
        }
        if let Some(exec) = running.exec
            && !self.owned.contains_key(&step_id)
        {
            self.abandon(exec);
        }
        self.lane_release();
    }

    /// The running step moved herdr's focus for the newest pick and couldn't
    /// run (the control connection dropped under it): what to send once more
    /// (B470). Left alone, the next read reported the focus where it was
    /// and pulled the screen back, silently. Over mosh the command waits for
    /// the next connection. Once a pick.
    fn pick_to_ask_again(&mut self, reason: &control::Refusal) -> Option<(Vec<String>, Landed)> {
        if !matches!(reason, control::Refusal::Unreachable) || self.pick_asked_again {
            return None;
        }
        let running = self.lane.running.as_ref()?;
        let landed = running.step.landed?;
        let Landed::PaneSwitch(generation) = landed;
        if generation != self.pane_switch_generation {
            return None;
        }
        let Body::Commands { subcommands, .. } = &running.step.body else {
            return None;
        };
        self.pick_asked_again = true;
        log::warn!("herdr: a pane switch's focus couldn't run: asked once more");
        Some((subcommands.clone(), landed))
    }

    /// The lane is done with its running step.
    fn lane_release(&mut self) {
        let Some(running) = self.lane.running.take() else {
            return;
        };
        let step = running.step;
        self.timers.cancel(&Slot::LaneHold(step.id));
        if let Some(wait) = self.owned.get_mut(&step.id) {
            wait.released = true;
        }
        if let Some(Landed::PaneSwitch(generation)) = step.landed
            && generation == self.pane_switch_generation
            && self.pane_switch_in_flight
        {
            self.pane_switch_in_flight = false;
            self.dirty = true;
        }
        if step.mutation {
            self.lane.queued = self.lane.queued.saturating_sub(1);
            if self.lane.queued == 0 {
                // The re-read for the UI, once nothing else is queued: a read
                // between two commands reports what the next is about to
                // change.
                self.poll_now();
            }
        }
        self.lane_advance();
    }

    /// An owned step's owner stopped waiting.
    fn patience_expired(&mut self, step_id: u64) {
        let Some(wait) = self.owned.remove(&step_id) else {
            return;
        };
        if let Some(index) = self.lane.steps.iter().position(|step| step.id == step_id) {
            // Never sent: nobody wants its answer now, so it isn't sent at all
            // (the app still ran it when its turn came — a worktree made after
            // the form had said none was).
            if let Some(step) = self.lane.steps.remove(index)
                && step.mutation
            {
                self.lane.queued = self.lane.queued.saturating_sub(1);
                if self.lane.queued == 0 {
                    self.poll_now();
                }
            }
        } else if wait.released
            && let Some(exec) = wait.exec
        {
            self.abandon(exec);
        }
        self.owner_answered(wait.owner, Answer::Unanswered);
    }

    /// An owned command's answer.
    fn owner_answered(&mut self, owner: Owner, answer: Answer) {
        match owner {
            Owner::TaskCreate(id) => self.task_created(id, answer),
            Owner::TaskLaunch(id) => self.task_launched(id, answer),
            Owner::TaskPrompt(id) => self.task_prompted(id, answer),
            Owner::TaskSubmit(id) => self.task_submitted(id, answer),
            Owner::Removal(id) => self.removal_answered(id, answer),
            Owner::ConfirmFocus(id) => {
                if self.confirms.remove(&id) {
                    let pane = match answer {
                        Answer::Printed(result) => snapshot::decode(&result.output)
                            .and_then(|decoded| decoded.snapshot.active_pane_id),
                        _ => None,
                    };
                    self.event(Event::FocusConfirmed {
                        request: RequestId(id),
                        pane,
                    });
                }
            }
            Owner::Bootstrap => self.bootstrap_created(),
        }
    }

    /// One command on the lane, fire and forget (`send`).
    fn send(&mut self, subcommand: &str, landed: Option<Landed>) {
        self.send_sequence(&[subcommand.to_owned()], landed);
    }

    /// Commands that must run in this order, as one step (`sendSequence`).
    fn send_sequence(&mut self, subcommands: &[String], landed: Option<Landed>) {
        if subcommands.is_empty() {
            return;
        }
        self.enqueue(
            Body::Commands {
                subcommands: subcommands.to_vec(),
                next: 0,
            },
            true,
            landed,
        );
    }

    // MARK: - Intents

    /// Do what the person asked. The ones with an answer return its id; the
    /// answer comes as an [`Event`].
    pub fn act(&mut self, action: Action, now: u64) -> Option<RequestId> {
        self.begin(now);
        let request = if self.closed {
            None
        } else {
            self.dispatch(action)
        };
        self.end();
        request
    }

    fn dispatch(&mut self, action: Action) -> Option<RequestId> {
        match action {
            Action::SelectSession(session) => {
                if self.config.immersive_zoom {
                    self.poll.zoom_after_next_tree();
                }
                self.send(&control::select_session(&session), None);
            }
            Action::SelectWindow(window) => {
                if self.config.immersive_zoom {
                    self.poll.zoom_after_next_tree();
                }
                self.send(&control::select_window(&window), None);
            }
            Action::SelectPane(pane) => self.select_pane(&pane),
            Action::NewSession { name } => self.new_session(name.as_deref()),
            Action::NewWindow { name } => self.send(&control::new_window(name.as_deref()), None),
            Action::NewPane => {
                if let Some(command) = control::new_pane(&self.poll.snapshot) {
                    self.send(&command, None);
                }
            }
            Action::NewWindowInSession { session, name } => {
                let steps =
                    control::new_window_in_session(&self.poll.snapshot, &session, name.as_deref());
                self.send_sequence(&steps, None);
            }
            Action::NewPaneInWindow(window) => {
                if let Some(steps) = control::new_pane_in_window(&self.poll.snapshot, &window) {
                    self.send_sequence(&steps, None);
                }
            }
            Action::RenameSession { session, name } => {
                self.send(&control::rename_session(&session, &name), None)
            }
            Action::KillSession(session) => self.send(&control::kill_session(&session), None),
            Action::RenameWindow { window, name } => {
                self.send(&control::rename_window(&window, &name), None)
            }
            Action::KillWindow(window) => self.send(&control::kill_window(&window), None),
            Action::KillPane(pane) => self.send(&control::kill_pane(&pane), None),
            Action::Refresh => {
                self.refreshing = true;
                self.dirty = true;
                self.quicken();
            }
            Action::StartAgentTask(request) => return Some(self.start_task(request)),
            Action::CancelAgentTask(request) => self.cancel_task(request),
            Action::RemoveWorktree { session, force } => {
                return Some(self.remove_worktree(&session, force));
            }
            Action::DiscoverRepos => return Some(self.discover_repos()),
            Action::AgentNames => return Some(self.list_agents()),
            Action::RestartServer => return self.restart_server(),
            Action::NoticeAction => return self.notice_action(),
            Action::ConfirmFocus { budget_ms } => return Some(self.confirm_focus(budget_ms)),
            Action::DeliverInput { pane, bytes } => return Some(self.deliver(&pane, bytes)),
            Action::DeliverPaste { pane, text } => {
                return Some(self.deliver(&pane, crate::frame::bracketed_paste(&text)));
            }
            Action::Scroll {
                pane,
                lines,
                source,
            } => self.scroll(&pane, lines, source),
            Action::Mouse {
                pane,
                action,
                button,
                column,
                row,
                modifiers,
            } => self.mouse(&pane, action, button, column, row, modifiers),
        }
        None
    }

    /// Focus one pane (`selectPane`): the frame channel goes there at once,
    /// from the id in hand, while the lane tells the server.
    fn select_pane(&mut self, pane: &str) {
        self.pane_switch_generation += 1;
        self.pane_switch_in_flight = true;
        self.pick_asked_again = false;
        let landed = Some(Landed::PaneSwitch(self.pane_switch_generation));
        if self.poll.snapshot.active_pane_id.as_deref() != Some(pane) {
            self.poll.snapshot.active_pane_id = Some(pane.to_owned());
        }
        self.dirty = true;
        self.event(Event::FocusedPane(pane.to_owned()));
        self.retarget(pane, true);
        if self.config.immersive_zoom {
            self.send(&control::immersive_zoom(pane), landed);
        } else {
            self.send(
                &control::select_pane(pane, self.config.custom_path.as_deref()),
                landed,
            );
        }
    }

    /// A workspace, or on a host with no server, a server (`newSession`).
    fn new_session(&mut self, name: Option<&str>) {
        if self.poll.server_not_running {
            self.bootstrap(None, name.map(str::to_owned));
        } else {
            self.send(&control::new_session(name), None);
        }
    }

    /// The server's focus once the commands queued so far have landed: a
    /// read that rides the lane for its place in line only.
    fn confirm_focus(&mut self, budget: u64) -> RequestId {
        let request = self.request_id();
        self.confirms.insert(request.0);
        let command = self.herdr(control::SNAPSHOT);
        self.enqueue_owned(command, Owner::ConfirmFocus(request.0), false, budget);
        request
    }

    /// Stream for `pane`, minted if it has none: the same one for as long as
    /// the pane lives.
    fn stream_for(&mut self, pane: &str) -> StreamId {
        if let Some(&stream) = self.streams.get(pane) {
            return stream;
        }
        let stream = self.next_stream;
        self.next_stream += 1;
        self.streams.insert(pane.to_owned(), stream);
        self.outputs.push_back(Output::StreamOpened {
            pane: pane.to_owned(),
            stream,
        });
        stream
    }

    /// Close the terminals of panes the tree no longer has (but the one the
    /// frame channel is pointed at, which a stale read may not show yet).
    fn prune_streams(&mut self) {
        let gone: Vec<String> = self
            .streams
            .keys()
            .filter(|pane| {
                !self.poll.snapshot.panes.contains_key(*pane)
                    && self.attach.target.as_deref() != Some(pane.as_str())
            })
            .cloned()
            .collect();
        for pane in gone {
            if let Some(stream) = self.streams.remove(&pane) {
                self.outputs
                    .push_back(Output::StreamClosed { pane, stream });
            }
        }
    }

    fn view(&mut self, pane: &str, action: ViewAction) {
        let stream = self.stream_for(pane);
        self.outputs.push_back(Output::View {
            pane: pane.to_owned(),
            stream,
            action,
        });
    }

    fn pane_bytes(&mut self, pane: &str, bytes: Vec<u8>) {
        let stream = self.stream_for(pane);
        self.outputs.push_back(Output::PaneBytes {
            pane: pane.to_owned(),
            stream,
            bytes,
        });
    }
}

/// The server version a snapshot names (`"version":"0.9.5"`), if it does.
fn snapshot_version(output: &str) -> Option<String> {
    let json = crate::json::first_object(output)?;
    let body = json
        .object("result")
        .and_then(|result| result.object("snapshot"))
        .or_else(|| json.object("snapshot"))
        .unwrap_or(&json);
    body.str("version")
        .filter(|version| !version.is_empty())
        .map(str::to_owned)
}
