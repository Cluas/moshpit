//! SSH private keys as people have them, for every platform: importing a
//! pasted or picked key, making one, telling what a stored key is, and the
//! one form the keychain keeps.
//!
//! Whatever is pasted — OpenSSH's own format (encrypted with any cipher
//! `ssh-keygen -Z` offers, or not), PKCS #1 RSA, SEC1 EC, PKCS #8 (plain or
//! PBES2), OpenSSL's traditional encrypted PEM, a PuTTY key — is read here
//! and stored as an unencrypted `openssh-key-v1` file ([`StoredKey::openssh`],
//! never logged, wiped when dropped). A passphrase is used once, at import;
//! the keychain (behind Face ID when asked) protects the key from then on.
//! Ed25519, RSA and ECDSA on P-256, P-384 and P-521 sign in; anything else
//! (security keys, DSA, Ed448, other curves) is refused at import, saying
//! which it is, not at the first connection.
//!
//! The store format is the app's since its first release: the text
//! `ssh-keygen` writes with an empty passphrase (70-column base64, `\n`
//! line endings), which every build — 1.0.8's Citadel readers included —
//! reads back. A key kept in another form (imported before B29, pasted into
//! a connection's form) is converted on its first use ([`assess`]).
//!
//! A hardware key's private half never comes here: the platform makes it
//! (the Secure Enclave, the Android Keystore) and hands over its public
//! half, which [`hardware_public_key`] turns into the record's
//! `authorized_keys` line and fingerprint.

mod armor;
mod cbc;
mod der;
mod material;
mod openssh;
mod pkcs;
#[cfg(test)]
mod tests;

use russh::keys::ssh_encoding::{Decode, Encode};
use russh::keys::ssh_key::private::KeypairData;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::ssh_key::{LineEnding, PrivateKey, PublicKey};
use zeroize::Zeroizing;

use moshpit_store::ssh_keys::SshKeyAlgorithm;

use crate::auth::{KeyInfo, KeyKind};
use crate::error::KeyError;
use crate::host_key::fingerprint_of_blob;
use armor::{Armor, base64_encode, strict_base64};
use openssh::Container;

pub use material::Curve as EcdsaCurve;

const OPENSSH_BEGIN: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";
const OPENSSH_END: &str = "-----END OPENSSH PRIVATE KEY-----";

/// Why a key can't be imported (or made) — each one something the person
/// can act on. Never carries key material. The text is the app's English
/// copy; iOS shows its localised own for each case.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    /// Not a private key this reads at all.
    #[error(
        "This isn't a private key Moshpit can read. Paste an OpenSSH or PEM private key, from its BEGIN line to its END line."
    )]
    Unrecognized,
    /// A private key, but what is inside doesn't add up: cut short, or
    /// garbled.
    #[error(
        "This private key is damaged or incomplete. Copy it again, from its BEGIN line to its END line."
    )]
    Damaged,
    /// The key is encrypted and no passphrase was given.
    #[error("This key is protected by a passphrase. Enter it to import the key.")]
    NeedsPassphrase,
    /// The passphrase given does not open it.
    #[error("That passphrase doesn't unlock this key.")]
    WrongPassphrase,
    /// A FIDO security key (`sk-ecdsa…`, `sk-ssh-ed25519…`): the file holds
    /// a handle, and the private half lives on the token.
    #[error("Security keys (sk keys), such as a YubiKey's, aren't supported yet.")]
    SecurityKey,
    /// A kind of key that can't sign in from here ("DSA", "Ed448", …).
    #[error("Moshpit can't sign in with {0} keys. Ed25519, RSA and ECDSA keys work.")]
    Unsupported(String),
    /// Encrypted with a scheme this can't undo.
    #[error(
        "This key is encrypted with {0}, which Moshpit can't unlock. Re-encrypt it with \u{201c}ssh-keygen -p\u{201d} and import it again."
    )]
    UnsupportedEncryption(String),
    /// Making or encoding a key failed.
    #[error("Key generation failed: {0}")]
    Failed(String),
}

/// A private key read from a key file, and the comment the file carries
/// (an `openssh-key-v1` or PuTTY file's; other forms have none).
pub struct Contents {
    pub key: PrivateKey,
    pub comment: String,
}

impl std::fmt::Debug for Contents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Contents")
            .field("algorithm", &self.key.algorithm())
            .finish_non_exhaustive()
    }
}

