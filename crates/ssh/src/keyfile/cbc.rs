//! The CBC ciphers PEM and PKCS #8 key files are encrypted with.

use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use zeroize::Zeroizing;

use super::der::oid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BlockCipher {
    Aes128,
    Aes192,
    Aes256,
    DesEde3,
}

impl BlockCipher {
    /// As named in a PEM `DEK-Info` header.
    pub fn from_pem_name(name: &str) -> Option<Self> {
        Some(match name.to_uppercase().as_str() {
            "AES-128-CBC" => Self::Aes128,
            "AES-192-CBC" => Self::Aes192,
            "AES-256-CBC" => Self::Aes256,
            "DES-EDE3-CBC" => Self::DesEde3,
            _ => return None,
        })
    }

    /// As named by OID in PBES2 parameters.
    pub fn from_oid(id: &[u8]) -> Option<Self> {
        Some(match id {
            oid::AES128_CBC => Self::Aes128,
            oid::AES192_CBC => Self::Aes192,
            oid::AES256_CBC => Self::Aes256,
            oid::DES_EDE3_CBC => Self::DesEde3,
            _ => return None,
        })
    }

    pub fn key_len(self) -> usize {
        match self {
            Self::Aes128 => 16,
            Self::Aes192 | Self::DesEde3 => 24,
            Self::Aes256 => 32,
        }
    }

    pub fn block_size(self) -> usize {
        match self {
            Self::DesEde3 => 8,
            _ => 16,
        }
    }
}

/// `data` decrypted in CBC mode and its PKCS #7 padding checked and taken
/// off; `None` when it isn't whole blocks or the padding is wrong — which a
/// wrong passphrase nearly always makes it.
pub(super) fn decrypt_cbc_padded(
    cipher: BlockCipher,
    data: &[u8],
    key: &[u8],
    iv: &[u8],
) -> Option<Zeroizing<Vec<u8>>> {
    if data.is_empty() || !data.len().is_multiple_of(cipher.block_size()) {
        return None;
    }
    let mut buffer = Zeroizing::new(data.to_vec());
    let plain = match cipher {
        BlockCipher::Aes128 => cbc::Decryptor::<aes::Aes128>::new_from_slices(key, iv)
            .ok()?
            .decrypt_padded::<Pkcs7>(&mut buffer)
            .ok()?
            .len(),
        BlockCipher::Aes192 => cbc::Decryptor::<aes::Aes192>::new_from_slices(key, iv)
            .ok()?
            .decrypt_padded::<Pkcs7>(&mut buffer)
            .ok()?
            .len(),
        BlockCipher::Aes256 => cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv)
            .ok()?
            .decrypt_padded::<Pkcs7>(&mut buffer)
            .ok()?
            .len(),
        BlockCipher::DesEde3 => cbc::Decryptor::<des::TdesEde3>::new_from_slices(key, iv)
            .ok()?
            .decrypt_padded::<Pkcs7>(&mut buffer)
            .ok()?
            .len(),
    };
    buffer.truncate(plain);
    Some(buffer)
}
