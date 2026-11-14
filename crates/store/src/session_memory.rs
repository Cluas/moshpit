//! What a session remembers about a connection between attempts and app
//! launches, where the app keeps it in `UserDefaults.standard`, as `Data`
//! holding JSON:
//!
//! - where the person was in tmux (`TmuxSelectionStore`):
//!   `moshpit.tmux.lastSelection.<UUID>` → `{"session":…,"window":…,"pane":…}`
//!   (window and pane left out when unknown, as `JSONEncoder` leaves out a
//!   `nil`);
//! - what the host could run when last probed (`HostCapabilityCache`):
//!   `hostcaps.<UUID>` → the capabilities (`moshpit-install`'s
//!   `HostCapabilities`, whose JSON is the app's).
//!
//! Same keys, same JSON: the Swift transport and the Rust one take turns on
//! them, and a connect on either lands where the other left off.
//!
//! Both go with their connection (B199): [`forget`] when it is deleted,
//! and [`orphans`] for what deletes before that left behind (an iPad had
//! about 250), swept once the saved connections can be told
//! ([`crate::connections::ConnectionStore::sweep_session_memory`]).
//!
//! Written only while the preferences can be believed ([`Readiness`]):
//! before the first unlock a launch reads them as empty, and a write then
//! would stand in for what is really there. (The Swift caches write
//! regardless; they are the small loss of a stale landing, but the rule is
//! the store's.)

use std::collections::BTreeSet;

use moshpit_seal::foundation::json;
use moshpit_seal::foundation::uuid::Uuid;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::codec;
use crate::host::Defaults;
use crate::plist::Plist;
use crate::readiness::Readiness;

/// `TmuxSelectionStore.key(_:)`'s prefix.
pub const TMUX_SELECTION_PREFIX: &str = "moshpit.tmux.lastSelection.";
/// `HostCapabilityCache`'s key prefix.
pub const HOST_CAPABILITIES_PREFIX: &str = "hostcaps.";

/// A tmux session/window/pane selection (`TmuxSelection`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TmuxSelection {
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

/// The key of a connection's tmux selection; `id` as the app spells a
/// UUID (`uuidString`, upper case).
pub fn tmux_selection_key(id: &str) -> String {
    format!("{TMUX_SELECTION_PREFIX}{id}")
}

/// The key of a connection's host capabilities.
pub fn host_capabilities_key(id: &str) -> String {
    format!("{HOST_CAPABILITIES_PREFIX}{id}")
}

/// Both prefixes: everything a session remembers about a connection.
pub const PREFIXES: [&str; 2] = [TMUX_SELECTION_PREFIX, HOST_CAPABILITIES_PREFIX];

/// The connection a session-memory key belongs to: its suffix, when the
/// key is one of the two and the suffix is a UUID (either case, as
/// `UUID(uuidString:)` reads). Anything else is not ours to judge.
pub fn connection_of(key: &str) -> Option<Uuid> {
    PREFIXES
        .iter()
        .find_map(|prefix| key.strip_prefix(prefix))
        .and_then(Uuid::parse)
}

/// Removes what sessions remember about connection `id` — unless the
/// preferences can't be believed yet (a removal then would be lost or, on a
/// sealed launch, land on nothing). Whether it was done.
pub fn forget(defaults: &dyn Defaults, readiness: &Readiness, id: Uuid) -> bool {
    if !readiness.is_readable(defaults) {
        return false;
    }
    let id = id.to_string();
    defaults.set(&tmux_selection_key(&id), None);
    defaults.set(&host_capabilities_key(&id), None);
    true
}

/// Of `keys`, the session-memory keys whose connection is not in `saved`:
/// what a delete left behind. Conservative: only the two families, only a
/// suffix that is a UUID, and `saved` must be every connection saved on the
/// device (`ConnectionStore::saved_ids`, never a guess).
pub fn orphans<'a>(keys: impl IntoIterator<Item = &'a str>, saved: &BTreeSet<Uuid>) -> Vec<String> {
    keys.into_iter()
        .filter(|key| connection_of(key).is_some_and(|id| !saved.contains(&id)))
        .map(str::to_owned)
        .collect()
}

/// The value under `key`, decoded the way `JSONDecoder` would; `None` when
/// there is none or it doesn't decode (what the Swift readers do with
/// `try?`).
pub fn read<T: DeserializeOwned>(defaults: &dyn Defaults, key: &str) -> Option<T> {
    match defaults.get(key)? {
        Plist::Data(bytes) => json::decode(&bytes).ok(),
        _ => None,
    }
}

