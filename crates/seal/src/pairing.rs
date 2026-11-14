//! A paired dev host: the two secrets that let it wake this phone, and the
//! connection they belong to (`PushPairing`).
//!
//! Pairing is per CONNECTION, not per device: unpairing one server cannot
//! break the others, a compromised host cannot forge notifications that
//! seem to come from another, and the relay's rate limit (keyed on the send
//! token) is per host.

use serde::{Deserialize, Serialize};

use crate::foundation::date::Date;
use crate::foundation::{text, uuid::Uuid};

/// One pairing, in the shape every build and the notification service read
/// (`push-pairings.json` holds an array of these): `JSONEncoder`'s default
/// `Codable` form, the connection id as an upper-case UUID string, both
/// dates as seconds since 2001, an absent optional left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushPairing {
    /// The app's own connection id. It travels to the host and comes back in
    /// every sealed envelope.
    #[serde(rename = "connectionId")]
    pub connection_id: Uuid,
    /// Display name of the host, for the settings row and the card. Never
    /// sent anywhere.
    #[serde(rename = "hostLabel")]
    pub host_label: String,
    /// 32 bytes, hex: the end-to-end key. Here and in the host's
    /// `~/.moshpit/push.d/<conn>.conf`, never on the relay.
    #[serde(rename = "secretHex")]
    pub secret_hex: String,
    /// The bearer credential the host presents to the relay — minted BY THE
    /// RELAY. Empty until there is a device token to mint against; a pairing
    /// with an empty token must never reach a host.
    #[serde(rename = "sendToken")]
    pub send_token: String,
    #[serde(rename = "relayURL")]
    pub relay_url: String,
    #[serde(rename = "createdAt")]
    pub created_at: Date,
    /// The device token `send_token` was minted over (on iOS an APNs token).
    #[serde(rename = "apnsToken", default, skip_serializing_if = "Option::is_none")]
    pub apns_token: Option<String>,
    /// `production` or `sandbox`: the build's own guess, a routing hint.
    #[serde(rename = "apnsEnv", default, skip_serializing_if = "Option::is_none")]
    pub apns_env: Option<String>,
    /// When the relay minted `send_token`.
    #[serde(
        rename = "sendTokenIssuedAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub send_token_issued_at: Option<Date>,
}

impl PushPairing {
    /// Re-mint this long after issue. The relay refuses tokens older than its
    /// TTL (45 days by default); refreshing at a third of that means a host
    /// only loses pushes after a month and a half with no connection from
    /// this phone.
    pub const REFRESH_AFTER_SECONDS: f64 = 14.0 * 24.0 * 3600.0;

    /// Mint the end-to-end secret for a connection (`PushPairing.make`): the
    /// secret from the system CSPRNG, the relay address trimmed of spaces
    /// and one trailing `/`. The send token is left to the relay.
    pub fn make(connection_id: Uuid, host_label: &str, relay_url: &str, now: Date) -> Self {
        Self {
            connection_id,
            host_label: host_label.to_owned(),
            secret_hex: crate::random::hex(32),
            send_token: String::new(),
            relay_url: normalise_relay_url(relay_url),
            created_at: now,
            apns_token: None,
            apns_env: None,
            send_token_issued_at: None,
        }
    }

    /// Whether the credential must be (re)minted before this pairing is
    /// worth installing: never minted, minted for a device token this phone
    /// no longer has, or old enough that expiry is in sight.
    pub fn needs_mint(&self, current_token: Option<&str>, now: Date) -> bool {
        if self.send_token.is_empty() {
            return true;
        }
        let (Some(apns_token), Some(issued)) = (&self.apns_token, self.send_token_issued_at) else {
            return true;
        };
        if let Some(current) = current_token
            && !same_token(apns_token, current)
        {
            return true;
        }
        now.seconds_since(issued) > Self::REFRESH_AFTER_SECONDS
    }

    /// A conf written from a pairing missing any of these is one the sender
    /// skips (at best) or the relay refuses (at worst) — never install it.
    pub fn is_ready_to_install(&self) -> bool {
        !self.send_token.is_empty()
            && self
                .apns_token
                .as_deref()
                .is_some_and(|token| !token.is_empty())
            && self.send_token_issued_at.is_some()
    }

