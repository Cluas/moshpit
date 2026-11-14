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
//! A lift within half a row of the live screen docks onto it.
//!
//! A lift coasts at the drag's speed (B521, the owner's call, overturning
//! 1.0.4's "no coast", V22): the speed is the travel of the drag's last
//! [`Velocity::WINDOW_MS`] (none when the finger rested before the lift),
//! slowed as UIScrollView slows. This terminal's own history coasts by the
//! pixel and stops at its ends; a wheel or the host's rows get the coast
//! paced and bounded ([`Fling`]), never a flood the program can't keep up
//! with. A touch stops a coast.

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

/// A coast's motion.
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
        let next = (v.abs() > STOP_SPEED).then_some(Momentum { velocity: v });
        (moved, next)
    }
}

/// Below this many pixels a second a coast has stopped (some 7 pt/s at 3x).
const STOP_SPEED: f64 = 20.0;

/// A lift slower than this many pixels a second doesn't coast: a drag let
/// go where it was put stays there.
pub const MIN_FLING: f64 = 60.0;

/// A drag's recent moves, for the speed it is let go at.
#[derive(Debug, Clone, Default)]
pub struct Velocity {
    /// When each move came (ms) and its pixels, oldest first.
    samples: std::collections::VecDeque<(u64, f64)>,
}

impl Velocity {
    /// The stretch of the drag its speed is read from.
    pub const WINDOW_MS: u64 = 100;
    /// A finger still this long before the lift let go at rest.
    pub const REST_MS: u64 = 50;
    /// A move's own span when nothing came before it in the drag.
    const FIRST_MS: u64 = 16;

    /// A move of `dy` pixels at `at_ms`.
    pub fn add(&mut self, at_ms: u64, dy: f64) {
        if !dy.is_finite() {
            return;
        }
        self.samples.push_back((at_ms, dy));
        // One move before the window stays: it dates the window's first.
        while self.samples.len() > 2 && self.samples[1].0 + Self::WINDOW_MS < at_ms {
            self.samples.pop_front();
        }
    }

