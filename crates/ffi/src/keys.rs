//! SSH keys for the hosts, through UniFFI: importing a pasted or picked key,
//! making one, telling what a stored key is and what the keychain should
//! keep — `moshpit_ssh::keyfile`, the one reader every platform shares.
//!
//! What stays on the platform: the keychain (and Face ID before it), the
//! Secure Enclave (its keys are made and sign there; only their public half
//! comes here, [`ssh_key_hardware_public_key`]), and the file picker.
//!
//! Key text and passphrases cross as strings because the host keeps them
//! (the keychain) or the person typed them; nothing here logs or keeps
//! them. Not a hot path: one call per import, generation or connect.

use moshpit_ssh::keyfile::{self, Assessment, EcdsaCurve, Generate, ImportError, SigningKey};

/// Why a key can't be imported or made — each one something the person can
/// act on; the host words it (iOS: `SSHKeyImportError`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum KeyImportFailure {
    /// Not a private key the app reads.
    Unrecognized,
    /// A private key, but cut short or garbled.
    Damaged,
    /// Behind a passphrase, and none was given.
    NeedsPassphrase,
    /// The passphrase doesn't open it.
    WrongPassphrase,
    /// A FIDO security key: its private half is on the token.
    SecurityKey,
    /// A kind of key that can't sign in ("DSA", "Ed448", …).
    Unsupported { kind: String },
    /// Encrypted with a scheme the app can't undo ("PKCS #12 PBE", …).
    UnsupportedEncryption { scheme: String },
    /// Making or encoding the key failed.
    Failed { message: String },
}

impl std::fmt::Display for KeyImportFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&ImportError::from(self.clone()).to_string())
    }
}

impl std::error::Error for KeyImportFailure {}

impl From<ImportError> for KeyImportFailure {
    fn from(error: ImportError) -> Self {
        match error {
            ImportError::Unrecognized => Self::Unrecognized,
            ImportError::Damaged => Self::Damaged,
            ImportError::NeedsPassphrase => Self::NeedsPassphrase,
            ImportError::WrongPassphrase => Self::WrongPassphrase,
            ImportError::SecurityKey => Self::SecurityKey,
            ImportError::Unsupported(kind) => Self::Unsupported { kind },
            ImportError::UnsupportedEncryption(scheme) => Self::UnsupportedEncryption { scheme },
            ImportError::Failed(message) => Self::Failed { message },
        }
    }
}

impl From<KeyImportFailure> for ImportError {
    fn from(failure: KeyImportFailure) -> Self {
        match failure {
            KeyImportFailure::Unrecognized => Self::Unrecognized,
            KeyImportFailure::Damaged => Self::Damaged,
            KeyImportFailure::NeedsPassphrase => Self::NeedsPassphrase,
            KeyImportFailure::WrongPassphrase => Self::WrongPassphrase,
            KeyImportFailure::SecurityKey => Self::SecurityKey,
            KeyImportFailure::Unsupported { kind } => Self::Unsupported(kind),
            KeyImportFailure::UnsupportedEncryption { scheme } => {
                Self::UnsupportedEncryption(scheme)
            }
            KeyImportFailure::Failed { message } => Self::Failed(message),
        }
    }
}

/// A key ready for the keychain, and what its record shows.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ImportedKey {
    /// What the record calls it, as stored (`SSHKeyAlgorithm`'s raw value):
    /// "ED25519", "RSA-4096" (every RSA key; its size is `bits`), "ECDSA".
    pub algorithm: String,
    /// The unencrypted `openssh-key-v1` text the keychain keeps.
    pub openssh: String,
    /// The `authorized_keys` line, ending in the comment when there is one.
    pub public_key_line: String,
    /// `SHA256:…`, as `ssh-keygen -l` prints it.
    pub fingerprint: String,
    /// An RSA key's modulus size.
    pub bits: Option<i64>,
}

impl std::fmt::Debug for ImportedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportedKey")
            .field("algorithm", &self.algorithm)
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl From<keyfile::StoredKey> for ImportedKey {
    fn from(key: keyfile::StoredKey) -> Self {
        ImportedKey {
            algorithm: key.record_algorithm().raw_value().to_owned(),
            openssh: key.openssh.to_string(),
            public_key_line: key.info.public_key.clone(),
            fingerprint: key.info.fingerprint.clone(),
            bits: key.bits,
        }
    }
}

/// What to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum KeyGeneration {
    Ed25519,
    /// RSA with this many bits (the app makes 4096).
    Rsa {
        bits: u32,
    },
}

