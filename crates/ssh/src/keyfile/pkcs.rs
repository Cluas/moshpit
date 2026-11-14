//! PKCS #1 RSA, SEC1 EC and PKCS #8 private keys — what `ssh-keygen -m PEM`
//! and `-m PKCS8` write, and OpenSSL — and PBES2, which encrypts PKCS #8.

use russh::keys::ssh_key::private::KeypairData;
use zeroize::Zeroizing;

use super::ImportError;
use super::cbc::{BlockCipher, decrypt_cbc_padded};
use super::der::{Der, magnitude, oid, small_integer};
use super::material::{self, Curve};

/// PKCS #1 RSAPrivateKey: SEQUENCE { version, n, e, d, p, q, d mod (p-1),
/// d mod (q-1), q⁻¹ mod p }.
pub(super) fn pkcs1(der: &[u8]) -> Result<KeypairData, ImportError> {
    let mut key = Der(der).sequence()?;
    // Version 1 is multi-prime RSA, which SSH has no way to carry.
    if !magnitude(key.integer()?).is_empty() {
        return Err(ImportError::Unsupported("multi-prime RSA".to_owned()));
    }
    let (n, e, d) = (key.integer()?, key.integer()?, key.integer()?);
    let (p, q) = (key.integer()?, key.integer()?);
    key.integer()?;
    key.integer()?;
    key.integer()?;
    material::rsa(n, e, d, p, q)
}

/// SEC1 ECPrivateKey: SEQUENCE { version 1, privateKey OCTET STRING,
/// [0] parameters OPTIONAL, [1] publicKey BIT STRING OPTIONAL }. The curve
/// comes from the PKCS #8 wrapper (`given`) or from [0]: named by OID, or
/// spelled out in full.
pub(super) fn sec1(der: &[u8], given: Option<Curve>) -> Result<KeypairData, ImportError> {
    let mut key = Der(der).sequence()?;
    if magnitude(key.integer()?) != [1] {
        return Err(ImportError::Damaged);
    }
    let scalar = key.octet_string()?;
    let mut curve = given;
    let mut embedded = None;
    while !key.is_at_end() {
        match key.element()? {
            (0xA0, contents) if curve.is_none() => {
                curve = Some(Curve::from_parameters(&mut Der(contents))?);
            }
            (0xA1, contents) => {
                let bits = Der(contents).expect(0x03)?;
                let Some((0, point)) = bits.split_first() else {
                    return Err(ImportError::Damaged);
                };
                embedded = Some(point);
            }
            _ => {}
        }
    }
    // Neither said which curve: the public point's size does, else the
    // scalar's.
    let curve = curve
        .or_else(|| embedded.and_then(|p| Curve::from_point_size(p.len())))
        .or_else(|| Curve::from_scalar_size(scalar.len()))
        .ok_or(ImportError::Damaged)?;
    let key = material::ecdsa(curve, scalar)?;
    // A public point given alongside must be this scalar's.
    if let (Some(embedded), KeypairData::Ecdsa(pair)) = (embedded, &key)
        && embedded != pair.public_key_bytes()
    {
        return Err(ImportError::Damaged);
    }
    Ok(key)
}

/// PKCS #8 PrivateKeyInfo: SEQUENCE { version, AlgorithmIdentifier,
/// privateKey OCTET STRING, … } around one of the above, or an Ed25519 seed.
pub(super) fn pkcs8(der: &[u8]) -> Result<KeypairData, ImportError> {
    let mut info = Der(der).sequence()?;
    info.integer()?;
    let mut algorithm = info.sequence()?;
    let id = algorithm.oid()?;
    let private_key = info.octet_string()?;
    match id {
        oid::RSA_ENCRYPTION => pkcs1(private_key),
        oid::EC_PUBLIC_KEY => {
            let curve = if algorithm.is_at_end() {
                None
            } else {
                Some(Curve::from_parameters(&mut algorithm)?)
            };
            sec1(private_key, curve)
        }
        oid::ED25519 => {
            let seed = Der(private_key).octet_string()?;
            if seed.len() != 32 {
                return Err(ImportError::Damaged);
            }
            material::ed25519(seed)
        }
        oid::DSA => Err(ImportError::Unsupported("DSA".to_owned())),
        oid::ED448 => Err(ImportError::Unsupported("Ed448".to_owned())),
        other => Err(ImportError::Unsupported(oid::dotted(other))),
    }
}

