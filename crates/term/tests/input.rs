//! What a view sends for keys and the mouse: the emulator's encoders, as
//! the remote program asked (DECCKM, the kitty keyboard protocol, SGR
//! mouse reports), so every platform sends the same bytes (the SwiftTerm
//! fork's key tables and its patches 50–52, 61, 62 and B237 on the view
//! side).

mod common;
use common::*;
use moshpit_term::{
    Key,
    input::mods::{ALT, CTRL, SHIFT},
};

fn key(t: &mut moshpit_term::Terminal, k: Key, mods: u8) -> String {
    String::from_utf8(t.key(k, mods, true, 1)).unwrap()
}

#[test]
fn arrows_follow_decckm() {
    let mut t = term(20, 4, 10);
    assert_eq!(key(&mut t, Key::Up, 0), "\x1b[A");
    assert_eq!(key(&mut t, Key::Left, 0), "\x1b[D");
    t.feed(b"\x1b[?1h");
    assert_eq!(key(&mut t, Key::Up, 0), "\x1bOA", "application cursor keys");
    assert_eq!(key(&mut t, Key::Home, 0), "\x1bOH");
    assert_eq!(
        key(&mut t, Key::Up, SHIFT),
        "\x1b[1;2A",
        "modified: CSI form"
    );
    t.feed(b"\x1b[?1l");
    assert_eq!(key(&mut t, Key::Up, 0), "\x1b[A");
}

#[test]
fn the_keys_a_terminal_needs() {
    let mut t = term(20, 4, 10);
    assert_eq!(key(&mut t, Key::Char('a'), 0), "a");
    assert_eq!(key(&mut t, Key::Char('c'), CTRL), "\x03");
    assert_eq!(key(&mut t, Key::Char('['), CTRL), "\x1b");
    assert_eq!(key(&mut t, Key::Char('x'), ALT), "\x1bx", "Option as Meta");
    assert_eq!(key(&mut t, Key::Enter, 0), "\r");
    assert_eq!(key(&mut t, Key::Backspace, 0), "\x7f");
    assert_eq!(key(&mut t, Key::Tab, 0), "\t");
    assert_eq!(
        key(&mut t, Key::Tab, SHIFT),
        "\x1b[Z",
        "Shift-Tab (patch 50)"
    );
    assert_eq!(key(&mut t, Key::Escape, 0), "\x1b", "Escape (patch 51)");
    assert_eq!(key(&mut t, Key::Delete, 0), "\x1b[3~");
    assert_eq!(key(&mut t, Key::PageUp, 0), "\x1b[5~");
    assert_eq!(key(&mut t, Key::Function(1), 0), "\x1bOP");
    assert_eq!(key(&mut t, Key::Function(5), 0), "\x1b[15~");
    assert_eq!(
        key(&mut t, Key::LeftShift, 0),
        "",
        "a modifier alone sends nothing"
    );
    assert!(
        t.key(Key::Char('a'), 0, false, 1).is_empty(),
        "no releases outside kitty"
    );
}

/// Option as Meta (B413): what the view's Option chords send, the core's
/// Alt encoding — ESC and the key, Meta-Enter and Meta-Backspace too —
/// and kitty's under its protocol.
#[test]
fn alt_chords_are_meta() {
    let mut t = term(20, 4, 10);
    assert_eq!(key(&mut t, Key::Char('b'), ALT), "\x1bb");
    assert_eq!(key(&mut t, Key::Char('e'), ALT | SHIFT), "\x1bE");
    assert_eq!(key(&mut t, Key::Enter, ALT), "\x1b\r", "Meta-Enter");
    assert_eq!(
        key(&mut t, Key::Backspace, ALT),
        "\x1b\x7f",
        "Meta-Backspace"
    );
    t.feed(b"\x1b[>1u");
    assert_eq!(key(&mut t, Key::Char('b'), ALT), "\x1b[98;3u");
    assert_eq!(key(&mut t, Key::Enter, ALT), "\x1b[13;3u");
}

