//! What a platform gives the store: a key-value store of property list
//! values that knows whether it can be believed right now, and a keychain.
//!
//! On iOS the key-value store is `UserDefaults.standard` and the keychain is
//! the app's generic passwords (`KeychainBackend`); on Android they are the
//! app's `SharedPreferences` and the Android Keystore. Nothing here touches a
//! file or a platform API itself.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

use crate::plist::Plist;

/// The app's preferences (`UserDefaults.standard`).
///
/// A value set under a key comes back — until it is set again or removed —
/// as itself, or as the value stored before when that one is equal to it
/// the way Foundation compares (`isEqual`, [`Plist::ns_equal`]):
/// `UserDefaults.set(false, forKey:)` over a stored `0` sometimes keeps the
/// `0`, depending on its caching (measured on macOS 27). Nothing here
/// depends on which of two equal values comes back.
pub trait Defaults: Send + Sync {
    /// `object(forKey:)`.
    fn get(&self, key: &str) -> Option<Plist>;

    /// `set(_:forKey:)`, or `removeObject(forKey:)` for `None`.
    fn set(&self, key: &str, value: Option<Plist>);

    /// Whether the system says protected data is readable right now: on iOS
    /// `UIApplication.isProtectedDataAvailable` (the device is unlocked), on
    /// Android whether the user has unlocked since boot
    /// (`UserManager.isUserUnlocked`). Before that, a launch reads the
    /// preferences as empty — see [`crate::readiness`].
    fn protected_data_available(&self) -> bool;
}

/// Why the keychain did not do what was asked (`KeychainError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeychainError {
    /// No item under that account.
    NotFound,
    Duplicate,
    /// Face ID / the passcode did not authenticate.
    AuthenticationFailed,
    /// The person cancelled the prompt.
    UserCanceled,
    /// The item can't be reached now: the device is locked
    /// (`errSecInteractionNotAllowed`).
    InteractionNotAllowed,
    /// The item's bytes are not what was asked for.
    InvalidData,
    /// Anything else, as the platform describes it.
    Other(String),
}

impl std::fmt::Display for KeychainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("Keychain item not found"),
            Self::Duplicate => f.write_str("Keychain item already exists"),
            Self::AuthenticationFailed => f.write_str("Biometric authentication failed"),
            Self::UserCanceled => f.write_str("User canceled biometric prompt"),
            Self::InteractionNotAllowed => f.write_str("Keychain interaction not allowed"),
            Self::InvalidData => f.write_str("Keychain data was invalid"),
            Self::Other(why) => write!(f, "Keychain error: {why}"),
        }
    }
}

impl std::error::Error for KeychainError {}

/// The keychain (`KeychainBackend`): secrets by account. Private keys and
/// passwords live here, and the known-hosts map; the store itself only ever
/// holds an account name (a "keychain ref").
pub trait Keychain: Send + Sync {
    /// Store `secret` under `account`, in place when an item is there: a
    /// failed write leaves the item as it was (B40). `require_biometry`
    /// gates every later read behind Face ID or the passcode; otherwise the
    /// item is readable while the device is unlocked, on this device only.
    fn save(
        &self,
        account: &str,
        secret: &[u8],
        require_biometry: bool,
    ) -> Result<(), KeychainError>;

    /// The item's bytes. `prompt` is what a biometric prompt says.
    fn load(&self, account: &str, prompt: &str) -> Result<Vec<u8>, KeychainError>;

    /// Remove the item; removing nothing is fine.
    fn delete(&self, account: &str) -> Result<(), KeychainError>;

    /// Whether an item is there, without prompting.
    fn exists(&self, account: &str) -> bool;

    /// The account of every item, without reading one (no prompt, nothing
    /// decrypted): what orphan recovery looks through
    /// ([`crate::secrets::recover_orphans`]). An error when it can't say — a
    /// locked device, a keychain that can't list — never an empty list for
    /// "don't know". A keychain that can't list says so, and nothing is
    /// recovered.
    fn accounts(&self) -> Result<Vec<String>, KeychainError> {
        Err(KeychainError::Other(
            "this keychain can't list its items".to_owned(),
        ))
    }

    /// The item's bytes if they can be had without asking anyone: an item
    /// behind Face ID is [`KeychainError::InteractionNotAllowed`] rather
    /// than a prompt. A keychain that can't tell says so.
    fn load_silently(&self, account: &str) -> Result<Vec<u8>, KeychainError> {
        let _ = account;
        Err(KeychainError::Other(
            "this keychain can't read without asking".to_owned(),
        ))
    }
}

/// Preferences in memory, for tests and previews: readable unless told
/// otherwise.
#[derive(Debug, Default)]
pub struct MemoryDefaults {
    values: Mutex<BTreeMap<String, Plist>>,
    sealed: Mutex<bool>,
}