/// The PBKDF2 pseudo-random functions PBES2 names.
#[derive(Clone, Copy)]
enum Prf {
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl Prf {
    fn from_oid(id: &[u8]) -> Option<Prf> {
        Some(match id {
            oid::HMAC_SHA1 => Prf::Sha1,
            oid::HMAC_SHA224 => Prf::Sha224,
            oid::HMAC_SHA256 => Prf::Sha256,
            oid::HMAC_SHA384 => Prf::Sha384,
            oid::HMAC_SHA512 => Prf::Sha512,
            _ => return None,
        })
    }

    fn derive(self, password: &[u8], salt: &[u8], rounds: u32, out: &mut [u8]) {
        use pbkdf2::pbkdf2_hmac;
        match self {
            Prf::Sha1 => pbkdf2_hmac::<sha1::Sha1>(password, salt, rounds, out),
            Prf::Sha224 => pbkdf2_hmac::<sha2::Sha224>(password, salt, rounds, out),
            Prf::Sha256 => pbkdf2_hmac::<sha2::Sha256>(password, salt, rounds, out),
            Prf::Sha384 => pbkdf2_hmac::<sha2::Sha384>(password, salt, rounds, out),
            Prf::Sha512 => pbkdf2_hmac::<sha2::Sha512>(password, salt, rounds, out),
        }
    }
}

/// The most PBKDF2 iterations a key is opened with: OpenSSL writes 2048,
/// and a garbled count of billions would hold the import for hours (it did
/// in the Swift reader).
pub(super) const MAX_PBKDF2_ROUNDS: u32 = 1 << 20;

/// EncryptedPrivateKeyInfo under PBES2 (PKCS #5 v2: PBKDF2, then a CBC
/// cipher) — what OpenSSL and `ssh-keygen -m PKCS8` write for a key with a
/// passphrase — decrypted to the PKCS #8 inside. The older PBES1 and
/// PKCS #12 schemes are refused by name.
pub(super) fn pbes2_decrypt(
    der: &[u8],
    passphrase: Option<&[u8]>,
) -> Result<Zeroizing<Vec<u8>>, ImportError> {
    let mut info = Der(der).sequence()?;
    let mut scheme = info.sequence()?;
    let scheme_oid = scheme.oid()?;
    if scheme_oid != oid::PBES2 {
        return Err(ImportError::UnsupportedEncryption(oid::encryption_name(
            scheme_oid,
        )));
    }
    let mut parameters = scheme.sequence()?;
    let mut kdf = parameters.sequence()?;
    let kdf_oid = kdf.oid()?;
    if kdf_oid != oid::PBKDF2 {
        return Err(ImportError::UnsupportedEncryption(oid::encryption_name(
            kdf_oid,
        )));
    }
    let mut kdf_parameters = kdf.sequence()?;
    let salt = kdf_parameters.octet_string()?;
    let rounds = small_integer(kdf_parameters.integer()?)
        .filter(|&n| n >= 1)
        .ok_or(ImportError::Damaged)?;
    let mut prf = Prf::Sha1;
    while !kdf_parameters.is_at_end() {
        // keyLength (an INTEGER) is implied by the cipher.
        let (0x30, contents) = kdf_parameters.element()? else {
            continue;
        };
        let prf_oid = Der(contents).oid()?;
        prf = Prf::from_oid(prf_oid)
            .ok_or_else(|| ImportError::UnsupportedEncryption(oid::encryption_name(prf_oid)))?;
    }
    let mut encryption = parameters.sequence()?;
    let cipher_oid = encryption.oid()?;
    let cipher = BlockCipher::from_oid(cipher_oid)
        .ok_or_else(|| ImportError::UnsupportedEncryption(oid::encryption_name(cipher_oid)))?;
    let iv = encryption.octet_string()?;
    let encrypted = info.octet_string()?;
    if iv.len() != cipher.block_size() {
        return Err(ImportError::Damaged);
    }
    let passphrase = passphrase.ok_or(ImportError::NeedsPassphrase)?;
    if rounds > MAX_PBKDF2_ROUNDS {
        return Err(ImportError::UnsupportedEncryption(format!(
            "PBKDF2 with {rounds} iterations"
        )));
    }
    let mut key = Zeroizing::new(vec![0u8; cipher.key_len()]);
    prf.derive(passphrase, salt, rounds, &mut key);
    decrypt_cbc_padded(cipher, encrypted, &key, iv).ok_or(ImportError::WrongPassphrase)
}