impl Contents {
    /// The public key in SSH wire form.
    pub fn public_blob(&self) -> Vec<u8> {
        material::public_blob(self.key.key_data()).unwrap_or_default()
    }
}

/// A key ready for the keychain.
pub struct StoredKey {
    /// The unencrypted `openssh-key-v1` text, `\n` line endings.
    pub openssh: Zeroizing<String>,
    /// Its public facts, as the key list shows them.
    pub info: KeyInfo,
    /// An RSA key's modulus size.
    pub bits: Option<i64>,
}

impl std::fmt::Debug for StoredKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredKey")
            .field("info", &self.info)
            .field("bits", &self.bits)
            .finish_non_exhaustive()
    }
}

impl StoredKey {
    /// What the key record calls it: every RSA key is "RSA-4096" (its size
    /// is `bits`), every software ECDSA key "ECDSA".
    pub fn record_algorithm(&self) -> SshKeyAlgorithm {
        match self.info.kind {
            KeyKind::Ed25519 => SshKeyAlgorithm::Ed25519,
            KeyKind::Rsa => SshKeyAlgorithm::Rsa4096,
            _ => SshKeyAlgorithm::Ecdsa,
        }
    }
}

/// What a generated key is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generate {
    Ed25519,
    /// RSA with this many bits (the app makes 4096).
    Rsa(usize),
}

/// Whether `text` holds a private key that needs a passphrase to read, so
/// the form asks for one. False for anything unreadable (reading it says
/// why), and for a key refused whatever the passphrase.
pub fn is_encrypted(text: &str) -> bool {
    if text.trim_start().starts_with("PuTTY-User-Key-File-") {
        return !text.contains("Encryption: none");
    }
    let Ok(armor) = Armor::find(text) else {
        return false;
    };
    match armor.label.as_str() {
        "ENCRYPTED PRIVATE KEY" => true,
        "OPENSSH PRIVATE KEY" => {
            Container::parse(&armor.body).is_ok_and(|c| c.is_encrypted() && c.key_type().is_ok())
        }
        _ => armor.is_legacy_encrypted(),
    }
}

/// The key in `text`, decrypted with `passphrase` when it needs one (an
/// empty passphrase is none).
pub fn read(text: &str, passphrase: Option<&str>) -> Result<Contents, ImportError> {
    let passphrase = passphrase.filter(|p| !p.is_empty());
    if text.trim_start().starts_with("PuTTY-User-Key-File-") {
        return read_putty(text, passphrase);
    }
    let armor = Armor::find(text)?;
    let secret = passphrase.map(str::as_bytes);
    let (key, comment) = match armor.label.as_str() {
        "OPENSSH PRIVATE KEY" => Container::parse(&armor.body)?.read(secret)?,
        "RSA PRIVATE KEY" | "EC PRIVATE KEY" => {
            let der = armor.legacy_decrypted(secret)?;
            let key = if armor.label == "RSA PRIVATE KEY" {
                pkcs::pkcs1(&der)
            } else {
                pkcs::sec1(&der, None)
            };
            match key {
                // Decrypted to garbage that happened to end in valid padding.
                Err(ImportError::Damaged) if armor.is_legacy_encrypted() => {
                    return Err(ImportError::WrongPassphrase);
                }
                other => (other?, String::new()),
            }
        }
        "PRIVATE KEY" => (pkcs::pkcs8(&armor.body)?, String::new()),
        "ENCRYPTED PRIVATE KEY" => {
            let der = pkcs::pbes2_decrypt(&armor.body, secret)?;
            match pkcs::pkcs8(&der) {
                Err(ImportError::Damaged) => return Err(ImportError::WrongPassphrase),
                other => (other?, String::new()),
            }
        }
        "DSA PRIVATE KEY" => return Err(ImportError::Unsupported("DSA".to_owned())),
        _ => return Err(ImportError::Unrecognized),
    };
    let key =
        PrivateKey::new(key, comment.as_str()).map_err(|e| ImportError::Failed(e.to_string()))?;
    Ok(Contents { key, comment })
}

