//! The terminal view's requirements that are logic, not UIKit
//! (docs/nsh/TERMINAL-VIEW.md, the requirements table): each test is named
//! after its row. The UIKit glue has its own device tests
//! (CoreTerminalViewTests).

mod common;
use common::*;
use moshpit_term::{
    Announcement, Ime, ScrollAction, SelectKind, Tap, Typed, cell_flags as F, ime::Outcome,
};

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

/// B690: a period the person types after deleting a space is a period. The
/// keyboard's double-space deletes the space and inserts ". " in the same
/// turn of the event loop; a person who deletes a space and types "."
/// later (`example .com` mended to `example.com`) got a space instead:
/// the delete left the auto-period armed until the next insertion.
#[test]
fn b690_a_period_typed_after_a_deleted_space_is_a_period() {
    let mut ime = focused();
    ime.insert("example ");
    assert_eq!(typed(&ime.delete_backward()), "⌫");
    ime.turn_end();
    assert_eq!(typed(&ime.insert(".")), ".", "the person's period");
    assert_eq!(typed(&ime.insert("com")), "com");
    // Full-width punctuation is the person's whatever came before it.
    let mut ime = focused();
    ime.insert("你好 ");
    ime.delete_backward();
    assert_eq!(typed(&ime.insert("。")), "。");
    // The keyboard's own double-space, in one turn, still sends spaces.
    let mut ime = focused();
    ime.insert("ls ");
    ime.delete_backward();
    assert_eq!(typed(&ime.insert(". ")), "  ");
    ime.turn_end();
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

/// B478: a long-pressed link stays whole while the finger rests on it (a
/// resting finger reports small moves: each one extended the selection to
/// the cell under it, so the link shrank to "https:"); sliding past either
/// end grows the selection from the link, keeping all of it.
#[test]
fn v15_a_long_pressed_link_stays_whole_under_the_finger() {
    let mut t = term(40, 4, 10);
    t.feed(b"see https://example.com/abc/def ok then");
    t.select_start(SelectKind::Semantic, 0, 10, false);
    let link = Some("https://example.com/abc/def".to_string());
    assert_eq!(t.selection_text(), link);
    for col in [10, 11, 4, 29] {
        t.select_extend(0, col, false);
        assert_eq!(t.selection_text(), link, "the finger at column {col}");
    }
    t.select_extend(0, 34, false);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("https://example.com/abc/def ok"),
        "past its end: from the link's start"
    );
    t.select_extend(0, 0, false);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("see https://example.com/abc/def"),
        "before its start: to the link's end"
    );
}

/// B480: a repaint that writes the same text (tmux's repair frame: each
/// row erased and written again, at the keyboard going up or down, the
/// return, a switch) leaves a selection standing; text that changes under
/// it still ends it (B79, B100).
#[test]
fn v15_a_selection_stands_through_a_repaint_of_the_same_text() {
    let mut t = term(20, 4, 100);
    t.feed(b"alpha beta\r\ngamma\r\n$ ");
    t.select_start(SelectKind::Word, 0, 7, false);
    assert_eq!(t.selection_text().as_deref(), Some("beta"));
    let repaint =
        b"\x1b[H\x1b[0malpha beta\x1b[0m\x1b[K\r\ngamma\x1b[0m\x1b[K\r\n$ \x1b[0m\x1b[K\x1b[J";
    t.feed(repaint);
    let _ = t.frame();
    assert_eq!(t.selection_text().as_deref(), Some("beta"), "the same text");
    t.feed(repaint);
    t.select_extend(1, 4, true);
    assert_eq!(t.selection_text().as_deref(), Some("beta\ngamma"));
    t.feed(repaint);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("beta\ngamma"),
        "extended, then repainted"
    );
    t.feed(b"\x1b[H\x1b[0malpha bets\x1b[0m\x1b[K");
    assert_eq!(t.selection_text(), None, "changed text ends it");
}

