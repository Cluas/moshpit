//! The shared terminal core (docs/nsh/TERMINAL-CORE.md): one emulator for
//! iOS, Android and the Mac, with drawing, input methods, gestures and
//! accessibility left to each platform's native view.
//!
//! [`Terminal`] is our fork of wezterm-term (vendor/wezterm, MIT; our
//! patches in docs/WEZTERM-PATCHES.md) behind Moshpit's own API, so the
//! rest of the app never names wezterm's types:
//!
//! - **feed** the remote's bytes in any cut ([`Terminal::feed`]); the
//!   replies the terminal owes the remote (DA, CPR, colours, …) come back
//!   from [`Terminal::take_replies`], in order and synchronously (the
//!   emulator's writer thread is off), what the host must act on (title,
//!   bell, OSC 52) from [`Terminal::take_events`];
//! - **draw** from a [`Frame`]: the rows that changed since the last one,
//!   each cell's text, final colours and flags, the images on it
//!   (kitty graphics, sixel, iTerm2), the cursor ([`Terminal::frame`]); a
//!   copy, so the view never holds a lock while it draws;
//! - **history** ([`Terminal::scroll`]), **selection**
//!   ([`Terminal::select_start`]), **search** ([`Terminal::search`]),
//!   **OSC 8 links** ([`Terminal::link_at`]), the **modes** the host's input
//!   follows ([`Terminal::modes`]), the visible text for the core's link
//!   detection ([`Terminal::visible_text`]).
//!
//! What this layer adds to the emulator: synchronized output (mode 2026,
//! which wezterm leaves to its multiplexer) held here until its end or
//! its deadline; OSC 52 clipboard queries (wezterm answers none); the
//! viewport and its scrolling; the selection and search models (wezterm
//! keeps them in its GUI), all on wezterm's stable row numbers.
//!
//! [`pane`] feeds a terminal from the byte channel (crates/stream) off the
//! main thread; [`ffi`] is the C ABI the native views call.
//!
//! **Marks** ([`Terminal::note_mark`]): what the view must do at a point in
//! the output (a pane switch's veil, its reveal) rides in the byte channel
//! among the bytes; the parse reaches it and the next [`Frame`] carries it
//! (`Frame::marks`), so the view acts on it exactly when the frame with the
//! bytes before it is drawn, never before (a reveal over a repaint not yet
//! parsed) and never after a frame with the bytes after it (a stale frame
//! shown before its veil). A frame and its marks are taken under one lock,
//! which is what makes that hold.

use std::{
    collections::HashMap,
    ops::Range,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use base64::Engine as _;
use moshpit_stream::Mark;
use wezterm_escape_parser::{
    Action, CSI, Esc, EscCode, OperatingSystemCommand,
    csi::{DecPrivateMode, DecPrivateModeCode, Device, Edit, EraseInDisplay, Mode},
    osc::{FinalTermSemanticPrompt, Selection as OscSelection},
    parser::Parser,
};
use wezterm_surface::{CursorShape, CursorVisibility, SequenceNo};
use wezterm_term::{
    Alert, AlertHandler, CellAttributes, Clipboard, ClipboardSelection, Intensity, Line,
    MouseEncoding, StableRowIndex, TerminalConfiguration, TerminalSize, Underline, UnicodeVersion,
    color::{ColorAttribute, ColorPalette},
};

pub mod ffi;
pub mod history_images;
mod images;
pub mod ime;
pub mod input;
pub mod links;
mod palette;
pub mod pane;
pub mod paths;
pub mod predict;
pub mod scroll;
mod select;

pub use images::{DecodedImage, ImageCache};
pub use ime::{Ime, Typed};
pub use input::Key;
pub use palette::{Palette, Rgb};
pub use scroll::{Momentum, ScrollAction};

/// How a terminal is made.
#[derive(Debug, Clone)]
pub struct Options {
    /// Lines of history kept (the app's default is 10 000).
    pub scrollback: usize,
    /// A cell's size in pixels: the size reports programs ask for, and how
    /// many cells an image covers.
    pub cell_width_px: u16,
    pub cell_height_px: u16,
    pub palette: Palette,
    /// Bold text in one of the 8 basic colours draws in its bright twin
    /// (the app's "bright colours" setting, the fork's patch 30).
    pub bold_is_bright: bool,
    /// The most RAM one decoded image may take; larger ones are ignored.
    pub max_image_bytes: usize,
    /// The most RAM the kitty images a program stored may take before the
    /// unplaced ones are let go (a terminal's own: it is protocol state).
    pub kitty_image_storage: usize,
    /// The most RAM the images of the rows scrolled into the history may
    /// keep; past it the oldest rows' images are let go (B733).
    pub history_image_bytes: usize,
    /// The decoded images kept for drawing: `None`, the process's, shared
    /// by every terminal ([`ImageCache::shared`]).
    pub image_cache: Option<ImageCache>,
    /// What a clear of the main screen keeps.
    pub scroll_on_clear: ScrollOnClear,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            scrollback: 10_000,
            cell_width_px: 8,
            cell_height_px: 16,
            palette: Palette::default(),
            bold_is_bright: true,
            // A phone: 4096 × 4096 RGBA is 64 MB.
            max_image_bytes: 32 << 20,
            // A terminal's own; six panes' together are what a phone's
            // background budget has to hold (TERMINAL-VIEW.md §5.8).
            kitty_image_storage: 32 << 20,
            // A terminal's own, as the kitty storage: what six panes'
            // histories may keep together on a phone.
            history_image_bytes: 32 << 20,
            image_cache: None,
            scroll_on_clear: ScrollOnClear::EraseDisplay,
        }
    }
}

impl Drop for Terminal {
    /// Its history's images leave the process's ledger with it (B733).
    fn drop(&mut self) {
        history_images::unregister(self.image_ledger_id);
    }
}

/// What the host acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The window title (OSC 0/2, screen's `ESC k`); empty: reset.
    Title(String),
    Bell,
    /// OSC 52: put this text on the clipboard.
    ClipboardStore(String),
    /// OSC 52: the remote asks for the clipboard; answer with
    /// [`Terminal::answer_clipboard`] (or not at all).
    ClipboardLoad,
    /// OSC 7: the program's working directory, as the URL it reported
    /// (`file://host/path`); empty when it reported none it could parse.
    Directory(String),
}

/// What a clear of the main screen keeps: what it showed moves into the
/// history first (`scroll-on-clear`), on which clears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollOnClear {
    /// `ESC [2J` (tmux's and alacritty's way, for a terminal of its own:
    /// plain SSH, mosh, herdr).
    #[default]
    EraseDisplay,
    /// A tmux pane: what tmux 3.6a's `scroll-on-clear` (on, its default)
    /// keeps, so scrolling back after `clear` shows what tmux's own history
    /// has: `ESC [2J`, and `ESC [J` from the top left, the `clear` a tmux
    /// pane's terminfo sends.
    Tmux,
}

/// The emulator's configuration: ours, read by wezterm-term.
#[derive(Debug)]
struct Config {
    scrollback: usize,
    palette: ColorPalette,
    max_image_bytes: usize,
    kitty_image_storage: usize,
    generation: usize,
    /// [`ScrollOnClear::Tmux`].
    tmux_clears: bool,
}

impl TerminalConfiguration for Config {
    fn generation(&self) -> usize {
        self.generation
    }
    fn scrollback_size(&self) -> usize {
        self.scrollback
    }
    fn color_palette(&self) -> ColorPalette {
        self.palette.clone()
    }
    fn enable_kitty_keyboard(&self) -> bool {
        true
    }
    fn enable_kitty_graphics(&self) -> bool {
        true
    }
    // What the remote's wcwidth and tmux use today (wezterm's default is 9,
    // for the emoji presentation of old systems).
    fn unicode_version(&self) -> UnicodeVersion {
        UnicodeVersion {
            version: 14,
            ambiguous_are_wide: false,
            cell_widths: None,
        }
    }
    // Our fork's switches (docs/WEZTERM-PATCHES.md).
    fn writer_thread(&self) -> bool {
        false
    }
    fn host_owns_columns(&self) -> bool {
        true
    }
    fn scroll_on_clear(&self) -> bool {
        true
    }
    fn scroll_on_clear_from_home(&self) -> bool {
        self.tmux_clears
    }
    fn max_image_bytes(&self) -> usize {
        self.max_image_bytes
    }
    fn kitty_image_storage_limit(&self) -> usize {
        self.kitty_image_storage
    }
}

/// What the emulator hands its embedder, gathered as it happens.
#[derive(Default)]
struct Inbox {
    replies: Vec<u8>,
    events: Vec<Event>,
    /// The title set last, until the terminal takes it.
    title: Option<String>,
    /// OSC 7 events whose directory the terminal has yet to fill in (the
    /// emulator's alert names none: the terminal reads it after the batch).
    directories_unread: usize,
}

/// Events kept for a host that does not take them (a bell storm).
const EVENTS_MAX: usize = 256;

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Inbox>>);

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inbox> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Inbox {
    /// An event for the host, the oldest let go past [`EVENTS_MAX`] (a host
    /// that takes none: the app in the background).
    fn push(&mut self, event: Event) {
        if self.events.len() >= EVENTS_MAX {
            self.events.remove(0);
        }
        self.events.push(event);
    }
}

impl std::io::Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.lock().replies.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl AlertHandler for Shared {
    fn alert(&mut self, alert: Alert) {
        let mut inbox = self.lock();
        let event = match alert {
            Alert::Bell => Event::Bell,
            Alert::WindowTitleChanged(t) => {
                inbox.title = Some(t.clone());
                Event::Title(t)
            }
            Alert::CurrentWorkingDirectoryChanged => {
                inbox.directories_unread += 1;
                Event::Directory(String::new())
            }
            _ => return,
        };
        inbox.push(event);
    }
}

impl Clipboard for Shared {
    fn set_contents(&self, _: ClipboardSelection, data: Option<String>) -> anyhow::Result<()> {
        // Bounded as the others are (B481): each holds its whole text.
        if let Some(text) = data {
            self.lock().push(Event::ClipboardStore(text));
        }
        Ok(())
    }
}

/// Cell flags in a [`DrawCell`].
pub mod cell_flags {
    pub const BOLD: u16 = 1;
    pub const ITALIC: u16 = 1 << 1;
    pub const UNDERLINE: u16 = 1 << 2;
    pub const DOUBLE_UNDERLINE: u16 = 1 << 3;
    pub const UNDERCURL: u16 = 1 << 4;
    pub const DOTTED_UNDERLINE: u16 = 1 << 5;
    pub const DASHED_UNDERLINE: u16 = 1 << 6;
    pub const STRIKEOUT: u16 = 1 << 7;
    pub const DIM: u16 = 1 << 8;
    /// The first of a two-column character.
    pub const WIDE: u16 = 1 << 9;
    /// The second column of a wide character: draw its background only.
    pub const SPACER: u16 = 1 << 10;
    pub const SELECTED: u16 = 1 << 11;
    pub const MATCH: u16 = 1 << 12;
    /// Part of an OSC 8 link (`DrawCell::link`).
    pub const LINK: u16 = 1 << 13;
}

/// One cell, ready to draw: colours resolved (palette, inverse, dim,
/// hidden, bold-is-bright), text as a first scalar plus the scalars joined
/// to it (`extra` of them, from the row's `extra` list at `extra_at`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrawCell {
    pub ch: u32,
    pub extra_at: u32,
    pub extra: u16,
    pub flags: u16,
    /// 0xRRGGBB.
    pub fg: u32,
    pub bg: u32,
    /// Underline colour, or `u32::MAX` for the foreground's.
    pub underline: u32,
    /// 1-based index into [`Frame::links`]; 0: no link.
    pub link: u32,
}

/// A run of cells of one row covered by one image: draw the image's
/// texture rectangle (fractions of the image) over the cells' rectangle.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ImageRun {
    pub col: u16,
    pub cols: u16,
    /// 1-based index into [`Frame::images`].
    pub image: u32,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    /// Below the text when negative (kitty's z-index).
    pub z: i32,
}

/// One row of a [`Frame`].
#[derive(Debug, Clone, Default)]
pub struct FrameRow {
    /// Row on the screen as shown (0: the top of the viewport).
    pub row: u16,
    /// Continues on the next row (soft wrap).
    pub wrapped: bool,
    pub cells: Vec<DrawCell>,
    /// Joined scalars of the cells (combining marks, ZWJ sequences, …).
    pub extra: Vec<u32>,
    /// Images over the row's cells, in drawing order.
    pub images: Vec<ImageRun>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CursorView {
    pub col: u16,
    pub row: u16,
    /// 0 block, 1 underline, 2 beam, 3 hollow block, 4 hidden.
    pub shape: u8,
    pub blinking: bool,
    /// On screen (not scrolled away, not hidden by DECTCEM).
    pub visible: bool,
}

/// What changed since the last frame, as copies.
#[derive(Debug, Clone, Default)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    /// Every row is in `changed` (resize, a full repaint).
    pub full: bool,
    /// The rows the last frame drew moved up this many rows (negative:
    /// down) before `changed` applies: the view scrolled or output scrolled
    /// the screen under it. Only the uncovered rows and the rows whose text
    /// changed are in `changed`.
    pub shift: i32,
    pub changed: Vec<FrameRow>,
    pub cursor: CursorView,
    /// Lines the view is scrolled back into the history.
    pub display_offset: u32,
    /// Pixels the rows are drawn up by: the part of a row a pixel scroll
    /// leaves above the view (the frame then carries the row under it).
    pub pixel_shift: u32,
    pub history: u32,
    pub links: Vec<String>,
    /// Default background, for what lies outside the grid.
    pub background: u32,
    /// The decoded images the rows' [`ImageRun`]s point at.
    pub images: Vec<DecodedImage>,
    /// Marks the parse reached since the last frame, in order: every byte
    /// before each is in this frame (or an earlier one), and no byte after
    /// the last is in an earlier one.
    pub marks: Vec<Mark>,
}

/// Modes the host's input follows.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modes {
    /// 0 off, 1 clicks (1000), 2 drags too (1002), 3 all motion (1003).
    pub mouse: u8,
    pub mouse_sgr: bool,
    pub mouse_utf8: bool,
    pub bracketed_paste: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub focus_reports: bool,
    pub alt_screen: bool,
    /// Wheel over the alternate screen sends arrow keys (1007).
    pub alternate_scroll: bool,
    /// kitty keyboard protocol flags in force (`CSI > n u`).
    pub kitty_flags: u8,
    pub cursor_visible: bool,
}

/// What a selection grows by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectKind {
    /// Cell by cell (a drag).
    Cells,
    /// Whole words (a double tap, a long press).
    Word,
    /// Whole lines (a triple tap).
    Line,
    Block,
    /// The link under the point (OSC 8 or a detected URL, wrapped rows
    /// included), else its word: a long press.
    Semantic,
}

/// What an input-method call sends: typed bytes, then a paste through the
/// platform's paste path; whether the platform tells its input delegate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImeSend {
    pub bytes: Vec<u8>,
    pub notify: bool,
    pub paste: Option<String>,
}

/// What a tap means (docs/nsh/TERMINAL-VIEW.md V12–V14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tap {
    /// A selection was showing: the tap only dismissed it (patch 20).
    Dismissed,
    /// A link: open it, without taking the keyboard (patches 4, 13).
    Link(String),
    /// A tap on the screen: a click for a program that wants the mouse,
    /// and the keyboard if it landed on the cursor's own rows while the
    /// live screen shows (patch 15, `tapWantsKeyboard`).
    Click { keyboard: bool },
}

/// How far, in rows, a tap from the cursor still means "I want to type".
pub const TYPING_ROWS: i64 = 2;

