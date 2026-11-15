//! What the person sends: keys, pastes, scroll ticks and taps — and the
//! agent-hook poll.
//!
//! Keys go as `send-keys -H` hex, except a lone arrow, which goes as tmux's
//! key name so tmux encodes it against the pane's REAL cursor-key mode
//! (DECCKM): the local emulator's flag is learned only by watching the
//! pane's `ESC[?1h` go by, which a fresh `-CC` attach never replays. A paste
//! goes through a tmux buffer with `paste-buffer -p`, so tmux brackets it by
//! the pane's real DECSET 2004 state — after the paste markers are taken out
//! of the text (round nineteen).
//!
//! A scroll tick goes one of three ways: the wheel (a program with the mouse
//! on scrolls itself), dropped (an alternate-screen pane whose mouse flags
//! haven't settled has nothing local worth paging), or the local scrollback
//! (a shell). Never tmux's copy-mode: tmux draws copy-mode only for tty
//! clients, so a `-CC` client scrolling it sees nothing while it hijacks any
//! desktop client on the window.

use moshpit_herdr::frame::{Piece, paste_body, split_arrows};
use moshpit_herdr::text::trim_whitespaces;

use crate::control::CommandResponse;
use crate::format::{command_string_literal, is_valid_id, words};
use crate::hooks::{AGENT_HOOK_FORMAT, parse_agent_hooks};

use super::timers::{Action, Slot};
use super::{Controller, Notification, Reply, ScreenState, ViewAction};

/// Where a scroll tick over the active pane goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollRoute {
    /// SGR wheel events into the pane's program.
    Wheel,
    /// The arrow keys, for an alternate-screen program without the mouse
    /// (less, man, git log): xterm's alternate scroll (B500).
    Arrows,
    /// Nothing.
    Drop,
    /// The view pages its local scrollback.
    Local,
}

/// What became of a pixel scroll ([`Controller::scroll_pixels`]).
#[derive(Debug, Clone, PartialEq)]
pub enum PixelScroll {
    /// Consumed this many pixels (the wheel takes the whole travel; a
    /// dropped tick none).
    Consumed(f64),
    /// The host scrolls the pane's local scrollback by the pixels itself
    /// (it knows what the view consumed).
    Local { pane: String },
}

/// Where a scroll over the active pane goes, as far as its program says
/// (`#{mouse_any_flag} #{alternate_on}`, kept per pane and read with every
/// repaint's cursor query, B363), with what follows from them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputFlags {
    /// The program has the mouse on (unknown: no).
    pub wants_mouse: bool,
    /// The pane is on the alternate screen (unknown: discovery's
    /// `alternate_on` stands in).
    pub alt_screen: bool,
    /// The program scrolls itself, so a scroll is always its wheel: an
    /// alternate-screen mouse app, or Claude Code with the mouse wherever
    /// it draws (its full-screen renderer runs on the primary screen too,
    /// where the pane's history is leftovers of its repaints, not the
    /// conversation).
    pub scrolls_itself: bool,
    /// Claude on the primary screen with no flags read yet: it could be
    /// either renderer, so a scroll is dropped rather than guessed (the
    /// attach's first repaint brings the flags).
    pub holds_ticks: bool,
    /// tmux has the pane in a mode — copy-mode, a view-mode — as discovery
    /// and every `%pane-mode-changed` since said (B614: the mosh path's
    /// copy-mode latch follows it; unknown: no).
    pub in_copy_mode: bool,
}

/// What tmux said about one pane's program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PaneFlags {
    pub wants_mouse: bool,
    pub alt_screen: bool,
}

/// Sub-row finger travel not yet a wheel tick (`ScrollRowCarry`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct RowCarry {
    remainder: f64,
}

