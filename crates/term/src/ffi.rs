//! The C ABI the native views call (`include/moshpit_term.h`). A terminal
//! is either fed by the byte channel on its pane's thread
//! ([`mp_term_open`]) or fed by the caller ([`mp_term_new`] and
//! [`mp_term_feed`]: tests, previews, a static frame). Every function is
//! thread-safe; a panic inside is caught and answered with the "nothing"
//! value of the function.

use std::{
    ffi::{CStr, CString, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Mutex,
};

use moshpit_stream::{Mark, StreamId};

use crate::{
    CursorView, DrawCell, Event, Frame, ImageRun, Key, Modes, Options, Palette, Rgb, ScrollAction,
    SelectKind, Tap, Terminal, pane,
};

/// Tells the view a frame is ready; called on the pane's thread.
pub type MpTermWaker = unsafe extern "C" fn(ctx: *mut c_void);

/// `MP_TERM_ABI_VERSION` (moshpit_term.h): bumped whenever the C ABI
/// changes shape, so a view refuses a stale library instead of misreading it.
pub const ABI_VERSION: u32 = 15;

/// The `MP_TERM_ABI_VERSION` this library was built with.
#[unsafe(no_mangle)]
pub extern "C" fn mp_term_abi_version() -> u32 {
    ABI_VERSION
}

pub enum MpTerm {
    Pane(pane::Pane),
    Bare(Box<Mutex<Terminal>>),
}

impl MpTerm {
    fn with<R>(&self, f: impl FnOnce(&mut Terminal) -> R) -> R {
        match self {
            MpTerm::Pane(p) => p.with(f),
            MpTerm::Bare(m) => f(&mut m.lock().unwrap_or_else(|e| e.into_inner())),
        }
    }
}

struct CWaker {
    waker: MpTermWaker,
    ctx: *mut c_void,
}
// SAFETY: mp_term_open's contract: the waker may be called with ctx on any
// thread until mp_term_close returns.
unsafe impl Send for CWaker {}
// SAFETY: as above.
unsafe impl Sync for CWaker {}

impl CWaker {
    fn wake(&self) {
        // SAFETY: mp_term_open's contract.
        unsafe { (self.waker)(self.ctx) }
    }
}

fn guard<R>(nothing: R, f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(nothing)
}

fn options(scrollback: u32) -> Options {
    Options {
        scrollback: scrollback as usize,
        ..Options::default()
    }
}

/// A terminal fed from byte-channel stream `stream` on a thread of its own;
/// `waker(ctx)` is called on that thread whenever a frame is ready. NULL if
/// the stream is gone.
///
/// # Safety
/// `waker` must be safe to call with `ctx` from any thread until
/// [`mp_term_close`] returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_open(
    stream: StreamId,
    cols: u16,
    rows: u16,
    scrollback: u32,
    waker: MpTermWaker,
    ctx: *mut c_void,
) -> *mut MpTerm {
    guard(std::ptr::null_mut(), || {
        let waker = CWaker { waker, ctx };
        let callbacks = pane::Callbacks {
            frame_ready: Box::new(move || waker.wake()),
            // The session's marks go to the view with the frame that has
            // the bytes before them (`mp_frame_mark`).
            mark: Box::new(|term, mark| term.note_mark(mark)),
        };
        let term = Terminal::new(cols, rows, options(scrollback));
        match pane::Pane::attach(stream, term, callbacks) {
            Some(p) => Box::into_raw(Box::new(MpTerm::Pane(p))),
            None => std::ptr::null_mut(),
        }
    })
}

/// A terminal the view feeds through a byte channel of its own
/// ([`mp_term_push`]), parsed on a thread of its own in the order pushed;
/// `waker(ctx)` is called on that thread whenever a frame (or a reply,
/// [`mp_term_take_replies`]) is ready.
///
/// # Safety
/// As [`mp_term_open`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_open_local(
    cols: u16,
    rows: u16,
    scrollback: u32,
    waker: MpTermWaker,
    ctx: *mut c_void,
) -> *mut MpTerm {
    guard(std::ptr::null_mut(), || {
        let waker = CWaker { waker, ctx };
        let callbacks = pane::Callbacks {
            frame_ready: Box::new(move || waker.wake()),
            // The view's own marks (`mp_term_push_mark`), back with the
            // frame that has the bytes pushed before them.
            mark: Box::new(|term, mark| term.note_mark(mark)),
        };
        let term = Terminal::new(cols, rows, options(scrollback));
        match pane::Pane::local(term, callbacks) {
            Some(p) => Box::into_raw(Box::new(MpTerm::Pane(p))),
            None => std::ptr::null_mut(),
        }
    })
}

/// A terminal the caller feeds ([`mp_term_feed`]).
#[unsafe(no_mangle)]
pub extern "C" fn mp_term_new(cols: u16, rows: u16, scrollback: u32) -> *mut MpTerm {
    guard(std::ptr::null_mut(), || {
        Box::into_raw(Box::new(MpTerm::Bare(Box::new(Mutex::new(Terminal::new(
            cols,
            rows,
            options(scrollback),
        ))))))
    })
}

/// Frees a terminal; for an opened one, its thread has stopped and the
/// waker is not called again once this returns.
///
/// # Safety
/// `t` is NULL or from mp_term_open/mp_term_new, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_close(t: *mut MpTerm) {
    if !t.is_null() {
        // SAFETY: the contract above.
        guard((), || drop(unsafe { Box::from_raw(t) }));
    }
}

macro_rules! term {
    ($t:expr) => {{
        if $t.is_null() {
            return;
        }
        // SAFETY: callers pass a live terminal (the header's contract).
        unsafe { &*$t }
    }};
    ($t:expr, $nothing:expr) => {{
        if $t.is_null() {
            return $nothing;
        }
        // SAFETY: callers pass a live terminal (the header's contract).
        unsafe { &*$t }
    }};
}

/// Feeds bytes (a terminal from mp_term_new; for an opened one the channel
/// does).
///
/// # Safety
/// `t` live; `bytes` points at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_feed(t: *const MpTerm, bytes: *const u8, len: usize) {
    let t = term!(t);
    if bytes.is_null() {
        return;
    }
    // SAFETY: the contract above.
    let b = unsafe { std::slice::from_raw_parts(bytes, len) };
    guard((), || t.with(|term| term.feed(b)));
}

