//! The words on an agent notification (`AgentNotificationCopy`): one
//! renderer for every copy of a card — the pushed one a notification service
//! writes and the local one the app posts — so the same event reads the same
//! however it reached the phone:
//!
//! ```text
//! ✓ claude finished · 12 min
//! Upload the build with the new Xcode — mac-mini · work
//!
//! claude needs you
//! Bash: rm -rf build — mac-mini · work
//! ```
//!
//! The two sentences and the duration come in the app's nine languages
//! ([`Language`]); the sentences are the notification service's own string
//! catalog (`Extensions/MoshpitPush/Localizable.xcstrings`), the durations
//! what Foundation's unit formatter writes on macOS 27.

use crate::foundation::text;

/// The longest detail (a prompt, a question, a tool line) a card shows, in
/// characters. The host already cuts a prompt to 80 bytes before it travels.
pub const DETAIL_LIMIT: usize = 72;

/// A finished turn shorter than this shows no duration.
pub const DURATION_FLOOR: i64 = 60;

/// The languages the app ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    En,
    De,
    Es,
    Fr,
    Ja,
    Ko,
    PtBr,
    ZhHans,
    ZhHant,
}

impl Language {
    pub const ALL: [Self; 9] = [
        Self::En,
        Self::De,
        Self::Es,
        Self::Fr,
        Self::Ja,
        Self::Ko,
        Self::PtBr,
        Self::ZhHans,
        Self::ZhHant,
    ];

    /// The catalog's name for it.
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::De => "de",
            Self::Es => "es",
            Self::Fr => "fr",
            Self::Ja => "ja",
            Self::Ko => "ko",
            Self::PtBr => "pt-BR",
            Self::ZhHans => "zh-Hans",
            Self::ZhHant => "zh-Hant",
        }
    }

    /// The best of the nine for a BCP 47 tag (`zh-TW`, `pt_PT`, `de-AT`…);
    /// English for anything else.
    pub fn from_tag(tag: &str) -> Self {
        let tag = tag.replace('_', "-").to_ascii_lowercase();
        let mut parts = tag.split('-');
        match parts.next().unwrap_or("") {
            "de" => Self::De,
            "es" => Self::Es,
            "fr" => Self::Fr,
            "ja" => Self::Ja,
            "ko" => Self::Ko,
            "pt" => Self::PtBr,
            "zh" => {
                let rest: Vec<&str> = parts.collect();
                if rest.contains(&"hant") || rest.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")) {
                    Self::ZhHant
                } else {
                    Self::ZhHans
                }
            }
            _ => Self::En,
        }
    }

    /// `"✓ %@ finished"` in this language.
    pub fn finished_template(self) -> &'static str {
        match self {
            Self::En => "✓ %@ finished",
            Self::De => "✓ %@ fertig",
            Self::Es => "✓ %@ terminó",
            Self::Fr => "✓ %@ a terminé",
            Self::Ja => "✓ %@ が完了しました",
            Self::Ko => "✓ %@ 완료",
            Self::PtBr => "✓ %@ terminou",
            Self::ZhHans | Self::ZhHant => "✓ %@ 已完成",
        }
    }

    /// `"%@ needs you"` in this language.
    pub fn needs_you_template(self) -> &'static str {
        match self {
            Self::En => "%@ needs you",
            Self::De => "%@ braucht dich",
            Self::Es => "%@ te necesita",
            Self::Fr => "%@ vous attend",
            Self::Ja => "%@ が待っています",
            Self::Ko => "%@ 대기 중",
            Self::PtBr => "%@ precisa de você",
            Self::ZhHans | Self::ZhHant => "%@ 在等你",
        }
    }

    /// How Foundation writes hours and minutes here (abbreviated units).
    fn units(self) -> Units {
        match self {
            Self::En => Units {
                hours: ("", " hr"),
                minutes: ("", " min"),
                join: ", ",
                group: ",",
                min_grouping: 1,
            },
            Self::De => Units {
                hours: ("", " Std."),
                minutes: ("", " Min."),
                join: ", ",
                group: ".",
                min_grouping: 1,
            },
            Self::Es => Units {
                hours: ("", " h"),
                minutes: ("", " min"),
                join: " y ",
                group: ".",
                min_grouping: 2,
            },
            Self::Fr => Units {
                hours: ("", "\u{202F}h"),
                minutes: ("", "\u{A0}min"),
                join: " et ",
                group: "\u{202F}",
                min_grouping: 1,
            },
            Self::Ja => Units {
                hours: ("", "時間"),
                minutes: ("", "分"),
                join: " ",
                group: ",",
                min_grouping: 1,
            },
            Self::Ko => Units {
                hours: ("", "시간"),
                minutes: ("", "분"),
                join: " ",
                group: ",",
                min_grouping: 1,
            },
            Self::PtBr => Units {
                hours: ("", " h"),
                minutes: ("", " min"),
                join: " e ",
                group: ".",
                min_grouping: 1,
            },
            Self::ZhHans => Units {
                hours: ("", "小时"),
                minutes: ("", "分钟"),
                join: "",
                group: ",",
                min_grouping: 1,
            },
            Self::ZhHant => Units {
                hours: ("", "小時"),
                minutes: ("", "分鐘"),
                join: "",
                group: ",",
                min_grouping: 1,
            },
        }
    }
}

