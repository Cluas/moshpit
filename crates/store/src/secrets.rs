//! The keychain items the saved records point at — each connection's
//! password or pasted key, each SSH key's private half — and what becomes of
//! the ones nothing points at any more (B232; the app's
//! `KeychainReferences.swift` and `SSHKeyDeletion`).
//!
//! Fix (B232): deleting a key removed its record and nothing else (the
//! Swift store's `remove`, and [`SshKeyStore::remove`] here): the private
//! key stayed in the keychain under an account no record named any more, so
//! nothing could ever reach it again — a Secure Enclave key's handle as much
//! as an imported key's file (the handle in that one item IS the key).
//! Deleting a connection that signed in with a managed key left its item
//! too, also once the key's record was gone. Now:
//!
//! - a key a saved connection still signs in with is not deleted at all
//!   ([`KeyDeletion::InUse`] names those connections);
//! - an item goes with the last record naming it, counted over every saved
//!   connection and every key record, the ones carried undecoded included
//!   ([`delete_key`], [`delete_connection`]);
//! - the private keys earlier builds orphaned are NOT deleted but listed
//!   again as key records ([`recover_orphans`]): a record can have been lost
//!   without the user deleting anything (a key list read as empty and saved
//!   over, B8 / B32), and then the orphan is the only copy of the key. Only
//!   accounts [`new_keychain_ref`](crate::host::new_keychain_ref) makes
//!   (`moshpit.secret.…`), named by nothing, and not made in this process
//!   are looked at — never the trusted host keys, never anything else in the
//!   keychain — and nothing there is ever deleted or rewritten.
//!
//! Nothing here guesses: a store whose read can't be trusted (the
//! sealed-launch rule, [`crate::readiness`]), or one carrying an entry whose
//! ref can't be read, makes the answer unknown, and unknown changes nothing.

use std::collections::BTreeSet;

use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::uuid::Uuid;

use crate::connections::ConnectionStore;
use crate::host::{KEYCHAIN_REF_PREFIX, Keychain, KeychainError, minted_this_launch};
use crate::ssh_keys::{SshKeyAlgorithm, SshKeyRecord, SshKeySource, SshKeyStore};

/// Every keychain account a saved connection or a stored key record names;
/// `None` when either store can't say. `excluding_connection` and
/// `excluding_key` leave out the record about to go.
pub fn referenced(
    connections: &ConnectionStore,
    keys: &SshKeyStore,
    excluding_connection: Option<Uuid>,
    excluding_key: Option<Uuid>,
) -> Option<BTreeSet<String>> {
    let mut held = connections.keychain_refs(excluding_connection)?;
    held.extend(keys.keychain_refs(excluding_key)?);
    Some(held)
}

/// What deleting a key would do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCheck {
    /// Nothing uses it: the record and its private key can go.
    Free,
    /// These saved connections still sign in with it: nothing goes.
    InUse(Vec<String>),
    /// Which connections use it can't be told now: nothing goes.
    Unknown,
}

/// What [`check_key`] says of key `id`, from the stores as they are now
/// (read again first when their last read can't be trusted); `None` when
/// there is no such key.
pub fn check_key(
    id: Uuid,
    keys: &mut SshKeyStore,
    connections: &mut ConnectionStore,
) -> Option<KeyCheck> {
    connections.reload_if_needed();
    keys.reload_if_needed();
    let key = keys.keys().iter().find(|k| k.id == id)?;
    if keys.keychain_refs(None).is_none() {
        return Some(KeyCheck::Unknown);
    }
    Some(
        match connections.names_using_key(id, key.keychain_ref.as_deref()) {
            None => KeyCheck::Unknown,
            Some(names) if names.is_empty() => KeyCheck::Free,
            Some(names) => KeyCheck::InUse(names),
        },
    )
}

/// What deleting a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyDeletion {
    /// The record is gone, and its private key with it (or left for another
    /// record naming the same item).
    Deleted,
    /// These saved connections still sign in with it: nothing was deleted.
    InUse(Vec<String>),
    /// Which connections use it can't be told now: nothing was deleted.
    Unknown,
    /// There is no such key.
    NotFound,
    /// The keychain wouldn't delete the private key: the record stays, the
    /// one way back to it.
    Keychain(KeychainError),
}

