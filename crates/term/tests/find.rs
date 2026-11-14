//! Find (T3), the core's half of the find bar: the text as written, every
//! match in history and on screen counted, the newest first, round past
//! either end, the view scrolled to the match and the match drawn marked.

mod common;
use common::*;
use moshpit_term::{FindOptions, FindStep, Terminal, cell_flags};

fn found(t: &mut Terminal, text: &str, step: FindStep) -> Option<(usize, usize, (u16, u16))> {
    t.find(text, FindOptions::default(), step)
        .map(|f| (f.index, f.count, f.matched.start))
}

#[test]
fn find_counts_every_match_starts_at_the_newest_and_goes_round() {
    let mut t = term(20, 4, 100);
    for i in 0..30 {
        let word = if i % 10 == 3 { "Needle" } else { "hay" };
        t.feed(format!("{i:02} {word}\r\n").as_bytes());
    }
    // Lines 03, 13, 23: the last two in history, 23 just above the screen.
    let (index, count, _) = found(&mut t, "needle", FindStep::Newest).expect("any case");
    assert_eq!((index, count), (2, 3), "the newest of three");
    assert!(t.display_offset() > 0, "scrolled back to it");
    assert_eq!(found(&mut t, "needle", FindStep::Previous).unwrap().0, 1);
    assert_eq!(found(&mut t, "needle", FindStep::Previous).unwrap().0, 0);
    assert_eq!(
        found(&mut t, "needle", FindStep::Previous).unwrap().0,
        2,
        "round past the oldest"
    );
    assert_eq!(
        found(&mut t, "needle", FindStep::Next).unwrap().0,
        0,
        "round past the newest"
    );
    assert_eq!(
        found(&mut t, "needle", FindStep::Newest).unwrap().0,
        0,
        "the same text keeps the place"
    );
    let f = t.frame();
    let marked = f
        .changed
        .iter()
        .any(|r| r.cells.iter().any(|c| c.flags & cell_flags::MATCH != 0));
    assert!(marked, "the match draws marked");
    assert!(found(&mut t, "", FindStep::Newest).is_none());
    let f = t.frame();
    assert!(
        f.changed
            .iter()
            .all(|r| r.cells.iter().all(|c| c.flags & cell_flags::MATCH == 0)),
        "cleared"
    );
}

#[test]
fn find_takes_the_text_as_written() {
    let mut t = term(30, 4, 10);
    t.feed(b"a.b axb (x) Rust rusty\r\n");
    assert_eq!(
        found(&mut t, "a.b", FindStep::Newest).map(|f| f.1),
        Some(1),
        "a dot is a dot"
    );
    assert_eq!(
        found(&mut t, "(x)", FindStep::Newest).map(|f| f.2),
        Some((0, 8))
    );
    assert_eq!(
        found(&mut t, "rust", FindStep::Newest).map(|f| f.1),
        Some(2)
    );
    let exact = FindOptions {
        case_sensitive: true,
        ..FindOptions::default()
    };
    assert_eq!(
        t.find("rust", exact, FindStep::Newest).map(|f| f.count),
        Some(1)
    );
    let words = FindOptions {
        word_start: true,
        word_end: true,
        ..FindOptions::default()
    };
    assert_eq!(
        t.find("rust", words, FindStep::Newest).map(|f| f.count),
        Some(1),
        "rusty is not the word"
    );
    assert!(found(&mut t, "nothing here", FindStep::Newest).is_none());
}

/// The cells drawn marked as the match, row by row, as the view shows them
/// after `f` (rows a frame doesn't repaint keep what they showed).
fn paint(shown: &mut [String], f: &moshpit_term::Frame) {
    for r in &f.changed {
        let marked: String = r
            .cells
            .iter()
            .filter(|c| c.flags & cell_flags::MATCH != 0)
            .filter_map(|c| char::from_u32(c.ch))
            .collect();
        shown[r.row as usize] = marked;
    }
}

/// B520: the match drawn marked stays on its text. A full-screen program
/// redrawing, or a `clear`, put other text where it was: the mark went, and
/// a repaint of the same text keeps it (tmux's repair frame, B480). It
/// stayed on the cells, over whatever was there now.
#[test]
fn the_match_mark_goes_when_its_text_does() {
    let mut t = term(20, 4, 100);
    t.feed(b"one needle\r\ntwo\r\n$ ");
    let mut shown = vec![String::new(); 4];
    assert_eq!(
        found(&mut t, "needle", FindStep::Newest).map(|f| f.2),
        Some((0, 4))
    );
    paint(&mut shown, &t.frame());
    assert_eq!(shown[0], "needle");
    // The same text written again: still marked.
    t.feed(b"\x1b[1;1H\x1b[2Kone needle\x1b[3;3H");
    paint(&mut shown, &t.frame());
    assert_eq!(shown[0], "needle", "a repaint of the same text");
    // Other text there now: no mark, on it or anywhere.
    t.feed(b"\x1b[1;1H\x1b[2Kone thread\x1b[3;3H");
    paint(&mut shown, &t.frame());
    assert_eq!(shown, vec![String::new(); 4], "the mark went with its text");
    // The next step finds again from the newest.
    t.feed(b"\r\nneedle\r\n");
    assert_eq!(
        found(&mut t, "needle", FindStep::Next).map(|f| (f.0, f.1)),
        Some((0, 1))
    );
}

/// B520: the width changes (the phone turned, the split view resized) and
/// the lines flow again: the match shown is the same one, marked on its
/// text where it now is. The mark stayed on the rows it had been on.
#[test]
fn the_match_mark_follows_its_text_through_a_reflow() {
    let mut t = term(20, 4, 100);
    t.feed(b"a needle\r\n0123456789012345 b needle c\r\n$ ");
    // The second line wraps after 20 columns.
    let (index, count, _) = found(&mut t, "needle", FindStep::Newest).unwrap();
    assert_eq!((index, count), (1, 2));
    let mut shown = vec![String::new(); 4];
    paint(&mut shown, &t.frame());
    assert_eq!(shown.concat(), "needle");
    t.resize(12, 4);
    let mut shown = vec![String::new(); 4];
    paint(&mut shown, &t.frame());
    assert_eq!(shown.concat(), "needle", "marked on its text: {shown:?}");
    assert_eq!(
        found(&mut t, "needle", FindStep::Previous).map(|f| (f.0, f.1)),
        Some((0, 2)),
        "the same match: the one before it is the first"
    );
}
