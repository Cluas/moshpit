//! How the machine runs its operations without doing I/O: each operation
//! (a care pass, a Host Setup step, a mint round) is a coroutine — an
//! `async` block — that the machine polls itself, with no runtime and no
//! waker. A coroutine waits only on the leaves here: a command on a host
//! ([`Exec`]), a request to the relay ([`Post`]), or a condition over the
//! machine's state with an optional deadline ([`Until`]). Asking for a
//! command or a request queues an [`Output`]; the answer comes back as an
//! input ([`crate::HostCare::exec_done`], [`crate::HostCare::post_done`])
//! and the next poll finds it. Deadlines are the caller's clock
//! ([`crate::Clock`]), passed with every input.
//!
//! Cancelling an operation (a delete or an unpair standing a care pass
//! down) makes every leaf it waits on, and every leaf it asks for after,
//! answer at once: a command as failed ([`CANCELLED`]), a wait as given up.
//! A command already sent can't be called back; its answer is dropped.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};

use moshpit_install::installer::{ChannelError, HostChannel};
use moshpit_seal::Uuid;

use crate::care::CareState;
use crate::push::PushState;
use crate::setup::Page;
use crate::store::PairingStore;
use crate::{Change, Clock, Config, Output, Settings};

/// One operation the machine runs.
pub type TaskId = u64;

pub(crate) type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// What a command answers once its operation was cancelled.
pub const CANCELLED: &str = "cancelled";

/// What a command answers once its deadline passed with no answer from
/// the host ([`crate::Note::HostNotAnswering`] on a screen).
pub const HOST_SILENT: &str = "the host didn't answer in time";

/// What a request to a relay came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub body: Vec<u8>,
}

enum Slot<T> {
    Waiting,
    Done(T),
}

/// The connections the phone keeps, as its store says.
#[derive(Debug, Clone, Default)]
pub(crate) struct Saved {
    /// The connections the store decoded (`ConnectionStore.connections`):
    /// care acts only for these.
    pub listed: HashSet<Uuid>,
    /// Every connection the store holds, decoded or not; `None` while it
    /// can't be told (`ConnectionStore.savedIds`).
    pub all: Option<HashSet<Uuid>>,
}

/// Everything the coroutines and the inputs share.
pub(crate) struct Shared {
    pub clock: Clock,
    pub config: Config,
    pub settings: Settings,
    pub saved: Saved,
    pub store: Box<dyn PairingStore>,
    pub care: CareState,
    pub push: PushState,
    pub pages: BTreeMap<u64, Page>,
    /// One pairing operation per host at a time (B123): each connection's
    /// queue, oldest (the one running) first.
    pub lanes: HashMap<Uuid, VecDeque<u64>>,
    pub outputs: VecDeque<Output>,
    pub changes: BTreeSet<Change>,
    next_id: u64,
    execs: HashMap<u64, Slot<Result<Vec<u8>, String>>>,
    posts: HashMap<u64, Slot<Result<HttpReply, String>>>,
    /// The deadlines the waits pending now are waiting for.
    pub deadlines: BTreeSet<u64>,
    /// Bumped whenever a wait ends: another pass may make progress.
    pub generation: u64,
    pub running: HashSet<TaskId>,
    pub cancelled: HashSet<TaskId>,
}

impl Shared {
    pub fn new(config: Config, store: Box<dyn PairingStore>, clock: Clock) -> Self {
        Shared {
            clock,
            config,
            settings: Settings::default(),
            saved: Saved::default(),
            store,
            care: CareState::default(),
            push: PushState::default(),
            pages: BTreeMap::new(),
            lanes: HashMap::new(),
            outputs: VecDeque::new(),
            changes: BTreeSet::new(),
            next_id: 1,
            execs: HashMap::new(),
            posts: HashMap::new(),
            deadlines: BTreeSet::new(),
            generation: 0,
            running: HashSet::new(),
            cancelled: HashSet::new(),
        }
    }

    pub fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn changed(&mut self, change: Change) {
        self.changes.insert(change);
    }

    /// The answer to command `id`; `false` when nothing waits for it (its
    /// operation was cancelled, or it was answered already).
    pub fn exec_done(&mut self, id: u64, result: Result<Vec<u8>, String>) -> bool {
        match self.execs.get_mut(&id) {
            Some(slot @ Slot::Waiting) => {
                *slot = Slot::Done(result);
                true
            }
            _ => false,
        }
    }

    pub fn post_done(&mut self, id: u64, result: Result<HttpReply, String>) -> bool {
        match self.posts.get_mut(&id) {
            Some(slot @ Slot::Waiting) => {
                *slot = Slot::Done(result);
                true
            }
            _ => false,
        }
    }
}

pub(crate) fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What a coroutine holds: the shared state, and which operation it is.
#[derive(Clone)]
pub(crate) struct Ctx {
    pub shared: Arc<Mutex<Shared>>,
    pub task: TaskId,
}

impl Ctx {
    pub fn with<R>(&self, f: impl FnOnce(&mut Shared) -> R) -> R {
        f(&mut lock(&self.shared))
    }

