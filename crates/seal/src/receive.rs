//! What a notification service does with a push that just arrived
//! (`NotificationService.didReceive`): the iOS extension's flow, for a
//! platform whose push service runs Rust (Android's FCM service).
//!
//! 1. Not one of ours (no envelope): show it as received.
//! 2. Ours, but no pairing secret opens it — a host paired to another
//!    install, or the store unreachable: the relay's translated fallback,
//!    with no actions (they would need the pane the envelope hides).
//! 3. Opened: an `attention` records its prompt as standing (which says
//!    whether this is the 0→1 edge and how many wait); any other state
//!    closes its pane's wait. Then the card, named the way the phone names
//!    the connection.
//!
//! Every path honours the user's sound switch, and leaves the diagnostic
//! lines the app's screen shows.

use crate::copy::{self, Language};
use crate::finishes::PushFinishesFile;
use crate::envelope::Envelope;
use crate::foundation::date::Date;
use crate::foundation::json::Value;
use crate::foundation::uuid::Uuid;
use crate::notification::{self, Card, Opening, Render};
use crate::pairing::PushPairing;
use crate::prefs::Prefs;
use crate::standing::{Entry, PushStandingFile};

/// What to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// No sealed envelope: the notification as it came.
    AsReceived,
    /// An envelope none of this device's secrets opened — or one a key
    /// opened that names another connection than the key's (B435): the
    /// relay's fallback text, no actions. `secrets_tried` is the
    /// diagnosis: 0 means the pairing store did not reach the service, more
    /// means the sending host is paired to a different install.
    Fallback { secrets_tried: usize },
    /// Opened and rendered.
    Card(Card),
}

/// The outcome of one push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    pub delivery: Delivery,
    /// Whether the notification may keep a sound at all (the user's switch;
    /// a [`Card`] has made its own decision within it).
    pub sound_allowed: bool,
    /// The lines to leave in the diagnostics ring, in order.
    pub diagnostics: Vec<String>,
}

