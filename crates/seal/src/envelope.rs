//! The push envelope, format v1 — sealed on the dev host by
//! `moshpit-push.sh` (openssl), by `push-relay/sealbox` (Go) in tests, and by
//! `PushSealedBox` (Swift); opened on the phone.
//!
//! ```text
//! KeHex = hex(HMAC-SHA256(key: ascii(secretHex), msg: "moshpit-push-enc-v1"))
//! KmHex = hex(HMAC-SHA256(key: ascii(secretHex), msg: "moshpit-push-mac-v1"))
//! ct    = base64(AES-256-CBC(key: KeHex, iv: ivHex, pkcs7(plaintext)))
//! mac   = base64(HMAC-SHA256(key: ascii(KmHex), msg: "v1|" + ivHex + "|" + ct))
//! wire  = {"v":1,"iv":ivHex,"ct":ct,"mac":mac}
//! ```
//!
//! CBC and hex-string keys are what a POSIX `sh` with only openssl can send:
//! `openssl enc` refuses AEAD ciphers, and LibreSSL's `dgst` has `-hmac
//! <string>` but no raw-key form. Encrypt-then-MAC with the MAC checked
//! before any padding is looked at is what makes that safe.

use aes::Aes256;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::foundation::{base64, hex, json};
use crate::status::Status;

type HmacSha256 = Hmac<Sha256>;

/// The only envelope version there is. It is covered by the MAC (the `v1|`
/// prefix), so nothing can strip it to force another reading.
pub const VERSION: i64 = 1;

const ENC_INFO: &str = "moshpit-push-enc-v1";
const MAC_INFO: &str = "moshpit-push-mac-v1";
const MAC_TAG: &str = "v1";

/// A pairing secret's length: 32 bytes, as 64 hex characters.
pub const SECRET_HEX_LEN: usize = 64;

/// One sealed status, as it rides in a push's `mp` object. Field names are
/// short because the whole APNs payload is capped at 4 KB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: i64,
    pub iv: String,
    pub ct: String,
    pub mac: String,
}

/// Why an envelope did not open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// The pairing secret is not 64 hex characters.
    BadSecret,
    UnsupportedVersion(i64),
    /// A field is not what the format says, or the plaintext is no status.
    Malformed,
    /// The MAC did not verify: the wrong pairing secret (a host paired to
    /// another install) or a tampered payload. Never tell the two apart to
    /// the user — both mean "do not show this".
    AuthenticationFailed,
    /// The ciphertext authenticated but is not PKCS#7-padded.
    ///
    /// Fix (padding-lenient): CommonCrypto on macOS 27 never refuses here —
    /// it strips as many bytes as the last one says without checking the
    /// others, and returns the whole block for a last byte of 0 or above 16 —
    /// so `PushSealedBox.openRaw` (`PushSealedBox.swift:121`, `:190-211`)
    /// hands such plaintext on. Go's `sealbox.Open` refuses it, as this does.
    /// Only a holder of the key can produce one; no sender does.
    DecryptFailed,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadSecret => f.write_str("the pairing secret must be 64 hex characters"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported envelope version {v}"),
            Self::Malformed => f.write_str("malformed envelope"),
            Self::AuthenticationFailed => f.write_str("the envelope did not authenticate"),
            Self::DecryptFailed => f.write_str("the envelope did not decrypt"),
        }
    }
}

impl std::error::Error for Failure {}

/// The two subkeys of a pairing secret, in the hex-string form openssl and
/// every implementation consume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keys {
    /// Passed to `openssl enc -K`; its 32 decoded bytes key AES-256.
    pub enc_hex: String,
    /// An ASCII HMAC key: its bytes are the 64 hex characters.
    pub mac_hex: String,
}

fn hmac_hex(key: &[u8], message: &[u8]) -> String {
    hex::encode(&hmac(key, message))
}

fn hmac(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(message);
    mac.finalize().into_bytes().into()
}

