//! herdr over SSH: W2b's control client (`moshpit_herdr::client::Client`)
//! driven on the session's connection, the way `tmux_leg` drives the tmux
//! controller. One task owns the client: it feeds it what the frame
//! channel, the push channel and the exec channels deliver, the person's
//! keys and intents, and its timers; and it carries out what the client
//! asks, in order.
//!
//! - **Execs** (`Output::Exec`): each `herdr …` command on a fresh exec
//!   channel of the session's connection, on a task of its own, answered
//!   with what it printed (stdout and stderr merged, as far as it got).
//!   The client keeps its command lane — one mutation at a time, each held
//!   at most its budget, every request answered once (B7, B26, B44, B89) —
//!   and abandons what it stopped waiting for; an abandoned exec's task is
//!   cancelled here, its channel with it. At most [`EXEC_CHANNELS`] run at
//!   once (sshd's `MaxSessions` is 10 a connection, the frame and push
//!   channels and the keepalive's probe included); the rest wait in order.
//! - **The frame channel** on the terminal channel the attempt opened: the
//!   client's lines (`FrameWrite`) written one at a time in order, before
//!   anything after them is done; what the channel prints fed back.
//! - **The push channel**: a second terminal channel on the same connection
//!   (not a second connection: no second sign-in, no second Face ID, and it
//!   goes when the session's connection goes), running the python pump.
//! - **Per-pane streams**: every pane the client renders gets a byte
//!   channel of its own; frames go into the pane whose attach printed them,
//!   in order, from this task. What a pane's view must do (the switch's
//!   veil, the reveal) rides in its channel as a mark, among its bytes.
//!
//! The input rule is the client's (W2b, B185): input reaches a pane only
//! once that pane's attach has painted; input typed at the pane the person
//! just picked, and content delivered on purpose (a notification's reply, a
//! share: [`HerdrCommand::Deliver`]), waits for that first frame and follows
//! it; anything else is dropped and said ([`HerdrNote::InputDropped`], and
//! the client's notice) — herdr 0.9.5 drops input that reaches an attach
//! before it is up.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use moshpit_herdr::client::{
    Action, Client, Config, Event as ClientEvent, ExecId, ExecResult, Notice, NoticeAction, Output,
    RequestId, RestartOutcome, ViewAction,
};
use moshpit_herdr::control::{TaskOutcome, WorktreeRemoval};
use moshpit_herdr::snapshot::{AgentHook, Snapshot};
use moshpit_herdr::version::VersionReport;
use moshpit_ssh::{Event, Process, ProcessReader, ProcessWriter, PtyRequest, Ssh};
use moshpit_stream::{Input, Mark, Stream, StreamId};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::event::PaneInfo;
use crate::tmux_leg::mark;

/// How many exec channels the client may have open at once.
pub const EXEC_CHANNELS: usize = 4;

/// What the person (or the UI) asks of herdr: the client's own intents.
pub type HerdrAction = Action;

/// What a herdr request came back with ([`crate::EventKind::HerdrAnswer`]).
#[derive(Debug, Clone, PartialEq)]
pub enum HerdrAnswer {
    Task(TaskOutcome),
    Removal(WorktreeRemoval),
    Repos(Vec<String>),
    /// `answered`: herdr's list came back (an empty list then means none).
    Agents {
        names: Vec<String>,
        answered: bool,
    },
    Restart(RestartOutcome),
    /// The server's focus once the commands queued before it landed.
    Focus(Option<String>),
    /// Content for one pane went out (or couldn't reach it in time).
    Delivered(bool),
    /// The action has no answer of its own (a selection, a rename); or
    /// the session had no herdr to ask.
    Done,
}

