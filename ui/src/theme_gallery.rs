//! Settings › Theme (`Moshpit/UI/Settings/ThemeGalleryView.swift`) in NSH:
//! the terminal's color schemes, each row previewing the palette it
//! offers, the built-in ones and the user's own.
//!
//! The host owns the themes and the setting: it posts what the page shows
//! ([`Message`]), `{"op": "themes", "builtIn": [Palette…], "custom": […],
//! "selected": "github-dark", "deleting": "custom-1a2b3c4d"}`, again
//! whenever any of it changes. A [`Palette`] is a theme's name and the
//! colors its row previews. The page tells the host:
//!
//! - `{"ev": "select", "id": …}`: a row was picked (the host applies it at
//!   once);
//! - `{"ev": "new"}`, `{"ev": "import"}`: the add menu's two choices (the
//!   editor and the import sheet are the host's);
//! - `{"ev": "delete", "id": …}`, `{"ev": "cancel-delete"}`: the answer to
//!   the delete question;
//! - `{"ev": "back"}`.
//!
//! (`{"op": "reveal"}` scrolls the user's own themes to the top, for a
//! capture run.)
//!
//! Each row's long-press menu is the host's (UIKit's, on the NSH surface:
//! `NSHContextMenus`), for the view identified `theme-cell-<id>`: Duplicate
//! & Edit on a built-in theme; Edit, Duplicate, Copy JSON and Delete on the
//! user's own. A press on a row only picks it: editing is the menu's, so a
//! row has one target (§6.10, pain point 5). Delete asks the host to
//! confirm, and the host's state says which theme to ask about
//! (`deleting`): the page asks in the system's alert (§4.10).
//!
//! The page is `docs/nsh/UI-DESIGN.md` §6.10 (R14): two sections, Built-in
//! and Yours (with its footer), on the `ui` layer's rows: each the theme's
//! name over its eight ANSI colors, the tick on the one in effect (screen
//! readers hear it selected); the accessibility identifiers
//! `theme-row-<id>` and `theme-add`.

use std::{rc::Rc, sync::Arc};

use nsh_core::{color::Rgba, prelude::*};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    screen::Screen,
    theme,
    ui::{
        alert::{AlertDialogOptions, alert_dialog, delete_title},
        empty::{EmptyVariant, empty},
        menu::MenuEntry,
        page::page,
        row::row as list_row,
        section::{Inset, Section, section},
        tokens::{number, token},
    },
};

/// A terminal theme as the gallery shows it: its name and the colors its
/// row previews.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    pub id: String,
    pub name: String,
    #[serde(
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub background: Rgba,
    #[serde(
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub foreground: Rgba,
    #[serde(
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub cursor: Rgba,
    /// ANSI 0–7, black to white.
    #[serde(with = "eight")]
    pub ansi: [Rgba; 8],
}

/// Eight colors written `#RRGGBB`.
pub(crate) mod eight {
    use nsh_core::color::Rgba;
    use serde::{Deserialize, Deserializer, Serializer, ser::SerializeSeq};