impl RowCarry {
    /// Positive = toward older output. The whole rows now due.
    ///
    /// Fix (carry-overflow): Swift's `Int(…)` traps on an absurd finite
    /// travel; the rows saturate here (the app's carry too since B149).
    ///
    /// B149, both sides: a remainder pushed past `f64`'s range starts over —
    /// left infinite it answered every later drag with `i64::MIN`, left NaN
    /// with nothing. A cell height that is not above zero, NaN included,
    /// takes nothing (Swift's `guard cellHeight > 0`).
    pub fn take(&mut self, pixels: f64, cell_height: f64) -> i64 {
        if cell_height.is_nan() || cell_height <= 0.0 || !pixels.is_finite() {
            return 0;
        }
        self.remainder += pixels;
        let rows = (self.remainder / cell_height).trunc();
        self.remainder -= rows * cell_height;
        if !self.remainder.is_finite() {
            self.remainder = 0.0;
        }
        rows as i64
    }

    pub fn reset(&mut self) {
        self.remainder = 0.0;
    }
}

/// SGR wheel events for a program with the mouse on: button 64 = up
/// (older), 65 = down; at most six a tick (`wheelBytes`).
pub fn wheel_bytes(lines: i64, col: i64, row: i64) -> Vec<u8> {
    if lines == 0 {
        return Vec::new();
    }
    let button = if lines > 0 { 64 } else { 65 };
    let count = lines.unsigned_abs().min(6);
    let event = format!("\u{1b}[<{button};{};{}M", col.max(1), row.max(1));
    event.repeat(count as usize).into_bytes()
}

/// Where a wheel goes in a pane `cols` × `rows` (1-based, as SGR has it):
/// the cell it was aimed at (0-based), kept within the pane, else the
/// middle (B578).
pub fn wheel_cell(aim: Option<(i64, i64)>, cols: i64, rows: i64) -> (i64, i64) {
    match aim {
        Some((col, row)) => (
            col.saturating_add(1).clamp(1, cols.max(1)),
            row.saturating_add(1).clamp(1, rows.max(1)),
        ),
        None => ((cols / 2).max(1), (rows / 2).max(1)),
    }
}

/// SGR press and release of the left button at a 0-based cell
/// (`clickBytes`).
pub fn click_bytes(col: i64, row: i64) -> Vec<u8> {
    let c = col.saturating_add(1).max(1);
    let r = row.saturating_add(1).max(1);
    format!("\u{1b}[<0;{c};{r}M\u{1b}[<0;{c};{r}m").into_bytes()
}

/// The most arrow presses one scroll tick sends: what a wheel tick at most
/// sends ([`wheel_bytes`]'s six) and twice that, for a coast's larger steps.
const MAX_ARROWS: u64 = 12;

/// How long what follows a paste waits once tmux has run it (B485), and at
/// most when tmux doesn't answer (the connection may be going).
const PASTE_SETTLE_MS: u64 = 150;
const PASTE_HOLD_MS: u64 = 2_000;

/// Input held behind a paste (B485).
#[derive(Debug)]
pub(super) enum Held {
    Input {
        pane: String,
        bytes: Vec<u8>,
    },
    Paste {
        pane: String,
        text: String,
    },
    /// A pager's arrow presses (B500).
    Arrows {
        pane: String,
        lines: i64,
    },
}

impl Controller {
    // MARK: Keys and paste

    /// Bytes for a pane (`sendInput`); also what the shortcut bar and the
    /// lock-screen actions send. The pane id is checked: it can come from
    /// outside the snapshot (an App Intent).
    pub fn send_input(&mut self, bytes: &[u8], pane: &str, now: u64) {
        self.begin_input(now);
        self.send_input_now(bytes, pane);
        self.finish_input();
    }

    pub(super) fn send_input_now(&mut self, bytes: &[u8], pane: &str) {
        if bytes.is_empty() || !is_valid_id(pane) {
            return;
        }
        // Behind a paste tmux hasn't run yet, or only just (B485).
        if let Some((_, held)) = self.paste_gate.as_mut() {
            held.push(Held::Input {
                pane: pane.to_owned(),
                bytes: bytes.to_vec(),
            });
            return;
        }
        // A keystroke snaps a reader back to the live bottom first.
        self.view(pane, ViewAction::ReleaseScrollHold);
        if self.pane_is_on_screen(pane) {
            self.note_user_interaction();
        }
        // Arrows that arrive together (key repeat, a busy moment) are still
        // each a key name (B360).
        for piece in split_arrows(bytes) {
            match piece {
                Piece::Arrow(key) => {
                    self.send(format!("send-keys -t {pane} {}", capitalized(key)));
                }
                Piece::Bytes(run) => {
                    let hex: Vec<String> = run.iter().map(|byte| format!("{byte:02x}")).collect();
                    self.send(format!("send-keys -t {pane} -H {}", hex.join(" ")));
                }
            }
        }
    }