/// The answer an intent gets when no herdr could take it (the session
/// wasn't live, the client had closed): what each waiting surface reads as
/// "couldn't".
pub fn unanswered(action: &HerdrAction) -> HerdrAnswer {
    use moshpit_herdr::control::{RemovalFailure, TaskFailure};
    match action {
        Action::StartAgentTask(_) => {
            HerdrAnswer::Task(TaskOutcome::Failed(TaskFailure::Unreachable))
        }
        Action::RemoveWorktree { .. } => {
            HerdrAnswer::Removal(WorktreeRemoval::Failed(RemovalFailure::Unreachable))
        }
        Action::DiscoverRepos => HerdrAnswer::Repos(Vec::new()),
        Action::AgentNames => HerdrAnswer::Agents {
            names: Vec::new(),
            answered: false,
        },
        Action::RestartServer | Action::NoticeAction => {
            HerdrAnswer::Restart(RestartOutcome::Failed)
        }
        Action::ConfirmFocus { .. } => HerdrAnswer::Focus(None),
        Action::DeliverInput { .. } | Action::DeliverPaste { .. } => HerdrAnswer::Delivered(false),
        _ => HerdrAnswer::Done,
    }
}

/// herdr as the chrome draws it: the tree and everything said around it.
#[derive(Debug, Clone, PartialEq)]
pub struct HerdrView {
    pub snapshot: Snapshot,
    pub agent_hooks: BTreeMap<String, AgentHook>,
    pub agent_hooks_loaded: bool,
    /// The banner and its button.
    pub notice: Option<(Notice, Option<NoticeAction>)>,
    pub refreshing: bool,
    pub restarting: bool,
    /// herdr is installed, its server isn't running: the empty state.
    pub server_not_running: bool,
    pub protocol_mismatch: Option<String>,
    pub version_skew: Option<VersionReport>,
    pub server_version: Option<String>,
    pub push_active: bool,
    pub pane_switch_in_flight: bool,
    /// The focused pane as the server last reported it, not as a pick
    /// here put it in `snapshot` already (B406).
    pub server_focus: Option<String>,
    /// Where each pane's program runs.
    pub pane_cwds: BTreeMap<String, String>,
    /// The repository a worktree workspace came from.
    pub worktree_repos: BTreeMap<String, String>,
    /// What the New agent task form offers before discovery answers.
    pub open_repo_guesses: Vec<String>,
    /// The pane the frame channel is pointed at, and the one taking input.
    pub frame_target: Option<String>,
    pub attached_pane: Option<String>,
    /// How long the backoff after a refusal is, while one runs: the
    /// banner counts it down (B490).
    pub retry_wait_ms: Option<u64>,
}

impl HerdrView {
    pub(crate) fn of(client: &Client) -> Self {
        let snapshot = client.snapshot().clone();
        let pane_cwds = snapshot
            .panes
            .keys()
            .filter_map(|pane| {
                client
                    .working_directory(pane)
                    .map(|cwd| (pane.clone(), cwd.to_owned()))
            })
            .collect();
        let worktree_repos = snapshot
            .sessions
            .keys()
            .filter_map(|session| {
                client
                    .worktree_repo(session)
                    .map(|repo| (session.clone(), repo.to_owned()))
            })
            .collect();
        Self {
            agent_hooks: client.agent_hooks().clone(),
            agent_hooks_loaded: client.agent_hooks_loaded(),
            notice: client
                .notice()
                .map(|(notice, action)| (notice.clone(), action)),
            refreshing: client.is_refreshing(),
            restarting: client.is_restarting(),
            server_not_running: client.server_not_running(),
            protocol_mismatch: client.protocol_mismatch().map(str::to_owned),
            version_skew: client.version_skew().cloned(),
            server_version: client.server_version().map(str::to_owned),
            push_active: client.push_active(),
            pane_switch_in_flight: client.pane_switch_in_flight(),
            server_focus: client.server_focus().map(str::to_owned),
            pane_cwds,
            worktree_repos,
            open_repo_guesses: client.open_repo_guesses(),
            frame_target: client.frame_target().map(str::to_owned),
            attached_pane: client.attached_pane().map(str::to_owned),
            retry_wait_ms: client.retry_wait(),
            snapshot,
        }
    }
}

