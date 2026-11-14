//! The channel's rules, one at a time: wakes, order, coalescing, lifecycle
//! and the C ABI.
#![cfg(not(moshpit_loom))]

use std::{
    ffi::c_void,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use moshpit_stream::{InputEvent, ffi, open, view};

/// A waker that counts its calls.
fn counter() -> (Arc<AtomicUsize>, impl Fn() + Send + Sync + 'static) {
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    (count, move || {
        c.fetch_add(1, Ordering::SeqCst);
    })
}

fn read_all(id: u64, cap: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0; cap];
    loop {
        let n = view::read(id, &mut buf);
        out.extend_from_slice(&buf[..n]);
        if n < cap {
            return out;
        }
    }
}

#[test]
fn one_wake_until_the_view_has_drained() {
    let (stream, _input) = open();
    let (wakes, waker) = counter();
    assert!(view::attach(stream.id(), waker));
    assert_eq!(wakes.load(Ordering::SeqCst), 0, "nothing waiting yet");

    for i in 0..100u8 {
        stream.push(&[i]);
    }
    assert_eq!(wakes.load(Ordering::SeqCst), 1, "a burst costs one wake");

    // A partial read leaves output waiting: still one wake out.
    let mut buf = [0; 10];
    assert_eq!(view::read(stream.id(), &mut buf), 10);
    stream.push(b"more");
    assert_eq!(wakes.load(Ordering::SeqCst), 1);

    // Drained: the next push wakes again, once.
    let rest = read_all(stream.id(), 7);
    assert_eq!(rest.len(), 90 + 4);
    stream.push(b"a");
    stream.push(b"b");
    assert_eq!(wakes.load(Ordering::SeqCst), 2);
    assert_eq!(read_all(stream.id(), 64), b"ab");

    let stats = stream.stats();
    assert_eq!(stats.pushes, 103);
    assert_eq!(stats.pushed, 100 + 4 + 2);
    assert_eq!(stats.read, stats.pushed);
    assert_eq!(stats.wakes, 2);
    assert_eq!(stats.drains, 2);
    assert_eq!(stats.queued, 0);
}

#[test]
fn a_read_that_fills_the_buffer_exactly_has_drained() {
    let (stream, _input) = open();
    let (wakes, waker) = counter();
    view::attach(stream.id(), waker);
    stream.push(b"12345678");
    let mut buf = [0; 8];
    assert_eq!(view::read(stream.id(), &mut buf), 8);
    // The queue is empty, so the next push owes the view a wake even though
    // the view could not tell it was done.
    stream.push(b"9");
    assert_eq!(wakes.load(Ordering::SeqCst), 2);
    assert_eq!(view::read(stream.id(), &mut buf), 1);
}

#[test]
fn attaching_with_output_waiting_wakes_right_away() {
    let (stream, _input) = open();
    stream.push(b"before the view");
    let (wakes, waker) = counter();
    view::attach(stream.id(), waker);
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    stream.push(b" and after");
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(read_all(stream.id(), 4), b"before the view and after");
}

#[test]
fn a_detached_waker_is_never_called_and_a_new_one_takes_over() {
    let (stream, _input) = open();
    let (old, waker) = counter();
    view::attach(stream.id(), waker);
    view::detach(stream.id());
    stream.push(b"x");
    assert_eq!(old.load(Ordering::SeqCst), 0);

    // Output is waiting: the new waker is called at once, the old never.
    let (new, waker) = counter();
    view::attach(stream.id(), waker);
    assert_eq!(new.load(Ordering::SeqCst), 1);
    stream.push(b"y");
    assert_eq!(new.load(Ordering::SeqCst), 1);
    assert_eq!(read_all(stream.id(), 16), b"xy");
    assert_eq!(old.load(Ordering::SeqCst), 0);
}

