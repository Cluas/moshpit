//! Turning an opened push into the card the user sees
//! (`PushRemoteNotification`), and the routing keys that let a tap land on
//! the pane exactly as a local notification's does.

use crate::copy::{self, InterruptionLevel, Language};
use crate::envelope::{self, Envelope};
use crate::foundation::text;
use crate::foundation::uuid::Uuid;
use crate::pairing::PushPairing;
use crate::pairing_file::newest_first;
use crate::prefs::Prefs;
use crate::status::Status;

/// Payload key holding the sealed envelope (the relay's `mp`).
pub const ENVELOPE_KEY: &str = "mp";

/// Agent label a pairing self-test carries; the app suppresses such a push
/// and the island ignores the label.
pub const SELF_TEST_AGENT: &str = "moshpit-selftest";

/// `userInfo` keys of every agent notification, local or pushed
/// (`AgentNotifications`): which pane a tap opens.
pub const CONNECTION_KEY: &str = "connectionId";
pub const PANE_KEY: &str = "paneId";

/// `userInfo` keys only an opened push carries: what the app needs to know
/// about it and cannot re-open the envelope to learn.
pub const AGENT_KEY: &str = "moshpitAgent";
pub const STATE_KEY: &str = "moshpitState";
pub const DETAIL_KEY: &str = "moshpitDetail";

/// A connection's waiting card: `<this><conn>`, the identifier the app
/// posts it under and the pushed copy's collapse id.
pub const ATTENTION_CARD: &str = "moshpit.attention.";

/// How long a finished turn must have RUN for its completion to make a sound.
pub const DONE_SOUND_THRESHOLD: i64 = 180;

/// Try every secret this device holds until one authenticates (newest
/// first is the caller's order). A MAC failure is the normal "not this
/// host"; only running out of keys is a failure, and it is `None`.
///
/// Harness-only (the `trysecrets` cases, Rust against Swift): which
/// connection sent it is not checked here. What is shown goes through
/// [`open_paired`] (B435); `tests/harness_only.rs` fails if Android or the
/// rest of this crate calls this one.
pub fn open(envelope: &Envelope, secrets: &[String]) -> Option<Status> {
    secrets
        .iter()
        .find_map(|secret| envelope::open(envelope, secret).ok())
}

/// What became of an envelope tried against this device's pairings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opening {
    /// A pairing's key opened it, and it names that pairing's connection.
    Opened(Status),
    /// A pairing's key opened it, but it names another connection (B435).
    Misnamed,
    /// A pairing's key opened it, but what it holds is no status this
    /// device reads (B635): the right key, so the host is the paired one —
    /// not, as an unopened push says, a host paired to another install.
    Unreadable,
    /// No key opened it.
    Unopened,
}

/// Open `envelope` with `pairings`' keys, newest pairing first, and believe
/// it only for the connection whose key opened it (B435).
///
/// A host holds one pairing's key and writes the connection id into what it
/// seals. That id was taken as given: a host paired with one connection
/// could seal a status naming another, and its card read as the other
/// connection's — its name, its waiting set and its rings, and a tap opened
/// its pane. A pairing promises a compromised host can't forge
/// notifications that appear to come from a different one; this is where
/// that holds. Only the key's holder can have sealed it, so no other key
/// needs trying.
pub fn open_paired(envelope: &Envelope, pairings: &[PushPairing]) -> Opening {
    for pairing in newest_first(pairings.to_vec()) {
        let Ok(plaintext) = envelope::open_raw(envelope, &pairing.secret_hex) else {
            continue;
        };
        // The key opened it: the MAC verified, only its holder sealed it.
        let Some(status) = Status::from_json(&plaintext) else {
            return Opening::Unreadable;
        };
        return if Uuid::parse(&status.conn) == Some(pairing.connection_id) {
            Opening::Opened(status)
        } else {
            Opening::Misnamed
        };
    }
    Opening::Unopened
}

/// Where the agent is, as the local cards say it: the name the user gave the
/// connection when known (`label`), else the host's own name, and the
/// session.
pub fn location(status: &Status, label: Option<&str>) -> String {
    let label = text::trim_inline_spaces(label.unwrap_or(""));
    copy::place(
        if label.is_empty() {
            &status.host
        } else {
            label
        },
        status.sess.as_deref(),
    )
}