/// The remote's bytes into a terminal from mp_term_open_local, after
/// everything pushed before; parsed on its thread. False for any other.
///
/// # Safety
/// `t` live; `bytes` points at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_push(t: *const MpTerm, bytes: *const u8, len: usize) -> bool {
    let t = term!(t, false);
    if bytes.is_null() && len > 0 {
        return false;
    }
    let b = if len == 0 {
        &[][..]
    } else {
        // SAFETY: the contract above.
        unsafe { std::slice::from_raw_parts(bytes, len) }
    };
    guard(false, || match t {
        MpTerm::Pane(p) => p.push(b),
        MpTerm::Bare(_) => false,
    })
}

/// A mark among the bytes of a terminal from mp_term_open_local: after
/// everything pushed before it, it comes back with the first frame that
/// has those bytes ([`mp_frame_mark`]), and no earlier frame has a byte
/// pushed after it. False for any other terminal, and for a code with the
/// top bit set (the terminal's own).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_push_mark(t: *const MpTerm, code: u32, arg: i64) -> bool {
    let t = term!(t, false);
    guard(false, || match t {
        MpTerm::Pane(p) => p.push_mark(Mark { code, arg }),
        MpTerm::Bare(_) => false,
    })
}

/// Forgets a half-read UTF-8 or escape sequence (a reconnect): for a local
/// terminal after the bytes pushed so far, otherwise at once.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_discard_pending_input(t: *const MpTerm) {
    let t = term!(t);
    guard((), || match t {
        MpTerm::Pane(p) => p.discard_pending_input(),
        MpTerm::Bare(_) => t.with(|term| term.discard_pending_input()),
    });
}

/// Bytes the library made, owned by the caller: free with
/// [`mp_bytes_free`]. `ptr` is NULL when `len` is 0.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MpOwnedBytes {
    pub ptr: *mut u8,
    pub len: usize,
}

impl MpOwnedBytes {
    const NONE: MpOwnedBytes = MpOwnedBytes {
        ptr: std::ptr::null_mut(),
        len: 0,
    };

    fn from(v: Vec<u8>) -> MpOwnedBytes {
        if v.is_empty() {
            return Self::NONE;
        }
        let b = v.into_boxed_slice();
        let len = b.len();
        MpOwnedBytes {
            ptr: Box::into_raw(b) as *mut u8,
            len,
        }
    }
}

/// # Safety
/// `b` from this library, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_bytes_free(b: MpOwnedBytes) {
    if !b.ptr.is_null() {
        // SAFETY: made by `MpOwnedBytes::from` from a boxed slice of `len`.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(b.ptr, b.len)) });
    }
}

/// A local terminal's replies to the remote since the last call, in order
/// (send them as the view sends keys); empty for any other (its replies
/// go into its session's stream).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_take_replies(t: *const MpTerm) -> MpOwnedBytes {
    let t = term!(t, MpOwnedBytes::NONE);
    guard(MpOwnedBytes::NONE, || match t {
        MpTerm::Pane(p) => MpOwnedBytes::from(p.take_replies()),
        MpTerm::Bare(_) => MpOwnedBytes::from(t.with(|term| term.take_replies())),
    })
}

/// The bytes a key sends (`MP_KEY_*`, the scalar for `MP_KEY_CHAR`,
/// `MP_MOD_*` bits), as the program on the remote asked keys to be encoded:
/// the kitty keyboard protocol (presses, `repeat` > 1, releases with
/// `down` false), else xterm's (DECCKM, CSI u). Empty: nothing to send.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_key(
    t: *const MpTerm,
    key: u32,
    scalar: u32,
    mods: u8,
    down: bool,
    repeat: u16,
) -> MpOwnedBytes {
    let t = term!(t, MpOwnedBytes::NONE);
    let Some(key) = Key::from_code(key, scalar) else {
        return MpOwnedBytes::NONE;
    };
    guard(MpOwnedBytes::NONE, || {
        MpOwnedBytes::from(t.with(|term| term.key(key, mods, down, repeat)))
    })
}

/// The bytes a mouse report sends (`kind` 0 press, 1 release, 2 motion;
/// `button` 0 left, 1 middle, 2 right, 3 none, 4–7 wheel up, down, left,
/// right; a viewport cell), in the program's encoding; empty while it asked
/// for no reports.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_mouse(
    t: *const MpTerm,
    kind: u8,
    button: u8,
    row: u16,
    col: u16,
    mods: u8,
) -> MpOwnedBytes {
    let t = term!(t, MpOwnedBytes::NONE);
    guard(MpOwnedBytes::NONE, || {
        MpOwnedBytes::from(t.with(|term| term.mouse(kind, button, row, col, mods)))
    })
}

/// The view gained or lost the keyboard: the focus report the program asked
/// for, or nothing.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_focus(t: *const MpTerm, focused: bool) -> MpOwnedBytes {
    let t = term!(t, MpOwnedBytes::NONE);
    guard(MpOwnedBytes::NONE, || {
        MpOwnedBytes::from(t.with(|term| term.focus(focused)))
    })
}

/// Event kinds of [`mp_term_take_events`].
pub mod event_kinds {
    pub const TITLE: u8 = 1;
    pub const BELL: u8 = 2;
    pub const CLIPBOARD_STORE: u8 = 3;
    pub const CLIPBOARD_LOAD: u8 = 4;
    pub const DIRECTORY: u8 = 5;
}

/// One event as `each(ctx, kind, text)` hears it: `text` (NUL-terminated,
/// valid during the call) is the title, the clipboard's text or OSC 7's
/// URL, else empty.
pub type MpTermEventFn = unsafe extern "C" fn(ctx: *mut c_void, kind: u8, text: *const c_char);

