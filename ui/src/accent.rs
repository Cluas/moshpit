//! Settings › Accent (`Moshpit/UI/Settings/AccentGalleryView.swift`) in NSH:
//! the accent color of the app's chrome, from the built-in ones and the
//! user's own.
//!
//! The host owns the accents and the setting: it posts what the page shows
//! ([`Message`]), `{"op": "accents", "builtIn": [{"id", "name", "accent",
//! "screenBG"}…], "custom": […], "selected": "signal-room", "deleting":
//! "app-1a2b3c4d"}`, again whenever that changes. The page tells it
//! `{"ev": "select", "id": …}` when a swatch is picked (the host ticks it and
//! applies it as the page goes away), `{"ev": "add"}` for New…,
//! `{"ev": "delete", "id": …}` and `{"ev": "cancel-delete"}` from the delete
//! question, and `{"ev": "back"}`. The editor (its own page,
//! `accent_editor`) and a custom accent's context menu (UIKit's, on the NSH
//! surface: Edit, Delete) are the host's; the menu finds a custom accent by
//! its accessibility identifier, `accent-cell-<id>`, and its Delete asks the
//! host, whose state then names the accent to ask about (`deleting`): the
//! page asks in the system's alert (§4.10).
//!
//! The page is `docs/nsh/UI-DESIGN.md` §6.10 (R14): the accents in a grid,
//! each a 44 pt [`crate::ui::swatch::Swatch`] with its name under it, the
//! one in use ringed (selected for screen readers); Built-in, then Yours.
//! The accessibility identifiers are `accent-row-<id>` (a swatch) and
//! `accent-add`.

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
        page::page,
        row::PADDING_X,
        section::{Inset, Section, section},
        swatch::swatch,
        tokens::{number, token},
    },
};

