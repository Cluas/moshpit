//! Install Assist (`Moshpit/UI/Terminal/InstallAssistView.swift`) in NSH,
//! in its sheet: the banner's or the empty state's Install, when the host
//! lacks tmux, mosh or herdr. It shows the install command for the host's
//! package manager and never installs anything itself: Run in terminal
//! types the command into the live shell (sudo and its output in view),
//! Copy command copies it, Re-check probes the host again and, once it is
//! there, offers to reconnect with it.
//!
//! The host owns the session, the probe and the clipboard: it posts the
//! facts ([`Message`]), `{"op": "install", "herdr", "moshOnly", "manager",
//! "command", "note", "rechecking", "result", "copied"}`, again whenever one
//! changes; the page says them in the app's words and tells it
//! `{"ev": "done" | "run" | "copy" | "recheck" | "reconnect"}`.
//!
//! The same identifiers as the SwiftUI sheet (`install-command`,
//! `install-note`, `install-run`, `install-copy`, `install-recheck`,
//! `install-reconnect`, `install-reconnect-nochannel`).

use std::rc::Rc;

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    face::{self, Design},
    screen::Screen,
    ui::{
        feedback::{SpinnerSize, spinner},
        icon::{IconSize, Tone, icon},
        tokens::{number, token},
    },
};

/// What a Re-check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Recheck {
    /// Everything is there now: reconnect to use it.
    Resolved,
    StillMissing,
    /// The transport has no channel to probe over (reconnect to apply).
    NoChannel,
}

/// `{"op": "install", …}`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Install {
    /// herdr, which no distribution packages: its own installer.
    pub herdr: bool,
    /// Only mosh is missing (a herdr + mosh or a plain mosh connection).
    pub mosh_only: bool,
    /// A package manager was found on the host.
    pub manager: bool,
    /// The command to run, when there is one (left out when there is
    /// none, as the app's message always left it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// herdr's installer puts it in `~/.local/bin`, which Moshpit looks in.
    pub note: bool,
    pub rechecking: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Recheck>,
    pub copied: bool,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Install(Box<Install>),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("install-assist", json)?;
        match op.as_str() {
            "install" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(|i| Message::Install(Box::new(i)))
                .map_err(|e| format!("install-assist: install: {e}")),
            other => Err(format!("install-assist: no op {other:?}")),
        }
    }
}

/// What the sheet's first line says is missing.
pub fn intro(install: &Install) -> String {
    if install.herdr {
        // herdr's installer needs no package manager.
        t!("herdr isn't installed on this host.")
    } else if !install.manager {
        t!("Couldn't detect a package manager.")
    } else if install.mosh_only {
        t!("mosh isn't installed on this host.")
    } else {
        t!("tmux + mosh aren't installed on this host.")
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

type Tell = Rc<dyn Fn(serde_json::Value)>;

/// Install Assist.
#[component]
pub fn InstallAssistScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let state = signal(None::<Install>);
    channel::hear(
        channel.as_ref(),
        Screen::InstallAssist,
        Message::parse,
        move |Message::Install(next)| {
            if state.with_untracked(|s| s.as_ref() != Some(&*next)) {
                state.set(Some(*next));
            }
        },
    );
    let tell: Tell = Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    });
    let ready = computed(move || state.with(Option::is_some));
    let body = {
        let tell = tell.clone();
        move || {
            let tell = tell.clone();
            scroll()
                .flex_1()
                .test_id("install-list")
                .child(
                    view()
                        .w_full()
                        .px(px(number("space-page")))
                        .pt(px(14.0))
                        .pb(move || px(28.0 + safe_area().bottom.0))
                        .child(dynamic(move || {
                            let install = state.get().unwrap_or_default();
                            let tell = tell.clone();
                            untracked(move || content(install, tell))
                        })),
                )
                .into_element()
        }
    };
    view()
        .flex_1()
        .bg(move || token("background"))
        .test_id("install-assist")
        .child(header(tell.clone()))
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
}

