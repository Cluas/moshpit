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
//!
//! Fix (B428): an item a connection let go of but couldn't drop at once was
//! left to "the sweep" — and the only sweep was recovery, which deletes
//! nothing. A save that moved a connection off its own password or pasted
//! key onto a managed key, while its session still signed in with the old
//! item (B325), held the drop in memory until the session reconnected or
//! ended; an app ended before that left the item for good: a password in
//! the keychain with nothing naming it, a pasted key listed again as a
//! "Recovered key". A drop the keychain refused, or one whose users couldn't
//! be told, ended the same way. Now such an item goes on a list in the
//! preferences ([`hold_release`], [`TO_RELEASE_KEY`]): dropped when its
//! session lets go ([`release`]), and at a later launch once nothing names
//! it ([`settle_releases`]); recovery never offers one as a key. Only what
//! the list recorded is ever deleted this way — and only once no record
//! names it at that moment — never an orphan nobody listed.

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};

use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::uuid::Uuid;

use crate::connections::ConnectionStore;
use crate::host::{Defaults, KEYCHAIN_REF_PREFIX, Keychain, KeychainError, minted_this_launch};
use crate::plist::{self, Plist};
use crate::readiness::Readiness;
use crate::ssh_keys::{SshKeyAlgorithm, SshKeyRecord, SshKeySource, SshKeyStore};

/// Where the items a connection let go of wait until they can be dropped
/// (B428): the keychain accounts, a list of strings in the preferences.
pub const TO_RELEASE_KEY: &str = "moshpit.secrets.toRelease";

/// The accounts put on the list in this process. A session of this launch
/// may still sign in with one (its reconnects read the item it started
/// with, B325), so this launch's [`settle_releases`] leaves them; their own
/// [`release`] drops them.
static HELD: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn held() -> MutexGuard<'static, BTreeSet<String>> {
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a test calls a later launch: `account` no longer held by this
/// process (as if it had ended), the list as stored untouched.
#[cfg(test)]
pub(crate) fn as_if_relaunched(account: &str) {
    held().remove(account);
}

/// The accounts waiting to be dropped, as stored; `None` while the
/// preferences can't be believed (a launch before the first unlock reads
/// the list as empty).
pub fn to_release(defaults: &dyn Defaults, readiness: &Readiness) -> Option<Vec<String>> {
    if !readiness.is_readable(defaults) {
        return None;
    }
    Some(plist::strings_in(defaults.get(TO_RELEASE_KEY).as_ref()).unwrap_or_default())
}

fn write_list(defaults: &dyn Defaults, list: Vec<String>) {
    let value =
        (!list.is_empty()).then(|| Plist::Array(list.into_iter().map(Plist::String).collect()));
    defaults.set(TO_RELEASE_KEY, value);
}

/// Put `account` on the list: a connection let go of it and it can't be
/// dropped now — its session still signs in with it (B325), what else
/// names it can't be told, or the keychain kept it. It goes when its
/// session lets go ([`release`]) or at a later launch once nothing names it
/// ([`settle_releases`]), and is never listed as a recovered key. Only an
/// account the app makes for a secret (`moshpit.secret.…`) goes on it, and
/// only while the preferences can be believed. Whether it is on the list.
pub fn hold_release(defaults: &dyn Defaults, readiness: &Readiness, account: &str) -> bool {
    if !account.starts_with(KEYCHAIN_REF_PREFIX) {
        return false;
    }
    held().insert(account.to_owned());
    let Some(mut list) = to_release(defaults, readiness) else {
        return false;
    };
    if !list.iter().any(|listed| listed == account) {
        list.push(account.to_owned());
        write_list(defaults, list);
    }
    true
}

/// Take `account` off the list (and out of this process's holds).
fn unlist(defaults: &dyn Defaults, readiness: &Readiness, account: &str) {
    held().remove(account);
    if let Some(list) = to_release(defaults, readiness)
        && list.iter().any(|listed| listed == account)
    {
        write_list(
            defaults,
            list.into_iter()
                .filter(|listed| listed != account)
                .collect(),
        );
    }
}

