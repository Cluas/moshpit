//! herdr over mosh (W10c): mosh renders herdr — the raw-attach loop
//! (`moshpit_herdr::launch::raw_attach_loop_command`) showing one pane, or
//! herdr's own TUI on a host whose herdr has no `terminal attach` — and
//! herdr's control client runs beside it on an SSH connection of its own
//! (first the bootstrap's, kept, as mosh + tmux's sidecar is), steering the
//! loop with execs ([`moshpit_herdr::renderer`]).
//!
//! [`Plane`] is the whole of it as a sans-I/O state machine on the caller's
//! clock: W2b's [`Client`] (no frame channel: mosh renders) and round
//! twenty-seven's [`Renderer`], wired the way the Swift path wires them in
//! `SessionHub`, plus what sits between them and mosh:
//!
//! - **The focus**: every read of the tree reports the server's focused
//!   pane, and the loop follows it — a plain attach, which herdr refuses
//!   while another device holds the pane ([`Renderer::show`]). The person's
//!   pick of a pane (a tap, a reply for it) attaches with `--takeover`
//!   ([`Renderer::pick`]): the plane marks the pane it is picking before the
//!   client reports the focus it just asked for.
//! - **One ordered lane into mosh** ([`Write`]): what the person types, the
//!   keys herdr encodes for the pane (arrows go as `pane send-keys`: herdr's
//!   `terminal attach` repaints the pane and passes no DECCKM to mosh, and
//!   hands keys to the pane as they come — measured on herdr 0.9.5: an `ESC
//!   [ A` typed through the attach reaches a pane in application cursor
//!   mode as `ESC [ A`), the start line typed again, a reply's bytes. A key
//!   and a retarget (which kills the attach the typing before it went to)
//!   wait — bounded — for mosh-server to have acknowledged everything sent
//!   before them ([`PlaneOut::Barrier`]): mosh's datagrams and the sidecar's
//!   exec are two networks, and a keystroke still in flight when the attach
//!   is bounced reached the pane switched to.
//! - **Typing** goes only while the pane asked for is on screen by the
//!   host's word ([`Renderer::typed`]); otherwise it is dropped and said —
//!   never into the pane being left, never into a login shell.
//! - **Content for one pane** (a notification's reply, a share): waits
//!   (bounded) for the tree to list the pane (B248), focuses it on the
//!   server and waits for the server to confirm it, puts the loop on it
//!   ([`Renderer::deliver`]: on the loop's word, or a check that it is
//!   still there), sends, and says sent only once mosh-server has
//!   acknowledged the bytes — never on their merely leaving the phone: a
//!   datagram the path drops is retransmitted only for the keystroke
//!   lifetime, and the path can be dead.
//! - **The notice**: the renderer's (a pane on another device with "Open
//!   here", herdr's refusal or the loop gone with "Retry", a one-off "not
//!   attached" or "couldn't switch") over the client's; its button takes the
//!   pane back ([`Renderer::take_back`]) or boots the loop again.
//! - **The veil**: a switch's veil and its lifting (once the new pane's
//!   repaint has gone quiet) ride the shell's byte channel as marks.
//!
//! [`HerdrMoshLeg`] carries it out: execs on the control connection (held
//! while it is being redialled), the push channel, mosh's lane, the marks.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use moshpit_herdr::client::{
    Action, Client, Config, Event as ClientEvent, ExecId as ClientExecId, ExecResult, Notice,
    NoticeAction, Output as ClientOutput, RequestId,
};
use moshpit_herdr::renderer::{
    Budgets as RendererBudgets, DeliveryId, Down, ExecId as RendererExecId,
    Notice as RendererNotice, Output as RendererOutput, Renderer,
};
use moshpit_herdr::{control, frame, launch};
use moshpit_ssh::{Event, Process, PtyRequest, Ssh};
use moshpit_stream::{Mark, Stream};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::herdr::{HerdrAction, HerdrAnswer, HerdrView, unanswered};
use crate::mosh_udp::MoshCommand;
use crate::tmux_leg::mark;

/// Every wait the plane adds to the client's and the renderer's, in
/// milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaneBudgets {
    /// A reply waits this long for the tree to list its pane (B248's 10 s:
    /// the control plane comes up a moment after mosh, and is redialled
    /// while mosh carries on).
    pub tree: u64,
    /// The server's confirmation that a reply's pane is focused
    /// (`herdrFocusBudget`, 5 s).
    pub focus: u64,
    /// How long a reply's bytes wait for mosh-server's acknowledgement
    /// before they count as not sent.
    pub ack: u64,
    /// How long a key or a retarget waits for the typing before it to be
    /// acknowledged before it goes anyway (`moshLandingBudget`, 1.5 s).
    pub landing: u64,
    /// How long one key's exec holds the typing behind it
    /// (`keySendBudget`, 3 s).
    pub key_hold: u64,
    /// A one-off notice's life (4 s, `flashHerdrNotice`).
    pub flash: u64,
    /// The switch veil's cap, told to the view with the veil.
    pub cover_cap: u64,
}

impl Default for PlaneBudgets {
    fn default() -> Self {
        Self {
            tree: 10_000,
            focus: 5_000,
            ack: 3_000,
            landing: 1_500,
            key_hold: 3_000,
            flash: 4_000,
            cover_cap: RendererBudgets::default().cover_cap,
        }
    }
}

/// How the plane starts.
#[derive(Debug, Clone)]
pub struct PlaneConfig {
    /// The client's config (`frame_channel` off; `immersive_zoom` with the
    /// full TUI).
    pub client: Config,
    /// The raw-attach loop's key ([`launch::mosh_renderer_key`]); `None`:
    /// mosh shows herdr's own TUI (a herdr without `terminal attach`).
    pub renderer_key: Option<String>,
    pub renderer: RendererBudgets,
    pub budgets: PlaneBudgets,
}

impl PlaneConfig {
    /// The app's: the connection's herdr path, the loop's key (or the TUI),
    /// push when the host has python3.
    pub fn new(
        connection_id: &str,
        custom_path: Option<String>,
        renderer_key: Option<String>,
        push: bool,
    ) -> Self {
        Self {
            client: Config {
                socket_path: crate::herdr::socket_path_for(custom_path.as_deref()),
                custom_path,
                frame_channel: false,
                immersive_zoom: renderer_key.is_none(),
                push,
                connection_id: connection_id.to_owned(),
                ..Config::default()
            },
            renderer_key,
            renderer: RendererBudgets::default(),
            budgets: PlaneBudgets::default(),
        }
    }
}

/// An exec the plane asked for, by whose it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExecTag {
    Client(u64),
    Renderer(u64),
    /// A key run's `pane send-keys`.
    Key(u64),
}

/// What the plane asks of its host, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaneOut {
    /// Run `command` on a fresh exec channel of the control connection
    /// (merged output, or the failure to run it): [`Plane::exec_finished`].
    Exec {
        tag: ExecTag,
        command: String,
    },
    /// Nobody waits for this exec any more.
    Abandon {
        tag: ExecTag,
    },
    /// The push channel (a terminal on the control connection, 80×24, the
    /// pump `program` its program): [`Plane::push_opened`] or
    /// [`Plane::push_failed`].
    OpenPush {
        program: String,
    },
    PushWrite(Vec<u8>),
    ClosePush,
    /// Bytes into the mosh session, in order with everything before.
    /// `end`: where they end in what the view wrote into the shell's
    /// channel (mosh's echo acknowledgement counts in it); empty bytes say
    /// that everything up to `end` will never be echoed.
    Mosh {
        bytes: Vec<u8>,
        end: u64,
    },
    /// Answer [`Plane::acked`] once mosh-server has acknowledged everything
    /// sent so far ([`MoshCommand::Barrier`]).
    Barrier(u64),
    /// A mark for the shell's view (the switch veil and its lifting).
    Mark(Mark),
    /// A delivery's answer.
    Delivered {
        token: u64,
        sent: bool,
    },
    /// An intent's answer.
    Answer {
        token: u64,
        answer: HerdrAnswer,
    },
    /// Typing that went nowhere (bytes).
    InputDropped {
        bytes: usize,
    },
    AgentHooksUpdated,
}

