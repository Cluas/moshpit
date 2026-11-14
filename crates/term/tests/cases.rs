//! vttest-style screen cases: xterm's behaviour, tmux 3.6a's character
//! widths, and every engine-level bug of the SwiftTerm fork and of Termux
//! that a screen can show (docs/nsh/PROGRESS.md rows; the T1 spike ran the
//! same cases on alacritty_terminal, wezterm-term and the fork,
//! spike/term-core/src/cases.rs). Each case is fed in one read and again
//! one byte per read: the screen must not depend on how the network cut
//! the bytes.

mod common;
use common::*;
use moshpit_term::{Event, Terminal};

/// Feeds `parts` whole and byte by byte; both must pass `check`.
fn both(cols: u16, rows: u16, scrollback: usize, parts: &[&[u8]], check: impl Fn(&mut Terminal)) {
    let mut whole = term(cols, rows, scrollback);
    for p in parts {
        whole.feed(p);
    }
    check(&mut whole);
    let mut bytewise = term(cols, rows, scrollback);
    for p in parts {
        for b in p.iter() {
            bytewise.feed(std::slice::from_ref(b));
        }
    }
    check(&mut bytewise);
    assert_eq!(
        state(&whole),
        state(&bytewise),
        "whole and byte-by-byte differ"
    );
}

fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("{i}\r\n")).collect()
}

#[test]
fn autowrap_marks_the_row() {
    both(10, 4, 100, &[b"0123456789AB"], |t| {
        assert_eq!(screen(t)[0], ("0123456789".into(), true));
        assert_eq!(rows(t)[1], "AB");
        assert_eq!(t.cursor(), (2, 1));
    });
}

#[test]
fn pending_wrap_then_cr_stays_on_the_row() {
    both(10, 4, 100, &[b"0123456789\rX"], |t| {
        assert_eq!(screen(t)[0], ("X123456789".into(), false));
        assert_eq!(t.cursor(), (1, 0));
    });
}

#[test]
fn b177_autowrap_with_history_marks_the_screen_row() {
    both(5, 3, 100, &[lines(6).as_bytes(), b"\x1b[HABCDEFG"], |t| {
        assert_eq!(screen(t)[0], ("ABCDE".into(), true));
        assert_eq!(rows(t)[1], "FG");
        assert!(history(t).iter().all(|(_, w)| !w), "{:?}", history(t));
    });
}

#[test]
fn b78_ed1_at_the_last_column_keeps_the_history_marks() {
    both(
        5,
        3,
        100,
        &[b"ABCDEFGHIJ\r\n1\r\n2\r\n3", b"\x1b[1;5H\x1b[1J"],
        |t| {
            assert_eq!(history(t)[0], ("ABCDE".into(), true));
            assert_eq!(rows(t)[0], "");
        },
    );
}

#[test]
fn b184_el_cuts_the_wrap() {
    both(10, 4, 100, &[b"0123456789AB\x1b[1;3H\x1b[K"], |t| {
        assert_eq!(screen(t)[0], ("01".into(), false));
    });
}

#[test]
fn b213_lines_reused_from_a_full_history_are_unwrapped() {
    let long = "ABCDEFGHIJ\r\n".repeat(10);
    let short = "x\r\n".repeat(10);
    both(5, 3, 5, &[long.as_bytes(), short.as_bytes()], |t| {
        assert_eq!(t.history_len(), 5);
        assert!(history(t).iter().all(|(_, w)| !w), "{:?}", history(t));
    });
}

#[test]
fn a_wide_character_that_does_not_fit_wraps_whole() {
    both(5, 3, 100, &["abcd中".as_bytes()], |t| {
        assert_eq!(screen(t)[0], ("abcd".into(), true));
        assert_eq!(rows(t)[1], "中");
        assert_eq!(t.cursor(), (2, 1));
    });
}

#[test]
fn erase_insert_delete() {
    for (input, want) in [
        (&b"abcdef\x1b[1;2H\x1b[2X"[..], "a  def"),
        (b"a\x1b[3b", "aaaa"),
        (b"abcdef\x1b[1;2H\x1b[2@", "a  bcdef"),
        (b"abcdef\x1b[1;2H\x1b[2P", "adef"),
        (b"abc\x1b[1;1H\x1b[4hX", "Xabc"),
    ] {
        both(10, 4, 100, &[input], |t| {
            assert_eq!(rows(t)[0], want, "{input:?}")
        });
    }
}

#[test]
fn patch18_writing_over_either_half_of_a_wide_character() {
    both(10, 4, 100, &["中\x1b[1;2Hx".as_bytes()], |t| {
        assert_eq!(rows(t)[0], " x")
    });
    both(10, 4, 100, &["中\x1b[1;1Hx".as_bytes()], |t| {
        assert_eq!(rows(t)[0], "x")
    });
}