/// Drop `account` now if nothing names it — `referenced`: every account a
/// saved record names ([`referenced`]), `None` when that can't be told —
/// and keep it listed until it is dropped: what a connection that let go of
/// its item does once its session no longer reads it (B325, B428).
///
/// - nothing names it: deleted, and off the list ([`ItemFate::Deleted`]);
/// - something names it again: kept, and off the list — the record naming
///   it has it now ([`ItemFate::Kept`]);
/// - the keychain keeps it: listed, tried again at a later launch
///   ([`ItemFate::Failed`]);
/// - unknown: listed, nothing deleted ([`ItemFate::Unknown`]).
pub fn release(
    defaults: &dyn Defaults,
    readiness: &Readiness,
    account: &str,
    referenced: Option<&BTreeSet<String>>,
    keychain: &dyn Keychain,
) -> ItemFate {
    let Some(named) = referenced else {
        hold_release(defaults, readiness, account);
        return ItemFate::Unknown;
    };
    if named.contains(account) {
        unlist(defaults, readiness, account);
        return ItemFate::Kept;
    }
    // "Not found" is gone already: a keychain that says so for an item it
    // doesn't have has nothing left to drop.
    match keychain.delete(account) {
        Ok(()) | Err(KeychainError::NotFound) => {
            unlist(defaults, readiness, account);
            ItemFate::Deleted
        }
        Err(error) => {
            log::error!("keychain: it kept an item nothing names ({error}); listed for later");
            hold_release(defaults, readiness, account);
            ItemFate::Failed(error)
        }
    }
}

/// What a later launch does with the list once the saved records can be
/// told (`referenced`, [`referenced`]'s): each account on it that nothing
/// names now is deleted and comes off it; one something names again stays
/// in the keychain and comes off it; one the keychain keeps stays on it.
/// Accounts listed in this process are left (a session of this launch may
/// still read one), and so is everything when the records can't be told.
/// An entry that isn't an account the app makes comes off untouched. What
/// became of each account looked at.
pub fn settle_releases(
    defaults: &dyn Defaults,
    readiness: &Readiness,
    referenced: Option<&BTreeSet<String>>,
    keychain: &dyn Keychain,
) -> Vec<(String, ItemFate)> {
    let (Some(list), Some(named)) = (to_release(defaults, readiness), referenced) else {
        return Vec::new();
    };
    let mut settled = Vec::new();
    for account in list {
        if held().contains(&account) {
            continue;
        }
        let fate = if !account.starts_with(KEYCHAIN_REF_PREFIX) || named.contains(&account) {
            ItemFate::Kept
        } else {
            match keychain.delete(&account) {
                Ok(()) | Err(KeychainError::NotFound) => ItemFate::Deleted,
                Err(error) => {
                    log::error!("keychain: a listed item stays ({error}); tried again later");
                    ItemFate::Failed(error)
                }
            }
        };
        if !matches!(fate, ItemFate::Failed(_)) {
            unlist(defaults, readiness, &account);
        }
        settled.push((account, fate));
    }
    settled
}

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
    /// Kept: what else names it can't be told now. Listed: a later launch
    /// drops it once nothing names it (B428).
    Unknown,
    /// The keychain wouldn't delete it. Listed: a later launch tries again
    /// (B428).
    Failed(KeychainError),
}

/// A saved connection's own item it no longer names (it signs in with a
/// managed key now, B325): dropped once nothing else names it, by
/// [`delete_connection`]'s rule, and listed until it is ([`release`]). Call
/// it after the connection is saved, and once its session no longer reads
/// the item ([`hold_release`] until then).
pub fn release_item(
    account: &str,
    connections: &mut ConnectionStore,
    keys: &mut SshKeyStore,
    keychain: &dyn Keychain,
) -> ItemFate {
    connections.reload_if_needed();
    keys.reload_if_needed();
    let named = referenced(connections, keys, None, None);
    let (defaults, readiness) = connections.preferences();
    release(defaults, readiness, account, named.as_ref(), keychain)
}

/// The list's items, at a launch once both stores can be trusted
/// ([`settle_releases`] over what they name now).
pub fn release_pending(
    connections: &mut ConnectionStore,
    keys: &mut SshKeyStore,
    keychain: &dyn Keychain,
) -> Vec<(String, ItemFate)> {
    connections.reload_if_needed();
    keys.reload_if_needed();
    let named = referenced(connections, keys, None, None);
    let (defaults, readiness) = connections.preferences();
    settle_releases(defaults, readiness, named.as_ref(), keychain)
}

