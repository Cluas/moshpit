//! What Moshpit has installed on a host, as recorded on that host
//! (`InstallManifest`, `~/.moshpit/manifest.json`), and the things it can
//! install (`InstallComponent`).
//!
//! The app reads, merges and writes the manifest; the host only stores it.
//! It is `JSONEncoder` output with `[.prettyPrinted, .sortedKeys]` and ISO
//! 8601 dates — reproduced byte for byte, so a host keeps one manifest
//! whichever implementation last wrote it.

use std::collections::BTreeMap;

use moshpit_seal::foundation::date::{self, Date};
use moshpit_seal::foundation::json::{self, Style};
use serde::{Deserialize, Serialize};

use crate::commands::DIR;

/// One thing the installer can put on a host.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InstallComponent {
    /// `~/.moshpit/moshpit-stamp.sh` — what an agent hook calls.
    Stamp,
    /// `~/.moshpit/moshpit-push.sh` — what turns a status into a push.
    Sender,
    /// A hook registration inside one agent's own config.
    Hooks { agent: String },
    /// `~/.moshpit/push.d/<conn>.<install>.conf` — one install's pairing
    /// of connection `conn`: its secrets, and the phone it pushes to.
    ///
    /// Per install (B443): a phone restored from another's backup has the
    /// same connections, ids and all, and both wrote `<conn>.conf` and the
    /// manifest's `pairing.<conn>`. Each phone's care found the other's
    /// pairing there and wrote its own over it, and the host pushed to
    /// whichever had connected last. `install` is a random id each install
    /// keeps out of its backups; `None` is the file and key from before it
    /// (`<conn>.conf`, `pairing.<conn>`), only ever read and removed.
    Pairing { conn: String, install: Option<String> },
}

/// Whether `id` can name an install in a pairing's file and key: 8 to 32
/// lowercase hex digits (what the phones mint), nothing a path or a shell
/// could read otherwise.
pub fn is_install_id(id: &str) -> bool {
    (8..=32).contains(&id.len()) && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl InstallComponent {
    /// The manifest key.
    pub fn key(&self) -> String {
        match self {
            Self::Stamp => "stamp".to_owned(),
            Self::Sender => "sender".to_owned(),
            Self::Hooks { agent } => format!("hooks.{agent}"),
            Self::Pairing { conn, install: None } => format!("pairing.{conn}"),
            Self::Pairing {
                conn,
                install: Some(install),
            } => format!("pairing.{conn}.{install}"),
        }
    }

    /// The absolute path of a component that is a file.
    pub fn path(&self) -> Option<String> {
        match self {
            Self::Stamp => Some(format!("{DIR}/moshpit-stamp.sh")),
            Self::Sender => Some(format!("{DIR}/moshpit-push.sh")),
            Self::Pairing { conn, install: None } => Some(format!("{DIR}/push.d/{conn}.conf")),
            Self::Pairing {
                conn,
                install: Some(install),
            } => Some(format!("{DIR}/push.d/{conn}.{install}.conf")),
            Self::Hooks { .. } => None,
        }
    }

    /// The mode a fresh copy is written with: the scripts executable, a
    /// pairing's two secrets readable only by the owner.
    pub fn mode(&self) -> &'static str {
        match self {
            Self::Stamp | Self::Sender => "755",
            Self::Pairing { .. } => "600",
            Self::Hooks { .. } => "644",
        }
    }
}

/// One recorded component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    /// SHA-256 of the file as the app wrote it; absent for a registration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(rename = "installedAt", with = "date::iso8601")]
    pub installed_at: Date,
    /// For `hooks.<agent>`: the config that was edited, so uninstall edits
    /// the same file.
    #[serde(
        rename = "configPath",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub config_path: Option<String>,
    /// For `pairing.<conn>`: which connection and relay. Never the secret.
    #[serde(
        rename = "connectionId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub connection_id: Option<String>,
    #[serde(rename = "relayURL", default, skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
}

impl Component {
    pub fn new(installed_at: Date) -> Self {
        Self {
            digest: None,
            installed_at,
            config_path: None,
            connection_id: None,
            relay_url: None,
        }
    }
}

/// The manifest itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallManifest {
    pub schema: i64,
    pub components: BTreeMap<String, Component>,
}

impl Default for InstallManifest {
    fn default() -> Self {
        Self::empty()
    }
}

impl InstallManifest {
    /// Bumped only if the file's shape changes. An unknown schema reads as
    /// nothing installed rather than being misread.
    pub const CURRENT_SCHEMA: i64 = 1;

    /// Where the manifest lives.
    pub const PATH: &'static str = "$HOME/.moshpit/manifest.json";