/// What the session tells the herdr task.
pub(crate) enum HerdrCommand {
    /// Keys typed at `pane`'s terminal (`stream`), in order.
    Input {
        pane: String,
        stream: StreamId,
        bytes: Vec<u8>,
    },
    Paste {
        pane: String,
        text: String,
    },
    Resize {
        pane: String,
        cols: u16,
        rows: u16,
    },
    /// An intent; `token` names its answer.
    Action {
        action: HerdrAction,
        token: Option<u64>,
    },
    /// The New agent task form went away: the task asked for under
    /// `token` stops where it stands.
    CancelTask {
        token: u64,
    },
    /// Content for one pane on purpose (a notification's reply, a share):
    /// switched to and held for its first frame, answered once it went out
    /// or couldn't.
    Deliver {
        pane: String,
        bytes: Vec<u8>,
        paste: bool,
        reply: oneshot::Sender<bool>,
    },
    Background,
    Foreground,
    /// Answer once every line asked for so far has been written.
    Flush(oneshot::Sender<()>),
    /// Stop: release the attach first if `courtesy`, then answer.
    Close {
        courtesy: bool,
        done: oneshot::Sender<()>,
    },
}

/// What the herdr task tells the session.
pub(crate) enum HerdrNote {
    /// A pane got a byte channel: the session routes its input.
    Opened {
        pane: String,
        stream: Arc<Stream>,
        input: Input,
    },
    /// The panes with channels changed (or which is focused).
    Panes(Vec<PaneInfo>),
    /// The tree or what is said around it changed.
    View(Box<HerdrView>),
    FirstFrame,
    AgentHooksUpdated,
    /// Input typed at a pane whose attach isn't reading was dropped.
    InputDropped {
        stream: StreamId,
        bytes: usize,
    },
    Answer {
        token: u64,
        answer: HerdrAnswer,
    },
    /// The frame channel or the connection is gone: the session redials.
    Lost(String),
}

/// How a herdr task starts.
pub(crate) struct HerdrStart {
    pub ssh: Ssh,
    /// The frame channel's terminal (the login shell, which the boot line is
    /// typed into).
    pub pty: Process,
    pub config: Config,
    pub grid: (u16, u16),
    pub inbound: Arc<AtomicU64>,
    pub foreground: bool,
}

pub(crate) struct HerdrLeg {
    pub commands: mpsc::UnboundedSender<HerdrCommand>,
    task: JoinHandle<()>,
}

impl HerdrLeg {
    pub fn spawn(
        start: HerdrStart,
        clock: impl Fn() -> u64 + Send + Sync + 'static,
        notes: Box<dyn Fn(HerdrNote) + Send + Sync>,
    ) -> Self {
        let (commands, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(start, rx, Box::new(clock), notes));
        Self { commands, task }
    }

    pub fn abort(&self) {
        self.task.abort();
    }
}

