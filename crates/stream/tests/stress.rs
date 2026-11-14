//! Many threads at once: ordering and no loss with concurrent producers and
//! one consumer, in both directions, and exactly one wake per drain.
#![cfg(not(moshpit_loom))]

use std::{
    sync::{Arc, Condvar, Mutex},
    thread,
    time::Duration,
};

use moshpit_stream::{InputEvent, open, view};

const PRODUCERS: usize = 8;
const RECORDS: u32 = if cfg!(debug_assertions) { 1500 } else { 6000 };
/// How long a consumer waits for a wake before calling it lost.
const PATIENCE: Duration = Duration::from_secs(20);

/// xorshift64*: fast, deterministic, good enough for sizes.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn payload_byte(producer: usize, seq: u32, i: usize) -> u8 {
    (producer as u32 * 31 + seq * 7 + i as u32) as u8
}

/// A record: producer, sequence number, payload length, payload. Pushed or
/// written with one call, so it must arrive whole and in its producer's
/// order.
fn record(producer: usize, seq: u32, rng: &mut Rng) -> Vec<u8> {
    let len = if rng.below(20) == 0 {
        rng.below(60_000) as usize
    } else {
        rng.below(256) as usize
    };
    let mut bytes = Vec::with_capacity(7 + len);
    bytes.push(producer as u8);
    bytes.extend_from_slice(&seq.to_le_bytes());
    bytes.extend_from_slice(&(len as u16).to_le_bytes());
    bytes.extend((0..len).map(|i| payload_byte(producer, seq, i)));
    bytes
}

/// Reassembles records from arbitrary slices and checks each.
struct Records {
    pending: Vec<u8>,
    next_seq: Vec<u32>,
    received: usize,
    bytes: u64,
}

impl Records {
    fn new() -> Records {
        Records {
            pending: Vec::new(),
            next_seq: vec![0; PRODUCERS],
            received: 0,
            bytes: 0,
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.bytes += bytes.len() as u64;
        self.pending.extend_from_slice(bytes);
        let mut at = 0;
        while self.pending.len() - at >= 7 {
            let head = &self.pending[at..];
            let producer = head[0] as usize;
            assert!(producer < PRODUCERS, "a record's header is torn");
            let seq = u32::from_le_bytes(head[1..5].try_into().unwrap());
            let len = u16::from_le_bytes(head[5..7].try_into().unwrap()) as usize;
            if head.len() < 7 + len {
                break;
            }
            assert_eq!(
                seq, self.next_seq[producer],
                "producer {producer}'s records out of order or lost"
            );
            for (i, &b) in head[7..7 + len].iter().enumerate() {
                assert_eq!(
                    b,
                    payload_byte(producer, seq, i),
                    "producer {producer} record {seq} torn at byte {i}"
                );
            }
            self.next_seq[producer] += 1;
            self.received += 1;
            at += 7 + len;
        }
        self.pending.drain(..at);
    }

    fn complete(&self) -> bool {
        self.received == PRODUCERS * RECORDS as usize
    }
}

/// A wake the consumer thread waits for.
#[derive(Default)]
struct Signal {
    woken: Mutex<bool>,
    condvar: Condvar,
}

impl Signal {
    fn wake(&self) {
        *self.woken.lock().unwrap() = true;
        self.condvar.notify_one();
    }

