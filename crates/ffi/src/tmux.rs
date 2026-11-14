//! The tmux control-mode parser's C ABI (`mp_tmux_parser_*` in
//! `include/moshpit_ffi.h`): [`moshpit_tmux::ControlParser`] behind an
//! opaque pointer, its events handed to a C callback one at a time with
//! their bytes borrowed.

use std::any::Any;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use moshpit_tmux::{ControlError, ControlParser, Event};

/// Bytes owned by the library, borrowed by C for as long as the call that
/// handed them out says.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MpBytes {
    pub ptr: *const u8,
    pub len: usize,
}

impl MpBytes {
    const EMPTY: MpBytes = MpBytes {
        ptr: ptr::null(),
        len: 0,
    };

    fn of(bytes: &[u8]) -> MpBytes {
        MpBytes {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        }
    }

    fn text(text: &str) -> MpBytes {
        MpBytes::of(text.as_bytes())
    }
}

/// `MpTmuxEventKind` in the header. Plain constants rather than a Rust
/// enum: the values cross the ABI as a `u32` either way.
pub mod kind {
    pub const PANE_OUTPUT: u32 = 1;
    pub const LAYOUT_CHANGE: u32 = 2;
    pub const WINDOW_ADD: u32 = 3;
    pub const WINDOW_CLOSE: u32 = 4;
    pub const WINDOW_RENAMED: u32 = 5;
    pub const SESSION_CHANGED: u32 = 6;
    pub const SESSION_RENAMED: u32 = 7;
    pub const SESSION_WINDOW_CHANGED: u32 = 8;
    pub const SESSIONS_CHANGED: u32 = 9;
    pub const UNLINKED_WINDOW_ADD: u32 = 10;
    pub const UNLINKED_WINDOW_CLOSE: u32 = 11;
    pub const UNLINKED_WINDOW_RENAMED: u32 = 12;
    pub const CLIENT_SESSION_CHANGED: u32 = 13;
    pub const ACTIVE_PANE_CHANGED: u32 = 14;
    pub const PAUSE: u32 = 15;
    pub const CONTINUE: u32 = 16;
    pub const CLIENT_DETACHED: u32 = 17;
    pub const EXIT: u32 = 18;
    pub const COMMAND_RESPONSE: u32 = 19;
    pub const PROTOCOL_ERROR: u32 = 20;
}

/// `MpTmuxFeedResult` in the header.
pub mod feed_result {
    pub const OK: i32 = 0;
    pub const INVALID_ARGUMENT: i32 = 1;
    pub const PANICKED: i32 = 2;
}

/// One event as C sees it (`MpTmuxEvent`). Which fields a kind uses is
/// documented with `MpTmuxEventKind` in the header.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MpTmuxEvent {
    pub kind: u32,
    pub first: MpBytes,
    pub second: MpBytes,
    pub third: MpBytes,
    pub flag: bool,
    /// `CommandResponse`: the block answers a line this client wrote (its
    /// `%begin` flags have bit 1 set, or say nothing readable). False for a
    /// block tmux writes on its own: an `after-<command>` hook's, the late
    /// half of an `-b` command (B273, B275).
    pub from_client: bool,
    pub command_id: i64,
    pub command_num: i64,
    pub lines: *const MpBytes,
    pub line_count: usize,
}

impl MpTmuxEvent {
    fn new(kind: u32) -> MpTmuxEvent {
        MpTmuxEvent {
            kind,
            first: MpBytes::EMPTY,
            second: MpBytes::EMPTY,
            third: MpBytes::EMPTY,
            flag: false,
            from_client: false,
            command_id: 0,
            command_num: 0,
            lines: ptr::null(),
            line_count: 0,
        }
    }

    fn with(kind: u32, first: &str, second: &str) -> MpTmuxEvent {
        MpTmuxEvent {
            first: MpBytes::text(first),
            second: MpBytes::text(second),
            ..MpTmuxEvent::new(kind)
        }
    }
}

/// `MpTmuxEventSink`: called once per event during a feed.
pub type MpTmuxEventSink = unsafe extern "C" fn(context: *mut c_void, event: *const MpTmuxEvent);