    pub fn empty() -> Self {
        Self {
            schema: Self::CURRENT_SCHEMA,
            components: BTreeMap::new(),
        }
    }

    pub fn get(&self, component: &InstallComponent) -> Option<&Component> {
        self.components.get(&component.key())
    }

    /// `manifest[component] = value`.
    pub fn set(&mut self, component: &InstallComponent, value: Option<Component>) {
        match value {
            Some(value) => self.components.insert(component.key(), value),
            None => self.components.remove(&component.key()),
        };
    }

    /// Connection `conn`'s pairing entry from before installs were told
    /// apart (B443): `pairing.<conn>`.
    pub fn pairing_entry(&self, conn: &str) -> Option<&Component> {
        self.components.get(&format!("pairing.{conn}"))
    }

    /// Install `install`'s pairing entry for connection `conn` — the legacy
    /// one with `None` ([`pairing_entry`](Self::pairing_entry)).
    pub fn pairing_entry_of(&self, conn: &str, install: Option<&str>) -> Option<&Component> {
        self.get(&InstallComponent::Pairing {
            conn: conn.to_owned(),
            install: install.map(str::to_owned),
        })
    }

    /// Every pairing of connection `conn` recorded on this host: the legacy
    /// one and each install's (B443), as components.
    pub fn pairings_of(&self, conn: &str) -> Vec<InstallComponent> {
        let legacy = format!("pairing.{conn}");
        self.components
            .keys()
            .filter_map(|key| {
                let rest = key.strip_prefix(&legacy)?;
                let install = match rest {
                    "" => None,
                    _ => Some(rest.strip_prefix('.').filter(|id| is_install_id(id))?.to_owned()),
                };
                Some(InstallComponent::Pairing {
                    conn: conn.to_owned(),
                    install,
                })
            })
            .collect()
    }

    /// Every device pairing recorded on this host.
    pub fn pairing_entries(&self) -> Vec<&Component> {
        self.components
            .iter()
            .filter(|(key, _)| key.starts_with("pairing."))
            .map(|(_, value)| value)
            .collect()
    }

    pub fn remove_pairing(&mut self, conn: &str) {
        self.components.remove(&format!("pairing.{conn}"));
    }

    /// The manifest as read off a host (`InstallManifest.read`): the text the
    /// read command printed, trimmed. Absent, empty, corrupt and
    /// future-schema files all read as [`empty`](Self::empty) — guessing at a
    /// file this build cannot parse is how an uninstall deletes the wrong
    /// thing.
    pub fn from_text(text: &str) -> Self {
        if text.is_empty() {
            return Self::empty();
        }
        match json::decode::<Self>(text.as_bytes()) {
            Ok(manifest) if manifest.schema == Self::CURRENT_SCHEMA => manifest,
            _ => Self::empty(),
        }
    }

