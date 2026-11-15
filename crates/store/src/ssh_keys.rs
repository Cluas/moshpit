//! SSH key metadata (`SSHKeyRecord`, `SSHKeyStore`): one JSON array under
//! `moshpit.sshkeys.v1`, dates as seconds since 2001.
//!
//! The private half is never here: it lives in the keychain (or inside the
//! Secure Enclave, or the Android Keystore), and a record holds only its
//! account name (`keychainRef`).
//!
//! Two of round six's rules shape a record:
//!
//! - B32: `algorithm` is the field every build reads, so it always holds a
//!   value every build decodes — "ECDSA-sk" for a plain ECDSA key, which is
//!   what builds up to 1.0.8 labelled every ECDSA key — and the real
//!   algorithm goes in `keyAlgorithm`. Those builds decode the list in one
//!   piece and took a record they couldn't decode as no keys at all, then
//!   saved that empty list over every key.
//! - B34: an RSA key's size goes in `bits` (beside the "RSA-4096" every
//!   build reads); a record without it gets its size off its public key.

use std::collections::BTreeSet;
use std::sync::Arc;

use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::json::{Number, Object, Value};
use moshpit_seal::foundation::uuid::Uuid;
use moshpit_seal::foundation::{base64, double, text};
use serde::de::DeserializeOwned;

use crate::codec;
use crate::host::Defaults;
use crate::plist::Plist;
use crate::readiness::Readiness;
use crate::secrets::KeyFacts;
use crate::stored_list::{self, Contents, Entry};
use crate::swift;

/// Where a key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SshKeySource {
    SecureEnclave,
    Generated,
    Imported,
    Hardware,
}

impl SshKeySource {
    pub const ALL: [Self; 4] = [
        Self::SecureEnclave,
        Self::Generated,
        Self::Imported,
        Self::Hardware,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::SecureEnclave => "secureEnclave",
            Self::Generated => "generated",
            Self::Imported => "imported",
            Self::Hardware => "hardware",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.raw_value() == raw)
    }
}

/// A key's algorithm, by the names the app stores (`SSHKeyAlgorithm`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SshKeyAlgorithm {
    Ed25519,
    /// A security key, and — in builds up to 1.0.8 — any ECDSA key.
    EcdsaSk,
    /// Any RSA key, whatever its size; the size is in `bits`.
    Rsa4096,
    /// A Secure Enclave P-256 key.
    SeP256,
    /// An imported software ECDSA key. No released build decodes it, so a
    /// record stores it only in `keyAlgorithm`.
    Ecdsa,
}

impl SshKeyAlgorithm {
    pub const ALL: [Self; 5] = [
        Self::Ed25519,
        Self::EcdsaSk,
        Self::Rsa4096,
        Self::SeP256,
        Self::Ecdsa,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Ed25519 => "ED25519",
            Self::EcdsaSk => "ECDSA-sk",
            Self::Rsa4096 => "RSA-4096",
            Self::SeP256 => "SE · P256",
            Self::Ecdsa => "ECDSA",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.raw_value() == raw)
    }

    /// What a record says in `algorithm`: a value every build decodes.
    pub fn older_builds_name(self) -> Self {
        if self == Self::Ecdsa {
            Self::EcdsaSk
        } else {
            self
        }
    }
}

/// Public metadata for one key.
#[derive(Debug, Clone, PartialEq)]
pub struct SshKeyRecord {
    pub id: Uuid,
    pub name: String,
    /// The real algorithm, whatever `algorithm` says in storage.
    pub algorithm: SshKeyAlgorithm,
    /// An RSA key's modulus size; `None` when not known (or not RSA).
    pub bits: Option<i64>,
    pub comment: String,
    /// `SHA256:…` of the public key blob.
    pub fingerprint: String,
    /// The full `authorized_keys` line.
    pub public_key: String,
    pub source: SshKeySource,
    pub bound_hosts: Vec<String>,
    /// The keychain account of the private half.
    pub keychain_ref: Option<String>,
    pub require_biometry: bool,
    pub created_at: Date,
    pub last_used_at: Option<Date>,
    /// The kind of key it is when no connect path signs in with it
    /// (`"ECDSA brainpoolP256r1"`, `"DSA"`), read off its private key
    /// (B780): one 1.0.8 imported without its public half, of a kind 1.0.9
    /// can't sign with. `None`: it signs in, or it hasn't been read.
    pub refused: Option<String>,
}

