//! tmux over SSH: W8's controller ([`Controller`]) driven on a terminal
//! channel that runs `tmux -CC attach`.
//!
//! One task owns the controller. It feeds it what the channel delivers, the
//! person's keys and intents, and its timers, and carries out what it asks,
//! in order: commands go to the writer (one write at a time, in order, so a
//! slow write never stalls the parser), a pane's bytes into that pane's byte
//! channel, view actions and notifications up to the session. Nothing on the
//! byte path takes a hop of its own.
//!
//! Every attach mints its panes fresh byte channels (the controller's
//! backfill fills a terminal from empty); the session keeps showing the
//! previous attach's until this one's first frame, the reconnect's cover.
//!
//! What a pane's view must do — cover, reveal, apply a held resize — goes
//! into that pane's byte channel as a mark ([`view_mark`]), right where the
//! controller asked for it among the pane's bytes: a reveal lands after the
//! repaint it uncovers, a held resize before the frame captured at the new
//! size. (It is still reported as an event too, for whoever watches.)
//!
//! Attach only: the boot line is `tmux -CC attach` and nothing else
//! (`moshpit_tmux::boot`). A server with no session shows the empty state
//! and the person creates the first one; the session never does.
//!
//! The same task runs mosh's `-CC` sidecar ([`SidecarStart`]): a control
//! plane only (`renders_output: false`) on a second channel while mosh
//! renders tmux's own client. It tracks the tree for the chrome, finds the
//! phone's renderer among tmux's clients by its pid (mosh's program), follows
//! it from session to session (`%session-changed`,
//! `%client-session-changed`), pins the renderer's window to the mosh grid,
//! and turns the status line off on the sessions Moshpit made — W8's
//! controller's sidecar mode. Without the renderer's pid (a host whose `ps`
//! can't say) it moves no client and sizes no window: guessing could take a
//! desktop's client for the phone's (B38, B52); the tree still reads.
//!
//! A controller attaching while its terminal isn't on screen (a reconnect on
//! Home or in the background) attaches unpinned (`TmuxStart::pinned`): no
//! window is squashed to the phone's size for a terminal nobody looks at;
//! it pins when it is shown.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use moshpit_ssh::{Event, Process, ProcessWriter, Ssh};
use moshpit_stream::{Input, Mark, Stream, StreamId};
use moshpit_tmux::controller::{
    Config, Controller, InputFlags, Notification, Output, PinLedger, StreamId as ControllerStream,
    ViewAction,
};
use moshpit_tmux::hooks::AgentHook;
use moshpit_tmux::lease::{PaneFollow, SizeFollow};
use moshpit_tmux::model::{Selection, Snapshot};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::event::PaneInfo;

/// What the person (or the UI) asks of tmux.
#[derive(Debug, Clone, PartialEq)]
pub enum TmuxAction {
    SelectSession(String),
    SelectWindow(String),
    SelectPane(String),
    /// The horizontal swipe.
    SwitchPaneOrWindow {
        forward: bool,
    },
    NewWindow {
        name: Option<String>,
    },
    NewWindowInSession {
        session: String,
        name: Option<String>,
    },
    NewPane,
    NewPaneInWindow(String),
    /// An explicit "new session" from the empty state: the one way a
    /// session gets created, and only when the person asks.
    NewSession {
        name: Option<String>,
    },
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
    /// The refresh control: re-list and repaint.
    Refresh,
    /// Re-list quietly: Home or the window picker showing (B586).
    Relist,
    /// The follower's tap: take the window's size back.
    TakeSizeOwnership,
    Paste {
        pane: String,
        text: String,
    },
    ScrollLines(i64),
    /// The scroll beginning is aimed at this cell (0-based, the input
    /// pane's): a program with the mouse gets its wheel there (B578).
    AimWheel {
        col: i64,
        row: i64,
    },
    Click {
        col: i64,
        row: i64,
    },
    /// A pane's view lifted its cover.
    ViewRevealed {
        pane: String,
    },
    /// A line was submitted in a pane.
    ReturnSent {
        pane: String,
    },
    /// The input method switched under a pane.
    InputModeChanged {
        pane: String,
    },
    /// A scroll drag starts over the active pane: drop the carried wheel
    /// remainder and read the pane's mouse flags again (the route the drag
    /// takes depends on them).
    ScrollBegan,
    /// Read the agents' hook stamps now (the Live Activity's sweep).
    PollAgentHooks,
    /// The overview's pictures of panes never shown (B751): their visible
    /// screens, answered as [`TmuxNote::Snapshot`], never fed to a pane.
    SnapshotPanes(Vec<String>),
    /// The overview closed: what is asked or on its way is dropped.
    StopSnapshots,
    /// A pane's view is about to resize and holds it (its container keeps
    /// the old grid) until the repaint that answers it. The controller
    /// resizes the client and releases the hold with an
    /// [`ViewAction::ApplyDeferredResize`] mark when the capture lands (or
    /// at once, when no repaint is coming).
    DeferResize {
        pane: String,
        cols: u16,
        rows: u16,
    },
}

/// Why the core won't do what was asked of tmux, said before anything is
/// sent: the host words it in the person's language (B202).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxRefusal {
    /// A name the person typed holds a line break, a tab or another control
    /// character (`MultiplexerName.refusal`, B139): a paste accident, not a
    /// name. Spliced into control mode it ends the command early and runs
    /// the rest as a command of its own; the first session's exec dropped
    /// it and made a session tmux named instead (B250).
    NameHasControlCharacters,
}

impl TmuxAction {
    /// Why this action is refused as asked, if it is: a name it carries,
    /// trimmed as the app trims what was typed, holds a control character.
    pub fn refusal(&self) -> Option<TmuxRefusal> {
        let name = match self {
            TmuxAction::NewWindow { name }
            | TmuxAction::NewWindowInSession { name, .. }
            | TmuxAction::NewSession { name } => name.as_deref()?,
            TmuxAction::RenameSession { name, .. } | TmuxAction::RenameWindow { name, .. } => {
                name.as_str()
            }
            _ => return None,
        };
        refuses_typed_name(name).then_some(TmuxRefusal::NameHasControlCharacters)
    }
}

/// A typed name, trimmed of the whitespace and line breaks around it,
/// holds a control character (`MultiplexerName.refusal`).
fn refuses_typed_name(name: &str) -> bool {
    moshpit_tmux::format::refuses_name(moshpit_herdr::text::trim_whitespaces_and_newlines(name))
}

