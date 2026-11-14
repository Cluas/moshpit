//! The terminal view's requirements that are logic, not UIKit
//! (docs/nsh/TERMINAL-VIEW.md, the requirements table): each test is named
//! after its row. The UIKit glue has its own device tests
//! (CoreTerminalViewTests).

mod common;
use common::*;
use moshpit_term::{Ime, ScrollAction, SelectKind, Tap, Typed, cell_flags as F, ime::Outcome};

fn typed(o: &Outcome) -> String {
    o.typed
        .iter()
        .map(|t| match t {
            Typed::Text(s) => s.clone(),
            Typed::Backspace => "⌫".into(),
            Typed::Enter => "⏎".into(),
        })
        .collect()
}

fn focused() -> Ime {
    let mut ime = Ime::new();
    ime.focus(true);
    ime
}

// --- The input method ------------------------------------------------------

#[test]
fn v6_a_delete_while_composing_edits_the_composition_and_sends_nothing() {
    let mut ime = focused();
    ime.set_marked(Some("ni"), (2, 0));
    ime.set_marked(Some("nih"), (3, 0));
    let o = ime.delete_backward();
    assert!(o.typed.is_empty(), "nothing was sent for the composition");
    assert_eq!(ime.marked_text().as_deref(), Some("ni"));
    ime.delete_backward();
    ime.delete_backward();
    assert_eq!(ime.marked(), None, "emptied: cancelled");
    assert_eq!(ime.text(), "\n", "the phantom is still in front");
    // Pinyin commits the candidate.
    ime.set_marked(Some("ni"), (2, 0));
    let o = ime.insert("你");
    assert_eq!(typed(&o), "你");
    assert!(!o.notify, "the input system's own edit");
}

#[test]
fn v6_composition_commits_by_unmark_and_cancels_by_nil() {
    let mut ime = focused();
    ime.set_marked(Some("にほ"), (2, 0));
    ime.set_marked(Some("日本"), (2, 0));
    assert_eq!(typed(&ime.unmark()), "日本");
    ime.set_marked(Some("x"), (1, 0));
    assert!(ime.set_marked(None, (0, 0)).typed.is_empty());
    assert_eq!(ime.marked(), None);
    assert_eq!(ime.text(), "\n日本");
}

#[test]
fn v8_holding_delete_over_the_remotes_text_keeps_repeating() {
    let mut ime = focused();
    // UIKit's check before each repeat: one character back from the caret
    // is not the caret (the phantom is always there).
    for _ in 0..20 {
        let (caret, _) = ime.selected();
        assert!(caret >= 1, "something in front of the caret");
        assert_eq!(
            typed(&ime.delete_backward()),
            "⌫",
            "one press, one backspace"
        );
    }
    assert_eq!(ime.text(), "\n");
}

/// B412: the space bar's trackpad dragged left past what was typed, into
/// the remote's text: the caret stops behind the phantom, and a held
/// delete goes on repeating. It went in front of it (offset 0), where
/// nothing is in front of the caret: the hold stopped after one press.
#[test]
fn v8_the_trackpad_never_puts_the_caret_in_front_of_the_phantom() {
    let mut ime = focused();
    ime.insert("ab");
    for _ in 0..5 {
        let (c, _) = ime.selected();
        let c = c.saturating_sub(1);
        ime.select(c, c);
    }
    assert_eq!(ime.selected(), (1, 1), "in front of the phantom");
    for _ in 0..20 {
        let (caret, _) = ime.selected();
        assert!(caret >= 1, "nothing in front of the caret: the hold stops");
        assert_eq!(typed(&ime.delete_backward()), "⌫");
    }
    // A selection from the front, phantom and all, is still one.
    let mut ime = focused();
    ime.insert("a");
    ime.select(0, 2);
    assert_eq!(ime.selected(), (0, 2));
}

