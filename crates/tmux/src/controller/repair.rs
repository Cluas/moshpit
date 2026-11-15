//! Backfill and repair frames: putting what tmux holds into a pane's
//! terminal.
//!
//! `%output` only carries NEW bytes and tmux never reconciles a client's
//! screen, so the controller repaints from tmux's model whenever the local
//! screen may have drifted: on attach (history plus the screen), after a
//! resize or a switch (twice: right away, and once the program's own
//! SIGWINCH redraw has settled), once a disruption goes quiet (the
//! convergence capture), after `%continue`. Every peer on this app is a diff
//! renderer, so a repair frame has to actually land: rejected frames (taken
//! at another client's size) are recaptured, a frame never overtakes the
//! history dump it belongs on top of, and a frame blanks the screen it
//! replaces instead of drawing over it.
//!
//! Never with `ESC [2J` (B364): the terminal core moves the screen into its
//! history on that erase (tmux's scroll-on-clear, which a pane's own `clear`
//! must keep), so every repaint — the attach's, a switch's, the return from
//! the background's, the one after each Enter — left one more copy of the
//! screen in the pane's history. A frame blanks the first row, then the
//! rest from the second row down ([`BLANK_SCREEN`]: an erase that doesn't
//! start at the top left never pushes), and the history dump is pushed into
//! the history whole, from the bottom row, so the pane's local history is
//! tmux's, line for line.

use crate::control::CommandResponse;
use crate::format::{cursor_position, frame_exceeds_width, swift_int, words};

use super::timers::{Action, Slot};
use super::{BACKFILL_HISTORY_LINES, Controller, PinReason, Reply, ScreenState, ViewAction};

/// Blanks the screen with the default pen and never pushes it into the
/// terminal's history: the first row, then from the second row down (B364).
pub(crate) const BLANK_SCREEN: &str = "\u{1b}[0m\u{1b}[H\u{1b}[K\u{1b}[2;1H\u{1b}[J";

impl Controller {
    // MARK: Backfill

    /// Replay what the pane printed before we attached (`backfill`): once
    /// per pane, and only once the command can actually be sent.
    pub(super) fn backfill(&mut self, pane: &str) {
        if self.backfilled_panes.contains(pane) || !self.snapshot.is_attached {
            return;
        }
        self.backfilled_panes.insert(pane.to_owned());
        self.backfills_in_flight.insert(pane.to_owned());
        // A dump whose reply never lands must not hold this pane's repaints
        // forever.
        self.arm(
            Slot::BackfillTimeout(pane.to_owned()),
            5000,
            Action::BackfillTimeout(pane.to_owned()),
        );
        match self.pane_screen_state.get(pane).copied() {
            Some(state) => self.issue_backfill_capture(pane, state),
            // A pane discovery hasn't described yet: ask, because replaying
            // history into an alternate-screen TUI corrupts it.
            None => self.send_command(
                format!("display-message -p -t {pane} '#{{alternate_on}} #{{pane_in_mode}}'"),
                Reply::BackfillProbe {
                    pane: pane.to_owned(),
                },
            ),
        }
    }

    pub(super) fn backfill_probe_answered(&mut self, pane: &str, response: &CommandResponse) {
        let fields: Vec<&str> = response
            .lines
            .first()
            .map(|line| words(line, b' '))
            .unwrap_or_default();
        let state = ScreenState {
            alternate: fields.first() == Some(&"1"),
            copy_mode: fields.len() >= 2 && fields[1] == "1",
        };
        self.pane_screen_state.insert(pane.to_owned(), state);
        self.issue_backfill_capture(pane, state);
    }

    /// The one `capture-pane` that fills a pane: an alternate-screen TUI gets
    /// its current screen only (tmux never re-emits a drawn TUI, and history
    /// replayed into one corrupts it); a primary-screen pane gets the history
    /// ABOVE the screen (`-E -1`), since the attach's own repaint brings the
    /// screen.
    fn issue_backfill_capture(&mut self, pane: &str, state: ScreenState) {
        // Still in copy-mode on first sight: a leftover from a connection
        // that died mid-scroll. The next swipe would continue from its
        // offset.
        if state.copy_mode {
            self.send(format!("send-keys -t {pane} -X cancel"));
        }
        let range = if state.alternate {
            String::new()
        } else {
            format!("-S -{BACKFILL_HISTORY_LINES} -E -1")
        };
        self.send_command(
            format!("capture-pane -p -e {range} -t {pane}"),
            Reply::Backfill {
                pane: pane.to_owned(),
                alternate: state.alternate,
            },
        );
        if !state.alternate {
            // Fix (B585): the screen right behind the history above it. The
            // frame used to go out once the dump's reply was in, a round
            // trip later: a pane printing meanwhile scrolled lines off
            // tmux's screen that were in neither capture — gone from the
            // phone's history. tmux runs what a client sent together back
            // to back; the replies keep this order, so the frame still
            // lands on the dump, never under it.
            self.resyncs_in_flight.insert(pane.to_owned());
            self.arm(
                Slot::ResyncTimeout(pane.to_owned()),
                10_000,
                Action::ResyncTimeout(pane.to_owned()),
            );
            self.send_resync_pair(pane, false);
        }
    }

