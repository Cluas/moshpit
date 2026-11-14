//! Spike G4's evidence from the core's side: what each stream's core end
//! took from the view, in the order it took it. The host compares it with
//! what it typed (keystrokes in order, ⌘ shortcuts typing nothing) and with
//! the terminal's grid after a window resize (the core heard the new size).
//!
//! The pane tasks and the echo record every input event here as they take
//! it, before they act on it; nothing else reads or changes the stream.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use moshpit_stream::{InputEvent, StreamId};

/// What one stream's core end took.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Taken {
    /// Every input byte, in order.
    pub bytes: Vec<u8>,
    /// Input events that carried bytes.
    pub chunks: u64,
    /// Every resize, in order: columns, rows, and when the core took it
    /// (the host's monotonic clock, nanoseconds; 0 off Apple platforms).
    pub resizes: Vec<(u16, u16, u64)>,
}

static TAKEN: LazyLock<Mutex<HashMap<StreamId, Taken>>> = LazyLock::new(Mutex::default);

/// The host's monotonic clock in nanoseconds: `DispatchTime.now()
/// .uptimeNanoseconds` reads the same one.
fn now_nanos() -> u64 {
    #[cfg(target_vendor = "apple")]
    {
        // `mach_absolute_time` in nanoseconds, as DispatchTime converts it.
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: writes the clock's value into `ts`, which outlives the call.
        unsafe { libc::clock_gettime(libc::CLOCK_UPTIME_RAW, &mut ts) };
        ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        0
    }
}

/// Records `event` as taken by stream `id`'s core end.
pub fn record(id: StreamId, event: &InputEvent) {
    let mut taken = TAKEN.lock().unwrap_or_else(|e| e.into_inner());
    let entry = taken.entry(id).or_default();
    match event {
        InputEvent::Bytes(bytes) => {
            entry.bytes.extend_from_slice(bytes);
            entry.chunks += 1;
        }
        InputEvent::Resize { cols, rows } => entry.resizes.push((*cols, *rows, now_nanos())),
        // The spike's views never paste through the stream.
        InputEvent::Paste(_) | InputEvent::Bar(_) => {}
    }
}

/// What stream `id`'s core end took so far (empty if nothing).
pub fn taken(id: StreamId) -> Taken {
    TAKEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned()
        .unwrap_or_default()
}

/// Runs `f` on what stream `id`'s core end took so far, without a copy.
pub fn with_taken<R>(id: StreamId, f: impl FnOnce(Option<&Taken>) -> R) -> R {
    f(TAKEN.lock().unwrap_or_else(|e| e.into_inner()).get(&id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshpit_stream::view;
    use std::sync::Arc;

    #[test]
    fn the_shell_records_bytes_and_resizes_in_the_order_it_took_them() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let (stream, input) = moshpit_stream::open();
        let stream = Arc::new(stream);
        let id = stream.id();
        runtime.block_on(async {
            let task = tokio::spawn(crate::panes::shell(stream.clone(), input, 3));
            assert!(view::write(id, b"ab"));
            assert!(view::resize(id, 80, 24));
            assert!(view::write(id, b"c"));
            assert!(view::write(id, b"\r"));
            assert!(view::resize(id, 100, 30));
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let taken = taken(id);
            assert_eq!(taken.bytes, b"abc\r");
            assert_eq!(taken.chunks, 2, "bytes between two resizes are one chunk");
            let sizes: Vec<(u16, u16)> = taken.resizes.iter().map(|&(c, r, _)| (c, r)).collect();
            assert_eq!(sizes, [(80, 24), (100, 30)]);
            task.abort();
        });
    }

    #[test]
    fn a_stream_that_took_nothing_has_nothing() {
        assert_eq!(taken(u64::MAX), Taken::default());
        assert!(with_taken(u64::MAX, |t| t.is_none()));
    }
}
