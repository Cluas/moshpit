//! Add Connection › Select Key (`docs/nsh/UI-DESIGN.md` §6.6) in NSH, in
//! its sheet: the saved keys to sign in with, or pasting a PEM by hand
//! instead; with no keys yet, a way to make one.
//!
//! The host owns the keys and the connection's choice: it posts what the
//! page shows ([`Message`]), `{"op": "keys", "keys": [{"id", "name",
//! "badge", "refused"}…], "selected": id | null}`. A key the connect path
//! can't sign with (an ECDSA P-384 key an older build imported) carries the
//! key type it refuses (`refused`): its row says so and is disabled, unless
//! it is the one already picked. The page tells the host
//! `{"ev": "select", "id": id | null}` (null: paste a PEM instead),
//! `{"ev": "add"}` (the bar's +, or the empty group's Generate key) and
//! `{"ev": "cancel"}`; the host applies the pick and closes the sheet, or
//! opens Add Key.
//!
//! Built from the ui layer (R10): SAVED KEYS as single-choice rows with a
//! tick (or an inline empty state with Generate key), OR with the paste
//! row. The identifiers: `key-picker-add`, `key-picker-cancel`,
//! `key-picker-key-<id>`, `key-picker-paste-pem`, `key-picker-empty` and
//! `key-picker-empty-generate`.

use std::{rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    screen::Screen,
    ui::{
        button::{Size, Variant, button},
        empty::{EmptyVariant, empty},
        page,
        row::{Row, row},
        section::{Inset, section},
        tokens::token,
    },
};

/// One saved key, as the picker shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickerKey {
    pub id: String,
    pub name: String,
    /// Its algorithm badge (`ED25519`).
    pub badge: String,
    /// The key type the app can't sign in with yet (`ECDSA P-384`), if
    /// that is this key's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Picker {
    pub keys: Vec<PickerKey>,
    /// The key picked now; `None`: a PEM pasted by hand.
    pub selected: Option<String>,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "keys", …}`: what the page shows now.
    Keys(Picker),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("key-picker", json)?;
        match op.as_str() {
            "keys" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Keys)
                .map_err(|e| format!("key-picker: keys: {e}")),
            other => Err(format!("key-picker: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Keys(picker) => {
                let mut value = serde_json::to_value(picker).expect("a picker serializes");
                value["op"] = json!("keys");
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

/// What tells the host.
type Tell = Rc<dyn Fn(serde_json::Value)>;

/// Select Key.
#[component]
pub fn KeyPickerScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let keys = signal(Vec::<PickerKey>::new());
    let selected = signal(None::<String>);
    let ready = signal(false);
    channel::hear(
        channel.as_ref(),
        Screen::KeyPicker,
        Message::parse,
        move |Message::Keys(picker)| {
            keys.set(picker.keys);
            selected.set(picker.selected);
            ready.set(true);
        },
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let body = {
        let tell = tell.clone();
        move || {
            let paste = section().header(t!("OR")).row(
                row(t!("Paste a PEM instead…"))
                    .subtitle(t!("Enter a private key by hand"))
                    .leading_icon("doc.text")
                    .check(move || selected.with(Option::is_none))
                    .selected(move || selected.with(Option::is_none))
                    .on_press({
                        let tell = tell.clone();
                        move || tell(json!({"ev": "select", "id": null}))
                    })
                    .test_id("key-picker-paste-pem"),
            );
            page::page(
                bar,
                scrolled,
                "key-picker-list",
                [saved(keys, selected, tell.clone()), paste.into_element()],
            )
            .into_element()
        }
    };
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("key-picker")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: String::new(),
            title: nav_bar::fixed(t!("Select Key")),
            leading: Some(nav_bar::BarButton {
                label: t!("Cancel"),
                content: nav_bar::BarContent::Text,
                enabled: computed(|| true),
                emphasized: false,
                test_id: "key-picker-cancel",
                on_press: Box::new({
                    let tell = tell.clone();
                    move || tell(json!({"ev": "cancel"}))
                }),
                menu: None,
            }),
            trailing: vec![nav_bar::BarButton {
                label: t!("Add Key"),
                content: nav_bar::BarContent::Circled("plus"),
                enabled: computed(|| true),
                emphasized: false,
                test_id: "key-picker-add",
                on_press: Box::new(move || tell(json!({"ev": "add"}))),
                menu: None,
            }],
            on_back: Box::new(|| {}),
            scrolled,
        }))
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

/// SAVED KEYS: a row for each key, the pick ticked; with none, what to do
/// about it. Made again when the keys change.
fn saved(keys: Signal<Vec<PickerKey>>, selected: Signal<Option<String>>, tell: Tell) -> AnyElement {
    dynamic(move || {
        let keys = keys.get();
        let tell = tell.clone();
        untracked(move || {
            let section = section()
                .header(t!("SAVED KEYS"))
                .test_id("key-picker-saved");
            if keys.is_empty() {
                return section
                    .item(
                        empty(t!("No keys yet"))
                            .variant(EmptyVariant::Inline)
                            .icon("key")
                            .action(
                                button(t!("Generate key"))
                                    .variant(Variant::Secondary)
                                    .size(Size::Sm)
                                    .on_press(move || tell(json!({"ev": "add"})))
                                    .test_id("key-picker-empty-generate"),
                            )
                            .test_id("key-picker-empty"),
                        Inset::Content,
                    )
                    .into_element();
            }
            section
                .rows(
                    keys.into_iter()
                        .map(|key| saved_row(key, selected, tell.clone())),
                )
                .into_element()
        })
    })
    .into_element()
}

/// A saved key's row: its name, and its badge or why it can't be used; the
/// pick ticked and selected for screen readers. A key the app can't sign
/// in with is disabled, unless it is the one already picked.
fn saved_row(key: PickerKey, selected: Signal<Option<String>>, tell: Tell) -> Row {
    let active = {
        let id = key.id.clone();
        move || selected.with(|s| s.as_deref() == Some(id.as_str()))
    };
    let meta = match &key.refused {
        Some(kind) => t!("Can't sign in with %@ keys yet", arg1 = kind),
        None => key.badge.clone(),
    };
    let refused = key.refused.is_some();
    let id = key.id.clone();
    row(key.name.clone())
        .subtitle(meta)
        .leading_icon("key")
        .check(active.clone())
        .selected(active.clone())
        .disabled(move || refused && !active())
        .on_press(move || tell(json!({"ev": "select", "id": id})))
        .test_id(&format!("key-picker-key-{}", key.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str, refused: Option<&str>) -> PickerKey {
        PickerKey {
            id: id.into(),
            name: format!("key {id}"),
            badge: "ED25519".into(),
            refused: refused.map(Into::into),
        }
    }

    #[test]
    fn messages_round_trip() {
        for picker in [
            Picker {
                keys: vec![],
                selected: None,
            },
            Picker {
                keys: vec![key("a", None), key("b", Some("ECDSA P-384"))],
                selected: Some("b".into()),
            },
        ] {
            let message = Message::Keys(picker);
            assert_eq!(Message::parse(&message.to_json()), Ok(message));
        }
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        for (json, says) in [
            ("1", "a message is a JSON object"),
            (r#"{"op":"pick"}"#, "no op \"pick\""),
            (r#"{"op":"keys","selected":null}"#, "keys"),
            (
                r#"{"op":"keys","keys":[{"id":"a","name":"a"}],"selected":null}"#,
                "badge",
            ),
        ] {
            let error = Message::parse(json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }
}
