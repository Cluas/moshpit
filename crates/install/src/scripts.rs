//! The shell Moshpit installs on a dev host (`HostScripts`).
//!
//! The canonical copies are `scripts/host/moshpit-stamp.sh` and
//! `moshpit-push.sh`, included here at build time with the trailing newline
//! dropped (as the Swift app's literals were cut, so a host installed by it
//! reads as current). A script's identity on a host is the SHA-256 of
//! exactly those bytes — the digest the manifest records and every later
//! visit compares — so one byte of difference here would make every host
//! look stale and be reinstalled.

use sha2::{Digest, Sha256};

use crate::manifest::InstallComponent;

const STAMP_FILE: &str = include_str!("../../../scripts/host/moshpit-stamp.sh");
const SENDER_FILE: &str = include_str!("../../../scripts/host/moshpit-push.sh");

/// What an agent's lifecycle hook calls: stamps the pane, hands attention
/// and done to the sender.
pub fn stamp() -> &'static str {
    STAMP_FILE.trim_end_matches('\n')
}

/// Seals one agent status and hands it to the push relay.
pub fn sender() -> &'static str {
    SENDER_FILE.trim_end_matches('\n')
}

/// SHA-256 of some content, lowercase hex (`ContentDigest.of`): the
/// identity used throughout the install manifest.
pub fn content_digest(text: &str) -> String {
    moshpit_seal::foundation::hex::encode(&Sha256::digest(text.as_bytes()))
}

/// The digest the manifest records for a fixed-content component; `None`
/// for a hook registration or a pairing.
pub fn digest(component: &InstallComponent) -> Option<String> {
    body(component).map(content_digest)
}

/// The body of a component that is a plain file with fixed content.
pub fn body(component: &InstallComponent) -> Option<&'static str> {
    match component {
        InstallComponent::Stamp => Some(stamp()),
        InstallComponent::Sender => Some(sender()),
        InstallComponent::Hooks { .. } | InstallComponent::Pairing { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::SELF_TEST_AGENT;

    /// `HostScriptsTests.hookSafety`.
    #[test]
    fn both_scripts_keep_the_two_properties_that_make_them_safe_in_a_hook() {
        for (name, script) in [("stamp", stamp()), ("sender", sender())] {
            assert!(script.starts_with("#!/bin/sh"), "{name} lost its shebang");
            assert!(script.contains("exit 0"), "{name} must always exit 0");
        }
        // Both arms of the hand-off detach.
        assert!(stamp().contains(">/dev/null 2>&1 & )"));
        assert!(stamp().contains(") >/dev/null 2>&1 &"));
        assert!(stamp().contains("MOSHPIT_NOTIFY_GRACE"));
        assert!(stamp().contains(r#"[ "$CUR" = "attention|$EPISODE" ] || exit 0"#));
        assert!(stamp().contains("waiting for your input"));
        assert!(stamp().contains("done) IDLE=1"));
        assert!(sender().contains("attention|done"));
    }

    /// `HostScriptsTests.selfTestLabel`.
    #[test]
    fn the_sender_self_test_carries_the_reserved_label_the_app_matches_on() {
        assert!(sender().contains(SELF_TEST_AGENT));
        assert_eq!(SELF_TEST_AGENT, "moshpit-selftest");
    }

    /// `HostScriptsTests.digests`.
    #[test]
    fn digests_are_content_addressed() {
        let stamp_digest = digest(&InstallComponent::Stamp).unwrap();
        let sender_digest = digest(&InstallComponent::Sender).unwrap();
        assert_ne!(stamp_digest, sender_digest);
        assert_eq!(stamp_digest.len(), 64);
        assert_eq!(stamp_digest, content_digest(stamp()));
        assert_eq!(
            digest(&InstallComponent::Pairing { conn: "any".into() }),
            None
        );
        assert_eq!(
            digest(&InstallComponent::Hooks {
                agent: "claude".into()
            }),
            None
        );
    }

    /// `HostScriptsTests.noEscapingNeeded`.
    #[test]
    fn the_scripts_are_delivered_as_files_so_nothing_needs_escaping() {
        assert!(sender().contains('\''));
        assert!(!sender().contains(r"'\''"));
    }

    /// What is installed is the file with its trailing newline dropped, and
    /// nothing else (`HostScriptsTests.stampMatchesRepo`).
    #[test]
    fn the_scripts_are_the_files_without_their_trailing_newline() {
        assert_eq!(format!("{}\n", stamp()), STAMP_FILE);
        assert_eq!(format!("{}\n", sender()), SENDER_FILE);
        assert!(!stamp().ends_with('\n'));
    }

    #[test]
    fn content_digest_is_sha256_hex() {
        assert_eq!(
            content_digest(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