/// The quiet after a VoiceOver announcement (V33): an agent streaming
/// output would otherwise talk over everything.
pub const ANNOUNCE_EVERY_MS: u64 = 1_000;

/// What VoiceOver hears now ([`Terminal::announce_at`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Announcement {
    pub text: Option<String>,
    /// Ask again in this many milliseconds: what appeared in the quiet is
    /// said then.
    pub again_in_ms: Option<u64>,
}

/// A search match in viewport coordinates (after the view scrolled to it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub start: (u16, u16),
    pub end: (u16, u16),
}

/// How [`Terminal::find`] matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub case_sensitive: bool,
    /// A match starts a word.
    pub word_start: bool,
    /// A match ends a word (with `word_start`: whole words).
    pub word_end: bool,
}

/// Which match [`Terminal::find`] shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindStep {
    /// The text as typed: the match shown if it still matches, else the
    /// newest.
    Newest,
    /// The next newer one, round to the oldest.
    Next,
    /// The next older one, round to the newest.
    Previous,
}

/// The match [`Terminal::find`] shows, and where it is among all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    pub matched: Match,
    /// 0-based, oldest first.
    pub index: usize,
    pub count: usize,
}

/// The cells an OSC 8 link covers: (viewport row, first col, last col)
/// runs, rows above the viewport negative.
pub type LinkRuns = Vec<(i32, u16, u16)>;

/// A link's cells on one row: (stable row, first col, last col).
type SpanRun = (StableRowIndex, usize, usize);

/// How many rows past a hard break a link may go on (a URL five times the
/// width; the runaway stop).
const MAX_LINK_ROWS: usize = 4;

/// Where a point stands in what a reflow keeps ([`Terminal::text_place`]).
#[derive(Debug, Clone, Copy)]
struct TextPlace {
    /// Its logical line, counted back from the last one with text.
    lines_back: usize,
    /// The cell of the line's text it is in (its length: past the text).
    cell: usize,
    /// The column within that cell, or past the text the columns past it.
    sub: usize,
}

/// A search's matches, (first cell, last cell), and the moment they were
/// found: the sequence number, the first row kept, the size.
type FoundAll = (
    (SequenceNo, StableRowIndex, (u16, u16)),
    Vec<(select::Point, select::Point)>,
);

/// One cell as the link rules read it.
#[derive(Debug, Clone)]
struct Cell {
    row: StableRowIndex,
    col: usize,
    width: usize,
    /// No link and no text.
    blank: bool,
    /// The URI of the OSC 8 link it carries.
    uri: Option<String>,
}

/// Cells in reading order as runs, one per row.
fn cell_runs(cells: &[Cell]) -> Vec<SpanRun> {
    let mut runs: Vec<SpanRun> = Vec::new();
    for x in cells {
        let end = x.col + x.width - 1;
        match runs.last_mut() {
            Some(run) if run.0 == x.row => run.2 = end,
            _ => runs.push((x.row, x.col, end)),
        }
    }
    runs
}

/// What a prompt mark (OSC 133, the shell's integration) turns off that a
/// program which died left on (B579): it never sent its own resets, and the
/// shell got its leftovers — a tap typed `ESC[<0;12;5M` at the prompt, a
/// focus change `ESC[I`, Ctrl-C arrived as `ESC[99;5u`. Every mouse report
/// mode and its encodings and focus reports at the prompt's start (A) or a
/// command's end (D); the kitty keyboard flags only at D, a command's end:
/// a shell that uses them itself (fish) pushes its own while reading input,
/// and pops them before it runs a command, so only a program's are left
/// then. Bracketed paste stays: shells turn it on themselves.
const PROMPT_START_RESETS: &[u8] =
    b"\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?1016l\x1b[?1004l";
const COMMAND_END_RESETS: &[u8] =
    b"\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?1016l\x1b[?1004l\x1b[<999u";

/// The resets a prompt mark brings, if `action` is one ([`PROMPT_START_RESETS`]).
fn prompt_resets(action: &Action) -> Option<&'static [u8]> {
    match action {
        Action::OperatingSystemCommand(osc) => match &**osc {
            OperatingSystemCommand::FinalTermSemanticPrompt(
                FinalTermSemanticPrompt::FreshLineAndStartPrompt { .. },
            ) => Some(PROMPT_START_RESETS),
            OperatingSystemCommand::FinalTermSemanticPrompt(
                FinalTermSemanticPrompt::CommandStatus { .. },
            ) => Some(COMMAND_END_RESETS),
            _ => None,
        },
        _ => None,
    }
}

/// How long a synchronized update may hold the screen (vte's and
/// alacritty's 150 ms).
const SYNC_TIMEOUT: Duration = Duration::from_millis(150);
/// The most actions a synchronized update may hold before it is drawn
/// anyway (about 2 MB of text).
const SYNC_MAX_ACTIONS: usize = 2 << 20;

pub struct Terminal {
    term: wezterm_term::Terminal,
    parser: Parser,
    shared: Shared,
    config: Arc<Config>,
    options: Options,
    title: String,
    /// Actions of a synchronized update, held until its end or deadline.
    held: Vec<Action>,
    holding_since: Option<Instant>,
    /// An image arrived since the history's were last weighed (B733).
    images_arrived: bool,
    /// This terminal's place in the process's ledger of history images,
    /// and what the ledger asks of it (B733).
    image_ledger_id: u64,
    image_share: Arc<history_images::Share>,
    /// The OSC 52 query being answered (its selection).
    clipboard_query: Option<OscSelection>,
    /// The viewport's top row when scrolled back; `None`: it follows the
    /// bottom.
    view_top: Option<StableRowIndex>,
    selection: Option<select::Selection>,
    /// The pattern, its regex, and whether a match must start and end a
    /// word (find's options, B730).
    search: Option<(String, regex::Regex, (bool, bool))>,
    /// The search's matches and when they were found: the terminal's
    /// sequence number, its first row and its size then (B734).
    found_all: Option<FoundAll>,
    found: Option<(StableRowIndex, usize, StableRowIndex, usize)>,
    /// The found match's rows as they read when it was shown, and when:
    /// the mark stays only on its text (B520).
    found_rows: Vec<(StableRowIndex, (String, bool))>,
    found_seqno: SequenceNo,
    // What the last frame showed.
    frame_seqno: SequenceNo,
    frame_top: Option<StableRowIndex>,
    frame_alt: bool,
    frame_cursor: Option<(u16, u16)>,
    /// Something the emulator does not track changed (the selection, a
    /// search match, the palette, the size).
    full_damage: bool,
    images: ImageCache,
    /// Rows below the viewport the frames carry too (a view scrolling by
    /// pixels shows part of the row under its last one).
    extra_rows: u16,
    /// The shadow document of the platform's text input.
    pub ime: Ime,
    scroll: scroll::Scroll,
    /// The drag's recent moves, and the coast after its lift (B521).
    velocity: scroll::Velocity,
    fling: Option<scroll::Fling>,
    /// The speed of the coast a touch stopped, and when: a quick flick the
    /// same way adds it (B523).
    carried: Option<(f64, u64)>,
    /// The view's top row the scroll model set last (a different one: the
    /// core moved the view itself).
    scroll_top: Option<StableRowIndex>,
    /// The rows VoiceOver was told of last.
    a11y_last: Vec<String>,
    /// The cursor's row VoiceOver's last look saw.
    a11y_last_cursor: usize,
    /// Until when (the terminal's clock, ms) VoiceOver hears nothing new:
    /// the quiet after an announcement ([`Terminal::announce_at`]).
    a11y_quiet_until: Option<u64>,
    /// mosh's predictive echo, laid over the frames (never into the grid).
    prediction: Option<predict::Prediction>,
    /// Screen rows the last frame's predictions covered (the next frame
    /// redraws them).
    predicted_rows: Vec<u16>,
    /// Marks reached, for the next frame ([`Terminal::note_mark`]).
    marks: Vec<Mark>,
    /// Paths as links ([`Terminal::set_path_rules`]): what the platform
    /// can preview, and the candidates the host confirmed — (folder only,
    /// token) to the URI a tap opens.
    path_rules: Option<paths::Rules>,
    path_links: HashMap<(bool, String), String>,
    /// Marks reached inside a synchronized update: they follow its bytes.
    held_marks: Vec<Mark>,
    born: Instant,
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminal")
            .field("size", &self.size())
            .finish_non_exhaustive()
    }
}

fn term_size(cols: u16, rows: u16, o: &Options) -> TerminalSize {
    let (cols, rows) = (cols.max(1) as usize, rows.max(1) as usize);
    TerminalSize {
        rows,
        cols,
        pixel_width: cols * o.cell_width_px as usize,
        pixel_height: rows * o.cell_height_px as usize,
        dpi: 0,
    }
}