    pub(super) fn backfill_answered(
        &mut self,
        pane: &str,
        alternate: bool,
        response: &CommandResponse,
    ) {
        self.feed_backfill(pane, alternate, response);
        // Whatever the reply held, the dump is over.
        self.finish_backfill(pane);
    }

    fn feed_backfill(&mut self, pane: &str, alternate: bool, response: &CommandResponse) {
        // Fix (refused-capture): tmux refusing the capture (`can't find
        // pane`) sends its complaint, not the pane: Swift fed it into the
        // terminal as history.
        if response.is_error {
            return;
        }
        let lines = trimmed_frame(&response.lines);
        if lines.is_empty() {
            return;
        }
        // Laid out for another client's width: feeding it wraps every line
        // into the scrollback for good. Unclaim it and recapture once the
        // sizes have settled.
        if frame_exceeds_width(lines, self.last_client_size.0) {
            self.backfilled_panes.remove(pane);
            self.schedule_backfill_retry(pane);
            return;
        }
        // A retried dump lands after the attach painted the live screen:
        // the screen goes back on top of the history it pushed away.
        if self.backfill_reject_streak.get(pane).copied().unwrap_or(0) > 0 {
            self.backfill_reject_streak.insert(pane.to_owned(), 0);
            self.deferred_resyncs.insert(pane.to_owned(), true);
        }
        if alternate {
            // The screen only: the repaint that follows draws over it.
            self.feed(pane, lines.join("\r\n").into_bytes());
            return;
        }
        // The history above the screen becomes the pane's local history,
        // exactly (B364): whatever is there goes (a retried dump lands on a
        // painted screen), the dump is printed from the top of a blank
        // screen and pushed whole into the history from the bottom row —
        // none of it left on screen for the repaint to draw over, nothing
        // pushed twice. The repaint that follows draws the screen.
        let rows = usize::try_from(self.pane_rows(pane)).unwrap_or(1).max(1);
        let mut text = String::from(BLANK_SCREEN);
        text.push_str("\u{1b}[3J\u{1b}[H");
        text.push_str(&lines.join("\r\n"));
        text.push_str("\u{1b}[0m\u{1b}[9999;1H");
        text.push_str(&"\n".repeat(lines.len().min(rows)));
        self.feed(pane, text.into_bytes());
    }

    /// The rows of a pane's terminal: what its view reported, else the
    /// client's.
    fn pane_rows(&self, pane: &str) -> i64 {
        self.pane_grids
            .get(pane)
            .map_or(self.last_client_size.1, |grid| grid.1)
    }

    /// A dump laid out for another width, recaptured once the sizes settle
    /// (at most four times; a size war can't loop it).
    fn schedule_backfill_retry(&mut self, pane: &str) {
        let streak = self.backfill_reject_streak.get(pane).copied().unwrap_or(0) + 1;
        self.backfill_reject_streak.insert(pane.to_owned(), streak);
        if streak > 4 {
            return;
        }
        self.arm(
            Slot::BackfillRetry(pane.to_owned()),
            700,
            Action::BackfillRetry(pane.to_owned()),
        );
    }

    /// The dump landed (or its 5 s bound passed): run the repaint parked
    /// behind it, under a veil re-armed from now.
    pub(super) fn finish_backfill(&mut self, pane: &str) {
        self.timers.cancel(&Slot::BackfillTimeout(pane.to_owned()));
        if !self.backfills_in_flight.remove(pane) {
            return;
        }
        let Some(reveal) = self.deferred_resyncs.remove(pane) else {
            return;
        };
        if self.streams.contains_key(pane) {
            self.view(pane, ViewAction::VeilUnlessCovered);
            let grace = self.cover_grace();
            self.view(pane, ViewAction::ExtendCover { seconds: grace });
        }
        self.resync_pane(pane, reveal);
    }

    // MARK: Repair frames

    /// Repaint the active pane from tmux's model (`resyncActivePane`).
    /// `reveal`: whether this frame may lift a cover; a capture known to
    /// race the program's own redraw corrects the buffer but stays covered.
    pub fn resync_active_pane_now(&mut self, reveal: bool, now: u64) {
        self.begin_input(now);
        self.resync_active_pane(reveal);
        self.finish_input();
    }

