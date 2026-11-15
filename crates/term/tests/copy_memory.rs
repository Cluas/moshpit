//! B732: what copying a big selection costs in memory, measured by this
//! test binary's own allocator (one test, so nothing else allocates
//! meanwhile).

mod common;
use common::*;
use moshpit_term::SelectKind;
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

/// B732: a copy of a selection over several rows asks whether it lies
/// inside one link a program broke over rows (B423), and gathered every
/// cell of the selection to find out: 10 000 rows of history selected
/// (Select All, B699) cost 50 MB to copy 600 KB of text, on top of what
/// the phone already holds. Only the selection's ends are looked at now.
#[test]
fn b732_a_big_copy_costs_about_its_text() {
    let mut t = term(80, 24, 10_000);
    let text: String = (0..10_000)
        .map(|i| format!("line {i} some output text here, 你好世界 and more words\r\n"))
        .collect();
    t.feed(text.as_bytes());
    t.scroll(i32::MAX);
    t.select_start(SelectKind::Line, 0, 0, false);
    t.scroll_to_bottom();
    t.select_extend(23, 79, true);
    let before = NOW.load(Relaxed);
    PEAK.store(before, Relaxed);
    let copied = t.selection_text().expect("selected");
    let peak = PEAK.load(Relaxed) - before;
    assert!(copied.starts_with("line 0 ") && copied.contains("line 9999 "));
    assert!(
        peak < 4 * copied.len() + (1 << 20),
        "{} KB to copy {} KB",
        peak >> 10,
        copied.len() >> 10
    );
    // A link a program broke over rows still copies as one (B423).
    let mut t = term(30, 4, 10);
    t.feed(b"\x1b]8;;https://example.com/aaaaaaaaaa\x1b\\https://example.com/\r\n  aaaaaaaaaa\x1b]8;;\x1b\\\r\n");
    t.select_start(SelectKind::Semantic, 0, 3, false);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("https://example.com/aaaaaaaaaa")
    );
}
