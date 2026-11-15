//! The SwiftTerm fork's selection bugs (docs/nsh/PROGRESS.md) on the
//! shared core, and search: a selection stays on its text or goes, never
//! lands on other text.

mod common;
use common::*;
use moshpit_term::{SelectKind, Tap};

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

/// B699: Select All (the edit menu's, ⌘A) on the core's view: from the
/// first row with text, the history's included, to the last cell with
/// text — not down the blank rows under the last line (B98). The SwiftTerm
/// path had it; the core's view had none, only a drag.
#[test]
fn b699_select_all_takes_the_text_and_no_more() {
    let mut t = term(80, 24, 10_000);
    assert!(!t.select_all(), "an empty terminal has nothing to select");
    assert_eq!(t.selection_text(), None);
    t.feed(b"\r\n\r\n");
    t.feed(lines(1..=10_000).as_bytes());
    t.feed("$ 你好".as_bytes());
    t.scroll(5);
    assert!(t.select_all());
    let text = t.selection_text().expect("selected");
    assert!(text.starts_with("L1\n"), "{:?}", &text[..10]);
    assert!(
        text.ends_with("L10000\n$ 你好"),
        "{:?}",
        &text[text.len() - 20..]
    );
    assert_eq!(text.lines().count(), 10_001);
    assert!(
        text.len() < 80_000,
        "{} bytes for 10 000 short lines",
        text.len()
    );
    // The view stays where it was; the selection stands while output that
    // repaints a row as it was arrives (B698), and a tap lets it go.
    assert_eq!(t.display_offset(), 5);
    t.feed("\r\x1b[2K$ 你好".as_bytes());
    assert!(t.selection_text().is_some());
    t.scroll_to_bottom();
    assert_eq!(t.tap(0, 0), Tap::Dismissed);
    // The alternate screen: its own rows only.
    t.feed(b"\x1b[?1049h\x1b[H\x1b[2Jtop\r\n\r\nbottom");
    assert!(t.select_all());
    assert_eq!(t.selection_text().as_deref(), Some("top\n\nbottom"));
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

/// B698: a selection over the whole history (10 000 rows) while a program
/// repaints the live rows it covers as they were (a spinner's line, a
/// TUI's frame): each frame read every row the selection covers again to
/// note it, 9–34 ms a frame, and the screen stuttered while it stood. A
/// frame with it now costs about what one without it does; a row that
/// really changes still takes it away.
#[test]
fn b698_a_big_selection_costs_a_frame_little() {
    use std::time::{Duration, Instant};
    fn repaint(t: &mut moshpit_term::Terminal, row: usize, text: &str) {
        t.feed(format!("\x1b[{row};1H\x1b[2K{text}\x1b[40;1H").as_bytes());
    }
    fn best_frame(select: bool) -> (Duration, moshpit_term::Terminal) {
        let mut t = term(80, 40, 10_000);
        let text: String = (0..10_000)
            .map(|i| format!("line {i} some output text here\r\n"))
            .collect();
        t.feed(text.as_bytes());
        if select {
            t.scroll(i32::MAX);
            t.select_start(SelectKind::Line, 0, 0, false);
            t.scroll_to_bottom();
            t.select_extend(38, 79, true);
        }
        let _ = t.frame();
        let mut best = Duration::MAX;
        for _ in 0..20 {
            for r in 1..=39 {
                repaint(
                    &mut t,
                    r,
                    &format!("line {} some output text here", 9_960 + r),
                );
            }
            let at = Instant::now();
            let _ = t.frame();
            best = best.min(at.elapsed());
        }
        (best, t)
    }
    let (without, _) = best_frame(false);
    let (with, mut t) = best_frame(true);
    let text = t
        .selection_text()
        .expect("the repaints took the selection away");
    assert!(text.starts_with("line 0 ") && text.contains("line 9999 "));
    assert!(
        with < without * 4 + Duration::from_millis(2),
        "a frame with the selection {with:?}, without {without:?}"
    );
    repaint(&mut t, 10, "something else");
    let _ = t.frame();
    assert_eq!(
        t.selection_text(),
        None,
        "a row that changed kept the selection"
    );
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

/// B697: a wide character that doesn't fit before the right margin goes
/// on to the next row, and the cell it leaves is erased with the pen's
/// background (xterm's way). On a coloured line that cell was kept as a
/// blank and copied as a space: a Chinese line that wrapped there read
/// `一二三四 五`. With no colour the cell isn't kept, and a space a program
/// printed in the last column before ASCII still copies as a space.
#[test]
fn b697_the_cell_a_wrapped_wide_character_left_is_not_a_space() {
    let copy = |cols: u16, bytes: &str| {
        let mut t = term(cols, 4, 10);
        t.feed(bytes.as_bytes());
        t.select_start(SelectKind::Line, 0, 0, false);
        t.selection_text()
    };
    assert_eq!(
        copy(9, "\x1b[44m\x1b[2J\x1b[H一二三四五").as_deref(),
        Some("一二三四五\n")
    );
    assert_eq!(
        copy(9, "\x1b[7m一二三四五六七八九").as_deref(),
        Some("一二三四五六七八九\n")
    );
    assert_eq!(copy(9, "一二三四五").as_deref(), Some("一二三四五\n"));
    // A double tap and a search see the same text.
    let mut t = term(9, 4, 10);
    t.feed("\x1b[44m\x1b[2J\x1b[H一二三四五".as_bytes());
    t.select_start(SelectKind::Word, 1, 0, false);
    assert_eq!(t.selection_text().as_deref(), Some("一二三四五"));
    assert!(
        t.search("四五", false).is_some(),
        "the search ran into the blank"
    );
    // A space before ASCII at the margin is a space.
    assert_eq!(
        copy(6, "\x1b[44mhello world").as_deref(),
        Some("hello world\n")
    );
    assert_eq!(copy(6, "hello world").as_deref(), Some("hello world\n"));
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

/// The word a double tap at column `col` of `text` selects.
fn word_at(text: &str, col: u16) -> Option<String> {
    let mut t = term(60, 3, 10);
    t.feed(text.as_bytes());
    t.select_start(SelectKind::Word, 0, col, false);
    t.selection_text()
}

/// B696: a double tap's word ran through full-width punctuation. In an
/// agent's Chinese (「已保存到 /tmp/首页.png，请查看。」) the path tapped came
/// with the comma and the sentence after it, and a tap on the sentence
/// took the path before it too. Full-width punctuation, the CJK brackets
/// and quotes, and the dashes and ellipses of Chinese prose end a word as
/// ASCII punctuation does; what lies between them is still one word.
#[test]
fn b696_full_width_punctuation_ends_a_word() {
    // 已保存到 0–7, space 8, /tmp/ 9–13, 首页 14–17, .png 18–21, ，22–23,
    // 请查看 24–29, 。30–31.
    let line = "已保存到 /tmp/首页.png，请查看。";
    assert_eq!(word_at(line, 10).as_deref(), Some("/tmp/首页.png"));
    assert_eq!(word_at(line, 24).as_deref(), Some("请查看"));
    assert_eq!(word_at(line, 0).as_deref(), Some("已保存到"));
    // Brackets, quotes, the colon, the enumeration comma, dash, ellipsis.
    assert_eq!(word_at("「首页」完成", 2).as_deref(), Some("首页"));
    assert_eq!(word_at("《设计稿》：完成", 2).as_deref(), Some("设计稿"));
    assert_eq!(word_at("设计稿：完成", 0).as_deref(), Some("设计稿"));
    assert_eq!(word_at("截图、首页", 6).as_deref(), Some("首页"));
    assert_eq!(word_at("见“out/a.png”", 4).as_deref(), Some("out/a.png"));
    assert_eq!(word_at("好了——下一步……", 0).as_deref(), Some("好了"));
    assert_eq!(word_at("（测试）", 2).as_deref(), Some("测试"));
    assert_eq!(word_at("完成！结束？", 0).as_deref(), Some("完成"));
    // Unchanged: Chinese without punctuation, paths, ASCII words.
    assert_eq!(word_at("你好世界", 2).as_deref(), Some("你好世界"));
    assert_eq!(
        word_at("cd ~/code/a-b_c.d", 5).as_deref(),
        Some("~/code/a-b_c.d")
    );
}
