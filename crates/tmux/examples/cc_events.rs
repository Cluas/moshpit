//! The Rust side of the G2 equivalence harness: the same command line and
//! the same output bytes as `harness/swift/main.swift`.
//!
//!   cc_events events <capture> <plan> [<out>]
//!   cc_events bench <capture> <chunk-size|whole> <iterations>
//!
//! `events` prints every parser event as one JSON line, then an `end`
//! record with a hash of the plan (see `moshpit_tmux::harness::plan`).
//! `bench` feeds the capture in fixed-size chunks to a parser whose sink
//! only counts, and prints one JSON line of timings.

use std::hint::black_box;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use moshpit_tmux::harness::{self, Step};
use moshpit_tmux::{ControlParser, Event};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("events") if args.len() >= 4 => events(&args[2], &args[3], args.get(4)),
        Some("bench") if args.len() >= 4 => {
            let iterations = args.get(4).map_or(Ok(5), |n| {
                n.parse().map_err(|_| format!("bad iterations {n:?}"))
            });
            iterations.and_then(|iterations| bench(&args[2], &args[3], iterations))
        }
        _ => Err("usage: cc_events events <capture> <plan> [<out>]\n       \
                  cc_events bench <capture> <chunk-size|whole> <iterations>"
            .to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))
}

fn events(capture: &str, plan: &str, out: Option<&String>) -> Result<(), String> {
    let data = read(capture)?;
    let output = harness::run(plan, &data)?;
    match out {
        Some(path) => {
            std::fs::write(path, &output).map_err(|error| format!("cannot write {path}: {error}"))
        }
        None => std::io::stdout()
            .write_all(&output)
            .map_err(|error| error.to_string()),
    }
}

fn bench(capture: &str, chunk: &str, iterations: usize) -> Result<(), String> {
    let data = read(capture)?;
    let spec = if chunk == "whole" {
        "whole".to_owned()
    } else {
        format!("fixed:{chunk}")
    };
    let chunks: Vec<&[u8]> = harness::plan(&spec, &data)?
        .into_iter()
        .filter_map(|step| match step {
            Step::Feed(range) => Some(&data[range]),
            Step::Reset => None,
        })
        .collect();
    let (mut events, mut output_bytes) = (0usize, 0usize);
    let mut best = Duration::MAX;
    let mut total = Duration::ZERO;
    for _ in 0..iterations.max(1) {
        let mut parser = ControlParser::new();
        let start = Instant::now();
        for piece in &chunks {
            parser.feed(black_box(piece), |event| {
                events += 1;
                if let Event::PaneOutput { data, .. } = &event {
                    output_bytes += data.len();
                }
                black_box(event);
            });
        }
        let elapsed = start.elapsed();
        total += elapsed;
        best = best.min(elapsed);
    }
    let iterations = iterations.max(1);
    let megabytes = data.len() as f64 / 1_000_000.0;
    let mean = total.as_secs_f64() / iterations as f64;
    let name = Path::new(capture).file_name().map_or_else(
        || capture.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    println!(
        r#"{{"impl":"rust","capture":"{name}","chunk":"{chunk}","iterations":{iterations},"bytes":{},"feeds":{},"events":{},"output_bytes":{},"best_s":{:.6},"mean_s":{mean:.6},"best_mb_s":{:.1},"mean_mb_s":{:.1}}}"#,
        data.len(),
        chunks.len(),
        events / iterations,
        output_bytes / iterations,
        best.as_secs_f64(),
        megabytes / best.as_secs_f64(),
        megabytes / mean,
    );
    Ok(())
}