impl Drop for HerdrLeg {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The socket herdr's server listens on for `custom_path`: its named
/// session's (`--session NAME`) when the path names one, else the default
/// one the app's pump has always read.
pub fn socket_path_for(custom_path: Option<&str>) -> String {
    let default = moshpit_herdr::client::DEFAULT_SOCKET_PATH.to_owned();
    let Some(path) = custom_path else {
        return default;
    };
    let words: Vec<&str> = path.split_whitespace().collect();
    let mut session = None;
    for (index, word) in words.iter().enumerate() {
        if let Some(name) = word.strip_prefix("--session=") {
            session = Some(name);
        } else if *word == "--session" {
            session = words.get(index + 1).copied();
        }
    }
    match session.filter(|name| {
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    }) {
        Some(name) => format!("~/.config/herdr/sessions/{name}/herdr.sock"),
        None => default,
    }
}

/// What a pane's view must do, as the mark that carries it (the tmux
/// codes: one set of marks for every terminal view).
fn view_mark(action: &ViewAction) -> Mark {
    match *action {
        ViewAction::Veil => Mark {
            code: mark::VEIL,
            arg: 0,
        },
        ViewAction::ExtendCover { seconds } => Mark {
            code: mark::EXTEND_COVER,
            arg: (seconds * 1000.0).round().clamp(0.0, i64::MAX as f64) as i64,
        },
        ViewAction::Reveal => Mark {
            code: mark::REVEAL,
            arg: 0,
        },
    }
}

/// What the task's helpers hand back.
enum Wake {
    Exec(ExecId, ExecResult),
    PushOpened(Process),
    PushFailed,
    PushBytes(Vec<u8>),
    PushClosed,
}

struct Task {
    client: Client,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    notes: Box<dyn Fn(HerdrNote) + Send + Sync>,
    ssh: Ssh,
    frame: mpsc::UnboundedSender<Write>,
    push: Option<Push>,
    wakes: mpsc::UnboundedSender<Wake>,
    execs: HashMap<ExecId, JoinHandle<()>>,
    channels: Arc<Semaphore>,
    /// The client's stream ids → this attach's byte channels.
    streams: BTreeMap<u64, (String, Arc<Stream>)>,
    panes_sent: Vec<PaneInfo>,
    view_sent: Option<HerdrView>,
    /// The client's request ids → the session's tokens.
    tokens: HashMap<RequestId, u64>,
    deliveries: HashMap<RequestId, oneshot::Sender<bool>>,
    first_frame: bool,
    /// The first frame painted in this batch: said after its panes.
    first_frame_due: bool,
    /// The frame channel is gone.
    lost: bool,
    inbound: Arc<AtomicU64>,
    /// The input being routed: a drop said in its batch is its own.
    routing: Option<(StreamId, usize)>,
}

/// The push channel: its ordered writer and its reader.
struct Push {
    writes: mpsc::UnboundedSender<Write>,
    tasks: [JoinHandle<()>; 2],
}

impl Drop for Push {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// What a frame or push writer is given, in order.
enum Write {
    Bytes(Vec<u8>),
    /// Everything before it has been written.
    Flush(oneshot::Sender<()>),
}

async fn run(
    start: HerdrStart,
    mut commands: mpsc::UnboundedReceiver<HerdrCommand>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    notes: Box<dyn Fn(HerdrNote) + Send + Sync>,
) {
    let HerdrStart {
        ssh,
        pty,
        config,
        grid,
        inbound,
        foreground,
    } = start;
    let (mut reader, writer) = pty.split();
    let (frame, frame_rx) = mpsc::unbounded_channel();
    let frame_writer = tokio::spawn(write(writer, frame_rx));
    let (wakes, mut wake_rx) = mpsc::unbounded_channel();
    let mut task = Task {
        client: Client::new(config),
        clock,
        notes,
        ssh,
        frame,
        push: None,
        wakes,
        execs: HashMap::new(),
        channels: Arc::new(Semaphore::new(EXEC_CHANNELS)),
        streams: BTreeMap::new(),
        panes_sent: Vec::new(),
        view_sent: None,
        tokens: HashMap::new(),
        deliveries: HashMap::new(),
        first_frame: false,
        first_frame_due: false,
        lost: false,
        inbound,
        routing: None,
    };
    let now = task.now();
    // The first attach line carries the grid the view starts at: the client
    // sizes a pane it has no report for by the last grid any pane reported.
    task.client
        .resize("", i64::from(grid.0), i64::from(grid.1), now);
    task.client.start(now);
    if !foreground {
        task.client.background(now);
    }
    let epoch = (Instant::now(), now);
    let mut closed = false;
    loop {
        task.pump();
        if task.lost {
            (task.notes)(HerdrNote::Lost("herdr's frame channel ended".to_owned()));
            break;
        }
        let deadline = task
            .client
            .next_deadline()
            .map(|at| epoch.0 + Duration::from_millis(at.saturating_sub(epoch.1)));
        tokio::select! {
            event = reader.next() => {
                let now = task.now();
                match event {
                    Some(Event::Stdout(bytes)) | Some(Event::Stderr(bytes)) => {
                        task.inbound.store(now, Ordering::Relaxed);
                        task.client.frame_bytes(&bytes, now);
                    }
                    Some(Event::Exited(_)) | Some(Event::Eof) => {}
                    Some(Event::Closed) | Some(Event::Lost) | None => {
                        task.client.frame_closed(now);
                        // The client says it once; this says it for certain.
                        task.lost = true;
                    }
                }
            }
            Some(wake) = wake_rx.recv() => task.wake(wake),
            command = commands.recv() => match command {
                Some(HerdrCommand::Close { courtesy, done }) => {
                    let now = task.now();
                    task.client.close(courtesy, now);
                    task.pump();
                    let (flushed, written) = oneshot::channel();
                    let _ = task.frame.send(Write::Flush(flushed));
                    let _ = tokio::time::timeout(CLOSE_BOUND, written).await;
                    let _ = done.send(());
                    closed = true;
                    break;
                }
                Some(command) => task.command(command),
                None => {
                    closed = true;
                    break;
                }
            },
            () = sleep_until(deadline) => {
                let now = task.now();
                task.client.tick(now);
            }
        }
    }
    if !closed {
        // The channel died: the session tears this attach down; its close
        // is answered without a round trip on the dead connection.
        while let Some(command) = commands.recv().await {
            if let HerdrCommand::Close { done, .. } = command {
                let _ = done.send(());
                break;
            }
        }
    }
    for (_, exec) in task.execs.drain() {
        exec.abort();
    }
    task.answer_everything();
    drop(task);
    drop(frame_writer);
}

/// How long a close waits for its release line to be written.
const CLOSE_BOUND: Duration = Duration::from_secs(2);

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
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
}

async fn read_push(mut reader: ProcessReader, wakes: mpsc::UnboundedSender<Wake>) {
    loop {
        match reader.next().await {
            Some(Event::Stdout(bytes)) | Some(Event::Stderr(bytes)) => {
                if wakes.send(Wake::PushBytes(bytes.to_vec())).is_err() {
                    return;
                }
            }
            Some(Event::Exited(_)) | Some(Event::Eof) => {}
            Some(Event::Closed) | Some(Event::Lost) | None => {
                let _ = wakes.send(Wake::PushClosed);
                return;
            }
        }
    }
}

impl Task {
    fn now(&self) -> u64 {
        (self.clock)()
    }

