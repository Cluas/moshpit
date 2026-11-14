//! Three-way interop without the tools: what `harness/interop.py record`
//! captured from the other implementations of format v1.
//!
//! - `envelopes.txt`: envelopes sealed by the host's `moshpit-push.sh`
//!   (every shell and openssl build on the recording Mac), by Go's sealbox,
//!   by Swift's `PushSealedBox` and by this crate; Rust must open each to
//!   exactly what Swift opened it to (`envelopes.swift.txt`).
//! - `sealed.txt`: seals with the IV pinned; Rust must write Go's bytes.
//! - `tokens.txt`: send tokens minted by `push-relay/token.go`; Rust must
//!   mint the same and verify them.
//! - `pairings.txt`: pairing files written by Swift's `PushPairingFile`;
//!   Rust must read what Swift read back.

use std::path::{Path, PathBuf};

use moshpit_seal::foundation::hex;
use moshpit_seal::{harness, token};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/interop")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn lines(name: &str) -> Vec<String> {
    fixture(name)
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("seal-interop-{}", moshpit_seal::random::hex(8)));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn every_implementations_envelopes_open_as_swift_opened_them() {
    let text = fixture("envelopes.txt");
    let mut sources = Vec::new();
    let mut cases = Vec::new();
    for line in text.lines() {
        if let Some(source) = line.strip_prefix("# ") {
            sources.push(source.to_owned());
        } else if !line.is_empty() {
            cases.push(line.to_owned());
        }
    }
    let swift = lines("envelopes.swift.txt");
    assert_eq!(cases.len(), swift.len());
    assert!(
        sources.iter().filter(|s| s.starts_with("host")).count() >= 70,
        "the host script's envelopes are missing"
    );
    for prefix in [
        "host sh",
        "host dash",
        "host bash",
        "host ksh",
        "go pinned",
        "go random",
        "swift pinned",
        "rust",
    ] {
        assert!(
            sources.iter().any(|s| s.starts_with(prefix)),
            "no envelope from {prefix}"
        );
    }
    let dir = scratch();
    for ((source, case), swift) in sources.iter().zip(&cases).zip(&swift) {
        let rust = harness::run(case, &dir).expect("a case");
        assert_eq!(&rust, swift, "{source}");
        if source.starts_with("host") {
            assert!(
                rust.contains(" status="),
                "{source}: the host's status did not decode: {rust}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_pinned_iv_seals_to_gos_bytes() {
    let lines = lines("sealed.txt");
    assert_eq!(lines.len() % 2, 0);
    let dir = scratch();
    for pair in lines.chunks(2) {
        assert_eq!(
            harness::run(&pair[0], &dir).expect("a case"),
            pair[1],
            "{}",
            pair[0]
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn send_tokens_are_the_relays() {
    let lines = lines("tokens.txt");
    assert!(lines.len() >= 20);
    for line in lines {
        let f: Vec<&str> = line.split(' ').collect();
        let master = hex::decode(f[0]).expect("hex master");
        let iat: i64 = f[3].parse().expect("iat");
        assert_eq!(token::mint(&master, f[1], f[2], iat), f[4], "{line}");
        assert!(
            token::verify(
                &master,
                f[4],
                f[1],
                f[2],
                iat,
                iat,
                token::DEFAULT_TTL_SECONDS
            ),
            "{line}"
        );
    }
}

#[test]
fn pairing_files_swift_wrote_read_as_swift_read_them() {
    let lines = lines("pairings.txt");
    assert_eq!(lines.len() % 2, 0);
    let dir = scratch();
    for pair in lines.chunks(2) {
        let rust = harness::run(&pair[0], &dir).expect("a case");
        let results = |line: &str| line.split(" || ").next().unwrap_or_default().to_owned();
        assert_eq!(results(&rust), results(&pair[1]), "{}", pair[0]);
    }
    let _ = std::fs::remove_dir_all(dir);
}