/// Hands every event since the last call to `each`, in order: a title
/// (OSC 0/2, screen's `ESC k`), a bell, OSC 52's store and query (answer a
/// query with [`mp_term_answer_clipboard`], or not at all). An opened
/// terminal's never wait for its lock (its thread moves them out as it
/// parses).
///
/// # Safety
/// `t` live; `each` callable with `ctx` during this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_take_events(
    t: *const MpTerm,
    each: MpTermEventFn,
    ctx: *mut c_void,
) {
    let t = term!(t);
    let events = guard(Vec::new(), || match t {
        MpTerm::Pane(p) => p.take_events(),
        MpTerm::Bare(_) => t.with(|term| term.take_events()),
    });
    for e in events {
        let (kind, text) = match e {
            Event::Title(s) => (event_kinds::TITLE, s),
            Event::Bell => (event_kinds::BELL, String::new()),
            Event::ClipboardStore(s) => (event_kinds::CLIPBOARD_STORE, s),
            Event::ClipboardLoad => (event_kinds::CLIPBOARD_LOAD, String::new()),
            Event::Directory(s) => (event_kinds::DIRECTORY, s),
        };
        let c = CString::new(text.replace('\0', "")).unwrap_or_default();
        // SAFETY: the contract above.
        unsafe { each(ctx, kind, c.as_ptr()) };
    }
}

/// Lets every decoded image go, for every terminal (the process keeps one
/// cache of them): a memory warning, the app going to the background. The
/// images stay on screen: the next frame that draws one decodes it again.
#[unsafe(no_mangle)]
pub extern "C" fn mp_term_images_purge() {
    guard((), || crate::ImageCache::shared().purge())
}

/// Bytes of decoded images the process keeps (debug probes).
#[unsafe(no_mangle)]
pub extern "C" fn mp_term_images_bytes() -> usize {
    guard(0, || crate::ImageCache::shared().bytes())
}

/// A memory warning, for `t`: what mp_term_release_image_caches lets go,
/// and the kitty images no cell shows any more (a program placing one
/// again gets ENOENT). Bytes freed.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_drop_unplaced_images(t: *const MpTerm) -> usize {
    let t = term!(t, 0);
    guard(0, || t.with(|term| term.drop_unplaced_images()))
}

/// The app goes to the background, for `t`: the images its emulator keeps
/// only to share identical ones are let go. Bytes freed.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_release_image_caches(t: *const MpTerm) -> usize {
    let t = term!(t, 0);
    guard(0, || t.with(|term| term.release_image_caches()))
}

/// `t` is a tmux pane's: a clear of its main screen keeps what tmux's
/// `scroll-on-clear` keeps (`ESC [2J`, and `ESC [J` from the top left, the
/// `clear` a tmux pane sends), so scrolling back shows what tmux's own
/// history has. Off: a terminal of its own (`ESC [2J` only). After the
/// bytes parsed so far.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_set_tmux_clears(t: *const MpTerm, on: bool) {
    let t = term!(t);
    let mode = if on {
        crate::ScrollOnClear::Tmux
    } else {
        crate::ScrollOnClear::EraseDisplay
    };
    guard((), || t.with(|term| term.set_scroll_on_clear(mode)))
}

/// Bytes `t`'s stored kitty images take (debug probes).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_kitty_image_bytes(t: *const MpTerm) -> usize {
    let t = term!(t, 0);
    guard(0, || t.with(|term| term.kitty_image_bytes()))
}

/// Paths as links (crates/term/src/paths.rs): `extensions`, comma
/// separated, are what the platform can preview; "" (or NULL) turns paths
/// off and forgets every confirmed one.
///
/// # Safety
/// `t` live; `extensions` NULL or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_set_path_extensions(t: *const MpTerm, extensions: *const c_char) {
    let t = term!(t);
    let list = c_str(extensions).unwrap_or_default();
    let exts: Vec<&str> = list.split(',').collect();
    guard((), || t.with(|term| term.set_path_rules(&exts)));
}

/// The path candidates on screen, each once, for the host to confirm: one
/// per line, `F` or `D` (only as a folder), a tab, the token. NULL when
/// there is none; free with [`mp_string_free`].
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_path_candidates(t: *const MpTerm) -> *mut c_char {
    let t = term!(t, std::ptr::null_mut());
    guard(std::ptr::null_mut(), || {
        let found = t.with(|term| term.path_candidates());
        if found.is_empty() {
            return std::ptr::null_mut();
        }
        let mut out = String::new();
        for (token, folder) in found {
            out.push(if folder { 'D' } else { 'F' });
            out.push('\t');
            out.push_str(&token);
            out.push('\n');
        }
        into_c(out)
    })
}

/// The candidates the host confirmed, replacing those before: one per line,
/// `F` or `D`, a tab, the token, a tab, the URI a tap opens. True when that
/// changes what draws as a link (take a frame: it redraws the screen).
///
/// # Safety
/// `t` live; `table` NULL (none) or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_set_path_links(t: *const MpTerm, table: *const c_char) -> bool {
    let t = term!(t, false);
    let table = c_str(table).unwrap_or_default();
    let links: Vec<(bool, String, String)> = table
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let folder = match parts.next()? {
                "D" => true,
                "F" => false,
                _ => return None,
            };
            let token = parts.next().filter(|s| !s.is_empty())?;
            let uri = parts.next().filter(|s| !s.is_empty())?;
            Some((folder, token.to_string(), uri.to_string()))
        })
        .collect();
    guard(false, || t.with(|term| term.set_path_links(links)))
}

/// Answers the remote's OSC 52 query with `text` (UTF-8); the reply comes
/// with the terminal's other replies.
///
/// # Safety
/// `t` live; `text` NULL (an empty answer) or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_answer_clipboard(t: *const MpTerm, text: *const c_char) {
    let t = term!(t);
    let text = if text.is_null() {
        String::new()
    } else {
        // SAFETY: the contract above.
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    };
    guard((), || t.with(|term| term.answer_clipboard(&text)));
}

fn rgb(v: u32) -> Rgb {
    Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    }
}