    /// Carries out what the client queued, in order.
    fn pump(&mut self) {
        let mut view = false;
        while let Some(output) = self.client.poll_output() {
            match output {
                Output::Exec { id, command } => self.exec(id, command),
                Output::Abandon { id } => {
                    if let Some(exec) = self.execs.remove(&id) {
                        exec.abort();
                    }
                }
                Output::FrameWrite(line) => {
                    let mut bytes = line.into_bytes();
                    bytes.push(b'\n');
                    let _ = self.frame.send(Write::Bytes(bytes));
                }
                Output::OpenPush { program } => self.open_push(program),
                Output::PushWrite(bytes) => {
                    if let Some(push) = &self.push {
                        let _ = push.writes.send(Write::Bytes(bytes));
                    }
                }
                Output::ClosePush => self.push = None,
                Output::StreamOpened { pane, stream } => {
                    if let std::collections::btree_map::Entry::Vacant(slot) =
                        self.streams.entry(stream)
                    {
                        let (channel, input) = moshpit_stream::open();
                        let channel = Arc::new(channel);
                        slot.insert((pane.clone(), channel.clone()));
                        (self.notes)(HerdrNote::Opened {
                            pane,
                            stream: channel,
                            input,
                        });
                    }
                }
                Output::StreamClosed { stream, .. } => {
                    self.streams.remove(&stream);
                }
                Output::PaneBytes { stream, bytes, .. } => {
                    if let Some((_, channel)) = self.streams.get(&stream) {
                        channel.push(&bytes);
                    }
                }
                Output::View { stream, action, .. } => {
                    if let Some((_, channel)) = self.streams.get(&stream) {
                        channel.mark(view_mark(&action));
                    }
                }
                Output::Event(event) => view |= self.event(event),
            }
        }
        let panes = self.panes();
        if panes != self.panes_sent {
            self.panes_sent = panes.clone();
            (self.notes)(HerdrNote::Panes(panes));
        }
        if std::mem::take(&mut self.first_frame_due) {
            (self.notes)(HerdrNote::FirstFrame);
        }
        if view || self.view_sent.is_none() {
            let current = HerdrView::of(&self.client);
            if self.view_sent.as_ref() != Some(&current) {
                self.view_sent = Some(current.clone());
                (self.notes)(HerdrNote::View(Box::new(current)));
            }
        }
    }

