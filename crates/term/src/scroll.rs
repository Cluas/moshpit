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

/// The fastest a coast goes, flicks stacked or not, in pixels a second
/// (some 10,000 pt/s at 3x: about the fastest flick a phone reads).
pub const MAX_FLING: f64 = 30_000.0;

/// A flick adds the coast its touch stopped to its own speed (B523, as
/// UIScrollView does: quick flicks one after another go faster and
/// further) when it is this fast at least, the same way, and lifted within
/// [`STACK_WITHIN_MS`] of that touch. A slow drag, a long hold or the
/// other way carries nothing.
pub const STACK_MIN: f64 = 300.0;
pub const STACK_WITHIN_MS: u64 = 400;

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
    /// A move's own span when nothing came before it in the drag and
    /// nothing after it tells the drag's pace.
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
        // The window's first move began where the one before it ended; with
        // none before it, a pace's length earlier (the drag's own pace).
        let start = match first.checked_sub(1) {
            Some(before) => self.samples[before].0,
            None => {
                let at = self.samples[first].0;
                let pace = self
                    .samples
                    .get(first + 1)
                    .map_or(Self::FIRST_MS, |(next, _)| next.saturating_sub(at).max(1));
                at.saturating_sub(pace)
            }
        };
        let span = last.saturating_sub(start).max(1) as f64 / 1000.0;
        travel / span
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// Where a paced coast goes (B521, B523): a row's pixels, and the most a
/// step and a whole coast may take there, whatever the speed. Each step is
/// work for the far end; past these it falls behind and the scroll runs on
/// after the finger, which reads as jitter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pacing {
    row: f64,
    step_ceiling: f64,
    most_ceiling: f64,
}

impl Pacing {
    /// The host's rows (tmux's copy-mode, herdr's scrollback): a step is one
    /// command (tmux `send-keys -X -N <n> scroll-up`, herdr's scroll of `n`
    /// lines) and one repaint whatever its rows, so the step may be large:
    /// at most 12 rows (360 a second at 30 steps a second, some ten screens
    /// a second) and 400 in all (about eight screens' worth of history).
    /// Thirty small commands a second keep up over a slow link; more
    /// commands, or bigger ones, is a queue the person waits out.
    pub fn host(row: f64) -> Self {
        Pacing {
            row,
            step_ceiling: 12.0,
            most_ceiling: 400.0,
        }
    }

    /// Wheel reports for a program that wants the mouse (vim, less
    /// --mouse, Claude Code): every report is a scroll it computes and
    /// redraws, so at most 6 a step (what a drag's move sends at most) and
    /// 150 in all.
    pub fn wheel(row: f64) -> Self {
        Pacing {
            row,
            step_ceiling: 6.0,
            most_ceiling: 150.0,
        }
    }
}

/// A coast after a lift. This terminal's own history takes it by the
/// pixel; a wheel or the host's rows take it paced (at most every
/// [`Fling::PACE`]), each step and the whole coast bounded by what its
/// speed travels, at least [`Fling::STEP_ROWS`] and [`Fling::MOST_ROWS`]
/// and at most the route's ceilings ([`Pacing`]): a faster fling (flicks
/// stacked, B523) goes faster and further, and never floods the program
/// (a fast fling's every pixel was a flood it fell behind on: the jitter
/// 1.0.4 heard about).
#[derive(Debug, Clone)]
pub struct Fling {
    momentum: Momentum,
    /// The row's pixels and this coast's bounds in rows, for a paced coast;
    /// `None`: by the pixel.
    paced: Option<(f64, f64, f64)>,
    /// Travel not yet released, and how long since the last release.
    pending: f64,
    since: f64,
    /// Rows' worth of travel released so far.
    sent: f64,
}

impl Fling {
    pub const PACE: f64 = 1.0 / 30.0;
    /// The least a paced step and coast may take, however slow.
    pub const STEP_ROWS: f64 = 3.0;
    pub const MOST_ROWS: f64 = 60.0;
    /// How far a coast at a speed goes, in seconds of that speed: the sum
    /// of 0.998 a ms is 1 / (1 - 0.998) ms.
    const REACH_S: f64 = 0.5;

