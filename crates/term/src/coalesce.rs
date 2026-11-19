//! When a pane frames what it parsed: implicit frame coalescing.
//!
//! A program that redraws without synchronized output (DEC 2026) still
//! writes each redraw in one go, but the bytes can reach the pane in
//! several reads: Gemini CLI erases its input box and draws it again in one
//! write that arrived as two reads in the same millisecond, and the frame
//! published between them showed the box half erased (docs/nsh/CLI-LAB.md).
//! tmux does not pass DEC 2026 through, and several CLIs turn it off inside
//! tmux or over SSH, so the markers cannot be relied on.
//!
//! So output is framed once the stream has gone quiet for [`QUIET`] (the
//! rest of a split write comes back to back), and at most [`CAP`] after the
//! first byte not framed yet (a stream that never pauses still paints at a
//! display's rate). Output that ends a synchronized update is framed at
//! once: the remote said the screen is whole.
//!
//! The cost is up to [`QUIET`] more before a typed key's echo shows, when
//! the echo is not inside a synchronized update.

use std::time::{Duration, Instant};

/// How long the stream must stay quiet before what it brought is framed.
/// Of the lab's gaps of 20 ms or less between two reads (979, keys
/// excluded), 76% are under 1 ms and 87% at most 4 ms. 4 ms is a quarter of a 60 Hz frame:
/// a typed echo shows at most one display refresh later, and only when it
/// lands within 4 ms of a refresh.
pub const QUIET: Duration = Duration::from_millis(4);

/// The longest output is held: about a display refresh, as the pane's
/// backlog frames ([`crate::pane::PUBLISH`]).
pub const CAP: Duration = Duration::from_millis(16);

/// Output parsed and not framed yet: when it came.
#[derive(Debug, Default, Clone)]
pub struct Coalescer {
    first: Option<Instant>,
    last: Option<Instant>,
    settled: bool,
}

impl Coalescer {
    /// Output was parsed at `now`, up to where the stream had nothing more.
    /// `settled`: it ended with the end of a synchronized update.
    pub fn output(&mut self, now: Instant, settled: bool) {
        self.first.get_or_insert(now);
        self.last = Some(now);
        self.settled = settled;
    }

    /// When the output held is to be framed; `None` if none is held.
    pub fn due(&self) -> Option<Instant> {
        let (first, last) = (self.first?, self.last?);
        if self.settled {
            return Some(last);
        }
        Some((last + QUIET).min(first + CAP))
    }

    /// The output held is due at `now`.
    pub fn ready(&self, now: Instant) -> bool {
        self.due().is_some_and(|at| at <= now)
    }

    /// The output held was framed (or left in the terminal for the frame
    /// the view asks for next).
    pub fn framed(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Plays reads at the given times (ms) and returns the times frames were
    /// published at, the way a pane does: a frame when the held output is
    /// due, checked at each read and at the deadline.
    fn frames(reads: &[(u64, bool)]) -> Vec<u64> {
        let t0 = Instant::now();
        let mut c = Coalescer::default();
        let mut out = vec![];
        for &(t, settled) in reads {
            let now = t0 + ms(t);
            while let Some(at) = c.due().filter(|&at| at < now) {
                out.push((at - t0).as_millis() as u64);
                c.framed();
            }
            c.output(now, settled);
            if c.ready(now) {
                out.push(t);
                c.framed();
            }
        }
        if let Some(at) = c.due() {
            out.push((at - t0).as_millis() as u64);
        }
        out
    }

    #[test]
    fn a_write_split_in_two_reads_is_one_frame() {
        assert_eq!(frames(&[(100, false), (100, false)]), vec![104]);
        assert_eq!(frames(&[(100, false), (102, false)]), vec![106]);
    }

    #[test]
    fn writes_apart_are_frames_of_their_own() {
        assert_eq!(frames(&[(100, false), (350, false)]), vec![104, 354]);
    }

    #[test]
    fn a_stream_that_never_pauses_paints_every_cap() {
        let reads: Vec<_> = (0..100).map(|i| (1000 + i * 2, false)).collect();
        let got = frames(&reads);
        assert!(got.len() >= 12, "{got:?}");
        // Each frame at most CAP after the first read it did not have (the
        // read after the frame before: 2 ms later).
        let mut last = 998;
        for t in got {
            assert!(t - (last + 2) <= CAP.as_millis() as u64, "{t} after {last}");
            last = t;
        }
    }

    #[test]
    fn the_end_of_a_synchronized_update_frames_at_once() {
        assert_eq!(frames(&[(100, true)]), vec![100]);
        // Unsynchronized bytes after it are held again.
        assert_eq!(frames(&[(100, true), (101, false)]), vec![100, 105]);
    }

    #[test]
    fn nothing_held_is_never_due() {
        let mut c = Coalescer::default();
        assert_eq!(c.due(), None);
        c.output(Instant::now(), false);
        c.framed();
        assert!(!c.ready(Instant::now() + ms(100)));
    }
}
