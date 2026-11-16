//! Settings › Voice Input › Language in NSH (its iOS host:
//! `Moshpit/UI/Settings/VoiceLanguageNSHPage.swift`): the language
//! dictation listens for. Which list it shows depends on the engine in use:
//! Apple's engines offer the languages this device can transcribe, one per
//! session, and Automatic; Whisper offers its own hundred or so, the user's
//! own first, and auto-detect.
//!
//! The host owns the setting and reads the lists (the device's speech
//! locales, Whisper's table, the names in the user's language): it posts
//! what the page shows ([`Message`]), `{"op": "languages", "engine":
//! "apple", "automatic": "English (United States)", "options": […],
//! "selected": ""}` (no `options` while Apple's are still being read) or
//! `{"op": "languages", "engine": "whisper", "suggested": […], "options":
//! […], "selected": "zh"}`, again whenever that changes. The page tells it
//! `{"ev": "select", "id": …}` (`""`: Automatic or auto-detect) and
//! `{"ev": "back"}`.
//!
//! The page is the redesign's (`docs/nsh/UI-DESIGN.md` §6.11, R15): a
//! search field over the list once there is one to search (it filters by
//! name or id, here on the page); Automatic first (Apple's with what it
//! resolves to under it, Whisper's on its own); Whisper's Common group of
//! the user's own languages, then All languages; each a check row, the
//! picked one checked and selected for screen readers; three skeleton rows
//! while Apple's languages are read. Test ids: `voice-language-<id>`
//! (`voice-language-automatic` for `""`), `voice-language-search`,
//! `voice-language-loading`, `voice-language-none` (a search that found
//! nothing).

use std::{rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    boundary::Tell,
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    screen::Screen,
    ui::{
        empty::{EmptyVariant, empty, skeleton_list},
        field::search_field,
        page,
        row::{Row, row},
        section::{Inset, section},
        tokens::token,
    },
    voice_engine::Engine,
};