    /// The bytes the app writes: `JSONEncoder` with pretty printing, sorted
    /// keys and ISO 8601 dates.
    pub fn to_json(&self) -> String {
        json::write_encoder(&json::to_value(self), Style::PRETTY_SORTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Date {
        Date::from_unix(1_762_800_000.0)
    }

    /// `HostAutoCareTests.pairingLookupIsPerDevice`.
    #[test]
    fn a_device_finds_its_own_pairing_and_no_other_devices() {
        let conn = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";
        let mut manifest = InstallManifest::empty();
        manifest.set(
            &InstallComponent::Pairing { conn: conn.into(), install: None },
            Some(Component {
                digest: Some("x".into()),
                connection_id: Some(conn.into()),
                relay_url: Some("r".into()),
                ..Component::new(now())
            }),
        );
        assert_eq!(
            manifest.pairing_entry(conn).unwrap().digest.as_deref(),
            Some("x")
        );
        assert!(manifest.pairing_entry("another-device").is_none());
    }

    /// `HostAutoCareTests.removalIsScoped`.
    /// B443: two installs' pairings of one connection (a phone and another
    /// restored from its backup) keep apart, and apart from the legacy one.
    #[test]
    fn two_installs_pairings_of_one_connection_keep_apart() {
        let mut manifest = InstallManifest::empty();
        for (install, digest) in [(None, "legacy"), (Some("aaaaaaaa"), "a"), (Some("bbbbbbbb"), "b")] {
            manifest.set(
                &InstallComponent::Pairing {
                    conn: "C".into(),
                    install: install.map(str::to_owned),
                },
                Some(Component {
                    digest: Some(digest.into()),
                    ..Component::new(Date::from_unix(0.0))
                }),
            );
        }
        let digest = |install| manifest.pairing_entry_of("C", install).and_then(|c| c.digest.clone());
        assert_eq!(digest(None).as_deref(), Some("legacy"));
        assert_eq!(digest(Some("aaaaaaaa")).as_deref(), Some("a"));
        assert_eq!(digest(Some("bbbbbbbb")).as_deref(), Some("b"));
        assert_eq!(manifest.pairing_entries().len(), 3);
    }

    #[test]
    fn an_install_id_is_eight_to_thirty_two_lowercase_hex_digits() {
        assert!(is_install_id("0123456789abcdef"));
        assert!(is_install_id("deadbeef"));
        for bad in ["", "abc", "0123456789ABCDEF", "../../x", "aaaa aaaa", "a.b.c.d.e", &"a".repeat(33)] {
            assert!(!is_install_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn removing_one_devices_pairing_leaves_every_other_devices_standing() {
        let mut manifest = InstallManifest::empty();
        for conn in ["a", "b"] {
            manifest.set(
                &InstallComponent::Pairing { conn: conn.into(), install: None },
                Some(Component {
                    connection_id: Some(conn.into()),
                    relay_url: Some("r".into()),
                    ..Component::new(now())
                }),
            );
        }
        manifest.remove_pairing("a");
        assert!(manifest.pairing_entry("a").is_none());
        assert!(manifest.pairing_entry("b").is_some());
        assert_eq!(manifest.pairing_entries().len(), 1);
    }

    /// `HostInstallerTests.unreadableManifest`, `absentManifest`.
    #[test]
    fn a_corrupt_absent_or_future_schema_manifest_is_not_guessed_at() {
        for body in [
            "",
            "{not json",
            r#"{"schema":99,"components":{"stamp":{}}}"#,
            r#"{"schema":1}"#,
        ] {
            assert_eq!(
                InstallManifest::from_text(body),
                InstallManifest::empty(),
                "{body}"
            );
        }
        // A component without its date is no component.
        assert_eq!(
            InstallManifest::from_text(r#"{"schema":1,"components":{"stamp":{"digest":"d"}}}"#),
            InstallManifest::empty()
        );
    }

    #[test]
    fn the_file_is_jsonencoders() {
        let mut manifest = InstallManifest::empty();
        assert_eq!(
            manifest.to_json(),
            "{\n  \"components\" : {\n\n  },\n  \"schema\" : 1\n}"
        );
        manifest.set(
            &InstallComponent::Hooks {
                agent: "claude".into(),
            },
            Some(Component {
                digest: Some("abc".into()),
                config_path: Some("$HOME/.claude/settings.json".into()),
                ..Component::new(now())
            }),
        );
        manifest.set(
            &InstallComponent::Stamp,
            Some(Component {
                digest: Some("d".into()),
                ..Component::new(now())
            }),
        );
        let text = manifest.to_json();
        assert_eq!(
            text,
            "{\n  \"components\" : {\n    \"hooks.claude\" : {\n      \"configPath\" : \"$HOME\\/.claude\\/settings.json\",\n      \
             \"digest\" : \"abc\",\n      \"installedAt\" : \"2025-11-10T18:40:00Z\"\n    },\n    \"stamp\" : {\n      \
             \"digest\" : \"d\",\n      \"installedAt\" : \"2025-11-10T18:40:00Z\"\n    }\n  },\n  \"schema\" : 1\n}"
        );
        assert_eq!(InstallManifest::from_text(&text), manifest);
    }

    #[test]
    fn components_know_their_key_path_and_mode() {
        let pairing = InstallComponent::Pairing { conn: "C".into(), install: None };
        assert_eq!(pairing.key(), "pairing.C");
        assert_eq!(
            pairing.path().as_deref(),
            Some("$HOME/.moshpit/push.d/C.conf")
        );
        assert_eq!(pairing.mode(), "600");
        // One install's pairing (B443): its own file and its own key.
        let own = InstallComponent::Pairing {
            conn: "C".into(),
            install: Some("0123456789abcdef".into()),
        };
        assert_eq!(own.key(), "pairing.C.0123456789abcdef");
        assert_eq!(
            own.path().as_deref(),
            Some("$HOME/.moshpit/push.d/C.0123456789abcdef.conf")
        );
        assert_eq!(own.mode(), "600");
        assert_eq!(
            InstallComponent::Stamp.path().as_deref(),
            Some("$HOME/.moshpit/moshpit-stamp.sh")
        );
        assert_eq!(InstallComponent::Sender.mode(), "755");
        assert_eq!(
            InstallComponent::Hooks {
                agent: "codex".into()
            }
            .key(),
            "hooks.codex"
        );
        assert_eq!(
            InstallComponent::Hooks {
                agent: "codex".into()
            }
            .path(),
            None
        );
    }
}
