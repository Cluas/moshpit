//! mosh's framebuffer, the part of its terminal emulator that host frames
//! use, and its frame renderer: what mosh-client keeps for every server
//! state it holds and how it repaints its terminal from one state to
//! another. Ported from mosh 1.4.0 (`terminalframebuffer.cc`, `terminal.cc`,
//! `terminalfunctions.cc`, `terminaldispatcher.cc`, `terminaldisplay.cc`).
//!
//! mosh-server's host bytes are `Display::new_frame` output: a repaint of
//! one of its framebuffers into another, in a small vocabulary (printing,
//! CR, LF, BS, BEL, CUP, EL, ED, ECH, SGR, DECSTBM, a few private modes,
//! OSC 0/1/2/52). A diff is relative to the screen of the state it names,
//! so it can be replayed only on that screen. [`Framebuffer::apply`]
//! interprets that vocabulary on a held state (anything else is ignored, as
//! mosh's emulator ignores what it does not know); [`new_frame`] writes the
//! difference between what the pane shows and a newer state the same way
//! mosh-server and mosh-client do.
//!
//! Rows are shared between states (`Arc`, copied on write, as mosh shares
//! them with `shared_ptr`), so holding several states costs only the rows
//! that changed. Each row carries mosh's generation number: a fresh one
//! when a row is created or erased whole, kept when it is copied or moved.
//! The renderer's scroll detection compares rows with it, so a blank row
//! never matches another blank row it did not come from.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use unicode_width::UnicodeWidthChar;

static GENERATION: AtomicU64 = AtomicU64::new(0);

fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// Most cells a framebuffer may have: a resize beyond it is ignored (a
/// server that names a huge size cannot make every held state that big).
pub const MAX_CELLS: usize = 1 << 20;
/// mosh's `MAXIMUM_CLIPBOARD_SIZE`: an OSC string stops growing here.
const MAX_OSC: usize = 16 * 1024;
/// mosh keeps at most 256 characters of a title.
const MAX_TITLE: usize = 256;
/// mosh keeps at most 32 bytes in a cell (`Cell::full`).
const MAX_CELL_BYTES: usize = 32;

/// mosh's `Renditions`: colours as mosh numbers them (0 default, 30..=37
/// and 40..=47 the ANSI set, 30 + n and 40 + n the 256-colour set, true
/// colour with bit 24 set) and attribute bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Renditions {
    fg: u32,
    bg: u32,
    attrs: u8,
}

const TRUE_COLOR: u32 = 0x100_0000;
const BOLD: u8 = 1 << 0;
const ITALIC: u8 = 1 << 2;
const UNDERLINED: u8 = 1 << 3;
const BLINK: u8 = 1 << 4;
const INVERSE: u8 = 1 << 5;
const INVISIBLE: u8 = 1 << 6;

impl Renditions {
    fn with_background(bg: u32) -> Self {
        Self {
            fg: 0,
            bg,
            attrs: 0,
        }
    }

    fn set_rendition(&mut self, num: i64) {
        match num {
            0 => *self = Self::default(),
            39 => self.fg = 0,
            49 => self.bg = 0,
            30..=37 => self.fg = num as u32,
            40..=47 => self.bg = num as u32,
            90..=97 => self.fg = (num - 90 + 38) as u32,
            100..=107 => self.bg = (num - 100 + 48) as u32,
            _ => {
                let value = num < 9;
                let bit = match num {
                    1 | 22 => BOLD,
                    3 | 23 => ITALIC,
                    4 | 24 => UNDERLINED,
                    5 | 25 => BLINK,
                    7 | 27 => INVERSE,
                    8 | 28 => INVISIBLE,
                    _ => return,
                };
                if value {
                    self.attrs |= bit;
                } else {
                    self.attrs &= !bit;
                }
            }
        }
    }

    fn color(num: i64, base: u32) -> Option<u32> {
        if (0..=255).contains(&num) {
            Some(base + num as u32)
        } else if num as u32 & TRUE_COLOR != 0 {
            Some(num as u32 & 0x1FF_FFFF)
        } else {
            None
        }
    }

    /// mosh's `Renditions::sgr`: a full reset, then every attribute.
    fn sgr(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"\x1b[0");
        for (bit, code) in [
            (BOLD, ";1"),
            (ITALIC, ";3"),
            (UNDERLINED, ";4"),
            (BLINK, ";5"),
            (INVERSE, ";7"),
            (INVISIBLE, ";8"),
        ] {
            if self.attrs & bit != 0 {
                out.extend_from_slice(code.as_bytes());
            }
        }
        for (color, ansi_max, base, extended) in [(self.fg, 37, 30, 38), (self.bg, 47, 40, 48)] {
            if color == 0 {
                continue;
            }
            let text = if color & TRUE_COLOR != 0 {
                format!(
                    ";{extended};2;{};{};{}",
                    (color >> 16) & 0xff,
                    (color >> 8) & 0xff,
                    color & 0xff
                )
            } else if color > ansi_max {
                format!(";{extended};5;{}", color - base)
            } else {
                format!(";{color}")
            };
            out.extend_from_slice(text.as_bytes());
        }
        out.push(b'm');
    }
}

/// A cell's text: a base character and its combining characters, as UTF-8.
/// Up to 15 bytes inline (nearly every cell), more on the heap.
#[derive(Clone, PartialEq, Eq)]
enum Contents {
    Inline { len: u8, bytes: [u8; 15] },
    Heap(Box<[u8]>),
}

impl Default for Contents {
    fn default() -> Self {
        Contents::Inline {
            len: 0,
            bytes: [0; 15],
        }
    }
}

