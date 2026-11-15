//! B733: what the images scrolled into the history keep in memory,
//! measured by this test binary's own allocator (one test, so nothing else
//! allocates meanwhile).

use base64::Engine as _;
use moshpit_term::{Frame, ImageCache, Options, Terminal};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// Counts the bytes live.
struct Counting;
static NOW: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            NOW.fetch_add(layout.size(), Relaxed);
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

fn term() -> Terminal {
    term_with(Options::default().history_image_bytes)
}

/// A terminal whose history's images may keep `budget` bytes.
fn term_with(budget: usize) -> Terminal {
    Terminal::new(
        80,
        24,
        Options {
            image_cache: Some(ImageCache::new(64 << 20)),
            history_image_bytes: budget,
            ..Options::default()
        },
    )
}

/// A `w`×`h` sixel image of one colour, then a new line.
fn sixel(w: u32, h: u32, red: u8) -> String {
    let mut s = format!("\x1bPq\"1;1;{w};{h}#0;2;{red};0;0#0");
    for _ in 0..h.div_ceil(6) {
        s.push_str(&format!("!{w}~-"));
    }
    s + "\x1b\\\r\n"
}

/// A kitty image `side`×`side`, transmitted and placed, then a new line.
fn kitty(side: u32, shade: u8, id: u32) -> String {
    let rgba: Vec<u8> = (0..side * side).flat_map(|_| [shade, 0, 0, 255]).collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&rgba);
    format!("\x1b_Ga=T,f=32,s={side},v={side},i={id},q=2;{b64}\x1b\\\r\n")
}

/// Whether any row of the frame draws an image.
fn draws_images(f: &Frame) -> bool {
    f.changed.iter().any(|r| !r.images.is_empty())
}

/// B733: the images of the rows scrolled into the history were kept as long
/// as the rows: 100 sixel plots of 800×600 (`img2sixel`, matplotlib's sixel
/// backend, `lsix`) kept 242 MB, and kitty images placed and scrolled
/// away were never the kitty storage's to let go (it lets only unplaced
/// ones go). The history's images are held to a budget (32 MB a terminal):
/// the oldest rows' images go first, their text stays, the newest ones
/// still draw.
#[test]
fn b733_the_historys_images_are_held_to_a_budget() {
    let mut t = term();
    let before = NOW.load(Relaxed);
    for k in 0..100u32 {
        t.feed(sixel(800, 600, k as u8).as_bytes());
        t.feed(format!("plot {k}\r\n").as_bytes());
    }
    let _ = t.frame();
    let kept = NOW.load(Relaxed) - before;
    assert!(kept < 48 << 20, "100 sixel images kept {} MB", kept >> 20);
    // The newest still draw from the history; the oldest rows keep their
    // text, without their images.
    t.scroll(30);
    assert!(draws_images(&t.frame()), "the newest images went too");
    t.scroll(i32::MAX);
    assert!(
        !draws_images(&t.frame()),
        "the oldest images are still kept"
    );
    t.scroll(-30);
    assert!(
        t.visible_text().iter().any(|r| r.0 == "plot 0"),
        "the text went with them"
    );
    drop(t);
    // Kitty images placed, then scrolled away.
    let mut t = term();
    let before = NOW.load(Relaxed);
    for k in 0..100u32 {
        t.feed(kitty(400, k as u8, k + 1).as_bytes());
        t.feed(format!("icat {k}\r\n").as_bytes());
    }
    let _ = t.frame();
    let kept = NOW.load(Relaxed) - before;
    assert!(kept < 56 << 20, "100 kitty images kept {} MB", kept >> 20);

    // Many terminals (a phone keeps one for every tmux pane it has shown,
    // every herdr pane, every session), each over its own budget: the
    // process holds them to a total, the terminals whose images came
    // longest ago letting theirs go first; every terminal's text stays.
    drop(t);
    moshpit_term::history_images::set_total(8 << 20);
    let mut panes: Vec<Terminal> = (0..5).map(|_| term_with(4 << 20)).collect();
    let before = NOW.load(Relaxed);
    for (n, t) in panes.iter_mut().enumerate() {
        for k in 0..40u32 {
            t.feed(sixel(200, 150, k as u8).as_bytes());
            t.feed(format!("pane {n} plot {k}\r\n").as_bytes());
        }
    }
    // Asked to let go, a terminal does at its next bytes or frame.
    for t in &mut panes {
        let _ = t.frame();
    }
    let kept = NOW.load(Relaxed) - before;
    // The total counts pixels; the rows that keep an image hold its cells'
    // attachments too (about as much again for small images), and each
    // screen shows two or three. Without the total, about 34 MB.
    assert!(
        kept < 18 << 20,
        "5 terminals' images kept {} MB, past twice the 8 MB total",
        kept >> 20
    );
    for (n, t) in panes.iter_mut().enumerate() {
        t.scroll(i32::MAX);
        let first = format!("pane {n} plot 0");
        assert!(
            t.visible_text().iter().any(|r| r.0 == first),
            "pane {n} lost its text"
        );
    }
    // The newest terminal's images are the last to go.
    let last = panes.last_mut().unwrap();
    last.scroll_to_bottom();
    last.scroll(12);
    assert!(draws_images(&last.frame()), "the newest images went first");
    moshpit_term::history_images::set_total(moshpit_term::history_images::DEFAULT_TOTAL);
}
