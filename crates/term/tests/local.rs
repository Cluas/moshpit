//! A pane the view feeds itself (`Pane::local`): what a view whose bytes
//! come from elsewhere (the app's Swift transports, or a reader of the
//! session's stream that acts on its marks first) pushes is parsed on the
//! pane's thread in the order pushed, nothing dropped; resizes and input
//! resets land between the bytes they came between; replies wait for the
//! view.

mod common;
use common::*;
use moshpit_stream::Mark;
use moshpit_term::pane::{Callbacks, Pane};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn local(cols: u16, rows: u16) -> (Pane, Arc<AtomicUsize>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let w = wakes.clone();
    let pane = Pane::local(
        term(cols, rows, 2_000),
        Callbacks {
            frame_ready: Box::new(move || {
                w.fetch_add(1, Ordering::SeqCst);
            }),
            // The view's own marks (`Pane::push_mark`), noted for the next
            // frame as the C ABI does; the pane's own codes never get here.
            mark: Box::new(|t, m| {
                assert_eq!(
                    m.code & 0x8000_0000,
                    0,
                    "a local pane's own codes are its own"
                );
                t.note_mark(m)
            }),
        },
    )
    .expect("a local pane opens");
    (pane, wakes)
}

/// Waits until `done` holds of the pane's terminal.
fn settle(pane: &Pane, done: impl Fn(&moshpit_term::Terminal) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !pane.with(|t| done(t)) {
        assert!(Instant::now() < deadline, "the pane never caught up");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn pushed_bytes_parse_in_order_with_nothing_lost() {
    let (pane, wakes) = local(40, 10);
    assert!(pane.is_local());
    let text: String = (0..20_000).map(|i| format!("{i}\r\n")).collect();
    let bytes = text.as_bytes();
    // Cut into uneven pushes, as reads come.
    let mut at = 0;
    let mut n = 1;
    while at < bytes.len() {
        let end = (at + n).min(bytes.len());
        assert!(pane.push(&bytes[at..end]));
        at = end;
        n = n * 7 % 4093 + 1;
    }
    pane.push(b"end");
    settle(&pane, |t| rows(t).iter().any(|r| r == "end"));
    pane.with(|t| {
        let hist: Vec<String> = history(t).into_iter().map(|r| r.0).collect();
        let tail = &hist[hist.len() - 1_500..];
        let start: usize = tail[0].parse().unwrap();
        for (k, line) in tail.iter().enumerate() {
            assert_eq!(line, &(start + k).to_string(), "out of order or lost");
        }
    });
    assert!(wakes.load(Ordering::SeqCst) > 0, "the view was told");
}

#[test]
fn a_resize_lands_between_the_bytes_around_it() {
    // What the remote drew for 20 columns, then for 10: the first line
    // wraps at 20, the second at 10, as on a direct feed in that order.
    let (pane, _) = local(20, 4);
    pane.push(b"abcdefghijklmnopqrstuvwxyz\r\n");
    pane.resize(10, 4);
    pane.push(b"ABCDEFGHIJKLMNOP\r\nok");
    settle(&pane, |t| rows(t).last().is_some_and(|r| r == "ok"));

    let mut direct = term(20, 4, 2_000);
    direct.feed(b"abcdefghijklmnopqrstuvwxyz\r\n");
    direct.resize(10, 4);
    direct.feed(b"ABCDEFGHIJKLMNOP\r\nok");
    pane.with(|t| {
        assert_eq!(t.size(), (10, 4));
        assert_eq!(state(t), state(&direct));
    });
}

#[test]
fn an_input_reset_forgets_only_the_dead_streams_half_sequence() {
    let (pane, _) = local(20, 4);
    // A reconnect cut an OSC in half: without the reset the new stream's
    // first repaint would be swallowed as the OSC's text.
    pane.push(b"old\x1b]0;half a tit");
    pane.discard_pending_input();
    pane.push(b"\r\nnew");
    settle(&pane, |t| rows(t).iter().any(|r| r == "new"));
    pane.with(|t| assert_eq!(rows(t)[0], "old"));
}

#[test]
fn replies_wait_for_the_view_in_order() {
    let (pane, wakes) = local(20, 4);
    pane.push(b"\x1b[6n\x1b[5n");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut got = Vec::new();
    while got.len() < 10 {
        assert!(Instant::now() < deadline, "no replies");
        got.extend(pane.take_replies());
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(got, b"\x1b[1;1R\x1b[0n");
    // The pass keeps its replies before it wakes the view: the wake may
    // land just after the replies were taken.
    while wakes.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline, "the view was never woken");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_session_pane_takes_no_pushes() {
    let (stream, _input) = moshpit_stream::open();
    let pane = Pane::attach(
        stream.id(),
        term(10, 2, 10),
        Callbacks {
            frame_ready: Box::new(|| {}),
            mark: Box::new(|_, _| {}),
        },
    )
    .unwrap();
    assert!(!pane.is_local());
    assert!(!pane.push(b"x"), "the core pushes there");
    assert!(!pane.push_mark(Mark { code: 4, arg: 0 }), "and marks there");
    assert!(pane.take_replies().is_empty());
}

#[test]
fn a_pushed_mark_comes_back_with_the_frame_of_the_bytes_before_it() {
    // What a pane switch needs of the core: its reveal comes with the frame
    // that has the repaint before it, never earlier; its veil comes no later
    // than the first frame with a byte after it. Line n is pushed, then mark
    // n, then line n + 1: whatever frame the view takes, while the pane's
    // thread parses ahead, the two must agree.
    let (pane, _) = local(20, 3);
    let pane = Arc::new(pane);
    const N: u64 = 2_000;
    let producer = {
        let pane = pane.clone();
        std::thread::spawn(move || {
            for n in 1..=N {
                assert!(pane.push(format!("\r\x1b[KL{n}").as_bytes()));
                assert!(pane.push_mark(Mark {
                    code: 4,
                    arg: n as i64
                }));
            }
        })
    };
    let mut reached = 0u64;
    let mut frames_with_marks = 0;
    // Row 0 as the frames draw it (a frame carries the rows that changed).
    let mut on_screen: Option<u64> = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    while reached < N {
        assert!(
            Instant::now() < deadline,
            "the marks never all came: {reached} of {N}"
        );
        // A view takes frames at the display's pace, not in a spin (the
        // pane's thread yields to a view that waits).
        std::thread::sleep(Duration::from_micros(300));
        // The frames in order: the ones the pane's thread published too.
        let frame = pane.frame();
        if let Some(r) = frame.changed.iter().find(|r| r.row == 0) {
            let text: String = r
                .cells
                .iter()
                .map(|c| char::from_u32(c.ch).unwrap_or(' '))
                .collect();
            on_screen = text.trim().strip_prefix('L').and_then(|n| n.parse().ok());
        }
        for m in &frame.marks {
            assert_eq!(m.code, 4);
            assert_eq!(m.arg as u64, reached + 1, "marks in order, none lost");
            reached = m.arg as u64;
        }
        if !frame.marks.is_empty() {
            frames_with_marks += 1;
            // The bytes before the last mark are in this frame.
            assert!(
                on_screen.unwrap_or(0) >= reached,
                "a mark came before its bytes"
            );
        }
        // No byte after a mark the view has not been handed.
        assert!(
            on_screen.unwrap_or(0) <= reached + 1,
            "a frame ran past an untaken mark"
        );
    }
    producer.join().unwrap();
    assert!(frames_with_marks > 0);
}

#[test]
fn a_mark_inside_a_synchronized_update_waits_for_its_end() {
    // The update's bytes are not drawn before its end (or its deadline),
    // so neither is the reveal that follows them.
    let mut t = term(20, 3, 10);
    t.feed(b"old");
    let _ = t.frame();
    t.feed(b"\x1b[?2026h\r\x1b[Knew");
    t.note_mark(Mark { code: 4, arg: 1 });
    t.feed(b"!");
    let f = t.frame();
    assert!(
        f.marks.is_empty(),
        "the reveal came before the update it follows"
    );
    assert_eq!(rows(&t)[0], "old");
    t.feed(b"\x1b[?2026l");
    let f = t.frame();
    assert_eq!(f.marks, vec![Mark { code: 4, arg: 1 }]);
    assert_eq!(rows(&t)[0], "new!");

    // An update that never ends is drawn at its deadline, its marks with it.
    let mut t = term(20, 3, 10);
    t.feed(b"\x1b[?2026hx");
    t.note_mark(Mark { code: 1, arg: 0 });
    assert!(t.frame().marks.is_empty());
    t.flush_sync();
    assert_eq!(t.frame().marks, vec![Mark { code: 1, arg: 0 }]);
}

#[test]
fn the_panes_own_codes_are_not_a_views_marks() {
    let (pane, _) = local(20, 3);
    assert!(!pane.push_mark(Mark {
        code: 0x8000_0001,
        arg: (5 << 16) | 2
    }));
    assert!(pane.push_mark(Mark {
        code: 0x7fff_ffff,
        arg: -1
    }));
}