#[test]
fn tabs_origin_saved_cursor_regions() {
    both(20, 4, 100, &[b"\tX"], |t| assert_eq!(t.cursor(), (9, 0)));
    both(20, 4, 100, &[b"\x1b[3g\x1b[1;4H\x1bH\r\tY"], |t| {
        assert_eq!(t.cursor(), (4, 0))
    });
    both(10, 5, 100, &[b"\x1b[2;3r\x1b[?6h\x1b[1;1HZ"], |t| {
        assert_eq!(rows(t)[1], "Z")
    });
    both(10, 5, 100, &[b"\x1b[2;3H\x1b7\x1b[4;1H\x1b8X"], |t| {
        assert_eq!(rows(t)[1], "  X")
    });
    both(
        10,
        4,
        100,
        &[b"1\r\n2\r\n3\r\n4\x1b[2;3r\x1b[2;1H\x1bM"],
        |t| assert_eq!(rows(t), ["1", "", "2", "4"]),
    );
}

#[test]
fn b218_wrap_and_ind_below_the_region_move_down() {
    both(
        5,
        5,
        100,
        &[b"11111\r\n22222\x1b[1;2r\x1b[3;1HABCDEFG"],
        |t| assert_eq!(&rows(t)[..4], ["11111", "22222", "ABCDE", "FG"]),
    );
    both(
        5,
        5,
        100,
        &[b"11111\r\n22222\x1b[1;2r\x1b[3;1Hx\x1bDy"],
        |t| assert_eq!(&rows(t)[..4], ["11111", "22222", "x", " y"]),
    );
}

#[test]
fn alternate_screen_and_resets() {
    both(
        10,
        4,
        100,
        &[b"main\x1b[?1049h\x1b[2;2Halt\x1b[?1049l"],
        |t| {
            assert_eq!(rows(t)[0], "main");
            assert_eq!(t.cursor(), (4, 0));
            assert!(!t.modes().alt_screen);
        },
    );
    // B214: a first trip to the alternate screen leaves no history.
    both(10, 3, 100, &[b"a\r\nb\r\nc\x1b[?1049hx\x1b[?1049l"], |t| {
        assert_eq!(t.history_len(), 0);
        assert_eq!(rows(t)[2], "c");
    });
    // B99: a full reset from the alternate screen homes on the main one.
    both(10, 5, 100, &[b"\x1b[?1049h\x1b[4;4Hfoo\x1bc"], |t| {
        assert_eq!(t.cursor(), (0, 0));
        assert!(!t.modes().alt_screen);
    });
}

#[test]
fn b182_b217_deccolm_leaves_the_grid_and_the_screen_alone() {
    both(10, 4, 100, &[b"abc\x1b[?3h"], |t| {
        assert_eq!(t.size(), (10, 4));
        assert_eq!(rows(t)[0], "abc");
    });
    both(
        10,
        3,
        100,
        &[lines(6).as_bytes(), b"\x1b[?40h\x1b[?3h"],
        |t| {
            assert_eq!(t.size(), (10, 3));
            assert_eq!(t.history_len(), 4);
            assert_eq!(rows(t)[0], "5");
        },
    );
}

#[test]
fn ed3_drops_the_history_and_keeps_the_screen() {
    both(10, 3, 100, &[lines(6).as_bytes(), b"\x1b[3J"], |t| {
        assert_eq!(t.history_len(), 0);
        assert_eq!(rows(t)[0], "5");
    });
}

#[test]
fn the_history_keeps_its_limit() {
    both(10, 4, 100, &[lines(1000).as_bytes()], |t| {
        assert_eq!(t.history_len(), 100);
        assert_eq!(history(t)[0].0, "898");
    });
}

#[test]
fn b257_screen_title_is_taken_out_and_reported() {
    both(20, 4, 100, &[b"\x1bkvim\x1b\\hello"], |t| {
        assert_eq!(rows(t)[0], "hello");
        assert_eq!(t.cursor(), (5, 0));
        assert_eq!(t.title(), "vim");
    });
    let mut t = term(20, 4, 100);
    t.feed(b"\x1bkgit\x1b\\");
    assert_eq!(t.take_events(), vec![Event::Title("git".into())]);
}

