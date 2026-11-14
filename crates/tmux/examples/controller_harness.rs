//! The Rust side of the controller equivalence harness
//! (`harness/controller/equivalence.py`): runs a script through
//! `moshpit_tmux::controller::Controller` and prints the log
//! (`moshpit_tmux::scenario`), byte for byte what
//! `harness/controller/swift/main.swift` prints for the Swift controller.
//!
//!   controller_harness run <script> [<out>]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match (args.get(1).map(String::as_str), args.get(2)) {
        (Some("run"), Some(path)) => {
            let script = match std::fs::read_to_string(path) {
                Ok(script) => script,
                Err(error) => {
                    eprintln!("{path}: {error}");
                    return ExitCode::from(2);
                }
            };
            let log = moshpit_tmux::scenario::run(&script);
            match args.get(3) {
                Some(out) => {
                    if let Err(error) = std::fs::write(out, log) {
                        eprintln!("{out}: {error}");
                        return ExitCode::from(2);
                    }
                }
                None => print!("{log}"),
            }
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: controller_harness run <script> [<out>]");
            ExitCode::from(2)
        }
    }
}