/// The sheet's header: the title and Done on the bar's glass, a hairline
/// under them.
fn header(tell: Tell) -> AnyElement {
    view()
        .w_full()
        .bg(move || token("chrome"))
        .border_b(px(1.0))
        .border_color(move || token("border"))
        .child(
            view()
                .flex_row()
                .items_center()
                .px(px(18.0))
                // The hairline is in the box here, over it in SwiftUI.
                .pt(px(14.0))
                .pb(px(14.0 - 1.0))
                .child(
                    one_line(t!("Install on host"), 17.0, FontWeight::SEMIBOLD)
                        .text_color(move || token("foreground"))
                        .role(AccessibilityRole::Header)
                        .flex_shrink(1.0),
                )
                .child(view().flex_1().min_w(px(8.0)))
                .child(
                    one_line(t!("Done"), 15.0, FontWeight::SEMIBOLD)
                        .text_color(move || token("primary-text"))
                        .role(AccessibilityRole::Button)
                        .test_id("install-done")
                        .active(|s| s.opacity(0.4))
                        .on_press(move || tell(json!({"ev": "done"}))),
                ),
        )
        .into_element()
}

/// The intro, the command (or what to do without one) and the Re-check's
/// result, 18 pt apart.
fn content(install: Install, tell: Tell) -> AnyElement {
    let mut column = view().gap(px(18.0)).child(
        view()
            .gap(px(6.0))
            .child(
                paragraph(intro(&install), 14.0, FontWeight::SEMIBOLD)
                    .text_color(move || token("foreground")),
            )
            .child(
                paragraph(
                    t!("Moshpit never installs anything silently. Run the command below in your shell — sudo and its output stay fully visible."),
                    12.0,
                    FontWeight::NORMAL,
                )
                .text_color(move || token("muted-foreground")),
            ),
    );
    column = column.child(match &install.command {
        Some(command) => command_block(command, &install, tell.clone()),
        None => view()
            .gap(px(10.0))
            .child(
                paragraph(
                    // Only mosh missing: that connection never uses tmux.
                    if install.mosh_only {
                        t!("Install mosh with your platform's package manager (or build from source), then re-check.")
                    } else {
                        t!("Install tmux and mosh with your platform's package manager (or build from source), then re-check.")
                    },
                    12.0,
                    FontWeight::NORMAL,
                )
                .text_color(move || token("muted-foreground")),
            )
            .child(recheck_button(install.rechecking, tell.clone()))
            .into_element(),
    });
    if let Some(result) = install.result {
        column = column.child(result_row(result, tell));
    }
    column.into_element()
}

/// The command, the note, Run in terminal and Copy command, Re-check.
fn command_block(command: &str, install: &Install, tell: Tell) -> AnyElement {
    let mut block = view().gap(px(12.0)).child(
        face::paragraph(
            face::face(
                text(command.to_owned()),
                Design::Monospaced,
                12.0,
                FontWeight::NORMAL,
            ),
            12.0,
        )
        .text_color(move || token("foreground"))
        .selectable(true)
        .w_full()
        // The border is in the box here, over it in SwiftUI.
        .p(px(12.0 - 1.0))
        .bg(move || token("terminal-bg"))
        .rounded(px(number("radius-lg")))
        .border_curve(BorderCurve::Continuous)
        .border(px(1.0))
        .border_color(move || token("border"))
        .test_id("install-command"),
    );
    if install.note {
        block = block.child(
            paragraph(
                t!("Installs to ~/.local/bin. Moshpit looks there when probing and launching, so you don't need to change PATH."),
                11.0,
                FontWeight::NORMAL,
            )
            .text_color(move || token("muted-foreground"))
            .test_id("install-note"),
        );
    }
    let copied = install.copied;
    block
        .child(
            view()
                .flex_row()
                .gap(px(10.0))
                .child(action_button(
                    t!("Run in terminal"),
                    "terminal",
                    true,
                    "install-run",
                    {
                        let tell = tell.clone();
                        move || tell(json!({"ev": "run"}))
                    },
                ))
                .child(action_button(
                    if copied {
                        t!("Copied")
                    } else {
                        t!("Copy command")
                    },
                    if copied { "checkmark" } else { "doc.on.doc" },
                    false,
                    "install-copy",
                    {
                        let tell = tell.clone();
                        move || tell(json!({"ev": "copy"}))
                    },
                )),
        )
        .child(recheck_button(install.rechecking, tell))
        .into_element()
}

