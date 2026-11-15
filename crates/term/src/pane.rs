//! A terminal fed from the byte channel (crates/stream) off the main
//! thread: the pane's own thread takes the role the native view had —
//! woken by the channel, it drains the output queue in order, in reads of
//! at most 64 KB, into the terminal, and writes the terminal's replies (DA,
//! CPR, colours) into the same input queue the view's keystrokes go to.
//! Then it tells the view a frame is ready; the view takes a [`Frame`]
//! (a copy of the changed rows) on its own thread and draws.
//!
//! A pane is fed one of two ways. [`Pane::attach`] reads a session's own
//! stream: the core pushes the host's bytes, the pane drains them, replies
//! go into that stream's input. [`Pane::local`] opens a stream of its own
//! that the view pushes into ([`Pane::push`]): for a view whose bytes come
//! from elsewhere (a session on the app's Swift transports, or a reader
//! that must see the stream's marks first) the parse still runs here, off
//! the main thread, in the order the bytes were pushed. Its resizes and
//! input resets travel in the same queue as marks, so each applies after
//! the bytes pushed before it and before the bytes after it; its replies
//! are kept for the view ([`Pane::take_replies`]), which sends them on its
//! own input path. The view may push marks of its own among the bytes
//! ([`Pane::push_mark`]): they reach `Callbacks::mark` exactly where they
//! were pushed, as a session stream's marks do.
//!
//! **Frames.** The pane's thread builds the frames itself, while it holds
//! the terminal anyway: at the end of every pass that changed something,
//! and every [`PUBLISH`] of a pass that goes on (a backlog: bytes waiting
//! faster than they parse, every read a full one). It leaves each in a
//! one-frame slot and tells the view, which takes it without the
//! terminal's lock ([`Pane::take_published`]): the main thread never waits
//! for a parse to show what is parsed, and a long catch-up keeps the screen
//! moving instead of freezing until it ends. A frame is built only once the
//! view took the one before: damage meanwhile stays in the terminal (the
//! next frame has it), and a view that takes a frame while newer bytes are
//! parsed asks for the rest. Frames carry the marks the parse reached, so
//! the view meets each mark with the frame that has the bytes before it.
//! The view's own changes (a scroll, a selection) are framed at once
//! ([`Pane::frame`]), after any published frame, in order.
//!
//! The parse's events (a title, a bell, OSC 7, a clipboard request) are
//! moved out of the terminal as they are met, so the view takes them
//! without its lock too ([`Pane::take_events`]).
//!
//! What the main thread pays per wake is applying a frame, not the parse
//! (docs/nsh/TERMINAL-CORE.md). The two rules of the byte path hold by
//! structure: one reader drains one queue, in order, and nothing is
//! dropped (the parser takes every byte; a synchronized update is held by
//! the emulator, not discarded, and drawn at its deadline).