/// One step of the lane into mosh.
#[derive(Debug, Clone, PartialEq)]
enum Write {
    Text {
        bytes: Vec<u8>,
        end: u64,
    },
    /// Nothing will be echoed up to `end`.
    Skip {
        end: u64,
    },
    /// Wait (bounded) for mosh-server to have acknowledged everything sent
    /// before, then do this; the lane waits too.
    Fence(Fenced),
    /// Ask for mosh-server's acknowledgement of everything sent so far
    /// (this barrier; `Plane::acks` says what for); the lane goes on.
    Ack(u64),
}

#[derive(Debug, Clone, PartialEq)]
enum Fenced {
    /// A retarget: the attach it kills read everything typed before it.
    Exec { tag: ExecTag, command: String },
    /// Keys herdr encodes for `pane`; the typing after them waits for the
    /// exec.
    Keys {
        pane: String,
        keys: Vec<&'static str>,
        end: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AckFor {
    Delivery(u64),
    /// A retyped start line: the renderer asks about the loop once it is in
    /// the tty.
    Retype,
}

/// The lane's head, waiting.
#[derive(Debug)]
struct Blocked {
    then: Fenced,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The acknowledgement of what was sent before.
    Landing { barrier: u64, deadline: u64 },
    /// The keys' exec.
    Keys { exec: u64, deadline: u64 },
}

/// A delivery's way to its pane.
#[derive(Debug)]
struct Pending {
    token: u64,
    pane: String,
    bytes: Vec<u8>,
    /// Its whole wait, from the session.
    deadline: u64,
    stage: Stage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// For the tree to list the pane, until then.
    Tree { until: u64 },
    /// For the server's focus. `again`: the pane looked focused already, so
    /// nothing was asked; asked once more if the server says otherwise.
    Focus { request: RequestId, again: bool },
    /// For the renderer to put the loop on the pane and send.
    Renderer(DeliveryId),
    /// For mosh's acknowledgement of the bytes, until then.
    Ack { barrier: u64, until: u64 },
}

/// The notice the plane puts up over the client's.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Said {
    notice: Notice,
    action: Option<NoticeAction>,
    /// A one-off's end; `None` until the renderer takes it down.
    until: Option<u64>,
}

/// herdr over mosh. See the module docs.
#[derive(Debug)]
pub struct Plane {
    client: Client,
    renderer: Option<Renderer>,
    budgets: PlaneBudgets,
    now: u64,
    outputs: VecDeque<PlaneOut>,
    /// The person's pick of this pane is on its way through the client.
    picking: Option<String>,
    /// The terminals last told to the renderer, and whether a server ran.
    told: Option<(BTreeSet<String>, bool)>,
    /// The session's requests: the client's ids → the session's tokens.
    tokens: HashMap<RequestId, u64>,
    pending: BTreeMap<u64, Pending>,
    next_pending: u64,
    renderer_deliveries: HashMap<DeliveryId, u64>,
    next_delivery: u64,
    lane: VecDeque<Write>,
    blocked: Option<Blocked>,
    /// Acknowledgements asked for outside the lane's head.
    acks: HashMap<u64, AckFor>,
    next_barrier: u64,
    next_key: u64,
    /// The view's offset after the last typing it handed over.
    end: u64,
    said: Option<Said>,
    /// The paint stamp last given to the renderer.
    painted: u64,
    dirty: bool,
    /// Deliveries are being moved on from the tree (asking for the focus
    /// drains the client, which would move them on again).
    advancing: bool,
}

impl Plane {
    pub fn new(config: PlaneConfig) -> Self {
        let custom = config.client.custom_path.clone();
        let renderer = config
            .renderer_key
            .map(|key| Renderer::with_budgets(key, custom.as_deref(), config.renderer));
        Self {
            client: Client::new(config.client),
            renderer,
            budgets: config.budgets,
            now: 0,
            outputs: VecDeque::new(),
            picking: None,
            told: None,
            tokens: HashMap::new(),
            pending: BTreeMap::new(),
            next_pending: 0,
            renderer_deliveries: HashMap::new(),
            next_delivery: 0,
            lane: VecDeque::new(),
            blocked: None,
            acks: HashMap::new(),
            next_barrier: 0,
            next_key: 0,
            end: 0,
            said: None,
            painted: 0,
            dirty: true,
            advancing: false,
        }
    }

    // MARK: - Reading

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn renderer(&self) -> Option<&Renderer> {
        self.renderer.as_ref()
    }

    /// The pane a terminal belongs to, by the last tree.
    fn pane_of(&self, terminal: &str) -> Option<String> {
        self.client
            .terminal_ids()
            .iter()
            .find(|(_, t)| t.as_str() == terminal)
            .map(|(pane, _)| pane.clone())
    }

    /// herdr as the chrome draws it: the client's, with the renderer's
    /// notice over the client's, its switch in the "switching" flag, and the
    /// panes its loop is asked to show and shows.
    pub fn view(&self) -> HerdrView {
        let mut view = HerdrView::of(&self.client);
        if let Some(said) = self.shown() {
            view.notice = Some((said.notice.clone(), said.action));
        }
        if let Some(renderer) = &self.renderer {
            view.pane_switch_in_flight |= renderer.switching();
            view.frame_target = renderer.target().and_then(|t| self.pane_of(t));
            view.attached_pane = renderer
                .on_screen()
                .filter(|_| renderer.down().is_none())
                .and_then(|t| self.pane_of(t));
        }
        view
    }

    /// Something the chrome reads changed since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn poll_output(&mut self) -> Option<PlaneOut> {
        self.outputs.pop_front()
    }

    pub fn next_deadline(&self) -> Option<u64> {
        let mut deadlines = vec![self.client.next_deadline()];
        if let Some(renderer) = &self.renderer {
            deadlines.push(renderer.next_deadline());
        }
        deadlines.push(self.blocked.as_ref().map(|blocked| match blocked.phase {
            Phase::Landing { deadline, .. } | Phase::Keys { deadline, .. } => deadline,
        }));
        deadlines.push(self.said.as_ref().and_then(|said| said.until));
        // A delivery the client or the renderer holds is theirs to bound:
        // given up here, it could still go out after its "not sent".
        deadlines.extend(self.pending.values().map(|pending| match pending.stage {
            Stage::Tree { until } | Stage::Ack { until, .. } => Some(until.min(pending.deadline)),
            Stage::Focus { .. } | Stage::Renderer(_) => None,
        }));
        deadlines.into_iter().flatten().min()
    }

    // MARK: - Inputs

    fn begin(&mut self, now: u64) {
        self.now = now.max(self.now);
    }

    /// Start: the first read now, the push channel if configured.
    pub fn start(&mut self, now: u64) {
        self.begin(now);
        self.client.start(self.now);
        self.settle();
    }

    pub fn foreground(&mut self, now: u64) {
        self.begin(now);
        self.client.foreground(self.now);
        self.settle();
    }

    pub fn background(&mut self, now: u64) {
        self.begin(now);
        self.client.background(self.now);
        self.settle();
    }

    /// A fresh control connection is up: read now, and push again if the
    /// last connection took the push channel with it.
    pub fn connection_up(&mut self, now: u64) {
        self.begin(now);
        self.client.read_now(self.now);
        self.client.connection_restored(self.now);
        self.settle();
    }

    /// The control connection is gone, and the push channel that rode it
    /// (B210): the client stops counting on push now, not 45 s of a safety
    /// net later, and opens it again on the next connection.
    pub fn connection_lost(&mut self, now: u64) {
        self.begin(now);
        self.client.push_connection_lost(self.now);
        self.settle();
    }