impl Contents {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Contents::Inline { len, bytes } => &bytes[..usize::from(*len)],
            Contents::Heap(bytes) => bytes,
        }
    }

    fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }

    fn push(&mut self, ch: char) {
        let mut buf = [0; 4];
        let add = ch.encode_utf8(&mut buf).as_bytes();
        match self {
            Contents::Inline { len, bytes } if usize::from(*len) + add.len() <= bytes.len() => {
                let at = usize::from(*len);
                bytes[at..at + add.len()].copy_from_slice(add);
                *len += add.len() as u8;
            }
            _ => {
                let mut all = self.as_bytes().to_vec();
                all.extend_from_slice(add);
                *self = Contents::Heap(all.into_boxed_slice());
            }
        }
    }
}

impl std::fmt::Debug for Contents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(self.as_bytes()))
    }
}

/// mosh's `Cell`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    contents: Contents,
    renditions: Renditions,
    wide: bool,
    /// The cell starts with a combining character.
    fallback: bool,
    /// The row wraps here (meaningful on a row's last cell).
    wrap: bool,
}

impl Cell {
    fn blank(bg: u32) -> Self {
        Self {
            contents: Contents::default(),
            renditions: Renditions::with_background(bg),
            wide: false,
            fallback: false,
            wrap: false,
        }
    }

    fn reset(&mut self, bg: u32) {
        *self = Self::blank(bg);
    }

    fn width(&self) -> usize {
        if self.wide { 2 } else { 1 }
    }

    /// The cell's text (empty for a blank cell).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(self.contents.as_bytes()).into_owned()
    }

    /// Same look: a blank and a printed space are one (mosh's `is_blank`).
    fn looks_like(&self, other: &Cell) -> bool {
        fn text(cell: &Cell) -> &[u8] {
            match cell.contents.as_bytes() {
                b"" => b" ",
                bytes => bytes,
            }
        }
        text(self) == text(other)
            && self.renditions == other.renditions
            && self.wide == other.wide
            && self.wrap == other.wrap
            && self.fallback == other.fallback
    }

    /// mosh's `print_grapheme`.
    fn print(&self, out: &mut Vec<u8>) {
        if self.contents.is_empty() {
            out.push(b' ');
            return;
        }
        if self.fallback {
            out.extend_from_slice("\u{a0}".as_bytes());
        }
        out.extend_from_slice(self.contents.as_bytes());
    }
}

/// mosh's `Row`: its cells and generation.
#[derive(Debug, Clone)]
pub struct Row {
    cells: Vec<Cell>,
    generation: u64,
}

impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation && self.cells == other.cells
    }
}

impl Row {
    fn new(width: usize, bg: u32) -> Self {
        Self {
            cells: vec![Cell::blank(bg); width],
            generation: next_generation(),
        }
    }

    fn reset(&mut self, bg: u32) {
        self.generation = next_generation();
        for cell in &mut self.cells {
            cell.reset(bg);
        }
    }

    fn wraps(&self) -> bool {
        self.cells.last().is_some_and(|cell| cell.wrap)
    }

    fn set_wrap(&mut self, wrap: bool) {
        if let Some(cell) = self.cells.last_mut() {
            cell.wrap = wrap;
        }
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }
}

/// mosh's `DrawState`, the fields its renderer and these host frames use.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DrawState {
    width: usize,
    height: usize,
    cursor_col: i64,
    cursor_row: i64,
    combining_col: i64,
    combining_row: i64,
    tabs: Vec<bool>,
    default_tabs: bool,
    scroll_top: i64,
    scroll_bottom: i64,
    renditions: Renditions,
    next_print_will_wrap: bool,
    origin_mode: bool,
    auto_wrap_mode: bool,
    cursor_visible: bool,
    reverse_video: bool,
    bracketed_paste: bool,
    mouse_reporting_mode: u16,
    mouse_focus_event: bool,
    mouse_alternate_scroll: bool,
    mouse_encoding_mode: u16,
    application_cursor_keys: bool,
}

impl DrawState {
    fn new(width: usize, height: usize) -> Self {
        let mut state = Self {
            width,
            height,
            cursor_col: 0,
            cursor_row: 0,
            combining_col: 0,
            combining_row: 0,
            tabs: vec![false; width],
            default_tabs: true,
            scroll_top: 0,
            scroll_bottom: height as i64 - 1,
            renditions: Renditions::default(),
            next_print_will_wrap: false,
            origin_mode: false,
            auto_wrap_mode: true,
            cursor_visible: true,
            reverse_video: false,
            bracketed_paste: false,
            mouse_reporting_mode: 0,
            mouse_focus_event: false,
            mouse_alternate_scroll: false,
            mouse_encoding_mode: 0,
            application_cursor_keys: false,
        };
        state.reinitialize_tabs(0);
        state
    }

    fn reinitialize_tabs(&mut self, start: usize) {
        for (i, tab) in self.tabs.iter_mut().enumerate().skip(start) {
            *tab = i % 8 == 0;
        }
    }

    fn limit_top(&self) -> i64 {
        if self.origin_mode { self.scroll_top } else { 0 }
    }

    fn limit_bottom(&self) -> i64 {
        if self.origin_mode {
            self.scroll_bottom
        } else {
            self.height as i64 - 1
        }
    }

    fn new_grapheme(&mut self) {
        self.combining_col = self.cursor_col;
        self.combining_row = self.cursor_row;
    }

    fn snap_cursor_to_border(&mut self) {
        self.cursor_row = self.cursor_row.clamp(self.limit_top(), self.limit_bottom());
        self.cursor_col = self.cursor_col.clamp(0, self.width as i64 - 1);
    }

    fn move_row(&mut self, n: i64, relative: bool) {
        if relative {
            self.cursor_row += n;
        } else {
            self.cursor_row = n + self.limit_top();
        }
        self.snap_cursor_to_border();
        self.new_grapheme();
        self.next_print_will_wrap = false;
    }

