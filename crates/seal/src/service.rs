//! The decisions of the phone's push service (`PushService`) that do not
//! need a platform: when to re-mint, when a foreground return is worth a
//! round, how mint requests made during a round are kept, whether a relay's
//! late answer may still be stored, and which pairings outlived their
//! connection. Asking the OS for a device token and talking HTTP to the
//! relay stay with the platform.

use std::collections::HashSet;

use crate::foundation::date::Date;
use crate::foundation::uuid::Uuid;
use crate::pairing::PushPairing;

/// Seconds a successful round keeps a foreground return from starting
/// another (`.active` fires every time the app switcher is scrubbed past).
pub const REFRESH_FLOOR_SECONDS: f64 = 60.0;

/// Whether a return to the foreground skips its re-mint round
/// (`PushService.shouldSkipRefresh`): only right after a success, never
/// while a failure is pending.
pub fn should_skip_refresh(
    sync_pending: bool,
    last_success: Option<Date>,
    now: Date,
    floor_seconds: f64,
) -> bool {
    match last_success {
        Some(last) if !sync_pending => now.seconds_since(last) < floor_seconds,
        _ => false,
    }
}

/// One round at a time, and none asked for during it lost (B73): a request
/// while a round is out is remembered, and the round runs once more when it
/// ends — once however many came in, over the device token current by then.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MintRounds {
    in_flight: bool,
    requested: bool,
}

impl MintRounds {
    /// A round is wanted. `true`: start one now. `false`: one is out, and it
    /// will run again when it ends.
    pub fn request(&mut self) -> bool {
        if self.in_flight {
            self.requested = true;
            false
        } else {
            self.in_flight = true;
            self.requested = false;
            true
        }
    }

    /// The round ended. `true`: run another (something asked meanwhile).
    pub fn round_ended(&mut self) -> bool {
        if self.requested {
            self.requested = false;
            true
        } else {
            self.in_flight = false;
            false
        }
    }

    /// The round could not even start (no device token yet).
    pub fn abandon(&mut self) {
        self.in_flight = false;
        self.requested = false;
    }

    pub fn is_in_flight(&self) -> bool {
        self.in_flight
    }
}

/// The pairings a round over `device_token` must mint, in stored order.
pub fn needing_mint(pairings: &[PushPairing], device_token: &str, now: Date) -> Vec<Uuid> {
    pairings
        .iter()
        .filter(|p| p.needs_mint(Some(device_token), now))
        .map(|p| p.connection_id)
        .collect()
}

/// The pairing to store once the relay answered a mint asked for `asked`
/// (`PushService.mintAndStore`), or `None` when the pairing moved on while
/// the relay was answering — forgotten (its connection deleted, its host
/// unpaired) or replaced (re-paired, or pointed at another relay). Storing
/// the answer regardless brought a forgotten pairing back to life.
pub fn store_minted(
    stored_now: Option<&PushPairing>,
    asked: &PushPairing,
    send_token: &str,
    issued_at: Date,
    device_token: &str,
    environment: &str,
) -> Option<PushPairing> {
    let stored = stored_now?;
    (stored.secret_hex == asked.secret_hex && stored.relay_url == asked.relay_url)
        .then(|| stored.minted(send_token, issued_at, device_token, environment))
}

/// The pairings whose connection is no longer saved
/// (`forgetOrphanedPairingsOnce`). `saved` is every connection the store
/// holds, decoded or not; `None` when that can't be told yet (a launch before
/// the first unlock reads the sealed list as empty), and then nothing is.
pub fn orphaned(pairings: &[PushPairing], saved: Option<&HashSet<Uuid>>) -> Vec<Uuid> {
    let Some(saved) = saved else {
        return Vec::new();
    };
    pairings
        .iter()
        .filter(|p| !saved.contains(&p.connection_id))
        .map(|p| p.connection_id)
        .collect()
}

/// Why a relay address was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadRelayUrl;

