//! Saved connections (`ServerConnection`, `ConnectionStore`): one JSON array
//! under `moshpit.connections`, dates in ISO 8601.
//!
//! Secrets are not here: a password or private key lives in the keychain,
//! and a connection holds only its account name (`keychainRef`).
//!
//! The list is read an entry at a time and never written over unless the
//! read could be trusted ([`crate::stored_list`], [`crate::readiness`]), and
//! every save writes J1's rollback fields: seven fields the model has
//! dropped, five of them non-optional in every build up to 1.0.8, which
//! decode the list in one piece and take a list they can't decode as empty
//! — then save that empty list over every host. So a build installed over
//! this one (TestFlight keeps them a tap away) still reads every connection.

use std::collections::BTreeSet;
use std::sync::Arc;

use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::json::{self, Number, Style, Value};
use moshpit_seal::foundation::uuid::Uuid;
use serde::{Deserialize, Serialize};

use crate::codec;
use crate::host::Defaults;
use crate::plist::Plist;
use crate::readiness::Readiness;
use crate::session_memory;
use crate::ssh_keys::SshKeyAlgorithm;
use crate::stored_list::{self, Contents, Entry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthMethod {
    #[serde(rename = "password")]
    Password,
    #[serde(rename = "key")]
    Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConnectionProtocol {
    #[serde(rename = "ssh")]
    Ssh,
    #[serde(rename = "mosh")]
    Mosh,
}

/// Which multiplexer a connection drives on the remote host. Stored raw
/// (`multiplexerRaw`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Multiplexer {
    /// One plain shell pane, no session persistence.
    None,
    Tmux,
    Herdr,
}

impl Multiplexer {
    pub const ALL: [Self; 3] = [Self::None, Self::Tmux, Self::Herdr];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Tmux => "tmux",
            Self::Herdr => "herdr",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.raw_value() == raw)
    }

    /// The binary probed on PATH and used to boot the session.
    pub fn binary_name(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Tmux => Some("tmux"),
            Self::Herdr => Some("herdr"),
        }
    }
}

/// Mosh's predictive echo (`PredictMode`), stored raw per connection
/// (`predictModeRaw`) and as a setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PredictMode {
    Adaptive,
    Always,
    Never,
    Experimental,
}

impl PredictMode {
    pub const ALL: [Self; 4] = [
        Self::Adaptive,
        Self::Always,
        Self::Never,
        Self::Experimental,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::Always => "always",
            Self::Never => "never",
            Self::Experimental => "experimental",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.raw_value() == raw)
    }
}

/// One saved connection, field for field as the app stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConnection {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_method: AuthMethod,
    pub connection_protocol: ConnectionProtocol,
    /// SSH port for mosh's bootstrap.
    pub ssh_port: i64,
    pub mosh_port_range_start: i64,
    pub mosh_port_range_end: i64,
    /// A custom `mosh-server` path on the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mosh_server_path: Option<String>,
    /// The keychain account of the password or private key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keychain_ref: Option<String>,
    #[serde(with = "iso8601")]
    pub created_at: Date,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "iso8601_opt")]
    pub last_connected_at: Option<Date>,
    /// LEGACY, superseded by `multiplexer_raw` and still read when that is
    /// absent; kept in step with it so an older build decides the same.
    pub use_tmux: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predict_mode_raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roam_on_cellular: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herdr_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplexer_raw: Option<String>,
    /// The managed key picked from SSH Keys, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_key_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_key_algorithm_raw: Option<String>,
    #[serde(
        rename = "useSOCKSProxy",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub use_socks_proxy: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socks_proxy_host: Option<String>,
    /// `None` is 1080.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socks_proxy_port: Option<i64>,
}

impl ServerConnection {
    /// The initializer's defaults: SSH on 22, password sign-in, mosh's UDP
    /// range 60001–60999, no multiplexer.
    pub fn new(id: Uuid, created_at: Date) -> Self {
        Self {
            id,
            name: String::new(),
            host: String::new(),
            port: 22,
            username: String::new(),
            auth_method: AuthMethod::Password,
            connection_protocol: ConnectionProtocol::Ssh,
            ssh_port: 22,
            mosh_port_range_start: 60001,
            mosh_port_range_end: 60999,
            mosh_server_path: None,
            keychain_ref: None,
            created_at,
            last_connected_at: None,
            use_tmux: false,
            predict_mode_raw: None,
            roam_on_cellular: None,
            tmux_path: None,
            herdr_path: None,
            multiplexer_raw: None,
            ssh_key_id: None,
            ssh_key_algorithm_raw: None,
            use_socks_proxy: None,
            socks_proxy_host: None,
            socks_proxy_port: None,
        }
    }

    /// Which multiplexer to drive: the stored choice, or — for a connection
    /// saved before the picker — tmux when `use_tmux` says so.
    pub fn multiplexer(&self) -> Multiplexer {
        self.multiplexer_raw
            .as_deref()
            .and_then(Multiplexer::from_raw)
            .unwrap_or(if self.use_tmux {
                Multiplexer::Tmux
            } else {
                Multiplexer::None
            })
    }