/// Delete key `id`: its private key from `keychain` first, then its
/// record — unless a saved connection still signs in with it, its users
/// can't be told, or the keychain keeps the item.
pub fn delete_key(
    id: Uuid,
    keys: &mut SshKeyStore,
    connections: &mut ConnectionStore,
    keychain: &dyn Keychain,
) -> KeyDeletion {
    match check_key(id, keys, connections) {
        None => return KeyDeletion::NotFound,
        Some(KeyCheck::InUse(names)) => return KeyDeletion::InUse(names),
        Some(KeyCheck::Unknown) => return KeyDeletion::Unknown,
        Some(KeyCheck::Free) => {}
    }
    let account = keys
        .keys()
        .iter()
        .find(|k| k.id == id)
        .and_then(|k| k.keychain_ref.clone());
    if let Some(account) = account {
        let Some(held) = referenced(connections, keys, None, Some(id)) else {
            return KeyDeletion::Unknown;
        };
        if !held.contains(&account)
            && let Err(error) = keychain.delete(&account)
        {
            log::error!("key delete: the keychain kept the private key ({error})");
            return KeyDeletion::Keychain(error);
        }
    }
    keys.remove(id);
    KeyDeletion::Deleted
}

/// What became of a deleted connection's keychain item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemFate {
    /// Deleted: nothing else named it.
    Deleted,
    /// Kept: another connection or a key record names it, or the connection
    /// had none (or wasn't saved).
    Kept,
    /// Kept: what else names it can't be told now. The sweep has it later.
    Unknown,
    /// The keychain wouldn't delete it. The sweep has it later.
    Failed(KeychainError),
}

/// Drop `account` once no saved connection and no key record names it
/// (`excluding_connection`: the record about to go is not counted): what a
/// connection deleted (B232) or no longer naming its own item (B325) does
/// with the item.
fn drop_unnamed(
    account: &str,
    connections: &ConnectionStore,
    keys: &SshKeyStore,
    excluding_connection: Option<Uuid>,
    keychain: &dyn Keychain,
) -> ItemFate {
    match referenced(connections, keys, excluding_connection, None) {
        None => ItemFate::Unknown,
        Some(held) if held.contains(account) => ItemFate::Kept,
        Some(_) => match keychain.delete(account) {
            Ok(()) => ItemFate::Deleted,
            Err(error) => {
                log::error!("keychain: it kept an item nothing names ({error})");
                ItemFate::Failed(error)
            }
        },
    }
}

/// A saved connection's own item it no longer names (it signs in with a
/// managed key now, B325): dropped once nothing else names it, by
/// [`delete_connection`]'s rule. Call it after the connection is saved.
pub fn release_item(
    account: &str,
    connections: &mut ConnectionStore,
    keys: &mut SshKeyStore,
    keychain: &dyn Keychain,
) -> ItemFate {
    connections.reload_if_needed();
    keys.reload_if_needed();
    drop_unnamed(account, connections, keys, None, keychain)
}

/// Delete connection `id`, and its keychain item with it once no other
/// connection and no key record names it — a managed key's item stays while
/// the key is listed; one whose key's record is already gone goes with the
/// last connection on it. The record goes whatever becomes of the item: an
/// item left behind is the sweep's.
pub fn delete_connection(
    id: Uuid,
    connections: &mut ConnectionStore,
    keys: &mut SshKeyStore,
    keychain: &dyn Keychain,
) -> ItemFate {
    connections.reload_if_needed();
    keys.reload_if_needed();
    let account = connections
        .connections()
        .iter()
        .find(|c| c.id == id)
        .and_then(|c| c.keychain_ref.clone());
    let fate = match account {
        None => ItemFate::Kept,
        Some(account) => drop_unnamed(&account, connections, keys, Some(id), keychain),
    };
    connections.delete(id);
    fate
}

/// What a private key is, read off the key alone: what its record shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFacts {
    pub algorithm: SshKeyAlgorithm,
    pub source: SshKeySource,
    /// The `authorized_keys` line.
    pub public_key: String,
    /// `SHA256:…`.
    pub fingerprint: String,
    /// An RSA key's modulus size.
    pub bits: Option<i64>,
}

