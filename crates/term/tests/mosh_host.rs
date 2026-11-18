//! A terminal fed mosh's frames ([`Options::mosh_host`]) keeps a
//! two-column character printed in the last column where mosh's server
//! keeps it. mosh sends diffs of its own screen; a terminal that wraps the
//! character first, as xterm and tmux do (docs/WEZTERM-PATCHES.md W5), is a
//! line off from that screen from then on, and the server's later diffs
//! leave fragments of old rows on it (herdr over mosh, after switching
//! tabs) until a full repaint.

mod common;
use moshpit_term::{Options, Terminal};

fn mosh(cols: u16, rows: u16) -> Terminal {
    Terminal::new(
        cols,
        rows,
        Options {
            scrollback: 100,
            mosh_host: true,
            ..Options::default()
        },
    )
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(common::repo().join("crates/term/tests/fixtures/mosh").join(name)).unwrap()
}

/// What a row of mosh's diff looks like when the program left autowrap off
/// (herdr's attach sends `ESC [?7l`): a wide character in the last column,
/// then CR LF to the next row.
#[test]
fn a_wide_character_in_the_last_column_stays_on_its_row() {
    let mut t = mosh(10, 3);
    t.feed("\x1b[1;1Habcdefghi中\r\nx".as_bytes());
    let rows = common::rows(&t);
    assert_eq!(rows[0], "abcdefghi中");
    assert_eq!(rows[1], "x");
    assert_eq!(rows[2], "");
}

/// On the last row too: wrapping would scroll the whole screen up a line.
#[test]
fn a_wide_character_in_the_last_column_of_the_last_row_scrolls_nothing() {
    let mut t = mosh(10, 3);
    t.feed("\x1b[1;1Htop\x1b[3;1Habcdefghi中\x1b[2;1Hmid".as_bytes());
    assert_eq!(common::rows(&t), ["top", "mid", "abcdefghi中"]);
}

/// The next print still wraps (autowrap on): the character in the last
/// column is the line's last.
#[test]
fn the_print_after_it_wraps() {
    let mut t = mosh(10, 3);
    t.feed("abcdefghi中x".as_bytes());
    assert_eq!(common::screen(&t)[0], ("abcdefghi中".into(), true));
    assert_eq!(common::rows(&t)[1], "x");
}

/// The switch can be turned on after the terminal is made (the app learns
/// the transport after minting the view), and off again.
#[test]
fn set_mosh_host_switches_from_the_next_byte() {
    let mut t = common::term(10, 3, 100);
    t.set_mosh_host(true);
    t.feed("\x1b[1;1Habcdefghi中\r\nx".as_bytes());
    assert_eq!(common::rows(&t)[..2], ["abcdefghi中", "x"]);
    t.set_mosh_host(false);
    t.feed("\x1b[3;1Habcdefghi中".as_bytes());
    // Wrapped first (W5): the screen scrolled a line.
    assert_eq!(common::rows(&t), ["x", "abcdefghi", "中"]);
}

/// Recorded: mosh's frames to a 50 × 30 phone while a herdr attach loop
/// switched tabs twelve times, a desktop client 120 columns wide attached
/// (synthetic program, private herdr). Before the switches' repaint the
/// screen must already be what the repaint draws.
#[test]
fn herdr_tab_switches_over_mosh_leave_nothing_behind() {
    let mut t = mosh(50, 30);
    t.feed(&fixture("herdr-switch.bin"));
    t.flush_sync();
    let before = common::rows(&t);
    t.feed(&fixture("herdr-switch-redraw.bin"));
    t.flush_sync();
    let after = common::rows(&t);
    let differ: Vec<_> = before
        .iter()
        .zip(&after)
        .enumerate()
        .filter(|(_, (b, a))| b != a)
        .map(|(i, (b, a))| format!("row {i}\n  before {b:?}\n  after  {a:?}"))
        .collect();
    assert!(differ.is_empty(), "residue:\n{}", differ.join("\n"));
}
