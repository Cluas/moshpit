//! mosh's predictive local echo as an overlay on the core's frames: show
//! what a keystroke will most likely do to the screen now, instead of one
//! round trip later (docs/nsh/TERMINAL-VIEW.md, the mosh prediction row).
//!
//! A port of the app's `MoshPrediction` (itself a port of mosh's
//! `Overlay::PredictionEngine`, `src/frontend/terminaloverlay.cc`), with one
//! structural change: the guesses are never written into the emulator. The
//! emulator's grid stays exactly what the server sent — mosh-server diffs
//! against what it believes this screen shows, so a guess fed over would
//! stay forever (the diff-peer contract) — and the guesses are laid over the
//! *frame* the view draws. The Swift engine's withdraw → feed → settle →
//! apply cycle around every feed is gone with it: a feed is a feed, judging
//! follows it, the frame shows the result.
//!
//! **Epochs.** Every guess belongs to an epoch, and an epoch is shown only
//! once the server has confirmed one of its guesses. Anything the engine
//! cannot model — a control key, an escape sequence, a wide character —
//! opens a new, unconfirmed epoch, so a prompt that doesn't echo (a
//! password) never shows a typed character.
//!
//! **Judging.** A guess is judged only when the server's echo ack covers
//! the user state that carried the keystroke. Wrong in a shown epoch →
//! every guess goes; wrong in a hidden one → that epoch goes, unseen.
//!
//! **Adaptive.** On a fast link the guesses are not drawn (shown above ~60
//! ms round trip, hidden again below ~40 ms); a guess stuck past 250 ms
//! shows anyway. On a slow link, with `trail`, unconfirmed characters are
//! drawn dim.
//!
//! Scope, as the Swift engine's: no newline or wrap prediction (a carriage
//! return or the last column waits for the server); no overwrite mode.

use std::collections::HashMap;

use wezterm_term::{CellAttributes, Intensity};

/// When guesses show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictMode {
    Never,
    /// On a slow link (mosh's SRTT trigger), or a glitch.
    Adaptive,
    Always,
    /// Always, and every keystroke in the confirmed epoch (mosh's
    /// "experimental").
    Experimental,
}

/// A cell as the engine reads and writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PCell {
    pub text: String,
    /// 0: the second column of a wide character.
    pub width: u8,
    pub attrs: CellAttributes,
}

impl PCell {
    pub fn blank(attrs: CellAttributes) -> Self {
        Self {
            text: " ".into(),
            width: 1,
            attrs,
        }
    }

    fn is_blank(&self) -> bool {
        self.text.is_empty() || self.text == " " || self.text == "\0"
    }

    fn matches(&self, other: &PCell) -> bool {
        (self.is_blank() && other.is_blank())
            || (self.width == other.width && self.text == other.text)
    }
}

/// The server's screen as the engine reads it.
pub trait Screen {
    fn cols(&self) -> usize;
    fn rows(&self) -> usize;
    /// (row, col), the column kept on the last one while a wrap is pending.
    fn cursor(&self) -> (usize, usize);
    /// The attributes text is printed with now.
    fn rendition(&self) -> CellAttributes;
    fn cell(&self, row: usize, col: usize) -> PCell;
}

// mosh's constants, in send-interval milliseconds.
const SRTT_TRIGGER_LOW: u64 = 20;
const SRTT_TRIGGER_HIGH: u64 = 30;
const FLAG_TRIGGER_LOW: u64 = 50;
const FLAG_TRIGGER_HIGH: u64 = 80;
const GLITCH_THRESHOLD: u64 = 250;
const GLITCH_REPAIR_COUNT: u32 = 10;
const GLITCH_REPAIR_MIN_INTERVAL: u64 = 150;
const GLITCH_FLAG_THRESHOLD: u64 = 5000;

#[derive(Debug, Clone)]
struct Cell {
    col: usize,
    active: bool,
    tentative_until: u64,
    batch: u64,
    prediction_time: u64,
    replacement: Option<PCell>,
    unknown: bool,
    original: Vec<PCell>,
}

impl Cell {
    fn new(col: usize, epoch: u64) -> Self {
        Self {
            col,
            active: false,
            tentative_until: epoch,
            batch: 0,
            prediction_time: u64::MAX,
            replacement: None,
            unknown: false,
            original: Vec::new(),
        }
    }

    fn tentative(&self, confirmed: u64) -> bool {
        self.tentative_until > confirmed
    }

