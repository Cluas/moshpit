//! The C ABI of the view's end (`include/moshpit_stream.h`): what the
//! native terminal view calls on its hot path, with no JSON and no copies
//! beyond the one into its buffer. Thin wrappers of [`crate::view`].

use std::ffi::c_void;

use crate::{Mark, StreamId, view};

/// Tells the view output is waiting; called on any thread with the `ctx`
/// given to [`mp_stream_attach`].
pub type MpStreamWaker = unsafe extern "C" fn(ctx: *mut c_void);

/// A stream's counters ([`crate::Stats`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MpStreamStats {
    pub pushes: u64,
    pub pushed: u64,
    pub read: u64,
    pub queued: u64,
    pub peak_queued: u64,
    pub wakes: u64,
    pub drains: u64,
    pub marks: u64,
}

/// A mark in the output ([`crate::Mark`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MpStreamMark {
    pub code: u32,
    pub arg: i64,
}

impl From<Mark> for MpStreamMark {
    fn from(mark: Mark) -> Self {
        Self {
            code: mark.code,
            arg: mark.arg,
        }
    }
}

/// The view's waker as C gave it.
struct CWaker {
    waker: MpStreamWaker,
    ctx: *mut c_void,
}

// SAFETY: `mp_stream_attach`'s contract: the waker may be called with `ctx`
// on any thread until `mp_stream_detach` returns.
unsafe impl Send for CWaker {}
// SAFETY: as above.
unsafe impl Sync for CWaker {}

impl CWaker {
    fn wake(&self) {
        // SAFETY: `mp_stream_attach`'s contract.
        unsafe { (self.waker)(self.ctx) }
    }
}

/// Makes `waker(ctx)` the view's waker for stream `id` (`NULL`: detaches).
/// It is called on any thread when output is waiting, at most once until
/// the view has drained the queue, and right away if output is already
/// waiting. Returns `false` if the stream is gone.
///
/// # Safety
///
/// `waker` must be safe to call with `ctx` from any thread until
/// [`mp_stream_detach`] returns (or another attach replaced it). It must not
/// call `mp_stream_attach`, `mp_stream_detach` or push; it may read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_stream_attach(
    id: StreamId,
    waker: Option<MpStreamWaker>,
    ctx: *mut c_void,
) -> bool {
    match waker {
        Some(waker) => {
            let waker = CWaker { waker, ctx };
            view::attach(id, move || waker.wake())
        }
        None => {
            view::detach(id);
            true
        }
    }
}

/// Drops the view's waker for stream `id`. Once this returns it is not
/// called again, so its `ctx` can be freed.
#[unsafe(no_mangle)]
pub extern "C" fn mp_stream_detach(id: StreamId) {
    view::detach(id);
}

/// Moves up to `cap` bytes of output into `buf`, oldest first; returns how
/// many. Less than `cap` means the queue is drained and the next push wakes
/// the view again.
///
/// # Safety
///
/// `buf` must be valid for `cap` bytes of writes (or `cap` 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_stream_read(id: StreamId, buf: *mut u8, cap: usize) -> usize {
    if buf.is_null() || cap == 0 {
        return 0;
    }
    // SAFETY: guaranteed by the caller.
    let buf = unsafe { std::slice::from_raw_parts_mut(buf, cap) };
    view::read(id, buf)
}

/// [`mp_stream_read`] that stops at the next mark: moves up to `cap` bytes
/// of output into `buf`, the ones before the mark, and once they are read
/// takes the mark into `mark` and sets `*marked`. The queue is drained when
/// it returns less than `cap` and `*marked` is false.
///
/// # Safety
///
/// `buf` must be valid for `cap` bytes of writes (or `cap` 0); `mark` and
/// `marked` must be valid for a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_stream_read_marked(
    id: StreamId,
    buf: *mut u8,
    cap: usize,
    mark: *mut MpStreamMark,
    marked: *mut bool,
) -> usize {
    if mark.is_null() || marked.is_null() {
        return 0;
    }
    let buf: &mut [u8] = if buf.is_null() || cap == 0 {
        &mut []
    } else {
        // SAFETY: guaranteed by the caller.
        unsafe { std::slice::from_raw_parts_mut(buf, cap) }
    };
    let (n, taken) = view::read_marked(id, buf);
    // SAFETY: guaranteed by the caller.
    unsafe {
        marked.write(taken.is_some());
        if let Some(taken) = taken {
            mark.write(taken.into());
        }
    }
    n
}

/// Appends `len` bytes at `ptr` to the input, synchronously, after
/// everything written before. Returns `false` if the stream is gone or its
/// core's end closed.
///
/// # Safety
///
/// `ptr` must be valid for `len` bytes of reads (or `len` 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_stream_write(id: StreamId, ptr: *const u8, len: usize) -> bool {
    let bytes = if ptr.is_null() || len == 0 {
        &[][..]
    } else {
        // SAFETY: guaranteed by the caller.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    };
    view::write(id, bytes)
}

/// Queues a paste of the `len` bytes of UTF-8 text at `ptr`, after
/// everything written before and ahead of anything written after: the core
/// sends it as one paste, bracketed by the pane's real mode (B361). Invalid
/// UTF-8 is replaced. Returns `false` if the stream is gone or its core's
/// end closed.
///
/// # Safety
///
/// `ptr` must be valid for `len` bytes of reads (or `len` 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_stream_paste(id: StreamId, ptr: *const u8, len: usize) -> bool {
    let bytes = if ptr.is_null() || len == 0 {
        &[][..]
    } else {
        // SAFETY: guaranteed by the caller.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    };
    view::paste(id, &String::from_utf8_lossy(bytes))
}

/// Queues the shortcut bar's `len` bytes after everything written before
/// and ahead of anything written after: keystrokes while the session is
/// live, never kept for later (B476). Returns `false` if the stream is gone
/// or its core's end closed.
///
/// # Safety
///
/// `ptr` must be valid for `len` bytes of reads (or `len` 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_stream_bar(id: StreamId, ptr: *const u8, len: usize) -> bool {
    let bytes = if ptr.is_null() || len == 0 {
        &[][..]
    } else {
        // SAFETY: guaranteed by the caller.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    };
    view::bar(id, bytes)
}

/// Tells the core the view's size in cells. Returns `false` if the stream
/// is gone or its core's end closed.
#[unsafe(no_mangle)]
pub extern "C" fn mp_stream_resize(id: StreamId, cols: u16, rows: u16) -> bool {
    view::resize(id, cols, rows)
}

/// Fills `out` with stream `id`'s counters. Returns `false` (and leaves
/// `out` alone) if the stream is gone.
///
/// # Safety
///
/// `out` must be valid for a write of an `MpStreamStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_stream_stats(id: StreamId, out: *mut MpStreamStats) -> bool {
    let Some(stats) = view::stats(id) else {
        return false;
    };
    if out.is_null() {
        return false;
    }
    let stats = MpStreamStats {
        pushes: stats.pushes,
        pushed: stats.pushed,
        read: stats.read,
        queued: stats.queued,
        peak_queued: stats.peak_queued,
        wakes: stats.wakes,
        drains: stats.drains,
        marks: stats.marks,
    };
    // SAFETY: guaranteed by the caller.
    unsafe { out.write(stats) };
    true
}
