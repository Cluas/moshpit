//! What Moshpit's layer costs over the emulator alone (our fork of
//! wezterm-term, `advance_bytes`): `seq`-like output, a colour-dense stream
//! and CJK with emoji, in 4 KB reads, MB/s; and an echo: one typed
//! character fed and its frame taken, on a screen with 1 000 lines of
//! history, the median of 20 000.
//!
//!   cargo bench -p moshpit-term --bench feed

use std::{sync::Arc, time::Instant};

use moshpit_term::{Options, Terminal};
use wezterm_term::{TerminalConfiguration, TerminalSize, color::ColorPalette};

#[derive(Debug)]
struct Plain;
impl TerminalConfiguration for Plain {
    fn color_palette(&self) -> ColorPalette {
        ColorPalette::default()
    }
    fn scrollback_size(&self) -> usize {
        10_000
    }
    fn writer_thread(&self) -> bool {
        false
    }
}

fn workloads() -> Vec<(&'static str, Vec<u8>)> {
    let seq: Vec<u8> = (1..=200_000)
        .flat_map(|i: u32| format!("{i}\r\n").into_bytes())
        .collect();
    let mut color = Vec::new();
    for i in 0..20_000u32 {
        for w in 0..12u32 {
            color.extend_from_slice(
                format!("\x1b[38;5;{}mword{w}\x1b[0m ", (i + w) % 256).as_bytes(),
            );
        }
        color.extend_from_slice(b"\r\n");
    }
    let cjk: Vec<u8> = "终端滚动选区 😀✅ 输入法 é ñ ─│\r\n"
        .repeat(30_000)
        .into_bytes();
    vec![("seq", seq), ("colour", color), ("cjk+emoji", cjk)]
}

fn best(mut f: impl FnMut() -> f64) -> f64 {
    (0..5).map(|_| f()).fold(f64::MAX, f64::min)
}

fn echo() {
    let mut t = Terminal::new(80, 24, Options::default());
    let history: Vec<u8> = (0..1000)
        .flat_map(|i: u32| format!("\x1b[32m{i}\x1b[0m line of output\r\n").into_bytes())
        .collect();
    t.feed(&history);
    let _ = t.frame();
    let mut feed = Vec::with_capacity(20_000);
    let mut frame = Vec::with_capacity(20_000);
    for i in 0..20_000u32 {
        let c = [b'a' + (i % 26) as u8];
        let a = Instant::now();
        t.feed(&c);
        let b = Instant::now();
        let f = t.frame();
        let e = Instant::now();
        assert!(!f.changed.is_empty());
        feed.push((b - a).as_nanos());
        frame.push((e - b).as_nanos());
        if i % 70 == 69 {
            t.feed(b"\r\n");
            let _ = t.frame();
        }
    }
    feed.sort_unstable();
    frame.sort_unstable();
    let p = |v: &Vec<u128>, q: f64| v[((v.len() - 1) as f64 * q) as usize];
    println!(
        "echo       feed p50 {} ns p99 {} ns  frame p50 {} ns p99 {} ns",
        p(&feed, 0.5),
        p(&feed, 0.99),
        p(&frame, 0.5),
        p(&frame, 0.99)
    );
}

fn main() {
    echo();
    for (name, bytes) in workloads() {
        let mb = bytes.len() as f64 / 1e6;
        let raw = best(|| {
            let size = TerminalSize {
                rows: 24,
                cols: 80,
                pixel_width: 640,
                pixel_height: 384,
                dpi: 0,
            };
            let mut term = wezterm_term::Terminal::new(
                size,
                Arc::new(Plain),
                "bench",
                "0",
                Box::new(std::io::sink()),
            );
            let t = Instant::now();
            for c in bytes.chunks(4096) {
                term.advance_bytes(c);
            }
            t.elapsed().as_secs_f64()
        });
        let ours = best(|| {
            let mut term = Terminal::new(80, 24, Options::default());
            let t = Instant::now();
            for c in bytes.chunks(4096) {
                term.feed(c);
            }
            t.elapsed().as_secs_f64()
        });
        println!(
            "{name:10} {mb:5.1} MB  wezterm-term alone {:7.1} MB/s  moshpit-term {:7.1} MB/s",
            mb / raw,
            mb / ours
        );
    }
}
