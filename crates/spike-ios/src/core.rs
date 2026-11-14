//! The spike's core: what the real core will own, cut down to the byte
//! path. One tokio runtime (NSH reuses it for the UI's tasks), one stream
//! for the terminal, a task echoing its input straight back to its output,
//! and the bench's burst producer; for G5, the panes the terminal screen
//! shows ([`crate::panes`]).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

use moshpit_stream::{Input, InputEvent, Stream, StreamId};

use crate::{
    burst::{FNV_OFFSET, Generator, Rng, fnv1a},
    panes,
};

/// How many lines of history the G5 shell pane starts with.
pub const SHELL_HISTORY: usize = 3000;

/// What the burst producer pushed, recorded before its last chunk so it is
/// there by the time the view has read everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BurstReport {
    pub bytes: u64,
    pub chunks: u64,
    pub hash: u64,
    /// From the first push to the last.
    pub push_nanos: u64,
}

pub struct Core {
    runtime: tokio::runtime::Runtime,
    terminal: Arc<Stream>,
    burst: Arc<Mutex<Option<BurstReport>>>,
    /// G5's panes by number, opened on first ask.
    panes: Mutex<HashMap<u32, Arc<Stream>>>,
}

static CORE: OnceLock<Core> = OnceLock::new();

/// The process's core, started on first use.
pub fn core() -> &'static Core {
    CORE.get_or_init(Core::start)
}

impl Core {
    fn start() -> Core {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("moshpit-core")
            .on_thread_start(core_thread_qos)
            .enable_all()
            .build()
            .expect("the core's runtime starts");
        let (terminal, input) = moshpit_stream::open();
        let terminal = Arc::new(terminal);
        runtime.spawn(echo(terminal.clone(), input));
        let qos = if std::env::args().any(|a| a == "-MOSHPIT_SPIKE_QOS_DEFAULT") {
            "default"
        } else {
            "user-interactive"
        };
        log::info!(
            target: "moshpit",
            "core up: stream {} echoing, 2 workers at {qos} QoS",
            terminal.id()
        );
        Core {
            runtime,
            terminal,
            burst: Arc::default(),
            panes: Mutex::default(),
        }
    }

    /// G5's pane `number`'s stream, opened on first ask: pane 1 is the
    /// shell with [`SHELL_HISTORY`] lines of history, any other the
    /// full-screen TUI.
    pub fn pane(&self, number: u32) -> StreamId {
        let mut panes = self.panes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(stream) = panes.get(&number) {
            return stream.id();
        }
        let (stream, input) = moshpit_stream::open();
        let stream = Arc::new(stream);
        if number == 1 {
            self.runtime
                .spawn(panes::shell(stream.clone(), input, SHELL_HISTORY));
        } else {
            self.runtime.spawn(panes::tui(stream.clone(), input));
        }
        log::info!(target: "moshpit", "g5 pane {number}: stream {}", stream.id());
        let id = stream.id();
        panes.insert(number, stream);
        id
    }

    /// Pushes `text` as a line of output to pane `number`'s stream (the
    /// director's marker, sent while the pane is off screen).
    pub fn pane_say(&self, number: u32, text: &str) {
        let panes = self.panes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(stream) = panes.get(&number) {
            let mut out = format!("\r\x1b[K{text}\r\n").into_bytes();
            out.extend_from_slice(panes::PROMPT);
            stream.push(&out);
        }
    }

    /// The runtime, for NSH to run the UI's tasks on too.
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    /// The terminal's stream.
    pub fn terminal(&self) -> StreamId {
        self.terminal.id()
    }

    /// Starts pushing `total` bytes of the burst (seed `seed`) to the
    /// terminal's stream, as fast as the producer can make them.
    pub fn start_burst(&self, total: u64, seed: u64) {
        *self.burst.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let stream = self.terminal.clone();
        let report = self.burst.clone();
        self.runtime.spawn(async move {
            let result = burst(&stream, total, seed, |r| {
                *report.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            })
            .await;
            log::info!(target: "moshpit", "burst pushed: {result:?}");
        });
    }

