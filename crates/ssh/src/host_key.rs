//! Host keys: their fingerprints, the trusted map, and trust on first use.
//!
//! A port of the Swift transport's `HostKeyValidator` (deleted in E4-SSH).
//! The map is the one the app keeps in the keychain, `"host:port" →
//! "SHA256:…"`, read and written whole through [`KnownHostsStore`] (the
//! platform's keychain); nothing here moves or reformats it, so the builds
//! that ran the Swift transport read the same trusted keys. The map's model — the store trait,
//! its errors, the entry key — and the keychain item's bytes are
//! `moshpit-store`'s (`known_hosts`), re-exported here.
//!
//! The rules carried over (B22, B40):
//! - a map that can't be read is not an empty map: locked means nothing is
//!   decided ([`Decision::Unverifiable`]), broken means the host is asked
//!   about as new but nothing asked then is written over the stored map;
//! - a trust is written on top of the map as read, never on top of a guess,
//!   and one that can't be written yet is kept for this process and written
//!   with the first read that succeeds;
//! - a failed read is never cached.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

use russh::keys::ssh_encoding::base64::{Base64Unpadded, Encoding};
use russh::keys::ssh_key::sha2::{Digest, Sha256};
use russh::keys::{HashAlg, PublicKey};

use crate::waiting::Waiting;

/// A boxed future, for the traits a platform implements.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The key a host presented during key exchange.
#[derive(Debug, Clone)]
pub struct HostKey {
    key: PublicKey,
    /// The older algorithm the key exchange had to use, when it did.
    older: Option<&'static str>,
}

/// The same key, however it was negotiated.
impl PartialEq for HostKey {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for HostKey {}

impl HostKey {
    /// `ssh-rsa` when the server offered nothing better (B564): its
    /// signature over the key exchange is SHA-1. `None` for every other
    /// negotiation, and for a key read from storage.
    pub fn older_algorithm(&self) -> Option<&'static str> {
        self.older
    }

    pub(crate) fn negotiated_as(mut self, older: Option<&'static str>) -> Self {
        self.older = older;
        self
    }

    /// `ssh-ed25519`, `ecdsa-sha2-nistp256`, `ssh-rsa`, …
    pub fn algorithm(&self) -> String {
        self.key.algorithm().as_str().to_owned()
    }

    /// `SHA256:` and the unpadded base64 of the SHA-256 of the key's wire
    /// blob: what `HostKeyValidator.fingerprint(of:)` stores and
    /// `ssh-keygen -l` prints.
    pub fn fingerprint(&self) -> String {
        self.key.fingerprint(HashAlg::Sha256).to_string()
    }

    /// The key as a `known_hosts` line without the host: `ssh-ed25519 AAAA…`.
    pub fn to_openssh(&self) -> String {
        self.key.to_openssh().unwrap_or_default()
    }

    pub fn from_openssh(line: &str) -> Option<HostKey> {
        PublicKey::from_openssh(line.trim()).ok().map(HostKey::from)
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.key
    }
}

impl From<PublicKey> for HostKey {
    fn from(mut key: PublicKey) -> Self {
        // A known_hosts line's comment is not part of the key.
        key.set_comment("");
        Self { key, older: None }
    }
}

/// The fingerprint of any public key blob, in the same form, for blobs
/// `ssh-key` can't parse: `SSHKeyFactory.fingerprint(of:)`.
pub fn fingerprint_of_blob(blob: &[u8]) -> String {
    let digest = Sha256::digest(blob);
    format!("SHA256:{}", Base64Unpadded::encode_string(&digest))
}

pub use moshpit_store::known_hosts::{KnownHostsStore, StoreError, entry as known_hosts_entry};

/// A store in memory, for tests and previews.
#[derive(Debug, Default)]
pub struct MemoryKnownHosts {
    entries: Mutex<BTreeMap<String, String>>,
}

impl MemoryKnownHosts {
    pub fn new(seed: BTreeMap<String, String>) -> Self {
        Self {
            entries: Mutex::new(seed),
        }
    }
}

impl KnownHostsStore for MemoryKnownHosts {
    fn read(&self) -> Result<BTreeMap<String, String>, StoreError> {
        Ok(lock(&self.entries).clone())
    }