    pub fn serialize<S: Serializer>(colors: &[Rgba; 8], serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(colors.len()))?;
        for color in colors {
            seq.serialize_element(&crate::theme::hex_string(*color))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[Rgba; 8], D::Error> {
        let texts = Vec::<String>::deserialize(deserializer)?;
        let colors = texts
            .iter()
            .map(|text| {
                crate::theme::parse_hex(text).ok_or_else(|| {
                    serde::de::Error::custom(format!("{text:?} is not #RRGGBB or #RRGGBBAA"))
                })
            })
            .collect::<Result<Vec<Rgba>, D::Error>>()?;
        colors
            .try_into()
            .map_err(|c: Vec<Rgba>| serde::de::Error::custom(format!("{} colors, not 8", c.len())))
    }
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Themes {
    /// The app's own themes.
    #[serde(rename = "builtIn")]
    pub built_in: Vec<Palette>,
    /// The user's, newest last.
    pub custom: Vec<Palette>,
    /// The id of the row to tick (the theme in effect).
    pub selected: String,
    /// The custom theme the delete card asks about, while it shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleting: Option<String>,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "themes", …}`: what the page shows now.
    Themes(Themes),
    /// `{"op": "reveal"}`: the user's own themes scrolled to the top, at
    /// once, their first row just under the bar (a capture run pictures
    /// them where the SwiftUI page scrolled to them shows them).
    Reveal,
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("theme", json)?;
        match op.as_str() {
            "themes" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Themes)
                .map_err(|e| format!("theme: themes: {e}")),
            "reveal" if fields.is_empty() => Ok(Message::Reveal),
            "reveal" => Err("theme: reveal takes nothing more".to_owned()),
            other => Err(format!("theme: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Themes(themes) => {
                let mut value = serde_json::to_value(themes).expect("themes serialize");
                value["op"] = json!("themes");
                value.to_string()
            }
            Message::Reveal => json!({"op": "reveal"}).to_string(),
        }
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// An ANSI color's square in a row's strip, and the room between two.
const SQUARE: f32 = 12.0;
const SQUARE_GAP: f32 = 2.0;

/// Settings › Theme.
#[component]
pub fn ThemeGalleryScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let params = channel.as_ref().map(|c| c.params()).unwrap_or_default();
    let bar = BarMetrics::or_standard(params.bar);
    let back_title = params.back.unwrap_or_else(|| t!("Settings"));
    let built_in = signal(Vec::<Palette>::new());
    let custom = signal(Vec::<Palette>::new());
    // None until the host's first post: a later change of the pick is felt.
    let selected = signal(None::<String>);
    let deleting = signal(None::<String>);
    // A point the bar's height above the first custom row: scrolled to the
    // list's top, it puts the row just under the bar.
    let custom_top = element_ref();
    channel::hear(
        channel.as_ref(),
        Screen::Theme,
        Message::parse,
        move |message| match message {
            Message::Themes(state) => {
                let before = selected.get_untracked();
                if before
                    .as_ref()
                    .is_some_and(|before| *before != state.selected)
                {
                    haptic(Haptic::Selection);
                }
                built_in.set(state.built_in);
                custom.set(state.custom);
                selected.set(Some(state.selected));
                deleting.set(state.deleting);
            }
            Message::Reveal => custom_top.reveal(
                RevealOptions::default()
                    .vertical(ScrollAlignment::Start)
                    .animated(false),
            ),
        },
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let select: Rc<dyn Fn(String)> = {
        let tell = tell.clone();
        Rc::new(move |id: String| tell(json!({"ev": "select", "id": id})))
    };
    // The theme the host asks about, while it asks: the system's alert.
    let asked = computed(move || {
        let id = deleting.get()?;
        custom.with(|c| c.iter().find(|p| p.id == id).cloned())
    });
    ask_to_delete(asked, deleting, tell.clone());
    let add = {
        let tell = tell.clone();
        move |choice: String| tell(json!({"ev": choice}))
    };
    let built_in_section = {
        let select = select.clone();
        dynamic(move || {
            let palettes = built_in.get();
            let select = select.clone();
            untracked(move || {
                themes(
                    section().header(t!("Built-in")),
                    &palettes,
                    selected,
                    select,
                )
                .test_id("theme-built-in")
                .into_element()
            })
        })
    };
    let custom_section = dynamic(move || {
        let palettes = custom.get();
        let select = select.clone();
        untracked(move || {
            let mine = section()
                .header(t!("Yours"))
                .footer(t!("Long-press a theme to duplicate, edit or delete it."))
                .test_id("theme-yours");
            if palettes.is_empty() {
                mine.item(
                    empty(t!("No custom themes yet."))
                        .variant(EmptyVariant::Inline)
                        .test_id("theme-none"),
                    Inset::Content,
                )
                .into_element()
            } else {
                themes(mine, &palettes, selected, select).into_element()
            }
        })
    });
    let list = page(
        bar,
        scrolled,
        "theme-list",
        [
            built_in_section,
            view()
                .relative()
                .w_full()
                .child(
                    view()
                        .absolute()
                        .left(px(0.0))
                        .right(px(0.0))
                        .top(move || px(-nav_bar::bar_space(bar).0))
                        .h(px(1.0))
                        .pointer_events(PointerEvents::None)
                        .element_ref(&custom_top),
                )
                .child(custom_section)
                .into_element(),
        ],
    );
    let nav = nav_bar::nav_bar(nav_bar::NavBar {
        metrics: bar,
        back_title,
        title: nav_bar::fixed(t!("Theme")),
        leading: None,
        trailing: vec![nav_bar::BarButton {
            label: t!("Add theme"),
            content: nav_bar::BarContent::Symbol("plus"),
            enabled: computed(|| true),
            emphasized: false,
            test_id: "theme-add",
            on_press: Box::new(|| {}),
            menu: Some(nav_bar::BarMenu {
                entries: vec![
                    MenuEntry::new("new", t!("New…")).symbol("plus"),
                    MenuEntry::new("import", t!("Import…")).symbol("square.and.arrow.down"),
                ],
                on_select: Box::new(add),
            }),
        }],
        on_back: Box::new({
            let tell = tell.clone();
            move || tell(json!({"ev": "back"}))
        }),
        scrolled,
    });
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("theme")
        .child(list)
        .child(nav)
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Rc<dyn Fn(serde_json::Value)> {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

/// Asks in the system's alert whether to delete the theme the host names
/// (`asked`), once each time it names one: Delete tells the host `delete`,
/// anything else `cancel-delete`; either way the page stops asking.
fn ask_to_delete(
    asked: Memo<Option<Palette>>,
    deleting: Signal<Option<String>>,
    tell: Rc<dyn Fn(serde_json::Value)>,
) {
    let asking = stored(None::<String>);
    effect(move || {
        let Some(palette) = asked.get() else {
            asking.replace(None);
            return;
        };
        if asking.with(|a| a.as_deref() == Some(palette.id.as_str())) {
            return;
        }
        asking.replace(Some(palette.id.clone()));
        let answer = delete_question(&palette.name).show();
        let tell = tell.clone();
        let id = palette.id;
        drop(spawn(async move {
            let delete = answer
                .await
                .is_ok_and(|r| r.action.as_deref() == Some("delete"));
            asking.replace(None);
            deleting.set(None);
            if delete {
                tell(json!({"ev": "delete", "id": id}));
            } else {
                tell(json!({"ev": "cancel-delete"}));
            }
        }));
    });
}

/// The question before deleting the user's theme `name` (§4.10).
pub fn delete_question(name: &str) -> AlertDialogOptions {
    alert_dialog()
        .title(delete_title(name))
        .description(t!(
            "This theme will be removed. Built-in themes are unaffected."
        ))
        .destructive("delete", t!("Delete"))
        .cancel("cancel", t!("Cancel"))
}

/// `palettes` in `section`, one row each; each row is the host's long-press
/// menu's part (`theme-cell-<id>`).
fn themes(
    mut section: Section,
    palettes: &[Palette],
    selected: Signal<Option<String>>,
    select: Rc<dyn Fn(String)>,
) -> Section {
    for palette in palettes {
        let id = palette.id.clone();
        let picked = {
            let id = id.clone();
            move || selected.with(|s| s.as_deref() == Some(id.as_str()))
        };
        let select = select.clone();
        let row = list_row(palette.name.clone())
            .below(strip(&palette.ansi))
            .check(picked)
            .on_press({
                let id = id.clone();
                move || select(id.clone())
            })
            .test_id(&format!("theme-row-{id}"));
        let inset = row.inset();
        section = section.item(
            view()
                .w_full()
                .test_id(&format!("theme-cell-{id}"))
                .child(row),
            inset,
        );
    }
    section
}

/// The eight base ANSI colors as a strip of small squares (`radius-xs`), a
/// hairline edge on each so a black or a white one shows on the card: the
/// palette's fingerprint.
pub fn strip(colors: &[Rgba; 8]) -> AnyElement {
    view()
        .flex_row()
        .flex_shrink_0()
        .gap(px(SQUARE_GAP))
        .accessibility_elements_hidden(true)
        .children(colors.iter().map(|color| {
            view()
                .w(px(SQUARE))
                .h(px(SQUARE))
                .rounded(px(number("radius-xs")))
                .border_curve(BorderCurve::Continuous)
                .bg(*color)
                .border(px(1.0))
                .border_color(move || token("border"))
                .into_element()
        }))
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(id: &str, name: &str) -> Palette {
        Palette {
            id: id.into(),
            name: name.into(),
            background: nsh_core::color::rgb(0x050507),
            foreground: nsh_core::color::rgb(0xC9D1D9),
            cursor: nsh_core::color::rgb(0x5FE3D8),
            ansi: [
                0x484F58, 0xFF7B72, 0x3FB950, 0xD29922, 0x58A6FF, 0xBC8CFF, 0x39C5CF, 0xB1BAC4,
            ]
            .map(nsh_core::color::rgb),
        }
    }

    #[test]
    fn messages_round_trip() {
        for themes in [
            Themes {
                built_in: vec![palette("github-dark", "GitHub Dark")],
                custom: vec![],
                selected: "github-dark".into(),
                deleting: None,
            },
            Themes {
                built_in: vec![palette("github-dark", "GitHub Dark")],
                custom: vec![palette("custom-1a2b3c4d", "Mine")],
                selected: "custom-1a2b3c4d".into(),
                deleting: Some("custom-1a2b3c4d".into()),
            },
        ] {
            let message = Message::Themes(themes);
            assert_eq!(Message::parse(&message.to_json()), Ok(message));
        }
    }

    #[test]
    fn reveal_takes_nothing_more() {
        assert_eq!(Message::parse(r#"{"op":"reveal"}"#), Ok(Message::Reveal));
        assert_eq!(
            Message::parse(&Message::Reveal.to_json()),
            Ok(Message::Reveal)
        );
        assert!(Message::parse(r#"{"op":"reveal","anchor":"custom"}"#).is_err());
    }

    #[test]
    fn the_host_message_as_written() {
        let message = Message::parse(
            r##"{"op":"themes","builtIn":[{"id":"github-dark","name":"GitHub Dark","background":"#050507","foreground":"#C9D1D9","cursor":"#5FE3D8","ansi":["#484F58","#FF7B72","#3FB950","#D29922","#58A6FF","#BC8CFF","#39C5CF","#B1BAC4"]}],"custom":[],"selected":"github-dark"}"##,
        );
        assert_eq!(
            message,
            Ok(Message::Themes(Themes {
                built_in: vec![palette("github-dark", "GitHub Dark")],
                custom: vec![],
                selected: "github-dark".into(),
                deleting: None,
            }))
        );
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        let base = || {
            serde_json::to_value(Themes {
                built_in: vec![palette("a", "A")],
                custom: vec![],
                selected: "a".into(),
                deleting: None,
            })
            .unwrap()
        };
        let with = |f: &dyn Fn(&mut serde_json::Value)| {
            let mut value = base();
            value["op"] = json!("themes");
            f(&mut value);
            value.to_string()
        };
        for (json, says) in [
            ("{".to_owned(), "not JSON"),
            (r#"{"op":"accents"}"#.to_owned(), "no op \"accents\""),
            (
                with(&|v| {
                    v.as_object_mut().unwrap().remove("selected");
                }),
                "selected",
            ),
            (
                with(&|v| v["builtIn"][0]["ansi"] = json!(["#000000"])),
                "1 colors, not 8",
            ),
            (with(&|v| v["builtIn"][0]["cursor"] = json!("teal")), "teal"),
            (with(&|v| v["builtIn"][0]["bright"] = json!([])), "bright"),
            (with(&|v| v["deleting"] = json!(7)), "integer `7`"),
        ] {
            let error = Message::parse(&json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }
}
