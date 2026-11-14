//! Images through the core's API — decoded pixels and the cells each one
//! covers — from what real tools print (tests/fixtures/images, written by
//! spike/term-core/images.sh, offline): kitty's own `kitten icat` (PNG as
//! PNG, JPEG decoded by icat into raw pixels), term-image (raw RGB, zlib),
//! `wezterm imgcat` (iTerm2's protocol: PNG, JPEG, GIF) and ImageMagick's
//! sixel encoder. The terminal is 80×24 with 8×16-pixel cells, as the tools
//! were told.

mod common;
use base64::Engine as _;
use moshpit_term::{DecodedImage, Frame, ImageCache, ImageRun, Options, Terminal};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        common::repo()
            .join("crates/term/tests/fixtures/images")
            .join(name),
    )
    .unwrap()
}

/// A terminal with a decoded-image cache of its own: the process's is
/// shared by every test running at once.
fn term() -> Terminal {
    Terminal::new(
        80,
        24,
        Options {
            image_cache: Some(ImageCache::new(64 << 20)),
            ..Options::default()
        },
    )
}

/// A kitty transmission of a `side` x `side` RGBA square of one shade,
/// placed at the cursor (`id`: the program's image id, if it names one).
fn kitty_square(side: u32, shade: u8, id: Option<u32>) -> String {
    let rgba: Vec<u8> = (0..side * side)
        .flat_map(|_| [shade, 0, 255 - shade, 255])
        .collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&rgba);
    let id = id.map(|i| format!("i={i},")).unwrap_or_default();
    format!("\x1b_Ga=T,f=32,s={side},v={side},{id}q=1;{b64}\x1b\\\r\n")
}

/// The frame's images and every row's image runs (row, run).
fn images(t: &mut Terminal) -> (Vec<DecodedImage>, Vec<(u16, ImageRun)>) {
    let f: Frame = t.frame();
    let runs = f
        .changed
        .iter()
        .flat_map(|r| r.images.iter().map(move |run| (r.row, *run)))
        .collect();
    (f.images, runs)
}

fn pixel(img: &DecodedImage, x: u32, y: u32) -> (u8, u8, u8) {
    let i = ((y * img.width + x) * 4) as usize;
    (img.rgba[i], img.rgba[i + 1], img.rgba[i + 2])
}

fn reddish((r, g, b): (u8, u8, u8)) -> bool {
    r > 180 && g < 80 && b < 80
}

fn bluish((r, g, b): (u8, u8, u8)) -> bool {
    b > 180 && g < 80 && r < 80
}

/// The 24×12 red-to-blue gradient: 3 cells wide, 1 row high.
fn assert_gradient(img: &DecodedImage, runs: &[(u16, ImageRun)], col: u16) {
    assert_eq!((img.width, img.height), (24, 12));
    assert!(reddish(pixel(img, 0, 0)), "{:?}", pixel(img, 0, 0));
    assert!(bluish(pixel(img, 0, 11)), "{:?}", pixel(img, 0, 11));
    assert_eq!(runs.len(), 1, "{runs:?}");
    let (row, run) = runs[0];
    assert_eq!((row, run.col, run.cols, run.image), (0, col, 3, 1));
    assert_eq!((run.left, run.right), (0.0, 1.0));
    assert!(run.top == 0.0 && run.bottom > 0.7, "{run:?}");
}

#[test]
fn kitten_icat_png_and_jpeg() {
    // icat centres the image: CR, then 38 columns right.
    let mut t = term();
    t.feed(&fixture("kitty-icat-png.bin"));
    let (imgs, runs) = images(&mut t);
    assert_eq!(imgs.len(), 1);
    assert_gradient(&imgs[0], &runs, 38);

    // JPEG: icat decodes it and sends raw RGB pixels.
    let mut t = term();
    t.feed(&fixture("kitty-icat-jpg.bin"));
    let (imgs, runs) = images(&mut t);
    assert_gradient(&imgs[0], &runs, 38);
}

