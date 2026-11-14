//! The phone's push side (`PushServiceTests`, the push half of
//! `ConnectionDeletionTests`): minting at the relay and keeping it fresh,
//! the token's waiters, a delete's handshake, the sweep of pairings whose
//! connection is gone.

mod support;

use moshpit_hostcare::{Note, RelayFailure};
use moshpit_seal::PushPairing;
use support::*;

fn stored(rig: &mut Rig, pairing: PushPairing) {
    rig.store.0.lock().unwrap().pairings.push(pairing);
}

fn unminted(rig: &Rig, label: &str) -> PushPairing {
    PushPairing::make(moshpit_seal::Uuid::new_v4(), label, RELAY, rig.date)
}

#[test]
fn a_new_token_mints_every_pairing_that_needs_it_and_only_those() {
    let mut rig = Rig::new();
    let fresh = ready_pairing(moshpit_seal::Uuid::new_v4());
    let stale = unminted(&rig, "b");
    stored(&mut rig, fresh.clone());
    stored(&mut rig, stale.clone());
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    assert_eq!(rig.mints, [stale.connection_id.to_string()]);
    let minted = rig.pairing(stale.connection_id).unwrap();
    assert!(!minted.send_token.is_empty());
    assert_eq!(minted.apns_token.as_deref(), Some(DEVICE_TOKEN));
    assert_eq!(minted.apns_env.as_deref(), Some("production"));
    assert_eq!(rig.machine.relay_error(), None);
}

/// B73: a request made while a round is out is not dropped — the round
/// runs once more, over the token current by then.
#[test]
fn a_device_token_that_changes_mid_round_is_minted_over_when_the_round_ends() {
    let mut rig = Rig::new();
    let a = unminted(&rig, "a");
    stored(&mut rig, a.clone());
    rig.relay = Relay::Holds;
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    assert_eq!(rig.held_posts.len(), 1);
    let rotated = "cd".repeat(32);
    rig.act(|m, k| m.device_token(rotated.clone(), k));
    assert_eq!(rig.held_posts.len(), 1, "one round at a time");
    rig.answer_mint(0);
    assert_eq!(
        rig.held_posts.len(),
        1,
        "the request made meanwhile runs a round"
    );
    assert!(rig.held_posts[0].2.contains(&rotated));
    rig.answer_mint(0);
    assert_eq!(
        rig.pairing(a.connection_id).unwrap().apns_token,
        Some(rotated)
    );
    assert!(rig.held_posts.is_empty());
}

/// The app's `PushMintRoundTests`: two hosts, the token rotated while the
/// first host's mint is out — both end minted over the new token.
#[test]
fn two_pairings_end_minted_over_the_token_that_changed_mid_round() {
    let mut rig = Rig::new();
    let a = unminted(&rig, "a");
    let b = unminted(&rig, "b");
    stored(&mut rig, a.clone());
    stored(&mut rig, b.clone());
    rig.relay = Relay::Holds;
    let (old, new) = ("aa".repeat(32), "bb".repeat(32));
    rig.act(|m, k| m.device_token(old.clone(), k));
    assert_eq!(rig.held_posts.len(), 1, "the round reached the relay");
    rig.act(|m, k| m.device_token(new.clone(), k));
    while !rig.held_posts.is_empty() {
        rig.answer_mint(0);
    }
    let all = rig.store.read();
    assert_eq!(all.len(), 2);
    assert!(
        all.iter()
            .all(|p| p.apns_token.as_deref() == Some(new.as_str())),
        "pairings left minted over a device token the phone no longer has"
    );
    assert!(all.iter().all(|p| !p.needs_mint(Some(&new), rig.date)));
}

#[test]
fn mints_asked_for_during_a_round_make_one_more_round_over_the_latest_token() {
    let mut rig = Rig::new();
    let (a, b) = (unminted(&rig, "a"), unminted(&rig, "b"));
    stored(&mut rig, a);
    stored(&mut rig, b);
    rig.relay = Relay::Holds;
    let (first, second, third) = ("11".repeat(32), "22".repeat(32), "33".repeat(32));
    rig.act(|m, k| m.device_token(first.clone(), k));
    rig.act(|m, k| m.device_token(second, k));
    rig.act(|m, k| m.device_token(third.clone(), k));
    let mut minted_over = Vec::new();
    while !rig.held_posts.is_empty() {
        let body: serde_json::Value = serde_json::from_str(&rig.held_posts[0].2).unwrap();
        minted_over.push(body["apnsToken"].as_str().unwrap().to_owned());
        rig.answer_mint(0);
    }
    assert_eq!(
        minted_over,
        [first.clone(), first, third.clone(), third],
        "the round, then one more over the latest token"
    );
}

