//! B731: what drawing an image costs in memory, measured by this test
//! binary's own allocator (one test, so nothing else allocates meanwhile).

use base64::Engine as _;
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

/// An animated GIF: `frames` frames of `side`×`side`, red then blue.
fn animated_gif(side: u16, frames: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let palette = [255u8, 0, 0, 0, 0, 255];
        let mut gif = gif::Encoder::new(&mut bytes, side, side, &palette).unwrap();
        gif.set_repeat(gif::Repeat::Infinite).unwrap();
        for k in 0..frames {
            let pixels = vec![(k % 2) as u8; side as usize * side as usize];
            gif.write_frame(&gif::Frame::from_indexed_pixels(side, side, pixels, None))
                .unwrap();
        }
    }
    bytes
}

/// B731: an animated GIF (APNG, WebP) over iTerm2's protocol — what
/// `imgcat` sends — was decoded whole to draw its first frame: every frame
/// at once, frames × width × height × 4 bytes. The emulator checks one
/// frame's size against its limit, never how many frames there are: a
/// 100-frame 512×512 GIF took 100 MB to draw, past what iOS lets a
/// terminal app keep. Only the first frame is decoded now.
#[test]
fn b731_an_animation_costs_one_frame_to_draw() {
    let gif = animated_gif(512, 100);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&gif);
    let mut t = Terminal::new(
        80,
        24,
        Options {
            image_cache: Some(ImageCache::new(64 << 20)),
            ..Options::default()
        },
    );
    t.feed(format!("\x1b]1337;File=inline=1;size={}:{b64}\x07\r\n", gif.len()).as_bytes());
    let before = NOW.load(Relaxed);
    PEAK.store(before, Relaxed);
    let f = t.frame();
    let peak = PEAK.load(Relaxed) - before;
    assert_eq!(f.images.len(), 1, "the image is drawn");
    let img = &f.images[0];
    assert_eq!((img.width, img.height), (512, 512));
    assert_eq!(&img.rgba[..4], &[255, 0, 0, 255], "its first frame");
    assert!(peak < 16 << 20, "{} MB to draw one 1 MB frame", peak >> 20);
}