/// A PuTTY key (`.ppk`, versions 2 and 3), by ssh-key's reader.
fn read_putty(text: &str, passphrase: Option<&str>) -> Result<Contents, ImportError> {
    let encrypted = !text.contains("Encryption: none");
    if encrypted && passphrase.is_none() {
        return Err(ImportError::NeedsPassphrase);
    }
    // Each line as PuTTYgen wrote it: a paste's indents and CRLF dropped.
    let text = Zeroizing::new(text.lines().map(str::trim).collect::<Vec<_>>().join("\n"));
    let key = PrivateKey::from_ppk(text.trim(), passphrase.map(str::to_owned)).map_err(|e| {
        // A wrong passphrase decrypts to a private half whose MAC doesn't
        // check (ssh-key doesn't export its PPK error type: its words).
        let mac = matches!(e, russh::keys::ssh_key::Error::Crypto)
            || e.to_string().contains("incorrect MAC");
        if encrypted && mac {
            ImportError::WrongPassphrase
        } else {
            ImportError::Damaged
        }
    })?;
    if let Some(refused) = refusal(key.algorithm().as_str()) {
        return Err(refused);
    }
    // Checked as every other form is: rebuilt from its numbers.
    let rebuilt = match key.key_data() {
        KeypairData::Ed25519(pair) => {
            let seed: &[u8; 32] = pair.private.as_ref();
            material::ed25519(seed)?
        }
        KeypairData::Ecdsa(pair) => material::ecdsa(
            EcdsaCurve::from_key_type(pair.algorithm().as_str()).ok_or(ImportError::Damaged)?,
            pair.private_key_bytes(),
        )?,
        KeypairData::Rsa(pair) => material::rsa(
            pair.public().n().as_bytes(),
            pair.public().e().as_bytes(),
            pair.private().d().as_bytes(),
            pair.private().p().as_bytes(),
            pair.private().q().as_bytes(),
        )?,
        _ => {
            return Err(ImportError::Unsupported(
                key.algorithm().as_str().to_owned(),
            ));
        }
    };
    if material::public_blob(&rebuilt)? != material::public_blob(key.key_data())? {
        return Err(ImportError::Damaged);
    }
    let comment = key.comment().as_str_lossy().to_owned();
    let key = PrivateKey::new(rebuilt, comment.as_str())
        .map_err(|e| ImportError::Failed(e.to_string()))?;
    Ok(Contents { key, comment })
}

/// `None` when a key of SSH type `key_type` signs in from here; otherwise
/// why it's refused.
pub fn refusal(key_type: &str) -> Option<ImportError> {
    match key_type {
        "ssh-ed25519"
        | "ssh-rsa"
        | "ecdsa-sha2-nistp256"
        | "ecdsa-sha2-nistp384"
        | "ecdsa-sha2-nistp521" => None,
        "ssh-dss" => Some(ImportError::Unsupported("DSA".to_owned())),
        other if other.starts_with("sk-") => Some(ImportError::SecurityKey),
        other => Some(ImportError::Unsupported(other.to_owned())),
    }
}

/// The kind of key a saved key is, by name ("DSA"), when it is one that
/// can't sign in — judged from its public key line alone, which is all the
/// key picker sees without unlocking the key. `None` when it signs in, and
/// when the line doesn't say (an old import saved without its public half):
/// such a key says why at connect. Only the types a saved key can really be;
/// security keys never get this far (the picker leaves ECDSA-sk out).
pub fn refused_type(public_key_line: &str) -> Option<String> {
    let key_type = public_key_line.split(' ').next()?;
    if key_type != "ssh-dss" {
        return None;
    }
    match refusal(key_type) {
        Some(ImportError::Unsupported(name)) => Some(name),
        _ => None,
    }
}

/// Reads a pasted private key (`passphrase` when it has one) and returns it
/// in the form the keychain keeps.
///
/// `comment` ends the public key line and is the stored key's comment.
/// Left empty, a pasted `.pub` line's comment stays (`public_line`; it
/// contributes nothing else), else the key file's own.
pub fn import(
    text: &str,
    passphrase: Option<&str>,
    public_line: Option<&str>,
    comment: &str,
) -> Result<StoredKey, ImportError> {
    let contents = read(text, passphrase)?;
    let pasted = public_line
        .and_then(comment_of_public_key_line)
        .unwrap_or_default();
    let comment = if !comment.is_empty() {
        comment.to_owned()
    } else if !pasted.is_empty() {
        pasted
    } else {
        contents.comment.clone()
    };
    stored(contents.key, &comment)
}