/// `MpTmuxParser`: a parser, and why it last panicked.
pub struct MpTmuxParser {
    parser: ControlParser,
    panic_message: String,
}

/// Lays `event` out for C and hands it to `deliver`, which must not keep
/// any pointer past its return.
fn with_c_event(event: &Event, deliver: impl FnOnce(&MpTmuxEvent)) {
    use kind::*;
    let c = match event {
        Event::PaneOutput { pane_id, data } => MpTmuxEvent {
            second: MpBytes::of(data),
            ..MpTmuxEvent::with(PANE_OUTPUT, pane_id, "")
        },
        Event::LayoutChange {
            window_id,
            layout,
            zoomed,
        } => MpTmuxEvent {
            flag: *zoomed,
            ..MpTmuxEvent::with(LAYOUT_CHANGE, window_id, layout)
        },
        Event::WindowAdd { window_id } => MpTmuxEvent::with(WINDOW_ADD, window_id, ""),
        Event::WindowClose { window_id } => MpTmuxEvent::with(WINDOW_CLOSE, window_id, ""),
        Event::WindowRenamed { window_id, name } => {
            MpTmuxEvent::with(WINDOW_RENAMED, window_id, name)
        }
        Event::SessionChanged { session_id, name } => {
            MpTmuxEvent::with(SESSION_CHANGED, session_id, name)
        }
        Event::SessionRenamed { session_id, name } => {
            MpTmuxEvent::with(SESSION_RENAMED, session_id, name)
        }
        Event::SessionWindowChanged {
            session_id,
            window_id,
        } => MpTmuxEvent::with(SESSION_WINDOW_CHANGED, session_id, window_id),
        Event::SessionsChanged => MpTmuxEvent::new(SESSIONS_CHANGED),
        Event::UnlinkedWindowAdd { window_id } => {
            MpTmuxEvent::with(UNLINKED_WINDOW_ADD, window_id, "")
        }
        Event::UnlinkedWindowClose { window_id } => {
            MpTmuxEvent::with(UNLINKED_WINDOW_CLOSE, window_id, "")
        }
        Event::UnlinkedWindowRenamed { window_id, name } => {
            MpTmuxEvent::with(UNLINKED_WINDOW_RENAMED, window_id, name)
        }
        Event::ClientSessionChanged {
            client,
            session_id,
            name,
        } => MpTmuxEvent {
            third: MpBytes::text(name),
            ..MpTmuxEvent::with(CLIENT_SESSION_CHANGED, client, session_id)
        },
        Event::ActivePaneChanged { window_id, pane_id } => {
            MpTmuxEvent::with(ACTIVE_PANE_CHANGED, window_id, pane_id)
        }
        Event::Pause { pane_id } => MpTmuxEvent::with(PAUSE, pane_id, ""),
        Event::Continue { pane_id } => MpTmuxEvent::with(CONTINUE, pane_id, ""),
        Event::ClientDetached { payload } => MpTmuxEvent::with(CLIENT_DETACHED, payload, ""),
        Event::Exit { reason } => MpTmuxEvent {
            flag: reason.is_some(),
            ..MpTmuxEvent::with(EXIT, reason.as_deref().unwrap_or(""), "")
        },
        Event::CommandResponse(response) => {
            // The one event with a list: its lines, laid out for C for the
            // length of the callback.
            let lines: Vec<MpBytes> = response.lines.iter().map(|l| MpBytes::text(l)).collect();
            let c = MpTmuxEvent {
                flag: response.is_error,
                from_client: response.from_client,
                command_id: response.command_id,
                command_num: response.command_num,
                lines: lines.as_ptr(),
                line_count: lines.len(),
                ..MpTmuxEvent::new(COMMAND_RESPONSE)
            };
            deliver(&c);
            return;
        }
        Event::ProtocolError(ControlError::MalformedLine(text)) => {
            MpTmuxEvent::with(PROTOCOL_ERROR, text, "")
        }
    };
    deliver(&c);
}

fn panic_text(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "panic with a non-string payload".to_owned()
    }
}

/// `MP_FFI_ABI_VERSION` as this library was built.
pub const ABI_VERSION: u32 = 2;