#[test]
fn v8_a_selection_delete_bills_only_real_characters() {
    let mut ime = focused();
    ime.insert("ab");
    ime.select(0, 3);
    assert_eq!(
        typed(&ime.delete_backward()),
        "⌫⌫",
        "the phantom costs nothing"
    );
    // Only the phantom selected: still one backspace (the hold must go on).
    ime.select(0, 1);
    assert_eq!(typed(&ime.delete_backward()), "⌫");
}

#[test]
fn v9_a_third_party_keyboards_delete_is_not_a_return() {
    // 豆包 deletes by selecting the two characters before the caret,
    // deleting both and inserting the first again.
    let mut ime = focused();
    ime.insert("a");
    ime.select(0, 2);
    let del = ime.delete_backward();
    assert_eq!(typed(&del), "⌫", "only `a` cost a backspace");
    assert!(!del.notify, "the keyboard's own edit is not reported to it");
    let put_back = ime.insert("\n");
    assert!(
        put_back.typed.is_empty(),
        "putting the phantom back pressed Return"
    );
    assert!(!put_back.notify);
    assert_eq!(ime.text(), "\n");
    // A Return typed later is a Return.
    assert_eq!(typed(&ime.insert("\n")), "⏎");
}

/// B410: a third-party keyboard deletes into the remote's own text (a
/// recalled command: the document is only the phantom) by selecting the
/// one character before the caret. Nothing is put back in the delete's
/// turn; the Return pressed after it is a Return. It was swallowed: the
/// wait for a put-back never ended.
#[test]
fn v9_a_return_after_a_delete_into_the_remotes_text_is_a_return() {
    let mut ime = focused();
    for _ in 0..3 {
        ime.select(0, 1);
        assert_eq!(typed(&ime.delete_backward()), "⌫");
        ime.turn_end();
    }
    assert_eq!(typed(&ime.insert("\n")), "⏎", "the Return was swallowed");
    // Typed, then deleted down into the remote's text the same way.
    let mut ime = focused();
    ime.insert("ab");
    for _ in 0..3 {
        let (c, _) = ime.selected();
        ime.select(c.saturating_sub(1), c);
        ime.delete_backward();
        ime.turn_end();
    }
    assert_eq!(typed(&ime.insert("\n")), "⏎");
    // Within the delete's turn, the phantom put back is still no Return.
    let mut ime = focused();
    ime.select(0, 1);
    ime.delete_backward();
    assert!(ime.insert("\n").typed.is_empty());
    ime.turn_end();
    assert_eq!(typed(&ime.insert("\n")), "⏎");
}

#[test]
fn v9_the_keyboards_own_edits_never_notify_but_resets_do() {
    let mut ime = Ime::new();
    assert!(
        ime.focus(true).notify,
        "the phantom appears: not the keyboard's doing"
    );
    assert!(!ime.insert("x").notify);
    assert!(!ime.replace(1, 2, "y").notify);
    assert!(!ime.delete_backward().notify);
    assert!(
        ime.insert("\n").notify,
        "Return resets the line behind the keyboard"
    );
    assert!(ime.reset().notify);
}

#[test]
fn v9_replacements_send_backspaces_then_the_text() {
    let mut ime = focused();
    ime.insert("teh");
    let o = ime.replace(1, 4, "the");
    assert_eq!(typed(&o), "⌫⌫⌫the");
    assert_eq!(ime.text(), "\nthe");
    assert_eq!(ime.selected(), (4, 4));
    // While composing, a replacement is ignored.
    ime.set_marked(Some("k"), (1, 0));
    assert!(ime.replace(1, 2, "zz").typed.is_empty());
}

#[test]
fn v10_auto_period_after_two_spaces() {
    let mut ime = focused();
    ime.insert("ls");
    ime.insert(" ");
    // The keyboard's double-space: deletes the space, inserts ". ".
    assert_eq!(typed(&ime.delete_backward()), "⌫");
    assert_eq!(
        typed(&ime.insert(". ")),
        "  ",
        "the shell gets its spaces, no period"
    );
    // Some keyboards replace the space in place.
    let mut ime = focused();
    ime.insert("ls ");
    assert_eq!(typed(&ime.replace(3, 4, ". ")), "⌫  ");
}