    /// Paste through a tmux buffer: `paste-buffer -p` brackets it by the
    /// pane's real mode, which the local emulator can't know after a fresh
    /// attach. One fixed buffer is safe: the two commands go out back to
    /// back on the one channel.
    pub fn send_paste(&mut self, text: &str, pane: &str, now: u64) {
        self.begin_input(now);
        // tmux's paste-buffer sends each line feed as a carriage return: a
        // CRLF text (from Windows, a web page) went out CR CR, every line
        // break doubled (B738). One line feed per line break, then.
        let text = paste_body(text).replace("\r\n", "\n");
        if !text.is_empty() && is_valid_id(pane) {
            self.paste_now(text, pane);
        }
        self.finish_input();
    }

    /// The paste, or behind the one before it (B485). What follows it waits
    /// until tmux has run it (paste-buffer's answer) and [`PASTE_SETTLE_MS`]
    /// more: a program taking a paste in may drop a key that comes with it
    /// (Claude Code, making an image's path an attachment, drops the Return
    /// behind it: the image panel's Send only pasted), and a key sent a
    /// moment after a paste still reached tmux with it when the network
    /// held them together. Without an answer, [`PASTE_HOLD_MS`] at most.
    fn paste_now(&mut self, text: String, pane: &str) {
        if let Some((_, held)) = self.paste_gate.as_mut() {
            held.push(Held::Paste {
                pane: pane.to_owned(),
                text,
            });
            return;
        }
        self.pastes += 1;
        let paste = self.pastes;
        let literal = command_string_literal(&text);
        self.send(format!("set-buffer -b moshpit-paste -- {literal}"));
        self.send_command(
            format!("paste-buffer -p -b moshpit-paste -d -t {pane}"),
            Reply::Pasted(paste),
        );
        if self.pane_is_on_screen(pane) {
            self.note_user_interaction();
        }
        if self.snapshot.is_attached {
            self.paste_gate = Some((paste, Vec::new()));
            self.arm(Slot::PasteSettle, PASTE_HOLD_MS, Action::PasteSettled);
        }
    }

    /// tmux ran paste `paste` (or refused it): what waits behind it goes
    /// once it has had its time (B485). An earlier paste's answer, late past
    /// its wait, settles nothing of a later one's.
    pub(super) fn pasted(&mut self, paste: u64) {
        if self
            .paste_gate
            .as_ref()
            .is_some_and(|(gate, _)| *gate == paste)
        {
            self.arm(Slot::PasteSettle, PASTE_SETTLE_MS, Action::PasteSettled);
        }
    }

    /// What waited behind the paste goes, in order, up to a paste, which
    /// the rest then waits behind.
    pub(super) fn paste_settled(&mut self) {
        let Some((_, held)) = self.paste_gate.take() else {
            return;
        };
        let mut held = held.into_iter();
        while let Some(step) = held.next() {
            match step {
                Held::Input { pane, bytes } => self.send_input_now(&bytes, &pane),
                Held::Arrows { pane, lines } => self.send_arrows(lines, &pane),
                Held::Paste { pane, text } => {
                    self.paste_now(text, &pane);
                    if let Some((_, gate)) = self.paste_gate.as_mut() {
                        gate.extend(held);
                    }
                    return;
                }
            }
        }
    }

    /// Whether the pane's window is the active one (`panes[p]?.windowId ==
    /// activeWindowId`, optionals compared).
    fn pane_is_on_screen(&self, pane: &str) -> bool {
        self.snapshot
            .panes
            .get(pane)
            .map(|info| info.window_id.as_str())
            == self.snapshot.active_window_id.as_deref()
    }

    /// Release every pane's scroll hold (on a switch, so a pane left
    /// scrolled up doesn't sit frozen when the person comes back to it).
    pub(super) fn release_scroll_holds(&mut self) {
        let panes: Vec<String> = self.streams_in_id_order();
        for pane in panes {
            self.view(&pane, ViewAction::ReleaseScrollHold);
        }
    }

