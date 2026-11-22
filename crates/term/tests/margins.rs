//! Scroll margins (DECSTBM) as programs really send them.

use moshpit_term::{Options, Terminal};

fn rows(term: &Terminal, n: i32) -> Vec<String> {
    (0..n).map(|r| term.row_text(r).0).collect()
}

/// agy (Antigravity CLI) in its altscreen mode sets margins above its
/// footer to scroll the conversation, then resets them with `CSI ; r`.
/// That reset was dropped: the margins stayed, and each newline written
/// below them later scrolled the band above the footer, so lines were
/// repeated and the conversation slid up on the phone.
#[test]
fn margins_left_empty_are_the_full_screen() {
    for reset in ["\x1b[;r", "\x1b[r", "\x1b[0;0r", "\x1b[1;r"] {
        let mut term = Terminal::new(10, 6, Options::default());
        term.feed(b"\x1b[?1049h\x1b[6;1Hfoot\x1b[1;3r");
        term.feed(reset.as_bytes());
        term.feed(b"\x1b[1;1Ha\r\nb\r\nc\r\nd\r\n");
        assert_eq!(
            rows(&term, 6),
            ["a", "b", "c", "d", "", "foot"],
            "reset {reset:?}"
        );
    }
}

/// agy's own scroll step, byte for byte in shape: margins over the
/// conversation, a newline at their foot, the margins reset, then rows
/// written down to just above the footer.
#[test]
fn agys_scroll_step_leaves_the_footer_and_repeats_nothing() {
    let mut term = Terminal::new(12, 8, Options::default());
    term.feed(b"\x1b[?1049h");
    for (i, row) in ["one", "two", "three", "four", "five"].iter().enumerate() {
        term.feed(format!("\x1b[{};1H{row}", i + 1).as_bytes());
    }
    term.feed(b"\x1b[7;1H----\x1b[8;1Hesc");
    // Scroll rows 1-5 by one, then reset the margins.
    term.feed(b"\x1b[1;5r\x1b[5;1H\n\x1b[;r");
    term.feed(b"\x1b[4;1H\x1b[Ksix\r\n\x1b[Kseven\r\n");
    assert_eq!(
        rows(&term, 8),
        ["two", "three", "four", "six", "seven", "", "----", "esc"]
    );
}