    pub(super) fn resync_active_pane(&mut self, reveal: bool) {
        if !self.config.renders_output || !self.snapshot.is_attached {
            return;
        }
        if let Some(pane) = self.snapshot.input_pane() {
            self.resync_pane(&pane, reveal);
        }
    }

    /// Capture the pane's screen, then its cursor (`resyncPane`). Never ahead
    /// of the pane's own backfill (the dump would scroll the frame away); one
    /// pair in flight per pane, with at most one more queued behind it — a
    /// burst of requests costs one capture that is current when it runs.
    pub(super) fn resync_pane(&mut self, pane: &str, reveal: bool) {
        if self.backfills_in_flight.contains(pane) {
            let parked = self.deferred_resyncs.get(pane).copied().unwrap_or(false);
            self.deferred_resyncs
                .insert(pane.to_owned(), reveal || parked);
            return;
        }
        if self.resyncs_in_flight.contains(pane) {
            let queued = self.queued_resyncs.get(pane).copied().unwrap_or(false);
            self.queued_resyncs
                .insert(pane.to_owned(), reveal || queued);
            return;
        }
        self.resyncs_in_flight.insert(pane.to_owned());
        // A connection that dies between the two replies must not leave the
        // pane "in flight" for good.
        self.arm(
            Slot::ResyncTimeout(pane.to_owned()),
            10_000,
            Action::ResyncTimeout(pane.to_owned()),
        );
        self.send_resync_pair(pane, reveal);
    }

    /// The repaint's two commands: the screen, then the cursor.
    fn send_resync_pair(&mut self, pane: &str, reveal: bool) {
        // Frame first, cursor second: %output between the two replies lands
        // on the fresh frame and is already in the cursor's answer.
        self.send_command(
            format!("capture-pane -p -e -t {pane}"),
            Reply::ResyncFrame {
                pane: pane.to_owned(),
                reveal,
                sent_at: self.now,
            },
        );
        // The program's flags ride along (B363): known from the attach's own
        // repaint on, kept fresh with every one, no extra round trip.
        self.send_command(
            format!(
                "display-message -p -t {pane} '#{{cursor_x}} #{{cursor_y}} #{{mouse_any_flag}} #{{alternate_on}}'"
            ),
            Reply::ResyncCursor {
                pane: pane.to_owned(),
            },
        );
    }

    pub(super) fn resync_frame_answered(
        &mut self,
        pane: &str,
        reveal: bool,
        sent_at: u64,
        response: &CommandResponse,
    ) {
        // The capture's round trip is the best live measure of the network
        // for every veil bound (`coverGrace`), in Swift's arithmetic:
        // whole seconds plus attoseconds.
        let elapsed = self.now.saturating_sub(sent_at);
        self.last_resync_round_trip =
            (elapsed / 1000) as f64 + ((elapsed % 1000) * 1_000_000_000_000_000) as f64 / 1e18;
        // Fix (refused-capture): a refused capture has no frame to paint.
        if response.is_error {
            return;
        }
        let lines = trimmed_frame(&response.lines);
        let (cols, rows) = self.last_client_size;
        // Captured at another client's size: authoritative for a grid this
        // terminal doesn't have. Drop it, re-pin, and recapture until one
        // lands at ours.
        if lines.len() as i64 > rows || frame_exceeds_width(lines, cols) {
            if let Some(window) = self.snapshot.active_window_id.clone() {
                self.request_pin(&window, PinReason::Resize);
            }
            self.schedule_resync_retry(pane, reveal);
            return;
        }
        self.resync_reject_streak.insert(pane.to_owned(), 0);
        // The screen blanked without a push into the history (B364), the
        // frame from home, SGR reset; the cursor follows in step two.
        let mut text = String::from(BLANK_SCREEN);
        text.push_str("\u{1b}[H");
        text.push_str(&lines.join("\r\n"));
        text.push_str("\u{1b}[0m");
        // A held resize and the frame that answers it land in one turn.
        self.release_deferred_resize(pane);
        self.feed(pane, text.into_bytes());
        if reveal {
            self.view(pane, ViewAction::Reveal);
            if self.streams.contains_key(pane) {
                self.note_pane_revealed();
            }
        }
    }