    fn streams_in_id_order(&self) -> Vec<String> {
        let mut panes: Vec<String> = self.streams.keys().cloned().collect();
        panes.sort_by(|a, b| crate::model::id_cmp(a, b));
        panes
    }

    // MARK: Scrolling

    /// The active pane's flags as last read, and what follows from them.
    pub fn active_input_flags(&self) -> InputFlags {
        self.snapshot
            .input_pane()
            .map_or_else(InputFlags::default, |pane| self.input_flags_of(&pane))
    }

    /// A pane's flags: what tmux said about it, else discovery's
    /// `alternate_on` for the screen (B363: the flags used to be one pair,
    /// unread after every attach until a drag began, so the first ticks of
    /// a swipe over a full-screen program paged the local history).
    pub(super) fn input_flags_of(&self, pane: &str) -> InputFlags {
        let known = self.pane_flags.get(pane).copied();
        let alt_screen = known.map_or_else(
            || {
                self.pane_screen_state
                    .get(pane)
                    .is_some_and(|s| s.alternate)
            },
            |flags| flags.alt_screen,
        );
        let wants_mouse = known.is_some_and(|flags| flags.wants_mouse);
        let claude = self.pane_runs_claude(pane);
        InputFlags {
            wants_mouse,
            alt_screen,
            scrolls_itself: wants_mouse && (alt_screen || claude),
            holds_ticks: !alt_screen && claude && known.is_none(),
            in_copy_mode: self
                .pane_screen_state
                .get(pane)
                .is_some_and(|s| s.copy_mode),
        }
    }

    /// Whether a pane runs Claude Code, by the command tmux reports: the
    /// agent poll's when it has one (it notices a Claude started after
    /// discovery), else discovery's.
    fn pane_runs_claude(&self, pane: &str) -> bool {
        let command = self
            .agent_hooks
            .get(pane)
            .and_then(|hook| hook.command.as_deref())
            .or_else(|| self.snapshot.panes.get(pane).map(|p| p.command.as_str()))
            .unwrap_or("");
        crate::model::display_name_for_command(command) == "claude"
    }

    /// Re-read the active pane's flags: on a switch, at a drag's start and
    /// with a tap.
    pub fn refresh_active_pane_mouse(&mut self, now: u64) {
        self.begin_input(now);
        self.refresh_active_pane_mouse_now();
        self.finish_input();
    }

    pub(super) fn refresh_active_pane_mouse_now(&mut self) {
        if !self.snapshot.is_attached {
            return;
        }
        let Some(pane) = self.snapshot.input_pane() else {
            return;
        };
        self.send_command(
            format!("display-message -p -t {pane} '#{{mouse_any_flag}} #{{alternate_on}}'"),
            Reply::MouseFlags {
                pane: pane.to_owned(),
            },
        );
    }

    /// Kept for the pane that was ASKED about: the active pane may have
    /// changed during the round trip.
    pub(super) fn mouse_flags_answered(&mut self, pane: &str, response: &CommandResponse) {
        // Fix (flags-refused): a refusal says nothing about the program;
        // Swift read it as "no mouse, primary screen".
        if response.is_error {
            return;
        }
        let flags: Vec<&str> = response
            .lines
            .first()
            .map(|line| words(trim_whitespaces(line), b' '))
            .unwrap_or_default();
        self.note_pane_flags(pane, &flags);
    }

    /// `mouse alternate`, as tmux prints them ("1"/"0").
    pub(super) fn note_pane_flags(&mut self, pane: &str, flags: &[&str]) {
        self.pane_flags.insert(
            pane.to_owned(),
            PaneFlags {
                wants_mouse: flags.first() == Some(&"1"),
                alt_screen: flags.len() > 1 && flags[1] == "1",
            },
        );
    }

    fn is_streaming(&self, pane: &str) -> bool {
        let now = self.wall();
        self.last_pane_output_at
            .get(pane)
            .is_some_and(|&at| now.since(at) < self.config.streaming_window)
    }