    /// Choose the multiplexer, keeping the legacy flag in step.
    pub fn set_multiplexer(&mut self, multiplexer: Multiplexer) {
        self.multiplexer_raw = Some(multiplexer.raw_value().to_owned());
        self.use_tmux = multiplexer == Multiplexer::Tmux;
    }

    /// The custom binary of the selected multiplexer, or `None` for PATH.
    pub fn multiplexer_path(&self) -> Option<&str> {
        match self.multiplexer() {
            Multiplexer::None => None,
            Multiplexer::Tmux => self.tmux_path.as_deref(),
            Multiplexer::Herdr => self.herdr_path.as_deref(),
        }
    }

    pub fn predict_mode(&self) -> PredictMode {
        self.predict_mode_raw
            .as_deref()
            .and_then(PredictMode::from_raw)
            .unwrap_or(PredictMode::Adaptive)
    }

    pub fn set_predict_mode(&mut self, mode: PredictMode) {
        self.predict_mode_raw = Some(mode.raw_value().to_owned());
    }

    pub fn ssh_key_algorithm(&self) -> Option<SshKeyAlgorithm> {
        self.ssh_key_algorithm_raw
            .as_deref()
            .and_then(SshKeyAlgorithm::from_raw)
    }

    /// Whether a connect to `self` goes to another box than one to
    /// `before`: another host (names compare as DNS does, case aside), port
    /// or SSH port. What was learned about `before`'s box — its push
    /// pairing (B328), what sessions remembered (B431) — isn't this one's.
    pub fn points_elsewhere_than(&self, before: &ServerConnection) -> bool {
        !self.host.eq_ignore_ascii_case(&before.host)
            || self.port != before.port
            || self.ssh_port != before.ssh_port
    }

    /// The name, or `user@host` when it has none.
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            format!("{}@{}", self.username, self.host)
        } else {
            self.name.clone()
        }
    }
}

impl Entry for ServerConnection {
    fn decode(value: &Value) -> Result<Self, String> {
        codec::decode(value)
    }

    fn encode_list(items: &[Self]) -> String {
        codec::encode(items)
    }
}

/// `.iso8601` dates: written as `2026-11-01T09:00:00Z` (whole seconds),
/// read as `JSONDecoder` reads them.
mod iso8601 {
    pub use moshpit_seal::foundation::date::iso8601::{deserialize, serialize};
}

mod iso8601_opt {
    use moshpit_seal::foundation::date::Date;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(date: &Option<Date>, serializer: S) -> Result<S::Ok, S::Error> {
        match date {
            Some(date) => serializer.serialize_str(&date.iso8601()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Date>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|text| {
                Date::parse_iso8601(&text).ok_or_else(|| {
                    serde::de::Error::custom("Expected date string to be ISO8601-formatted.")
                })
            })
            .transpose()
    }
}

/// The saved connections (`ConnectionStore`).
pub struct ConnectionStore {
    defaults: Arc<dyn Defaults>,
    readiness: Readiness,
    connections: Vec<ServerConnection>,
    /// True once `connections` came from preferences known to be readable —
    /// and, when the read skipped anything, once what it skipped is safe.
    is_authoritative: bool,
    /// Stored connections this build could not decode, exactly as stored.
    carried: Vec<Value>,
    /// False when the last read found something other than a list.
    read_a_list: bool,
}

impl ConnectionStore {
    pub const STORAGE_KEY: &'static str = "moshpit.connections";

    /// Reads the list straight away, as the app's store does in `init`.
    pub fn new(defaults: Arc<dyn Defaults>, readiness: Readiness) -> Self {
        let mut store = Self {
            defaults,
            readiness,
            connections: Vec::new(),
            is_authoritative: false,
            carried: Vec::new(),
            read_a_list: true,
        };
        store.load();
        store
    }

    pub fn connections(&self) -> &[ServerConnection] {
        &self.connections
    }

    /// The preferences it reads and writes, and their readiness: where the
    /// items its connections let go of wait to be dropped
    /// ([`crate::secrets::hold_release`]).
    pub(crate) fn preferences(&self) -> (&dyn Defaults, &Readiness) {
        (self.defaults.as_ref(), &self.readiness)
    }

    /// Whether what is shown came from a read that can be trusted; until
    /// then nothing is written back.
    pub fn is_authoritative(&self) -> bool {
        self.is_authoritative
    }

    // Every mutation re-reads first if the last read could not be trusted:
    // whoever taps "+" on a first-run card that should not be there has
    // unlocked the phone to do it, so the real list is readable by now — and
    // the new connection joins it instead of replacing it.

    pub fn add(&mut self, connection: ServerConnection) {
        self.reload_if_needed();
        self.connections.push(connection);
        self.persist();
    }

