//! `openssh-key-v1`: what `ssh-keygen` writes by default, and the one form
//! the keychain keeps.
//!
//! "openssh-key-v1\0", cipher, kdf, kdf options, key count, public key,
//! private section (enciphered unless the cipher is "none"), and for AEAD
//! ciphers the tag after it. The public key is always in the clear.

use russh::keys::ssh_key::Cipher;
use russh::keys::ssh_key::private::KeypairData;
use zeroize::Zeroizing;

use super::material::{self, Curve};
use super::{ImportError, refusal};

/// The most bcrypt rounds a key is opened with. `ssh-keygen -a` defaults to
/// 16 and people pick a few hundred; each round is a bcrypt hash, so a file
/// asking for millions (a garbled one, or one made to) would hold the
/// import for hours — in the Swift reader it did. Above this the key is
/// refused, saying how to re-encrypt it.
pub(super) const MAX_BCRYPT_ROUNDS: u32 = 4096;

pub(super) struct Container<'a> {
    pub cipher: String,
    kdf: String,
    kdf_options: &'a [u8],
    pub public_blob: &'a [u8],
    section: &'a [u8],
    trailer: &'a [u8],
}

impl<'a> Container<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Container<'a>, ImportError> {
        let mut reader = Wire(data);
        let damaged = ImportError::Damaged;
        if reader.bytes(15) != Some(b"openssh-key-v1\0") {
            return Err(damaged);
        }
        let cipher = reader.text().ok_or(ImportError::Damaged)?;
        let kdf = reader.text().ok_or(ImportError::Damaged)?;
        let kdf_options = reader.string().ok_or(ImportError::Damaged)?;
        let keys = reader.uint32().ok_or(ImportError::Damaged)?;
        let public_blob = reader.string().ok_or(ImportError::Damaged)?;
        let section = reader.string().ok_or(ImportError::Damaged)?;
        // ssh-keygen only ever writes one.
        if keys != 1 {
            return Err(ImportError::Unrecognized);
        }
        Ok(Container {
            cipher,
            kdf,
            kdf_options,
            public_blob,
            section,
            trailer: reader.0,
        })
    }

    pub fn is_encrypted(&self) -> bool {
        self.cipher != "none"
    }

    /// The key type the clear public half names, if it is one that signs in
    /// from here — known before anything is asked for: a security key or a
    /// DSA key is no use whatever the passphrase.
    pub fn key_type(&self) -> Result<String, ImportError> {
        let key_type = Wire(self.public_blob).text().ok_or(ImportError::Damaged)?;
        match refusal(&key_type) {
            Some(refused) => Err(refused),
            None => Ok(key_type),
        }
    }

    /// The private key, and the comment the file carries.
    pub fn read(&self, passphrase: Option<&[u8]>) -> Result<(KeypairData, String), ImportError> {
        let key_type = self.key_type()?;
        let plain = self.decrypted_section(passphrase)?;
        let mut reader = Wire(&plain);
        let (Some(check1), Some(check2)) = (reader.uint32(), reader.uint32()) else {
            return Err(ImportError::Damaged);
        };
        // The one place a wrong passphrase shows for the CTR and CBC ciphers.
        if check1 != check2 {
            return Err(if self.is_encrypted() {
                ImportError::WrongPassphrase
            } else {
                ImportError::Damaged
            });
        }
        if reader.text().as_deref() != Some(key_type.as_str()) {
            return Err(ImportError::Damaged);
        }
        let key = match key_type.as_str() {
            "ssh-ed25519" => {
                let (Some(public), Some(pair)) = (reader.string(), reader.string()) else {
                    return Err(ImportError::Damaged);
                };
                if pair.len() != 64 || &pair[32..] != public {
                    return Err(ImportError::Damaged);
                }
                material::ed25519(&pair[..32])?
            }
            "ssh-rsa" => {
                let mut next = || reader.string().ok_or(ImportError::Damaged);
                let (n, e, d) = (next()?, next()?, next()?);
                let (_iqmp, p, q) = (next()?, next()?, next()?);
                material::rsa(n, e, d, p, q)?
            }
            other => {
                let curve = Curve::from_key_type(other).ok_or(ImportError::Damaged)?;
                if reader.text().as_deref() != Some(curve.ssh_id()) {
                    return Err(ImportError::Damaged);
                }
                // The point, derived again from the scalar below.
                reader.string().ok_or(ImportError::Damaged)?;
                let scalar = reader.string().ok_or(ImportError::Damaged)?;
                material::ecdsa(curve, scalar)?
            }
        };
        let comment = reader.text().unwrap_or_default();
        // The public half in the clear must be this private key's.
        if material::public_blob(&key)? != self.public_blob {
            return Err(ImportError::Damaged);
        }
        Ok((key, comment))
    }

    fn decrypted_section(
        &self,
        passphrase: Option<&[u8]>,
    ) -> Result<Zeroizing<Vec<u8>>, ImportError> {
        if !self.is_encrypted() {
            if self.kdf != "none" {
                return Err(ImportError::Damaged);
            }
            return Ok(Zeroizing::new(self.section.to_vec()));
        }
        if self.kdf != "bcrypt" {
            return Err(ImportError::UnsupportedEncryption(self.kdf.clone()));
        }
        let cipher = SectionCipher::named(&self.cipher)
            .ok_or_else(|| ImportError::UnsupportedEncryption(self.cipher.clone()))?;
        let passphrase = passphrase.ok_or(ImportError::NeedsPassphrase)?;
        let mut options = Wire(self.kdf_options);
        let (Some(salt), Some(rounds)) = (options.string(), options.uint32()) else {
            return Err(ImportError::Damaged);
        };
        if rounds < 1
            || !self.section.len().is_multiple_of(cipher.block_size())
            || self.trailer.len() < cipher.tag_len()
        {
            return Err(ImportError::Damaged);
        }
        if rounds > MAX_BCRYPT_ROUNDS {
            return Err(ImportError::UnsupportedEncryption(format!(
                "bcrypt with {rounds} rounds"
            )));
        }
        let mut derived = Zeroizing::new(vec![0u8; cipher.key_len() + cipher.iv_len()]);
        bcrypt_pbkdf::bcrypt_pbkdf(passphrase, salt, rounds, &mut derived)
            .map_err(|_| ImportError::Damaged)?;
        let (key, iv) = derived.split_at(cipher.key_len());
        let mut plain = Zeroizing::new(self.section.to_vec());
        cipher.decrypt(key, iv, &mut plain, &self.trailer[..cipher.tag_len()])?;
        Ok(plain)
    }
}