/// The theme: its 16 colours (0xRRGGBB), foreground, background, cursor.
/// A program's own colour changes give way to it.
///
/// # Safety
/// `t` live; `ansi` points at 16 values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_set_palette(
    t: *const MpTerm,
    ansi: *const u32,
    foreground: u32,
    background: u32,
    cursor: u32,
) {
    let t = term!(t);
    if ansi.is_null() {
        return;
    }
    // SAFETY: the contract above.
    let a = unsafe { std::slice::from_raw_parts(ansi, 16) };
    let mut palette = Palette {
        ansi: [Rgb::default(); 16],
        foreground: rgb(foreground),
        background: rgb(background),
        cursor: rgb(cursor),
    };
    for (i, v) in a.iter().enumerate() {
        palette.ansi[i] = rgb(*v);
    }
    guard((), || t.with(|term| term.set_palette(palette)));
}

/// A cell's size in pixels (size reports, how many cells an image covers).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_set_cell_pixels(t: *const MpTerm, width: u16, height: u16) {
    let t = term!(t);
    guard((), || t.with(|term| term.set_cell_pixels(width, height)));
}

/// Bold text in the 8 basic colours draws bright.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_set_bold_is_bright(t: *const MpTerm, on: bool) {
    let t = term!(t);
    guard((), || t.with(|term| term.set_bold_is_bright(on)));
}

/// Lines of history kept; fewer drops the oldest at once.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_set_scrollback(t: *const MpTerm, lines: u32) {
    let t = term!(t);
    guard((), || t.with(|term| term.set_scrollback(lines as usize)));
}

/// Frames carry `n` rows below the viewport too (0 or 1, for a view
/// scrolled by pixels).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_set_extra_rows(t: *const MpTerm, n: u16) {
    let t = term!(t);
    guard((), || t.with(|term| term.set_extra_rows(n.min(1))));
}

/// The view back on the live screen.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_scroll_to_bottom(t: *const MpTerm) {
    let t = term!(t);
    guard((), || t.with(|term| term.scroll_to_bottom()));
}

/// Lines the view is scrolled back, and lines of history there are.
///
/// # Safety
/// `t` live; the outs writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_scroll_state(
    t: *const MpTerm,
    offset: *mut u32,
    history: *mut u32,
) {
    let t = term!(t);
    let (o, h) = guard((0, 0), || {
        t.with(|term| (term.display_offset() as u32, term.history_len() as u32))
    });
    // SAFETY: the contract above.
    unsafe {
        if !offset.is_null() {
            *offset = o;
        }
        if !history.is_null() {
            *history = h;
        }
    }
}

/// A selection's ends (viewport rows, negative above the viewport; its end
/// column inclusive).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MpSelectionBounds {
    pub start_row: i32,
    pub start_col: u16,
    pub end_row: i32,
    pub end_col: u16,
}

/// Where the selection's handles go; false: nothing selected.
///
/// # Safety
/// `t` live; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_selection_bounds(
    t: *const MpTerm,
    out: *mut MpSelectionBounds,
) -> bool {
    let t = term!(t, false);
    if out.is_null() {
        return false;
    }
    match guard(None, || t.with(|term| term.selection_bounds())) {
        Some(((sr, sc), (er, ec))) => {
            // SAFETY: the contract above.
            unsafe {
                *out = MpSelectionBounds {
                    start_row: sr,
                    start_col: sc,
                    end_row: er,
                    end_col: ec,
                };
            }
            true
        }
        None => false,
    }
}

/// The search's matches no longer show.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_search_clear(t: *const MpTerm) {
    let t = term!(t);
    guard((), || t.with(|term| term.search_clear()));
}

/// Viewport row `row`'s text (free with mp_string_free), trailing blanks
/// trimmed, and whether it continues on the next row.
///
/// # Safety
/// `t` live; `wrapped` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_view_row_text(
    t: *const MpTerm,
    row: u16,
    wrapped: *mut bool,
) -> *mut c_char {
    let t = term!(t, std::ptr::null_mut());
    let (text, w) = guard((String::new(), false), || {
        t.with(|term| term.view_row_text(row))
    });
    if !wrapped.is_null() {
        // SAFETY: the contract above.
        unsafe { *wrapped = w };
    }
    into_c(text)
}

/// The view's size in cells: an opened terminal also tells the core, a
/// local one takes it after the bytes pushed so far.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_resize(t: *const MpTerm, cols: u16, rows: u16) {
    let t = term!(t);
    guard((), || match t {
        MpTerm::Pane(p) => p.resize(cols, rows),
        MpTerm::Bare(_) => t.with(|term| term.resize(cols, rows)),
    });
}

/// The next frame, copied (free with [`mp_frame_free`]): the one the
/// terminal's thread published if it waits (then what changed after it is
/// framed now and left for [`mp_term_take_frame`]), else what changed since
/// the last frame. For a view framing a change of its own (a scroll, a
/// selection): take this, then [`mp_term_take_frame`] until it is NULL.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_frame(t: *const MpTerm) -> *mut Frame {
    let t = term!(t, std::ptr::null_mut());
    guard(std::ptr::null_mut(), || {
        let frame = match t {
            MpTerm::Pane(p) => p.frame(),
            MpTerm::Bare(_) => t.with(|term| term.frame()),
        };
        Box::into_raw(Box::new(frame))
    })
}

/// The frame the terminal's thread published (it publishes one at the end
/// of each pass and about every display refresh of a long one), or NULL:
/// never waits for the terminal's lock. What a view's wake takes.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_take_frame(t: *const MpTerm) -> *mut Frame {
    let t = term!(t, std::ptr::null_mut());
    guard(std::ptr::null_mut(), || match t {
        MpTerm::Pane(p) => p
            .take_published()
            .map_or(std::ptr::null_mut(), |f| Box::into_raw(Box::new(f))),
        MpTerm::Bare(_) => std::ptr::null_mut(),
    })
}