impl SshKeyRecord {
    /// A record with the initializer's defaults: no comment, fingerprint or
    /// public key, no bound hosts, biometry required.
    pub fn new(
        id: Uuid,
        name: &str,
        algorithm: SshKeyAlgorithm,
        source: SshKeySource,
        created_at: Date,
    ) -> Self {
        Self {
            id,
            name: name.to_owned(),
            algorithm,
            bits: None,
            comment: String::new(),
            fingerprint: String::new(),
            public_key: String::new(),
            source,
            bound_hosts: Vec::new(),
            keychain_ref: None,
            require_biometry: true,
            created_at,
            last_used_at: None,
            refused: None,
        }
    }

    /// The badge: `SE · P256`, `RSA-2048` (`RSA` when the size isn't
    /// known), or the algorithm's name.
    pub fn badge_text(&self) -> String {
        if self.source == SshKeySource::SecureEnclave {
            let name = if self.algorithm == SshKeyAlgorithm::SeP256 {
                "P256"
            } else {
                self.algorithm.raw_value()
            };
            return format!("SE · {name}");
        }
        if self.algorithm == SshKeyAlgorithm::Rsa4096 {
            return self
                .bits
                .map_or_else(|| "RSA".to_owned(), |bits| format!("RSA-{bits}"));
        }
        self.algorithm.raw_value().to_owned()
    }

    /// The algorithm of a record with no (readable) `keyAlgorithm`: one an
    /// older build saved. Its ECDSA-sk covers every ECDSA key; the public
    /// key line says which are plain ECDSA, and so is one with no line at
    /// all (B726): a SEC1 EC key 1.0.8 imported without its public half.
    /// 1.0.8 never labelled a security key ECDSA-sk (it took one's public
    /// line for ED25519); a hardware key an older build saved back keeps
    /// its `sk-` line, and its label.
    fn recovered_algorithm(
        older_builds_name: SshKeyAlgorithm,
        public_key: &str,
        source: SshKeySource,
    ) -> SshKeyAlgorithm {
        if older_builds_name == SshKeyAlgorithm::EcdsaSk
            && source != SshKeySource::Hardware
            && (public_key.is_empty() || swift::has_prefix(public_key, "ecdsa-sha2-"))
        {
            SshKeyAlgorithm::Ecdsa
        } else {
            older_builds_name
        }
    }

    /// Whether what the key is has to come off its private key (B726): a
    /// record with no public key — one 1.0.8 imported pasted without its
    /// public half, labelled by a guess at the PEM's first line — whose
    /// private key is a keychain item the app can read.
    /// One read already found it a kind no connect path signs with (B780)
    /// has nothing more to learn.
    pub fn lacks_facts(&self) -> bool {
        self.public_key.is_empty()
            && self.refused.is_none()
            && self.keychain_ref.is_some()
            && matches!(
                self.source,
                SshKeySource::Imported | SshKeySource::Generated
            )
    }