#[test]
fn input_comes_in_order_coalesced_up_to_each_resize() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (stream, mut input) = open();
    let id = stream.id();
    assert!(view::write(id, b"ab"));
    assert!(view::write(id, b"c"));
    assert!(view::resize(id, 80, 24));
    assert!(view::resize(id, 100, 30)); // nothing typed between: replaces 80x24
    assert!(view::write(id, b"d"));
    assert!(view::resize(id, 120, 40));
    assert!(view::write(id, b"")); // empty writes are accepted and vanish
    runtime.block_on(async {
        assert_eq!(input.next().await, Some(InputEvent::Bytes(b"abc".to_vec())));
        assert_eq!(
            input.next().await,
            Some(InputEvent::Resize {
                cols: 100,
                rows: 30
            })
        );
        assert_eq!(input.next().await, Some(InputEvent::Bytes(b"d".to_vec())));
        assert_eq!(
            input.next().await,
            Some(InputEvent::Resize {
                cols: 120,
                rows: 40
            })
        );
        // Nothing waiting: the next write wakes the task.
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            view::write(id, b"late")
        });
        assert_eq!(
            input.next().await,
            Some(InputEvent::Bytes(b"late".to_vec()))
        );
        assert!(writer.join().unwrap());
    });
    drop(stream);
}

/// B361: a paste is an event in its place among the keystrokes, never
/// merged into the bytes around it, and it stops two resizes collapsing.
#[test]
fn a_paste_keeps_its_place_among_the_bytes_and_resizes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (stream, mut input) = open();
    let id = stream.id();
    assert!(view::write(id, b"ab"));
    assert!(view::paste(id, "x\x1b[Ay"));
    assert!(view::write(id, b"c"));
    assert!(view::resize(id, 80, 24));
    assert!(view::paste(id, "z"));
    assert!(view::resize(id, 90, 30)); // a paste between: kept apart
    assert!(view::resize(id, 100, 30)); // nothing between: replaces 90x30
    assert!(view::paste(id, "")); // empty pastes are accepted and vanish
    assert!(view::paste(id, "p1"));
    assert!(view::paste(id, "p2"));
    runtime.block_on(async {
        let expected = [
            InputEvent::Bytes(b"ab".to_vec()),
            InputEvent::Paste("x\x1b[Ay".into()),
            InputEvent::Bytes(b"c".to_vec()),
            InputEvent::Resize { cols: 80, rows: 24 },
            InputEvent::Paste("z".into()),
            InputEvent::Resize {
                cols: 100,
                rows: 30,
            },
            InputEvent::Paste("p1".into()),
            InputEvent::Paste("p2".into()),
        ];
        for event in expected {
            assert_eq!(input.next().await, Some(event));
        }
        // Nothing waiting: the next paste wakes the task.
        let paster = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            view::paste(id, "late")
        });
        assert_eq!(input.next().await, Some(InputEvent::Paste("late".into())));
        assert!(paster.join().unwrap());
    });
    drop(stream);
}

#[test]
fn closing_ends_the_input_after_its_tail_and_refuses_writes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (stream, mut input) = open();
    let id = stream.id();
    view::write(id, b"tail");
    drop(stream);
    assert!(!view::write(id, b"too late"));
    assert!(!view::resize(id, 1, 1));
    assert!(!view::paste(id, "too late"));
    runtime.block_on(async {
        assert_eq!(
            input.next().await,
            Some(InputEvent::Bytes(b"tail".to_vec()))
        );
        assert_eq!(input.next().await, None);
    });
}

#[test]
fn input_nobody_takes_is_refused() {
    let (stream, input) = open();
    let id = stream.id();
    assert!(view::write(id, b"heard"));
    drop(input);
    assert!(!view::write(id, b"unheard"));
    assert!(!view::resize(id, 80, 24));
    assert!(!view::paste(id, "unheard"));
    // The output still works.
    stream.push(b"still here");
    assert_eq!(read_all(id, 64), b"still here");
}

#[test]
fn a_closed_stream_stays_readable_until_the_view_detaches() {
    let (stream, _input) = open();
    let id = stream.id();
    let (_wakes, waker) = counter();
    view::attach(id, waker);
    stream.push(b"last words");
    drop(stream);
    // The view can still drain the tail…
    assert!(view::stats(id).is_some());
    assert_eq!(read_all(id, 3), b"last words");
    // …and once it detaches the stream is gone.
    view::detach(id);
    assert!(view::stats(id).is_none());
    assert!(!view::attach(id, || {}));
}

