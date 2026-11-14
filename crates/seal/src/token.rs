//! The send token: the bearer credential a host presents to the relay.
//!
//! Minted by the RELAY (`push-relay/token.go`), never by the phone: an HMAC,
//! under a key only the relay holds, over the routing facts every push
//! carries — so a stateless relay can verify a push with nothing to look up.
//!
//! ```text
//! sendToken = hex(HMAC-SHA256(master, "moshpit-send-v2|" + lower(apnsToken) + "|" + conn + "|" + iat))
//! ```
//!
//! The phone only stores it ([`crate::pairing::PushPairing::send_token`]).
//! The formula is here so tests (and a relay written against this crate) can
//! mint and check tokens exactly as the Go relay does; `harness/interop.py`
//! holds the two to the same bytes.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::foundation::hex;

type HmacSha256 = Hmac<Sha256>;

/// How long the relay honours a token by default (45 days).
pub const DEFAULT_TTL_SECONDS: i64 = 45 * 24 * 3600;

/// How far in the future a mint time may be before it is refused.
const FUTURE_SKEW_SECONDS: i64 = 5 * 60;

/// Lower-case the A-F of a hex token and nothing else (`lowerHex`).
fn lower_hex(text: &str) -> String {
    text.chars()
        .map(|c| {
            if ('A'..='F').contains(&c) {
                c.to_ascii_lowercase()
            } else {
                c
            }
        })
        .collect()
}

/// `mintSendToken`.
pub fn mint(master: &[u8], apns_token: &str, conn: &str, iat: i64) -> String {
    let mut mac = HmacSha256::new_from_slice(master).expect("HMAC takes any key length");
    mac.update(format!("moshpit-send-v2|{}|{conn}|{iat}", lower_hex(apns_token)).as_bytes());
    hex::encode(&mac.finalize().into_bytes())
}

/// `verifySendToken`: the token was minted over these facts, `iat` is
/// positive, not more than five minutes ahead of `now` and not older than
/// `ttl_seconds`; compared in constant time.
pub fn verify(
    master: &[u8],
    presented: &str,
    apns_token: &str,
    conn: &str,
    iat: i64,
    now: i64,
    ttl_seconds: i64,
) -> bool {
    if iat <= 0
        || iat > now.saturating_add(FUTURE_SKEW_SECONDS)
        || now.saturating_sub(iat) > ttl_seconds
    {
        return false;
    }
    let Some(presented) = hex::decode(presented) else {
        return false;
    };
    let mut mac = HmacSha256::new_from_slice(master).expect("HMAC takes any key length");
    mac.update(format!("moshpit-send-v2|{}|{conn}|{iat}", lower_hex(apns_token)).as_bytes());
    mac.verify_slice(&presented).is_ok()
}

/// `SendTokenHash`: the relay's throttle key and log handle for a token.
pub fn hash(send_token: &str) -> String {
    hex::encode(&Sha256::digest(send_token.as_bytes()))
}

/// What the phone posts to `/v1/mint` (`PushRelayClient.mint`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MintRequest {
    #[serde(rename = "apnsToken")]
    pub apns_token: String,
    /// The connection's UUID string.
    pub conn: String,
}

/// The relay's answer to a mint: `{"ok":true,"sendToken":…,"iat":…}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MintResponse {
    #[serde(rename = "sendToken")]
    pub send_token: String,
    /// Mint time, Unix seconds.
    pub iat: i64,
}

impl MintResponse {
    /// The body of a 200 answer, read as `PushRelayClient.mint` reads it —
    /// and only with a send token shaped as the relay mints one.
    ///
    /// Fix (send-token-shape, B357): the token is written into the host's push conf and from there into
    /// curl's config on every push. Taken as any string, a relay could put
    /// shell or a second line into it, which the sender (sourcing its confs
    /// before B357) ran on every host paired through it. A relay is trusted
    /// with routing, never with the hosts: what isn't 64 hex digits is a
    /// malformed answer.
    pub fn from_json(body: &[u8]) -> Option<Self> {
        crate::foundation::json::decode::<Self>(body)
            .ok()
            .filter(|minted| is_send_token(&minted.send_token))
    }
}