    /// Bytes reached the screen at `stamp` (the session's clock): the
    /// switch veil's only progress signal.
    pub fn painted(&mut self, stamp: u64) {
        if stamp > self.painted {
            self.painted = stamp;
            if let Some(renderer) = &mut self.renderer {
                renderer.painted(stamp);
            }
        }
    }

    pub fn tick(&mut self, now: u64) {
        self.begin(now);
        self.client.tick(self.now);
        self.drain_client();
        if let Some(renderer) = &mut self.renderer {
            renderer.tick(self.now);
        }
        self.drain_renderer();
        // The lane's head: a fence that waited its bound goes anyway; a key
        // run that held the typing its bound lets it go.
        if let Some(blocked) = &self.blocked {
            match blocked.phase {
                Phase::Landing { deadline, .. } if self.now >= deadline => self.fence_landed(),
                Phase::Keys { deadline, .. } if self.now >= deadline => {
                    self.keys_done();
                }
                _ => {}
            }
        }
        if self
            .said
            .as_ref()
            .and_then(|said| said.until)
            .is_some_and(|until| self.now >= until)
        {
            self.said = None;
            self.dirty = true;
        }
        self.expire_deliveries();
        self.settle();
    }

    /// An exec the plane asked for came back (`None`: it couldn't run).
    pub fn exec_finished(&mut self, tag: ExecTag, output: Option<Vec<u8>>, now: u64) {
        self.begin(now);
        match tag {
            ExecTag::Client(id) => {
                let result = match output {
                    Some(bytes) => ExecResult::Output(bytes),
                    None => ExecResult::Failed("the control connection is gone".to_owned()),
                };
                self.client
                    .exec_finished(ClientExecId(id), result, self.now);
            }
            ExecTag::Renderer(id) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.exec_finished(RendererExecId(id), output.as_deref(), self.now);
                }
            }
            ExecTag::Key(id) => {
                if matches!(
                    self.blocked,
                    Some(Blocked { phase: Phase::Keys { exec, .. }, .. }) if exec == id
                ) {
                    self.keys_done();
                }
            }
        }
        self.settle();
    }

    pub fn push_opened(&mut self, now: u64) {
        self.begin(now);
        self.client.push_opened(self.now);
        self.settle();
    }

    pub fn push_failed(&mut self, now: u64) {
        self.begin(now);
        self.client.push_failed(self.now);
        self.settle();
    }

    pub fn push_bytes(&mut self, bytes: &[u8], now: u64) {
        self.begin(now);
        self.client.push_bytes(bytes, self.now);
        self.settle();
    }

    pub fn push_closed(&mut self, now: u64) {
        self.begin(now);
        self.client.push_closed(self.now);
        self.settle();
    }

    /// mosh-server acknowledged everything sent before barrier `id`
    /// (`acked`), or the loop ended without it.
    pub fn acked(&mut self, id: u64, acked: bool, now: u64) {
        self.begin(now);
        if let Some(Blocked {
            phase: Phase::Landing { barrier, .. },
            ..
        }) = &self.blocked
            && *barrier == id
        {
            self.fence_landed();
        } else if let Some(what) = self.acks.remove(&id) {
            match what {
                AckFor::Retype => {
                    if acked && let Some(renderer) = &mut self.renderer {
                        renderer.retyped(self.now);
                    }
                }
                AckFor::Delivery(key) => {
                    if let Some(pending) = self.pending.get(&key)
                        && matches!(pending.stage, Stage::Ack { barrier, .. } if barrier == id)
                    {
                        self.answer(key, acked);
                    }
                }
            }
        }
        self.settle();
    }

    /// Keys typed at the shell's view, which end at `end` in what it wrote.
    pub fn typed(&mut self, bytes: &[u8], end: u64, now: u64) {
        self.begin(now);
        self.end = self.end.max(end);
        if bytes.is_empty() {
            return;
        }
        let sent = match self.renderer.as_mut() {
            Some(renderer) => {
                // What typed() says is all its own: the queue was drained
                // after the last input.
                let allowed = renderer.typed(bytes, self.now);
                let mut said = Vec::new();
                while let Some(output) = renderer.poll_output() {
                    said.push(output);
                }
                for output in said {
                    match output {
                        // What it lets through goes below, its arrows as
                        // keys herdr encodes.
                        RendererOutput::Send(_) => {}
                        other => self.renderer_output(other),
                    }
                }
                if allowed {
                    let pane = renderer_pane(&self.client, self.renderer.as_ref());
                    self.queue_typed(pane, bytes, end);
                }
                allowed
            }
            None => {
                // herdr's own TUI types into the server's focused pane: not
                // while a switch hasn't landed (it would reach the pane being
                // left).
                if self.client.pane_switch_in_flight() {
                    self.flash(Notice::InputDropped);
                    false
                } else {
                    let pane = self.client.snapshot().active_pane_id.clone();
                    self.queue_typed(pane, bytes, end);
                    true
                }
            }
        };
        if !sent {
            self.lane.push_back(Write::Skip { end });
            self.outputs
                .push_back(PlaneOut::InputDropped { bytes: bytes.len() });
        }
        self.settle();
    }

    /// A paste into the pane on screen: one bracketed paste, under the same
    /// rule as typing.
    pub fn paste(&mut self, text: &str, end: u64, now: u64) {
        if text.is_empty() {
            return;
        }
        let payload = frame::bracketed_paste(text);
        // Never a key: an arrow inside a bracketed paste is pasted text.
        self.typed(&payload, end, now);
    }

    /// The person's (or the UI's) intent; `token` names its answer.
    pub fn act(&mut self, action: HerdrAction, token: Option<u64>, now: u64) {
        self.begin(now);
        match action {
            Action::NoticeAction if self.shown().is_some_and(|s| s.action.is_some()) => {
                self.take_back();
                self.answered(token, HerdrAnswer::Done);
            }
            Action::SelectPane(pane) => {
                self.picking = Some(pane.clone());
                self.client.act(Action::SelectPane(pane), self.now);
                self.drain_client();
                self.picking = None;
                self.answered(token, HerdrAnswer::Done);
            }
            // The frame channel's: nothing of the kind runs over mosh (the
            // view scrolls and clicks through mosh itself).
            Action::Scroll { .. }
            | Action::Mouse { .. }
            | Action::DeliverInput { .. }
            | Action::DeliverPaste { .. } => {
                let answer = unanswered(&action);
                self.answered(token, answer);
            }
            action => {
                let fallback = unanswered(&action);
                match (self.client.act(action, self.now), token) {
                    (Some(request), Some(token)) => {
                        self.tokens.insert(request, token);
                    }
                    (None, Some(token)) => self.outputs.push_back(PlaneOut::Answer {
                        token,
                        answer: fallback,
                    }),
                    _ => {}
                }
            }
        }
        self.settle();
    }

    /// The New agent task form went away: the task asked for under `token`
    /// stops where it stands.
    pub fn cancel_task(&mut self, token: u64, now: u64) {
        self.begin(now);
        if let Some(request) = self
            .tokens
            .iter()
            .find(|(_, t)| **t == token)
            .map(|(request, _)| *request)
        {
            self.client.act(Action::CancelAgentTask(request), self.now);
        }
        self.settle();
    }

    /// Content for `pane` on purpose (a notification's reply, a share),
    /// answered by [`PlaneOut::Delivered`] under `token` — sent only once
    /// mosh-server has it; waiting at most until `deadline`.
    pub fn deliver(
        &mut self,
        token: u64,
        pane: &str,
        bytes: Vec<u8>,
        paste: bool,
        deadline: u64,
        now: u64,
    ) {
        self.begin(now);
        let bytes = if paste {
            frame::bracketed_paste(&String::from_utf8_lossy(&bytes))
        } else {
            bytes
        };
        if bytes.is_empty() {
            self.outputs
                .push_back(PlaneOut::Delivered { token, sent: false });
            return;
        }
        self.next_pending += 1;
        let key = self.next_pending;
        self.pending.insert(
            key,
            Pending {
                token,
                pane: pane.to_owned(),
                bytes,
                deadline,
                stage: Stage::Tree {
                    until: self.now + self.budgets.tree,
                },
            },
        );
        self.advance_tree_waits();
        self.settle();
    }

    /// Everything waited on is answered no; nothing more is asked.
    pub fn close(&mut self, now: u64) {
        self.begin(now);
        self.client.close(false, self.now);
        let keys: Vec<u64> = self.pending.keys().copied().collect();
        for key in keys {
            self.answer(key, false);
        }
        self.drain_client();
    }

    // MARK: - Carrying out

    /// After every input: what the client and the renderer said, the lane.
    fn settle(&mut self) {
        self.drain_client();
        self.drain_renderer();
        self.pump_lane();
    }

    fn drain_client(&mut self) {
        let mut outputs = Vec::new();
        while let Some(output) = self.client.poll_output() {
            outputs.push(output);
        }
        if outputs.is_empty() {
            return;
        }
        self.dirty = true;
        // The renderer reads the tree before the focus it names.
        self.tell_tree();
        for output in outputs {
            match output {
                ClientOutput::Exec { id, command } => self.outputs.push_back(PlaneOut::Exec {
                    tag: ExecTag::Client(id.0),
                    command,
                }),
                ClientOutput::Abandon { id } => self.outputs.push_back(PlaneOut::Abandon {
                    tag: ExecTag::Client(id.0),
                }),
                ClientOutput::OpenPush { program } => {
                    self.outputs.push_back(PlaneOut::OpenPush { program })
                }
                ClientOutput::PushWrite(bytes) => {
                    self.outputs.push_back(PlaneOut::PushWrite(bytes))
                }
                ClientOutput::ClosePush => self.outputs.push_back(PlaneOut::ClosePush),
                // No frame channel over mosh: none of these come.
                ClientOutput::FrameWrite(_)
                | ClientOutput::StreamOpened { .. }
                | ClientOutput::StreamClosed { .. }
                | ClientOutput::PaneBytes { .. }
                | ClientOutput::View { .. } => {}
                ClientOutput::Event(event) => self.client_event(event),
            }
        }
        self.advance_tree_waits();
    }

    /// The tree, for the renderer, when it changed (and once one was read).
    fn tell_tree(&mut self) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let up = !self.client.server_not_running();
        if up && !self.client.snapshot().is_attached {
            return;
        }
        let listed: BTreeSet<String> = self.client.terminal_ids().values().cloned().collect();
        let current = Some((listed.clone(), up));
        if self.told != current {
            self.told = current;
            renderer.tree(listed, up, self.now);
        }
    }

    fn client_event(&mut self, event: ClientEvent) {
        match event {
            ClientEvent::FocusedPane(pane) => {
                let terminal = self.client.terminal_ids().get(&pane).cloned();
                if let (Some(renderer), Some(terminal)) = (&mut self.renderer, terminal) {
                    if self.picking.as_deref() == Some(pane.as_str()) {
                        renderer.pick(&terminal, self.now);
                    } else {
                        renderer.show(&terminal, self.now);
                    }
                }
            }
            ClientEvent::AgentHooksUpdated => self.outputs.push_back(PlaneOut::AgentHooksUpdated),
            ClientEvent::FocusConfirmed { request, pane } => self.focus_confirmed(request, pane),
            ClientEvent::Task { request, outcome } => {
                self.answer_request(request, HerdrAnswer::Task(outcome))
            }
            ClientEvent::Removal { request, outcome } => {
                self.answer_request(request, HerdrAnswer::Removal(outcome))
            }
            ClientEvent::Repos { request, repos } => {
                self.answer_request(request, HerdrAnswer::Repos(repos))
            }
            ClientEvent::Agents {
                request,
                names,
                answered,
            } => self.answer_request(request, HerdrAnswer::Agents { names, answered }),
            ClientEvent::Restart { request, outcome } => {
                self.answer_request(request, HerdrAnswer::Restart(outcome))
            }
            ClientEvent::Delivered { request, delivered } => {
                self.answer_request(request, HerdrAnswer::Delivered(delivered))
            }
            ClientEvent::CommandFailed { command, reason } => {
                log::info!("herdr: `{command}` failed: {}", reason.english());
            }
            ClientEvent::StateChanged
            | ClientEvent::PushActive(_)
            | ClientEvent::FirstFrame
            | ClientEvent::FrameChannelLost
            | ClientEvent::InputDropped { .. }
            | ClientEvent::InputIgnored(_) => {}
        }
    }

    fn answer_request(&mut self, request: RequestId, answer: HerdrAnswer) {
        if let Some(token) = self.tokens.remove(&request) {
            self.outputs.push_back(PlaneOut::Answer { token, answer });
        }
    }

    fn answered(&mut self, token: Option<u64>, answer: HerdrAnswer) {
        if let Some(token) = token {
            self.outputs.push_back(PlaneOut::Answer { token, answer });
        }
    }

    fn drain_renderer(&mut self) {
        loop {
            let Some(output) = self.renderer.as_mut().and_then(Renderer::poll_output) else {
                return;
            };
            self.renderer_output(output);
        }
    }

    fn renderer_output(&mut self, output: RendererOutput) {
        self.dirty = true;
        match output {
            RendererOutput::Exec { id, command } => {
                let tag = ExecTag::Renderer(id.0);
                if command.contains(launch::RETARGET_LANDED_MARKER) {
                    // A retarget kills the attach the typing before it went
                    // to: it goes once that typing is in the tty.
                    self.lane
                        .push_back(Write::Fence(Fenced::Exec { tag, command }));
                } else {
                    self.outputs.push_back(PlaneOut::Exec { tag, command });
                }
            }
            RendererOutput::Send(bytes) => {
                // A delivery's bytes: Delivered follows.
                let end = self.end;
                self.lane.push_back(Write::Text { bytes, end });
            }
            RendererOutput::Delivered { id, sent } => {
                let Some(key) = self.renderer_deliveries.remove(&id) else {
                    return;
                };
                if !sent {
                    self.answer(key, false);
                    return;
                }
                // Out of the renderer's hands, into mosh's: sent once
                // mosh-server has it.
                let barrier = self.barrier_id();
                let until = self.now + self.budgets.ack;
                if let Some(pending) = self.pending.get_mut(&key) {
                    pending.stage = Stage::Ack { barrier, until };
                    self.acks.insert(barrier, AckFor::Delivery(key));
                    self.lane.push_back(Write::Ack(barrier));
                }
            }
            RendererOutput::Veil => {
                self.outputs.push_back(PlaneOut::Mark(Mark {
                    code: mark::VEIL,
                    arg: 0,
                }));
                self.outputs.push_back(PlaneOut::Mark(Mark {
                    code: mark::EXTEND_COVER,
                    arg: i64::try_from(self.budgets.cover_cap).unwrap_or(i64::MAX),
                }));
            }
            RendererOutput::Reveal => self.outputs.push_back(PlaneOut::Mark(Mark {
                code: mark::REVEAL,
                arg: 0,
            })),
            RendererOutput::Notice(notice) => self.say(notice),
            RendererOutput::ClearNotice => {
                if self.said.as_ref().is_some_and(|said| said.until.is_none()) {
                    self.said = None;
                }
            }
            RendererOutput::RefreshTree => self.client.read_now(self.now),
            RendererOutput::Retype(bytes) => {
                let end = self.end;
                self.lane.push_back(Write::Text { bytes, end });
                let barrier = self.barrier_id();
                self.acks.insert(barrier, AckFor::Retype);
                self.lane.push_back(Write::Ack(barrier));
            }
        }
    }

    fn barrier_id(&mut self) -> u64 {
        self.next_barrier += 1;
        self.next_barrier
    }

    /// What the renderer says, as the banner says it: a pane on another
    /// device ("Open here"), herdr's refusal or the loop gone ("Retry") stay
    /// until the next landing; a one-off flashes, never over another.
    fn say(&mut self, notice: RendererNotice) {
        let (notice, action) = match notice {
            RendererNotice::NotOnPane => return self.flash(Notice::InputDropped),
            RendererNotice::CouldntSwitch => return self.flash(Notice::CouldntSwitch),
            RendererNotice::OnAnotherDevice => (Notice::OnAnotherDevice, NoticeAction::OpenHere),
            RendererNotice::Refused(said) if said.is_empty() => {
                (Notice::InputDropped, NoticeAction::Retry)
            }
            RendererNotice::Refused(said) => (Notice::Refused(said), NoticeAction::Retry),
            RendererNotice::LoopGone => (Notice::InputDropped, NoticeAction::Retry),
        };
        self.said = Some(Said {
            notice,
            action: Some(action),
            until: None,
        });
        self.dirty = true;
    }

    /// What the renderer says, as it shows over the client's notice: not
    /// herdr refusing the attach while the client names what is wrong with
    /// the server (a version skew, a restart), whose doing the refusal is
    /// (B420). The client's says which side is behind and its button is the
    /// remedy; the refusal's raw words and a Retry that can only be refused
    /// again covered it, and the button retried the attach.
    fn shown(&self) -> Option<&Said> {
        let said = self.said.as_ref()?;
        let server_said = matches!(
            self.client.notice(),
            Some((
                Notice::Mismatch(_) | Notice::Skew(_) | Notice::Restarting | Notice::RestartFailed,
                _
            ))
        );
        if server_said && matches!(said.notice, Notice::Refused(_)) {
            return None;
        }
        Some(said)
    }

    fn flash(&mut self, notice: Notice) {
        if self.said.is_some() || self.client.notice().is_some() {
            return;
        }
        self.said = Some(Said {
            notice,
            action: None,
            until: Some(self.now + self.budgets.flash),
        });
        self.dirty = true;
    }

    /// The banner's button: the pane taken back, or the loop booted again
    /// (its line into mosh).
    fn take_back(&mut self) {
        // A pane another device holds is taken back as a tap on it is: the
        // server's focus goes there too (B409), else the next read steers
        // the loop to where the other device moved the focus.
        let held = self
            .renderer
            .as_ref()
            .and_then(|renderer| match renderer.down() {
                Some(Down::TakenOver(terminal) | Down::Refused(terminal)) => Some(terminal),
                _ => None,
            })
            .and_then(|terminal| self.pane_of(terminal));
        if let Some(pane) = held {
            self.picking = Some(pane.clone());
            self.client.act(Action::SelectPane(pane), self.now);
            self.drain_client();
            self.picking = None;
            self.dirty = true;
            return;
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        if let Some(line) = renderer.take_back(self.now) {
            let end = self.end;
            self.lane.push_back(Write::Text {
                bytes: line.into_bytes(),
                end,
            });
        }
        self.dirty = true;
    }

    // MARK: - The lane into mosh

    /// What was typed, ending at `end`, into the lane in order: each lone
    /// arrow as a key herdr encodes by `pane`'s real cursor-key mode —
    /// arrows that arrive together too (B360) — the rest as text. With no
    /// pane to name, all of it as text.
    fn queue_typed(&mut self, pane: Option<String>, bytes: &[u8], end: u64) {
        let Some(pane) = pane else {
            self.lane.push_back(Write::Text {
                bytes: bytes.to_vec(),
                end,
            });
            return;
        };
        let mut left = bytes.len() as u64;
        for piece in frame::split_arrows(bytes) {
            let len = match piece {
                frame::Piece::Arrow(_) => 3,
                frame::Piece::Bytes(run) => run.len(),
            };
            left -= len as u64;
            let piece_end = end.saturating_sub(left);
            match piece {
                frame::Piece::Arrow(key) => self.queue_key(pane.clone(), key, piece_end),
                frame::Piece::Bytes(run) => self.lane.push_back(Write::Text {
                    bytes: run.to_vec(),
                    end: piece_end,
                }),
            }
        }
    }

    /// A key for `pane`, behind what the lane holds; one that comes while
    /// the run at the tail hasn't started joins it (a held arrow repeats).
    fn queue_key(&mut self, pane: String, key: &'static str, end: u64) {
        if let Some(Write::Fence(Fenced::Keys {
            pane: tail,
            keys,
            end: tail_end,
        })) = self.lane.back_mut()
            && *tail == pane
        {
            keys.push(key);
            *tail_end = end;
            return;
        }
        self.lane.push_back(Write::Fence(Fenced::Keys {
            pane,
            keys: vec![key],
            end,
        }));
    }

    fn pump_lane(&mut self) {
        while self.blocked.is_none() {
            let Some(write) = self.lane.pop_front() else {
                return;
            };
            match write {
                Write::Text { bytes, end } => self.outputs.push_back(PlaneOut::Mosh { bytes, end }),
                Write::Skip { end } => self.outputs.push_back(PlaneOut::Mosh {
                    bytes: Vec::new(),
                    end,
                }),
                Write::Ack(barrier) => {
                    // Not for anyone any more (a delivery answered meanwhile):
                    // not asked.
                    if self.acks.contains_key(&barrier) {
                        self.outputs.push_back(PlaneOut::Barrier(barrier));
                    }
                }
                Write::Fence(then) => {
                    let barrier = self.barrier_id();
                    self.outputs.push_back(PlaneOut::Barrier(barrier));
                    self.blocked = Some(Blocked {
                        then,
                        phase: Phase::Landing {
                            barrier,
                            deadline: self.now + self.budgets.landing,
                        },
                    });
                }
            }
        }
    }

    /// The fence at the lane's head was acknowledged (or waited its bound).
    fn fence_landed(&mut self) {
        let Some(blocked) = self.blocked.take() else {
            return;
        };
        match blocked.then {
            Fenced::Exec { tag, command } => {
                self.outputs.push_back(PlaneOut::Exec { tag, command });
                self.pump_lane();
            }
            Fenced::Keys { pane, keys, end } => match control::send_keys(&keys, &pane) {
                Some(subcommand) => {
                    self.next_key += 1;
                    let exec = self.next_key;
                    let command = control::command_line(
                        &subcommand,
                        self.client.config().custom_path.as_deref(),
                    );
                    self.outputs.push_back(PlaneOut::Exec {
                        tag: ExecTag::Key(exec),
                        command,
                    });
                    self.blocked = Some(Blocked {
                        then: Fenced::Keys { pane, keys, end },
                        phase: Phase::Keys {
                            exec,
                            deadline: self.now + self.budgets.key_hold,
                        },
                    });
                }
                None => {
                    self.outputs.push_back(PlaneOut::Mosh {
                        bytes: Vec::new(),
                        end,
                    });
                    self.pump_lane();
                }
            },
        }
    }

    /// The key run answered (or held the lane its bound): the typing behind
    /// it goes.
    fn keys_done(&mut self) {
        if let Some(Blocked {
            then: Fenced::Keys { end, .. },
            ..
        }) = self.blocked.take()
        {
            // The keys never went through mosh: nothing of them to echo.
            self.outputs.push_back(PlaneOut::Mosh {
                bytes: Vec::new(),
                end,
            });
        }
        self.pump_lane();
    }

    // MARK: - Deliveries

    /// Deliveries waiting for the tree: the ones whose pane it lists go on to
    /// the focus; a tree read without it (or no server) is no.
    fn advance_tree_waits(&mut self) {
        if self.advancing {
            return;
        }
        self.advancing = true;
        self.advance_tree_waits_once();
        self.advancing = false;
    }

    fn advance_tree_waits_once(&mut self) {
        let read = self.client.snapshot().is_attached || self.client.server_not_running();
        let waiting: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, p)| matches!(p.stage, Stage::Tree { .. }))
            .map(|(key, _)| *key)
            .collect();
        for key in waiting {
            let Some(pending) = self.pending.get(&key) else {
                continue;
            };
            if !matches!(pending.stage, Stage::Tree { .. }) {
                continue;
            }
            let pane = pending.pane.clone();
            let known = self.client.snapshot().panes.contains_key(&pane)
                && (self.renderer.is_none() || self.client.terminal_ids().contains_key(&pane));
            if known {
                self.ask_focus(key, &pane);
            } else if read {
                // The server is read and has no such pane (or no server).
                self.flash(Notice::CouldntSwitch);
                self.answer(key, false);
            }
        }
    }

    /// The server is asked to focus the pane (unless it already looks
    /// focused) and to say what it has focused once that has landed.
    fn ask_focus(&mut self, key: u64, pane: &str) {
        let there = self.client.snapshot().active_pane_id.as_deref() == Some(pane);
        if !there {
            // The person's reply, for this pane: a pick, as a tap would be.
            self.picking = Some(pane.to_owned());
            self.client
                .act(Action::SelectPane(pane.to_owned()), self.now);
            self.drain_client();
            self.picking = None;
        }
        let budget = self.budgets.focus;
        let Some(request) = self
            .client
            .act(Action::ConfirmFocus { budget_ms: budget }, self.now)
        else {
            self.answer(key, false);
            return;
        };
        if let Some(pending) = self.pending.get_mut(&key) {
            pending.stage = Stage::Focus {
                request,
                again: there,
            };
        }
    }

    fn focus_confirmed(&mut self, request: RequestId, focused: Option<String>) {
        let Some((&key, pending)) = self
            .pending
            .iter()
            .find(|(_, p)| matches!(p.stage, Stage::Focus { request: r, .. } if r == request))
        else {
            return;
        };
        let Stage::Focus { again, .. } = pending.stage else {
            return;
        };
        let pane = pending.pane.clone();
        if focused.as_deref() != Some(pane.as_str()) {
            if again {
                // The tree was older than the server (the desktop moved the
                // focus since): asked now.
                self.picking = Some(pane.clone());
                self.client.act(Action::SelectPane(pane.clone()), self.now);
                self.drain_client();
                self.picking = None;
                match self.client.act(
                    Action::ConfirmFocus {
                        budget_ms: self.budgets.focus,
                    },
                    self.now,
                ) {
                    Some(request) => {
                        if let Some(pending) = self.pending.get_mut(&key) {
                            pending.stage = Stage::Focus {
                                request,
                                again: false,
                            };
                        }
                    }
                    None => self.answer(key, false),
                }
                return;
            }
            log::info!("herdr over mosh: {pane} never came into focus — not sent");
            self.flash(Notice::CouldntSwitch);
            self.answer(key, false);
            return;
        }
        match self.renderer.is_some() {
            true => {
                let Some(terminal) = self.client.terminal_ids().get(&pane).cloned() else {
                    self.flash(Notice::CouldntSwitch);
                    self.answer(key, false);
                    return;
                };
                self.next_delivery += 1;
                let id = DeliveryId(self.next_delivery);
                self.renderer_deliveries.insert(id, key);
                let bytes = self.pending[&key].bytes.clone();
                if let Some(pending) = self.pending.get_mut(&key) {
                    pending.stage = Stage::Renderer(id);
                }
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.deliver(id, &terminal, bytes, self.now);
                }
                self.drain_renderer();
            }
            false => {
                // herdr's own TUI types into the focused pane.
                if self.client.pane_switch_in_flight() {
                    self.flash(Notice::CouldntSwitch);
                    self.answer(key, false);
                    return;
                }
                let bytes = self.pending[&key].bytes.clone();
                let end = self.end;
                self.lane.push_back(Write::Text { bytes, end });
                let barrier = self.barrier_id();
                let until = self.now + self.budgets.ack;
                if let Some(pending) = self.pending.get_mut(&key) {
                    pending.stage = Stage::Ack { barrier, until };
                }
                self.acks.insert(barrier, AckFor::Delivery(key));
                self.lane.push_back(Write::Ack(barrier));
            }
        }
    }

    /// Deliveries out of time: no.
    fn expire_deliveries(&mut self) {
        let due: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, p)| match p.stage {
                Stage::Tree { until } | Stage::Ack { until, .. } => {
                    self.now >= until.min(p.deadline)
                }
                Stage::Focus { .. } | Stage::Renderer(_) => false,
            })
            .map(|(key, _)| *key)
            .collect();
        for key in due {
            if matches!(self.pending[&key].stage, Stage::Tree { .. }) {
                self.flash(Notice::CouldntSwitch);
            }
            self.answer(key, false);
        }
    }

    fn answer(&mut self, key: u64, sent: bool) {
        let Some(pending) = self.pending.remove(&key) else {
            return;
        };
        if let Stage::Renderer(id) = pending.stage {
            self.renderer_deliveries.remove(&id);
        }
        if let Stage::Ack { barrier, .. } = pending.stage {
            self.acks.remove(&barrier);
        }
        self.outputs.push_back(PlaneOut::Delivered {
            token: pending.token,
            sent,
        });
    }
}