    /// The record as the app's `encode(to:)` writes it.
    pub fn to_json(&self) -> Value {
        let mut fields = Object::new();
        let string = |text: &str| Value::String(text.to_owned());
        let date = |date: Date| {
            Value::Number(Number::from_literal(double::encoder(
                date.since_reference(),
            )))
        };
        fields.insert("id", string(&self.id.to_string()));
        fields.insert("name", string(&self.name));
        fields.insert(
            "algorithm",
            string(self.algorithm.older_builds_name().raw_value()),
        );
        fields.insert("keyAlgorithm", string(self.algorithm.raw_value()));
        if let Some(bits) = self.bits {
            fields.insert(
                "bits",
                Value::Number(Number::from_literal(bits.to_string())),
            );
        }
        fields.insert("comment", string(&self.comment));
        fields.insert("fingerprint", string(&self.fingerprint));
        fields.insert("publicKey", string(&self.public_key));
        fields.insert("source", string(self.source.raw_value()));
        fields.insert(
            "boundHosts",
            Value::Array(self.bound_hosts.iter().map(|h| string(h)).collect()),
        );
        if let Some(reference) = &self.keychain_ref {
            fields.insert("keychainRef", string(reference));
        }
        fields.insert("requireBiometry", Value::Bool(self.require_biometry));
        fields.insert("createdAt", date(self.created_at));
        if let Some(used) = self.last_used_at {
            fields.insert("lastUsedAt", date(used));
        }
        if let Some(kind) = &self.refused {
            fields.insert("refused", string(kind));
        }
        Value::Object(fields)
    }

    /// The record as the app's `init(from:)` reads it.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let fields = value
            .as_object()
            .ok_or_else(|| "expected an object".to_owned())?;
        let id: Uuid = required(fields, "id")?;
        let name: String = required(fields, "name")?;
        let comment: String = required(fields, "comment")?;
        let fingerprint: String = required(fields, "fingerprint")?;
        let public_key: String = required(fields, "publicKey")?;
        let source = required::<String>(fields, "source").and_then(|raw| {
            SshKeySource::from_raw(&raw).ok_or_else(|| format!("no SSHKeySource {raw:?}"))
        })?;
        let bound_hosts: Vec<String> = required(fields, "boundHosts")?;
        let keychain_ref: Option<String> = optional(fields, "keychainRef")?;
        let require_biometry: bool = required(fields, "requireBiometry")?;
        let created_at: Date = required(fields, "createdAt")?;
        let last_used_at: Option<Date> = optional(fields, "lastUsedAt")?;
        let refused = fields
            .get("refused")
            .and_then(Value::as_str)
            .map(str::to_owned);
        // Unreadable here means unreadable everywhere: carried, not guessed.
        let older_builds_name = required::<String>(fields, "algorithm").and_then(|raw| {
            SshKeyAlgorithm::from_raw(&raw).ok_or_else(|| format!("no SSHKeyAlgorithm {raw:?}"))
        })?;
        // A real algorithm from a newer build may be one this build has no
        // name for; then it reads what older builds read.
        let real = fields
            .get("keyAlgorithm")
            .and_then(Value::as_str)
            .and_then(SshKeyAlgorithm::from_raw);
        let algorithm = real
            .unwrap_or_else(|| Self::recovered_algorithm(older_builds_name, &public_key, source));
        // A record from before the size was kept (or saved back by an older
        // build): the public key line still says.
        let bits = match fields.get("bits") {
            Some(Value::Number(number)) => number.decoder_int(),
            _ => None,
        }
        .or_else(|| {
            (algorithm == SshKeyAlgorithm::Rsa4096)
                .then(|| rsa_bits_of_public_key_line(&public_key))
                .flatten()
        });
        Ok(Self {
            id,
            name,
            algorithm,
            bits,
            comment,
            fingerprint,
            public_key,
            source,
            bound_hosts,
            keychain_ref,
            require_biometry,
            created_at,
            last_used_at,
            refused,
        })
    }
}

fn required<T: DeserializeOwned>(fields: &Object, key: &str) -> Result<T, String> {
    let value = fields
        .get(key)
        .ok_or_else(|| format!("no value for key {key:?}"))?;
    codec::decode(value).map_err(|error| format!("{key}: {error}"))
}

/// `decodeIfPresent`: absent or `null` is `None`, anything else must decode.
fn optional<T: DeserializeOwned>(fields: &Object, key: &str) -> Result<Option<T>, String> {
    match fields.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => codec::decode(value)
            .map(Some)
            .map_err(|error| format!("{key}: {error}")),
    }
}

impl Entry for SshKeyRecord {
    fn decode(value: &Value) -> Result<Self, String> {
        Self::from_json(value)
    }