/// The mint endpoint of a relay (`PushRelayClient.mint`): `https`, or plain
/// `http` only to `localhost` / `127.0.0.1` (a relay on the same machine).
///
/// Foundation's `URL(string:)` is the judge on iOS; this checks only what the
/// rule needs — a scheme, `://`, a host free of spaces — and refuses
/// anything else.
pub fn mint_url(relay_url: &str) -> Result<String, BadRelayUrl> {
    let url = format!("{relay_url}/v1/mint");
    let (scheme, rest) = url.split_once("://").ok_or(BadRelayUrl)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) || host.is_empty() {
        return Err(BadRelayUrl);
    }
    let scheme = scheme.to_ascii_lowercase();
    if scheme == "https" || (scheme == "http" && (host == "localhost" || host == "127.0.0.1")) {
        Ok(url)
    } else {
        Err(BadRelayUrl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - PushRefreshFloorTests

    #[test]
    fn a_recent_success_is_not_re_announced() {
        let now = Date::now();
        assert!(should_skip_refresh(
            false,
            Some(now.adding(-5.0)),
            now,
            REFRESH_FLOOR_SECONDS
        ));
    }

    #[test]
    fn an_older_success_is_re_announced() {
        let now = Date::now();
        assert!(!should_skip_refresh(
            false,
            Some(now.adding(-120.0)),
            now,
            REFRESH_FLOOR_SECONDS
        ));
    }

    #[test]
    fn a_pending_failure_ignores_the_floor() {
        let now = Date::now();
        assert!(!should_skip_refresh(
            true,
            Some(now.adding(-1.0)),
            now,
            REFRESH_FLOOR_SECONDS
        ));
    }

    #[test]
    fn never_having_succeeded_always_announces() {
        assert!(!should_skip_refresh(
            false,
            None,
            Date::now(),
            REFRESH_FLOOR_SECONDS
        ));
    }

    // MARK: - PushMintRoundTests

    /// A relay that holds its first answer; the token changes mid-round.
    #[test]
    fn a_device_token_that_changes_mid_round_is_minted_over_when_the_round_ends() {
        let mut rounds = MintRounds::default();
        let mut minted_over: Vec<&str> = Vec::new();
        let mut token = "aa";
        assert!(rounds.request());
        minted_over.push(token); // first host, held at the relay
        token = "bb"; // iOS hands over a new token: handle(deviceToken:) asks for a round
        assert!(!rounds.request(), "a round is out: remembered, not started");
        minted_over.push("aa"); // the round's second host, still over its token
        assert!(
            rounds.round_ended(),
            "the request made meanwhile runs one more round"
        );
        minted_over.push(token);
        minted_over.push(token);
        assert!(!rounds.round_ended());
        assert!(!rounds.is_in_flight());
        assert_eq!(minted_over, ["aa", "aa", "bb", "bb"]);
    }

    #[test]
    fn mints_asked_for_during_a_round_make_one_more_round() {
        let mut rounds = MintRounds::default();
        assert!(rounds.request());
        assert!(!rounds.request());
        assert!(!rounds.request());
        assert!(rounds.round_ended());
        assert!(!rounds.round_ended());
        assert!(
            rounds.request(),
            "idle again: the next request starts a round"
        );
    }

    #[test]
    fn a_pairing_forgotten_or_replaced_while_minting_is_not_stored() {
        let now = Date::now();
        let asked = PushPairing::make(Uuid::new_v4(), "a", "https://r", now);
        let stored = store_minted(Some(&asked), &asked, "tok", now, "dev", "production").unwrap();
        assert_eq!(stored.send_token, "tok");
        assert_eq!(stored.apns_token.as_deref(), Some("dev"));
        assert_eq!(stored.apns_env.as_deref(), Some("production"));
        assert!(
            store_minted(None, &asked, "tok", now, "dev", "production").is_none(),
            "forgotten"
        );
        let repaired = PushPairing::make(asked.connection_id, "a", "https://r", now);
        assert!(
            store_minted(Some(&repaired), &asked, "tok", now, "dev", "production").is_none(),
            "re-paired"
        );
        let mut moved = asked.clone();
        moved.relay_url = "https://other".into();
        assert!(
            store_minted(Some(&moved), &asked, "tok", now, "dev", "production").is_none(),
            "another relay"
        );
    }

    #[test]
    fn a_round_mints_only_what_needs_it() {
        let now = Date::now();
        let fresh = PushPairing::make(Uuid::new_v4(), "a", "https://r", now).minted(
            "t",
            now,
            "dev",
            "production",
        );
        let stale = PushPairing::make(Uuid::new_v4(), "b", "https://r", now);
        assert_eq!(
            needing_mint(&[fresh.clone(), stale.clone()], "dev", now),
            [stale.connection_id]
        );
        assert_eq!(
            needing_mint(std::slice::from_ref(&fresh), "rotated", now),
            [fresh.connection_id]
        );
    }

    #[test]
    fn pairings_whose_connection_is_gone() {
        let now = Date::now();
        let kept = PushPairing::make(Uuid::new_v4(), "a", "https://r", now);
        let gone = PushPairing::make(Uuid::new_v4(), "b", "https://r", now);
        let saved = HashSet::from([kept.connection_id]);
        assert_eq!(
            orphaned(&[kept.clone(), gone.clone()], Some(&saved)),
            [gone.connection_id]
        );
        assert!(
            orphaned(&[kept, gone], None).is_empty(),
            "an untrusted store sweeps nothing"
        );
    }

    #[test]
    fn the_mint_endpoint() {
        assert_eq!(
            mint_url("https://push.example.org").unwrap(),
            "https://push.example.org/v1/mint"
        );
        assert!(mint_url("http://localhost:8080").is_ok());
        assert!(mint_url("http://127.0.0.1").is_ok());
        assert!(mint_url("http://push.example.org").is_err());
        assert!(mint_url("push.example.org").is_err());
        assert!(mint_url("https://").is_err());
        assert!(mint_url("https://push example.org").is_err());
    }
}