    /// Waits for a wake; `false` if none came in time.
    fn wait(&self, timeout: Duration) -> bool {
        let woken = self.woken.lock().unwrap();
        let (mut woken, result) = self
            .condvar
            .wait_timeout_while(woken, timeout, |w| !*w)
            .unwrap();
        if result.timed_out() {
            return false;
        }
        *woken = false;
        true
    }
}

#[test]
fn concurrent_producers_one_view_no_loss_no_reordering_one_wake_per_drain() {
    let (stream, _input) = open();
    let stream = Arc::new(stream);
    let id = stream.id();
    let signal = Arc::new(Signal::default());
    let s = signal.clone();
    assert!(view::attach(id, move || s.wake()));

    let consumer = thread::spawn(move || {
        let mut records = Records::new();
        let mut rng = Rng::new(99);
        let mut buf = vec![0; 70_000];
        while !records.complete() {
            if !signal.wait(PATIENCE) {
                let stats = view::stats(id).unwrap();
                panic!(
                    "no wake for {PATIENCE:?} with {} bytes queued",
                    stats.queued
                );
            }
            // Drain with ragged reads, as a view with a budget would.
            loop {
                let cap = 1 + rng.below(buf.len() as u64) as usize;
                let n = view::read(id, &mut buf[..cap]);
                records.feed(&buf[..n]);
                if n < cap {
                    break;
                }
            }
        }
        records
    });

    let producers: Vec<_> = (0..PRODUCERS)
        .map(|producer| {
            let stream = stream.clone();
            thread::spawn(move || {
                let mut rng = Rng::new(producer as u64 + 1);
                let mut bytes = 0u64;
                for seq in 0..RECORDS {
                    let record = record(producer, seq, &mut rng);
                    bytes += record.len() as u64;
                    stream.push(&record);
                    if rng.below(64) == 0 {
                        thread::yield_now();
                    }
                }
                bytes
            })
        })
        .collect();
    let pushed: u64 = producers.into_iter().map(|p| p.join().unwrap()).sum();
    let records = consumer.join().unwrap();

    assert_eq!(records.bytes, pushed);
    assert!(records.pending.is_empty());
    let stats = stream.stats();
    assert_eq!(stats.pushed, pushed);
    assert_eq!(stats.read, pushed);
    assert_eq!(stats.queued, 0);
    // Wakes and drains alternate: never a second wake before a drain.
    assert_eq!(stats.wakes, stats.drains, "{stats:?}");
    assert!(stats.wakes >= 1);
    eprintln!(
        "{} pushes, {} bytes, {} wakes, peak {} bytes queued",
        stats.pushes, stats.pushed, stats.wakes, stats.peak_queued
    );
}

#[test]
fn concurrent_writers_one_core_task_no_loss_no_reordering() {
    const RESIZES: u16 = 2000;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .unwrap();
    let (stream, mut input) = open();
    let id = stream.id();

    let consumer = runtime.spawn(async move {
        let mut records = Records::new();
        let mut last_size = 0u16;
        while !records.complete() || last_size < RESIZES {
            let event = tokio::time::timeout(PATIENCE, input.next())
                .await
                .expect("a write was not woken")
                .expect("the input ended early");
            match event {
                InputEvent::Bytes(bytes) => {
                    assert!(!bytes.is_empty());
                    records.feed(&bytes);
                }
                InputEvent::Resize { cols, rows } => {
                    assert_eq!(cols, rows);
                    assert!(cols > last_size, "resize {cols} after {last_size}");
                    last_size = cols;
                }
                InputEvent::Paste(text) => panic!("nothing was pasted: {text:?}"),
            }
        }
        records
    });

    let resizer = thread::spawn(move || {
        for size in 1..=RESIZES {
            assert!(view::resize(id, size, size));
            if size % 16 == 0 {
                thread::yield_now();
            }
        }
    });
    let writers: Vec<_> = (0..PRODUCERS)
        .map(|producer| {
            thread::spawn(move || {
                let mut rng = Rng::new(1000 + producer as u64);
                let mut bytes = 0u64;
                for seq in 0..RECORDS {
                    let record = record(producer, seq, &mut rng);
                    bytes += record.len() as u64;
                    assert!(view::write(id, &record));
                }
                bytes
            })
        })
        .collect();
    let written: u64 = writers.into_iter().map(|w| w.join().unwrap()).sum();
    resizer.join().unwrap();
    let records = runtime.block_on(consumer).unwrap();
    assert_eq!(records.bytes, written);
    assert!(records.pending.is_empty());
    drop(stream);
}

/// FNV-1a, 64 bits: what the device bench hashes with on both sides.
fn fnv1a(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    })
}
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

#[test]
fn a_burst_in_ragged_chunks_arrives_byte_for_byte() {
    let total: usize = if cfg!(debug_assertions) {
        8 << 20
    } else {
        64 << 20
    };
    let (stream, _input) = open();
    let id = stream.id();
    let signal = Arc::new(Signal::default());
    let s = signal.clone();
    view::attach(id, move || s.wake());

    let producer = thread::spawn(move || {
        let mut rng = Rng::new(7);
        let mut hash = FNV_OFFSET;
        let mut chunk = vec![0u8; 1 << 16];
        let mut sent = 0;
        while sent < total {
            // Log-uniform sizes from 1 B to 64 KB: small ones are common.
            let size = (1usize << rng.below(17)).min(1 << 16);
            let size = (size / 2 + rng.below(size as u64 / 2 + 1) as usize)
                .clamp(1, 1 << 16)
                .min(total - sent);
            for b in &mut chunk[..size] {
                *b = rng.next() as u8;
            }
            hash = fnv1a(hash, &chunk[..size]);
            stream.push(&chunk[..size]);
            sent += size;
        }
        (stream, hash)
    });

    let mut hash = FNV_OFFSET;
    let mut received = 0;
    let mut rng = Rng::new(8);
    let mut buf = vec![0; 1 << 17];
    while received < total {
        assert!(signal.wait(PATIENCE), "lost wake at {received} bytes");
        loop {
            let cap = 1 + rng.below(buf.len() as u64) as usize;
            let n = view::read(id, &mut buf[..cap]);
            hash = fnv1a(hash, &buf[..n]);
            received += n;
            if n < cap {
                break;
            }
        }
    }
    let (stream, sent_hash) = producer.join().unwrap();
    assert_eq!(received, total);
    assert_eq!(hash, sent_hash);
    let stats = stream.stats();
    assert_eq!(stats.wakes, stats.drains);
}