/// Why [`recover_orphans`] didn't look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    /// Which items are named can't be told yet (a store read before the
    /// first unlock, an entry whose ref can't be read).
    Unknown,
    /// The keychain couldn't list its items.
    Keychain(KeychainError),
}

/// Whether `account` is an item recovery may look at at all: an account the
/// app made for a connection or a key, and not in this process (whose
/// record may still be on its way: an item is written before the record
/// naming it).
pub fn is_ours(account: &str) -> bool {
    account.starts_with(KEYCHAIN_REF_PREFIX) && !minted_this_launch(account)
}

/// Give every orphaned private key a key record again — its account
/// [`is_ours`], no saved connection and no key record names it — and return
/// the records added. Each item is read without asking
/// ([`Keychain::load_silently`]: one behind Face ID is left for now) and
/// becomes a record only if `describe` reads it as a private key; anything
/// else (a deleted connection's password, a key behind a passphrase) stays
/// where it is and is logged. `name` words the record's name from the key's
/// facts (the app: "Recovered key" and the start of its fingerprint). The
/// record is the key's from then on, so a later run adds nothing for it;
/// deleting it in SSH Keys deletes the item ([`delete_key`]). Nothing is
/// ever deleted or rewritten here. The app runs it once a launch, the first
/// time both stores can be trusted.
pub fn recover_orphans(
    connections: &ConnectionStore,
    keys: &mut SshKeyStore,
    keychain: &dyn Keychain,
    describe: &dyn Fn(&[u8]) -> Option<KeyFacts>,
    name: &dyn Fn(&KeyFacts) -> String,
    now: Date,
) -> Result<Vec<SshKeyRecord>, RecoveryError> {
    referenced(connections, keys, None, None).ok_or(RecoveryError::Unknown)?;
    let mut accounts = keychain.accounts().map_err(RecoveryError::Keychain)?;
    accounts.sort();
    let mut recovered = Vec::new();
    for account in accounts {
        if !is_ours(&account) {
            continue;
        }
        let held = referenced(connections, keys, None, None).ok_or(RecoveryError::Unknown)?;
        if held.contains(&account) {
            continue;
        }
        let secret = match keychain.load_silently(&account) {
            Ok(secret) => secret,
            Err(error) => {
                log::info!(
                    "keychain recovery: an orphaned item can't be read without asking ({error}); left as it is"
                );
                continue;
            }
        };
        let Some(facts) = describe(&secret) else {
            log::info!(
                "keychain recovery: an orphaned item isn't a private key (a password, or a key behind a passphrase); left as it is"
            );
            continue;
        };
        let mut record = SshKeyRecord::new(
            Uuid::new_v4(),
            &name(&facts),
            facts.algorithm,
            facts.source,
            now,
        );
        record.bits = facts.bits;
        record.fingerprint = facts.fingerprint;
        record.public_key = facts.public_key;
        record.keychain_ref = Some(account);
        record.require_biometry = false;
        keys.add(record.clone());
        recovered.push(record);
    }
    if !recovered.is_empty() {
        log::info!(
            "keychain recovery: {} orphaned private key(s) listed again",
            recovered.len()
        );
    }
    Ok(recovered)
}