/// A rendered notification: the words, how it may interrupt, and the
/// `userInfo` it carries. No category, ever: Moshpit's notifications carry
/// no action buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub title: String,
    /// Where: the connection's name and the session (B639).
    pub subtitle: String,
    pub body: String,
    pub sound: bool,
    pub level: InterruptionLevel,
    /// `connectionId`: the connection the envelope's `conn` names.
    pub connection_id: String,
    /// `paneId`.
    pub pane_id: String,
    /// `moshpitAgent`: the agent, or empty.
    pub agent: String,
    /// `moshpitState`.
    pub state: String,
    /// `moshpitDetail`: the hook's title, trimmed, shown or not — the app
    /// matches self-test nonces and acknowledges prompts through it.
    pub detail: String,
}

impl Card {
    /// A pairing self-test's nonce, when this card is one
    /// (`AgentNotificationHandler.selfTestNonce`).
    pub fn self_test_nonce(&self) -> Option<&str> {
        (self.agent == SELF_TEST_AGENT).then_some(self.detail.as_str())
    }
}

/// How to render one status.
#[derive(Debug, Clone, Copy)]
pub struct Render<'a> {
    /// Is this the moment "nobody is waiting" became "someone is waiting"?
    /// Only that edge rings and pierces Focus.
    pub attention_edge: bool,
    /// Prompts waiting, this one included.
    pub standing_count: i64,
    pub prefs: Prefs,
    /// What the phone calls this connection.
    pub label: Option<&'a str>,
    pub language: Language,
    /// false: this turn's finish already rang (the finishes record), so a
    /// `done` only updates the card.
    pub done_rings: bool,
}

impl Default for Render<'_> {
    fn default() -> Self {
        Self {
            attention_edge: true,
            standing_count: 1,
            prefs: Prefs::default(),
            label: None,
            language: Language::En,
            done_rings: true,
        }
    }
}

/// The connection's waiting card once a pushed `done` took a pane out of
/// the waiting set (B669, `PushRemoteNotification.summary`): the newest
/// question still standing, with the count, silent — or none when nothing
/// stands, and the card goes. `host` names the agent and the place when an
/// entry has none, as the app's own digest does.
pub fn summary(
    remaining: &[crate::standing::Entry],
    conn: &str,
    host: &str,
    prefs: Prefs,
    language: Language,
) -> Option<Card> {
    // The first of the newest, as Swift's `max(by:)` picks.
    let newest = remaining.iter().rev().max_by_key(|entry| entry.since)?;
    let who = newest.agent.as_deref().filter(|agent| !agent.is_empty()).unwrap_or(host);
    let stored = newest.location.as_deref().map(text::trim_white_space).unwrap_or("");
    let detail = newest.title.as_deref().map(text::trim_white_space);
    let words = copy::attention(
        who,
        i64::try_from(remaining.len()).unwrap_or(i64::MAX),
        if prefs.show_detail { detail } else { None },
        if stored.is_empty() { host } else { stored },
        false,
        false,
        language,
    );
    Some(Card {
        title: words.title,
        subtitle: words.subtitle,
        body: words.body,
        sound: words.sound,
        level: words.level,
        connection_id: conn.to_owned(),
        pane_id: newest.pane.clone(),
        agent: newest.agent.clone().unwrap_or_default(),
        state: "attention".to_owned(),
        detail: detail.unwrap_or("").to_owned(),
    })
}

