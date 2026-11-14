//! The Rust side of the install equivalence harness: the same command line
//! and the same output bytes as `harness/swift/main.swift`.
//!
//!   install_harness run <cases> <out>     one result line per case
//!   install_harness gen <seed> <count>    seeded cases, one per line
//!   install_harness divergences           one minimal case per fix

use std::process::ExitCode;

use moshpit_install::harness;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") if args.len() == 4 => {
            let cases = std::fs::read_to_string(&args[2]).expect("cases");
            let mut out = String::new();
            for line in cases.lines() {
                if let Some(result) = harness::run(line) {
                    out.push_str(&result);
                    out.push('\n');
                }
            }
            std::fs::write(&args[3], out).expect("write results");
            ExitCode::SUCCESS
        }
        Some("gen") if args.len() == 4 => {
            for case in harness::generate(
                args[2].parse().expect("seed"),
                args[3].parse().expect("count"),
            ) {
                println!("{case}");
            }
            ExitCode::SUCCESS
        }
        Some("divergences") => {
            for line in harness::divergence_cases() {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "usage: install_harness run <cases> <out> | gen <seed> <count> | divergences"
            );
            ExitCode::from(2)
        }
    }
}
