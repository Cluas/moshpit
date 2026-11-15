//! The registry: every session the app has, by id. Plain calls in, plain
//! data out — what the FFI layer wraps.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use std::time::Duration;

use moshpit_ssh::{LinkFault, Ssh};
use moshpit_tmux::model::Snapshot;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use crate::event::{Event, EventKind, Failure, SessionId};
use crate::herdr::{HerdrAction, HerdrView, unanswered};
use crate::services::Services;
use crate::session::{Actor, Msg, SessionView};
use crate::spec::{ConnectionSpec, Timing};
use crate::tmux_leg::{TmuxAction, TmuxRefusal, TmuxView};

struct Entry {
    mailbox: mpsc::UnboundedSender<Msg>,
    view: Arc<Mutex<SessionView>>,
}

struct Inner {
    runtime: Handle,
    services: Services,
    timing: Timing,
    events: mpsc::UnboundedSender<Event>,
    sessions: Mutex<HashMap<SessionId, Entry>>,
    next: AtomicU64,
    /// The tokens herdr's answers are named by.
    tokens: AtomicU64,
    foreground: AtomicBool,
    /// Resume in place as last set ([`Sessions::set_resume_in_place`]):
    /// what a new session starts with.
    resume_in_place: AtomicBool,
    visible: Mutex<Option<SessionId>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // The registry is gone: so is every session it had.
        let sessions = self.sessions.get_mut().unwrap_or_else(|e| e.into_inner());
        for (_, entry) in sessions.drain() {
            let _ = entry.mailbox.send(Msg::Disconnect(None));
        }
    }
}

/// Every session, and the app's lifecycle and network as they reach them.
/// Cheap to clone; every method returns at once (the work runs on the
/// runtime it was made with).
#[derive(Clone)]
pub struct Sessions {
    inner: Arc<Inner>,
}

/// The events of every session, in the order each session produced them.
pub type Events = mpsc::UnboundedReceiver<Event>;

/// How long [`Sessions::flushed`] waits for the slowest session.
pub const FLUSH_BOUND: Duration = Duration::from_secs(5);

impl Sessions {
    /// A registry on `runtime` (the core's, `moshpit_runtime::handle()`),
    /// with the app in the foreground.
    pub fn new(runtime: Handle, services: Services, timing: Timing) -> (Sessions, Events) {
        let (events, rx) = mpsc::unbounded_channel();
        let resume_in_place = AtomicBool::new(timing.resume_in_place);
        let sessions = Sessions {
            inner: Arc::new(Inner {
                runtime,
                services,
                timing,
                resume_in_place,
                events,
                sessions: Mutex::new(HashMap::new()),
                next: AtomicU64::new(1),
                tokens: AtomicU64::new(1),
                foreground: AtomicBool::new(true),
                visible: Mutex::new(None),
            }),
        };
        (sessions, rx)
    }

    /// Opens a session to `spec` and connects it.
    pub fn connect(&self, spec: ConnectionSpec) -> SessionId {
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let (mailbox, rx) = mpsc::unbounded_channel();
        let view = Arc::new(Mutex::new(SessionView::initial()));
        let timing = Timing {
            resume_in_place: self.inner.resume_in_place.load(Ordering::SeqCst),
            ..self.inner.timing.clone()
        };
        let actor = Actor::new(
            id,
            spec,
            timing,
            self.inner.services.clone(),
            self.inner.foreground.load(Ordering::SeqCst),
            rx,
            mailbox.clone(),
            self.inner.events.clone(),
            view.clone(),
        );
        let _ = mailbox.send(Msg::Connect);
        self.inner.runtime.spawn(actor.run());
        self.lock().insert(id, Entry { mailbox, view });
        id
    }

    /// The person's "try again" (after a failure or an end).
    pub fn reconnect(&self, id: SessionId) {
        self.send(id, Msg::Connect);
    }

    /// The session's connection was saved while it runs (B329): its next
    /// connect — a reconnect after a drop, or the person's — goes to
    /// `spec`'s address as `spec`'s user, through `spec`'s proxy (and asks
    /// the credentials for the connection again, as every attempt does).
    /// The transport live now stays where it is; the rest of the spec (the
    /// protocol, the multiplexer) is the session's own.
    pub fn respec(&self, id: SessionId, spec: &ConnectionSpec) {
        self.send(
            id,
            Msg::Respec {
                target: spec.target.clone(),
                proxy: spec.proxy.clone(),
            },
        );
    }

    /// Closes a session for good and forgets it. Returns once it is torn
    /// down (or at once if it is unknown): await the receiver for that.
    pub fn disconnect(&self, id: SessionId) -> oneshot::Receiver<()> {
        let (done, finished) = oneshot::channel();
        let entry = self.lock().remove(&id);
        let mut visible = self.inner.visible.lock().unwrap_or_else(|e| e.into_inner());
        if *visible == Some(id) {
            *visible = None;
        }
        match entry {
            Some(entry) => {
                let _ = entry.mailbox.send(Msg::Disconnect(Some(done)));
            }
            None => {
                let _ = done.send(());
            }
        }
        finished
    }