/// A half-width action: filled in the accent (Run), or its tint.
fn action_button(
    title: String,
    symbol: &'static str,
    filled: bool,
    test_id: &'static str,
    on_press: impl Fn() + 'static,
) -> AnyElement {
    let (fill, words, edge, tone) = if filled {
        (
            "primary",
            "primary-foreground",
            "primary-active",
            Tone::OnPrimary,
        )
    } else {
        (
            "primary-soft",
            "primary-text",
            "primary-soft",
            Tone::Primary,
        )
    };
    view()
        .flex_1()
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        // The border is in the box here, over it in SwiftUI.
        .py(px(11.0 - 1.0))
        .rounded(px(number("radius")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || token(fill))
        .border(px(1.0))
        .border_color(move || token(edge))
        .role(AccessibilityRole::Button)
        .label(title.clone())
        .test_id(test_id)
        .active(|s| s.opacity(0.6))
        .on_press(on_press)
        .child(icon(symbol).size(IconSize::Sm).tone(tone))
        .child(
            one_line(title, 13.0, FontWeight::SEMIBOLD)
                .text_color(move || token(words))
                .accessibility_elements_hidden(true),
        )
        .into_element()
}

/// Re-check: the spinner and Re-checking… while it runs (and it can't be
/// pressed again).
fn recheck_button(rechecking: bool, tell: Tell) -> AnyElement {
    let title = if rechecking {
        t!("Re-checking…")
    } else {
        t!("Re-check")
    };
    let mark = if rechecking {
        spinner(SpinnerSize::Sm)
    } else {
        icon("arrow.clockwise")
            .size(IconSize::Sm)
            .tone(Tone::Primary)
            .into_element()
    };
    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .py(px(10.0 - 1.0))
        .rounded(px(number("radius")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || token("primary-soft"))
        .border(px(1.0))
        .border_color(move || token("primary-soft"))
        .role(AccessibilityRole::Button)
        .label(title.clone())
        .accessibility_state(AccessibilityState {
            disabled: rechecking,
            ..AccessibilityState::default()
        })
        .disabled(rechecking)
        .test_id("install-recheck")
        .active(|s| s.opacity(0.6))
        .on_press(move || tell(json!({"ev": "recheck"})))
        .child(mark)
        .child(
            one_line(title, 13.0, FontWeight::SEMIBOLD)
                .text_color(move || token("primary-text"))
                .accessibility_elements_hidden(true),
        )
        .into_element()
}

/// What the Re-check found, and Reconnect where reconnecting helps.
fn result_row(result: Recheck, tell: Tell) -> AnyElement {
    match result {
        Recheck::Resolved => view()
            .gap(px(10.0))
            .child(label(
                "checkmark.circle.fill",
                Tone::Success,
                paragraph(
                    t!("Installed — reconnect to enable it."),
                    12.0,
                    FontWeight::SEMIBOLD,
                )
                .text_color(move || token("foreground")),
                12.0,
            ))
            .child(reconnect("install-reconnect", tell))
            .into_element(),
        Recheck::StillMissing => label(
            "exclamationmark.circle.fill",
            Tone::Warning,
            paragraph(
                t!("Still not detected. Run the command, wait for it to finish, then re-check."),
                12.0,
                FontWeight::NORMAL,
            )
            .text_color(move || token("muted-foreground")),
            12.0,
        )
        .into_element(),
        Recheck::NoChannel => view()
            .gap(px(10.0))
            .child(label(
                "wifi.slash",
                Tone::Warning,
                paragraph(
                    t!("Can't re-check over this transport — reconnect to apply."),
                    12.0,
                    FontWeight::NORMAL,
                )
                .text_color(move || token("muted-foreground")),
                12.0,
            ))
            .child(reconnect("install-reconnect-nochannel", tell))
            .into_element(),
    }
}

/// Reconnect: filled in the accent, the card's rounding.
fn reconnect(test_id: &'static str, tell: Tell) -> AnyElement {
    view()
        .w_full()
        .items_center()
        .py(px(11.0))
        .rounded(px(number("radius-lg")))
        .border_curve(BorderCurve::Continuous)
        .bg(move || token("primary"))
        .role(AccessibilityRole::Button)
        .label(t!("Reconnect"))
        .test_id(test_id)
        .active(|s| s.opacity(0.6))
        .on_press(move || tell(json!({"ev": "reconnect"})))
        .child(
            one_line(t!("Reconnect"), 14.0, FontWeight::SEMIBOLD)
                .text_color(move || token("primary-foreground"))
                .accessibility_elements_hidden(true),
        )
        .into_element()
}

/// A `Label`: its symbol in `tone` at a row's size, the text beside it, the
/// symbol centered on the text's first line (design size `first`).
fn label(symbol: &'static str, tone: Tone, content: Text, first: f32) -> View {
    let drawn = signal(0.0_f32);
    let line = move || face::frame(face::scaled(first, font_scale()), pixel_ratio());
    view()
        .flex_row()
        .items_start()
        .gap(px(LABEL_SPACING))
        .child(
            view()
                .flex_shrink_0()
                .mt(move || px(((line() - drawn.get()) / 2.0).max(0.0)))
                .on_layout(move |bounds: &Bounds<Points>| {
                    let height = bounds.size.height.0;
                    if (drawn.get_untracked() - height).abs() > 0.01 {
                        drawn.set(height);
                    }
                })
                .child(icon(symbol).size(IconSize::Md).tone(tone)),
        )
        .child(content.flex_1().flex_shrink(1.0))
}

/// The room between a `Label`'s symbol and its title (measured on the
/// iPad's Host Setup page).
const LABEL_SPACING: f32 = 7.0;

/// `Face.text(size, weight)` on one line.
fn one_line(content: String, size: f32, weight: FontWeight) -> Text {
    face::one_line(
        face::face(text(content), Design::Default, size, weight),
        size,
    )
}

/// `Face.text(size, weight)` that wraps, never after a word's hyphen
/// ("re-check" stays whole, as UIKit keeps it).
fn paragraph(content: String, size: f32, weight: FontWeight) -> Text {
    face::paragraph(
        face::face(
            text(no_hyphen_breaks(&content)),
            Design::Default,
            size,
            weight,
        ),
        size,
    )
}

/// `text` with a word joiner after each hyphen inside a word.
pub fn no_hyphen_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '-' && chars.peek().is_some_and(|n| n.is_alphanumeric()) {
            out.push('\u{2060}');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hyphenated_word_stays_whole() {
        assert_eq!(no_hyphen_breaks("then re-check."), "then re-\u{2060}check.");
        assert_eq!(no_hyphen_breaks("a - b"), "a - b");
    }

    #[test]
    fn messages_parse_and_bad_ones_are_refused() {
        let message = Message::parse(r#"{"op":"install","herdr":false,"mosh_only":false}"#);
        assert!(message.is_err(), "camelCase fields");
        let Ok(Message::Install(install)) = Message::parse(
            r#"{"op":"install","moshOnly":true,"manager":true,"command":"brew install mosh","result":"stillMissing"}"#,
        ) else {
            panic!()
        };
        assert!(install.mosh_only && install.manager);
        assert_eq!(install.result, Some(Recheck::StillMissing));
        assert!(Message::parse(r#"{"op":"install","result":"maybe"}"#).is_err());
        assert!(Message::parse(r#"{"op":"run"}"#).is_err());
    }
}