    /// Replace the connection with the same id; a no-op for an unknown id.
    /// Pointed at another box ([`ServerConnection::points_elsewhere_than`]),
    /// it forgets what sessions remembered about the old one — its
    /// capabilities, its tmux landing — once the list is trusted (B431: the
    /// first connect to the new host routed on the old one's).
    pub fn update(&mut self, connection: ServerConnection) {
        self.reload_if_needed();
        let Some(index) = self.connections.iter().position(|c| c.id == connection.id) else {
            return;
        };
        let moved = connection.points_elsewhere_than(&self.connections[index]);
        let id = connection.id;
        self.connections[index] = connection;
        self.persist();
        if moved && self.is_authoritative {
            session_memory::forget(self.defaults.as_ref(), &self.readiness, id);
        }
    }

    /// Delete connection `id`, and what sessions remember about it
    /// (`session_memory`: its last tmux selection, its host's
    /// capabilities) — once the list is trusted, as the delete itself.
    /// Those keys used to stay for good (B199).
    pub fn delete(&mut self, id: Uuid) {
        self.reload_if_needed();
        self.connections.retain(|c| c.id != id);
        self.persist();
        if self.is_authoritative {
            session_memory::forget(self.defaults.as_ref(), &self.readiness, id);
        }
    }

    /// Remove what sessions remember about connections no longer saved,
    /// among `keys` (the preferences' keys, or those of them starting with
    /// one of [`session_memory::PREFIXES`]): what deletes left behind
    /// before they took it along (B199). Only when every saved connection
    /// can be told ([`Self::saved_ids`]) and the preferences can be
    /// believed — a sealed launch reads the list as empty, and would take
    /// every connection's for an orphan. Returns the keys removed.
    pub fn sweep_session_memory<'a>(&self, keys: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let Some(saved) = self.saved_ids() else {
            return Vec::new();
        };
        let defaults = self.defaults.as_ref();
        if !self.readiness.is_readable(defaults) {
            return Vec::new();
        }
        let orphans = session_memory::orphans(keys, &saved);
        for key in &orphans {
            defaults.set(key, None);
        }
        orphans
    }

    /// Read the list again.
    pub fn load(&mut self) {
        let defaults = self.defaults.as_ref();
        let readable = self.readiness.is_readable(defaults);
        let contents: Contents<ServerConnection> = stored_list::read(defaults, Self::STORAGE_KEY);
        self.is_authoritative =
            readable && stored_list::secure(defaults, Self::STORAGE_KEY, &contents);
        self.connections = contents.elements;
        self.carried = contents.carried;
        self.read_a_list = contents.is_list;
        if readable {
            self.readiness.mark_readable(defaults);
        }
    }

    /// Read again if the last read could not be trusted. The app calls this
    /// when it comes forward and when protected data becomes available.
    pub fn reload_if_needed(&mut self) {
        if !self.is_authoritative {
            self.load();
        }
    }

    /// Every connection saved on this phone, by id: the ones shown and the
    /// ones kept as stored because this build can't decode them. `None` when
    /// that can't be known — the read isn't trustworthy, or something stored
    /// has no readable id (an entry that isn't an object, a value that isn't
    /// a list). Whoever would let go of something because its connection is
    /// gone must take `None` as "any connection may still be here".
    pub fn saved_ids(&self) -> Option<BTreeSet<Uuid>> {
        if !self.is_authoritative || !self.read_a_list {
            return None;
        }
        let mut ids: BTreeSet<Uuid> = self.connections.iter().map(|c| c.id).collect();
        for entry in &self.carried {
            let id = entry
                .as_object()?
                .get("id")?
                .as_str()
                .and_then(Uuid::parse)?;
            ids.insert(id);
        }
        Some(ids)
    }

    /// Every keychain ref a saved connection holds — the shown ones' and the
    /// carried ones' (a newer build's connection still signs in with its
    /// item). `None` when that can't be known, by [`Self::saved_ids`]' rules,
    /// or when a carried entry holds a `keychainRef` that isn't a string:
    /// whoever would delete an item because no connection names it must take
    /// `None` as "any connection may" (B232). `excluding` leaves one
    /// connection out (the one being deleted).
    pub fn keychain_refs(&self, excluding: Option<Uuid>) -> Option<BTreeSet<String>> {
        if !self.is_authoritative || !self.read_a_list {
            return None;
        }
        let mut refs: BTreeSet<String> = self
            .connections
            .iter()
            .filter(|c| Some(c.id) != excluding)
            .filter_map(|c| c.keychain_ref.clone())
            .collect();
        for entry in &self.carried {
            match entry.as_object()?.get("keychainRef") {
                None | Some(Value::Null) => {}
                Some(Value::String(account)) => {
                    refs.insert(account.clone());
                }
                Some(_) => return None,
            }
        }
        Some(refs)
    }

    /// The names of the saved connections that sign in with the SSH key
    /// `key` (or point at its keychain item `keychain_ref`), carried ones
    /// included. `None` when that can't be known ([`Self::keychain_refs`]'
    /// rules).
    pub fn names_using_key(&self, key: Uuid, keychain_ref: Option<&str>) -> Option<Vec<String>> {
        if !self.is_authoritative || !self.read_a_list {
            return None;
        }
        let mut names: Vec<String> = self
            .connections
            .iter()
            .filter(|c| {
                c.ssh_key_id == Some(key)
                    || (keychain_ref.is_some() && c.keychain_ref.as_deref() == keychain_ref)
            })
            .map(ServerConnection::display_name)
            .collect();
        for entry in &self.carried {
            let fields = entry.as_object()?;
            let text = |name: &str| fields.get(name).and_then(Value::as_str);
            let by_id = text("sshKeyId").and_then(Uuid::parse) == Some(key);
            let by_ref = keychain_ref.is_some() && text("keychainRef") == keychain_ref;
            if !(by_id || by_ref) {
                continue;
            }
            names.push(match text("name").filter(|name| !name.is_empty()) {
                Some(name) => name.to_owned(),
                None => format!(
                    "{}@{}",
                    text("username").unwrap_or_default(),
                    text("host").unwrap_or_default()
                ),
            });
        }
        Some(names)
    }

    fn persist(&self) {
        // Never write over a list that could not be read: a sealed file is
        // not an empty list, and the list on disk is the user's.
        if !self.is_authoritative {
            return;
        }
        let Some(data) = stored_list::encode(&self.connections, &self.carried) else {
            return;
        };
        self.defaults.set(
            Self::STORAGE_KEY,
            Some(Plist::Data(Self::adding_rollback_fields(&data))),
        );
    }

    /// Fields `ServerConnection` no longer has but every build up to 1.0.8
    /// REQUIRES on each stored connection, with the values those builds
    /// wrote by default (`ConnectionStore.rollbackFields`).
    pub fn rollback_fields() -> [(&'static str, Value); 6] {
        let number = |n: &str| Value::Number(Number::from_literal(n));
        [
            ("compression", Value::Bool(false)),
            ("forwardAgent", Value::Bool(false)),
            ("keepAliveInterval", number("60")),
            ("useTailscale", Value::Bool(false)),
            ("tmuxAutoAttach", Value::Bool(true)),
            ("tmuxCreateNew", Value::Bool(true)),
        ]
    }

    /// `data`, an encoded list, with [`Self::rollback_fields`] filled in
    /// wherever an object entry lacks them; other entries pass through. As
    /// the app does it, through `JSONSerialization` — but with every string
    /// as stored (Fix (bom-strip), [`crate::stored_list`]); returned
    /// unchanged if it doesn't read as a list.
    pub fn adding_rollback_fields(data: &[u8]) -> Vec<u8> {
        let Some(Value::Array(mut list)) = stored_list::serialization_tree(data) else {
            return data.to_vec();
        };
        for entry in &mut list {
            let Value::Object(fields) = entry else {
                continue;
            };
            for (key, value) in Self::rollback_fields() {
                if !fields.contains_key(key) {
                    fields.insert(key, value);
                }
            }
        }
        json::write_serialization(&Value::Array(list), Style::COMPACT).into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::MemoryDefaults;
    use moshpit_seal::foundation::json::Dialect;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn defaults() -> Arc<MemoryDefaults> {
        Arc::new(MemoryDefaults::new())
    }

    fn sample(name: &str) -> ServerConnection {
        let mut connection =
            ServerConnection::new(Uuid::new_v4(), Date::from_unix(1_762_800_000.5));
        connection.name = name.into();
        connection.host = "example.com".into();
        connection.username = "alice".into();
        connection
    }

    fn names(store: &ConnectionStore) -> Vec<String> {
        store.connections().iter().map(|c| c.name.clone()).collect()
    }

    fn stored_entries(defaults: &MemoryDefaults) -> Vec<Value> {
        let Some(Plist::Data(data)) = defaults.get(ConnectionStore::STORAGE_KEY) else {
            panic!("nothing stored")
        };
        let Value::Array(entries) = json::parse(&data, Dialect::Decoder).unwrap() else {
            panic!("not a list")
        };
        entries
    }

    fn stored_names(defaults: &MemoryDefaults) -> Vec<String> {
        let mut names: Vec<String> = stored_entries(defaults)
            .iter()
            .filter_map(|e| Some(e.as_object()?.get("name")?.as_str()?.to_owned()))
            .collect();
        names.sort();
        names
    }

    /// A phone that starts locked and can be unlocked mid-test.
    fn phone() -> (Arc<AtomicBool>, Readiness) {
        let unlocked = Arc::new(AtomicBool::new(false));
        let flag = unlocked.clone();
        (
            unlocked,
            Readiness::custom(move || flag.load(Ordering::SeqCst)),
        )
    }

    /// Put `connections` in `defaults` the way a previous trusted run would
    /// have, without the sentinel.
    fn write_payload(connections: &[ServerConnection], into: &MemoryDefaults) {
        let scratch = defaults();
        let mut writer = ConnectionStore::new(scratch.clone(), Readiness::Always);
        for c in connections {
            writer.add(c.clone());
        }
        into.set(
            ConnectionStore::STORAGE_KEY,
            scratch.get(ConnectionStore::STORAGE_KEY),
        );
    }

    #[test]
    fn crud_and_persistence_across_instances() {
        let defaults = defaults();
        let mut store = ConnectionStore::new(defaults.clone(), Readiness::Always);
        assert!(store.connections().is_empty());
        let mut a = sample("alpha");
        let b = sample("bravo");
        store.add(a.clone());
        store.add(b.clone());
        a.name = "renamed".into();
        store.update(a.clone());
        store.update(sample("ghost"));
        assert_eq!(names(&store), ["renamed", "bravo"]);
        store.delete(b.id);
        store.delete(Uuid::new_v4());
        let reader = ConnectionStore::new(defaults.clone(), Readiness::Always);
        assert_eq!(names(&reader), ["renamed"]);
        // Whole seconds on disk: `.iso8601` drops the fraction.
        assert_eq!(
            reader.connections()[0].created_at,
            Date::from_unix(1_762_800_000.0)
        );
    }

    #[test]
    fn corrupt_json_recovers_to_an_empty_list_without_crashing() {
        let defaults = defaults();
        defaults.set(
            ConnectionStore::STORAGE_KEY,
            Some(Plist::Data(b"not valid json {{{".to_vec())),
        );
        assert!(
            ConnectionStore::new(defaults.clone(), Readiness::Always)
                .connections()
                .is_empty()
        );
    }

    #[test]
    fn a_launch_before_first_unlock_shows_nothing_and_trusts_nothing() {
        let defaults = defaults();
        let (unlocked, readiness) = phone();
        let mut store = ConnectionStore::new(defaults.clone(), readiness);
        assert!(store.connections().is_empty());
        assert!(!store.is_authoritative());
        assert_eq!(defaults.get(crate::readiness::SENTINEL_KEY), None);

        write_payload(&[sample("alpha"), sample("bravo")], &defaults);
        unlocked.store(true, Ordering::SeqCst);
        store.reload_if_needed();
        assert_eq!(names(&store), ["alpha", "bravo"]);
        assert!(store.is_authoritative());
        assert_eq!(
            defaults.get(crate::readiness::SENTINEL_KEY),
            Some(Plist::Bool(true))
        );
    }

    #[test]
    fn adding_from_a_first_run_card_that_should_not_be_there_keeps_what_was_on_disk() {
        let defaults = defaults();
        let (unlocked, readiness) = phone();
        let mut store = ConnectionStore::new(defaults.clone(), readiness);
        write_payload(&[sample("alpha"), sample("bravo")], &defaults);
        unlocked.store(true, Ordering::SeqCst);
        store.add(sample("charlie"));
        assert_eq!(names(&store), ["alpha", "bravo", "charlie"]);
        assert_eq!(
            ConnectionStore::new(defaults.clone(), Readiness::Always)
                .connections()
                .len(),
            3
        );
    }

    #[test]
    fn nothing_is_written_while_the_file_is_still_sealed() {
        let defaults = defaults();
        let (_, readiness) = phone();
        let mut store = ConnectionStore::new(defaults.clone(), readiness);
        store.add(sample("charlie"));
        assert_eq!(store.connections().len(), 1);
        assert_eq!(defaults.get(ConnectionStore::STORAGE_KEY), None);
    }

    #[test]
    fn the_sentinel_makes_a_locked_read_trusted() {
        let defaults = defaults();
        write_payload(&[sample("alpha")], &defaults);
        defaults.set(crate::readiness::SENTINEL_KEY, Some(Plist::Bool(true)));
        let (_, readiness) = phone();
        let store = ConnectionStore::new(defaults.clone(), readiness);
        assert_eq!(names(&store), ["alpha"]);
        assert!(store.is_authoritative());
    }

    /// Two hosts this build reads, and between them one it can't.
    const LIST_WITH_AN_UNREADABLE_HOST: &str = r#"
    [{"id":"0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F1","name":"alpha","host":"alpha.example.com",
      "port":22,"username":"dev","authMethod":"password","connectionProtocol":"ssh","sshPort":22,
      "moshPortRangeStart":60001,"moshPortRangeEnd":60999,"createdAt":"2026-11-01T09:00:00Z",
      "useTmux":true},
     {"id":"1A2B3C4D-5E6F-4071-8293-A4B5C6D7E8F9","name":"from-a-newer-build","host":"new.example.com",
      "port":22,"username":"dev","authMethod":"certificate","connectionProtocol":"ssh","sshPort":22,
      "moshPortRangeStart":60001,"moshPortRangeEnd":60999,"createdAt":"2026-11-02T09:00:00Z",
      "useTmux":true,"certificateRef":"cert-1"},
     {"id":"2B3C4D5E-6F70-4182-93A4-B5C6D7E8F901","name":"bravo","host":"bravo.example.com",
      "port":2222,"username":"ops","authMethod":"key","connectionProtocol":"mosh","sshPort":2222,
      "moshPortRangeStart":60001,"moshPortRangeEnd":60999,"createdAt":"2026-11-03T09:00:00Z",
      "useTmux":false}]
    "#;

    fn with_unreadable_host() -> Arc<MemoryDefaults> {
        let defaults = defaults();
        defaults.set(
            ConnectionStore::STORAGE_KEY,
            Some(Plist::Data(
                LIST_WITH_AN_UNREADABLE_HOST.as_bytes().to_vec(),
            )),
        );
        defaults
    }

    #[test]
    fn a_host_this_build_cant_decode_is_skipped_not_the_whole_list() {
        let store = ConnectionStore::new(with_unreadable_host(), Readiness::Always);
        assert_eq!(names(&store), ["alpha", "bravo"]);
        assert!(store.is_authoritative());
        assert_eq!(store.saved_ids().unwrap().len(), 3);
    }

    #[test]
    fn saving_after_a_skip_keeps_the_skipped_host_exactly_as_stored() {
        let defaults = with_unreadable_host();
        let mut store = ConnectionStore::new(defaults.clone(), Readiness::Always);
        store.add(sample("charlie"));
        let mut bravo = store.connections()[1].clone();
        bravo.host = "bravo2.example.com".into();
        store.update(bravo);
        assert_eq!(
            stored_names(&defaults),
            ["alpha", "bravo", "charlie", "from-a-newer-build"]
        );
        let entries = stored_entries(&defaults);
        let kept = entries
            .iter()
            .find(|e| {
                e.as_object().unwrap().get("name").unwrap().as_str() == Some("from-a-newer-build")
            })
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(
            kept.get("authMethod").unwrap().as_str(),
            Some("certificate")
        );
        assert_eq!(kept.get("certificateRef").unwrap().as_str(), Some("cert-1"));
        let reread = ConnectionStore::new(defaults.clone(), Readiness::Always);
        assert_eq!(names(&reread), ["alpha", "bravo", "charlie"]);
        assert_eq!(reread.connections()[1].host, "bravo2.example.com");
    }

    #[test]
    fn deleting_every_visible_host_keeps_the_one_it_cant_see() {
        let defaults = with_unreadable_host();
        let mut store = ConnectionStore::new(defaults.clone(), Readiness::Always);
        for id in store.connections().iter().map(|c| c.id).collect::<Vec<_>>() {
            store.delete(id);
        }
        assert!(store.connections().is_empty());
        assert_eq!(stored_names(&defaults), ["from-a-newer-build"]);
    }

    #[test]
    fn the_first_list_with_a_skip_is_backed_up_untouched() {
        let defaults = with_unreadable_host();
        let mut store = ConnectionStore::new(defaults.clone(), Readiness::Always);
        store.add(sample("charlie"));
        assert_eq!(
            defaults.get("moshpit.connections.backup"),
            Some(Plist::Array(vec![Plist::Data(
                LIST_WITH_AN_UNREADABLE_HOST.as_bytes().to_vec()
            )]))
        );
    }

    #[test]
    fn a_skip_on_a_launch_that_cant_trust_the_file_writes_nothing() {
        let defaults = with_unreadable_host();
        let (unlocked, readiness) = phone();
        let mut store = ConnectionStore::new(defaults.clone(), readiness);
        assert_eq!(names(&store), ["alpha", "bravo"]);
        assert!(!store.is_authoritative());
        store.add(sample("charlie"));
        assert_eq!(
            defaults.get(ConnectionStore::STORAGE_KEY),
            Some(Plist::Data(
                LIST_WITH_AN_UNREADABLE_HOST.as_bytes().to_vec()
            ))
        );
        assert_eq!(defaults.get("moshpit.connections.backup"), None);
        assert_eq!(store.saved_ids(), None);

        unlocked.store(true, Ordering::SeqCst);
        store.add(sample("delta"));
        assert!(store.is_authoritative());
        assert_eq!(
            stored_names(&defaults),
            ["alpha", "bravo", "delta", "from-a-newer-build"]
        );
    }

    /// What every build up to 1.0.8 demands of each stored connection.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    struct EarlierBuildConnection {
        id: Uuid,
        name: String,
        host: String,
        port: i64,
        username: String,
        auth_method: String,
        connection_protocol: String,
        ssh_port: i64,
        mosh_port_range_start: i64,
        mosh_port_range_end: i64,
        #[serde(with = "iso8601")]
        created_at: Date,
        forward_agent: bool,
        compression: bool,
        keep_alive_interval: i64,
        use_tailscale: bool,
        tmux_auto_attach: bool,
        tmux_create_new: bool,
        use_tmux: bool,
    }

    #[test]
    fn what_this_build_saves_still_decodes_in_an_earlier_build() {
        let defaults = defaults();
        let mut store = ConnectionStore::new(defaults.clone(), Readiness::Always);
        store.add(sample("alpha"));
        store.add(sample("bravo"));
        let Some(Plist::Data(data)) = defaults.get(ConnectionStore::STORAGE_KEY) else {
            panic!()
        };
        let earlier: Vec<EarlierBuildConnection> = json::decode(&data).unwrap();
        assert_eq!(
            earlier.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["alpha", "bravo"]
        );
        assert!(
            earlier
                .iter()
                .all(|c| c.keep_alive_interval == 60 && c.tmux_create_new)
        );
    }

    #[test]
    fn a_list_an_earlier_build_stored_still_loads_and_saves_back_readable_by_it() {
        let earlier_list = r#"[{
          "id": "5D1A7E0C-3F42-4B8E-9C61-0A2B3C4D5E6F", "name": "studio",
          "host": "studio.example.com", "port": 2222, "username": "dev",
          "authMethod": "key", "connectionProtocol": "mosh", "sshPort": 2222,
          "moshPortRangeStart": 60001, "moshPortRangeEnd": 60999,
          "moshServerPath": "/opt/homebrew/bin/mosh-server", "keychainRef": "ref-studio",
          "createdAt": "2026-08-27T09:00:00Z", "lastConnectedAt": "2026-11-01T12:30:00Z",
          "startupCommand": "cd ~/code", "forwardAgent": true, "compression": true,
          "keepAliveInterval": 30, "jumpHost": "bastion.example.com", "useTailscale": true,
          "tmuxAutoAttach": false, "tmuxCreateNew": false, "useTmux": true,
          "predictModeRaw": "always", "roamOnCellular": false
        }]"#;
        let defaults = defaults();
        defaults.set(
            ConnectionStore::STORAGE_KEY,
            Some(Plist::Data(earlier_list.as_bytes().to_vec())),
        );
        let mut store = ConnectionStore::new(defaults.clone(), Readiness::Always);
        let loaded = store.connections()[0].clone();
        assert_eq!(
            loaded.id.to_string(),
            "5D1A7E0C-3F42-4B8E-9C61-0A2B3C4D5E6F"
        );
        assert_eq!(loaded.auth_method, AuthMethod::Key);
        assert_eq!(loaded.connection_protocol, ConnectionProtocol::Mosh);
        assert_eq!(loaded.predict_mode(), PredictMode::Always);
        assert_eq!(loaded.roam_on_cellular, Some(false));
        assert_eq!(loaded.multiplexer(), Multiplexer::Tmux);
        assert!(loaded.last_connected_at.is_some());

        store.update(loaded);
        let Some(Plist::Data(data)) = defaults.get(ConnectionStore::STORAGE_KEY) else {
            panic!()
        };
        let earlier: Vec<EarlierBuildConnection> = json::decode(&data).unwrap();
        assert_eq!(earlier[0].name, "studio");
        assert!(earlier[0].use_tmux);
    }

    /// What a session remembers about `id`, as the app stores it.
    fn remember(defaults: &MemoryDefaults, id: Uuid) {
        let id = id.to_string();
        defaults.set(
            &session_memory::tmux_selection_key(&id),
            Some(Plist::Data(br#"{"session":"$1"}"#.to_vec())),
        );
        defaults.set(
            &session_memory::host_capabilities_key(&id),
            Some(Plist::Data(br#"{"tmux":true}"#.to_vec())),
        );
    }

    fn remembers(defaults: &MemoryDefaults, id: Uuid) -> [bool; 2] {
        let id = id.to_string();
        [
            defaults
                .get(&session_memory::tmux_selection_key(&id))
                .is_some(),
            defaults
                .get(&session_memory::host_capabilities_key(&id))
                .is_some(),
        ]
    }

    /// B199: deleting a connection left its `moshpit.tmux.lastSelection.<id>`
    /// and `hostcaps.<id>` behind for good.
    #[test]
    fn a_delete_takes_what_sessions_remember_with_it() {
        let defaults = defaults();
        let mut store = ConnectionStore::new(defaults.clone(), Readiness::Always);
        let (gone, kept) = (sample("gone"), sample("kept"));
        store.add(gone.clone());
        store.add(kept.clone());
        remember(&defaults, gone.id);
        remember(&defaults, kept.id);
        store.delete(gone.id);
        assert_eq!(remembers(&defaults, gone.id), [false, false]);
        assert_eq!(remembers(&defaults, kept.id), [true, true]);
    }

    /// B431: a connection pointed at another host (or port) kept what
    /// sessions remembered about the box it pointed at before — the first
    /// connect to the new host routed on the old one's capabilities (tmux
    /// "missing", mosh falling back to SSH) and landed on its tmux `$N`.
    /// A move forgets both; a save that keeps the address keeps them.
    #[test]
    fn a_move_to_another_host_or_port_forgets_what_sessions_remembered() {
        let defaults = defaults();
        let mut store = ConnectionStore::new(defaults.clone(), Readiness::Always);
        let box_ = sample("box");
        store.add(box_.clone());

        remember(&defaults, box_.id);
        let mut renamed = box_.clone();
        renamed.name = "renamed".into();
        renamed.username = "bob".into();
        store.update(renamed.clone());
        assert_eq!(remembers(&defaults, box_.id), [true, true], "a rename");
        let mut cased = renamed.clone();
        cased.host = "EXAMPLE.com".into();
        store.update(cased.clone());
        assert_eq!(remembers(&defaults, box_.id), [true, true], "host case");

        let mut moved = cased.clone();
        moved.host = "other.example.com".into();
        store.update(moved.clone());
        assert_eq!(
            remembers(&defaults, box_.id),
            [false, false],
            "another host"
        );

        for change in [
            |c: &mut ServerConnection| c.port = 2222,
            |c: &mut ServerConnection| c.ssh_port = 2200,
        ] {
            remember(&defaults, box_.id);
            let mut next = store.connections()[0].clone();
            change(&mut next);
            store.update(next);
            assert_eq!(
                remembers(&defaults, box_.id),
                [false, false],
                "another port"
            );
        }
        assert!(moved.points_elsewhere_than(&cased));
        assert!(!cased.points_elsewhere_than(&renamed));
    }

    /// A delete on a sealed launch writes nothing — the list or the keys.
    #[test]
    fn a_sealed_delete_takes_nothing() {
        let defaults = defaults();
        let gone = sample("gone");
        write_payload(std::slice::from_ref(&gone), &defaults);
        remember(&defaults, gone.id);
        let (_, readiness) = phone();
        let mut store = ConnectionStore::new(defaults.clone(), readiness);
        store.delete(gone.id);
        assert_eq!(stored_names(&defaults), ["gone"]);
        assert_eq!(remembers(&defaults, gone.id), [true, true]);
    }

    fn session_keys(defaults: &MemoryDefaults) -> Vec<String> {
        defaults
            .snapshot()
            .into_keys()
            .filter(|key| session_memory::connection_of(key).is_some())
            .collect()
    }

    /// B199: what earlier deletes left is swept — only the two families,
    /// only connections not saved (the carried one counts as saved), and
    /// only once the list can be trusted.
    #[test]
    fn the_sweep_takes_only_what_no_saved_connection_has() {
        let defaults = with_unreadable_host();
        let carried = Uuid::parse("1A2B3C4D-5E6F-4071-8293-A4B5C6D7E8F9").unwrap();
        let shown = Uuid::parse("0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F1").unwrap();
        let (gone, also_gone) = (Uuid::new_v4(), Uuid::new_v4());
        for id in [carried, shown, gone, also_gone] {
            remember(&defaults, id);
        }
        defaults.set("hostcaps.not-a-uuid", Some(Plist::Bool(true)));

        // Sealed: the list reads as empty, so nothing can be told.
        let (unlocked, readiness) = phone();
        let mut store = ConnectionStore::new(defaults.clone(), readiness);
        assert!(
            store
                .sweep_session_memory(session_keys(&defaults).iter().map(String::as_str))
                .is_empty()
        );
        assert_eq!(remembers(&defaults, gone), [true, true]);

        unlocked.store(true, Ordering::SeqCst);
        store.reload_if_needed();
        let mut removed =
            store.sweep_session_memory(session_keys(&defaults).iter().map(String::as_str));
        removed.sort();
        let mut expected: Vec<String> = [gone, also_gone]
            .iter()
            .flat_map(|id| {
                let id = id.to_string();
                [
                    session_memory::host_capabilities_key(&id),
                    session_memory::tmux_selection_key(&id),
                ]
            })
            .collect();
        expected.sort();
        assert_eq!(removed, expected);
        assert_eq!(remembers(&defaults, gone), [false, false]);
        assert_eq!(remembers(&defaults, also_gone), [false, false]);
        assert_eq!(remembers(&defaults, shown), [true, true]);
        assert_eq!(remembers(&defaults, carried), [true, true]);
        assert_eq!(defaults.get("hostcaps.not-a-uuid"), Some(Plist::Bool(true)));
    }

    /// A stored value that isn't a list can't be told by id: nothing is
    /// anyone's orphan.
    #[test]
    fn the_sweep_takes_nothing_when_the_ids_cant_be_told() {
        let defaults = defaults();
        defaults.set(
            ConnectionStore::STORAGE_KEY,
            Some(Plist::Data(b"{}".to_vec())),
        );
        let gone = Uuid::new_v4();
        remember(&defaults, gone);
        let store = ConnectionStore::new(defaults.clone(), Readiness::Always);
        assert!(
            store
                .sweep_session_memory(session_keys(&defaults).iter().map(String::as_str))
                .is_empty()
        );
        assert_eq!(remembers(&defaults, gone), [true, true]);
    }

    #[test]
    fn the_multiplexer_falls_back_to_use_tmux_and_keeps_it_in_step() {
        let mut c = sample("x");
        assert_eq!(c.multiplexer(), Multiplexer::None);
        c.use_tmux = true;
        assert_eq!(c.multiplexer(), Multiplexer::Tmux);
        c.set_multiplexer(Multiplexer::Herdr);
        assert!(!c.use_tmux);
        assert_eq!(c.multiplexer_raw.as_deref(), Some("herdr"));
        c.multiplexer_raw = Some("zellij".into());
        assert_eq!(c.multiplexer(), Multiplexer::None);
        assert_eq!(c.display_name(), "x");
        c.name.clear();
        assert_eq!(c.display_name(), "alice@example.com");
    }
}
