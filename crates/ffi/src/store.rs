//! The store's API for the hosts (Swift, Kotlin), through UniFFI: what
//! `moshpit-store` reads and writes, behind the two traits a platform
//! implements — its preferences ([`PreferencesHost`]: on iOS
//! `UserDefaults.standard`) and its keychain ([`KeychainHost`]).
//!
//! Not a hot path: a store is read when the app starts and comes forward,
//! written when the user changes something. The byte channel and the tmux
//! parser keep their hand-written C ABI.
//!
//! Dates cross as seconds since 2001-01-01 (Swift's
//! `timeIntervalSinceReferenceDate`, so a `Date` round-trips bit for bit),
//! UUIDs as their upper-case strings. A preferences value crosses as a flat
//! list of nodes (UniFFI's Swift bindings have no recursive enums): node 0
//! is the value, an array or dictionary names its members by index, always
//! greater than its own.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::uuid::Uuid;
use moshpit_store::app_themes::{self, AppTheme, AppThemeStore};
use moshpit_store::connections::{self, ConnectionStore, ServerConnection};
use moshpit_store::push::{PushPairing, PushPairingFile};
use moshpit_store::secrets;
use moshpit_store::settings::{self, Settings};
use moshpit_store::shortcuts::{self, ShortcutStore, TerminalShortcut};
use moshpit_store::ssh_keys::{self, SshKeyRecord, SshKeyStore};
use moshpit_store::themes::{self, AnsiSlot, TerminalTheme, ThemeStore};
use moshpit_store::{Defaults, Keychain, KeychainError, Plist, Readiness};

// MARK: - Errors

/// A value from the host the store can't take.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum StoreError {
    /// Not a UUID string (`UUID(uuidString:)` would say nil).
    InvalidId { id: String },
    /// No setting of that name, or a value of another type than the
    /// setting's.
    InvalidSetting { name: String },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidId { id } => write!(f, "not a UUID: {id:?}"),
            Self::InvalidSetting { name } => write!(f, "no such setting, or not its type: {name}"),
        }
    }
}

impl std::error::Error for StoreError {}

fn uuid(id: &str) -> Result<Uuid, StoreError> {
    Uuid::parse(id).ok_or_else(|| StoreError::InvalidId { id: id.to_owned() })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

// MARK: - Preferences values

/// One node of a preferences value.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum PlistNode {
    Bool {
        value: bool,
    },
    Int {
        value: i64,
    },
    Double {
        value: f64,
    },
    String {
        value: String,
    },
    Data {
        value: Vec<u8>,
    },
    /// Seconds since 2001-01-01T00:00:00Z.
    Date {
        seconds: f64,
    },
    /// Members by node index.
    Array {
        members: Vec<u32>,
    },
    /// Keys, and their values by node index, in the same order.
    Dictionary {
        keys: Vec<String>,
        members: Vec<u32>,
    },
}

/// A preferences value: `nodes[0]`, its members after it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlistValue {
    pub nodes: Vec<PlistNode>,
}

fn to_value(value: &Plist) -> PlistValue {
    fn push(value: &Plist, nodes: &mut Vec<PlistNode>) -> u32 {
        let index = nodes.len();
        nodes.push(PlistNode::Bool { value: false });
        let node = match value {
            Plist::Bool(v) => PlistNode::Bool { value: *v },
            Plist::Int(v) => PlistNode::Int { value: *v },
            Plist::Double(v) => PlistNode::Double { value: *v },
            Plist::String(v) => PlistNode::String { value: v.clone() },
            Plist::Data(v) => PlistNode::Data { value: v.clone() },
            Plist::Date(v) => PlistNode::Date { seconds: *v },
            Plist::Array(items) => PlistNode::Array {
                members: items.iter().map(|item| push(item, nodes)).collect(),
            },
            Plist::Dictionary(entries) => PlistNode::Dictionary {
                keys: entries.iter().map(|(k, _)| k.clone()).collect(),
                members: entries.iter().map(|(_, v)| push(v, nodes)).collect(),
            },
        };
        nodes[index] = node;
        u32::try_from(index).expect("a value of fewer than 2^32 nodes")
    }
    let mut nodes = Vec::new();
    push(value, &mut nodes);
    PlistValue { nodes }
}

/// The value a host gave back; `None` for a malformed node list (a member
/// index out of range or not after its parent).
fn from_value(value: &PlistValue) -> Option<Plist> {
    fn node(nodes: &[PlistNode], index: usize) -> Option<Plist> {
        let member = |m: u32| -> Option<Plist> {
            let m = m as usize;
            (m > index).then(|| node(nodes, m)).flatten()
        };
        Some(match nodes.get(index)? {
            PlistNode::Bool { value } => Plist::Bool(*value),
            PlistNode::Int { value } => Plist::Int(*value),
            PlistNode::Double { value } => Plist::Double(*value),
            PlistNode::String { value } => Plist::String(value.clone()),
            PlistNode::Data { value } => Plist::Data(value.clone()),
            PlistNode::Date { seconds } => Plist::Date(*seconds),
            PlistNode::Array { members } => {
                Plist::Array(members.iter().map(|&m| member(m)).collect::<Option<_>>()?)
            }
            PlistNode::Dictionary { keys, members } => {
                if keys.len() != members.len() {
                    return None;
                }
                Plist::Dictionary(
                    keys.iter()
                        .zip(members)
                        .map(|(k, &m)| member(m).map(|v| (k.clone(), v)))
                        .collect::<Option<_>>()?,
                )
            }
        })
    }
    node(&value.nodes, 0)
}

// MARK: - Host traits

/// The app's preferences (iOS: `UserDefaults.standard`; Android: its
/// `SharedPreferences`). A value set comes back as set — or as the value
/// stored before when that one is equal to it, as `UserDefaults` does.
#[uniffi::export(with_foreign)]
pub trait PreferencesHost: Send + Sync {
    /// `object(forKey:)`.
    fn get(&self, key: String) -> Option<PlistValue>;
    /// `set(_:forKey:)`; `None` removes.
    fn set(&self, key: String, value: Option<PlistValue>);
    /// Whether protected data is readable now (iOS
    /// `isProtectedDataAvailable`; Android: the user unlocked since boot).
    fn protected_data_available(&self) -> bool;
}

/// Why the keychain did not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum KeychainFailure {
    NotFound,
    Duplicate,
    AuthenticationFailed,
    UserCanceled,
    /// The device is locked (`errSecInteractionNotAllowed`).
    InteractionNotAllowed,
    InvalidData,
    Other {
        message: String,
    },
}

impl std::fmt::Display for KeychainFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", KeychainError::from(self.clone()))
    }
}

impl std::error::Error for KeychainFailure {}

impl From<uniffi::UnexpectedUniFFICallbackError> for KeychainFailure {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Other {
            message: error.reason,
        }
    }
}

impl From<KeychainFailure> for KeychainError {
    fn from(failure: KeychainFailure) -> Self {
        match failure {
            KeychainFailure::NotFound => Self::NotFound,
            KeychainFailure::Duplicate => Self::Duplicate,
            KeychainFailure::AuthenticationFailed => Self::AuthenticationFailed,
            KeychainFailure::UserCanceled => Self::UserCanceled,
            KeychainFailure::InteractionNotAllowed => Self::InteractionNotAllowed,
            KeychainFailure::InvalidData => Self::InvalidData,
            KeychainFailure::Other { message } => Self::Other(message),
        }
    }
}

/// The keychain (iOS: the app's generic passwords; Android: the Keystore).
/// Secrets stay there: the stores hold account names.
#[uniffi::export(with_foreign)]
pub trait KeychainHost: Send + Sync {
    /// Store `secret` under `account` in place (a failed write leaves the
    /// item as it was); `require_biometry` gates every later read.
    fn save(
        &self,
        account: String,
        secret: Vec<u8>,
        require_biometry: bool,
    ) -> Result<(), KeychainFailure>;
    fn load(&self, account: String, prompt: String) -> Result<Vec<u8>, KeychainFailure>;
    fn delete(&self, account: String) -> Result<(), KeychainFailure>;
    fn exists(&self, account: String) -> bool;
    /// Every item's account, read without reading an item (no prompt):
    /// what orphan recovery looks through. A failure when it can't say —
    /// never an empty list for "don't know".
    fn accounts(&self) -> Result<Vec<String>, KeychainFailure>;
    /// The item's bytes if they can be had without asking anyone: an item
    /// behind Face ID fails `InteractionNotAllowed` instead of prompting.
    fn load_silently(&self, account: String) -> Result<Vec<u8>, KeychainFailure>;
}

pub(crate) struct HostDefaults(pub(crate) Arc<dyn PreferencesHost>);

impl Defaults for HostDefaults {
    fn get(&self, key: &str) -> Option<Plist> {
        self.0.get(key.to_owned()).as_ref().and_then(from_value)
    }

    fn set(&self, key: &str, value: Option<Plist>) {
        self.0.set(key.to_owned(), value.as_ref().map(to_value));
    }

    fn protected_data_available(&self) -> bool {
        self.0.protected_data_available()
    }
}

pub(crate) struct HostKeychain(pub(crate) Arc<dyn KeychainHost>);

impl Keychain for HostKeychain {
    fn save(
        &self,
        account: &str,
        secret: &[u8],
        require_biometry: bool,
    ) -> Result<(), KeychainError> {
        self.0
            .save(account.to_owned(), secret.to_vec(), require_biometry)
            .map_err(Into::into)
    }

    fn load(&self, account: &str, prompt: &str) -> Result<Vec<u8>, KeychainError> {
        self.0
            .load(account.to_owned(), prompt.to_owned())
            .map_err(Into::into)
    }

    fn delete(&self, account: &str) -> Result<(), KeychainError> {
        self.0.delete(account.to_owned()).map_err(Into::into)
    }

    fn exists(&self, account: &str) -> bool {
        self.0.exists(account.to_owned())
    }

    fn accounts(&self) -> Result<Vec<String>, KeychainError> {
        self.0.accounts().map_err(Into::into)
    }

    fn load_silently(&self, account: &str) -> Result<Vec<u8>, KeychainError> {
        self.0.load_silently(account.to_owned()).map_err(Into::into)
    }
}

fn defaults(host: Arc<dyn PreferencesHost>) -> Arc<dyn Defaults> {
    Arc::new(HostDefaults(host))
}

/// Where a store hears whether the preferences can be believed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ReadinessMode {
    /// The sentinel, or the host's `protected_data_available` (the app).
    System,
    /// Always readable (tests, scratch domains).
    Always,
}

fn readiness(mode: ReadinessMode) -> Readiness {
    match mode {
        ReadinessMode::System => Readiness::System,
        ReadinessMode::Always => Readiness::Always,
    }
}

/// `DefaultsReadiness.isReadable`.
#[uniffi::export]
pub fn store_defaults_readable(host: Arc<dyn PreferencesHost>, mode: ReadinessMode) -> bool {
    readiness(mode).is_readable(&HostDefaults(host))
}

/// `DefaultsReadiness.markReadable`.
#[uniffi::export]
pub fn store_mark_defaults_readable(host: Arc<dyn PreferencesHost>) {
    Readiness::Always.mark_readable(&HostDefaults(host));
}

/// The key of the sentinel the stores write after their first trusted
/// read (tests: a sealed launch is one without it).
#[uniffi::export]
pub fn store_defaults_sentinel_key() -> String {
    moshpit_store::readiness::SENTINEL_KEY.to_owned()
}

/// A fresh keychain account for a connection's password or key:
/// `moshpit.secret.<lower-case UUID>`.
#[uniffi::export]
pub fn store_new_keychain_ref() -> String {
    moshpit_store::host::new_keychain_ref()
}

// MARK: - Connections

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
// Named for the bindings so it can live beside the app's own `AuthMethod` in
// one Swift module (the app compiles the generated file into itself).
#[uniffi(name = "StoreAuthMethod")]
pub enum AuthMethod {
    Password,
    Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
// Named for the bindings so it can live beside the app's own `ConnectionProtocol` in
// one Swift module (the app compiles the generated file into itself).
#[uniffi(name = "StoreConnectionProtocol")]
pub enum ConnectionProtocol {
    Ssh,
    Mosh,
}

/// A saved connection, field for field (`ServerConnection`).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConnectionRecord {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_method: AuthMethod,
    pub connection_protocol: ConnectionProtocol,
    pub ssh_port: i64,
    pub mosh_port_range_start: i64,
    pub mosh_port_range_end: i64,
    pub mosh_server_path: Option<String>,
    /// The keychain account of the password or key.
    pub keychain_ref: Option<String>,
    pub created_at: f64,
    pub last_connected_at: Option<f64>,
    pub use_tmux: bool,
    pub predict_mode_raw: Option<String>,
    pub roam_on_cellular: Option<bool>,
    pub tmux_path: Option<String>,
    pub herdr_path: Option<String>,
    pub multiplexer_raw: Option<String>,
    pub ssh_key_id: Option<String>,
    pub ssh_key_algorithm_raw: Option<String>,
    pub use_socks_proxy: Option<bool>,
    pub socks_proxy_host: Option<String>,
    pub socks_proxy_port: Option<i64>,
    /// The emoji its mark shows in place of its host key's face.
    pub emoji: Option<String>,
    /// The saved connection it goes through (a UUID; F1).
    #[uniffi(default = None)]
    pub via_connection_id: Option<String>,
}

impl From<&ServerConnection> for ConnectionRecord {
    fn from(c: &ServerConnection) -> Self {
        Self {
            id: c.id.to_string(),
            name: c.name.clone(),
            host: c.host.clone(),
            port: c.port,
            username: c.username.clone(),
            auth_method: match c.auth_method {
                connections::AuthMethod::Password => AuthMethod::Password,
                connections::AuthMethod::Key => AuthMethod::Key,
            },
            connection_protocol: match c.connection_protocol {
                connections::ConnectionProtocol::Ssh => ConnectionProtocol::Ssh,
                connections::ConnectionProtocol::Mosh => ConnectionProtocol::Mosh,
            },
            ssh_port: c.ssh_port,
            mosh_port_range_start: c.mosh_port_range_start,
            mosh_port_range_end: c.mosh_port_range_end,
            mosh_server_path: c.mosh_server_path.clone(),
            keychain_ref: c.keychain_ref.clone(),
            created_at: c.created_at.since_reference(),
            last_connected_at: c.last_connected_at.map(Date::since_reference),
            use_tmux: c.use_tmux,
            predict_mode_raw: c.predict_mode_raw.clone(),
            roam_on_cellular: c.roam_on_cellular,
            tmux_path: c.tmux_path.clone(),
            herdr_path: c.herdr_path.clone(),
            multiplexer_raw: c.multiplexer_raw.clone(),
            ssh_key_id: c.ssh_key_id.map(|id| id.to_string()),
            ssh_key_algorithm_raw: c.ssh_key_algorithm_raw.clone(),
            use_socks_proxy: c.use_socks_proxy,
            socks_proxy_host: c.socks_proxy_host.clone(),
            socks_proxy_port: c.socks_proxy_port,
            emoji: c.emoji.clone(),
            via_connection_id: c.via_connection_id.map(|id| id.to_string()),
        }
    }
}