/// # Safety
/// `f` NULL or from mp_term_frame, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_frame_free(f: *mut Frame) {
    if !f.is_null() {
        // SAFETY: the contract above.
        drop(unsafe { Box::from_raw(f) });
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MpFrameInfo {
    pub cols: u16,
    pub rows: u16,
    pub full: bool,
    pub changed: u32,
    /// Move the rows drawn last up this many rows (negative: down) first.
    pub shift: i32,
    pub cursor: CursorView,
    pub display_offset: u32,
    pub pixel_shift: u32,
    pub history: u32,
    pub background: u32,
    pub links: u32,
    /// Decoded images the rows' image runs point at.
    pub images: u32,
    /// Marks the parse reached since the last frame ([`mp_frame_mark`]).
    pub marks: u32,
}

/// # Safety
/// `f` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_frame_info(f: *const Frame) -> MpFrameInfo {
    if f.is_null() {
        return MpFrameInfo::default();
    }
    // SAFETY: the contract above.
    let f = unsafe { &*f };
    MpFrameInfo {
        cols: f.cols,
        rows: f.rows,
        full: f.full,
        changed: f.changed.len() as u32,
        shift: f.shift,
        cursor: f.cursor,
        display_offset: f.display_offset,
        pixel_shift: f.pixel_shift,
        history: f.history,
        background: f.background,
        links: f.links.len() as u32,
        images: f.images.len() as u32,
        marks: f.marks.len() as u32,
    }
}

/// The frame's `i`-th mark, in the order the output carried them: act on
/// it once this frame is drawn.
///
/// # Safety
/// `f` live; `code` and `arg` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_frame_mark(
    f: *const Frame,
    i: u32,
    code: *mut u32,
    arg: *mut i64,
) -> bool {
    if f.is_null() {
        return false;
    }
    // SAFETY: the contract above.
    let f = unsafe { &*f };
    let Some(m) = f.marks.get(i as usize) else {
        return false;
    };
    // SAFETY: the contract above.
    unsafe {
        if !code.is_null() {
            *code = m.code;
        }
        if !arg.is_null() {
            *arg = m.arg;
        }
    }
    true
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MpFrameRow {
    pub row: u16,
    pub wrapped: bool,
    pub cells: *const DrawCell,
    pub ncells: u32,
    pub extra: *const u32,
    pub nextra: u32,
    pub images: *const ImageRun,
    pub nimages: u32,
}

/// The `i`-th changed row; the pointers live as long as the frame.
///
/// # Safety
/// `f` live, `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_frame_row(f: *const Frame, i: u32, out: *mut MpFrameRow) -> bool {
    if f.is_null() || out.is_null() {
        return false;
    }
    // SAFETY: the contract above.
    let f = unsafe { &*f };
    let Some(r) = f.changed.get(i as usize) else {
        return false;
    };
    // SAFETY: the contract above.
    unsafe {
        *out = MpFrameRow {
            row: r.row,
            wrapped: r.wrapped,
            cells: r.cells.as_ptr(),
            ncells: r.cells.len() as u32,
            extra: r.extra.as_ptr(),
            nextra: r.extra.len() as u32,
            images: r.images.as_ptr(),
            nimages: r.images.len() as u32,
        };
    }
    true
}

/// The URI of link `link` (a cell's 1-based `link`), NUL-terminated, a
/// copy: free with [`mp_string_free`].
///
/// # Safety
/// `f` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_frame_link(f: *const Frame, link: u32) -> *mut c_char {
    if f.is_null() || link == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: the contract above.
    let f = unsafe { &*f };
    f.links
        .get(link as usize - 1)
        .map_or(std::ptr::null_mut(), |s| into_c(s.clone()))
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MpImage {
    /// The same for the same image data, across frames: a texture cache key.
    pub id: u64,
    pub width: u32,
    pub height: u32,
    /// RGBA, 8 bits a channel, row after row; lives as long as the frame.
    pub rgba: *const u8,
    pub len: usize,
}

/// Image `i` (an image run's 1-based `image` minus one) of a frame.
///
/// # Safety
/// `f` live, `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_frame_image(f: *const Frame, i: u32, out: *mut MpImage) -> bool {
    if f.is_null() || out.is_null() {
        return false;
    }
    // SAFETY: the contract above.
    let f = unsafe { &*f };
    let Some(img) = f.images.get(i as usize) else {
        return false;
    };
    // SAFETY: the contract above.
    unsafe {
        *out = MpImage {
            id: img.id,
            width: img.width,
            height: img.height,
            rgba: img.rgba.as_ptr(),
            len: img.rgba.len(),
        };
    }
    true
}

fn into_c(s: String) -> *mut c_char {
    CString::new(s.replace('\0', "")).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// # Safety
/// `s` NULL or a string this library returned, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_string_free(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: the contract above.
        drop(unsafe { CString::from_raw(s) });
    }
}

/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_scroll(t: *const MpTerm, lines: i32) {
    let t = term!(t);
    guard((), || t.with(|term| term.scroll(lines)));
}

/// `kind`: 0 cells, 1 word, 2 line, 3 block, 4 semantic (the link under
/// the point, else its word).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_select_start(
    t: *const MpTerm,
    kind: u8,
    row: u16,
    col: u16,
    right_half: bool,
) {
    let t = term!(t);
    let kind = match kind {
        1 => SelectKind::Word,
        2 => SelectKind::Line,
        3 => SelectKind::Block,
        4 => SelectKind::Semantic,
        _ => SelectKind::Cells,
    };
    guard((), || {
        t.with(|term| term.select_start(kind, row, col, right_half))
    });
}

/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_select_extend(
    t: *const MpTerm,
    row: u16,
    col: u16,
    right_half: bool,
) {
    let t = term!(t);
    guard((), || {
        t.with(|term| term.select_extend(row, col, right_half))
    });
}

/// A handle's drag starts: the selection keeps its other end, on screen or
/// not (`end`: keeps the end), and mp_term_select_extend moves the dragged
/// one (B477). False: no selection.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_select_keep_end(t: *const MpTerm, end: bool) -> bool {
    let t = term!(t, false);
    guard(false, || t.with(|term| term.select_keep_end(end)))
}

/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_select_clear(t: *const MpTerm) {
    let t = term!(t);
    guard((), || t.with(|term| term.select_clear()));
}

/// The selected text (free with mp_string_free); NULL: nothing selected.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_selection_text(t: *const MpTerm) -> *mut c_char {
    let t = term!(t, std::ptr::null_mut());
    guard(std::ptr::null_mut(), || {
        t.with(|term| term.selection_text())
            .map_or(std::ptr::null_mut(), into_c)
    })
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MpMatch {
    pub start_row: u16,
    pub start_col: u16,
    pub end_row: u16,
    pub end_col: u16,
}