/// The pane on screen (the loop's, by the host's word), for a key.
fn renderer_pane(client: &Client, renderer: Option<&Renderer>) -> Option<String> {
    let terminal = renderer?.on_screen()?;
    client
        .terminal_ids()
        .iter()
        .find(|(_, t)| t.as_str() == terminal)
        .map(|(pane, _)| pane.clone())
        .or_else(|| client.snapshot().active_pane_id.clone())
}

// MARK: - The leg

/// How many exec channels the plane may have open at once on its
/// connection: the renderer's watch (a minute long), a read, the command
/// lane, the retarget lane, a key run and one spare (sshd's `MaxSessions` is
/// 10, the push channel and the borrowers' SFTP included).
pub const EXEC_CHANNELS: usize = 6;

/// How the leg finds a control connection gone dead without a word (a
/// stall: mosh carries on, nothing closes). Only silence is a sign, and
/// only a question shows it (B276):
///
/// - Something was asked of it (an exec, the push channel or a write to it)
///   and the host has sent nothing at all since, [`SUSPECT_AFTER`] on: a
///   live host answers a request within a round trip (an open confirmed),
///   whatever the command then takes. A healthy connection in use is never
///   pinged.
/// - Nothing asked and nothing heard for [`PROBE_AFTER`]: push carries the
///   news and the poll is 45 s apart, so a link that died idle would
///   otherwise go unseen until then. The idle poll's cadence without push.
///
/// Either way the connection is pinged ([`Ssh::ping`]: a global request
/// sshd answers itself, two small packets, no channel, nothing run on the
/// host — less than the exec `true` every 20 s it replaces), and no answer
/// in [`PROBE_TIMEOUT`] is a lost connection. A false alarm costs a redial
/// of the control plane, never the terminal: mosh carries that. Was a 20 s
/// quiet time plus an 8 s exec: 28–29 s to notice.
const SUSPECT_AFTER: Duration = Duration::from_secs(2);
const PROBE_AFTER: Duration = Duration::from_secs(8);
/// The app's foreground probe deadline: a 700 ms path's round trip plus
/// two retransmits.
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

