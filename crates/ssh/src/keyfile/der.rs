//! Just enough DER for key files: definite lengths, elements read in order,
//! and the object identifiers a key file names.

use super::ImportError;

/// Reads the elements of a DER value's contents in order. Anything short
/// or malformed is [`ImportError::Damaged`].
#[derive(Clone, Copy)]
pub(super) struct Der<'a>(pub &'a [u8]);

pub(super) const OID_TAG: u8 = 0x06;

impl<'a> Der<'a> {
    pub fn is_at_end(&self) -> bool {
        self.0.is_empty()
    }

    pub fn next_tag(&self) -> Option<u8> {
        self.0.first().copied()
    }

    /// The next element: its tag and contents. Long-form lengths of one to
    /// four bytes, minimal or not.
    pub fn element(&mut self) -> Result<(u8, &'a [u8]), ImportError> {
        let [tag, first, rest @ ..] = self.0 else {
            return Err(ImportError::Damaged);
        };
        let (len, rest) = if first & 0x80 == 0 {
            (usize::from(*first), rest)
        } else {
            let count = usize::from(first & 0x7f);
            if !(1..=4).contains(&count) || rest.len() < count {
                return Err(ImportError::Damaged);
            }
            let len = rest[..count]
                .iter()
                .fold(0usize, |acc, &b| acc << 8 | usize::from(b));
            (len, &rest[count..])
        };
        if rest.len() < len {
            return Err(ImportError::Damaged);
        }
        self.0 = &rest[len..];
        Ok((*tag, &rest[..len]))
    }

    /// The next element's contents, which must have tag `tag`.
    pub fn expect(&mut self, tag: u8) -> Result<&'a [u8], ImportError> {
        match self.element()? {
            (found, contents) if found == tag => Ok(contents),
            _ => Err(ImportError::Damaged),
        }
    }

    pub fn sequence(&mut self) -> Result<Der<'a>, ImportError> {
        self.expect(0x30).map(Der)
    }

    pub fn integer(&mut self) -> Result<&'a [u8], ImportError> {
        self.expect(0x02)
    }

    pub fn octet_string(&mut self) -> Result<&'a [u8], ImportError> {
        self.expect(0x04)
    }

    pub fn oid(&mut self) -> Result<&'a [u8], ImportError> {
        self.expect(OID_TAG)
    }
}

/// `raw` without its leading zero bytes.
pub(super) fn magnitude(raw: &[u8]) -> &[u8] {
    let start = raw.iter().position(|&b| b != 0).unwrap_or(raw.len());
    &raw[start..]
}

/// A small non-negative integer; `None` past four bytes.
pub(super) fn small_integer(raw: &[u8]) -> Option<u32> {
    let bytes = magnitude(raw);
    (bytes.len() <= 4).then(|| bytes.iter().fold(0u32, |acc, &b| acc << 8 | u32::from(b)))
}

/// Object identifiers by their DER contents.
pub(super) mod oid {
    pub const RSA_ENCRYPTION: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x01];
    pub const EC_PUBLIC_KEY: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
    pub const PRIME_FIELD: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x01, 0x01];
    pub const DSA: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x38, 0x04, 0x01];
    pub const ED25519: &[u8] = &[0x2B, 0x65, 0x70];
    pub const ED448: &[u8] = &[0x2B, 0x65, 0x71];
    pub const PRIME256V1: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];
    pub const SECP384R1: &[u8] = &[0x2B, 0x81, 0x04, 0x00, 0x22];
    pub const SECP521R1: &[u8] = &[0x2B, 0x81, 0x04, 0x00, 0x23];
    pub const SECP256K1: &[u8] = &[0x2B, 0x81, 0x04, 0x00, 0x0A];
    pub const PBES2: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x05, 0x0D];
    pub const PBKDF2: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x05, 0x0C];
    pub const AES128_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x02];
    pub const AES192_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x16];
    pub const AES256_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x2A];
    pub const DES_EDE3_CBC: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x03, 0x07];
    pub const HMAC_SHA1: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02, 0x07];
    pub const HMAC_SHA224: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02, 0x08];
    pub const HMAC_SHA256: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02, 0x09];
    pub const HMAC_SHA384: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02, 0x0A];
    pub const HMAC_SHA512: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02, 0x0B];
    const PKCS5: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x05];
    const PKCS12: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x0C];

    /// What to call an encryption scheme nothing here undoes: the family
    /// for the password schemes older than PBES2, the dotted OID otherwise.
    pub fn encryption_name(oid: &[u8]) -> String {
        if oid.starts_with(PKCS12) {
            "PKCS #12 PBE".to_owned()
        } else if oid.starts_with(PKCS5) {
            "PBES1".to_owned()
        } else {
            dotted(oid)
        }
    }

    /// `1.2.840.10045.3.1.7`, for naming one nothing here knows.
    pub fn dotted(oid: &[u8]) -> String {
        let Some((&first, rest)) = oid.split_first() else {
            return "?".to_owned();
        };
        let mut parts = vec![u64::from(first) / 40, u64::from(first) % 40];
        let mut value = 0u64;
        for &byte in rest {
            value = value << 7 | u64::from(byte & 0x7f);
            if byte & 0x80 == 0 {
                parts.push(value);
                value = 0;
            }
        }
        parts
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(".")
    }
}