    fn move_col(&mut self, n: i64, relative: bool, implicit: bool) {
        if implicit {
            self.new_grapheme();
        }
        if relative {
            self.cursor_col += n;
        } else {
            self.cursor_col = n;
        }
        if implicit {
            self.next_print_will_wrap = self.cursor_col >= self.width as i64;
        }
        self.snap_cursor_to_border();
        if !implicit {
            self.new_grapheme();
            self.next_print_will_wrap = false;
        }
    }

    fn set_scrolling_region(&mut self, top: i64, bottom: i64) {
        self.scroll_top = top.max(0);
        self.scroll_bottom = bottom.min(self.height as i64 - 1).max(self.scroll_top);
        if self.origin_mode {
            self.snap_cursor_to_border();
            self.new_grapheme();
        }
    }

    fn resize(&mut self, width: usize, height: usize) {
        if width != self.width || height != self.height {
            self.scroll_top = 0;
            self.scroll_bottom = height as i64 - 1;
        }
        let old_width = self.width;
        self.tabs.resize(width, false);
        if self.default_tabs {
            self.reinitialize_tabs(old_width);
        }
        self.width = width;
        self.height = height;
        self.snap_cursor_to_border();
        if self.combining_col >= width as i64 || self.combining_row >= height as i64 {
            self.combining_col = -1;
            self.combining_row = -1;
        }
    }

    fn next_tab(&self) -> Option<i64> {
        ((self.cursor_col + 1).max(0) as usize..self.width)
            .find(|&i| self.tabs[i])
            .map(|i| i as i64)
    }
}

/// One mosh server state's screen: what mosh-client's `Terminal::Complete`
/// keeps of it.
#[derive(Debug, Clone)]
pub struct Framebuffer {
    rows: Vec<Arc<Row>>,
    ds: DrawState,
    icon_name: String,
    window_title: String,
    clipboard: String,
    bell_count: u32,
    title_initialized: bool,
}