    fn reset(&mut self) {
        self.batch = 0;
        self.tentative_until = u64::MAX;
        self.active = false;
        self.unknown = false;
        self.original.clear();
    }

    fn reset_with_original(&mut self) {
        if !self.active || self.unknown {
            return self.reset();
        }
        if let Some(r) = &self.replacement {
            self.original.push(r.clone());
        }
        self.batch = 0;
        self.tentative_until = u64::MAX;
        self.active = false;
    }

    fn expire(&mut self, batch: u64, now: u64) {
        self.batch = batch;
        self.prediction_time = now;
    }
}

#[derive(Debug, Clone)]
struct Row {
    row: usize,
    cells: Vec<Cell>,
}

#[derive(Debug, Clone)]
struct CursorMove {
    row: usize,
    col: usize,
    active: bool,
    tentative_until: u64,
    batch: u64,
    prediction_time: u64,
}

impl CursorMove {
    fn tentative(&self, confirmed: u64) -> bool {
        self.tentative_until > confirmed
    }
    fn expire(&mut self, batch: u64, now: u64) {
        self.batch = batch;
        self.prediction_time = now;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Validity {
    Pending,
    Correct,
    CorrectNoCredit,
    IncorrectOrExpired,
    Inactive,
}

/// What a frame lays over the server's screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overlay {
    /// (row, col, the cell to draw there): a guess, or the server's cell
    /// dimmed (an unknown cell under `trail`).
    pub cells: Vec<(usize, usize, PCell)>,
    /// Where the cursor is drawn, when the guess moved it.
    pub cursor: Option<(usize, usize)>,
}

/// The engine.
#[derive(Debug, Clone)]
pub struct Prediction {
    pub mode: PredictMode,
    pub trail: bool,
    /// The transport's smoothed round trip, ms (0: not measured yet, which
    /// counts as slow).
    pub srtt_ms: f64,
    overlay: Vec<Row>,
    cursors: Vec<CursorMove>,
    prediction_epoch: u64,
    confirmed_epoch: u64,
    flagging: bool,
    srtt_trigger: bool,
    glitch_trigger: u32,
    last_quick_confirmation: u64,
    last_size: (usize, usize),
    next_batch: u64,
    batch_states: HashMap<u64, u64>,
    echo_ack: u64,
    cursor_barrier: Option<u64>,
    scanner: InputScanner,
}

impl Prediction {
    pub fn new(mode: PredictMode, trail: bool) -> Self {
        Self {
            mode,
            trail,
            srtt_ms: 0.0,
            overlay: Vec::new(),
            cursors: Vec::new(),
            prediction_epoch: 1,
            confirmed_epoch: 0,
            flagging: false,
            srtt_trigger: false,
            glitch_trigger: 0,
            last_quick_confirmation: 0,
            last_size: (0, 0),
            next_batch: 1,
            batch_states: HashMap::new(),
            echo_ack: 0,
            cursor_barrier: None,
            scanner: InputScanner::default(),
        }
    }

    // --- Inputs ---------------------------------------------------------------

    /// Keystrokes about to go to the server: guess their echo. The batch to
    /// hand back to [`Prediction::sent`], or None when predicting is off.
    pub fn user_input(&mut self, data: &[u8], screen: &dyn Screen, now: u64) -> Option<u64> {
        if self.mode == PredictMode::Never || data.is_empty() {
            return None;
        }
        let batch = self.next_batch;
        self.next_batch += 1;
        self.cull(screen, batch, now);
        for event in self.scanner.scan(data) {
            if self.mode == PredictMode::Experimental {
                self.prediction_epoch = self.confirmed_epoch;
            }
            self.handle(event, screen, batch, now);
        }
        Some(batch)
    }

    /// The transport queued `batch` as user state `num` (None: it couldn't).
    pub fn sent(&mut self, batch: u64, num: Option<u64>) {
        match num {
            Some(n) => {
                self.batch_states.insert(batch, n);
            }
            None => self.reset(),
        }
    }

    /// A frame's echo ack (monotonic).
    pub fn echo_ack(&mut self, ack: u64) {
        self.echo_ack = self.echo_ack.max(ack);
    }

    /// Judges every guess the echo ack now covers (after host bytes).
    pub fn settle(&mut self, screen: &dyn Screen, now: u64) {
        let batch = self.next_batch;
        self.cull(screen, batch, now);
    }