/// What the terminal chrome draws from tmux: the tree, and what the
/// controller says around it (a command's complaint, the refresh spinner,
/// the size-lease follower's veil, the agents' stamps, the active pane's
/// mouse and screen flags the scroll routing reads).
#[derive(Debug, Clone, PartialEq)]
pub struct TmuxView {
    pub snapshot: Snapshot,
    pub notice: Option<String>,
    pub refreshing: bool,
    pub size_follow: Option<SizeFollow>,
    /// Another client picked another pane in the window shown: the phone
    /// keeps its own under a veil (B587).
    pub pane_follow: Option<PaneFollow>,
    pub agent_hooks: BTreeMap<String, AgentHook>,
    pub agent_hooks_loaded: bool,
    pub input: InputFlags,
    /// The server has no session at all — asked, not guessed (`has-session`
    /// over exec, once the attach would have stalled): not an attach that
    /// stalled, but one waiting for the person's first session, made from
    /// the empty state (B228). Until something attaches.
    pub no_session: bool,
}

impl TmuxView {
    fn of(controller: &Controller) -> Self {
        Self {
            snapshot: controller.snapshot().clone(),
            notice: controller.notice().map(str::to_owned),
            refreshing: controller.is_refreshing(),
            size_follow: controller.size_follow().cloned(),
            pane_follow: controller.pane_follow().cloned(),
            agent_hooks: controller.agent_hooks().clone(),
            agent_hooks_loaded: controller.agent_hooks_loaded(),
            input: controller.active_input_flags(),
            no_session: false,
        }
    }
}

/// The mark codes of [`ViewAction`]s in a pane's byte channel. The view
/// reads them with `mp_stream_read_marked` and does what they say before it
/// feeds the bytes after them.
pub mod mark {
    pub const VEIL: u32 = 1;
    pub const VEIL_UNLESS_COVERED: u32 = 2;
    /// `arg`: milliseconds.
    pub const EXTEND_COVER: u32 = 3;
    pub const REVEAL: u32 = 4;
    pub const RELEASE_SCROLL_HOLD: u32 = 5;
    /// `arg`: rows, positive = older.
    pub const SCROLL_LOCAL: u32 = 6;
    pub const SETTLE_LOCAL_SCROLL: u32 = 7;
    pub const APPLY_DEFERRED_RESIZE: u32 = 8;
    /// mosh's echo acknowledgement (the shell pane of a mosh session):
    /// `arg` is how many bytes of the input written into this channel the
    /// server has echoed, if it was going to — what predictive echo judges
    /// its guesses by. Put after the frame it came with.
    pub const ECHO_ACK: u32 = 9;
}

/// A view action as the mark that carries it.
pub fn view_mark(action: &ViewAction) -> Mark {
    let (code, arg) = match *action {
        ViewAction::Veil => (mark::VEIL, 0),
        ViewAction::VeilUnlessCovered => (mark::VEIL_UNLESS_COVERED, 0),
        ViewAction::ExtendCover { seconds } => (
            mark::EXTEND_COVER,
            (seconds * 1000.0).round().clamp(0.0, i64::MAX as f64) as i64,
        ),
        ViewAction::Reveal => (mark::REVEAL, 0),
        ViewAction::ReleaseScrollHold => (mark::RELEASE_SCROLL_HOLD, 0),
        ViewAction::ScrollLocal { lines } => (mark::SCROLL_LOCAL, lines),
        ViewAction::SettleLocalScroll => (mark::SETTLE_LOCAL_SCROLL, 0),
        ViewAction::ApplyDeferredResize => (mark::APPLY_DEFERRED_RESIZE, 0),
    };
    Mark { code, arg }
}

/// The view action a mark carries; `None` for a code this build doesn't
/// know.
pub fn view_action(mark: Mark) -> Option<ViewAction> {
    Some(match mark.code {
        mark::VEIL => ViewAction::Veil,
        mark::VEIL_UNLESS_COVERED => ViewAction::VeilUnlessCovered,
        mark::EXTEND_COVER => ViewAction::ExtendCover {
            seconds: mark.arg as f64 / 1000.0,
        },
        mark::REVEAL => ViewAction::Reveal,
        mark::RELEASE_SCROLL_HOLD => ViewAction::ReleaseScrollHold,
        mark::SCROLL_LOCAL => ViewAction::ScrollLocal { lines: mark.arg },
        mark::SETTLE_LOCAL_SCROLL => ViewAction::SettleLocalScroll,
        mark::APPLY_DEFERRED_RESIZE => ViewAction::ApplyDeferredResize,
        _ => return None,
    })
}

/// What the session tells the tmux task.
pub(crate) enum TmuxCommand {
    Input {
        pane: String,
        bytes: Vec<u8>,
    },
    Grid {
        pane: String,
        cols: u16,
        rows: u16,
    },
    Action(TmuxAction),
    /// Hand the windows back (the app left the foreground).
    Background,
    /// Take them again.
    Foreground,
    /// Answer once everything asked of tmux so far has been written to the
    /// connection (the pins handed back on the way to the background).
    Flush(oneshot::Sender<()>),
    /// Stop. `courtesy`: hand back what we hold on the server first, over an
    /// exec channel (a full disconnect). The reply carries what a
    /// replacement inherits and where the person was.
    Close {
        courtesy: bool,
        reply: oneshot::Sender<Handoff>,
    },
    /// The sidecar's mosh grid: the renderer's window is pinned to it.
    MoshGrid {
        cols: u16,
        rows: u16,
    },
    /// The sidecar's renderer: tmux's client with this pid.
    RendererPid(i64),
    /// One round trip on the control channel (the in-place return's
    /// question): `Some(alive)` once tmux answered or the channel closed,
    /// `None` when nothing is attached to ask.
    Ping(oneshot::Sender<Option<bool>>),
    /// The server's tmux prefix (`show-options -gqv prefix`), for the copy-
    /// mode keys a scroll types on mosh's client (B362): what tmux printed,
    /// `C-b` when it can't say.
    Prefix(oneshot::Sender<String>),
}

/// What a retired controller leaves its replacement.
#[derive(Debug, Default)]
pub(crate) struct Handoff {
    pub ledger: Option<PinLedger>,
    pub hooks: Option<BTreeMap<String, AgentHook>>,
    pub selection: Option<Selection>,
}