/// What to do with a key as the keychain holds it.
#[derive(Clone, PartialEq, Eq, uniffi::Enum)]
pub enum KeyAssessment {
    /// Every connect path reads it as stored.
    Usable,
    /// The same key in the form every connect path reads; replace the
    /// stored one with it (already checked to be the same key).
    Converted { openssh: String },
    /// Behind a passphrase, which the person has to give once.
    NeedsPassphrase,
}

impl std::fmt::Debug for KeyAssessment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Usable => "Usable",
            Self::Converted { .. } => "Converted",
            Self::NeedsPassphrase => "NeedsPassphrase",
        })
    }
}

/// A Secure Enclave key's record facts.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HardwarePublicKey {
    pub public_key_line: String,
    pub fingerprint: String,
}

/// The NIST curve of an ECDSA key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum KeyCurve {
    P256,
    P384,
    P521,
}

/// A software key's private numbers, for the Swift connect path's signers
/// (CryptoKit, Security) — big-endian; an ECDSA scalar at its curve's full
/// width, the rest without leading zeros.
#[derive(Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SigningParts {
    Ed25519 {
        seed: Vec<u8>,
    },
    Ecdsa {
        curve: KeyCurve,
        scalar: Vec<u8>,
    },
    Rsa {
        n: Vec<u8>,
        e: Vec<u8>,
        d: Vec<u8>,
        p: Vec<u8>,
        q: Vec<u8>,
    },
}

impl std::fmt::Debug for SigningParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ed25519 { .. } => "SigningParts::Ed25519",
            Self::Ecdsa { .. } => "SigningParts::Ecdsa",
            Self::Rsa { .. } => "SigningParts::Rsa",
        })
    }
}

/// Reads a pasted or picked private key (`passphrase` when it has one; an
/// empty one is none) into the form the keychain keeps. `comment` ends the
/// public key line; left empty, a pasted `.pub` line's comment stays, else
/// the key file's own.
#[uniffi::export]
pub fn ssh_key_import(
    text: String,
    passphrase: Option<String>,
    public_key_line: Option<String>,
    comment: String,
) -> Result<ImportedKey, KeyImportFailure> {
    Ok(keyfile::import(
        &text,
        passphrase.as_deref(),
        public_key_line.as_deref(),
        &comment,
    )?
    .into())
}

/// A new key pair, its comment `comment`.
#[uniffi::export]
pub fn ssh_key_generate(
    kind: KeyGeneration,
    comment: String,
) -> Result<ImportedKey, KeyImportFailure> {
    let kind = match kind {
        KeyGeneration::Ed25519 => Generate::Ed25519,
        KeyGeneration::Rsa { bits } => Generate::Rsa(bits as usize),
    };
    Ok(keyfile::generate(kind, &comment)?.into())
}

/// Whether the pasted text is a private key behind a passphrase (the form
/// asks for one then).
#[uniffi::export]
pub fn ssh_key_is_encrypted(text: String) -> bool {
    keyfile::is_encrypted(&text)
}

/// What to do with a stored key before a connect (B125): use it, replace it
/// with the converted key, or ask for its passphrase.
#[uniffi::export]
pub fn ssh_key_assess(
    stored: String,
    passphrase: Option<String>,
) -> Result<KeyAssessment, KeyImportFailure> {
    Ok(match keyfile::assess(&stored, passphrase.as_deref())? {
        Assessment::Usable => KeyAssessment::Usable,
        Assessment::Converted(openssh) => KeyAssessment::Converted {
            openssh: openssh.to_string(),
        },
        Assessment::NeedsPassphrase => KeyAssessment::NeedsPassphrase,
    })
}

/// What to keep for a key pasted into a connection's own form (B133).
#[uniffi::export]
pub fn ssh_key_storable(pasted: String) -> String {
    keyfile::storable(&pasted).to_string()
}

/// What a stored secret is, when it is an unencrypted private key that
/// signs in (for a keychain item no record names); `None` otherwise.
#[uniffi::export]
pub fn ssh_key_describe_stored(text: String) -> Option<ImportedKey> {
    keyfile::describe_stored(&text).map(Into::into)
}

/// The kind of a saved key that can't sign in, from its public key line
/// ("DSA"); `None` when it signs in or the line doesn't say.
#[uniffi::export]
pub fn ssh_key_refused_type(public_key_line: String) -> Option<String> {
    keyfile::refused_type(&public_key_line)
}