#[test]
fn graphemes_are_one_character() {
    let mut ime = focused();
    ime.insert("👨‍👩‍👧é");
    assert_eq!(ime.len(), 3);
    assert_eq!(typed(&ime.delete_backward()), "⌫");
    assert_eq!(ime.text(), "\n👨‍👩‍👧");
}

#[test]
fn typed_text_is_encoded_as_the_program_asked() {
    let mut t = term(20, 4, 10);
    let bytes = t.ime_apply(|ime| ime.focus(true)).bytes;
    assert!(bytes.is_empty());
    let bytes = t.ime_apply(|ime| ime.insert("ok")).bytes;
    assert_eq!(bytes, b"ok");
    let bytes = t.ime_apply(|ime| ime.insert("\n")).bytes;
    assert_eq!(bytes, b"\r");
    let bytes = t.ime_apply(|ime| ime.delete_backward()).bytes;
    assert_eq!(bytes, b"\x7f");
    // kitty's report-all-keys: a typed letter is its key's code.
    t.feed(b"\x1b[>9u");
    let bytes = t.ime_apply(|ime| ime.insert("a")).bytes;
    assert_eq!(bytes, b"\x1b[97;1u");
}

/// B414: lines a keyboard inserts at once (its clipboard panel, quick
/// phrases, dictation with line breaks) are a paste — the platform's paste
/// path brackets it as the program asked, or hands it to tmux or herdr —
/// never typed, which ran each line as it arrived. The line is the
/// remote's after it; a lone Return is still a Return.
#[test]
fn lines_inserted_at_once_are_a_paste() {
    let mut t = term(40, 5, 10);
    t.feed(b"\x1b[?2004h");
    t.ime_apply(|ime| ime.focus(true));
    t.ime_apply(|ime| ime.insert("x"));
    let sent = t.ime_apply(|ime| ime.insert("echo one\necho two"));
    assert!(
        sent.bytes.is_empty(),
        "typed: {:?}",
        String::from_utf8_lossy(&sent.bytes)
    );
    assert_eq!(sent.paste.as_deref(), Some("echo one\necho two"));
    assert!(sent.notify, "the document was reset: the delegate hears it");
    assert_eq!(
        t.ime.text(),
        "\n",
        "primed again, nothing of the paste in it"
    );
    for one in ["\r\n", "a\r", "\nb"] {
        assert!(
            t.ime_apply(|ime| ime.insert(one)).paste.is_some() == (one != "\r\n"),
            "{one:?}"
        );
    }
    let sent = t.ime_apply(|ime| ime.insert("\n"));
    assert_eq!((sent.bytes.as_slice(), sent.paste), (&b"\r"[..], None));
}

// --- Taps ------------------------------------------------------------------

#[test]
fn v12_a_tap_takes_the_keyboard_only_near_the_cursor_on_the_live_screen() {
    let mut t = term(20, 10, 100);
    t.feed(b"$ ");
    assert_eq!(t.tap(0, 5), Tap::Click { keyboard: true });
    assert_eq!(
        t.tap(2, 5),
        Tap::Click { keyboard: true },
        "two rows from the cursor"
    );
    assert_eq!(t.tap(3, 5), Tap::Click { keyboard: false });
    // Reading history: never.
    t.feed(
        (0..30)
            .map(|i| format!("{i}\r\n"))
            .collect::<String>()
            .as_bytes(),
    );
    t.feed(b"$ ");
    t.set_cell_pixels(10, 20);
    t.scroll_drag(45.0, false);
    let cursor_row = t.frame().cursor.row;
    assert_eq!(t.tap(cursor_row.min(9), 1), Tap::Click { keyboard: false });
}

#[test]
fn v13_a_tap_on_a_link_opens_it() {
    let mut t = term(40, 4, 10);
    t.feed(b"\x1b]8;;https://example.com/a\x1b\\open\x1b]8;;\x1b\\ see https://example.com/b");
    assert_eq!(t.tap(0, 1), Tap::Link("https://example.com/a".into()));
    assert_eq!(
        t.tap(0, 12),
        Tap::Link("https://example.com/b".into()),
        "a detected URL too"
    );
}

