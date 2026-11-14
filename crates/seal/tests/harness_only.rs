//! The envelope opens that don't check which connection sent a push are
//! the seal harness's alone (B435): Swift's `PushRemoteNotification.open(_:
//! secrets:)` and Rust's `notification::open`. Everything that shows a
//! notification goes through the opens that do — `open(_:pairings:)` and
//! `notification::open_paired` — and this fails the moment the notification
//! service, the app or Android calls one of the other two.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every file under `dir` (relative to the repo) with extension `ext`.
fn sources(dir: &str, ext: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root().join(dir)];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == ext) {
                found.push(path);
            }
        }
    }
    found
}

/// A call of Swift's secrets-only open: `open(<envelope>, secrets:`.
fn calls_swift_secrets_open(text: &str) -> bool {
    text.match_indices("open(").any(|(at, _)| {
        let rest = &text[at + "open(".len()..];
        let call = &rest[..rest.find(')').unwrap_or(rest.len())];
        call.contains("secrets:")
    })
}

#[test]
fn only_the_harness_opens_an_envelope_without_checking_its_connection() {
    let mut callers = Vec::new();
    for dir in ["Extensions/MoshpitPush", "Moshpit"] {
        let files = sources(dir, "swift");
        assert!(!files.is_empty(), "no Swift found under {dir}");
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if calls_swift_secrets_open(&text) {
                callers.push(path);
            }
        }
    }
    for dir in ["crates/android/src", "crates/seal/src"] {
        for path in sources(dir, "rs") {
            if path.ends_with("crates/seal/src/harness.rs") || path.ends_with("crates/seal/src/notification.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if text.contains("notification::open(") {
                callers.push(path);
            }
        }
    }
    assert!(callers.is_empty(), "calls an open that doesn't check the connection: {callers:?}");
}

#[test]
fn the_check_sees_a_call_when_there_is_one() {
    assert!(calls_swift_secrets_open(
        "let s = PushRemoteNotification.open(envelope, secrets: list)"
    ));
    assert!(!calls_swift_secrets_open(
        "switch PushRemoteNotification.open(envelope, pairings: pairings) {"
    ));
}
