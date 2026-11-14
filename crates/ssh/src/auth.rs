//! Signing in: a password, a private key, or a hardware key that signs
//! through the platform.
//!
//! Private keys arrive as the keychain holds them: unencrypted
//! `openssh-key-v1` text (the app converts other formats, and asks for a
//! passphrase, when it imports a key). The text and the parsed key are wiped
//! when dropped, and neither is ever logged. A hardware key (the Secure
//! Enclave, Android Keystore) never enters Rust at all: the connection holds
//! its public half and asks [`KeySigner::sign`] for each signature.

use std::fmt;
use std::sync::Arc;

use russh::keys::signature::Verifier;
use russh::keys::ssh_encoding::{Encode, Mpint};
use russh::keys::ssh_key::Signature;
use russh::keys::{Algorithm, EcdsaCurve, HashAlg, PrivateKey, PublicKey};
use zeroize::Zeroizing;

use crate::error::{KeyError, SignerError};
use crate::host_key::BoxFuture;
use crate::waiting::Waiting;

/// How to prove who we are.
pub enum Auth {
    Password(Zeroizing<String>),
    /// An unencrypted `openssh-key-v1` private key, as text.
    Key(Zeroizing<String>),
    Hardware(HardwareKey),
}

impl Auth {
    pub fn password(password: impl Into<String>) -> Self {
        Self::Password(Zeroizing::new(password.into()))
    }

    pub fn key(openssh_text: impl Into<String>) -> Self {
        Self::Key(Zeroizing::new(openssh_text.into()))
    }
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Auth::Password(_) => f.write_str("Auth::Password"),
            Auth::Key(_) => f.write_str("Auth::Key"),
            Auth::Hardware(key) => write!(f, "Auth::Hardware({})", key.key_ref),
        }
    }
}

/// A key whose private half stays in the hardware.
#[derive(Clone)]
pub struct HardwareKey {
    /// The public half, as the key record's `authorized_keys` line holds it.
    pub public_key: PublicKey,
    /// What the platform knows the key by (a keychain reference).
    pub key_ref: String,
    pub signer: Arc<dyn KeySigner>,
}

impl HardwareKey {
    /// From the key record's `authorized_keys` line
    /// (`ecdsa-sha2-nistp256 AAAA… comment`).
    pub fn new(
        public_key_line: &str,
        key_ref: impl Into<String>,
        signer: Arc<dyn KeySigner>,
    ) -> Result<Self, KeyError> {
        let public_key = PublicKey::from_openssh(public_key_line.trim())
            .map_err(|e| KeyError::BadPublicKey(e.to_string()))?;
        match public_key.algorithm() {
            Algorithm::Ecdsa { .. } | Algorithm::Ed25519 => {}
            other => return Err(KeyError::Unsupported(other.as_str().to_owned())),
        }
        Ok(Self {
            public_key,
            key_ref: key_ref.into(),
            signer,
        })
    }
}

/// A signature as the platform makes it.
#[derive(Clone, PartialEq, Eq)]
pub enum HostSignature {
    /// ECDSA as ASN.1 DER (`SEQUENCE { r INTEGER, s INTEGER }`): CryptoKit's
    /// `derRepresentation`, Android's `Signature.sign()`.
    Der(Vec<u8>),
    /// Fixed width: ECDSA `r ‖ s` (CryptoKit's `rawRepresentation`, 64 bytes
    /// for P-256), or the 64 bytes of an Ed25519 signature.
    Raw(Vec<u8>),
}

impl fmt::Debug for HostSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostSignature::Der(bytes) => write!(f, "HostSignature::Der({} bytes)", bytes.len()),
            HostSignature::Raw(bytes) => write!(f, "HostSignature::Raw({} bytes)", bytes.len()),
        }
    }
}

/// The platform side of a hardware key: `sign(key_ref, data)`.
pub trait KeySigner: Send + Sync {
    /// Signs `data` with the key `key_ref` names: ECDSA with the curve's
    /// hash (SHA-256 for P-256, as `SecureEnclave.P256.Signing.PrivateKey.
    /// signature(for:)` does), or Ed25519. May wait on the person (Face ID);
    /// that time doesn't count against the handshake.
    fn sign<'a>(
        &'a self,
        key_ref: &'a str,
        data: &'a [u8],
    ) -> BoxFuture<'a, Result<HostSignature, SignerError>>;
}