/// The ABI version the library was built with (`MP_FFI_ABI_VERSION`).
#[unsafe(no_mangle)]
pub extern "C" fn mp_ffi_abi_version() -> u32 {
    ABI_VERSION
}

/// A new parser; free it with [`mp_tmux_parser_free`].
#[unsafe(no_mangle)]
pub extern "C" fn mp_tmux_parser_new() -> *mut MpTmuxParser {
    Box::into_raw(Box::new(MpTmuxParser {
        parser: ControlParser::new(),
        panic_message: String::new(),
    }))
}

/// Frees a parser made by [`mp_tmux_parser_new`]. NULL is fine.
///
/// # Safety
///
/// `parser` is NULL or a parser from [`mp_tmux_parser_new`] not freed yet,
/// and not in use on another thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_tmux_parser_free(parser: *mut MpTmuxParser) {
    if !parser.is_null() {
        // SAFETY: the caller's contract.
        drop(unsafe { Box::from_raw(parser) });
    }
}

/// Drops the partial line and the open block; the next block is a boot
/// block again.
///
/// # Safety
///
/// `parser` is NULL (ignored) or a live parser not in use on another
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_tmux_parser_reset(parser: *mut MpTmuxParser) {
    // SAFETY: the caller's contract.
    if let Some(parser) = unsafe { parser.as_mut() } {
        parser.parser.reset();
    }
}

/// Parses `len` bytes at `data`, handing every event to `sink(context, …)`
/// in stream order before it returns. A panic in the parser is caught
/// here: the events before it were delivered, the parser starts over and
/// the result says so.
///
/// # Safety
///
/// `parser` is a live parser not in use on another thread; `data` points
/// at `len` readable bytes (or `len` is 0); `sink` is safe to call with
/// `context` during this call and does not call back into `parser`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_tmux_parser_feed(
    parser: *mut MpTmuxParser,
    data: *const u8,
    len: usize,
    sink: Option<MpTmuxEventSink>,
    context: *mut c_void,
) -> i32 {
    // SAFETY: the caller's contract.
    let (Some(parser), Some(sink)) = (unsafe { parser.as_mut() }, sink) else {
        return feed_result::INVALID_ARGUMENT;
    };
    let data: &[u8] = match (data.is_null(), len) {
        (_, 0) => &[],
        (true, _) => return feed_result::INVALID_ARGUMENT,
        // SAFETY: the caller's contract.
        (false, len) => unsafe { std::slice::from_raw_parts(data, len) },
    };
    let state = &mut parser.parser;
    let parsed = catch_unwind(AssertUnwindSafe(|| {
        #[cfg(test)]
        tests::maybe_panic(data);
        state.feed(data, |event| {
            // SAFETY: the caller's contract; `c` outlives the call.
            with_c_event(&event, |c| unsafe { sink(context, c) });
        });
    }));
    match parsed {
        Ok(()) => feed_result::OK,
        Err(payload) => {
            // Whatever the panic left half done is not to be trusted: start
            // over, the way a reset (or a reconnect) does.
            parser.parser = ControlParser::new();
            parser.panic_message = panic_text(payload.as_ref());
            feed_result::PANICKED
        }
    }
}