/// A find's result ([`mp_term_find`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MpFound {
    pub matched: MpMatch,
    /// 0-based, oldest first.
    pub index: u32,
    pub count: u32,
}

/// Find flags.
pub mod find_flags {
    pub const CASE_SENSITIVE: u8 = 1;
    pub const WORD_START: u8 = 2;
    pub const WORD_END: u8 = 4;
}

/// Find `text` as a person asks for it (not a pattern; any case unless
/// `flags` says): `step` 0 shows the match kept or the newest, 1 the next
/// newer, 2 the next older, each round past the end; the view scrolls to
/// it, it draws marked. False: no match (an empty `text` clears).
///
/// # Safety
/// `t` live; `text` NULL or NUL-terminated; `out` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_find(
    t: *const MpTerm,
    text: *const c_char,
    flags: u8,
    step: u8,
    out: *mut MpFound,
) -> bool {
    let t = term!(t, false);
    let text = c_str(text).unwrap_or_default();
    let options = crate::FindOptions {
        case_sensitive: flags & find_flags::CASE_SENSITIVE != 0,
        word_start: flags & find_flags::WORD_START != 0,
        word_end: flags & find_flags::WORD_END != 0,
    };
    let step = match step {
        1 => crate::FindStep::Next,
        2 => crate::FindStep::Previous,
        _ => crate::FindStep::Newest,
    };
    guard(false, || {
        match t.with(|term| term.find(&text, options, step)) {
            Some(f) => {
                if !out.is_null() {
                    // SAFETY: the contract above.
                    unsafe {
                        *out = MpFound {
                            matched: MpMatch {
                                start_row: f.matched.start.0,
                                start_col: f.matched.start.1,
                                end_row: f.matched.end.0,
                                end_col: f.matched.end.1,
                            },
                            index: f.index as u32,
                            count: f.count as u32,
                        };
                    }
                }
                true
            }
            None => false,
        }
    })
}

/// The next match of `pattern` (UTF-8 regex) and the view scrolled to it.
///
/// # Safety
/// `t` live; `pattern` a NUL-terminated string; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_search(
    t: *const MpTerm,
    pattern: *const c_char,
    forward: bool,
    out: *mut MpMatch,
) -> bool {
    let t = term!(t, false);
    if pattern.is_null() || out.is_null() {
        return false;
    }
    // SAFETY: the contract above.
    let Ok(p) = unsafe { CStr::from_ptr(pattern) }.to_str() else {
        return false;
    };
    guard(false, || match t.with(|term| term.search(p, forward)) {
        Some(m) => {
            // SAFETY: the contract above.
            unsafe {
                *out = MpMatch {
                    start_row: m.start.0,
                    start_col: m.start.1,
                    end_row: m.end.0,
                    end_col: m.end.1,
                };
            }
            true
        }
        None => false,
    })
}

/// The URI of the OSC 8 link under a viewport cell (free with
/// mp_string_free); NULL: none.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_link_at(t: *const MpTerm, row: u16, col: u16) -> *mut c_char {
    let t = term!(t, std::ptr::null_mut());
    guard(std::ptr::null_mut(), || {
        t.with(|term| term.link_at(row, col))
            .map_or(std::ptr::null_mut(), |(uri, _)| into_c(uri))
    })
}

/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_modes(t: *const MpTerm) -> Modes {
    let t = term!(t, Modes::default());
    guard(Modes::default(), || t.with(|term| term.modes()))
}

// --- Pastes, taps -----------------------------------------------------------

fn c_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: callers pass NUL-terminated strings (the header's contract).
    Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

/// What a paste of `text` sends: bracketed if the program asked for it or
/// `force`, never a marker inside; the view back on the live screen.
///
/// # Safety
/// `t` live; `text` NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_paste(
    t: *const MpTerm,
    text: *const c_char,
    force: bool,
) -> MpOwnedBytes {
    let t = term!(t, MpOwnedBytes::NONE);
    let Some(text) = c_str(text) else {
        return MpOwnedBytes::NONE;
    };
    guard(MpOwnedBytes::NONE, || {
        MpOwnedBytes::from(t.with(|term| term.paste(&text, force)))
    })
}

/// What a tap means.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MpTap {
    /// 0 a click (`keyboard`: on the cursor's rows, take the keyboard),
    /// 1 a selection dismissed, 2 a link (`link`, free with
    /// mp_string_free).
    pub kind: u8,
    pub keyboard: bool,
    pub link: *mut c_char,
}

/// A tap at a viewport cell.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_tap(t: *const MpTerm, row: u16, col: u16) -> MpTap {
    let none = MpTap {
        kind: 0,
        keyboard: false,
        link: std::ptr::null_mut(),
    };
    let t = term!(t, none);
    match guard(Tap::Click { keyboard: false }, || {
        t.with(|term| term.tap(row, col))
    }) {
        Tap::Click { keyboard } => MpTap { keyboard, ..none },
        Tap::Dismissed => MpTap { kind: 1, ..none },
        Tap::Link(uri) => MpTap {
            kind: 2,
            link: into_c(uri),
            ..none
        },
    }
}

// --- The input method ----------------------------------------------------

/// An input-method call's result: the bytes to send, and whether the
/// platform tells its input delegate (a change the input system did not
/// make itself).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MpImeResult {
    pub bytes: MpOwnedBytes,
    pub notify: bool,
    /// Text to send as a paste, through the platform's paste path, after
    /// `bytes`; null when none. Free with `mp_string_free`.
    pub paste: *mut c_char,
}

impl MpImeResult {
    const NONE: Self = Self {
        bytes: MpOwnedBytes::NONE,
        notify: false,
        paste: std::ptr::null_mut(),
    };
}