    /// The person's Repaint (the mosh diagnostics): mosh-server redraws
    /// the whole screen. Nothing for a session without mosh (B338).
    pub fn repaint(&self, id: SessionId) {
        self.send(id, Msg::Repaint);
    }

    /// A mosh session that hears nothing back: use SSH instead.
    pub fn switch_to_ssh(&self, id: SessionId) {
        self.send(id, Msg::SwitchToSsh);
    }

    /// The person's answer to a [`crate::EventKind::HostKeyPrompt`].
    pub fn answer_host_key(&self, id: SessionId, prompt: u64, trust: bool) {
        self.send(id, Msg::AnswerHostKey { prompt, trust });
    }

    /// The person's answer to a [`crate::EventKind::SignInQuestion`]
    /// (`None`: declined).
    pub fn answer_question(&self, id: SessionId, prompt: u64, answer: Option<String>) {
        self.send(id, Msg::AnswerQuestion { prompt, answer });
    }

    /// Something for the session's tmux to do — unless it is refused as
    /// asked ([`TmuxAction::refusal`]): then nothing is sent, and the host
    /// says why.
    pub fn tmux(&self, id: SessionId, action: TmuxAction) -> Result<(), TmuxRefusal> {
        if let Some(refusal) = action.refusal() {
            return Err(refusal);
        }
        self.send(id, Msg::Tmux(action));
        Ok(())
    }

    /// Something for the session's herdr to do. Returns the token its
    /// answer ([`crate::EventKind::HerdrAnswer`]) carries: every intent is
    /// answered once — the ones with nothing to report with
    /// [`crate::HerdrAnswer::Done`], and every one the session couldn't take
    /// (not live, no herdr) at once, with what "couldn't" reads as.
    pub fn herdr(&self, id: SessionId, action: HerdrAction) -> u64 {
        let token = self.inner.tokens.fetch_add(1, Ordering::Relaxed);
        let sent = self.lock().get(&id).is_some_and(|entry| {
            entry
                .mailbox
                .send(Msg::Herdr {
                    action: action.clone(),
                    token,
                })
                .is_ok()
        });
        if !sent {
            let _ = self.inner.events.send(Event {
                session: id,
                kind: EventKind::HerdrAnswer {
                    token,
                    answer: unanswered(&action),
                },
            });
        }
        token
    }

    /// A paste into `pane` of session `id` (the terminal on screen): one
    /// bracketed paste, which tmux and herdr fit to the pane's real mode.
    /// Dropped when the session isn't live, like typing.
    pub fn paste(&self, id: SessionId, pane: &str, text: String) {
        self.send(
            id,
            Msg::Paste {
                pane: pane.to_owned(),
                text,
            },
        );
    }

    /// The New agent task form went away while the task asked for under
    /// `token` was starting: it stops where it stands (and is answered).
    pub fn herdr_cancel_task(&self, id: SessionId, token: u64) {
        self.send(id, Msg::HerdrCancel { token });
    }

    /// Content for one pane of session `id`, on purpose: a notification's
    /// reply, a share. `paste`: as a bracketed paste (the multiplexer fits
    /// it to the pane's real mode; a plain shell gets the markers). Unlike
    /// typing it waits out a reconnect (at most [`crate::DELIVERY_WAIT`]),
    /// and herdr switches to the pane and holds it for the pane's first
    /// frame. Resolves to whether it went out.
    pub fn deliver(
        &self,
        id: SessionId,
        pane: &str,
        bytes: Vec<u8>,
        paste: bool,
    ) -> oneshot::Receiver<bool> {
        let (reply, answer) = oneshot::channel();
        match self.lock().get(&id) {
            Some(entry) => {
                let _ = entry.mailbox.send(Msg::Deliver {
                    pane: pane.to_owned(),
                    bytes,
                    paste,
                    reply,
                });
            }
            None => {
                let _ = reply.send(false);
            }
        }
        answer
    }

    /// The device was unlocked: sessions that failed on a locked keychain
    /// dial again (B201).
    pub fn device_unlocked(&self) {
        self.broadcast(|| Msg::Unlocked);
    }

    /// The app came to the foreground (`true`) or left it.
    pub fn set_foreground(&self, foreground: bool) {
        self.inner.foreground.store(foreground, Ordering::SeqCst);
        self.broadcast(|| {
            if foreground {
                Msg::Foreground
            } else {
                Msg::Background
            }
        });
    }