    /// Pixels a second at `at_ms` (the lift): the travel of the moves in the
    /// last [`Self::WINDOW_MS`] over the time it took; none when the finger
    /// rested [`Self::REST_MS`] before the lift.
    pub fn at(&self, at_ms: u64) -> f64 {
        let Some(&(last, _)) = self.samples.back() else {
            return 0.0;
        };
        if at_ms.saturating_sub(last) > Self::REST_MS {
            return 0.0;
        }
        let from = at_ms.saturating_sub(Self::WINDOW_MS);
        let first = self.samples.iter().position(|(t, _)| *t >= from);
        let Some(first) = first else {
            return 0.0;
        };
        let travel: f64 = self.samples.iter().skip(first).map(|(_, dy)| dy).sum();
        let start = match first.checked_sub(1) {
            Some(before) => self.samples[before].0,
            None => self.samples[first].0.saturating_sub(Self::FIRST_MS),
        };
        let span = last.saturating_sub(start).max(1) as f64 / 1000.0;
        travel / span
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// A coast after a lift. This terminal's own history takes it by the
/// pixel; a wheel or the host's rows take it paced (at most every
/// [`Fling::PACE`]), at most [`Fling::STEP_ROWS`] rows a step and
/// [`Fling::MOST_ROWS`] in all: each step there is a wheel report or a
/// host command, and a fast fling's every pixel was a flood the program
/// fell behind on (the jitter 1.0.4 heard about).
#[derive(Debug, Clone)]
pub struct Fling {
    momentum: Momentum,
    /// The row's pixels, for a paced coast; `None`: by the pixel.
    paced_row: Option<f64>,
    /// Travel not yet released, and how long since the last release.
    pending: f64,
    since: f64,
    /// Rows' worth of travel released so far.
    sent: f64,
}

impl Fling {
    pub const PACE: f64 = 1.0 / 30.0;
    pub const STEP_ROWS: f64 = 3.0;
    pub const MOST_ROWS: f64 = 60.0;

    /// A coast at `velocity` pixels a second; `paced_row`: the row's pixels
    /// when it goes to a wheel or the host.
    pub fn new(velocity: f64, paced_row: Option<f64>) -> Self {
        Fling {
            momentum: Momentum { velocity },
            paced_row: paced_row.filter(|row| *row > 0.0),
            pending: 0.0,
            since: 0.0,
            sent: 0.0,
        }
    }

    pub fn velocity(&self) -> f64 {
        self.momentum.velocity
    }

    /// Whether it goes to a wheel or the host (paced and bounded).
    pub fn is_paced(&self) -> bool {
        self.paced_row.is_some()
    }

    /// `dt` seconds on: the pixels to move now (0: none this refresh), and
    /// whether it coasts on.
    pub fn step(&mut self, dt: f64) -> (f64, bool) {
        if !dt.is_finite() || dt <= 0.0 {
            return (0.0, true);
        }
        let (moved, next) = self.momentum.step(dt);
        let going = next.is_some();
        if let Some(m) = next {
            self.momentum = m;
        }
        let Some(row) = self.paced_row else {
            return (moved, going);
        };
        self.pending += moved;
        self.since += dt;
        if self.since + 1e-9 < Self::PACE && going {
            return (0.0, true);
        }
        self.since = 0.0;
        let room = (Self::MOST_ROWS - self.sent).max(0.0) * row;
        let cap = (Self::STEP_ROWS * row).min(room);
        // What the step can't take is dropped, not owed: a backlog is what
        // the program falls behind on.
        let out = self.pending.clamp(-cap, cap);
        self.pending = 0.0;
        self.sent += out.abs() / row;
        (out, going && self.sent < Self::MOST_ROWS)
    }
}

/// The model's state, kept by the terminal.
#[derive(Debug, Clone, Default)]
pub struct Scroll {
    /// Pixels into the history (0: the live screen).
    pub offset_px: i64,
    /// Travel not yet a whole row or tick.
    pub carry_px: f64,
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

    /// B521: the speed a drag is let go at is its last 100 ms of travel; a
    /// finger that rested before the lift lets go at rest.
    #[test]
    fn the_speed_is_the_last_stretch_of_the_drag() {
        let mut v = Velocity::default();
        // 10 px every 10 ms: 1000 px/s, an older slow start left out.
        v.add(0, 1.0);
        v.add(200, 1.0);
        for i in 1..=12 {
            v.add(200 + i * 10, 10.0);
        }
        let at = v.at(320);
        assert!((at - 1000.0).abs() < 1.0, "{at}");
        // Upward, the same.
        let mut up = Velocity::default();
        for i in 1..=6 {
            up.add(i * 16, -16.0);
        }
        assert!((up.at(96) + 1000.0).abs() < 1.0, "{}", up.at(96));
        // The finger rested 60 ms: no speed.
        assert_eq!(v.at(380), 0.0);
        assert_eq!(Velocity::default().at(5), 0.0);
        // One move only: its own span.
        let mut one = Velocity::default();
        one.add(100, 16.0);
        assert!((one.at(100) - 1000.0).abs() < 1.0);
    }

    /// B521: by the pixel, a coast goes as far as UIScrollView's would and
    /// stops; paced, it releases every 1/30 s, at most three rows a step and
    /// sixty in all, whatever the speed.
    #[test]
    fn a_fling_paces_and_bounds_a_wheel_or_the_hosts_rows() {
        let mut free = Fling::new(3000.0, None);
        let (mut total, mut steps) = (0.0, 0);
        loop {
            let (moved, going) = free.step(1.0 / 60.0);
            total += moved;
            steps += 1;
            if !going {
                break;
            }
            assert!(steps < 1000);
        }
        // 3000 px/s at 0.998 a ms, down to the stop at 20: 1490 px.
        assert!((total - 1490.0).abs() < 5.0, "{total}");

        let row = 50.0;
        let mut paced = Fling::new(20_000.0, Some(row));
        let mut releases = Vec::new();
        let mut frames = 0;
        loop {
            let (moved, going) = paced.step(1.0 / 60.0);
            frames += 1;
            if moved != 0.0 {
                releases.push((frames, moved));
            }
            if !going {
                break;
            }
            assert!(frames < 10_000);
        }
        assert!(
            releases.iter().all(|(_, m)| *m <= 3.0 * row + 1e-9),
            "{releases:?}"
        );
        let rows: f64 = releases.iter().map(|(_, m)| m / row).sum();
        assert!((rows - 60.0).abs() < 1e-6, "sixty rows in all: {rows}");
        // Every other frame at 60 Hz.
        assert!(
            releases.windows(2).all(|w| w[1].0 - w[0].0 >= 2),
            "{releases:?}"
        );

        // A slow paced fling ends with what it travelled, released.
        let mut slow = Fling::new(-300.0, Some(row));
        let mut travelled = 0.0;
        loop {
            let (moved, going) = slow.step(1.0 / 60.0);
            travelled += moved;
            if !going {
                break;
            }
        }
        // 300 px/s down to the stop at 20: 140 px.
        assert!((travelled + 140.0).abs() < 5.0, "{travelled}");
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