impl TryFrom<ConnectionRecord> for ServerConnection {
    type Error = StoreError;

    fn try_from(c: ConnectionRecord) -> Result<Self, StoreError> {
        let mut connection = ServerConnection::new(uuid(&c.id)?, Date(c.created_at));
        connection.name = c.name;
        connection.host = c.host;
        connection.port = c.port;
        connection.username = c.username;
        connection.auth_method = match c.auth_method {
            AuthMethod::Password => connections::AuthMethod::Password,
            AuthMethod::Key => connections::AuthMethod::Key,
        };
        connection.connection_protocol = match c.connection_protocol {
            ConnectionProtocol::Ssh => connections::ConnectionProtocol::Ssh,
            ConnectionProtocol::Mosh => connections::ConnectionProtocol::Mosh,
        };
        connection.ssh_port = c.ssh_port;
        connection.mosh_port_range_start = c.mosh_port_range_start;
        connection.mosh_port_range_end = c.mosh_port_range_end;
        connection.mosh_server_path = c.mosh_server_path;
        connection.keychain_ref = c.keychain_ref;
        connection.last_connected_at = c.last_connected_at.map(Date);
        connection.use_tmux = c.use_tmux;
        connection.predict_mode_raw = c.predict_mode_raw;
        connection.roam_on_cellular = c.roam_on_cellular;
        connection.tmux_path = c.tmux_path;
        connection.herdr_path = c.herdr_path;
        connection.multiplexer_raw = c.multiplexer_raw;
        connection.ssh_key_id = c.ssh_key_id.as_deref().map(uuid).transpose()?;
        connection.ssh_key_algorithm_raw = c.ssh_key_algorithm_raw;
        connection.use_socks_proxy = c.use_socks_proxy;
        connection.socks_proxy_host = c.socks_proxy_host;
        connection.socks_proxy_port = c.socks_proxy_port;
        connection.emoji = c.emoji;
        connection.via_connection_id = match c.via_connection_id.as_deref() {
            None | Some("") => None,
            Some(id) => Some(uuid(id)?),
        };
        Ok(connection)
    }
}

/// The saved connections that go through another (F1), by id, as the
/// store last read or wrote them: what a session's spec reaching the core
/// without its way through is checked against
/// ([`crate::session::SessionSpec`]), so a caller that forgot it fails the
/// connect instead of dialling the host directly.
static THROUGH: LazyLock<Mutex<HashMap<Uuid, Uuid>>> = LazyLock::new(Default::default);

/// Whether the saved connection `id` goes through another (F1).
pub(crate) fn goes_through(id: &str) -> bool {
    Uuid::parse(id).is_some_and(|id| lock(&THROUGH).contains_key(&id))
}

/// Notes which of `store`'s connections go through another, and that
/// `gone` (deleted) goes through nothing.
fn note_routes(store: &ConnectionStore, gone: Option<Uuid>) {
    let mut through = lock(&THROUGH);
    if let Some(gone) = gone {
        through.remove(&gone);
    }
    for c in store.connections() {
        match c.via_connection_id {
            Some(via) => through.insert(c.id, via),
            None => through.remove(&c.id),
        };
    }
}

/// The saved connections (`ConnectionStore`), read when made.
#[derive(uniffi::Object)]
pub struct ConnectionStoreHandle(Mutex<ConnectionStore>);

#[uniffi::export]
impl ConnectionStoreHandle {
    #[uniffi::constructor]
    pub fn new(host: Arc<dyn PreferencesHost>, mode: ReadinessMode) -> Arc<Self> {
        let store = ConnectionStore::new(defaults(host), readiness(mode));
        note_routes(&store, None);
        Arc::new(Self(Mutex::new(store)))
    }

    pub fn connections(&self) -> Vec<ConnectionRecord> {
        lock(&self.0).connections().iter().map(Into::into).collect()
    }

    pub fn is_authoritative(&self) -> bool {
        lock(&self.0).is_authoritative()
    }

    pub fn add(&self, connection: ConnectionRecord) -> Result<(), StoreError> {
        let mut store = lock(&self.0);
        store.add(connection.try_into()?);
        note_routes(&store, None);
        Ok(())
    }

    pub fn update(&self, connection: ConnectionRecord) -> Result<(), StoreError> {
        let mut store = lock(&self.0);
        store.update(connection.try_into()?);
        note_routes(&store, None);
        Ok(())
    }

    pub fn delete(&self, id: String) -> Result<(), StoreError> {
        let id = uuid(&id)?;
        let mut store = lock(&self.0);
        store.delete(id);
        note_routes(&store, Some(id));
        Ok(())
    }

    /// Connection `id` signed in now (B783): its `last_connected_at`,
    /// written at most once a day (`ConnectionStore::mark_connected`) —
    /// what tells a host connected to before. Whether it was written.
    pub fn mark_connected(&self, id: String) -> Result<bool, StoreError> {
        Ok(lock(&self.0).mark_connected(uuid(&id)?, Date::now()))
    }

    pub fn reload_if_needed(&self) {
        let mut store = lock(&self.0);
        store.reload_if_needed();
        note_routes(&store, None);
    }

    /// Read the list again, trusted or not (`ConnectionStore.load()`).
    pub fn load(&self) {
        let mut store = lock(&self.0);
        store.load();
        note_routes(&store, None);
    }

    /// Remove what sessions remember about connections no longer saved,
    /// among `keys` (the preferences' keys): what deletes left behind before
    /// they took it along (B199). Nothing unless every saved connection can
    /// be told and the preferences believed
    /// (`ConnectionStore::sweep_session_memory`). The keys removed.
    pub fn sweep_session_memory(&self, keys: Vec<String>) -> Vec<String> {
        lock(&self.0).sweep_session_memory(keys.iter().map(String::as_str))
    }

    /// Every id saved on this device, shown or carried; `None` when that
    /// can't be known (see `ConnectionStore::saved_ids`).
    pub fn saved_ids(&self) -> Option<Vec<String>> {
        lock(&self.0)
            .saved_ids()
            .map(|ids| ids.iter().map(ToString::to_string).collect())
    }

    /// Every keychain ref a saved connection holds, the carried ones' too;
    /// `None` when that can't be known (`ConnectionStore::keychain_refs`,
    /// B232). `excluding` leaves one connection out.
    pub fn keychain_refs(
        &self,
        excluding: Option<String>,
    ) -> Result<Option<Vec<String>>, StoreError> {
        let excluding = excluding.as_deref().map(uuid).transpose()?;
        Ok(lock(&self.0)
            .keychain_refs(excluding)
            .map(|refs| refs.into_iter().collect()))
    }

    /// The names of the saved connections that sign in with key `key` (or
    /// name its keychain item `keychain_ref`), carried ones included; `None`
    /// when that can't be known (`ConnectionStore::names_using_key`).
    pub fn names_using_key(
        &self,
        key: String,
        keychain_ref: Option<String>,
    ) -> Result<Option<Vec<String>>, StoreError> {
        let key = uuid(&key)?;
        Ok(lock(&self.0).names_using_key(key, keychain_ref.as_deref()))
    }

    /// Delete connection `id`, and its keychain item once no other
    /// connection and no key record names it (`secrets::delete_connection`,
    /// B232). The record goes whatever becomes of the item.
    pub fn delete_releasing_secret(
        &self,
        id: String,
        keys: Arc<KeyStoreHandle>,
        keychain: Arc<dyn KeychainHost>,
    ) -> Result<StoreSecretFate, StoreError> {
        let id = uuid(&id)?;
        // Connections, then keys: the order every two-store call locks in.
        let mut connections = lock(&self.0);
        let mut keys = lock(&keys.0);
        Ok(
            secrets::delete_connection(id, &mut connections, &mut keys, &HostKeychain(keychain))
                .into(),
        )
    }

    /// Drop `account`, an item a connection let go of, once no saved
    /// connection and no key record names it — listed until it is
    /// (`secrets::release_item`, B325/B428). B810: who names what is the
    /// core's to tell; the app used to work it out in Swift and pass it in.
    pub fn release_item(
        &self,
        account: String,
        keys: Arc<KeyStoreHandle>,
        keychain: Arc<dyn KeychainHost>,
    ) -> StoreSecretFate {
        let mut connections = lock(&self.0);
        let mut keys = lock(&keys.0);
        secrets::release_item(
            &account,
            &mut connections,
            &mut keys,
            &HostKeychain(keychain),
        )
        .into()
    }

    /// The items earlier launches listed to drop, settled over what the
    /// records name now (`secrets::release_pending`, B428): once a launch,
    /// before orphan recovery.
    pub fn release_pending(
        &self,
        keys: Arc<KeyStoreHandle>,
        keychain: Arc<dyn KeychainHost>,
    ) -> Vec<StoreSettledRelease> {
        let mut connections = lock(&self.0);
        let mut keys = lock(&keys.0);
        secrets::release_pending(&mut connections, &mut keys, &HostKeychain(keychain))
            .into_iter()
            .map(|(account, fate)| StoreSettledRelease {
                account,
                fate: fate.into(),
            })
            .collect()
    }
}

/// What became of a deleted connection's keychain item.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum StoreSecretFate {
    /// Deleted: nothing else named it.
    Deleted,
    /// Kept: something else names it (or there was none).
    Kept,
    /// Kept: what else names it can't be told now. Listed for a later
    /// launch (B428).
    Unknown,
    /// The keychain wouldn't delete it. Listed for a later launch (B428).
    Failed { message: String },
}

impl From<secrets::ItemFate> for StoreSecretFate {
    fn from(fate: secrets::ItemFate) -> Self {
        match fate {
            secrets::ItemFate::Deleted => Self::Deleted,
            secrets::ItemFate::Kept => Self::Kept,
            secrets::ItemFate::Unknown => Self::Unknown,
            secrets::ItemFate::Failed(error) => Self::Failed {
                message: error.to_string(),
            },
        }
    }
}

// MARK: - Items let go of (B428)

/// Put `account` on the list of items a connection let go of and can't drop
/// yet — its session still signs in with it (B325), or the drop couldn't
/// happen now — so it goes later, never offered back as a recovered key
/// (`secrets::hold_release`). Whether it is listed (only the app's own
/// accounts, only while the preferences can be believed).
#[uniffi::export]
pub fn store_hold_release(
    preferences: Arc<dyn PreferencesHost>,
    mode: ReadinessMode,
    account: String,
) -> bool {
    secrets::hold_release(&HostDefaults(preferences), &readiness(mode), &account)
}

/// Drop `account` now if no saved record names it — `referenced`: every
/// account the saved connections and key records name, `None` when that
/// can't be told — keeping it listed until it is dropped
/// (`secrets::release`): what a connection's item it let go of gets once its
/// session no longer reads it.
#[uniffi::export]
pub fn store_release(
    preferences: Arc<dyn PreferencesHost>,
    mode: ReadinessMode,
    account: String,
    referenced: Option<Vec<String>>,
    keychain: Arc<dyn KeychainHost>,
) -> StoreSecretFate {
    let named = referenced.map(|refs| refs.into_iter().collect::<BTreeSet<_>>());
    secrets::release(
        &HostDefaults(preferences),
        &readiness(mode),
        &account,
        named.as_ref(),
        &HostKeychain(keychain),
    )
    .into()
}

/// One listed item, as a launch's settling left it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StoreSettledRelease {
    pub account: String,
    pub fate: StoreSecretFate,
}

/// The listed items, at a launch once the saved records can be told
/// (`secrets::settle_releases`): each one nothing names is deleted, one
/// named again is kept, one the keychain keeps stays listed; the ones this
/// process listed wait for their session. Unknown (`referenced` `None`)
/// changes nothing.
#[uniffi::export]
pub fn store_settle_releases(
    preferences: Arc<dyn PreferencesHost>,
    mode: ReadinessMode,
    referenced: Option<Vec<String>>,
    keychain: Arc<dyn KeychainHost>,
) -> Vec<StoreSettledRelease> {
    let named = referenced.map(|refs| refs.into_iter().collect::<BTreeSet<_>>());
    secrets::settle_releases(
        &HostDefaults(preferences),
        &readiness(mode),
        named.as_ref(),
        &HostKeychain(keychain),
    )
    .into_iter()
    .map(|(account, fate)| StoreSettledRelease {
        account,
        fate: fate.into(),
    })
    .collect()
}

/// The accounts waiting to be dropped, as stored; `None` while the
/// preferences can't be believed. Recovery never offers one of
/// these as a key.
#[uniffi::export]
pub fn store_to_release(
    preferences: Arc<dyn PreferencesHost>,
    mode: ReadinessMode,
) -> Option<Vec<String>> {
    secrets::to_release(&HostDefaults(preferences), &readiness(mode))
}

// MARK: - SSH keys

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum KeySource {
    SecureEnclave,
    Generated,
    Imported,
    Hardware,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum KeyAlgorithm {
    Ed25519,
    EcdsaSk,
    Rsa,
    SeP256,
    Ecdsa,
}

/// One key's metadata (`SSHKeyRecord`); the private half is only ever its
/// keychain account.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct KeyRecord {
    pub id: String,
    pub name: String,
    pub algorithm: KeyAlgorithm,
    pub bits: Option<i64>,
    pub comment: String,
    pub fingerprint: String,
    pub public_key: String,
    pub source: KeySource,
    pub bound_hosts: Vec<String>,
    pub keychain_ref: Option<String>,
    pub require_biometry: bool,
    pub created_at: f64,
    pub last_used_at: Option<f64>,
    /// `SE · P256`, `RSA-2048`, `ED25519`… (read only).
    pub badge_text: String,
    /// The kind of key it is when no connect path signs in with it
    /// (`"ECDSA brainpoolP256r1"`), read off its private key (B780).
    pub refused: Option<String>,
}

