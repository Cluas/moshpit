//! Settings › Feedback (`Moshpit/UI/Settings/FeedbackView.swift`) in NSH: a
//! note to support, composed here and sent through the user's own Mail.
//!
//! Sending touches the system (the Mail compose sheet, a `mailto:` handler,
//! the pasteboard), so it stays the host's. The host posts what the page
//! shows ([`Message`]), `{"op": "feedback", …}` ([`FeedbackState`]): the two
//! switches, whether a send is being prepared (Send waits), whether Mail is
//! missing (the address to copy shows) and whether it was copied; the first
//! post also carries the message typed so far, so a surface made again
//! starts from it. The page tells it:
//!
//! - `{"ev": "text", "value": …}` as the message changes (the host keeps
//!   it);
//! - `{"ev": "set", "key": "environment" | "log", "value": …}` for the
//!   switches;
//! - `{"ev": "send", "message": …}`, `{"ev": "cancel"}`,
//!   `{"ev": "copy-address"}` and `{"ev": "github-issue"}`.
//!
//! The page is the redesign's (`docs/nsh/UI-DESIGN.md` §6.12, R16): the
//! message in a stacked field that grows from four lines to twelve (the
//! keyboard in it as the page opens), the two switches and what the log
//! is, an `info` banner with Copy address and the address in mono while
//! Mail is missing, and Open an issue on GitHub (`{"ev": "github-issue"}`:
//! the host opens the page in the browser; the arrow says it leaves the
//! app). Cancel and Send in the bar; Send waits while the mail is
//! prepared. The same accessibility identifiers (`feedback-message`,
//! `-environment`, `-log`, `-send`, `-cancel`, `-copy-address`), and
//! `feedback-github-issue`, `feedback-no-mail`, `feedback-address`.

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
        control::switch,
        field::{FieldLayout, field, text_input},
        icon::{IconSize, Tone, icon},
        notice::{BannerTone, banner},
        page,
        row::{PADDING_X, RowTone, row},
        section::{Inset, Section, section},
        tokens::token,
        typography::TextToken,
    },
};

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FeedbackState {
    /// The message typed so far: only when the page should take it (its
    /// first post), not as the user types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Include the device and app facts.
    pub environment: bool,
    /// Attach the recent log.
    pub log: bool,
    /// A send is being prepared: Send waits.
    pub preparing: bool,
    /// Neither Mail nor a `mailto:` handler took it: show the address.
    pub show_address: bool,
    /// The address was copied.
    pub copied: bool,
    /// Where feedback goes (`FeedbackMail.recipient`).
    pub recipient: String,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "feedback", …}`: what the page shows now.
    Feedback(FeedbackState),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("feedback", json)?;
        match op.as_str() {
            "feedback" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Feedback)
                .map_err(|e| format!("feedback: feedback: {e}")),
            other => Err(format!("feedback: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Feedback(state) => {
                let mut value = serde_json::to_value(state).expect("a state serializes");
                value["op"] = json!("feedback");
                value.to_string()
            }
        }
    }
}

/// What the page tells the host (`{"ev": …}`). The one spelling of it: the
/// page tells through it, and the host reads it back typed (`crates/ffi`'s
/// `pages`, E6b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "kebab-case")]
pub enum FeedbackEvent {
    /// Cancel: the sheet goes.
    Cancel,
    /// The message as typed so far.
    Text { value: String },
    /// One of the two switches flipped.
    Set { key: FeedbackSwitch, value: bool },
    /// Send, with the message as typed.
    Send { message: String },
    /// Copy Address.
    CopyAddress,
    /// Open an issue on GitHub.
    GithubIssue,
}

/// The page's two switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackSwitch {
    /// Include the device and app facts.
    Environment,
    /// Attach the recent log.
    Log,
}

impl FeedbackEvent {
    /// The event as the page tells it.
    pub fn json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("an event serializes")
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// The message field's least and most lines (`.lineLimit(4...12)`).
const LINES: (u32, u32) = (4, 12);

/// Settings › Send feedback (§6.12).
#[component]
pub fn FeedbackScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let state = signal(None::<FeedbackState>);
    // The text the field starts with: the host's, from its first post.
    let initial = signal(None::<String>);
    let message = signal(String::new());
    channel::hear(
        channel.as_ref(),
        Screen::Feedback,
        Message::parse,
        move |Message::Feedback(next)| {
            if let Some(text) = &next.message
                && initial.get_untracked().is_none()
            {
                message.set(text.clone());
                initial.set(Some(text.clone()));
            }
            state.set(Some(next));
        },
    );
    let read = move |f: fn(&FeedbackState) -> bool| state.with(|s| s.as_ref().is_some_and(f));
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let ready = computed(move || state.with(Option::is_some));
    let send = {
        let tell = tell.clone();
        move || {
            // The keyboard goes down while the mail is prepared, as it does
            // on the SwiftUI page.
            dismiss_keyboard();
            tell(
                FeedbackEvent::Send {
                    message: message.get_untracked(),
                }
                .json(),
            );
        }
    };
    let cancel = {
        let tell = tell.clone();
        move || tell(FeedbackEvent::Cancel.json())
    };
    let preparing = computed(move || read(|s| s.preparing));
    let body = {
        let tell = tell.clone();
        move || {
            let tell = tell.clone();
            page::page(
                bar,
                scrolled,
                "feedback-list",
                [
                    message_field(
                        initial.get_untracked().unwrap_or_default(),
                        message,
                        tell.clone(),
                    )
                    .into_element(),
                    include(read, tell.clone()).into_element(),
                    when(move || read(|s| s.show_address), {
                        let tell = tell.clone();
                        move || address(state, tell.clone())
                    })
                    .into_element(),
                    issue(tell).into_element(),
                ],
            )
            .into_element()
        }
    };
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("feedback")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: String::new(),
            title: nav_bar::fixed(t!("Send feedback")),
            leading: Some(nav_bar::BarButton {
                label: t!("Cancel"),
                content: nav_bar::BarContent::Close,
                enabled: computed(|| true),
                emphasized: false,
                test_id: "feedback-cancel",
                on_press: Box::new(cancel),
                menu: None,
            }),
            // While the mail is prepared, Send holds the spinner.
            trailing: vec![
                nav_bar::BarButton {
                    label: t!("Send"),
                    content: nav_bar::BarContent::Text,
                    enabled: computed(|| true),
                    emphasized: true,
                    test_id: "feedback-send",
                    on_press: Box::new(send),
                    menu: None,
                }
                .busy(preparing),
            ],
            on_back: Box::new(|| {}),
            scrolled,
        }))
}

/// The message: a stacked field whose text grows from four lines to
/// twelve, the keyboard in it as the page opens.
fn message_field(start: String, message: Signal<String>, tell: Tell) -> AnyElement {
    section()
        .item(
            field(
                t!("Message"),
                text_input()
                    .multiline(LINES.0, LINES.1)
                    .default_value(move || start.clone())
                    .placeholder(t!("What broke, or what you want"))
                    .auto_focus()
                    .on_change(move |text| {
                        tell(
                            FeedbackEvent::Text {
                                value: text.clone(),
                            }
                            .json(),
                        );
                        message.set(text);
                    }),
            )
            .layout(FieldLayout::Stacked)
            .test_id("feedback-message"),
            Inset::Content,
        )
        .into_element()
}

/// The two switches and what the log is.
fn include(
    read: impl Fn(fn(&FeedbackState) -> bool) -> bool + Copy + 'static,
    tell: Tell,
) -> Section {
    let set = |key: FeedbackSwitch| {
        let tell = tell.clone();
        move |on: bool| tell(FeedbackEvent::Set { key, value: on }.json())
    };
    section()
        .row(
            row(t!("Include device and version")).trailing(
                switch(
                    move || read(|s| s.environment),
                    set(FeedbackSwitch::Environment),
                )
                .label(t!("Include device and version"))
                .test_id("feedback-environment"),
            ),
        )
        .row(
            row(t!("Include recent log")).trailing(
                switch(move || read(|s| s.log), set(FeedbackSwitch::Log))
                    .label(t!("Include recent log"))
                    .test_id("feedback-log"),
            ),
        )
        .footer(t!(
            "The log is the last 30 minutes, the same lines as Recent log."
        ))
}

/// Mail is missing: an `info` banner with Copy address (Copied once it
/// is), and the address in mono under it.
fn address(state: Signal<Option<FeedbackState>>, tell: Tell) -> AnyElement {
    let copied = computed(move || state.with(|s| s.as_ref().is_some_and(|s| s.copied)));
    let recipient =
        move || state.with(|s| s.as_ref().map(|s| s.recipient.clone()).unwrap_or_default());
    view()
        .w_full()
        .gap(px(8.0))
        .child(dynamic(move || {
            let copied = copied.get();
            let tell = tell.clone();
            untracked(move || {
                banner(BannerTone::Info, || {
                    t!("Mail isn't set up on this device").into()
                })
                .icon("envelope")
                .action(
                    if copied {
                        t!("Copied")
                    } else {
                        t!("Copy address")
                    },
                    move || tell(FeedbackEvent::CopyAddress.json()),
                )
                .action_test_id("feedback-copy-address")
                .test_id("feedback-no-mail")
            })
        }))
        .child(
            view().px(px(PADDING_X)).child(
                TextToken::MonoFootnote
                    .apply(text(recipient))
                    .text_color(move || token("muted-foreground"))
                    .number_of_lines(1u32)
                    .test_id("feedback-address"),
            ),
        )
        .into_element()
}

/// Open an issue on GitHub: a web page, so it says it leaves the app.
fn issue(tell: Tell) -> Section {
    section()
        .row(
            row(t!("Open an issue on GitHub"))
                .tone(RowTone::Primary)
                .trailing(
                    icon("arrow.up.right")
                        .size(IconSize::Sm)
                        .tone(Tone::Primary),
                )
                .on_press(move || tell(FeedbackEvent::GithubIssue.json()))
                .test_id("feedback-github-issue"),
        )
        .footer(t!("Issues are public; keep security reports to mail."))
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

    fn state() -> FeedbackState {
        FeedbackState {
            message: None,
            environment: true,
            log: false,
            preparing: false,
            show_address: false,
            copied: false,
            recipient: "support@cluas.eu.org".into(),
        }
    }

    #[test]
    fn messages_round_trip() {
        let message = Message::Feedback(state());
        assert_eq!(Message::parse(&message.to_json()), Ok(message));
        let message = Message::Feedback(FeedbackState {
            message: Some("It froze".into()),
            show_address: true,
            copied: true,
            ..state()
        });
        assert_eq!(Message::parse(&message.to_json()), Ok(message));
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        for (json, says) in [
            ("{", "not JSON"),
            (r#"{"op":"settings"}"#, "no op \"settings\""),
            (
                r#"{"op":"feedback","environment":true,"log":false,"preparing":false,"showAddress":false,"copied":false}"#,
                "recipient",
            ),
            (
                r#"{"op":"feedback","environment":"yes","log":false,"preparing":false,"showAddress":false,"copied":false,"recipient":"a@b"}"#,
                "yes",
            ),
        ] {
            let error = Message::parse(json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }
}
