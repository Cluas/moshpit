//! A Markdown file of the host, shown as a document rather than its source
//! (QuickLook shows a `.md` as plain text): a path link or the folder sheet
//! opens it here once its copy is on the phone.
//!
//! The host reads the file and posts it, `{"op": "markdown", "name",
//! "text"}` (`MarkdownPreviewNSHPage.swift`); NSH's `TextView` lays it out
//! (CommonMark and GitHub's tables, task lists, alerts, footnotes), in the
//! app's tokens and the body's text size. The page tells the host `{"ev":
//! "close"}`, `{"ev": "share"}` (the file itself, to another app) and
//! `{"ev": "link", "url"}` for a link pressed in the document (the host
//! opens a web or mail link; a link within the document, `#…`, scrolls
//! nowhere yet).
//!
//! Identifiers: `markdown-preview`, `markdown-preview-list`,
//! `markdown-preview-document`, `markdown-preview-close`,
//! `markdown-preview-share`.

use std::{rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    boundary::Tell,
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    screen::Screen,
    ui::{document::document, page, tokens::token},
};

/// What the host says: the file's name and its text.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct MarkdownFile {
    /// The file's name, the page's title.
    pub name: String,
    /// The file's Markdown.
    pub text: String,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "markdown", …}`.
    Markdown(MarkdownFile),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("markdown-preview", json)?;
        match op.as_str() {
            "markdown" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Markdown)
                .map_err(|e| format!("markdown-preview: markdown: {e}")),
            other => Err(format!("markdown-preview: no op {other:?}")),
        }
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// A Markdown file, laid out.
#[component]
pub fn MarkdownPreviewScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let state = signal(None::<MarkdownFile>);
    channel::hear(
        channel.as_ref(),
        Screen::MarkdownPreview,
        Message::parse,
        move |Message::Markdown(next)| state.set(Some(next)),
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let ready = computed(move || state.with(Option::is_some));
    let title = move || state.with(|s| s.as_ref().map(|s| s.name.clone()).unwrap_or_default());
    let body =
        {
            let tell = tell.clone();
            move || {
                let tell = tell.clone();
                page::page(
                    bar,
                    scrolled,
                    "markdown-preview-list",
                    [view()
                        .w_full()
                        .test_id("markdown-preview-document")
                        .child(document(
                            move || {
                                SharedString::from(state.with(|s| {
                                    s.as_ref().map(|s| s.text.clone()).unwrap_or_default()
                                }))
                            },
                            move |url| tell(json!({"ev": "link", "url": url})),
                        ))
                        .into_element()],
                )
                .into_element()
            }
        };
    let close = {
        let tell = tell.clone();
        move || tell(json!({"ev": "close"}))
    };
    let share = {
        let tell = tell.clone();
        move || tell(json!({"ev": "share"}))
    };
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("markdown-preview")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: String::new(),
            title: Box::new(title),
            leading: Some(nav_bar::BarButton {
                label: t!("Done"),
                content: nav_bar::BarContent::Close,
                enabled: computed(|| true),
                emphasized: false,
                test_id: "markdown-preview-close",
                on_press: Box::new(close),
                menu: None,
            }),
            trailing: vec![nav_bar::BarButton {
                label: t!("Share"),
                content: nav_bar::BarContent::Symbol("square.and.arrow.up"),
                enabled: ready,
                emphasized: false,
                test_id: "markdown-preview-share",
                on_press: Box::new(share),
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
            Message::parse(r##"{"op":"markdown","name":"README.md","text":"# Hi"}"##),
            Ok(Message::Markdown(MarkdownFile {
                name: "README.md".into(),
                text: "# Hi".into()
            }))
        );
        assert!(Message::parse(r#"{"op":"markdown","path":"/x"}"#).is_err());
        assert!(Message::parse(r#"{"op":"folder"}"#).is_err());
    }
}