/// What the session tells the leg.
pub(crate) enum HerdrMoshCommand {
    Typed {
        bytes: Vec<u8>,
        end: u64,
    },
    Paste {
        text: String,
        end: u64,
    },
    Action {
        action: HerdrAction,
        token: Option<u64>,
    },
    CancelTask {
        token: u64,
    },
    Deliver {
        pane: String,
        bytes: Vec<u8>,
        paste: bool,
        /// On the session's clock.
        deadline: u64,
        reply: oneshot::Sender<bool>,
    },
    /// A control connection (redialled after one was lost).
    Connection(Ssh),
    Background,
    Foreground,
    Close(oneshot::Sender<()>),
}

/// What the leg tells the session.
pub(crate) enum HerdrMoshNote {
    View(Box<HerdrView>),
    Answer {
        token: u64,
        answer: HerdrAnswer,
    },
    AgentHooksUpdated,
    InputDropped {
        bytes: usize,
    },
    /// The control connection is gone: the session dials another
    /// ([`HerdrMoshCommand::Connection`]); mosh carries on.
    ConnectionLost,
}

pub(crate) struct HerdrMoshStart {
    pub ssh: Ssh,
    pub mosh: mpsc::UnboundedSender<MoshCommand>,
    pub shell: Arc<Stream>,
    pub painted: Arc<AtomicU64>,
    pub config: PlaneConfig,
    pub foreground: bool,
}