/// The modulus size of the `ssh-rsa` key on an `authorized_keys` line.
#[uniffi::export]
pub fn ssh_key_rsa_bits(public_key_line: String) -> Option<i64> {
    moshpit_store::ssh_keys::rsa_bits_of_public_key_line(&public_key_line)
}

/// `SHA256:…` of an SSH public key blob.
#[uniffi::export]
pub fn ssh_key_fingerprint(public_blob: Vec<u8>) -> String {
    moshpit_ssh::host_key::fingerprint_of_blob(&public_blob)
}

/// The SSH public key blob of the private key in `text` (unencrypted, any
/// form import reads).
#[uniffi::export]
pub fn ssh_key_public_blob(text: String) -> Result<Vec<u8>, KeyImportFailure> {
    Ok(keyfile::read(&text, None)?.public_blob())
}

/// A Secure Enclave key's `authorized_keys` line and fingerprint, from the
/// public half CryptoKit hands over (`x963Representation`).
#[uniffi::export]
pub fn ssh_key_hardware_public_key(
    public_key: Vec<u8>,
    comment: String,
) -> Result<HardwarePublicKey, KeyImportFailure> {
    let (public_key_line, fingerprint) = keyfile::hardware_public_key(&public_key, &comment)?;
    Ok(HardwarePublicKey {
        public_key_line,
        fingerprint,
    })
}

/// The private numbers of the unencrypted key in `text`, for a signer on
/// the platform (the Swift connect path's).
#[uniffi::export]
pub fn ssh_key_signing_parts(text: String) -> Result<SigningParts, KeyImportFailure> {
    Ok(match keyfile::signing_key(&text)? {
        SigningKey::Ed25519 { seed } => SigningParts::Ed25519 {
            seed: seed.to_vec(),
        },
        SigningKey::Ecdsa { curve, scalar } => SigningParts::Ecdsa {
            curve: match curve {
                EcdsaCurve::P256 => KeyCurve::P256,
                EcdsaCurve::P384 => KeyCurve::P384,
                EcdsaCurve::P521 => KeyCurve::P521,
            },
            scalar: scalar.to_vec(),
        },
        SigningKey::Rsa { n, e, d, p, q } => SigningParts::Rsa {
            n,
            e,
            d: d.to_vec(),
            p: p.to_vec(),
            q: q.to_vec(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_key_crosses_whole_and_imports_back() {
        let made = ssh_key_generate(KeyGeneration::Ed25519, "phone".into()).unwrap();
        assert_eq!(made.algorithm, "ED25519");
        assert!(made.public_key_line.ends_with(" phone"));
        assert!(!ssh_key_is_encrypted(made.openssh.clone()));
        assert_eq!(
            ssh_key_assess(made.openssh.clone(), None).unwrap(),
            KeyAssessment::Usable
        );
        let again = ssh_key_import(made.openssh.clone(), None, None, String::new()).unwrap();
        assert_eq!(again.fingerprint, made.fingerprint);
        assert_eq!(
            ssh_key_describe_stored(made.openssh.clone())
                .unwrap()
                .fingerprint,
            made.fingerprint
        );
        assert_eq!(
            ssh_key_fingerprint(ssh_key_public_blob(made.openssh.clone()).unwrap()),
            made.fingerprint
        );
        assert!(matches!(
            ssh_key_signing_parts(made.openssh).unwrap(),
            SigningParts::Ed25519 { seed } if seed.len() == 32
        ));
        assert!(
            !format!("{again:?}").contains("BEGIN"),
            "never prints the key"
        );
    }

    #[test]
    fn refusals_cross_with_what_they_name() {
        assert_eq!(
            ssh_key_import("hello".into(), None, None, String::new()).unwrap_err(),
            KeyImportFailure::Unrecognized
        );
        assert_eq!(
            KeyImportFailure::from(ImportError::Unsupported("DSA".into())),
            KeyImportFailure::Unsupported { kind: "DSA".into() }
        );
        assert!(
            KeyImportFailure::NeedsPassphrase
                .to_string()
                .contains("passphrase")
        );
        assert_eq!(
            ssh_key_refused_type("ssh-dss AAAA".into()).as_deref(),
            Some("DSA")
        );
        assert_eq!(
            ssh_key_refused_type("ecdsa-sha2-nistp384 AAAA".into()),
            None
        );
        assert_eq!(ssh_key_storable("not a key".into()), "not a key");
    }
}
