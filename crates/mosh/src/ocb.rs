//! AES-128-OCB3 per RFC 7253 with a 128-bit tag and a 96-bit nonce (the
//! AEAD_AES_128_OCB_TAGLEN128 parameter set), the cipher of mosh's datagram
//! layer, which always passes empty associated data.
//!
//! The mode is RustCrypto's `ocb3` crate over its `aes` (hardware AES where
//! the CPU has it). This module gives it the shape of the app's Swift `OCB3`
//! (`Moshpit/Services/Mosh/OCB3.swift`): byte slices checked at run time,
//! `seal` returning `ciphertext || tag`, `open` taking that, and the same
//! errors in the same order. Both implementations are checked against the
//! RFC 7253 vectors and against each other (`harness/`).

use aes::Aes128;
use ocb3::aead::array::Array;
use ocb3::{AeadInOut, KeyInit};

/// AES-128 key length.
pub const KEY_LEN: usize = 16;
/// OCB nonce length mosh uses.
pub const NONCE_LEN: usize = 12;
/// Full 128-bit tag.
pub const TAG_LEN: usize = 16;

/// Why a seal or open failed. The first four are the Swift `OCB3Error` cases
/// that can occur (its `cipherFailure` wraps a CommonCrypto status that has no
/// counterpart here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcbError {
    InvalidKeyLength,
    InvalidNonceLength,
    CiphertextTooShort,
    AuthenticationFailed,
    /// The message or associated data is 1 GiB or longer, beyond the
    /// precomputed table of the `ocb3` crate. A mosh datagram is at most a
    /// few kilobytes.
    MessageTooLong,
}

impl std::fmt::Display for OcbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            OcbError::InvalidKeyLength => "OCB3 key must be 16 bytes",
            OcbError::InvalidNonceLength => "OCB3 nonce must be 12 bytes",
            OcbError::CiphertextTooShort => "OCB3 ciphertext is shorter than its tag",
            OcbError::AuthenticationFailed => "OCB3 authentication failed",
            OcbError::MessageTooLong => "OCB3 message too long",
        };
        f.write_str(text)
    }
}

impl std::error::Error for OcbError {}

/// An AES-128-OCB3 key schedule. Cheap to clone; build once per session.
#[derive(Clone)]
pub struct Ocb3 {
    inner: ocb3::Ocb3<Aes128>,
}

impl std::fmt::Debug for Ocb3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ocb3 { .. }")
    }
}

impl Ocb3 {
    /// - `key`: the 16-byte AES-128 key.
    pub fn new(key: &[u8]) -> Result<Self, OcbError> {
        let key: [u8; KEY_LEN] = key.try_into().map_err(|_| OcbError::InvalidKeyLength)?;
        Ok(Self {
            inner: ocb3::Ocb3::new(&Array::from(key)),
        })
    }

    /// Encrypts and authenticates `plaintext`, returning `ciphertext || tag`.
    /// `nonce` must be 12 bytes and unique per key and direction.
    pub fn seal(
        &self,
        plaintext: &[u8],
        nonce: &[u8],
        associated_data: &[u8],
    ) -> Result<Vec<u8>, OcbError> {
        let nonce = nonce_array(nonce)?;
        let mut out = Vec::with_capacity(plaintext.len() + TAG_LEN);
        out.extend_from_slice(plaintext);
        self.seal_in_place(&nonce, associated_data, &mut out, 0)?;
        Ok(out)
    }

    /// Encrypts `buffer[start..]` in place and appends the tag. The bytes
    /// before `start` are left alone (the datagram's clear nonce goes there).
    pub fn seal_in_place(
        &self,
        nonce: &[u8; NONCE_LEN],
        associated_data: &[u8],
        buffer: &mut Vec<u8>,
        start: usize,
    ) -> Result<(), OcbError> {
        let tag = self
            .inner
            .encrypt_inout_detached(
                &Array::from(*nonce),
                associated_data,
                (&mut buffer[start..]).into(),
            )
            .map_err(|_| OcbError::MessageTooLong)?;
        buffer.extend_from_slice(&tag);
        Ok(())
    }

    /// Decrypts and verifies `ciphertext || tag`. No plaintext comes back
    /// unless the tag verifies.
    pub fn open(
        &self,
        ciphertext: &[u8],
        nonce: &[u8],
        associated_data: &[u8],
    ) -> Result<Vec<u8>, OcbError> {
        let nonce = nonce_array(nonce)?;
        if ciphertext.len() < TAG_LEN {
            return Err(OcbError::CiphertextTooShort);
        }
        let mut out = ciphertext.to_vec();
        self.open_in_place(&nonce, associated_data, &mut out)?;
        Ok(out)
    }

    /// Decrypts `buffer` (`ciphertext || tag`) in place and truncates it to
    /// the plaintext. On failure the buffer holds garbage.
    pub fn open_in_place(
        &self,
        nonce: &[u8; NONCE_LEN],
        associated_data: &[u8],
        buffer: &mut Vec<u8>,
    ) -> Result<(), OcbError> {
        let Some(body_len) = buffer.len().checked_sub(TAG_LEN) else {
            return Err(OcbError::CiphertextTooShort);
        };
        let (body, tag) = buffer.split_at_mut(body_len);
        let tag: [u8; TAG_LEN] = (&*tag).try_into().expect("tag is TAG_LEN bytes");
        self.inner
            .decrypt_inout_detached(
                &Array::from(*nonce),
                associated_data,
                body.into(),
                &Array::from(tag),
            )
            .map_err(|_| OcbError::AuthenticationFailed)?;
        buffer.truncate(body_len);
        Ok(())
    }
}

fn nonce_array(nonce: &[u8]) -> Result<[u8; NONCE_LEN], OcbError> {
    nonce.try_into().map_err(|_| OcbError::InvalidNonceLength)
}