#[test]
fn osc_titles_links_replies() {
    both(20, 4, 100, &[b"\x1b]2;my title\x07ok"], |t| {
        assert_eq!(t.title(), "my title")
    });
    both(
        20,
        4,
        100,
        &[b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ x"],
        |t| {
            let (uri, runs) = t.link_at(0, 2).unwrap();
            assert_eq!(uri, "https://example.com");
            assert_eq!(runs, vec![(0, 0, 3)]);
            assert!(t.link_at(0, 5).is_none());
        },
    );
    // Patch 22: one link wrapped over two rows is one link from either row.
    both(
        5,
        4,
        100,
        &[b"\x1b]8;;https://example.com/a\x1b\\abcdefgh\x1b]8;;\x1b\\"],
        |t| {
            assert_eq!(t.link_at(1, 1).unwrap().1, vec![(0, 0, 4), (1, 0, 2)]);
        },
    );
    for (ask, want) in [
        // DA1: a VT500 with sixel and OSC 52 (wezterm's answer).
        (&b"\x1b[c"[..], &b"\x1b[?65;4;6;18;22;52c"[..]),
        (b"\x1b[3;4H\x1b[6n", b"\x1b[3;4R"),
        (b"\x1b[>1u\x1b[?u", b"\x1b[?1u"),
    ] {
        let mut t = term(10, 4, 100);
        t.feed(ask);
        assert_eq!(t.take_replies(), want, "{ask:?}");
    }
    // Replies come in the order of their requests, in the same feed.
    let mut t = term(10, 4, 100);
    t.feed(b"\x1b[6n\x1b[2;2H\x1b[6n\x1b[5n");
    assert_eq!(t.take_replies(), b"\x1b[1;1R\x1b[2;2R\x1b[0n");
}

#[test]
fn strings_are_swallowed() {
    both(10, 4, 100, &[b"\x1b_Gf=100;AAAA\x1b\\X"], |t| {
        assert_eq!(rows(t)[0], "X")
    });
    both(10, 4, 100, &[b"\x1bP1$r0m\x1b\\X"], |t| {
        assert_eq!(rows(t)[0], "X")
    });
}

#[test]
fn widths_as_tmux_gives_them() {
    // Cursor column after the text, measured on tmux 3.6a.
    for (text, cursor) in [
        ("\u{1f468}\u{200d}\u{1f469} z", 4),
        ("\u{1f44d}\u{1f3fd}|", 3),
        ("\u{26a0}\u{fe0f}|", 3),
        ("\u{1f1e8}\u{1f1f3}|", 3),
        ("\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}x", 3),
        ("中文|", 5),
        ("e\u{301}|", 2),
    ] {
        both(20, 4, 100, &[text.as_bytes()], |t| {
            assert_eq!(t.cursor(), (cursor, 0), "{text:?}");
            assert_eq!(rows(t)[0], text, "{text:?}");
        });
    }
}

#[test]
fn a_cut_two_byte_character_keeps_what_follows() {
    // The recorded tmux-cjk-emoji capture's row 093 (vte 0.15 lost the space).
    both(20, 4, 100, &[b"a \xc3", b"\xbc \xe7\xb5\x82"], |t| {
        assert_eq!(rows(t)[0], "a \u{fc} \u{7d42}")
    });
}

#[test]
fn reflow_keeps_the_text() {
    let mut t = term(10, 4, 100);
    t.feed(b"0123456789ABCDE");
    t.resize(20, 4);
    assert_eq!(screen(&t)[0], ("0123456789ABCDE".into(), false));
    let mut t = term(20, 4, 100);
    t.feed(b"0123456789ABCDE");
    t.resize(10, 4);
    let all: Vec<_> = history(&t)
        .into_iter()
        .chain(screen(&t))
        .filter(|r| !r.0.is_empty())
        .collect();
    assert_eq!(
        all,
        vec![("0123456789".into(), true), ("ABCDE".into(), false)]
    );
}

#[test]
fn osc52_store_and_load() {
    let mut t = term(20, 4, 100);
    t.feed(b"\x1b]52;c;aGVsbG8=\x07\x1b]52;c;?\x07\x07");
    assert_eq!(
        t.take_events(),
        vec![
            Event::ClipboardStore("hello".into()),
            Event::ClipboardLoad,
            Event::Bell
        ]
    );
    t.answer_clipboard("hi");
    assert_eq!(t.take_replies(), b"\x1b]52;c;aGk=\x07");
}

#[test]
fn synchronized_output_is_held_until_its_end_or_deadline() {
    let mut t = term(20, 4, 100);
    t.feed(b"\x1b[?2026hframe");
    assert_eq!(rows(&t)[0], "", "half a frame on the grid");
    assert!(t.sync_deadline().is_some());
    t.feed(b"\x1b[?2026l");
    assert_eq!(rows(&t)[0], "frame");
    // A program that never ends its update: the deadline draws it.
    t.feed(b"\r\x1b[?2026hlater");
    assert_eq!(rows(&t)[0], "frame");
    t.flush_sync();
    assert_eq!(rows(&t)[0], "later");
}

#[test]
fn modes_the_host_reads() {
    let mut t = term(20, 4, 100);
    t.feed(b"\x1b[?1002h\x1b[?1006h\x1b[?2004h\x1b[?1h\x1b[>1u\x1b[?1004h");
    let m = t.modes();
    assert_eq!(
        (
            m.mouse,
            m.mouse_sgr,
            m.bracketed_paste,
            m.app_cursor,
            m.kitty_flags,
            m.focus_reports
        ),
        (2, true, true, true, 1, true)
    );
}

// --- Our fork's fixes to wezterm-term (docs/WEZTERM-PATCHES.md), each in
// one read and one byte per read.

#[test]
fn w5_a_wide_character_at_the_margin_wraps() {
    both(10, 4, 100, &["abcdefghi中x".as_bytes()], |t| {
        assert_eq!(screen(t)[0], ("abcdefghi".into(), true));
        assert_eq!(rows(t)[1], "中x");
        assert_eq!(t.cursor(), (3, 1));
    });
}

#[test]
fn w6_graphemes_cut_by_a_read_join_their_cell() {
    for (parts, want, cursor) in [
        (vec!["e", "\u{301}|"], "e\u{301}|", 2),
        (
            vec!["\u{1f468}", "\u{200d}\u{1f469} z"],
            "\u{1f468}\u{200d}\u{1f469} z",
            4,
        ),
        (
            vec!["\u{1f468}\u{200d}", "\u{1f469} z"],
            "\u{1f468}\u{200d}\u{1f469} z",
            4,
        ),
        (vec!["\u{26a0}", "\u{fe0f}|"], "\u{26a0}\u{fe0f}|", 3),
        (vec!["\u{1f1e8}", "\u{1f1f3}|"], "\u{1f1e8}\u{1f1f3}|", 3),
        (vec!["\u{1f44d}", "\u{1f3fd}|"], "\u{1f44d}\u{1f3fd}|", 3),
        // A Prepend character (U+0604) at the end of a read waits for what
        // it attaches to.
        (vec!["[\u{604}", "\\x"], "[\u{604}\\x", 3),
    ] {
        let mut t = term(20, 4, 100);
        for p in &parts {
            t.feed(p.as_bytes());
        }
        assert_eq!(
            (rows(&t)[0].as_str(), t.cursor()),
            (want, (cursor, 0)),
            "{parts:?}"
        );
    }
}

#[test]
fn w11_a_line_ending_in_a_wide_character_keeps_its_wrap_mark() {
    // Edited out of order (a join rewrote a cell), then wrapped.
    let mut t = term(4, 3, 100);
    t.feed("a\u{1f468}".as_bytes());
    t.feed("\u{200d}\u{1f469}中b".as_bytes());
    assert_eq!(screen(&t)[0], ("a\u{1f468}\u{200d}\u{1f469}".into(), true));
}

#[test]
fn w12_cells_that_would_form_one_grapheme_stay_two() {
    // An emoji ending in ZWJ, a control, another emoji: two cells, as
    // printed (the line's one text read them back as one, every cell
    // after shifted).
    both(
        20,
        3,
        100,
        &[b"[\xf0\x9f\x91\xa8\xe2\x80\x8d\x01\xf0\x9f\x91\xa8+EN"],
        |t| {
            assert_eq!(rows(t)[0], "[\u{1f468}\u{200d}\u{1f468}+EN");
            assert_eq!(t.cursor(), (8, 0));
            let f = t.frame();
            let c = &f.changed[0].cells;
            assert_eq!(
                (char::from_u32(c[3].ch), char::from_u32(c[5].ch)),
                (Some('\u{1f468}'), Some('+'))
            );
        },
    );
}

#[test]
fn w13_w14_cursor_far_right() {
    // CHA and CUP past the last column stop at it, as in xterm.
    both(10, 3, 100, &[b"\x1b[999G\x1b[2DX\x1b[2;99HY"], |t| {
        assert_eq!(rows(t)[0], "       X");
        assert_eq!(rows(t)[1], "         Y");
    });
    // HTS there does not crash.
    both(10, 3, 100, &[b"\x1b[1;99H\x1bH\r\tZ"], |t| {
        assert_eq!(rows(t)[0], "        Z")
    });
}

#[test]
fn w10_clear_keeps_the_screen_in_the_history() {
    both(10, 4, 100, &[b"one\r\ntwo\x1b[H\x1b[2J"], |t| {
        let h: Vec<String> = history(t).into_iter().map(|r| r.0).collect();
        assert_eq!(h, ["one", "two"]);
        assert!(rows(t).iter().all(|r| r.is_empty()));
    });
    // Not on the alternate screen.
    both(10, 4, 100, &[b"\x1b[?1049hone\x1b[2J\x1b[?1049l"], |t| {
        assert_eq!(t.history_len(), 0)
    });
}
