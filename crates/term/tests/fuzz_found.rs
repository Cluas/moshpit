//! What the fuzzer (tests/fuzz.rs) found, each as its smallest bytes: the
//! terminal must take them without a panic, in bounded time and memory,
//! and still mean what they meant.

mod common;
use common::*;
use std::time::{Duration, Instant};

fn takes(bytes: &[u8]) -> Duration {
    let mut t = term(80, 24, 1000);
    let at = Instant::now();
    t.feed(bytes);
    let _ = t.frame();
    at.elapsed()
}

/// B791 (W28): counts that ran a loop once per unit, whatever the screen.
/// ICH (`CSI n @`) inserted a cell n times: 999999999, 80 s; REP (`CSI n
/// b`) wrote n cells, wrapping and scrolling whatever DECAWM says:
/// 4294967295, 82 s; CHT and CBT (`CSI n I`, `CSI n Z`) moved one tab stop
/// n times: 2 s; a sixel repeat (`!n`) drew n columns past the image's
/// edge, then overflowed; CUB (`CSI n D`) backspaced n times: 4294967295,
/// 73 s; a kitty keyboard pop (`CSI < n u`) popped n times: 8 s. One line of output from a broken program or `cat` of the
/// wrong file held the pane's thread that long, the screen frozen.
#[test]
fn b791_a_huge_count_takes_no_longer_than_the_screen() {
    let slow: Vec<String> = [
        &b"\x1b[20000000@"[..],
        b"a\x1b[100000000b",
        b"\x1b[999999999I",
        b"\x1b[999999999Z",
        b"\x1bPq\"1;1;10;6#0!4294967295~\x1b\\",
        b"\x1b[999999999D",
        b"\x1b[?45h\x1b[999999999D",
        b"\x1b[>1u\x1b[<999999999u",
    ]
    .into_iter()
    .filter_map(|seq| {
        let took = takes(seq);
        (took >= Duration::from_secs(1))
            .then(|| format!("{:?} took {took:?}", String::from_utf8_lossy(seq)))
    })
    .collect();
    assert!(slow.is_empty(), "{slow:#?}");
}

/// The counts still mean what they meant: REP's screen, history and cursor
/// as the count said, ICH past the margin as far as the margin, CUB as
/// many backspaces, a pop as far as the stack goes, CHT and CBT to the last
/// and first stops.
#[test]
fn b791_a_bounded_count_means_what_the_count_meant() {
    // REP: once a 4×2 screen and its 3 lines of history are full of the
    // cell ((2+3+1)×4 cells), each 4 more only scroll a line of it away
    // and bring the cursor back: a count past that point means the same
    // as itself less any multiple of 4 (what W28 counts by), on the old
    // code as on the new. And split in two, it means the same whole.
    let rep = |counts: &[usize]| {
        let mut t = term(4, 2, 3);
        t.feed(b"a");
        for n in counts {
            t.feed(format!("\x1b[{n}b").as_bytes());
        }
        state(&t)
    };
    for n in [24usize, 25, 27, 100, 1001] {
        for k in [1usize, 5, 250] {
            assert_eq!(rep(&[n]), rep(&[n + 4 * k]), "REP {n} and {}", n + 4 * k);
        }
        assert_eq!(rep(&[n]), rep(&[n / 2, n - n / 2]), "REP {n} in two");
    }
    // (REP repeats the cell left of the cursor, wezterm's reading: a
    // split that leaves the cursor in column 0 repeats a blank.)
    assert_eq!(rep(&[7]), rep(&[2, 5]));
    // ICH past the margin: the rest of the line blank, as far as it goes.
    let mut t = term(10, 2, 0);
    t.feed(b"abcdefghij\x1b[1;3H\x1b[999999999@");
    assert_eq!(rows(&t)[0], "ab");
    let mut exact = term(10, 2, 0);
    exact.feed(b"abcdefghij\x1b[1;3H\x1b[8@");
    assert_eq!(state(&t), state(&exact));
    // CUB means as many backspaces (BS, one byte each), stopping at the
    // left margin or, with reverse wraparound (?45), going round the
    // margins backwards: the same place for counts well past a round, with
    // margins whose round doesn't divide the screen and in origin mode.
    for setup in [
        "",
        "\x1b[?45h",
        "\x1b[?45h\x1b[?69h\x1b[2;4s\x1b[2;3r",
        "\x1b[?45h\x1b[?69h\x1b[2;4s\x1b[2;3r\x1b[?6h",
    ] {
        let cub = |bytes: &[u8]| {
            let mut t = term(7, 3, 0);
            t.feed(format!("{setup}\x1b[2;3Hab").as_bytes());
            t.feed(bytes);
            (t.cursor(), state(&t))
        };
        for n in [5usize, 48, 49, 50, 100, 1001, 9005] {
            assert_eq!(
                cub(format!("\x1b[{n}D").as_bytes()),
                cub(&vec![b'\x08'; n]),
                "CUB {n} after {setup:?}"
            );
        }
    }
    // A pop past the stack empties it, and only it.
    let mut t = term(10, 2, 0);
    t.feed(b"\x1b[>1u\x1b[>3u\x1b[<999999999u");
    assert_eq!(t.modes().kitty_flags, 0);
    t.feed(b"\x1b[>5u");
    assert_eq!(t.modes().kitty_flags, 5);
    // CHT to the last stop, CBT to the first column.
    let mut t = term(80, 2, 0);
    t.feed(b"\x1b[1;5H\x1b[999999999I");
    assert_eq!(t.cursor().0, 79);
    t.feed(b"\x1b[999999999Z");
    assert_eq!(t.cursor().0, 0);
}