    /// Where a tick over the active pane goes. A program that scrolls
    /// itself (an alt-screen mouse app, Claude with the mouse) always gets
    /// the wheel — its own repaint would otherwise read as streaming and
    /// park every later tick. Claude on the primary screen with no flags yet
    /// gets nothing. A primary-screen mouse app that is repainting right now
    /// would fight the wheel over its viewport: park locally. An alt-screen
    /// pane without the mouse has no local history worth paging: its
    /// program gets the arrow keys (B500), once tmux has said the mouse is
    /// off — before that, nothing rather than a guess.
    pub fn scroll_route(&self) -> Option<(String, ScrollRoute)> {
        let pane = self.snapshot.input_pane()?;
        let flags = self.input_flags_of(&pane);
        let route = if flags.scrolls_itself {
            ScrollRoute::Wheel
        } else if flags.holds_ticks {
            ScrollRoute::Drop
        } else if flags.wants_mouse && !self.is_streaming(&pane) {
            ScrollRoute::Wheel
        } else if flags.alt_screen && self.pane_flags.contains_key(&pane) {
            ScrollRoute::Arrows
        } else if flags.alt_screen {
            ScrollRoute::Drop
        } else {
            ScrollRoute::Local
        };
        Some((pane, route))
    }

    /// A tick in rows (positive = older).
    pub fn scroll_lines(&mut self, lines: i64, now: u64) {
        self.begin_input(now);
        if lines != 0
            && let Some((pane, route)) = self.scroll_route()
        {
            match route {
                ScrollRoute::Wheel => self.send_wheel(lines, &pane),
                ScrollRoute::Arrows => self.send_arrows(lines, &pane),
                ScrollRoute::Drop => {}
                ScrollRoute::Local => self.view(&pane, ViewAction::ScrollLocal { lines }),
            }
        }
        self.finish_input();
    }

    /// A pan in pixels (positive = older): the wheel gets whole rows with the
    /// remainder carried; the local scrollback moves by the exact distance
    /// (done by the host, which knows what its view consumed).
    pub fn scroll_pixels(&mut self, dy: f64, cell_height: f64, now: u64) -> PixelScroll {
        self.begin_input(now);
        let result = match self.scroll_route() {
            None => PixelScroll::Consumed(0.0),
            Some(_) if dy == 0.0 => PixelScroll::Consumed(0.0),
            Some((pane, ScrollRoute::Wheel)) => {
                let rows = self.wheel_carry.take(dy, cell_height);
                if rows != 0 {
                    self.send_wheel(rows, &pane);
                }
                PixelScroll::Consumed(dy)
            }
            Some((pane, ScrollRoute::Arrows)) => {
                let rows = self.wheel_carry.take(dy, cell_height);
                if rows != 0 {
                    self.send_arrows(rows, &pane);
                }
                PixelScroll::Consumed(dy)
            }
            Some((_, ScrollRoute::Drop)) => PixelScroll::Consumed(0.0),
            // A pane without a terminal has no scrollback to move.
            Some((pane, ScrollRoute::Local)) if !self.streams.contains_key(&pane) => {
                PixelScroll::Consumed(0.0)
            }
            Some((pane, ScrollRoute::Local)) => PixelScroll::Local { pane },
        };
        self.finish_input();
        result
    }

    /// The drag came to rest: a locally scrolled pane docks onto the bottom
    /// if it stopped within half a row.
    pub fn scroll_did_end(&mut self, now: u64) {
        self.begin_input(now);
        if let Some((pane, ScrollRoute::Local)) = self.scroll_route() {
            self.view(&pane, ViewAction::SettleLocalScroll);
        }
        self.finish_input();
    }

    /// A drag starts: drop the carried remainder, re-read the flags.
    pub fn scroll_began(&mut self, now: u64) {
        self.begin_input(now);
        self.wheel_carry.reset();
        self.refresh_active_pane_mouse_now();
        self.finish_input();
    }