#[test]
fn v14_a_tap_with_a_selection_only_dismisses_it() {
    let mut t = term(20, 4, 10);
    t.feed(b"hello world");
    t.select_start(SelectKind::Word, 0, 1, false);
    assert!(t.selection_text().is_some());
    assert_eq!(t.tap(0, 8), Tap::Dismissed);
    assert!(t.selection_text().is_none());
    assert_ne!(t.tap(0, 8), Tap::Dismissed, "the next tap is a tap");
}

// --- Selection -------------------------------------------------------------

#[test]
fn v15_a_long_press_selects_the_link_or_the_word() {
    let mut t = term(20, 4, 10);
    // A URL wrapped over two rows is one link (the fork's patch 22).
    t.feed(b"go https://example.com/long/path ok");
    t.select_start(SelectKind::Semantic, 1, 2, false);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("https://example.com/long/path")
    );
    t.select_start(SelectKind::Semantic, 0, 0, false);
    assert_eq!(t.selection_text().as_deref(), Some("go"));
    // It slides from its own word: another press re-anchors (patch 56).
    t.select_start(SelectKind::Semantic, 1, 14, false);
    assert_eq!(t.selection_text().as_deref(), Some("ok"));
}

#[test]
fn v17_a_drag_to_the_edge_after_the_last_column_takes_it() {
    let mut t = term(10, 2, 10);
    t.feed(b"0123456789");
    t.select_start(SelectKind::Cells, 0, 5, false);
    t.select_extend(0, 9, true);
    assert_eq!(t.selection_text().as_deref(), Some("56789"));
}

#[test]
fn v18_double_and_triple_taps() {
    let mut t = term(10, 3, 10);
    t.feed(b"abc def ghij-kl");
    t.select_start(SelectKind::Word, 0, 5, false);
    assert_eq!(t.selection_text().as_deref(), Some("def"));
    t.select_start(SelectKind::Line, 0, 1, false);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("abc def ghij-kl\n"),
        "the whole wrapped line"
    );
}

// --- Scrolling -------------------------------------------------------------

fn tall(rows: usize) -> moshpit_term::Terminal {
    let mut t = term(20, 5, 1_000);
    t.set_cell_pixels(10, 30);
    t.feed(
        (0..rows)
            .map(|i| format!("{i}\r\n"))
            .collect::<String>()
            .as_bytes(),
    );
    t.feed(b"$");
    let _ = t.frame();
    t
}

#[test]
fn v19_local_history_moves_by_the_pixel() {
    let mut t = tall(50);
    assert_eq!(
        t.scroll_drag(45.4, false),
        ScrollAction::Local { moved_px: 45 }
    );
    let f = t.frame();
    assert_eq!(
        f.display_offset, 2,
        "two lines back for 45 px of 30-px rows"
    );
    assert_eq!(
        f.pixel_shift, 15,
        "drawn 15 px up: half of the row above shows"
    );
    assert_eq!(f.changed.len(), 6, "with the row under the viewport");
    // Never past the history's top.
    let ScrollAction::Local { moved_px } = t.scroll_drag(1e9, false) else {
        panic!()
    };
    assert!(moved_px > 0);
    assert_eq!(t.scroll_drag(10.0, false), ScrollAction::None, "at the top");
}

#[test]
fn v19_a_lift_near_the_live_screen_docks_onto_it() {
    let mut t = tall(50);
    t.scroll_drag(14.0, false);
    assert_eq!(t.scroll_end(), ScrollAction::Local { moved_px: -14 });
    assert_eq!(t.frame().display_offset, 0);
    t.scroll_drag(16.0, false);
    assert_eq!(
        t.scroll_end(),
        ScrollAction::None,
        "half a row or more stays"
    );
}

