//! Micro-bench of the byte channel on this machine:
//!
//! ```text
//! cargo bench -p moshpit-stream
//! ```
//!
//! 1. Op cost: one push plus one read of a few bytes, uncontended.
//! 2. Throughput: a producer thread pushing ragged chunks (1 B to 64 KB), a
//!    view thread draining on wakes, as the device bench does.
//! 3. Echo round trip: a write from a "view" thread, a tokio task echoing it
//!    to the output, the wake back to the view thread, the read. What the
//!    channel adds to a keystroke, minus the main-thread hop of the device.
//!
//! Without `--bench` (as `cargo test --all-targets` runs it) every part runs
//! a few iterations only.

use std::{
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

use moshpit_stream::{InputEvent, open, view};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

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

    fn wait(&self) {
        let woken = self.woken.lock().unwrap();
        let mut woken = self.condvar.wait_while(woken, |w| !*w).unwrap();
        *woken = false;
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn op_cost(iterations: u32) {
    let (stream, _input) = open();
    let id = stream.id();
    view::attach(id, || {});
    let mut buf = [0u8; 64];
    let started = Instant::now();
    for _ in 0..iterations {
        stream.push(b"abcd");
        let n = view::read(id, &mut buf);
        assert_eq!(n, 4);
    }
    let per = started.elapsed() / iterations;
    println!("op cost: push 4 B + read (wake each time): {per:?} per pair");
    view::detach(id);
}

fn throughput(total: usize) {
    let (stream, _input) = open();
    let id = stream.id();
    let signal = Arc::new(Signal::default());
    let s = signal.clone();
    view::attach(id, move || s.wake());
    let started = Instant::now();
    let producer = thread::spawn(move || {
        let mut rng = Rng(0x5eed);
        let chunk = vec![b'x'; 1 << 16];
        let mut sent = 0;
        while sent < total {
            let size = ((1usize << (rng.next() % 17)) + (rng.next() as usize % 64))
                .clamp(1, 1 << 16)
                .min(total - sent);
            stream.push(&chunk[..size]);
            sent += size;
        }
        stream
    });
    let mut buf = vec![0u8; 1 << 16];
    let mut received = 0;
    while received < total {
        signal.wait();
        loop {
            let n = view::read(id, &mut buf);
            received += n;
            if n < buf.len() {
                break;
            }
        }
    }
    let elapsed = started.elapsed();
    let stream = producer.join().unwrap();
    let stats = stream.stats();
    println!(
        "throughput: {} MB in {elapsed:?} = {:.0} MB/s; {} pushes, {} wakes, peak {:.1} MB queued",
        total >> 20,
        total as f64 / elapsed.as_secs_f64() / 1e6,
        stats.pushes,
        stats.wakes,
        stats.peak_queued as f64 / 1e6
    );
}

fn echo_round_trip(rounds: usize) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .build()
        .unwrap();
    let (stream, mut input) = open();
    let id = stream.id();
    runtime.spawn(async move {
        while let Some(event) = input.next().await {
            if let InputEvent::Bytes(bytes) = event {
                stream.push(&bytes);
            }
        }
    });
    let signal = Arc::new(Signal::default());
    let s = signal.clone();
    view::attach(id, move || s.wake());
    let mut times = Vec::with_capacity(rounds);
    let mut buf = [0u8; 16];
    for i in 0..rounds {
        // A pause as between keystrokes, so the workers park.
        thread::sleep(Duration::from_micros(200));
        let key = [b'a' + (i % 26) as u8];
        let started = Instant::now();
        assert!(view::write(id, &key));
        signal.wait();
        let n = view::read(id, &mut buf);
        times.push(started.elapsed());
        assert_eq!(&buf[..n], &key);
    }
    times.sort();
    println!(
        "echo round trip over {rounds}: p50 {:?} p95 {:?} p99 {:?} max {:?}",
        percentile(&times, 0.50),
        percentile(&times, 0.95),
        percentile(&times, 0.99),
        times[times.len() - 1]
    );
    view::detach(id);
}

fn main() {
    let full = std::env::args().any(|a| a == "--bench");
    if full {
        op_cost(2_000_000);
        throughput(512 << 20);
        echo_round_trip(20_000);
    } else {
        op_cost(1_000);
        throughput(1 << 20);
        echo_round_trip(50);
    }
}
