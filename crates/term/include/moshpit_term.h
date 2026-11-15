// The shared terminal core's C ABI (crates/term/src/ffi.rs): the emulator
// runs in Rust, off the main thread; the native view draws frames of it and
// sends what the user types through the byte channel (moshpit_stream.h).
// Every function is thread-safe. Design: docs/nsh/TERMINAL-CORE.md.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Bumped whenever anything below changes shape: a view checks
/// `mp_term_abi_version() == MP_TERM_ABI_VERSION` before it uses the core.
#define MP_TERM_ABI_VERSION 17
uint32_t mp_term_abi_version(void);

typedef struct MpTerm MpTerm;
typedef struct MpFrame MpFrame;

/// A frame is ready: called on the terminal's own thread. Hop to the view's
/// thread and return.
typedef void (*MpTermWaker)(void *_Nullable ctx);

/// Cell flags.
enum {
    MP_CELL_BOLD = 1 << 0,
    MP_CELL_ITALIC = 1 << 1,
    MP_CELL_UNDERLINE = 1 << 2,
    MP_CELL_DOUBLE_UNDERLINE = 1 << 3,
    MP_CELL_UNDERCURL = 1 << 4,
    MP_CELL_DOTTED_UNDERLINE = 1 << 5,
    MP_CELL_DASHED_UNDERLINE = 1 << 6,
    MP_CELL_STRIKEOUT = 1 << 7,
    MP_CELL_DIM = 1 << 8,
    MP_CELL_WIDE = 1 << 9,    ///< first column of a two-column character
    MP_CELL_SPACER = 1 << 10, ///< second column (or padding): background only
    MP_CELL_SELECTED = 1 << 11,
    MP_CELL_MATCH = 1 << 12,
    MP_CELL_LINK = 1 << 13,
};

/// One cell, colours resolved (0xRRGGBB). Its text: `ch`, then `extra`
/// joined scalars from the row's `extra` list starting at `extra_at`.
typedef struct {
    uint32_t ch;
    uint32_t extra_at;
    uint16_t extra;
    uint16_t flags;
    uint32_t fg;
    uint32_t bg;
    uint32_t underline; ///< UINT32_MAX: the foreground
    uint32_t link;      ///< 1-based link index in the frame; 0: none
} MpDrawCell;

typedef struct {
    uint16_t col;
    uint16_t row;
    uint8_t shape; ///< 0 block, 1 underline, 2 beam, 3 hollow, 4 hidden
    bool blinking;
    bool visible;
} MpCursor;

/// An image over a run of one row's cells: draw the texture rectangle
/// (fractions of the image) over the cells' rectangle.
typedef struct {
    uint16_t col;
    uint16_t cols;
    uint32_t image; ///< 1-based index of the frame's images (mp_frame_image)
    float left, top, right, bottom;
    int32_t z;      ///< below the text when negative
} MpImageRun;

/// A decoded image (kitty graphics, sixel, iTerm2), RGBA 8 bits a channel.
typedef struct {
    uint64_t id; ///< the same for the same data: a texture cache key
    uint32_t width;
    uint32_t height;
    const uint8_t *_Nullable rgba; ///< lives as long as the frame
    size_t len;
} MpImage;

typedef struct {
    uint16_t cols;
    uint16_t rows;
    bool full;          ///< every row is in the frame
    uint32_t changed;   ///< rows in the frame
    int32_t shift;      ///< move the rows drawn last up this many rows (negative: down) first
    MpCursor cursor;
    uint32_t display_offset;
    uint32_t pixel_shift; ///< draw the rows this many pixels up (pixel scroll)
    uint32_t history;
    uint32_t background;
    uint32_t links;
    uint32_t images;    ///< decoded images (mp_frame_image)
    uint32_t marks;     ///< marks the parse reached (mp_frame_mark)
} MpFrameInfo;

typedef struct {
    uint16_t row;  ///< row on screen as shown
    bool wrapped;  ///< continues on the next row
    const MpDrawCell *_Nullable cells;
    uint32_t ncells;
    const uint32_t *_Nullable extra;
    uint32_t nextra;
    const MpImageRun *_Nullable images;
    uint32_t nimages;
} MpFrameRow;