/// The ciphers `ssh-keygen -Z` offers: AES in CTR, CBC and GCM, and
/// ChaCha20-Poly1305 (3DES-CBC is refused by name).
#[derive(Clone, Copy)]
enum SectionCipher {
    Aes(Cipher),
    ChaChaPoly,
}

impl SectionCipher {
    fn named(name: &str) -> Option<SectionCipher> {
        match name {
            "aes128-ctr"
            | "aes192-ctr"
            | "aes256-ctr"
            | "aes128-cbc"
            | "aes192-cbc"
            | "aes256-cbc"
            | "aes128-gcm@openssh.com"
            | "aes256-gcm@openssh.com" => Cipher::new(name).ok().map(SectionCipher::Aes),
            "chacha20-poly1305@openssh.com" => Some(SectionCipher::ChaChaPoly),
            _ => None,
        }
    }

    /// OpenSSH's key length: for ChaCha20-Poly1305 two 256-bit keys, the
    /// second (the packet-length key) unused here.
    fn key_len(self) -> usize {
        match self {
            SectionCipher::Aes(cipher) => cipher.key_and_iv_size().map_or(0, |(k, _)| k),
            SectionCipher::ChaChaPoly => 64,
        }
    }

    /// ChaCha20-Poly1305's nonce is the packet number, 0: no IV derived.
    fn iv_len(self) -> usize {
        match self {
            SectionCipher::Aes(cipher) => cipher.key_and_iv_size().map_or(0, |(_, iv)| iv),
            SectionCipher::ChaChaPoly => 0,
        }
    }

    fn block_size(self) -> usize {
        match self {
            SectionCipher::Aes(cipher) => cipher.block_size(),
            SectionCipher::ChaChaPoly => 8,
        }
    }

    fn tag_len(self) -> usize {
        match self {
            SectionCipher::Aes(cipher) if cipher.has_tag() => 16,
            SectionCipher::Aes(_) => 0,
            SectionCipher::ChaChaPoly => 16,
        }
    }

    /// Deciphers `buffer` in place. An AEAD tag that doesn't check is the
    /// only sign of a wrong passphrase there.
    fn decrypt(
        self,
        key: &[u8],
        iv: &[u8],
        buffer: &mut [u8],
        tag: &[u8],
    ) -> Result<(), ImportError> {
        let tag_of = |tag: &[u8]| russh::keys::ssh_key::cipher::Tag::try_from(tag).ok();
        match self {
            SectionCipher::Aes(cipher) if cipher.has_tag() => cipher
                .decrypt(key, iv, buffer, tag_of(tag))
                .map_err(|_| ImportError::WrongPassphrase),
            SectionCipher::Aes(cipher) => cipher
                .decrypt(key, iv, buffer, None)
                .map_err(|_| ImportError::Damaged),
            SectionCipher::ChaChaPoly => Cipher::ChaCha20Poly1305
                .decrypt(&key[..32], &[0; 8], buffer, tag_of(tag))
                .map_err(|_| ImportError::WrongPassphrase),
        }
    }
}

/// RFC 4251 framing, read in order; a short read is `None`.
pub(super) struct Wire<'a>(pub &'a [u8]);

impl<'a> Wire<'a> {
    pub fn bytes(&mut self, count: usize) -> Option<&'a [u8]> {
        if self.0.len() < count {
            return None;
        }
        let (head, rest) = self.0.split_at(count);
        self.0 = rest;
        Some(head)
    }

    pub fn uint32(&mut self) -> Option<u32> {
        self.bytes(4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn string(&mut self) -> Option<&'a [u8]> {
        let len = self.uint32()?;
        self.bytes(usize::try_from(len).ok()?)
    }

    pub fn text(&mut self) -> Option<String> {
        self.string()
            .and_then(|b| std::str::from_utf8(b).ok())
            .map(str::to_owned)
    }
}