/// A new key pair, its comment `comment`.
pub fn generate(kind: Generate, comment: &str) -> Result<StoredKey, ImportError> {
    let mut rng = rand::rng();
    let failed = |e: russh::keys::ssh_key::Error| ImportError::Failed(e.to_string());
    let key = match kind {
        Generate::Ed25519 => {
            PrivateKey::random(&mut rng, russh::keys::Algorithm::Ed25519).map_err(failed)?
        }
        Generate::Rsa(bits) => {
            let pair = russh::keys::ssh_key::private::RsaKeypair::random(&mut rng, bits)
                .map_err(failed)?;
            PrivateKey::new(KeypairData::Rsa(pair), "").map_err(failed)?
        }
    };
    stored(key, comment)
}

/// What a stored private key is, read off the key alone — for a keychain
/// item no record names any more: its type, public key line, fingerprint
/// and size, the key's own comment on the line. Any form the app reads, as
/// long as it is unencrypted and signs in; `None` for anything else (a
/// password, a key behind a passphrase). The stored text isn't rewritten.
pub fn describe_stored(text: &str) -> Option<StoredKey> {
    if !text.contains("-----BEGIN") {
        return None;
    }
    import(text, None, None, "").ok()
}

/// What to do with a key as the keychain holds it.
#[derive(Debug, PartialEq, Eq)]
pub enum Assessment {
    /// Every connect path reads it as stored, 1.0.8's included.
    Usable,
    /// The same key as the `openssh-key-v1` file every connect path reads.
    /// Already checked: it reads back to the same public key.
    Converted(Zeroizing<String>),
    /// It is behind a passphrase, which the person has to give once.
    NeedsPassphrase,
}

/// What to do with `stored` — decrypted with `passphrase` when it needs one
/// (B125). Keys imported before B29 were stored as pasted, and so is a key
/// pasted into a connection's own form: a PEM key, one behind a passphrase,
/// one with Windows line endings or indented lines. Those are converted;
/// one already in the stored form is left exactly as it is.
pub fn assess(stored: &str, passphrase: Option<&str>) -> Result<Assessment, ImportError> {
    let contents = match read(stored, passphrase) {
        Err(ImportError::NeedsPassphrase) => return Ok(Assessment::NeedsPassphrase),
        other => other?,
    };
    if !is_encrypted(stored) && reads_as_stored(stored) {
        return Ok(Assessment::Usable);
    }
    let converted = to_openssh(&contents.key)?;
    let again = read(&converted, None)?;
    if again.public_blob() != contents.public_blob() || !reads_as_stored(&converted) {
        return Err(ImportError::Damaged);
    }
    Ok(Assessment::Converted(converted))
}

/// What to keep for a key pasted into a connection's own form (B133): the
/// converted key when it reads without a passphrase and isn't in the stored
/// form already; otherwise exactly what was pasted — a clean key as it is,
/// and a key behind a passphrase, or text that isn't a key, for the connect
/// to ask about or explain ([`assess`]).
pub fn storable(pasted: &str) -> Zeroizing<String> {
    if !pasted.is_empty()
        && let Ok(Assessment::Converted(converted)) = assess(pasted, None)
    {
        return converted;
    }
    Zeroizing::new(pasted.to_owned())
}

/// Whether every connect path reads `text` as it is, without converting
/// it: an `openssh-key-v1` file that reads, written the way the 1.0.8
/// Swift connect path takes one — Citadel's `detectPrivateKeyType` and the
/// readers after it remove only `\n` between the lines and then want strict
/// base64, so a `\r`, an indent or a stray space fails them.
pub fn reads_as_stored(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.starts_with(OPENSSH_BEGIN) {
        return false;
    }
    let joined = Zeroizing::new(trimmed.replace('\n', ""));
    let Some(body) = joined
        .strip_prefix(OPENSSH_BEGIN)
        .and_then(|rest| rest.strip_suffix(OPENSSH_END))
    else {
        return false;
    };
    let Some(data) = strict_base64(body) else {
        return false;
    };
    let Ok(container) = Container::parse(&data) else {
        return false;
    };
    !container.is_encrypted() && container.key_type().is_ok() && container.read(None).is_ok()
}