    /// One client event; whether the view may have changed.
    fn event(&mut self, event: ClientEvent) -> bool {
        match event {
            ClientEvent::StateChanged
            | ClientEvent::FocusedPane(_)
            | ClientEvent::PushActive(_) => {
                return true;
            }
            ClientEvent::AgentHooksUpdated => {
                (self.notes)(HerdrNote::AgentHooksUpdated);
                return true;
            }
            ClientEvent::FirstFrame => {
                // Said once the panes it painted are: after this batch's.
                if !self.first_frame {
                    self.first_frame = true;
                    self.first_frame_due = true;
                }
            }
            ClientEvent::FrameChannelLost => self.lost = true,
            ClientEvent::InputDropped { pane } => {
                let (stream, bytes) = self
                    .routing
                    .take()
                    .unwrap_or_else(|| (self.stream_of(&pane).unwrap_or(0), 0));
                (self.notes)(HerdrNote::InputDropped { stream, bytes });
            }
            ClientEvent::CommandFailed { command, reason } => {
                log::info!("herdr: `{command}` failed: {}", reason.english());
            }
            ClientEvent::InputIgnored(message) => {
                log::info!("herdr: a line was turned away: {message}");
            }
            ClientEvent::Task { request, outcome } => {
                self.answer(request, HerdrAnswer::Task(outcome))
            }
            ClientEvent::Removal { request, outcome } => {
                self.answer(request, HerdrAnswer::Removal(outcome))
            }
            ClientEvent::Repos { request, repos } => {
                self.answer(request, HerdrAnswer::Repos(repos))
            }
            ClientEvent::Agents {
                request,
                names,
                answered,
            } => self.answer(request, HerdrAnswer::Agents { names, answered }),
            ClientEvent::Restart { request, outcome } => {
                self.answer(request, HerdrAnswer::Restart(outcome))
            }
            ClientEvent::FocusConfirmed { request, pane } => {
                self.answer(request, HerdrAnswer::Focus(pane))
            }
            ClientEvent::Delivered { request, delivered } => {
                if let Some(reply) = self.deliveries.remove(&request) {
                    let _ = reply.send(delivered);
                }
                self.answer(request, HerdrAnswer::Delivered(delivered));
            }
        }
        false
    }

    fn stream_of(&self, pane: &str) -> Option<StreamId> {
        self.streams
            .values()
            .find(|(name, _)| name == pane)
            .map(|(_, channel)| channel.id())
    }

    fn panes(&self) -> Vec<PaneInfo> {
        self.client
            .panes()
            .into_iter()
            .filter_map(|(pane, stream, active)| {
                self.streams.get(&stream).map(|(_, channel)| PaneInfo {
                    pane,
                    stream: channel.id(),
                    active,
                })
            })
            .collect()
    }