    /// A coast at `velocity` pixels a second; `pacing`: where it goes when
    /// that is a wheel or the host.
    pub fn new(velocity: f64, pacing: Option<Pacing>) -> Self {
        let paced = pacing.filter(|p| p.row > 0.0).map(|p| {
            let rows_a_second = velocity.abs() / p.row;
            // What the speed travels in a step and in all, kept between
            // the floor and the route's ceiling.
            let step = (rows_a_second * Self::PACE).clamp(Self::STEP_ROWS, p.step_ceiling);
            let most = (rows_a_second * Self::REACH_S).clamp(Self::MOST_ROWS, p.most_ceiling);
            (p.row, step, most)
        });
        Fling {
            momentum: Momentum { velocity },
            paced,
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
        self.paced.is_some()
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
        let Some((row, step, most)) = self.paced else {
            return (moved, going);
        };
        self.pending += moved;
        self.since += dt;
        if self.since + 1e-9 < Self::PACE && going {
            return (0.0, true);
        }
        self.since = 0.0;
        let room = (most - self.sent).max(0.0) * row;
        let cap = (step * row).min(room);
        // What the step can't take is dropped, not owed: a backlog is what
        // the program falls behind on.
        let out = self.pending.clamp(-cap, cap);
        self.pending = 0.0;
        self.sent += out.abs() / row;
        (out, going && self.sent < most)
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

        // Paced, a coast releases every 1/30 s (every other refresh at 60
        // Hz), each step and the whole coast what its speed travels, here
        // past the host's ceilings: 12 rows a step, 400 in all at most (what
        // the capped steps dropped while the speed was past them isn't
        // owed, so less).
        let row = 30.0;
        let run = |mut fling: Fling| {
            let (mut releases, mut frames) = (Vec::new(), 0);
            loop {
                let (moved, going) = fling.step(1.0 / 60.0);
                frames += 1;
                if moved != 0.0 {
                    releases.push((frames, moved / row));
                }
                if !going {
                    return releases;
                }
                assert!(frames < 10_000);
            }
        };
        let host = run(Fling::new(MAX_FLING, Some(Pacing::host(row))));
        assert!(host.iter().all(|(_, r)| *r <= 12.0 + 1e-9), "{host:?}");
        assert!(host.iter().any(|(_, r)| (*r - 12.0).abs() < 1e-9));
        let rows: f64 = host.iter().map(|(_, r)| r).sum();
        assert!(rows > 300.0 && rows <= 400.0 + 1e-6, "{rows}");
        assert!(host.windows(2).all(|w| w[1].0 - w[0].0 >= 2), "{host:?}");
        // The wheel's ceilings: 6 a step, 150 in all.
        let wheel = run(Fling::new(MAX_FLING, Some(Pacing::wheel(row))));
        assert!(wheel.iter().all(|(_, r)| *r <= 6.0 + 1e-9), "{wheel:?}");
        let rows: f64 = wheel.iter().map(|(_, r)| r).sum();
        assert!((rows - 150.0).abs() < 1e-6, "{rows}");
        // Below the ceilings a coast goes as far as its speed takes it, and
        // a faster one further and faster (B523).
        let slow = run(Fling::new(3_000.0, Some(Pacing::host(row))));
        let fast = run(Fling::new(9_000.0, Some(Pacing::host(row))));
        let (far, farther) = (
            slow.iter().map(|(_, r)| r).sum::<f64>(),
            fast.iter().map(|(_, r)| r).sum::<f64>(),
        );
        assert!((far - 1490.0 / row).abs() < 0.5, "{far}");
        assert!((farther - 4490.0 / row).abs() < 0.5, "{farther}");
        let peak = |r: &[(i32, f64)]| r.iter().map(|(_, x)| *x).fold(0.0, f64::max);
        assert!(
            peak(&fast) > 2.5 * peak(&slow),
            "{} {}",
            peak(&fast),
            peak(&slow)
        );

        // A slow paced fling ends with what it travelled, released.
        let mut slow = Fling::new(-300.0, Some(Pacing::host(50.0)));
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