#[test]
fn kitten_icat_a_larger_image_spans_rows() {
    let mut t = term();
    t.feed(&fixture("kitty-icat-chunked.bin"));
    let (imgs, runs) = images(&mut t);
    assert_eq!((imgs[0].width, imgs[0].height), (96, 96));
    // 96 px: 12 cells across, 6 rows down; the texture continues row to row.
    assert_eq!(runs.len(), 6, "{runs:?}");
    for (k, (row, run)) in runs.iter().enumerate() {
        assert_eq!((*row as usize, run.cols), (k, 12));
        let top = k as f32 / 6.0;
        assert!(
            (run.top - top).abs() < 1e-3 && (run.bottom - (top + 1.0 / 6.0)).abs() < 1e-3,
            "{run:?}"
        );
    }
}

#[test]
fn a_chunked_kitty_transmission() {
    // The protocol's chunking (m=1 … m=0, 4096 bytes of base64 each, as
    // kitty's spec shows it) of the same PNG icat sent in one piece.
    let png = fixture("noise.png");
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let chunks: Vec<&str> = b64
        .as_bytes()
        .chunks(4096)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    let mut stream = Vec::new();
    for (k, c) in chunks.iter().enumerate() {
        let more = if k + 1 < chunks.len() { 1 } else { 0 };
        let head = if k == 0 { "a=T,f=100,q=2," } else { "" };
        stream.extend_from_slice(format!("\x1b_G{head}m={more};{c}\x1b\\").as_bytes());
    }
    assert!(chunks.len() > 3);
    let mut t = term();
    t.feed(&stream);
    let (imgs, runs) = images(&mut t);
    assert_eq!((imgs[0].width, imgs[0].height, runs.len()), (96, 96, 6));
}

#[test]
fn term_image_raw_rgb_zlib() {
    let mut t = term();
    t.feed(&fixture("kitty-termimage-rgb-zlib.bin"));
    let (imgs, runs) = images(&mut t);
    // Scaled by term-image to 6×2 pixels, placed over 6 cells by 1 row.
    assert_eq!((imgs[0].width, imgs[0].height), (6, 2));
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].1.col, runs[0].1.cols), (0, 6));
}

#[test]
fn wezterm_imgcat_png_jpeg_gif() {
    for (name, w, h) in [
        ("iterm-imgcat-grad-png.bin", 24, 12),
        ("iterm-imgcat-grad-jpg.bin", 24, 12),
    ] {
        let mut t = term();
        t.feed(&fixture(name));
        let (imgs, runs) = images(&mut t);
        assert_eq!(
            (imgs.len(), imgs[0].width, imgs[0].height),
            (1, w, h),
            "{name}"
        );
        assert_gradient(&imgs[0], &runs, 0);
    }
    let mut t = term();
    t.feed(&fixture("iterm-imgcat-box-gif.bin"));
    let (imgs, runs) = images(&mut t);
    let img = &imgs[0];
    assert_eq!((img.width, img.height), (16, 8));
    let (r, g, b) = pixel(img, 0, 0);
    assert!(g > 100 && r < 50 && b < 50, "green corner {:?}", (r, g, b));
    let (r, g, b) = pixel(img, 8, 4);
    assert!(
        r > 200 && g > 200 && b < 80,
        "yellow middle {:?}",
        (r, g, b)
    );
    assert_eq!((runs[0].1.col, runs[0].1.cols), (0, 2));
}

#[test]
fn imagemagick_sixel() {
    let mut t = term();
    t.feed(&fixture("sixel-magick.bin"));
    let (imgs, runs) = images(&mut t);
    assert_eq!(imgs.len(), 1);
    assert_gradient(&imgs[0], &runs, 0);
}

