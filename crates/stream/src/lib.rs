//! The byte channel (`docs/nsh/DESIGN.md`, 3.3 and 3.4): one ordered,
//! lossless byte queue per pane in each direction between the Rust core and
//! the platform's native terminal view, with no polling and nothing but
//! bytes on the way.
//!
//! - **Output** (core → view): the core [`Stream::push`]es what the host
//!   sent. The view attaches a waker ([`view::attach`],
//!   [`ffi::mp_stream_attach`]) and, when woken, drains the queue on its own
//!   thread ([`view::read`], [`ffi::mp_stream_read`]). At most one wake is
//!   outstanding until the view has drained the queue, so a burst of pushes
//!   costs the view one hop to its thread.
//! - **Input** (view → core): the view writes keystrokes synchronously
//!   from any thread ([`view::write`], [`ffi::mp_stream_write`]), hands over
//!   pastes ([`view::paste`], [`ffi::mp_stream_paste`]) and reports its size
//!   ([`view::resize`]); the core takes them all, in order, from the async
//!   stream [`Input`], with the bytes written between two pastes or resizes
//!   coalesced into one chunk.
//!
//! - **Marks** (core → view, in the output): what the view must do between
//!   two bytes — lift a cover once the repaint before it is fed, apply a
//!   held resize before the frame after it ([`Stream::mark`]). A view that
//!   wants them reads with [`view::read_marked`] / [`ffi::mp_stream_read_marked`],
//!   which stops at each mark; a plain read passes over them.
//!
//! Both queues are unbounded and never drop bytes. Order is a property of
//! the structure, not of the callers' care: one queue per direction, every
//! push or write appended whole under the queue's lock. Concurrent producers
//! interleave whole chunks; each producer's chunks stay in its order.
//!
//! Streams are named by a [`StreamId`] (what the UI hands the view as a
//! prop) and live until both ends are done: the core's [`Stream`] dropped
//! and the view detached.

use std::{
    collections::HashMap,
    pin::Pin,
    sync::{
        Arc, LazyLock, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
};

pub mod ffi;
#[cfg(all(test, moshpit_loom))]
mod loom_tests;
mod queue;
mod shared;
mod sync;
pub mod view;

use shared::{Shared, Unregister};

/// A stream's name: what the UI hands the view. Never 0, never reused.
pub type StreamId = u64;

/// What the view sent the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// Bytes (keystrokes, the terminal's replies, a paste the view
    /// bracketed itself), every byte written since the last event up to the
    /// next paste or resize.
    Bytes(Vec<u8>),
    /// The view's size in cells changed.
    Resize { cols: u16, rows: u16 },
    /// A paste, in its place among the keystrokes: text for the core to
    /// send as one paste, bracketed by the pane's real mode (B361).
    Paste(String),
}

/// Something the view does at a point in the output: a code the core and
/// the view agree on, and an argument for it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mark {
    pub code: u32,
    pub arg: i64,
}

/// Counters of a stream's output, for benches and diagnostics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Pushes (non-empty).
    pub pushes: u64,
    /// Bytes pushed.
    pub pushed: u64,
    /// Bytes read by the view.
    pub read: u64,
    /// Bytes waiting now.
    pub queued: u64,
    /// The most bytes ever waiting.
    pub peak_queued: u64,
    /// Wakes sent to the view.
    pub wakes: u64,
    /// Times the view drained the queue after a wake.
    pub drains: u64,
    /// Marks put into the output.
    pub marks: u64,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Live streams by id: the view's way in.
static STREAMS: LazyLock<RwLock<HashMap<StreamId, Arc<Shared>>>> = LazyLock::new(RwLock::default);

/// The stream `id`, if it is live.
fn find(id: StreamId) -> Option<Arc<Shared>> {
    STREAMS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned()
}

fn unregister(id: StreamId) {
    STREAMS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
}

/// Opens a stream: the core's end, which pushes output, and the input it
/// takes. The view finds it by [`Stream::id`].
pub fn open() -> (Stream, Input) {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let shared = Arc::new(Shared::new());
    STREAMS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, shared.clone());
    (
        Stream {
            id,
            shared: shared.clone(),
        },
        Input { shared },
    )
}

/// The core's end of a stream. Share it (`Arc<Stream>`) to push from
/// several tasks; dropping it closes the stream.
pub struct Stream {
    id: StreamId,
    shared: Arc<Shared>,
}

impl Stream {
    /// The id the view finds the stream by.
    pub fn id(&self) -> StreamId {
        self.id
    }

    /// Appends `bytes` to the output, whole, after everything pushed
    /// before; wakes the view if no wake is outstanding. Never blocks for
    /// longer than a copy of `bytes` under the queue's lock.
    pub fn push(&self, bytes: &[u8]) {
        self.shared.push(bytes);
    }

    /// Puts `mark` into the output after everything pushed so far: a view
    /// reading with marks meets it exactly there. Wakes the view as a push
    /// does.
    pub fn mark(&self, mark: Mark) {
        self.shared.push_mark(mark);
    }

    /// The output's counters.
    pub fn stats(&self) -> Stats {
        self.shared.stats()
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if self.shared.close() == Unregister::Remove {
            unregister(self.id);
        }
    }
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream").field("id", &self.id).finish()
    }
}

/// What the view sends the core, as an async stream of [`InputEvent`]s in
/// the order the view sent them. Ends once the [`Stream`] is dropped and
/// everything sent before was taken. Dropping it refuses the view's input
/// from then on.
pub struct Input {
    shared: Arc<Shared>,
}

impl Input {
    /// The next event; `None` at the end.
    pub async fn next(&mut self) -> Option<InputEvent> {
        std::future::poll_fn(|cx| self.shared.poll_input(cx)).await
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        // Nothing will take input again: the view's writes fail from now on
        // instead of piling up.
        self.shared.close_input();
    }
}

impl futures_core::Stream for Input {
    type Item = InputEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<InputEvent>> {
        self.shared.poll_input(cx)
    }
}

impl std::fmt::Debug for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Input").finish_non_exhaustive()
    }
}