typedef struct {
    uint8_t mouse; ///< 0 off, 1 clicks, 2 drags, 3 motion
    bool mouse_sgr;
    bool mouse_utf8;
    bool bracketed_paste;
    bool app_cursor;
    bool app_keypad;
    bool focus_reports;
    bool alt_screen;
    bool alternate_scroll;
    uint8_t kitty_flags;
    bool cursor_visible;
} MpModes;

typedef struct {
    uint16_t start_row, start_col, end_row, end_col;
} MpMatch;

/// A find's result (mp_term_find).
typedef struct {
    MpMatch matched;
    uint32_t index; ///< 0-based, oldest first
    uint32_t count;
} MpFound;

/// Find flags.
enum {
    MP_FIND_CASE_SENSITIVE = 1 << 0,
    MP_FIND_WORD_START = 1 << 1,
    MP_FIND_WORD_END = 1 << 2,
};

/// A terminal fed from byte-channel stream `stream` on its own thread.
/// NULL if the stream is gone. `waker(ctx)` may be called until
/// mp_term_close returns.
MpTerm *_Nullable mp_term_open(uint64_t stream, uint16_t cols, uint16_t rows, uint32_t scrollback,
                               MpTermWaker _Nonnull waker, void *_Nullable ctx);
/// A terminal the view feeds itself (mp_term_push) through a byte channel of
/// its own, parsed on its own thread in the order pushed. `waker(ctx)` is
/// called on that thread when a frame or a reply (mp_term_take_replies) is
/// ready, until mp_term_close returns.
MpTerm *_Nullable mp_term_open_local(uint16_t cols, uint16_t rows, uint32_t scrollback,
                                     MpTermWaker _Nonnull waker, void *_Nullable ctx);
/// A terminal the caller feeds with mp_term_feed.
MpTerm *_Nullable mp_term_new(uint16_t cols, uint16_t rows, uint32_t scrollback);
void mp_term_close(MpTerm *_Nullable t);
/// Lets every decoded image go, for every terminal (one cache for the
/// process): a memory warning, the app going to the background. Images stay
/// on screen: the next frame that draws one decodes it again.
void mp_term_images_purge(void);
/// Bytes of decoded images the process keeps (debug probes).
size_t mp_term_images_bytes(void);
/// The app goes to the background, for `t`: the images its emulator keeps
/// only to share identical ones are let go. Bytes freed.
size_t mp_term_release_image_caches(const MpTerm *_Nonnull t);
/// A memory warning, for `t`: what mp_term_release_image_caches lets go, and
/// the kitty images no cell shows any more (a program placing one again gets
/// ENOENT). Bytes freed.
size_t mp_term_drop_unplaced_images(const MpTerm *_Nonnull t);
/// `t` is a tmux pane's: a clear of its main screen keeps what tmux's
/// scroll-on-clear keeps (ESC [2J, and ESC [J from the top left, the clear a
/// tmux pane sends). Off: a terminal of its own (ESC [2J only).
void mp_term_set_tmux_clears(const MpTerm *_Nonnull t, bool on);
/// Bytes `t`'s stored kitty images take (debug probes).
size_t mp_term_kitty_image_bytes(const MpTerm *_Nonnull t);
void mp_term_feed(const MpTerm *_Nonnull t, const uint8_t *_Nonnull bytes, size_t len);
/// The remote's bytes into a local terminal, after everything pushed before.
/// False for any other kind.
bool mp_term_push(const MpTerm *_Nonnull t, const uint8_t *_Nullable bytes, size_t len);
/// A mark among a local terminal's bytes (a session's code and argument):
/// it comes back with the first frame that has every byte pushed before it
/// (mp_frame_mark), and no earlier frame has a byte pushed after it. An
/// opened terminal's frames carry its stream's marks the same way. False
/// for any other kind, and for a code with the top bit set.
bool mp_term_push_mark(const MpTerm *_Nonnull t, uint32_t code, int64_t arg);
/// The view's size in cells. An opened terminal's grid follows at once and
/// the core hears of it; a local one's follows after the bytes pushed so far.
void mp_term_resize(const MpTerm *_Nonnull t, uint16_t cols, uint16_t rows);
/// Forgets a half-read sequence (a reconnect); a local terminal after the
/// bytes pushed so far.
void mp_term_discard_pending_input(const MpTerm *_Nonnull t);

