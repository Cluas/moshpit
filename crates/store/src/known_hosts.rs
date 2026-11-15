//! The trusted host keys: one map `"host:port" → "SHA256:…"`, kept as a
//! single JSON object in the keychain item `moshpit.knownHosts`
//! (`KeychainKnownHostsStore`), read and written whole.
//!
//! The decisions made from it — trust on first use, a changed key, a map
//! that can't be read yet — are `moshpit-ssh`'s (`host_key::KnownHosts`),
//! over [`KnownHostsStore`]; this module owns the model they share and the
//! item's bytes, so the Rust and Swift transports take turns on the same
//! trusted keys.
//!
//! The rules carried over (B22, B40):
//! - a map that can't be read is not an empty one: a locked device is
//!   [`StoreError::Locked`], anything else [`StoreError::Unavailable`];
//! - a map this build can't decode is copied, untouched, to
//!   `moshpit.knownHosts.backup` (read back, not assumed) before the entries
//!   that do read as host → fingerprint are handed out, so the write that
//!   follows a trust never destroys the only copy; no copy, no map;
//! - a write replaces the item where it stands (the keychain's job: a
//!   delete followed by an add lost every key when the add failed);
//! - Fix (B226): "no item" read while protected data is unavailable — a
//!   launch before the first unlock after a reboot — is the lock's answer,
//!   not an empty map ([`KeychainKnownHosts::with_unlock_probe`]): taken as
//!   one, it was cached as the table, every trusted host read as new (a
//!   changed key too, offered as a new host instead of refused as changed),
//!   and in Swift the first trust after the unlock wrote one entry over
//!   every fingerprint.
//!
//! The item is not secret but must not be tampered with: on iOS it is a
//! generic password, `WhenUnlockedThisDeviceOnly`, no biometry — out of
//! backups and bound to the device.

use std::collections::BTreeMap;
use std::sync::Arc;

use moshpit_seal::foundation::json::{self, Dialect, Value};

use crate::codec;
use crate::host::{Keychain, KeychainError};
use crate::stored_list;

/// The keychain account of the map.
pub const ACCOUNT: &str = "moshpit.knownHosts";

/// The trusted map: entry ([`entry`]) → fingerprint.
pub type Table = BTreeMap<String, String>;

/// The map's key for a host: `host:port`, an IPv6 address unbracketed.
///
/// B630: one form for one host — lowercase, without the root's trailing
/// dot. DNS names and an IPv6 address's hex are case-blind, and
/// `example.com.` is `example.com`; the host as typed
/// (`HostKeyValidator.entryKey`) made each spelling a host of its own, so a
/// key trusted under one was nothing to another, and a different key there
/// got the quiet New Host card instead of the loud Key Changed one.
pub fn entry(host: &str, port: u16) -> String {
    format!("{}:{port}", host_form(host))
}

/// The [`entry`] a key stored by an earlier build stands for (it kept the
/// host as typed). `None` for one that isn't `host:port`.
pub fn normalized(stored: &str) -> Option<String> {
    let (host, port) = stored.rsplit_once(':')?;
    Some(format!("{}:{port}", host_form(host)))
}

fn host_form(host: &str) -> String {
    host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase()
}