/// One push through the service. `payload` is the push's data as JSON
/// (`userInfo`: an object carrying `mp`).
pub fn receive(
    payload: &Value,
    pairings: &[PushPairing],
    standing: &PushStandingFile,
    finishes: &PushFinishesFile,
    prefs: Prefs,
    language: Language,
    now: Date,
) -> Received {
    let mut diagnostics = vec!["woke for a push".to_owned()];
    let sound_allowed = prefs.sound;
    let Some(envelope) = Envelope::from_payload(payload) else {
        diagnostics.push("no sealed envelope — delivered as received".to_owned());
        return Received {
            delivery: Delivery::AsReceived,
            sound_allowed,
            diagnostics,
        };
    };
    let status = match notification::open_paired(&envelope, pairings) {
        Opening::Opened(status) => status,
        Opening::Misnamed => {
            // Sealed with one connection's key, naming another: nothing in
            // it is shown, as if no key had opened it (B435).
            diagnostics.push(
                "a push opened with one connection's key named another — showed the fallback".to_owned(),
            );
            return Received {
                delivery: Delivery::Fallback {
                    secrets_tried: pairings.len(),
                },
                sound_allowed,
                diagnostics,
            };
        }
        Opening::Unopened => {
            diagnostics.push(format!(
                "no key opened this envelope ({} available) — showed the fallback",
                pairings.len()
            ));
            return Received {
                delivery: Delivery::Fallback {
                    secrets_tried: pairings.len(),
                },
                sound_allowed,
                diagnostics,
            };
        }
    };
    let self_test = if status.agent.as_deref() == Some(notification::SELF_TEST_AGENT) {
        format!(" (self-test {})", status.title.as_deref().unwrap_or("?"))
    } else {
        String::new()
    };
    diagnostics.push(format!(
        "opened a {} push from {}{self_test}",
        status.state, status.host
    ));
    let label = Uuid::parse(&status.conn)
        .and_then(|id| pairings.iter().find(|p| p.connection_id == id))
        .map(|p| p.host_label.clone());
    let how = Render {
        prefs,
        label: label.as_deref(),
        language,
        ..Render::default()
    };
    let card = if status.agent.as_deref() == Some(notification::SELF_TEST_AGENT) {
        // A pairing self-test is no agent's turn (B439): it neither joins
        // nor leaves the waiting set, and rings no finish. Its "done" used
        // to clear the pane it named — `%0`, where an agent often waits.
        notification::render(&status, how)
    } else if status.state == "attention" {
        let edge = standing.note_standing(
            &status.conn,
            Entry {
                pane: status.pane.clone(),
                agent: status.agent.clone(),
                title: status.title.clone(),
                location: None,
                since: status.ts,
                recorded_at: now,
            },
            now,
        );
        let count = standing.standing(&status.conn, now).len();
        notification::render(
            &status,
            Render {
                attention_edge: edge,
                standing_count: i64::try_from(count).unwrap_or(i64::MAX),
                ..how
            },
        )
    } else {
        standing.clear(&status.conn, &status.pane, now);
        // A long turn's finish rings once between this card and the app's
        // local copy of it: whichever lands first.
        let done_rings = status.state != "done"
            || !copy::done_rings(
                status.dur.unwrap_or(0),
                prefs.sound,
                notification::DONE_SOUND_THRESHOLD,
            )
            || finishes.note_ring(&status.conn, &status.pane, status.ts, now);
        notification::render(&status, Render { done_rings, ..how })
    };
    Received {
        delivery: Delivery::Card(card),
        sound_allowed,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copy::InterruptionLevel;
    use crate::envelope::seal;
    use crate::foundation::json::{self, Dialect};
    use crate::status::Status;
    use crate::store::testing::TempDir;

    const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const CONN: &str = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";

    fn pairing(secret: &str, label: &str) -> PushPairing {
        PushPairing {
            connection_id: Uuid::parse(CONN).unwrap(),
            host_label: label.into(),
            secret_hex: secret.into(),
            send_token: "0".repeat(64),
            relay_url: "https://push.example.org".into(),
            created_at: Date::now(),
            apns_token: None,
            apns_env: None,
            send_token_issued_at: None,
        }
    }

    /// A payload in the shape the relay produces.
    fn payload(state: &str, title: Option<&str>, key: &str) -> Value {
        payload_naming(CONN, state, title, key)
    }

    /// [`payload`] for a status that names `conn`.
    fn payload_naming(conn: &str, state: &str, title: Option<&str>, key: &str) -> Value {
        let status = Status {
            conn: conn.into(),
            host: "m1-pro".into(),
            sess: Some("work".into()),
            pane: "%3".into(),
            agent: Some("claude".into()),
            state: state.into(),
            title: title.map(str::to_owned),
            ts: Date::now().unix() as i64,
            dur: None,
        };
        let sealed = seal(status.to_json().as_bytes(), key, None).unwrap();
        let text = format!(
            r#"{{"aps":{{"alert":{{"title-loc-key":"An agent needs you","loc-key":"Open Moshpit to see what it is asking."}},"mutable-content":1}},"mp":{}}}"#,
            json::encode(&sealed)
        );
        json::parse(
            text.as_bytes(),
            Dialect::Serialization {
                fragments_allowed: false,
            },
        )
        .unwrap()
    }

    fn deliver(payload: &Value, pairings: &[PushPairing], dir: &TempDir) -> Received {
        let standing = PushStandingFile::new(dir.path(PushStandingFile::FILE_NAME));
        receive(
            payload,
            pairings,
            &standing,
            &PushFinishesFile::new(dir.path("rung-finishes.json")),
            Prefs::default(),
            Language::En,
            Date::now(),
        )
    }

    fn card(received: Received) -> Card {
        match received.delivery {
            Delivery::Card(card) => card,
            other => panic!("not a card: {other:?}"),
        }
    }

    // MARK: - PushNotificationServiceTests

    /// B439: the push test named `%0` (it runs outside any pane), and its
    /// "done" cleared that pane's waiting entry: a question standing in
    /// tmux's first pane left the set, and rang again as a new one. A
    /// self-test leaves the set as it found it, whatever pane it names.
    #[test]
    fn a_push_self_test_leaves_the_waiting_set_alone() {
        let dir = TempDir::new("nse-selftest");
        let standing = PushStandingFile::new(dir.path(PushStandingFile::FILE_NAME));
        let asked = Date::now();
        let entry = Entry {
            pane: "%0".into(),
            agent: Some("claude".into()),
            title: None,
            location: None,
            since: asked.unix() as i64,
            recorded_at: asked,
        };
        assert!(standing.note_standing(CONN, entry, asked));
        for pane in ["%0", "selftest"] {
            let status = Status {
                conn: CONN.into(),
                host: "m1-pro".into(),
                sess: Some(String::new()),
                pane: pane.into(),
                agent: Some(notification::SELF_TEST_AGENT.into()),
                state: "done".into(),
                title: Some("selftest-ab12".into()),
                ts: asked.unix() as i64,
                dur: None,
            };
            let sealed = seal(status.to_json().as_bytes(), SECRET, None).unwrap();
            let text = format!(r#"{{"aps":{{"mutable-content":1}},"mp":{}}}"#, json::encode(&sealed));
            let payload = json::parse(text.as_bytes(), Dialect::Serialization { fragments_allowed: false }).unwrap();
            let received = deliver(&payload, &[pairing(SECRET, "m1-pro")], &dir);
            assert!(matches!(received.delivery, Delivery::Card(_)), "{:?}", received.delivery);
            let panes: Vec<String> = standing.standing(CONN, Date::now()).into_iter().map(|e| e.pane).collect();
            assert_eq!(panes, ["%0"], "the test's {pane} cleared a waiting pane");
        }
    }

    /// B435: a host holds one pairing's key and writes the connection id
    /// into what it seals. Sealed with this pairing's key but naming
    /// another connection, the push read as that connection's: its name,
    /// its waiting set, a tap into its pane. It keeps the fallback, as if
    /// unopened, and records nothing; the connection id is still matched as
    /// a UUID, whatever its case.
    #[test]
    fn a_push_sealed_with_one_connections_key_naming_another_keeps_the_fallback() {
        let dir = TempDir::new("nse-misnamed");
        let other = "9B2C1D3E-0000-4000-8000-00000000000A";
        let received = deliver(
            &payload_naming(other, "attention", Some("approve the deploy"), SECRET),
            &[pairing(SECRET, "m1-pro")],
            &dir,
        );
        assert_eq!(received.delivery, Delivery::Fallback { secrets_tried: 1 });
        assert!(
            received.diagnostics.iter().any(|line| line.contains("named another")),
            "{:?}",
            received.diagnostics
        );
        let standing = PushStandingFile::new(dir.path(PushStandingFile::FILE_NAME));
        assert!(standing.standing(other, Date::now()).is_empty());

        let lower = deliver(
            &payload_naming(&CONN.to_lowercase(), "attention", None, SECRET),
            &[pairing(SECRET, "m1-pro")],
            &dir,
        );
        assert!(matches!(lower.delivery, Delivery::Card(_)), "{:?}", lower.delivery);
    }

    #[test]
    fn the_service_opens_the_envelope_and_rewrites_the_notification() {
        let dir = TempDir::new("nse");
        let card = card(deliver(
            &payload("attention", Some("Bash: rm -rf build"), SECRET),
            &[pairing(SECRET, "m1-pro")],
            &dir,
        ));
        assert_eq!(card.title, "claude needs you");
        assert_eq!(card.body, "Bash: rm -rf build — m1-pro · work");
        assert_eq!(card.connection_id, CONN);
        assert_eq!(card.pane_id, "%3");
    }

    #[test]
    fn a_done_push_comes_through_as_finished() {
        let dir = TempDir::new("nse");
        let card = card(deliver(
            &payload("done", None, SECRET),
            &[pairing(SECRET, "m1-pro")],
            &dir,
        ));
        assert_eq!(card.title, "✓ claude finished");
    }

    #[test]
    fn the_service_names_the_connection_what_the_phone_calls_it() {
        let dir = TempDir::new("nse");
        let card = card(deliver(
            &payload("attention", Some("Bash: rm -rf build"), SECRET),
            &[pairing(SECRET, "mac-mini")],
            &dir,
        ));
        assert_eq!(card.body, "Bash: rm -rf build — mac-mini · work");
    }

    #[test]
    fn a_push_this_device_has_no_key_for_keeps_the_fallback() {
        let dir = TempDir::new("nse");
        let received = deliver(
            &payload("attention", Some("t"), &"ab".repeat(32)),
            &[pairing(SECRET, "m1-pro")],
            &dir,
        );
        assert_eq!(received.delivery, Delivery::Fallback { secrets_tried: 1 });
    }

    #[test]
    fn with_nothing_paired_the_fallback_arrives_and_the_count_names_the_cause() {
        let dir = TempDir::new("nse");
        let received = deliver(&payload("attention", Some("t"), SECRET), &[], &dir);
        assert_eq!(received.delivery, Delivery::Fallback { secrets_tried: 0 });
        assert!(
            received
                .diagnostics
                .iter()
                .any(|line| line.contains("(0 available)"))
        );
    }

    #[test]
    fn a_notification_with_no_envelope_passes_through_untouched() {
        let dir = TempDir::new("nse");
        let other =
            json::parse(br#"{"aps":{"alert":"Something else"}}"#, Dialect::Decoder).unwrap();
        let received = deliver(&other, &[pairing(SECRET, "m1-pro")], &dir);
        assert_eq!(received.delivery, Delivery::AsReceived);
        assert_eq!(
            received.diagnostics,
            [
                "woke for a push",
                "no sealed envelope — delivered as received"
            ]
        );
    }

    /// A long turn's finish reaches the phone twice when the app is awake:
    /// the push, and the app's own card. Whichever is first rings.
    #[test]
    fn a_long_finish_the_app_already_rang_arrives_silent() {
        let dir = TempDir::new("nse-done");
        let standing = PushStandingFile::new(dir.path(PushStandingFile::FILE_NAME));
        let finishes = PushFinishesFile::new(dir.path("rung-finishes.json"));
        let long = |ts: i64| {
            let mut status: Status = serde_json::from_str(
                &format!(r#"{{"conn":"{CONN}","host":"m1-pro","pane":"%3","agent":"claude","state":"done","ts":{ts},"dur":600}}"#),
            )
            .unwrap();
            status.title = None;
            let sealed = seal(status.to_json().as_bytes(), SECRET, None).unwrap();
            json::parse(
                format!(r#"{{"mp":{}}}"#, json::encode(&sealed)).as_bytes(),
                Dialect::Serialization { fragments_allowed: false },
            )
            .unwrap()
        };
        let card = |received: Received| match received.delivery {
            Delivery::Card(card) => card,
            other => panic!("not a card: {other:?}"),
        };
        let now = Date::now();
        let pairings = [pairing(SECRET, "m1-pro")];
        let first = card(receive(&long(1_790_000_600), &pairings, &standing, &finishes,
                                 Prefs::default(), Language::En, now));
        assert!(first.sound, "nobody rang this turn yet");
        // The app rang the next turn's finish first.
        assert!(finishes.note_ring(CONN, "%3", 1_790_001_500, now));
        let second = card(receive(&long(1_790_001_501), &pairings, &standing, &finishes,
                                  Prefs::default(), Language::En, now));
        assert!(!second.sound, "one turn's finish rang twice");
        assert_eq!(second.level, crate::copy::InterruptionLevel::Passive);
    }

    #[test]
    fn the_first_prompt_rings_the_next_one_on_the_same_connection_does_not() {
        let dir = TempDir::new("nse");
        let pairings = [pairing(SECRET, "m1-pro")];
        let first = card(deliver(
            &payload("attention", Some("may I"), SECRET),
            &pairings,
            &dir,
        ));
        assert!(first.sound);
        assert_eq!(first.level, InterruptionLevel::TimeSensitive);
        // The same pane asking again (a new episode) while still standing.
        let second = card(deliver(
            &payload("attention", Some("and this?"), SECRET),
            &pairings,
            &dir,
        ));
        assert!(!second.sound);
        assert_eq!(second.level, InterruptionLevel::Passive);
        // A done closes the pane's wait; the next question rings again.
        card(deliver(&payload("done", None, SECRET), &pairings, &dir));
        let again = card(deliver(
            &payload("attention", Some("one more"), SECRET),
            &pairings,
            &dir,
        ));
        assert!(again.sound);
    }

    #[test]
    fn the_sound_switch_reaches_every_path() {
        let dir = TempDir::new("nse");
        let standing = PushStandingFile::new(dir.path(PushStandingFile::FILE_NAME));
        let quiet = Prefs {
            show_detail: true,
            sound: false,
        };
        let opened = receive(
            &payload("attention", Some("t"), SECRET),
            &[pairing(SECRET, "m")],
            &standing,
            &PushFinishesFile::new(dir.path("rung-finishes.json")),
            quiet,
            Language::En,
            Date::now(),
        );
        assert!(!opened.sound_allowed);
        assert!(!card(opened).sound);
        let fallback = receive(
            &payload("attention", Some("t"), SECRET),
            &[],
            &standing,
            &PushFinishesFile::new(dir.path("rung-finishes.json")),
            quiet,
            Language::En,
            Date::now(),
        );
        assert!(!fallback.sound_allowed);
    }

    #[test]
    fn a_self_test_is_named_in_the_diagnostics() {
        let dir = TempDir::new("nse");
        let status = Status {
            conn: CONN.into(),
            host: "m1-pro".into(),
            sess: None,
            pane: "%0".into(),
            agent: Some(notification::SELF_TEST_AGENT.into()),
            state: "done".into(),
            title: Some("selftest-abc".into()),
            ts: 1,
            dur: None,
        };
        let sealed = seal(status.to_json().as_bytes(), SECRET, None).unwrap();
        let payload = json::parse(
            format!(r#"{{"mp":{}}}"#, json::encode(&sealed)).as_bytes(),
            Dialect::Decoder,
        )
        .unwrap();
        let received = deliver(&payload, &[pairing(SECRET, "m")], &dir);
        assert_eq!(
            received.diagnostics.last().unwrap(),
            "opened a done push from m1-pro (self-test selftest-abc)"
        );
        assert_eq!(card(received).self_test_nonce(), Some("selftest-abc"));
    }
}