/// Bytes the library made, the caller's: free with mp_bytes_free.
typedef struct {
    uint8_t *_Nullable ptr;
    size_t len;
} MpOwnedBytes;
void mp_bytes_free(MpOwnedBytes b);

/// A local terminal's replies to the remote (DA, CPR, colours…) since the
/// last call, in order: send them as keys are sent.
MpOwnedBytes mp_term_take_replies(const MpTerm *_Nonnull t);

/// Keys (`key` of mp_term_key).
enum {
    MP_KEY_CHAR = 0, ///< `scalar` is the text the key makes
    MP_KEY_ENTER = 1,
    MP_KEY_TAB = 2,
    MP_KEY_BACKSPACE = 3,
    MP_KEY_ESCAPE = 4,
    MP_KEY_UP = 5,
    MP_KEY_DOWN = 6,
    MP_KEY_LEFT = 7,
    MP_KEY_RIGHT = 8,
    MP_KEY_HOME = 9,
    MP_KEY_END = 10,
    MP_KEY_PAGE_UP = 11,
    MP_KEY_PAGE_DOWN = 12,
    MP_KEY_INSERT = 13,
    MP_KEY_DELETE = 14,
    MP_KEY_LEFT_SHIFT = 15,
    MP_KEY_RIGHT_SHIFT = 16,
    MP_KEY_LEFT_CONTROL = 17,
    MP_KEY_RIGHT_CONTROL = 18,
    MP_KEY_LEFT_ALT = 19,
    MP_KEY_RIGHT_ALT = 20,
    MP_KEY_LEFT_SUPER = 21,
    MP_KEY_RIGHT_SUPER = 22,
    MP_KEY_CAPS_LOCK = 23,
    MP_KEY_FUNCTION = 0x100, ///< + n: Fn, 1–24
};
/// Modifier bits.
enum {
    MP_MOD_SHIFT = 1 << 0,
    MP_MOD_ALT = 1 << 1,
    MP_MOD_CTRL = 1 << 2,
    MP_MOD_SUPER = 1 << 3,
    MP_MOD_CAPS_LOCK = 1 << 4,
};
/// The bytes a key sends, encoded as the remote program asked (the kitty
/// keyboard protocol's presses, repeats when `repeat` > 1 and releases when
/// `down` is false; else xterm's, with DECCKM). Empty: nothing to send.
MpOwnedBytes mp_term_key(const MpTerm *_Nonnull t, uint32_t key, uint32_t scalar, uint8_t mods, bool down,
                         uint16_t repeat);
/// The bytes a mouse report sends: `kind` 0 press, 1 release, 2 motion;
/// `button` 0 left, 1 middle, 2 right, 3 none, 4–7 wheel up/down/left/right;
/// a viewport cell. Empty while the program asked for no reports.
MpOwnedBytes mp_term_mouse(const MpTerm *_Nonnull t, uint8_t kind, uint8_t button, uint16_t row, uint16_t col,
                           uint8_t mods);
/// The focus report the program asked for (CSI ? 1004 h), or nothing.
MpOwnedBytes mp_term_focus(const MpTerm *_Nonnull t, bool focused);

/// Event kinds.
enum {
    MP_TERM_EVENT_TITLE = 1,
    MP_TERM_EVENT_BELL = 2,
    MP_TERM_EVENT_CLIPBOARD_STORE = 3,
    MP_TERM_EVENT_CLIPBOARD_LOAD = 4,
    MP_TERM_EVENT_DIRECTORY = 5, ///< OSC 7: the URL the program reported ("" if none parsed)
};
/// `text`: the title or the clipboard's text (valid during the call), else "".
typedef void (*MpTermEventFn)(void *_Nullable ctx, uint8_t kind, const char *_Nonnull text);
/// Every event since the last call, in order, to `each`. An opened terminal's
/// never wait for its lock (its thread moves them out as it parses).
void mp_term_take_events(const MpTerm *_Nonnull t, MpTermEventFn _Nonnull each, void *_Nullable ctx);
/// Answers an OSC 52 query (NULL: an empty clipboard).
void mp_term_answer_clipboard(const MpTerm *_Nonnull t, const char *_Nullable text);

