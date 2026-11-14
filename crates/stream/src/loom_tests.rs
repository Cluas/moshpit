//! Model checks of the wake protocol under every interleaving loom finds:
//!
//! ```text
//! RUSTFLAGS="--cfg moshpit_loom" cargo test -p moshpit-stream --lib --release
//! ```

use std::{
    future::Future,
    mem::ManuallyDrop,
    pin::pin,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use loom::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
    },
    thread,
};

use crate::{InputEvent, shared::Shared};

fn model(f: impl Fn() + Sync + Send + 'static) {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(3);
    builder.check(f);
}

/// A wake a thread blocks on.
#[derive(Default)]
struct Bell {
    rung: Mutex<bool>,
    condvar: Condvar,
}

impl Bell {
    fn ring(&self) {
        *self.rung.lock().unwrap() = true;
        self.condvar.notify_one();
    }

    fn wait(&self) {
        let mut rung = self.rung.lock().unwrap();
        while !*rung {
            rung = self.condvar.wait(rung).unwrap();
        }
        *rung = false;
    }
}

/// Drives `future` to completion on this thread, blocking between polls
/// until its waker is called. Unlike `loom::future::block_on` it never
/// wakes spuriously, so a lost wake shows: the thread blocks for good and
/// loom reports the deadlock.
fn block_on<F: Future>(future: F) -> F::Output {
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop_waker);
    fn bell(data: *const ()) -> ManuallyDrop<Arc<Bell>> {
        // SAFETY: `data` came from `Arc::into_raw` below.
        ManuallyDrop::new(unsafe { Arc::from_raw(data.cast::<Bell>()) })
    }
    unsafe fn clone(data: *const ()) -> RawWaker {
        let _ = ManuallyDrop::new(Arc::clone(&bell(data)));
        RawWaker::new(data, &VTABLE)
    }
    unsafe fn wake(data: *const ()) {
        ManuallyDrop::into_inner(bell(data)).ring();
    }
    unsafe fn wake_by_ref(data: *const ()) {
        bell(data).ring();
    }
    unsafe fn drop_waker(data: *const ()) {
        drop(ManuallyDrop::into_inner(bell(data)));
    }

    let bell = Arc::new(Bell::default());
    let data = Arc::into_raw(bell.clone()).cast::<()>();
    // SAFETY: the vtable keeps the `Arc`'s count.
    let waker = unsafe { Waker::from_raw(RawWaker::new(data, &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        bell.wait();
    }
}

fn counting(count: Arc<AtomicUsize>) -> crate::shared::Waker {
    Box::new(move || {
        count.fetch_add(1, SeqCst);
    })
}

/// A view that drains after every wake never leaves output behind: every
/// push after a drain finds no wake outstanding and sends one.
#[test]
fn no_lost_wake() {
    model(|| {
        let shared = Arc::new(Shared::new());
        let wakes = Arc::new(AtomicUsize::new(0));
        shared.attach(counting(wakes.clone()));
        let done = Arc::new(AtomicBool::new(false));
        let producer = {
            let (shared, done) = (shared.clone(), done.clone());
            thread::spawn(move || {
                shared.push(b"a");
                shared.push(b"bc");
                done.store(true, SeqCst);
            })
        };

        let mut seen = 0;
        let mut got = Vec::new();
        loop {
            let now = wakes.load(SeqCst);
            if now > seen {
                seen = now;
                // A one-byte buffer: partial reads leave the wake out.
                let mut buf = [0u8; 1];
                loop {
                    let n = shared.read(&mut buf);
                    got.extend_from_slice(&buf[..n]);
                    if n < buf.len() {
                        break;
                    }
                }
                continue;
            }
            if done.load(SeqCst) && wakes.load(SeqCst) == seen {
                break;
            }
            thread::yield_now();
        }
        producer.join().unwrap();
        assert_eq!(got, b"abc");
        let stats = shared.stats();
        assert_eq!(stats.wakes, stats.drains, "a second wake before a drain");
    });
}

/// Once detach returns the waker is never called, whatever a push on
/// another thread was doing.
#[test]
fn no_wake_after_detach() {
    model(|| {
        let shared = Arc::new(Shared::new());
        let detached = Arc::new(AtomicBool::new(false));
        let d = detached.clone();
        shared.attach(Box::new(move || {
            assert!(!d.load(SeqCst), "woken after detach returned");
        }));
        let producer = {
            let shared = shared.clone();
            thread::spawn(move || {
                shared.push(b"x");
                shared.push(b"y");
            })
        };
        shared.detach();
        detached.store(true, SeqCst);
        producer.join().unwrap();
    });
}

/// A wake decided for an earlier attachment never reaches the next waker,
/// and the next waker is woken once for output already waiting.
#[test]
fn a_new_waker_is_woken_once() {
    model(|| {
        let shared = Arc::new(Shared::new());
        let (old, new) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        shared.attach(counting(old.clone()));
        let producer = {
            let shared = shared.clone();
            thread::spawn(move || shared.push(b"x"))
        };
        shared.attach(counting(new.clone()));
        producer.join().unwrap();
        assert_eq!(new.load(SeqCst), 1);
        assert!(old.load(SeqCst) <= 1);
    });
}

/// The first attach racing the first push: one wake, never none or two.
#[test]
fn attach_racing_a_push_wakes_once() {
    model(|| {
        let shared = Arc::new(Shared::new());
        let wakes = Arc::new(AtomicUsize::new(0));
        let producer = {
            let shared = shared.clone();
            thread::spawn(move || shared.push(b"x"))
        };
        shared.attach(counting(wakes.clone()));
        producer.join().unwrap();
        assert_eq!(wakes.load(SeqCst), 1);
    });
}

/// The core's task waiting for input is woken by a write on another thread
/// (a lost wake leaves `block_on` blocked for good, which loom reports).
#[test]
fn a_write_wakes_the_core() {
    model(|| {
        let shared = Arc::new(Shared::new());
        let writer = {
            let shared = shared.clone();
            thread::spawn(move || assert!(shared.write(b"k")))
        };
        let event = block_on(std::future::poll_fn(|cx| shared.poll_input(cx)));
        assert_eq!(event, Some(InputEvent::Bytes(b"k".to_vec())));
        writer.join().unwrap();
    });
}

/// …and by a resize: each call that queues input wakes on its own.
#[test]
fn a_resize_wakes_the_core() {
    model(|| {
        let shared = Arc::new(Shared::new());
        let writer = {
            let shared = shared.clone();
            thread::spawn(move || assert!(shared.resize(80, 24)))
        };
        let event = block_on(std::future::poll_fn(|cx| shared.poll_input(cx)));
        assert_eq!(event, Some(InputEvent::Resize { cols: 80, rows: 24 }));
        writer.join().unwrap();
    });
}

/// …and by the core's end closing, which ends the input.
#[test]
fn a_close_wakes_the_core() {
    model(|| {
        let shared = Arc::new(Shared::new());
        let closer = {
            let shared = shared.clone();
            thread::spawn(move || {
                shared.close();
            })
        };
        let end = block_on(std::future::poll_fn(|cx| shared.poll_input(cx)));
        assert_eq!(end, None);
        closer.join().unwrap();
    });
}