/// What the tmux task tells the session.
pub(crate) enum TmuxNote {
    /// A pane got a byte channel: the session routes its input.
    Opened {
        pane: String,
        stream: Arc<Stream>,
        input: Input,
    },
    /// The panes with channels changed (or which is active).
    Panes(Vec<PaneInfo>),
    View {
        stream: StreamId,
        action: ViewAction,
    },
    /// Boxed: the tree is far bigger than every other note.
    Tree(Box<TmuxView>),
    FirstFrame,
    Activity(String),
    Bell(String),
    HooksUpdated,
    /// A never-shown pane's visible screen, for the overview (B751).
    Snapshot {
        pane: String,
        rows: Vec<String>,
    },
    AttachStalled,
    /// The control channel is dead, or its shell went away: a drop (the
    /// sessions the person works in live on the server).
    Lost(String),
    /// The person's first session is made (B228): the control client
    /// attaches it again on a new channel of the same connection, the leg
    /// replaced in place. Never a drop: the session doesn't reconnect for
    /// it, whose Reconnecting showed as a disconnect after New Session
    /// (B855).
    Reattach,
}

/// How a tmux task starts.
pub(crate) struct TmuxStart {
    pub ssh: Ssh,
    pub pty: Process,
    pub tmux: String,
    pub config: Config,
    pub grid: (u16, u16),
    pub restore: Option<Selection>,
    /// `restore` is the person's own pick (B405): it moves a desktop on that
    /// session as a tap would. Otherwise it is where the app last was, and
    /// leaves a desktop where it is (B457).
    pub restore_chosen: bool,
    pub inherited: Handoff,
    pub attach_stall: Duration,
    pub inbound: Arc<AtomicU64>,
    /// The terminal is on screen with the app in front: the attach pins
    /// its window. Else it attaches unpinned and pins once shown.
    pub pinned: bool,
    /// Run as mosh's sidecar instead of the terminal's controller.
    pub sidecar: Option<SidecarStart>,
}

/// How mosh's `-CC` sidecar starts. The session a rebuilt sidecar goes
/// back to is in its terminal's program (`sidecar_attach_program`).
pub(crate) struct SidecarStart {
    /// The windows before the renderer sized them (B270), for the first
    /// sidecar of a mosh session.
    pub layouts_before_renderer: Option<String>,
    /// The renderer's pid, when already known.
    pub renderer_pid: Option<i64>,
    /// The mosh grid, when already known.
    pub grid: Option<(u16, u16)>,
}

pub(crate) struct TmuxLeg {
    pub commands: mpsc::UnboundedSender<TmuxCommand>,
    task: JoinHandle<()>,
}

impl TmuxLeg {
    /// A leg whose commands a test reads, running nothing.
    #[cfg(test)]
    pub fn for_tests(commands: mpsc::UnboundedSender<TmuxCommand>) -> Self {
        TmuxLeg {
            commands,
            task: tokio::spawn(async {}),
        }
    }

    pub fn spawn(
        start: TmuxStart,
        clock: impl Fn() -> u64 + Send + Sync + 'static,
        notes: Box<dyn Fn(TmuxNote) + Send + Sync>,
    ) -> Self {
        let (commands, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(start, rx, clock, notes));
        Self { commands, task }
    }

    pub fn abort(&self) {
        self.task.abort();
    }
}