impl MemoryDefaults {
    pub fn new() -> Self {
        Self::default()
    }

    /// With these values stored.
    pub fn with(values: impl IntoIterator<Item = (String, Plist)>) -> Self {
        Self {
            values: Mutex::new(values.into_iter().collect()),
            sealed: Mutex::new(false),
        }
    }

    /// Whether the system says protected data is unavailable — a launch
    /// before the first unlock. (What a sealed file reads as is up to the
    /// test: an empty one, or one whose contents are already there.)
    pub fn set_protected_data_available(&self, available: bool) {
        *lock(&self.sealed) = !available;
    }

    /// Everything stored, by key.
    pub fn snapshot(&self) -> BTreeMap<String, Plist> {
        lock(&self.values).clone()
    }
}

impl Defaults for MemoryDefaults {
    fn get(&self, key: &str) -> Option<Plist> {
        lock(&self.values).get(key).cloned()
    }

    fn set(&self, key: &str, value: Option<Plist>) {
        let mut values = lock(&self.values);
        match value {
            Some(value) => {
                values.insert(key.to_owned(), value);
            }
            None => {
                values.remove(key);
            }
        }
    }

    fn protected_data_available(&self) -> bool {
        !*lock(&self.sealed)
    }
}

/// A keychain in memory (`InMemoryKeychainBackend`), which can be locked.
#[derive(Debug, Default)]
pub struct MemoryKeychain {
    items: Mutex<BTreeMap<String, Vec<u8>>>,
    locked: Mutex<bool>,
}

impl MemoryKeychain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(items: impl IntoIterator<Item = (String, Vec<u8>)>) -> Self {
        Self {
            items: Mutex::new(items.into_iter().collect()),
            locked: Mutex::new(false),
        }
    }

    /// A locked device: every read and write fails with
    /// [`KeychainError::InteractionNotAllowed`].
    pub fn set_locked(&self, locked: bool) {
        *lock(&self.locked) = locked;
    }

    pub fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        lock(&self.items).clone()
    }
}

impl Keychain for MemoryKeychain {
    fn save(&self, account: &str, secret: &[u8], _: bool) -> Result<(), KeychainError> {
        if *lock(&self.locked) {
            return Err(KeychainError::InteractionNotAllowed);
        }
        lock(&self.items).insert(account.to_owned(), secret.to_vec());
        Ok(())
    }

    fn load(&self, account: &str, _: &str) -> Result<Vec<u8>, KeychainError> {
        if *lock(&self.locked) {
            return Err(KeychainError::InteractionNotAllowed);
        }
        lock(&self.items)
            .get(account)
            .cloned()
            .ok_or(KeychainError::NotFound)
    }

    fn delete(&self, account: &str) -> Result<(), KeychainError> {
        if *lock(&self.locked) {
            return Err(KeychainError::InteractionNotAllowed);
        }
        lock(&self.items).remove(account);
        Ok(())
    }

    fn exists(&self, account: &str) -> bool {
        lock(&self.items).contains_key(account)
    }

    fn accounts(&self) -> Result<Vec<String>, KeychainError> {
        if *lock(&self.locked) {
            return Err(KeychainError::InteractionNotAllowed);
        }
        Ok(lock(&self.items).keys().cloned().collect())
    }

    fn load_silently(&self, account: &str) -> Result<Vec<u8>, KeychainError> {
        self.load(account, "")
    }
}

/// A poisoned lock only means another thread panicked mid-update; every
/// update here is one insert, remove or assignment.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// What every keychain account [`new_keychain_ref`] makes starts with — the
/// only items [`crate::secrets`] ever looks at on its own.
pub const KEYCHAIN_REF_PREFIX: &str = "moshpit.secret.";

/// The accounts [`new_keychain_ref`] made in this process.
static MINTED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// A fresh keychain account for a connection's password or private key
/// (`KeychainService.generateRef`): `moshpit.secret.` and a lower-case UUID.
/// The store keeps only this name; the secret stays in the keychain.
///
/// Remembered for the life of the process ([`minted_this_launch`]): an item
/// is written under a fresh account before the record naming it is saved,
/// so for a moment nothing names it, and orphan recovery must not take it
/// for one.
pub fn new_keychain_ref() -> String {
    let account = format!(
        "{KEYCHAIN_REF_PREFIX}{}",
        moshpit_seal::foundation::uuid::Uuid::new_v4()
            .to_string()
            .to_lowercase()
    );
    lock(&MINTED).insert(account.clone());
    account
}

/// Whether [`new_keychain_ref`] made `account` in this process.
pub fn minted_this_launch(account: &str) -> bool {
    lock(&MINTED).contains(account)
}