/// The `authorized_keys` line and `SHA256:` fingerprint of a hardware
/// P-256 key, from the public half the platform hands over: X.509
/// SubjectPublicKeyInfo DER (Android's `PublicKey.getEncoded()`) or the
/// bare 65-byte uncompressed point (CryptoKit's `x963Representation`).
pub fn hardware_public_key(public: &[u8], comment: &str) -> Result<(String, String), ImportError> {
    const POINT: usize = 65;
    // P-256 SubjectPublicKeyInfo: a fixed 26-byte header, then the point.
    const SPKI_HEADER: [u8; 26] = [
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    let point = match public.len() {
        POINT => public,
        n if n == SPKI_HEADER.len() + POINT && public.starts_with(&SPKI_HEADER) => {
            &public[SPKI_HEADER.len()..]
        }
        _ => return Err(ImportError::Unrecognized),
    };
    if point[0] != 0x04 {
        return Err(ImportError::Damaged);
    }
    let q = russh::keys::ssh_key::public::EcdsaPublicKey::from_sec1_bytes(point)
        .map_err(|_| ImportError::Damaged)?;
    if !matches!(q, russh::keys::ssh_key::public::EcdsaPublicKey::NistP256(_)) {
        return Err(ImportError::Damaged);
    }
    let blob = KeyData::Ecdsa(q)
        .encode_vec()
        .map_err(|e| ImportError::Failed(e.to_string()))?;
    Ok((
        authorized_keys_line("ecdsa-sha2-nistp256", &blob, comment),
        fingerprint_of_blob(&blob),
    ))
}

/// `type base64` and the comment, when there is one.
pub fn authorized_keys_line(key_type: &str, blob: &[u8], comment: &str) -> String {
    let line = format!("{key_type} {}", base64_encode(blob));
    if comment.is_empty() {
        line
    } else {
        format!("{line} {comment}")
    }
}

/// The comment ending a pasted `.pub` line (`type base64-blob [comment]`);
/// `None` unless the line reads as one.
fn comment_of_public_key_line(line: &str) -> Option<String> {
    let line = line.trim();
    // The first two space-separated words (runs of spaces between them
    // count once), then the rest.
    let mut words = Vec::with_capacity(2);
    let mut rest = line;
    while words.len() < 2 {
        let rest_trimmed = rest.trim_start_matches(' ');
        if rest_trimmed.is_empty() {
            break;
        }
        match rest_trimmed.find(' ') {
            Some(at) => {
                words.push(&rest_trimmed[..at]);
                rest = &rest_trimmed[at + 1..];
            }
            None => {
                words.push(rest_trimmed);
                rest = "";
            }
        }
    }
    if words.len() < 2 || strict_base64(words[1]).is_none() {
        return None;
    }
    Some(
        rest.trim_matches(|c: char| c.is_whitespace() && c != '\n')
            .to_owned(),
    )
}

/// The key as an unencrypted `openssh-key-v1` file, `\n` line endings.
fn to_openssh(key: &PrivateKey) -> Result<Zeroizing<String>, ImportError> {
    key.to_openssh(LineEnding::LF)
        .map(|text| Zeroizing::new(text.to_string()))
        .map_err(|e| ImportError::Failed(e.to_string()))
}

fn stored(mut key: PrivateKey, comment: &str) -> Result<StoredKey, ImportError> {
    if let Some(refused) = refusal(key.algorithm().as_str()) {
        return Err(refused);
    }
    key.set_comment(comment);
    let blob = material::public_blob(key.key_data())?;
    let kind = KeyKind::of(&key.algorithm());
    let bits = match key.key_data() {
        KeypairData::Rsa(pair) => Some(i64::from(pair.key_size())),
        _ => None,
    };
    let info = KeyInfo {
        public_key: authorized_keys_line(kind.ssh_name(), &blob, comment),
        fingerprint: fingerprint_of_blob(&blob),
        comment: comment.to_owned(),
        encrypted: false,
        kind,
    };
    Ok(StoredKey {
        openssh: to_openssh(&key)?,
        info,
        bits,
    })
}

// MARK: - The connect path

/// The private key a connection signs with, from the keychain's text: an
/// unencrypted `openssh-key-v1` file of a kind that signs in, read by the
/// same reader import uses (whitespace anywhere forgiven).
pub(crate) fn openssh_private_key(text: &str) -> Result<PrivateKey, KeyError> {
    let container_text = text.trim();
    if !container_text.starts_with(OPENSSH_BEGIN) {
        return Err(KeyError::NotOpenSsh);
    }
    let armor = Armor::find(container_text).map_err(|e| KeyError::Malformed(e.to_string()))?;
    let container =
        Container::parse(&armor.body).map_err(|e| KeyError::Malformed(e.to_string()))?;
    if container.is_encrypted() {
        return Err(KeyError::Encrypted);
    }
    if let Err(refused) = container.key_type() {
        return Err(KeyError::Unsupported(match refused {
            ImportError::Unsupported(_) | ImportError::SecurityKey => {
                openssh::Wire(container.public_blob)
                    .text()
                    .unwrap_or_default()
            }
            other => return Err(KeyError::Malformed(other.to_string())),
        }));
    }
    let (key, comment) = container
        .read(None)
        .map_err(|e| KeyError::Malformed(e.to_string()))?;
    PrivateKey::new(key, comment.as_str()).map_err(|e| KeyError::Malformed(e.to_string()))
}

/// The public facts of an `openssh-key-v1` private key: its public half is
/// in the clear, so an encrypted key's too (without its comment, which is
/// in the encrypted half).
pub(crate) fn inspect_openssh(text: &str) -> Result<KeyInfo, KeyError> {
    let text = text.trim();
    if !text.starts_with(OPENSSH_BEGIN) {
        return Err(KeyError::NotOpenSsh);
    }
    let malformed = |e: ImportError| KeyError::Malformed(e.to_string());
    let armor = Armor::find(text).map_err(malformed)?;
    let container = Container::parse(&armor.body).map_err(malformed)?;
    let public = KeyData::decode(&mut &*container.public_blob)
        .map_err(|e| KeyError::Malformed(e.to_string()))?;
    let kind = KeyKind::of(&public.algorithm());
    let comment = if !container.is_encrypted() && kind.can_sign_in() {
        container.read(None).map_err(malformed)?.1
    } else {
        String::new()
    };
    let line = PublicKey::new(public, comment.as_str())
        .to_openssh()
        .unwrap_or_default();
    Ok(KeyInfo {
        kind,
        public_key: line,
        fingerprint: fingerprint_of_blob(container.public_blob),
        comment,
        encrypted: container.is_encrypted(),
    })
}

// MARK: - The numbers, for a signer outside Rust

/// A software key's private numbers, for the Swift connect path's signers
/// (CryptoKit, Security): big-endian, no leading zero bytes except where a
/// fixed width says so. Wiped when dropped; never logged.
pub enum SigningKey {
    /// The 32-byte seed.
    Ed25519 { seed: Zeroizing<Vec<u8>> },
    /// The scalar at the curve's full width (32, 48 or 66 bytes).
    Ecdsa {
        curve: EcdsaCurve,
        scalar: Zeroizing<Vec<u8>>,
    },
    Rsa {
        n: Vec<u8>,
        e: Vec<u8>,
        d: Zeroizing<Vec<u8>>,
        p: Zeroizing<Vec<u8>>,
        q: Zeroizing<Vec<u8>>,
    },
}

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SigningKey::Ed25519 { .. } => "SigningKey::Ed25519",
            SigningKey::Ecdsa { .. } => "SigningKey::Ecdsa",
            SigningKey::Rsa { .. } => "SigningKey::Rsa",
        })
    }
}