impl Drop for TmuxLeg {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// How long the teardown courtesy (unpin, layouts, leases) may take as a
/// whole: a socket that died between the decision and the commands must not
/// hold the disconnect hostage.
const COURTESY_BOUND: Duration = Duration::from_secs(3);

/// How long the server may take to say whether it has a session (B228;
/// the app's `serverHasNoSession` waited as long).
const HAS_SESSION_BOUND: Duration = Duration::from_secs(5);

struct Task {
    controller: Controller,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    notes: Box<dyn Fn(TmuxNote) + Send + Sync>,
    writes: mpsc::UnboundedSender<Write>,
    /// The controller's stream ids → this attach's byte channels.
    streams: BTreeMap<ControllerStream, (String, Arc<Stream>)>,
    panes_sent: Vec<PaneInfo>,
    /// The flags last reported: they change on a reply that touches nothing
    /// else the view draws.
    flags_sent: InputFlags,
    /// Flushes asked for, queued behind the next pump's commands.
    flushes: Vec<oneshot::Sender<()>>,
    /// Pings waiting for tmux's answer, by the controller's request.
    pings: BTreeMap<u64, oneshot::Sender<Option<bool>>>,
    /// Prefix questions waiting for tmux's answer, by request.
    prefixes: BTreeMap<u64, oneshot::Sender<String>>,
    first_frame: bool,
    /// Running as mosh's sidecar.
    sidecar: bool,
    /// The windows are ours to size (on screen, app in front).
    pinned: bool,
    /// The sidecar's mosh grid, and the renderer's window last pinned to it.
    mosh_grid: Option<(u16, u16)>,
    synced: Option<(String, (u16, u16))>,
    /// The renderer's pane whose program's flags the sidecar last asked
    /// for, this attach.
    flags_read_for: Option<String>,
    /// For what control mode can't carry yet: the first session, made
    /// before anything is attached.
    ssh: Ssh,
    tmux: String,
    /// The server answered that it has no session (B228), and nothing has
    /// attached since.
    no_session: bool,
    /// tmux's control client said `%exit` this pump (B786): the server is
    /// asked whether it still has a session.
    exited: bool,
    /// The `has-session` out is about an exit, not a stalled first attach.
    after_exit: bool,
}

/// What the writer is given, in order.
enum Write {
    Bytes(Vec<u8>),
    /// Everything before it has been written.
    Flush(oneshot::Sender<()>),
}

async fn run(
    start: TmuxStart,
    mut commands: mpsc::UnboundedReceiver<TmuxCommand>,
    clock: impl Fn() -> u64 + Send + Sync + 'static,
    notes: Box<dyn Fn(TmuxNote) + Send + Sync>,
) {
    let TmuxStart {
        ssh,
        pty,
        tmux,
        mut config,
        grid,
        restore,
        restore_chosen,
        inherited,
        attach_stall,
        inbound,
        pinned,
        sidecar,
    } = start;
    if sidecar.is_some() {
        config.renders_output = false;
    }
    let (mut reader, writer) = pty.split();
    let (writes, write_rx) = mpsc::unbounded_channel();
    let writer_task = tokio::spawn(write(writer, write_rx));
    let mut task = Task {
        controller: Controller::new(config),
        clock: Box::new(clock),
        notes,
        writes,
        streams: BTreeMap::new(),
        panes_sent: Vec::new(),
        flags_sent: InputFlags::default(),
        flushes: Vec::new(),
        pings: BTreeMap::new(),
        prefixes: BTreeMap::new(),
        first_frame: false,
        sidecar: sidecar.is_some(),
        pinned,
        mosh_grid: sidecar.as_ref().and_then(|s| s.grid),
        synced: None,
        flags_read_for: None,
        ssh: ssh.clone(),
        tmux: tmux.clone(),
        no_session: false,
        exited: false,
        after_exit: false,
    };
    let now = (task.clock)();
    task.controller
        .set_initial_client_size(i64::from(grid.0), i64::from(grid.1));
    task.controller.set_pending_restore(restore);
    task.controller.set_restore_chosen(restore_chosen);
    if let Some(ledger) = inherited.ledger {
        task.controller.inherit_pin_ledger(ledger);
    }
    if let Some(hooks) = inherited.hooks {
        task.controller.inherit_agent_hooks(hooks);
    }
    if !pinned {
        // Nobody looks at this terminal: attach without taking the window.
        task.controller.release_window_pins(now);
    }
    // Reading first, no discovery: tmux isn't running yet. Its first
    // %session-changed starts the attach.
    task.controller.begin_control_mode(now);
    if let Some(pid) = sidecar.as_ref().and_then(|s| s.renderer_pid) {
        task.controller.expect_renderer_pid(pid, now);
    }
    if let Some(listing) = sidecar
        .as_ref()
        .and_then(|s| s.layouts_before_renderer.as_deref())
    {
        task.controller.note_layouts_before_renderer(listing);
    }
    // Nothing is typed: the terminal's program is tmux's control client
    // (`control_attach_program`, `sidecar_attach_program`), already
    // running; what it printed waits in the channel for the read below.
    let stall_at = Instant::now() + attach_stall;
    let mut stall_reported = false;
    // Deadlines are the session clock's, which counts the device's
    // sleep and `Instant` doesn't: each sleeps for what is left of it (B611).

    // Whether the session closed this attach (else its channel died).
    let mut closed = false;
    // The first session is being made by exec: this attach gave up, the
    // session attaches again once it is there.
    let mut reattach: Option<oneshot::Receiver<Result<(), String>>> = None;
    // Whether the server has any session, asked when the attach would have
    // stalled (B228).
    let mut asked: Option<oneshot::Receiver<Option<bool>>> = None;
    loop {
        if task.pump() {
            break;
        }
        // The control client ended under a live channel (B786): the shell
        // runs on after it, so nothing reads as dead. Asked which it was.
        if std::mem::take(&mut task.exited) && asked.is_none() {
            task.after_exit = true;
            asked = Some(task.ask_has_session());
        }
        let deadline = task
            .controller
            .next_deadline()
            .map(|at| Instant::now() + Duration::from_millis(at.saturating_sub(task.now())));
        let stall =
            (!stall_reported && !task.controller.snapshot().is_attached).then_some(stall_at);
        tokio::select! {
            event = reader.next() => match event {
                Some(Event::Stdout(bytes)) | Some(Event::Stderr(bytes)) => {
                    let now = (task.clock)();
                    inbound.store(now, Ordering::Relaxed);
                    task.controller.receive(&bytes, now);
                }
                Some(Event::Exited(_)) | Some(Event::Eof) => {}
                Some(Event::Closed) | Some(Event::Lost) | None => {
                    // The control shell exited or the connection dropped:
                    // either way tmux's sessions outlive it, so a drop (the
                    // controller reports its channel dead).
                    let now = (task.clock)();
                    task.controller.channel_closed(now);
                    task.pump();
                    break;
                }
            },
            command = commands.recv() => match command {
                Some(TmuxCommand::Close { courtesy, reply }) => {
                    let handoff = task.close(courtesy, &ssh, &tmux).await;
                    let _ = reply.send(handoff);
                    closed = true;
                    break;
                }
                Some(command) => {
                    if let Some(made) = task.command(command) {
                        reattach = Some(made);
                    }
                }
                None => {
                    closed = true;
                    break;
                }
            },
            made = wait(&mut reattach) => match made {
                Ok(()) if !task.sidecar => {
                    (task.notes)(TmuxNote::Reattach);
                    break;
                }
                // mosh's sidecar and renderer attach on their own.
                Ok(()) => {}
                // Not made: nothing changed on the server. Attaching again
                // only flashed a disconnect onto the same empty state;
                // tmux's words go to the notice, and New Session is back
                // (B788).
                Err(why) => {
                    task.controller.surface_failure(why);
                    task.pump();
                    let view = task.view();
                    (task.notes)(TmuxNote::Tree(Box::new(view)));
                }
            },
            () = sleep_until(deadline) => {
                let now = (task.clock)();
                task.controller.tick(now);
            }
            () = sleep_until(stall), if stall.is_some() => {
                stall_reported = true;
                if !task.controller.snapshot().is_attached {
                    // A server with no session is no stalled attach: asked
                    // before anything is said (B228).
                    asked = Some(task.ask_has_session());
                }
            }
            told = answer(&mut asked) => {
                let after_exit = std::mem::take(&mut task.after_exit);
                if !task.controller.snapshot().is_attached {
                    if told.flatten() == Some(false) {
                        task.no_session = true;
                        // No session on the server, or no server: one that
                        // died under the attach (killed, crashed, its last
                        // session gone, B786) takes its sessions, panes and
                        // agents' stamps with it, and the empty state offers
                        // a new session — never a dead server's tree on Home
                        // and a blank pane behind its row.
                        task.controller.forget_server();
                        task.pump();
                        let view = task.view();
                        (task.notes)(TmuxNote::Tree(Box::new(view)));
                    } else if after_exit {
                        // The server still has sessions: this client was
                        // detached. Attached again, as a dropped attach is.
                        (task.notes)(TmuxNote::Lost(
                            "the tmux control client was detached".to_owned(),
                        ));
                        break;
                    } else {
                        (task.notes)(TmuxNote::AttachStalled);
                    }
                }
            }
        }
    }
    if !closed {
        // The channel is dead and the session will tear this attach down:
        // what it owes the server (its pins) and where the person was go
        // to the session then — never a round trip on the dead channel. A
        // pick made meanwhile (a notification tapped as the return finds
        // the socket dead) is where they are now (B405).
        let mut picked: Option<Selection> = None;
        while let Some(command) = commands.recv().await {
            match command {
                TmuxCommand::Close { reply, .. } => {
                    let mut handoff = task.close(false, &ssh, &tmux).await;
                    if let Some(pick) = picked.take() {
                        handoff.selection = Some(pick);
                    }
                    let _ = reply.send(handoff);
                    break;
                }
                TmuxCommand::Action(action) => {
                    if let Some(pick) = pick_selection(&action, Some(task.controller.snapshot())) {
                        picked = Some(pick);
                    }
                }
                _ => {}
            }
        }
    }
    // The writer finishes what it holds on its own, or fails once the
    // session closes the connection; nothing here waits on a connection
    // that may be dead.
    drop(task);
    drop(writer_task);
}

/// Where a pick puts the person, as a restore names it: the pane, window
/// or session picked, with the window and session it sits in when
/// `snapshot` knows them (B405). `None` for anything but a pick.
pub(crate) fn pick_selection(
    action: &TmuxAction,
    snapshot: Option<&Snapshot>,
) -> Option<Selection> {
    let window_of = |pane: &str| {
        snapshot
            .and_then(|snapshot| snapshot.panes.get(pane))
            .map(|pane| pane.window_id.clone())
            .filter(|window| !window.is_empty())
    };
    let session_of = |window: &str| {
        snapshot
            .and_then(|snapshot| snapshot.windows.get(window))
            .map(|window| window.session_id.clone())
            .unwrap_or_default()
    };
    match action {
        TmuxAction::SelectPane(pane) => {
            let window = window_of(pane);
            Some(Selection {
                session: window.as_deref().map(session_of).unwrap_or_default(),
                window,
                pane: Some(pane.clone()),
            })
        }
        TmuxAction::SelectWindow(window) => Some(Selection {
            session: session_of(window),
            window: Some(window.clone()),
            pane: None,
        }),
        TmuxAction::SelectSession(session) => Some(Selection {
            session: session.clone(),
            window: None,
            pane: None,
        }),
        _ => None,
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// `slot`'s answer once it comes (`None`: its sender went away); forever
/// without one.
async fn answer<T>(slot: &mut Option<oneshot::Receiver<T>>) -> Option<T> {
    match slot {
        Some(receiver) => {
            let told = receiver.await.ok();
            *slot = None;
            told
        }
        None => std::future::pending().await,
    }
}

/// Resolves once `slot`'s receiver does (forever without one).
/// How the first session's exec went, once it is done; never while none
/// is being made.
async fn wait(slot: &mut Option<oneshot::Receiver<Result<(), String>>>) -> Result<(), String> {
    match slot {
        Some(receiver) => {
            let made = receiver
                .await
                .unwrap_or_else(|_| Err("new-session failed".to_owned()));
            *slot = None;
            made
        }
        None => std::future::pending().await,
    }
}

async fn write(writer: ProcessWriter, mut writes: mpsc::UnboundedReceiver<Write>) {
    while let Some(write) = writes.recv().await {
        match write {
            Write::Bytes(bytes) => {
                if writer.write(&bytes).await.is_err() {
                    break;
                }
            }
            Write::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
    // A dead connection: whoever waits for a flush hears the writer is done
    // (the dropped senders), never waits forever.
}

impl Task {
    fn now(&self) -> u64 {
        (self.clock)()
    }

    /// Carries out what the controller queued. `true` once its channel is
    /// dead.
    fn pump(&mut self) -> bool {
        let mut dead = false;
        let mut tree = false;
        while let Some(output) = self.controller.poll_output() {
            match output {
                Output::Command(command) => {
                    let mut line = command.into_bytes();
                    line.push(b'\n');
                    let _ = self.writes.send(Write::Bytes(line));
                }
                Output::PaneOutput { stream, bytes, .. } => {
                    if let Some((_, channel)) = self.streams.get(&stream) {
                        channel.push(&bytes);
                    }
                }
                Output::View { stream, action, .. } => {
                    if let Some((_, channel)) = self.streams.get(&stream) {
                        channel.mark(view_mark(&action));
                        (self.notes)(TmuxNote::View {
                            stream: channel.id(),
                            action,
                        });
                    }
                }
                Output::StreamOpened { .. } if self.sidecar => {}
                Output::StreamOpened { pane, stream } => {
                    if let std::collections::btree_map::Entry::Vacant(slot) =
                        self.streams.entry(stream)
                    {
                        let (channel, input) = moshpit_stream::open();
                        let channel = Arc::new(channel);
                        slot.insert((pane.clone(), channel.clone()));
                        (self.notes)(TmuxNote::Opened {
                            pane,
                            stream: channel,
                            input,
                        });
                    }
                }
                Output::StreamClosed { stream, .. } => {
                    self.streams.remove(&stream);
                }
                Output::Notify(note) => match note {
                    Notification::StateChanged => tree = true,
                    Notification::FirstFramePainted => {
                        if !self.first_frame {
                            self.first_frame = true;
                            (self.notes)(TmuxNote::FirstFrame);
                        }
                    }
                    Notification::ChannelDead { reason } => {
                        (self.notes)(TmuxNote::Lost(format!("tmux channel {reason}")));
                        dead = true;
                    }
                    Notification::PaneActivity { pane } => (self.notes)(TmuxNote::Activity(pane)),
                    Notification::PaneBell { pane } => (self.notes)(TmuxNote::Bell(pane)),
                    Notification::PaneSnapshot { pane, rows } => {
                        (self.notes)(TmuxNote::Snapshot { pane, rows })
                    }
                    Notification::AgentHooksUpdated => {
                        tree = true;
                        (self.notes)(TmuxNote::HooksUpdated);
                    }
                    Notification::Pong { request, alive } => {
                        if let Some(reply) = self.pings.remove(&request) {
                            let _ = reply.send(Some(alive));
                        }
                    }
                    Notification::PrefixKey { request, key } => {
                        if let Some(reply) = self.prefixes.remove(&request) {
                            let _ = reply.send(key);
                        }
                    }
                    Notification::ControlExited { reason } => {
                        log::info!("tmux control client ended: {reason:?}");
                        self.exited = true;
                    }
                    Notification::ClientCount { .. }
                    | Notification::PanePath { .. }
                    | Notification::Flushed { .. } => {}
                },
            }
        }
        if self.sidecar && (self.sync_mosh_window() | self.read_renderer_flags()) {
            // Its commands go out with this pump's.
            while let Some(output) = self.controller.poll_output() {
                if let Output::Command(command) = output {
                    let mut line = command.into_bytes();
                    line.push(b'\n');
                    let _ = self.writes.send(Write::Bytes(line));
                }
            }
        }
        for done in self.flushes.drain(..) {
            let _ = self.writes.send(Write::Flush(done));
        }
        let flags = self.controller.active_input_flags();
        if tree || flags != self.flags_sent {
            self.flags_sent = flags;
            let view = self.view();
            (self.notes)(TmuxNote::Tree(Box::new(view)));
        }
        let panes = self.panes();
        if panes != self.panes_sent {
            self.panes_sent = panes.clone();
            (self.notes)(TmuxNote::Panes(panes));
        }
        dead
    }

    /// The sidecar pins the renderer's window to the mosh grid once both
    /// are known, and again whenever either moves (`syncMoshWindow`).
    /// Whether it asked for anything.
    fn sync_mosh_window(&mut self) -> bool {
        if !self.pinned {
            return false;
        }
        let Some(grid) = self.mosh_grid else {
            return false;
        };
        let snapshot = self.controller.snapshot();
        let Some(session) = self.controller.renderer_session_id() else {
            return false;
        };
        if snapshot.active_session_id.as_deref() != Some(session) {
            return false;
        }
        let Some(window) = snapshot.active_window_id.clone().filter(|window| {
            snapshot
                .windows
                .get(window)
                .is_some_and(|info| info.session_id == session)
        }) else {
            return false;
        };
        let wanted = (window, grid);
        if self.synced.as_ref() == Some(&wanted) {
            return false;
        }
        self.synced = Some(wanted);
        let now = self.now();
        self.controller
            .sync_mosh_window(i64::from(grid.0), i64::from(grid.1), now);
        true
    }

    /// The sidecar reads the flags of the renderer's pane's program (does
    /// it want the mouse) once attached, and again when that pane changes:
    /// a mosh scroll's route depends on them (B362). It never repaints, the
    /// controller's way of reading them; read only as a drag began, a
    /// program already running at the attach got its first ticks as
    /// copy-mode. Whether it asked.
    fn read_renderer_flags(&mut self) -> bool {
        let snapshot = self.controller.snapshot();
        if !snapshot.is_attached {
            self.flags_read_for = None;
            return false;
        }
        let Some(pane) = snapshot.input_pane() else {
            return false;
        };
        if self.flags_read_for.as_ref() == Some(&pane) {
            return false;
        }
        self.flags_read_for = Some(pane);
        let now = self.now();
        self.controller.refresh_active_pane_mouse(now);
        true
    }

    fn panes(&self) -> Vec<PaneInfo> {
        if self.sidecar {
            return Vec::new();
        }
        let active = self.controller.snapshot().active_pane_id.clone();
        self.streams
            .values()
            .map(|(pane, channel)| PaneInfo {
                pane: pane.clone(),
                stream: channel.id(),
                active: active.as_deref() == Some(pane.as_str()),
            })
            .collect()
    }

    /// Carries out one command; a receiver when the attach must be made
    /// again once it resolves (the first session, made by exec).
    fn command(&mut self, command: TmuxCommand) -> Option<oneshot::Receiver<Result<(), String>>> {
        let now = self.now();
        let c = &mut self.controller;
        match command {
            TmuxCommand::Input { pane, bytes } => c.view_input(&pane, &bytes, now),
            TmuxCommand::Grid { pane, cols, rows } => {
                c.view_grid(&pane, i64::from(cols), i64::from(rows), now)
            }
            TmuxCommand::Background => {
                self.pinned = false;
                self.synced = None;
                c.release_window_pins(now)
            }
            TmuxCommand::Foreground => {
                self.pinned = true;
                // The repin sizes the renderer's window to the last grid.
                self.synced = None;
                c.repin_active_window(now)
            }
            TmuxCommand::MoshGrid { cols, rows } => {
                self.mosh_grid = Some((cols, rows));
            }
            TmuxCommand::RendererPid(pid) => c.expect_renderer_pid(pid, now),
            TmuxCommand::Prefix(reply) => {
                let request = c.query_prefix_key(now);
                self.prefixes.insert(request, reply);
            }
            TmuxCommand::Ping(reply) => {
                if c.snapshot().is_attached {
                    let request = c.ping(now);
                    self.pings.insert(request, reply);
                } else {
                    let _ = reply.send(None);
                }
            }
            TmuxCommand::Flush(done) => {
                // Behind what the commands before this one queued: the
                // controller's outputs go to the writer at the next pump,
                // and the marker right after them.
                self.flushes.push(done);
            }
            TmuxCommand::Action(TmuxAction::NewSession { name }) if !c.snapshot().is_attached => {
                return self.first_session(name);
            }
            TmuxCommand::Action(action) => match action {
                TmuxAction::SelectSession(id) => c.select_session(&id, now),
                TmuxAction::SelectWindow(id) => c.select_window(&id, now),
                TmuxAction::SelectPane(id) => c.select_pane(&id, now),
                TmuxAction::SwitchPaneOrWindow { forward } => c.switch_pane_or_window(forward, now),
                TmuxAction::NewWindow { name } => c.new_window(name.as_deref(), now),
                TmuxAction::NewWindowInSession { session, name } => {
                    c.new_window_in_session(&session, name.as_deref(), now)
                }
                TmuxAction::NewPane => c.new_pane(now),
                TmuxAction::NewPaneInWindow(window) => c.new_pane_in_window(&window, now),
                TmuxAction::NewSession { name } => c.new_session(name.as_deref(), now),
                TmuxAction::RenameSession { session, name } => {
                    c.rename_session(&session, &name, now)
                }
                TmuxAction::KillSession(id) => c.kill_session(&id, now),
                TmuxAction::RenameWindow { window, name } => c.rename_window(&window, &name, now),
                TmuxAction::KillWindow(id) => c.kill_window(&id, now),
                TmuxAction::KillPane(id) => c.kill_pane(&id, now),
                TmuxAction::Refresh => c.refresh(now),
                TmuxAction::Relist => c.relist(now),
                TmuxAction::TakeSizeOwnership => c.take_size_ownership(now),
                TmuxAction::Paste { pane, text } => c.send_paste(&text, &pane, now),
                TmuxAction::ScrollLines(lines) => c.scroll_lines(lines, now),
                TmuxAction::AimWheel { col, row } => c.aim_wheel(col, row),
                TmuxAction::Click { col, row } => c.click(col, row, now),
                TmuxAction::ViewRevealed { pane } => c.view_revealed(&pane, now),
                TmuxAction::ReturnSent { pane } => c.view_return_sent(&pane, now),
                TmuxAction::InputModeChanged { pane } => c.view_input_mode_changed(&pane, now),
                TmuxAction::ScrollBegan => c.scroll_began(now),
                TmuxAction::PollAgentHooks => c.poll_agent_hooks(now),
                TmuxAction::SnapshotPanes(panes) => c.snapshot_panes(&panes, now),
                TmuxAction::StopSnapshots => c.stop_snapshots(now),
                TmuxAction::DeferResize { pane, cols, rows } => {
                    let held =
                        c.view_should_defer_resize(&pane, i64::from(cols), i64::from(rows), now);
                    if !held {
                        // No repaint is coming to release the view's hold:
                        // release it now, after everything already queued
                        // for the pane.
                        if let Some(&stream) = c.streams().get(&pane)
                            && let Some((_, channel)) = self.streams.get(&stream)
                        {
                            channel.mark(view_mark(&ViewAction::ApplyDeferredResize));
                        }
                    }
                }
            },
            TmuxCommand::Close { .. } => {}
        }
        None
    }

    /// What the chrome draws now: the controller's tree, and whether the
    /// server was found with no session — until something attaches.
    fn view(&mut self) -> TmuxView {
        if self.controller.snapshot().is_attached {
            self.no_session = false;
        }
        TmuxView {
            no_session: self.no_session,
            ..TmuxView::of(&self.controller)
        }
    }

    /// Whether the server has any session, asked over an exec channel: the
    /// control client says nothing that tells (mosh's sidecar retries its
    /// attach in silence). `None` when it can't be told — no answer in
    /// time, no tmux the exec finds.
    fn ask_has_session(&self) -> oneshot::Receiver<Option<bool>> {
        let command = has_session_command(&self.tmux);
        let ssh = self.ssh.clone();
        let (tell, told) = oneshot::channel();
        tokio::spawn(async move {
            let run = tokio::time::timeout(HAS_SESSION_BOUND, ssh.run(&command)).await;
            let _ = tell.send(match run {
                Ok(Ok(output)) => has_session(output.status()),
                _ => None,
            });
        });
        told
    }

    /// The person's "new session" from the empty state, with nothing
    /// attached: control mode can't carry it (a command written now lands in
    /// the shell), so an exec channel makes it. The sidecar's attach chain
    /// and mosh's waiting renderer pick it up by themselves; the terminal's
    /// own `-CC attach` already gave up ("no sessions"), so this attach is
    /// done with and the session attaches again, onto the new session.
    ///
    /// A name with a control character makes no session at all
    /// ([`TmuxRefusal::NameHasControlCharacters`], which the registry
    /// answers before it gets here): it was dropped, and a session tmux
    /// named made in its place (B250).
    fn first_session(
        &mut self,
        name: Option<String>,
    ) -> Option<oneshot::Receiver<Result<(), String>>> {
        let name = match first_session_name(name) {
            Ok(name) => name,
            Err(refusal) => {
                log::info!("tmux: the first session wasn't made: {refusal:?}");
                return None;
            }
        };
        let command = first_session_command(&self.tmux, name.as_deref());
        let ssh = self.ssh.clone();
        let (made, done) = oneshot::channel();
        tokio::spawn(async move {
            let outcome = match ssh.run(&command).await {
                Ok(output) if output.success() => Ok(()),
                // tmux's own words ("duplicate session: x"), or the
                // shell's.
                Ok(output) => Err(String::from_utf8_lossy(&output.merged())
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .unwrap_or("new-session failed")
                    .to_owned()),
                Err(error) => Err(error.to_string()),
            };
            if let Err(why) = &outcome {
                log::info!("tmux: the first session wasn't made: {why}");
            }
            let _ = made.send(outcome);
        });
        Some(done)
    }

    /// The end of this controller. A full disconnect first hands back what
    /// we hold on the server — pinned window sizes, the layouts our pin
    /// squashed, our size leases — over a one-shot exec channel (an in-band
    /// write races the detach), bounded as a whole; a reconnect spends
    /// nothing on a connection judged dead and hands the debt over instead.
    async fn close(&mut self, courtesy: bool, ssh: &Ssh, tmux: &str) -> Handoff {
        let now = self.now();
        let snapshot = self.controller.snapshot();
        let selection = snapshot
            .active_session_id
            .clone()
            .filter(|_| snapshot.is_attached)
            .map(|session| Selection {
                session,
                window: snapshot.active_window_id.clone(),
                pane: snapshot.active_pane_id.clone(),
            });
        let mut handoff = Handoff {
            selection,
            ..Handoff::default()
        };
        if courtesy {
            let leases = self.controller.size_lease_release_commands();
            // The phone's renderer out first (B270): attached, it sizes the
            // windows straight back to the phone when they go back to
            // automatic sizing. The session is ending; so is its mosh. Then
            // the pins and the layouts, each window's only while it is still
            // in a layout of ours (round thirty-four): one script that reads
            // them all before anything changes a size.
            let commands = courtesy_commands(&mut self.controller, tmux, leases);
            if !commands.is_empty() {
                let _ = tokio::time::timeout(COURTESY_BOUND, async {
                    for command in &commands {
                        let _ = ssh.run(command).await;
                    }
                })
                .await;
            }
        } else {
            handoff.ledger = Some(self.controller.take_pin_ledger());
            handoff.hooks = self.controller.take_agent_hooks();
        }
        self.controller.detach(now);
        self.pump();
        handoff
    }
}

/// What a courtesy teardown runs over exec, each through
/// [`moshpit_tmux::boot::exec`] (B850): the phone's renderer out first
/// (B270), then the pins and the layouts in one script, the size leases
/// (`leases`, the controller's), and the status line the sidecar turned off.
/// Each ran bare `tmux` and did nothing where the exec's PATH has no tmux.
pub(crate) fn courtesy_commands(
    controller: &mut Controller,
    tmux: &str,
    leases: Vec<String>,
) -> Vec<String> {
    let renderer: Vec<String> = controller
        .renderer_detach_command(tmux)
        .into_iter()
        .collect();
    let hand_back = controller.pin_hand_back_script(tmux, &renderer);
    let ledger = controller.take_pin_ledger();
    let mut commands: Vec<String> = hand_back.into_iter().collect();
    commands.extend(
        leases
            .into_iter()
            .map(|command| format!("{tmux} {command}")),
    );
    if let Some(session) = ledger.hidden_status.as_deref() {
        // The status line the sidecar turned off, back.
        commands.extend(moshpit_tmux::boot::status_restore_commands(tmux, session));
    }
    // B812: the phone's own group ends with the session; tmux's
    // `destroy-unattached` no longer does it (it could crash the server
    // under a client still pointing at the group).
    if let Some(command) = controller.group_release_command() {
        commands.push(format!("{tmux} {command}"));
    }
    commands
        .iter()
        .map(|command| moshpit_tmux::boot::exec(command))
        .collect()
}

/// The name the first session is made with (`first_session`): as typed,
/// trimmed; none for a blank one (tmux names it); refused with a control
/// character in it, never dropped for tmux's name.
pub(crate) fn first_session_name(name: Option<String>) -> Result<Option<String>, TmuxRefusal> {
    let Some(name) = name else {
        return Ok(None);
    };
    let trimmed = moshpit_herdr::text::trim_whitespaces_and_newlines(&name);
    if refuses_typed_name(trimmed) {
        return Err(TmuxRefusal::NameHasControlCharacters);
    }
    Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()))
}

/// The exec that makes the person's first session when nothing is attached
/// (`first_session`): detached, named when they named it — single-quoted,
/// with every `#` doubled, since tmux expands `new-session -s` as a format
/// (tmux 3.6a, private socket: `-s 'a#{session_id}b'` is stored as `ab`,
/// `c##d` as `c#d`). The app's `tmuxFirstSessionCommand` makes the same
/// session (B228).
///
/// tmux is found as the control client and `has-session` find it
/// ([`moshpit_tmux::boot::exec`]). It ran on the bare PATH: on a Mac, whose
/// non-interactive PATH has no Homebrew, tmux wasn't found, nothing was
/// made, and the app attached again to the same empty server (B788).
pub(crate) fn first_session_command(tmux: &str, name: Option<&str>) -> String {
    let mut command = format!("{tmux} new-session -d");
    if let Some(name) = name {
        command.push_str(" -s ");
        command.push_str(&moshpit_herdr::launch::quote(&name.replace('#', "##")));
    }
    moshpit_tmux::boot::exec(&command)
}

/// The exec that asks whether the server has any session (B228): tmux's
/// own answer, its status, with tmux found as the control client finds it
/// (`moshpit_tmux::boot`'s PATH). Under `sh -c` (B138).
pub(crate) fn has_session_command(tmux: &str) -> String {
    moshpit_tmux::boot::exec(&format!("{tmux} has-session 2>/dev/null"))
}

/// What `has-session`'s status says: 0, a session; 1, none (or no server);
/// anything else (no tmux found, no status) can't tell.
fn has_session(status: Option<u32>) -> Option<bool> {
    match status {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod first_session_tests {
    use super::{
        TmuxAction, TmuxRefusal, first_session_command, first_session_name, has_session,
        has_session_command,
    };

    /// B228: whether a server has any session is tmux's own answer, asked
    /// under `sh -c` with tmux found as the control client finds it.
    #[test]
    fn whether_a_server_has_a_session_is_told_by_its_status() {
        let command = has_session_command("tmux -L x");
        assert!(command.starts_with("sh -c '"), "{command}");
        assert!(
            command.contains("tmux -L x has-session 2>/dev/null"),
            "{command}"
        );
        assert!(command.contains("$PATH:"), "{command}");
        assert_eq!(has_session(Some(0)), Some(true));
        assert_eq!(has_session(Some(1)), Some(false), "none, or no server");
        assert_eq!(has_session(Some(127)), None, "no tmux the exec finds");
        assert_eq!(has_session(None), None, "no status");
    }

    /// B250: a name with a control character in it — a pasted line break, a
    /// tab — is refused, not dropped: the first session was made anyway,
    /// with tmux's own name, and nobody said why. Blank still means tmux's
    /// name; whitespace and line breaks around a name are trimmed, as the
    /// app trims what was typed.
    #[test]
    fn a_first_session_name_with_a_control_character_is_refused_not_dropped() {
        for name in [
            "a\nb",
            "work\tlog",
            "x\u{1b}[31m",
            "\u{7f}del",
            "ab\u{85}cd",
        ] {
            assert_eq!(
                first_session_name(Some(name.to_owned())),
                Err(TmuxRefusal::NameHasControlCharacters),
                "{name:?}"
            );
        }
        assert_eq!(first_session_name(None), Ok(None));
        assert_eq!(first_session_name(Some("  \n".to_owned())), Ok(None));
        assert_eq!(
            first_session_name(Some(" work\n".to_owned())),
            Ok(Some("work".to_owned()))
        );
        assert_eq!(
            first_session_name(Some("issue #12".to_owned())),
            Ok(Some("issue #12".to_owned()))
        );
    }

    /// Every action that carries a typed name is refused the same way,
    /// before anything reaches tmux; the rest never are.
    #[test]
    fn every_typed_name_is_held_to_the_same_rule() {
        let bad = || "two\nlines".to_owned();
        let refused = [
            TmuxAction::NewSession { name: Some(bad()) },
            TmuxAction::NewWindow { name: Some(bad()) },
            TmuxAction::NewWindowInSession {
                session: "$1".to_owned(),
                name: Some(bad()),
            },
            TmuxAction::RenameSession {
                session: "$1".to_owned(),
                name: bad(),
            },
            TmuxAction::RenameWindow {
                window: "@1".to_owned(),
                name: bad(),
            },
        ];
        for action in refused {
            assert_eq!(
                action.refusal(),
                Some(TmuxRefusal::NameHasControlCharacters),
                "{action:?}"
            );
        }
        let taken = [
            TmuxAction::NewSession { name: None },
            TmuxAction::NewSession {
                name: Some("work\n".to_owned()),
            },
            TmuxAction::RenameWindow {
                window: "@1".to_owned(),
                name: "logs".to_owned(),
            },
            TmuxAction::Paste {
                pane: "%1".to_owned(),
                text: "a\nb".to_owned(),
            },
            TmuxAction::Refresh,
            TmuxAction::Relist,
        ];
        for action in taken {
            assert_eq!(action.refusal(), None, "{action:?}");
        }
    }

    #[test]
    fn the_first_session_is_detached_and_named_as_typed() {
        // Under `sh -c`, tmux on the probed PATH (B788, B850).
        let expected = moshpit_tmux::boot::exec;
        assert_eq!(
            first_session_command("tmux", None),
            expected("tmux new-session -d")
        );
        assert_eq!(
            first_session_command("/opt/tmux", Some("work")),
            expected("/opt/tmux new-session -d -s 'work'")
        );
        // A `#` stays a `#` once tmux has expanded the name.
        assert_eq!(
            first_session_command("tmux", Some("issue #12")),
            expected("tmux new-session -d -s 'issue ##12'")
        );
        assert_eq!(
            first_session_command("tmux", Some("it's")),
            expected("tmux new-session -d -s 'it'\\''s'")
        );
    }
}