    /// Drops every guess: the screen is the server's again.
    pub fn reset(&mut self) {
        self.cursors.clear();
        self.overlay.clear();
        self.cursor_barrier = None;
        self.become_tentative();
    }

    /// Whether [`Prediction::tick`] should run every 50 ms.
    pub fn needs_tick(&self) -> bool {
        if self.mode == PredictMode::Never || (self.glitch_trigger > 0 && self.flagging) {
            return false;
        }
        self.overlay.iter().any(|r| {
            r.cells
                .iter()
                .any(|c| c.active && self.echo_ack < self.state(c.batch))
        })
    }

    /// The clock-only part of judging; true when what shows changed.
    pub fn tick(&mut self, now: u64) -> bool {
        if self.mode == PredictMode::Never {
            return false;
        }
        let before = (self.showing(), self.draw_flagged());
        self.update_triggers();
        let pending: Vec<u64> = self
            .overlay
            .iter()
            .flat_map(|r| r.cells.iter())
            .filter(|c| c.active && self.echo_ack < self.state(c.batch))
            .map(|c| c.prediction_time)
            .collect();
        for t in pending {
            self.note_pending(t, now);
        }
        before != (self.showing(), self.draw_flagged())
    }

    // --- Keystrokes (mosh's new_user_byte) --------------------------------------

    fn handle(&mut self, event: Event, screen: &dyn Screen, batch: u64, now: u64) {
        match event {
            Event::Print('\u{7f}') => self.backspace(screen, batch, now),
            Event::Print(c) => {
                if predictable(c) {
                    self.insert(c, screen, batch, now);
                } else {
                    self.become_tentative();
                }
            }
            Event::Execute(b) => {
                self.become_tentative();
                if b == 0x0d || b == 0x0a {
                    self.cursor_barrier = Some(batch);
                }
            }
            Event::Csi {
                final_byte: b'C',
                plain: true,
            } => self.move_right(screen, batch, now),
            Event::Csi {
                final_byte: b'D',
                plain: true,
            } => self.move_left(screen, batch, now),
            Event::Csi { .. } | Event::Escape | Event::String => self.become_tentative(),
        }
    }

    fn cursor_mut(&mut self) -> &mut CursorMove {
        let n = self.cursors.len() - 1;
        &mut self.cursors[n]
    }

    fn insert(&mut self, ch: char, screen: &dyn Screen, batch: u64, now: u64) {
        if self.cursor_barrier.is_some() {
            return self.become_tentative();
        }
        self.init_cursor(screen, batch);
        let (crow, ccol) = {
            let c = self.cursors.last().unwrap();
            (c.row, c.col)
        };
        let width = screen.cols();
        if crow >= screen.rows() || ccol >= width {
            return self.become_tentative();
        }
        let r = self.row_index(crow, width);
        if ccol + 1 >= width {
            self.become_tentative();
        }
        let attrs = if ccol > 0 {
            self.attrs_left_of(ccol, crow, r, screen)
        } else {
            screen.rendition()
        };
        let ghost = self.ghost_tail(ccol, crow, r, screen, &attrs);
        let epoch = self.prediction_epoch;
        let mut i = width - 1;
        while i > ccol {
            let prev_active = self.overlay[r].cells[i - 1].active;
            let prev_unknown = self.overlay[r].cells[i - 1].unknown;
            let prev_replacement = self.overlay[r].cells[i - 1].replacement.clone();
            let original = screen.cell(crow, i);
            let shifted = if !prev_active {
                Some(screen.cell(crow, i - 1))
            } else {
                None
            };
            let cell = &mut self.overlay[r].cells[i];
            cell.reset_with_original();
            cell.active = true;
            cell.tentative_until = epoch;
            cell.expire(batch, now);
            cell.original.push(original);
            if i == width - 1 || ghost {
                cell.unknown = true;
            } else if prev_active {
                if prev_unknown {
                    cell.unknown = true;
                } else {
                    cell.unknown = false;
                    cell.replacement = prev_replacement;
                }
            } else {
                cell.unknown = false;
                cell.replacement = shifted;
            }
            i -= 1;
        }
        let original = screen.cell(crow, ccol);
        let cell = &mut self.overlay[r].cells[ccol];
        cell.reset_with_original();
        cell.active = true;
        cell.tentative_until = epoch;
        cell.expire(batch, now);
        cell.replacement = Some(PCell {
            text: ch.to_string(),
            width: 1,
            attrs,
        });
        cell.original.push(original);
        let cursor = self.cursor_mut();
        cursor.expire(batch, now);
        if ccol < width - 1 {
            cursor.col += 1;
        } else {
            self.become_tentative();
            self.cursor_barrier = Some(batch);
        }
    }

