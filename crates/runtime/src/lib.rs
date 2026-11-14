//! The Rust core's tokio runtime: one per process, started the first time
//! something asks for it (`docs/nsh/DESIGN.md` §3.2, 线程).
//!
//! NSH runs the UI's async tasks on it too: an NSH surface is handed
//! [`handle`] (`SurfaceOptions::runtime`), so the app has one set of worker
//! threads, not the core's and NSH's own. Nothing starts it but a caller:
//! an app that never shows an NSH screen nor calls the async core starts
//! no threads.
//!
//! Its threads carry keystrokes and their echoes, so on Apple platforms
//! they run at the main thread's quality of service, user-interactive.
//! Spike G1 measured why on the iPhone 16 Pro: at the default QoS the
//! keystroke-to-waker p99 was about 630 µs against about 150 µs, and the
//! echo's p95 0.7 ms higher with a tail to 8 ms. NSH raises its own engine
//! thread the same way.

use std::sync::OnceLock;

use tokio::runtime::{Handle, Runtime};

/// The worker threads: two, as in the spikes. The core's work is IO and
/// parsing, a few connections at a time; the app's main thread and NSH's
/// engine thread do the drawing.
pub const WORKER_THREADS: usize = 2;

/// What its threads are called (in a crash report, in Instruments).
pub const THREAD_NAME: &str = "moshpit-core";

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// The process's runtime, started on first use.
pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(start)
}

/// A handle to [`runtime`], for NSH and for spawning from any thread.
pub fn handle() -> Handle {
    runtime().handle().clone()
}

/// Whether the runtime has started (for tests and diagnostics: asking for
/// it starts it).
pub fn started() -> bool {
    RUNTIME.get().is_some()
}

fn start() -> Runtime {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKER_THREADS)
        .thread_name(THREAD_NAME)
        .on_thread_start(raise_thread_priority)
        .enable_all()
        .build()
        .expect("the core's runtime starts");
    log::info!(
        target: "moshpit",
        "core runtime up: {WORKER_THREADS} workers at {} QoS",
        if cfg!(target_vendor = "apple") { "user-interactive" } else { "the default" }
    );
    runtime
}

/// Runs on every thread the runtime starts, workers and blocking threads
/// alike: the main thread's QoS on Apple platforms (the module doc).
fn raise_thread_priority() {
    #[cfg(target_vendor = "apple")]
    // SAFETY: sets the calling thread's own QoS class; no pointers.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use super::*;

    /// What `f` returns when run on one of the runtime's workers.
    fn on_worker<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        handle().spawn(async move {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(10))
            .expect("the runtime runs the task")
    }

    #[test]
    fn there_is_one_runtime_and_asking_starts_it() {
        let first = runtime() as *const Runtime;
        assert!(started());
        assert_eq!(first, runtime() as *const Runtime);
        assert_eq!(handle().metrics().num_workers(), WORKER_THREADS);
    }

    #[test]
    fn tasks_run_on_the_named_worker_threads() {
        let name = on_worker(|| std::thread::current().name().map(str::to_owned));
        assert_eq!(name.as_deref(), Some(THREAD_NAME));
    }

    #[test]
    fn timers_run() {
        let (tx, rx) = mpsc::channel();
        handle().spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(10))
            .expect("the time driver is enabled");
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn its_threads_run_at_user_interactive_qos() {
        // The class as its number (`qos_class_t` has no `PartialEq`).
        fn qos() -> u32 {
            let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
            let mut relative = 0;
            // SAFETY: reads the calling thread's QoS into the two locals.
            let status = unsafe {
                libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative)
            };
            assert_eq!(status, 0);
            class as u32
        }
        let interactive = libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE as u32;
        assert_eq!(on_worker(qos), interactive);
        // The blocking pool's threads too.
        let (tx, rx) = mpsc::channel();
        handle().spawn_blocking(move || {
            let _ = tx.send(qos());
        });
        let blocking = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(blocking, interactive);
    }
}