fn ime_call(t: &MpTerm, f: impl FnOnce(&mut crate::Ime) -> crate::ime::Outcome) -> MpImeResult {
    guard(MpImeResult::NONE, || {
        let sent = t.with(|term| term.ime_apply(f));
        MpImeResult {
            bytes: MpOwnedBytes::from(sent.bytes),
            notify: sent.notify,
            paste: sent.paste.map_or(std::ptr::null_mut(), into_c),
        }
    })
}

/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_ime_focus(t: *const MpTerm, focused: bool) -> MpImeResult {
    let t = term!(t, MpImeResult::NONE);
    ime_call(t, |ime| ime.focus(focused))
}

/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_ime_reset(t: *const MpTerm) -> MpImeResult {
    let t = term!(t, MpImeResult::NONE);
    ime_call(t, |ime| ime.reset())
}

/// # Safety
/// `t` live; `text` NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_ime_insert(t: *const MpTerm, text: *const c_char) -> MpImeResult {
    let t = term!(t, MpImeResult::NONE);
    let text = c_str(text).unwrap_or_default();
    ime_call(t, |ime| ime.insert(&text))
}

/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_ime_delete(t: *const MpTerm) -> MpImeResult {
    let t = term!(t, MpImeResult::NONE);
    ime_call(t, |ime| ime.delete_backward())
}

/// The composition: `text` (NULL or empty: cancelled), its caret
/// `sel_start` graphemes in, `sel_len` selected.
///
/// # Safety
/// `t` live; `text` NULL or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_ime_set_marked(
    t: *const MpTerm,
    text: *const c_char,
    sel_start: u32,
    sel_len: u32,
) -> MpImeResult {
    let t = term!(t, MpImeResult::NONE);
    let text = c_str(text);
    ime_call(t, |ime| {
        ime.set_marked(text.as_deref(), (sel_start as usize, sel_len as usize))
    })
}

/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_ime_unmark(t: *const MpTerm) -> MpImeResult {
    let t = term!(t, MpImeResult::NONE);
    ime_call(t, |ime| ime.unmark())
}

/// # Safety
/// `t` live; `text` NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_ime_replace(
    t: *const MpTerm,
    start: u32,
    end: u32,
    text: *const c_char,
) -> MpImeResult {
    let t = term!(t, MpImeResult::NONE);
    let text = c_str(text).unwrap_or_default();
    ime_call(t, |ime| ime.replace(start as usize, end as usize, &text))
}

/// The platform's event loop finished the turn of a delete: a Return now
/// is the person's, not the input system putting back the phantom (B410).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_ime_turn_end(t: *const MpTerm) {
    let t = term!(t);
    guard((), || t.with(|term| term.ime.turn_end()));
}

/// The input system moved its caret or selection.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_ime_select(t: *const MpTerm, start: u32, end: u32) {
    let t = term!(t);
    guard((), || {
        t.with(|term| term.ime.select(start as usize, end as usize))
    });
}

/// The shadow document's shape, in graphemes.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MpImeState {
    pub len: u32,
    pub sel_start: u32,
    pub sel_end: u32,
    /// -1: no composition.
    pub marked_start: i32,
    pub marked_end: i32,
}

/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_ime_state(t: *const MpTerm) -> MpImeState {
    let t = term!(t, MpImeState::default());
    guard(MpImeState::default(), || {
        t.with(|term| {
            let ime = &term.ime;
            let (a, b) = ime.selected();
            let (ma, mb) = ime.marked().map_or((-1, -1), |(a, b)| (a as i32, b as i32));
            MpImeState {
                len: ime.len() as u32,
                sel_start: a as u32,
                sel_end: b as u32,
                marked_start: ma,
                marked_end: mb,
            }
        })
    })
}

/// The text of graphemes `start..end` of the shadow document (free with
/// mp_string_free).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_ime_text(t: *const MpTerm, start: u32, end: u32) -> *mut c_char {
    let t = term!(t, std::ptr::null_mut());
    guard(std::ptr::null_mut(), || {
        into_c(t.with(|term| term.ime.text_in(start as usize, end as usize)))
    })
}

// --- The scroll model ------------------------------------------------------

/// What a scroll step did.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MpScrollAction {
    /// 0 nothing, 1 the history moved (`moved_px`), 2 wheel reports to send
    /// (`bytes`, free with mp_bytes_free), 3 rows for the host (`rows`).
    pub kind: u8,
    pub moved_px: i32,
    pub rows: i32,
    pub bytes: MpOwnedBytes,
}

fn scroll_action(a: ScrollAction) -> MpScrollAction {
    let mut out = MpScrollAction {
        kind: 0,
        moved_px: 0,
        rows: 0,
        bytes: MpOwnedBytes::NONE,
    };
    match a {
        ScrollAction::None => {}
        ScrollAction::Local { moved_px } => {
            out.kind = 1;
            out.moved_px = moved_px.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        }
        ScrollAction::Wheel(bytes) => {
            out.kind = 2;
            out.bytes = MpOwnedBytes::from(bytes);
        }
        ScrollAction::Host { rows } => {
            out.kind = 3;
            out.rows = rows;
        }
    }
    out
}

/// A drag of `dy` device pixels (positive: older output); `host_rows`: the
/// session scrolls its own buffer.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_scroll_drag(
    t: *const MpTerm,
    dy: f64,
    host_rows: bool,
) -> MpScrollAction {
    let t = term!(t, scroll_action(ScrollAction::None));
    scroll_action(guard(ScrollAction::None, || {
        t.with(|term| term.scroll_drag(dy, host_rows))
    }))
}

/// The drag ended (a lift near the live screen docks onto it).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_scroll_end(t: *const MpTerm) -> MpScrollAction {
    let t = term!(t, scroll_action(ScrollAction::None));
    scroll_action(guard(ScrollAction::None, || {
        t.with(|term| term.scroll_end())
    }))
}

/// A drag's vertical move, `dy` device pixels (positive: older), wherever
/// it went: what the lift's speed is read from (B521).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_fling_track(t: *const MpTerm, dy: f64) {
    let t = term!(t);
    guard((), || t.with(|term| term.fling_track(dy)));
}

/// The drag's lift: true if it coasts. `elsewhere`: the drag went to the
/// host (tmux, herdr).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_fling_start(t: *const MpTerm, elsewhere: bool) -> bool {
    let t = term!(t, false);
    guard(false, || t.with(|term| term.fling_start(elsewhere)))
}