/// Delete connection `id`, and its keychain item with it once no other
/// connection and no key record names it — a managed key's item stays while
/// the key is listed; one whose key's record is already gone goes with the
/// last connection on it. The record goes whatever becomes of the item: an
/// item left behind is listed for a later launch ([`release`], B428).
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
        Some(account) => {
            let named = referenced(connections, keys, Some(id), None);
            let (defaults, readiness) = connections.preferences();
            release(defaults, readiness, &account, named.as_ref(), keychain)
        }
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
/// [`is_ours`], no saved connection and no key record names it, and no
/// connection let go of it (on the list [`hold_release`] keeps, B428:
/// those are to be dropped, not offered back) — and return
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
    // What a connection let go of is to be dropped, not offered back as a
    // key (B428).
    let (defaults, readiness) = connections.preferences();
    let to_release = to_release(defaults, readiness).ok_or(RecoveryError::Unknown)?;
    let mut accounts = keychain.accounts().map_err(RecoveryError::Keychain)?;
    accounts.sort();
    let mut recovered = Vec::new();
    for account in accounts {
        if !is_ours(&account) || to_release.contains(&account) || held().contains(&account) {
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

/// What each key 1.0.8 imported without its public half is (B726): every
/// record that [lacks its facts](SshKeyRecord::lacks_facts) has its private
/// key read without asking ([`Keychain::load_silently`]) and, if `describe`
/// reads it as a private key, those facts kept
/// ([`SshKeyStore::learn_facts`]); one of a kind no connect path signs
/// with (`refused`), that kind ([`SshKeyStore::mark_refused`], B780). One
/// behind Face ID is left for its first
/// sign-in, which reads it anyway; one that reads as nothing is left as it
/// is. Returns the records changed. A record with its facts is never read,
/// so a later launch reads nothing for one this put right. The app runs it
/// once a launch, beside [`recover_orphans`].
pub fn learn_key_facts(
    keys: &mut SshKeyStore,
    keychain: &dyn Keychain,
    describe: &dyn Fn(&[u8]) -> Option<KeyFacts>,
    refused: &dyn Fn(&[u8]) -> Option<String>,
) -> Vec<SshKeyRecord> {
    keys.reload_if_needed();
    let lacking: Vec<(Uuid, String)> = keys
        .keys()
        .iter()
        .filter(|k| k.lacks_facts())
        .filter_map(|k| Some((k.id, k.keychain_ref.clone()?)))
        .collect();
    let mut learned = Vec::new();
    for (id, account) in lacking {
        let secret = match keychain.load_silently(&account) {
            Ok(secret) => secret,
            Err(error) => {
                log::info!(
                    "key facts: a key can't be read without asking ({error}); left for its first sign-in"
                );
                continue;
            }
        };
        let written = match describe(&secret) {
            Some(facts) => keys.learn_facts(id, &facts),
            // A kind no connect path signs with (B780): kept, so the key
            // list and the picker say so.
            None => match refused(&secret) {
                Some(kind) => keys.mark_refused(id, &kind),
                None => {
                    log::info!(
                        "key facts: a key's item doesn't read as a private key; left as it is"
                    );
                    continue;
                }
            },
        };
        if written && let Some(record) = keys.keys().iter().find(|k| k.id == id) {
            learned.push(record.clone());
        }
    }
    if !learned.is_empty() {
        log::info!(
            "key facts: {} key(s) an earlier build imported now show what they are",
            learned.len()
        );
    }
    learned
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

    // --- B428: what a connection let go of, listed until it is dropped ----

    /// The list as stored.
    fn listed(s: &Stores) -> Vec<String> {
        to_release(s.defaults.as_ref(), &Readiness::Always).unwrap()
    }

    /// Connection `c`, saved, now signs in with managed key `key`: the item
    /// it held for itself is named by nothing (B325).
    fn moved_onto(s: &mut Stores, c: &ServerConnection, key: &SshKeyRecord) {
        let mut moved = c.clone();
        moved.auth_method = AuthMethod::Key;
        moved.keychain_ref = key.keychain_ref.clone();
        moved.ssh_key_id = Some(key.id);
        s.connections.update(moved);
    }

    /// B428: a save let go of a connection's own pasted key while its
    /// session still signed in with it; the app ended before the session
    /// did. At the next launch the item is dropped — not listed in SSH Keys
    /// as a "Recovered key".
    #[test]
    fn an_item_let_go_of_while_its_session_ran_is_dropped_at_the_next_launch_not_recovered() {
        let mut s = stores();
        let (pasted, managed) = (account(), account());
        let keychain = MemoryKeychain::with([
            (pasted.clone(), b"KEY pasted".to_vec()),
            (managed.clone(), b"KEY managed".to_vec()),
        ]);
        let laptop = key("laptop", Some(&managed));
        s.keys.add(laptop.clone());
        let pi = connection("pi", None, Some(&pasted));
        s.connections.add(pi.clone());
        moved_onto(&mut s, &pi, &laptop);
        // Its session still reads the item: held, not dropped.
        let (defaults, readiness) = s.connections.preferences();
        assert!(hold_release(defaults, readiness, &pasted));
        assert_eq!(listed(&s), std::slice::from_ref(&pasted));
        assert!(keychain.exists(&pasted));

        // The app ends; the next launch.
        as_if_relaunched(&pasted);
        let mut next = stores_over(&s);
        let recovered = recover(&mut next, &keychain).unwrap();
        assert!(recovered.is_empty(), "offered back as a key: {recovered:?}");
        let settled = release_pending(&mut next.connections, &mut next.keys, &keychain);
        assert_eq!(settled, [(pasted.clone(), ItemFate::Deleted)]);
        assert!(!keychain.exists(&pasted));
        assert!(keychain.exists(&managed));
        assert!(listed(&next).is_empty());
    }

    /// Stores over `s`'s preferences, read afresh: the next launch's.
    fn stores_over(s: &Stores) -> Stores {
        Stores {
            connections: ConnectionStore::new(s.defaults.clone(), Readiness::Always),
            keys: SshKeyStore::new(s.defaults.clone(), Readiness::Always),
            defaults: s.defaults.clone(),
        }
    }

    /// While this launch's session may still read it, this launch's
    /// settling leaves a held item alone; the session letting go drops it.
    #[test]
    fn a_held_item_waits_for_its_session_not_for_this_launchs_settling() {
        let mut s = stores();
        let pasted = account();
        let keychain = MemoryKeychain::with([(pasted.clone(), b"KEY pasted".to_vec())]);
        let (defaults, readiness) = s.connections.preferences();
        assert!(hold_release(defaults, readiness, &pasted));
        assert_eq!(
            release_pending(&mut s.connections, &mut s.keys, &keychain),
            []
        );
        assert!(keychain.exists(&pasted));
        // The session reconnected as saved, or ended.
        assert_eq!(
            release_item(&pasted, &mut s.connections, &mut s.keys, &keychain),
            ItemFate::Deleted
        );
        assert!(!keychain.exists(&pasted));
        assert!(listed(&s).is_empty());
    }

    /// Deleting can't be undone: a listed item a record names again — a
    /// connection or a key picked it up since — stays in the keychain, and
    /// comes off the list (that record has it now).
    #[test]
    fn a_listed_item_something_names_again_is_kept() {
        let mut s = stores();
        let (by_connection, by_key) = (account(), account());
        let keychain = MemoryKeychain::with([
            (by_connection.clone(), b"pw".to_vec()),
            (by_key.clone(), b"KEY listed".to_vec()),
        ]);
        // An earlier launch listed both.
        s.defaults.set(
            TO_RELEASE_KEY,
            Some(Plist::Array(vec![
                Plist::String(by_connection.clone()),
                Plist::String(by_key.clone()),
            ])),
        );
        s.connections
            .add(connection("pi", None, Some(&by_connection)));
        s.keys.add(key("laptop", Some(&by_key)));
        let settled = release_pending(&mut s.connections, &mut s.keys, &keychain);
        assert_eq!(
            settled,
            [
                (by_connection.clone(), ItemFate::Kept),
                (by_key.clone(), ItemFate::Kept)
            ]
        );
        assert!(keychain.exists(&by_connection));
        assert!(keychain.exists(&by_key));
        assert!(listed(&s).is_empty());
        // Named again at the moment of the release, too.
        let (defaults, readiness) = s.connections.preferences();
        assert!(hold_release(defaults, readiness, &by_key));
        assert_eq!(
            release_item(&by_key, &mut s.connections, &mut s.keys, &keychain),
            ItemFate::Kept
        );
        assert!(keychain.exists(&by_key));
        assert!(listed(&s).is_empty());
    }

    /// A drop the keychain refuses is listed and tried again at a later
    /// launch; so is one whose users can't be told — and unknown deletes
    /// nothing.
    #[test]
    fn a_refused_or_unknown_drop_is_listed_and_tried_again() {
        let mut s = stores();
        let (refused, unknown) = (account(), account());
        let keychain = MemoryKeychain::with([
            (refused.clone(), b"pw".to_vec()),
            (unknown.clone(), b"pw".to_vec()),
        ]);
        keychain.set_locked(true);
        let (defaults, readiness) = s.connections.preferences();
        assert!(matches!(
            release(
                defaults,
                readiness,
                &refused,
                Some(&BTreeSet::new()),
                &keychain
            ),
            ItemFate::Failed(_)
        ));
        assert_eq!(
            release(defaults, readiness, &unknown, None, &keychain),
            ItemFate::Unknown
        );
        assert_eq!(listed(&s), [refused.clone(), unknown.clone()]);
        keychain.set_locked(false);
        assert!(keychain.exists(&refused) && keychain.exists(&unknown));
        // Unknown at the next launch too: nothing goes.
        as_if_relaunched(&refused);
        as_if_relaunched(&unknown);
        let (defaults, readiness) = s.connections.preferences();
        assert_eq!(settle_releases(defaults, readiness, None, &keychain), []);
        assert!(keychain.exists(&refused) && keychain.exists(&unknown));
        // Told: both go.
        let settled = release_pending(&mut s.connections, &mut s.keys, &keychain);
        assert_eq!(
            settled,
            [
                (refused.clone(), ItemFate::Deleted),
                (unknown.clone(), ItemFate::Deleted)
            ]
        );
        assert!(keychain.snapshot().is_empty());
        assert!(listed(&s).is_empty());
    }

    /// A deleted connection whose item the keychain wouldn't drop: the
    /// record goes, the item is listed for a later launch.
    #[test]
    fn a_deleted_connections_item_the_keychain_keeps_is_listed() {
        let mut s = stores();
        let own = account();
        let keychain = MemoryKeychain::with([(own.clone(), b"pw".to_vec())]);
        let pi = connection("pi", None, Some(&own));
        s.connections.add(pi.clone());
        keychain.set_locked(true);
        assert!(matches!(
            delete_connection(pi.id, &mut s.connections, &mut s.keys, &keychain),
            ItemFate::Failed(_)
        ));
        assert!(s.connections.connections().is_empty());
        assert_eq!(listed(&s), std::slice::from_ref(&own));
        keychain.set_locked(false);
        as_if_relaunched(&own);
        let mut next = stores_over(&s);
        assert_eq!(
            release_pending(&mut next.connections, &mut next.keys, &keychain),
            [(own.clone(), ItemFate::Deleted)]
        );
        assert!(!keychain.exists(&own));
    }

    /// A keychain that answers "not found" for a listed item has nothing
    /// left to drop: it comes off the list rather than being tried at every
    /// launch.
    #[test]
    fn a_listed_item_the_keychain_no_longer_has_comes_off_the_list() {
        struct Absent;
        impl Keychain for Absent {
            fn save(&self, _: &str, _: &[u8], _: bool) -> Result<(), KeychainError> {
                Ok(())
            }
            fn load(&self, _: &str, _: &str) -> Result<Vec<u8>, KeychainError> {
                Err(KeychainError::NotFound)
            }
            fn delete(&self, _: &str) -> Result<(), KeychainError> {
                Err(KeychainError::NotFound)
            }
            fn exists(&self, _: &str) -> bool {
                false
            }
        }
        let mut s = stores();
        let gone = account();
        s.defaults.set(
            TO_RELEASE_KEY,
            Some(Plist::Array(vec![Plist::String(gone.clone())])),
        );
        assert_eq!(
            release_pending(&mut s.connections, &mut s.keys, &Absent),
            [(gone.clone(), ItemFate::Deleted)]
        );
        assert!(listed(&s).is_empty());
    }

    /// Only an account the app makes for a secret is ever listed or dropped
    /// this way: never the trusted host keys, never another app's item;
    /// and nothing is listed while the preferences can't be believed.
    #[test]
    fn only_the_apps_own_accounts_are_listed_and_only_while_readable() {
        let s = stores();
        let keychain = MemoryKeychain::with([
            ("moshpit.knownHosts".into(), b"{}".to_vec()),
            ("someone.else".into(), b"pw".to_vec()),
        ]);
        let (defaults, readiness) = s.connections.preferences();
        assert!(!hold_release(defaults, readiness, "moshpit.knownHosts"));
        assert!(listed(&s).is_empty());
        // Listed by something else: taken off, never deleted.
        s.defaults.set(
            TO_RELEASE_KEY,
            Some(Plist::Array(vec![
                Plist::String("moshpit.knownHosts".into()),
                Plist::String("someone.else".into()),
            ])),
        );
        let settled = settle_releases(defaults, readiness, Some(&BTreeSet::new()), &keychain);
        assert_eq!(settled.len(), 2);
        assert!(settled.iter().all(|(_, fate)| *fate == ItemFate::Kept));
        assert_eq!(keychain.snapshot().len(), 2);
        assert!(listed(&s).is_empty());
        // Sealed: not readable, nothing written.
        let sealed = MemoryDefaults::new();
        sealed.set_protected_data_available(false);
        let system = Readiness::System;
        assert_eq!(to_release(&sealed, &system), None);
        assert!(!hold_release(&sealed, &system, &account()));
        assert_eq!(sealed.get(TO_RELEASE_KEY), None);
    }
}