/// One accent, as the page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Accent {
    pub id: String,
    pub name: String,
    /// Its accent color (`#RRGGBB`).
    #[serde(
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub accent: Rgba,
    /// The screen background it derives (`#RRGGBB`), its swatch's ground.
    #[serde(
        rename = "screenBG",
        deserialize_with = "theme::color",
        serialize_with = "theme::write_color"
    )]
    pub screen_bg: Rgba,
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Accents {
    /// The app's own accents.
    #[serde(rename = "builtIn")]
    pub built_in: Vec<Accent>,
    /// The user's, newest last.
    pub custom: Vec<Accent>,
    /// The id of the row to tick.
    pub selected: String,
    /// The custom accent the delete card asks about, while it shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleting: Option<String>,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "accents", …}`: what the page shows now.
    Accents(Accents),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("accent", json)?;
        match op.as_str() {
            "accents" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Accents)
                .map_err(|e| format!("accent: accents: {e}")),
            other => Err(format!("accent: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Accents(accents) => {
                let mut value = serde_json::to_value(accents).expect("accents serialize");
                value["op"] = json!("accents");
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

/// A swatch's circle on this page (§6.10).
pub const SWATCH: f32 = 44.0;
/// The grid: columns at least this wide, this far apart; rows `ROW_GAP`
/// apart, inside the card's padding.
pub const COLUMN_MIN: f32 = 72.0;
pub const COLUMN_GAP: f32 = 8.0;
const ROW_GAP: f32 = 16.0;

/// The grid's columns in `width` (the card's content): how many fit at
/// least [`COLUMN_MIN`] wide, and how wide each is then.
pub fn columns(width: f32) -> (usize, f32) {
    let count = ((width + COLUMN_GAP) / (COLUMN_MIN + COLUMN_GAP))
        .floor()
        .max(1.0);
    let column = (width - COLUMN_GAP * (count - 1.0)) / count;
    (count as usize, column.max(0.0))
}

/// Settings › Accent.
#[component]
pub fn AccentScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let built_in = signal(Vec::<Accent>::new());
    let custom = signal(Vec::<Accent>::new());
    // None until the host's first post: then a later change of the pick is
    // felt.
    let selected = signal(None::<String>);
    let deleting = signal(None::<String>);
    channel::hear(
        channel.as_ref(),
        Screen::Accent,
        Message::parse,
        move |Message::Accents(state)| {
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
        },
    );
    // The accent the host asks about, while it asks: the system's alert.
    let asked = computed(move || {
        let id = deleting.get()?;
        custom.with(|c| c.iter().find(|a| a.id == id).cloned())
    });
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    ask_to_delete(asked, deleting, tell.clone());
    let select: Rc<dyn Fn(String)> = {
        let tell = tell.clone();
        Rc::new(move |id: String| tell(json!({"ev": "select", "id": id})))
    };
    let add = {
        let tell = tell.clone();
        move || tell(json!({"ev": "add"}))
    };
    // The card's content width: the page's, less the page's and the card's
    // insets on both sides.
    let width = move || window().width.0 - 2.0 * (number("space-page") + PADDING_X);
    let built_in_section = {
        let select = select.clone();
        dynamic(move || {
            let accents = built_in.get();
            let select = select.clone();
            untracked(move || {
                grid_section(
                    section().header(t!("Built-in")),
                    accents,
                    false,
                    selected,
                    select,
                    width,
                )
                .test_id("accent-built-in")
                .into_element()
            })
        })
    };
    let custom_section = dynamic(move || {
        let accents = custom.get();
        let select = select.clone();
        untracked(move || {
            let mine = section()
                .header(t!("Yours"))
                .footer(t!("Long-press an accent to edit or delete it."))
                .test_id("accent-yours");
            if accents.is_empty() {
                mine.item(
                    empty(t!("No custom accents yet."))
                        .variant(EmptyVariant::Inline)
                        .test_id("accent-none"),
                    Inset::Content,
                )
                .into_element()
            } else {
                grid_section(mine, accents, true, selected, select, width).into_element()
            }
        })
    });
    let nav = nav_bar::nav_bar(nav_bar::NavBar {
        metrics: bar,
        back_title: t!("Settings"),
        title: nav_bar::fixed(t!("Accent")),
        leading: None,
        trailing: vec![nav_bar::BarButton {
            label: t!("New accent"),
            content: nav_bar::BarContent::Symbol("plus"),
            enabled: computed(|| true),
            emphasized: false,
            test_id: "accent-add",
            on_press: Box::new(add),
            menu: None,
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
        .test_id("accent")
        .child(page(
            bar,
            scrolled,
            "accent-list",
            [built_in_section, custom_section],
        ))
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

/// Asks in the system's alert whether to delete the accent the host names
/// (`asked`), once each time it names one: Delete tells the host `delete`,
/// anything else `cancel-delete`.
fn ask_to_delete(
    asked: Memo<Option<Accent>>,
    deleting: Signal<Option<String>>,
    tell: Rc<dyn Fn(serde_json::Value)>,
) {
    let asking = stored(None::<String>);
    effect(move || {
        let Some(accent) = asked.get() else {
            asking.replace(None);
            return;
        };
        if asking.with(|a| a.as_deref() == Some(accent.id.as_str())) {
            return;
        }
        asking.replace(Some(accent.id.clone()));
        let answer = delete_question(&accent.name).show();
        let tell = tell.clone();
        let id = accent.id;
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

/// The question before deleting the user's accent `name` (§4.10).
pub fn delete_question(name: &str) -> AlertDialogOptions {
    alert_dialog()
        .title(delete_title(name))
        .description(t!(
            "This accent will be removed. Built-in accents are unaffected."
        ))
        .destructive("delete", t!("Delete"))
        .cancel("cancel", t!("Cancel"))
}

/// `accents` as a grid of swatches on `section`'s card; a custom accent's
/// cell is the host's long-press menu's part (`accent-cell-<id>`).
fn grid_section(
    section: Section,
    accents: Vec<Accent>,
    custom: bool,
    selected: Signal<Option<String>>,
    select: Rc<dyn Fn(String)>,
    width: impl Fn() -> f32 + Copy + 'static,
) -> Section {
    let column = move || px(columns(width()).1 - 0.01);
    let cells = accents.into_iter().map(|accent| {
        let id = accent.id.clone();
        let picked = {
            let id = id.clone();
            move || selected.with(|s| s.as_deref() == Some(id.as_str()))
        };
        let select = select.clone();
        let tint = accent.accent;
        let mut cell = view().w(column).items_center();
        if custom {
            cell = cell.test_id(&format!("accent-cell-{id}"));
        }
        cell.child(
            swatch(accent.name.clone(), move || AdaptiveColor::from(tint))
                .size(SWATCH)
                .selected(picked)
                .on_press({
                    let id = id.clone();
                    move || select(id.clone())
                })
                .test_id(&format!("accent-row-{id}")),
        )
        .into_element()
    });
    let grid = view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .gap_x(px(COLUMN_GAP))
        .gap_y(px(ROW_GAP))
        .p(px(PADDING_X))
        .children(cells.collect::<Vec<_>>());
    section.item(grid, Inset::Content)
}

/// `#RRGGBB` of an opaque color, as the app stores an accent.
pub fn hex(color: Rgba) -> String {
    theme::hex_string(color)[1..7].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accent(id: &str, name: &str, accent: u32, screen_bg: u32) -> Accent {
        Accent {
            id: id.into(),
            name: name.into(),
            accent: nsh_core::color::rgb(accent),
            screen_bg: nsh_core::color::rgb(screen_bg),
        }
    }

    fn signal_room() -> Accent {
        accent("signal-room", "Signal Room", 0x6C6BEF, 0x06070A)
    }

    #[test]
    fn messages_round_trip() {
        for accents in [
            Accents {
                built_in: vec![signal_room()],
                custom: vec![],
                selected: "signal-room".into(),
                deleting: None,
            },
            Accents {
                built_in: vec![signal_room()],
                custom: vec![accent("app-1a2b3c4d", "My Accent", 0x2FA871, 0x020503)],
                selected: "app-1a2b3c4d".into(),
                deleting: Some("app-1a2b3c4d".into()),
            },
        ] {
            let message = Message::Accents(accents);
            assert_eq!(Message::parse(&message.to_json()), Ok(message));
        }
    }

    #[test]
    fn the_host_message_as_written() {
        assert_eq!(
            Message::parse(
                r##"{"op":"accents","builtIn":[{"id":"signal-room","name":"Signal Room","accent":"#6C6BEF","screenBG":"#06070A"}],"custom":[],"selected":"signal-room"}"##
            ),
            Ok(Message::Accents(Accents {
                built_in: vec![signal_room()],
                custom: vec![],
                selected: "signal-room".into(),
                deleting: None,
            }))
        );
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        for (json, says) in [
            ("{", "not JSON"),
            (r#"{"op":"icons"}"#, "no op \"icons\""),
            (r#"{"op":"accents","builtIn":[],"custom":[]}"#, "selected"),
            (
                r##"{"op":"accents","builtIn":[{"id":"a","name":"A","accent":"teal","screenBG":"#000000"}],"custom":[],"selected":"a"}"##,
                "teal",
            ),
            (
                r##"{"op":"accents","builtIn":[{"id":"a","name":"A","accent":"#000000"}],"custom":[],"selected":"a"}"##,
                "screenBG",
            ),
        ] {
            let error = Message::parse(json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }

    #[test]
    fn the_code_is_the_stored_hex() {
        assert_eq!(hex(nsh_core::color::rgb(0x6C6BEF)), "6C6BEF");
        assert_eq!(hex(nsh_core::color::rgb(0x02A0F3)), "02A0F3");
    }
}
