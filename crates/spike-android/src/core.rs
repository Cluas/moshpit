//! The spike's core: one pane whose "remote" is an echo, on the core's own
//! multi-threaded Tokio runtime (which NSH shares, `bridge::run_rust`).
//!
//! The pane's byte channel (crates/stream) carries everything between the
//! core and the terminal view: the view writes keystrokes, the core echoes
//! them ([`crate::echo`]); a burst ([`burst`]) and numbered lines
//! ([`lines`]) are pushed on request of the test hooks. The UI reads a
//! [`Status`] from a watch channel.

use std::{
    sync::{Arc, LazyLock, Mutex, OnceLock},
    time::Instant,
};

use moshpit_stream::{InputEvent, Stream, StreamId};
use tokio::sync::watch;

use crate::{
    alog,
    burst::{self, Fnv64, Rng},
    echo::{self, LineEcho},
};

/// The core's runtime: two workers, IO and time on.
pub static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("moshpit-core")
        .enable_all()
        .build()
        .expect("the core's runtime")
});

/// What the status line shows.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub stream: StreamId,
    pub cols: u16,
    pub rows: u16,
    pub chunks_in: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// The last input chunk, readable.
    pub last: String,
    /// The burst's state, when one ran.
    pub burst: String,
}

impl Status {
    pub fn line(&self) -> String {
        let size = if self.cols == 0 {
            "no size yet".to_owned()
        } else {
            format!("{}×{}", self.cols, self.rows)
        };
        let mut line = format!(
            "pane #{} · {size} · in {} B / {} chunks · out {} B",
            self.stream, self.bytes_in, self.chunks_in, self.bytes_out
        );
        if !self.last.is_empty() {
            line.push_str(" · last ");
            line.push_str(&self.last);
        }
        if !self.burst.is_empty() {
            line.push('\n');
            line.push_str(&self.burst);
        }
        line
    }
}

/// The one pane of the spike.
pub struct Pane {
    pub id: StreamId,
    stream: Arc<Stream>,
    status: watch::Sender<Status>,
}

static PANE: OnceLock<Pane> = OnceLock::new();

/// The pane, opened (and its echo started) on first use.
pub fn pane() -> &'static Pane {
    PANE.get_or_init(|| {
        let (stream, input) = moshpit_stream::open();
        let id = stream.id();
        let stream = Arc::new(stream);
        let (status, _) = watch::channel(Status {
            stream: id,
            ..Status::default()
        });
        stream.push(b"\x1b[1mMoshpit G3\x1b[0m: Termux view in NSH, echo core in Rust\r\n");
        stream.push(echo::PROMPT);
        RUNTIME.spawn(echo_loop(stream.clone(), input, status.clone()));
        alog::info(format!("pane: stream {id} open, echo running"));
        Pane { id, stream, status }
    })
}

impl Pane {
    pub fn status(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
    }
}

async fn echo_loop(
    stream: Arc<Stream>,
    mut input: moshpit_stream::Input,
    status: watch::Sender<Status>,
) {
    let mut line = LineEcho::new();
    let mut seq = 0u64;
    while let Some(event) = input.next().await {
        match event {
            InputEvent::Bytes(bytes) => {
                seq += 1;
                let described = echo::describe(&bytes);
                let mut shown = described.clone();
                if shown.chars().count() > 400 {
                    shown = shown.chars().take(400).collect::<String>() + "…";
                }
                let hex = echo::hex(&bytes[..bytes.len().min(256)]);
                alog::info(format!("in #{seq} {}B hex={hex} keys={shown}", bytes.len()));
                let out = line.feed(&bytes);
                stream.push(&out);
                status.send_modify(|s| {
                    s.chunks_in = seq;
                    s.bytes_in += bytes.len() as u64;
                    s.bytes_out += out.len() as u64;
                    s.last = described.chars().take(40).collect();
                });
            }
            InputEvent::Resize { cols, rows } => {
                alog::info(format!("resize {cols}x{rows}"));
                status.send_modify(|s| {
                    s.cols = cols;
                    s.rows = rows;
                });
            }
            // The spike's view never pastes through the stream.
            InputEvent::Paste(_) => {}
        }
    }
    alog::info("pane: input ended");
}

/// The burst the view checks: its length, hash and chunk count, published
/// before its last chunk is pushed.
pub static BURST: Mutex<Option<(u64, u64, u64)>> = Mutex::new(None);

/// Pushes `total` bytes of [`burst::Text`] in random chunk sizes, as fast as
/// the runtime goes.
pub fn burst(total: u64, seed: u64) {
    *BURST.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let pane = pane();
    let stream = pane.stream.clone();
    let status = pane.status.clone();
    RUNTIME.spawn(async move {
        let mut text = burst::Text::new(seed);
        let mut rng = Rng::new(seed);
        let mut hash = Fnv64::default();
        let mut chunk = Vec::with_capacity(1 << 16);
        let (mut sent, mut chunks) = (0u64, 0u64);
        let before = stream.stats();
        let start = Instant::now();
        alog::info(format!("burst: {total} B, seed {seed}"));
        while sent < total {
            let size = (burst::chunk_size(&mut rng) as u64).min(total - sent);
            chunk.clear();
            text.fill(&mut chunk, size as usize);
            hash.update(&chunk);
            sent += size;
            chunks += 1;
            if sent == total {
                *BURST.lock().unwrap_or_else(|e| e.into_inner()) = Some((sent, hash.0, chunks));
            }
            stream.push(&chunk);
            // Let the echo and NSH's tasks run on the shared runtime too.
            if chunks % 64 == 0 {
                tokio::task::yield_now().await;
            }
        }
        let pushed = start.elapsed();
        let after = stream.stats();
        let line = format!(
            "burst: pushed {sent} B in {chunks} chunks in {:.1} ms, hash {:016x}; core wakes {} drains {} peak queued {} B",
            pushed.as_secs_f64() * 1e3,
            hash.0,
            after.wakes - before.wakes,
            after.drains - before.drains,
            after.peak_queued,
        );
        alog::info(&line);
        status.send_modify(|s| {
            s.burst = format!(
                "burst {:.1} MB in {chunks} chunks pushed in {:.0} ms",
                sent as f64 / 1e6,
                pushed.as_secs_f64() * 1e3
            )
        });
    });
}

/// Pushes `count` numbered lines: scrollback for the scroll test.
pub fn lines(count: u32) {
    let pane = pane();
    let mut out = Vec::new();
    out.extend_from_slice(b"\r\n");
    for n in 1..=count {
        out.extend_from_slice(
            format!(
                "line {n:04} \x1b[2m{}\x1b[0m\r\n",
                "·".repeat((n % 40) as usize)
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(echo::PROMPT);
    alog::info(format!("lines: {count} ({} B)", out.len()));
    pane.stream.push(&out);
}