    fn write(&self, entries: &BTreeMap<String, String>) -> Result<(), StoreError> {
        *lock(&self.entries) = entries.clone();
        Ok(())
    }
}

/// What the map says about one presented key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The trusted fingerprint: go on.
    Trusted,
    /// Never seen: ask the person.
    Unknown,
    /// Another key was trusted for this host: possibly someone in the
    /// middle. `expected` is the trusted fingerprint.
    Changed { expected: String },
    /// The map can't be read until the device is unlocked. Neither a prompt
    /// nor a pass: "unknown" would offer to trust a host trusted long ago.
    Unverifiable,
}

/// The trusted keys of every host, shared by every connection of the
/// process (`HostKeyValidator`). Decisions and trusts are serialised.
pub struct KnownHosts {
    store: Arc<dyn KnownHostsStore>,
    state: Mutex<TableState>,
}

#[derive(Default)]
struct TableState {
    /// The map as last read, plus every trust written since. `None` until a
    /// read has succeeded.
    table: Option<BTreeMap<String, String>>,
    /// Trusts not in the store yet.
    unsaved: BTreeMap<String, String>,
}

enum TableRead {
    Read(BTreeMap<String, String>),
    Locked,
    Broken,
}

impl KnownHosts {
    /// Reads nothing yet: this can run while the device is locked. The first
    /// decision reads.
    pub fn new(store: Arc<dyn KnownHostsStore>) -> Self {
        Self {
            store,
            state: Mutex::new(TableState::default()),
        }
    }

    /// Over a [`MemoryKnownHosts`] seeded with `seed`.
    pub fn in_memory(seed: BTreeMap<String, String>) -> Self {
        Self::new(Arc::new(MemoryKnownHosts::new(seed)))
    }

    /// The decision for one presented key. Reads the map the first time it
    /// can be read; changes nothing.
    pub fn decide(&self, host: &str, port: u16, fingerprint: &str) -> Decision {
        let key = known_hosts_entry(host, port);
        let mut state = lock(&self.state);
        let stored = match state.unsaved.get(&key) {
            Some(pending) => Some(pending.clone()),
            None => match self.load(&mut state) {
                TableRead::Read(table) => table.get(&key).cloned(),
                TableRead::Locked => return Decision::Unverifiable,
                // Not a lock that lifts: refusing every connection until the
                // keychain heals would strand the person. Ask, as for a new
                // host; nothing asked here is written over the map (see
                // `save`).
                TableRead::Broken => None,
            },
        };
        match stored {
            None => Decision::Unknown,
            Some(stored) if stored == fingerprint => Decision::Trusted,
            Some(stored) => Decision::Changed { expected: stored },
        }
    }

    /// Trust `fingerprint` for the host, after the person accepted a new
    /// host or a changed key.
    pub fn trust(&self, host: &str, port: u16, fingerprint: &str) {
        let mut state = lock(&self.state);
        state
            .unsaved
            .insert(known_hosts_entry(host, port), fingerprint.to_owned());
        self.save(&mut state);
    }

    /// Reads the map if no read has succeeded yet and writes out any trust
    /// still waiting. For the moment protected data becomes available.
    pub fn reload(&self) {
        let mut state = lock(&self.state);
        let _ = self.load(&mut state);
        self.save(&mut state);
    }

    /// The fingerprint trusted for `host` on `port`: the one waiting to be
    /// written, else the map's, read now if no read has succeeded yet. None
    /// for a host never trusted, or while the map can't be read.
    pub fn trusted(&self, host: &str, port: u16) -> Option<String> {
        let key = known_hosts_entry(host, port);
        let mut state = lock(&self.state);
        if let Some(pending) = state.unsaved.get(&key) {
            return Some(pending.clone());
        }
        match self.load(&mut state) {
            TableRead::Read(table) => table.get(&key).cloned(),
            TableRead::Locked | TableRead::Broken => None,
        }
    }

    /// Everything trusted, as this process knows it.
    pub fn snapshot(&self) -> BTreeMap<String, String> {
        let state = lock(&self.state);
        let mut map = state.table.clone().unwrap_or_default();
        map.extend(state.unsaved.clone());
        map
    }