    fn backspace(&mut self, screen: &dyn Screen, batch: u64, now: u64) {
        if self.cursor_barrier.is_some() {
            return self.become_tentative();
        }
        self.init_cursor(screen, batch);
        let (crow, ccol) = {
            let c = self.cursors.last().unwrap();
            (c.row, c.col)
        };
        let width = screen.cols();
        if crow >= screen.rows() || ccol == 0 || ccol >= width {
            return;
        }
        let r = self.row_index(crow, width);
        let erased = self.displayed(crow, ccol - 1, r, screen);
        if erased.width == 0 {
            self.become_tentative();
            self.cursor_barrier = Some(batch);
            return;
        }
        let ghost = self.ghost_tail(ccol, crow, r, screen, &erased.attrs);
        let col = ccol - 1;
        {
            let cursor = self.cursor_mut();
            cursor.col = col;
            cursor.expire(batch, now);
        }
        let epoch = self.prediction_epoch;
        for i in col..width {
            let next_active = i + 1 < width && self.overlay[r].cells[i + 1].active;
            let next_unknown = i + 1 < width && self.overlay[r].cells[i + 1].unknown;
            let next_replacement = if i + 1 < width {
                self.overlay[r].cells[i + 1].replacement.clone()
            } else {
                None
            };
            let original = screen.cell(crow, i);
            let next_screen = (i + 1 < width).then(|| screen.cell(crow, i + 1));
            let cell = &mut self.overlay[r].cells[i];
            cell.reset_with_original();
            cell.active = true;
            cell.tentative_until = epoch;
            cell.expire(batch, now);
            cell.original.push(original);
            if ghost {
                if i == col {
                    cell.unknown = false;
                    cell.replacement = Some(PCell::blank(erased.attrs.clone()));
                } else {
                    cell.unknown = true;
                }
            } else if i + 2 < width {
                if next_active {
                    if next_unknown {
                        cell.unknown = true;
                    } else {
                        cell.unknown = false;
                        cell.replacement = next_replacement;
                    }
                } else {
                    cell.unknown = false;
                    cell.replacement = next_screen;
                }
            } else {
                cell.unknown = true;
            }
        }
    }

    fn move_right(&mut self, screen: &dyn Screen, batch: u64, now: u64) {
        if self.cursor_barrier.is_some() {
            return self.become_tentative();
        }
        self.init_cursor(screen, batch);
        let (crow, ccol) = {
            let c = self.cursors.last().unwrap();
            (c.row, c.col)
        };
        let width = screen.cols();
        if crow >= screen.rows() || ccol + 1 >= width {
            return;
        }
        let r = self.row_index(crow, width);
        if (ccol..width).all(|c| self.displayed(crow, c, r, screen).is_blank()) {
            return self.become_tentative();
        }
        if self.displayed(crow, ccol + 1, r, screen).width == 0 {
            self.become_tentative();
            self.cursor_barrier = Some(batch);
            return;
        }
        let cursor = self.cursor_mut();
        cursor.col += 1;
        cursor.expire(batch, now);
    }

    fn move_left(&mut self, screen: &dyn Screen, batch: u64, now: u64) {
        if self.cursor_barrier.is_some() {
            return self.become_tentative();
        }
        self.init_cursor(screen, batch);
        let (crow, ccol) = {
            let c = self.cursors.last().unwrap();
            (c.row, c.col)
        };
        if crow >= screen.rows() || ccol == 0 || ccol >= screen.cols() {
            return;
        }
        let r = self.row_index(crow, screen.cols());
        if self.displayed(crow, ccol - 1, r, screen).width == 0 {
            self.become_tentative();
            self.cursor_barrier = Some(batch);
            return;
        }
        let cursor = self.cursor_mut();
        cursor.col -= 1;
        cursor.expire(batch, now);
    }

    // --- Judging (mosh's cull) --------------------------------------------------

