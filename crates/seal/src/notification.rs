//! Turning an opened push into the card the user sees
//! (`PushRemoteNotification`), and the routing keys that let a tap land on
//! the pane exactly as a local notification's does.

use crate::copy::{self, InterruptionLevel, Language};
use crate::envelope::{self, Envelope};
use crate::foundation::text;
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

/// How long a finished turn must have RUN for its completion to make a sound.
pub const DONE_SOUND_THRESHOLD: i64 = 180;

/// Try every secret this device holds until one authenticates (newest
/// first is the caller's order). A MAC failure is the normal "not this
/// host"; only running out of keys is a failure, and it is `None`.
pub fn open(envelope: &Envelope, secrets: &[String]) -> Option<Status> {
    secrets
        .iter()
        .find_map(|secret| envelope::open(envelope, secret).ok())
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

/// Render a status into a card (`PushRemoteNotification.apply`).
///
/// With "Show detail" off the body names where — never what the agent is
/// running, asking, or was asked; `detail` keeps the real text either way.
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
        assert_eq!(card.body, "Bash: rm -rf build — m1-pro · work");
        assert_eq!(card.connection_id, "3F2504E0-4F89-11D3-9A0C-0305E82C3301");
        assert_eq!(card.pane_id, "%3");
        assert_eq!(card.state, "attention");
        assert_eq!(card.agent, "claude");
    }

    #[test]
    fn a_done_push_reads_as_finished() {
        let card = render(
            &status("done", None, Some("claude"), Some("work"), None),
            Render::default(),
        );
        assert_eq!(card.title, "✓ claude finished");
        assert_eq!(card.body, "m1-pro · work");
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
        assert_eq!(
            card.body,
            "Upload the build with the new Xcode — m1-pro · work"
        );
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
        assert_eq!(done.body, "m1-pro · work");
        let asked = render(&attention(), off);
        assert_eq!(asked.title, "claude needs you");
        assert_eq!(asked.body, "m1-pro · work");
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
        assert_eq!(card.body, "Bash: rm -rf build — mac-mini · work");
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
        assert_eq!(card.body, "m1-pro · work");
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
        assert_eq!(card.body, "Bash: rm -rf build — m1-pro");
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