    pub fn now(&self) -> Clock {
        self.with(|s| s.clock)
    }

    pub fn cancelled(&self) -> bool {
        self.with(|s| s.cancelled.contains(&self.task))
    }

    /// Run `command` on `connection`'s host, given up as failed after
    /// `budget_ms` without an answer.
    pub fn exec(&self, connection: Uuid, command: &str, budget_ms: u64) -> Exec {
        Exec {
            ctx: self.clone(),
            connection,
            command: Some(command.to_owned()),
            id: None,
            budget_ms,
            deadline_ms: 0,
        }
    }

    /// POST `body` (JSON) to `url`, given up after `timeout_seconds`.
    pub fn post(&self, url: String, body: String, timeout_seconds: u32) -> Post {
        Post {
            ctx: self.clone(),
            request: Some((url, body, timeout_seconds)),
            id: None,
        }
    }

    /// Wait until `check` answers, or `deadline_ms` passes (`None`), or
    /// the operation is cancelled (`None`).
    pub fn until<R, F>(&self, deadline_ms: Option<u64>, check: F) -> Until<F>
    where
        F: FnMut(&mut Shared) -> Option<R> + Unpin,
    {
        Until {
            ctx: self.clone(),
            deadline_ms,
            check,
        }
    }

    /// Wait `ms` milliseconds; `false` when cancelled first.
    pub async fn sleep(&self, ms: u64) -> bool {
        let deadline = self.now().ms.saturating_add(ms);
        self.until(Some(deadline), |_| None::<()>).await;
        !self.cancelled()
    }
}

/// A command on a host, with a deadline: a host that never answers (a
/// shell hung on a dead mount, a link the session hasn't noticed is gone)
/// held the operation waiting on it, and the host's pairing lane with it,
/// for the rest of the app run. Past the deadline the command has failed
/// ([`HOST_SILENT`]) and its late answer is dropped.
pub(crate) struct Exec {
    ctx: Ctx,
    connection: Uuid,
    command: Option<String>,
    id: Option<u64>,
    budget_ms: u64,
    deadline_ms: u64,
}

impl Future for Exec {
    type Output = Result<Vec<u8>, ChannelError>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut shared = lock(&this.ctx.shared);
        if shared.cancelled.contains(&this.ctx.task) {
            if let Some(id) = this.id.take() {
                shared.execs.remove(&id);
            }
            shared.generation += 1;
            return Poll::Ready(Err(ChannelError(CANCELLED.to_owned())));
        }
        let Some(id) = this.id else {
            let id = shared.fresh_id();
            shared.execs.insert(id, Slot::Waiting);
            let command = this.command.take().unwrap_or_default();
            shared.outputs.push_back(Output::Exec {
                id,
                connection: this.connection,
                command,
            });
            this.id = Some(id);
            this.deadline_ms = shared.clock.ms.saturating_add(this.budget_ms);
            shared.deadlines.insert(this.deadline_ms);
            return Poll::Pending;
        };
        if matches!(shared.execs.get(&id), Some(Slot::Done(_))) {
            let Some(Slot::Done(result)) = shared.execs.remove(&id) else {
                unreachable!()
            };
            this.id = None;
            shared.generation += 1;
            return Poll::Ready(result.map_err(ChannelError));
        }
        if shared.clock.ms >= this.deadline_ms {
            shared.execs.remove(&id);
            this.id = None;
            shared.generation += 1;
            log::info!(target: "moshpit::care", "a command on {} got no answer in {} s \u{2014} given up", this.connection, this.budget_ms / 1000);
            return Poll::Ready(Err(ChannelError(HOST_SILENT.to_owned())));
        }
        shared.deadlines.insert(this.deadline_ms);
        Poll::Pending
    }
}

impl Drop for Exec {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            lock(&self.ctx.shared).execs.remove(&id);
        }
    }
}

/// A request to a relay.
pub(crate) struct Post {
    ctx: Ctx,
    request: Option<(String, String, u32)>,
    id: Option<u64>,
}

impl Future for Post {
    type Output = Result<HttpReply, String>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut shared = lock(&this.ctx.shared);
        if shared.cancelled.contains(&this.ctx.task) {
            if let Some(id) = this.id.take() {
                shared.posts.remove(&id);
            }
            shared.generation += 1;
            return Poll::Ready(Err(CANCELLED.to_owned()));
        }
        let Some(id) = this.id else {
            let id = shared.fresh_id();
            shared.posts.insert(id, Slot::Waiting);
            let (url, body, timeout_seconds) = this.request.take().unwrap_or_default();
            shared.outputs.push_back(Output::Post {
                id,
                url,
                body,
                timeout_seconds,
            });
            this.id = Some(id);
            return Poll::Pending;
        };
        if matches!(shared.posts.get(&id), Some(Slot::Done(_))) {
            let Some(Slot::Done(result)) = shared.posts.remove(&id) else {
                unreachable!()
            };
            this.id = None;
            shared.generation += 1;
            return Poll::Ready(result);
        }
        Poll::Pending
    }
}

