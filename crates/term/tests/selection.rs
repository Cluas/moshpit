//! The SwiftTerm fork's selection bugs (docs/nsh/PROGRESS.md) on the
//! shared core, and search: a selection stays on its text or goes, never
//! lands on other text.

mod common;
use common::*;
use moshpit_term::SelectKind;

fn lines(range: std::ops::RangeInclusive<usize>) -> String {
    range.map(|i| format!("L{i}\r\n")).collect()
}

/// Scrolls the view, from the top of the history down, until `text` shows;
/// its viewport row.
fn find_row(t: &mut moshpit_term::Terminal, text: &str) -> u16 {
    t.scroll(i32::MAX);
    loop {
        if let Some(r) = t.visible_text().iter().position(|r| r.0 == text) {
            return r as u16;
        }
        assert!(t.display_offset() > 0, "{text} is not in the terminal");
        t.scroll(-1);
    }
}

#[test]
fn b79_ed3_drops_a_selection_in_the_history() {
    let mut t = term(10, 3, 100);
    t.feed(lines(1..=8).as_bytes());
    let row = find_row(&mut t, "L2");
    t.select_start(SelectKind::Cells, row, 0, false);
    t.select_extend(row, 1, true);
    assert_eq!(t.selection_text().as_deref(), Some("L2"));
    t.feed(b"\x1b[3J");
    assert_eq!(t.selection_text(), None);
}

#[test]
fn b100_a_full_reset_drops_the_selection() {
    let mut t = term(10, 3, 100);
    t.feed(b"hello");
    t.select_start(SelectKind::Word, 0, 1, false);
    assert_eq!(t.selection_text().as_deref(), Some("hello"));
    t.feed(b"\x1bc");
    assert_eq!(t.selection_text(), None);
}

#[test]
fn b180_a_full_history_scrolls_under_the_selection() {
    let mut t = term(10, 3, 5);
    t.feed(lines(1..=10).as_bytes());
    let row = find_row(&mut t, "L6");
    t.select_start(SelectKind::Cells, row, 0, false);
    t.select_extend(row, 1, true);
    t.scroll_to_bottom();
    assert_eq!(t.selection_text().as_deref(), Some("L6"));
    t.feed(b"x\r\ny\r\n");
    assert_eq!(
        t.selection_text().as_deref(),
        Some("L6"),
        "the selection slid off its text"
    );
    t.feed(lines(20..=30).as_bytes());
    assert_eq!(
        t.selection_text(),
        None,
        "L6 left the history, its selection must go"
    );
}

#[test]
fn b181_switching_screens_drops_the_selection() {
    let mut t = term(10, 3, 100);
    t.feed(b"hello");
    t.select_start(SelectKind::Word, 0, 0, false);
    t.feed(b"\x1b[?1049h");
    assert_eq!(t.selection_text(), None);
}

#[test]
fn b183_b215_the_last_column_belongs_to_the_selection() {
    let mut t = term(5, 3, 100);
    t.feed(b"ABCDE");
    t.select_start(SelectKind::Line, 0, 2, false);
    assert_eq!(t.selection_text().as_deref(), Some("ABCDE\n"));
    t.select_start(SelectKind::Cells, 0, 2, false);
    t.select_extend(0, 4, true);
    assert_eq!(t.selection_text().as_deref(), Some("CDE"));
}

#[test]
fn b216_a_shorter_history_under_a_selection() {
    let mut t = term(10, 3, 100);
    t.feed(lines(1..=50).as_bytes());
    let row = find_row(&mut t, "L1");
    t.select_start(SelectKind::Cells, row, 0, false);
    t.select_extend(row, 1, true);
    assert_eq!(t.selection_text().as_deref(), Some("L1"));
    t.set_scrollback(10);
    assert_eq!(t.history_len(), 10);
    assert_eq!(t.selection_text(), None, "L1 is gone, so is its selection");
    // One that survives the cut stays on its text.
    let row = find_row(&mut t, "L45");
    t.select_start(SelectKind::Cells, row, 0, false);
    t.select_extend(row, 2, true);
    t.set_scrollback(5);
    assert_eq!(t.selection_text().as_deref(), Some("L45"));
}

#[test]
fn b98_select_all_costs_what_the_text_costs() {
    let mut t = term(80, 24, 10_000);
    t.feed(lines(1..=1000).as_bytes());
    t.scroll(i32::MAX);
    t.select_start(SelectKind::Line, 0, 0, false);
    t.scroll_to_bottom();
    t.select_extend(23, 79, true);
    let text = t.selection_text().unwrap();
    assert!(
        text.starts_with("L1\n") && text.contains("L1000\n"),
        "{}",
        &text[..20]
    );
    assert!(
        text.len() < 10_000,
        "{} bytes for 1000 short lines",
        text.len()
    );
}

#[test]
fn a_wide_character_is_selected_whole() {
    let mut t = term(10, 3, 100);
    t.feed("a中b".as_bytes());
    t.select_start(SelectKind::Cells, 0, 2, false); // the spacer of 中
    t.select_extend(0, 3, true);
    assert_eq!(t.selection_text().as_deref(), Some("中b"));
}