/// One display refresh of a coast, `dt` seconds on: true while it coasts;
/// `dy` (may be NULL) gets the pixels to route now, as a drag's (0: none
/// this refresh).
///
/// # Safety
/// `t` live; `dy` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_fling_step(t: *const MpTerm, dt: f64, dy: *mut f64) -> bool {
    let t = term!(t, false);
    let step = guard(None, || t.with(|term| term.fling_step(dt)));
    if !dy.is_null() {
        // SAFETY: the caller's writable f64.
        unsafe { *dy = step.unwrap_or(0.0) };
    }
    step.is_some()
}

/// A touch: true if it stopped a coast (that touch is then not a tap).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_fling_stop(t: *const MpTerm) -> bool {
    let t = term!(t, false);
    guard(false, || t.with(|term| term.fling_stop()))
}

// --- Accessibility -----------------------------------------------------------

/// What a screen reader hears now (free with mp_string_free; NULL:
/// nothing), paced: `again_ms` (may be NULL) gets when to ask again, 0 for
/// no need (B427).
///
/// # Safety
/// `t` live; `again_ms` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_a11y_announce(
    t: *const MpTerm,
    again_ms: *mut u32,
) -> *mut c_char {
    let t = term!(t, std::ptr::null_mut());
    guard(std::ptr::null_mut(), || {
        let said = t.with(|term| term.announce());
        if !again_ms.is_null() {
            let again = said
                .again_in_ms
                .map_or(0, |ms| ms.clamp(1, u32::MAX as u64) as u32);
            // SAFETY: the contract above.
            unsafe { *again_ms = again };
        }
        said.text.map_or(std::ptr::null_mut(), into_c)
    })
}

// --- mosh's predictive echo --------------------------------------------------

/// Predictive echo on (`mode` 1 adaptive, 2 always, 3 experimental; 0
/// off); `trail` dims unconfirmed guesses on a slow link.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_predict_attach(t: *const MpTerm, mode: u8, trail: bool) {
    use crate::predict::PredictMode as M;
    let t = term!(t);
    let mode = match mode {
        1 => M::Adaptive,
        2 => M::Always,
        3 => M::Experimental,
        _ => M::Never,
    };
    guard((), || t.with(|term| term.predict_attach(mode, trail)));
}

/// The transport's smoothed round trip, ms.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_predict_srtt(t: *const MpTerm, ms: f64) {
    let t = term!(t);
    guard((), || t.with(|term| term.predict_srtt(ms)));
}

/// Keystrokes on their way: the batch to report with
/// mp_term_predict_sent, 0 when nothing was predicted.
///
/// # Safety
/// `t` live; `bytes` points at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_predict_input(
    t: *const MpTerm,
    bytes: *const u8,
    len: usize,
) -> u64 {
    let t = term!(t, 0);
    if bytes.is_null() || len == 0 {
        return 0;
    }
    // SAFETY: the contract above.
    let b = unsafe { std::slice::from_raw_parts(bytes, len) };
    guard(0, || t.with(|term| term.predict_input(b)).unwrap_or(0))
}

/// The transport queued `batch` as user state `num` (`queued` false: it
/// couldn't, every guess goes).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_predict_sent(
    t: *const MpTerm,
    batch: u64,
    num: u64,
    queued: bool,
) {
    let t = term!(t);
    guard((), || {
        t.with(|term| term.predict_sent(batch, queued.then_some(num)))
    });
}

/// A frame's echo ack: judged after the bytes pushed before it.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_predict_echo_ack(t: *const MpTerm, ack: u64) {
    let t = term!(t);
    guard((), || match t {
        MpTerm::Pane(p) => p.echo_ack(ack),
        MpTerm::Bare(_) => t.with(|term| term.predict_echo_ack(ack)),
    });
}

/// Every guess goes.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_predict_reset(t: *const MpTerm) {
    let t = term!(t);
    guard((), || t.with(|term| term.predict_reset()));
}

/// Whether mp_term_predict_tick should run every 50 ms.
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_predict_needs_tick(t: *const MpTerm) -> bool {
    let t = term!(t, false);
    guard(false, || t.with(|term| term.predict_needs_tick()))
}

/// The clock's part of judging: true when what shows changed (take a
/// frame).
///
/// # Safety
/// `t` live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_term_predict_tick(t: *const MpTerm) -> bool {
    let t = term!(t, false);
    guard(false, || t.with(|term| term.predict_tick()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    unsafe extern "C" fn no_wake(_: *mut c_void) {}

    #[test]
    fn a_pushed_mark_rides_back_on_a_frame() {
        // SAFETY: a no-op waker; the terminal is closed below.
        let t = unsafe { mp_term_open_local(10, 2, 10, no_wake, std::ptr::null_mut()) };
        assert!(!t.is_null());
        // SAFETY: `t` is live until the close.
        unsafe {
            assert!(mp_term_push(t, b"hi".as_ptr(), 2));
            assert!(mp_term_push_mark(t, 4, -7));
            assert!(
                !mp_term_push_mark(t, 0x8000_0002, 0),
                "the terminal's own code"
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let f = mp_term_frame(t);
                let info = mp_frame_info(f);
                if info.marks > 0 {
                    assert_eq!(info.marks, 1);
                    let (mut code, mut arg) = (0u32, 0i64);
                    assert!(mp_frame_mark(f, 0, &mut code, &mut arg));
                    assert_eq!((code, arg), (4, -7));
                    assert!(!mp_frame_mark(f, 1, &mut code, &mut arg));
                    mp_frame_free(f);
                    break;
                }
                mp_frame_free(f);
                assert!(Instant::now() < deadline, "the mark never came");
                std::thread::sleep(Duration::from_millis(2));
            }
            mp_term_close(t);
        }
        let bare = mp_term_new(4, 1, 0);
        // SAFETY: `bare` is live until the close.
        unsafe {
            assert!(
                !mp_term_push_mark(bare, 4, 0),
                "a bare terminal has no queue"
            );
            mp_term_close(bare);
        }
    }
}