pub(crate) struct HerdrMoshLeg {
    pub commands: mpsc::UnboundedSender<HerdrMoshCommand>,
    task: JoinHandle<()>,
}

impl HerdrMoshLeg {
    pub fn spawn(
        start: HerdrMoshStart,
        clock: impl Fn() -> u64 + Send + Sync + 'static,
        notes: Box<dyn Fn(HerdrMoshNote) + Send + Sync>,
    ) -> Self {
        let (commands, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(start, rx, Box::new(clock), notes));
        Self { commands, task }
    }

    pub fn abort(&self) {
        self.task.abort();
    }
}

impl Drop for HerdrMoshLeg {
    fn drop(&mut self) {
        self.task.abort();
    }
}

enum Wake {
    /// An exec's answer; `closed`: it failed because its connection (of
    /// that epoch) is gone.
    Exec(ExecTag, Option<Vec<u8>>, bool, u64),
    Barrier(u64, bool),
    /// The push channel's, each with the epoch of the connection it rode:
    /// one from a connection since replaced says nothing about the push
    /// channel on the new one (B210).
    PushOpened(Process, u64),
    PushFailed(u64),
    PushBytes(Vec<u8>, u64),
    PushClosed(u64),
    Probe(bool, u64),
}

struct Task {
    plane: Plane,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    notes: Box<dyn Fn(HerdrMoshNote) + Send + Sync>,
    ssh: Option<Ssh>,
    /// Bumped with every connection: a failure on an older one says nothing
    /// about the one in use.
    ssh_epoch: u64,
    mosh: mpsc::UnboundedSender<MoshCommand>,
    shell: Arc<Stream>,
    painted: Arc<AtomicU64>,
    wakes: mpsc::UnboundedSender<Wake>,
    execs: HashMap<ExecTag, JoinHandle<()>>,
    /// Execs asked for while no connection is up, in order.
    waiting: VecDeque<(ExecTag, String)>,
    channels: Arc<Semaphore>,
    push: Option<PushChannel>,
    deliveries: HashMap<u64, oneshot::Sender<bool>>,
    next_token: u64,
    view_sent: Option<HerdrView>,
    /// Requests made of the control connection the host has not said a
    /// word since: when the oldest and the newest went out.
    asked: Option<(Instant, Instant)>,
    probing: bool,
}