#[test]
fn v20_the_drags_destination() {
    // The session scrolls (tmux, herdr): whole rows for it, carried.
    let mut t = tall(50);
    assert_eq!(t.scroll_drag(20.0, true), ScrollAction::None);
    assert_eq!(t.scroll_drag(20.0, true), ScrollAction::Host { rows: 1 });
    assert_eq!(
        t.frame().display_offset,
        0,
        "the local history did not move"
    );
    // A program that wants the mouse: wheel ticks, at most six a move.
    let mut t = tall(50);
    t.feed(b"\x1b[?1000h\x1b[?1006h");
    let ScrollAction::Wheel(bytes) = t.scroll_drag(65.0, false) else {
        panic!("no wheel")
    };
    assert_eq!(bytes, b"\x1b[<64;11;3M\x1b[<64;11;3M");
    let ScrollAction::Wheel(bytes) = t.scroll_drag(-1000.0, false) else {
        panic!("no wheel")
    };
    assert_eq!(bytes.iter().filter(|&&b| b == b'M').count(), 6);
}

#[test]
fn v21_output_while_reading_leaves_the_view_and_typing_returns_it() {
    let mut t = tall(50);
    t.scroll_drag(75.0, false);
    let before = t.frame();
    let top = t.view_row_text(0);
    t.feed(b"\r\nmore\r\nand more");
    let after = t.frame();
    assert_eq!(t.view_row_text(0), top, "the text under the finger stays");
    assert_eq!(after.pixel_shift, before.pixel_shift, "to the pixel");
    let bytes = t.key(moshpit_term::Key::Char('x'), 0, true, 1);
    assert_eq!(bytes, b"x");
    assert_eq!(
        t.frame().display_offset,
        0,
        "typing goes back to the live screen"
    );
    t.scroll_drag(75.0, false);
    t.paste("p", false);
    assert_eq!(t.frame().display_offset, 0, "so does a paste");
}

/// B424: reading history when the program clears it (ESC[3J: `clear`,
/// Claude Code's repaint, tmux forwarding either): the history the view
/// stood in is gone, so the view is live again and follows what comes
/// next. It kept a row below the new history's start: right after the
/// clear it looked live, then output went on below the screen and the
/// screen stayed put until a drag or a key.
#[test]
fn v21_a_view_whose_history_is_cleared_goes_live() {
    let mut t = term(40, 10, 1000);
    t.set_cell_pixels(10, 16);
    for i in 0..100 {
        t.feed(format!("line {i}\r\n").as_bytes());
    }
    t.scroll_drag(30.0 * 16.0, false);
    assert!(t.display_offset() > 0);
    t.feed(b"\x1b[2J\x1b[3J\x1b[H");
    for i in 0..30 {
        t.feed(format!("new {i}\r\n").as_bytes());
    }
    let f = t.frame();
    assert_eq!(f.display_offset, 0, "the view follows the output");
    assert_eq!(f.pixel_shift, 0);
    assert_eq!(t.view_row_text(8).0, "new 29");

    // History that goes on existing stays the reader's: rows leaving a
    // full history under the view don't send it live.
    let mut t = term(40, 10, 50);
    for i in 0..100 {
        t.feed(format!("line {i}\r\n").as_bytes());
    }
    t.scroll(40);
    for i in 0..20 {
        t.feed(format!("more {i}\r\n").as_bytes());
    }
    assert!(t.display_offset() > 0, "still reading");
    // An ED 3 on the alternate screen clears none of the main screen's.
    let mut t = term(40, 10, 1000);
    for i in 0..100 {
        t.feed(format!("line {i}\r\n").as_bytes());
    }
    let history = t.history_len();
    t.feed(b"\x1b[?1049h");
    t.feed(b"\x1b[3J\x1b[?1049l");
    t.scroll(30);
    assert_eq!(t.display_offset(), 30);
    assert_eq!(t.history_len(), history);
}

