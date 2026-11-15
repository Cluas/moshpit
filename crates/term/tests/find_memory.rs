//! B734: what a find step costs in memory, measured by this test binary's
//! own allocator (one test, so nothing else allocates meanwhile).

mod common;
use common::*;
use moshpit_term::{FindOptions, FindStep};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// Counts the bytes live and the most there were.
struct Counting;
static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = NOW.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(now, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        NOW.fetch_sub(layout.size(), Relaxed);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The most bytes live at once while `f` ran, beyond what was live before.
fn peak(f: impl FnOnce()) -> usize {
    let before = NOW.load(Relaxed);
    PEAK.store(before, Relaxed);
    f();
    PEAK.load(Relaxed) - before
}

/// B734: every step of a find (each letter typed in the bar, each Next)
/// looked through the whole history again and noted where every character
/// of it sat: 26 ms and some 20 MB a step over 10 000 rows, Next as much as
/// the first search, though nothing had changed. The text is looked
/// through first and only the lines that match are placed; the matches are
/// kept until the terminal changes.
#[test]
fn b734_a_find_step_costs_what_changed() {
    let mut t = term(80, 24, 10_000);
    let text: String = (0..10_000)
        .map(|i| {
            let word = if i % 1000 == 7 { "needle" } else { "hay" };
            format!("line {i} some output text here, 你好世界 {word}\r\n")
        })
        .collect();
    t.feed(text.as_bytes());
    let mut count = 0;
    let first = peak(|| {
        count = t
            .find("needle", FindOptions::default(), FindStep::Newest)
            .map_or(0, |f| f.count);
    });
    assert_eq!(count, 10);
    assert!(first < 4 << 20, "the first search took {} KB", first >> 10);
    let mut index = 0;
    let next = peak(|| {
        index = t
            .find("needle", FindOptions::default(), FindStep::Previous)
            .map_or(0, |f| f.index);
    });
    assert_eq!(index, 8);
    assert!(
        next < 64 << 10,
        "a step with nothing new took {} KB",
        next >> 10
    );
    // New output: found again, the new match counted.
    t.feed(b"one more needle\r\n");
    assert_eq!(
        t.find("needle", FindOptions::default(), FindStep::Next)
            .map(|f| f.count),
        Some(11)
    );
}
