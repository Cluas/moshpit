//! The mosh framebuffer, emulator and renderer (`screen`): a rendered
//! repaint from one screen to another, played on the first, gives the
//! second, for every kind of change mosh-server's frames make.

use moshpit_mosh::screen::{Framebuffer, new_frame};

/// Deterministic pseudo-random numbers (SplitMix64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// One change, written the way an application writes to its terminal.
fn change(rng: &mut Rng, width: usize, height: usize) -> String {
    let (w, h) = (width as u64, height as u64);
    match rng.below(18) {
        0..=4 => {
            let len = 1 + rng.below(2 * w);
            // A combining mark always follows a base character: one on
            // its own (mosh's fallback cell) comes back as NBSP + mark.
            (0..len)
                .map(|_| match rng.below(20) {
                    0 => "界".to_owned(),
                    1 => "é".to_owned(),
                    2 => "e\u{301}".to_owned(),
                    3 => " ".to_owned(),
                    _ => ((b'a' + rng.below(26) as u8) as char).to_string(),
                })
                .collect()
        }
        5 => "\r\n".into(),
        6 => format!("\x1b[{};{}H", 1 + rng.below(h), 1 + rng.below(w)),
        7 => format!("\x1b[{}K", rng.below(3)),
        8 => format!("\x1b[{}J", rng.below(3)),
        9 => match rng.below(6) {
            0 => "\x1b[0m".into(),
            1 => format!("\x1b[1;{}m", 30 + rng.below(8)),
            2 => format!("\x1b[38;5;{}m", rng.below(256)),
            3 => format!(
                "\x1b[48;2;{};{};{}m",
                rng.below(256),
                rng.below(256),
                rng.below(256)
            ),
            4 => "\x1b[7;4m".into(),
            _ => format!("\x1b[{}m", 100 + rng.below(8)),
        },
        // Scroll the whole screen.
        10 => format!("\x1b[{h};1H{}", "\n".repeat(1 + rng.below(3) as usize)),
        // Scroll a region (tmux's status line stays put).
        11 => {
            let bottom = h.saturating_sub(1).max(2);
            format!(
                "\x1b[1;{bottom}r\x1b[{bottom};1H\n{}\x1b[r",
                (b'A' + rng.below(26) as u8) as char
            )
        }
        12 => format!("\x1b[{}X", 1 + rng.below(10)),
        13 => if rng.below(2) == 0 {
            "\x1b[?25l"
        } else {
            "\x1b[?25h"
        }
        .into(),
        14 => format!("\x1b]0;title {}\x07", rng.below(5)),
        15 => if rng.below(2) == 0 {
            "\x1b[?2004h\x1b[?1002h\x1b[?1006h"
        } else {
            "\x1b[?2004l\x1b[?1002l\x1b[?1006l"
        }
        .into(),
        16 => "\x07".into(),
        _ => format!(
            "\x1b[{};1H\x1b[K{}",
            1 + rng.below(h),
            "x".repeat(w as usize)
        ),
    }
}

fn describe(screen: &Framebuffer) -> String {
    (0..screen.height())
        .map(|row| screen.row_text(row))
        .collect::<Vec<_>>()
        .join("|\n")
        + &format!("|\ncursor {:?}", screen.cursor())
}

fn assert_repaints(from: &Framebuffer, to: &Framebuffer, initialized: bool, context: &str) {
    let bytes = new_frame(initialized, from, to);
    let mut played = from.clone();
    // A new size travels beside the bytes (the diff's resize), first.
    played.resize(to.width(), to.height());
    played.apply(&bytes);
    let first_difference = (0..to.height().min(played.height())).find_map(|row| {
        let (want, got) = (to.row(row).cells(), played.row(row).cells());
        (0..want.len().min(got.len()))
            .find(|&col| want[col] != got[col])
            .map(|col| {
                format!(
                    "row {row} col {col}: wanted {:?} got {:?}",
                    want[col], got[col]
                )
            })
    });
    assert!(
        played.looks_like(to),
        "{context}: {first_difference:?} {:?}\nwanted\n{}\ngot\n{}",
        String::from_utf8_lossy(&bytes),
        describe(to),
        describe(&played)
    );
}

/// From each screen to the next, from a few screens back (what a diff
/// from an older held state needs), and from scratch.
#[test]
fn a_rendered_repaint_turns_one_screen_into_another() {
    for seed in 0..40 {
        let mut rng = Rng(seed);
        let (width, height) = (20 + rng.below(30) as usize, 5 + rng.below(10) as usize);
        let mut history = vec![Framebuffer::new(width, height)];
        let mut screen = history[0].clone();
        screen.resize(width, height);
        for step in 0..120 {
            let changes = 1 + rng.below(4);
            for _ in 0..changes {
                let text = change(&mut rng, width, height);
                screen.apply(text.as_bytes());
            }
            let context = format!("seed {seed} step {step}");
            let last = history.last().unwrap();
            assert_repaints(last, &screen, true, &context);
            let back = history.len().saturating_sub(1 + rng.below(6) as usize);
            assert_repaints(
                &history[back],
                &screen,
                true,
                &format!("{context} from {back}"),
            );
            assert_repaints(&history[0], &screen, false, &format!("{context} whole"));
            history.push(screen.clone());
        }
    }
}

/// A size change repaints everything at the new size.
#[test]
fn a_resize_repaints_at_the_new_size() {
    let mut small = Framebuffer::new(80, 24);
    small.apply(b"hello\r\nworld");
    let mut big = small.clone();
    big.resize(100, 30);
    big.apply("\x1b[30;1Hbottom 界".as_bytes());
    assert_repaints(&small, &big, true, "grow");
    let mut narrow = big.clone();
    narrow.resize(40, 10);
    assert_repaints(&big, &narrow, true, "shrink");
}

/// Rows shared between screens are counted once.
#[test]
fn held_screens_share_unchanged_rows() {
    let mut first = Framebuffer::new(80, 24);
    for row in 0..24 {
        first.apply(format!("\x1b[{};1Hline {row}", row + 1).as_bytes());
    }
    let one = moshpit_mosh::screen::held_bytes([&first]);
    let mut second = first.clone();
    second.apply(b"\x1b[24;1Hchanged");
    let both = moshpit_mosh::screen::held_bytes([&first, &second]);
    // One changed row and the second screen's row table, not a whole copy.
    assert!(both - one < one / 10, "one {one}, both {both}");
}

/// The cells mosh-server writes land where its own client puts them.
#[test]
fn host_bytes_land_like_mosh() {
    let mut screen = Framebuffer::new(10, 3);
    screen.apply("\x1b[?25l\x1b[1;1H界e\u{301}x\x1b[2;9Habc\x1b[3;1H\x1b[31mr\x1b[0m".as_bytes());
    assert_eq!(screen.row_text(0), "界e\u{301}x      ");
    // "c" wrapped onto the next row.
    assert_eq!(screen.row_text(1), "        ab");
    assert_eq!(screen.row_text(2), "r         ");
    assert_eq!(screen.cursor(), (1, 2, false));
}