#[test]
fn v22_no_coast_unless_the_host_asks() {
    let mut t = tall(500);
    assert!(
        !t.scroll_fling(3_000.0),
        "off by default (1.0.4's feedback)"
    );
    t.set_coast(true);
    assert!(t.scroll_fling(3_000.0));
    let mut moved = 0;
    for _ in 0..600 {
        match t.scroll_tick(1.0 / 60.0) {
            ScrollAction::Local { moved_px } => moved += moved_px,
            _ => break,
        }
    }
    assert!(moved > 300, "coasted {moved} px");
    assert!(!t.coasting(), "and stopped");
}

#[test]
fn v19_a_search_moves_the_view_on_whole_rows() {
    let mut t = tall(50);
    t.scroll_drag(45.0, false);
    t.search("^7$", true).unwrap();
    let f = t.frame();
    assert_eq!(f.pixel_shift, 0);
    assert!(f.display_offset > 0);
}

// --- Pastes ----------------------------------------------------------------

#[test]
fn v30_a_paste_is_bracketed_as_the_program_asked_without_markers() {
    let mut t = term(20, 4, 10);
    assert_eq!(t.paste("a\nb", false), b"a\nb");
    t.feed(b"\x1b[?2004h");
    assert_eq!(t.paste("a\x1b[201~b", false), b"\x1b[200~ab\x1b[201~");
    t.feed(b"\x1b[?2004l");
    assert_eq!(
        t.paste("x", true),
        b"\x1b[200~x\x1b[201~",
        "the host forces it (herdr)"
    );
}

// --- Accessibility ---------------------------------------------------------

#[test]
fn v33_what_appears_is_announced_once() {
    let mut t = term(20, 4, 100);
    t.feed(b"first\r\n$ ");
    assert_eq!(
        t.take_announcement(),
        None,
        "the first look sets the baseline"
    );
    t.feed(b"ls\r\nfile.txt\r\n$ ");
    assert_eq!(t.take_announcement().as_deref(), Some("$ ls\nfile.txt"));
    assert_eq!(t.take_announcement(), None, "once");
    // Typing on the cursor's row is the keyboard's to speak.
    t.feed(b"c");
    assert_eq!(t.take_announcement(), None);
    // Output that scrolls the screen: only the new lines.
    t.feed(b"\r\none\r\ntwo\r\nthree\r\n$ ");
    assert_eq!(t.take_announcement().as_deref(), Some("one\ntwo\nthree"));
}

// --- Links -----------------------------------------------------------------

#[test]
fn v34_detected_urls_draw_as_links_across_wrapped_rows() {
    let mut t = term(20, 4, 10);
    t.feed(b"go https://example.com/long/path ok");
    let f = t.frame();
    let row = |r: u16| f.changed.iter().find(|x| x.row == r).unwrap();
    let linked = |r: u16| -> Vec<usize> {
        row(r)
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.flags & F::LINK != 0)
            .map(|(i, _)| i)
            .collect()
    };
    assert_eq!(linked(0), (3..20).collect::<Vec<_>>());
    assert_eq!(
        linked(1),
        (0..12).collect::<Vec<_>>(),
        "the wrapped tail too"
    );
    assert_eq!(t.link_at(1, 4).unwrap().0, "https://example.com/long/path");
}