    fn cull(&mut self, screen: &dyn Screen, batch: u64, now: u64) {
        if self.mode == PredictMode::Never {
            return;
        }
        if (screen.cols(), screen.rows()) != self.last_size {
            self.last_size = (screen.cols(), screen.rows());
            self.reset();
        }
        self.update_triggers();
        let mut i = 0;
        while i < self.overlay.len() {
            let row = self.overlay[i].row;
            if row >= screen.rows() {
                self.overlay.remove(i);
                continue;
            }
            for j in 0..self.overlay[i].cells.len() {
                let validity = self.validity_cell(&self.overlay[i].cells[j], row, screen);
                match validity {
                    Validity::IncorrectOrExpired => {
                        let cell = &self.overlay[i].cells[j];
                        if self.mode == PredictMode::Experimental {
                            self.overlay[i].cells[j].reset();
                        } else if cell.tentative(self.confirmed_epoch) {
                            let epoch = cell.tentative_until;
                            self.kill_epoch(epoch, screen, batch);
                        } else {
                            self.reset();
                            return;
                        }
                    }
                    Validity::Correct => {
                        let cell = &self.overlay[i].cells[j];
                        if cell.tentative_until > self.confirmed_epoch {
                            self.confirmed_epoch = cell.tentative_until;
                        }
                        if now.wrapping_sub(cell.prediction_time) < GLITCH_THRESHOLD
                            && self.glitch_trigger > 0
                            && now
                                >= self
                                    .last_quick_confirmation
                                    .wrapping_add(GLITCH_REPAIR_MIN_INTERVAL)
                        {
                            self.glitch_trigger -= 1;
                            self.last_quick_confirmation = now;
                        }
                        self.match_renditions(i, j, screen);
                        self.overlay[i].cells[j].reset();
                    }
                    Validity::CorrectNoCredit => {
                        // No credit, but the server drew this character here:
                        // its colours are the truth for the guesses after it.
                        let cell = &self.overlay[i].cells[j];
                        if !cell.unknown
                            && let Some(rep) = &cell.replacement
                            && !rep.is_blank()
                            && screen.cell(row, cell.col).matches(rep)
                        {
                            self.match_renditions(i, j, screen);
                        }
                        self.overlay[i].cells[j].reset();
                    }
                    Validity::Pending => {
                        let t = self.overlay[i].cells[j].prediction_time;
                        self.note_pending(t, now);
                    }
                    Validity::Inactive => {}
                }
            }
            i += 1;
        }
        if let Some(barrier) = self.cursor_barrier
            && self.echo_ack >= self.state(barrier)
        {
            self.cursor_barrier = None;
            self.cursors.clear();
        }
        if let Some(last) = self.cursors.last()
            && self.validity_cursor(last, screen) == Validity::IncorrectOrExpired
        {
            if self.mode == PredictMode::Experimental {
                self.cursors.clear();
            } else {
                self.reset();
                return;
            }
        }
        let keep: Vec<CursorMove> = self
            .cursors
            .iter()
            .filter(|c| self.validity_cursor(c, screen) == Validity::Pending)
            .cloned()
            .collect();
        self.cursors = keep;
        self.prune_batches();
    }

    fn match_renditions(&mut self, i: usize, j: usize, screen: &dyn Screen) {
        let actual = screen
            .cell(self.overlay[i].row, self.overlay[i].cells[j].col)
            .attrs;
        for k in j..self.overlay[i].cells.len() {
            if let Some(r) = self.overlay[i].cells[k].replacement.as_mut() {
                r.attrs = actual.clone();
            }
        }
    }

    fn update_triggers(&mut self) {
        let interval = self.send_interval();
        if interval > SRTT_TRIGGER_HIGH {
            self.srtt_trigger = true;
        } else if self.srtt_trigger && interval <= SRTT_TRIGGER_LOW && !self.is_active() {
            self.srtt_trigger = false;
        }
        if interval > FLAG_TRIGGER_HIGH {
            self.flagging = true;
        } else if interval <= FLAG_TRIGGER_LOW {
            self.flagging = false;
        }
        if self.glitch_trigger > GLITCH_REPAIR_COUNT {
            self.flagging = true;
        }
    }

    fn note_pending(&mut self, since: u64, now: u64) {
        let age = now.wrapping_sub(since);
        if since == u64::MAX {
            return;
        }
        if age >= GLITCH_FLAG_THRESHOLD {
            self.glitch_trigger = GLITCH_REPAIR_COUNT * 2;
        } else if age >= GLITCH_THRESHOLD && self.glitch_trigger < GLITCH_REPAIR_COUNT {
            self.glitch_trigger = GLITCH_REPAIR_COUNT;
        }
    }

