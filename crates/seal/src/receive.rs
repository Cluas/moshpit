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

use crate::closed::PushClosedFile;
use crate::copy::{self, Language};
use crate::finishes::PushFinishesFile;
use crate::envelope::Envelope;
use crate::foundation::date::Date;
use crate::foundation::json::Value;
use crate::foundation::uuid::Uuid;
use crate::notification::{self, Card, Opening, Render};
use crate::pairing::PushPairing;
use crate::prefs::Prefs;
use crate::standing::{Entry, LIFETIME_SECONDS, PushStandingFile};

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

/// What becomes of the connection's waiting card after a finish took its
/// pane out of the waiting set (B669): posted again in place, silently,
/// under [`notification::ATTENTION_CARD`]`<conn>`, or withdrawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waiting {
    Redraw { id: String, card: Card },
    Withdraw { id: String },
}

/// The outcome of one push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    pub delivery: Delivery,
    /// The waiting card's update a finish calls for (B669); none otherwise.
    pub waiting: Option<Waiting>,
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
            waiting: None,
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
                waiting: None,
                sound_allowed,
                diagnostics,
            };
        }
        // The paired host's key opened it, but it holds no status this
        // device reads (B635): the fallback, saying which it was.
        Opening::Unreadable => {
            diagnostics.push(
                "a key opened this envelope, but it holds no status this phone reads — showed the fallback".to_owned(),
            );
            return Received {
                delivery: Delivery::Fallback {
                    secrets_tried: pairings.len(),
                },
                waiting: None,
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
                waiting: None,
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
    // B636: when the host last said this pane's turn ended. Its lock busy
    // past its budget reads as nothing ended — today's behaviour — never a
    // push held up.
    let closed = PushClosedFile::beside(standing.path());
    let ended = if status.state == "attention" {
        closed
            .closed_at(&status.conn, &status.pane, now)
            .unwrap_or_else(|_| {
                diagnostics.push("turn endings busy — read as nothing ended".to_owned());
                None
            })
    } else {
        None
    };
    let mut waiting = None;
    let card = if status.agent.as_deref() == Some(notification::SELF_TEST_AGENT) {
        // A pairing self-test is no agent's turn (B439): it neither joins
        // nor leaves the waiting set, and rings no finish. Its "done" used
        // to clear the pane it named — `%0`, where an agent often waits.
        notification::render(&status, how)
    } else if status.state == "attention" && ended.is_some_and(|ended| ended > status.ts) {
        // B636: a question the host already said ended — APNs reordered
        // it behind its `done`, or a relay sent it again. Not a new one: it
        // doesn't stand and rings nothing.
        diagnostics.push(
            "an attention the host already ended came late — shown without a sound, not standing"
                .to_owned(),
        );
        let count = standing.standing(&status.conn, now).len();
        notification::render(
            &status,
            Render {
                attention_edge: false,
                standing_count: i64::try_from(count).unwrap_or(i64::MAX),
                ..how
            },
        )
    } else if status.state == "attention" {
        let edge = standing.note_standing(
            &status.conn,
            // With its place, as the card says it: the app redraws this
            // card from the entry when the set changes, and has only this.
            Entry {
                pane: status.pane.clone(),
                agent: status.agent.clone(),
                title: status.title.clone(),
                location: Some(notification::location(&status, label.as_deref())),
                since: status.ts,
                recorded_at: now,
                lifetime: Some(LIFETIME_SECONDS),
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
        // B636: the host's word that the pane's turn ended, by its clock:
        // what it asked up to then stops waiting, and a question from
        // before it that comes later is told for what it is.
        if closed
            .note_closed(&status.conn, &status.pane, status.ts, now)
            .is_err()
        {
            diagnostics.push("turn endings busy — this ending isn't remembered".to_owned());
        }
        let was_waiting = standing
            .standing(&status.conn, now)
            .iter()
            .any(|entry| entry.pane == status.pane && entry.since <= status.ts);
        let remaining = standing.clear_ended(&status.conn, &status.pane, status.ts, now);
        // Its waiting card says what still waits, or goes (B669).
        if was_waiting {
            let id = format!("{}{}", notification::ATTENTION_CARD, status.conn);
            let host = label.as_deref().unwrap_or(&status.host);
            waiting = Some(match notification::summary(&remaining, &status.conn, host, prefs, language) {
                Some(card) => {
                    diagnostics.push(format!(
                        "a finish left {} waiting — redrew the waiting card",
                        remaining.len()
                    ));
                    Waiting::Redraw { id, card }
                }
                None => {
                    diagnostics.push("a finish left nothing waiting — withdrew the waiting card".to_owned());
                    Waiting::Withdraw { id }
                }
            });
        }
        // A long turn's finish rings once between this card and the app's
        // local copy of it: whichever lands first.
        let done_rings = status.state != "done"
            || !copy::done_wakes(status.dur.unwrap_or(0), notification::DONE_SOUND_THRESHOLD)
            || finishes.note_ring(&status.conn, &status.pane, status.ts, now);
        notification::render(&status, Render { done_rings, ..how })
    };
    Received {
        delivery: Delivery::Card(card),
        waiting,
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

    /// A payload sealed over `plaintext` exactly as given.
    fn payload_of(plaintext: &[u8], key: &str) -> Value {
        let sealed = seal(plaintext, key, None).unwrap();
        let text = format!(
            r#"{{"aps":{{"mutable-content":1}},"mp":{}}}"#,
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

    /// [`payload`] for a status sent at `ts` (the host's clock).
    fn payload_at(state: &str, ts: i64) -> Value {
        payload_in("%3", state, ts)
    }

    /// [`payload_at`] for `pane`.
    fn payload_in(pane: &str, state: &str, ts: i64) -> Value {
        let status = Status {
            conn: CONN.into(),
            host: "m1-pro".into(),
            sess: Some("work".into()),
            pane: pane.into(),
            agent: Some("claude".into()),
            state: state.into(),
            title: Some("Bash: ls".into()),
            ts,
            dur: None,
        };
        let sealed = seal(status.to_json().as_bytes(), SECRET, None).unwrap();
        let text = format!(
            r#"{{"aps":{{"mutable-content":1}},"mp":{}}}"#,
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

    /// B635: a title a host cut in the middle of a character (GNU `cut -c`
    /// counts bytes, and a host without `iconv` keeps the half) is shown,
    /// the half as U+FFFD — not refused whole as a push no key opened.
    #[test]
    fn a_title_cut_mid_character_still_shows() {
        let dir = TempDir::new("nse-cut-title");
        let title = "构建".as_bytes();
        let mut plaintext = format!(
            r#"{{"conn":"{CONN}","host":"m1-pro","pane":"%3","agent":"claude","state":"done","ts":1755900000,"title":"Bash: "#
        )
        .into_bytes();
        plaintext.extend_from_slice(&title[..4]);
        plaintext.extend_from_slice(br#""}"#);
        let received = deliver(
            &payload_of(&plaintext, SECRET),
            &[pairing(SECRET, "Mac")],
            &dir,
        );
        let Delivery::Card(card) = &received.delivery else {
            panic!("not shown: {:?}", received.diagnostics);
        };
        assert!(format!("{card:?}").contains('构'), "{card:?}");
    }

    /// B635: a push the paired host's key opened but that holds no status
    /// is said as such — not as one sealed for another install, which sends
    /// the person to pair again for nothing.
    #[test]
    fn a_push_the_key_opened_but_holding_no_status_is_said_as_such() {
        let dir = TempDir::new("nse-unreadable");
        let received = deliver(
            &payload_of(b"not a status", SECRET),
            &[pairing(SECRET, "Mac")],
            &dir,
        );
        assert_eq!(received.delivery, Delivery::Fallback { secrets_tried: 1 });
        let said = received.diagnostics.join("\n");
        assert!(said.contains("holds no status"), "{said}");
        assert!(!said.contains("no key opened"), "{said}");
    }

    fn standing_panes(dir: &TempDir) -> Vec<String> {
        PushStandingFile::new(dir.path(PushStandingFile::FILE_NAME))
            .standing(CONN, Date::now())
            .into_iter()
            .map(|e| e.pane)
            .collect()
    }

    /// B636: a question that arrives after the `done` that ended it (APNs
    /// reordered them, or a relay sent the envelope again) is not a new
    /// one: it neither stands nor rings. It rang "needs you" and stood six
    /// hours on a pane nobody waited on.
    #[test]
    fn an_attention_that_comes_after_its_done_neither_stands_nor_rings() {
        let dir = TempDir::new("nse-reordered");
        let pairings = [pairing(SECRET, "Mac")];
        deliver(&payload_at("done", 1_790_000_600), &pairings, &dir);
        let late = card(deliver(
            &payload_at("attention", 1_790_000_500),
            &pairings,
            &dir,
        ));
        assert!(!late.sound, "the late question rang");
        assert!(standing_panes(&dir).is_empty(), "the late question stands");

        // A question asked after that ending is a new one, as before.
        let next = card(deliver(
            &payload_at("attention", 1_790_000_700),
            &pairings,
            &dir,
        ));
        assert!(next.sound);
        assert_eq!(standing_panes(&dir), vec!["%3".to_owned()]);
    }

    /// B636: a `done` that comes again leaves the question its pane asked
    /// after it standing. It cleared it, and the waiting card went.
    #[test]
    fn a_done_that_comes_again_leaves_a_later_question_standing() {
        let dir = TempDir::new("nse-replayed-done");
        let pairings = [pairing(SECRET, "Mac")];
        deliver(&payload_at("attention", 1_790_000_500), &pairings, &dir);
        deliver(&payload_at("done", 1_790_000_600), &pairings, &dir);
        assert!(standing_panes(&dir).is_empty(), "its own done ends it");
        deliver(&payload_at("attention", 1_790_000_700), &pairings, &dir);
        let replayed = payload_at("done", 1_790_000_600);
        deliver(&replayed, &pairings, &dir);
        assert_eq!(
            standing_panes(&dir),
            vec!["%3".to_owned()],
            "an old done cleared the question asked after it"
        );
    }

    /// E7b: the notification service's Swift twin recorded a question's
    /// place with its standing entry (the app redraws the waiting card from
    /// it); this one recorded none, and a card redrawn after another pane
    /// left the wait said the host alone.
    #[test]
    fn a_standing_question_keeps_its_place() {
        let dir = TempDir::new("nse-place");
        deliver(&payload_at("attention", 1_790_000_500), &[pairing(SECRET, "Mac")], &dir);
        let entries = PushStandingFile::new(dir.path(PushStandingFile::FILE_NAME)).standing(CONN, Date::now());
        assert_eq!(entries[0].location.as_deref(), Some("Mac · work"), "{entries:?}");
    }

    /// B669 (the Swift service's, now this one's): a pushed `done` took
    /// its pane out of the waiting set but left the connection's waiting
    /// card as it was. The answer says to redraw it from what still waits
    /// — the newest question, the count, silent — or to withdraw it.
    #[test]
    fn a_finish_redraws_the_waiting_card_or_withdraws_it() {
        let dir = TempDir::new("nse-waiting");
        let pairings = [pairing(SECRET, "Mac")];
        deliver(&payload_in("%3", "attention", 1_790_000_500), &pairings, &dir);
        deliver(&payload_in("%5", "attention", 1_790_000_510), &pairings, &dir);
        let id = format!("moshpit.attention.{CONN}");
        let first = deliver(&payload_in("%5", "done", 1_790_000_600), &pairings, &dir);
        let Some(Waiting::Redraw { id: redraw_id, card }) = first.waiting else {
            panic!("no redraw: {:?}", first.waiting)
        };
        assert_eq!(redraw_id, id);
        assert_eq!(card.pane_id, "%3", "the one still waiting");
        assert_eq!(card.title, "claude needs you");
        assert!(!card.sound && card.level == InterruptionLevel::Passive, "a redraw rings nothing");
        assert!(first.diagnostics.iter().any(|l| l == "a finish left 1 waiting — redrew the waiting card"));
        let last = deliver(&payload_in("%3", "done", 1_790_000_700), &pairings, &dir);
        assert_eq!(last.waiting, Some(Waiting::Withdraw { id }));
        // A finish for a pane that wasn't waiting leaves the card alone.
        let other = deliver(&payload_in("%9", "done", 1_790_000_800), &pairings, &dir);
        assert_eq!(other.waiting, None);
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
            lifetime: Some(LIFETIME_SECONDS),
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

        // B442: with the sound off the record wasn't consulted, and both
        // copies of a long finish woke the screen. One wakes it.
        let quiet = Prefs { sound: false, ..Prefs::default() };
        let first = card(receive(&long(1_790_003_000), &pairings, &standing, &finishes,
                                 quiet, Language::En, now));
        assert_eq!((first.sound, first.level), (false, crate::copy::InterruptionLevel::Active));
        assert!(finishes.note_ring(CONN, "%3", 1_790_004_000, now));
        let second = card(receive(&long(1_790_004_001), &pairings, &standing, &finishes,
                                  quiet, Language::En, now));
        assert_eq!(second.level, crate::copy::InterruptionLevel::Passive, "one turn woke the screen twice");
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
