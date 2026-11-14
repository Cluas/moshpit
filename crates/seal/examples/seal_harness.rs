//! The Rust side of the seal equivalence harness: the same command line and
//! the same output bytes as `harness/swift/main.swift`, over this crate.
//!
//!   seal_harness run <cases> <out> <scratch-dir>   one result line per case
//!   seal_harness gen <seed> <count>                seeded cases, one per line
//!   seal_harness divergences                       one minimal case per fix

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use moshpit_seal::harness;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") if args.len() == 5 => {
            let cases = std::fs::read_to_string(&args[2]).expect("cases");
            let scratch = Path::new(&args[4]);
            let mut out = String::new();
            for line in cases.lines() {
                if let Some(result) = harness::run(line, scratch) {
                    out.push_str(&result);
                    out.push('\n');
                }
            }
            std::fs::write(&args[3], out).expect("write results");
            ExitCode::SUCCESS
        }
        Some("divergences") => {
            for line in harness::divergence_cases() {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Some("gen") if args.len() == 4 => {
            let seed = args[2].parse().expect("seed");
            let count = args[3].parse().expect("count");
            let mut stdout = std::io::stdout().lock();
            for case in harness::generate(seed, count) {
                writeln!(stdout, "{case}").expect("stdout");
            }
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: seal_harness run <cases> <out> <scratch> | gen <seed> <count>");
            ExitCode::from(2)
        }
    }
}