    fn validity_cell(&self, cell: &Cell, row: usize, screen: &dyn Screen) -> Validity {
        if !cell.active {
            return Validity::Inactive;
        }
        if row >= screen.rows() || cell.col >= screen.cols() {
            return Validity::IncorrectOrExpired;
        }
        if self.echo_ack < self.state(cell.batch) {
            return Validity::Pending;
        }
        if cell.unknown {
            return Validity::CorrectNoCredit;
        }
        let Some(rep) = &cell.replacement else {
            return Validity::CorrectNoCredit;
        };
        if rep.is_blank() {
            return Validity::CorrectNoCredit;
        }
        let current = screen.cell(row, cell.col);
        if !current.matches(rep) {
            return Validity::IncorrectOrExpired;
        }
        if cell.original.iter().any(|o| o.matches(rep)) {
            Validity::CorrectNoCredit
        } else {
            Validity::Correct
        }
    }

    fn validity_cursor(&self, c: &CursorMove, screen: &dyn Screen) -> Validity {
        if !c.active {
            return Validity::Inactive;
        }
        if c.row >= screen.rows() || c.col >= screen.cols() {
            return Validity::IncorrectOrExpired;
        }
        if self.echo_ack < self.state(c.batch) {
            return Validity::Pending;
        }
        if screen.cursor() == (c.row, c.col) {
            Validity::Correct
        } else {
            Validity::IncorrectOrExpired
        }
    }

    fn kill_epoch(&mut self, epoch: u64, screen: &dyn Screen, batch: u64) {
        self.cursors.retain(|c| c.tentative_until < epoch);
        let (row, col) = screen.cursor();
        self.cursors.push(CursorMove {
            row,
            col,
            active: true,
            tentative_until: self.prediction_epoch,
            batch,
            prediction_time: u64::MAX,
        });
        for r in &mut self.overlay {
            for c in &mut r.cells {
                if c.tentative_until >= epoch {
                    c.reset();
                }
            }
        }
        self.become_tentative();
    }

    fn become_tentative(&mut self) {
        if self.mode != PredictMode::Experimental {
            self.prediction_epoch += 1;
        }
    }

    fn init_cursor(&mut self, screen: &dyn Screen, batch: u64) {
        let epoch = self.prediction_epoch;
        match self.cursors.last() {
            None => {
                let (row, col) = screen.cursor();
                self.cursors.push(CursorMove {
                    row,
                    col,
                    active: true,
                    tentative_until: epoch,
                    batch,
                    prediction_time: u64::MAX,
                });
            }
            Some(last) if last.tentative_until != epoch => {
                let (row, col) = (last.row, last.col);
                self.cursors.push(CursorMove {
                    row,
                    col,
                    active: true,
                    tentative_until: epoch,
                    batch,
                    prediction_time: u64::MAX,
                });
            }
            _ => {}
        }
    }

    fn row_index(&mut self, row: usize, width: usize) -> usize {
        if let Some(i) = self.overlay.iter().position(|r| r.row == row) {
            return i;
        }
        let epoch = self.prediction_epoch;
        self.overlay.push(Row {
            row,
            cells: (0..width).map(|c| Cell::new(c, epoch)).collect(),
        });
        self.overlay.len() - 1
    }

    fn state(&self, batch: u64) -> u64 {
        self.batch_states.get(&batch).copied().unwrap_or(u64::MAX)
    }

    fn prune_batches(&mut self) {
        let mut oldest = self.next_batch;
        for r in &self.overlay {
            for c in &r.cells {
                if c.active {
                    oldest = oldest.min(c.batch);
                }
            }
        }
        for c in &self.cursors {
            oldest = oldest.min(c.batch);
        }
        if let Some(b) = self.cursor_barrier {
            oldest = oldest.min(b);
        }
        self.batch_states.retain(|k, _| *k >= oldest);
    }

    fn is_active(&self) -> bool {
        !self.cursors.is_empty()
            || self
                .overlay
                .iter()
                .any(|r| r.cells.iter().any(|c| c.active))
    }

    /// mosh's `send_interval`: half the round trip, clamped to 20…250 ms.
    fn send_interval(&self) -> u64 {
        let srtt = if self.srtt_ms > 0.0 {
            self.srtt_ms
        } else {
            1000.0
        };
        (srtt / 2.0).ceil().clamp(20.0, 250.0) as u64
    }