/// Paths as links: `extensions` (comma separated) are what the platform
/// can preview; "" or NULL turns paths off and forgets the confirmed ones.
void mp_term_set_path_extensions(const MpTerm *_Nonnull t, const char *_Nullable extensions);
/// The path candidates on screen, each once: lines "F<tab>token" or
/// "D<tab>token" (only as a folder). NULL: none. Free with mp_string_free.
char *_Nullable mp_term_path_candidates(const MpTerm *_Nonnull t);
/// The confirmed ones, replacing those before: lines "F|D<tab>token<tab>uri".
/// True when what draws as a link changed (take a frame).
bool mp_term_set_path_links(const MpTerm *_Nonnull t, const char *_Nullable table);

/// The theme (0xRRGGBB): 16 colours, foreground, background, cursor.
void mp_term_set_palette(const MpTerm *_Nonnull t, const uint32_t *_Nonnull ansi, uint32_t foreground,
                         uint32_t background, uint32_t cursor);
void mp_term_set_cell_pixels(const MpTerm *_Nonnull t, uint16_t width, uint16_t height);
void mp_term_set_bold_is_bright(const MpTerm *_Nonnull t, bool on);
void mp_term_set_scrollback(const MpTerm *_Nonnull t, uint32_t lines);
/// Frames carry `n` (0 or 1) rows below the viewport too (the scroll model
/// sets it; for tests).
void mp_term_set_extra_rows(const MpTerm *_Nonnull t, uint16_t n);

/// The next frame, copied (draw from it on any thread): the one the terminal's
/// thread published if it waits (what changed after it is then framed and left
/// for mp_term_take_frame), else what changed since the last frame. For a
/// change of the view's own: take this, then mp_term_take_frame until NULL.
MpFrame *_Nullable mp_term_frame(const MpTerm *_Nonnull t);
/// The frame the terminal's thread published (at the end of each pass, and
/// about every display refresh of a long one), or NULL; never waits for the
/// terminal's lock. What a view's wake takes.
MpFrame *_Nullable mp_term_take_frame(const MpTerm *_Nonnull t);
void mp_frame_free(MpFrame *_Nullable f);
MpFrameInfo mp_frame_info(const MpFrame *_Nonnull f);
/// The frame's `i`-th mark, in the output's order: act on it once the
/// frame is drawn.
bool mp_frame_mark(const MpFrame *_Nonnull f, uint32_t i, uint32_t *_Nullable code, int64_t *_Nullable arg);
bool mp_frame_row(const MpFrame *_Nonnull f, uint32_t i, MpFrameRow *_Nonnull out);
/// Image `i` of the frame (an image run's `image` minus one).
bool mp_frame_image(const MpFrame *_Nonnull f, uint32_t i, MpImage *_Nonnull out);
/// A link's URI (free with mp_string_free).
char *_Nullable mp_frame_link(const MpFrame *_Nonnull f, uint32_t link);
void mp_string_free(char *_Nullable s);