impl Framebuffer {
    /// A blank screen, as mosh-server's and mosh-client's state 0.
    pub fn new(width: usize, height: usize) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        // One blank row shared by every line, as mosh starts.
        let blank = Arc::new(Row::new(width, 0));
        Self {
            rows: std::iter::repeat_n(blank, height).collect(),
            ds: DrawState::new(width, height),
            icon_name: String::new(),
            window_title: String::new(),
            clipboard: String::new(),
            bell_count: 0,
            title_initialized: false,
        }
    }

    pub fn width(&self) -> usize {
        self.ds.width
    }

    pub fn height(&self) -> usize {
        self.ds.height
    }

    /// Cursor (column, row) and whether it is shown.
    pub fn cursor(&self) -> (usize, usize, bool) {
        (
            self.ds.cursor_col as usize,
            self.ds.cursor_row as usize,
            self.ds.cursor_visible,
        )
    }

    pub fn row(&self, row: usize) -> &Row {
        &self.rows[row]
    }

    /// Row `row` as text, blank cells as spaces, wide cells once.
    pub fn row_text(&self, row: usize) -> String {
        let mut text = String::new();
        let cells = &self.rows[row].cells;
        let mut col = 0;
        while col < cells.len() {
            let cell = &cells[col];
            if cell.contents.is_empty() {
                text.push(' ');
            } else {
                text.push_str(&cell.text());
            }
            col += cell.width();
        }
        text
    }

    pub fn window_title(&self) -> &str {
        &self.window_title
    }

    /// The rows this screen holds: shared with other screens where they
    /// have not changed since.
    pub fn rows(&self) -> &[Arc<Row>] {
        &self.rows
    }

    fn bg(&self) -> u32 {
        self.ds.renditions.bg
    }

    fn new_row(&self) -> Arc<Row> {
        Arc::new(Row::new(self.ds.width, self.bg()))
    }

    fn row_mut(&mut self, row: i64) -> &mut Row {
        let row = if row == -1 { self.ds.cursor_row } else { row };
        Arc::make_mut(&mut self.rows[row as usize])
    }

    fn cell_mut(&mut self, row: i64, col: i64) -> &mut Cell {
        let col = if col == -1 { self.ds.cursor_col } else { col };
        &mut self.row_mut(row).cells[col as usize]
    }

    fn reset_cell(&mut self, row: i64, col: i64) {
        let bg = self.bg();
        self.cell_mut(row, col).reset(bg);
    }

    fn reset_row(&mut self, row: i64) {
        let bg = self.bg();
        self.row_mut(row).reset(bg);
    }

    fn clear_line(&mut self, row: i64, start: i64, end: i64) {
        for col in start..=end {
            self.reset_cell(row, col);
        }
    }

    /// mosh's `Framebuffer::resize`.
    pub fn resize(&mut self, width: usize, height: usize) {
        if width == 0 || height == 0 || width.saturating_mul(height) > MAX_CELLS {
            return;
        }
        let (old_width, old_height) = (self.ds.width, self.ds.height);
        self.ds.resize(width, height);
        let blank = self.new_row();
        if old_height != height {
            self.rows.resize(height, blank.clone());
        }
        if old_width == width {
            return;
        }
        let bg = self.bg();
        for row in &mut self.rows {
            if Arc::ptr_eq(row, &blank) {
                break;
            }
            let mut copy = Row::clone(row);
            copy.set_wrap(false);
            copy.cells.resize(width, Cell::blank(bg));
            *row = Arc::new(copy);
        }
    }

    fn scroll(&mut self, n: i64) {
        if n >= 0 {
            self.delete_line(self.ds.scroll_top, n);
        } else {
            self.insert_line(self.ds.scroll_top, -n);
        }
    }

    fn insert_line(&mut self, before: i64, count: i64) {
        if before < self.ds.scroll_top || before > self.ds.scroll_bottom + 1 {
            return;
        }
        let scroll = (self.ds.scroll_bottom + 1 - before).min(count);
        if scroll <= 0 {
            return;
        }
        let start = (self.ds.scroll_bottom + 1 - scroll) as usize;
        self.rows.drain(start..start + scroll as usize);
        let blank = self.new_row();
        let at = before as usize;
        self.rows
            .splice(at..at, std::iter::repeat_n(blank, scroll as usize));
    }

    fn delete_line(&mut self, row: i64, count: i64) {
        if row < self.ds.scroll_top || row > self.ds.scroll_bottom {
            return;
        }
        let scroll = (self.ds.scroll_bottom + 1 - row).min(count);
        if scroll <= 0 {
            return;
        }
        let at = row as usize;
        self.rows.drain(at..at + scroll as usize);
        let blank = self.new_row();
        let start = (self.ds.scroll_bottom + 1 - scroll) as usize;
        self.rows
            .splice(start..start, std::iter::repeat_n(blank, scroll as usize));
    }

    fn move_rows_autoscroll(&mut self, rows: i64) {
        let row = self.ds.cursor_row;
        if row < self.ds.scroll_top || row > self.ds.scroll_bottom {
            self.ds.move_row(rows, true);
            return;
        }
        if row + rows > self.ds.scroll_bottom {
            let n = row + rows - self.ds.scroll_bottom;
            self.scroll(n);
            self.ds.move_row(-n, true);
        } else if row + rows < self.ds.scroll_top {
            let n = row + rows - self.ds.scroll_top;
            self.scroll(n);
            self.ds.move_row(-n, true);
        }
        self.ds.move_row(rows, true);
    }

    /// Interprets host bytes (mosh-server's `new_frame` output) as mosh's
    /// emulator does. A host instruction always ends on a whole sequence,
    /// so each call starts in the ground state.
    pub fn apply(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let mut parser = Parser::Ground;
        for ch in text.chars() {
            parser = self.step(parser, ch);
        }
    }

    fn step(&mut self, parser: Parser, ch: char) -> Parser {
        let code = ch as u32;
        // C0 controls execute in every state but a string's (where BEL
        // and ESC end it).
        match parser {
            Parser::Osc(mut text) => {
                return match ch {
                    '\u{7}' => {
                        self.osc(&text);
                        Parser::Ground
                    }
                    '\u{1b}' => Parser::OscEsc(text),
                    _ if code < 0x20 => Parser::Osc(text),
                    _ => {
                        if text.chars().count() < MAX_OSC {
                            text.push(ch);
                        }
                        Parser::Osc(text)
                    }
                };
            }
            Parser::OscEsc(text) => {
                if ch == '\\' {
                    self.osc(&text);
                    return Parser::Ground;
                }
                // Anything but ST cancels the string and starts an escape.
                return self.step(Parser::Esc(String::new()), ch);
            }
            _ => {}
        }
        if ch == '\u{1b}' {
            return Parser::Esc(String::new());
        }
        if code < 0x20 || code == 0x7f {
            if code != 0x7f {
                self.control(ch);
            }
            return parser;
        }
        match parser {
            Parser::Ground => {
                self.print(ch);
                Parser::Ground
            }
            Parser::Esc(mut collected) => match ch {
                '[' if collected.is_empty() => Parser::Csi(String::new(), String::new()),
                ']' if collected.is_empty() => Parser::Osc(String::new()),
                '\u{20}'..='\u{2f}' => {
                    collected.push(ch);
                    Parser::Esc(collected)
                }
                _ => {
                    // An escape these frames never use (DECSC, RIS, ...):
                    // as mosh, unknown functions only clear the wrap state.
                    self.ds.next_print_will_wrap = false;
                    Parser::Ground
                }
            },
            Parser::Csi(mut params, mut collected) => match ch {
                '0'..='9' | ';' | ':' => {
                    params.push(ch);
                    Parser::Csi(params, collected)
                }
                '<' | '=' | '>' | '?' | '\u{20}'..='\u{2f}' => {
                    collected.push(ch);
                    Parser::Csi(params, collected)
                }
                '\u{40}'..='\u{7e}' => {
                    collected.push(ch);
                    self.csi(&params, &collected);
                    Parser::Ground
                }
                _ => Parser::Ground,
            },
            Parser::Osc(_) | Parser::OscEsc(_) => unreachable!(),
        }
    }

    fn control(&mut self, ch: char) {
        match ch {
            '\u{7}' => {
                self.ds.next_print_will_wrap = false;
                self.bell_count = self.bell_count.wrapping_add(1);
            }
            '\u{8}' => {
                self.ds.next_print_will_wrap = false;
                self.ds.move_col(-1, true, false);
            }
            '\t' => {
                // mosh's HT keeps the wrap state.
                let col = self.ds.next_tab().unwrap_or(self.ds.width as i64 - 1);
                let wrap = self.ds.next_print_will_wrap;
                self.ds.move_col(col, false, false);
                self.ds.next_print_will_wrap = wrap;
            }
            '\n' | '\u{b}' | '\u{c}' => {
                self.ds.next_print_will_wrap = false;
                self.move_rows_autoscroll(1);
            }
            '\r' => {
                self.ds.next_print_will_wrap = false;
                self.ds.move_col(0, false, false);
            }
            _ => self.ds.next_print_will_wrap = false,
        }
    }

    fn csi(&mut self, params: &str, key: &str) {
        let parsed: Vec<i64> = params
            .split([';', ':'])
            .map(|p| p.parse::<i64>().ok().filter(|&v| v <= 65_535).unwrap_or(-1))
            .collect();
        let param = |n: usize, default: i64| -> i64 {
            parsed
                .get(n)
                .copied()
                .filter(|&v| v >= 1)
                .unwrap_or(default)
        };
        // SGR and the private modes keep the wrap state; everything else,
        // known or not, clears it.
        if !matches!(key, "m" | "?h" | "?l") {
            self.ds.next_print_will_wrap = false;
        }
        match key {
            "A" => self.ds.move_row(-param(0, 1), true),
            "B" => self.ds.move_row(param(0, 1), true),
            "C" => self.ds.move_col(param(0, 1), true, false),
            "D" => self.ds.move_col(-param(0, 1), true, false),
            "G" | "`" => self.ds.move_col(param(0, 1) - 1, false, false),
            "d" => self.ds.move_row(param(0, 1) - 1, false),
            "H" | "f" => {
                self.ds.move_row(param(0, 1) - 1, false);
                self.ds.move_col(param(1, 1) - 1, false, false);
            }
            "J" => match param(0, 0) {
                0 => {
                    let col = self.ds.cursor_col;
                    self.clear_line(-1, col, self.ds.width as i64 - 1);
                    for y in self.ds.cursor_row + 1..self.ds.height as i64 {
                        self.reset_row(y);
                    }
                }
                1 => {
                    for y in 0..self.ds.cursor_row {
                        self.reset_row(y);
                    }
                    let col = self.ds.cursor_col;
                    self.clear_line(-1, 0, col);
                }
                2 => {
                    for y in 0..self.ds.height as i64 {
                        self.reset_row(y);
                    }
                }
                _ => {}
            },
            "K" => match param(0, 0) {
                0 => {
                    let col = self.ds.cursor_col;
                    self.clear_line(-1, col, self.ds.width as i64 - 1);
                }
                1 => {
                    let col = self.ds.cursor_col;
                    self.clear_line(-1, 0, col);
                }
                2 => self.reset_row(-1),
                _ => {}
            },
            "X" => {
                let col = self.ds.cursor_col;
                let limit = (col + param(0, 1) - 1).min(self.ds.width as i64 - 1);
                self.clear_line(-1, col, limit);
            }
            "m" => self.sgr(&parsed),
            "r" => {
                let height = self.ds.height as i64;
                let top = param(0, 1);
                let bottom = param(1, height);
                if bottom <= top || top > height || (top == 0 && bottom == 1) {
                    return;
                }
                self.ds.set_scrolling_region(top - 1, bottom - 1);
                self.ds.move_row(0, false);
                self.ds.move_col(0, false, false);
            }
            "?h" | "?l" => {
                let on = key == "?h";
                for n in 0..parsed.len() {
                    self.private_mode(param(n, 0), on);
                }
            }
            _ => {}
        }
    }

    fn sgr(&mut self, parsed: &[i64]) {
        let param = |n: usize, default: i64| -> i64 {
            parsed
                .get(n)
                .copied()
                .filter(|&v| v >= 1)
                .unwrap_or(default)
        };
        let count = parsed.len();
        let mut i = 0;
        while i < count {
            let rendition = param(i, 0);
            let base = if rendition == 38 { 30 } else { 40 };
            if (rendition == 38 || rendition == 48) && count - i >= 3 && param(i + 1, -1) == 5 {
                if let Some(color) = Renditions::color(param(i + 2, 0), base) {
                    self.set_color(rendition, color);
                }
                i += 3;
                continue;
            }
            if (rendition == 38 || rendition == 48) && count - i >= 5 && param(i + 1, -1) == 2 {
                let channel = |n: usize| (param(n, 0) as u32) & 0xff;
                let color =
                    TRUE_COLOR | (channel(i + 2) << 16) | (channel(i + 3) << 8) | channel(i + 4);
                self.set_color(rendition, color);
                i += 5;
                continue;
            }
            self.ds.renditions.set_rendition(rendition);
            i += 1;
        }
    }

    fn set_color(&mut self, rendition: i64, color: u32) {
        if rendition == 38 {
            self.ds.renditions.fg = color;
        } else {
            self.ds.renditions.bg = color;
        }
    }

    fn private_mode(&mut self, mode: i64, on: bool) {
        match mode {
            9 | 1000..=1003 => self.ds.mouse_reporting_mode = if on { mode as u16 } else { 0 },
            1005 | 1006 | 1015 => self.ds.mouse_encoding_mode = if on { mode as u16 } else { 0 },
            1 => self.ds.application_cursor_keys = on,
            3 => {
                self.ds.move_row(0, false);
                self.ds.move_col(0, false, false);
                for y in 0..self.ds.height as i64 {
                    self.reset_row(y);
                }
            }
            5 => self.ds.reverse_video = on,
            6 => {
                self.ds.move_row(0, false);
                self.ds.move_col(0, false, false);
                self.ds.origin_mode = on;
            }
            7 => self.ds.auto_wrap_mode = on,
            25 => self.ds.cursor_visible = on,
            1004 => self.ds.mouse_focus_event = on,
            1007 => self.ds.mouse_alternate_scroll = on,
            2004 => self.ds.bracketed_paste = on,
            _ => {}
        }
    }

    fn osc(&mut self, text: &str) {
        if let Some(clipboard) = text.strip_prefix("52;c;") {
            self.clipboard = clipboard.to_owned();
            return;
        }
        let mut chars = text.chars();
        let (command, rest) = match (chars.next(), chars.next()) {
            (Some(';'), _) => (0, &text[1..]),
            (Some(digit @ '0'..='9'), Some(';')) => (digit as u32 - '0' as u32, &text[2..]),
            _ => return,
        };
        let (icon, title) = (command == 0 || command == 1, command == 0 || command == 2);
        if !icon && !title {
            return;
        }
        self.title_initialized = true;
        // mosh keeps the first 256 characters of the whole string.
        let keep = MAX_TITLE.saturating_sub(text.len() - rest.len());
        let value: String = rest.chars().take(keep).collect();
        if icon {
            self.icon_name = value.clone();
        }
        if title {
            self.window_title = value;
        }
    }

    /// mosh's `Emulator::print`.
    fn print(&mut self, ch: char) {
        let width = char_width(ch);
        match width {
            1 | 2 => {
                let last_col = self.ds.width as i64 - 1;
                if self.ds.auto_wrap_mode && self.ds.next_print_will_wrap {
                    self.row_mut(-1).set_wrap(true);
                    self.ds.move_col(0, false, false);
                    self.move_rows_autoscroll(1);
                } else if self.ds.auto_wrap_mode && width == 2 && self.ds.cursor_col == last_col {
                    self.reset_cell(-1, -1);
                    self.row_mut(-1).set_wrap(false);
                    self.ds.move_col(0, false, false);
                    self.move_rows_autoscroll(1);
                }
                let renditions = self.ds.renditions;
                let bg = self.bg();
                let cell = self.cell_mut(-1, -1);
                cell.reset(bg);
                cell.contents.push(ch);
                cell.wide = width == 2;
                cell.renditions = renditions;
                if width == 2 && self.ds.cursor_col + 1 < self.ds.width as i64 {
                    let next = self.ds.cursor_col + 1;
                    self.reset_cell(-1, next);
                }
                self.ds.move_col(width, true, true);
            }
            0 => {
                let (col, row) = (self.ds.combining_col, self.ds.combining_row);
                if col < 0 || row < 0 || col >= self.ds.width as i64 || row >= self.ds.height as i64
                {
                    return;
                }
                let empty = self.cell_mut(row, col).contents.is_empty();
                if empty {
                    self.cell_mut(row, col).fallback = true;
                    self.ds.move_col(1, true, true);
                }
                let cell = self.cell_mut(row, col);
                if cell.contents.as_bytes().len() < MAX_CELL_BYTES {
                    cell.contents.push(ch);
                }
            }
            _ => {}
        }
    }
}