#[test]
fn search_finds_and_scrolls_to_matches() {
    let mut t = term(20, 4, 1000);
    t.feed(b"alpha\r\n");
    t.feed(lines(1..=50).as_bytes());
    t.feed(b"alpha beta\r\n");
    let last = t.search("alpha", false).unwrap();
    assert_eq!(t.display_offset(), 0);
    assert_eq!(t.visible_text()[last.start.0 as usize].0, "alpha beta");
    let first = t.search("alpha", false).unwrap();
    assert!(
        t.display_offset() > 0,
        "the view scrolled to the older match"
    );
    assert_eq!(t.visible_text()[first.start.0 as usize].0, "alpha");
    assert_eq!((first.start.1, first.end.1), (0, 4));
    assert!(t.search("[", true).is_none(), "a bad pattern finds nothing");
}

// --- B572: a column reflow keeps the selection on its text -------------------

/// A rotation (or an iPad's split view) reflows the text to the new width:
/// the selection used to go with it. It stays on the text it held,
/// wherever that text flows, by its place in its logical line.
#[test]
fn b572_a_selection_keeps_its_text_across_a_reflow() {
    let mut t = term(20, 4, 100);
    t.feed(b"abcdefghijklmnopqrst\r\nsecond line here\r\n");
    t.select_start(SelectKind::Cells, 1, 0, false);
    t.select_extend(1, 5, true);
    assert_eq!(t.selection_text().as_deref(), Some("second"));
    t.resize(10, 4);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("second"),
        "narrower: {:?}",
        rows(&t)
    );
    t.resize(30, 4);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("second"),
        "wider again: {:?}",
        rows(&t)
    );
}

/// Across the wrap, through wide characters, a word's, a line's.
#[test]
fn b572_wrapped_wide_and_semantic_selections_follow_the_text() {
    let mut t = term(10, 6, 100);
    t.feed("0123456789abcdefghij\r\nab中文cd ef\r\n".as_bytes());
    // "89ab" over the wrap.
    t.select_start(SelectKind::Cells, 0, 8, false);
    t.select_extend(1, 1, true);
    assert_eq!(t.selection_text().as_deref(), Some("89ab"));
    t.resize(7, 6);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("89ab"),
        "{:?}",
        rows(&t)
    );
    // A wide character, whole.
    t.resize(10, 6);
    let row = rows(&t).iter().position(|r| r.starts_with("ab中")).unwrap() as u16;
    t.select_start(SelectKind::Cells, row, 2, false);
    t.select_extend(row, 5, true);
    assert_eq!(t.selection_text().as_deref(), Some("中文"));
    t.resize(5, 6);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("中文"),
        "{:?}",
        rows(&t)
    );
    // A word, and a whole line.
    t.resize(10, 6);
    let row = rows(&t).iter().position(|r| r.starts_with("ab中")).unwrap() as u16;
    t.select_start(SelectKind::Word, row, 0, false);
    let word = t.selection_text();
    t.resize(4, 6);
    assert_eq!(t.selection_text(), word);
    t.resize(10, 6);
    t.select_start(SelectKind::Line, 0, 3, false);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("0123456789abcdefghij\n")
    );
    t.resize(6, 6);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("0123456789abcdefghij\n")
    );
}

/// A drag that ended past the text (on the blanks after it) still ends
/// there; a block's columns mean nothing after a reflow, and an end whose
/// line the reflow pushed out of the history can't be placed: those go,
/// as before.
#[test]
fn b572_past_the_end_stays_and_what_cannot_be_placed_goes() {
    let mut t = term(20, 4, 100);
    t.feed(b"short\r\nnext\r\n");
    t.select_start(SelectKind::Cells, 0, 2, false);
    t.select_extend(0, 15, true);
    assert_eq!(t.selection_text().as_deref(), Some("ort"));
    t.resize(4, 4);
    assert_eq!(t.selection_text().as_deref(), Some("ort"), "{:?}", rows(&t));
    // A block on text the reflow moves: goes, as before.
    let mut b = term(20, 4, 100);
    b.feed(b"abcdefghijklmnopqrstuvwxyz\r\n");
    b.select_start(SelectKind::Block, 0, 0, false);
    b.select_extend(1, 2, true);
    assert_eq!(b.selection_text().as_deref(), Some("abc\nuvw"));
    b.resize(10, 4);
    assert_eq!(b.selection_text(), None);
    // A line the history no longer holds: goes.
    let mut u = term(20, 3, 2);
    u.feed(b"keep this one\r\nb\r\nc\r\n");
    let row = find_row(&mut u, "keep this one");
    u.select_start(SelectKind::Cells, row, 0, false);
    u.select_extend(row, 3, true);
    assert_eq!(u.selection_text().as_deref(), Some("keep"));
    u.resize(4, 3);
    assert_ne!(
        u.selection_text().as_deref(),
        Some("b"),
        "landed on other text"
    );
    assert!(matches!(u.selection_text().as_deref(), None | Some("keep")));
}
