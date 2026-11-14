//! The scroll model: a drag over the terminal, in device pixels, becomes
//! one of three things (docs/nsh/TERMINAL-VIEW.md V19–V22):
//!
//! - the host's rows, when the session scrolls its own buffer (tmux's
//!   copy-mode, herdr): whole rows, the remainder carried to the next move
//!   so a slow drag adds up to what a fast one does;
//! - wheel ticks to the program, when it asked for the mouse (vim, less
//!   --mouse, Claude Code), carried the same way, at most six per move;
//! - this terminal's own history, by the pixel: the view draws its rows
//!   shifted by the part of a row the offset leaves, on the device's pixel
//!   grid (fork patch 21), with the row under the viewport in its frames.
//!
//! A lift within half a row of the live screen docks onto it. No coast by
//! default (1.0.4's device feedback: content that moves after the finger
//! stops reads as overshoot); momentum is data a view may play
//! (`Momentum`), off unless the host turns it on.

/// What a scroll step did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ScrollAction {
    #[default]
    None,
    /// This terminal's history moved this many pixels (positive: older).
    Local { moved_px: i64 },
    /// Wheel reports for the program, encoded.
    Wheel(Vec<u8>),
    /// Rows for the host to scroll (positive: older).
    Host { rows: i32 },
}

/// A fling's motion, if the host wants coasting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Momentum {
    /// Pixels per second, positive toward older output.
    pub velocity: f64,
}

/// Velocity lost per second (UIScrollView's normal rate, 0.998 a ms).
const DECELERATION: f64 = 0.998;

impl Momentum {
    /// The pixels to move after `dt` seconds, and the motion left (None:
    /// it has stopped).
    pub fn step(self, dt: f64) -> (f64, Option<Momentum>) {
        let factor = DECELERATION.powf(dt * 1000.0);
        let v = self.velocity * factor;
        let moved = (self.velocity - v) / (1.0 - DECELERATION) / 1000.0;
        let next = (v.abs() > 20.0).then_some(Momentum { velocity: v });
        (moved, next)
    }
}

/// The model's state, kept by the terminal.
#[derive(Debug, Clone, Default)]
pub struct Scroll {
    /// Pixels into the history (0: the live screen).
    pub offset_px: i64,
    /// Travel not yet a whole row or tick.
    pub carry_px: f64,
    /// Whether a fling coasts.
    pub coast: bool,
    pub momentum: Option<Momentum>,
}

impl Scroll {
    /// Whole units of `unit_px` out of the carried travel plus `dy`; the
    /// rest stays carried.
    pub fn take(&mut self, dy: f64, unit_px: f64) -> i64 {
        if !dy.is_finite() || unit_px <= 0.0 {
            return 0;
        }
        self.carry_px += dy;
        let units = (self.carry_px / unit_px).trunc();
        self.carry_px -= units * unit_px;
        if !self.carry_px.is_finite() {
            self.carry_px = 0.0;
        }
        units.clamp(i64::MIN as f64, i64::MAX as f64) as i64
    }

    /// The whole lines the view is scrolled back for `offset_px`.
    pub fn lines(offset_px: i64, cell_px: i64) -> i64 {
        if offset_px <= 0 || cell_px <= 0 {
            0
        } else {
            (offset_px + cell_px - 1) / cell_px
        }
    }

    /// How far up the rows are drawn: the part of a row the offset leaves.
    pub fn shift(offset_px: i64, cell_px: i64) -> i64 {
        Self::lines(offset_px, cell_px) * cell_px - offset_px
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slow_drag_adds_up() {
        let mut s = Scroll::default();
        let rows: i64 = (0..30).map(|_| s.take(1.0, 10.0)).sum();
        assert_eq!(rows, 3);
        assert_eq!(s.take(-35.0, 10.0), -3);
        assert_eq!(s.take(f64::NAN, 10.0), 0);
    }

    #[test]
    fn shift_and_lines() {
        assert_eq!((Scroll::lines(0, 30), Scroll::shift(0, 30)), (0, 0));
        assert_eq!((Scroll::lines(1, 30), Scroll::shift(1, 30)), (1, 29));
        assert_eq!((Scroll::lines(30, 30), Scroll::shift(30, 30)), (1, 0));
        assert_eq!((Scroll::lines(45, 30), Scroll::shift(45, 30)), (2, 15));
    }

    #[test]
    fn momentum_slows_and_stops() {
        let mut m = Some(Momentum { velocity: 3000.0 });
        let mut total = 0.0;
        let mut steps = 0;
        while let Some(now) = m {
            let (moved, next) = now.step(1.0 / 60.0);
            total += moved;
            m = next;
            steps += 1;
            assert!(steps < 1000);
        }
        assert!(total > 500.0 && total < 3000.0, "{total}");
    }
}