struct Units {
    hours: (&'static str, &'static str),
    minutes: (&'static str, &'static str),
    join: &'static str,
    /// Thousands separator.
    group: &'static str,
    /// Digits that must precede the first group before one is used (CLDR's
    /// minimumGroupingDigits: Spanish writes `1000` but `10.000`).
    min_grouping: usize,
}

fn grouped(value: i64, units: &Units) -> String {
    let digits = value.to_string();
    if digits.len() < 4 + units.min_grouping - 1 {
        return digits;
    }
    let mut out = String::new();
    let lead = digits.len() % 3;
    for (index, c) in digits.chars().enumerate() {
        if index > 0 && (index + 3 - lead).is_multiple_of(3) {
            out.push_str(units.group);
        }
        out.push(c);
    }
    out
}

/// `String(localized:)` of a one-argument template.
fn fill(template: &str, argument: &str) -> String {
    template.replacen("%@", argument, 1)
}

/// The detail as a card shows it: whitespace runs (newlines included)
/// collapsed to one space — by character, so a space carrying a combining
/// mark is a space — trimmed, and cut to [`DETAIL_LIMIT`] characters with an
/// ellipsis. `None` when nothing is left.
pub fn detail(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let words = text::split_characters(raw, |c| {
        text::character_is_whitespace(c) || text::character_is_newline(c)
    });
    let collapsed = words.join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if text::character_count(&collapsed) <= DETAIL_LIMIT {
        return Some(collapsed);
    }
    Some(format!(
        "{}…",
        text::character_prefix(&collapsed, DETAIL_LIMIT - 1)
    ))
}

/// Where an agent is, as a card names it: `label · session`.
///
/// A session name that is only numbers is no name: tmux calls its first
/// session `0`, and "mac-mini.lan · 0" reads as a broken counter. "Number"
/// is Swift's `Character.isNumber`, so `一` and `½` count too.
pub fn place(label: &str, session: Option<&str>) -> String {
    let session = text::trim_inline_spaces(session.unwrap_or(""));
    if session.is_empty() || text::characters(session).all(text::character_is_number) {
        return label.to_owned();
    }
    format!("{label} · {session}")
}

/// How long a turn ran, as a card shows it — `12 min`, `1 hr, 5 min` —
/// whole minutes only. `None` under [`DURATION_FLOOR`].
pub fn duration(seconds: i64, language: Language) -> Option<String> {
    if seconds < DURATION_FLOOR {
        return None;
    }
    let minutes = seconds / 60;
    let (hours, minutes) = (minutes / 60, minutes % 60);
    let units = language.units();
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!(
            "{}{}{}",
            units.hours.0,
            grouped(hours, &units),
            units.hours.1
        ));
    }
    if minutes > 0 {
        parts.push(format!(
            "{}{}{}",
            units.minutes.0,
            grouped(minutes, &units),
            units.minutes.1
        ));
    }
    Some(parts.join(units.join))
}