/// The numbers of the unencrypted key in `text` (any form [`read`] takes).
pub fn signing_key(text: &str) -> Result<SigningKey, ImportError> {
    let contents = read(text, None)?;
    let positive = |m: &russh::keys::ssh_encoding::Mpint| {
        Zeroizing::new(m.as_positive_bytes().unwrap_or_default().to_vec())
    };
    Ok(match contents.key.key_data() {
        KeypairData::Ed25519(pair) => {
            let seed: &[u8; 32] = pair.private.as_ref();
            SigningKey::Ed25519 {
                seed: Zeroizing::new(seed.to_vec()),
            }
        }
        KeypairData::Ecdsa(pair) => SigningKey::Ecdsa {
            curve: EcdsaCurve::from_key_type(pair.algorithm().as_str())
                .ok_or(ImportError::Damaged)?,
            scalar: Zeroizing::new(pair.private_key_bytes().to_vec()),
        },
        KeypairData::Rsa(pair) => SigningKey::Rsa {
            n: positive(pair.public().n()).to_vec(),
            e: positive(pair.public().e()).to_vec(),
            d: positive(pair.private().d()),
            p: positive(pair.private().p()),
            q: positive(pair.private().q()),
        },
        _ => {
            return Err(ImportError::Unsupported(
                contents.key.algorithm().as_str().to_owned(),
            ));
        }
    })
}