fn key_algorithm(a: ssh_keys::SshKeyAlgorithm) -> KeyAlgorithm {
    match a {
        ssh_keys::SshKeyAlgorithm::Ed25519 => KeyAlgorithm::Ed25519,
        ssh_keys::SshKeyAlgorithm::EcdsaSk => KeyAlgorithm::EcdsaSk,
        ssh_keys::SshKeyAlgorithm::Rsa4096 => KeyAlgorithm::Rsa,
        ssh_keys::SshKeyAlgorithm::SeP256 => KeyAlgorithm::SeP256,
        ssh_keys::SshKeyAlgorithm::Ecdsa => KeyAlgorithm::Ecdsa,
    }
}

impl From<&SshKeyRecord> for KeyRecord {
    fn from(k: &SshKeyRecord) -> Self {
        Self {
            id: k.id.to_string(),
            name: k.name.clone(),
            algorithm: key_algorithm(k.algorithm),
            bits: k.bits,
            comment: k.comment.clone(),
            fingerprint: k.fingerprint.clone(),
            public_key: k.public_key.clone(),
            source: match k.source {
                ssh_keys::SshKeySource::SecureEnclave => KeySource::SecureEnclave,
                ssh_keys::SshKeySource::Generated => KeySource::Generated,
                ssh_keys::SshKeySource::Imported => KeySource::Imported,
                ssh_keys::SshKeySource::Hardware => KeySource::Hardware,
            },
            bound_hosts: k.bound_hosts.clone(),
            keychain_ref: k.keychain_ref.clone(),
            require_biometry: k.require_biometry,
            created_at: k.created_at.since_reference(),
            last_used_at: k.last_used_at.map(Date::since_reference),
            badge_text: k.badge_text(),
            refused: k.refused.clone(),
        }
    }
}

/// What importing a key's private key again did (`ssh_keys::Refill`, B781).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StoreRefill {
    /// The record has the key again.
    Refilled,
    /// Another key than the record's: nothing changed.
    DifferentKey,
    /// No such key, or a list that can't be trusted: nothing changed.
    Unknown,
}

fn ssh_algorithm(a: KeyAlgorithm) -> ssh_keys::SshKeyAlgorithm {
    match a {
        KeyAlgorithm::Ed25519 => ssh_keys::SshKeyAlgorithm::Ed25519,
        KeyAlgorithm::EcdsaSk => ssh_keys::SshKeyAlgorithm::EcdsaSk,
        KeyAlgorithm::Rsa => ssh_keys::SshKeyAlgorithm::Rsa4096,
        KeyAlgorithm::SeP256 => ssh_keys::SshKeyAlgorithm::SeP256,
        KeyAlgorithm::Ecdsa => ssh_keys::SshKeyAlgorithm::Ecdsa,
    }
}

fn ssh_key_source(s: KeySource) -> ssh_keys::SshKeySource {
    match s {
        KeySource::SecureEnclave => ssh_keys::SshKeySource::SecureEnclave,
        KeySource::Generated => ssh_keys::SshKeySource::Generated,
        KeySource::Imported => ssh_keys::SshKeySource::Imported,
        KeySource::Hardware => ssh_keys::SshKeySource::Hardware,
    }
}

impl TryFrom<KeyRecord> for SshKeyRecord {
    type Error = StoreError;

    fn try_from(k: KeyRecord) -> Result<Self, StoreError> {
        let algorithm = ssh_algorithm(k.algorithm);
        let source = ssh_key_source(k.source);
        let mut record =
            SshKeyRecord::new(uuid(&k.id)?, &k.name, algorithm, source, Date(k.created_at));
        record.bits = k.bits;
        record.comment = k.comment;
        record.fingerprint = k.fingerprint;
        record.public_key = k.public_key;
        record.bound_hosts = k.bound_hosts;
        record.keychain_ref = k.keychain_ref;
        record.require_biometry = k.require_biometry;
        record.last_used_at = k.last_used_at.map(Date);
        record.refused = k.refused;
        Ok(record)
    }
}

/// The key records (`SSHKeyStore`).
#[derive(uniffi::Object)]
pub struct KeyStoreHandle(Mutex<SshKeyStore>);

#[uniffi::export]
impl KeyStoreHandle {
    #[uniffi::constructor]
    pub fn new(host: Arc<dyn PreferencesHost>, mode: ReadinessMode) -> Arc<Self> {
        Arc::new(Self(Mutex::new(SshKeyStore::new(
            defaults(host),
            readiness(mode),
        ))))
    }

    pub fn keys(&self) -> Vec<KeyRecord> {
        lock(&self.0).keys().iter().map(Into::into).collect()
    }

    pub fn is_authoritative(&self) -> bool {
        lock(&self.0).is_authoritative()
    }

    pub fn add(&self, record: KeyRecord) -> Result<(), StoreError> {
        lock(&self.0).add(record.try_into()?);
        Ok(())
    }

    /// The record alone; deleting a key is [`Self::delete_key`].
    pub fn remove(&self, id: String) -> Result<(), StoreError> {
        lock(&self.0).remove(uuid(&id)?);
        Ok(())
    }

    /// Key `id` signed in now (B601): its last use, written at most once a
    /// day (`SshKeyStore::mark_used`). Whether it was written.
    pub fn mark_used(&self, id: String) -> Result<bool, StoreError> {
        Ok(lock(&self.0).mark_used(uuid(&id)?, Date::now()))
    }

    /// Every keychain ref a key record holds, the carried ones' too; `None`
    /// when that can't be known (`SshKeyStore::keychain_refs`, B232).
    /// `excluding` leaves one record out.
    pub fn keychain_refs(
        &self,
        excluding: Option<String>,
    ) -> Result<Option<Vec<String>>, StoreError> {
        let excluding = excluding.as_deref().map(uuid).transpose()?;
        Ok(lock(&self.0)
            .keychain_refs(excluding)
            .map(|refs| refs.into_iter().collect()))
    }

    /// Key `id`'s private key imported again on a phone that didn't have it
    /// (B781): the key's facts kept on the same record — its name, comment,
    /// hosts and connections kept — with Face ID as `require_biometry`
    /// says, unless it is another key (`SshKeyStore::refill`). Its item is
    /// already saved under the record's keychain account.
    #[allow(clippy::too_many_arguments)]
    pub fn refill(
        &self,
        id: String,
        algorithm: KeyAlgorithm,
        bits: Option<i64>,
        public_key: String,
        fingerprint: String,
        require_biometry: bool,
    ) -> Result<StoreRefill, StoreError> {
        let facts = secrets::KeyFacts {
            algorithm: ssh_algorithm(algorithm),
            source: ssh_keys::SshKeySource::Imported,
            bits,
            public_key,
            fingerprint,
        };
        Ok(
            match lock(&self.0).refill(uuid(&id)?, &facts, require_biometry) {
                ssh_keys::Refill::Refilled => StoreRefill::Refilled,
                ssh_keys::Refill::DifferentKey => StoreRefill::DifferentKey,
                ssh_keys::Refill::Unknown => StoreRefill::Unknown,
            },
        )
    }

    /// Key `id`'s facts, read off `private_key` — its keychain item, as
    /// the first sign-in with it read it (B726): for a record 1.0.8 saved
    /// without them (`SshKeyStore::learn_facts`); for a key of a kind no
    /// connect path signs with, that kind (`SshKeyStore::mark_refused`,
    /// B780). Whether either was written; false when the record has them,
    /// or the text reads as no private key.
    pub fn learn_facts(&self, id: String, private_key: String) -> Result<bool, StoreError> {
        let id = uuid(&id)?;
        let mut keys = lock(&self.0);
        if !keys.keys().iter().any(|k| k.id == id && k.lacks_facts()) {
            return Ok(false);
        }
        Ok(match describe_private_key(private_key.as_bytes()) {
            Some(facts) => keys.learn_facts(id, &facts),
            // A kind no connect path signs with (B780).
            None => refused_kind(private_key.as_bytes())
                .is_some_and(|kind| keys.mark_refused(id, &kind)),
        })
    }

    pub fn reload_if_needed(&self) {
        lock(&self.0).reload_if_needed();
    }

    /// What deleting key `id` would do now: whether a saved connection
    /// still signs in with it (B232).
    pub fn check_delete(
        &self,
        id: String,
        connections: Arc<ConnectionStoreHandle>,
    ) -> Result<StoreKeyCheck, StoreError> {
        let id = uuid(&id)?;
        let mut connections = lock(&connections.0);
        let mut keys = lock(&self.0);
        Ok(match secrets::check_key(id, &mut keys, &mut connections) {
            None => StoreKeyCheck::NotFound,
            Some(secrets::KeyCheck::Free) => StoreKeyCheck::Free,
            Some(secrets::KeyCheck::InUse(names)) => StoreKeyCheck::InUse { connections: names },
            Some(secrets::KeyCheck::Unknown) => StoreKeyCheck::Unknown,
        })
    }

    /// Delete key `id`: its private key from `keychain`, then its record —
    /// unless a saved connection still signs in with it, its users can't be
    /// told, or the keychain keeps the item (B232).
    pub fn delete_key(
        &self,
        id: String,
        connections: Arc<ConnectionStoreHandle>,
        keychain: Arc<dyn KeychainHost>,
    ) -> Result<StoreKeyDeletion, StoreError> {
        let id = uuid(&id)?;
        let mut connections = lock(&connections.0);
        let mut keys = lock(&self.0);
        Ok(
            match secrets::delete_key(id, &mut keys, &mut connections, &HostKeychain(keychain)) {
                secrets::KeyDeletion::Deleted => StoreKeyDeletion::Deleted,
                secrets::KeyDeletion::InUse(names) => {
                    StoreKeyDeletion::InUse { connections: names }
                }
                secrets::KeyDeletion::Unknown => StoreKeyDeletion::Unknown,
                secrets::KeyDeletion::NotFound => StoreKeyDeletion::NotFound,
                secrets::KeyDeletion::Keychain(error) => StoreKeyDeletion::KeychainFailed {
                    message: error.to_string(),
                },
            },
        )
    }
}

/// What deleting a key would do now.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum StoreKeyCheck {
    /// Nothing uses it.
    Free,
    /// These saved connections still sign in with it.
    InUse {
        connections: Vec<String>,
    },
    /// Which connections use it can't be told now.
    Unknown,
    NotFound,
}

/// What deleting a key did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum StoreKeyDeletion {
    /// The record is gone, and its private key with it.
    Deleted,
    /// These saved connections still sign in with it: nothing was deleted.
    InUse {
        connections: Vec<String>,
    },
    /// Which connections use it can't be told now: nothing was deleted.
    Unknown,
    NotFound,
    /// The keychain kept the private key; the record stays.
    KeychainFailed {
        message: String,
    },
}

/// What looking for orphaned private keys did.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum StoreRecovery {
    /// These records were added (none, often); nothing was deleted.
    Recovered { keys: Vec<KeyRecord> },
    /// Which items are named can't be told yet: nothing was looked at.
    Unknown,
    /// The keychain couldn't list its items.
    KeychainFailed { message: String },
}

/// What a stored private key the core can't read itself is, as the
/// platform reads it (B810): the facts its record shows.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StoredKeyFacts {
    pub algorithm: KeyAlgorithm,
    pub source: KeySource,
    /// An RSA key's modulus size.
    pub bits: Option<i64>,
    /// The `authorized_keys` line.
    pub public_key: String,
    /// `SHA256:…`.
    pub fingerprint: String,
}

/// The platform's reader for the stored private keys only it can open
/// (iOS: a Secure Enclave key's handle, which only CryptoKit reads), asked
/// by [`store_recover_orphaned_keys`] about what the core can't read. It is
/// asked with both stores held, so it must not call back into them.
#[uniffi::export(with_foreign)]
pub trait StoredKeyReader: Send + Sync {
    /// `secret`'s facts if it is a private key this platform reads; `None`
    /// for anything else.
    fn describe(&self, secret: Vec<u8>) -> Option<StoredKeyFacts>;
}

/// Give the private keys earlier builds orphaned key records again:
/// `moshpit.secret.` items no saved connection and no key record names, not
/// made in this process (the one set [`store_new_keychain_ref`] keeps), not
/// listed to drop (B428), read without asking, that read as a private key
/// (`secrets::recover_orphans`, B232). The core reads an unencrypted key
/// file itself and asks `reader` only about the rest — a Secure Enclave
/// key's handle, the only copy of that key (B810: the app's Swift recovery
/// read those, and is gone). Nothing is deleted: an orphan may be the only
/// copy of a key whose record a lost list took with it. `name_template` is
/// the record's name with `%@` for the start of its fingerprint (the app's
/// "Recovered key %@", in its language). The app runs it once a launch,
/// the first time both stores can be trusted.
#[uniffi::export]
pub fn store_recover_orphaned_keys(
    connections: Arc<ConnectionStoreHandle>,
    keys: Arc<KeyStoreHandle>,
    keychain: Arc<dyn KeychainHost>,
    reader: Option<Arc<dyn StoredKeyReader>>,
    name_template: String,
    now: f64,
) -> StoreRecovery {
    let connections = lock(&connections.0);
    let mut keys = lock(&keys.0);
    let name = |facts: &secrets::KeyFacts| {
        name_template.replace("%@", &secrets::short_fingerprint(&facts.fingerprint))
    };
    let describe = |secret: &[u8]| {
        describe_private_key(secret).or_else(|| {
            let facts = reader.as_ref()?.describe(secret.to_vec())?;
            Some(secrets::KeyFacts {
                algorithm: ssh_algorithm(facts.algorithm),
                source: ssh_key_source(facts.source),
                public_key: facts.public_key,
                fingerprint: facts.fingerprint,
                bits: facts.bits,
            })
        })
    };
    match secrets::recover_orphans(
        &connections,
        &mut keys,
        &HostKeychain(keychain),
        &describe,
        &name,
        Date(now),
    ) {
        Ok(records) => StoreRecovery::Recovered {
            keys: records.iter().map(Into::into).collect(),
        },
        Err(secrets::RecoveryError::Unknown) => StoreRecovery::Unknown,
        Err(secrets::RecoveryError::Keychain(error)) => StoreRecovery::KeychainFailed {
            message: error.to_string(),
        },
    }
}

