//! The five easter eggs (PET-DESIGN §15.7), as pure rules: the same
//! inputs, the same answer, on every device. None needs grinding and none
//! is limited in time; none reads anything a terminal shows.

use std::collections::VecDeque;

use crate::genome::NIGHT_HOUR;

/// An easter egg.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Egg {
    /// Hatched between 03:00 and 03:59 local time: the hidden moth.
    NightHatch,
    /// The first mosh roam after the hatch that kept its session: a
    /// travel badge from then on.
    MadeItThrough,
    /// The thousandth finished task: a gold outline.
    Task1000,
    /// Named `sudo`: a small crown, for as long as the name stands.
    Sudo,
    /// Seven taps inside two seconds: a pogo.
    Pogo,
}

/// The record's flag for [`Egg::MadeItThrough`].
pub const FLAG_ROAMED: u8 = 1 << 0;
/// The record's flag for [`Egg::Task1000`].
pub const FLAG_GOLD: u8 = 1 << 1;

/// Finished tasks for the gold outline.
pub const GOLD_TASKS: u64 = 1000;

/// Taps that start a pogo, and the window they fall in.
pub const POGO_TAPS: usize = 7;
pub const POGO_WINDOW_S: f64 = 2.0;

/// Whether a hatch revealed at this local hour brings the moth.
pub fn night_hatch(hatch_hour: u8) -> bool {
    hatch_hour == NIGHT_HOUR
}

/// Whether this total earns the gold outline.
pub fn gold(total_tasks: u64) -> bool {
    total_tasks >= GOLD_TASKS
}

/// Whether the name wears the crown: `sudo`, whatever the case and the
/// spaces around it.
pub fn crowned(name: &str) -> bool {
    name.trim().to_lowercase() == "sudo"
}

/// Counts taps for the pogo.
#[derive(Debug, Clone, Default)]
pub struct Taps {
    times: VecDeque<f64>,
}

impl Taps {
    /// A tap at `now` (seconds). True when it completes seven inside two
    /// seconds; the count starts over after a pogo.
    pub fn tap(&mut self, now: f64) -> bool {
        self.times.push_back(now);
        while self.times.front().is_some_and(|t| now - t > POGO_WINDOW_S) {
            self.times.pop_front();
        }
        if self.times.len() >= POGO_TAPS {
            self.times.clear();
            return true;
        }
        false
    }
}