/// Split a pairing secret into its two subkeys.
///
/// The HMAC key is the ASCII of the hex STRING, lower-cased first as the
/// shell's `tr` does — otherwise a secret typed in upper case derives other
/// keys on the two ends and every message fails its MAC.
pub fn derive_keys(secret_hex: &str) -> Result<Keys, Failure> {
    let normalised = secret_hex.to_ascii_lowercase();
    if normalised.len() != SECRET_HEX_LEN || hex::decode(&normalised).is_none() {
        return Err(Failure::BadSecret);
    }
    Ok(Keys {
        enc_hex: hmac_hex(normalised.as_bytes(), ENC_INFO.as_bytes()),
        mac_hex: hmac_hex(normalised.as_bytes(), MAC_INFO.as_bytes()),
    })
}

fn tagged_message(iv_hex: &str, ct_b64: &str) -> String {
    format!("{MAC_TAG}|{iv_hex}|{ct_b64}")
}

/// Authenticate and decrypt an envelope, then read the status inside.
pub fn open(envelope: &Envelope, secret_hex: &str) -> Result<Status, Failure> {
    let plaintext = open_raw(envelope, secret_hex)?;
    Status::from_json(&plaintext).ok_or(Failure::Malformed)
}

/// Authenticate and decrypt an envelope.
///
/// The checks run in `PushSealedBox.openRaw`'s order — version, secret, the
/// shape of every field, the MAC, the padding — so each envelope fails with
/// the same [`Failure`] it does on iOS. The MAC is verified before anything
/// is decrypted, in constant time: reaching the unpadding step with
/// unauthenticated ciphertext is what a CBC padding oracle needs.
pub fn open_raw(envelope: &Envelope, secret_hex: &str) -> Result<Vec<u8>, Failure> {
    if envelope.v != VERSION {
        return Err(Failure::UnsupportedVersion(envelope.v));
    }
    let keys = derive_keys(secret_hex)?;
    let iv = hex::decode(&envelope.iv)
        .filter(|iv| iv.len() == 16)
        .ok_or(Failure::Malformed)?;
    let ct = base64::decode(&envelope.ct)
        .filter(|ct| !ct.is_empty() && ct.len().is_multiple_of(16))
        .ok_or(Failure::Malformed)?;
    let tag = base64::decode(&envelope.mac).ok_or(Failure::Malformed)?;

    let mut mac =
        HmacSha256::new_from_slice(keys.mac_hex.as_bytes()).expect("HMAC takes any key length");
    mac.update(tagged_message(&envelope.iv.to_ascii_lowercase(), &envelope.ct).as_bytes());
    mac.verify_slice(&tag)
        .map_err(|_| Failure::AuthenticationFailed)?;

    let key = hex::decode(&keys.enc_hex).expect("a derived key is hex");
    let decryptor =
        cbc::Decryptor::<Aes256>::new_from_slices(&key, &iv).expect("32-byte key, 16-byte IV");
    decryptor
        .decrypt_padded_vec::<Pkcs7>(&ct)
        .map_err(|_| Failure::DecryptFailed)
}

/// Seal `plaintext`. Production sealing happens on the dev host, in shell;
/// this exists so the format can be tested from every end, and for the
/// pairing self-check. `iv_hex` pins the IV (a test vector); `None` draws one
/// from the CSPRNG.
pub fn seal(plaintext: &[u8], secret_hex: &str, iv_hex: Option<&str>) -> Result<Envelope, Failure> {
    let keys = derive_keys(secret_hex)?;
    let iv = match iv_hex {
        Some(iv_hex) => hex::decode(iv_hex)
            .filter(|iv| iv.len() == 16)
            .ok_or(Failure::Malformed)?,
        None => {
            let mut iv = vec![0u8; 16];
            crate::random::fill(&mut iv);
            iv
        }
    };
    let key = hex::decode(&keys.enc_hex).expect("a derived key is hex");
    let encryptor =
        cbc::Encryptor::<Aes256>::new_from_slices(&key, &iv).expect("32-byte key, 16-byte IV");
    let ct = base64::encode(&encryptor.encrypt_padded_vec::<Pkcs7>(plaintext));
    let iv_hex = hex::encode(&iv);
    let mac = base64::encode(&hmac(
        keys.mac_hex.as_bytes(),
        tagged_message(&iv_hex, &ct).as_bytes(),
    ));
    Ok(Envelope {
        v: VERSION,
        iv: iv_hex,
        ct,
        mac,
    })
}