#[test]
fn a_stream_closed_with_no_view_is_gone_at_once() {
    let (stream, input) = open();
    let id = stream.id();
    drop(stream);
    drop(input);
    assert!(view::stats(id).is_none());
    assert!(!view::attach(id, || {}));
    assert_eq!(view::read(id, &mut [0; 4]), 0);
}

#[test]
fn unknown_ids_do_nothing() {
    assert!(!view::attach(0, || {}));
    view::detach(0);
    assert_eq!(view::read(u64::MAX, &mut [0; 4]), 0);
    assert!(!view::write(u64::MAX, b"x"));
    assert!(!view::resize(u64::MAX, 1, 1));
}

// --- The C ABI ----------------------------------------------------------------

unsafe extern "C" fn count_wake(ctx: *mut c_void) {
    // SAFETY: the tests below pass a live `AtomicUsize`.
    let count = unsafe { &*(ctx as *const AtomicUsize) };
    count.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn the_c_abi_round_trips() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (stream, mut input) = open();
    let id = stream.id();
    let wakes = Box::new(AtomicUsize::new(0));
    let ctx = std::ptr::from_ref::<AtomicUsize>(&wakes)
        .cast_mut()
        .cast::<c_void>();
    // SAFETY: `wakes` outlives the attachment (detached below).
    assert!(unsafe { ffi::mp_stream_attach(id, Some(count_wake), ctx) });

    let keys = b"ls -la\r";
    // SAFETY: valid pointers and lengths.
    unsafe {
        assert!(ffi::mp_stream_write(id, keys.as_ptr(), keys.len()));
        assert!(ffi::mp_stream_write(id, std::ptr::null(), 0));
    }
    assert!(ffi::mp_stream_resize(id, 90, 33));
    let text = "终 端\n".as_bytes();
    let broken = b"a\xffb";
    // SAFETY: valid pointers and lengths.
    unsafe {
        assert!(ffi::mp_stream_paste(id, text.as_ptr(), text.len()));
        assert!(ffi::mp_stream_paste(id, std::ptr::null(), 0));
        assert!(ffi::mp_stream_paste(id, broken.as_ptr(), broken.len()));
    }
    runtime.block_on(async {
        assert_eq!(input.next().await, Some(InputEvent::Bytes(keys.to_vec())));
        assert_eq!(
            input.next().await,
            Some(InputEvent::Resize { cols: 90, rows: 33 })
        );
        assert_eq!(
            input.next().await,
            Some(InputEvent::Paste("终 端\n".into()))
        );
        assert_eq!(
            input.next().await,
            Some(InputEvent::Paste("a\u{fffd}b".into()))
        );
    });

    stream.push(b"total 0\r\n");
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    let mut buf = [0u8; 64];
    // SAFETY: valid buffer.
    let n = unsafe { ffi::mp_stream_read(id, buf.as_mut_ptr(), buf.len()) };
    assert_eq!(&buf[..n], b"total 0\r\n");
    // SAFETY: a null buffer reads nothing.
    assert_eq!(
        unsafe { ffi::mp_stream_read(id, std::ptr::null_mut(), 8) },
        0
    );

    let mut stats = ffi::MpStreamStats::default();
    // SAFETY: valid out pointer.
    assert!(unsafe { ffi::mp_stream_stats(id, &mut stats) });
    assert_eq!((stats.pushed, stats.read, stats.wakes), (9, 9, 1));

    ffi::mp_stream_detach(id);
    stream.push(b"after detach");
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    // SAFETY: a NULL waker detaches.
    assert!(unsafe { ffi::mp_stream_attach(id, None, std::ptr::null_mut()) });
}

// --- Marks -----------------------------------------------------------------