struct PushChannel {
    writes: mpsc::UnboundedSender<Vec<u8>>,
    tasks: [JoinHandle<()>; 2],
}

impl Drop for PushChannel {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn run(
    start: HerdrMoshStart,
    mut commands: mpsc::UnboundedReceiver<HerdrMoshCommand>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    notes: Box<dyn Fn(HerdrMoshNote) + Send + Sync>,
) {
    let HerdrMoshStart {
        ssh,
        mosh,
        shell,
        painted,
        config,
        foreground,
    } = start;
    let (wakes, mut wake_rx) = mpsc::unbounded_channel();
    let mut task = Task {
        plane: Plane::new(config),
        clock,
        notes,
        ssh: Some(ssh),
        ssh_epoch: 0,
        mosh,
        shell,
        painted,
        wakes,
        execs: HashMap::new(),
        waiting: VecDeque::new(),
        channels: Arc::new(Semaphore::new(EXEC_CHANNELS)),
        push: None,
        deliveries: HashMap::new(),
        next_token: 1 << 48,
        view_sent: None,
        asked: None,
        probing: false,
    };
    let now = task.now();
    task.plane.start(now);
    if !foreground {
        task.plane.background(now);
    }
    let epoch = (Instant::now(), now);
    loop {
        task.pump();
        let deadline = task
            .plane
            .next_deadline()
            .map(|at| epoch.0 + Duration::from_millis(at.saturating_sub(epoch.1)));
        let probe_at = task.probe_at();
        tokio::select! {
            Some(wake) = wake_rx.recv() => task.wake(wake),
            command = commands.recv() => match command {
                Some(HerdrMoshCommand::Close(done)) => {
                    let now = task.now();
                    task.plane.close(now);
                    task.pump();
                    let _ = done.send(());
                    break;
                }
                Some(command) => task.command(command),
                None => break,
            },
            () = sleep_until(deadline) => {
                task.feed_paint();
                let now = task.now();
                task.plane.tick(now);
            }
            // The host may have spoken since this was armed: asked again.
            () = sleep_until(probe_at) => {
                if task.probe_at().is_some_and(|at| at <= Instant::now()) {
                    task.probe();
                }
            }
        }
    }
    for (_, exec) in task.execs.drain() {
        exec.abort();
    }
    for (_, reply) in task.deliveries.drain() {
        let _ = reply.send(false);
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

impl Task {
    fn now(&self) -> u64 {
        (self.clock)()
    }

    fn feed_paint(&mut self) {
        let stamp = self.painted.load(Ordering::Relaxed);
        if stamp > 0 {
            self.plane.painted(stamp);
        }
    }

    /// Carries out what the plane queued, in order.
    fn pump(&mut self) {
        while let Some(output) = self.plane.poll_output() {
            match output {
                PlaneOut::Exec { tag, command } => self.exec(tag, command),
                PlaneOut::Abandon { tag } => {
                    self.waiting.retain(|(waiting, _)| *waiting != tag);
                    if let Some(exec) = self.execs.remove(&tag) {
                        exec.abort();
                    }
                }
                PlaneOut::OpenPush { program } => self.open_push(program),
                PlaneOut::PushWrite(bytes) => {
                    if let Some(push) = &self.push
                        && push.writes.send(bytes).is_ok()
                    {
                        self.asked();
                    }
                }
                PlaneOut::ClosePush => self.push = None,
                PlaneOut::Mosh { bytes, end } => {
                    let _ = self.mosh.send(MoshCommand::Keys { bytes, end });
                }
                PlaneOut::Barrier(id) => {
                    let (reply, acked) = oneshot::channel();
                    let wakes = self.wakes.clone();
                    if self.mosh.send(MoshCommand::Barrier(reply)).is_ok() {
                        tokio::spawn(async move {
                            let acked = acked.await.is_ok();
                            let _ = wakes.send(Wake::Barrier(id, acked));
                        });
                    } else {
                        let _ = wakes.send(Wake::Barrier(id, false));
                    }
                }
                PlaneOut::Mark(mark) => self.shell.mark(mark),
                PlaneOut::Delivered { token, sent } => {
                    if let Some(reply) = self.deliveries.remove(&token) {
                        let _ = reply.send(sent);
                    }
                }
                PlaneOut::Answer { token, answer } => {
                    (self.notes)(HerdrMoshNote::Answer { token, answer })
                }
                PlaneOut::InputDropped { bytes } => {
                    (self.notes)(HerdrMoshNote::InputDropped { bytes })
                }
                PlaneOut::AgentHooksUpdated => (self.notes)(HerdrMoshNote::AgentHooksUpdated),
            }
        }
        if self.plane.take_dirty() || self.view_sent.is_none() {
            let view = self.plane.view();
            if self.view_sent.as_ref() != Some(&view) {
                self.view_sent = Some(view.clone());
                (self.notes)(HerdrMoshNote::View(Box::new(view)));
            }
        }
    }

    fn exec(&mut self, tag: ExecTag, command: String) {
        let Some(ssh) = self.ssh.clone() else {
            // The connection is being redialled: run it on the next one
            // (its owner's budget bounds the wait).
            self.waiting.push_back((tag, command));
            return;
        };
        self.asked();
        let wakes = self.wakes.clone();
        let channels = self.channels.clone();
        let epoch = self.ssh_epoch;
        let task = tokio::spawn(async move {
            let _permit = channels.acquire_owned().await;
            let (output, closed) = match ssh.run(&command).await {
                Ok(output) => (Some(output.merged()), false),
                Err(_) => (None, ssh.is_closed()),
            };
            let _ = wakes.send(Wake::Exec(tag, output, closed, epoch));
        });
        self.execs.insert(tag, task);
    }

    /// The push channel: a terminal whose program is the pump (B267: never
    /// typed into a login shell still starting).
    fn open_push(&mut self, program: String) {
        self.push = None;
        let epoch = self.ssh_epoch;
        let Some(ssh) = self.ssh.clone() else {
            let _ = self.wakes.send(Wake::PushFailed(epoch));
            return;
        };
        self.asked();
        let wakes = self.wakes.clone();
        tokio::spawn(async move {
            let mut request = PtyRequest::new(80, 24);
            request.command = Some(program);
            let wake = match ssh.terminal(&request).await {
                Ok(process) => Wake::PushOpened(process, epoch),
                Err(_) => Wake::PushFailed(epoch),
            };
            let _ = wakes.send(wake);
        });
    }

    /// A request just went out on the control connection.
    fn asked(&mut self) {
        let now = Instant::now();
        self.asked = Some(match self.asked {
            Some((first, _)) => (first, now),
            None => (now, now),
        });
    }

    /// When the control connection is next due a ping ([`SUSPECT_AFTER`],
    /// [`PROBE_AFTER`]); requests the host has since answered are let go.
    fn probe_at(&mut self) -> Option<Instant> {
        if self.probing {
            return None;
        }
        let heard = Instant::from_std(self.ssh.as_ref()?.last_heard());
        if let Some((first, last)) = self.asked {
            self.asked = if heard >= last {
                None
            } else if heard >= first {
                // Heard after the oldest, not after the newest: the newest
                // is still owed a word (the check comes up to that late).
                Some((last, last))
            } else {
                Some((first, last))
            };
        }
        let idle = heard + PROBE_AFTER;
        Some(match self.asked {
            Some((first, _)) => idle.min(first + SUSPECT_AFTER),
            None => idle,
        })
    }

    fn probe(&mut self) {
        let Some(ssh) = self.ssh.clone() else {
            return;
        };
        self.probing = true;
        log::debug!(
            "herdr over mosh: pinging the control connection ({})",
            match self.asked {
                Some(_) => "a request unanswered",
                None => "quiet",
            }
        );
        let wakes = self.wakes.clone();
        let epoch = self.ssh_epoch;
        tokio::spawn(async move {
            let alive = ssh.ping(PROBE_TIMEOUT).await;
            let _ = wakes.send(Wake::Probe(alive, epoch));
        });
    }

    /// The control connection is gone: what runs on it fails, what comes
    /// waits for the next one.
    fn lost(&mut self) {
        self.asked = None;
        if let Some(ssh) = self.ssh.take() {
            tokio::spawn(async move { ssh.close().await });
            // Dropping the channel aborts its reader: no close will come
            // from it, so the plane hears it here (B210).
            self.push = None;
            let now = self.now();
            self.plane.connection_lost(now);
            (self.notes)(HerdrMoshNote::ConnectionLost);
        }
    }

    fn wake(&mut self, wake: Wake) {
        self.feed_paint();
        let now = self.now();
        match wake {
            Wake::Exec(tag, output, closed, epoch) => {
                self.execs.remove(&tag);
                if closed && epoch == self.ssh_epoch {
                    self.lost();
                }
                self.plane.exec_finished(tag, output, now);
            }
            Wake::Barrier(id, acked) => self.plane.acked(id, acked, now),
            Wake::PushOpened(process, epoch) if epoch != self.ssh_epoch => {
                // Opened on a connection since replaced: not ours to use.
                drop(process);
            }
            Wake::PushFailed(epoch) | Wake::PushClosed(epoch) | Wake::PushBytes(_, epoch)
                if epoch != self.ssh_epoch => {}
            Wake::PushOpened(process, epoch) => {
                let (mut reader, writer) = process.split();
                let (writes, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
                let wakes = self.wakes.clone();
                self.push = Some(PushChannel {
                    writes,
                    tasks: [
                        tokio::spawn(async move {
                            while let Some(bytes) = rx.recv().await {
                                if writer.write(&bytes).await.is_err() {
                                    break;
                                }
                            }
                        }),
                        tokio::spawn(async move {
                            loop {
                                match reader.next().await {
                                    Some(Event::Stdout(bytes)) | Some(Event::Stderr(bytes)) => {
                                        if wakes
                                            .send(Wake::PushBytes(bytes.to_vec(), epoch))
                                            .is_err()
                                        {
                                            return;
                                        }
                                    }
                                    Some(Event::Exited(_)) | Some(Event::Eof) => {}
                                    Some(Event::Closed) | Some(Event::Lost) | None => {
                                        let _ = wakes.send(Wake::PushClosed(epoch));
                                        return;
                                    }
                                }
                            }
                        }),
                    ],
                });
                self.plane.push_opened(now);
            }
            Wake::PushFailed(_) => self.plane.push_failed(now),
            Wake::PushBytes(bytes, _) => self.plane.push_bytes(&bytes, now),
            Wake::PushClosed(_) => {
                self.push = None;
                if self.ssh.as_ref().is_some_and(Ssh::is_closed) {
                    self.lost();
                }
                self.plane.push_closed(now);
            }
            Wake::Probe(_, epoch) if epoch != self.ssh_epoch => self.probing = false,
            Wake::Probe(alive, _) => {
                self.probing = false;
                if !alive {
                    log::info!("herdr over mosh: the control connection stopped answering");
                    self.lost();
                }
            }
        }
    }

    fn command(&mut self, command: HerdrMoshCommand) {
        self.feed_paint();
        let now = self.now();
        match command {
            HerdrMoshCommand::Typed { bytes, end } => self.plane.typed(&bytes, end, now),
            HerdrMoshCommand::Paste { text, end } => self.plane.paste(&text, end, now),
            HerdrMoshCommand::Action { action, token } => self.plane.act(action, token, now),
            HerdrMoshCommand::CancelTask { token } => self.plane.cancel_task(token, now),
            HerdrMoshCommand::Deliver {
                pane,
                bytes,
                paste,
                deadline,
                reply,
            } => {
                self.next_token += 1;
                let token = self.next_token;
                self.deliveries.insert(token, reply);
                self.plane
                    .deliver(token, &pane, bytes, paste, deadline, now);
            }
            HerdrMoshCommand::Connection(ssh) => {
                // A push channel still held rode the connection this one
                // replaces: what it says is the old connection's (B210).
                if self.push.take().is_some() {
                    self.plane.connection_lost(now);
                }
                self.ssh_epoch += 1;
                self.ssh = Some(ssh);
                self.asked = None;
                self.probing = false;
                let waiting = std::mem::take(&mut self.waiting);
                for (tag, command) in waiting {
                    self.exec(tag, command);
                }
                self.plane.connection_up(now);
            }
            HerdrMoshCommand::Background => self.plane.background(now),
            HerdrMoshCommand::Foreground => self.plane.foreground(now),
            HerdrMoshCommand::Close(_) => unreachable!("handled by the loop"),
        }
    }
}