/// Stores `value` under `key` as `JSONEncoder` writes it — unless the
/// preferences can't be believed yet. Whether it was written.
pub fn write<T: Serialize>(
    defaults: &dyn Defaults,
    readiness: &Readiness,
    key: &str,
    value: &T,
) -> bool {
    if !readiness.is_readable(defaults) {
        return false;
    }
    defaults.set(key, Some(Plist::Data(codec::encode(value).into_bytes())));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::MemoryDefaults;

    const ID: &str = "0F2E2C4A-6E1B-4D8A-9C5B-2A7D3E9F1B11";

    #[test]
    fn a_selection_reads_and_writes_as_the_app_keeps_it() {
        let defaults = MemoryDefaults::new();
        // What `JSONEncoder` wrote for `TmuxSelection(session: "$1",
        // window: "@3", pane: nil)` (member order is the Dictionary's).
        defaults.set(
            &tmux_selection_key(ID),
            Some(Plist::Data(br#"{"window":"@3","session":"$1"}"#.to_vec())),
        );
        let stored: TmuxSelection = read(&defaults, &tmux_selection_key(ID)).unwrap();
        assert_eq!(
            stored,
            TmuxSelection {
                session: "$1".into(),
                window: Some("@3".into()),
                pane: None
            }
        );
        let selection = TmuxSelection {
            session: "$2".into(),
            window: Some("@7".into()),
            pane: Some("%12".into()),
        };
        assert!(write(
            &defaults,
            &Readiness::Always,
            &tmux_selection_key(ID),
            &selection
        ));
        assert_eq!(
            defaults.get(&format!("moshpit.tmux.lastSelection.{ID}")),
            Some(Plist::Data(
                br#"{"pane":"%12","session":"$2","window":"@7"}"#.to_vec()
            ))
        );
        assert_eq!(
            read::<TmuxSelection>(&defaults, &tmux_selection_key(ID)),
            Some(selection)
        );
    }

    #[test]
    fn what_doesnt_decode_reads_as_nothing() {
        let defaults = MemoryDefaults::new();
        defaults.set(
            &tmux_selection_key(ID),
            Some(Plist::Data(b"{\"window\":1}".to_vec())),
        );
        assert_eq!(
            read::<TmuxSelection>(&defaults, &tmux_selection_key(ID)),
            None
        );
        defaults.set(&tmux_selection_key(ID), Some(Plist::String("x".into())));
        assert_eq!(
            read::<TmuxSelection>(&defaults, &tmux_selection_key(ID)),
            None
        );
        assert_eq!(read::<TmuxSelection>(&defaults, "absent"), None);
    }

    #[test]
    fn nothing_is_written_before_the_preferences_can_be_believed() {
        let defaults = MemoryDefaults::new();
        defaults.set_protected_data_available(false);
        let selection = TmuxSelection {
            session: "$1".into(),
            window: None,
            pane: None,
        };
        assert!(!write(
            &defaults,
            &Readiness::System,
            &tmux_selection_key(ID),
            &selection
        ));
        assert_eq!(defaults.get(&tmux_selection_key(ID)), None);
        defaults.set_protected_data_available(true);
        assert!(write(
            &defaults,
            &Readiness::System,
            &tmux_selection_key(ID),
            &selection
        ));
        assert_eq!(
            defaults.get(&tmux_selection_key(ID)),
            Some(Plist::Data(br#"{"session":"$1"}"#.to_vec()))
        );
    }

    #[test]
    fn forgetting_a_connection_removes_both_and_nothing_else() {
        let defaults = MemoryDefaults::new();
        let id = Uuid::parse(ID).unwrap();
        let other = "1B2C3D4E-0000-4000-8000-000000000001";
        for key in [
            tmux_selection_key(ID),
            host_capabilities_key(ID),
            tmux_selection_key(other),
            host_capabilities_key(other),
        ] {
            defaults.set(&key, Some(Plist::Data(b"{}".to_vec())));
        }
        defaults.set_protected_data_available(false);
        assert!(!forget(&defaults, &Readiness::System, id));
        assert!(defaults.get(&host_capabilities_key(ID)).is_some());
        defaults.set_protected_data_available(true);
        assert!(forget(&defaults, &Readiness::System, id));
        assert_eq!(defaults.get(&tmux_selection_key(ID)), None);
        assert_eq!(defaults.get(&host_capabilities_key(ID)), None);
        assert!(defaults.get(&tmux_selection_key(other)).is_some());
        assert!(defaults.get(&host_capabilities_key(other)).is_some());
    }

    #[test]
    fn orphans_are_only_the_two_families_of_unsaved_uuids() {
        let saved_id = "0F2E2C4A-6E1B-4D8A-9C5B-2A7D3E9F1B11";
        let gone = "1B2C3D4E-0000-4000-8000-000000000001";
        let saved: BTreeSet<Uuid> = [Uuid::parse(saved_id).unwrap()].into();
        let keys = [
            format!("hostcaps.{saved_id}"),
            format!("moshpit.tmux.lastSelection.{saved_id}"),
            // The saved one, spelled in lower case: still saved.
            format!("hostcaps.{}", saved_id.to_lowercase()),
            format!("hostcaps.{gone}"),
            format!("moshpit.tmux.lastSelection.{gone}"),
            // Not a UUID, another family, a prefix of a family: left alone.
            "hostcaps.legacy".to_owned(),
            "hostcaps.".to_owned(),
            format!("moshpit.tmux.other.{gone}"),
            format!("moshpit.connections.{gone}"),
            format!("hostcaps.{gone}x"),
        ];
        assert_eq!(
            orphans(keys.iter().map(String::as_str), &saved),
            [
                format!("hostcaps.{gone}"),
                format!("moshpit.tmux.lastSelection.{gone}")
            ]
        );
    }

    #[test]
    fn the_keys_are_the_apps() {
        assert_eq!(
            tmux_selection_key(ID),
            format!("moshpit.tmux.lastSelection.{ID}")
        );
        assert_eq!(host_capabilities_key(ID), format!("hostcaps.{ID}"));
    }
}
