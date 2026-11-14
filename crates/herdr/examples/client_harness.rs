//! The Rust side of the client equivalence runs (`harness/client/`): run a
//! scenario with this crate's client on the fake host and print its log,
//! the log the Swift side prints for the app's `HerdrControlClient.swift`.
//!
//!   client_harness <script>...

fn main() -> std::process::ExitCode {
    let mut failed = false;
    for path in std::env::args().skip(1) {
        let text = std::fs::read_to_string(&path).expect("script readable");
        match moshpit_herdr::client::scenario::run(&text) {
            Ok(log) => print!("{log}"),
            Err(error) => {
                eprintln!("{path}: {error}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