/// B484: text changing under one row of a selection over several ends it,
/// and no row stays painted selected. The selection was let go while the
/// frame was being drawn, after it had picked its rows, and the repaint it
/// asked for was forgotten at the frame's end: only the changed row came
/// out unselected, the others stayed painted selected (over nothing a tap
/// or a copy could reach) until something else redrew them.
#[test]
fn v15_a_selection_ended_by_new_text_leaves_no_row_painted_selected() {
    let mut t = term(20, 4, 100);
    t.feed(b"alpha\r\nbeta\r\ngamma\r\n$ ");
    t.select_start(SelectKind::Cells, 0, 0, false);
    t.select_extend(2, 4, true);
    assert_eq!(t.selection_text().as_deref(), Some("alpha\nbeta\ngamma"));
    // What the view shows: each row painted selected or not, frame by frame.
    let mut shown = [false; 4];
    let mut paint = |f: &moshpit_term::Frame| {
        for r in &f.changed {
            shown[r.row as usize] = r.cells.iter().any(|c| c.flags & F::SELECTED != 0);
        }
        shown
    };
    assert_eq!(paint(&t.frame()), [true, true, true, false]);
    // Only the middle row changes; the cursor goes back to the prompt.
    t.feed(b"\x1b[2;1Hbeto\x1b[4;3H");
    let first = paint(&t.frame());
    assert_eq!(t.selection_text(), None, "changed text ends it");
    assert_eq!(first, [false; 4], "no row painted selected");
    assert_eq!(paint(&t.frame()), [false; 4], "nor after");
}