/// What kind of key a private key is, from its public half.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyKind {
    Ed25519,
    Rsa,
    EcdsaP256,
    EcdsaP384,
    EcdsaP521,
    /// FIDO security keys: their private half is on the token.
    SkEcdsaP256,
    SkEd25519,
    Dsa,
    Other(String),
}

impl KeyKind {
    pub(crate) fn of(algorithm: &Algorithm) -> Self {
        match algorithm {
            Algorithm::Ed25519 => KeyKind::Ed25519,
            Algorithm::Rsa { .. } => KeyKind::Rsa,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            } => KeyKind::EcdsaP256,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP384,
            } => KeyKind::EcdsaP384,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP521,
            } => KeyKind::EcdsaP521,
            Algorithm::SkEcdsaSha2NistP256 => KeyKind::SkEcdsaP256,
            Algorithm::SkEd25519 => KeyKind::SkEd25519,
            Algorithm::Dsa => KeyKind::Dsa,
            other => KeyKind::Other(other.as_str().to_owned()),
        }
    }

    /// The SSH name of the key type: `ssh-ed25519`, `ssh-rsa`, …
    pub fn ssh_name(&self) -> &str {
        match self {
            KeyKind::Ed25519 => "ssh-ed25519",
            KeyKind::Rsa => "ssh-rsa",
            KeyKind::EcdsaP256 => "ecdsa-sha2-nistp256",
            KeyKind::EcdsaP384 => "ecdsa-sha2-nistp384",
            KeyKind::EcdsaP521 => "ecdsa-sha2-nistp521",
            KeyKind::SkEcdsaP256 => "sk-ecdsa-sha2-nistp256@openssh.com",
            KeyKind::SkEd25519 => "sk-ssh-ed25519@openssh.com",
            KeyKind::Dsa => "ssh-dss",
            KeyKind::Other(name) => name,
        }
    }

    /// Whether a key of this kind, held as text, can sign in from here.
    pub fn can_sign_in(&self) -> bool {
        matches!(
            self,
            KeyKind::Ed25519
                | KeyKind::Rsa
                | KeyKind::EcdsaP256
                | KeyKind::EcdsaP384
                | KeyKind::EcdsaP521
        )
    }
}

/// The public facts of a stored private key: what the key list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    pub kind: KeyKind,
    /// The `authorized_keys` line, with the key's own comment when it has one.
    pub public_key: String,
    /// `SHA256:…`, as `ssh-keygen -l` prints it.
    pub fingerprint: String,
    pub comment: String,
    pub encrypted: bool,
}

/// Reads the public facts of an `openssh-key-v1` private key. Works on an
/// encrypted key too: its public half is in the clear.
pub fn inspect_private_key(text: &str) -> Result<KeyInfo, KeyError> {
    crate::keyfile::inspect_openssh(text)
}

/// The private key a connection signs with. Refuses what can't sign in:
/// another container, an encrypted key, a kind we can't use. Read by the
/// same reader import uses (`keyfile`), whitespace anywhere forgiven.
pub(crate) fn load_private_key(text: &str) -> Result<PrivateKey, KeyError> {
    crate::keyfile::openssh_private_key(text)
}

