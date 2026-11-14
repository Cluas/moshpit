//! T4: a backlog — bytes waiting faster than they parse, as a long capture,
//! a big `cat` or a spell away leaves them — keeps the screen moving: the
//! pane's thread publishes a frame about every display refresh while it
//! catches up, the view takes them without the terminal's lock, and nothing
//! is lost or reordered. Frames taken late bring the rest; a view's own
//! change is framed after what was published.

mod common;
use common::*;
use moshpit_stream::Mark;
use moshpit_term::{
    Event, Frame,
    pane::{Callbacks, PUBLISH, Pane},
};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

fn pane(cols: u16, rows: u16) -> (Pane, mpsc::Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let pane = Pane::local(
        term(cols, rows, 1_000),
        Callbacks {
            frame_ready: Box::new(move || {
                let _ = tx.lock().unwrap().send(());
            }),
            mark: Box::new(|t, m| t.note_mark(m)),
        },
    )
    .expect("a pane opens");
    (pane, rx)
}

/// A frame's row `row` as text, if it carries it.
fn row(frame: &Frame, row: u16) -> Option<String> {
    let r = frame.changed.iter().find(|r| r.row == row)?;
    let s: String = r
        .cells
        .iter()
        .map(|c| char::from_u32(c.ch).unwrap_or(' '))
        .collect();
    Some(s.trim_end().to_string())
}

/// The highest line number a frame shows.
fn newest(frame: &Frame) -> Option<u64> {
    frame
        .changed
        .iter()
        .filter_map(|r| {
            let s: String = r
                .cells
                .iter()
                .map(|c| char::from_u32(c.ch).unwrap_or(' '))
                .collect();
            s.split_whitespace().next()?.parse().ok()
        })
        .max()
}

#[test]
fn a_long_backlog_yields_frames_as_it_goes() {
    let (pane, ready) = pane(90, 50);
    // A backlog: everything is waiting before the parse is half way.
    let tail = "abcdefgh".repeat(10);
    let lines: u64 = if cfg!(debug_assertions) {
        60_000
    } else {
        600_000
    };
    let mut chunk = String::new();
    let start = Instant::now();
    for i in 0..lines {
        chunk.push_str(&format!("{i} {tail}\r\n"));
        if chunk.len() > 32 * 1024 {
            pane.push(chunk.as_bytes());
            chunk.clear();
        }
    }
    pane.push(chunk.as_bytes());
    // The view: takes what is published when told, as the app's view does.
    let mut times: Vec<Instant> = Vec::new();
    let mut shown: Vec<u64> = Vec::new();
    let last = lines - 1;
    let deadline = Instant::now() + Duration::from_secs(120);
    while shown.last() != Some(&last) {
        assert!(
            Instant::now() < deadline,
            "the backlog never showed its end"
        );
        if ready.recv_timeout(Duration::from_millis(500)).is_err() {
            continue;
        }
        while let Some(f) = pane.take_published() {
            if let Some(n) = newest(&f) {
                times.push(Instant::now());
                shown.push(n);
            }
        }
    }
    let took = start.elapsed();
    // Forward, never back.
    assert!(
        shown.windows(2).all(|w| w[0] <= w[1]),
        "a frame went back: {shown:?}"
    );
    // The screen moved while it caught up: frames all along (one at the
    // end, before: the whole catch-up was one still screen). A debug build
    // on a loaded machine parses a read slowly, so the bound is relative;
    // the device measures the rate (TERMINAL-VIEW.md 5.6).
    let mut gaps: Vec<Duration> = std::iter::once(times[0] - start)
        .chain(times.windows(2).map(|w| w[1] - w[0]))
        .collect();
    gaps.sort();
    let longest = gaps.last().copied().unwrap_or(took);
    assert!(times.len() >= 10, "{} frames in {took:?}", times.len());
    assert!(
        longest * 4 < took,
        "the screen stood still {longest:?} of {took:?}"
    );
    // About a display refresh apart, optimized (a debug build parses one
    // read slower than that: a frame can't come between reads).
    if !cfg!(debug_assertions) {
        let median = gaps[gaps.len() / 2];
        assert!(median < PUBLISH * 3, "frames every {median:?}");
    }
    // Every line, in order, nothing lost.
    pane.with(|t| {
        let hist: Vec<String> = history(t).into_iter().map(|r| r.0).collect();
        let nums: Vec<u64> = hist
            .iter()
            .chain(rows(t).iter())
            .filter_map(|r| r.split_whitespace().next()?.parse().ok())
            .collect();
        assert_eq!(nums.last(), Some(&last));
        assert!(
            nums.windows(2).all(|w| w[1] == w[0] + 1),
            "out of order or lost"
        );
    });
    eprintln!(
        "backlog of {lines} lines: {} frames in {took:?}, longest gap {longest:?}, median {:?}",
        times.len(),
        gaps.get(gaps.len() / 2).copied().unwrap_or_default()
    );
}

