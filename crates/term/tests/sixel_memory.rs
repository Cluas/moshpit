//! B735: what a sixel image costs in memory before it is drawn, measured by
//! this test binary's own allocator (one test, so nothing else allocates
//! meanwhile).

use moshpit_term::{ImageCache, Options, Terminal};
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

/// The most bytes live at once while `bytes` were fed and framed.
fn peak(t: &mut Terminal, bytes: &[u8]) -> usize {
    let before = NOW.load(Relaxed);
    PEAK.store(before, Relaxed);
    t.feed(bytes);
    let _ = t.frame();
    PEAK.load(Relaxed) - before
}

/// B735: the sixel parser took a sixel's raster attributes at their word
/// and reserved room for width × height commands up front: a header
/// naming 10000×10000 — a few dozen bytes, from a broken program or `cat`
/// of the wrong file — allocated 762 MB before a single command came, what
/// gets a terminal app killed on a phone. And it kept every command of an
/// image without a limit: 20 MB of sixel data took 384 MB. The reserve is
/// bounded, and an image past 4 M commands is dropped whole; what follows
/// it shows, and an ordinary image still draws.
#[test]
fn b735_a_sixel_costs_what_it_draws() {
    let mut t = Terminal::new(
        80,
        24,
        Options {
            image_cache: Some(ImageCache::new(64 << 20)),
            ..Options::default()
        },
    );
    let header = peak(&mut t, b"\x1bPq\"1;1;10000;10000#0~\x1b\\after\r\n");
    assert!(
        header < 8 << 20,
        "a 10000x10000 header took {} MB",
        header >> 20
    );
    assert!(t.visible_text().iter().any(|r| r.0 == "after"));
    let mut endless = b"\x1bPq#0".to_vec();
    endless.extend(std::iter::repeat_n(b'~', 20 << 20));
    endless.extend(b"\x1b\\still here\r\n");
    let data = peak(&mut t, &endless);
    assert!(
        data < 96 << 20,
        "20 MB of sixel data took {} MB",
        data >> 20
    );
    assert!(t.visible_text().iter().any(|r| r.0 == "still here"));
    // An ordinary image still draws: 60×12, red.
    let mut ok = String::from("\x1bPq\"1;1;60;12#0;2;100;0;0#0");
    ok.push_str("!60~-!60~");
    ok.push_str("\x1b\\\r\n");
    t.feed(ok.as_bytes());
    let f = t.frame();
    assert_eq!(f.images.len(), 1, "the ordinary image is drawn");
    assert_eq!((f.images[0].width, f.images[0].height), (60, 12));
}