enum Parser {
    Ground,
    /// Intermediates collected.
    Esc(String),
    /// Parameters, then private markers and intermediates.
    Csi(String, String),
    Osc(String),
    /// ESC inside a string: ST if `\` follows.
    OscEsc(String),
}

/// mosh's width rule: printable ISO 8859-1 is narrow, otherwise
/// `wcwidth` (here Unicode's East Asian width); -1 for what does not print.
fn char_width(ch: char) -> i64 {
    let code = ch as u32;
    if ch == '\0' {
        -1
    } else if (0x20..=0x7e).contains(&code) || (0xa0..=0xff).contains(&code) {
        1
    } else {
        ch.width().map_or(-1, |w| w as i64)
    }
}

/// mosh's `Display::new_frame` with mosh-server's settings (ECH, BCE and
/// titles on): the bytes that repaint a terminal showing `last` into one
/// showing `f`. `initialized` false repaints everything (clears first).
pub fn new_frame(initialized: bool, last: &Framebuffer, f: &Framebuffer) -> Vec<u8> {
    let mut frame = FrameState {
        out: Vec::with_capacity(256),
        cursor_x: 0,
        cursor_y: 0,
        rendition: Renditions::default(),
        cursor_visible: last.ds.cursor_visible,
    };
    let mut initialized = initialized;

    if f.bell_count != last.bell_count {
        frame.out.push(0x07);
    }
    if f.title_initialized
        && (!initialized || f.icon_name != last.icon_name || f.window_title != last.window_title)
    {
        if f.icon_name == f.window_title {
            frame.append(b"\x1b]0;");
            frame.append(f.window_title.as_bytes());
            frame.out.push(0x07);
        } else {
            frame.append(b"\x1b]1;");
            frame.append(f.icon_name.as_bytes());
            frame.out.push(0x07);
            frame.append(b"\x1b]2;");
            frame.append(f.window_title.as_bytes());
            frame.out.push(0x07);
        }
    }
    if f.clipboard != last.clipboard {
        frame.append(b"\x1b]52;c;");
        frame.append(f.clipboard.as_bytes());
        frame.out.push(0x07);
    }
    if !initialized || f.ds.reverse_video != last.ds.reverse_video {
        frame.append(if f.ds.reverse_video {
            b"\x1b[?5h"
        } else {
            b"\x1b[?5l"
        });
    }
    let (width, height) = (f.ds.width, f.ds.height);
    if !initialized || width != last.ds.width || height != last.ds.height {
        frame.append(b"\x1b[r");
        frame.append(b"\x1b[0m\x1b[H\x1b[2J");
        initialized = false;
        frame.cursor_x = 0;
        frame.cursor_y = 0;
        frame.rendition = Renditions::default();
    } else {
        frame.cursor_x = last.ds.cursor_col;
        frame.cursor_y = last.ds.cursor_row;
        frame.rendition = last.ds.renditions;
    }
    if !initialized {
        frame.cursor_visible = false;
        frame.append(b"\x1b[?25l");
    }

    let mut frame_y = 0;
    let mut rows = last.rows.clone();
    if last.ds.width < width {
        let bg = f.bg();
        for row in &mut rows {
            let mut copy = Row::clone(row);
            copy.cells.resize(width, Cell::blank(bg));
            *row = Arc::new(copy);
        }
    }
    let mut blank_row: Option<Arc<Row>> = None;
    if rows.len() < height {
        let blank = Arc::new(Row::new(width, 0));
        rows.resize(height, blank.clone());
        blank_row = Some(blank);
    }

    // Has the display moved up by some lines?
    if initialized {
        let mut lines_scrolled = 0;
        let mut scroll_height = 0;
        for row in 0..height {
            let new_row = &f.rows[0];
            let old_row = &rows[row];
            if !(Arc::ptr_eq(new_row, old_row) || **new_row == **old_row) {
                continue;
            }
            if row == 0 {
                break;
            }
            lines_scrolled = row;
            scroll_height = 1;
            let mut region = 1;
            while lines_scrolled + region < height {
                if *f.rows[region] == *rows[lines_scrolled + region] {
                    scroll_height = region + 1;
                } else {
                    break;
                }
                region += 1;
            }
            break;
        }
        if scroll_height > 0 {
            frame_y = scroll_height;
            if lines_scrolled > 0 {
                let blank = blank_row
                    .get_or_insert_with(|| Arc::new(Row::new(width, 0)))
                    .clone();
                frame.update_rendition(Renditions::default(), true);
                let top = 0;
                let bottom = top + lines_scrolled + scroll_height - 1;
                if scroll_height + lines_scrolled == height && frame.cursor_y + 1 == height as i64 {
                    frame.out.push(b'\r');
                    frame.out.extend(std::iter::repeat_n(b'\n', lines_scrolled));
                    frame.cursor_x = 0;
                } else {
                    frame.append(format!("\x1b[{};{}r", top + 1, bottom + 1).as_bytes());
                    frame.cursor_x = -1;
                    frame.cursor_y = -1;
                    frame.silent_move(bottom as i64, 0);
                    frame.out.extend(std::iter::repeat_n(b'\n', lines_scrolled));
                    frame.append(b"\x1b[r");
                    frame.cursor_x = -1;
                    frame.cursor_y = -1;
                }
                for i in top..=bottom {
                    rows[i] = if i + lines_scrolled <= bottom {
                        rows[i + lines_scrolled].clone()
                    } else {
                        blank.clone()
                    };
                }
            }
        }
    }

    let mut wrap = false;
    while frame_y < height {
        wrap = put_row(initialized, &mut frame, f, frame_y, &rows[frame_y], wrap);
        frame_y += 1;
    }

    // Not in mosh: after the last cell of the last row the cursor sits on
    // the last column, not past it, so a backspace from "past it" would
    // land one column short. Position it explicitly.
    if frame.cursor_x >= width as i64 {
        frame.cursor_x = -1;
        frame.cursor_y = -1;
    }
    if !initialized || f.ds.cursor_row != frame.cursor_y || f.ds.cursor_col != frame.cursor_x {
        frame.move_to(f.ds.cursor_row, f.ds.cursor_col);
    }
    if !initialized || f.ds.cursor_visible != frame.cursor_visible {
        frame.append(if f.ds.cursor_visible {
            b"\x1b[?25h"
        } else {
            b"\x1b[?25l"
        });
    }
    frame.update_rendition(f.ds.renditions, !initialized);
    if !initialized || f.ds.bracketed_paste != last.ds.bracketed_paste {
        frame.append(if f.ds.bracketed_paste {
            b"\x1b[?2004h"
        } else {
            b"\x1b[?2004l"
        });
    }
    if !initialized || f.ds.mouse_reporting_mode != last.ds.mouse_reporting_mode {
        if f.ds.mouse_reporting_mode == 0 {
            frame.append(b"\x1b[?1003l\x1b[?1002l\x1b[?1001l\x1b[?1000l");
        } else {
            if last.ds.mouse_reporting_mode != 0 {
                frame.append(format!("\x1b[?{}l", last.ds.mouse_reporting_mode).as_bytes());
            }
            frame.append(format!("\x1b[?{}h", f.ds.mouse_reporting_mode).as_bytes());
        }
    }
    if !initialized || f.ds.mouse_focus_event != last.ds.mouse_focus_event {
        frame.append(if f.ds.mouse_focus_event {
            b"\x1b[?1004h"
        } else {
            b"\x1b[?1004l"
        });
    }
    if !initialized || f.ds.mouse_encoding_mode != last.ds.mouse_encoding_mode {
        if f.ds.mouse_encoding_mode == 0 {
            frame.append(b"\x1b[?1015l\x1b[?1006l\x1b[?1005l");
        } else {
            if last.ds.mouse_encoding_mode != 0 {
                frame.append(format!("\x1b[?{}l", last.ds.mouse_encoding_mode).as_bytes());
            }
            frame.append(format!("\x1b[?{}h", f.ds.mouse_encoding_mode).as_bytes());
        }
    }
    frame.out
}