#[test]
fn a_frame_taken_late_brings_the_rest() {
    let (pane, ready) = pane(20, 3);
    pane.push(b"1");
    ready
        .recv_timeout(Duration::from_secs(10))
        .expect("a frame");
    // The view is slow to take it; more is parsed meanwhile.
    pane.push(b"2");
    let deadline = Instant::now() + Duration::from_secs(10);
    while pane.with(|t| rows(t)[0] != "12") {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::sleep(Duration::from_millis(30));
    while ready.try_recv().is_ok() {}
    let first = pane.take_published().expect("the frame left for the view");
    assert_eq!(
        row(&first, 0).as_deref(),
        Some("1"),
        "what was parsed when it was built"
    );
    // Taking it asked for the rest.
    ready
        .recv_timeout(Duration::from_secs(10))
        .expect("the rest");
    let rest = pane.take_published().expect("the rest, published");
    assert_eq!(row(&rest, 0).as_deref(), Some("12"));
}

#[test]
fn the_views_own_change_is_framed_after_what_was_published() {
    let (pane, ready) = pane(20, 3);
    pane.push(b"a");
    ready
        .recv_timeout(Duration::from_secs(10))
        .expect("a frame");
    let _ = pane.take_published();
    pane.push(b"b");
    ready
        .recv_timeout(Duration::from_secs(10))
        .expect("a frame");
    // Published (\"ab\") and not taken, when the view changes the look.
    pane.with(|t| {
        // A new look, whichever it was.
        t.set_bold_is_bright(false);
        t.set_bold_is_bright(true);
    });
    let first = pane.frame();
    assert_eq!(
        row(&first, 0).as_deref(),
        Some("ab"),
        "the published frame first"
    );
    assert!(!first.full);
    let second = pane
        .take_published()
        .expect("the view's change, left for the take");
    assert!(second.full, "the whole screen in the new look");
    assert!(pane.take_published().is_none(), "nothing else waits");
}

/// The view takes the parse's events (a title, a bell) without the
/// terminal's lock: a frame's turn never waits for a parse.
#[test]
fn events_are_taken_while_the_terminal_is_busy() {
    let (busy, parsing) = mpsc::channel();
    let busy = std::sync::Mutex::new(busy);
    let pane = Pane::local(
        term(20, 3, 100),
        Callbacks {
            frame_ready: Box::new(|| {}),
            // A long parse: the mark's handler holds the terminal.
            mark: Box::new(move |_, m| {
                let _ = busy.lock().unwrap().send(());
                std::thread::sleep(Duration::from_millis(m.arg as u64));
            }),
        },
    )
    .expect("a pane opens");
    pane.push(b"\x1b]2;one\x07\x07");
    assert!(pane.push_mark(Mark { code: 1, arg: 600 }));
    parsing
        .recv_timeout(Duration::from_secs(10))
        .expect("the parse reached the mark");
    let at = Instant::now();
    let events = pane.take_events();
    assert!(
        at.elapsed() < Duration::from_millis(300),
        "waited {:?} for the terminal",
        at.elapsed()
    );
    assert_eq!(events, vec![Event::Title("one".into()), Event::Bell]);
}