#[test]
fn images_do_not_depend_on_cuts() {
    for name in [
        "kitty-icat-png.bin",
        "kitty-icat-jpg.bin",
        "kitty-termimage-rgb-zlib.bin",
        "iterm-imgcat-grad-png.bin",
        "iterm-imgcat-box-gif.bin",
        "sixel-magick.bin",
    ] {
        let bytes = fixture(name);
        let mut whole = term();
        whole.feed(&bytes);
        let mut cut = term();
        for b in &bytes {
            cut.feed(std::slice::from_ref(b));
        }
        let (a, ra) = images(&mut whole);
        let (b, rb) = images(&mut cut);
        assert_eq!(
            a.iter()
                .map(|i| (i.id, i.width, i.height))
                .collect::<Vec<_>>(),
            b.iter()
                .map(|i| (i.id, i.width, i.height))
                .collect::<Vec<_>>(),
            "{name}"
        );
        assert_eq!(ra, rb, "{name}");
    }
}

#[test]
fn the_same_image_twice_is_decoded_once() {
    let mut t = term();
    t.feed(&fixture("iterm-imgcat-grad-png.bin"));
    t.feed(&fixture("iterm-imgcat-grad-png.bin"));
    let (imgs, runs) = images(&mut t);
    assert_eq!(imgs.len(), 1, "one decoded image for both placements");
    assert_eq!(runs.len(), 2);
    assert_eq!(t.image_cache_bytes(), 24 * 12 * 4);
}

