//! The keys a key file can hold, built from their numbers and checked: an
//! Ed25519 seed, an ECDSA scalar on a NIST curve, RSA's numbers.

use russh::keys::ssh_encoding::{Encode, Mpint};
use russh::keys::ssh_key::private::{
    EcdsaKeypair, Ed25519Keypair, KeypairData, RsaKeypair, RsaPrivateKey,
};
use russh::keys::ssh_key::public::{KeyData, RsaPublicKey};
use zeroize::Zeroizing;

use super::ImportError;
use super::der::{Der, OID_TAG, magnitude, oid};

/// The NIST curves an EC key file can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    P256,
    P384,
    P521,
}

impl Curve {
    const ALL: [Curve; 3] = [Curve::P256, Curve::P384, Curve::P521];

    /// What a person calls it ("ECDSA P-384").
    pub fn name(self) -> &'static str {
        match self {
            Curve::P256 => "ECDSA P-256",
            Curve::P384 => "ECDSA P-384",
            Curve::P521 => "ECDSA P-521",
        }
    }

    /// Its name inside an SSH key (`nistp384`).
    pub fn ssh_id(self) -> &'static str {
        match self {
            Curve::P256 => "nistp256",
            Curve::P384 => "nistp384",
            Curve::P521 => "nistp521",
        }
    }

    /// The SSH key type (`ecdsa-sha2-nistp384`).
    pub fn key_type(self) -> &'static str {
        match self {
            Curve::P256 => "ecdsa-sha2-nistp256",
            Curve::P384 => "ecdsa-sha2-nistp384",
            Curve::P521 => "ecdsa-sha2-nistp521",
        }
    }

    pub fn from_key_type(key_type: &str) -> Option<Curve> {
        Curve::ALL.into_iter().find(|c| c.key_type() == key_type)
    }

    pub fn scalar_size(self) -> usize {
        match self {
            Curve::P256 => 32,
            Curve::P384 => 48,
            Curve::P521 => 66,
        }
    }

    /// The field prime: what tells the NIST curves from each other and from
    /// every other curve in use (secp256k1, Brainpool, SM2).
    fn prime(self) -> Vec<u8> {
        match self {
            Curve::P256 => [
                &[0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x01][..],
                &[0; 12],
                &[0xFF; 12],
            ]
            .concat(),
            Curve::P384 => [&[0xFF; 31][..], &[0xFE], &[0xFF; 4], &[0; 8], &[0xFF; 4]].concat(),
            Curve::P521 => [&[0x01][..], &[0xFF; 65]].concat(),
        }
    }

    /// EC domain parameters as a key file gives them: a named curve's OID,
    /// or the curve spelled out (`ECParameters`, what LibreSSL's
    /// `ssh-keygen -m PEM` writes).
    pub(super) fn from_parameters(parameters: &mut Der<'_>) -> Result<Curve, ImportError> {
        match parameters.next_tag() {
            Some(OID_TAG) => match parameters.oid()? {
                oid::PRIME256V1 => Ok(Curve::P256),
                oid::SECP384R1 => Ok(Curve::P384),
                oid::SECP521R1 => Ok(Curve::P521),
                oid::SECP256K1 => Err(ImportError::Unsupported("ECDSA secp256k1".to_owned())),
                other => Err(ImportError::Unsupported(format!(
                    "ECDSA {}",
                    oid::dotted(other)
                ))),
            },
            Some(0x30) => {
                // ECParameters: SEQUENCE { version, fieldID SEQUENCE {
                // fieldType, prime }, curve, base, order, cofactor }.
                let mut explicit = parameters.sequence()?;
                explicit.integer()?;
                let mut field = explicit.sequence()?;
                if field.oid()? != oid::PRIME_FIELD {
                    return Err(ImportError::Unsupported("ECDSA (binary field)".to_owned()));
                }
                let prime = magnitude(field.integer()?);
                Curve::ALL
                    .into_iter()
                    .find(|c| c.prime() == prime)
                    .ok_or_else(|| ImportError::Unsupported("ECDSA (custom curve)".to_owned()))
            }
            _ => Err(ImportError::Damaged),
        }
    }

    /// The curve whose uncompressed points (`04 ‖ X ‖ Y`) are `size` bytes.
    pub(super) fn from_point_size(size: usize) -> Option<Curve> {
        Curve::ALL
            .into_iter()
            .find(|c| 1 + 2 * c.scalar_size() == size)
    }

    /// The smallest curve a scalar of `size` bytes fits: leading zero bytes
    /// may be missing, never extra.
    pub(super) fn from_scalar_size(size: usize) -> Option<Curve> {
        Curve::ALL.into_iter().find(|c| size <= c.scalar_size())
    }
}