/// The trusted map can't be read right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The device is locked (the item is readable only while unlocked): the
    /// map comes back the moment it unlocks.
    Locked,
    /// Anything else: a failing keychain, bytes that couldn't be kept safe.
    /// Won't heal on its own.
    Unavailable(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Locked => f.write_str("known hosts unreadable: the device is locked"),
            Self::Unavailable(why) => write!(f, "known hosts unreadable: {why}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Where the trusted map lives.
///
/// `read` returns the map as stored, empty when nothing was ever stored, and
/// fails when it can't be read now. A map this build can't decode is the
/// store's to keep safe before it hands out the entries that do decode.
/// `write` replaces the map where it stands.
pub trait KnownHostsStore: Send + Sync {
    fn read(&self) -> Result<Table, StoreError>;
    fn write(&self, entries: &Table) -> Result<(), StoreError>;
}

/// Whether the device's protected data is readable now (iOS
/// `isProtectedDataAvailable`).
pub type UnlockProbe = Arc<dyn Fn() -> bool + Send + Sync>;

/// The map in the keychain (`KeychainKnownHostsStore`).
pub struct KeychainKnownHosts {
    keychain: Arc<dyn Keychain>,
    account: String,
    /// Asked around a read that found no item; `None` takes "no item" at
    /// its word (tests, platforms without a sealed launch).
    unlocked: Option<UnlockProbe>,
}

impl KeychainKnownHosts {
    pub fn new(keychain: Arc<dyn Keychain>) -> Self {
        Self::with_account(keychain, ACCOUNT)
    }

    pub fn with_account(keychain: Arc<dyn Keychain>, account: &str) -> Self {
        Self {
            keychain,
            account: account.to_owned(),
            unlocked: None,
        }
    }

    /// The sealed-launch rule for "no item" (B226): believed only when
    /// `unlocked` says the device was unlocked before and after the read;
    /// otherwise the read is [`StoreError::Locked`], and the map is read
    /// again once the device is.
    pub fn with_unlock_probe(mut self, unlocked: UnlockProbe) -> Self {
        self.unlocked = Some(unlocked);
        self
    }

    /// Where a map this build can't decode is copied before anything may be
    /// written over it.
    pub fn backup_account(&self) -> String {
        format!("{}.backup", self.account)
    }

    /// A map that won't decode as `[host:port: fingerprint]`: its bytes go to
    /// a backup account first (read back), then whatever entries of it are
    /// strings come out.
    ///
    /// Fix (known-hosts-backup): `KeychainKnownHostsStore.salvage`
    /// (`HostKeyValidator.swift:98-103`) has one backup account and writes
    /// each undecodable map over it, so a second one (another build's, or
    /// the same build's next format) destroys the only copy of the first.
    /// Here the backup account is written only while it is empty; a map
    /// that differs from the one kept there goes to
    /// `<account>.backup-<unix seconds>` (a counter appended when that is
    /// taken), as the pairing file keeps its copies.
    fn salvage(&self, data: &[u8]) -> Result<Table, StoreError> {
        let first = self.backup_account();
        let mut backup = first.clone();
        let kept = |account: &str| self.keychain.load(account, "").ok();
        let mut existing = kept(&backup);
        if existing.is_some() && existing.as_deref() != Some(data) {
            let seconds = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs());
            backup = format!("{first}-{seconds}");
            let mut counter = 1;
            loop {
                existing = kept(&backup);
                if existing.is_none() || existing.as_deref() == Some(data) {
                    break;
                }
                counter += 1;
                backup = format!("{first}-{seconds}-{counter}");
            }
        }
        if existing.as_deref() != Some(data) {
            let _ = self.keychain.save(&backup, data, false);
            if kept(&backup).as_deref() != Some(data) {
                return Err(StoreError::Unavailable(
                    "undecodable, and no copy of it could be kept".to_owned(),
                ));
            }
            log::error!(
                "known hosts: stored map undecodable ({} bytes); copied to {backup}",
                data.len()
            );
        }
        // `JSONSerialization.jsonObject(with:) as? [String: Any]`, then the
        // values that are strings (with every string as stored: Fix
        // (bom-strip), crate::stored_list).
        let Some(Value::Object(object)) = stored_list::serialization_tree(data) else {
            return Ok(Table::new());
        };
        let mut table = Table::new();
        for (key, value) in object.iter() {
            if let Some(fingerprint) = value.as_str() {
                table
                    .entry(key.to_owned())
                    .or_insert_with(|| fingerprint.to_owned());
            }
        }
        Ok(table)
    }
}

/// `JSONDecoder().decode([String: String].self, from: data)`.
pub fn decode(data: &[u8]) -> Option<Table> {
    let Value::Object(object) = json::parse(data, Dialect::Decoder).ok()? else {
        return None;
    };
    object
        .iter()
        .map(|(key, value)| value.as_str().map(|v| (key.to_owned(), v.to_owned())))
        .collect()
}

/// `JSONEncoder().encode(entries)` (members sorted).
pub fn encode(entries: &Table) -> Vec<u8> {
    codec::encode(entries).into_bytes()
}

impl KnownHostsStore for KeychainKnownHosts {
    fn read(&self) -> Result<Table, StoreError> {
        let unlocked = || self.unlocked.as_ref().is_none_or(|probe| probe());
        let unlocked_before = unlocked();
        let data = match self.keychain.load(&self.account, "") {
            Ok(data) => data,
            // Nothing trusted yet — or nothing the keychain shows before
            // the first unlock (B226).
            Err(KeychainError::NotFound) if unlocked_before && unlocked() => {
                return Ok(Table::new());
            }
            Err(KeychainError::NotFound) => {
                log::info!("known hosts: no item read before the first unlock; not believed");
                return Err(StoreError::Locked);
            }
            // What a `WhenUnlocked` item answers while the device is locked.
            Err(KeychainError::InteractionNotAllowed) => return Err(StoreError::Locked),
            Err(other) => return Err(StoreError::Unavailable(other.to_string())),
        };
        match decode(&data) {
            Some(entries) => Ok(entries),
            None => self.salvage(&data),
        }
    }

    fn write(&self, entries: &Table) -> Result<(), StoreError> {
        // No biometry: the item is `WhenUnlockedThisDeviceOnly` and reads
        // never prompt.
        self.keychain
            .save(&self.account, &encode(entries), false)
            .map_err(|error| match error {
                KeychainError::InteractionNotAllowed => StoreError::Locked,
                other => StoreError::Unavailable(other.to_string()),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::MemoryKeychain;

    fn table(entries: &[(&str, &str)]) -> Table {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn entries_are_keyed_host_colon_port_as_typed() {
        assert_eq!(entry("dev.local", 22), "dev.local:22");
        assert_eq!(entry("::1", 2222), "::1:2222");
    }

    #[test]
    fn nothing_stored_is_an_empty_map_and_a_write_reads_back() {
        let keychain = Arc::new(MemoryKeychain::new());
        let store = KeychainKnownHosts::new(keychain.clone());
        assert_eq!(store.read(), Ok(Table::new()));
        let map = table(&[(
            "box:22",
            "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU",
        )]);
        store.write(&map).unwrap();
        assert_eq!(
            keychain.snapshot()[ACCOUNT],
            br#"{"box:22":"SHA256:47DEQpj8HBSa+\/TImW+5JCeuQeRkm5NMpJWZG3hSuFU"}"#
        );
        assert_eq!(store.read(), Ok(map));
    }

    /// B22: locked is not empty.
    #[test]
    fn a_locked_keychain_is_locked_not_empty() {
        let keychain = Arc::new(MemoryKeychain::with([(ACCOUNT.to_owned(), b"{}".to_vec())]));
        keychain.set_locked(true);
        let store = KeychainKnownHosts::new(keychain);
        assert_eq!(store.read(), Err(StoreError::Locked));
        assert_eq!(store.write(&Table::new()), Err(StoreError::Locked));
    }

    /// A keychain before the device's first unlock since boot: no item
    /// shows at all (`notFound`), and nothing can be written.
    struct Sealed {
        inner: MemoryKeychain,
        sealed: std::sync::atomic::AtomicBool,
    }

    impl Sealed {
        fn sealed(&self) -> bool {
            self.sealed.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Keychain for Sealed {
        fn save(&self, account: &str, secret: &[u8], biometry: bool) -> Result<(), KeychainError> {
            if self.sealed() {
                return Err(KeychainError::InteractionNotAllowed);
            }
            self.inner.save(account, secret, biometry)
        }
        fn load(&self, account: &str, prompt: &str) -> Result<Vec<u8>, KeychainError> {
            if self.sealed() {
                return Err(KeychainError::NotFound);
            }
            self.inner.load(account, prompt)
        }
        fn delete(&self, account: &str) -> Result<(), KeychainError> {
            self.inner.delete(account)
        }
        fn exists(&self, account: &str) -> bool {
            !self.sealed() && self.inner.exists(account)
        }
    }

    /// B226: before the first unlock, "no item" is the lock's answer.
    #[test]
    fn no_item_before_the_first_unlock_is_locked_not_empty() {
        let trusted = table(&[("a:22", "SHA256:a"), ("b:22", "SHA256:b")]);
        let keychain = Arc::new(Sealed {
            inner: MemoryKeychain::with([(ACCOUNT.to_owned(), encode(&trusted))]),
            sealed: true.into(),
        });
        let probe = keychain.clone();
        let store = KeychainKnownHosts::new(keychain.clone())
            .with_unlock_probe(Arc::new(move || !probe.sealed()));
        assert_eq!(store.read(), Err(StoreError::Locked));
        keychain
            .sealed
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(store.read(), Ok(trusted));
        // Unlocked, no item is still the one empty map.
        let empty = KeychainKnownHosts::new(Arc::new(MemoryKeychain::new()))
            .with_unlock_probe(Arc::new(|| true));
        assert_eq!(empty.read(), Ok(Table::new()));
    }

    #[test]
    fn an_undecodable_map_is_copied_aside_before_its_string_entries_are_handed_out() {
        let stored = br#"{"a:22":"SHA256:a","b:22":{"v":2,"fp":"SHA256:b"}}"#.to_vec();
        let keychain = Arc::new(MemoryKeychain::with([(ACCOUNT.to_owned(), stored.clone())]));
        let store = KeychainKnownHosts::new(keychain.clone());
        assert_eq!(store.read(), Ok(table(&[("a:22", "SHA256:a")])));
        assert_eq!(keychain.snapshot()["moshpit.knownHosts.backup"], stored);
        // Garbage keeps its copy too and hands out nothing.
        let keychain = Arc::new(MemoryKeychain::with([(
            ACCOUNT.to_owned(),
            b"garbage".to_vec(),
        )]));
        let store = KeychainKnownHosts::new(keychain.clone());
        assert_eq!(store.read(), Ok(Table::new()));
        assert_eq!(keychain.snapshot()["moshpit.knownHosts.backup"], b"garbage");
    }

    /// B155: a salvaged map keeps a host that starts with U+FEFF as it was
    /// stored (Swift's JSONSerialization dropped it from every key, and the
    /// fingerprint came back under another host). Pinned in Rust when the
    /// Swift store and its test went (C).
    #[test]
    fn a_salvaged_map_keeps_a_host_that_starts_with_a_byte_order_mark() {
        let stored =
            "{\"\u{FEFF}b.example:22\":\"SHA256:BBBB\",\"a.example:22\":[\"SHA256:AAAA\"]}"
                .as_bytes()
                .to_vec();
        let keychain = Arc::new(MemoryKeychain::with([(ACCOUNT.to_owned(), stored)]));
        let store = KeychainKnownHosts::new(keychain);
        assert_eq!(
            store.read(),
            Ok(table(&[("\u{FEFF}b.example:22", "SHA256:BBBB")]))
        );
    }

    /// Fix (known-hosts-backup): Swift wrote the second map over the first
    /// one's copy.
    #[test]
    fn a_second_undecodable_map_does_not_overwrite_the_first_ones_copy() {
        let keychain = Arc::new(MemoryKeychain::with([(
            ACCOUNT.to_owned(),
            b"first".to_vec(),
        )]));
        let store = KeychainKnownHosts::new(keychain.clone());
        assert_eq!(store.read(), Ok(Table::new()));
        keychain.save(ACCOUNT, b"second", false).unwrap();
        assert_eq!(store.read(), Ok(Table::new()));
        assert_eq!(store.read(), Ok(Table::new()));
        let items = keychain.snapshot();
        assert_eq!(items["moshpit.knownHosts.backup"], b"first");
        let stamped: Vec<&Vec<u8>> = items
            .iter()
            .filter(|(k, _)| k.starts_with("moshpit.knownHosts.backup-"))
            .map(|(_, v)| v)
            .collect();
        assert_eq!(stamped, [&b"second".to_vec()]);
    }

    /// A keychain whose backup write doesn't stick.
    struct NoBackups(MemoryKeychain);

    impl Keychain for NoBackups {
        fn save(&self, account: &str, secret: &[u8], biometry: bool) -> Result<(), KeychainError> {
            if account.ends_with(".backup") {
                return Err(KeychainError::Other("full".into()));
            }
            self.0.save(account, secret, biometry)
        }
        fn load(&self, account: &str, prompt: &str) -> Result<Vec<u8>, KeychainError> {
            self.0.load(account, prompt)
        }
        fn delete(&self, account: &str) -> Result<(), KeychainError> {
            self.0.delete(account)
        }
        fn exists(&self, account: &str) -> bool {
            self.0.exists(account)
        }
    }

    #[test]
    fn no_copy_no_map() {
        let keychain = NoBackups(MemoryKeychain::with([(
            ACCOUNT.to_owned(),
            b"[1]".to_vec(),
        )]));
        let store = KeychainKnownHosts::new(Arc::new(keychain));
        assert!(matches!(store.read(), Err(StoreError::Unavailable(_))));
    }
}
