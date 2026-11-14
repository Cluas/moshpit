// The C ABI of the one Rust static library the iOS app links
// (crates/ffi, libmoshpit_ffi.a). Swift imports this header as the
// MoshpitFFI module (module.modulemap next to it).
//
// So far: the tmux control-mode (-CC) parser of crates/tmux, which
// TmuxControlClient runs instead of its own when the Rust parser switch is
// on. The byte channel in the same library has its own header,
// crates/stream/include/moshpit_stream.h.
//
// A hand-written C ABI, not UniFFI: the parser is fed every chunk of the
// control stream, and this shape costs one call per chunk plus one callback
// per event, with the bytes borrowed rather than serialised and copied
// (the measurements are in crates/ffi/src/lib.rs).
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Bumped whenever anything below changes shape. The app checks
/// `mp_ffi_abi_version() == MP_FFI_ABI_VERSION` before it uses the library,
/// so a stale libmoshpit_ffi.a is refused instead of misread.
#define MP_FFI_ABI_VERSION 2

/// The `MP_FFI_ABI_VERSION` the library was built with.
uint32_t mp_ffi_abi_version(void);

/// Bytes owned by the library, valid until the callback they were passed to
/// returns (or as the function returning them says). `ptr` may be NULL or
/// dangling when `len` is 0.
typedef struct {
    const uint8_t *_Nullable ptr;
    size_t len;
} MpBytes;

// MARK: - tmux control mode

/// One parser: the state of one control-mode stream (its partial line, the
/// open %begin block). Not thread-safe: one thread at a time.
typedef struct MpTmuxParser MpTmuxParser;

/// What an event is, and which `MpTmuxEvent` fields carry it. Ids keep
/// tmux's spelling: `$1` sessions, `@2` windows, `%3` panes. All text is
/// valid UTF-8; `PaneOutput`'s `second` is raw pane bytes (tmux cuts
/// mid-character).
typedef enum __attribute__((enum_extensibility(open))) MpTmuxEventKind : uint32_t {
    /// %output: first = pane id, second = the bytes, octal escapes decoded.
    MpTmuxEventPaneOutput = 1,
    /// %layout-change: first = window id, second = layout, flag = zoomed.
    MpTmuxEventLayoutChange = 2,
    /// %window-add: first = window id.
    MpTmuxEventWindowAdd = 3,
    /// %window-close: first = window id.
    MpTmuxEventWindowClose = 4,
    /// %window-renamed: first = window id, second = name.
    MpTmuxEventWindowRenamed = 5,
    /// %session-changed: first = session id, second = name (may be empty).
    MpTmuxEventSessionChanged = 6,
    /// %session-renamed: first = session id, second = name.
    MpTmuxEventSessionRenamed = 7,
    /// %session-window-changed: first = session id, second = window id.
    MpTmuxEventSessionWindowChanged = 8,
    /// %sessions-changed: no fields.
    MpTmuxEventSessionsChanged = 9,
    /// %unlinked-window-add: first = window id.
    MpTmuxEventUnlinkedWindowAdd = 10,
    /// %unlinked-window-close: first = window id.
    MpTmuxEventUnlinkedWindowClose = 11,
    /// %unlinked-window-renamed: first = window id, second = name.
    MpTmuxEventUnlinkedWindowRenamed = 12,
    /// %client-session-changed: first = client, second = session id,
    /// third = name.
    MpTmuxEventClientSessionChanged = 13,
    /// %window-pane-changed: first = window id, second = pane id.
    MpTmuxEventActivePaneChanged = 14,
    /// %pause: first = pane id.
    MpTmuxEventPause = 15,
    /// %continue: first = pane id.
    MpTmuxEventContinue = 16,
    /// %client-detached: first = whatever followed the directive, or empty.
    MpTmuxEventClientDetached = 17,
    /// %exit: flag = a reason was given, first = the reason.
    MpTmuxEventExit = 18,
    /// A finished %begin block: command_id, command_num, flag = closed by
    /// %error, from_client = it answers a line this client wrote (the
    /// %begin flags have bit 1 set, or no readable flags), lines/line_count
    /// = its lines.
    MpTmuxEventCommandResponse = 19,
    /// A line that could not be decoded, or a recovery note: first = text.
    MpTmuxEventProtocolError = 20,
} MpTmuxEventKind;

/// One event. Fields an event's kind does not use are empty, false or 0.
typedef struct {
    MpTmuxEventKind kind;
    MpBytes first;
    MpBytes second;
    MpBytes third;
    bool flag;
    /// CommandResponse only: false for a block tmux wrote on its own (an
    /// after-<command> hook's, the late half of an -b command).
    bool from_client;
    int64_t command_id;
    int64_t command_num;
    const MpBytes *_Nullable lines;
    size_t line_count;
} MpTmuxEvent;

/// Receives the events of one `mp_tmux_parser_feed`, in stream order, on
/// the feeding thread, before it returns. `event` and everything it points
/// at are valid until the callback returns. It must not call back into the
/// same parser.
typedef void (*MpTmuxEventSink)(void *_Nullable context, const MpTmuxEvent *_Nonnull event);

typedef enum __attribute__((enum_extensibility(open))) MpTmuxFeedResult : int32_t {
    /// Every complete line was parsed; a trailing partial line is kept.
    MpTmuxFeedOk = 0,
    /// A NULL parser or sink, or NULL data with a nonzero length. Nothing
    /// was parsed.
    MpTmuxFeedInvalidArgument = 1,
    /// The parser panicked (a bug). Events before the panic were delivered;
    /// the rest of the chunk was dropped and the parser starts over, as if
    /// reset. `mp_tmux_parser_panic_message` says what happened.
    MpTmuxFeedPanicked = 2,
} MpTmuxFeedResult;

/// A new parser. Free it with `mp_tmux_parser_free`.
MpTmuxParser *_Nonnull mp_tmux_parser_new(void);

/// Frees a parser. NULL is fine.
void mp_tmux_parser_free(MpTmuxParser *_Nullable parser);

/// Drops the partial line and the open block, and expects a boot block
/// again: for a reconnect, or to resync.
void mp_tmux_parser_reset(MpTmuxParser *_Nonnull parser);

/// Parses `len` bytes of the control stream: every complete line is
/// dispatched to `sink` now, a trailing partial line waits for the next
/// call. `data` may be NULL when `len` is 0.
MpTmuxFeedResult mp_tmux_parser_feed(MpTmuxParser *_Nonnull parser,
                                     const uint8_t *_Nullable data, size_t len,
                                     MpTmuxEventSink _Nonnull sink,
                                     void *_Nullable context);

/// The message of the parser's last panic, empty if it never panicked.
/// Valid until the next call on the parser.
MpBytes mp_tmux_parser_panic_message(const MpTmuxParser *_Nonnull parser);

#ifdef __cplusplus
}
#endif