/// A language the page offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Language {
    /// What the setting stores: a BCP 47 identifier (Apple's) or a Whisper
    /// code.
    pub id: String,
    /// Its name in the user's language.
    pub name: String,
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Languages {
    /// The engine in use, whose languages these are.
    pub engine: Engine,
    /// Apple's: what Automatic resolves to now, spelled out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic: Option<String>,
    /// Whisper's: the user's own languages, offered first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested: Vec<Language>,
    /// Every language (none while Apple's are being read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<Language>>,
    /// The picked language's id (`""`: Automatic, auto-detect).
    pub selected: String,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "languages", …}`: what the page shows now.
    Languages(Languages),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("voice-language", json)?;
        match op.as_str() {
            "languages" => {
                let languages: Languages =
                    serde_json::from_value(serde_json::Value::Object(fields))
                        .map_err(|e| format!("voice-language: languages: {e}"))?;
                match languages.engine {
                    Engine::Apple if languages.automatic.is_none() => {
                        Err("voice-language: Apple's languages say what Automatic is".to_owned())
                    }
                    Engine::Whisper if languages.options.is_none() => Err(
                        "voice-language: Whisper's languages come with their options".to_owned(),
                    ),
                    _ => Ok(Message::Languages(languages)),
                }
            }
            other => Err(format!("voice-language: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Languages(languages) => {
                let mut value = serde_json::to_value(languages).expect("languages serialize");
                value["op"] = json!("languages");
                value.to_string()
            }
        }
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// The test id of language `id`'s row.
pub fn row_id(id: &str) -> String {
    if id.is_empty() {
        "voice-language-automatic".to_owned()
    } else {
        format!("voice-language-{id}")
    }
}

/// Whether `language` is what `query` looks for: its name or its id holds
/// the query, case aside (an empty query finds everything).
pub fn matches(language: &Language, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || language.name.to_lowercase().contains(&query)
        || language.id.to_lowercase().contains(&query)
}

type Select = Rc<dyn Fn(String)>;

/// Settings › Voice Input › Language (§6.11).
#[component]
pub fn VoiceLanguageScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    // None until the host's first post.
    let engine = signal(None::<Engine>);
    let automatic = signal(String::new());
    let suggested = signal(Vec::<Language>::new());
    let options = signal(None::<Vec<Language>>);
    let selected = signal(None::<String>);
    // The body waits for the host's first post (B366's `ready_body`).
    let ready = signal(false);
    channel::hear(
        channel.as_ref(),
        Screen::VoiceLanguage,
        Message::parse,
        move |Message::Languages(state)| {
            engine.set(Some(state.engine));
            automatic.set(state.automatic.unwrap_or_default());
            suggested.set(state.suggested);
            options.set(state.options);
            selected.set(Some(state.selected));
            ready.set(true);
        },
    );
    let query = signal(String::new());
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let select: Select = {
        let tell = tell.clone();
        Rc::new(move |id: String| {
            // Felt on every press, the picked row's too, as the SwiftUI
            // page's `Haptics.select()`.
            haptic(Haptic::Selection);
            tell(json!({"ev": "select", "id": id}));
        })
    };
    // The lists, each a memo: the host posts them again with every pick,
    // and an unchanged list is not made again.
    let suggested = computed(move || suggested.get());
    let loaded = computed(move || options.with(Option::is_some));
    let all = computed(move || options.get().unwrap_or_default());
    let searching = computed(move || !query.with(|q| q.trim().is_empty()));
    let found = computed(move || {
        let query = query.get();
        all.with(|all| {
            all.iter()
                .filter(|l| matches(l, &query))
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    let body = move || {
        let select = select.clone();
        let lists = dynamic(move || {
            let (engine, searching) = (engine.get(), searching.get());
            let select = select.clone();
            untracked(move || match (engine, searching) {
                (None, _) => Vec::new(),
                (Some(_), true) => vec![results(found, selected, select).into_element()],
                (Some(Engine::Apple), false) => {
                    vec![apple(automatic, loaded, all, selected, select).into_element()]
                }
                (Some(Engine::Whisper), false) => whisper(suggested, all, selected, select),
            })
        });
        page::page(
            bar,
            scrolled,
            "voice-language-list",
            [
                when(
                    move || loaded.get(),
                    move || {
                        search_field(t!("Search languages"), move |text| query.set(text))
                            .test_id("voice-language-search")
                    },
                )
                .into_element(),
                lists,
            ],
        )
        .into_element()
    };
    let failed = tell.clone();
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("voice-language")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            failed,
        ))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: t!("Settings"),
            title: nav_bar::fixed(t!("Language")),
            leading: None,
            trailing: Vec::new(),
            on_back: Box::new(move || tell(json!({"ev": "back"}))),
            scrolled,
        }))
}

/// A language's row: its name (over `detail`, if any), checked when it is
/// the one picked; the whole row its button.
fn language_row(
    id: String,
    name: String,
    detail: Option<Rc<dyn Fn() -> String>>,
    selected: Signal<Option<String>>,
    select: Select,
) -> Row {
    let picked = {
        let id = id.clone();
        move || selected.with(|s| s.as_deref() == Some(id.as_str()))
    };
    let mut row = row(name).check(picked).test_id(&row_id(&id));
    if let Some(detail) = detail {
        row = row.subtitle_with(move || detail().into());
    }
    row.on_press(move || select(id.clone()))
}

/// Rows of `languages`, one each.
fn rows(languages: &[Language], selected: Signal<Option<String>>, select: &Select) -> Vec<Row> {
    languages
        .iter()
        .map(|l| language_row(l.id.clone(), l.name.clone(), None, selected, select.clone()))
        .collect()
}

/// Apple's engines: Automatic (what it resolves to under it), then the
/// device's languages, or three skeleton rows while they are read.
fn apple(
    automatic: Signal<String>,
    loaded: Memo<bool>,
    all: Memo<Vec<Language>>,
    selected: Signal<Option<String>>,
    select: Select,
) -> AnyElement {
    dynamic(move || {
        let (loaded, all) = (loaded.get(), all.get());
        let select = select.clone();
        untracked(move || {
            let auto = language_row(
                String::new(),
                t!("Automatic"),
                Some(Rc::new(move || automatic.get())),
                selected,
                select.clone(),
            );
            let mut group = section().row(auto);
            if loaded {
                group = group.rows(rows(&all, selected, &select));
            } else {
                group = group.item(
                    skeleton_list(3).test_id("voice-language-loading"),
                    Inset::Content,
                );
            }
            group.footer(t!(
                "Only languages this device can transcribe. Each downloads once, then works offline."
            ))
        })
    })
    .into_element()
}

/// Whisper: Automatic on its own, the user's own languages (Common), then
/// all of them.
fn whisper(
    suggested: Memo<Vec<Language>>,
    all: Memo<Vec<Language>>,
    selected: Signal<Option<String>>,
    select: Select,
) -> Vec<AnyElement> {
    let automatic = section()
        .row(language_row(
            String::new(),
            t!("Automatic"),
            Some(Rc::new(|| t!("Whisper decides from what it hears"))),
            selected,
            select.clone(),
        ))
        .footer(t!(
            "Picking your language keeps the foreign words you say mid-sentence intact."
        ));
    let common = {
        let select = select.clone();
        dynamic(move || {
            let suggested = suggested.get();
            let select = select.clone();
            untracked(move || {
                (!suggested.is_empty()).then(|| {
                    section()
                        .header(t!("Common"))
                        .rows(rows(&suggested, selected, &select))
                })
            })
        })
    };
    let rest = dynamic(move || {
        let all = all.get();
        let select = select.clone();
        untracked(move || {
            section()
                .header(t!("All languages"))
                .rows(rows(&all, selected, &select))
        })
    });
    vec![
        automatic.into_element(),
        common.into_element(),
        rest.into_element(),
    ]
}

/// What a search found, or that it found nothing.
fn results(
    found: Memo<Vec<Language>>,
    selected: Signal<Option<String>>,
    select: Select,
) -> AnyElement {
    dynamic(move || {
        let found = found.get();
        let select = select.clone();
        untracked(move || {
            if found.is_empty() {
                return empty(t!("No language matches"))
                    .variant(EmptyVariant::Inline)
                    .icon("magnifyingglass")
                    .test_id("voice-language-none")
                    .into_element();
            }
            section()
                .rows(rows(&found, selected, &select))
                .into_element()
        })
    })
    .into_element()
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn language(id: &str, name: &str) -> Language {
        Language {
            id: id.into(),
            name: name.into(),
        }
    }

    #[test]
    fn messages_round_trip() {
        for languages in [
            Languages {
                engine: Engine::Apple,
                automatic: Some("English (United States)".into()),
                suggested: vec![],
                options: None,
                selected: String::new(),
            },
            Languages {
                engine: Engine::Apple,
                automatic: Some("Chinese (China mainland)".into()),
                suggested: vec![],
                options: Some(vec![language("en-US", "English (United States)")]),
                selected: "en-US".into(),
            },
            Languages {
                engine: Engine::Whisper,
                automatic: None,
                suggested: vec![language("zh", "Chinese"), language("en", "English")],
                options: Some(vec![language("af", "Afrikaans"), language("zh", "Chinese")]),
                selected: "zh".into(),
            },
        ] {
            let message = Message::Languages(languages);
            assert_eq!(Message::parse(&message.to_json()), Ok(message));
        }
    }

    #[test]
    fn the_host_messages_as_written() {
        assert_eq!(
            Message::parse(
                r#"{"op":"languages","engine":"apple","automatic":"English","selected":""}"#
            ),
            Ok(Message::Languages(Languages {
                engine: Engine::Apple,
                automatic: Some("English".into()),
                suggested: vec![],
                options: None,
                selected: String::new(),
            }))
        );
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        for (json, says) in [
            ("{", "not JSON"),
            (r#"{"op":"engine"}"#, "no op \"engine\""),
            (
                r#"{"op":"languages","engine":"apple","selected":""}"#,
                "Automatic",
            ),
            (
                r#"{"op":"languages","engine":"whisper","selected":""}"#,
                "options",
            ),
            (
                r#"{"op":"languages","engine":"whisper","options":[{"id":"zh"}],"selected":""}"#,
                "name",
            ),
            (
                r#"{"op":"languages","engine":"apple","automatic":"x","selected":"","more":1}"#,
                "more",
            ),
        ] {
            let error = Message::parse(json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }

    #[test]
    fn automatic_has_its_own_row_id() {
        assert_eq!(row_id(""), "voice-language-automatic");
        assert_eq!(row_id("zh"), "voice-language-zh");
    }
}