/// Render a status into a card (`PushRemoteNotification.apply`).
///
/// The subtitle names where (B639); with "Show detail" off the body is
/// empty — never what the agent is running, asking, or was asked; `detail`
/// keeps the real text either way.
/// A `done` says what finished (the host sends the prompt the turn
/// answered) and how long it ran; every other state renders as a question.
pub fn render(status: &Status, how: Render<'_>) -> Card {
    let who = match status.agent.as_deref() {
        Some(agent) if !agent.is_empty() => agent,
        _ => status.host.as_str(),
    };
    let place = location(status, how.label);
    let raw_detail = status.title.as_deref().map(text::trim_white_space);
    let shown = if how.prefs.show_detail {
        raw_detail
    } else {
        None
    };
    let words = if status.state == "done" {
        // `dur` is absent from older senders; absent reads as short, because
        // "quieter than intended" is the recoverable direction.
        copy::done(
            who,
            shown,
            &place,
            status.dur.unwrap_or(0),
            how.prefs.sound,
            DONE_SOUND_THRESHOLD,
            how.done_rings,
            how.language,
        )
    } else {
        copy::attention(
            who,
            how.standing_count,
            shown,
            &place,
            how.attention_edge,
            how.prefs.sound,
            how.language,
        )
    };
    Card {
        title: words.title,
        subtitle: words.subtitle,
        body: words.body,
        sound: words.sound,
        level: words.level,
        connection_id: status.conn.clone(),
        pane_id: status.pane.clone(),
        agent: status.agent.clone().unwrap_or_default(),
        state: status.state.clone(),
        detail: raw_detail.unwrap_or("").to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::seal;

    const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn status(
        state: &str,
        title: Option<&str>,
        agent: Option<&str>,
        sess: Option<&str>,
        dur: Option<i64>,
    ) -> Status {
        Status {
            conn: "3F2504E0-4F89-11D3-9A0C-0305E82C3301".into(),
            host: "m1-pro".into(),
            sess: sess.map(str::to_owned),
            pane: "%3".into(),
            agent: agent.map(str::to_owned),
            state: state.into(),
            title: title.map(str::to_owned),
            ts: crate::foundation::date::Date::now().unix() as i64,
            dur,
        }
    }

    fn attention() -> Status {
        status(
            "attention",
            Some("Bash: rm -rf build"),
            Some("claude"),
            Some("work"),
            None,
        )
    }

    // MARK: - PushRemoteNotificationTests

    #[test]
    fn each_stored_secret_is_tried_until_one_authenticates() {
        let other = "ab".repeat(32);
        let sealed = seal(attention().to_json().as_bytes(), &other, None).unwrap();
        assert_eq!(open(&sealed, &[SECRET.to_owned()]), None);
        assert_eq!(
            open(&sealed, &[SECRET.to_owned(), other.clone()])
                .unwrap()
                .host,
            "m1-pro"
        );
        assert_eq!(open(&sealed, &[]), None);
    }

    #[test]
    fn an_attention_push_reads_like_its_local_twin_and_routes_like_one() {
        let card = render(&attention(), Render::default());
        assert_eq!(card.title, "claude needs you");
        assert_eq!(card.body, "Bash: rm -rf build");
        assert_eq!(card.subtitle, "m1-pro · work");
        assert_eq!(card.connection_id, "3F2504E0-4F89-11D3-9A0C-0305E82C3301");
        assert_eq!(card.pane_id, "%3");
        assert_eq!(card.state, "attention");
        assert_eq!(card.agent, "claude");
    }

    /// B639: a pushed card shows no bidi control from the host's title,
    /// agent name, host name or session, whatever the sender left in.
    #[test]
    fn a_pushed_card_shows_no_bidi_control() {
        let mut spoof = status(
            "attention",
            Some("Allow?\u{202E}moc.elpmaxe.tsurt — "),
            Some("claude\u{202E}"),
            Some("work\u{2067}"),
            None,
        );
        spoof.host = "m1-pro\u{200F}".into();
        let card = render(&spoof, Render::default());
        assert_eq!(card.title, "claude needs you");
        assert_eq!(card.body, "Allow?moc.elpmaxe.tsurt —");
        assert_eq!(card.subtitle, "m1-pro · work");
        assert_eq!(
            card.detail, "Allow?\u{202E}moc.elpmaxe.tsurt —",
            "userInfo keeps what was sent"
        );
    }

    #[test]
    fn a_done_push_reads_as_finished() {
        let card = render(
            &status("done", None, Some("claude"), Some("work"), None),
            Render::default(),
        );
        assert_eq!(card.title, "✓ claude finished");
        assert_eq!(card.body, "");
        assert_eq!(card.subtitle, "m1-pro · work");
    }

    #[test]
    fn a_done_push_says_what_finished_and_how_long_it_ran() {
        let card = render(
            &status(
                "done",
                Some("Upload the build with the new Xcode"),
                Some("claude"),
                Some("work"),
                Some(12 * 60),
            ),
            Render::default(),
        );
        assert!(card.title.starts_with("✓ claude finished · "));
        assert!(card.title.contains("12"));
        assert_eq!(card.body, "Upload the build with the new Xcode");
        assert_eq!(card.subtitle, "m1-pro · work");
    }

    #[test]
    fn show_detail_off_keeps_the_prompt_and_the_question_out() {
        let off = Render {
            prefs: Prefs {
                show_detail: false,
                sound: true,
            },
            ..Render::default()
        };
        let done = render(
            &status(
                "done",
                Some("Upload the build"),
                Some("claude"),
                Some("work"),
                None,
            ),
            off,
        );
        assert_eq!(done.title, "✓ claude finished");
        assert_eq!(done.body, "");
        assert_eq!(done.subtitle, "m1-pro · work");
        let asked = render(&attention(), off);
        assert_eq!(asked.title, "claude needs you");
        assert_eq!(asked.body, "");
        assert_eq!(asked.subtitle, "m1-pro · work");
        // The unrendered copy stays: the app matches nonces through it.
        assert_eq!(asked.detail, "Bash: rm -rf build");
    }

    #[test]
    fn the_card_names_the_connection_the_way_the_phone_does() {
        let card = render(
            &attention(),
            Render {
                label: Some("mac-mini"),
                ..Render::default()
            },
        );
        assert_eq!(card.body, "Bash: rm -rf build");
        assert_eq!(card.subtitle, "mac-mini · work");
        assert_eq!(location(&attention(), Some("  ")), "m1-pro · work");
    }

    #[test]
    fn alert_sound_off_silences_even_the_edge_that_earned_a_sound() {
        let quiet = Render {
            prefs: Prefs {
                show_detail: true,
                sound: false,
            },
            ..Render::default()
        };
        let card = render(&attention(), quiet);
        assert!(!card.sound);
        assert_eq!(card.level, InterruptionLevel::TimeSensitive);
        assert!(
            !render(
                &status("done", None, Some("claude"), None, Some(600)),
                quiet
            )
            .sound
        );
    }

    #[test]
    fn a_missing_hook_title_shows_the_location() {
        let card = render(
            &status("attention", Some("   "), Some("claude"), Some("work"), None),
            Render::default(),
        );
        assert_eq!(card.title, "claude needs you");
        assert_eq!(card.body, "");
        assert_eq!(card.subtitle, "m1-pro · work");
    }

    #[test]
    fn tmuxs_default_numeric_session_name_is_not_shown_as_a_location() {
        let at = |sess: &str| location(&status("attention", None, None, Some(sess), None), None);
        assert_eq!(at("0"), "m1-pro");
        assert_eq!(at("12"), "m1-pro");
        assert_eq!(at("  "), "m1-pro");
        assert_eq!(at("work"), "m1-pro · work");
        assert_eq!(at("api-2"), "m1-pro · api-2");
    }

    #[test]
    fn a_session_less_host_renders_as_just_the_host() {
        let card = render(
            &status(
                "attention",
                Some("Bash: rm -rf build"),
                Some("claude"),
                None,
                None,
            ),
            Render::default(),
        );
        assert_eq!(card.body, "Bash: rm -rf build");
        assert_eq!(card.subtitle, "m1-pro");
        assert_eq!(
            location(&status("attention", None, None, Some(""), None), None),
            "m1-pro"
        );
    }

    #[test]
    fn an_agent_less_status_falls_back_to_the_host_as_the_title() {
        let card = render(
            &status("attention", Some("t"), None, None, None),
            Render::default(),
        );
        assert_eq!(card.title, "m1-pro needs you");
        assert_eq!(card.agent, "");
    }

    // MARK: - QuietRenderingTests

    #[test]
    fn only_the_edge_rings_and_pierces_focus() {
        let edge = render(
            &attention(),
            Render {
                attention_edge: true,
                standing_count: 1,
                ..Render::default()
            },
        );
        assert!(edge.sound);
        assert_eq!(edge.level, InterruptionLevel::TimeSensitive);
        let update = render(
            &attention(),
            Render {
                attention_edge: false,
                standing_count: 3,
                ..Render::default()
            },
        );
        assert!(!update.sound, "a count changing is not worth a sound");
        assert_eq!(update.level, InterruptionLevel::Passive);
        assert_eq!(update.title, "claude +2 needs you");
    }

    #[test]
    fn a_finished_short_turn_is_information_not_an_interruption() {
        let done = |dur| {
            render(
                &status(
                    "done",
                    Some("Bash: make"),
                    Some("claude"),
                    Some("work"),
                    dur,
                ),
                Render::default(),
            )
        };
        let short = done(Some(25));
        assert!(!short.sound);
        assert_eq!(short.level, InterruptionLevel::Passive);
        let long = done(Some(240));
        assert!(
            long.sound,
            "walking away from a 4-minute build deserves the chime"
        );
        assert_eq!(long.level, InterruptionLevel::Active);
        let unknown = done(None);
        assert!(!unknown.sound);
        assert_eq!(unknown.level, InterruptionLevel::Passive);
    }

    // MARK: - SelfTestRecognitionTests

    #[test]
    fn a_self_test_push_yields_its_nonce_and_nothing_else_does() {
        let test = status(
            "done",
            Some("selftest-abc123"),
            Some(SELF_TEST_AGENT),
            None,
            None,
        );
        assert_eq!(
            render(&test, Render::default()).self_test_nonce(),
            Some("selftest-abc123")
        );
        assert_eq!(
            render(&attention(), Render::default()).self_test_nonce(),
            None
        );
    }
}
