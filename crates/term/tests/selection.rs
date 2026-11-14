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