use std::{
    sync::{
        Arc, Condvar, Mutex, MutexGuard, TryLockError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use moshpit_stream::{Mark, Stream, StreamId, view};

use crate::{Event, Frame, Terminal};

/// Called on the pane's thread when a frame is ready (hop to the view's
/// thread and return), and for each mark met in the output (with the
/// terminal, between the bytes before the mark and after it).
pub struct Callbacks {
    pub frame_ready: Box<dyn Fn() + Send + Sync>,
    pub mark: MarkFn,
}

/// What a pane does at a mark in the output.
pub type MarkFn = Box<dyn Fn(&mut Terminal, Mark) + Send + Sync>;

#[derive(Default)]
struct Signal {
    pending: bool,
    closed: bool,
}

struct Inner {
    id: StreamId,
    term: Mutex<Terminal>,
    signal: Mutex<Signal>,
    wake: Condvar,
    callbacks: Callbacks,
    /// A local pane's replies, kept for the view (there is no core to write
    /// them to); `None` for a pane on a session's stream.
    kept: Option<Mutex<Vec<u8>>>,
    /// A frame the pane's thread built for the view, not taken yet.
    published: Mutex<Option<Frame>>,
    /// The events the parse met, moved out of the terminal for the view.
    events: Mutex<Vec<Event>>,
    /// The parse changed something (bytes, marks) no frame has yet. Set and
    /// cleared under the terminal's lock.
    dirty: AtomicBool,
    /// The view is waiting for the terminal (a frame, a key): the pane's
    /// thread lets it in between two reads. The lock is not fair: during a
    /// burst the thread took it back read after read and the main thread
    /// waited up to 200 ms for one frame (measured on an iPad mini).
    wanted: AtomicUsize,
}

pub struct Pane {
    inner: Arc<Inner>,
    thread: Option<JoinHandle<()>>,
    /// A local pane's output end: what the view pushes.
    local: Option<Stream>,
}

/// Mark codes a local pane acts on itself (the session's codes are small
/// numbers; these never reach `Callbacks::mark`). A view's own marks
/// ([`Pane::push_mark`]) may not use this range.
const LOCAL_CODES: u32 = 0x8000_0000;
const LOCAL_RESIZE: u32 = 0x8000_0001;
const LOCAL_DISCARD_INPUT: u32 = 0x8000_0002;
const LOCAL_ECHO_ACK: u32 = 0x8000_0003;

/// The most read in one go before the terminal's lock is let go, so a view
/// taking a frame during a burst waits for at most one read's parse.
const READ: usize = 64 * 1024;

/// The events kept for a view that does not take them (a bell storm), as
/// many as the terminal keeps.
const EVENTS_KEPT: usize = 256;

/// How often a pass that goes on publishes a frame (about a display
/// refresh; a frame is published only once the view took the one before).
pub const PUBLISH: Duration = Duration::from_millis(16);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Pane {
    /// Feeds `term` from stream `id` from now on. `None` if the stream is
    /// gone.
    pub fn attach(id: StreamId, term: Terminal, callbacks: Callbacks) -> Option<Pane> {
        Self::start(id, term, callbacks, None)
    }

    /// A pane fed by the view itself ([`Pane::push`]) through a stream of
    /// its own, parsed on the pane's thread like any other.
    pub fn local(term: Terminal, callbacks: Callbacks) -> Option<Pane> {
        let (stream, input) = moshpit_stream::open();
        // Nothing reads the input: the view keeps what it sends.
        drop(input);
        Self::start(stream.id(), term, callbacks, Some(stream))
    }

    fn start(
        id: StreamId,
        term: Terminal,
        callbacks: Callbacks,
        local: Option<Stream>,
    ) -> Option<Pane> {
        let ledger_id = term.image_ledger_id();
        let inner = Arc::new(Inner {
            id,
            term: Mutex::new(term),
            signal: Mutex::new(Signal::default()),
            wake: Condvar::new(),
            callbacks,
            kept: local.as_ref().map(|_| Mutex::new(Vec::new())),
            published: Mutex::new(None),
            events: Mutex::new(Vec::new()),
            dirty: AtomicBool::new(false),
            wanted: AtomicUsize::new(0),
        });
        // Over the process's total of history images, another terminal
        // asks this one to let some go: at once if it is idle (B733).
        let shedder = Arc::downgrade(&inner);
        crate::history_images::set_trimmer(
            ledger_id,
            Arc::new(move || {
                if let Some(inner) = shedder.upgrade()
                    && let Ok(mut term) = inner.term.try_lock()
                {
                    term.shed_history_images();
                }
            }),
        );
        let waker = Arc::downgrade(&inner);
        let attached = view::attach(id, move || {
            if let Some(inner) = waker.upgrade() {
                lock(&inner.signal).pending = true;
                inner.wake.notify_one();
            }
        });
        if !attached {
            return None;
        }
        let worker = inner.clone();
        let thread = std::thread::Builder::new()
            .name(format!("moshpit-term-{id}"))
            .spawn(move || run(&worker))
            .ok()?;
        Some(Pane {
            inner,
            thread: Some(thread),
            local,
        })
    }

    /// A local pane: the remote's bytes, after everything pushed before.
    /// False for a pane on a session's stream (the core pushes there).
    pub fn push(&self, bytes: &[u8]) -> bool {
        match &self.local {
            Some(stream) => {
                stream.push(bytes);
                true
            }
            None => false,
        }
    }

    /// A local pane: `mark` after everything pushed before it, reaching
    /// `Callbacks::mark` between those bytes and the ones pushed after.
    /// False for a pane on a session's stream (the core marks there) and
    /// for a code in the pane's own range.
    pub fn push_mark(&self, mark: Mark) -> bool {
        match &self.local {
            Some(stream) if mark.code & LOCAL_CODES == 0 => {
                stream.mark(mark);
                true
            }
            _ => false,
        }
    }

    /// A local pane's replies to the remote (DA, CPR, colours, …) since the
    /// last call, in order.
    pub fn take_replies(&self) -> Vec<u8> {
        self.inner
            .kept
            .as_ref()
            .map(|k| std::mem::take(&mut *lock(k)))
            .unwrap_or_default()
    }

    /// Forgets a half-read sequence (a reconnect: the next bytes are not the
    /// dead stream's continuation). A local pane does it after the bytes
    /// pushed so far, a pane on a session's stream at once.
    pub fn discard_pending_input(&self) {
        match &self.local {
            Some(stream) => stream.mark(Mark {
                code: LOCAL_DISCARD_INPUT,
                arg: 0,
            }),
            None => lock(&self.inner.term).discard_pending_input(),
        }
    }

    /// mosh's echo ack, judged after the bytes of its frame: a local pane
    /// after the bytes pushed so far, any other at once.
    pub fn echo_ack(&self, ack: u64) {
        match &self.local {
            Some(stream) => stream.mark(Mark {
                code: LOCAL_ECHO_ACK,
                arg: ack.min(i64::MAX as u64) as i64,
            }),
            None => lock(&self.inner.term).predict_echo_ack(ack),
        }
    }

    /// Whether the view feeds this pane ([`Pane::local`]).
    pub fn is_local(&self) -> bool {
        self.local.is_some()
    }

    /// The stream this pane reads.
    pub fn id(&self) -> StreamId {
        self.inner.id
    }

    /// Runs `f` with the terminal (the pane's thread waits meanwhile).
    pub fn with<R>(&self, f: impl FnOnce(&mut Terminal) -> R) -> R {
        self.inner.wanted.fetch_add(1, Ordering::AcqRel);
        let out = f(&mut lock(&self.inner.term));
        self.inner.wanted.fetch_sub(1, Ordering::AcqRel);
        out
    }

    /// The next frame, in order: the one the pane's thread published if
    /// the view has not taken it (what changed after it is framed now and
    /// left for [`Pane::take_published`]), else what changed since the last
    /// frame. A view framing a change of its own (a scroll) takes this,
    /// then what is published.
    ///
    /// Never [`Terminal::frame`] through [`Pane::with`]: a published frame
    /// would be skipped, and its rows and marks with it.
    pub fn frame(&self) -> Frame {
        self.with(|t| {
            let fresh = t.frame();
            self.inner.dirty.store(false, Ordering::Release);
            let mut slot = lock(&self.inner.published);
            match slot.take() {
                Some(earlier) => {
                    *slot = Some(fresh);
                    earlier
                }
                None => fresh,
            }
        })
    }

    /// The frame the pane's thread published, if one waits: taken without
    /// the terminal's lock (the view's wake never waits for a parse). If
    /// the parse has moved on since, the pane's thread is asked for the
    /// rest.
    pub fn take_published(&self) -> Option<Frame> {
        let frame = lock(&self.inner.published).take()?;
        if self.inner.dirty.load(Ordering::Acquire) {
            lock(&self.inner.signal).pending = true;
            self.inner.wake.notify_one();
        }
        Some(frame)
    }

    /// The events the parse met since the last call (a title, a bell, OSC 7,
    /// a clipboard request), in order, taken without waiting for the
    /// terminal: the pane's thread moves them out as it parses. Any a call
    /// of the view's own to the terminal raised come after, if the terminal
    /// is free.
    pub fn take_events(&self) -> Vec<Event> {
        let mut events = std::mem::take(&mut *lock(&self.inner.events));
        match self.inner.term.try_lock() {
            Ok(mut term) => events.extend(term.take_events()),
            Err(TryLockError::Poisoned(e)) => events.extend(e.into_inner().take_events()),
            Err(TryLockError::WouldBlock) => {}
        }
        events
    }

    /// The view's new size. On a session's stream the grid follows at once,
    /// and the core hears of it after the bytes written so far (the
    /// channel's order); a local pane's grid follows after the bytes pushed
    /// so far, as the remote drew them for the old size.
    pub fn resize(&self, cols: u16, rows: u16) {
        if let Some(stream) = &self.local {
            stream.mark(Mark {
                code: LOCAL_RESIZE,
                arg: (cols as i64) << 16 | rows as i64,
            });
            return;
        }
        lock(&self.inner.term).resize(cols, rows);
        view::resize(self.inner.id, cols, rows);
    }

    /// Keystrokes and pastes, encoded by the view: the same input queue.
    pub fn write(&self, bytes: &[u8]) -> bool {
        view::write(self.inner.id, bytes)
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        view::detach(self.inner.id);
        lock(&self.inner.signal).closed = true;
        self.inner.wake.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(inner: &Inner) {
    let mut buf = vec![0u8; READ];
    loop {
        // Wait for output, for a synchronized update's deadline, or to close.
        {
            let deadline = lock(&inner.term).sync_deadline();
            let mut s = lock(&inner.signal);
            while !s.pending && !s.closed {
                match deadline {
                    Some(at) => {
                        let now = Instant::now();
                        if at <= now {
                            break;
                        }
                        s = inner
                            .wake
                            .wait_timeout(s, at - now)
                            .unwrap_or_else(|e| e.into_inner())
                            .0;
                        if Instant::now() >= at {
                            break;
                        }
                    }
                    None => s = inner.wake.wait(s).unwrap_or_else(|e| e.into_inner()),
                }
            }
            if s.closed {
                return;
            }
            s.pending = false;
        }

        let mut published_at = Instant::now();
        loop {
            let (n, mark) = view::read_marked(inner.id, &mut buf);
            if n > 0 || mark.is_some() {
                let mut term = lock(&inner.term);
                inner.dirty.store(true, Ordering::Release);
                if n > 0 {
                    term.feed(&buf[..n]);
                    keep_events(inner, &mut term);
                }
                if let Some(mark) = mark {
                    match mark.code {
                        LOCAL_RESIZE if inner.kept.is_some() => {
                            term.resize((mark.arg >> 16) as u16, mark.arg as u16)
                        }
                        LOCAL_DISCARD_INPUT if inner.kept.is_some() => term.discard_pending_input(),
                        LOCAL_ECHO_ACK if inner.kept.is_some() => {
                            term.predict_echo_ack(mark.arg.max(0) as u64)
                        }
                        _ => (inner.callbacks.mark)(&mut term, mark),
                    }
                    keep_events(inner, &mut term);
                }
            }
            if n < buf.len() && mark.is_none() {
                break;
            }
            // A pass that goes on (a backlog) shows what it parsed as it
            // goes: a frame about every display refresh.
            if published_at.elapsed() >= PUBLISH {
                published_at = Instant::now();
                if publish(inner) {
                    (inner.callbacks.frame_ready)();
                }
            }
            // Between two reads of a burst: the view first, if it waits.
            let since = Instant::now();
            while inner.wanted.load(Ordering::Acquire) > 0
                && since.elapsed() < std::time::Duration::from_millis(8)
            {
                std::thread::yield_now();
            }
        }

        let (replies, holding) = {
            let mut term = lock(&inner.term);
            if term.sync_deadline().is_some_and(|at| at <= Instant::now()) {
                term.flush_sync();
                inner.dirty.store(true, Ordering::Release);
            }
            (term.take_replies(), term.sync_deadline().is_some())
        };
        let mut owed = false;
        if !replies.is_empty() {
            match &inner.kept {
                Some(kept) => {
                    lock(kept).extend_from_slice(&replies);
                    owed = true;
                }
                None => {
                    view::write(inner.id, &replies);
                }
            }
        }
        // The pass's frame (also when the view asked for the rest of one it
        // took). Inside a synchronized update the grid has not changed yet;
        // a local pane's view must still come for the replies it sends.
        let shown = !holding && publish(inner);
        if shown || owed {
            (inner.callbacks.frame_ready)();
        }
    }
}

/// The events the parse met, out of the terminal to where the view takes
/// them (under the terminal's lock: an OSC 7 event is filled in at the end
/// of its read).
fn keep_events(inner: &Inner, term: &mut Terminal) {
    let met = term.take_events();
    if met.is_empty() {
        return;
    }
    let mut kept = lock(&inner.events);
    kept.extend(met);
    let over = kept.len().saturating_sub(EVENTS_KEPT);
    kept.drain(..over);
}

/// Frames what the parse changed and leaves it for the view, if the view
/// took the frame before (else it asks for the rest when it takes that
/// one: the damage stays in the terminal meanwhile). True if a frame was
/// published.
fn publish(inner: &Inner) -> bool {
    let mut term = lock(&inner.term);
    if !inner.dirty.load(Ordering::Acquire) || term.sync_deadline().is_some() {
        return false;
    }
    // Only the pane's thread fills the slot, under the terminal's lock (as
    // the view's own framing does): empty now, it stays empty until filled
    // here.
    if lock(&inner.published).is_some() {
        return false;
    }
    let frame = term.frame();
    inner.dirty.store(false, Ordering::Release);
    *lock(&inner.published) = Some(frame);
    true
}