    fn encode_list(items: &[Self]) -> String {
        codec::write(&Value::Array(items.iter().map(Self::to_json).collect()))
    }
}

/// The modulus size of the `ssh-rsa` key on an `authorized_keys` line;
/// `None` for any other line (`SSHKeyFactory.rsaBits(ofPublicKeyLine:)`).
pub fn rsa_bits_of_public_key_line(line: &str) -> Option<i64> {
    let fields = text::split_characters(line, |c| c == " ");
    if fields.len() < 2 || !swift::eq(fields[0], "ssh-rsa") {
        return None;
    }
    rsa_bits_of_public_blob(&base64::decode(fields[1])?)
}

/// The modulus size of an `ssh-rsa` public key blob (`string "ssh-rsa"`,
/// `mpint e`, `mpint n`), as `ssh-keygen -l` reports it.
pub fn rsa_bits_of_public_blob(blob: &[u8]) -> Option<i64> {
    let mut fields: Vec<&[u8]> = Vec::new();
    let mut rest = blob;
    while fields.len() < 3 && rest.len() >= 4 {
        let length = u32::from_be_bytes(rest[..4].try_into().expect("four bytes")) as usize;
        rest = &rest[4..];
        if rest.len() < length {
            return None;
        }
        fields.push(&rest[..length]);
        rest = &rest[length..];
    }
    if fields.len() != 3 || fields[0] != b"ssh-rsa" {
        return None;
    }
    let modulus: Vec<u8> = fields[2].iter().copied().skip_while(|&b| b == 0).collect();
    let top = *modulus.first()?;
    Some((modulus.len() as i64 - 1) * 8 + (8 - i64::from(top.leading_zeros())))
}

/// The SSH key records (`SSHKeyStore`).
pub struct SshKeyStore {
    defaults: Arc<dyn Defaults>,
    readiness: Readiness,
    keys: Vec<SshKeyRecord>,
    is_authoritative: bool,
    /// Records this build could not decode, written back as stored: dropping
    /// one orphaned its private half in the keychain.
    carried: Vec<Value>,
    /// False when the last read found something other than a list.
    read_a_list: bool,
}

impl SshKeyStore {
    pub const STORAGE_KEY: &'static str = "moshpit.sshkeys.v1";

    pub fn new(defaults: Arc<dyn Defaults>, readiness: Readiness) -> Self {
        let mut store = Self {
            defaults,
            readiness,
            keys: Vec::new(),
            is_authoritative: false,
            carried: Vec::new(),
            read_a_list: true,
        };
        store.load();
        store
    }

    pub fn keys(&self) -> &[SshKeyRecord] {
        &self.keys
    }

    pub fn is_authoritative(&self) -> bool {
        self.is_authoritative
    }

    /// Keys made in this device's Secure Enclave.
    pub fn device_keys(&self) -> Vec<&SshKeyRecord> {
        self.keys
            .iter()
            .filter(|k| k.source == SshKeySource::SecureEnclave)
            .collect()
    }

    pub fn imported_keys(&self) -> Vec<&SshKeyRecord> {
        self.keys
            .iter()
            .filter(|k| k.source != SshKeySource::SecureEnclave)
            .collect()
    }