impl Drop for Post {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            lock(&self.ctx.shared).posts.remove(&id);
        }
    }
}

/// A condition over the machine's state, with a deadline.
pub(crate) struct Until<F> {
    ctx: Ctx,
    deadline_ms: Option<u64>,
    check: F,
}

impl<R, F> Future for Until<F>
where
    F: FnMut(&mut Shared) -> Option<R> + Unpin,
{
    type Output = Option<R>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut shared = lock(&this.ctx.shared);
        if let Some(answer) = (this.check)(&mut shared) {
            shared.generation += 1;
            return Poll::Ready(Some(answer));
        }
        let timed_out = this.deadline_ms.is_some_and(|d| shared.clock.ms >= d);
        if timed_out || shared.cancelled.contains(&this.ctx.task) {
            shared.generation += 1;
            return Poll::Ready(None);
        }
        if let Some(deadline) = this.deadline_ms {
            shared.deadlines.insert(deadline);
        }
        Poll::Pending
    }
}

/// The installer's channel: a command on `connection`'s host, through the
/// machine's outputs, each given up after `budget_ms`.
pub(crate) struct Link {
    ctx: Ctx,
    connection: Uuid,
    budget_ms: u64,
}

impl Link {
    /// Care's channel ([`crate::Budgets::care_exec_ms`]).
    pub fn care(ctx: &Ctx, connection: Uuid) -> Self {
        let budget_ms = ctx.with(|s| s.config.budgets.care_exec_ms);
        Link {
            ctx: ctx.clone(),
            connection,
            budget_ms,
        }
    }

    /// A deleted connection's last commands
    /// ([`crate::Budgets::forget_on_delete_ms`], B377).
    pub fn forget(ctx: &Ctx, connection: Uuid) -> Self {
        let budget_ms = ctx.with(|s| s.config.budgets.forget_on_delete_ms);
        Link {
            ctx: ctx.clone(),
            connection,
            budget_ms,
        }
    }

    /// A Host Setup screen's channel ([`crate::Budgets::setup_exec_ms`]).
    pub fn setup(ctx: &Ctx, connection: Uuid) -> Self {
        let budget_ms = ctx.with(|s| s.config.budgets.setup_exec_ms);
        Link {
            ctx: ctx.clone(),
            connection,
            budget_ms,
        }
    }
}

impl HostChannel for Link {
    fn run(&self, command: &str) -> impl Future<Output = Result<Vec<u8>, ChannelError>> {
        self.ctx.exec(self.connection, command, self.budget_ms)
    }
}

/// A place in a connection's pairing queue (B123): held while the
/// operation runs, left when it is dropped.
pub(crate) struct LaneTicket {
    ctx: Ctx,
    connection: Uuid,
    id: u64,
    /// Another operation was queued for the host first: what the host held,
    /// read before, may be stale.
    pub waited: bool,
}

impl LaneTicket {
    /// Queue for `connection`'s host and wait for the operations queued
    /// before; `None` when cancelled meanwhile (`Task.checkCancellation`).
    pub async fn enter(ctx: &Ctx, connection: Uuid) -> Option<LaneTicket> {
        let (id, waited) = ctx.with(|s| {
            let id = s.fresh_id();
            let queue = s.lanes.entry(connection).or_default();
            let waited = !queue.is_empty();
            queue.push_back(id);
            (id, waited)
        });
        let ticket = LaneTicket {
            ctx: ctx.clone(),
            connection,
            id,
            waited,
        };
        ctx.until(None, move |s| {
            (s.lanes.get(&connection).and_then(VecDeque::front) == Some(&id)).then_some(())
        })
        .await;
        (!ctx.cancelled()).then_some(ticket)
    }
}

impl Drop for LaneTicket {
    fn drop(&mut self) {
        let mut shared = lock(&self.ctx.shared);
        if let Some(queue) = shared.lanes.get_mut(&self.connection) {
            queue.retain(|&id| id != self.id);
            if queue.is_empty() {
                shared.lanes.remove(&self.connection);
            }
        }
        shared.generation += 1;
    }
}

/// What `connection`'s host holds: its facts and manifest, the manifest
/// made to say what is actually there — files deleted or edited by hand
/// and hooks taken out of an agent's config read so (B315). Every read the
/// machine acts or reports on goes through here.
pub(crate) async fn look(
    installer: &moshpit_install::HostInstaller<'_, Link>,
    connection: Uuid,
    label: &str,
) -> Result<moshpit_install::InstallState, ChannelError> {
    let state = installer.inspect().await?;
    let conn = connection.to_string();
    let (mut state, drifts) = installer.verify(state, Some(&conn)).await?;
    for drift in drifts {
        log::info!(target: "moshpit::care", "{label}: changed on the host since it was installed: {drift:?}");
    }
    // Which agents' hooks have run here: Codex's trust step reads as done
    // once Codex has run one (B380).
    state.facts.marks = Some(installer.agents().await?);
    Ok(state)
}
