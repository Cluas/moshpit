//! Settings › Theme › Import… (`ThemeImportView.swift` was) in NSH, in its
//! sheet: a theme's JSON pasted or typed, imported by the host.
//!
//! The text is the page's: a stacked mono field that grows with what is
//! pasted, Paste from Clipboard (which replaces the text with the
//! clipboard's, as the SwiftUI sheet's did), and Import in the bar once
//! there is something to import. The host parses and saves
//! (`ThemeStore.importThemes`) and posts why it couldn't ([`Message`]),
//! `{"op": "themeImport", "error"}`; the page tells it `{"ev": "import",
//! "text"}` and `{"ev": "cancel"}`.
//!
//! Identifiers: `theme-import`, `theme-import-list`, `theme-import-editor`
//! (`-input`: the text), `theme-import-paste`, `theme-import-error`,
//! `theme-import-cancel`, `theme-import-submit`.

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
        field::{FieldLayout, field, remount, text_input},
        notice::{BannerTone, banner},
        page,
        row::{RowTone, row},
        section::{Inset, section},
        tokens::token,
    },
};

/// What the host says.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct ThemeImport {
    /// Why the last import didn't take.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "themeImport", …}`.
    ThemeImport(ThemeImport),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("theme-import", json)?;
        match op.as_str() {
            "themeImport" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::ThemeImport)
                .map_err(|e| format!("theme-import: themeImport: {e}")),
            other => Err(format!("theme-import: no op {other:?}")),
        }
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// The editor's least and most lines before it scrolls with the page.
const LINES: (u32, u32) = (10, 40);

/// Import Theme.
#[component]
pub fn ThemeImportScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let state = signal(None::<ThemeImport>);
    channel::hear(
        channel.as_ref(),
        Screen::ThemeImport,
        Message::parse,
        move |Message::ThemeImport(next)| state.set(Some(next)),
    );
    let text = signal(String::new());
    // Moves when the page sets the text (a paste): the field is made again
    // from it.
    let pasted = signal(0u64);
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let ready = computed(move || state.with(Option::is_some));
    let filled = computed(move || !text.with(|t| t.trim().is_empty()));
    let error = move || state.with(|s| s.as_ref().and_then(|s| s.error.clone()));
    let body = move || {
        page::page(
            bar,
            scrolled,
            "theme-import-list",
            [
                remount(
                    move || pasted.get(),
                    move || {
                        section()
                            .footer(t!(
                                "Paste a theme exported from Moshpit, or any JSON object with \"background\", \"foreground\" and the eight ANSI color names (\"black\", \"red\", …). Bright colors are optional."
                            ))
                            .item(
                                field(
                                    t!("JSON"),
                                    text_input()
                                        .mono()
                                        .multiline(LINES.0, LINES.1)
                                        .default_value(move || text.get_untracked())
                                        .on_change(move |typed| text.set(typed)),
                                )
                                .layout(FieldLayout::Stacked)
                                .test_id("theme-import-editor"),
                                Inset::Content,
                            )
                            .into_element()
                    },
                ),
                when(
                    move || error().is_some(),
                    move || {
                        banner(BannerTone::Destructive, move || {
                            error().unwrap_or_default().into()
                        })
                        .icon("exclamationmark.circle.fill")
                        .test_id("theme-import-error")
                    },
                )
                .into_element(),
                section()
                    .row(
                        row(t!("Paste from Clipboard"))
                            .tone(RowTone::Primary)
                            .leading_icon("doc.on.clipboard")
                            .test_id("theme-import-paste")
                            .on_press(move || {
                                drop(spawn(async move {
                                    let clip = clipboard_string().await;
                                    if !clip.is_empty() {
                                        text.set(clip);
                                        pasted.update(|n| *n += 1);
                                    }
                                }));
                            }),
                    )
                    .into_element(),
                // Room for the keyboard while it is up.
                view().w_full().pb_keyboard().into_element(),
            ],
        )
        .into_element()
    };
    let cancel = {
        let tell = tell.clone();
        move || tell(json!({"ev": "cancel"}))
    };
    let import = {
        let tell = tell.clone();
        move || {
            if filled.get_untracked() {
                dismiss_keyboard();
                tell(json!({"ev": "import", "text": text.get_untracked()}));
            }
        }
    };
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("theme-import")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: String::new(),
            title: nav_bar::fixed(t!("Import Theme")),
            leading: Some(nav_bar::BarButton {
                label: t!("Cancel"),
                content: nav_bar::BarContent::Text,
                enabled: computed(|| true),
                emphasized: false,
                test_id: "theme-import-cancel",
                on_press: Box::new(cancel),
                menu: None,
            }),
            trailing: vec![nav_bar::BarButton {
                label: t!("Import"),
                content: nav_bar::BarContent::Text,
                enabled: filled,
                emphasized: true,
                test_id: "theme-import-submit",
                on_press: Box::new(import),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_parses() {
        assert_eq!(
            Message::parse(r#"{"op":"themeImport"}"#),
            Ok(Message::ThemeImport(ThemeImport::default()))
        );
        assert_eq!(
            Message::parse(r#"{"op":"themeImport","error":"Not JSON"}"#),
            Ok(Message::ThemeImport(ThemeImport {
                error: Some("Not JSON".into())
            }))
        );
        assert!(Message::parse(r#"{"op":"themeImport","text":"x"}"#).is_err());
    }
}