/// Everything a marked reader sees, bytes and marks in order, until drained.
fn read_all_marked(id: u64, cap: usize) -> Vec<Result<Vec<u8>, (u32, i64)>> {
    let mut seen = Vec::new();
    let mut buf = vec![0; cap];
    loop {
        let (n, mark) = view::read_marked(id, &mut buf);
        if n > 0 {
            seen.push(Ok(buf[..n].to_vec()));
        }
        match mark {
            Some(mark) => seen.push(Err((mark.code, mark.arg))),
            None if n < cap => return seen,
            None => {}
        }
    }
}

#[test]
fn marks_land_exactly_between_the_bytes_they_were_put_between() {
    use moshpit_stream::Mark;
    let (stream, _input) = open();
    stream.push(b"ab");
    stream.mark(Mark { code: 1, arg: -5 });
    stream.push(b"cd");
    stream.mark(Mark { code: 2, arg: 0 });
    stream.mark(Mark { code: 3, arg: 7 });
    stream.push(b"efgh");
    assert_eq!(
        read_all_marked(stream.id(), 3),
        vec![
            Ok(b"ab".to_vec()),
            Err((1, -5)),
            Ok(b"cd".to_vec()),
            Err((2, 0)),
            Err((3, 7)),
            Ok(b"efg".to_vec()),
            Ok(b"h".to_vec()),
        ]
    );
    assert_eq!(stream.stats().marks, 3);
}

#[test]
fn a_mark_wakes_the_view_and_counts_as_waiting_until_taken() {
    use moshpit_stream::Mark;
    let (stream, _input) = open();
    let (wakes, waker) = counter();
    assert!(view::attach(stream.id(), waker));
    stream.mark(Mark { code: 9, arg: 1 });
    assert_eq!(wakes.load(Ordering::SeqCst), 1, "a mark alone wakes");
    stream.push(b"x");
    assert_eq!(wakes.load(Ordering::SeqCst), 1, "still one wake out");
    let mut buf = [0; 8];
    // The mark sits before the byte: taken first, with nothing read.
    assert_eq!(
        view::read_marked(stream.id(), &mut buf),
        (0, Some(Mark { code: 9, arg: 1 }))
    );
    stream.push(b"y");
    assert_eq!(wakes.load(Ordering::SeqCst), 1, "not drained yet");
    assert_eq!(view::read_marked(stream.id(), &mut buf), (2, None));
    assert_eq!(&buf[..2], b"xy");
    stream.mark(Mark { code: 4, arg: 0 });
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        2,
        "drained: the next mark wakes"
    );
}

#[test]
fn a_plain_read_passes_over_marks() {
    use moshpit_stream::Mark;
    let (stream, _input) = open();
    stream.push(b"one");
    stream.mark(Mark { code: 1, arg: 0 });
    stream.push(b"two");
    stream.mark(Mark { code: 2, arg: 0 });
    assert_eq!(read_all(stream.id(), 64), b"onetwo");
    let mut buf = [0; 8];
    assert_eq!(view::read_marked(stream.id(), &mut buf), (0, None));
}

#[test]
fn the_c_abi_reads_up_to_a_mark() {
    use moshpit_stream::Mark;
    let (stream, _input) = open();
    stream.push(b"hello");
    stream.mark(Mark { code: 6, arg: 1500 });
    stream.push(b"!");
    let mut buf = [0u8; 16];
    let mut mark = ffi::MpStreamMark::default();
    let mut marked = false;
    // SAFETY: all pointers are valid locals.
    let n = unsafe {
        ffi::mp_stream_read_marked(
            stream.id(),
            buf.as_mut_ptr(),
            buf.len(),
            &mut mark,
            &mut marked,
        )
    };
    assert_eq!((n, marked, mark.code, mark.arg), (5, true, 6, 1500));
    assert_eq!(&buf[..5], b"hello");
    // SAFETY: as above.
    let n = unsafe {
        ffi::mp_stream_read_marked(
            stream.id(),
            buf.as_mut_ptr(),
            buf.len(),
            &mut mark,
            &mut marked,
        )
    };
    assert_eq!((n, marked), (1, false));
    assert_eq!(buf[0], b'!');
}