#[test]
fn a_relay_that_refuses_is_said_and_retried_on_the_next_foreground() {
    let mut rig = Rig::new();
    let a = unminted(&rig, "a");
    stored(&mut rig, a.clone());
    rig.relay = Relay::Refuses(429, "slow down".into());
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    assert_eq!(
        rig.machine.relay_error(),
        Some(Note::Relay(RelayFailure::Status {
            code: 429,
            body: "slow down".into()
        }))
    );
    assert_eq!(
        rig.mints.len(),
        1,
        "a status is the relay's answer: not retried"
    );
    rig.relay = Relay::Mints;
    rig.act(|m, k| m.foreground(k));
    assert_eq!(rig.mints.len(), 2, "a pending failure ignores the floor");
    assert_eq!(rig.machine.relay_error(), None);
    // And now the floor holds.
    rig.act(|m, k| m.foreground(k));
    assert_eq!(rig.mints.len(), 2);
}

#[test]
fn a_mint_that_doesnt_reach_the_relay_is_tried_once_more_two_seconds_on() {
    let mut rig = Rig::new();
    let a = unminted(&rig, "a");
    stored(&mut rig, a.clone());
    rig.relay = Relay::Down;
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    assert_eq!(rig.mints.len(), 1);
    rig.advance(1_999);
    assert_eq!(rig.mints.len(), 1);
    rig.relay = Relay::Mints;
    rig.advance(1);
    assert_eq!(rig.mints.len(), 2);
    assert!(!rig.pairing(a.connection_id).unwrap().send_token.is_empty());
}

#[test]
fn a_relay_address_that_isnt_https_is_refused_before_anything_is_sent() {
    let mut rig = Rig::new();
    let mut a = unminted(&rig, "a");
    a.relay_url = "http://push.example.org".into();
    stored(&mut rig, a);
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    assert!(rig.mints.is_empty());
    assert_eq!(
        rig.machine.relay_error(),
        Some(Note::Relay(RelayFailure::BadUrl))
    );
}

#[test]
fn the_foreground_re_mints_at_most_once_a_minute_after_a_success() {
    let mut rig = Rig::new();
    let a = unminted(&rig, "a");
    stored(&mut rig, a.clone());
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    assert_eq!(rig.mints.len(), 1);
    // Something needs a mint again (a stale credential).
    let mut stale = rig.pairing(a.connection_id).unwrap();
    stale.send_token = String::new();
    rig.store.0.lock().unwrap().pairings = vec![stale];
    rig.advance(5_000);
    rig.act(|m, k| m.foreground(k));
    assert_eq!(rig.mints.len(), 1, "inside the floor");
    rig.advance(60_000);
    rig.act(|m, k| m.foreground(k));
    assert_eq!(rig.mints.len(), 2);
}

#[test]
fn nothing_paired_never_prompts() {
    let mut rig = Rig::new();
    rig.act(|m, k| m.register_for_push(k));
    assert_eq!(rig.registered, 0);
    stored(&mut rig, ready_pairing(moshpit_seal::Uuid::new_v4()));
    rig.act(|m, k| m.register_for_push(k));
    assert_eq!(rig.registered, 1);
}

#[test]
fn a_refused_registration_is_said_and_asked_again_on_the_foreground() {
    let mut rig = Rig::new();
    let a = unminted(&rig, "a");
    stored(&mut rig, a);
    rig.act(|m, k| m.registration_failed("no aps-environment".into(), k));
    assert_eq!(
        rig.machine.relay_error(),
        Some(Note::Said("no aps-environment".into()))
    );
    rig.act(|m, k| m.foreground(k));
    assert_eq!(rig.registered, 1, "a pending failure asks the OS again");
}