    /// Resume in place on (`true`) or off (B365): every session follows it
    /// from its next return, and a session opened after this starts with
    /// it. The app sets it as its switch says before every move to the
    /// background and back; nothing is sent when it doesn't change.
    pub fn set_resume_in_place(&self, on: bool) {
        if self.inner.resume_in_place.swap(on, Ordering::SeqCst) == on {
            return;
        }
        self.broadcast(|| Msg::ResumeInPlace(on));
    }

    /// Resume in place as last set: what a new session starts with.
    pub fn resume_in_place(&self) -> bool {
        self.inner.resume_in_place.load(Ordering::SeqCst)
    }

    /// The device's network path changed (an interface, a VPN tunnel).
    pub fn network_changed(&self) {
        self.broadcast(|| Msg::NetworkChanged);
    }

    /// The session whose terminal is on screen (`None` on Home): only it
    /// takes its tmux window back when the app returns.
    pub fn set_visible(&self, id: Option<SessionId>) {
        let mut visible = self.inner.visible.lock().unwrap_or_else(|e| e.into_inner());
        if *visible == id {
            return;
        }
        if let Some(old) = visible.take() {
            self.send(old, Msg::Visible(false));
        }
        if let Some(new) = id {
            self.send(new, Msg::Visible(true));
        }
        *visible = id;
    }

    /// What the UI reads about a session.
    pub fn view(&self, id: SessionId) -> Option<SessionView> {
        self.lock()
            .get(&id)
            .map(|entry| entry.view.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    /// The session's tmux tree, when tmux runs.
    pub fn tmux_snapshot(&self, id: SessionId) -> Option<Snapshot> {
        self.view(id).and_then(|view| view.tmux)
    }

    /// The session's tmux tree and what the chrome draws around it.
    pub fn tmux_view(&self, id: SessionId) -> Option<TmuxView> {
        self.lock().get(&id).and_then(|entry| {
            entry
                .view
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .tmux_view
                .clone()
        })
    }

    /// The session's herdr tree and what is said around it.
    pub fn herdr_view(&self, id: SessionId) -> Option<HerdrView> {
        self.lock().get(&id).and_then(|entry| {
            entry
                .view
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .herdr
                .clone()
        })
    }

    /// Resolves once every session has written what it was asked to send
    /// its server so far — on the way to the background, the window pins it
    /// handed back — or after [`FLUSH_BOUND`], whichever comes first: what
    /// a platform holds the process awake for (iOS's background assertion).
    pub fn flushed(&self) -> oneshot::Receiver<()> {
        let (done, finished) = oneshot::channel();
        let waits: Vec<oneshot::Receiver<()>> = self
            .lock()
            .values()
            .map(|entry| {
                let (tx, rx) = oneshot::channel();
                let _ = entry.mailbox.send(Msg::Flush(tx));
                rx
            })
            .collect();
        self.inner.runtime.spawn(async move {
            let all = async {
                for wait in waits {
                    let _ = wait.await;
                }
            };
            let _ = tokio::time::timeout(FLUSH_BOUND, all).await;
            let _ = done.send(());
        });
        finished
    }

    /// Breaks the live connection of session `id` the way a network would
    /// (a debug build's fault injection; the session redials as it would
    /// after the real thing).
    #[doc(hidden)]
    pub fn inject_fault(&self, id: SessionId, fault: LinkFault) {
        self.send(id, Msg::Fault(fault));
    }

    /// Ends session `id`'s mosh loop as if its transport had died (a test's
    /// fault: the session reconnects, starting a new mosh-server).
    #[doc(hidden)]
    pub fn inject_mosh_loss(&self, id: SessionId) {
        self.send(id, Msg::MoshLoss);
    }

    /// An SSH connection to the host of session `id`, for what runs beside
    /// the terminal (SFTP, uploads, a file preview, an exec): the session's
    /// own when it has one, else a side connection it dials once and keeps
    /// (mosh). Fails while the session isn't live. Probe it before trusting
    /// it after a suspension: a socket iOS killed still looks open.
    pub fn borrow_ssh(&self, id: SessionId) -> oneshot::Receiver<Result<Ssh, Failure>> {
        let (reply, answer) = oneshot::channel();
        match self.lock().get(&id) {
            Some(entry) => {
                let _ = entry.mailbox.send(Msg::BorrowSsh(reply));
            }
            None => {
                let _ = reply.send(Err(Failure::lost("no such session")));
            }
        }
        answer
    }

    /// The live sessions' ids.
    pub fn ids(&self) -> Vec<SessionId> {
        let mut ids: Vec<SessionId> = self.lock().keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, Entry>> {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, id: SessionId, message: Msg) {
        if let Some(entry) = self.lock().get(&id) {
            let _ = entry.mailbox.send(message);
        }
    }

    fn broadcast(&self, message: impl Fn() -> Msg) {
        for entry in self.lock().values() {
            let _ = entry.mailbox.send(message());
        }
    }
}