void mp_term_scroll(const MpTerm *_Nonnull t, int32_t lines);
void mp_term_scroll_to_bottom(const MpTerm *_Nonnull t);
/// Lines the view is scrolled back, lines of history.
void mp_term_scroll_state(const MpTerm *_Nonnull t, uint32_t *_Nullable offset, uint32_t *_Nullable history);
/// kind: 0 cells, 1 word, 2 line, 3 block, 4 semantic (the link under the
/// point, wrapped rows included, else its word).
void mp_term_select_start(const MpTerm *_Nonnull t, uint8_t kind, uint16_t row, uint16_t col, bool right_half);
void mp_term_select_extend(const MpTerm *_Nonnull t, uint16_t row, uint16_t col, bool right_half);
/// A handle's drag starts: the selection keeps its other end, on screen or
/// not (`end`: keeps the end); mp_term_select_extend moves the dragged one
/// (B477). False: no selection.
bool mp_term_select_keep_end(const MpTerm *_Nonnull t, bool end);
void mp_term_select_clear(const MpTerm *_Nonnull t);
/// Selects everything: the first row with text, history included, to the
/// last cell with text (B699). False: no text anywhere.
bool mp_term_select_all(const MpTerm *_Nonnull t);
char *_Nullable mp_term_selection_text(const MpTerm *_Nonnull t);
typedef struct {
    int32_t start_row; ///< viewport rows; negative: above the viewport
    uint16_t start_col;
    int32_t end_row;
    uint16_t end_col;
} MpSelectionBounds;
bool mp_term_selection_bounds(const MpTerm *_Nonnull t, MpSelectionBounds *_Nonnull out);
bool mp_term_search(const MpTerm *_Nonnull t, const char *_Nonnull pattern, bool forward, MpMatch *_Nonnull out);
void mp_term_search_clear(const MpTerm *_Nonnull t);
/// Find `text` as typed (not a pattern; any case unless the flags say):
/// `step` 0 the match kept or the newest, 1 the next newer, 2 the next
/// older, each round past the end; the view scrolls to it and it draws
/// marked. False: no match (an empty text clears).
bool mp_term_find(const MpTerm *_Nonnull t, const char *_Nullable text, uint8_t flags, uint8_t step,
                  MpFound *_Nullable out);
/// Viewport row `row`'s text (free with mp_string_free) and whether it wraps.
char *_Nullable mp_term_view_row_text(const MpTerm *_Nonnull t, uint16_t row, bool *_Nullable wrapped);
char *_Nullable mp_term_link_at(const MpTerm *_Nonnull t, uint16_t row, uint16_t col);
MpModes mp_term_modes(const MpTerm *_Nonnull t);

// MARK: Pastes and taps

/// What a paste sends: bracketed if the program asked for it or `force`,
/// never a marker inside; the view back on the live screen.
MpOwnedBytes mp_term_paste(const MpTerm *_Nonnull t, const char *_Nonnull text, bool force);

typedef struct {
    uint8_t kind;  ///< 0 a click, 1 a selection dismissed, 2 a link
    bool keyboard; ///< a click on the cursor's rows on the live screen: take the keyboard
    char *_Nullable link; ///< kind 2: free with mp_string_free
} MpTap;
/// What a tap at a viewport cell means.
MpTap mp_term_tap(const MpTerm *_Nonnull t, uint16_t row, uint16_t col);

// MARK: The input method (docs/nsh/TERMINAL-VIEW.md V6–V10)

/// Bytes to send, and whether the platform tells its input delegate; then
/// text to send as a paste through the platform's paste path (lines a
/// keyboard inserted at once, B414).
typedef struct {
    MpOwnedBytes bytes;
    bool notify;
    /// Free with mp_string_free.
    char *_Nullable paste;
} MpImeResult;
/// Positions are graphemes of the shadow document.
typedef struct {
    uint32_t len;
    uint32_t sel_start;
    uint32_t sel_end;
    int32_t marked_start; ///< -1: no composition
    int32_t marked_end;
} MpImeState;
MpImeResult mp_term_ime_focus(const MpTerm *_Nonnull t, bool focused);
MpImeResult mp_term_ime_reset(const MpTerm *_Nonnull t);
MpImeResult mp_term_ime_insert(const MpTerm *_Nonnull t, const char *_Nonnull text);
MpImeResult mp_term_ime_delete(const MpTerm *_Nonnull t);
MpImeResult mp_term_ime_set_marked(const MpTerm *_Nonnull t, const char *_Nullable text, uint32_t sel_start,
                                   uint32_t sel_len);