/// The start of a fingerprint's hash, to tell two recovered keys apart in
/// their names: `SHA256:` dropped, eight characters kept.
pub fn short_fingerprint(fingerprint: &str) -> String {
    fingerprint
        .strip_prefix("SHA256:")
        .unwrap_or(fingerprint)
        .chars()
        .take(8)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use moshpit_seal::foundation::date::Date;
    use moshpit_seal::foundation::json::{self, Dialect, Value};

    use super::*;
    use crate::connections::{AuthMethod, ServerConnection};
    use crate::host::{Defaults, MemoryDefaults, MemoryKeychain, new_keychain_ref};
    use crate::plist::Plist;
    use crate::readiness::Readiness;
    use crate::ssh_keys::{SshKeyAlgorithm, SshKeyRecord, SshKeySource};

    struct Stores {
        defaults: Arc<MemoryDefaults>,
        connections: ConnectionStore,
        keys: SshKeyStore,
    }

    fn stores() -> Stores {
        let defaults = Arc::new(MemoryDefaults::new());
        Stores {
            connections: ConnectionStore::new(defaults.clone(), Readiness::Always),
            keys: SshKeyStore::new(defaults.clone(), Readiness::Always),
            defaults,
        }
    }

    /// An account an earlier launch made (not through `new_keychain_ref`).
    fn account() -> String {
        format!(
            "{KEYCHAIN_REF_PREFIX}{}",
            Uuid::new_v4().to_string().to_lowercase()
        )
    }

    fn key(name: &str, account: Option<&str>) -> SshKeyRecord {
        let mut key = SshKeyRecord::new(
            Uuid::new_v4(),
            name,
            SshKeyAlgorithm::Ed25519,
            SshKeySource::Generated,
            Date::from_unix(1_762_800_000.0),
        );
        key.keychain_ref = account.map(str::to_owned);
        key
    }

    fn connection(name: &str, key: Option<&SshKeyRecord>, own: Option<&str>) -> ServerConnection {
        let mut c = ServerConnection::new(Uuid::new_v4(), Date::from_unix(1_762_800_000.0));
        c.name = name.into();
        c.host = format!("{name}.example.com");
        c.username = "dev".into();
        if let Some(key) = key {
            c.auth_method = AuthMethod::Key;
            c.keychain_ref = key.keychain_ref.clone();
            c.ssh_key_id = Some(key.id);
            c.ssh_key_algorithm_raw = Some(key.algorithm.raw_value().into());
        } else {
            c.keychain_ref = own.map(str::to_owned);
        }
        c
    }

    fn items(keychain: &MemoryKeychain) -> BTreeSet<String> {
        keychain.snapshot().into_keys().collect()
    }

    /// Adds a connection this build can't decode — a newer build's — to the
    /// stored list, naming `account`.
    fn carry_connection(s: &mut Stores, account: &str, key: Option<Uuid>) {
        let Some(Plist::Data(data)) = s.defaults.get(ConnectionStore::STORAGE_KEY) else {
            panic!("no stored list")
        };
        let Value::Array(mut entries) = json::parse(&data, Dialect::Decoder).unwrap() else {
            panic!("not a list")
        };
        let mut future = json::Object::new();
        future.insert("id", Value::String(Uuid::new_v4().to_string()));
        future.insert("name", Value::String("future".into()));
        future.insert("authMethod", Value::String("passkey".into()));
        future.insert("keychainRef", Value::String(account.into()));
        if let Some(key) = key {
            future.insert("sshKeyId", Value::String(key.to_string()));
        }
        entries.push(Value::Object(future));
        s.defaults.set(
            ConnectionStore::STORAGE_KEY,
            Some(Plist::Data(
                crate::codec::write(&Value::Array(entries)).into_bytes(),
            )),
        );
        s.connections.load();
    }

    #[test]
    fn deleting_a_key_nothing_uses_takes_its_private_key_with_it() {
        let mut s = stores();
        let (mine, other) = (account(), account());
        let keychain = MemoryKeychain::with([
            (mine.clone(), b"PRIVATE".to_vec()),
            (other.clone(), b"OTHER".to_vec()),
        ]);
        let laptop = key("laptop", Some(&mine));
        let kept = key("kept", Some(&other));
        s.keys.add(laptop.clone());
        s.keys.add(kept.clone());
        assert_eq!(
            check_key(laptop.id, &mut s.keys, &mut s.connections),
            Some(KeyCheck::Free)
        );
        assert_eq!(
            delete_key(laptop.id, &mut s.keys, &mut s.connections, &keychain),
            KeyDeletion::Deleted
        );
        assert_eq!(
            s.keys.keys().iter().map(|k| k.id).collect::<Vec<_>>(),
            [kept.id]
        );
        assert_eq!(items(&keychain), BTreeSet::from([other]));
        // The record is gone from storage too.
        assert_eq!(
            SshKeyStore::new(s.defaults.clone(), Readiness::Always)
                .keys()
                .len(),
            1
        );
    }

    #[test]
    fn a_key_a_saved_connection_signs_in_with_stays_and_says_who() {
        let mut s = stores();
        let mine = account();
        let keychain = MemoryKeychain::with([(mine.clone(), b"PRIVATE".to_vec())]);
        let laptop = key("laptop", Some(&mine));
        s.keys.add(laptop.clone());
        s.connections.add(connection("pi", Some(&laptop), None));
        assert_eq!(
            delete_key(laptop.id, &mut s.keys, &mut s.connections, &keychain),
            KeyDeletion::InUse(vec!["pi".into()])
        );
        assert_eq!(s.keys.keys().len(), 1);
        assert_eq!(items(&keychain), BTreeSet::from([mine.clone()]));
        // A newer build's connection on it counts too.
        s.connections.delete(s.connections.connections()[0].id);
        carry_connection(&mut s, &mine, Some(laptop.id));
        assert_eq!(
            delete_key(laptop.id, &mut s.keys, &mut s.connections, &keychain),
            KeyDeletion::InUse(vec!["future".into()])
        );
        assert_eq!(items(&keychain), BTreeSet::from([mine]));
    }

    #[test]
    fn untrusted_stores_delete_nothing() {
        let defaults = Arc::new(MemoryDefaults::new());
        let mine = account();
        let laptop = key("laptop", Some(&mine));
        SshKeyStore::new(defaults.clone(), Readiness::Always).add(laptop.clone());
        defaults.set(crate::readiness::SENTINEL_KEY, None);
        defaults.set_protected_data_available(false);
        let mut keys = SshKeyStore::new(defaults.clone(), Readiness::System);
        let mut connections = ConnectionStore::new(defaults.clone(), Readiness::System);
        let keychain = MemoryKeychain::with([(mine.clone(), b"PRIVATE".to_vec())]);
        assert_eq!(
            delete_key(laptop.id, &mut keys, &mut connections, &keychain),
            KeyDeletion::Unknown
        );
        assert_eq!(referenced(&connections, &keys, None, None), None);
        assert_eq!(items(&keychain), BTreeSet::from([mine]));
    }

    #[test]
    fn a_keychain_that_keeps_the_private_key_keeps_the_record() {
        let mut s = stores();
        let mine = account();
        let keychain = MemoryKeychain::with([(mine.clone(), b"PRIVATE".to_vec())]);
        let laptop = key("laptop", Some(&mine));
        s.keys.add(laptop.clone());
        keychain.set_locked(true);
        assert_eq!(
            delete_key(laptop.id, &mut s.keys, &mut s.connections, &keychain),
            KeyDeletion::Keychain(KeychainError::InteractionNotAllowed)
        );
        assert_eq!(s.keys.keys().len(), 1);
        keychain.set_locked(false);
        assert_eq!(items(&keychain), BTreeSet::from([mine]));
        assert_eq!(
            delete_key(Uuid::new_v4(), &mut s.keys, &mut s.connections, &keychain),
            KeyDeletion::NotFound
        );
    }

    #[test]
    fn an_item_two_records_name_stays_until_the_last_goes() {
        let mut s = stores();
        let shared = account();
        let keychain = MemoryKeychain::with([(shared.clone(), b"PRIVATE".to_vec())]);
        let a = key("a", Some(&shared));
        let b = key("b", Some(&shared));
        s.keys.add(a.clone());
        s.keys.add(b.clone());
        assert_eq!(
            delete_key(a.id, &mut s.keys, &mut s.connections, &keychain),
            KeyDeletion::Deleted
        );
        assert_eq!(items(&keychain), BTreeSet::from([shared.clone()]));
        assert_eq!(
            delete_key(b.id, &mut s.keys, &mut s.connections, &keychain),
            KeyDeletion::Deleted
        );
        assert!(items(&keychain).is_empty());
    }

    #[test]
    fn the_last_connection_on_a_deleted_keys_item_takes_the_item() {
        let mut s = stores();
        let orphaned = account();
        let listed = account();
        let shared = account();
        let keychain = MemoryKeychain::with([
            (orphaned.clone(), b"gone key".to_vec()),
            (listed.clone(), b"listed key".to_vec()),
            (shared.clone(), b"password".to_vec()),
        ]);
        // A key an earlier build deleted while `pi` used it: no record.
        let gone = key("gone", Some(&orphaned));
        let laptop = key("laptop", Some(&listed));
        s.keys.add(laptop.clone());
        let pi = connection("pi", Some(&gone), None);
        let nas = connection("nas", Some(&laptop), None);
        let a = connection("a", None, Some(&shared));
        let b = connection("b", None, Some(&shared));
        for c in [&pi, &nas, &a, &b] {
            s.connections.add((*c).clone());
        }
        let mut fate = |id| delete_connection(id, &mut s.connections, &mut s.keys, &keychain);
        assert_eq!(fate(pi.id), ItemFate::Deleted);
        assert_eq!(fate(nas.id), ItemFate::Kept);
        assert_eq!(fate(a.id), ItemFate::Kept);
        assert_eq!(fate(b.id), ItemFate::Deleted);
        assert_eq!(items(&keychain), BTreeSet::from([listed]));
        assert!(s.connections.connections().is_empty());
    }

    /// The tests' key reader: a "KEY <name>" secret is a private key whose
    /// fingerprint is `SHA256:<name>…`; anything else isn't one.
    fn describe(secret: &[u8]) -> Option<KeyFacts> {
        let name = std::str::from_utf8(secret).ok()?.strip_prefix("KEY ")?;
        Some(KeyFacts {
            algorithm: SshKeyAlgorithm::Ed25519,
            source: SshKeySource::Imported,
            public_key: format!("ssh-ed25519 AAAA{name} {name}"),
            fingerprint: format!("SHA256:{name}0123456789"),
            bits: None,
        })
    }

    fn recovered_name(facts: &KeyFacts) -> String {
        format!("Recovered key {}", short_fingerprint(&facts.fingerprint))
    }

    fn recover(
        s: &mut Stores,
        keychain: &dyn Keychain,
    ) -> Result<Vec<SshKeyRecord>, RecoveryError> {
        recover_orphans(
            &s.connections,
            &mut s.keys,
            keychain,
            &describe,
            &recovered_name,
            Date::from_unix(1_762_900_000.0),
        )
    }

    /// The coordinator's rule over B232's sweep: an orphan may be the only
    /// copy of a key whose record a lost list took with it — it becomes a
    /// record, and nothing is deleted.
    #[test]
    fn an_orphaned_private_key_becomes_a_record_and_nothing_is_deleted() {
        let mut s = stores();
        let orphan = account();
        let password = account();
        let by_connection = account();
        let by_key = account();
        let by_carried = account();
        let on_its_way = new_keychain_ref();
        let keychain = MemoryKeychain::with([
            (orphan.clone(), b"KEY lost".to_vec()),
            (password.clone(), b"hunter2".to_vec()),
            (by_connection.clone(), b"pw".to_vec()),
            (by_key.clone(), b"KEY listed".to_vec()),
            (by_carried.clone(), b"KEY newer".to_vec()),
            (on_its_way.clone(), b"KEY record not saved yet".to_vec()),
            ("moshpit.knownHosts".into(), b"{}".to_vec()),
            ("someone.else".into(), b"KEY not ours".to_vec()),
        ]);
        let before = keychain.snapshot();
        s.keys.add(key("laptop", Some(&by_key)));
        s.connections
            .add(connection("pi", None, Some(&by_connection)));
        carry_connection(&mut s, &by_carried, None);

        let recovered = recover(&mut s, &keychain).unwrap();

        assert_eq!(keychain.snapshot(), before, "recovery changed the keychain");
        assert_eq!(recovered.len(), 1);
        let record = &recovered[0];
        assert_eq!(record.keychain_ref.as_deref(), Some(orphan.as_str()));
        assert_eq!(record.name, "Recovered key lost0123");
        assert_eq!(record.fingerprint, "SHA256:lost0123456789");
        assert_eq!(record.public_key, "ssh-ed25519 AAAAlost lost");
        assert_eq!(record.algorithm, SshKeyAlgorithm::Ed25519);
        assert!(!record.require_biometry);
        // Saved: the next launch's list has it.
        let reread = SshKeyStore::new(s.defaults.clone(), Readiness::Always);
        assert!(reread.keys().iter().any(|k| k.id == record.id));
    }

    #[test]
    fn a_second_run_adds_nothing_and_a_deleted_recovered_key_takes_its_secret() {
        let mut s = stores();
        let orphan = account();
        let keychain = MemoryKeychain::with([(orphan.clone(), b"KEY lost".to_vec())]);
        let record = recover(&mut s, &keychain).unwrap().remove(0);
        // The next launch: fresh stores over the same preferences.
        let mut next = Stores {
            connections: ConnectionStore::new(s.defaults.clone(), Readiness::Always),
            keys: SshKeyStore::new(s.defaults.clone(), Readiness::Always),
            defaults: s.defaults.clone(),
        };
        assert_eq!(recover(&mut next, &keychain), Ok(vec![]));
        assert_eq!(next.keys.keys().len(), 1);
        // Deleted in SSH Keys, it goes the B232 way: secret and all.
        assert_eq!(
            delete_key(record.id, &mut next.keys, &mut next.connections, &keychain),
            KeyDeletion::Deleted
        );
        assert!(items(&keychain).is_empty());
        assert_eq!(recover(&mut next, &keychain), Ok(vec![]));
    }

    #[test]
    fn an_item_behind_face_id_is_left_for_later_not_prompted() {
        struct Gated(MemoryKeychain);
        impl Keychain for Gated {
            fn save(&self, a: &str, s: &[u8], b: bool) -> Result<(), KeychainError> {
                self.0.save(a, s, b)
            }
            fn load(&self, _: &str, _: &str) -> Result<Vec<u8>, KeychainError> {
                panic!("asked for Face ID")
            }
            fn delete(&self, a: &str) -> Result<(), KeychainError> {
                self.0.delete(a)
            }
            fn exists(&self, a: &str) -> bool {
                self.0.exists(a)
            }
            fn accounts(&self) -> Result<Vec<String>, KeychainError> {
                self.0.accounts()
            }
            fn load_silently(&self, _: &str) -> Result<Vec<u8>, KeychainError> {
                Err(KeychainError::InteractionNotAllowed)
            }
        }
        let mut s = stores();
        let orphan = account();
        let keychain = Gated(MemoryKeychain::with([(
            orphan.clone(),
            b"KEY gated".to_vec(),
        )]));
        assert_eq!(recover(&mut s, &keychain), Ok(vec![]));
        assert!(keychain.0.exists(&orphan));
        assert!(s.keys.keys().is_empty());
    }

    #[test]
    fn a_launch_before_the_first_unlock_recovers_nothing() {
        let named = account();
        let sealed = Arc::new(MemoryDefaults::new());
        sealed.set_protected_data_available(false);
        let mut s = Stores {
            connections: ConnectionStore::new(sealed.clone(), Readiness::System),
            keys: SshKeyStore::new(sealed.clone(), Readiness::System),
            defaults: sealed,
        };
        let keychain = MemoryKeychain::with([(named.clone(), b"KEY pi".to_vec())]);
        assert_eq!(recover(&mut s, &keychain), Err(RecoveryError::Unknown));
        assert!(s.keys.keys().is_empty());
        assert_eq!(items(&keychain), BTreeSet::from([named]));
    }

    #[test]
    fn a_keychain_that_cant_list_recovers_nothing() {
        struct NoListing(MemoryKeychain);
        impl Keychain for NoListing {
            fn save(&self, a: &str, s: &[u8], b: bool) -> Result<(), KeychainError> {
                self.0.save(a, s, b)
            }
            fn load(&self, a: &str, p: &str) -> Result<Vec<u8>, KeychainError> {
                self.0.load(a, p)
            }
            fn delete(&self, a: &str) -> Result<(), KeychainError> {
                self.0.delete(a)
            }
            fn exists(&self, a: &str) -> bool {
                self.0.exists(a)
            }
        }
        let mut s = stores();
        let orphan = account();
        let keychain = NoListing(MemoryKeychain::with([(orphan.clone(), b"KEY x".to_vec())]));
        assert!(matches!(
            recover(&mut s, &keychain),
            Err(RecoveryError::Keychain(_))
        ));
        assert!(keychain.0.exists(&orphan));
        assert!(s.keys.keys().is_empty());
    }
}