/// How a card may interrupt (`UNNotificationInterruptionLevel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptionLevel {
    /// In the list, no sound, no screen.
    Passive,
    /// An ordinary alert.
    Active,
    /// Breaks through Focus.
    TimeSensitive,
}

/// The visible half of a card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardText {
    pub title: String,
    pub body: String,
    /// Whether the card plays the default sound.
    pub sound: bool,
    pub level: InterruptionLevel,
}

/// "what — where", or just "where" when there is no what.
pub fn body(detail_text: Option<&str>, place: &str) -> String {
    match detail(detail_text) {
        Some(detail) => format!("{detail} — {place}"),
        None => place.to_owned(),
    }
}

/// Whether a finish of a turn that ran `duration_seconds` would wake the
/// screen — `Active`, with the sound if the user allows one: the finishes
/// the finishes record shares between the pushed copy and the local one.
///
/// It used to ask the sound switch too (B442): with the sound off the
/// record was never consulted, both copies of a long finish were `Active`,
/// and one turn woke the screen twice.
pub fn done_wakes(duration_seconds: i64, threshold: i64) -> bool {
    duration_seconds >= threshold
}

/// The finished card. A turn of `threshold` seconds or more is worth a sound
/// (if `sound` allows one) and is `Active`; a shorter one is silent and
/// `Passive` — information, not an interruption. `rings` false: this turn's
/// finish already rang (the other copy of it, pushed or local), and this
/// one only updates the card, silent and `Passive`.
#[allow(clippy::too_many_arguments)]
pub fn done(
    agent: &str,
    detail_text: Option<&str>,
    place: &str,
    duration_seconds: i64,
    sound: bool,
    threshold: i64,
    rings: bool,
    language: Language,
) -> CardText {
    let mut title = fill(language.finished_template(), agent);
    if let Some(ran) = duration(duration_seconds, language) {
        title.push_str(" · ");
        title.push_str(&ran);
    }
    let long = duration_seconds >= threshold && rings;
    CardText {
        title,
        body: body(detail_text, place),
        sound: long && sound,
        level: if long {
            InterruptionLevel::Active
        } else {
            InterruptionLevel::Passive
        },
    }
}