    fn answer(&mut self, request: RequestId, answer: HerdrAnswer) {
        if let Some(token) = self.tokens.remove(&request) {
            (self.notes)(HerdrNote::Answer { token, answer });
        }
    }

    /// The task is over: whoever still waits hears no.
    fn answer_everything(&mut self) {
        for (_, reply) in self.deliveries.drain() {
            let _ = reply.send(false);
        }
        for (_, token) in self.tokens.drain() {
            (self.notes)(HerdrNote::Answer {
                token,
                answer: HerdrAnswer::Done,
            });
        }
    }

    fn exec(&mut self, id: ExecId, command: String) {
        let ssh = self.ssh.clone();
        let wakes = self.wakes.clone();
        let channels = self.channels.clone();
        let task = tokio::spawn(async move {
            let _permit = channels.acquire_owned().await;
            let result = match ssh.run(&command).await {
                Ok(output) => ExecResult::Output(output.merged()),
                Err(error) => ExecResult::Failed(error.to_string()),
            };
            let _ = wakes.send(Wake::Exec(id, result));
        });
        self.execs.insert(id, task);
    }

    /// The push channel: a terminal whose program is the pump (B267: never
    /// typed into a login shell still starting).
    fn open_push(&mut self, program: String) {
        self.push = None;
        let ssh = self.ssh.clone();
        let wakes = self.wakes.clone();
        tokio::spawn(async move {
            let mut request = PtyRequest::new(80, 24);
            request.command = Some(program);
            let wake = match ssh.terminal(&request).await {
                Ok(process) => Wake::PushOpened(process),
                Err(_) => Wake::PushFailed,
            };
            let _ = wakes.send(wake);
        });
    }

    fn wake(&mut self, wake: Wake) {
        let now = self.now();
        match wake {
            Wake::Exec(id, result) => {
                self.execs.remove(&id);
                if matches!(result, ExecResult::Output(_)) {
                    self.inbound.store(now, Ordering::Relaxed);
                }
                self.client.exec_finished(id, result, now);
            }
            Wake::PushOpened(process) => {
                let (reader, writer) = process.split();
                let (writes, rx) = mpsc::unbounded_channel();
                self.push = Some(Push {
                    writes,
                    tasks: [
                        tokio::spawn(write(writer, rx)),
                        tokio::spawn(read_push(reader, self.wakes.clone())),
                    ],
                });
                self.client.push_opened(now);
            }
            Wake::PushFailed => self.client.push_failed(now),
            Wake::PushBytes(bytes) => {
                self.inbound.store(now, Ordering::Relaxed);
                self.client.push_bytes(&bytes, now);
            }
            Wake::PushClosed => {
                self.push = None;
                self.client.push_closed(now);
            }
        }
    }

