//! What a view draws from: frames of the changed rows with colours
//! resolved, and the byte channel feeding a pane off the main thread.

mod common;
use common::*;
use moshpit_term::{
    Frame, SelectKind, cell_flags as F,
    pane::{Callbacks, Pane},
};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

fn text(frame: &Frame, row: u16) -> String {
    let r = frame.changed.iter().find(|r| r.row == row).unwrap();
    let mut s = String::new();
    for c in &r.cells {
        if c.flags & F::SPACER != 0 {
            continue;
        }
        s.push(char::from_u32(c.ch).unwrap());
        for &x in &r.extra[c.extra_at as usize..c.extra_at as usize + c.extra as usize] {
            s.push(char::from_u32(x).unwrap());
        }
    }
    s.trim_end().to_string()
}

#[test]
fn the_first_frame_is_full_then_only_what_changed() {
    let mut t = term(40, 10, 100);
    t.feed(b"$ ");
    let f = t.frame();
    assert!(f.full);
    assert_eq!(f.changed.len(), 10);
    assert_eq!(text(&f, 0), "$");
    // One typed character: its row (and the cursor's) only.
    t.feed(b"l");
    let f = t.frame();
    assert!(!f.full);
    assert_eq!(f.changed.iter().map(|r| r.row).collect::<Vec<_>>(), vec![0]);
    assert_eq!(text(&f, 0), "$ l");
    assert_eq!((f.cursor.col, f.cursor.row, f.cursor.visible), (3, 0, true));
    // Nothing new: nothing but the cursor's row.
    let f = t.frame();
    assert!(f.changed.len() <= 1);
}

#[test]
fn colours_are_resolved_for_drawing() {
    let mut t = term(40, 4, 100);
    t.feed(b"\x1b[31mr\x1b[1mR\x1b[0;38;2;1;2;3mt\x1b[7mi\x1b[0;4:3mu\x1b[0m");
    let f = t.frame();
    let c = &f.changed[0].cells;
    assert_eq!(c[0].fg, 0xcd0000, "SGR 31 from the palette");
    assert_eq!(c[1].fg, 0xff0000, "bold red draws bright");
    assert_ne!(c[1].flags & F::BOLD, 0);
    assert_eq!(c[2].fg, 0x010203, "true colour");
    assert_eq!((c[3].fg, c[3].bg), (0x000000, 0x010203), "inverse swaps");
    assert_ne!(c[4].flags & F::UNDERCURL, 0);
    assert_eq!(c[5].bg, f.background);
}

#[test]
fn wide_cells_links_selection_in_a_frame() {
    let mut t = term(20, 4, 100);
    t.feed(
        "中\x1b]8;;https://example.com\x1b\\ab\x1b]8;;\x1b\\ \u{1f468}\u{200d}\u{1f469}".as_bytes(),
    );
    t.select_start(SelectKind::Cells, 0, 2, false);
    t.select_extend(0, 3, true);
    let f = t.frame();
    let c = &f.changed[0].cells;
    assert_ne!(c[0].flags & F::WIDE, 0);
    assert_ne!(c[1].flags & F::SPACER, 0);
    assert_eq!(f.links, vec!["https://example.com".to_string()]);
    assert_eq!((c[2].link, c[3].link, c[4].link), (1, 1, 0));
    assert_ne!(c[2].flags & F::SELECTED, 0);
    assert_eq!(c[4].flags & F::SELECTED, 0);
    assert_eq!(text(&f, 0), "中ab \u{1f468}\u{200d}\u{1f469}");
    assert_ne!(c[5].flags & F::WIDE, 0, "the ZWJ pair is one wide cell");
}

#[test]
fn a_pane_is_fed_by_the_byte_channel_off_the_main_thread() {
    let (stream, mut input) = moshpit_stream::open();
    let (ready_tx, ready) = mpsc::channel::<()>();
    let ready_tx = Mutex::new(ready_tx);
    let marks = Arc::new(Mutex::new(Vec::new()));
    let seen = marks.clone();
    let pane = Pane::attach(
        stream.id(),
        term(40, 5, 1000),
        Callbacks {
            frame_ready: Box::new(move || {
                let _ = ready_tx.lock().unwrap().send(());
            }),
            mark: Box::new(move |t, m| {
                seen.lock()
                    .unwrap()
                    .push((m.code, t.visible_text()[0].0.clone()))
            }),
        },
    )
    .unwrap();

    // The core pushes output; the pane's thread parses it and says a frame
    // is ready; the view takes it.
    stream.push(b"hello\r\n");
    stream.mark(moshpit_stream::Mark { code: 7, arg: 0 });
    stream.push(b"\x1b[c");
    ready.recv_timeout(Duration::from_secs(5)).expect("a frame");
    let f = pane.frame();
    assert_eq!(text(&f, 0), "hello");
    assert_eq!(
        marks.lock().unwrap().as_slice(),
        [(7, "hello".to_string())],
        "the mark between the bytes"
    );

    // The terminal's reply goes to the core on the same input queue the
    // view's keystrokes use.
    let got = futures_lite_block_on(async { input.next().await });
    assert_eq!(
        got,
        Some(moshpit_stream::InputEvent::Bytes(
            b"\x1b[?65;4;6;18;22;52c".to_vec()
        ))
    );
    assert!(pane.write(b"k"));
    let got = futures_lite_block_on(async { input.next().await });
    assert_eq!(got, Some(moshpit_stream::InputEvent::Bytes(b"k".to_vec())));

    // A burst in many pushes arrives whole and in order.
    let mut want = Vec::new();
    for i in 0..20_000 {
        let line = format!("{i}\r\n");
        want.push(i.to_string());
        stream.push(line.as_bytes());
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let _ = ready.recv_timeout(Duration::from_millis(200));
        let done = pane.with(|t| t.visible_text().iter().any(|r| r.0 == "19999"));
        if done {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the burst never finished"
        );
    }
    pane.with(|t| {
        let hist: Vec<String> = history(t).into_iter().map(|r| r.0).collect();
        let tail = &hist[hist.len() - 900..];
        let start: usize = tail[0].parse().unwrap();
        for (k, line) in tail.iter().enumerate() {
            assert_eq!(line, &(start + k).to_string(), "out of order or lost");
        }
    });
    drop(pane);
    drop(stream);
}

