// The byte channel's view end (crates/stream/src/ffi.rs): one ordered,
// lossless byte queue per pane in each direction between the Rust core and
// the native terminal view. Every function is thread-safe; an unknown or
// finished stream id does nothing (reads return 0, writes return false).
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Tells the view output is waiting. Called on any thread, at most once
/// until the view has drained the queue (read until a read returns less
/// than asked). Hop to the view's thread and return; do not attach, detach
/// or push from here (reading is fine).
typedef void (*MpStreamWaker)(void *_Nullable ctx);

/// A stream's output counters.
typedef struct {
    uint64_t pushes;      ///< non-empty pushes by the core
    uint64_t pushed;      ///< bytes pushed
    uint64_t read;        ///< bytes read by the view
    uint64_t queued;      ///< bytes waiting now
    uint64_t peak_queued; ///< the most bytes ever waiting
    uint64_t wakes;       ///< wakes sent to the view
    uint64_t drains;      ///< times the view drained the queue after a wake
    uint64_t marks;       ///< marks put into the output
} MpStreamStats;

/// What the view does at a point in the output (a code the core and the
/// view agree on, and its argument).
typedef struct {
    uint32_t code;
    int64_t arg;
} MpStreamMark;

/// Makes `waker(ctx)` the view's waker for stream `id` (NULL: detach). It is
/// called right away if output is already waiting. Replaces an earlier
/// waker. False if the stream is gone.
bool mp_stream_attach(uint64_t id, MpStreamWaker _Nullable waker, void *_Nullable ctx);

/// Drops the view's waker. Once this returns it is not called again, so
/// `ctx` can be freed.
void mp_stream_detach(uint64_t id);

/// Moves up to `cap` bytes of output into `buf`, oldest first; returns how
/// many. Less than `cap` means the queue is drained and the next push wakes
/// the view again.
size_t mp_stream_read(uint64_t id, uint8_t *_Nullable buf, size_t cap);

/// mp_stream_read that stops at the next mark: the bytes before it, then,
/// once they are read, the mark itself in `*mark` with `*marked` set. The
/// queue is drained when it returns less than `cap` and `*marked` is false.
size_t mp_stream_read_marked(uint64_t id, uint8_t *_Nullable buf, size_t cap,
                             MpStreamMark *_Nonnull mark, bool *_Nonnull marked);

/// Appends `len` bytes to the input, synchronously, after everything written
/// before (keystrokes, the terminal's replies, a paste the view brackets
/// itself). False if the stream is gone or its core's end closed.
bool mp_stream_write(uint64_t id, const uint8_t *_Nullable ptr, size_t len);

/// Queues a paste of `len` bytes of UTF-8 text, synchronously, after
/// everything written before: the core sends it as one paste, bracketed by
/// the pane's real mode, in its place among the keystrokes. Invalid UTF-8 is
/// replaced. False if the stream is gone or its core's end closed.
bool mp_stream_paste(uint64_t id, const uint8_t *_Nullable ptr, size_t len);

/// Queues the shortcut bar's `len` bytes, synchronously, after everything
/// written before: keystrokes while the session is live, never kept for a
/// reconnect to deliver later (B476). False if the stream is gone or its
/// core's end closed.
bool mp_stream_bar(uint64_t id, const uint8_t *_Nullable ptr, size_t len);

/// Tells the core the view's size in cells, after the bytes written so far.
bool mp_stream_resize(uint64_t id, uint16_t cols, uint16_t rows);

/// Fills `out` with the stream's counters. False if the stream is gone.
bool mp_stream_stats(uint64_t id, MpStreamStats *_Nonnull out);

#ifdef __cplusplus
}
#endif
