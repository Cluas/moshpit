//! The names the Swift side spells out for itself are this crate's (E7b).
//!
//! The notification service hands crates/seal the files' places and the
//! switches by name, and the app reads a card's routing keys by name; both
//! are Swift literals, since UniFFI carries no constants. A literal that
//! drifts from the crate fails quietly: a file the service writes and the
//! app never reads, a switch that never holds, a tap that lands nowhere.
//! This reads the Swift sources, so a literal changed on either side is
//! caught the day it changes.

use std::path::Path;

use moshpit_seal::{
    PushFinishesFile, PushPairingFile, PushStandingFile, closed::PushClosedFile,
    diagnostics::DiagnosticsFile, notification, prefs,
};

fn source(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `text` names `value` as a Swift string literal.
fn names(text: &str, value: &str) -> bool {
    text.contains(&format!("\"{value}\""))
}

#[test]
fn the_card_keys_are_the_crates() {
    let text = source("Packages/MoshpitKit/Sources/MoshpitKit/Island/AgentNotifications.swift");
    for (swift, value) in [
        ("connectionKey", notification::CONNECTION_KEY),
        ("paneKey", notification::PANE_KEY),
        ("agentKey", notification::AGENT_KEY),
        ("stateKey", notification::STATE_KEY),
        ("detailKey", notification::DETAIL_KEY),
        ("selfTestAgent", notification::SELF_TEST_AGENT),
    ] {
        let line = format!("public static let {swift} = \"{value}\"");
        assert!(
            text.contains(&line),
            "AgentNotifications.swift: expected `{line}`"
        );
    }
}

#[test]
fn the_service_names_the_crates_files_and_switches() {
    let text = source("Extensions/MoshpitPush/NotificationService.swift");
    for value in [
        PushPairingFile::FILE_NAME,
        DiagnosticsFile::FILE_NAME,
        PushStandingFile::FILE_NAME,
        PushFinishesFile::FILE_NAME,
        PushClosedFile::FILE_NAME,
        prefs::SOUND_KEY,
        prefs::DETAIL_KEY,
    ] {
        assert!(
            names(&text, value),
            "NotificationService.swift doesn't name \"{value}\""
        );
    }
}

#[test]
fn the_app_reads_the_files_the_service_writes() {
    let monitor = source("Moshpit/Services/Island/AgentActivityMonitor.swift");
    for value in [PushStandingFile::FILE_NAME, PushFinishesFile::FILE_NAME] {
        assert!(
            names(&monitor, value),
            "AgentActivityMonitor.swift doesn't name \"{value}\""
        );
    }
    let diagnostics = source("Moshpit/UI/Settings/DiagnosticsLog.swift");
    assert!(
        names(&diagnostics, DiagnosticsFile::FILE_NAME),
        "DiagnosticsLog.swift doesn't name \"{}\"",
        DiagnosticsFile::FILE_NAME
    );
    let pairings = source("Moshpit/Services/Rust/RustStores.swift");
    assert!(
        names(&pairings, PushPairingFile::FILE_NAME),
        "RustStores.swift doesn't name \"{}\"",
        PushPairingFile::FILE_NAME
    );
}