/// Put right what each key 1.0.8 imported without its public half shows
/// (B726): its algorithm, size, public key and fingerprint, off its own
/// private key, read without asking (`secrets::learn_key_facts`), or the
/// kind it is when 1.1.0 can't sign with it (B780). One
/// behind Face ID waits for its first sign-in
/// ([`KeyStoreHandle::learn_facts`]). Returns the records changed. The app
/// runs it once a launch, beside the orphan recovery.
#[uniffi::export]
pub fn store_learn_key_facts(
    keys: Arc<KeyStoreHandle>,
    keychain: Arc<dyn KeychainHost>,
) -> Vec<KeyRecord> {
    let mut keys = lock(&keys.0);
    secrets::learn_key_facts(
        &mut keys,
        &HostKeychain(keychain),
        &describe_private_key,
        &refused_kind,
    )
    .iter()
    .map(Into::into)
    .collect()
}

/// Whether the keychain item `account` moves with a backup to a new phone
/// (`known_hosts::moves_with_backups`, B783): the trusted host keys alone —
/// public, the trust anchors a restored phone should keep. The platform
/// gives those the class that moves; every other item stays on the device.
#[uniffi::export]
pub fn store_item_moves_with_backups(account: String) -> bool {
    moshpit_store::known_hosts::moves_with_backups(&account)
}

/// What a stored secret is, if it is an unencrypted private key of a kind
/// that signs in, in any form the app reads (the facts a key record shows;
/// `moshpit_ssh::keyfile::describe_stored`). A Secure Enclave key's handle
/// can't be read here (the platform's [`StoredKeyReader`] reads those); a
/// password or a key behind a passphrase is `None`.
fn describe_private_key(secret: &[u8]) -> Option<secrets::KeyFacts> {
    let key = moshpit_ssh::keyfile::describe_stored(std::str::from_utf8(secret).ok()?)?;
    Some(secrets::KeyFacts {
        algorithm: key.record_algorithm(),
        source: ssh_keys::SshKeySource::Imported,
        bits: key.bits,
        public_key: key.info.public_key,
        fingerprint: key.info.fingerprint,
    })
}

/// What this device's keychain doesn't hold of what the saved records name
/// (`secrets::MissingItems`, B781, B782): connections whose own password
/// or pasted key isn't here, key records whose private key isn't.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StoreMissingItems {
    pub connections: Vec<String>,
    pub keys: Vec<String>,
}

/// [`StoreMissingItems`] now (`secrets::missing_items`), asked without
/// reading an item — no prompt; one behind Face ID, or on a locked device,
/// is there. `None` while either store's read can't be trusted. A phone
/// restored from a backup has the records and none of their items, all of
/// which are `ThisDeviceOnly`.
#[uniffi::export]
pub fn store_missing_items(
    connections: Arc<ConnectionStoreHandle>,
    keys: Arc<KeyStoreHandle>,
    keychain: Arc<dyn KeychainHost>,
) -> Option<StoreMissingItems> {
    let connections = lock(&connections.0);
    let keys = lock(&keys.0);
    secrets::missing_items(&connections, &keys, &HostKeychain(keychain)).map(|missing| {
        StoreMissingItems {
            connections: missing.connections.iter().map(Uuid::to_string).collect(),
            keys: missing.keys.iter().map(Uuid::to_string).collect(),
        }
    })
}

/// The kind of key a stored secret is when it is one no connect path signs
/// in with (`moshpit_ssh::keyfile::refused_kind`, B780).
fn refused_kind(secret: &[u8]) -> Option<String> {
    moshpit_ssh::keyfile::refused_kind(std::str::from_utf8(secret).ok()?)
}

/// The modulus size of the `ssh-rsa` key on an `authorized_keys` line.
#[uniffi::export]
pub fn store_rsa_bits(public_key_line: String) -> Option<i64> {
    ssh_keys::rsa_bits_of_public_key_line(&public_key_line)
}

// MARK: - Shortcuts

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
// Named for the bindings so it can live beside the app's own `ShortcutKind` in
// one Swift module (the app compiles the generated file into itself).
#[uniffi(name = "StoreShortcutKind")]
pub enum ShortcutKind {
    KeyCombo,
    Text,
    Command,
    Dpad,
    Arrows,
    Paste,
    Scroll,
    Ctrl,
    Mic,
    Image,
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
// Named for the bindings so it can live beside the app's own `ShortcutModifier` in
// one Swift module (the app compiles the generated file into itself).
#[uniffi(name = "StoreShortcutModifier")]
pub enum ShortcutModifier {
    Ctrl,
    Alt,
    Shift,
    Cmd,
}

/// One chip (`TerminalShortcut`).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ShortcutRecord {
    pub id: String,
    pub kind: ShortcutKind,
    /// A set, ⌃⌥⇧⌘ order.
    pub modifiers: Vec<ShortcutModifier>,
    pub key: String,
    pub payload: String,
    pub append_return: bool,
    pub repeat_on_hold: bool,
    pub chip_label: String,
    pub summary: String,
    pub color_id: String,
    /// A set: empty is every host.
    pub scope_hosts: Vec<String>,
    pub only_in_tmux: bool,
    pub is_builtin: bool,
    pub in_toolbar: bool,
}

const KINDS: [(shortcuts::ShortcutKind, ShortcutKind); 11] = [
    (shortcuts::ShortcutKind::KeyCombo, ShortcutKind::KeyCombo),
    (shortcuts::ShortcutKind::Text, ShortcutKind::Text),
    (shortcuts::ShortcutKind::Command, ShortcutKind::Command),
    (shortcuts::ShortcutKind::Dpad, ShortcutKind::Dpad),
    (shortcuts::ShortcutKind::Arrows, ShortcutKind::Arrows),
    (shortcuts::ShortcutKind::Paste, ShortcutKind::Paste),
    (shortcuts::ShortcutKind::Scroll, ShortcutKind::Scroll),
    (shortcuts::ShortcutKind::Ctrl, ShortcutKind::Ctrl),
    (shortcuts::ShortcutKind::Mic, ShortcutKind::Mic),
    (shortcuts::ShortcutKind::Image, ShortcutKind::Image),
    (shortcuts::ShortcutKind::Files, ShortcutKind::Files),
];

const MODIFIERS: [(shortcuts::ShortcutModifier, ShortcutModifier); 4] = [
    (shortcuts::ShortcutModifier::Ctrl, ShortcutModifier::Ctrl),
    (shortcuts::ShortcutModifier::Alt, ShortcutModifier::Alt),
    (shortcuts::ShortcutModifier::Shift, ShortcutModifier::Shift),
    (shortcuts::ShortcutModifier::Cmd, ShortcutModifier::Cmd),
];

impl From<&TerminalShortcut> for ShortcutRecord {
    fn from(s: &TerminalShortcut) -> Self {
        Self {
            id: s.id.to_string(),
            kind: KINDS
                .iter()
                .find(|(k, _)| *k == s.kind)
                .expect("every kind")
                .1,
            modifiers: s
                .modifiers
                .iter()
                .map(|m| {
                    MODIFIERS
                        .iter()
                        .find(|(r, _)| r == m)
                        .expect("every modifier")
                        .1
                })
                .collect(),
            key: s.key.clone(),
            payload: s.payload.clone(),
            append_return: s.append_return,
            repeat_on_hold: s.repeat_on_hold,
            chip_label: s.chip_label.clone(),
            summary: s.summary.clone(),
            color_id: s.color_id.clone(),
            scope_hosts: s.scope_hosts.clone(),
            only_in_tmux: s.only_in_tmux,
            is_builtin: s.is_builtin,
            in_toolbar: s.in_toolbar,
        }
    }
}

impl TryFrom<ShortcutRecord> for TerminalShortcut {
    type Error = StoreError;

    fn try_from(s: ShortcutRecord) -> Result<Self, StoreError> {
        let mut shortcut = TerminalShortcut::new(uuid(&s.id)?);
        shortcut.kind = KINDS
            .iter()
            .find(|(_, k)| *k == s.kind)
            .expect("every kind")
            .0;
        shortcut.modifiers = s
            .modifiers
            .iter()
            .map(|m| {
                MODIFIERS
                    .iter()
                    .find(|(_, f)| f == m)
                    .expect("every modifier")
                    .0
            })
            .collect::<BTreeSet<_>>();
        shortcut.key = s.key;
        shortcut.payload = s.payload;
        shortcut.append_return = s.append_return;
        shortcut.repeat_on_hold = s.repeat_on_hold;
        shortcut.chip_label = s.chip_label;
        shortcut.summary = s.summary;
        shortcut.color_id = s.color_id;
        shortcut.set_scope_hosts(s.scope_hosts);
        shortcut.only_in_tmux = s.only_in_tmux;
        shortcut.is_builtin = s.is_builtin;
        shortcut.in_toolbar = s.in_toolbar;
        Ok(shortcut)
    }
}

/// The shortcut bar (`ShortcutStore`); order is toolbar order.
#[derive(uniffi::Object)]
pub struct ShortcutStoreHandle(Mutex<ShortcutStore>);

#[uniffi::export]
impl ShortcutStoreHandle {
    #[uniffi::constructor]
    pub fn new(host: Arc<dyn PreferencesHost>, mode: ReadinessMode) -> Arc<Self> {
        Arc::new(Self(Mutex::new(ShortcutStore::new(
            defaults(host),
            readiness(mode),
        ))))
    }

    pub fn shortcuts(&self) -> Vec<ShortcutRecord> {
        lock(&self.0).shortcuts().iter().map(Into::into).collect()
    }

    pub fn toolbar(&self) -> Vec<ShortcutRecord> {
        lock(&self.0)
            .toolbar()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// The toolbar for one connection (its name, if known).
    pub fn toolbar_for(
        &self,
        host_name: Option<String>,
        in_multiplexer: bool,
    ) -> Vec<ShortcutRecord> {
        lock(&self.0)
            .toolbar_for(host_name.as_deref(), in_multiplexer)
            .into_iter()
            .map(Into::into)
            .collect()
    }

    pub fn is_authoritative(&self) -> bool {
        lock(&self.0).is_authoritative()
    }

    /// Adds `shortcut`; whether it went on the toolbar (false: the bar was
    /// full, it is under AVAILABLE, and the editor says so, B252).
    pub fn add(&self, shortcut: ShortcutRecord) -> Result<bool, StoreError> {
        Ok(lock(&self.0).add(shortcut.try_into()?))
    }

    pub fn update(&self, shortcut: ShortcutRecord) -> Result<(), StoreError> {
        lock(&self.0).update(shortcut.try_into()?);
        Ok(())
    }

    pub fn remove(&self, id: String) -> Result<(), StoreError> {
        lock(&self.0).remove(uuid(&id)?);
        Ok(())
    }

    /// false when it can't go on: the bar is full (B233), or no such
    /// shortcut.
    pub fn add_to_toolbar(&self, id: String) -> Result<bool, StoreError> {
        Ok(lock(&self.0).add_to_toolbar(uuid(&id)?))
    }

    /// The bar holds its twelve: ＋ adds nothing until one comes off.
    pub fn toolbar_is_full(&self) -> bool {
        lock(&self.0).toolbar_is_full()
    }

    pub fn remove_from_toolbar(&self, id: String) -> Result<(), StoreError> {
        lock(&self.0).remove_from_toolbar(uuid(&id)?);
        Ok(())
    }

    /// Move toolbar positions `from` to before toolbar position `to`.
    pub fn move_in_toolbar(&self, from: Vec<u32>, to: u32) {
        let from: BTreeSet<usize> = from.into_iter().map(|i| i as usize).collect();
        lock(&self.0).move_in_toolbar(&from, to as usize);
    }

    pub fn restore_defaults(&self) {
        lock(&self.0).restore_defaults();
    }

    pub fn reload_if_needed(&self) {
        lock(&self.0).reload_if_needed();
    }
}

/// The writes a tap on `shortcut` makes, in order (none for the chips the
/// caller resolves).
#[uniffi::export]
pub fn store_shortcut_stages(shortcut: ShortcutRecord) -> Result<Vec<Vec<u8>>, StoreError> {
    Ok(TerminalShortcut::try_from(shortcut)?.encoded_stages())
}

// MARK: - Terminal themes

/// A terminal colour scheme (`TerminalTheme`), colours as bare hex.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ThemeRecord {
    pub id: String,
    pub name: String,
    pub background_hex: String,
    pub foreground_hex: String,
    pub cursor_hex: String,
    pub ansi_hex: Vec<String>,
    pub bright_hex: Option<Vec<String>>,
    pub is_built_in: bool,
}

impl From<&TerminalTheme> for ThemeRecord {
    fn from(t: &TerminalTheme) -> Self {
        Self {
            id: t.id.clone(),
            name: t.name.clone(),
            background_hex: t.background_hex.clone(),
            foreground_hex: t.foreground_hex.clone(),
            cursor_hex: t.cursor_hex.clone(),
            ansi_hex: t.ansi_hex.clone(),
            bright_hex: t.bright_hex.clone(),
            is_built_in: t.is_built_in,
        }
    }
}

impl From<ThemeRecord> for TerminalTheme {
    fn from(t: ThemeRecord) -> Self {
        Self {
            id: t.id,
            name: t.name,
            background_hex: t.background_hex,
            foreground_hex: t.foreground_hex,
            cursor_hex: t.cursor_hex,
            ansi_hex: t.ansi_hex,
            bright_hex: t.bright_hex,
            is_built_in: t.is_built_in,
        }
    }
}

/// Why an import failed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum ThemeImportError {
    NotJson,
    NoThemes,
}

impl std::fmt::Display for ThemeImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotJson => "not theme JSON",
            Self::NoThemes => "no themes in that JSON",
        })
    }
}

impl std::error::Error for ThemeImportError {}

/// The terminal themes (`ThemeStore`): built-ins and the user's own.
#[derive(uniffi::Object)]
pub struct ThemeStoreHandle(Mutex<ThemeStore>);

