//! The view's end of a stream, by id: what [`crate::ffi`] wraps for Swift
//! and what a Rust view (or a test) calls directly. Every function is
//! thread-safe; an unknown or finished id does nothing.

use crate::{Mark, Stats, StreamId, find, shared::Unregister, unregister};

/// Makes `waker` the view's for stream `id`: it is called on any thread
/// when output is waiting, at most once until the view has drained the
/// queue ([`read`] until it returns less than asked), and right away if
/// output is already waiting. Replaces an earlier waker. `false` if the
/// stream is gone.
///
/// The waker must be quick (hop to the view's thread and return) and must
/// not attach, detach or push; reading is fine.
pub fn attach(id: StreamId, waker: impl Fn() + Send + Sync + 'static) -> bool {
    let Some(shared) = find(id) else {
        return false;
    };
    shared.attach(Box::new(waker));
    true
}

/// Drops the view's waker for stream `id`. Once this returns the waker is
/// not called again (a wake already running finishes first), so whatever it
/// holds can be freed.
pub fn detach(id: StreamId) {
    if let Some(shared) = find(id)
        && shared.detach() == Unregister::Remove
    {
        unregister(id);
    }
}

/// Moves up to `buf.len()` bytes of output into `buf`, oldest first;
/// returns how many. Less than `buf.len()` means the queue is drained and
/// the next push wakes the view again.
pub fn read(id: StreamId, buf: &mut [u8]) -> usize {
    find(id).map_or(0, |shared| shared.read(buf))
}

/// Moves up to `buf.len()` bytes of output into `buf`, oldest first,
/// stopping at the next mark: the bytes before it, and the mark once they
/// are read. Drained when it returns fewer bytes than asked and no mark.
pub fn read_marked(id: StreamId, buf: &mut [u8]) -> (usize, Option<Mark>) {
    find(id).map_or((0, None), |shared| shared.read_marked(buf))
}

/// Appends `bytes` to the input, whole, after everything written before.
/// `false` if the stream is gone or its core's end closed.
pub fn write(id: StreamId, bytes: &[u8]) -> bool {
    find(id).is_some_and(|shared| shared.write(bytes))
}

/// Tells the core the view's size in cells, after the bytes written so far.
/// `false` if the stream is gone or its core's end closed.
pub fn resize(id: StreamId, cols: u16, rows: u16) -> bool {
    find(id).is_some_and(|shared| shared.resize(cols, rows))
}

/// Queues a paste after the bytes written so far, in its place among them:
/// the core sends it as one paste, bracketed by the pane's real mode
/// (B361). `false` if the stream is gone or its core's end closed.
pub fn paste(id: StreamId, text: &str) -> bool {
    find(id).is_some_and(|shared| shared.paste(text.to_owned()))
}

/// Queues the shortcut bar's keys after the bytes written so far, in their
/// place among them: keystrokes while the session is live, never kept for
/// later (B476). `false` if the stream is gone or its core's end closed.
pub fn bar(id: StreamId, bytes: &[u8]) -> bool {
    find(id).is_some_and(|shared| shared.bar(bytes.to_vec()))
}

/// The output's counters, if the stream is live.
pub fn stats(id: StreamId) -> Option<Stats> {
    find(id).map(|shared| shared.stats())
}