    pub(super) fn resync_cursor_answered(&mut self, pane: &str, response: &CommandResponse) {
        // Fix (refused-capture): no cursor to restore from a refusal (Swift
        // moved the cursor home).
        if !response.is_error {
            let fields: Vec<&str> = response
                .lines
                .first()
                .map(|line| words(line, b' '))
                .unwrap_or_default();
            // The cursor is the first two; the program's flags follow
            // (B363), "1"/"0" each.
            let parts: Vec<i64> = fields
                .iter()
                .take(2)
                .filter_map(|field| swift_int(field))
                .collect();
            let (col, row) = if parts.len() == 2 {
                (parts[0], parts[1])
            } else {
                (0, 0)
            };
            self.feed(pane, cursor_position(col, row).into_bytes());
            if fields.len() >= 4 {
                self.note_pane_flags(pane, &fields[2..4]);
            }
        }
        // The cursor is the pair's second half: the resync is over here.
        self.finish_resync(pane);
    }

    pub(super) fn finish_resync(&mut self, pane: &str) {
        self.timers.cancel(&Slot::ResyncTimeout(pane.to_owned()));
        if !self.resyncs_in_flight.remove(pane) {
            return;
        }
        if let Some(reveal) = self.queued_resyncs.remove(pane) {
            self.resync_pane(pane, reveal);
        }
    }

    /// A frame at a size this grid can't hold: recapture, bounded.
    fn schedule_resync_retry(&mut self, pane: &str, reveal: bool) {
        let streak = self.resync_reject_streak.get(pane).copied().unwrap_or(0) + 1;
        self.resync_reject_streak.insert(pane.to_owned(), streak);
        if streak > 4 {
            return;
        }
        self.arm(
            Slot::ResyncRetry,
            300,
            Action::ResyncRetry {
                pane: pane.to_owned(),
                reveal,
            },
        );
    }

    /// The two-pass repaint after a resize or switch
    /// (`resyncActivePaneSettling`): a silent capture now, the revealing one
    /// once the program's redraw has settled, and a convergence capture once
    /// output goes quiet.
    pub(super) fn resync_active_pane_settling(&mut self) {
        self.arm_convergence_resync();
        self.resync_active_pane(false);
        let grace = self.cover_grace();
        self.extend_active_cover_timeout(grace);
        self.arm(Slot::SettleResync, 700, Action::SettledResync);
    }

    /// How long a cover may outlive its default while a resync is in flight:
    /// generous against the measured round trip, 2–8 s.
    pub(super) fn cover_grace(&self) -> f64 {
        (self.last_resync_round_trip * 1.5 + 0.5).clamp(2.0, 8.0)
    }

    pub(super) fn extend_active_cover_timeout(&mut self, seconds: f64) {
        if let Some(pane) = self.snapshot.input_pane() {
            self.view(&pane, ViewAction::ExtendCover { seconds });
        }
    }

    // MARK: Convergence

    /// After a disruption, one more capture once the pane has finished
    /// writing — or after 3 s of continuous output at the latest.
    pub(super) fn arm_convergence_resync(&mut self) {
        self.convergence_pending = true;
        self.convergence_saw_output = false;
        self.schedule_convergence_resync();
        if !self.timers.is_armed(&Slot::ConvergenceDeadline) {
            self.arm(Slot::ConvergenceDeadline, 3000, Action::Convergence);
        }
    }

    /// Restart the quiet debounce: 600 ms once output flows, before that a
    /// round trip and a half (200–600 ms): a switch to an idle pane writes
    /// nothing at all.
    pub(super) fn schedule_convergence_resync(&mut self) {
        if !self.convergence_pending {
            return;
        }
        let debounce = if self.convergence_saw_output {
            600
        } else {
            ((self.last_resync_round_trip * 1.5).clamp(0.2, 0.6) * 1000.0) as u64
        };
        self.arm(Slot::ConvergenceDebounce, debounce, Action::Convergence);
    }

    pub(super) fn fire_convergence_resync(&mut self) {
        if !self.convergence_pending {
            return;
        }
        self.convergence_pending = false;
        self.timers.cancel(&Slot::ConvergenceDebounce);
        self.timers.cancel(&Slot::ConvergenceDeadline);
        self.resync_active_pane(true);
    }

    // MARK: Held resizes

    /// Apply a pane's held resize, if it holds one.
    pub(super) fn release_deferred_resize(&mut self, pane: &str) {
        self.timers.cancel(&Slot::DeferredResize(pane.to_owned()));
        if self.pane_resize_awaiting_capture.remove(pane) {
            self.view(pane, ViewAction::ApplyDeferredResize);
            if let Some(grid) = self.held_grids.remove(pane) {
                self.pane_grids.insert(pane.to_owned(), grid);
            }
        }
    }
}

/// A capture's lines without the trailing blank ones (whitespace-only,
/// Foundation's whitespace set).
fn trimmed_frame(lines: &[String]) -> &[String] {
    let mut end = lines.len();
    while end > 0 && moshpit_herdr::text::trim_whitespaces(&lines[end - 1]).is_empty() {
        end -= 1;
    }
    &lines[..end]
}