    fn load(&self, state: &mut TableState) -> TableRead {
        if let Some(table) = &state.table {
            return TableRead::Read(table.clone());
        }
        match self.store.read() {
            Ok(table) => {
                state.table = Some(table.clone());
                TableRead::Read(table)
            }
            Err(StoreError::Locked) => {
                log::info!("host keys: the device is locked; not deciding from them");
                TableRead::Locked
            }
            Err(StoreError::Unavailable(why)) => {
                log::info!("host keys: unreadable ({why}); not deciding from them");
                TableRead::Broken
            }
        }
    }

    /// Writes the waiting trusts over the map as stored right now. Nothing
    /// is written until a read has succeeded, so the stored map never
    /// shrinks to what this process happens to know.
    ///
    /// Fix (known-hosts-fresh-merge): read again for the write, not from the
    /// copy decisions are made from. The keychain item had other writers in
    /// the same process — the Swift transport's `HostKeyValidator`, until
    /// E4-SSH — and a trust one of them wrote after this copy was read was
    /// written away by the next trust made here. Any other writer of the
    /// item is kept the same way.
    fn save(&self, state: &mut TableState) {
        if state.unsaved.is_empty() {
            return;
        }
        let mut merged = match self.store.read() {
            Ok(table) => table,
            Err(error) => {
                log::info!("host keys: not written now ({error}); kept for the next try");
                return;
            }
        };
        merged.extend(state.unsaved.clone());
        match self.store.write(&merged) {
            Ok(()) => {
                state.table = Some(merged);
                state.unsaved.clear();
            }
            Err(e) => log::error!("host keys: write failed ({e:?}); kept for the next try"),
        }
    }
}

/// Asks the person about a host key: the "New Host" card and the loud
/// "Key Changed" card.
pub trait HostKeyPrompt: Send + Sync {
    /// A host this device never trusted: show `key.fingerprint()` and ask
    /// whether to trust it.
    fn trust_new_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
    ) -> BoxFuture<'a, bool>;

    /// The host presents a key other than the trusted `expected`. Refused
    /// without asking unless a platform overrides this.
    fn accept_changed_host<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
        expected: &'a str,
    ) -> BoxFuture<'a, bool> {
        let _ = (host, port, key, expected);
        Box::pin(async { false })
    }
}

/// A prompt that says no to everything: what a connection made without a
/// person to ask gets (`SSHService.connect`'s defaults).
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyAll;

impl HostKeyPrompt for DenyAll {
    fn trust_new_host<'a>(&'a self, _: &'a str, _: u16, _: &'a HostKey) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }
}

/// Why a host key was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyRefusal {
    /// A new host the person (or the policy) didn't trust.
    NotTrusted,
    /// Not the trusted key, and the change wasn't accepted.
    Changed { expected: String },
    /// The trusted keys can't be read while the device is locked.
    Locked,
    /// The connection was given up while the decision was pending.
    Abandoned,
}

/// Decides, once per key exchange, whether the key a host presented is the
/// one to trust. Time spent asking a person goes through `waiting`, so it
/// doesn't count against the handshake.
pub trait HostKeyPolicy: Send + Sync {
    fn check<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
        waiting: &'a Waiting,
    ) -> BoxFuture<'a, Result<(), HostKeyRefusal>>;
}

/// Trust on first use against the shared map (`TOFUHostKeyDelegate`): a
/// known key passes, a new host is asked about, a changed key is refused
/// unless the prompt accepts it, a locked map refuses without asking. An
/// accepted key is trusted before sign-in, as before.
pub struct TrustOnFirstUse {
    known: Arc<KnownHosts>,
    prompt: Arc<dyn HostKeyPrompt>,
}

impl TrustOnFirstUse {
    pub fn new(known: Arc<KnownHosts>, prompt: Arc<dyn HostKeyPrompt>) -> Self {
        Self { known, prompt }
    }
}