/// mosh's `Display::put_row`.
fn put_row(
    initialized: bool,
    frame: &mut FrameState,
    f: &Framebuffer,
    frame_y: usize,
    old_row: &Arc<Row>,
    wrap: bool,
) -> bool {
    let mut frame_x = 0;
    let row_ptr = &f.rows[frame_y];
    let cells = &row_ptr.cells;
    let old_cells = &old_row.cells;
    let y = frame_y as i64;

    if wrap {
        let cell = &cells[0];
        frame.update_rendition(cell.renditions, false);
        cell.print(&mut frame.out);
        frame_x += cell.width();
        frame.cursor_x += cell.width() as i64;
    }
    if initialized && Arc::ptr_eq(row_ptr, old_row) {
        return false;
    }

    let wrap_this = row_ptr.wraps();
    let row_width = f.ds.width;
    let mut clear_count = 0usize;
    let mut wrote_last_cell = false;
    let mut blank_renditions = Renditions::default();

    while frame_x < row_width {
        let cell = &cells[frame_x];
        if initialized && clear_count == 0 && old_cells.get(frame_x) == Some(cell) {
            frame_x += cell.width();
            continue;
        }
        if cell.contents.is_empty() {
            if clear_count == 0 {
                blank_renditions = cell.renditions;
            }
            if cell.renditions == blank_renditions {
                clear_count += 1;
                frame_x += 1;
                continue;
            }
        }
        if clear_count > 0 {
            frame.silent_move(y, (frame_x - clear_count) as i64);
            frame.update_rendition(blank_renditions, false);
            // BCE: erasing fills with the current background.
            if clear_count > 4 {
                frame.append(format!("\x1b[{clear_count}X").as_bytes());
            } else {
                frame.out.extend(std::iter::repeat_n(b' ', clear_count));
                frame.cursor_x = frame_x as i64;
            }
            clear_count = 0;
            if cell.contents.is_empty() {
                blank_renditions = cell.renditions;
                clear_count = 1;
                frame_x += 1;
                continue;
            }
        }
        let cell_width = cell.width();
        if wrap_this && frame_x + cell_width >= row_width {
            frame.cursor_x = -1;
            frame.cursor_y = -1;
        }
        frame.silent_move(y, frame_x as i64);
        frame.update_rendition(cell.renditions, false);
        cell.print(&mut frame.out);
        frame_x += cell_width;
        frame.cursor_x += cell_width as i64;
        if frame_x >= row_width {
            wrote_last_cell = true;
        }
    }

    if clear_count > 0 {
        frame.silent_move(y, (frame_x - clear_count) as i64);
        frame.update_rendition(blank_renditions, false);
        if !wrap_this {
            frame.append(b"\x1b[K");
        } else {
            frame.out.extend(std::iter::repeat_n(b' ', clear_count));
            frame.cursor_x = frame_x as i64;
            wrote_last_cell = true;
        }
    }

    if !(wrote_last_cell && frame_y < f.ds.height - 1) {
        return false;
    }
    if wrap_this {
        frame.cursor_x = 0;
        frame.cursor_y += 1;
        return true;
    }
    frame.append(b"\r\n");
    frame.cursor_x = 0;
    frame.cursor_y += 1;
    false
}