/// B477: a handle drags with the selection's other end off the screen
/// (output scrolled the selected rows up, a long-pressed link starts above
/// the view): the selection keeps that end where it is, wherever it is,
/// and the dragged end follows the finger. The view re-anchored on the
/// other end through the viewport, and refused one outside it: the handle
/// didn't move.
#[test]
fn v16_a_handle_drags_with_the_other_end_off_screen() {
    let mut t = term(20, 4, 100);
    t.feed(b"first line\r\nsecond\r\n");
    t.select_start(SelectKind::Word, 0, 1, false);
    assert_eq!(t.selection_text().as_deref(), Some("first"));
    // Output scrolls it up, out of the view (which follows the output).
    t.feed(b"a\r\nb\r\nc\r\nd\r\n$ ");
    let ((start_row, _), _) = t.selection_bounds().unwrap();
    assert!(start_row < 0, "the selection's start is above the view");
    // The end handle: the start stays, the end follows the finger.
    assert!(t.select_keep_end(false));
    t.select_extend(0, 0, true);
    assert_eq!(
        t.selection_text().as_deref(),
        Some("first line\nsecond\na\nb"),
        "from the start off screen to the finger"
    );
    // The start handle of a selection whose end is below the view.
    t.scroll(4);
    assert!(t.select_keep_end(true));
    t.select_extend(0, 6, false);
    assert!(
        t.selection_text().unwrap().ends_with("\nb"),
        "the end below the view stays"
    );
    assert!(
        !{
            t.select_clear();
            t.select_keep_end(true)
        },
        "nothing to keep"
    );
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

/// B425: the width changes while reading history (the phone turns, a
/// pinch sets another font size, an iPad window is resized): the text
/// reflows, and the view stays on the line it was reading. It kept a row
/// number the reflow gave to other text: narrower threw the reader far
/// back, mid-line; wider dropped it on the live screen.
#[test]
fn v21_a_resize_while_reading_keeps_the_line_read() {
    let feed = |t: &mut moshpit_term::Terminal| {
        for i in 0..200 {
            t.feed(format!("L{i:03} {}\r\n", "x".repeat(30)).as_bytes());
        }
    };
    let top = |t: &moshpit_term::Terminal| t.view_row_text(0).0.chars().take(4).collect::<String>();
    let mut t = term(40, 10, 1000);
    t.set_cell_pixels(10, 16);
    feed(&mut t);
    t.scroll(50);
    assert_eq!(top(&t), "L141");
    t.resize(20, 10);
    assert_eq!(top(&t), "L141", "narrower: the same line, from its start");
    t.resize(40, 10);
    assert_eq!(top(&t), "L141", "wider again");
    t.resize(40, 6);
    assert_eq!(top(&t), "L141", "the keyboard (rows only)");
    assert!(t.display_offset() > 0, "still reading");
    t.feed(b"more\r\n");
    assert_eq!(
        top(&t),
        "L141",
        "output after it leaves the view where it is"
    );

    // From the second row of a line: wider, that line's start.
    let mut t = term(20, 10, 1000);
    t.set_cell_pixels(10, 16);
    feed(&mut t);
    t.scroll(60);
    if t.view_row_text(0).0.starts_with('L') {
        t.scroll(1);
    }
    let off = t.display_offset() as i32;
    let (above, _) = t.row_text(-off - 1);
    assert!(
        !t.view_row_text(0).0.starts_with('L'),
        "a line's second row"
    );
    t.resize(40, 10);
    assert_eq!(top(&t), above[..4], "wider: the line it was in");
    assert!(t.display_offset() > 0, "not the live screen");

    // A live view stays live.
    let mut t = term(40, 10, 1000);
    feed(&mut t);
    t.resize(20, 10);
    assert_eq!(t.display_offset(), 0);
}

/// B479: a new font size while reading history (a pinch): the cells take
/// new pixels, and the reader stays on its rows, drawn on whole rows of the
/// new size. The pixel offset stayed the old size's: the frames drew the
/// rows shifted by a part of a row.
#[test]
fn v21_new_cell_pixels_while_reading_keep_whole_rows() {
    let mut t = tall(50);
    t.scroll_drag(45.0, false);
    let f = t.frame();
    assert!(
        f.display_offset > 0 && f.pixel_shift > 0,
        "a row and a half up"
    );
    let top = t.view_row_text(0);
    t.set_cell_pixels(12, 40);
    let f = t.frame();
    assert_eq!(t.view_row_text(0), top, "the same rows");
    assert_eq!(f.pixel_shift, 0, "on whole rows of the new size");
    assert!(f.display_offset > 0, "still reading");
}

/// B500 (xterm's alternate scroll, the owner's call): a swipe over an
/// alternate-screen program without the mouse (less, man, git log) sends
/// it the arrow keys, by its cursor-key mode, at most six a move, and a
/// coast sends them paced as a wheel's reports are. It moved nothing: the
/// alternate screen keeps no history. With the mouse on it is still the
/// wheel; on the main screen, the history.
#[test]
fn v20_a_swipe_over_a_pager_is_its_arrow_keys() {
    let mut t = tall(50);
    t.feed(b"\x1b[?1049h");
    let _ = t.frame();
    assert_eq!(
        t.scroll_drag(90.0, false),
        ScrollAction::Wheel(b"\x1b[A\x1b[A\x1b[A".to_vec()),
        "older is up"
    );
    assert_eq!(
        t.scroll_drag(-30.0, false),
        ScrollAction::Wheel(b"\x1b[B".to_vec())
    );
    assert_eq!(t.scroll_offset_px(), 0, "no history moved");
    // A pager in application cursor mode.
    t.feed(b"\x1b[?1h");
    assert_eq!(
        t.scroll_drag(30.0, false),
        ScrollAction::Wheel(b"\x1bOA".to_vec())
    );
    let ScrollAction::Wheel(bytes) = t.scroll_drag(3_000.0, false) else {
        panic!("arrows")
    };
    assert_eq!(bytes, b"\x1bOA".repeat(6), "six a move at most");
    // A coast: paced as a wheel, at most six presses a step.
    let mut now = 0;
    for _ in 0..5 {
        now += 10;
        t.fling_track_at(now, 100.0);
    }
    assert!(t.fling_start_at(now, false));
    let mut presses = Vec::new();
    while let Some(dy) = t.fling_step(1.0 / 60.0) {
        if let ScrollAction::Wheel(bytes) = t.scroll_drag(dy, false) {
            presses.push(bytes.len() / 3);
        }
        assert!(presses.len() < 1_000);
    }
    assert!(presses.iter().all(|n| (1..=6).contains(n)), "{presses:?}");
    assert!(presses.iter().sum::<usize>() <= 150);
    // The mouse on: its wheel.
    t.feed(b"\x1b[?1000h\x1b[?1006h");
    let ScrollAction::Wheel(bytes) = t.scroll_drag(30.0, false) else {
        panic!("the wheel")
    };
    assert!(bytes.starts_with(b"\x1b[<64;"), "{bytes:?}");
    // The main screen again: its history.
    t.feed(b"\x1b[?1000l\x1b[?1049l");
    let _ = t.frame();
    assert!(matches!(
        t.scroll_drag(30.0, false),
        ScrollAction::Local { .. }
    ));
}

/// A drag of `px` pixels in moves of `step` every 10 ms from `at`, routed
/// as the view routes it and tracked for the lift; when it ended.
fn drag_for_a_fling(t: &mut moshpit_term::Terminal, at: u64, px: f64, step: f64) -> u64 {
    let mut now = at;
    let mut left = px;
    while left.abs() > 0.0 {
        let dy = if left.abs() < step.abs() { left } else { step };
        now += 10;
        t.fling_track_at(now, dy);
        let _ = t.scroll_drag(dy, false);
        left -= dy;
    }
    now
}

/// Plays a coast through the view's routing, 60 Hz: the history's moved
/// pixels, and the wheel reports per refresh.
fn coast(t: &mut moshpit_term::Terminal) -> (i64, Vec<usize>) {
    let (mut moved, mut ticks) = (0, Vec::new());
    for _ in 0..2_000 {
        let Some(dy) = t.fling_step(1.0 / 60.0) else {
            return (moved, ticks);
        };
        match t.scroll_drag(dy, false) {
            ScrollAction::Local { moved_px } => moved += moved_px,
            ScrollAction::Wheel(bytes) => ticks.push(bytes.iter().filter(|b| **b == b'M').count()),
            _ => {}
        }
    }
    panic!("the coast never stopped");
}

/// B521 (V22 overturned, the owner's call): a lift coasts at the drag's
/// speed, slowing as UIScrollView slows, and stops; a finger that rested
/// before the lift doesn't coast; it stops at the history's top and on the
/// live screen. It didn't coast at all unless the host turned coasting on,
/// which it never did.
#[test]
fn v22_a_lift_coasts_at_the_drags_speed() {
    let mut t = tall(500);
    // 30 px every 10 ms: 3000 px/s.
    let lift = drag_for_a_fling(&mut t, 0, 300.0, 30.0);
    let before = t.scroll_offset_px();
    assert!(t.fling_start_at(lift, false), "it coasts");
    let (moved, _) = coast(&mut t);
    assert!(
        (1_400..=1_600).contains(&moved),
        "about UIScrollView's 1490 px for 3000 px/s: {moved}"
    );
    assert_eq!(t.scroll_offset_px(), before + moved);
    assert!(!t.flinging(), "and stopped");

    // Let go at rest: no coast.
    let lift = drag_for_a_fling(&mut t, 10_000, 300.0, 30.0);
    assert!(!t.fling_start_at(lift + 80, false), "the finger rested");
    assert!(!t.flinging());

    // Toward the top of a short history: it stops there.
    let mut t = tall(30);
    let lift = drag_for_a_fling(&mut t, 0, 120.0, 40.0);
    assert!(t.fling_start_at(lift, false));
    let (moved, _) = coast(&mut t);
    assert!(moved > 0);
    assert_eq!(t.scroll_drag(10.0, false), ScrollAction::None, "at the top");

    // Toward the live screen: it lands there and stops.
    let lift = drag_for_a_fling(&mut t, 20_000, -120.0, -40.0);
    assert!(t.fling_start_at(lift, false));
    let _ = coast(&mut t);
    assert_eq!(t.scroll_offset_px(), 0, "on the live screen");
}

/// B521: a touch stops a coast (and says so: that touch is not a tap);
/// typing stops it, as a resize does.
#[test]
fn v22_a_touch_or_typing_stops_a_coast() {
    let mut t = tall(500);
    let lift = drag_for_a_fling(&mut t, 0, 300.0, 30.0);
    assert!(t.fling_start_at(lift, false));
    assert!(t.fling_step(1.0 / 60.0).is_some());
    assert!(t.fling_stop(), "a coast was stopped: not a tap");
    assert!(!t.fling_stop(), "nothing to stop: a tap");
    assert_eq!(t.fling_step(1.0 / 60.0), None);

    let lift = drag_for_a_fling(&mut t, 1_000, 300.0, 30.0);
    assert!(t.fling_start_at(lift, false));
    let _ = t.key(moshpit_term::Key::Char('a'), 0, true, 1);
    assert!(!t.flinging(), "typing stops it");

    let lift = drag_for_a_fling(&mut t, 2_000, 300.0, 30.0);
    assert!(t.fling_start_at(lift, false));
    t.resize(30, 6);
    assert!(!t.flinging(), "a resize stops it");
}

/// B521, B523: a coast for a program that wants the mouse goes as wheel
/// reports, paced (every other refresh at most) and bounded: each step and
/// the whole coast what its speed travels, at most six reports a step and
/// 150 in all, however fast the fling (each report is work for the
/// program, and a flood is what it falls behind on). The host's rows (tmux,
/// herdr) are paced the same, with room for more: a step is one command
/// whatever its rows. It was three a step and sixty in all at any speed:
/// a faster fling went no faster.
#[test]
fn v22_a_coast_to_a_wheel_or_the_host_is_paced_and_bounded() {
    let mut t = tall(50);
    t.feed(b"\x1b[?1000h\x1b[?1006h");
    let _ = t.frame();
    // A very fast fling: 100 px every 10 ms, 10000 px/s.
    let mut now = 0;
    for _ in 0..5 {
        now += 10;
        t.fling_track_at(now, 100.0);
    }
    assert!(t.fling_start_at(now, false));
    let (moved, ticks) = coast(&mut t);
    assert_eq!(moved, 0, "the history stays: the program scrolls");
    assert!(ticks.iter().all(|n| (1..=6).contains(n)), "{ticks:?}");
    assert!(ticks.contains(&6), "at the ceiling: {ticks:?}");
    // Past six a step what the step couldn't take was dropped: some 145 of
    // the 166 rows the speed travels, never past 150.
    let all = ticks.iter().sum::<usize>();
    assert!((120..=150).contains(&all), "{all} {ticks:?}");

    // The host's rows: by the row, below its ceilings as far and as fast as
    // the speed goes (10000 px/s of 30-px rows: some 166 rows, 11 a step).
    let rows = host_coast(&mut tall(50), &[100.0]).remove(0);
    assert!(rows.iter().all(|n| (1..=12).contains(n)), "{rows:?}");
    let all: i32 = rows.iter().sum();
    assert!((160..=167).contains(&all), "{all} rows: {rows:?}");
}

/// Flicks of `px` pixels a move (5 moves 10 ms apart), each lifted, its
/// coast run 150 ms, then stopped by the next flick's touch; the last one's
/// coast runs out. Every coast's travel goes through the view's routing
/// (`host`: the host's rows). The flicks' speeds, the travel in all, the
/// most a refresh moved.
fn flicks(t: &mut moshpit_term::Terminal, px: &[f64], host: bool) -> (Vec<f64>, f64, f64) {
    let (mut speeds, mut travel, mut peak) = (Vec::new(), 0.0, 0.0f64);
    let mut now = 1_000;
    for (i, step) in px.iter().enumerate() {
        for _ in 0..5 {
            now += 10;
            t.fling_track_at(now, *step);
            let _ = t.scroll_drag(*step, host);
        }
        assert!(t.fling_start_at(now, host), "flick {i} coasts");
        speeds.push(t.fling_velocity().unwrap());
        let last = i + 1 == px.len();
        let frames = if last { 10_000 } else { 9 };
        for _ in 0..frames {
            let Some(dy) = t.fling_step(1.0 / 60.0) else {
                break;
            };
            travel += dy;
            peak = peak.max(dy.abs());
            let _ = t.scroll_drag(dy, host);
        }
        now += 150;
        if !last {
            assert!(t.fling_stop_at(now), "the next touch stops the coast");
        }
    }
    (speeds, travel, peak)
}

/// The host's rows each coast of `px`-pixel flicks sends, step by step.
fn host_coast(t: &mut moshpit_term::Terminal, px: &[f64]) -> Vec<Vec<i32>> {
    let mut coasts = Vec::new();
    let mut now = 1_000;
    for (i, step) in px.iter().enumerate() {
        for _ in 0..5 {
            now += 10;
            t.fling_track_at(now, *step);
        }
        assert!(t.fling_start_at(now, true));
        let last = i + 1 == px.len();
        let mut rows = Vec::new();
        for _ in 0..(if last { 10_000 } else { 9 }) {
            let Some(dy) = t.fling_step(1.0 / 60.0) else {
                break;
            };
            if let ScrollAction::Host { rows: n } = t.scroll_drag(dy, true) {
                rows.push(n);
            }
        }
        coasts.push(rows);
        now += 150;
        if !last {
            assert!(t.fling_stop_at(now));
        }
    }
    coasts
}

/// B523 (the user, on Canary 131: quick flicks one after another didn't
/// add up): a flick the same way as the coast its touch stopped goes at
/// both speeds added, as UIScrollView's do, so two and three quick flicks
/// go faster and further than one. It started from its own speed again.
#[test]
fn v22_quick_flicks_the_same_way_stack() {
    let (one, far, fast) = flicks(&mut tall(5_000), &[30.0], false);
    let (two, farther, faster) = flicks(&mut tall(5_000), &[30.0, 30.0], false);
    let (three, farthest, fastest) = flicks(&mut tall(5_000), &[30.0, 30.0, 30.0], false);
    assert!((one[0] - 3_000.0).abs() < 1.0, "{one:?}");
    // The second carries what was left of the first: 3000 px/s after
    // 150 ms of 0.998 a ms, some 2220.
    assert!((4_900.0..5_400.0).contains(&two[1]), "{two:?}");
    assert!(three[2] > two[1], "{three:?}");
    assert!(
        farther > far * 1.5 && farthest > farther,
        "{far} {farther} {farthest}"
    );
    assert!(
        faster > fast && fastest > faster,
        "{fast} {faster} {fastest}"
    );
}

/// B523: only a quick flick the same way stacks. The other way starts
/// from its own speed; a finger that rested lets go at rest; a slow drag
/// or a lift long after the touch carries nothing; a tap that only stopped
/// the coast leaves nothing for later.
#[test]
fn v22_only_a_quick_flick_the_same_way_stacks() {
    // The other way.
    let (speeds, _, _) = flicks(&mut tall(5_000), &[30.0, -30.0], false);
    assert!((speeds[1] + 3_000.0).abs() < 1.0, "{speeds:?}");
    // Slow: 2 px every 10 ms, 200 px/s, coasts at its own speed.
    let (speeds, _, _) = flicks(&mut tall(5_000), &[30.0, 2.0], false);
    assert!((speeds[1] - 200.0).abs() < 1.0, "{speeds:?}");

    // At rest before the lift: no coast, nothing carried.
    let mut t = tall(5_000);
    let lift = drag_for_a_fling(&mut t, 0, 150.0, 30.0);
    assert!(t.fling_start_at(lift, false));
    assert!(t.fling_stop_at(lift + 100));
    let lift = drag_for_a_fling(&mut t, lift + 100, 150.0, 30.0);
    assert!(!t.fling_start_at(lift + 80, false), "rested");

    // Lifted long after the touch: its own speed.
    let mut t = tall(5_000);
    let lift = drag_for_a_fling(&mut t, 0, 150.0, 30.0);
    assert!(t.fling_start_at(lift, false));
    assert!(t.fling_stop_at(lift + 100));
    let lift = drag_for_a_fling(&mut t, lift + 500, 150.0, 30.0);
    assert!(t.fling_start_at(lift, false));
    assert!((t.fling_velocity().unwrap() - 3_000.0).abs() < 1.0);
}

/// B523: on the host's route a stacked coast sends more rows a step and
/// more in all, up to the ceilings (12 a step, 400 in all), each step one
/// command; what a step can't take is dropped, never owed.
#[test]
fn v22_a_stacked_coast_to_the_host_grows_up_to_its_ceilings() {
    // The last coast of each: what the stacked speed sends.
    let one = host_coast(&mut tall(50), &[30.0]).pop().unwrap();
    let three = host_coast(&mut tall(50), &[30.0, 30.0, 30.0])
        .pop()
        .unwrap();
    let many = host_coast(&mut tall(50), &[200.0; 6]);
    let sizes = |r: &[i32]| (r.iter().copied().max().unwrap_or(0), r.iter().sum::<i32>());
    let ((top1, all1), (top3, all3)) = (sizes(&one), sizes(&three));
    assert!(top3 > top1 && all3 > all1, "{one:?} {three:?}");
    assert!(
        many.iter().flatten().all(|n| *n <= 12),
        "never past 12 a step: {many:?}"
    );
    let (top_many, all_many) = sizes(many.last().unwrap());
    assert_eq!(top_many, 12, "at the step's ceiling: {many:?}");
    assert!(
        all_many > all3 && all_many <= 400,
        "further, never past 400: {all_many}"
    );
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
    assert_eq!(t.paste("a\nb", false), b"a\rb", "a line break is Return (B574)");
    t.feed(b"\x1b[?2004h");
    assert_eq!(t.paste("a\x1b[201~b", false), b"\x1b[200~ab\x1b[201~");
    t.feed(b"\x1b[?2004l");
    assert_eq!(
        t.paste("x", true),
        b"\x1b[200~x\x1b[201~",
        "the host forces it (herdr)"
    );
}

/// B574: a paste the program didn't ask to have bracketed is typed: its
/// line breaks go out as Return, as xterm, iTerm and Terminal.app send
/// them — CRLF and a lone LF as CR. Sent as LF, a CRLF text from elsewhere
/// doubled every line break in a program that reads raw (`cat > file`
/// through the tty's ICRNL, an old REPL). A bracketed paste is the
/// program's to read: byte for byte.
#[test]
fn b574_a_plain_paste_sends_its_line_breaks_as_return() {
    let mut t = term(20, 4, 10);
    assert_eq!(t.paste("a\r\nb\nc\rd", false), b"a\rb\rc\rd");
    assert_eq!(t.paste("\r\n\r\n", false), b"\r\r", "a blank line stays one");
    assert_eq!(t.paste("no breaks", false), b"no breaks");
    t.feed(b"\x1b[?2004h");
    assert_eq!(
        t.paste("a\r\nb\nc", false),
        b"\x1b[200~a\r\nb\nc\x1b[201~",
        "bracketed: as it is"
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

/// B427: VoiceOver hears what appears at most once a second, and what
/// appears in that second is said at its end, in one; only what was said
/// starts the quiet. The view's own gate dropped output that ended within
/// a second of the last announcement until the next, unrelated change,
/// and armed itself on frames with nothing to say.
#[test]
fn v33_announcements_are_paced_and_the_last_is_said() {
    let mut t = term(20, 6, 100);
    t.feed(b"first\r\n$ ");
    assert_eq!(t.announce_at(0), Announcement::default(), "the baseline");
    t.feed(b"ls\r\nfile.txt\r\n$ ");
    assert_eq!(t.announce_at(100).text.as_deref(), Some("$ ls\nfile.txt"));
    // The command's last lines, within the second: held, and when to ask.
    t.feed(b"cat x\r\nlast line\r\n$ ");
    let held = t.announce_at(300);
    assert_eq!(
        held,
        Announcement {
            text: None,
            again_in_ms: Some(800)
        }
    );
    t.feed(b"more\r\n$ ");
    assert_eq!(t.announce_at(600).again_in_ms, Some(500));
    // The second is over: everything since, at once.
    assert_eq!(
        t.announce_at(1_100).text.as_deref(),
        Some("$ cat x\nlast line\n$ more"),
        "the last output was never said"
    );
    // Nothing to say starts no quiet: the next output is said at once.
    t.feed(b"c");
    assert_eq!(t.announce_at(2_200), Announcement::default());
    t.feed(b"\r\nnew\r\n$ ");
    assert_eq!(
        t.announce_at(2_300).text.as_deref(),
        Some("$ c\nnew"),
        "the line typed, as echoed, and its output"
    );
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