    /// Store a fresh mint (`PushService.mintAndStore`): the token, when it was
    /// minted, and the device token and environment it was minted over.
    pub fn minted(
        &self,
        send_token: &str,
        issued_at: Date,
        device_token: &str,
        environment: &str,
    ) -> Self {
        Self {
            send_token: send_token.to_owned(),
            send_token_issued_at: Some(issued_at),
            apns_token: Some(device_token.to_owned()),
            apns_env: Some(environment.to_owned()),
            ..self.clone()
        }
    }
}

/// Device tokens compare without regard to case (`lowercased()` on both).
fn same_token(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// `relayURL.trimmingCharacters(in: .whitespaces).trimmingSuffix("/")`.
pub fn normalise_relay_url(relay_url: &str) -> String {
    let trimmed = text::trim_inline_spaces(relay_url);
    // `hasSuffix` and `dropLast` count Characters: a "/" carrying a combining
    // mark is no slash.
    match text::characters(trimmed).next_back() {
        Some("/") => trimmed[..trimmed.len() - 1].to_owned(),
        _ => trimmed.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Date {
        Date::from_unix(1_762_800_000.0)
    }

    /// `PushPairingTests.material`.
    #[test]
    fn make_mints_the_secret_locally_and_leaves_the_credential_to_the_relay() {
        let a = PushPairing::make(Uuid::new_v4(), "m1-pro", "https://push.example.org", now());
        let b = PushPairing::make(Uuid::new_v4(), "m1-pro", "https://push.example.org", now());
        for pairing in [&a, &b] {
            assert_eq!(pairing.secret_hex.len(), 64);
            assert!(crate::foundation::hex::decode(&pairing.secret_hex).is_some());
            assert!(pairing.send_token.is_empty());
            assert!(!pairing.is_ready_to_install());
            assert!(pairing.needs_mint(None, now()));
        }
        assert_ne!(a.secret_hex, b.secret_hex);
    }

    /// `PushPairingTests.mintLifecycle`.
    #[test]
    fn a_credential_re_mints_when_the_device_token_rotates_or_age_nears_the_relay_ttl() {
        let mut pairing = PushPairing::make(Uuid::new_v4(), "h", "https://r", now());
        pairing.send_token = "ab".repeat(32);
        pairing.apns_token = Some("CD".repeat(32));
        pairing.apns_env = Some("production".into());
        pairing.send_token_issued_at = Some(now());

        assert!(pairing.is_ready_to_install());
        // Same token, fresh mint: nothing to do. Hex casing must not split it.
        assert!(!pairing.needs_mint(Some(&"cd".repeat(32)), now()));
        // The phone's token rotated: the HMAC no longer matches.
        assert!(pairing.needs_mint(Some(&"ee".repeat(32)), now()));
        // Ageing towards the relay-side TTL re-mints long before expiry.
        assert!(pairing.needs_mint(
            Some(&"cd".repeat(32)),
            now().adding(PushPairing::REFRESH_AFTER_SECONDS + 1.0)
        ));
        assert!(!pairing.needs_mint(
            Some(&"cd".repeat(32)),
            now().adding(PushPairing::REFRESH_AFTER_SECONDS)
        ));
        // A pairing stored by an older build (no routing fields) needs a mint.
        pairing.apns_token = None;
        assert!(pairing.needs_mint(Some(&"cd".repeat(32)), now()));
        assert!(!pairing.is_ready_to_install());
    }

    #[test]
    fn no_current_token_trusts_the_stored_one() {
        let mut pairing = PushPairing::make(Uuid::new_v4(), "h", "https://r", now());
        pairing = pairing.minted("tok", now(), &"ab".repeat(32), "production");
        assert!(!pairing.needs_mint(None, now()));
        pairing.apns_token = Some(String::new());
        assert!(!pairing.is_ready_to_install());
    }

    /// `PushPairingTests.urlNormalisation`.
    #[test]
    fn a_trailing_slash_on_the_relay_url_does_not_become_a_double_slash() {
        let pairing =
            PushPairing::make(Uuid::new_v4(), "h", "  https://push.example.org/  ", now());
        assert_eq!(pairing.relay_url, "https://push.example.org");
        assert_eq!(normalise_relay_url("https://r//"), "https://r/");
        assert_eq!(normalise_relay_url("\u{A0}https://r\t"), "https://r");
        assert_eq!(
            normalise_relay_url("https://r/\u{301}"),
            "https://r/\u{301}"
        );
    }
}