MpImeResult mp_term_ime_unmark(const MpTerm *_Nonnull t);
MpImeResult mp_term_ime_replace(const MpTerm *_Nonnull t, uint32_t start, uint32_t end, const char *_Nonnull text);
/// The view moved the caret itself, sending its own arrows (the space
/// bar's trackpad).
void mp_term_ime_select(const MpTerm *_Nonnull t, uint32_t start, uint32_t end);
/// The input system set its caret or selection (UIKit's
/// `selectedTextRange`): a caret it moved goes to the remote as arrows,
/// within the line typed; a selection or a composition's caret sends
/// nothing (B695).
MpImeResult mp_term_ime_input_select(const MpTerm *_Nonnull t, uint32_t start, uint32_t end);
/// The event loop finished the turn of a delete: a Return now is the
/// person's (B410).
void mp_term_ime_turn_end(const MpTerm *_Nonnull t);
MpImeState mp_term_ime_state(const MpTerm *_Nonnull t);
/// Free with mp_string_free.
char *_Nullable mp_term_ime_text(const MpTerm *_Nonnull t, uint32_t start, uint32_t end);

// MARK: The scroll model (V19–V22)

typedef struct {
    uint8_t kind;     ///< 0 nothing, 1 the history moved, 2 wheel reports, 3 rows for the host
    int32_t moved_px; ///< kind 1
    int32_t rows;     ///< kind 3
    MpOwnedBytes bytes; ///< kind 2: send, then mp_bytes_free
} MpScrollAction;
/// A drag of `dy` device pixels (positive: older); `host_rows`: the session
/// scrolls its own buffer (tmux, herdr).
MpScrollAction mp_term_scroll_drag(const MpTerm *_Nonnull t, double dy, bool host_rows);
MpScrollAction mp_term_scroll_end(const MpTerm *_Nonnull t);
/// A drag's vertical move, `dy` device pixels (positive: older), wherever it
/// went: the lift's speed is read from these (B521).
void mp_term_fling_track(const MpTerm *_Nonnull t, double dy);
/// The drag's lift: true if it coasts; `elsewhere`: the drag went to the
/// host (tmux, herdr). There, and for a program that wants the mouse, the
/// coast is paced and bounded.
bool mp_term_fling_start(const MpTerm *_Nonnull t, bool elsewhere);
/// One display refresh of a coast, `dt` seconds on: true while it coasts;
/// `dy` gets the pixels to route now, as a drag's (0: none this refresh).
bool mp_term_fling_step(const MpTerm *_Nonnull t, double dt, double *_Nullable dy);
/// A touch: true if it stopped a coast (that touch is then not a tap).
bool mp_term_fling_stop(const MpTerm *_Nonnull t);

// MARK: Accessibility (V33)

/// What a screen reader hears now (free with mp_string_free; NULL:
/// nothing), paced: a second of quiet after each announcement, what appears
/// in it said at its end. `again_ms` (may be NULL) gets when to ask again,
/// 0 for no need (B427).
char *_Nullable mp_term_a11y_announce(const MpTerm *_Nonnull t, uint32_t *_Nullable again_ms);

// MARK: mosh's predictive echo, laid over the frames (never into the grid)

/// `mode` 0 off, 1 adaptive, 2 always, 3 experimental.
void mp_term_predict_attach(const MpTerm *_Nonnull t, uint8_t mode, bool trail);
void mp_term_predict_srtt(const MpTerm *_Nonnull t, double ms);
/// Keystrokes on their way: the batch for mp_term_predict_sent; 0: none.
uint64_t mp_term_predict_input(const MpTerm *_Nonnull t, const uint8_t *_Nullable bytes, size_t len);
void mp_term_predict_sent(const MpTerm *_Nonnull t, uint64_t batch, uint64_t num, bool queued);
/// A frame's echo ack, judged after the bytes pushed before it.
void mp_term_predict_echo_ack(const MpTerm *_Nonnull t, uint64_t ack);
void mp_term_predict_reset(const MpTerm *_Nonnull t);
bool mp_term_predict_needs_tick(const MpTerm *_Nonnull t);
/// True when what shows changed: take a frame.
bool mp_term_predict_tick(const MpTerm *_Nonnull t);

#ifdef __cplusplus
}
#endif