#[test]
fn a_key_never_takes_replies_owed_to_the_remote() {
    let mut t = term(20, 4, 10);
    t.feed(b"\x1b[6n");
    assert_eq!(key(&mut t, Key::Char('q'), 0), "q");
    assert_eq!(
        t.take_replies(),
        b"\x1b[1;1R",
        "the reply stays where it was"
    );
}

#[test]
fn kitty_protocol_presses_repeats_and_releases() {
    let mut t = term(20, 4, 10);
    // Disambiguate + event types (1 | 2).
    t.feed(b"\x1b[>3u");
    assert_eq!(t.modes().kitty_flags, 3);
    assert_eq!(
        key(&mut t, Key::Escape, 0),
        "\x1b[27;1u",
        "Escape disambiguated"
    );
    assert_eq!(key(&mut t, Key::Char('a'), 0), "a", "plain text stays text");
    assert_eq!(key(&mut t, Key::Char('c'), CTRL), "\x1b[99;5u");
    let up = String::from_utf8(t.key(Key::Char('a'), 0, false, 1)).unwrap();
    assert_eq!(
        up, "\x1b[97;1:3u",
        "a release (B237: what a cancelled press sends)"
    );
    let held = String::from_utf8(t.key(Key::Up, 0, true, 2)).unwrap();
    assert_eq!(held, "\x1b[1;1:2A", "a repeat");
    // Pop: back to xterm's keys.
    t.feed(b"\x1b[<u");
    assert_eq!(t.modes().kitty_flags, 0);
    assert_eq!(key(&mut t, Key::Escape, 0), "\x1b");
}

#[test]
fn kitty_report_all_keys_sends_modifiers_and_text_as_codes() {
    let mut t = term(20, 4, 10);
    t.feed(b"\x1b[>11u");
    assert_eq!(
        key(&mut t, Key::Char('a'), 0),
        "\x1b[97;1u",
        "text as codes (W18)"
    );
    assert!(!key(&mut t, Key::LeftShift, SHIFT).is_empty());
}

#[test]
fn mouse_reports_follow_the_program() {
    let mut t = term(40, 10, 10);
    assert!(
        t.mouse(0, 0, 2, 3, 0).is_empty(),
        "no reports asked for: nothing"
    );
    t.feed(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(t.modes().mouse, 1);
    assert_eq!(t.mouse(0, 0, 2, 3, 0), b"\x1b[<0;4;3M");
    assert_eq!(t.mouse(1, 0, 2, 3, 0), b"\x1b[<0;4;3m");
    assert_eq!(t.mouse(0, 4, 0, 0, 0), b"\x1b[<64;1;1M", "wheel up");
    assert_eq!(t.mouse(0, 5, 0, 0, 0), b"\x1b[<65;1;1M", "wheel down");
}

#[test]
fn focus_reports_only_when_asked() {
    let mut t = term(20, 4, 10);
    assert!(t.focus(false).is_empty());
    assert!(t.focus(true).is_empty());
    t.feed(b"\x1b[?1004h");
    assert_eq!(t.focus(false), b"\x1b[O");
    assert_eq!(t.focus(true), b"\x1b[I");
}

#[test]
fn view_rows_read_what_the_view_shows() {
    let mut t = term(10, 3, 100);
    t.feed(b"one\r\ntwo\r\nthree\r\nfour\r\n0123456789ab");
    assert_eq!(t.view_row_text(0).0, "four");
    assert_eq!(t.view_row_text(1), ("0123456789".to_string(), true));
    t.scroll(2);
    assert_eq!(t.view_row_text(0).0, "two");
    assert_eq!(
        t.view_row_text(3).0,
        "0123456789",
        "the row under the viewport"
    );
}

#[test]
fn an_extra_row_rides_with_the_frame() {
    let mut t = term(10, 3, 100);
    t.feed(b"1\r\n2\r\n3\r\n4\r\n5");
    t.scroll(1);
    t.set_extra_rows(1);
    let f = t.frame();
    assert!(f.full);
    assert_eq!(f.changed.len(), 4, "three rows and the one under them");
    assert_eq!(f.rows, 3, "the grid is still three rows");
    let row3 = f.changed.iter().find(|r| r.row == 3).unwrap();
    assert_eq!(char::from_u32(row3.cells[0].ch), Some('5'));
}