/// A tiny executor for the input stream (the crate has no async runtime).
fn futures_lite_block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::task::{Context, Poll, Wake, Waker};
    struct Thread(std::thread::Thread);
    impl Wake for Thread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Thread(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
}

#[test]
fn patch25_erase_and_repeat_reach_the_frame() {
    let mut t = term(20, 4, 100);
    t.feed(b"abcdef\r\n\r\nxyz");
    let _ = t.frame();
    t.feed(b"\x1b[1;2H\x1b[2X");
    assert_eq!(text(&t.frame(), 0), "a  def", "ECH's row is in the frame");
    t.feed(b"\x1b[3;1Hq\x1b[3b");
    assert_eq!(text(&t.frame(), 2), "qqqq", "REP's row is in the frame");
}

#[test]
fn patch29_32_clear_and_reset_while_scrolled_back_repaint_at_the_bottom() {
    for (what, seq) in [("ED 3", &b"\x1b[3J"[..]), ("RIS", &b"\x1bc"[..])] {
        let mut t = term(20, 4, 100);
        t.feed(
            (1..=30)
                .map(|i| format!("{i}\r\n"))
                .collect::<String>()
                .as_bytes(),
        );
        t.scroll(10);
        let _ = t.frame();
        t.feed(seq);
        let f = t.frame();
        assert_eq!(
            f.display_offset, 0,
            "{what}: the view is back at the bottom"
        );
        assert!(f.full, "{what}: every row is repainted");
        if what == "ED 3" {
            assert_eq!(f.history, 0, "{what}");
        }
    }
}

/// What a view keeps from frames: rows moved by each frame's shift, then
/// the rows it carries.
fn replay(view: &mut Vec<Vec<(u32, u32, u32, u16)>>, f: &moshpit_term::Frame) {
    let span = f.rows as usize + if f.display_offset > 0 { 1 } else { 0 };
    if view.len() != span || f.full {
        view.clear();
        view.resize(span, Vec::new());
    } else if f.shift > 0 {
        let k = f.shift as usize;
        view.drain(..k);
        view.extend(std::iter::repeat_n(Vec::new(), k));
    } else if f.shift < 0 {
        let k = (-f.shift) as usize;
        view.truncate(span - k);
        for _ in 0..k {
            view.insert(0, Vec::new());
        }
    }
    for r in &f.changed {
        if (r.row as usize) < view.len() {
            view[r.row as usize] = r
                .cells
                .iter()
                .map(|c| (c.ch, c.fg, c.bg, c.flags))
                .collect();
        }
    }
}

#[test]
fn output_that_scrolls_the_screen_moves_the_rows_and_sends_only_the_new_ones() {
    let mut t = term(20, 5, 100);
    t.feed(b"1\r\n2\r\n3\r\n4\r\n5");
    let _ = t.frame();
    t.feed(b"\r\n6\r\n7");
    let f = t.frame();
    assert!(!f.full);
    assert_eq!(f.shift, 2);
    let rows: Vec<u16> = f.changed.iter().map(|r| r.row).collect();
    assert_eq!(
        rows,
        vec![2, 3, 4],
        "the cursor's old row (moved) and the two new ones"
    );
}

#[test]
fn frames_with_shifts_rebuild_the_screen_exactly() {
    // A view that applies shifts and changed rows draws what a full frame
    // of the same state draws, through output, scrolling back and forth,
    // and output while scrolled back.
    let mut seed: u64 = 0x5eed_2026;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    let bits = [
        "line\r\n",
        "\x1b[31mred\x1b[0m ",
        "中文",
        "\r\n",
        "\x1b[2K",
        "x",
        "\x1b[3;1H",
        "\r\n\r\n\r\n",
    ];
    let mut t = term(24, 8, 200);
    t.set_cell_pixels(10, 20);
    let mut view = Vec::new();
    let mut all = Vec::new();
    for step in 0..400 {
        let mut s = String::new();
        for _ in 0..(next() % 6) {
            s += bits[next() % bits.len()];
        }
        t.feed(s.as_bytes());
        all.extend_from_slice(s.as_bytes());
        match next() % 5 {
            0 => {
                t.scroll_drag((next() % 90) as f64, false);
            }
            1 => {
                t.scroll_drag(-((next() % 90) as f64), false);
            }
            _ => {}
        }
        let f = t.frame();
        replay(&mut view, &f);
        // A fresh frame of the same state: every row.
        let mut fresh = Vec::new();
        let off = t.scroll_offset_px();
        let mut u = term(24, 8, 200);
        u.set_cell_pixels(10, 20);
        u.feed(&all);
        if off > 0 {
            u.scroll_drag(off as f64, false);
        }
        replay(&mut fresh, &u.frame());
        assert_eq!(
            view, fresh,
            "step {step}: frames with shifts drew something else"
        );
    }
}
