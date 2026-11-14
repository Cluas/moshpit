//! Command-line front end for the Rust port, the twin of
//! `harness/swift/main.swift`: one result line per case, in the same format.
//!
//!   herdr_harness cases <corpus>
//!   herdr_harness bench <stream> <chunk-size|whole> <iterations>
//!
//! `harness/equivalence.py` runs both over the same corpora and compares.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if let [_, mode, path, chunk, iterations] = args.as_slice()
        && mode == "bench"
    {
        return bench(path, chunk, iterations.parse().unwrap_or(1));
    }
    let [_, mode, path] = args.as_slice() else {
        eprintln!("usage: herdr_harness cases <corpus>");
        return ExitCode::from(2);
    };
    if mode != "cases" {
        eprintln!("usage: herdr_harness cases <corpus>");
        return ExitCode::from(2);
    }
    let corpus = match std::fs::read_to_string(path) {
        Ok(corpus) => corpus,
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut out = String::new();
    for line in moshpit_herdr::harness::run_corpus(&corpus) {
        out.push_str(&line);
        out.push('\n');
    }
    print!("{out}");
    ExitCode::SUCCESS
}

/// The parser alone, as the Swift front end's `bench`: `iterations` fresh
/// parsers over the stream in `chunk`-byte feeds; MB/s and the frame count.
fn bench(path: &str, chunk: &str, iterations: usize) -> ExitCode {
    let Ok(stream) = std::fs::read(path) else {
        eprintln!("{path}: unreadable");
        return ExitCode::FAILURE;
    };
    let size = if chunk == "whole" {
        stream.len().max(1)
    } else {
        chunk.parse().unwrap_or(4096)
    };
    let mut frames = 0;
    let started = std::time::Instant::now();
    for _ in 0..iterations {
        let mut parser = moshpit_herdr::FrameParser::new();
        frames = stream
            .chunks(size)
            .map(|piece| parser.consume(piece).len())
            .sum::<usize>();
    }
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "rust {:.1} MB/s frames={frames}",
        (stream.len() * iterations) as f64 / 1e6 / seconds
    );
    ExitCode::SUCCESS
}