fn osc8(url: &str, params: &str, text: &str) -> String {
    format!("\x1b]8;{params};{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

/// B423 (a): a program that lays out its own text (Claude Code) prints a
/// long link as rows of its own, each re-opening it with the full address,
/// the continuation indented. It is one link: a long press selects all of
/// it, and a copy of it (or of a drag around it) is the address, without
/// the row breaks and the indent (the fork's 577828e, b6042c4). It was
/// the row under the finger, and a drag copied the rows apart.
#[test]
fn v15_a_link_a_program_wrapped_itself_selects_and_copies_whole() {
    let url = format!("https://example.com/{}/end", "a".repeat(40));
    let (first, rest) = url.split_at(38);
    let mut t = term(40, 6, 100);
    t.feed(
        format!(
            "  {}\r\n  {}\r\n$ ",
            osc8(&url, "", first),
            osc8(&url, "", rest)
        )
        .as_bytes(),
    );
    let (uri, runs) = t.link_at(0, 10).unwrap();
    assert_eq!(uri, url);
    assert_eq!(
        runs,
        vec![(0, 2, 39), (1, 2, 27)],
        "both rows, not the indent"
    );
    assert_eq!(t.link_at(1, 10).unwrap().1, runs);
    for row in [0, 1] {
        t.select_start(SelectKind::Semantic, row, 10, false);
        assert_eq!(
            t.selection_text(),
            Some(url.clone()),
            "long press on row {row}"
        );
    }
    // A drag from the indent to the blanks after it.
    t.select_start(SelectKind::Cells, 0, 0, false);
    t.select_extend(1, 39, true);
    assert_eq!(
        t.selection_text(),
        Some(url.clone()),
        "a drag around the link"
    );

    // After a tmux repaint one row carries tmux's id: the URL decides.
    let mut t = term(40, 6, 100);
    t.feed(
        format!(
            "  {}\r\n  {}\r\n$ ",
            osc8(&url, "id=128g0ok", first),
            osc8(&url, "", rest)
        )
        .as_bytes(),
    );
    assert_eq!(t.link_at(1, 3).unwrap().1, vec![(0, 2, 39), (1, 2, 27)]);
}

/// The same address on consecutive lines that don't continue each other,
/// and a wrapped label that is not the address, stay as they read.
#[test]
fn v15_links_that_only_look_continued_stay_apart() {
    let url = "https://example.com/x";
    let mut t = term(40, 6, 100);
    t.feed(format!("- {}\r\n- {}\r\n", osc8(url, "", url), osc8(url, "", url)).as_bytes());
    assert_eq!(
        t.link_at(1, 4).unwrap().1,
        vec![(1, 2, 22)],
        "a list of the same link"
    );
    let mut t = term(12, 6, 100);
    let label = format!(
        "{}\r\n{}",
        osc8(url, "", "the project"),
        osc8(url, "", "docs page")
    );
    t.feed(label.as_bytes());
    t.select_start(SelectKind::Cells, 0, 0, false);
    t.select_extend(1, 8, true);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("the project\ndocs page"),
        "a label keeps its rows"
    );
}

/// B423 (b): a bare URL a program broke at the right edge itself goes on
/// over the address-like run that starts the next row (after its indent):
/// one link, its tail drawn and tapped as it, a long press copies the
/// whole address (`PlainLinkDetector.hardWrapContinuation`). The tap
/// opened the first row's fragment; the tail was no link.
#[test]
fn v34_a_url_a_program_wrapped_itself_is_one_link() {
    let url = "https://example.com/docx/ZqLmOFywpjhAb3dEfGh";
    let (first, rest) = url.split_at(38);
    let mut t = term(40, 6, 100);
    t.feed(format!("  {first}\r\n  {rest} done\r\n$ ").as_bytes());
    assert_eq!(t.link_at(0, 10).unwrap().0, url);
    let (tail, runs) = t.link_at(1, 3).unwrap();
    assert_eq!(tail, url, "the tail is the same link");
    assert_eq!(runs, vec![(0, 2, 39), (1, 2, 7)]);
    assert!(t.link_at(1, 10).is_none(), "the word after it is prose");
    let f = t.frame();
    let linked: Vec<usize> = f
        .changed
        .iter()
        .find(|r| r.row == 1)
        .unwrap()
        .cells
        .iter()
        .enumerate()
        .filter(|(_, c)| c.flags & F::LINK != 0)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        linked,
        (2..8).collect::<Vec<_>>(),
        "the tail draws as a link"
    );
    t.select_start(SelectKind::Semantic, 1, 3, false);
    assert_eq!(t.selection_text().as_deref(), Some(url));

    // A complete URL that happens to end at the edge, then prose: as is.
    let whole = "https://example.com/abcdefghijklmnopqr";
    let mut t = term(40, 6, 100);
    t.feed(format!("  {whole}\r\n  and then more\r\n").as_bytes());
    assert_eq!(t.link_at(0, 10).unwrap().0, whole);
    assert!(t.link_at(1, 3).is_none());
}