#[test]
fn a_new_token_clears_the_relay_error() {
    let mut rig = Rig::new();
    rig.act(|m, k| m.registration_failed("offline".into(), k));
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    assert_eq!(rig.machine.relay_error(), None);
}

// MARK: - Deleting a connection

#[test]
fn deleting_a_connection_forgets_its_pairing_and_its_token_is_never_minted_again() {
    let mut rig = Rig::new();
    let a = unminted(&rig, "a");
    stored(&mut rig, a.clone());
    rig.act(|m, k| m.deletion_began(a.connection_id, k).unwrap());
    rig.act(|m, k| m.deletion_ended(a.connection_id, k));
    assert!(rig.pairing(a.connection_id).is_none());
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    assert!(rig.mints.is_empty());
}

#[test]
fn a_pairing_deleted_while_its_token_is_out_at_the_relay_stays_deleted() {
    let mut rig = Rig::new();
    let a = unminted(&rig, "a");
    stored(&mut rig, a.clone());
    rig.relay = Relay::Holds;
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    assert_eq!(rig.held_posts.len(), 1);
    rig.act(|m, k| m.deletion_began(a.connection_id, k).unwrap());
    rig.answer_mint(0);
    assert!(
        rig.pairing(a.connection_id).is_none(),
        "the relay's late answer brought a forgotten pairing back"
    );
}

#[test]
fn a_pairing_replaced_while_its_token_is_out_stays_replaced() {
    let mut rig = Rig::new();
    let a = unminted(&rig, "a");
    stored(&mut rig, a.clone());
    rig.relay = Relay::Holds;
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    let replacement = PushPairing::make(a.connection_id, "a", RELAY, rig.date);
    rig.store.0.lock().unwrap().pairings = vec![replacement.clone()];
    rig.answer_mint(0);
    let now = rig.pairing(a.connection_id).unwrap();
    assert_eq!(now.secret_hex, replacement.secret_hex);
    assert!(now.send_token.is_empty() || rig.mints.len() > 1);
}

#[test]
fn a_phone_that_cant_forget_the_pairing_keeps_the_connection_and_says_so() {
    let mut rig = Rig::new();
    let a = ready_pairing(moshpit_seal::Uuid::new_v4());
    stored(&mut rig, a.clone());
    rig.store.0.lock().unwrap().refuse_writes = true;
    let clock = rig.clock();
    let result = rig.machine.deletion_began(a.connection_id, clock);
    rig.settle();
    assert_eq!(result, Err(Note::Relay(RelayFailure::CouldNotForget)));
    assert!(rig.pairing(a.connection_id).is_some());
}

#[test]
fn a_connection_never_paired_deletes_as_before() {
    let mut rig = Rig::new();
    let c = conn("never");
    rig.act(|m, k| m.deletion_began(c.id, k).unwrap());
    rig.act(|m, k| m.deletion_ended(c.id, k));
}

// MARK: - A connection moved to another host

/// B328: a connection saved pointing at another host kept its pairing, so
/// the box it pointed at before — its hooks, its secret, its send token —
/// went on pushing under the connection. The phone forgets it now, as a
/// delete does; nothing more is minted for it, and care pairs the new
/// host on its next pass.
#[test]
fn a_connection_moved_to_another_host_forgets_its_pairing() {
    let mut rig = Rig::new();
    let a = unminted(&rig, "a");
    stored(&mut rig, a.clone());
    rig.act(|m, k| m.host_changed(a.connection_id, k).unwrap());
    assert!(rig.pairing(a.connection_id).is_none());
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    assert!(rig.mints.is_empty(), "the old box's token was minted again");
}

#[test]
fn a_moved_connection_never_paired_moves_as_before() {
    let mut rig = Rig::new();
    let c = conn("never");
    rig.act(|m, k| m.host_changed(c.id, k).unwrap());
    assert!(rig.pairing(c.id).is_none());
}

#[test]
fn a_phone_that_cant_forget_a_moved_connections_pairing_says_so() {
    let mut rig = Rig::new();
    let a = ready_pairing(moshpit_seal::Uuid::new_v4());
    stored(&mut rig, a.clone());
    rig.store.0.lock().unwrap().refuse_writes = true;
    let clock = rig.clock();
    let result = rig.machine.host_changed(a.connection_id, clock);
    rig.settle();
    assert_eq!(result, Err(Note::Relay(RelayFailure::CouldNotForget)));
    assert!(rig.pairing(a.connection_id).is_some());
}