/// The message of the parser's last panic, or empty bytes. Valid until the
/// next call on `parser`.
///
/// # Safety
///
/// `parser` is a live parser not in use on another thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_tmux_parser_panic_message(parser: *const MpTmuxParser) -> MpBytes {
    // SAFETY: the caller's contract.
    match unsafe { parser.as_ref() } {
        Some(parser) => MpBytes::text(&parser.panic_message),
        None => MpBytes::EMPTY,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::mem::{align_of, offset_of, size_of};

    use super::*;

    thread_local! {
        /// A chunk that makes the next feed panic, for the panic test.
        static PANIC_ON: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    }

    pub(super) fn maybe_panic(data: &[u8]) {
        let hit = PANIC_ON.with(|on| on.borrow().as_deref() == Some(data));
        if hit {
            panic!("injected for the test");
        }
    }

    unsafe extern "C" fn count(context: *mut c_void, event: *const MpTmuxEvent) {
        // SAFETY: the test passes a `Vec<u32>` and a live event.
        let (seen, event) = unsafe { (&mut *context.cast::<Vec<u32>>(), &*event) };
        seen.push(event.kind);
    }

    #[test]
    fn the_event_has_the_layout_the_header_declares() {
        // kind, 4 bytes of padding, three {ptr, len}, flag, from_client and
        // 6 bytes of padding, two int64, {ptr, count}: the C compiler's layout of
        // MpTmuxEvent on every 64-bit Apple and Android target.
        assert_eq!(size_of::<MpBytes>(), 16);
        assert_eq!(size_of::<MpTmuxEvent>(), 96);
        assert_eq!(align_of::<MpTmuxEvent>(), 8);
        assert_eq!(offset_of!(MpTmuxEvent, first), 8);
        assert_eq!(offset_of!(MpTmuxEvent, second), 24);
        assert_eq!(offset_of!(MpTmuxEvent, third), 40);
        assert_eq!(offset_of!(MpTmuxEvent, flag), 56);
        assert_eq!(offset_of!(MpTmuxEvent, from_client), 57);
        assert_eq!(offset_of!(MpTmuxEvent, command_id), 64);
        assert_eq!(offset_of!(MpTmuxEvent, command_num), 72);
        assert_eq!(offset_of!(MpTmuxEvent, lines), 80);
        assert_eq!(offset_of!(MpTmuxEvent, line_count), 88);
    }

    #[test]
    fn a_panic_is_caught_reported_and_the_parser_starts_over() {
        let parser = mp_tmux_parser_new();
        let mut seen: Vec<u32> = Vec::new();
        let context = (&raw mut seen).cast::<c_void>();
        let feed = |bytes: &[u8]| unsafe {
            mp_tmux_parser_feed(parser, bytes.as_ptr(), bytes.len(), Some(count), context)
        };
        assert_eq!(
            feed(b"%begin 1 1 0\n%end 1 1 0\n%window-add @1\n%output %0 par"),
            0
        );
        PANIC_ON.with(|on| *on.borrow_mut() = Some(b"tial\n".to_vec()));
        assert_eq!(feed(b"tial\n"), feed_result::PANICKED);
        PANIC_ON.with(|on| *on.borrow_mut() = None);
        let message = unsafe { mp_tmux_parser_panic_message(parser) };
        let message = unsafe { std::slice::from_raw_parts(message.ptr, message.len) };
        assert_eq!(message, b"injected for the test");
        // Started over: the partial "%output %0 par" is gone and the next
        // block is a boot block again, swallowed.
        assert_eq!(feed(b"%begin 2 2 0\n%end 2 2 0\n%window-add @2\n"), 0);
        assert_eq!(seen, [kind::WINDOW_ADD, kind::WINDOW_ADD]);
        unsafe { mp_tmux_parser_free(parser) };
    }

    #[test]
    fn bad_arguments_parse_nothing() {
        let parser = mp_tmux_parser_new();
        let mut seen: Vec<u32> = Vec::new();
        let context = (&raw mut seen).cast::<c_void>();
        unsafe {
            let line = b"%window-add @1\n";
            assert_eq!(
                mp_tmux_parser_feed(
                    ptr::null_mut(),
                    line.as_ptr(),
                    line.len(),
                    Some(count),
                    context
                ),
                feed_result::INVALID_ARGUMENT
            );
            assert_eq!(
                mp_tmux_parser_feed(parser, line.as_ptr(), line.len(), None, context),
                feed_result::INVALID_ARGUMENT
            );
            assert_eq!(
                mp_tmux_parser_feed(parser, ptr::null(), 3, Some(count), context),
                feed_result::INVALID_ARGUMENT
            );
            // NULL with nothing to read is just an empty feed.
            assert_eq!(
                mp_tmux_parser_feed(parser, ptr::null(), 0, Some(count), context),
                0
            );
            assert_eq!(mp_tmux_parser_panic_message(parser).len, 0);
            assert_eq!(mp_tmux_parser_panic_message(ptr::null()).len, 0);
            mp_tmux_parser_reset(ptr::null_mut());
            mp_tmux_parser_free(parser);
            mp_tmux_parser_free(ptr::null_mut());
        }
        assert!(seen.is_empty());
    }
}