impl Envelope {
    /// The envelope out of a push payload (`PushRemoteNotification.
    /// envelope(in:)`): `mp` must be an object holding an integral `v` and
    /// string `iv`, `ct` and `mac`. `None` for anything else — every
    /// notification that is not one of ours, a local one included.
    pub fn from_payload(payload: &json::Value) -> Option<Self> {
        let raw = payload
            .as_object()?
            .get(crate::notification::ENVELOPE_KEY)?
            .as_object()?;
        Some(Self {
            v: raw.get("v")?.as_swift_int()?,
            iv: raw.get("iv")?.as_str()?.to_owned(),
            ct: raw.get("ct")?.as_str()?.to_owned(),
            mac: raw.get("mac")?.as_str()?.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The secret the Go tests and `push-vector.sh` use (and the Swift
    /// tests did).
    pub(crate) const SECRET: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// The frozen cross-language vector (`push-relay/sealbox/sealbox_test.go`
    /// `TestOpenSSLVector`; the Swift `PushSealedBoxTests.Vector` until
    /// E7b), produced by the openssl pipeline `moshpit-push.sh` runs with the
    /// IV pinned.
    pub(crate) mod vector {
        pub const IV: &str = "000102030405060708090a0b0c0d0e0f";
        pub const CT: &str = "21HMutLOqDHXUpjykOXFikZCpepvsm5jV5f/9b8UHg7qOFw7ihvJXuhQMQ9iG/gbmStNwvoMQDCC6nukKAF2bmyrRIX4+lriKtR+xgIN1WWSeBXfoh2nk0KhJvMsvXqd9xJIt+ShRcbZH+NePfCXwHiOcRQ/Q3ZO89pgX/d3TO8yI0UEsVNyclSjlgqri/d/9l5q9KB/4xFJ/+mxZFLcm56XO8DAXn6nQt2mQ9Z7Dt6bBqw68BRhNdH05E7O3OqR";
        pub const MAC: &str = "ZiSLy5cGr2XOMlJiX3ep8bVSAcmERPDSMpHnysktnBA=";
        pub const ENC_HEX: &str =
            "4ff81bf9801df5440677a30d2622a56d9ecfc556ea57c383cb057bde865e058d";
        pub const MAC_HEX: &str =
            "7fe30bcca8aa9e48de360f5d4cd48f1f83dfc68345ae6861f5cd449f5247ca31";
        pub const PLAINTEXT: &str = r#"{"conn":"3F2504E0-4F89-11D3-9A0C-0305E82C3301","host":"m1-pro","sess":"work","pane":"%3","agent":"claude","state":"attention","title":"Bash: rm -rf \"build\" 构建","ts":1755900000}"#;

        pub fn envelope() -> super::Envelope {
            super::Envelope {
                v: 1,
                iv: IV.into(),
                ct: CT.into(),
                mac: MAC.into(),
            }
        }
    }

    // MARK: - PushSealedBoxTests

    #[test]
    fn key_derivation_matches_openssl_byte_for_byte() {
        let keys = derive_keys(SECRET).unwrap();
        assert_eq!(keys.enc_hex, vector::ENC_HEX);
        assert_eq!(keys.mac_hex, vector::MAC_HEX);
        assert_ne!(keys.enc_hex, keys.mac_hex);
    }

    #[test]
    fn opens_an_envelope_sealed_by_the_host_shell_script() {
        let status = open(&vector::envelope(), SECRET).unwrap();
        assert_eq!(status.conn, "3F2504E0-4F89-11D3-9A0C-0305E82C3301");
        assert_eq!(status.host, "m1-pro");
        assert_eq!(status.sess.as_deref(), Some("work"));
        assert_eq!(status.pane, "%3");
        assert_eq!(status.agent.as_deref(), Some("claude"));
        assert_eq!(status.state, "attention");
        assert_eq!(
            status.title.as_deref(),
            Some(r#"Bash: rm -rf "build" 构建"#)
        );
        assert_eq!(status.ts, 1_755_900_000);
        assert_eq!(status.dur, None);
    }

    #[test]
    fn reproduces_the_openssl_bytes_for_the_same_input_and_iv() {
        let sealed = seal(vector::PLAINTEXT.as_bytes(), SECRET, Some(vector::IV)).unwrap();
        assert_eq!(sealed.ct, vector::CT);
        assert_eq!(sealed.mac, vector::MAC);
        assert_eq!(sealed, vector::envelope());
    }

    #[test]
    fn round_trips_payloads_of_every_padding_shape() {
        let long = "字".repeat(200);
        for plaintext in ["", "x", &"a".repeat(16), &long] {
            let sealed = seal(plaintext.as_bytes(), SECRET, None).unwrap();
            assert_eq!(open_raw(&sealed, SECRET).unwrap(), plaintext.as_bytes());
        }
    }

    #[test]
    fn a_fresh_iv_is_used_for_every_seal() {
        let a = seal(b"same", SECRET, None).unwrap();
        let b = seal(b"same", SECRET, None).unwrap();
        assert_ne!(a.iv, b.iv);
        assert_ne!(a.ct, b.ct);
    }

    #[test]
    fn the_secret_is_case_insensitive_as_the_shells_tr_makes_it() {
        assert_eq!(
            open(&vector::envelope(), &SECRET.to_uppercase())
                .unwrap()
                .host,
            "m1-pro"
        );
        assert_eq!(derive_keys(&SECRET.to_uppercase()), derive_keys(SECRET));
    }

    fn flip(b64: &str) -> String {
        let mut bytes = base64::decode(b64).unwrap();
        bytes[0] ^= 0x01;
        base64::encode(&bytes)
    }

    #[test]
    fn tampering_is_rejected() {
        let env = |v: i64, iv: &str, ct: &str, mac: &str| Envelope {
            v,
            iv: iv.into(),
            ct: ct.into(),
            mac: mac.into(),
        };
        let cases = [
            (
                "flipped ciphertext",
                env(1, vector::IV, &flip(vector::CT), vector::MAC),
            ),
            (
                "flipped MAC",
                env(1, vector::IV, vector::CT, &flip(vector::MAC)),
            ),
            (
                "swapped IV",
                env(1, &"00".repeat(16), vector::CT, vector::MAC),
            ),
            ("stripped MAC", env(1, vector::IV, vector::CT, "")),
            (
                "future version",
                env(2, vector::IV, vector::CT, vector::MAC),
            ),
            ("empty ciphertext", env(1, vector::IV, "", vector::MAC)),
            ("short IV", env(1, "00", vector::CT, vector::MAC)),
            (
                "non-block ciphertext",
                env(1, vector::IV, &base64::encode(b"abc"), vector::MAC),
            ),
        ];
        for (name, envelope) in &cases {
            assert!(open(envelope, SECRET).is_err(), "{name} was accepted");
        }
        // Each fails where PushSealedBox fails it.
        assert_eq!(
            open(&cases[0].1, SECRET),
            Err(Failure::AuthenticationFailed)
        );
        assert_eq!(
            open(&cases[1].1, SECRET),
            Err(Failure::AuthenticationFailed)
        );
        assert_eq!(
            open(&cases[2].1, SECRET),
            Err(Failure::AuthenticationFailed)
        );
        assert_eq!(
            open(&cases[3].1, SECRET),
            Err(Failure::AuthenticationFailed)
        );
        assert_eq!(
            open(&cases[4].1, SECRET),
            Err(Failure::UnsupportedVersion(2))
        );
        assert_eq!(open(&cases[5].1, SECRET), Err(Failure::Malformed));
        assert_eq!(open(&cases[6].1, SECRET), Err(Failure::Malformed));
        assert_eq!(open(&cases[7].1, SECRET), Err(Failure::Malformed));
    }

    /// A padding error here would mean the ciphertext was decrypted before it
    /// was authenticated — the shape a CBC padding oracle needs.
    #[test]
    fn the_wrong_secret_fails_authentication_never_decryption() {
        assert_eq!(
            open(&vector::envelope(), &"ff".repeat(32)),
            Err(Failure::AuthenticationFailed)
        );
    }

    #[test]
    fn malformed_secrets_are_refused_outright() {
        for secret in [
            "",
            "abc",
            &"z".repeat(64),
            &"a".repeat(63),
            &"+f".repeat(32),
        ] {
            assert_eq!(derive_keys(secret), Err(Failure::BadSecret), "{secret}");
        }
    }

    #[test]
    fn plaintext_that_is_not_a_status_is_an_error_not_an_empty_status() {
        let sealed = seal(b"not json", SECRET, None).unwrap();
        assert_eq!(open(&sealed, SECRET), Err(Failure::Malformed));
    }

    /// Go's `TestSealWithIVIsDeterministic`.
    #[test]
    fn pinned_iv_sealing_is_deterministic() {
        let iv = Some("000102030405060708090a0b0c0d0e0f");
        assert_eq!(
            seal(b"same", SECRET, iv).unwrap(),
            seal(b"same", SECRET, iv).unwrap()
        );
        // An upper-case IV is written back lower-case, as Go does.
        assert_eq!(
            seal(b"same", SECRET, Some("000102030405060708090A0B0C0D0E0F")).unwrap(),
            seal(b"same", SECRET, iv).unwrap()
        );
    }

    /// Fix (padding-lenient): an authenticated ciphertext with a bad pad.
    #[test]
    fn an_authenticated_ciphertext_must_still_be_padded() {
        use cbc::cipher::block_padding::NoPadding;
        let keys = derive_keys(SECRET).unwrap();
        let key = hex::decode(&keys.enc_hex).unwrap();
        let iv = [1u8; 16];
        for block in [[0x41u8; 16], {
            let mut b = [0x41u8; 16];
            b[14] = 9;
            b[15] = 2;
            b
        }] {
            let encryptor = cbc::Encryptor::<Aes256>::new_from_slices(&key, &iv).unwrap();
            let ct = base64::encode(&encryptor.encrypt_padded_vec::<NoPadding>(&block));
            let iv_hex = hex::encode(&iv);
            let mac = base64::encode(&hmac(
                keys.mac_hex.as_bytes(),
                tagged_message(&iv_hex, &ct).as_bytes(),
            ));
            let envelope = Envelope {
                v: 1,
                iv: iv_hex,
                ct,
                mac,
            };
            assert_eq!(open_raw(&envelope, SECRET), Err(Failure::DecryptFailed));
        }
    }

    #[test]
    fn the_envelope_is_read_out_of_a_payload() {
        let payload = |text: &str| {
            json::parse(
                text.as_bytes(),
                json::Dialect::Serialization {
                    fragments_allowed: true,
                },
            )
            .unwrap()
        };
        let full =
            payload(r#"{"aps":{"mutable-content":1},"mp":{"v":1,"iv":"00","ct":"c","mac":"m"}}"#);
        assert_eq!(
            Envelope::from_payload(&full),
            Some(Envelope {
                v: 1,
                iv: "00".into(),
                ct: "c".into(),
                mac: "m".into()
            })
        );
        assert_eq!(Envelope::from_payload(&payload("{}")), None);
        assert_eq!(
            Envelope::from_payload(&payload(r#"{"connectionId":"x","paneId":"%3"}"#)),
            None
        );
        // A partial envelope must not decode into something half-valid.
        assert_eq!(
            Envelope::from_payload(&payload(r#"{"mp":{"v":1,"iv":"00"}}"#)),
            None
        );
        assert_eq!(
            Envelope::from_payload(&payload(r#"{"mp":{"v":"1","iv":"","ct":"","mac":""}}"#)),
            None
        );
        assert_eq!(
            Envelope::from_payload(&payload(r#"{"mp":{"v":1.5,"iv":"","ct":"","mac":""}}"#)),
            None
        );
        assert_eq!(
            Envelope::from_payload(&payload(r#"{"mp":{"v":1.0,"iv":"","ct":"","mac":""}}"#))
                .map(|e| e.v),
            Some(1)
        );
    }
}