impl HostKeyPolicy for TrustOnFirstUse {
    fn check<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        key: &'a HostKey,
        waiting: &'a Waiting,
    ) -> BoxFuture<'a, Result<(), HostKeyRefusal>> {
        Box::pin(async move {
            let fingerprint = key.fingerprint();
            match self.known.decide(host, port, &fingerprint) {
                Decision::Trusted => Ok(()),
                Decision::Unknown => {
                    let accepted = waiting
                        .on_person(self.prompt.trust_new_host(host, port, key))
                        .await;
                    if !accepted {
                        return Err(HostKeyRefusal::NotTrusted);
                    }
                    self.known.trust(host, port, &fingerprint);
                    Ok(())
                }
                Decision::Changed { expected } => {
                    let accepted = waiting
                        .on_person(self.prompt.accept_changed_host(host, port, key, &expected))
                        .await;
                    if !accepted {
                        log::warn!("{host}:{port}: host key changed; refused");
                        return Err(HostKeyRefusal::Changed { expected });
                    }
                    self.known.trust(host, port, &fingerprint);
                    Ok(())
                }
                Decision::Unverifiable => Err(HostKeyRefusal::Locked),
            }
        })
    }
}

/// A poisoned lock only means another thread panicked mid-update; the map
/// itself is whole (every update is one insert or one assignment).
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// A keychain that can be locked, broken, or refuse writes, and counts
    /// what is asked of it.
    #[derive(Default)]
    struct Keychain {
        entries: Mutex<BTreeMap<String, String>>,
        locked: Mutex<bool>,
        broken: Mutex<bool>,
        refuse_writes: Mutex<bool>,
        reads: AtomicUsize,
        writes: AtomicUsize,
    }

    impl Keychain {
        fn with(entries: BTreeMap<String, String>) -> Arc<Self> {
            Arc::new(Self {
                entries: Mutex::new(entries),
                ..Default::default()
            })
        }
        fn stored(&self) -> BTreeMap<String, String> {
            self.entries.lock().unwrap().clone()
        }
    }

    impl KnownHostsStore for Keychain {
        fn read(&self) -> Result<BTreeMap<String, String>, StoreError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if *self.locked.lock().unwrap() {
                return Err(StoreError::Locked);
            }
            if *self.broken.lock().unwrap() {
                return Err(StoreError::Unavailable("errSecDecode".into()));
            }
            Ok(self.stored())
        }
        fn write(&self, entries: &BTreeMap<String, String>) -> Result<(), StoreError> {
            if *self.locked.lock().unwrap() || *self.refuse_writes.lock().unwrap() {
                return Err(StoreError::Locked);
            }
            self.writes.fetch_add(1, Ordering::SeqCst);
            *self.entries.lock().unwrap() = entries.clone();
            Ok(())
        }
    }

    #[test]
    fn a_hosts_trusted_fingerprint_is_read_from_the_map_or_the_trust_waiting() {
        let known = KnownHosts::in_memory(BTreeMap::from([(
            "box:22".to_owned(),
            "SHA256:a".to_owned(),
        )]));
        assert_eq!(known.trusted("box", 22).as_deref(), Some("SHA256:a"));
        assert_eq!(
            known.trusted("box", 2222),
            None,
            "another port, another entry"
        );
        assert_eq!(known.trusted("other", 22), None);
        known.trust("other", 22, "SHA256:b");
        assert_eq!(known.trusted("other", 22).as_deref(), Some("SHA256:b"));
    }

    #[test]
    fn decisions_follow_the_map() {
        let known = KnownHosts::in_memory(map(&[("box:22", "SHA256:a")]));
        assert_eq!(known.decide("box", 22, "SHA256:a"), Decision::Trusted);
        assert_eq!(
            known.decide("box", 22, "SHA256:b"),
            Decision::Changed {
                expected: "SHA256:a".into()
            }
        );
        assert_eq!(known.decide("box", 2222, "SHA256:a"), Decision::Unknown);
        assert_eq!(known.decide("Box", 22, "SHA256:a"), Decision::Unknown);
    }

    #[test]
    fn entries_are_keyed_host_colon_port_as_typed() {
        assert_eq!(known_hosts_entry("dev.local", 22), "dev.local:22");
        assert_eq!(known_hosts_entry("::1", 2222), "::1:2222");
        assert_eq!(known_hosts_entry("192.0.2.22", 22), "192.0.2.22:22");
    }

    /// B22: a read that fails while the device is locked is not an empty map.
    #[test]
    fn a_locked_map_decides_nothing_and_is_not_cached() {
        let keychain = Keychain::with(map(&[("box:22", "SHA256:a")]));
        *keychain.locked.lock().unwrap() = true;
        let known = KnownHosts::new(keychain.clone());
        assert_eq!(known.decide("box", 22, "SHA256:a"), Decision::Unverifiable);
        assert_eq!(known.decide("new", 22, "SHA256:z"), Decision::Unverifiable);

        *keychain.locked.lock().unwrap() = false;
        assert_eq!(known.decide("box", 22, "SHA256:a"), Decision::Trusted);
    }

    /// B22's other half: the trust given while the map couldn't be read is
    /// written on top of the map, not instead of it.
    #[test]
    fn a_trust_made_while_locked_lands_on_top_of_the_map_after_unlock() {
        let keychain = Keychain::with(map(&[("a:22", "SHA256:a"), ("b:22", "SHA256:b")]));
        *keychain.locked.lock().unwrap() = true;
        let known = KnownHosts::new(keychain.clone());
        known.trust("c", 22, "SHA256:c");
        assert_eq!(keychain.writes.load(Ordering::SeqCst), 0);
        assert_eq!(known.decide("c", 22, "SHA256:c"), Decision::Trusted);

        *keychain.locked.lock().unwrap() = false;
        known.reload();
        assert_eq!(
            keychain.stored(),
            map(&[
                ("a:22", "SHA256:a"),
                ("b:22", "SHA256:b"),
                ("c:22", "SHA256:c")
            ])
        );
    }

    #[test]
    fn a_broken_map_asks_as_new_but_never_writes_over_the_store() {
        let keychain = Keychain::with(map(&[("a:22", "SHA256:a")]));
        *keychain.broken.lock().unwrap() = true;
        let known = KnownHosts::new(keychain.clone());
        assert_eq!(known.decide("a", 22, "SHA256:a"), Decision::Unknown);
        known.trust("a", 22, "SHA256:a");
        assert_eq!(keychain.writes.load(Ordering::SeqCst), 0);
        assert_eq!(keychain.stored(), map(&[("a:22", "SHA256:a")]));
        assert_eq!(known.decide("a", 22, "SHA256:a"), Decision::Trusted);
    }

    #[test]
    fn a_failed_write_is_kept_for_the_next_try() {
        let keychain = Keychain::with(map(&[("a:22", "SHA256:a")]));
        *keychain.refuse_writes.lock().unwrap() = true;
        let known = KnownHosts::new(keychain.clone());
        known.trust("b", 22, "SHA256:b");
        assert_eq!(keychain.stored(), map(&[("a:22", "SHA256:a")]));

        *keychain.refuse_writes.lock().unwrap() = false;
        known.reload();
        assert_eq!(
            keychain.stored(),
            map(&[("a:22", "SHA256:a"), ("b:22", "SHA256:b")])
        );
        assert_eq!(known.snapshot(), keychain.stored());
    }

    #[test]
    fn the_map_is_read_once_it_has_been_read() {
        let keychain = Keychain::with(map(&[("a:22", "SHA256:a")]));
        let known = KnownHosts::new(keychain.clone());
        for _ in 0..5 {
            assert_eq!(known.decide("a", 22, "SHA256:a"), Decision::Trusted);
        }
        assert_eq!(keychain.reads.load(Ordering::SeqCst), 1);
    }

    /// Two writers of one item (the Swift and the Rust transport in one
    /// process): a trust is written onto the map as stored when it is
    /// written, so neither writes away what the other trusted after its own
    /// read.
    #[test]
    fn a_trust_lands_on_the_map_as_stored_now_not_as_first_read() {
        let keychain = Keychain::with(map(&[("a:22", "SHA256:a")]));
        let rust = KnownHosts::new(keychain.clone());
        let other = KnownHosts::new(keychain.clone());
        assert_eq!(rust.decide("a", 22, "SHA256:a"), Decision::Trusted);
        assert_eq!(other.decide("a", 22, "SHA256:a"), Decision::Trusted);
        other.trust("b", 22, "SHA256:b");
        rust.trust("c", 22, "SHA256:c");
        assert_eq!(
            keychain.stored(),
            map(&[
                ("a:22", "SHA256:a"),
                ("b:22", "SHA256:b"),
                ("c:22", "SHA256:c")
            ])
        );
        // And the copy decisions come from has caught up with the store.
        assert_eq!(rust.decide("b", 22, "SHA256:b"), Decision::Trusted);
    }

    #[test]
    fn a_changed_key_once_accepted_replaces_the_old_one() {
        let keychain = Keychain::with(map(&[("a:22", "SHA256:old")]));
        let known = KnownHosts::new(keychain.clone());
        known.trust("a", 22, "SHA256:new");
        assert_eq!(keychain.stored(), map(&[("a:22", "SHA256:new")]));
        assert_eq!(
            known.decide("a", 22, "SHA256:old"),
            Decision::Changed {
                expected: "SHA256:new".into()
            }
        );
    }

    /// B226, over the keychain item itself: a launch before the first
    /// unlock finds no item at all (`notFound`) where the map is. Without
    /// the unlock probe that read was an empty map, cached as the table:
    /// every trusted host read as new until the process died — a changed
    /// key offered as a new host instead of refused. With it the read is
    /// the lock's, nothing is cached, and after the unlock the hosts are
    /// known and a trust lands on the whole map.
    #[test]
    fn no_item_before_the_first_unlock_decides_nothing_and_loses_nothing() {
        use moshpit_store::known_hosts::{KeychainKnownHosts, encode};
        use moshpit_store::{Keychain as _, KeychainError, MemoryKeychain};
        use std::sync::atomic::AtomicBool;

        struct Sealed {
            inner: MemoryKeychain,
            sealed: AtomicBool,
        }
        impl moshpit_store::Keychain for Sealed {
            fn save(&self, a: &str, s: &[u8], b: bool) -> Result<(), KeychainError> {
                if self.sealed.load(Ordering::SeqCst) {
                    return Err(KeychainError::InteractionNotAllowed);
                }
                self.inner.save(a, s, b)
            }
            fn load(&self, a: &str, p: &str) -> Result<Vec<u8>, KeychainError> {
                if self.sealed.load(Ordering::SeqCst) {
                    return Err(KeychainError::NotFound);
                }
                self.inner.load(a, p)
            }
            fn delete(&self, a: &str) -> Result<(), KeychainError> {
                self.inner.delete(a)
            }
            fn exists(&self, a: &str) -> bool {
                self.inner.exists(a)
            }
        }

        let trusted = map(&[("a:22", "SHA256:a"), ("b:22", "SHA256:b")]);
        let keychain = Arc::new(Sealed {
            inner: MemoryKeychain::with([(
                moshpit_store::known_hosts::ACCOUNT.to_owned(),
                encode(&trusted),
            )]),
            sealed: AtomicBool::new(true),
        });
        let probe = keychain.clone();
        let known = KnownHosts::new(Arc::new(
            KeychainKnownHosts::new(keychain.clone())
                .with_unlock_probe(Arc::new(move || !probe.sealed.load(Ordering::SeqCst))),
        ));
        assert_eq!(known.decide("a", 22, "SHA256:a"), Decision::Unverifiable);
        assert_eq!(
            known.decide("a", 22, "SHA256:mitm"),
            Decision::Unverifiable,
            "a changed key would have been offered as a new host"
        );
        keychain.sealed.store(false, Ordering::SeqCst);
        assert_eq!(known.decide("a", 22, "SHA256:a"), Decision::Trusted);
        known.trust("c", 22, "SHA256:c");
        let stored = moshpit_store::known_hosts::decode(
            &keychain
                .load(moshpit_store::known_hosts::ACCOUNT, "")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            stored,
            map(&[
                ("a:22", "SHA256:a"),
                ("b:22", "SHA256:b"),
                ("c:22", "SHA256:c")
            ])
        );
    }

    #[test]
    fn a_blob_fingerprint_is_unpadded_base64_of_its_sha256() {
        // SHA-256 of nothing, as `ssh-keygen`'s format would print it.
        assert_eq!(
            fingerprint_of_blob(b""),
            "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU"
        );
    }
}