    fn command(&mut self, command: HerdrCommand) {
        let now = self.now();
        match command {
            HerdrCommand::Input {
                pane,
                stream,
                bytes,
            } => {
                self.routing = Some((stream, bytes.len()));
                self.client.input(&pane, &bytes, now);
                // A drop said in this batch is this input's.
                self.pump();
                self.routing = None;
            }
            HerdrCommand::Paste { pane, text } => self.client.paste(&pane, &text, now),
            HerdrCommand::Resize { pane, cols, rows } => {
                self.client
                    .resize(&pane, i64::from(cols), i64::from(rows), now)
            }
            HerdrCommand::Action { action, token } => {
                let fallback = unanswered(&action);
                let request = self.client.act(action, now);
                match (request, token) {
                    (Some(request), Some(token)) => {
                        self.tokens.insert(request, token);
                    }
                    (None, Some(token)) => (self.notes)(HerdrNote::Answer {
                        token,
                        answer: fallback,
                    }),
                    _ => {}
                }
            }
            HerdrCommand::Deliver {
                pane,
                bytes,
                paste,
                reply,
            } => {
                let action = if paste {
                    Action::DeliverPaste {
                        pane,
                        text: String::from_utf8_lossy(&bytes).into_owned(),
                    }
                } else {
                    Action::DeliverInput { pane, bytes }
                };
                match self.client.act(action, now) {
                    Some(request) => {
                        self.deliveries.insert(request, reply);
                    }
                    None => {
                        let _ = reply.send(false);
                    }
                }
            }
            HerdrCommand::CancelTask { token } => {
                if let Some(request) = self
                    .tokens
                    .iter()
                    .find(|(_, t)| **t == token)
                    .map(|(request, _)| *request)
                {
                    self.client.act(Action::CancelAgentTask(request), now);
                }
            }
            HerdrCommand::Background => self.client.background(now),
            HerdrCommand::Foreground => self.client.foreground(now),
            HerdrCommand::Flush(done) => {
                let _ = self.frame.send(Write::Flush(done));
            }
            HerdrCommand::Close { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_push_pump_finds_a_named_sessions_socket() {
        assert_eq!(socket_path_for(None), "~/.config/herdr/herdr.sock");
        assert_eq!(
            socket_path_for(Some("/opt/homebrew/bin/herdr")),
            "~/.config/herdr/herdr.sock"
        );
        assert_eq!(
            socket_path_for(Some("/opt/homebrew/bin/herdr --session work-1")),
            "~/.config/herdr/sessions/work-1/herdr.sock"
        );
        assert_eq!(
            socket_path_for(Some("herdr --session=lab")),
            "~/.config/herdr/sessions/lab/herdr.sock"
        );
        // A name the pump's line can't carry stays on the default.
        assert_eq!(
            socket_path_for(Some("herdr --session it's")),
            "~/.config/herdr/herdr.sock"
        );
    }

    #[test]
    fn an_intent_nobody_could_take_is_answered_as_couldnt() {
        use moshpit_herdr::control::{AgentTaskRequest, RemovalFailure, TaskFailure};
        let request = AgentTaskRequest {
            repo_path: "/r".into(),
            branch: "b".into(),
            agent: "claude".into(),
            prompt: String::new(),
        };
        assert_eq!(
            unanswered(&Action::StartAgentTask(request)),
            HerdrAnswer::Task(TaskOutcome::Failed(TaskFailure::Unreachable))
        );
        assert_eq!(
            unanswered(&Action::RemoveWorktree {
                session: "w".into(),
                force: false
            }),
            HerdrAnswer::Removal(WorktreeRemoval::Failed(RemovalFailure::Unreachable))
        );
        assert_eq!(
            unanswered(&Action::AgentNames),
            HerdrAnswer::Agents {
                names: vec![],
                answered: false
            }
        );
        assert_eq!(
            unanswered(&Action::DeliverInput {
                pane: "p".into(),
                bytes: b"y".to_vec()
            }),
            HerdrAnswer::Delivered(false)
        );
        assert_eq!(
            unanswered(&Action::ConfirmFocus { budget_ms: 1 }),
            HerdrAnswer::Focus(None)
        );
        assert_eq!(unanswered(&Action::NewPane), HerdrAnswer::Done);
    }

    #[test]
    fn a_views_marks_are_the_tmux_codes() {
        assert_eq!(
            view_mark(&ViewAction::Veil),
            Mark {
                code: mark::VEIL,
                arg: 0
            }
        );
        assert_eq!(
            view_mark(&ViewAction::ExtendCover { seconds: 1.25 }),
            Mark {
                code: mark::EXTEND_COVER,
                arg: 1250
            }
        );
        assert_eq!(
            view_mark(&ViewAction::Reveal),
            Mark {
                code: mark::REVEAL,
                arg: 0
            }
        );
    }
}