// MARK: - Pairings whose connection is gone

#[test]
fn pairings_left_by_deleted_connections_are_forgotten_once_the_store_can_be_trusted() {
    let mut rig = Rig::new();
    let kept = ready_pairing(moshpit_seal::Uuid::new_v4());
    let gone = ready_pairing(moshpit_seal::Uuid::new_v4());
    stored(&mut rig, kept.clone());
    stored(&mut rig, gone.clone());
    // Before the first unlock: which connections are saved can't be told.
    let clock = rig.clock();
    rig.machine.set_saved(&[], None, clock);
    assert!(rig.machine.forget_orphaned_pairings(clock).is_empty());
    assert_eq!(rig.store.read().len(), 2);
    rig.save(&[kept.connection_id]);
    let clock = rig.clock();
    let forgotten = rig.machine.forget_orphaned_pairings(clock);
    assert_eq!(forgotten, std::slice::from_ref(&gone));
    assert_eq!(rig.store.read(), std::slice::from_ref(&kept));
    // Once a launch.
    stored(&mut rig, gone.clone());
    assert!(rig.machine.forget_orphaned_pairings(clock).is_empty());
}

#[test]
fn a_connection_this_build_cant_decode_keeps_its_pairing() {
    let mut rig = Rig::new();
    let undecoded = ready_pairing(moshpit_seal::Uuid::new_v4());
    stored(&mut rig, undecoded.clone());
    let clock = rig.clock();
    rig.machine
        .set_saved(&[], Some(&[undecoded.connection_id]), clock);
    assert!(rig.machine.forget_orphaned_pairings(clock).is_empty());
}

// MARK: - Without a push service (Android)

#[test]
fn a_build_without_push_pairs_nothing_and_says_why() {
    let mut rig = Rig::with_config(moshpit_hostcare::Config {
        push: moshpit_hostcare::PushSupport::Unavailable,
        autocare_off: false,
        budgets: Default::default(),
    });
    let c = conn("m1-pro");
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_pair(page, k));
    let view = rig.view(page);
    assert_eq!(view.error, Some(Note::Relay(RelayFailure::Unavailable)));
    assert_eq!(
        view.relay_error,
        Some(Note::Relay(RelayFailure::Unavailable))
    );
    assert!(matches!(
        view.push_proof,
        moshpit_hostcare::Proof::Unavailable(_)
    ));
    assert!(rig.mints.is_empty());
    rig.act(|m, k| m.register_for_push(k));
    assert_eq!(rig.registered, 0);
}

/// B357: the relay's send token went into the host's push conf as it came,
/// and the sender sourced that conf: a relay answering with shell in the
/// token, or a second line, ran it on every host paired through it. A token
/// the relay couldn't have minted is a malformed answer, and nothing of it
/// reaches the phone's pairing or a host.
#[test]
fn a_send_token_the_relay_couldnt_have_minted_never_reaches_a_host() {
    for token in [
        "x;touch /tmp/moshpit-pwned".to_owned(),
        format!("{}\\nRELAY_URL=https://evil.example", "ab".repeat(16)),
        format!("{}$(id)", &"ab".repeat(32)[..59]),
    ] {
        let mut rig = Rig::new().with_token();
        let c = conn("m1-pro");
        rig.save(&[c.id]);
        rig.set_host(c.id, Host::hooked());
        rig.relay = Relay::Refuses(
            200,
            format!(r#"{{"ok":true,"sendToken":"{token}","iat":1762800000}}"#),
        );
        rig.tend(&c);
        assert_eq!(rig.mints.len(), 1, "{token:?}");
        assert!(rig.host(c.id).pairing_conf(c.id).is_none(), "{token:?}");
        assert!(
            rig.host(c.id)
                .commands
                .iter()
                .all(|c| !c.contains("push.d")),
            "{token:?}"
        );
        let phone = rig.pairing(c.id);
        assert!(
            phone.as_ref().is_none_or(|p| p.send_token.is_empty()),
            "{token:?}: {phone:?}"
        );
    }
}