fn rgb_u32(c: Rgb) -> u32 {
    (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32
}

impl Terminal {
    pub fn new(cols: u16, rows: u16, options: Options) -> Self {
        let (image_ledger_id, image_share) = history_images::register();
        let shared = Shared::default();
        let config = Arc::new(Config {
            scrollback: options.scrollback,
            palette: options.palette.to_wezterm(),
            max_image_bytes: options.max_image_bytes,
            kitty_image_storage: options.kitty_image_storage,
            generation: 0,
            tmux_clears: options.scroll_on_clear == ScrollOnClear::Tmux,
        });
        let mut term = wezterm_term::Terminal::new(
            term_size(cols, rows, &options),
            config.clone(),
            "Moshpit",
            env!("CARGO_PKG_VERSION"),
            Box::new(shared.clone()),
        );
        term.set_notification_handler(Box::new(shared.clone()));
        let clipboard: Arc<dyn Clipboard> = Arc::new(shared.clone());
        term.set_clipboard(&clipboard);
        Self {
            term,
            parser: Parser::new(),
            shared,
            config,
            images: options
                .image_cache
                .clone()
                .unwrap_or_else(ImageCache::shared),
            options,
            title: String::new(),
            held: Vec::new(),
            holding_since: None,
            images_arrived: false,
            image_ledger_id,
            image_share,
            clipboard_query: None,
            view_top: None,
            selection: None,
            search: None,
            found_all: None,
            found: None,
            found_rows: Vec::new(),
            found_seqno: 0,
            frame_seqno: 0,
            frame_top: None,
            frame_alt: false,
            frame_cursor: None,
            full_damage: true,
            extra_rows: 0,
            ime: Ime::new(),
            scroll: scroll::Scroll::default(),
            velocity: scroll::Velocity::default(),
            fling: None,
            carried: None,
            scroll_top: None,
            a11y_last: Vec::new(),
            a11y_last_cursor: 0,
            a11y_quiet_until: None,
            prediction: None,
            predicted_rows: Vec::new(),
            marks: Vec::new(),
            held_marks: Vec::new(),
            path_rules: None,
            path_links: HashMap::new(),
            born: Instant::now(),
        }
    }

    /// The parse reached `mark` (crates/stream's, among the bytes): the
    /// next frame carries it, after the bytes before it. Inside a
    /// synchronized update it waits for the update's bytes, which the
    /// frames do not show before its end either.
    pub fn note_mark(&mut self, mark: Mark) {
        if self.holding_since.is_some() {
            self.held_marks.push(mark);
        } else {
            self.marks.push(mark);
        }
    }

    /// A synchronized update ended: the marks it held follow its bytes.
    fn release_held_marks(&mut self) {
        if self.holding_since.is_none() && !self.held_marks.is_empty() {
            self.marks.append(&mut self.held_marks);
        }
    }

    /// Takes the remote's bytes, cut anywhere.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.shed_history_images();
        let mut actions = Vec::new();
        self.parser.parse(bytes, |a| actions.push(a));
        let mut batch = Vec::with_capacity(actions.len());
        for action in actions {
            self.route(action, &mut batch);
        }
        self.perform(batch);
        if self.holding_since.is_some() && self.held.len() > SYNC_MAX_ACTIONS {
            self.flush_sync();
        }
        self.release_held_marks();
        self.predict_settle();
        if std::mem::take(&mut self.images_arrived) {
            self.trim_history_images();
        }
    }

    /// Lets the images of the oldest rows in the history go, row by row,
    /// while the history's take more than [`Options::history_image_bytes`]
    /// (B733); their text stays. Images were kept as long as their rows
    /// were: 100 sixel plots of 800×600 kept 242 MB, kitty images placed
    /// and scrolled away were never the kitty storage's to let go. Weighed
    /// when an image arrives (an image on the screen is not the
    /// history's, so the history keeps at most the budget and what the
    /// screen showed then).
    fn trim_history_images(&mut self) {
        let kept = self.weigh_history_images(self.options.history_image_bytes);
        let (own, trims) = history_images::weighed(self.image_ledger_id, kept);
        if let Some(own) = own {
            let kept = self.weigh_history_images(own);
            history_images::settle(self.image_ledger_id, kept);
        }
        // Other terminals over the process's total let theirs go now if
        // they are free (only trying their locks: never a wait on another
        // terminal while this one's is held), else at their next bytes or
        // frame.
        for trim in trims {
            trim();
        }
    }

    /// What the process's ledger asked of this terminal's history images,
    /// done (B733).
    pub(crate) fn shed_history_images(&mut self) {
        use std::sync::atomic::Ordering;
        if self.image_share.due.swap(false, Ordering::AcqRel) {
            let target = self.image_share.target.load(Ordering::Relaxed);
            let kept = self.weigh_history_images(target);
            history_images::settle(self.image_ledger_id, kept);
        }
    }

    /// This terminal's place in the process's ledger of history images.
    pub(crate) fn image_ledger_id(&self) -> u64 {
        self.image_ledger_id
    }

    /// Lets the history's images go, oldest rows first, until they take
    /// at most `budget`; what they take then.
    fn weigh_history_images(&mut self, budget: usize) -> usize {
        use std::collections::HashSet;
        use wezterm_term::image::ImageData;
        let seqno = self.term.current_seqno();
        let screen = self.term.screen_mut();
        let lines = screen.scrollback_rows();
        let history = lines.saturating_sub(screen.physical_rows);
        if history == 0 {
            return 0;
        }
        // Each image the history shows: its bytes, and how many of its
        // rows show it; the history's rows with images, oldest first.
        let mut held: HashMap<*const ImageData, (usize, usize)> = HashMap::new();
        let mut shown: HashSet<*const ImageData> = HashSet::new();
        let mut rows: Vec<(usize, Vec<*const ImageData>)> = Vec::new();
        screen.with_phys_lines(0..lines, |all| {
            for (phys, line) in all.iter().enumerate() {
                let mut on_row: Vec<*const ImageData> = Vec::new();
                for cell in line.visible_cells() {
                    for image in cell.attrs().images().unwrap_or_default() {
                        let data = image.image_data();
                        let key = Arc::as_ptr(data);
                        if on_row.contains(&key) {
                            continue;
                        }
                        on_row.push(key);
                        if phys < history {
                            held.entry(key).or_insert((data.len(), 0)).1 += 1;
                        } else {
                            shown.insert(key);
                        }
                    }
                }
                if phys < history && !on_row.is_empty() {
                    rows.push((phys, on_row));
                }
            }
        });
        let mut total: usize = held
            .iter()
            .filter(|(k, _)| !shown.contains(*k))
            .map(|(_, (len, _))| len)
            .sum();
        let mut trimmed = false;
        let mut gone: HashSet<*const ImageData> = HashSet::new();
        for (phys, keys) in rows {
            if total <= budget {
                break;
            }
            trimmed = true;
            let line = screen.line_mut(phys);
            for cell in line.cells_mut_for_attr_changes_only() {
                cell.attrs_mut().clear_images();
            }
            // Back to the compact storage the history keeps its rows in
            // (clearing took the row out of it: seven times the bytes).
            line.compress_for_scrollback();
            line.update_last_change_seqno(seqno);
            for key in keys {
                if let Some((len, count)) = held.get_mut(&key) {
                    *count -= 1;
                    if *count == 0 && !shown.contains(&key) {
                        total -= *len;
                        gone.insert(key);
                    }
                }
            }
        }
        if trimmed {
            // The emulator keeps the last 16 images so an identical one
            // shares their data, whatever their size: those let go here
            // stayed in memory through it (W20). And a kitty image's
            // placement stays on record after its rows let it go: the
            // store counted it placed and kept it (W27).
            self.term.release_image_dedup();
            self.term.kitty_drop_data(&gone);
        }
        total
    }

    /// Sorts one action: held (inside a synchronized update), handled here,
    /// or into `batch` for the emulator.
    fn route(&mut self, action: Action, batch: &mut Vec<Action>) {
        if matches!(&action, Action::Sixel(_) | Action::KittyImage(_))
            || matches!(&action, Action::OperatingSystemCommand(osc)
                if matches!(**osc, OperatingSystemCommand::ITermProprietary(_)))
        {
            self.images_arrived = true;
        }
        match &action {
            Action::CSI(CSI::Mode(Mode::SetDecPrivateMode(DecPrivateMode::Code(
                DecPrivateModeCode::SynchronizedOutput,
            )))) => {
                // The update starts: what came before draws now.
                let earlier = std::mem::take(batch);
                self.perform(earlier);
                if self.holding_since.is_none() {
                    self.holding_since = Some(Instant::now());
                }
                return;
            }
            Action::CSI(CSI::Mode(Mode::ResetDecPrivateMode(DecPrivateMode::Code(
                DecPrivateModeCode::SynchronizedOutput,
            )))) => {
                self.holding_since = None;
                batch.append(&mut self.held);
                return;
            }
            Action::CSI(CSI::Device(d)) if matches!(**d, Device::SoftReset) => {
                self.holding_since = None;
                batch.append(&mut self.held);
            }
            Action::OperatingSystemCommand(osc) => {
                if let OperatingSystemCommand::QuerySelection(sel) = **osc {
                    // wezterm answers no OSC 52 query; the host may.
                    let earlier = std::mem::take(batch);
                    self.perform(earlier);
                    self.clipboard_query = Some(sel);
                    self.shared.lock().push(Event::ClipboardLoad);
                    return;
                }
            }
            _ => {}
        }
        // What ends a selection: a full reset, the other screen.
        let drops_selection = matches!(
            &action,
            Action::Esc(Esc::Code(EscCode::FullReset))
                | Action::CSI(CSI::Mode(
                    Mode::SetDecPrivateMode(DecPrivateMode::Code(
                        DecPrivateModeCode::ClearAndEnableAlternateScreen
                            | DecPrivateModeCode::EnableAlternateScreen
                            | DecPrivateModeCode::OptEnableAlternateScreen
                    )) | Mode::ResetDecPrivateMode(DecPrivateMode::Code(
                        DecPrivateModeCode::ClearAndEnableAlternateScreen
                            | DecPrivateModeCode::EnableAlternateScreen
                            | DecPrivateModeCode::OptEnableAlternateScreen
                    ))
                ))
        );
        if drops_selection {
            self.selection = None;
            self.full_damage = true;
            if matches!(&action, Action::Esc(Esc::Code(EscCode::FullReset))) {
                self.view_top = None;
            }
        }
        // ED 3 clears the history the view may be reading (`clear`, Claude
        // Code's repaint, tmux passing either on): the view goes live, and
        // follows what comes next (B424). Left on its row, it fell below the
        // new history's start and stayed there as output went on under it.
        // The alternate screen's ED 3 leaves the main screen's history (the
        // batch before it is applied first, so its screen is the one now).
        if self.view_top.is_some()
            && matches!(
                &action,
                Action::CSI(CSI::Edit(Edit::EraseInDisplay(
                    EraseInDisplay::EraseScrollback
                )))
            )
        {
            if self.holding_since.is_none() {
                let earlier = std::mem::take(batch);
                self.perform(earlier);
            }
            if !self.term.is_alt_screen_active() {
                self.scroll_to_bottom();
                self.full_damage = true;
            }
        }
        let resets = prompt_resets(&action);
        if self.holding_since.is_some() {
            self.held.push(action);
        } else {
            batch.push(action);
        }
        // The shell's prompt is back (B579): what a program that died left on
        // goes, after the mark, in the same place.
        if let Some(resets) = resets {
            let target = if self.holding_since.is_some() {
                &mut self.held
            } else {
                batch
            };
            target.extend(Parser::new().parse_as_vec(resets));
        }
    }

    fn perform(&mut self, actions: Vec<Action>) {
        if actions.is_empty() {
            return;
        }
        self.term.perform_actions(actions);
        let mut inbox = self.shared.lock();
        if let Some(t) = inbox.title.take() {
            self.title = t;
        }
        // OSC 7: the directory the batch left, into the events that said it
        // changed (the newest ones).
        if inbox.directories_unread > 0 {
            let dir = self
                .term
                .get_current_dir()
                .map(|u| u.to_string())
                .unwrap_or_default();
            let mut left = std::mem::take(&mut inbox.directories_unread);
            for e in inbox.events.iter_mut().rev() {
                if left == 0 {
                    break;
                }
                if let Event::Directory(d) = e {
                    d.clone_from(&dir);
                    left -= 1;
                }
            }
        }
    }

    /// When a synchronized update (mode 2026) the remote opened must be
    /// drawn anyway; [`Terminal::flush_sync`] then.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.holding_since.map(|t| t + SYNC_TIMEOUT)
    }

    /// Applies a synchronized update that outlived its deadline.
    pub fn flush_sync(&mut self) {
        self.holding_since = None;
        let held = std::mem::take(&mut self.held);
        self.perform(held);
        self.release_held_marks();
    }

    /// Forgets a half-read sequence: the stream after a reconnect is not
    /// the old one's continuation (the fork's patch 19).
    pub fn discard_pending_input(&mut self) {
        self.parser = Parser::new();
        self.flush_sync();
    }

    /// Bytes the terminal owes the remote, in order.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.shared.lock().replies)
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.shared.lock().events)
    }

    /// Answers the remote's OSC 52 query with the clipboard's text.
    pub fn answer_clipboard(&mut self, text: &str) {
        if let Some(sel) = self.clipboard_query.take() {
            let target = if sel.contains(OscSelection::PRIMARY) {
                "p"
            } else {
                "c"
            };
            let b64 = base64::engine::general_purpose::STANDARD.encode(text);
            let reply = format!("\x1b]52;{target};{b64}\x07");
            self.shared
                .lock()
                .replies
                .extend_from_slice(reply.as_bytes());
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn size(&self) -> (u16, u16) {
        let s = self.term.screen();
        (s.physical_cols as u16, s.physical_rows as u16)
    }

    /// The grid follows the view (text reflows). A view reading history
    /// stays on the line it was reading (B425): the reflow gives row
    /// numbers to other text, so the view's row number alone threw it far
    /// back (narrower) or onto the live screen (wider).
    ///
    /// A selection stays on its text when the columns change (B572), by
    /// each end's place in its logical line; one with an end that can't be
    /// placed (its line left the history) goes, as a block's does.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let reading = self.reading_place();
        let found = self.found_place();
        let selected = if cols != self.size().0 {
            self.selection_places()
        } else {
            None
        };
        self.term.resize(term_size(cols, rows, &self.options));
        if let Some(place) = reading {
            self.go_to_reading_place(place);
        }
        if let Some(place) = found {
            self.go_to_found_place(place);
        }
        if let Some(places) = selected {
            self.restore_selection(places);
        }
        self.full_damage = true;
    }

    /// Where point `p` stands in what a reflow keeps (B572): its logical
    /// line, counted back from the last one with text (as
    /// [`Terminal::reading_place`]), then in the cell of that line's text
    /// it is in (a wide character one cell) and the column within it, or
    /// past the text, the columns past its end. None below the last line
    /// with text (blank rows come and go) or off the kept rows.
    fn text_place(&self, (row, col): select::Point) -> Option<TextPlace> {
        let (cells, first, _) = self.line_cells(row);
        let last = self.last_text_line();
        if first > last || row < self.first_row() {
            return None;
        }
        let mut lines_back = 0;
        let mut at = last;
        while at > first {
            at = self.line_start(at - 1);
            lines_back += 1;
        }
        if at != first {
            return None;
        }
        let text = Self::text_cells(&cells);
        if let Some(k) = text
            .iter()
            .position(|x| x.row == row && (x.col..x.col + x.width).contains(&col))
        {
            return Some(TextPlace {
                lines_back,
                cell: k,
                sub: col - text[k].col,
            });
        }
        let cols = self.size().0.max(1) as usize;
        let linear = |r: StableRowIndex, c: usize| (r - first) as usize * cols + c;
        let end = text.last().map_or(0, |x| linear(x.row, x.col + x.width));
        let here = linear(row, col);
        if here < end {
            // Inside the text but on no cell of it (a wrap's gap): nowhere.
            return None;
        }
        Some(TextPlace {
            lines_back,
            cell: text.len(),
            sub: here - end,
        })
    }

    /// A line's cells up to its last with text or a link (its trailing
    /// blanks are the width's, not the text's: a reflow may keep or drop
    /// them).
    fn text_cells(cells: &[Cell]) -> &[Cell] {
        let n = cells.iter().rposition(|x| !x.blank).map_or(0, |k| k + 1);
        &cells[..n]
    }

    /// The point [`Terminal::text_place`] read, where the lines flow now.
    fn place_point(&self, place: TextPlace) -> Option<select::Point> {
        let mut first = self.last_text_line();
        for _ in 0..place.lines_back {
            if first <= self.first_row() {
                return None;
            }
            first = self.line_start(first - 1);
        }
        let (cells, first, last) = self.line_cells(first);
        let text = Self::text_cells(&cells);
        if place.cell < text.len() {
            let x = &text[place.cell];
            return Some((x.row, x.col + place.sub.min(x.width - 1)));
        }
        if place.cell > text.len() {
            return None;
        }
        let cols = self.size().0.max(1) as usize;
        let end = text
            .last()
            .map_or(0, |x| (x.row - first) as usize * cols + x.col + x.width)
            + place.sub;
        let row = first + (end / cols) as StableRowIndex;
        Some(if row > last {
            (last, cols - 1)
        } else {
            (row, end % cols)
        })
    }

    /// Where the selection's ends stand in what a column reflow keeps
    /// (B572). None: no selection standing, a block's (its columns mean
    /// nothing after a reflow), on the alternate screen, or an end that
    /// can't be placed: it goes with the reflow, as before.
    fn selection_places(&mut self) -> Option<Vec<TextPlace>> {
        self.selection_range()?;
        if self.term.is_alt_screen_active() {
            return None;
        }
        let sel = self.selection.as_ref()?;
        if sel.kind == SelectKind::Block {
            return None;
        }
        sel.ends().into_iter().map(|p| self.text_place(p)).collect()
    }

    /// The selection on the text [`Terminal::selection_places`] read, where
    /// it flows now; gone when an end can't be placed.
    fn restore_selection(&mut self, places: Vec<TextPlace>) {
        let ends: Option<Vec<select::Point>> =
            places.into_iter().map(|p| self.place_point(p)).collect();
        match (ends, self.selection.as_ref()) {
            (Some(ends), Some(sel)) => {
                self.selection = Some(sel.moved(&ends));
                self.stamp_selection();
            }
            _ => self.selection = None,
        }
    }

    /// The first row of the logical line holding stable row `row`.
    fn line_start(&self, row: StableRowIndex) -> StableRowIndex {
        let mut first = row;
        while first > self.first_row() && self.wraps(first - 1) {
            first -= 1;
        }
        first
    }

    /// The first row of the last logical line with text: what a reflow
    /// keeps under the history (the cursor is no mark: the reflow may put
    /// it on another line, and the blank rows under the text come and go).
    fn last_text_line(&self) -> StableRowIndex {
        let mut row = self.screen_top() + self.size().1 as StableRowIndex - 1;
        while row > self.first_row()
            && !self.wraps(row - 1)
            && self
                .with_line(row, |l| Self::line_text(l).0.is_empty())
                .unwrap_or(true)
        {
            row -= 1;
        }
        self.line_start(row)
    }

    /// Where a view reading the main screen's history stands, in what a
    /// reflow keeps: logical lines from its line to the last one with text,
    /// and cells into its line.
    fn reading_place(&self) -> Option<(usize, usize)> {
        if self.view_top.is_none() || self.term.is_alt_screen_active() {
            return None;
        }
        let top = self.view_top();
        if top >= self.screen_top() {
            return None;
        }
        let first = self.line_start(top);
        let cells_in = (top - first) as usize * self.size().0 as usize;
        let mut lines = 0;
        let mut row = self.last_text_line();
        while row > first {
            row = self.line_start(row - 1);
            lines += 1;
        }
        Some((lines, cells_in))
    }

    /// Puts the view back on the line [`Terminal::reading_place`] read, on
    /// whole rows; the live screen if that line is on it now.
    fn go_to_reading_place(&mut self, (lines, cells_in): (usize, usize)) {
        let mut first = self.last_text_line();
        for _ in 0..lines {
            if first <= self.first_row() {
                break;
            }
            first = self.line_start(first - 1);
        }
        let mut last = first;
        while last < self.screen_top() && self.wraps(last) {
            last += 1;
        }
        let cols = self.size().0.max(1) as usize;
        let top = (first + (cells_in / cols) as StableRowIndex).min(last);
        if top >= self.screen_top() {
            self.scroll_to_bottom();
            return;
        }
        self.view_top = Some(top);
        self.scroll_top = self.view_top;
        self.scroll.offset_px = self.display_offset() as i64 * self.cell_px();
        self.fling = None;
    }

    fn reconfigure(&mut self) {
        let config = Arc::new(Config {
            scrollback: self.options.scrollback,
            palette: self.options.palette.to_wezterm(),
            max_image_bytes: self.options.max_image_bytes,
            kitty_image_storage: self.options.kitty_image_storage,
            generation: self.config.generation + 1,
            tmux_clears: self.options.scroll_on_clear == ScrollOnClear::Tmux,
        });
        self.config = config.clone();
        self.term.set_config(config);
        self.full_damage = true;
    }

    /// A new history limit (the app's setting); a shorter one drops the
    /// oldest lines, and a selection on them (B216).
    pub fn set_scrollback(&mut self, lines: usize) {
        self.options.scrollback = lines;
        self.reconfigure();
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.options.palette = palette;
        self.reconfigure();
        // OSC 4/10/11 changes give way to the new theme.
        *self.term.palette_mut() = self.options.palette.to_wezterm();
    }

    // --- Rows --------------------------------------------------------------

    /// The stable row number of the screen's top row.
    fn screen_top(&self) -> StableRowIndex {
        self.term.screen().visible_row_to_stable_row(0)
    }

    /// The oldest row still kept.
    fn first_row(&self) -> StableRowIndex {
        self.term.screen().phys_to_stable_row_index(0)
    }

    /// The viewport's top row.
    fn view_top(&self) -> StableRowIndex {
        let bottom = self.screen_top();
        match self.view_top {
            Some(t) => t.clamp(self.first_row(), bottom),
            None => bottom,
        }
    }

    /// Runs `f` on the line at stable row `row`, if it is kept.
    fn with_line<R>(&self, row: StableRowIndex, f: impl FnOnce(&Line) -> R) -> Option<R> {
        let screen = self.term.screen();
        let phys = screen.stable_row_to_phys(row)?;
        let mut out = None;
        let mut f = Some(f);
        screen.with_phys_lines(phys..phys + 1, |lines| {
            if let (Some(line), Some(f)) = (lines.first(), f.take()) {
                out = Some(f(line));
            }
        });
        out
    }

    /// A kept row's cells and wrap mark, for selection, search and links.
    fn row_cells(&self, row: StableRowIndex) -> Option<select::RowCells> {
        let (mut cells, wrapped): select::RowCells = self.with_line(row, |l| {
            let cells = l
                .visible_cells()
                .map(|c| (c.cell_index(), c.width().max(1), c.str().to_string()))
                .collect();
            (cells, l.last_cell_was_wrapped())
        })?;
        // The cell a wide character left at the right margin when it went
        // on to the next row is nothing (B697). Erased with the pen's
        // background (the fork's wrap of a wide character, as xterm does),
        // it is kept as a blank cell, and a copy of a coloured Chinese line
        // that wrapped there read `一二三四 五`. A blank there before a wide
        // character is taken for that cell: a space the program printed in
        // the last column before one reads the same, and goes too.
        let cols = self.size().0 as usize;
        if wrapped
            && cells
                .last()
                .is_some_and(|(i, w, s)| *i + 1 == cols && *w == 1 && s == " ")
            && self
                .with_line(row + 1, |l| {
                    l.visible_cells()
                        .next()
                        .map(|c| (c.cell_index(), c.width()))
                })
                .flatten()
                .is_some_and(|(i, w)| i == 0 && w > 1)
        {
            cells.pop();
        }
        Some((cells, wrapped))
    }

    fn line_text(line: &Line) -> (String, bool) {
        let mut s = String::new();
        let mut next = 0;
        for cell in line.visible_cells() {
            let idx = cell.cell_index();
            while next < idx {
                s.push(' ');
                next += 1;
            }
            s.push_str(cell.str());
            next = idx + cell.width().max(1);
        }
        s.truncate(s.trim_end_matches(' ').len());
        (s, line.last_cell_was_wrapped())
    }

    /// One row's text (wide spacers left out, joined scalars kept, trailing
    /// blanks trimmed) and whether it continues on the next row. `line`: 0
    /// is the screen's top row, negative rows are history.
    pub fn row_text(&self, line: i32) -> (String, bool) {
        let row = self.screen_top() + line as StableRowIndex;
        self.with_line(row, Self::line_text).unwrap_or_default()
    }

    /// The rows on screen as shown (scrolled view included), for the core's
    /// link detection (docs/nsh/DESIGN.md 3.2).
    pub fn visible_text(&self) -> Vec<(String, bool)> {
        let top = self.view_top();
        let (_, rows) = self.size();
        (0..rows as StableRowIndex)
            .map(|r| self.with_line(top + r, Self::line_text).unwrap_or_default())
            .collect()
    }

    /// Viewport row `row`'s text and whether it continues on the next row,
    /// as [`Terminal::visible_text`] gives it (row `rows` is the row under
    /// the viewport, for a view scrolled by pixels).
    pub fn view_row_text(&self, row: u16) -> (String, bool) {
        let top = self.view_top();
        self.with_line(top + row as StableRowIndex, Self::line_text)
            .unwrap_or_default()
    }

    /// The cursor on the screen (not the view).
    pub fn cursor(&self) -> (u16, u16) {
        let c = self.term.cursor_pos();
        let (cols, _) = self.size();
        ((c.x as u16).min(cols.saturating_sub(1)), c.y.max(0) as u16)
    }

    // --- Drawing -----------------------------------------------------------

    fn color(&self, palette: &ColorPalette, c: ColorAttribute, fg: bool, bold: bool) -> Rgb {
        let c = match c {
            ColorAttribute::PaletteIndex(i) if bold && self.options.bold_is_bright && i < 8 => {
                ColorAttribute::PaletteIndex(i + 8)
            }
            c => c,
        };
        palette::from_tuple(if fg {
            palette.resolve_fg(c)
        } else {
            palette.resolve_bg(c)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_cell(
        &self,
        palette: &ColorPalette,
        reverse_video: bool,
        attrs: &CellAttributes,
        text: &str,
        wide: bool,
        selected: bool,
        matched: bool,
        links: &mut Vec<String>,
        extra: &mut Vec<u32>,
    ) -> DrawCell {
        use cell_flags as F;
        let bold = attrs.intensity() == Intensity::Bold;
        let mut fg = self.color(palette, attrs.foreground(), true, bold);
        let mut bg = self.color(palette, attrs.background(), false, false);
        let mut flags = 0u16;
        match attrs.intensity() {
            Intensity::Bold => flags |= F::BOLD,
            Intensity::Half => {
                flags |= F::DIM;
                fg = palette::dim(fg);
            }
            Intensity::Normal => {}
        }
        if attrs.reverse() != reverse_video {
            std::mem::swap(&mut fg, &mut bg);
        }
        if attrs.invisible() {
            fg = bg;
        }
        if attrs.italic() {
            flags |= F::ITALIC;
        }
        flags |= match attrs.underline() {
            Underline::None => 0,
            Underline::Single => F::UNDERLINE,
            Underline::Double => F::DOUBLE_UNDERLINE,
            Underline::Curly => F::UNDERCURL,
            Underline::Dotted => F::DOTTED_UNDERLINE,
            Underline::Dashed => F::DASHED_UNDERLINE,
        };
        if attrs.strikethrough() {
            flags |= F::STRIKEOUT;
        }
        if wide {
            flags |= F::WIDE;
        }
        if selected {
            flags |= F::SELECTED;
        }
        if matched {
            flags |= F::MATCH;
        }
        let mut link = 0;
        if let Some(h) = attrs.hyperlink() {
            flags |= F::LINK;
            let uri = h.uri();
            link = match links.iter().position(|l| l == uri) {
                Some(i) => i as u32 + 1,
                None => {
                    links.push(uri.to_string());
                    links.len() as u32
                }
            };
        }
        let underline = match attrs.underline_color() {
            ColorAttribute::Default => u32::MAX,
            c => rgb_u32(self.color(palette, c, true, false)),
        };
        let mut chars = text.chars();
        let ch = chars.next().unwrap_or(' ');
        let extra_at = extra.len() as u32;
        extra.extend(chars.map(|c| c as u32));
        DrawCell {
            ch: ch as u32,
            extra_at,
            extra: (extra.len() as u32 - extra_at) as u16,
            flags,
            fg: rgb_u32(fg),
            bg: rgb_u32(bg),
            underline,
            link,
        }
    }

    /// The rows that changed since the last frame (all of them after a
    /// resize, a scroll of the view or the screen, a selection or search
    /// change), copied, with the images on them, and the cursor. Resets
    /// the damage.
    pub fn frame(&mut self) -> Frame {
        self.shed_history_images();
        self.sync_scroll();
        // Whether the selection still stands, before the rows to draw are
        // picked: one that goes takes every row it covered with it (B484).
        let sel = self.selection_range();
        self.check_found();
        let (cols, rows) = self.size();
        let top = self.view_top();
        let alt = self.term.is_alt_screen_active();
        let seqno = self.frame_seqno;
        let span = rows + self.extra_rows;
        // The view moved by whole rows since the last frame (output scrolled
        // the screen under a view following it, or the view scrolled): what
        // was drawn moves with it, and only the rows it uncovers are new.
        let shift: i32 = match self.frame_top {
            Some(ft)
                if !self.full_damage
                    && self.frame_alt == alt
                    && ft != top
                    && (top - ft).unsigned_abs() < span as usize =>
            {
                (top - ft) as i32
            }
            _ => 0,
        };
        let full = self.full_damage
            || self.frame_alt != alt
            || (self.frame_top != Some(top) && shift == 0);

        let c = self.term.cursor_pos();
        let offset = (self.screen_top() - top) as i64;
        let cursor_row = c.y + offset;
        let cursor = (
            c.x.min(cols as usize - 1) as u16,
            cursor_row.clamp(0, u16::MAX as i64) as u16,
        );
        let shown =
            c.visibility == CursorVisibility::Visible && (0..rows as i64).contains(&cursor_row);
        // mosh's guesses, over the live screen only.
        let overlay = match &self.prediction {
            Some(p) if self.view_top.is_none() => p.overlay(&predict_screen::Live::new(self)),
            _ => predict::Overlay::default(),
        };
        let mut cursor = cursor;
        if let Some((r, c)) = overlay.cursor {
            cursor = (c as u16, r as u16);
        }
        let mut predicted: Vec<u16> = overlay.cells.iter().map(|(r, _, _)| *r as u16).collect();
        predicted.sort_unstable();
        predicted.dedup();

        let moved = |r: u16| -> Option<u16> {
            let m = r as i32 - shift;
            (0..span as i32).contains(&m).then_some(m as u16)
        };
        let mut wanted: Vec<u16> = if full {
            (0..span).collect()
        } else {
            let mut v: Vec<u16> = (0..span)
                .filter(|&r| {
                    self.with_line(top + r as StableRowIndex, |l| l.changed_since(seqno))
                        .unwrap_or(false)
                })
                .collect();
            // The rows a shift uncovered.
            if shift > 0 {
                v.extend(span.saturating_sub(shift as u16)..span);
            } else if shift < 0 {
                v.extend(0..((-shift) as u16).min(span));
            }
            // The cursor's old row (where it was drawn, moved with the
            // content) and its new one: a move alone changes no line.
            if let Some((_, r)) = self.frame_cursor {
                v.extend(moved(r));
            }
            if cursor.1 < rows {
                v.push(cursor.1);
            }
            // Rows a guess covered or covers now.
            v.extend(self.predicted_rows.iter().filter_map(|&r| moved(r)));
            v.extend(predicted.iter().copied());
            v.sort_unstable();
            v.dedup();
            v
        };
        wanted.retain(|&r| r < span);

        let palette = self.term.palette();
        let reverse_video = self.term.get_reverse_video();
        let mut links = Vec::new();
        let mut images: Vec<DecodedImage> = Vec::new();
        let mut out = Vec::with_capacity(wanted.len());
        for r in wanted {
            let row = top + r as StableRowIndex;
            let mut frame_row = FrameRow {
                row: r,
                ..FrameRow::default()
            };
            let blank = self.draw_cell(
                &palette,
                reverse_video,
                &CellAttributes::blank(),
                " ",
                false,
                false,
                false,
                &mut links,
                &mut Vec::new(),
            );
            frame_row.cells = vec![blank; cols as usize];
            let mut image_cells: Vec<(usize, Vec<wezterm_term::image::ImageCell>)> = Vec::new();
            let mut extra = Vec::new();
            let wrapped = self
                .with_line(row, |line| {
                    for cell in line.visible_cells() {
                        let idx = cell.cell_index();
                        if idx >= cols as usize {
                            break;
                        }
                        let width = cell.width().max(1);
                        let selected = sel.as_ref().is_some_and(|s| s.contains(row, idx, width));
                        let matched = self
                            .found
                            .is_some_and(|f| select::in_range((f.0, f.1), (f.2, f.3), row, idx));
                        let d = self.draw_cell(
                            &palette,
                            reverse_video,
                            cell.attrs(),
                            cell.str(),
                            width > 1,
                            selected,
                            matched,
                            &mut links,
                            &mut extra,
                        );
                        frame_row.cells[idx] = d;
                        for w in 1..width {
                            if idx + w < cols as usize {
                                let mut spacer = d;
                                spacer.ch = ' ' as u32;
                                spacer.extra = 0;
                                spacer.flags = (d.flags & !cell_flags::WIDE) | cell_flags::SPACER;
                                frame_row.cells[idx + w] = spacer;
                            }
                        }
                        if let Some(imgs) = cell.attrs().images() {
                            image_cells.push((idx, imgs));
                        }
                    }
                    line.last_cell_was_wrapped()
                })
                .unwrap_or(false);
            frame_row.wrapped = wrapped;
            // Links nobody declared: detected URLs draw as links (V34),
            // looked for only in the rows this frame carries.
            let continues = wrapped
                || self
                    .with_line(row - 1, |l| l.last_cell_was_wrapped())
                    .unwrap_or(false);
            let has_http = frame_row
                .cells
                .windows(4)
                .any(|w| w.iter().map(|c| c.ch).eq("http".chars().map(|c| c as u32)));
            // The rest of a URL a program broke at the edge of the row before.
            if continues || has_http || self.may_hold_url_tail(row) {
                for (a, b) in self.detected_link_cols(row) {
                    for c in a..=b.min(cols as usize - 1) {
                        frame_row.cells[c].flags |= cell_flags::LINK;
                    }
                }
            }
            // Paths the host confirmed (there is never one before the host
            // has confirmed something).
            if !self.path_links.is_empty()
                && (continues
                    || frame_row
                        .cells
                        .iter()
                        .any(|c| c.ch == '/' as u32 || c.ch == '.' as u32))
            {
                for (a, b) in self.path_link_cols(row) {
                    for c in a..=b.min(cols as usize - 1) {
                        frame_row.cells[c].flags |= cell_flags::LINK;
                    }
                }
            }
            for (_, c, cell) in overlay.cells.iter().filter(|(pr, _, _)| *pr == r as usize) {
                if *c < cols as usize {
                    frame_row.cells[*c] = self.draw_cell(
                        &palette,
                        reverse_video,
                        &cell.attrs,
                        &cell.text,
                        false,
                        false,
                        false,
                        &mut links,
                        &mut extra,
                    );
                }
            }
            frame_row.extra = extra;
            frame_row.images = self.image_runs(image_cells, &mut images);
            out.push(frame_row);
        }

        self.frame_seqno = self.term.current_seqno();
        self.frame_top = Some(top);
        self.frame_alt = alt;
        self.frame_cursor = Some(cursor);
        self.predicted_rows = predicted;
        self.full_damage = false;
        let (shape, blinking) = match c.shape {
            CursorShape::Default => (0, false),
            CursorShape::BlinkingBlock => (0, true),
            CursorShape::SteadyBlock => (0, false),
            CursorShape::BlinkingUnderline => (1, true),
            CursorShape::SteadyUnderline => (1, false),
            CursorShape::BlinkingBar => (2, true),
            CursorShape::SteadyBar => (2, false),
        };
        Frame {
            cols,
            rows,
            full,
            shift: if full { 0 } else { shift },
            changed: out,
            cursor: CursorView {
                col: cursor.0,
                row: cursor.1,
                shape,
                blinking,
                visible: shown,
            },
            display_offset: offset as u32,
            pixel_shift: scroll::Scroll::shift(self.scroll.offset_px, self.cell_px()) as u32,
            history: self.history_len() as u32,
            links,
            background: rgb_u32(palette::from_tuple(palette.background)),
            images,
            marks: std::mem::take(&mut self.marks),
        }
    }

    /// The images on one row's cells as runs: one per image and stretch of
    /// cells whose texture continues from cell to cell.
    fn image_runs(
        &mut self,
        cells: Vec<(usize, Vec<wezterm_term::image::ImageCell>)>,
        images: &mut Vec<DecodedImage>,
    ) -> Vec<ImageRun> {
        let mut runs: Vec<ImageRun> = Vec::new();
        for (col, cell_images) in cells {
            for ic in cell_images {
                let Some(decoded) = self.images.get(ic.image_data()) else {
                    continue;
                };
                let index = match images.iter().position(|i| i.id == decoded.id) {
                    Some(i) => i as u32 + 1,
                    None => {
                        images.push(decoded);
                        images.len() as u32
                    }
                };
                let (tl, br) = (ic.top_left(), ic.bottom_right());
                let (left, top, right, bottom) = (*tl.x, *tl.y, *br.x, *br.y);
                match runs
                    .iter_mut()
                    .rev()
                    .find(|r| r.image == index && r.z == ic.z_index())
                {
                    Some(r)
                        if r.col + r.cols == col as u16
                            && (r.right - left).abs() < 1e-4
                            && (r.top - top).abs() < 1e-4 =>
                    {
                        r.cols += 1;
                        r.right = right;
                    }
                    _ => runs.push(ImageRun {
                        col: col as u16,
                        cols: 1,
                        image: index,
                        left,
                        top,
                        right,
                        bottom,
                        z: ic.z_index(),
                    }),
                }
            }
        }
        runs
    }

    /// Bytes of decoded images kept for drawing, in the cache this terminal
    /// draws from (the process's, shared by every terminal, unless it was
    /// made with its own).
    pub fn image_cache_bytes(&self) -> usize {
        self.images.bytes()
    }

    /// Bytes the kitty images a program stored take (this terminal's).
    pub fn kitty_image_bytes(&self) -> usize {
        self.term.kitty_image_bytes()
    }

    /// The app goes to the background: the images the emulator keeps only
    /// so an identical one shares their data are let go (those no cell
    /// shows any more are freed). Bytes freed.
    pub fn release_image_caches(&mut self) -> usize {
        self.term.release_image_dedup()
    }

    /// A memory warning: what [`Terminal::release_image_caches`] lets go,
    /// and the kitty images no cell shows any more (on either screen or in
    /// the history), their ids with them (a program placing one again gets
    /// kitty's ENOENT). Bytes freed.
    pub fn drop_unplaced_images(&mut self) -> usize {
        self.term.release_image_dedup() + self.term.kitty_drop_unplaced()
    }

    /// Frames carry `n` rows below the viewport too (0 or 1: a view
    /// scrolling by pixels shows part of the row under its last one); a
    /// change makes the next frame full.
    pub fn set_extra_rows(&mut self, n: u16) {
        if self.extra_rows != n {
            self.extra_rows = n;
            self.full_damage = true;
        }
    }

    /// A cell's size in pixels: what size reports answer and how many cells
    /// an image covers. The grid keeps its rows and columns.
    pub fn set_cell_pixels(&mut self, width: u16, height: u16) {
        let (width, height) = (width.max(1), height.max(1));
        if (self.options.cell_width_px, self.options.cell_height_px) == (width, height) {
            return;
        }
        self.options.cell_width_px = width;
        self.options.cell_height_px = height;
        let (cols, rows) = self.size();
        self.term.resize(term_size(cols, rows, &self.options));
        // A reader stays on its rows, drawn on whole rows of the new size
        // (B479): the old size's pixel offset shifted them by part of a row.
        if self.view_top.is_some() {
            self.scroll_top = self.view_top;
            self.scroll.offset_px = self.display_offset() as i64 * self.cell_px();
            self.fling = None;
            self.full_damage = true;
        }
    }

    /// What a clear of the main screen keeps (a tmux pane's: what tmux
    /// keeps); from the next byte on.
    pub fn set_scroll_on_clear(&mut self, mode: ScrollOnClear) {
        if self.options.scroll_on_clear != mode {
            self.options.scroll_on_clear = mode;
            self.reconfigure();
        }
    }

    /// Bold text in one of the 8 basic colours draws in its bright twin.
    pub fn set_bold_is_bright(&mut self, on: bool) {
        if self.options.bold_is_bright != on {
            self.options.bold_is_bright = on;
            self.full_damage = true;
        }
    }

    // --- Input ---------------------------------------------------------------

    /// Runs `f` with the emulator's writer collecting only what `f` makes:
    /// replies already owed stay where they are, ahead of it.
    fn encoded(&mut self, f: impl FnOnce(&mut wezterm_term::Terminal)) -> Vec<u8> {
        let owed = std::mem::take(&mut self.shared.lock().replies);
        f(&mut self.term);
        std::mem::replace(&mut self.shared.lock().replies, owed)
    }

    /// The bytes a key sends: `mods` are `input::mods` bits; `down` false is
    /// its release (sent only under the kitty protocol with event types);
    /// `repeat` above 1 is a held key's repeat. Encoded by the kitty
    /// protocol while a program has it on (`CSI > n u`), by the xterm
    /// encoder otherwise (DECCKM's application cursor keys, CSI u for what
    /// xterm has no byte for).
    pub fn key(&mut self, key: Key, mods: u8, down: bool, repeat: u16) -> Vec<u8> {
        let flags = self.modes().kitty_flags;
        let bytes = if flags != 0 {
            input::kitty(key, mods, down, repeat, flags)
        } else if !down {
            Vec::new()
        } else if let Some((key, mods)) = input::legacy(key, mods) {
            self.encoded(|t| {
                let _ = t.key_down(key, mods);
            })
        } else {
            Vec::new()
        };
        if down && !bytes.is_empty() {
            self.scroll_to_bottom();
        }
        bytes
    }

    /// The bytes a mouse report sends, in the encoding the program asked
    /// for (nothing while it asked for none; the wheel over the alternate
    /// screen may send arrow keys, as xterm's alternate scroll does).
    /// `kind` 0 press, 1 release, 2 motion; `button` 0 left, 1 middle,
    /// 2 right, 3 none, 4–7 wheel up, down, left, right; viewport cells.
    pub fn mouse(&mut self, kind: u8, button: u8, row: u16, col: u16, mods: u8) -> Vec<u8> {
        let event = input::mouse_event(kind, button, row, col, mods);
        self.encoded(|t| {
            let _ = t.mouse_event(event);
        })
    }

    /// The view gained or lost the keyboard: the focus report a program
    /// asked for (`CSI ? 1004 h`), or nothing.
    pub fn focus(&mut self, focused: bool) -> Vec<u8> {
        self.encoded(|t| t.focus_changed(focused))
    }

    // --- History -----------------------------------------------------------

    /// Scrolls the view `lines` into the history (negative: back down).
    pub fn scroll(&mut self, lines: i32) {
        let bottom = self.screen_top();
        let top = (self.view_top() as i64 - lines as i64)
            .clamp(self.first_row() as i64, bottom as i64) as StableRowIndex;
        self.view_top = if top >= bottom { None } else { Some(top) };
    }

    pub fn scroll_to_bottom(&mut self) {
        self.view_top = None;
        self.scroll.offset_px = 0;
        self.fling = None;
        self.scroll_top = None;
        self.set_extra_rows(0);
    }

    pub fn display_offset(&self) -> usize {
        (self.screen_top() - self.view_top()) as usize
    }

    pub fn history_len(&self) -> usize {
        let s = self.term.screen();
        s.scrollback_rows() - s.physical_rows
    }

    // --- Selection ---------------------------------------------------------

    fn point(&self, row: u16, col: u16) -> (StableRowIndex, usize) {
        let (cols, _) = self.size();
        (
            self.view_top() + row as StableRowIndex,
            (col as usize).min(cols as usize - 1),
        )
    }

    /// The selection's cells, if it still stands on its text: it goes when
    /// its rows leave the history or anything under it changes (an erase, a
    /// reset, new text), as the fork's fixes made it (B79, B100, B180,
    /// B181, B216). A row written again with the same text (tmux's repair
    /// frame at the keyboard going up or down, the return, a switch) still
    /// reads as it did: the selection stands (B480).
    fn selection_range(&mut self) -> Option<select::Range> {
        let sel = self.selection.as_ref()?;
        let range = sel.range(|row| self.row_cells(row));
        let mut rewritten = false;
        let stands = range.as_ref().is_some_and(|r| {
            r.start.0 >= self.first_row()
                && (r.start.0..=r.end.0).all(|row| {
                    match self.with_line(row, |l| {
                        (
                            l.changed_since(sel.seqno),
                            l.changed_since(sel.seqno).then(|| Self::line_text(l)),
                        )
                    }) {
                        None => false,
                        Some((false, _)) => true,
                        Some((true, text)) => {
                            let same = text.as_ref() == sel.read(row);
                            rewritten |= same;
                            same
                        }
                    }
                })
        });
        if stands {
            if rewritten {
                // Every row was just checked: unchanged, or rewritten to what
                // it read. The noted text stands; only the moment moves on
                // (B698: noting it again read every row the selection
                // covers, each frame a program repainted a row as it was —
                // 10 000 rows of history selected cost 9–34 ms a frame).
                let seqno = self.term.current_seqno();
                if let Some(sel) = self.selection.as_mut() {
                    sel.seqno = seqno;
                }
            }
            range
        } else {
            self.selection = None;
            self.full_damage = true;
            None
        }
    }

    /// Whether the rows `sel` noted are still kept and read as they did.
    fn reads_as_before(&self, sel: &select::Selection) -> bool {
        sel.rows.iter().all(|(row, text)| {
            *row >= self.first_row()
                && self
                    .with_line(*row, |l| {
                        !l.changed_since(sel.seqno) || Self::line_text(l) == *text
                    })
                    .unwrap_or(false)
        })
    }

    /// Notes the selection's rows as they read now, and when.
    fn stamp_selection(&mut self) {
        let Some(sel) = self.selection.as_ref() else {
            return;
        };
        let rows = sel
            .range(|row| self.row_cells(row))
            .map(|r| {
                (r.start.0..=r.end.0)
                    .filter_map(|row| self.with_line(row, Self::line_text).map(|t| (row, t)))
                    .collect()
            })
            .unwrap_or_default();
        let seqno = self.term.current_seqno();
        if let Some(sel) = self.selection.as_mut() {
            sel.rows = rows;
            sel.seqno = seqno;
        }
    }

    /// Starts a selection at a viewport cell (`right_half`: the touch was in
    /// the cell's right half, so a drag from there starts after it).
    pub fn select_start(&mut self, kind: SelectKind, row: u16, col: u16, right_half: bool) {
        if kind == SelectKind::Semantic {
            // The whole link under the point, wrapped rows included (the
            // fork's patch 22), else the word.
            if let Some((_, runs)) = self.link_at(row, col)
                && let (Some(first), Some(last)) = (runs.first(), runs.last())
            {
                let top = self.view_top();
                let seqno = self.term.current_seqno();
                let mut sel = select::Selection::new(
                    SelectKind::Cells,
                    (top + first.0 as StableRowIndex, first.1 as usize),
                    false,
                    seqno,
                );
                sel.extend((top + last.0 as StableRowIndex, last.2 as usize), true);
                sel.keep_whole();
                self.selection = Some(sel);
                self.stamp_selection();
                self.full_damage = true;
                return;
            }
            return self.select_start(SelectKind::Word, row, col, right_half);
        }
        let p = self.point(row, col);
        self.selection = Some(select::Selection::new(
            kind,
            p,
            right_half,
            self.term.current_seqno(),
        ));
        self.stamp_selection();
        self.full_damage = true;
    }

    /// Moves the selection's moving end to a viewport cell.
    pub fn select_extend(&mut self, row: u16, col: u16, right_half: bool) {
        let p = self.point(row, col);
        // What it stood on so far must still read the same before it moves.
        if self
            .selection
            .as_ref()
            .is_some_and(|s| !self.reads_as_before(s))
        {
            self.selection = None;
            self.full_damage = true;
            return;
        }
        if let Some(s) = self.selection.as_mut() {
            s.extend(p, right_half);
            self.stamp_selection();
            self.full_damage = true;
        }
    }

    /// A handle's drag starts: the selection keeps its other end where it
    /// is, on screen or not (`end`: keeps the end, the start handle moves;
    /// else keeps the start), cell by cell from here on, and
    /// [`Terminal::select_extend`] moves the dragged one (B477). False when
    /// there is no selection.
    pub fn select_keep_end(&mut self, end: bool) -> bool {
        let Some(r) = self.selection_range() else {
            return false;
        };
        let kept = if end { (r.end, true) } else { (r.start, false) };
        let seqno = self.term.current_seqno();
        self.selection = Some(select::Selection::new(
            SelectKind::Cells,
            kept.0,
            kept.1,
            seqno,
        ));
        self.stamp_selection();
        self.full_damage = true;
        true
    }

    pub fn select_clear(&mut self) {
        if self.selection.take().is_some() {
            self.full_damage = true;
        }
    }

    /// Everything there is (the edit menu's Select All, B699): from the
    /// first row with text, the history's included, to the last cell with
    /// text on the last row that has any — not down the blank rows under
    /// it (B98). False: no text anywhere.
    pub fn select_all(&mut self) -> bool {
        let first = self.first_row();
        let last = self.screen_top() + self.size().1 as StableRowIndex - 1;
        let has_text = |row: StableRowIndex| {
            self.with_line(row, |l| !Self::line_text(l).0.trim().is_empty())
                .unwrap_or(false)
        };
        let Some(top) = (first..=last).find(|&r| has_text(r)) else {
            return false;
        };
        let bottom = (top..=last).rev().find(|&r| has_text(r)).unwrap_or(top);
        let end = self
            .row_cells(bottom)
            .and_then(|(cells, _)| {
                cells
                    .iter()
                    .rev()
                    .find(|(_, _, s)| !s.trim().is_empty())
                    .map(|(i, w, _)| i + w - 1)
            })
            .unwrap_or(0);
        let mut sel = select::Selection::new(
            SelectKind::Cells,
            (top, 0),
            false,
            self.term.current_seqno(),
        );
        sel.extend((bottom, end), true);
        self.selection = Some(sel);
        self.stamp_selection();
        self.full_damage = true;
        true
    }

    /// The selected text (wrapped rows joined, a line break between rows).
    pub fn selection_text(&mut self) -> Option<String> {
        let range = self.selection_range()?;
        let block = self
            .selection
            .as_ref()
            .is_some_and(|s| s.kind == SelectKind::Block);
        let lines = self
            .selection
            .as_ref()
            .is_some_and(|s| s.kind == SelectKind::Line);
        let mut out = String::new();
        for row in range.start.0..=range.end.0 {
            let (cells, wrapped) = self.row_cells(row).unwrap_or_default();
            let mut text = String::new();
            let mut next = None;
            for (idx, width, s) in cells {
                if !range.contains(row, idx, width) {
                    continue;
                }
                if let Some(n) = next {
                    for _ in n..idx {
                        text.push(' ');
                    }
                }
                text.push_str(&s);
                next = Some(idx + width);
            }
            let last = row == range.end.0;
            if wrapped && !last && !block {
                out.push_str(&text);
            } else {
                out.push_str(text.trim_end_matches(' '));
                if !last || lines {
                    out.push('\n');
                }
            }
        }
        if !block && let Some(joined) = self.link_text_without_breaks(&range, &out) {
            return Some(joined);
        }
        (!out.is_empty()).then_some(out)
    }

    /// A selection inside one link a program broke over rows itself copies
    /// as the link reads: without the row breaks and the continuation rows'
    /// indent — when the result is part of the link's address, which a
    /// broken URL is (B423, the fork's 577828e and b6042c4). A label broken
    /// over rows keeps them: there the break stands for a space. The
    /// selection may start on the indent before the link and end on the
    /// blanks after it.
    fn link_text_without_breaks(&self, range: &select::Range, text: &str) -> Option<String> {
        let body = text.trim_end_matches('\n');
        if !body.contains('\n') {
            return None;
        }
        // The first and last cells with content, looked for from either
        // end: every cell of the selection gathered first cost a big
        // selection's copy 50 MB for 10 000 rows, to learn that it was no
        // link (B732).
        let content = |row: StableRowIndex| -> Vec<Cell> {
            self.link_cells(row)
                .unwrap_or_default()
                .into_iter()
                .filter(|x| !x.blank && range.contains(x.row, x.col, x.width))
                .collect()
        };
        let first =
            (range.start.0..=range.end.0).find_map(|row| content(row).into_iter().next())?;
        let (uri, runs) = self.link_span(first.row, first.col)?;
        let covers = |x: &Cell| {
            runs.iter()
                .any(|r| r.0 == x.row && x.col >= r.1 && x.col + x.width - 1 <= r.2)
        };
        if !covers(&first) {
            return None;
        }
        let last = (range.start.0..=range.end.0)
            .rev()
            .find_map(|row| content(row).pop())?;
        if !covers(&last) {
            return None;
        }
        let joined: String = body
            .split('\n')
            .map(|row| row.trim_start_matches(' '))
            .collect();
        uri.contains(&joined).then_some(joined)
    }

    /// The selection's ends in viewport coordinates, rows may lie outside
    /// the viewport (negative: above it): where the handles go.
    pub fn selection_bounds(&mut self) -> Option<((i32, u16), (i32, u16))> {
        let r = self.selection_range()?;
        let top = self.view_top();
        Some((
            ((r.start.0 - top) as i32, r.start.1 as u16),
            ((r.end.0 - top) as i32, r.end.1 as u16),
        ))
    }

    // --- Search ------------------------------------------------------------

    /// The next match of `pattern` (a regex; plain text is a regex too)
    /// after the last one, or from the bottom of the view; the view
    /// scrolls to it. `None`: no match (or a bad pattern).
    pub fn search(&mut self, pattern: &str, forward: bool) -> Option<Match> {
        self.use_pattern(pattern, (false, false))?;
        let matches = self.matches();
        let pick = match (self.found, forward) {
            (Some(f), true) => matches.iter().find(|(s, _)| *s > (f.0, f.1)).copied(),
            (Some(f), false) => matches.iter().rev().find(|(s, _)| *s < (f.0, f.1)).copied(),
            (None, true) => matches.first().copied(),
            (None, false) => matches.last().copied(),
        }?;
        Some(self.show_found(pick, 0))
    }

    /// `pattern` is the search's from now on, its matches starting and
    /// ending words as `edges` says (a new one forgets the match shown).
    /// `None`: not a pattern.
    fn use_pattern(&mut self, pattern: &str, edges: (bool, bool)) -> Option<()> {
        if self
            .search
            .as_ref()
            .is_none_or(|(p, _, e)| p != pattern || *e != edges)
        {
            let re = regex::Regex::new(pattern).ok()?;
            self.search = Some((pattern.to_string(), re, edges));
            self.found_all = None;
            if self.found.take().is_some() {
                self.full_damage = true;
            }
        }
        Some(())
    }

    /// Every match of the search's pattern, history and screen, oldest
    /// first: (first cell, last cell). Kept until the terminal changes
    /// (B734): each step of a find looked through the whole history again
    /// — 26 ms and 20 MB a step over 10 000 rows — though nothing had
    /// changed since the last.
    fn matches(&mut self) -> Vec<(select::Point, select::Point)> {
        let key = (self.term.current_seqno(), self.first_row(), self.size());
        if let Some((k, found)) = &self.found_all
            && *k == key
        {
            return found.clone();
        }
        let found = self.find_all();
        self.found_all = Some((key, found.clone()));
        found
    }

    fn find_all(&self) -> Vec<(select::Point, select::Point)> {
        let Some((_, re, (starts, ends))) = &self.search else {
            return Vec::new();
        };
        let first = self.first_row();
        let last = self.screen_top() + self.size().1 as StableRowIndex - 1;
        let mut matches = Vec::new();
        // The text first; where its characters sit only for the lines with
        // a match.
        for line in select::logical_texts(first, last, |row| self.row_cells(row)) {
            if !re.is_match(&line.text) {
                continue;
            }
            let Some(line) =
                select::logical_lines(line.rows.0, line.rows.1, |row| self.row_cells(row))
                    .into_iter()
                    .next()
            else {
                continue;
            };
            let text = &line.text;
            let mut from = 0;
            while let Some(m) = re.find_at(text, from) {
                let (a, b) = (m.start(), m.end());
                if a == b {
                    from = b + text[b..].chars().next().map_or(1, char::len_utf8);
                    continue;
                }
                let start_ok =
                    !starts || word_edge(text[..a].chars().last(), text[a..].chars().next());
                let end_ok = !ends || word_edge(text[b..].chars().next(), text[..b].chars().last());
                if !(start_ok && end_ok) {
                    // Not on word edges: a match starting a character on
                    // may be.
                    from = a + text[a..].chars().next().map_or(1, char::len_utf8);
                    continue;
                }
                from = b;
                let (Some(s), Some(e)) = (line.at(a), line.at(b - 1)) else {
                    continue;
                };
                matches.push((s, e));
            }
        }
        matches
    }

    /// The match shown, as what a reflow keeps: its place among the
    /// matches, and how many there are.
    fn found_place(&mut self) -> Option<(usize, usize)> {
        let f = self.found?;
        let matches = self.matches();
        let at = matches.iter().position(|(s, _)| *s == (f.0, f.1))?;
        Some((at, matches.len()))
    }

    /// The match [`Terminal::found_place`] read, where the lines now flow
    /// (the view stays where the reflow left it); none if the matches
    /// aren't the same ones.
    fn go_to_found_place(&mut self, (at, count): (usize, usize)) {
        let matches = self.matches();
        match matches.get(at) {
            Some(&((sr, sc), (er, ec))) if matches.len() == count => {
                self.found = Some((sr, sc, er, ec));
                self.stamp_found();
            }
            _ => self.found = None,
        }
    }

    /// Notes the found match's rows as they read now, and when.
    fn stamp_found(&mut self) {
        let Some((sr, _, er, _)) = self.found else {
            self.found_rows.clear();
            return;
        };
        self.found_rows = (sr..=er)
            .filter_map(|row| self.with_line(row, Self::line_text).map(|t| (row, t)))
            .collect();
        self.found_seqno = self.term.current_seqno();
    }

    /// The match shown stays marked while its rows are kept and read as
    /// they did (a repaint of the same text keeps it, B480); other text
    /// there now (a full-screen program redrawing, a `clear`) takes the
    /// mark with it, every row it covered drawn again (B520).
    fn check_found(&mut self) {
        if self.found.is_none() {
            return;
        }
        let seqno = self.found_seqno;
        let mut rewritten = false;
        let stands = !self.found_rows.is_empty()
            && self.found_rows.iter().all(|(row, text)| {
                *row >= self.first_row()
                    && self
                        .with_line(*row, |l| {
                            if !l.changed_since(seqno) {
                                return true;
                            }
                            let same = Self::line_text(l) == *text;
                            rewritten |= same;
                            same
                        })
                        .unwrap_or(false)
            });
        if !stands {
            self.found = None;
            self.found_rows.clear();
            self.full_damage = true;
        } else if rewritten {
            self.found_seqno = self.term.current_seqno();
        }
    }

    /// Makes `pick` the match shown, the view scrolled so it shows with
    /// `context` rows above it when it was off screen.
    fn show_found(
        &mut self,
        pick: (select::Point, select::Point),
        context: StableRowIndex,
    ) -> Match {
        let ((sr, sc), (er, ec)) = pick;
        self.found = Some((sr, sc, er, ec));
        self.stamp_found();
        let rows = self.size().1 as StableRowIndex;
        let top = self.view_top();
        if sr < top || er >= top + rows {
            let first = self.first_row();
            let bottom = self.screen_top();
            let t = (sr - context).clamp(first, bottom);
            self.view_top = if t >= bottom { None } else { Some(t) };
        }
        self.full_damage = true;
        let top = self.view_top();
        Match {
            start: ((sr - top).max(0) as u16, sc as u16),
            end: ((er - top).max(0) as u16, ec as u16),
        }
    }

    /// Find, as a person asks for it: `text` as written (not a pattern),
    /// any case unless `options` says, every match in history and on screen
    /// counted. A new text shows the newest match (the output just read is
    /// at the bottom); then [`FindStep::Next`] goes newer and
    /// [`FindStep::Previous`] older, each round past the end. The view
    /// scrolls to show it; it draws marked. `None`: no match (an empty text
    /// clears).
    pub fn find(&mut self, text: &str, options: FindOptions, step: FindStep) -> Option<Found> {
        if text.is_empty() {
            self.search_clear();
            return None;
        }
        let mut pattern = String::new();
        if !options.case_sensitive {
            pattern.push_str("(?i)");
        }
        pattern.push_str(&regex::escape(text));
        self.use_pattern(&pattern, (options.word_start, options.word_end))?;
        let matches = self.matches();
        if matches.is_empty() {
            if self.found.take().is_some() {
                self.full_damage = true;
            }
            return None;
        }
        let at = self
            .found
            .and_then(|f| matches.iter().position(|(s, _)| *s == (f.0, f.1)));
        let index = match (at, step) {
            (Some(i), FindStep::Next) => (i + 1) % matches.len(),
            (Some(i), FindStep::Previous) => (i + matches.len() - 1) % matches.len(),
            // The same text again (output arrived): the person's place stays.
            (Some(i), FindStep::Newest) => i,
            _ => matches.len() - 1,
        };
        let context = self.size().1 as StableRowIndex / 3;
        let matched = self.show_found(matches[index], context);
        Some(Found {
            matched,
            index,
            count: matches.len(),
        })
    }

    pub fn search_clear(&mut self) {
        self.search = None;
        self.found_all = None;
        if self.found.take().is_some() {
            self.full_damage = true;
        }
    }

    // --- Links -------------------------------------------------------------

    /// The link under a viewport cell: an OSC 8 link the program declared,
    /// else a URL in the text, else a path the host confirmed; its URI and
    /// every cell it covers, rows the program broke it over included, as
    /// (row, first col, last col) runs.
    pub fn link_at(&self, row: u16, col: u16) -> Option<(String, LinkRuns)> {
        let (r, c) = self.point(row, col);
        let (uri, cells) = self.link_span(r, c)?;
        Some((uri, self.view_runs(&cells)))
    }

    /// [`Terminal::link_at`] at stable row `r`, column `c`: the URI and the
    /// cells as (stable row, first col, last col) runs.
    fn link_span(&self, r: StableRowIndex, c: usize) -> Option<(String, Vec<SpanRun>)> {
        let cells = self.link_cells(r)?;
        match cells.iter().find(|x| (x.col..x.col + x.width).contains(&c)) {
            Some(Cell { uri: Some(uri), .. }) => self.osc8_span(r, c, uri),
            _ => self
                .detected_link_at(r, c)
                .or_else(|| self.path_link_at(r, c)),
        }
    }

    /// Stable-row runs as viewport runs.
    fn view_runs(&self, cells: &[SpanRun]) -> LinkRuns {
        let top = self.view_top();
        cells
            .iter()
            .map(|&(row, a, b)| ((row - top) as i32, a as u16, b as u16))
            .collect()
    }

    /// A kept row's cells with the URI of the OSC 8 link each carries, and
    /// whether the row continues on the next one.
    fn link_cells(&self, row: StableRowIndex) -> Option<Vec<Cell>> {
        self.with_line(row, |l| {
            l.visible_cells()
                .map(|cell| Cell {
                    row,
                    col: cell.cell_index(),
                    width: cell.width().max(1),
                    blank: cell.attrs().hyperlink().is_none() && cell.str().trim().is_empty(),
                    uri: cell.attrs().hyperlink().map(|h| h.uri().to_string()),
                })
                .collect()
        })
    }

    fn wraps(&self, row: StableRowIndex) -> bool {
        self.with_line(row, |l| l.last_cell_was_wrapped())
            .unwrap_or(false)
    }

    /// The cells of the logical line (rows joined where they soft-wrap)
    /// holding stable row `row`, with its first and last rows.
    fn line_cells(&self, row: StableRowIndex) -> (Vec<Cell>, StableRowIndex, StableRowIndex) {
        let mut first = row;
        while first > self.first_row() && self.wraps(first - 1) {
            first -= 1;
        }
        let bottom = self.screen_top() + self.size().1 as StableRowIndex - 1;
        let mut last = row;
        while last < bottom && self.wraps(last) {
            last += 1;
        }
        let cells = (first..=last)
            .flat_map(|r| self.link_cells(r).unwrap_or_default())
            .collect();
        (cells, first, last)
    }

    /// The OSC 8 link `uri` at stable row `r`, column `c`, as runs. Its rows
    /// compare by URI alone: tmux's repaints give a link an `id` of their own
    /// making, so after one the rows of a link differ in everything else. A
    /// link that reaches the end of a line's text and goes on at the start
    /// of the next line's (after its indent) is the same link: a program
    /// that lays out its own text (Claude Code) prints a long one as rows of
    /// its own, each re-opening it (B423, the fork's 577828e and b6042c4).
    fn osc8_span(&self, r: StableRowIndex, c: usize, uri: &str) -> Option<(String, Vec<SpanRun>)> {
        let carries = |x: &Cell| x.uri.as_deref() == Some(uri);
        let (seq, mut top, mut bottom) = self.line_cells(r);
        let at = seq
            .iter()
            .position(|x| x.row == r && (x.col..x.col + x.width).contains(&c))?;
        let mut a = at;
        while a > 0 && carries(&seq[a - 1]) {
            a -= 1;
        }
        let mut b = at;
        while b + 1 < seq.len() && carries(&seq[b + 1]) {
            b += 1;
        }
        let mut span: Vec<Cell> = seq[a..=b].to_vec();
        // Up: the line before ends with this link, and this part began it.
        let mut leads = seq[..a].iter().all(|x| x.blank);
        for _ in 0..MAX_LINK_ROWS {
            if !leads || top <= self.first_row() {
                break;
            }
            let (prev, first, _) = self.line_cells(top - 1);
            let Some(t) = prev.iter().rposition(|x| !x.blank) else {
                break;
            };
            if !carries(&prev[t]) {
                break;
            }
            let mut s = t;
            while s > 0 && carries(&prev[s - 1]) {
                s -= 1;
            }
            span.splice(0..0, prev[s..=t].iter().cloned());
            leads = prev[..s].iter().all(|x| x.blank);
            top = first;
        }
        // Down: this part ends its line, and the next line begins with it.
        let mut trails = seq[b + 1..].iter().all(|x| x.blank);
        let screen_bottom = self.screen_top() + self.size().1 as StableRowIndex - 1;
        for _ in 0..MAX_LINK_ROWS {
            if !trails || bottom >= screen_bottom {
                break;
            }
            let (next, _, last) = self.line_cells(bottom + 1);
            let Some(l) = next.iter().position(|x| !x.blank) else {
                break;
            };
            if !carries(&next[l]) {
                break;
            }
            let mut e = l;
            while e + 1 < next.len() && carries(&next[e + 1]) {
                e += 1;
            }
            span.extend(next[l..=e].iter().cloned());
            trails = next[e + 1..].iter().all(|x| x.blank);
            bottom = last;
        }
        Some((uri.to_string(), cell_runs(&span)))
    }

    /// The logical line around stable row `row` (rows joined where they
    /// wrap, at most 16 either way).
    fn logical_line_of(&self, row: StableRowIndex) -> Option<select::LogicalLine> {
        let mut first = row;
        while row - first < 16 && first > self.first_row() && self.wraps(first - 1) {
            first -= 1;
        }
        let mut last = row;
        while last - row < 16 && self.wraps(last) {
            last += 1;
        }
        select::logical_lines(first, last, |r| self.row_cells(r))
            .into_iter()
            .next()
    }

    /// The address-like run a row starts with after its indent (letters,
    /// digits, URL punctuation; trailing sentence punctuation off), as
    /// (first col, text): the rest of a URL the row before broke at its
    /// right edge, if one did (`PlainLinkDetector.hardWrapContinuation`).
    fn url_tail(&self, row: StableRowIndex) -> Option<(usize, String)> {
        let (cells, _) = self.row_cells(row)?;
        let mut next = 0;
        let mut it = cells.iter().peekable();
        while let Some((col, _, s)) = it.peek() {
            if *col != next || !s.trim().is_empty() {
                break;
            }
            next += 1;
            it.next();
        }
        let indent = next;
        let mut run = String::new();
        for (col, width, s) in it {
            let mut chars = s.chars();
            let (Some(ch), None) = (chars.next(), chars.next()) else {
                break;
            };
            if *col != next || *width != 1 || !links::is_tail_char(ch) {
                break;
            }
            run.push(ch);
            next += 1;
        }
        let run = run.trim_end_matches(|c: char| ".,:;!?".contains(c));
        links::looks_like_address_tail(run).then(|| (indent, run.to_string()))
    }

    /// Whether row `row` may hold the rest of a URL a program broke at the
    /// right edge of the row before (no soft wrap; that row full).
    fn may_hold_url_tail(&self, row: StableRowIndex) -> bool {
        let cols = self.size().0 as usize;
        // Cheap first (every frame asks it of every row it carries): the
        // row before is full and ends on text, with no soft wrap.
        let full = row > self.first_row()
            && self
                .with_line(row - 1, |l| {
                    !l.last_cell_was_wrapped()
                        && l.visible_cells().last().is_some_and(|c| {
                            c.cell_index() + c.width().max(1) == cols && !c.str().trim().is_empty()
                        })
                })
                .unwrap_or(false);
        full && self.url_tail(row).is_some()
    }

    /// The URLs in the text whose cells touch stable row `row`: their
    /// addresses and (stable row, first col, last col) runs. A URL that runs
    /// flush into the right edge of a line goes on over the address-like
    /// runs that begin the rows after it, while each reaches the edge too: a
    /// program that lays out its own text broke it there (B423).
    fn detected_links(&self, row: StableRowIndex) -> Vec<(String, Vec<SpanRun>)> {
        let (cols, _) = self.size();
        let mut heads = vec![row];
        // A tail row: the URL it continues began up to MAX_LINK_ROWS before.
        let mut r = row;
        while heads.len() <= MAX_LINK_ROWS && self.may_hold_url_tail(r) {
            r -= 1;
            heads.push(r);
        }
        let mut out: Vec<(String, Vec<SpanRun>)> = Vec::new();
        let mut seen: Vec<StableRowIndex> = Vec::new();
        for head in heads {
            let Some(line) = self.logical_line_of(head) else {
                continue;
            };
            let Some(last_row) = line.last_row() else {
                continue;
            };
            let first_row = line.cell(0).map_or(head, |x| x.0);
            if seen.contains(&first_row) {
                continue;
            }
            seen.push(first_row);
            if !line.text.contains("http") && !line.text.contains("HTTP") {
                continue;
            }
            for range in links::detect(&line.text) {
                let (Some(s), Some(e)) = (line.at(range.start), line.at(range.end - 1)) else {
                    continue;
                };
                let mut url = line.text[range.clone()].to_string();
                let mut runs: Vec<SpanRun> = (s.0..=e.0)
                    .map(|r| {
                        let a = if r == s.0 { s.1 } else { 0 };
                        let b = if r == e.0 { e.1 } else { cols as usize - 1 };
                        (r, a, b)
                    })
                    .collect();
                if e.0 == last_row && e.1 + 1 == cols as usize && !self.wraps(last_row) {
                    for t in (last_row + 1..).take(MAX_LINK_ROWS) {
                        let Some((indent, tail)) = self.url_tail(t) else {
                            break;
                        };
                        url.push_str(&tail);
                        runs.push((t, indent, indent + tail.len() - 1));
                        if indent + tail.len() != cols as usize || self.wraps(t) {
                            break;
                        }
                    }
                }
                if runs.iter().any(|x| x.0 == row) {
                    out.push((url, runs));
                }
            }
        }
        out
    }

    /// The columns of stable row `row` a detected URL covers, as
    /// (first, last) runs.
    fn detected_link_cols(&self, row: StableRowIndex) -> Vec<(usize, usize)> {
        self.detected_links(row)
            .into_iter()
            .flat_map(|(_, runs)| runs)
            .filter(|x| x.0 == row)
            .map(|x| (x.1, x.2))
            .collect()
    }

    /// A detected URL under stable row `r`, column `c`: its text and runs.
    fn detected_link_at(&self, r: StableRowIndex, c: usize) -> Option<(String, Vec<SpanRun>)> {
        self.detected_links(r)
            .into_iter()
            .find(|(_, runs)| runs.iter().any(|x| x.0 == r && (x.1..=x.2).contains(&c)))
    }

    // --- Paths as links -----------------------------------------------------

    /// What the platform can preview (extensions, no dots): paths on screen
    /// become candidates ([`Terminal::path_candidates`]) and the ones the
    /// host confirms draw and tap as links ([`Terminal::set_path_links`]).
    /// None turns paths off and forgets every confirmed one.
    pub fn set_path_rules(&mut self, extensions: &[&str]) {
        let rules = paths::Rules::new(extensions);
        let rules = (!rules.is_empty()).then_some(rules);
        if rules == self.path_rules {
            return;
        }
        self.path_rules = rules;
        if !self.path_links.is_empty() {
            self.path_links.clear();
            self.full_damage = true;
        }
    }

    /// The path candidates on screen, each once, in order, for the host to
    /// confirm: (token, folder only). Not one whose first cell is already a
    /// link (OSC 8, a URL), nor one inside a confirmed path before it (the
    /// shorter starts of a CJK-named path the host settled).
    pub fn path_candidates(&self) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = Vec::new();
        let Some(rules) = &self.path_rules else {
            return out;
        };
        let top = self.view_top();
        let bottom = top + self.size().1 as StableRowIndex;
        let mut row = top;
        while row < bottom {
            let Some(line) = self.logical_line_of(row) else {
                row += 1;
                continue;
            };
            row = line.last_row().map_or(row, |last| last.max(row)) + 1;
            if !line.text.contains(['/', '.']) {
                continue;
            }
            let cells = line.cells();
            let urls = links::detect(&line.text);
            let mut claimed: Vec<Range<usize>> = Vec::new();
            for c in paths::scan(&cells, rules) {
                if claimed.iter().any(|r| r.contains(&c.cells.start))
                    || self.linked_already(&line, &urls, c.cells.start)
                {
                    continue;
                }
                let key = (c.token, c.directory_only);
                if self
                    .path_links
                    .contains_key(&(c.directory_only, key.0.clone()))
                {
                    claimed.push(c.cells);
                }
                if !out.contains(&key) {
                    out.push(key);
                }
            }
        }
        out
    }

    /// The candidates the host confirmed and the URIs they open, replacing
    /// those before; true when that changes what draws as a link (the next
    /// frame redraws the screen).
    pub fn set_path_links(
        &mut self,
        links: impl IntoIterator<Item = (bool, String, String)>,
    ) -> bool {
        let links: HashMap<(bool, String), String> = links
            .into_iter()
            .map(|(folder, token, uri)| ((folder, token), uri))
            .collect();
        if self.path_rules.is_none() || links == self.path_links {
            return false;
        }
        self.path_links = links;
        self.full_damage = true;
        true
    }

    /// Whether cell `k` of `line` is a link already: an OSC 8 one, or in a
    /// detected URL (`urls`, byte ranges of the line's text).
    fn linked_already(&self, line: &select::LogicalLine, urls: &[Range<usize>], k: usize) -> bool {
        if line
            .offset(k)
            .is_some_and(|o| urls.iter().any(|u| u.contains(&o)))
        {
            return true;
        }
        let Some((row, col, _)) = line.cell(k) else {
            return false;
        };
        self.with_line(row, |l| {
            l.visible_cells()
                .find(|cell| {
                    (cell.cell_index()..cell.cell_index() + cell.width().max(1)).contains(&col)
                })
                .is_some_and(|cell| cell.attrs().hyperlink().is_some())
        })
        .unwrap_or(false)
    }

    /// The confirmed paths on `line`: the cells each covers and its URI,
    /// the first confirmed start of each run (the app's link pass: whatever
    /// links a path's first cell already wins).
    fn path_spans(&self, line: &select::LogicalLine) -> Vec<(Range<usize>, String)> {
        let mut out: Vec<(Range<usize>, String)> = Vec::new();
        let Some(rules) = &self.path_rules else {
            return out;
        };
        if self.path_links.is_empty() || !line.text.contains(['/', '.']) {
            return out;
        }
        let cells = line.cells();
        let urls = links::detect(&line.text);
        for c in paths::scan(&cells, rules) {
            let Some(uri) = self.path_links.get(&(c.directory_only, c.token)) else {
                continue;
            };
            if out.iter().any(|(r, _)| r.contains(&c.cells.start))
                || self.linked_already(line, &urls, c.cells.start)
            {
                continue;
            }
            out.push((c.cells, uri.clone()));
        }
        out
    }

    /// The columns of stable row `row` a confirmed path covers, as
    /// (first, last) runs.
    fn path_link_cols(&self, row: StableRowIndex) -> Vec<(usize, usize)> {
        let Some(line) = self.logical_line_of(row) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (cells, _) in self.path_spans(&line) {
            for k in cells {
                if let Some((r, a, b)) = line.cell(k)
                    && r == row
                {
                    out.push((a, b));
                }
            }
        }
        out
    }

    /// A confirmed path under stable row `r`, column `c`: its URI and runs.
    fn path_link_at(&self, r: StableRowIndex, c: usize) -> Option<(String, Vec<SpanRun>)> {
        let line = self.logical_line_of(r)?;
        for (cells, uri) in self.path_spans(&line) {
            let under = cells.clone().any(|k| {
                line.cell(k)
                    .is_some_and(|(row, a, b)| row == r && (a..=b).contains(&c))
            });
            if !under {
                continue;
            }
            let mut runs: Vec<SpanRun> = Vec::new();
            for k in cells {
                let Some((row, a, b)) = line.cell(k) else {
                    continue;
                };
                match runs.last_mut() {
                    Some(run) if run.0 == row => run.2 = b,
                    _ => runs.push((row, a, b)),
                }
            }
            return Some((uri, runs));
        }
        None
    }

    // --- Typed text, pastes, taps ------------------------------------------

    /// What the remote receives for an input-method outcome's typing, keys
    /// encoded as the program asked (under the kitty protocol's "report all
    /// keys" a typed ASCII character goes as its key's code). Typing puts
    /// the view back on the live screen.
    pub fn type_out(&mut self, typed: &[Typed]) -> Vec<u8> {
        let mut out = Vec::new();
        let all_keys = self.modes().kitty_flags & 8 != 0;
        for t in typed {
            match t {
                Typed::Text(s) => {
                    for ch in s.chars() {
                        if all_keys && ch.is_ascii() && !ch.is_ascii_control() {
                            out.extend(self.key(Key::Char(ch), 0, true, 1));
                        } else if ch == '\t' {
                            out.extend(self.key(Key::Tab, 0, true, 1));
                        } else {
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                    }
                }
                Typed::Backspace => out.extend(self.key(Key::Backspace, 0, true, 1)),
                Typed::Enter => out.extend(self.key(Key::Enter, 0, true, 1)),
                Typed::Left => out.extend(self.key(Key::Left, 0, true, 1)),
                Typed::Right => out.extend(self.key(Key::Right, 0, true, 1)),
            }
        }
        if !out.is_empty() {
            self.scroll_to_bottom();
        }
        out
    }

    /// Runs an input-method call on the shadow document: what to send.
    pub fn ime_apply(&mut self, f: impl FnOnce(&mut Ime) -> ime::Outcome) -> ImeSend {
        let outcome = f(&mut self.ime);
        if outcome.paste.is_some() {
            self.scroll_to_bottom();
        }
        ImeSend {
            bytes: self.type_out(&outcome.typed),
            notify: outcome.notify,
            paste: outcome.paste,
        }
    }

    /// What a paste sends: bracketed when the program asked for it (or
    /// `force`), markers in the text taken out (V30). Back to the live
    /// screen.
    pub fn paste(&mut self, text: &str, force: bool) -> Vec<u8> {
        let bracketed = force || self.modes().bracketed_paste;
        self.scroll_to_bottom();
        input::paste(text, bracketed)
    }

    /// What a tap at a viewport cell means.
    pub fn tap(&mut self, row: u16, col: u16) -> Tap {
        if self.selection.is_some() && self.selection_range().is_some() {
            self.select_clear();
            return Tap::Dismissed;
        }
        self.selection = None;
        if let Some((uri, _)) = self.link_at(row, col) {
            return Tap::Link(uri);
        }
        let reading = self.display_offset() > 0 || self.scroll.offset_px > 0;
        let cursor = self.term.cursor_pos().y + (self.screen_top() - self.view_top()) as i64;
        Tap::Click {
            keyboard: !reading && (row as i64 - cursor).abs() <= TYPING_ROWS,
        }
    }

    // --- The scroll model --------------------------------------------------

    fn cell_px(&self) -> i64 {
        self.options.cell_height_px.max(1) as i64
    }

    /// A drag of `dy` device pixels (positive: toward older output). With
    /// `host_rows` the session scrolls its own buffer (tmux, herdr): whole
    /// rows for it. Else a program that wants the mouse gets wheel ticks,
    /// and otherwise this terminal's history moves by the pixel.
    pub fn scroll_drag(&mut self, dy: f64, host_rows: bool) -> ScrollAction {
        let cell = self.cell_px() as f64;
        if host_rows {
            let rows = self
                .scroll
                .take(dy, cell)
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
            return if rows == 0 {
                ScrollAction::None
            } else {
                ScrollAction::Host { rows }
            };
        }
        if self.modes().mouse != 0 {
            let ticks = self.scroll.take(dy, cell).clamp(-6, 6);
            if ticks == 0 {
                return ScrollAction::None;
            }
            let (cols, rows) = self.size();
            let mut bytes = Vec::new();
            for _ in 0..ticks.unsigned_abs() {
                bytes.extend(self.mouse(0, if ticks > 0 { 4 } else { 5 }, rows / 2, cols / 2, 0));
            }
            return if bytes.is_empty() {
                ScrollAction::None
            } else {
                ScrollAction::Wheel(bytes)
            };
        }
        // An alternate-screen program without the mouse (less, man, git
        // log) gets the arrow keys, as xterm's alternate scroll gives them
        // (B500): the alternate screen keeps no history to page.
        if self.term.is_alt_screen_active() {
            let ticks = self.scroll.take(dy, cell).clamp(-6, 6);
            let bytes = self.arrow_presses(ticks > 0, ticks.unsigned_abs());
            return if bytes.is_empty() {
                ScrollAction::None
            } else {
                ScrollAction::Wheel(bytes)
            };
        }
        self.scroll_local(dy)
    }

    /// `count` presses of Up (`up`, older) or Down, as the program reads
    /// them (its cursor-key mode, or the kitty protocol when it has that
    /// on), without taking the view back to the live screen as typing does:
    /// a coast's steps come through here.
    fn arrow_presses(&mut self, up: bool, count: u64) -> Vec<u8> {
        if count == 0 {
            return Vec::new();
        }
        let key = if up { Key::Up } else { Key::Down };
        let flags = self.modes().kitty_flags;
        let one = if flags != 0 {
            input::kitty(key, 0, true, 1, flags)
        } else if let Some((key, mods)) = input::legacy(key, 0) {
            self.encoded(|t| {
                let _ = t.key_down(key, mods);
            })
        } else {
            Vec::new()
        };
        one.repeat(count as usize)
    }

    fn scroll_local(&mut self, dy: f64) -> ScrollAction {
        if !dy.is_finite() {
            return ScrollAction::None;
        }
        self.sync_scroll();
        let max = self.history_len() as i64 * self.cell_px();
        let target = (self.scroll.offset_px as f64 + dy)
            .round()
            .clamp(0.0, max as f64) as i64;
        let moved = target - self.scroll.offset_px;
        if moved == 0 {
            return ScrollAction::None;
        }
        self.set_scroll_offset(target);
        ScrollAction::Local { moved_px: moved }
    }

    fn set_scroll_offset(&mut self, px: i64) {
        if px <= 0 {
            self.scroll_to_bottom();
            return;
        }
        self.scroll.offset_px = px;
        let lines = scroll::Scroll::lines(px, self.cell_px());
        let bottom = self.screen_top();
        let top = (bottom - lines as StableRowIndex).max(self.first_row());
        self.view_top = Some(top);
        self.scroll_top = Some(top);
        self.set_extra_rows(1);
    }

    /// Keeps the pixel offset with the view: output that scrolls the screen
    /// under a view reading history moves the offset with it (the text
    /// stays put); a view the core moved itself (a search, a reset, `clear`)
    /// lands on whole rows.
    fn sync_scroll(&mut self) {
        let cell = self.cell_px();
        let lines = self.display_offset() as i64;
        if scroll::Scroll::lines(self.scroll.offset_px, cell) == lines {
            return;
        }
        if lines == 0 {
            self.scroll.offset_px = 0;
            self.scroll_top = None;
            self.set_extra_rows(0);
            return;
        }
        let shift = scroll::Scroll::shift(self.scroll.offset_px, cell);
        let same_top = self.view_top.is_some() && self.view_top == self.scroll_top;
        self.scroll.offset_px = lines * cell - if same_top { shift } else { 0 };
        self.scroll_top = self.view_top;
        self.set_extra_rows(1);
    }

    /// The drag ended: within half a row of the live screen docks onto it.
    pub fn scroll_end(&mut self) -> ScrollAction {
        self.scroll.carry_px = 0.0;
        self.sync_scroll();
        let off = self.scroll.offset_px;
        if off > 0 && off < self.cell_px() / 2 {
            self.scroll_to_bottom();
            return ScrollAction::Local { moved_px: -off };
        }
        ScrollAction::None
    }

    // --- The coast after a lift (B521) ---------------------------------------

    /// A drag's vertical move, `dy` device pixels (positive: older),
    /// wherever it went: the speed its lift lets go at is read from these.
    pub fn fling_track(&mut self, dy: f64) {
        let now = self.now_ms();
        self.fling_track_at(now, dy);
    }

    /// [`Terminal::fling_track`] at `at_ms` on the terminal's clock.
    pub fn fling_track_at(&mut self, at_ms: u64, dy: f64) {
        self.velocity.add(at_ms, dy);
    }

    /// The drag's lift: true if it coasts, at the drag's last speed.
    /// `elsewhere`: the drag went to the host (tmux, herdr) and not to this
    /// terminal; there, and for a program that wants the mouse (wheel
    /// ticks), the coast is paced and bounded.
    pub fn fling_start(&mut self, elsewhere: bool) -> bool {
        let now = self.now_ms();
        self.fling_start_at(now, elsewhere)
    }

    /// [`Terminal::fling_start`] at `at_ms` on the terminal's clock. A
    /// quick flick the same way as the coast its touch stopped goes at both
    /// speeds added (B523: flicks stack, as UIScrollView's do), up to
    /// [`scroll::MAX_FLING`].
    pub fn fling_start_at(&mut self, at_ms: u64, elsewhere: bool) -> bool {
        let mut velocity = self.velocity.at(at_ms);
        self.velocity.clear();
        self.fling = None;
        let carried = self.carried.take();
        if !velocity.is_finite() || velocity.abs() < scroll::MIN_FLING {
            return false;
        }
        if let Some((carry, stopped)) = carried
            && carry.signum() == velocity.signum()
            && velocity.abs() >= scroll::STACK_MIN
            && at_ms.saturating_sub(stopped) <= scroll::STACK_WITHIN_MS
        {
            velocity += carry;
        }
        let velocity = velocity.clamp(-scroll::MAX_FLING, scroll::MAX_FLING);
        let row = self.cell_px() as f64;
        let pacing = if elsewhere {
            Some(scroll::Pacing::host(row))
        } else if self.modes().mouse != 0 || self.term.is_alt_screen_active() {
            // The wheel's reports, or an alternate-screen program's arrow
            // keys (B500): each is the program's work.
            Some(scroll::Pacing::wheel(row))
        } else {
            None
        };
        self.fling = Some(scroll::Fling::new(velocity, pacing));
        true
    }

    /// The coast's speed now, pixels a second (positive: older).
    pub fn fling_velocity(&self) -> Option<f64> {
        self.fling.as_ref().map(scroll::Fling::velocity)
    }

    /// One display refresh of a coast, `dt` seconds on: the pixels to move
    /// now (0: none this refresh), to route as a drag's are. `None`: it has
    /// stopped. This terminal's own history stops it at either end.
    pub fn fling_step(&mut self, dt: f64) -> Option<f64> {
        let fling = self.fling.as_mut()?;
        let paced = fling.is_paced();
        let (dy, going) = fling.step(dt);
        if !going {
            self.fling = None;
        }
        if !paced && dy != 0.0 {
            self.sync_scroll();
            let max = self.history_len() as i64 * self.cell_px();
            let off = self.scroll.offset_px;
            if (dy < 0.0 && off <= 0) || (dy > 0.0 && off >= max) {
                self.fling = None;
                return None;
            }
        }
        (going || dy != 0.0).then_some(dy)
    }

    /// A touch: true if it stopped a coast (that touch is not a tap). The
    /// coast's speed is kept for a flick to add (B523).
    pub fn fling_stop(&mut self) -> bool {
        let now = self.now_ms();
        self.fling_stop_at(now)
    }

    /// [`Terminal::fling_stop`] at `at_ms` on the terminal's clock.
    pub fn fling_stop_at(&mut self, at_ms: u64) -> bool {
        self.velocity.clear();
        let stopped = self.fling.take();
        self.carried = stopped.as_ref().map(|f| (f.velocity(), at_ms));
        stopped.is_some()
    }

    /// Whether a coast is under way.
    pub fn flinging(&self) -> bool {
        self.fling.is_some()
    }

    /// Pixels into the history.
    pub fn scroll_offset_px(&self) -> i64 {
        self.scroll.offset_px
    }

    // --- mosh's predictive echo -------------------------------------------

    fn now_ms(&self) -> u64 {
        self.born.elapsed().as_millis() as u64
    }

    /// Predictive echo on (`Never`: off), dim unconfirmed guesses with
    /// `trail`.
    pub fn predict_attach(&mut self, mode: predict::PredictMode, trail: bool) {
        self.prediction =
            (mode != predict::PredictMode::Never).then(|| predict::Prediction::new(mode, trail));
        self.full_damage = true;
    }

    /// The transport's smoothed round trip (ms).
    pub fn predict_srtt(&mut self, ms: f64) {
        if let Some(p) = self.prediction.as_mut() {
            p.srtt_ms = ms;
        }
    }

    /// Keystrokes on their way to the server: the batch to report with
    /// [`Terminal::predict_sent`], or None. Reading history: no guesses
    /// (they address the live screen's rows), and the engine starts over.
    pub fn predict_input(&mut self, bytes: &[u8]) -> Option<u64> {
        let now = self.now_ms();
        let reading = self.view_top.is_some();
        let mut p = self.prediction.take()?;
        let batch = if reading {
            p.reset();
            None
        } else {
            p.user_input(bytes, &predict_screen::Live::new(self), now)
        };
        self.prediction = Some(p);
        batch
    }

    pub fn predict_sent(&mut self, batch: u64, num: Option<u64>) {
        if let Some(p) = self.prediction.as_mut() {
            p.sent(batch, num);
        }
    }

    /// The server's echo ack, after the bytes of its frame: the guesses it
    /// covers are judged.
    pub fn predict_echo_ack(&mut self, ack: u64) {
        if let Some(p) = self.prediction.as_mut() {
            p.echo_ack(ack);
        }
        self.predict_settle();
    }

    fn predict_settle(&mut self) {
        if self.prediction.is_none() {
            return;
        }
        let now = self.now_ms();
        let mut p = self.prediction.take().unwrap();
        p.settle(&predict_screen::Live::new(self), now);
        self.prediction = Some(p);
    }

    /// Every guess goes (bytes the engine cannot model are on their way).
    pub fn predict_reset(&mut self) {
        if let Some(p) = self.prediction.as_mut() {
            p.reset();
        }
    }

    /// Whether [`Terminal::predict_tick`] should run every 50 ms.
    pub fn predict_needs_tick(&self) -> bool {
        self.prediction.as_ref().is_some_and(|p| p.needs_tick())
    }

    /// The clock's part of judging: true when what shows changed.
    pub fn predict_tick(&mut self) -> bool {
        let now = self.now_ms();
        self.prediction.as_mut().is_some_and(|p| p.tick(now))
    }

    // --- Accessibility -----------------------------------------------------

    /// The rows on screen as a screen reader reads them.
    pub fn a11y_lines(&self) -> Vec<String> {
        self.visible_text().into_iter().map(|(t, _)| t).collect()
    }

    /// What appeared since the last call, for a screen reader to announce:
    /// the rows from where the cursor was then on (output, the line it
    /// echoed), never the cursor's row now (what is being typed is the
    /// keyboard's to speak). The rows above the cursor's then are settled;
    /// finding them again, moved up, tells how far the screen scrolled, so
    /// a prompt row typed into while the screen scrolled is not taken for
    /// new text. Without them in view, the rows that changed.
    pub fn take_announcement(&mut self) -> Option<String> {
        let now = self.a11y_lines();
        let cursor =
            (self.term.cursor_pos().y + (self.screen_top() - self.view_top()) as i64) as usize;
        let last = std::mem::replace(&mut self.a11y_last, now.clone());
        let last_cursor = std::mem::replace(&mut self.a11y_last_cursor, cursor);
        if last.is_empty() {
            return None;
        }
        let n = now.len().min(last.len());
        let settled = last_cursor.min(n);
        let shift = (0..settled).find(|&k| {
            let kept = &now[..settled - k];
            kept == &last[k..settled] && kept.iter().any(|l| !l.is_empty())
        });
        let fresh: Vec<&str> = now
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                *i != cursor
                    && !l.is_empty()
                    && match shift {
                        Some(k) => *i >= settled - k,
                        None => last.get(*i) != Some(*l),
                    }
            })
            .map(|(_, l)| l.as_str())
            .collect();
        (!fresh.is_empty()).then(|| fresh.join("\n"))
    }

    /// What VoiceOver hears now (V33), paced: after an announcement a
    /// second of quiet, and what appears in it is said at its end, in one
    /// (`again_in_ms`: ask then, frame or no frame). Only what is said
    /// starts a quiet: a change with nothing to say (the cursor's row)
    /// leaves the next one free (B427). The view's own one-second gate let
    /// output that ended within a second of the last announcement go
    /// unsaid until the next, unrelated change, and armed itself on frames
    /// with nothing to say. `now_ms` on the terminal's clock
    /// ([`Terminal::announce`] reads it).
    pub fn announce_at(&mut self, now_ms: u64) -> Announcement {
        if let Some(until) = self.a11y_quiet_until
            && now_ms < until
        {
            return Announcement {
                text: None,
                again_in_ms: Some(until - now_ms),
            };
        }
        self.a11y_quiet_until = None;
        let text = self.take_announcement();
        if text.is_some() {
            self.a11y_quiet_until = Some(now_ms + ANNOUNCE_EVERY_MS);
        }
        Announcement {
            text,
            again_in_ms: None,
        }
    }

    /// [`Terminal::announce_at`] now.
    pub fn announce(&mut self) -> Announcement {
        let now = self.now_ms();
        self.announce_at(now)
    }

    // --- Modes -------------------------------------------------------------

    pub fn modes(&self) -> Modes {
        let t = &self.term;
        let kitty_flags = match t.get_keyboard_encoding() {
            termwiz::input::KeyboardEncoding::Kitty(flags) => flags.bits() as u8,
            _ => 0,
        };
        Modes {
            mouse: if t.any_event_mouse_enabled() {
                3
            } else if t.button_event_mouse_enabled() {
                2
            } else if t.mouse_tracking_enabled() {
                1
            } else {
                0
            },
            mouse_sgr: matches!(
                t.get_mouse_encoding(),
                MouseEncoding::SGR | MouseEncoding::SgrPixels
            ),
            mouse_utf8: matches!(t.get_mouse_encoding(), MouseEncoding::Utf8),
            bracketed_paste: t.bracketed_paste_enabled(),
            app_cursor: t.application_cursor_keys_enabled(),
            app_keypad: t.application_keypad_enabled(),
            focus_reports: t.focus_tracking_enabled(),
            alt_screen: t.is_alt_screen_active(),
            alternate_scroll: false,
            kitty_flags,
            cursor_visible: t.cursor_pos().visibility == CursorVisibility::Visible,
        }
    }
}

/// The server's screen as the prediction engine reads it.
mod predict_screen {
    use std::{cell::RefCell, collections::HashMap};

    use wezterm_term::{CellAttributes, StableRowIndex};

    use crate::{Terminal, predict};

    pub(crate) struct Live<'a> {
        term: &'a Terminal,
        rows: RefCell<HashMap<usize, Vec<predict::PCell>>>,
    }

    impl<'a> Live<'a> {
        pub(crate) fn new(term: &'a Terminal) -> Self {
            Self {
                term,
                rows: RefCell::default(),
            }
        }

        fn row(&self, row: usize) -> Vec<predict::PCell> {
            let (cols, _) = self.term.size();
            let blank = predict::PCell::blank(CellAttributes::blank());
            let mut out = vec![blank; cols as usize];
            let stable = self.term.screen_top() + row as StableRowIndex;
            self.term.with_line(stable, |line| {
                for cell in line.visible_cells() {
                    let i = cell.cell_index();
                    if i >= out.len() {
                        break;
                    }
                    let w = cell.width().max(1);
                    out[i] = predict::PCell {
                        text: cell.str().to_string(),
                        width: w as u8,
                        attrs: cell.attrs().clone(),
                    };
                    for k in 1..w {
                        if i + k < out.len() {
                            out[i + k] = predict::PCell {
                                text: String::new(),
                                width: 0,
                                attrs: cell.attrs().clone(),
                            };
                        }
                    }
                }
            });
            out
        }
    }

    impl predict::Screen for Live<'_> {
        fn cols(&self) -> usize {
            self.term.size().0 as usize
        }
        fn rows(&self) -> usize {
            self.term.size().1 as usize
        }
        fn cursor(&self) -> (usize, usize) {
            let (col, row) = self.term.cursor();
            (row as usize, col as usize)
        }
        fn rendition(&self) -> CellAttributes {
            self.term.term.pen()
        }
        fn cell(&self, row: usize, col: usize) -> predict::PCell {
            let mut rows = self.rows.borrow_mut();
            let cells = rows.entry(row).or_insert_with(|| self.row(row));
            cells
                .get(col)
                .cloned()
                .unwrap_or_else(|| predict::PCell::blank(CellAttributes::blank()))
        }
    }
}

/// Whether a match's edge — `inside` its first (last) character, `outside`
/// the one before (after) it — is a word's edge, for find's "starts with"
/// and "whole words" (B730). The regex's `\b` stood there: a text that
/// begins or ends with punctuation (`--force`, `./run`, `-v`) has no `\b`
/// after a space, so it was never found; and Chinese and Japanese have no
/// spaces between words, so a word inside a sentence (世界 in 你好世界) was
/// never one either. A punctuation mark is an edge; so is any side of a
/// CJK character (there is no telling where a word there ends), and a
/// letter next to one.
fn word_edge(outside: Option<char>, inside: Option<char>) -> bool {
    fn word(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }
    fn cjk(c: char) -> bool {
        matches!(c,
            '\u{3040}'..='\u{30FF}'
            | '\u{31F0}'..='\u{31FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2FA1F}')
    }
    match (outside, inside) {
        (None, _) | (_, None) => true,
        (Some(o), Some(i)) => !word(o) || !word(i) || cjk(o) || cjk(i),
    }
}