    fn showing(&self) -> bool {
        match self.mode {
            PredictMode::Never => false,
            PredictMode::Always | PredictMode::Experimental => true,
            PredictMode::Adaptive => self.srtt_trigger || self.glitch_trigger > 0,
        }
    }

    fn draw_flagged(&self) -> bool {
        self.trail && self.flagging
    }

    // --- Reading the screen --------------------------------------------------

    fn displayed(&self, row: usize, col: usize, r: usize, screen: &dyn Screen) -> PCell {
        let cell = &self.overlay[r].cells[col];
        match (&cell.replacement, cell.active && !cell.unknown) {
            (Some(rep), true) => rep.clone(),
            _ => screen.cell(row, col),
        }
    }

    /// What follows the cursor is drawn unlike the text being edited (a
    /// shell's autosuggestion): typing consumes or redraws it, only the
    /// server knows which, so its cells are left unknown.
    fn ghost_tail(
        &self,
        col: usize,
        row: usize,
        r: usize,
        screen: &dyn Screen,
        text: &CellAttributes,
    ) -> bool {
        let here = self.displayed(row, col, r, screen);
        !here.is_blank() && here.attrs != *text
    }

    /// The attributes of a character typed at `col`: its left neighbour's,
    /// skipping a wide character's second column.
    fn attrs_left_of(
        &self,
        col: usize,
        row: usize,
        r: usize,
        screen: &dyn Screen,
    ) -> CellAttributes {
        let mut left = col - 1;
        if left > 0 && self.displayed(row, left, r, screen).width == 0 {
            left -= 1;
        }
        self.displayed(row, left, r, screen).attrs
    }

    // --- The overlay -----------------------------------------------------------

    /// What the frame lays over the server's screen now.
    pub fn overlay(&self, screen: &dyn Screen) -> Overlay {
        let mut out = Overlay::default();
        if !self.showing() {
            return out;
        }
        let flag = self.draw_flagged();
        if let Some(c) = self
            .cursors
            .iter()
            .rev()
            .find(|c| c.active && !c.tentative(self.confirmed_epoch))
            && c.row < screen.rows()
            && c.col < screen.cols()
            && (c.row, c.col) != screen.cursor()
        {
            out.cursor = Some((c.row, c.col));
        }
        for row in &self.overlay {
            if row.row >= screen.rows() {
                continue;
            }
            for cell in &row.cells {
                if !cell.active || cell.tentative(self.confirmed_epoch) || cell.col >= screen.cols()
                {
                    continue;
                }
                let current = screen.cell(row.row, cell.col);
                let rep = cell.replacement.as_ref();
                let flagged = flag && !(rep.is_none_or(PCell::is_blank) && current.is_blank());
                if cell.unknown {
                    if flagged && cell.col != screen.cols() - 1 {
                        let mut marked = current;
                        marked.attrs.set_intensity(Intensity::Half);
                        out.cells.push((row.row, cell.col, marked));
                    }
                    continue;
                }
                let Some(rep) = rep else { continue };
                if current.attrs == rep.attrs && current.matches(rep) {
                    continue;
                }
                let mut drawn = rep.clone();
                if flagged {
                    drawn.attrs.set_intensity(Intensity::Half);
                }
                out.cells.push((row.row, cell.col, drawn));
            }
        }
        out
    }
}

/// Characters the engine can place: one column, no combining, no controls.
/// Everything from U+1100 up (Hangul, CJK, emoji…) may be double width.
pub fn predictable(c: char) -> bool {
    let v = c as u32;
    if v < 0x20 || (0x7f..=0x9f).contains(&v) || v >= 0x1100 {
        return false;
    }
    // Combining marks (the blocks below U+1100 that hold them).
    !matches!(v, 0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05C7 | 0x0610..=0x061A
        | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06ED | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E
        | 0x00AD | 0x200B..=0x200F)
}

// --- The keystroke scanner --------------------------------------------------

/// What the engine cares about in outgoing keystroke bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Print(char),
    /// A C0 control.
    Execute(u8),
    /// `ESC x`.
    Escape,
    /// CSI or SS3; `plain`: no parameters or intermediates.
    Csi {
        final_byte: u8,
        plain: bool,
    },
    /// OSC, DCS, APC, PM, SOS.
    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ScanState {
    #[default]
    Ground,
    Escape,
    EscapeIntermediate,
    Csi,
    Ss3,
    Str,
    StrEscape,
}

/// A small VT input parser; state carries across calls.
#[derive(Debug, Clone, Default)]
pub struct InputScanner {
    state: ScanState,
    plain: bool,
    utf8: Vec<u8>,
    need: usize,
}

impl InputScanner {
    pub fn scan(&mut self, data: &[u8]) -> Vec<Event> {
        let mut out = Vec::new();
        for &b in data {
            self.feed(b, &mut out);
        }
        out
    }