    /// Every keychain ref a stored record holds, the carried ones' too —
    /// each the only handle on a private key. `None` when that can't be
    /// known: a read that can't be trusted, a stored value that isn't a
    /// list, a carried entry whose `keychainRef` can't be read (B232).
    /// `excluding` leaves one record out (the key being deleted).
    pub fn keychain_refs(&self, excluding: Option<Uuid>) -> Option<BTreeSet<String>> {
        if !self.is_authoritative || !self.read_a_list {
            return None;
        }
        let mut refs: BTreeSet<String> = self
            .keys
            .iter()
            .filter(|k| Some(k.id) != excluding)
            .filter_map(|k| k.keychain_ref.clone())
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

    pub fn add(&mut self, record: SshKeyRecord) {
        self.reload_if_needed();
        self.keys.push(record);
        self.persist();
    }

    /// The record alone; deleting a key — its private key too — is
    /// [`crate::secrets::delete_key`] (B232).
    pub fn remove(&mut self, id: Uuid) {
        self.reload_if_needed();
        self.keys.retain(|k| k.id != id);
        self.persist();
    }

    /// How often a key's last use is written: once a day is all SSH Keys
    /// tells (unused for ninety days, or since a month).
    pub const LAST_USED_GRANULARITY: f64 = 24.0 * 60.0 * 60.0;

    /// Key `id` signed in `now` (B601): its `last_used_at`, written at most
    /// once a day. Nothing ever wrote it — in no build — so SSH Keys' unused
    /// key (the gray avatar, "unused since") never showed. Whether it was
    /// written; false for an unknown key, a day not yet out, or a list that
    /// can't be trusted.
    pub fn mark_used(&mut self, id: Uuid, now: Date) -> bool {
        self.reload_if_needed();
        let Some(key) = self.keys.iter_mut().find(|k| k.id == id) else {
            return false;
        };
        if let Some(last) = key.last_used_at {
            let since = now.seconds_since(last);
            // A clock set back leaves the stored day ahead: written again.
            if (0.0..Self::LAST_USED_GRANULARITY).contains(&since) {
                return false;
            }
        }
        key.last_used_at = Some(now);
        self.persist();
        self.is_authoritative
    }

    /// Key `id`'s algorithm, size, public key and fingerprint, as read off
    /// its own private key (`facts`), for a record that
    /// [lacks them](SshKeyRecord::lacks_facts) (B726). Nothing else about
    /// the record changes — its name ("Recovered key …" too), its hosts,
    /// its Face ID. Whether it was written; false for an unknown key, one
    /// that has its facts, facts of a Secure Enclave key (never a keychain
    /// item read as text), or a list that can't be trusted.
    pub fn learn_facts(&mut self, id: Uuid, facts: &KeyFacts) -> bool {
        self.reload_if_needed();
        let Some(key) = self.keys.iter_mut().find(|k| k.id == id) else {
            return false;
        };
        if !key.lacks_facts()
            || facts.public_key.is_empty()
            || facts.algorithm == SshKeyAlgorithm::SeP256
        {
            return false;
        }
        key.algorithm = facts.algorithm;
        key.bits = facts.bits;
        key.public_key = facts.public_key.clone();
        key.fingerprint = facts.fingerprint.clone();
        self.persist();
        self.is_authoritative
    }

    /// Key `id` is a kind no connect path signs in with (`kind`, read off
    /// its private key, B780), for a record that
    /// [lacks its facts](SshKeyRecord::lacks_facts): kept, so the key list
    /// and the picker say so before a sign-in fails. Whether it was
    /// written.
    pub fn mark_refused(&mut self, id: Uuid, kind: &str) -> bool {
        self.reload_if_needed();
        let Some(key) = self.keys.iter_mut().find(|k| k.id == id) else {
            return false;
        };
        if !key.lacks_facts() || kind.is_empty() {
            return false;
        }
        key.refused = Some(kind.to_owned());
        self.persist();
        self.is_authoritative
    }

    pub fn reload_if_needed(&mut self) {
        if !self.is_authoritative {
            self.load();
        }
    }

    pub fn load(&mut self) {
        let defaults = self.defaults.as_ref();
        let readable = self.readiness.is_readable(defaults);
        let contents: Contents<SshKeyRecord> = stored_list::read(defaults, Self::STORAGE_KEY);
        self.is_authoritative =
            readable && stored_list::secure(defaults, Self::STORAGE_KEY, &contents);
        self.keys = contents.elements;
        self.carried = contents.carried;
        self.read_a_list = contents.is_list;
        if readable {
            self.readiness.mark_readable(defaults);
        }
    }

    fn persist(&self) {
        // A sealed file is not an empty list; never write over one.
        if !self.is_authoritative {
            return;
        }
        if let Some(data) = stored_list::encode(&self.keys, &self.carried) {
            self.defaults
                .set(Self::STORAGE_KEY, Some(Plist::Data(data)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::MemoryDefaults;
    use moshpit_seal::foundation::json;
    use serde::{Deserialize, Serialize};

    fn now() -> Date {
        Date::from_unix(1_762_800_000.25)
    }

    fn record(
        name: &str,
        algorithm: SshKeyAlgorithm,
        public_key: &str,
        source: SshKeySource,
    ) -> SshKeyRecord {
        let mut record = SshKeyRecord::new(Uuid::new_v4(), name, algorithm, source, now());
        record.public_key = public_key.into();
        record
    }

    /// One key of every kind the app makes, plain ECDSA among them
    /// (`SSHKeyRecordTests.everyKind`).
    fn every_kind() -> Vec<SshKeyRecord> {
        vec![
            record(
                "laptop",
                SshKeyAlgorithm::Ed25519,
                "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA== laptop",
                SshKeySource::Generated,
            ),
            record(
                "work",
                SshKeyAlgorithm::Ecdsa,
                "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTY= work",
                SshKeySource::Imported,
            ),
            record(
                "server",
                SshKeyAlgorithm::Rsa4096,
                "ssh-rsa AAAAB3NzaC1yc2E= server",
                SshKeySource::Generated,
            ),
            record(
                "phone",
                SshKeyAlgorithm::SeP256,
                "",
                SshKeySource::SecureEnclave,
            ),
            record(
                "yubikey",
                SshKeyAlgorithm::EcdsaSk,
                "sk-ecdsa-sha2-nistp256@openssh.com AAAA yubikey",
                SshKeySource::Hardware,
            ),
        ]
    }

    /// `SSHKeyRecord` as every build up to and including 1.0.8 declares it.
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OlderBuildKeyRecord {
        id: Uuid,
        name: String,
        algorithm: OlderAlgorithm,
        comment: String,
        fingerprint: String,
        public_key: String,
        source: String,
        bound_hosts: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keychain_ref: Option<String>,
        require_biometry: bool,
        created_at: Date,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_used_at: Option<Date>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
    enum OlderAlgorithm {
        #[serde(rename = "ED25519")]
        Ed25519,
        #[serde(rename = "ECDSA-sk")]
        EcdsaSk,
        #[serde(rename = "RSA-4096")]
        Rsa4096,
        #[serde(rename = "SE · P256")]
        SeP256,
    }

    fn stored(defaults: &MemoryDefaults) -> Vec<u8> {
        match defaults.get(SshKeyStore::STORAGE_KEY) {
            Some(Plist::Data(data)) => data,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_older_build_reads_every_key_this_build_saved() {
        let defaults = Arc::new(MemoryDefaults::new());
        let mut store = SshKeyStore::new(defaults.clone(), Readiness::Always);
        for record in every_kind() {
            store.add(record);
        }
        let older: Vec<OlderBuildKeyRecord> = json::decode(&stored(&defaults)).unwrap();
        assert_eq!(
            older.iter().map(|k| k.algorithm).collect::<Vec<_>>(),
            [
                OlderAlgorithm::Ed25519,
                OlderAlgorithm::EcdsaSk,
                OlderAlgorithm::Rsa4096,
                OlderAlgorithm::SeP256,
                OlderAlgorithm::EcdsaSk
            ]
        );
        assert_eq!(
            older.iter().map(|k| k.id).collect::<Vec<_>>(),
            store.keys().iter().map(|k| k.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn this_build_reads_its_own_records_back_with_their_real_algorithms() {
        let defaults = Arc::new(MemoryDefaults::new());
        let mut store = SshKeyStore::new(defaults.clone(), Readiness::Always);
        for record in every_kind() {
            store.add(record);
        }
        let reread = SshKeyStore::new(defaults.clone(), Readiness::Always);
        assert_eq!(reread.keys(), store.keys());
    }

    #[test]
    fn what_an_older_build_saves_back_this_build_reads_plain_ecdsa_still_plain() {
        let defaults = Arc::new(MemoryDefaults::new());
        let mut store = SshKeyStore::new(defaults.clone(), Readiness::Always);
        for record in every_kind() {
            store.add(record);
        }
        let mut older: Vec<OlderBuildKeyRecord> = json::decode(&stored(&defaults)).unwrap();
        older.retain(|k| k.name != "server");
        defaults.set(
            SshKeyStore::STORAGE_KEY,
            Some(Plist::Data(json::encode(&older).into_bytes())),
        );
        let reread = SshKeyStore::new(defaults.clone(), Readiness::Always);
        let expected: Vec<SshKeyRecord> = every_kind()
            .into_iter()
            .filter(|k| k.name != "server")
            .collect();
        assert_eq!(
            reread
                .keys()
                .iter()
                .map(|k| (k.name.clone(), k.algorithm))
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|k| (k.name.clone(), k.algorithm))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_unknown_real_algorithm_falls_back_to_the_shared_name() {
        let mut value = record(
            "token",
            SshKeyAlgorithm::Ed25519,
            "",
            SshKeySource::Imported,
        )
        .to_json();
        if let Value::Object(fields) = &mut value {
            fields.insert("keyAlgorithm", Value::String("ED448".into()));
        }
        assert_eq!(
            SshKeyRecord::from_json(&value).unwrap().algorithm,
            SshKeyAlgorithm::Ed25519
        );
    }

    /// The 2048-bit `ssh-keygen` key of `SSHKeyFixtures` ("RSA 2048 · OpenSSH").
    const RSA_2048: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDkYs+QICsPo/VgCPbUXwgMestI7VPDYen1WcQ6K8o6e++LVX8B/HqOMs7XL0DWIrU7J4vVuPdL7Pyvhc1Gc946Z8woOaE/BpUnydTWZCTORrjUqWAkZ5NnlWpTPf92RVXbp2qpjXXKTiDmP0COkQfbyfubOpN3O0bcy0LZQplOt4DIn5iCEuV01qU4d2OJIrZhZwxwIegMfLYSid7sYa0EoJhW2JWMoi4LuYRDhFLusotZaT/Qahxxd587CQz+Tch5hLUeeL1Cei8EI78o2ZOhguryU8zA+r3ncyrjH5hls9p91MWE8UDf7f2mK8Ecfux1GOlFhgfqNpqwEYpUQDsN fixture-rsa-openssh-none";

    #[test]
    fn an_rsa_keys_size_comes_off_its_public_key() {
        assert_eq!(rsa_bits_of_public_key_line(RSA_2048), Some(2048));
        assert_eq!(
            rsa_bits_of_public_key_line("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA=="),
            None
        );
        assert_eq!(rsa_bits_of_public_key_line("ssh-rsa"), None);
        assert_eq!(rsa_bits_of_public_key_line("ssh-rsa !!"), None);
        let mut older = record(
            "server",
            SshKeyAlgorithm::Rsa4096,
            RSA_2048,
            SshKeySource::Imported,
        );
        let value = older.to_json();
        let read = SshKeyRecord::from_json(&value).unwrap();
        assert_eq!(read.bits, Some(2048));
        assert_eq!(read.badge_text(), "RSA-2048");
        older.public_key.clear();
        assert_eq!(
            SshKeyRecord::from_json(&older.to_json())
                .unwrap()
                .badge_text(),
            "RSA"
        );
        older.bits = Some(3072);
        assert_eq!(
            SshKeyRecord::from_json(&older.to_json()).unwrap().bits,
            Some(3072)
        );
    }

    #[test]
    fn badges() {
        let mut r = record(
            "p",
            SshKeyAlgorithm::SeP256,
            "",
            SshKeySource::SecureEnclave,
        );
        assert_eq!(r.badge_text(), "SE · P256");
        r.algorithm = SshKeyAlgorithm::Ed25519;
        assert_eq!(r.badge_text(), "SE · ED25519");
        r.source = SshKeySource::Imported;
        assert_eq!(r.badge_text(), "ED25519");
        r.algorithm = SshKeyAlgorithm::Rsa4096;
        r.bits = Some(4096);
        assert_eq!(r.badge_text(), "RSA-4096");
    }

    #[test]
    fn a_record_this_build_cant_decode_is_kept_not_dropped() {
        let defaults = Arc::new(MemoryDefaults::new());
        let laptop = record(
            "laptop",
            SshKeyAlgorithm::Ed25519,
            "",
            SshKeySource::Generated,
        );
        let mut stranger = record(
            "yubikey",
            SshKeyAlgorithm::EcdsaSk,
            "",
            SshKeySource::Hardware,
        )
        .to_json();
        if let Value::Object(fields) = &mut stranger {
            fields.insert("algorithm", Value::String("ED448-sk".into()));
        }
        let list = Value::Array(vec![laptop.to_json(), stranger]);
        defaults.set(
            SshKeyStore::STORAGE_KEY,
            Some(Plist::Data(codec::write(&list).into_bytes())),
        );
        let mut store = SshKeyStore::new(defaults.clone(), Readiness::Always);
        assert_eq!(store.keys().len(), 1);
        store.add(record(
            "phone",
            SshKeyAlgorithm::SeP256,
            "",
            SshKeySource::SecureEnclave,
        ));
        let Value::Array(entries) =
            json::parse(&stored(&defaults), json::Dialect::Decoder).unwrap()
        else {
            panic!()
        };
        assert_eq!(entries.len(), 3);
        assert_eq!(
            SshKeyStore::new(defaults.clone(), Readiness::Always)
                .keys()
                .len(),
            2
        );
    }
    /// B601: a key's last use, recorded at most once a day, the day kept
    /// across a launch; nothing for a key not stored, nothing over a list
    /// that can't be trusted.
    #[test]
    fn a_sign_in_records_the_keys_last_use_once_a_day() {
        let defaults = Arc::new(MemoryDefaults::new());
        let mut store = SshKeyStore::new(defaults.clone(), Readiness::Always);
        let key = SshKeyRecord::new(
            Uuid::new_v4(),
            "laptop",
            SshKeyAlgorithm::Ed25519,
            SshKeySource::Generated,
            now(),
        );
        let id = key.id;
        store.add(key);
        let first = Date::from_unix(1_790_000_000.0);
        assert!(store.mark_used(id, first));
        assert_eq!(store.keys()[0].last_used_at, Some(first));
        let reread = SshKeyStore::new(defaults.clone(), Readiness::Always);
        assert_eq!(reread.keys()[0].last_used_at, Some(first), "stored");

        assert!(
            !store.mark_used(id, first.adding(3_600.0)),
            "within the day"
        );
        assert_eq!(store.keys()[0].last_used_at, Some(first));
        let next_day = first.adding(SshKeyStore::LAST_USED_GRANULARITY);
        assert!(store.mark_used(id, next_day));
        assert_eq!(store.keys()[0].last_used_at, Some(next_day));
        let set_back = first.adding(-SshKeyStore::LAST_USED_GRANULARITY);
        assert!(
            store.mark_used(id, set_back),
            "a clock set back writes again"
        );

        assert!(!store.mark_used(Uuid::new_v4(), next_day), "no such key");

        let sealed = Arc::new(MemoryDefaults::new());
        let mut writer = SshKeyStore::new(sealed.clone(), Readiness::Always);
        let other = SshKeyRecord::new(
            Uuid::new_v4(),
            "pi",
            SshKeyAlgorithm::Ed25519,
            SshKeySource::Generated,
            now(),
        );
        let other_id = other.id;
        writer.add(other);
        // A launch before the first unlock: no sentinel, the phone locked.
        sealed.set(crate::readiness::SENTINEL_KEY, None);
        sealed.set_protected_data_available(false);
        let mut locked = SshKeyStore::new(sealed.clone(), Readiness::System);
        let stored = sealed.get(SshKeyStore::STORAGE_KEY);
        assert!(!locked.mark_used(other_id, next_day), "an untrusted list");
        assert_eq!(
            sealed.get(SshKeyStore::STORAGE_KEY),
            stored,
            "nothing written"
        );
    }
}