    /// Wheel events at the pane's centre, through the keystroke path (they
    /// count as an interaction).
    /// `lines` arrow presses (positive: older, up), one `send-keys` for
    /// tmux to encode by the pane's real cursor-key mode (the arrow-keys
    /// rule), at most [`MAX_ARROWS`] at once (B500).
    fn send_arrows(&mut self, lines: i64, pane: &str) {
        if lines == 0 || !is_valid_id(pane) {
            return;
        }
        // Behind a paste, with the keys typed after it (B485): in the order
        // given.
        if let Some((_, held)) = self.paste_gate.as_mut() {
            held.push(Held::Arrows {
                pane: pane.to_owned(),
                lines,
            });
            return;
        }
        let key = if lines > 0 { "Up" } else { "Down" };
        let count = lines.unsigned_abs().min(MAX_ARROWS);
        self.send(format!("send-keys -t {pane} -N {count} {key}"));
    }

    /// Aims the wheel of the scroll beginning at cell (`col`, `row`),
    /// 0-based in the input pane: its ticks go there, the coast after the
    /// lift too, until the next drag aims again (B578). Unaimed, a wheel
    /// goes to the pane's middle.
    pub fn aim_wheel(&mut self, col: i64, row: i64) {
        self.wheel_aim = Some((col, row));
    }

    fn send_wheel(&mut self, lines: i64, pane: &str) {
        let (cols, rows) = self.pane_grid(pane);
        let (col, row) = wheel_cell(self.wheel_aim, cols, rows);
        let wheel = wheel_bytes(lines, col, row);
        if !wheel.is_empty() {
            self.send_input_now(&wheel, pane);
        }
    }

    /// A tap at a 0-based cell, as a click for a program with the mouse on
    /// (a shell would echo the bytes). The flags are re-read with it.
    pub fn click(&mut self, col: i64, row: i64, now: u64) {
        self.begin_input(now);
        if self.active_input_flags().wants_mouse
            && let Some(pane) = self.snapshot.input_pane()
        {
            self.send_input_now(&click_bytes(col, row), &pane);
        }
        self.refresh_active_pane_mouse_now();
        self.finish_input();
    }

    /// The pane's view grid: born at the client size, then what the view
    /// reported; 80×24 for a pane without a terminal.
    fn pane_grid(&self, pane: &str) -> (i64, i64) {
        self.pane_grids.get(pane).copied().unwrap_or((80, 24))
    }

    // MARK: Small questions

    /// The server's tmux prefix (for the mosh copy-mode driver), answered
    /// with [`Notification::PrefixKey`]; `C-b` when tmux can't say.
    pub fn query_prefix_key(&mut self, now: u64) -> u64 {
        self.begin_input(now);
        let request = self.request_id();
        self.enqueue(
            "show-options -gqv prefix".to_owned(),
            Reply::PrefixKey { request },
            true,
        );
        self.finish_input();
        request
    }

    pub(super) fn prefix_key_answered(&mut self, request: u64, response: &CommandResponse) {
        let key = match response.lines.first() {
            Some(line) if !response.is_error => line.clone(),
            _ => "C-b".to_owned(),
        };
        self.notify(Notification::PrefixKey { request, key });
    }

    /// Cancel a copy-mode the active pane was left in by a connection that
    /// died mid-scroll (the mosh path, right after its renderer attaches).
    pub fn cancel_stale_copy_mode(&mut self, now: u64) {
        self.begin_input(now);
        if self.snapshot.is_attached
            && let Some(pane) = self.snapshot.input_pane()
        {
            self.send_command(
                format!("display-message -p -t {pane} '#{{pane_in_mode}}'"),
                Reply::StaleCopyMode { pane },
            );
        }
        self.finish_input();
    }

    /// `%pane-mode-changed`: which mode, asked (B614). The mosh path keeps
    /// a latch of the copy-mode its scrolls enter; this is the truth it
    /// follows — a `q` the person typed, a switch to another pane.
    pub(super) fn pane_mode_changed(&mut self, pane: &str) {
        self.send_command(
            format!("display-message -p -t {pane} '#{{pane_in_mode}} #{{alternate_on}}'"),
            Reply::PaneMode {
                pane: pane.to_owned(),
            },
        );
    }

    pub(super) fn pane_mode_answered(&mut self, pane: &str, response: &CommandResponse) {
        if response.is_error {
            return;
        }
        let Some(line) = response.lines.first() else {
            return;
        };
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 2 {
            return;
        }
        self.pane_screen_state.insert(
            pane.to_owned(),
            ScreenState {
                copy_mode: fields[0] == "1",
                alternate: fields[1] == "1",
            },
        );
    }