#[uniffi::export]
impl ThemeStoreHandle {
    #[uniffi::constructor]
    pub fn new(host: Arc<dyn PreferencesHost>, mode: ReadinessMode) -> Arc<Self> {
        Arc::new(Self(Mutex::new(ThemeStore::new(
            defaults(host),
            readiness(mode),
        ))))
    }

    /// The name a theme saved without one gets, localized.
    pub fn set_untitled_name(&self, untitled: String) {
        lock(&self.0).set_untitled_name(&untitled);
    }

    pub fn custom_themes(&self) -> Vec<ThemeRecord> {
        lock(&self.0)
            .custom_themes()
            .iter()
            .map(Into::into)
            .collect()
    }

    pub fn all_themes(&self) -> Vec<ThemeRecord> {
        lock(&self.0).all_themes().iter().map(Into::into).collect()
    }

    /// The theme with this id, or the default.
    pub fn theme(&self, id: String) -> ThemeRecord {
        (&lock(&self.0).theme(&id)).into()
    }

    pub fn is_custom(&self, id: String) -> bool {
        lock(&self.0).is_custom(&id)
    }

    pub fn is_authoritative(&self) -> bool {
        lock(&self.0).is_authoritative()
    }

    pub fn save(&self, theme: ThemeRecord) {
        lock(&self.0).save(theme.into());
    }

    pub fn delete(&self, id: String) {
        lock(&self.0).delete(&id);
    }

    pub fn remove_all_custom(&self) {
        lock(&self.0).remove_all_custom();
    }

    pub fn duplicate(&self, theme: ThemeRecord) -> ThemeRecord {
        (&lock(&self.0).duplicate(&theme.into())).into()
    }

    pub fn make_draft(&self) -> ThemeRecord {
        (&lock(&self.0).make_draft()).into()
    }

    pub fn import_themes(&self, json: String) -> Result<Vec<ThemeRecord>, ThemeImportError> {
        lock(&self.0)
            .import_themes(&json)
            .map(|themes| themes.iter().map(Into::into).collect())
            .map_err(|error| match error {
                themes::ImportError::NotJson => ThemeImportError::NotJson,
                themes::ImportError::NoThemes => ThemeImportError::NoThemes,
            })
    }

    pub fn reload_if_needed(&self) {
        lock(&self.0).reload_if_needed();
    }
}

/// One theme as pretty JSON with sorted keys (the export).
#[uniffi::export]
pub fn store_theme_export_json(theme: ThemeRecord) -> String {
    ThemeStore::export_json(&theme.into())
}

/// The sixteen colours a theme paints with: ANSI 0–7, then 8–15 (each
/// override, or derived).
#[uniffi::export]
pub fn store_theme_palette(theme: ThemeRecord) -> Vec<String> {
    let theme: TerminalTheme = theme.into();
    let base = AnsiSlot::ALL.iter().map(|&slot| theme.ansi(slot));
    let bright = AnsiSlot::ALL.iter().map(|&slot| theme.bright(slot));
    base.chain(bright).collect()
}

/// `Color.normalizedHex`.
#[uniffi::export]
pub fn store_normalized_hex(hex: String) -> String {
    themes::normalized_hex(&hex)
}

/// `Color.blend(_:toward:by:)`.
#[uniffi::export]
pub fn store_blend_hex(hex: String, toward: String, by: f64) -> String {
    themes::blend(&hex, &toward, by)
}

// MARK: - Accent themes and icons

/// An accent theme (`AppTheme`), colours as bare hex.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AccentThemeRecord {
    pub id: String,
    pub name: String,
    pub accent_hex: String,
    pub accent_pressed_hex: String,
    pub screen_bg_hex: String,
    pub terminal_bg_hex: String,
    pub is_built_in: bool,
}

impl From<&AppTheme> for AccentThemeRecord {
    fn from(t: &AppTheme) -> Self {
        Self {
            id: t.id.clone(),
            name: t.name.clone(),
            accent_hex: t.accent_hex.clone(),
            accent_pressed_hex: t.accent_pressed_hex.clone(),
            screen_bg_hex: t.screen_bg_hex.clone(),
            terminal_bg_hex: t.terminal_bg_hex.clone(),
            is_built_in: t.is_built_in,
        }
    }
}

/// A custom accent theme from one accent, the rest derived.
#[uniffi::export]
pub fn store_accent_theme(id: String, name: String, accent_hex: String) -> AccentThemeRecord {
    (&AppTheme::custom(&id, &name, &accent_hex)).into()
}

/// The user's accent themes (`AppThemeStore`).
#[derive(uniffi::Object)]
pub struct AccentThemeStoreHandle(Mutex<AppThemeStore>);

#[uniffi::export]
impl AccentThemeStoreHandle {
    #[uniffi::constructor]
    pub fn new(host: Arc<dyn PreferencesHost>, mode: ReadinessMode) -> Arc<Self> {
        Arc::new(Self(Mutex::new(AppThemeStore::new(
            defaults(host),
            readiness(mode),
        ))))
    }

    pub fn set_untitled_name(&self, untitled: String) {
        lock(&self.0).set_untitled_name(&untitled);
    }

    pub fn custom_themes(&self) -> Vec<AccentThemeRecord> {
        lock(&self.0)
            .custom_themes()
            .iter()
            .map(Into::into)
            .collect()
    }

    /// Built-ins, then the user's own.
    pub fn all(&self) -> Vec<AccentThemeRecord> {
        lock(&self.0).all().iter().map(Into::into).collect()
    }

    /// The theme with this id, or Signal Room.
    pub fn theme(&self, id: String) -> AccentThemeRecord {
        (&lock(&self.0).theme(&id)).into()
    }

    pub fn is_authoritative(&self) -> bool {
        lock(&self.0).is_authoritative()
    }

    /// Save a custom theme (its derived colours are rebuilt from the accent;
    /// a built-in is refused).
    pub fn save(&self, theme: AccentThemeRecord) {
        let mut rebuilt = AppTheme::custom(&theme.id, &theme.name, &theme.accent_hex);
        rebuilt.is_built_in = theme.is_built_in;
        lock(&self.0).save(&rebuilt);
    }

    pub fn delete(&self, id: String) {
        lock(&self.0).delete(&id);
    }

    pub fn remove_all_custom(&self) {
        lock(&self.0).remove_all_custom();
    }

    pub fn make_draft(&self, based_on: AccentThemeRecord) -> AccentThemeRecord {
        let base = AppTheme::custom(&based_on.id, &based_on.name, &based_on.accent_hex);
        (&lock(&self.0).make_draft(&base)).into()
    }

    pub fn reload_if_needed(&self) {
        lock(&self.0).reload_if_needed();
    }
}

/// One bundled app icon.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct IconOption {
    pub id: String,
    pub name: String,
    /// iOS's alternate icon name; `None` for the primary.
    pub alternate_name: Option<String>,
    pub preview_asset: String,
}

impl From<app_themes::AppIconOption> for IconOption {
    fn from(o: app_themes::AppIconOption) -> Self {
        Self {
            id: o.id.to_owned(),
            name: o.name.to_owned(),
            alternate_name: o.alternate_name.map(str::to_owned),
            preview_asset: o.preview_asset.to_owned(),
        }
    }
}

#[uniffi::export]
pub fn store_app_icons() -> Vec<IconOption> {
    app_themes::icons::ALL.into_iter().map(Into::into).collect()
}

/// The icon with this id, or the primary (B13).
#[uniffi::export]
pub fn store_app_icon(id: String) -> IconOption {
    app_themes::icons::option(&id).into()
}

// MARK: - Settings

/// A setting's value.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SettingValue {
    Bool { value: bool },
    Int { value: i64 },
    Double { value: f64 },
    String { value: String },
    StringSet { values: Vec<String> },
}

impl From<settings::SettingValue> for SettingValue {
    fn from(v: settings::SettingValue) -> Self {
        match v {
            settings::SettingValue::Bool(value) => Self::Bool { value },
            settings::SettingValue::Int(value) => Self::Int { value },
            settings::SettingValue::Double(value) => Self::Double { value },
            settings::SettingValue::String(value) => Self::String { value },
            settings::SettingValue::StringSet(values) => Self::StringSet { values },
        }
    }
}

impl From<SettingValue> for settings::SettingValue {
    fn from(v: SettingValue) -> Self {
        match v {
            SettingValue::Bool { value } => Self::Bool(value),
            SettingValue::Int { value } => Self::Int(value),
            SettingValue::Double { value } => Self::Double(value),
            SettingValue::String { value } => Self::String(value),
            SettingValue::StringSet { values } => Self::StringSet(values),
        }
    }
}

/// The two switches the notification service reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PushPrefsRecord {
    pub show_detail: bool,
    pub sound: bool,
}

/// The names of every setting (`AppSettings`' properties), in order.
#[uniffi::export]
pub fn store_setting_names() -> Vec<String> {
    settings::ALL.iter().map(|s| s.name.to_owned()).collect()
}

/// The settings (`AppSettings`), read live.
#[derive(uniffi::Object)]
pub struct SettingsHandle(Mutex<Settings>);

#[uniffi::export]
impl SettingsHandle {
    /// `mirror` is where the notification service reads its two switches
    /// (iOS: the App Group's preferences).
    #[uniffi::constructor]
    pub fn new(
        host: Arc<dyn PreferencesHost>,
        mirror: Option<Arc<dyn PreferencesHost>>,
        mode: ReadinessMode,
        debug_build: bool,
    ) -> Arc<Self> {
        let mut settings = Settings::new(defaults(host), mirror.map(defaults), readiness(mode));
        settings.set_debug_build(debug_build);
        Arc::new(Self(Mutex::new(settings)))
    }

    /// One setting as the app reads it (an enum-backed one checked: an
    /// unknown stored value reads as the default).
    pub fn get(&self, name: String) -> Result<SettingValue, StoreError> {
        let settings = lock(&self.0);
        let spec =
            settings::spec_named(&name).ok_or(StoreError::InvalidSetting { name: name.clone() })?;
        let value = match spec.name {
            "cursorShape" => {
                settings::SettingValue::String(settings.cursor_shape().raw_value().to_owned())
            }
            "predictMode" => {
                settings::SettingValue::String(settings.predict_mode().raw_value().to_owned())
            }
            "voiceEngine" => {
                settings::SettingValue::String(settings.voice_engine().raw_value().to_owned())
            }
            "pushRelayURL" => settings::SettingValue::String(settings.push_relay_url()),
            _ => settings.get(spec),
        };
        Ok(value.into())
    }

    /// Write one setting as its setter does.
    pub fn set(&self, name: String, value: SettingValue) -> Result<(), StoreError> {
        let settings = lock(&self.0);
        let spec =
            settings::spec_named(&name).ok_or(StoreError::InvalidSetting { name: name.clone() })?;
        if settings.set(spec, value.into()) {
            Ok(())
        } else {
            Err(StoreError::InvalidSetting { name })
        }
    }

    /// The setting stored under preferences key `key`, as `AppSettings`'
    /// getter reads it before its own checks: what is stored, if it is of
    /// the setting's type, else the default — an enum-backed one's raw
    /// string unchecked. `None` when no setting has that key.
    pub fn value(&self, key: String) -> Option<SettingValue> {
        let spec = settings::ALL.iter().find(|s| s.key == key)?;
        Some(lock(&self.0).get(spec).into())
    }

    /// Write the setting stored under `key` as its setter does (and the
    /// notification service's mirror, for its two switches). False when no
    /// setting has that key, or `value` is of another type than its own.
    pub fn set_value(&self, key: String, value: SettingValue) -> bool {
        let Some(spec) = settings::ALL.iter().find(|s| s.key == key) else {
            return false;
        };
        lock(&self.0).set(spec, value.into())
    }

    pub fn push_prefs(&self) -> PushPrefsRecord {
        let prefs = lock(&self.0).push_prefs();
        PushPrefsRecord {
            show_detail: prefs.show_detail,
            sound: prefs.sound,
        }
    }

    pub fn reload_if_needed(&self) {
        lock(&self.0).reload_if_needed();
    }
}

// MARK: - Push pairings

/// One paired host (`PushPairing`): dates as seconds since 2001, the
/// connection id as its upper-case UUID string.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PairingRecord {
    pub connection_id: String,
    pub host_label: String,
    pub secret_hex: String,
    pub send_token: String,
    pub relay_url: String,
    pub created_at: f64,
    pub apns_token: Option<String>,
    pub apns_env: Option<String>,
    pub send_token_issued_at: Option<f64>,
}

impl From<&PushPairing> for PairingRecord {
    fn from(p: &PushPairing) -> Self {
        Self {
            connection_id: p.connection_id.to_string(),
            host_label: p.host_label.clone(),
            secret_hex: p.secret_hex.clone(),
            send_token: p.send_token.clone(),
            relay_url: p.relay_url.clone(),
            created_at: p.created_at.since_reference(),
            apns_token: p.apns_token.clone(),
            apns_env: p.apns_env.clone(),
            send_token_issued_at: p.send_token_issued_at.map(Date::since_reference),
        }
    }
}

impl TryFrom<PairingRecord> for PushPairing {
    type Error = StoreError;

    fn try_from(p: PairingRecord) -> Result<Self, StoreError> {
        Ok(Self {
            connection_id: uuid(&p.connection_id)?,
            host_label: p.host_label,
            secret_hex: p.secret_hex,
            send_token: p.send_token,
            relay_url: p.relay_url,
            created_at: Date(p.created_at),
            apns_token: p.apns_token,
            apns_env: p.apns_env,
            send_token_issued_at: p.send_token_issued_at.map(Date),
        })
    }
}

/// The pairings file (`PushPairingFile`, round five's B23 rule: read an
/// entry at a time, carry what can't be read, copy a file that isn't fully
/// readable aside before writing over it, never write over one that can't
/// be read) at a path of the host's choosing — on iOS the App Group's
/// `push-pairings.json`, which the notification service reads. The file's
/// protection class and backup exclusion are the host's to set.
#[derive(uniffi::Object)]
pub struct PushPairingFileHandle(PushPairingFile);

#[uniffi::export]
impl PushPairingFileHandle {
    #[uniffi::constructor]
    pub fn new(path: String) -> Arc<Self> {
        Arc::new(Self(PushPairingFile::new(path)))
    }

    /// Where a copy of a file this build couldn't fully read is kept.
    pub fn backup_path(&self) -> String {
        self.0.backup_path().to_string_lossy().into_owned()
    }

