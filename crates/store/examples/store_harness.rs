//! The Rust side of the store's equivalence harness (`harness/equivalence.py`);
//! the case format is in `src/harness.rs`.
//!
//!   store_harness gen SEED COUNT              cases on stdout
//!   store_harness run CASES OUT FINALS        a result line and a final snapshot per case
//!   store_harness canon[-strict] IN OUT       results in canonical form (strict: no files fold)
//!   store_harness view108 IN OUT              `STORE SNAPSHOT [KNOWN]` lines → what 1.0.8 should see
//!   store_harness pairings SEED COUNT FILES VIEWS   pairing files moshpit-seal wrote, and Rust's view

use std::fs;

use moshpit_store::harness;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gen") => {
            let seed = args[2].parse().expect("a seed");
            let count = args[3].parse().expect("a count");
            for case in harness::generate(seed, count) {
                println!("{case}");
            }
        }
        Some("run") => {
            let cases = fs::read_to_string(&args[2]).expect("cases");
            let (results, finals) = harness::run(&cases);
            fs::write(&args[3], results).expect("results");
            fs::write(&args[4], finals).expect("finals");
        }
        Some(command @ ("canon" | "canon-strict")) => {
            let input = fs::read_to_string(&args[2]).expect("results");
            let canonical = if command == "canon" {
                harness::canonical
            } else {
                harness::canonical_strict
            };
            let out: String = input.lines().map(|l| canonical(l) + "\n").collect();
            fs::write(&args[3], out).expect("canonical results");
        }
        Some("view108") => {
            let input = fs::read_to_string(&args[2]).expect("snapshots");
            let out: String = input
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| {
                    let fields: Vec<&str> = l.split(' ').collect();
                    let known = fields.get(2).unwrap_or(&fields[1]);
                    harness::canonical(&harness::view108_knowing(fields[0], fields[1], known))
                        + "\n"
                })
                .collect();
            fs::write(&args[3], out).expect("views");
        }
        Some("pairings") => {
            let seed = args[2].parse().expect("a seed");
            let count = args[3].parse().expect("a count");
            let files = harness::pairing_files(seed, count);
            let views: String = files
                .iter()
                .map(|f| harness::pairing_view(f) + "\n")
                .collect();
            fs::write(&args[4], files.join("\n") + "\n").expect("files");
            fs::write(&args[5], views).expect("views");
        }
        _ => {
            eprintln!("usage: store_harness gen|run|canon|view108|pairings …");
            std::process::exit(2);
        }
    }
}