/// The SSH signature blob (`string algorithm, string signature`) for what a
/// hardware key answered, checked against the public half before it goes
/// anywhere: a server's "no" can't tell a broken pairing from a wrong key.
pub(crate) fn ssh_signature(
    public_key: &PublicKey,
    data: &[u8],
    signature: HostSignature,
) -> Result<Vec<u8>, SignerError> {
    let algorithm = public_key.algorithm();
    let raw = match (&algorithm, signature) {
        (Algorithm::Ecdsa { curve }, HostSignature::Der(der)) => {
            let (r, s) = parse_der_signature(&der)?;
            ecdsa_blob(*curve, &r, &s)?
        }
        (Algorithm::Ecdsa { curve }, HostSignature::Raw(raw)) => {
            let width = field_bytes(*curve);
            if raw.len() != 2 * width {
                return Err(SignerError::Malformed(format!(
                    "{} bytes for {}, expected {}",
                    raw.len(),
                    curve.as_str(),
                    2 * width
                )));
            }
            ecdsa_blob(*curve, &raw[..width], &raw[width..])?
        }
        (Algorithm::Ed25519, HostSignature::Raw(raw)) if raw.len() == 64 => raw,
        (Algorithm::Ed25519, other) => {
            return Err(SignerError::Malformed(format!(
                "an Ed25519 signature is 64 raw bytes, got {other:?}"
            )));
        }
        (other, _) => return Err(SignerError::Unsupported(other.as_str().to_owned())),
    };
    let signature =
        Signature::new(algorithm, raw).map_err(|e| SignerError::Malformed(e.to_string()))?;
    <PublicKey as Verifier<Signature>>::verify(public_key, data, &signature)
        .map_err(|_| SignerError::Mismatch)?;
    let mut blob = Vec::new();
    signature
        .encode(&mut blob)
        .map_err(|e| SignerError::Malformed(e.to_string()))?;
    Ok(blob)
}

fn field_bytes(curve: EcdsaCurve) -> usize {
    match curve {
        EcdsaCurve::NistP256 => 32,
        EcdsaCurve::NistP384 => 48,
        EcdsaCurve::NistP521 => 66,
    }
}

/// `mpint r, mpint s`: the inside of an `ecdsa-sha2-*` signature.
fn ecdsa_blob(curve: EcdsaCurve, r: &[u8], s: &[u8]) -> Result<Vec<u8>, SignerError> {
    let width = field_bytes(curve);
    let mut out = Vec::with_capacity(2 * width + 10);
    for part in [r, s] {
        let part = strip_zeros(part);
        if part.is_empty() || part.len() > width {
            return Err(SignerError::Malformed(format!(
                "an integer of {} bytes for {}",
                part.len(),
                curve.as_str()
            )));
        }
        Mpint::from_positive_bytes(part)
            .encode(&mut out)
            .map_err(|e| SignerError::Malformed(e.to_string()))?;
    }
    Ok(out)
}

fn strip_zeros(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    &bytes[start..]
}

/// `SEQUENCE { INTEGER r, INTEGER s }`, strict DER.
fn parse_der_signature(der: &[u8]) -> Result<(Vec<u8>, Vec<u8>), SignerError> {
    let bad = |why: &str| SignerError::Malformed(format!("DER: {why}"));
    let (tag, body, rest) = der_element(der).ok_or_else(|| bad("truncated"))?;
    if tag != 0x30 || !rest.is_empty() {
        return Err(bad("not one SEQUENCE"));
    }
    let (tag, r, rest) = der_element(body).ok_or_else(|| bad("truncated r"))?;
    if tag != 0x02 {
        return Err(bad("r is not an INTEGER"));
    }
    let (tag, s, rest) = der_element(rest).ok_or_else(|| bad("truncated s"))?;
    if tag != 0x02 || !rest.is_empty() {
        return Err(bad("s is not the last INTEGER"));
    }
    for int in [r, s] {
        // Positive and minimal: a sign byte only before a high bit.
        if int.is_empty()
            || int[0] & 0x80 != 0
            || (int.len() > 1 && int[0] == 0 && int[1] & 0x80 == 0)
        {
            return Err(bad("an INTEGER that isn't positive and minimal"));
        }
    }
    Ok((r.to_vec(), s.to_vec()))
}

/// One DER element: tag, contents, what follows. Lengths up to 0xffff.
fn der_element(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = bytes.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = match first {
        n if n < 0x80 => (n as usize, rest),
        0x81 => {
            let (&n, rest) = rest.split_first()?;
            if n < 0x80 {
                return None;
            }
            (n as usize, rest)
        }
        0x82 => {
            let (bytes, rest) = rest.split_at_checked(2)?;
            let n = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
            if n < 0x100 {
                return None;
            }
            (n, rest)
        }
        _ => return None,
    };
    let (contents, rest) = rest.split_at_checked(len)?;
    Some((tag, contents, rest))
}