/// The needs-you card. `standing_count` counts the prompts waiting
/// including this one; above one it rides on the name as `+N`. Only the
/// `edge` — nobody was waiting, now someone is — rings and is
/// `TimeSensitive`; everything else updates the card silently.
pub fn attention(
    agent: &str,
    standing_count: i64,
    detail_text: Option<&str>,
    place: &str,
    edge: bool,
    sound: bool,
    language: Language,
) -> CardText {
    let who = if standing_count > 1 {
        format!("{agent} +{}", standing_count - 1)
    } else {
        agent.to_owned()
    };
    CardText {
        title: fill(language.needs_you_template(), &who),
        body: body(detail_text, place),
        sound: edge && sound,
        level: if edge {
            InterruptionLevel::TimeSensitive
        } else {
            InterruptionLevel::Passive
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - AgentNotificationCopyTests

    #[test]
    fn a_long_prompt_is_cut_by_character_with_an_ellipsis() {
        let shown = detail(Some(&"x".repeat(200))).unwrap();
        assert_eq!(text::character_count(&shown), DETAIL_LIMIT);
        assert!(shown.ends_with('…'));
        // Characters, not bytes: a CJK prompt is cut at the same count.
        assert_eq!(
            text::character_count(&detail(Some(&"长".repeat(200))).unwrap()),
            DETAIL_LIMIT
        );
        // And a cluster is never split.
        let flags = "🇨🇳".repeat(80);
        let cut = detail(Some(&flags)).unwrap();
        assert_eq!(cut, format!("{}…", "🇨🇳".repeat(71)));
    }

    #[test]
    fn a_prompt_that_fits_is_shown_whole() {
        assert_eq!(
            detail(Some("Upload the build with the new Xcode")).as_deref(),
            Some("Upload the build with the new Xcode")
        );
        let exact = "y".repeat(DETAIL_LIMIT);
        assert_eq!(detail(Some(&exact)), Some(exact));
    }

    #[test]
    fn newlines_and_whitespace_runs_collapse() {
        assert_eq!(
            detail(Some("  fix the\n\n  scroll  jitter\t please \n")).as_deref(),
            Some("fix the scroll jitter please")
        );
        // A space carrying a combining mark is one Swift Character, and a
        // whitespace one: it goes, mark and all.
        assert_eq!(detail(Some("a \u{301}b")).as_deref(), Some("a b"));
    }

    #[test]
    fn nothing_left_is_none_not_an_empty_line() {
        assert_eq!(detail(None), None);
        assert_eq!(detail(Some("")), None);
        assert_eq!(detail(Some(" \n\t ")), None);
    }

    #[test]
    fn the_place_is_the_connections_name_and_the_session() {
        assert_eq!(place("mac-mini", Some("work")), "mac-mini · work");
        assert_eq!(place("mac-mini", None), "mac-mini");
        assert_eq!(place("mac-mini", Some("  ")), "mac-mini");
    }

    #[test]
    fn tmuxs_numeric_default_session_name_is_not_a_place() {
        assert_eq!(place("mac-mini", Some("0")), "mac-mini");
        assert_eq!(place("mac-mini", Some("12")), "mac-mini");
        assert_eq!(place("mac-mini", Some("api-2")), "mac-mini · api-2");
        // Swift's isNumber: a CJK numeral is a number too.
        assert_eq!(place("mac-mini", Some("三")), "mac-mini");
        assert_eq!(place("mac-mini", Some("三号")), "mac-mini · 三号");
    }

    #[test]
    fn a_turn_under_a_minute_shows_no_duration() {
        assert_eq!(duration(0, Language::En), None);
        assert_eq!(duration(59, Language::En), None);
    }

    #[test]
    fn minutes_and_hours_read_as_foundation_spells_them() {
        assert_eq!(duration(60, Language::En).as_deref(), Some("1 min"));
        assert_eq!(
            duration(12 * 60 + 40, Language::En).as_deref(),
            Some("12 min")
        );
        assert_eq!(
            duration(65 * 60, Language::En).as_deref(),
            Some("1 hr, 5 min")
        );
    }

    /// Foundation's `Duration.UnitsFormatStyle` output on macOS 27.
    #[test]
    fn every_language_as_foundation_writes_it() {
        let seconds = [
            60,
            3660,
            7200,
            1000 * 3600 + 60,
            10_000 * 3600,
            1_234_567 * 3600,
        ];
        let want: [(Language, [&str; 6]); 9] = [
            (
                Language::En,
                [
                    "1 min",
                    "1 hr, 1 min",
                    "2 hr",
                    "1,000 hr, 1 min",
                    "10,000 hr",
                    "1,234,567 hr",
                ],
            ),
            (
                Language::De,
                [
                    "1 Min.",
                    "1 Std., 1 Min.",
                    "2 Std.",
                    "1.000 Std., 1 Min.",
                    "10.000 Std.",
                    "1.234.567 Std.",
                ],
            ),
            (
                Language::Es,
                [
                    "1 min",
                    "1 h y 1 min",
                    "2 h",
                    "1000 h y 1 min",
                    "10.000 h",
                    "1.234.567 h",
                ],
            ),
            (
                Language::Fr,
                [
                    "1\u{A0}min",
                    "1\u{202F}h et 1\u{A0}min",
                    "2\u{202F}h",
                    "1\u{202F}000\u{202F}h et 1\u{A0}min",
                    "10\u{202F}000\u{202F}h",
                    "1\u{202F}234\u{202F}567\u{202F}h",
                ],
            ),
            (
                Language::Ja,
                [
                    "1分",
                    "1時間 1分",
                    "2時間",
                    "1,000時間 1分",
                    "10,000時間",
                    "1,234,567時間",
                ],
            ),
            (
                Language::Ko,
                [
                    "1분",
                    "1시간 1분",
                    "2시간",
                    "1,000시간 1분",
                    "10,000시간",
                    "1,234,567시간",
                ],
            ),
            (
                Language::PtBr,
                [
                    "1 min",
                    "1 h e 1 min",
                    "2 h",
                    "1.000 h e 1 min",
                    "10.000 h",
                    "1.234.567 h",
                ],
            ),
            (
                Language::ZhHans,
                [
                    "1分钟",
                    "1小时1分钟",
                    "2小时",
                    "1,000小时1分钟",
                    "10,000小时",
                    "1,234,567小时",
                ],
            ),
            (
                Language::ZhHant,
                [
                    "1分鐘",
                    "1小時1分鐘",
                    "2小時",
                    "1,000小時1分鐘",
                    "10,000小時",
                    "1,234,567小時",
                ],
            ),
        ];
        for (language, expected) in want {
            for (seconds, expected) in seconds.iter().zip(expected) {
                assert_eq!(
                    duration(*seconds, language).as_deref(),
                    Some(expected),
                    "{language:?} {seconds}"
                );
            }
        }
    }

    #[test]
    fn a_finished_card_what_finished_and_how_long_it_ran_over_where() {
        let card = done(
            "claude",
            Some("Upload the build with the new Xcode"),
            "mac-mini · work",
            12 * 60,
            true,
            180,
            true,
            Language::En,
        );
        assert_eq!(card.title, "✓ claude finished · 12 min");
        assert_eq!(
            card.body,
            "Upload the build with the new Xcode — mac-mini · work"
        );
        assert!(card.sound);
        assert_eq!(card.level, InterruptionLevel::Active);
    }

    #[test]
    fn a_short_finish_is_information_not_an_interruption() {
        let card = done("claude", None, "mac-mini", 20, true, 180, true, Language::En);
        assert_eq!(card.title, "✓ claude finished");
        assert_eq!(card.body, "mac-mini");
        assert!(!card.sound);
        assert_eq!(card.level, InterruptionLevel::Passive);
    }

    #[test]
    fn a_needs_you_card_carries_the_question_and_the_count_on_the_name() {
        let card = attention(
            "claude",
            1,
            Some("Bash: rm -rf build"),
            "mac-mini · work",
            true,
            true,
            Language::En,
        );
        assert_eq!(card.title, "claude needs you");
        assert_eq!(card.body, "Bash: rm -rf build — mac-mini · work");
        assert!(card.sound);
        assert_eq!(card.level, InterruptionLevel::TimeSensitive);

        let more = attention("claude", 3, None, "mac-mini", false, true, Language::En);
        assert_eq!(more.title, "claude +2 needs you");
        assert_eq!(more.body, "mac-mini");
        assert!(!more.sound);
        assert_eq!(more.level, InterruptionLevel::Passive);
    }

    #[test]
    fn titles_in_every_language() {
        assert_eq!(
            done("claude", None, "m", 0, true, 180, true, Language::ZhHans).title,
            "✓ claude 已完成"
        );
        assert_eq!(
            attention("claude", 2, None, "m", true, true, Language::De).title,
            "claude +1 braucht dich"
        );
        assert_eq!(
            done("claude", None, "m", 3600, true, 180, true, Language::Ja).title,
            "✓ claude が完了しました · 1時間"
        );
    }

    #[test]
    fn language_from_a_tag() {
        assert_eq!(Language::from_tag("zh-Hans-CN"), Language::ZhHans);
        assert_eq!(Language::from_tag("zh_CN"), Language::ZhHans);
        assert_eq!(Language::from_tag("zh-TW"), Language::ZhHant);
        assert_eq!(Language::from_tag("zh-Hant-HK"), Language::ZhHant);
        assert_eq!(Language::from_tag("pt_PT"), Language::PtBr);
        assert_eq!(Language::from_tag("de-AT"), Language::De);
        assert_eq!(Language::from_tag("ru"), Language::En);
        assert_eq!(Language::from_tag(""), Language::En);
    }
}