/// mosh's `FrameState`: the bytes so far and what the terminal they go to
/// has (cursor, -1 when unknown; rendition; cursor visibility).
struct FrameState {
    out: Vec<u8>,
    cursor_x: i64,
    cursor_y: i64,
    rendition: Renditions,
    cursor_visible: bool,
}

impl FrameState {
    fn append(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    fn silent_move(&mut self, y: i64, x: i64) {
        if self.cursor_x == x && self.cursor_y == y {
            return;
        }
        if self.cursor_visible {
            self.append(b"\x1b[?25l");
            self.cursor_visible = false;
        }
        self.move_to(y, x);
    }

    fn move_to(&mut self, y: i64, x: i64) {
        let (last_x, last_y) = (self.cursor_x, self.cursor_y);
        self.cursor_x = x;
        self.cursor_y = y;
        if last_x != -1 && last_y != -1 {
            if x == 0 && y - last_y >= 0 && y - last_y < 5 {
                if last_x != 0 {
                    self.out.push(b'\r');
                }
                self.out
                    .extend(std::iter::repeat_n(b'\n', (y - last_y) as usize));
                return;
            }
            if y == last_y && x - last_x < 0 && x - last_x > -5 {
                self.out
                    .extend(std::iter::repeat_n(b'\x08', (last_x - x) as usize));
                return;
            }
        }
        self.append(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
    }

    fn update_rendition(&mut self, r: Renditions, force: bool) {
        if force || self.rendition != r {
            r.sgr(&mut self.out);
            self.rendition = r;
        }
    }
}

/// Approximate heap bytes of the rows `screens` hold, each shared row
/// counted once: what holding them all costs.
pub fn held_bytes<'a>(screens: impl IntoIterator<Item = &'a Framebuffer>) -> usize {
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0;
    for screen in screens {
        bytes += screen.rows.len() * std::mem::size_of::<Arc<Row>>()
            + std::mem::size_of::<Framebuffer>();
        for row in &screen.rows {
            if seen.insert(Arc::as_ptr(row)) {
                bytes += std::mem::size_of::<Row>()
                    + row.cells.len() * std::mem::size_of::<Cell>()
                    + row
                        .cells
                        .iter()
                        .map(|cell| match &cell.contents {
                            Contents::Heap(heap) => heap.len(),
                            Contents::Inline { .. } => 0,
                        })
                        .sum::<usize>();
            }
        }
    }
    bytes
}

impl Framebuffer {
    /// Whether a terminal showing `self` looks like one showing `other`:
    /// the cells, the cursor, the current rendition, the modes, titles and
    /// clipboard (not where combining characters would go, nor a pending
    /// wrap, which mosh's renderer does not carry either; a blank cell
    /// looks like a printed space).
    pub fn looks_like(&self, other: &Framebuffer) -> bool {
        let (a, b) = (&self.ds, &other.ds);
        a.width == b.width
            && a.height == b.height
            && a.cursor_col == b.cursor_col
            && a.cursor_row == b.cursor_row
            && a.cursor_visible == b.cursor_visible
            && a.renditions == b.renditions
            && a.reverse_video == b.reverse_video
            && a.bracketed_paste == b.bracketed_paste
            && a.mouse_reporting_mode == b.mouse_reporting_mode
            && a.mouse_focus_event == b.mouse_focus_event
            && a.mouse_encoding_mode == b.mouse_encoding_mode
            && self.window_title == other.window_title
            && self.icon_name == other.icon_name
            && self.clipboard == other.clipboard
            && self
                .rows
                .iter()
                .zip(&other.rows)
                .all(|(x, y)| row_looks_like(&x.cells, &y.cells))
    }
}

/// The cells of two rows look the same. The right half of a wide cell is
/// covered by it: what is stored there does not show (mosh keeps a cell
/// written onto it, and its renderer skips it).
fn row_looks_like(a: &[Cell], b: &[Cell]) -> bool {
    let mut col = 0;
    while col < a.len().min(b.len()) {
        if !a[col].looks_like(&b[col]) {
            return false;
        }
        col += a[col].width();
    }
    true
}