/// russh's side of a hardware key: forwards the data to sign to the
/// platform, with the handshake clock stopped, and appends the signature
/// the way an agent would.
pub(crate) struct PlatformSigner {
    pub key: HardwareKey,
    pub waiting: Waiting,
}

#[derive(Debug)]
pub(crate) enum PlatformSignError {
    Signer(SignerError),
    Ssh(russh::Error),
}

impl From<russh::SendError> for PlatformSignError {
    fn from(_: russh::SendError) -> Self {
        PlatformSignError::Ssh(russh::Error::SendError)
    }
}

impl russh::Signer for PlatformSigner {
    type Error = PlatformSignError;

    async fn auth_sign(
        &mut self,
        _key: &russh::keys::agent::AgentIdentity,
        _hash_alg: Option<HashAlg>,
        to_sign: Vec<u8>,
    ) -> Result<Vec<u8>, Self::Error> {
        let signature = self
            .waiting
            .on_person(self.key.signer.sign(&self.key.key_ref, &to_sign))
            .await
            .map_err(PlatformSignError::Signer)?;
        let blob = ssh_signature(&self.key.public_key, &to_sign, signature)
            .map_err(PlatformSignError::Signer)?;
        let mut signed = to_sign;
        blob.as_slice()
            .encode(&mut signed)
            .map_err(|e| PlatformSignError::Ssh(russh::Error::from(e)))?;
        Ok(signed)
    }
}

/// The SSH public key blob a hardware key's line names, to compare against.
pub fn public_key_blob(key: &PublicKey) -> Result<Vec<u8>, KeyError> {
    key.key_data()
        .encode_vec()
        .map_err(|e| KeyError::BadPublicKey(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENSSH_BEGIN: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";

    #[test]
    fn der_integers_must_be_positive_and_minimal() {
        // r = 1, s = 0x80 (needs its sign byte).
        let good = [0x30, 0x07, 0x02, 0x01, 0x01, 0x02, 0x02, 0x00, 0x80];
        assert_eq!(
            parse_der_signature(&good).unwrap(),
            (vec![0x01], vec![0x00, 0x80])
        );
        for bad in [
            &[0x30, 0x06, 0x02, 0x01, 0x81, 0x02, 0x01, 0x01][..], // negative r
            &[0x30, 0x07, 0x02, 0x02, 0x00, 0x01, 0x02, 0x01, 0x01][..], // padded r
            &[0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01, 0x00][..], // trailing byte
            &[0x31, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01][..], // a SET
            &[0x30, 0x81, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01][..], // long form for 6
            &[0x30, 0x08, 0x02, 0x01, 0x01][..],                   // truncated
        ] {
            assert!(parse_der_signature(bad).is_err(), "{bad:x?}");
        }
    }

    #[test]
    fn pem_bodies_read_whatever_their_lines() {
        let tidy = crate::keyfile::generate(crate::keyfile::Generate::Ed25519, "x").unwrap();
        let body: String = tidy
            .openssh
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .collect();
        let messy = format!(
            "{OPENSSH_BEGIN}\r\n  {}\r\n{} \n-----END OPENSSH PRIVATE KEY-----\r\n",
            &body[..17],
            &body[17..]
        );
        let key = load_private_key(&messy).unwrap();
        assert_eq!(
            public_key_blob(key.public_key()).unwrap(),
            public_key_blob(load_private_key(&tidy.openssh).unwrap().public_key()).unwrap()
        );
    }

    #[test]
    fn what_is_not_an_openssh_key_says_so() {
        for text in [
            "",
            "ssh-ed25519 AAAA",
            "-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----\n",
            "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
            "PuTTY-User-Key-File-3: ssh-ed25519\n",
        ] {
            assert_eq!(
                load_private_key(text).err(),
                Some(KeyError::NotOpenSsh),
                "{text:?}"
            );
        }
    }
}