/// A send token as [`mint`] makes one: 64 hex digits (HMAC-SHA256).
pub fn is_send_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &[u8] = b"moshpit-dry-run-master-not-for-production";

    #[test]
    fn verifies_what_it_mints_and_nothing_else() {
        let token = mint(MASTER, &"AB".repeat(32), "C1", 1_756_000_000);
        assert_eq!(token.len(), 64);
        let now = 1_756_000_100;
        assert!(verify(
            MASTER,
            &token,
            &"ab".repeat(32),
            "C1",
            1_756_000_000,
            now,
            DEFAULT_TTL_SECONDS
        ));
        // Retargeted at another phone, another host, another mint time.
        assert!(!verify(
            MASTER,
            &token,
            &"cd".repeat(32),
            "C1",
            1_756_000_000,
            now,
            DEFAULT_TTL_SECONDS
        ));
        assert!(!verify(
            MASTER,
            &token,
            &"ab".repeat(32),
            "C2",
            1_756_000_000,
            now,
            DEFAULT_TTL_SECONDS
        ));
        assert!(!verify(
            MASTER,
            &token,
            &"ab".repeat(32),
            "C1",
            1_756_000_001,
            now,
            DEFAULT_TTL_SECONDS
        ));
        // Expired, from the future, unminted.
        assert!(!verify(
            MASTER,
            &token,
            &"ab".repeat(32),
            "C1",
            1_756_000_000,
            1_756_000_000 + DEFAULT_TTL_SECONDS + 1,
            DEFAULT_TTL_SECONDS
        ));
        assert!(!verify(
            MASTER,
            &token,
            &"ab".repeat(32),
            "C1",
            1_756_000_000,
            1_756_000_000 - 301,
            DEFAULT_TTL_SECONDS
        ));
        assert!(!verify(
            MASTER,
            &token,
            &"ab".repeat(32),
            "C1",
            0,
            now,
            DEFAULT_TTL_SECONDS
        ));
        assert!(!verify(
            MASTER,
            "not hex",
            &"ab".repeat(32),
            "C1",
            1_756_000_000,
            now,
            DEFAULT_TTL_SECONDS
        ));
    }

    #[test]
    fn the_mint_wire_format() {
        let request = MintRequest {
            apns_token: "ab".into(),
            conn: "C".into(),
        };
        assert_eq!(
            crate::foundation::json::encode(&request),
            r#"{"apnsToken":"ab","conn":"C"}"#
        );
        let token = mint(MASTER, &"ab".repeat(32), "C", 1_756_000_000);
        let answer = MintResponse::from_json(
            format!(r#"{{"ok":true,"sendToken":"{token}","iat":1756000000}}"#).as_bytes(),
        )
        .unwrap();
        assert_eq!(
            answer,
            MintResponse {
                send_token: token,
                iat: 1_756_000_000
            }
        );
        assert!(MintResponse::from_json(br#"{"ok":false,"error":"x"}"#).is_none());
    }

    /// B357: a send token that isn't one the relay could have minted is a
    /// malformed answer — it would be written into every paired host's conf.
    #[test]
    fn only_a_token_shaped_as_the_relay_mints_one_is_taken() {
        let hex = "ab".repeat(32);
        for bad in [
            "tok".to_owned(),
            String::new(),
            format!("{}x", &hex[..63]),
            format!("{hex}0"),
            format!("{};touch /tmp/p", &hex[..48]),
            format!("{}\nRELAY_URL=https://evil.example", &hex[..32]),
            format!("{}$(id)", &hex[..59]),
        ] {
            let body = format!(r#"{{"ok":true,"sendToken":"{bad}","iat":1756000000}}"#);
            assert!(
                MintResponse::from_json(body.as_bytes()).is_none(),
                "{bad:?}"
            );
        }
        assert!(is_send_token(&hex.to_uppercase()));
    }
}