    /// The burst's report, once its producer reached its last chunk.
    pub fn burst_report(&self) -> Option<BurstReport> {
        *self.burst.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The core's threads carry keystrokes and their echoes, so they run at the
/// main thread's quality of service. On the iPhone 16 Pro that took the
/// keystroke-to-waker p99 from about 630 us (default QoS) to about 150 us.
/// `-MOSHPIT_SPIKE_QOS_DEFAULT` leaves them at the default, to compare.
fn core_thread_qos() {
    #[cfg(target_vendor = "apple")]
    if !std::env::args().any(|a| a == "-MOSHPIT_SPIKE_QOS_DEFAULT") {
        // SAFETY: sets the calling thread's own QoS class; no pointers.
        unsafe {
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0)
        };
    }
}

/// Echoes every input chunk straight back to the output: the spike's
/// stand-in for the host's echo.
pub async fn echo(stream: Arc<Stream>, mut input: Input) {
    while let Some(event) = input.next().await {
        crate::probe::record(stream.id(), &event);
        if let InputEvent::Bytes(bytes) = event {
            stream.push(&bytes);
        }
    }
}

/// Pushes `total` bytes of the burst to `stream` in ragged chunks, hashing
/// them in order; hands the report to `report` before the last push.
pub async fn burst(
    stream: &Stream,
    total: u64,
    seed: u64,
    report: impl FnOnce(BurstReport),
) -> BurstReport {
    let mut generator = Generator::new(seed);
    let mut sizes = Rng::new(seed.rotate_left(17));
    let mut chunk = vec![0u8; 1 << 16];
    let mut result = BurstReport {
        bytes: 0,
        chunks: 0,
        hash: FNV_OFFSET,
        push_nanos: 0,
    };
    let mut report = Some(report);
    let mut since_yield = 0usize;
    let started = Instant::now();
    loop {
        let n = (sizes.chunk_size() as u64).min(total - result.bytes) as usize;
        generator.fill(&mut chunk[..n]);
        result.hash = fnv1a(result.hash, &chunk[..n]);
        result.bytes += n as u64;
        result.chunks += u64::from(n > 0);
        result.push_nanos = started.elapsed().as_nanos() as u64;
        let last = result.bytes == total;
        if last && let Some(report) = report.take() {
            report(result);
        }
        stream.push(&chunk[..n]);
        if last {
            return result;
        }
        // Let the runtime's other tasks in now and then.
        since_yield += n;
        if since_yield >= 1 << 20 {
            since_yield = 0;
            tokio::task::yield_now().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshpit_stream::view;

    #[test]
    fn the_burst_reaches_a_view_whole_and_reports_before_its_last_byte() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .build()
            .unwrap();
        let (stream, _input) = moshpit_stream::open();
        let id = stream.id();
        let total = 3 << 20;
        let reported = Arc::new(Mutex::new(None));
        let r = reported.clone();
        let pushed = runtime.block_on(burst(&stream, total, 5, move |report| {
            *r.lock().unwrap() = Some(report);
        }));
        assert_eq!(pushed.bytes, total);
        assert_eq!(*reported.lock().unwrap(), Some(pushed));

        let mut hash = FNV_OFFSET;
        let mut received = 0u64;
        let mut buf = vec![0u8; 16 << 10];
        loop {
            let n = view::read(id, &mut buf);
            hash = fnv1a(hash, &buf[..n]);
            received += n as u64;
            if n < buf.len() {
                break;
            }
        }
        assert_eq!(received, total);
        assert_eq!(hash, pushed.hash);
    }

    #[test]
    fn the_echo_task_returns_every_chunk_in_order() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .build()
            .unwrap();
        let (stream, input) = moshpit_stream::open();
        let stream = Arc::new(stream);
        let id = stream.id();
        runtime.spawn(echo(stream.clone(), input));
        let (tx, rx) = std::sync::mpsc::channel();
        view::attach(id, move || {
            let _ = tx.send(());
        });
        let mut echoed = Vec::new();
        let mut buf = [0u8; 256];
        for key in b'a'..=b'z' {
            assert!(view::write(id, &[key]));
            while !echoed.ends_with(&[key]) {
                rx.recv_timeout(std::time::Duration::from_secs(5))
                    .expect("the echo wakes the view");
                loop {
                    let n = view::read(id, &mut buf);
                    echoed.extend_from_slice(&buf[..n]);
                    if n < buf.len() {
                        break;
                    }
                }
            }
        }
        assert_eq!(echoed, (b'a'..=b'z').collect::<Vec<_>>());
        view::detach(id);
    }
}