#[test]
fn limits_hold() {
    // Larger than the 32 MB a decoded image may take: ignored.
    let mut t = term();
    t.feed(b"\x1b_Ga=T,f=32,s=5000,v=5000,q=2;AAAA\x1b\\");
    assert!(images(&mut t).0.is_empty());

    // A chunked transmission that would exceed the limit is dropped, and
    // the terminal keeps working.
    let mut t = Terminal::new(
        80,
        24,
        Options {
            max_image_bytes: 64 << 10,
            ..Options::default()
        },
    );
    let chunk = "A".repeat(4096);
    t.feed(b"\x1b_Ga=T,f=32,s=256,v=256,q=2,m=1;");
    t.feed(chunk.as_bytes());
    t.feed(b"\x1b\\");
    for _ in 0..64 {
        t.feed(format!("\x1b_Gm=1;{chunk}\x1b\\").as_bytes());
    }
    t.feed(b"\x1b_Gm=0;\x1b\\after");
    assert!(images(&mut t).0.is_empty());
    assert!(common::rows(&t)[0].ends_with("after"));

    // A transmission naming a file of this machine (t=f) is refused: the
    // program sending it runs on another one.
    let dir = std::env::temp_dir().join(format!("moshpit-term-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("local.png");
    std::fs::write(&file, fixture("grad.png")).unwrap();
    let path_b64 =
        base64::engine::general_purpose::STANDARD.encode(file.to_string_lossy().as_bytes());
    let mut t = term();
    t.feed(format!("\x1b_Ga=T,f=100,t=f,q=2;{path_b64}\x1b\\").as_bytes());
    assert!(images(&mut t).0.is_empty());
    let mut t = term();
    t.feed(format!("\x1b_Ga=T,f=100,t=t,q=2;{path_b64}\x1b\\").as_bytes());
    assert!(images(&mut t).0.is_empty());
    assert!(
        file.exists(),
        "t=t deletes its file; it must not have been read"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The decoded images kept for drawing stay within their budget however
/// many different images go by, the screen and the history cleared after
/// each as a user would (`clear` and `ESC [3J`); the same image shown
/// again is decoded once. What the iPad's footprint showed (TERMINAL-VIEW.md
/// §5.8): a level reached once, then flat, not a leak.
#[test]
fn the_decoded_images_stay_within_their_budget() {
    let side = 32u32;
    let one = (side * side * 4) as usize;
    let budget = 3 * one;
    let mut t = Terminal::new(
        80,
        24,
        Options {
            image_cache: Some(ImageCache::new(budget)),
            ..Options::default()
        },
    );
    let show = |t: &mut Terminal, shade: u8| {
        t.feed(kitty_square(side, shade, None).as_bytes());
        let (imgs, _) = images(t);
        assert_eq!(imgs.len(), 1, "image {shade} drawn");
        t.feed(b"\x1b[H\x1b[2J\x1b[3J");
        let _ = images(t);
    };
    for shade in 0..20u8 {
        show(&mut t, shade * 10);
        assert!(
            t.image_cache_bytes() <= budget,
            "{} bytes kept after {} images, budget {budget}",
            t.image_cache_bytes(),
            shade + 1
        );
    }
    assert_eq!(
        t.image_cache_bytes(),
        budget,
        "the budget's worth kept, the oldest let go"
    );
    // The newest again: still decoded, nothing added.
    show(&mut t, 190);
    assert_eq!(t.image_cache_bytes(), budget);
}

/// Every terminal draws from one budget: three of them showing different
/// images keep no more than it, the least recently drawn let go whichever
/// terminal drew it; an image two terminals show is kept once.
#[test]
fn terminals_share_one_budget() {
    let side = 32u32;
    let one = (side * side * 4) as usize;
    let budget = 4 * one;
    let cache = ImageCache::new(budget);
    let mut terms: Vec<Terminal> = (0..3)
        .map(|_| {
            Terminal::new(
                80,
                24,
                Options {
                    image_cache: Some(cache.clone()),
                    ..Options::default()
                },
            )
        })
        .collect();
    for round in 0..6u8 {
        for (k, t) in terms.iter_mut().enumerate() {
            t.feed(kitty_square(side, round * 30 + k as u8 * 7, None).as_bytes());
            assert_eq!(images(t).0.len(), 1);
            assert!(
                cache.bytes() <= budget,
                "{} kept, budget {budget}",
                cache.bytes()
            );
            assert_eq!(t.image_cache_bytes(), cache.bytes(), "one cache");
        }
    }
    assert_eq!(cache.bytes(), budget);
    // The same image in two terminals: decoded once.
    cache.purge();
    terms[0].feed(kitty_square(side, 3, None).as_bytes());
    terms[1].feed(kitty_square(side, 3, None).as_bytes());
    let (a, _) = images(&mut terms[0]);
    let (b, _) = images(&mut terms[1]);
    assert_eq!(a[0].id, b[0].id);
    assert_eq!(cache.bytes(), one);
}

/// A purge (a memory warning, the background) lets every decoded image go;
/// an image still on the screen draws again: the next frame that shows it
/// decodes it again, the same pixels.
#[test]
fn a_purge_lets_the_decoded_images_go_and_they_draw_again() {
    let cache = ImageCache::new(8 << 20);
    let mut t = Terminal::new(
        80,
        24,
        Options {
            image_cache: Some(cache.clone()),
            ..Options::default()
        },
    );
    t.feed(&fixture("iterm-imgcat-grad-png.bin"));
    let (imgs, runs) = images(&mut t);
    assert_eq!(imgs.len(), 1);
    assert!(!runs.is_empty());
    let first = imgs[0].clone();
    assert_eq!(cache.bytes(), first.rgba.len());
    cache.purge();
    assert_eq!(cache.bytes(), 0);
    // A frame that draws the screen again (the look changed).
    t.set_bold_is_bright(false);
    let (again, runs) = images(&mut t);
    assert_eq!(again.len(), 1, "the image on screen is still drawn");
    assert!(!runs.is_empty());
    assert_eq!(again[0].id, first.id);
    assert_eq!(*again[0].rgba, *first.rgba);
    assert_eq!(cache.bytes(), first.rgba.len(), "decoded again");
}

/// A memory warning lets go of the kitty images no cell shows any more
/// (erased, out of the history) and keeps the ones on screen; placing a
/// dropped id again is kitty's ENOENT, placing a kept one works.
#[test]
fn a_memory_warning_lets_go_of_the_kitty_images_no_cell_shows() {
    let side = 16u32;
    let one = (side * side * 4) as usize;
    let mut t = term();
    t.feed(kitty_square(side, 200, Some(2)).as_bytes());
    // Image 2 erased, the history with it; then image 1, on screen.
    t.feed(b"\x1b[H\x1b[2J\x1b[3J");
    t.feed(kitty_square(side, 10, Some(1)).as_bytes());
    let _ = t.take_replies();
    assert_eq!(t.kitty_image_bytes(), 2 * one);
    assert_eq!(t.drop_unplaced_images(), one, "image 2 let go");
    assert_eq!(t.kitty_image_bytes(), one);
    assert_eq!(t.drop_unplaced_images(), 0, "image 1 is on screen");
    t.feed(b"\x1b_Ga=p,i=2\x1b\\");
    let replies = String::from_utf8_lossy(&t.take_replies()).into_owned();
    assert!(replies.contains("ENOENT"), "{replies:?}");
    t.feed(b"\x1b_Ga=p,i=1,q=1\x1b\\");
    let replies = String::from_utf8_lossy(&t.take_replies()).into_owned();
    assert!(!replies.contains("ENOENT"), "{replies:?}");
    // Still drawn.
    assert!(!images(&mut t).0.is_empty());
}

/// The images an emulator keeps so identical ones share data (its last 16,
/// whatever their size) are let go in the background: a sixel erased from
/// the screen and the history is freed then, one on screen stays drawn.
#[test]
fn the_background_lets_go_of_the_images_kept_to_share() {
    let mut t = term();
    t.feed(&fixture("sixel-magick.bin"));
    t.feed(b"\x1b[H\x1b[2J\x1b[3J");
    assert!(images(&mut t).0.is_empty());
    let freed = t.release_image_caches();
    assert!(freed > 0, "the erased sixel was still kept");
    assert_eq!(t.release_image_caches(), 0);
    // One on screen: nothing to free, still drawn.
    t.feed(&fixture("sixel-magick.bin"));
    assert_eq!(t.release_image_caches(), 0);
    t.set_bold_is_bright(false);
    assert_eq!(images(&mut t).0.len(), 1);
}

/// An image wider than the screen is clipped at the right margin: its cells
/// past it were never drawn and no erase reached them, so after `clear` (as
/// a tmux pane sends it, `ESC [H ESC [J`) and `ESC [3J` its data stayed
/// alive as long as their lines stayed on the screen (B295).
#[test]
fn an_image_wider_than_the_screen_is_let_go_when_erased() {
    // 40 columns of 8 px: 320 px wide; the image 480 px.
    let mut t = Terminal::new(
        40,
        24,
        Options {
            image_cache: Some(ImageCache::new(64 << 20)),
            ..Options::default()
        },
    );
    t.feed(kitty_square(480, 90, Some(7)).as_bytes());
    let (imgs, runs) = images(&mut t);
    assert_eq!(imgs.len(), 1, "drawn");
    assert!(
        runs.iter().all(|(_, r)| r.col + r.cols <= 40),
        "no run past the margin: {runs:?}"
    );
    t.feed(b"\x1b[H\x1b[J\x1b[3J");
    assert!(images(&mut t).0.is_empty());
    assert_eq!(
        t.drop_unplaced_images(),
        480 * 480 * 4,
        "nothing holds it once erased"
    );
}

/// kitty images sent without an id share id 0: each replaces the one
/// before in the store, and the store's byte count follows (it added each
/// and never took the replaced one off: three anonymous images counted
/// three on the iPad while the store held one, so a memory warning seemed
/// to leave two behind).
#[test]
fn anonymous_kitty_images_are_counted_once() {
    let side = 16u32;
    let one = (side * side * 4) as usize;
    let mut t = term();
    for shade in [10, 20, 30] {
        t.feed(kitty_square(side, shade, None).as_bytes());
    }
    assert_eq!(t.kitty_image_bytes(), one, "the store holds the last one");
    t.feed(b"\x1b[H\x1b[2J\x1b[3J");
    // All three go: the stored one, and the two it replaced that the
    // emulator's sharing cache still held.
    assert_eq!(t.drop_unplaced_images(), 3 * one);
    assert_eq!(t.kitty_image_bytes(), 0);
}