    fn feed(&mut self, b: u8, out: &mut Vec<Event>) {
        use ScanState as S;
        if b == 0x18 || b == 0x1a {
            self.utf8.clear();
            self.need = 0;
            self.state = S::Ground;
            out.push(Event::Execute(b));
            return;
        }
        match self.state {
            S::Ground => self.ground(b, out),
            S::Escape => match b {
                0x1b => {}
                0x5b => {
                    self.state = S::Csi;
                    self.plain = true;
                }
                0x4f => self.state = S::Ss3,
                0x50 | 0x58 | 0x5d | 0x5e | 0x5f => self.state = S::Str,
                0x20..=0x2f => self.state = S::EscapeIntermediate,
                0x30..=0x7e => {
                    self.state = S::Ground;
                    out.push(Event::Escape);
                }
                0x7f => {}
                0x00..=0x1f => out.push(Event::Execute(b)),
                _ => {
                    self.state = S::Ground;
                    out.push(Event::Escape);
                    self.ground(b, out);
                }
            },
            S::EscapeIntermediate => match b {
                0x1b => self.state = S::Escape,
                0x20..=0x2f | 0x7f => {}
                0x30..=0x7e => {
                    self.state = S::Ground;
                    out.push(Event::Escape);
                }
                0x00..=0x1f => out.push(Event::Execute(b)),
                _ => {
                    self.state = S::Ground;
                    out.push(Event::Escape);
                }
            },
            S::Csi => match b {
                0x1b => self.state = S::Escape,
                0x20..=0x3f => self.plain = false,
                0x40..=0x7e => {
                    self.state = S::Ground;
                    out.push(Event::Csi {
                        final_byte: b,
                        plain: self.plain,
                    });
                }
                0x7f => {}
                0x00..=0x1f => out.push(Event::Execute(b)),
                _ => {
                    self.state = S::Ground;
                    out.push(Event::Escape);
                }
            },
            S::Ss3 => match b {
                0x1b => self.state = S::Escape,
                0x40..=0x7e => {
                    self.state = S::Ground;
                    out.push(Event::Csi {
                        final_byte: b,
                        plain: true,
                    });
                }
                0x00..=0x1f => out.push(Event::Execute(b)),
                _ => {
                    self.state = S::Ground;
                    out.push(Event::Escape);
                }
            },
            S::Str => {
                if b == 0x07 {
                    self.state = S::Ground;
                    out.push(Event::String);
                } else if b == 0x1b {
                    self.state = S::StrEscape;
                }
            }
            S::StrEscape => {
                out.push(Event::String);
                self.state = S::Escape;
                if b != 0x5c {
                    self.feed(b, out);
                } else {
                    self.state = S::Ground;
                }
            }
        }
    }

    fn ground(&mut self, b: u8, out: &mut Vec<Event>) {
        if self.need > 0 {
            if b & 0xc0 == 0x80 {
                self.utf8.push(b);
                self.need -= 1;
                if self.need == 0 {
                    let c = std::str::from_utf8(&self.utf8)
                        .ok()
                        .and_then(|s| s.chars().next())
                        .unwrap_or('\u{fffd}');
                    self.utf8.clear();
                    out.push(Event::Print(c));
                }
                return;
            }
            self.utf8.clear();
            self.need = 0;
            out.push(Event::Print('\u{fffd}'));
        }
        match b {
            0x1b => self.state = ScanState::Escape,
            0x00..=0x1f => out.push(Event::Execute(b)),
            0x20..=0x7f => out.push(Event::Print(b as char)),
            0xc2..=0xdf => {
                self.utf8 = vec![b];
                self.need = 1;
            }
            0xe0..=0xef => {
                self.utf8 = vec![b];
                self.need = 2;
            }
            0xf0..=0xf4 => {
                self.utf8 = vec![b];
                self.need = 3;
            }
            _ => out.push(Event::Print('\u{fffd}')),
        }
    }
}