/// An Ed25519 key from its 32-byte seed.
pub(super) fn ed25519(seed: &[u8]) -> Result<KeypairData, ImportError> {
    let seed: &[u8; 32] = seed.try_into().map_err(|_| ImportError::Damaged)?;
    Ok(KeypairData::Ed25519(Ed25519Keypair::from_seed(seed)))
}

/// An ECDSA key on `curve` from its scalar, whose leading zero bytes may
/// have been dropped on the way (OpenSSH writes it as a minimal `mpint`).
pub(super) fn ecdsa(curve: Curve, scalar: &[u8]) -> Result<KeypairData, ImportError> {
    let scalar = magnitude(scalar);
    let size = curve.scalar_size();
    if scalar.is_empty() || scalar.len() > size {
        return Err(ImportError::Damaged);
    }
    let mut fixed = Zeroizing::new(vec![0u8; size]);
    fixed[size - scalar.len()..].copy_from_slice(scalar);
    let damaged = |_| ImportError::Damaged;
    Ok(KeypairData::Ecdsa(match curve {
        Curve::P256 => {
            let private = p256::SecretKey::from_slice(&fixed).map_err(damaged)?;
            let public = private.public_key();
            EcdsaKeypair::NistP256 {
                public: public.into(),
                private: private.into(),
            }
        }
        Curve::P384 => {
            let private = p384::SecretKey::from_slice(&fixed).map_err(damaged)?;
            let public = private.public_key();
            EcdsaKeypair::NistP384 {
                public: public.into(),
                private: private.into(),
            }
        }
        Curve::P521 => {
            let private = p521::SecretKey::from_slice(&fixed).map_err(damaged)?;
            let public = private.public_key();
            EcdsaKeypair::NistP521 {
                public: public.into(),
                private: private.into(),
            }
        }
    }))
}

/// An RSA key from its numbers (big-endian, leading zeros or not), checked
/// the way signing will check it: `n = p·q`, `e·d ≡ 1`. Its CRT coefficient
/// is computed rather than taken.
pub(super) fn rsa(
    n: &[u8],
    e: &[u8],
    d: &[u8],
    p: &[u8],
    q: &[u8],
) -> Result<KeypairData, ImportError> {
    let int = |raw: &[u8]| -> Result<Mpint, ImportError> {
        let raw = magnitude(raw);
        if raw.is_empty() {
            return Err(ImportError::Damaged);
        }
        Ok(Mpint::from_positive_bytes(raw))
    };
    let public = RsaPublicKey::new(int(e)?, int(n)?).map_err(|_| ImportError::Damaged)?;
    // A stand-in coefficient: replaced by the one the check computes.
    let private = RsaPrivateKey::new(int(d)?, int(&[1])?, int(p)?, int(q)?)
        .map_err(|_| ImportError::Damaged)?;
    let keypair = RsaKeypair::new(public, private).map_err(|_| ImportError::Damaged)?;
    let checked = rsa::RsaPrivateKey::try_from(&keypair).map_err(|_| ImportError::Damaged)?;
    let keypair = RsaKeypair::try_from(&checked).map_err(|_| ImportError::Damaged)?;
    Ok(KeypairData::Rsa(keypair))
}

/// The public key in SSH wire form: what `authorized_keys` carries in
/// base64 and the fingerprint is taken over.
pub(super) fn public_blob(key: &KeypairData) -> Result<Vec<u8>, ImportError> {
    let public = KeyData::try_from(key).map_err(|e| ImportError::Failed(e.to_string()))?;
    public
        .encode_vec()
        .map_err(|e| ImportError::Failed(e.to_string()))
}