    /// The pairings this build can read, in stored order.
    pub fn read(&self) -> Vec<PairingRecord> {
        self.0.read().iter().map(Into::into).collect()
    }

    /// Replace every pairing; true once it reads back.
    pub fn write(&self, pairings: Vec<PairingRecord>) -> Result<bool, StoreError> {
        let pairings = pairings
            .into_iter()
            .map(PushPairing::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self.0.write(&pairings))
    }

    /// Replace (or add) one connection's pairing; true once it reads back.
    pub fn upsert(&self, pairing: PairingRecord) -> Result<bool, StoreError> {
        Ok(self.0.upsert(&pairing.try_into()?))
    }

    /// Forget one connection's pairing; true once nothing for it is left.
    pub fn remove(&self, connection_id: String) -> Result<bool, StoreError> {
        Ok(self.0.remove(uuid(&connection_id)?))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct Prefs(Mutex<HashMap<String, PlistValue>>);

    impl PreferencesHost for Prefs {
        fn get(&self, key: String) -> Option<PlistValue> {
            lock(&self.0).get(&key).cloned()
        }
        fn set(&self, key: String, value: Option<PlistValue>) {
            match value {
                Some(value) => lock(&self.0).insert(key, value),
                None => lock(&self.0).remove(&key),
            };
        }
        fn protected_data_available(&self) -> bool {
            true
        }
    }

    /// E7c: what the core's connect saved about a host (`hostcaps.<id>`)
    /// reads back through the app's own preferences, the package manager
    /// by its raw id; nothing before a connect has probed it.
    #[test]
    fn a_hosts_saved_capabilities_read_back_through_the_host() {
        let host: Arc<dyn PreferencesHost> = Arc::new(Prefs::default());
        let id = "6F9619FF-8B86-D011-B42D-00C04FC964FF";
        assert_eq!(
            crate::hostcare::store_host_capabilities(host.clone(), id.into()),
            None
        );
        let caps =
            moshpit_install::HostCapabilities::parse("/usr/bin/tmux\n::Linux::/usr/bin/apt-get\n");
        assert!(moshpit_store::session_memory::write(
            &HostDefaults(host.clone()),
            &Readiness::Always,
            &moshpit_store::session_memory::host_capabilities_key(id),
            &caps,
        ));
        let read = crate::hostcare::store_host_capabilities(host, id.into()).expect("saved");
        assert!(read.has_tmux && !read.has_mosh_server);
        assert_eq!(read.os, "Linux");
        assert_eq!(read.package_manager.as_deref(), Some("aptGet"));
    }

    #[test]
    fn a_nested_value_crosses_as_nodes_and_back() {
        let value = Plist::Array(vec![
            Plist::Data(vec![1, 2]),
            Plist::Dictionary(vec![("a".into(), Plist::Array(vec![Plist::Bool(true)]))]),
            Plist::Double(0.5),
        ]);
        let nodes = to_value(&value);
        assert_eq!(nodes.nodes.len(), 6);
        assert_eq!(from_value(&nodes), Some(value));
        // A member that points back up is refused, not followed.
        let cycle = PlistValue {
            nodes: vec![PlistNode::Array { members: vec![0] }],
        };
        assert_eq!(from_value(&cycle), None);
    }

    /// The connection a stored one goes through crosses the record both
    /// ways (F1): a save from Swift keeps it, and clears it when cleared.
    #[test]
    fn f1_a_record_carries_the_way_through() {
        let host: Arc<dyn PreferencesHost> = Arc::new(Prefs::default());
        let bastion = Uuid::new_v4();
        let mut hpc = ServerConnection::new(Uuid::new_v4(), Date(784_000_000.5));
        hpc.via_connection_id = Some(bastion);
        {
            let mut direct =
                ConnectionStore::new(Arc::new(HostDefaults(host.clone())), Readiness::Always);
            direct.add(hpc.clone());
        }
        let store = ConnectionStoreHandle::new(host.clone(), ReadinessMode::Always);
        let mut record = store.connections()[0].clone();
        record.name = "renamed".into();
        store.update(record).unwrap();
        let reread = ConnectionStore::new(Arc::new(HostDefaults(host.clone())), Readiness::Always);
        assert_eq!(reread.connections()[0].name, "renamed");
        assert_eq!(reread.connections()[0].via_connection_id, Some(bastion));
        let mut record = store.connections()[0].clone();
        assert_eq!(record.via_connection_id, Some(bastion.to_string()));
        record.via_connection_id = None;
        store.update(record).unwrap();
        let reread = ConnectionStore::new(Arc::new(HostDefaults(host)), Readiness::Always);
        assert_eq!(reread.connections()[0].via_connection_id, None);
    }

    #[test]
    fn the_stores_work_through_the_host_traits() {
        let host: Arc<dyn PreferencesHost> = Arc::new(Prefs::default());
        let store = ConnectionStoreHandle::new(host.clone(), ReadinessMode::System);
        let mut record: ConnectionRecord =
            (&ServerConnection::new(Uuid::new_v4(), Date(784_000_000.5))).into();
        record.name = "box".into();
        store.add(record.clone()).unwrap();
        let reread = ConnectionStoreHandle::new(host.clone(), ReadinessMode::System);
        assert_eq!(reread.connections()[0].name, "box");
        assert_eq!(reread.connections()[0].created_at, 784_000_000.0);
        assert!(store.delete("nope".into()).is_err());
        assert!(store_defaults_readable(host.clone(), ReadinessMode::System));

        let shortcuts = ShortcutStoreHandle::new(host.clone(), ReadinessMode::Always);
        assert_eq!(shortcuts.toolbar().len(), 8);
        let two_step = shortcuts
            .shortcuts()
            .into_iter()
            .find(|s| s.chip_label == "⇥⏎")
            .unwrap();
        assert_eq!(
            store_shortcut_stages(two_step).unwrap(),
            [vec![0x09], vec![0x0D]]
        );

        let settings = SettingsHandle::new(host.clone(), None, ReadinessMode::Always, false);
        assert_eq!(
            settings.get("fontSize".into()).unwrap(),
            SettingValue::Double { value: 9.0 }
        );
        settings
            .set("fontSize".into(), SettingValue::Double { value: 12.0 })
            .unwrap();
        assert_eq!(
            settings.get("fontSize".into()).unwrap(),
            SettingValue::Double { value: 12.0 }
        );
        assert!(
            settings
                .set("fontSize".into(), SettingValue::Int { value: 1 })
                .is_err()
        );
        assert_eq!(store_setting_names().len(), 32);
        assert_eq!(store_app_icon("nope".into()).id, "default");
        assert_eq!(
            store_theme_palette((&TerminalTheme::nord()).into()).len(),
            16
        );
    }

    /// What the app's adapters ask besides the stores' own lists: the refs
    /// and users B232's services count, a setting by its preferences key,
    /// and the pairings file.
    #[test]
    fn refs_settings_by_key_and_the_pairings_file() {
        let host: Arc<dyn PreferencesHost> = Arc::new(Prefs::default());
        let connections = ConnectionStoreHandle::new(host.clone(), ReadinessMode::Always);
        let keys = KeyStoreHandle::new(host.clone(), ReadinessMode::Always);
        let key = SshKeyRecord::new(
            Uuid::new_v4(),
            "laptop",
            ssh_keys::SshKeyAlgorithm::Ed25519,
            ssh_keys::SshKeySource::Generated,
            Date(784_000_000.0),
        );
        let mut record: KeyRecord = (&key).into();
        record.keychain_ref = Some("moshpit.secret.k".into());
        keys.add(record).unwrap();
        let mut pi = ServerConnection::new(Uuid::new_v4(), Date(784_000_000.0));
        pi.username = "me".into();
        pi.host = "pi".into();
        pi.keychain_ref = Some("moshpit.secret.c".into());
        pi.ssh_key_id = Some(key.id);
        connections.add((&pi).into()).unwrap();
        assert_eq!(
            connections.keychain_refs(None).unwrap(),
            Some(vec!["moshpit.secret.c".to_owned()])
        );
        assert_eq!(
            connections.keychain_refs(Some(pi.id.to_string())).unwrap(),
            Some(vec![])
        );
        assert_eq!(
            connections
                .names_using_key(key.id.to_string(), None)
                .unwrap(),
            Some(vec!["me@pi".to_owned()])
        );
        assert!(connections.names_using_key("nope".into(), None).is_err());
        assert_eq!(
            keys.keychain_refs(None).unwrap(),
            Some(vec!["moshpit.secret.k".to_owned()])
        );
        assert_eq!(
            keys.keychain_refs(Some(key.id.to_string())).unwrap(),
            Some(vec![])
        );
        // Sealed: nothing can be counted.
        let sealed = ConnectionStoreHandle::new(
            Arc::new(SealedPrefs(Arc::new(Sealed {
                unlocked: std::sync::atomic::AtomicBool::new(false),
            }))),
            ReadinessMode::System,
        );
        assert_eq!(sealed.keychain_refs(None).unwrap(), None);

        let settings = SettingsHandle::new(host.clone(), None, ReadinessMode::Always, false);
        assert_eq!(
            settings.value("moshpit.settings.moshByDefault".into()),
            Some(SettingValue::Bool { value: true })
        );
        assert_eq!(
            settings.value("moshByDefault".into()),
            None,
            "a key, not a name"
        );
        assert!(settings.set_value(
            "moshpit.settings.moshByDefault".into(),
            SettingValue::Bool { value: false }
        ));
        assert_eq!(
            settings.get("moshByDefault".into()).unwrap(),
            SettingValue::Bool { value: false }
        );
        assert!(!settings.set_value(
            "moshpit.settings.moshByDefault".into(),
            SettingValue::Int { value: 0 }
        ));
        assert!(!settings.set_value("nope".into(), SettingValue::Bool { value: true }));
        // An enum-backed setting reads raw: the app's getter checks it.
        settings
            .set(
                "cursorShape".into(),
                SettingValue::String { value: "x".into() },
            )
            .unwrap();
        assert_eq!(
            settings.value("moshpit.settings.cursorShape".into()),
            Some(SettingValue::String { value: "x".into() })
        );

        let dir = std::env::temp_dir().join(format!("w5b-pairings-{}", Uuid::new_v4()));
        let path = dir.join("push-pairings.json");
        let file = PushPairingFileHandle::new(path.to_string_lossy().into_owned());
        assert!(file.read().is_empty());
        let pairing = PairingRecord {
            connection_id: pi.id.to_string(),
            host_label: "pi".into(),
            secret_hex: "ab".repeat(32),
            send_token: String::new(),
            relay_url: "https://relay.example.com".into(),
            created_at: 784_000_000.25,
            apns_token: None,
            apns_env: None,
            send_token_issued_at: None,
        };
        assert!(file.upsert(pairing.clone()).unwrap());
        assert_eq!(file.read(), std::slice::from_ref(&pairing));
        let mut minted = pairing.clone();
        minted.send_token = "t".into();
        minted.apns_token = Some("aa".into());
        minted.send_token_issued_at = Some(784_000_100.0);
        assert!(file.write(vec![minted.clone()]).unwrap());
        assert_eq!(file.read(), [minted]);
        assert!(
            file.remove(Uuid::new_v4().to_string()).unwrap(),
            "nothing to forget"
        );
        assert!(file.remove(pi.id.to_string()).unwrap());
        assert!(file.read().is_empty());
        assert!(file.remove("nope".into()).is_err());
        assert!(file.backup_path().ends_with("push-pairings.json.backup"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The host's keychain, in memory.
    #[derive(Default)]
    struct Vault(Mutex<HashMap<String, Vec<u8>>>);

    impl KeychainHost for Vault {
        fn save(&self, account: String, secret: Vec<u8>, _: bool) -> Result<(), KeychainFailure> {
            lock(&self.0).insert(account, secret);
            Ok(())
        }
        fn load(&self, account: String, _: String) -> Result<Vec<u8>, KeychainFailure> {
            lock(&self.0)
                .get(&account)
                .cloned()
                .ok_or(KeychainFailure::NotFound)
        }
        fn delete(&self, account: String) -> Result<(), KeychainFailure> {
            lock(&self.0).remove(&account);
            Ok(())
        }
        fn exists(&self, account: String) -> bool {
            lock(&self.0).contains_key(&account)
        }
        fn accounts(&self) -> Result<Vec<String>, KeychainFailure> {
            Ok(lock(&self.0).keys().cloned().collect())
        }
        fn load_silently(&self, account: String) -> Result<Vec<u8>, KeychainFailure> {
            self.load(account, String::new())
        }
    }

    /// An unencrypted ed25519 OpenSSH private key (crates/ssh's fixture).
    const ED25519: &str = include_str!("../../ssh/tests/fixtures/keys/ed25519");

    /// A device before its first unlock since boot: protected data is
    /// unavailable until `unlocked`.
    struct Sealed {
        unlocked: std::sync::atomic::AtomicBool,
    }

    impl Sealed {
        fn sealed(&self) -> bool {
            !self.unlocked.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    struct SealedPrefs(Arc<Sealed>);

    impl PreferencesHost for SealedPrefs {
        fn get(&self, _: String) -> Option<PlistValue> {
            None
        }
        fn set(&self, _: String, _: Option<PlistValue>) {}
        fn protected_data_available(&self) -> bool {
            !self.0.sealed()
        }
    }

    /// B232 through the handles: a key a connection uses stays and says
    /// who; deleted, its private key goes too; a deleted connection takes
    /// an item nothing else names; an orphaned private key is listed again
    /// — once, and nothing is deleted.
    #[test]
    fn deleting_a_key_takes_its_private_key_and_orphaned_keys_are_recovered() {
        let host: Arc<dyn PreferencesHost> = Arc::new(Prefs::default());
        let vault = Arc::new(Vault::default());
        let keychain: Arc<dyn KeychainHost> = vault.clone();
        let connections = ConnectionStoreHandle::new(host.clone(), ReadinessMode::Always);
        let keys = KeyStoreHandle::new(host.clone(), ReadinessMode::Always);
        let private = "moshpit.secret.0f0e0d0c-0000-4000-8000-000000000001".to_owned();
        let password = "moshpit.secret.0f0e0d0c-0000-4000-8000-000000000002".to_owned();
        let orphan = "moshpit.secret.0f0e0d0c-0000-4000-8000-000000000003".to_owned();
        for account in [&private, &password] {
            keychain
                .save(account.clone(), b"secret".to_vec(), false)
                .unwrap();
        }
        keychain
            .save(orphan.clone(), ED25519.as_bytes().to_vec(), false)
            .unwrap();
        keychain
            .save("moshpit.knownHosts".into(), b"{}".to_vec(), false)
            .unwrap();
        let mut key = SshKeyRecord::new(
            Uuid::new_v4(),
            "laptop",
            ssh_keys::SshKeyAlgorithm::Ed25519,
            ssh_keys::SshKeySource::Generated,
            Date(784_000_000.0),
        );
        key.keychain_ref = Some(private.clone());
        keys.add((&key).into()).unwrap();
        let mut pi = ServerConnection::new(Uuid::new_v4(), Date(784_000_000.0));
        pi.name = "pi".into();
        pi.keychain_ref = Some(private.clone());
        pi.ssh_key_id = Some(key.id);
        connections.add((&pi).into()).unwrap();
        let mut nas = ServerConnection::new(Uuid::new_v4(), Date(784_000_000.0));
        nas.name = "nas".into();
        nas.keychain_ref = Some(password.clone());
        connections.add((&nas).into()).unwrap();

        let id = key.id.to_string();
        assert_eq!(
            keys.check_delete(id.clone(), connections.clone()).unwrap(),
            StoreKeyCheck::InUse {
                connections: vec!["pi".into()]
            }
        );
        assert_eq!(
            keys.delete_key(id.clone(), connections.clone(), keychain.clone())
                .unwrap(),
            StoreKeyDeletion::InUse {
                connections: vec!["pi".into()]
            }
        );
        assert!(keychain.exists(private.clone()));
        let recover = || {
            store_recover_orphaned_keys(
                connections.clone(),
                keys.clone(),
                keychain.clone(),
                None,
                "Recovered key %@".into(),
                784_000_100.0,
            )
        };
        let StoreRecovery::Recovered { keys: recovered } = recover() else {
            panic!("nothing looked at")
        };
        assert_eq!(recovered.len(), 1, "{recovered:?}");
        let found = &recovered[0];
        assert_eq!(found.keychain_ref.as_deref(), Some(orphan.as_str()));
        assert_eq!(found.algorithm, KeyAlgorithm::Ed25519);
        assert!(found.fingerprint.starts_with("SHA256:"));
        assert!(found.public_key.starts_with("ssh-ed25519 "));
        assert_eq!(
            found.name,
            format!(
                "Recovered key {}",
                &found.fingerprint["SHA256:".len().."SHA256:".len() + 8]
            )
        );
        assert!(keychain.exists(orphan.clone()), "recovery deleted it");
        assert!(keychain.exists(password.clone()), "a password isn't a key");
        assert_eq!(
            recover(),
            StoreRecovery::Recovered { keys: vec![] },
            "a second look added it again"
        );
        let found_id = found.id.clone();
        assert_eq!(
            keys.delete_key(found_id, connections.clone(), keychain.clone())
                .unwrap(),
            StoreKeyDeletion::Deleted
        );
        assert!(!keychain.exists(orphan.clone()));
        assert_eq!(
            connections
                .delete_releasing_secret(pi.id.to_string(), keys.clone(), keychain.clone())
                .unwrap(),
            StoreSecretFate::Kept,
            "the key still names its item"
        );
        assert_eq!(
            keys.delete_key(id.clone(), connections.clone(), keychain.clone())
                .unwrap(),
            StoreKeyDeletion::Deleted
        );
        assert!(!keychain.exists(private));
        assert!(keys.keys().is_empty());
        assert_eq!(
            connections
                .delete_releasing_secret(nas.id.to_string(), keys.clone(), keychain.clone())
                .unwrap(),
            StoreSecretFate::Deleted
        );
        let mut left = keychain.accounts().unwrap();
        left.sort();
        assert_eq!(left, ["moshpit.knownHosts"]);
    }

    /// A Secure Enclave key's handle as the keychain holds it: bytes only
    /// the platform (CryptoKit) opens.
    const ENCLAVE_HANDLE: &[u8] = b"\x04 a Secure Enclave handle";

    /// The platform's reader (iOS: CryptoKit over a Secure Enclave
    /// handle): knows `ENCLAVE_HANDLE`, and keeps what it was asked.
    #[derive(Default)]
    struct Enclave(Mutex<Vec<Vec<u8>>>);

    impl StoredKeyReader for Enclave {
        fn describe(&self, secret: Vec<u8>) -> Option<StoredKeyFacts> {
            lock(&self.0).push(secret.clone());
            (secret == ENCLAVE_HANDLE).then(|| StoredKeyFacts {
                algorithm: KeyAlgorithm::SeP256,
                source: KeySource::SecureEnclave,
                bits: None,
                public_key: "ecdsa-sha2-nistp256 AAAAenclave".into(),
                fingerprint: "SHA256:enclave0123456789".into(),
            })
        }
    }

    /// Two stores over one fresh set of preferences, and an empty keychain.
    fn ref_stores() -> (Arc<ConnectionStoreHandle>, Arc<KeyStoreHandle>, Arc<Vault>) {
        let host: Arc<dyn PreferencesHost> = Arc::new(Prefs::default());
        (
            ConnectionStoreHandle::new(host.clone(), ReadinessMode::Always),
            KeyStoreHandle::new(host, ReadinessMode::Always),
            Arc::new(Vault::default()),
        )
    }

    /// A ref as the app's Swift `KeychainService.generateRef()` made them
    /// before B810, in an earlier launch: the same `moshpit.secret.` and a
    /// lower-case UUID, never in this process's set.
    fn swift_minted_ref() -> String {
        format!(
            "moshpit.secret.{}",
            Uuid::new_v4().to_string().to_lowercase()
        )
    }

    fn recovered(
        connections: &Arc<ConnectionStoreHandle>,
        keys: &Arc<KeyStoreHandle>,
        vault: &Arc<Vault>,
        reader: Option<Arc<dyn StoredKeyReader>>,
    ) -> Vec<KeyRecord> {
        match store_recover_orphaned_keys(
            connections.clone(),
            keys.clone(),
            vault.clone(),
            reader,
            "Recovered key %@".into(),
            784_000_100.0,
        ) {
            StoreRecovery::Recovered { keys } => keys,
            other => panic!("nothing looked at: {other:?}"),
        }
    }

    /// B810: orphan recovery is the core's alone (the app's Swift copy is
    /// gone), and that copy also read a Secure Enclave key's handle, which
    /// the core can't: for a Secure Enclave key the item is the only copy
    /// there is. The platform is asked about what the core can't read, and
    /// only that: the orphaned Secure Enclave key is listed again as one,
    /// the OpenSSH key is read by the core without asking, a password is
    /// asked about and left where it is.
    #[test]
    fn recovery_asks_the_platform_only_about_what_the_core_cant_read() {
        let (connections, keys, vault) = ref_stores();
        let enclave = swift_minted_ref();
        let openssh = swift_minted_ref();
        let password = swift_minted_ref();
        vault
            .save(enclave.clone(), ENCLAVE_HANDLE.to_vec(), false)
            .unwrap();
        vault
            .save(openssh.clone(), ED25519.as_bytes().to_vec(), false)
            .unwrap();
        vault
            .save(password.clone(), b"hunter2".to_vec(), false)
            .unwrap();
        let reader = Arc::new(Enclave::default());

        let found = recovered(&connections, &keys, &vault, Some(reader.clone()));

        let by_ref = |account: &str| {
            found
                .iter()
                .find(|k| k.keychain_ref.as_deref() == Some(account))
        };
        let se = by_ref(&enclave).expect("the Secure Enclave key wasn't recovered");
        assert_eq!(se.algorithm, KeyAlgorithm::SeP256);
        assert_eq!(se.source, KeySource::SecureEnclave);
        assert_eq!(se.fingerprint, "SHA256:enclave0123456789");
        assert_eq!(se.name, "Recovered key enclave0");
        assert!(!se.require_biometry);
        let imported = by_ref(&openssh).expect("the OpenSSH key wasn't recovered");
        assert_eq!(imported.algorithm, KeyAlgorithm::Ed25519);
        assert_eq!(imported.source, KeySource::Imported);
        assert!(by_ref(&password).is_none(), "a password became a key");
        assert_eq!(found.len(), 2);
        let asked = lock(&reader.0).clone();
        assert!(
            !asked.contains(&ED25519.as_bytes().to_vec()),
            "the platform was asked about a key the core reads"
        );
        assert!(asked.contains(&ENCLAVE_HANDLE.to_vec()));
        for account in [&enclave, &openssh, &password] {
            assert!(vault.exists(account.clone()), "recovery deleted an item");
        }
    }

    /// B810: one set of the refs made in this process. A ref the app made
    /// this launch — through `store_new_keychain_ref`, as the Swift path
    /// does now — whose record isn't saved yet is never taken for an
    /// orphan; one an earlier launch made on the Swift path, the same
    /// `moshpit.secret.<lower-case UUID>`, is recovered by the core. No
    /// item is touched either way, and nothing asks for Face ID (`Vault`'s
    /// silent read).
    #[test]
    fn a_ref_made_this_launch_is_never_an_orphan_and_one_from_an_earlier_launch_is() {
        let (connections, keys, vault) = ref_stores();
        let on_its_way = store_new_keychain_ref();
        assert!(on_its_way.starts_with("moshpit.secret."));
        assert_eq!(on_its_way, on_its_way.to_lowercase());
        let from_before = swift_minted_ref();
        vault
            .save(on_its_way.clone(), ED25519.as_bytes().to_vec(), false)
            .unwrap();
        vault
            .save(from_before.clone(), ED25519.as_bytes().to_vec(), false)
            .unwrap();
        let before = lock(&vault.0).clone();

        let found = recovered(&connections, &keys, &vault, None);

        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].keychain_ref.as_deref(), Some(from_before.as_str()));
        assert_eq!(*lock(&vault.0), before, "recovery changed the keychain");
    }

    /// B810: deleting takes the item whichever path minted its ref, and a
    /// key the user deleted is never listed again by a later recovery.
    #[test]
    fn deleting_takes_the_item_whoever_minted_it_and_recovery_never_brings_it_back() {
        let (connections, keys, vault) = ref_stores();
        let keychain: Arc<dyn KeychainHost> = vault.clone();
        for minted in [swift_minted_ref(), store_new_keychain_ref()] {
            // A connection's own password.
            vault.save(minted.clone(), b"pw".to_vec(), false).unwrap();
            let mut pi = ServerConnection::new(Uuid::new_v4(), Date(784_000_000.0));
            pi.name = "pi".into();
            pi.keychain_ref = Some(minted.clone());
            connections.add((&pi).into()).unwrap();
            assert_eq!(
                connections
                    .delete_releasing_secret(pi.id.to_string(), keys.clone(), keychain.clone())
                    .unwrap(),
                StoreSecretFate::Deleted
            );
            assert!(
                !vault.exists(minted.clone()),
                "{minted}: the password stayed"
            );
        }
        for minted in [swift_minted_ref(), store_new_keychain_ref()] {
            // A key's private half.
            vault
                .save(minted.clone(), ED25519.as_bytes().to_vec(), false)
                .unwrap();
            let mut key = SshKeyRecord::new(
                Uuid::new_v4(),
                "laptop",
                ssh_keys::SshKeyAlgorithm::Ed25519,
                ssh_keys::SshKeySource::Imported,
                Date(784_000_000.0),
            );
            key.keychain_ref = Some(minted.clone());
            keys.add((&key).into()).unwrap();
            assert_eq!(
                keys.delete_key(key.id.to_string(), connections.clone(), keychain.clone())
                    .unwrap(),
                StoreKeyDeletion::Deleted
            );
            assert!(
                !vault.exists(minted.clone()),
                "{minted}: the private key stayed"
            );
        }
        assert!(recovered(&connections, &keys, &vault, None).is_empty());
        assert!(keys.keys().is_empty(), "a deleted key came back");
    }

    /// B810: what a connection let go of is dropped by the core's rule over
    /// both stores (`secrets::release_item`, `secrets::release_pending`):
    /// the app no longer works out in Swift which items the records name.
    /// Named again, it stays and comes off the list; named by nothing, it
    /// goes; listed by an earlier launch, the next launch drops it.
    #[test]
    fn what_a_connection_let_go_of_is_dropped_by_the_cores_rule_over_both_stores() {
        let (connections, keys, vault) = ref_stores();
        let keychain: Arc<dyn KeychainHost> = vault.clone();
        let mine = swift_minted_ref();
        let named = swift_minted_ref();
        let listed = swift_minted_ref();
        for account in [&mine, &named, &listed] {
            vault.save(account.clone(), b"pw".to_vec(), false).unwrap();
        }
        let mut key = SshKeyRecord::new(
            Uuid::new_v4(),
            "laptop",
            ssh_keys::SshKeyAlgorithm::Ed25519,
            ssh_keys::SshKeySource::Imported,
            Date(784_000_000.0),
        );
        key.keychain_ref = Some(named.clone());
        keys.add((&key).into()).unwrap();

        assert_eq!(
            connections.release_item(mine.clone(), keys.clone(), keychain.clone()),
            StoreSecretFate::Deleted
        );
        assert!(!vault.exists(mine));
        assert_eq!(
            connections.release_item(named.clone(), keys.clone(), keychain.clone()),
            StoreSecretFate::Kept,
            "a key record names it"
        );
        assert!(vault.exists(named.clone()));

        // An earlier launch listed one; this launch settles it.
        let prefs: Arc<dyn PreferencesHost> = Arc::new(Prefs::default());
        let (connections, keys) = (
            ConnectionStoreHandle::new(prefs.clone(), ReadinessMode::Always),
            KeyStoreHandle::new(prefs.clone(), ReadinessMode::Always),
        );
        prefs.set(
            secrets::TO_RELEASE_KEY.into(),
            Some(to_value(&Plist::Array(vec![Plist::String(listed.clone())]))),
        );
        let settled = connections.release_pending(keys.clone(), keychain.clone());
        assert_eq!(
            settled,
            [StoreSettledRelease {
                account: listed.clone(),
                fate: StoreSecretFate::Deleted
            }]
        );
        assert!(!vault.exists(listed));
    }

    /// A keychain whose items in `guarded` are behind Face ID: read without
    /// asking, they fail as iOS fails them. Counts those reads.
    #[derive(Default)]
    struct FaceIdVault {
        vault: Vault,
        guarded: Mutex<BTreeSet<String>>,
        silent_reads: Mutex<usize>,
    }

    impl KeychainHost for FaceIdVault {
        fn save(
            &self,
            account: String,
            secret: Vec<u8>,
            guarded: bool,
        ) -> Result<(), KeychainFailure> {
            if guarded {
                lock(&self.guarded).insert(account.clone());
            }
            self.vault.save(account, secret, guarded)
        }
        fn load(&self, account: String, prompt: String) -> Result<Vec<u8>, KeychainFailure> {
            self.vault.load(account, prompt)
        }
        fn delete(&self, account: String) -> Result<(), KeychainFailure> {
            self.vault.delete(account)
        }
        fn exists(&self, account: String) -> bool {
            self.vault.exists(account)
        }
        fn accounts(&self) -> Result<Vec<String>, KeychainFailure> {
            self.vault.accounts()
        }
        fn load_silently(&self, account: String) -> Result<Vec<u8>, KeychainFailure> {
            *lock(&self.silent_reads) += 1;
            if lock(&self.guarded).contains(&account) {
                return Err(KeychainFailure::InteractionNotAllowed);
            }
            self.vault.load_silently(account)
        }
    }

    /// Keys 1.0.8 imported pasted without their public half, as it kept
    /// them (`SSHKeyFactory.importKey`, `AddKeyView.submit`): the label
    /// guessed from the PEM's first line — `BEGIN RSA` RSA-4096, `BEGIN EC`
    /// ECDSA-sk, anything else ED25519 — no public key, no fingerprint, the
    /// PEM trimmed into the keychain. Each: its name, the key as pasted,
    /// 1.0.8's label, Face ID on it, and what the key really is (algorithm,
    /// bits, `ssh-keygen -l`'s fingerprint, the fixtures' own).
    #[allow(clippy::type_complexity)]
    const PASTED_IN_1_0_8: [(&str, &str, &str, bool, KeyAlgorithm, Option<i64>, &str); 7] = [
        (
            "openssh rsa",
            include_str!("../../ssh/tests/fixtures/keys/rsa2048"),
            "ED25519",
            false,
            KeyAlgorithm::Rsa,
            Some(2048),
            "SHA256:N2LwkXVb8yni0Be3hZglQ7dx5gMpc8PwDBsVnb9psTw",
        ),
        (
            "openssh ecdsa",
            include_str!("../../ssh/tests/fixtures/keys/ecdsa256"),
            "ED25519",
            false,
            KeyAlgorithm::Ecdsa,
            None,
            "SHA256:0Y13tT5ReKS2k3cGXtoz0wDe+P+tGDk0SEJ20ssdW/E",
        ),
        (
            "pkcs8 ecdsa",
            include_str!("../../ssh/tests/fixtures/keys/ecdsa256-pkcs8"),
            "ED25519",
            false,
            KeyAlgorithm::Ecdsa,
            None,
            "SHA256:+/NM1+yXgl+S2qVb51R2UzjQJkEgeXRLbKly4kx/51c",
        ),
        (
            "pkcs8 rsa",
            include_str!("../../ssh/tests/fixtures/keys/matrix/keygen-rsa2048-pkcs8"),
            "ED25519",
            false,
            KeyAlgorithm::Rsa,
            Some(2048),
            "SHA256:/0oMA7hmnrMEcGfKKGdBmHikVtX37Jnj9u+BT1KUW+4",
        ),
        (
            "sec1 ec",
            include_str!("../../ssh/tests/fixtures/keys/matrix/openssl-p256-sec1"),
            "ECDSA-sk",
            true,
            KeyAlgorithm::Ecdsa,
            None,
            "SHA256:ymmErxUAgIrJg8Vbe3LmwEO/vCFHYvSy9MyH0Fx8eWM",
        ),
        (
            "pkcs1 rsa",
            include_str!("../../ssh/tests/fixtures/keys/rsa2048-pem"),
            "RSA-4096",
            false,
            KeyAlgorithm::Rsa,
            Some(2048),
            "SHA256:YI308USvR1zZ2NtAEtFF7tm0SR2whhSScC1VPQYyh8w",
        ),
        (
            "openssh ed25519",
            include_str!("../../ssh/tests/fixtures/keys/ed25519"),
            "ED25519",
            true,
            KeyAlgorithm::Ed25519,
            None,
            "SHA256:vUkJtFTGJFHkmC/nyQRfUJBNnAVbxp0w/5MKe0lMTK0",
        ),
    ];

    /// B726: a key 1.0.8 imported without its public half shows as what it
    /// is once 1.1.0 has read it — its algorithm, size, public key and
    /// fingerprint off the key itself, kept — and nothing else about the
    /// record changes. Read without asking at launch; one behind Face ID at
    /// its first sign-in. A SEC1 EC key (1.0.8's ECDSA-sk, which 1.0.8 never
    /// made for a security key) is a plain ECDSA key from the start, so the
    /// connection form's picker offers it before any read. A record with
    /// its facts is never read again.
    #[test]
    fn a_key_1_0_8_imported_without_its_public_half_learns_its_facts() {
        let host: Arc<dyn PreferencesHost> = Arc::new(Prefs::default());
        let vault = Arc::new(FaceIdVault::default());
        let keychain: Arc<dyn KeychainHost> = vault.clone();
        let mut stored = Vec::new();
        for (n, (name, pem, label, face_id, ..)) in PASTED_IN_1_0_8.iter().enumerate() {
            let account = format!("moshpit.secret.0f0e0d0c-0000-4000-8000-00000000010{n}");
            keychain
                .save(account.clone(), pem.trim().as_bytes().to_vec(), *face_id)
                .unwrap();
            stored.push(format!(
                r#"{{"id":"6A1F0B2C-0000-4000-8000-00000000010{n}","name":"{name}","algorithm":"{label}","comment":"","fingerprint":"","publicKey":"","source":"imported","boundHosts":[],"keychainRef":"{account}","requireBiometry":{face_id},"createdAt":784000000.5}}"#
            ));
        }
        // And one pasted with its public half: 1.0.8 labelled it right.
        let with_public = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGd2ZXJ5LXJlYWwta2V5LWJ5dGVzLWhlcmUtMzI= dev@example.com";
        keychain
            .save(
                "moshpit.secret.0f0e0d0c-0000-4000-8000-000000000200".into(),
                ED25519.trim().as_bytes().to_vec(),
                false,
            )
            .unwrap();
        stored.push(format!(
            r#"{{"id":"6A1F0B2C-0000-4000-8000-000000000200","name":"laptop","algorithm":"ED25519","comment":"","fingerprint":"SHA256:kept","publicKey":"{with_public}","source":"imported","boundHosts":["192.0.2.7"],"keychainRef":"moshpit.secret.0f0e0d0c-0000-4000-8000-000000000200","requireBiometry":false,"createdAt":784000000.5}}"#
        ));
        host.set(
            SshKeyStore::STORAGE_KEY.into(),
            Some(to_value(&Plist::Data(
                format!("[{}]", stored.join(",")).into_bytes(),
            ))),
        );
        let keys = KeyStoreHandle::new(host.clone(), ReadinessMode::Always);
        let named = |keys: &KeyStoreHandle, name: &str| {
            keys.keys().into_iter().find(|k| k.name == name).unwrap()
        };
        assert_eq!(
            named(&keys, "sec1 ec").algorithm,
            KeyAlgorithm::Ecdsa,
            "1.0.8's ECDSA-sk is a plain ECDSA key: the picker offers it"
        );

        let learned = store_learn_key_facts(keys.clone(), keychain.clone());
        let mut names: Vec<String> = learned.iter().map(|k| k.name.clone()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "openssh ecdsa",
                "openssh rsa",
                "pkcs1 rsa",
                "pkcs8 ecdsa",
                "pkcs8 rsa"
            ],
            "the ones read without asking"
        );
        assert_eq!(
            *lock(&vault.silent_reads),
            7,
            "one read for each without facts"
        );

        // The two behind Face ID, at their first sign-in.
        for name in ["sec1 ec", "openssh ed25519"] {
            let pem = PASTED_IN_1_0_8.iter().find(|c| c.0 == name).unwrap().1;
            let id = named(&keys, name).id;
            assert!(
                keys.learn_facts(id.clone(), pem.trim().into()).unwrap(),
                "{name}"
            );
            assert!(
                !keys.learn_facts(id, pem.trim().into()).unwrap(),
                "{name}: learned once"
            );
        }

        let reread = KeyStoreHandle::new(host.clone(), ReadinessMode::Always);
        for (name, _, _, face_id, algorithm, bits, fingerprint) in PASTED_IN_1_0_8 {
            for k in [named(&keys, name), named(&reread, name)] {
                assert_eq!(k.algorithm, algorithm, "{name}");
                assert_eq!(k.bits, bits, "{name}");
                assert_eq!(k.fingerprint, fingerprint, "{name}");
                assert!(
                    k.public_key.starts_with(match algorithm {
                        KeyAlgorithm::Rsa => "ssh-rsa AAAA",
                        KeyAlgorithm::Ecdsa => "ecdsa-sha2-nistp256 AAAA",
                        _ => "ssh-ed25519 AAAA",
                    }),
                    "{name}: {}",
                    k.public_key
                );
                assert_eq!(k.name, name, "the name stays");
                assert!(k.bound_hosts.is_empty(), "{name}: bound to nothing");
                assert_eq!(k.require_biometry, face_id, "{name}");
                assert_eq!(k.source, KeySource::Imported, "{name}");
            }
        }
        let laptop = named(&reread, "laptop");
        assert_eq!(
            (
                laptop.public_key.as_str(),
                laptop.fingerprint.as_str(),
                laptop.bound_hosts.clone()
            ),
            (with_public, "SHA256:kept", vec!["192.0.2.7".to_owned()]),
            "a record with its public key is left as it is"
        );

        assert!(store_learn_key_facts(reread.clone(), keychain.clone()).is_empty());
        assert_eq!(*lock(&vault.silent_reads), 7, "nothing left to read");
    }

    /// B780: a key 1.0.8 took (it kept any PEM saying PRIVATE KEY) of a kind
    /// 1.1.0 can't sign with — a brainpool SEC1 key it labelled ECDSA-sk, a
    /// DSA key it labelled ED25519 — says so once it has been read, so SSH
    /// Keys and the picker show why before a sign-in fails: read without
    /// asking at launch, or at its first sign-in behind Face ID. It keeps
    /// everything else, is never read again, and still deletes.
    #[test]
    fn a_key_1_0_9_cant_sign_with_says_what_kind_it_is() {
        let host: Arc<dyn PreferencesHost> = Arc::new(Prefs::default());
        let vault = Arc::new(FaceIdVault::default());
        let keychain: Arc<dyn KeychainHost> = vault.clone();
        let connections = ConnectionStoreHandle::new(host.clone(), ReadinessMode::Always);
        let pasted = [
            (
                "router",
                include_str!("../../ssh/tests/fixtures/keys/matrix/openssl-brainpool256-sec1"),
                "ECDSA-sk",
                false,
                "ECDSA brainpoolP256r1",
            ),
            (
                "old nas",
                include_str!("../../ssh/tests/fixtures/keys/matrix/openssl-dsa-traditional"),
                "ED25519",
                true,
                "DSA",
            ),
        ];
        let mut stored = Vec::new();
        for (n, (name, pem, label, face_id, _)) in pasted.iter().enumerate() {
            let account = format!("moshpit.secret.0f0e0d0c-0000-4000-8000-00000000030{n}");
            keychain
                .save(account.clone(), pem.trim().as_bytes().to_vec(), *face_id)
                .unwrap();
            stored.push(format!(
                r#"{{"id":"6A1F0B2C-0000-4000-8000-00000000030{n}","name":"{name}","algorithm":"{label}","comment":"","fingerprint":"","publicKey":"","source":"imported","boundHosts":[],"keychainRef":"{account}","requireBiometry":{face_id},"createdAt":784000000.5}}"#
            ));
        }
        host.set(
            SshKeyStore::STORAGE_KEY.into(),
            Some(to_value(&Plist::Data(
                format!("[{}]", stored.join(",")).into_bytes(),
            ))),
        );
        let keys = KeyStoreHandle::new(host.clone(), ReadinessMode::Always);
        let named = |keys: &KeyStoreHandle, name: &str| {
            keys.keys().into_iter().find(|k| k.name == name).unwrap()
        };

        let learned = store_learn_key_facts(keys.clone(), keychain.clone());
        assert_eq!(
            learned.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(),
            ["router"],
            "the one read without asking"
        );
        assert_eq!(
            named(&keys, "router").refused.as_deref(),
            Some("ECDSA brainpoolP256r1")
        );
        assert_eq!(
            named(&keys, "old nas").refused,
            None,
            "behind Face ID: not read yet"
        );
        // Its first sign-in reads it.
        let id = named(&keys, "old nas").id;
        assert!(
            keys.learn_facts(id.clone(), pasted[1].1.trim().into())
                .unwrap()
        );
        assert!(
            !keys.learn_facts(id, pasted[1].1.trim().into()).unwrap(),
            "once"
        );

        let reread = KeyStoreHandle::new(host.clone(), ReadinessMode::Always);
        for (name, _, _, face_id, kind) in pasted {
            for k in [named(&keys, name), named(&reread, name)] {
                assert_eq!(k.refused.as_deref(), Some(kind), "{name}");
                assert!(
                    k.public_key.is_empty() && k.fingerprint.is_empty(),
                    "{name}"
                );
                assert_eq!(k.name, name);
                assert_eq!(k.require_biometry, face_id, "{name}");
            }
        }
        let reads = *lock(&vault.silent_reads);
        assert!(store_learn_key_facts(reread.clone(), keychain.clone()).is_empty());
        assert_eq!(*lock(&vault.silent_reads), reads, "never read again");

        let router = named(&reread, "router");
        assert_eq!(
            reread
                .delete_key(router.id, connections.clone(), keychain.clone())
                .unwrap(),
            StoreKeyDeletion::Deleted
        );
        assert!(
            !keychain.exists(router.keychain_ref.unwrap()),
            "its item went too"
        );
    }
}