    pub(super) fn stale_copy_mode_answered(&mut self, pane: &str, response: &CommandResponse) {
        let in_mode = response
            .lines
            .first()
            .is_some_and(|line| trim_whitespaces(line) == "1");
        if in_mode {
            self.send(format!("send-keys -t {pane} -X cancel"));
        }
    }

    /// The pane's working directory as tmux reports it, answered with
    /// [`Notification::PanePath`]: `None` when not attached, refused, not
    /// absolute, or after 4 s without a reply. The line is kept exactly as
    /// tmux printed it: trailing spaces belong to the directory's name.
    pub fn request_pane_path(&mut self, pane: &str, now: u64) -> u64 {
        self.begin_input(now);
        let request = self.request_id();
        // Fix (path-target): Swift spliced the caller's pane unchecked.
        if self.snapshot.is_attached && is_valid_id(pane) {
            self.waiting_requests.insert(request);
            self.enqueue(
                format!("display-message -p -t {pane} '#{{pane_current_path}}'"),
                Reply::PanePath { request },
                true,
            );
            if self.waiting_requests.contains(&request) {
                self.arm(
                    Slot::PanePath(request),
                    4000,
                    Action::PanePathTimeout(request),
                );
            }
        } else {
            self.notify(Notification::PanePath {
                request,
                path: None,
            });
        }
        self.finish_input();
        request
    }

    pub(super) fn pane_path_answered(&mut self, request: u64, response: &CommandResponse) {
        if !self.waiting_requests.remove(&request) {
            return;
        }
        self.timers.cancel(&Slot::PanePath(request));
        let path = response.lines.first().cloned().unwrap_or_default();
        let absolute = path.as_bytes().first() == Some(&b'/');
        let path = (!response.is_error && absolute).then_some(path);
        self.notify(Notification::PanePath { request, path });
    }

    pub(super) fn pane_path_timed_out(&mut self, request: u64) {
        if self.waiting_requests.remove(&request) {
            self.notify(Notification::PanePath {
                request,
                path: None,
            });
        }
    }

    // MARK: Agent hooks

    /// Read every pane's `@moshpit_*` stamps in one command (the monitor's
    /// 2 s poll, and once right behind an attach's pane discovery).
    pub fn poll_agent_hooks(&mut self, now: u64) {
        self.begin_input(now);
        self.poll_agent_hooks_now();
        self.finish_input();
    }

    pub(super) fn poll_agent_hooks_now(&mut self) {
        if !self.snapshot.is_attached {
            return;
        }
        self.send_command(
            format!("list-panes -a -F {AGENT_HOOK_FORMAT}"),
            Reply::AgentHooks,
        );
    }

    pub(super) fn agent_hooks_answered(&mut self, response: &CommandResponse) {
        // Refused, it says nothing about the stamps: keep the last ones.
        if response.is_error {
            return;
        }
        self.agent_hooks = parse_agent_hooks(&response.lines);
        self.agent_hooks_loaded = true;
        self.notify(Notification::AgentHooksUpdated);
    }
}

fn capitalized(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B149: absurd travel saturates, and the carry works again after it.
    #[test]
    fn absurd_travel_saturates_and_the_carry_recovers() {
        let mut carry = RowCarry::default();
        // 1e300 / 1e-300 is past f64: infinite rows, the most i64 says.
        assert_eq!(carry.take(1e300, 1e-300), i64::MAX);
        assert_eq!(carry.remainder, 0.0);
        assert_eq!(carry.take(25.0, 10.0), 2);
        let mut down = RowCarry::default();
        assert_eq!(down.take(-1e300, 1.0), i64::MIN);
        assert_eq!(
            down.take(-1e300, f64::MIN_POSITIVE * f64::EPSILON),
            i64::MIN
        );
        assert_eq!(down.take(-15.0, 10.0), -1);
        // Swift's `guard cellHeight > 0`: NaN takes nothing.
        let mut odd = RowCarry::default();
        assert_eq!(odd.take(5.0, f64::NAN), 0);
        assert_eq!(odd.take(15.0, 10.0), 1);
        assert_eq!(odd.remainder, 5.0);
    }
}
