//! Settings › Recent Log (`Moshpit/UI/Settings/DiagnosticsLogView.swift`)
//! in NSH: what the app logged in the last 30 minutes, newest first, so a
//! bug seen on the phone can be read (or screenshotted, or copied) there.
//! Display only.
//!
//! The host reads the log as the SwiftUI page does (`DiagnosticsLog.recent()`:
//! the process's `OSLogStore` and the push extension's trail) and posts what
//! the page shows ([`Message`]): `{"op": "log", "state": "loading"}`, then
//! `"loaded"` with the entries, or `"failed"` with the message to show. The
//! times come formatted as the SwiftUI page formats them (the host's
//! `DiagnosticsLog.clock`). The page tells the host `{"ev": "back"}` when the
//! user goes back and `{"ev": "reload"}` to read again; Copy puts the
//! entries on the clipboard itself, one line each as `DiagnosticsLog.line`
//! writes them ([`plain_text`]).
//!
//! The page is the redesign's (`docs/nsh/UI-DESIGN.md` §6.12, R16): an
//! entry per row, its time (mono, muted) and its category's badge over its
//! message in mono, four lines until a press opens it whole (screen readers
//! hear it expanded or collapsed), the newest [`SHOWN`] of them (a footer
//! says so when there are more; Copy takes them all); six skeleton rows
//! while the host reads;
//! an empty page when nothing was logged; when the read failed, the host's
//! words and Retry (`{"ev": "reload"}`: the host reads again; `retry` is
//! the error boundary's, which runs the screen again). Copy in the
//! bar, and a toast saying how many lines it copied. The same
//! accessibility identifiers (`recent-log-…`), and `recent-log-retry`,
//! `recent-log-toast`.

use std::{rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    boundary::Tell,
    channel::{self, Channel},
    nav_bar,
    screen::Screen,
    ui::{
        button::{Variant, button},
        empty::{empty, skeleton_list},
        feedback::{separator, touch_target},
        motion::Duration,
        notice::toasts,
        page,
        row::{PADDING_X, PADDING_Y},
        section::{Inset, section},
        status::badge,
        tokens::token,
        typography::TextToken,
    },
};

/// One entry, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// When, formatted by the host (`HH:mm:ss.SSS`).
    pub time: String,
    /// The log category (`layout`, `health`, the push extension's source).
    pub category: String,
    pub message: String,
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum LogState {
    /// The host is reading the log.
    Loading,
    /// It could not: `message` is what to show.
    Failed { message: String },
    /// The entries, newest first (none: nothing was logged).
    Loaded { entries: Vec<Entry> },
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "log", "state": …}`: what the page shows now.
    Log(LogState),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("recent-log: not JSON: {e}"))?;
        let Some(mut object) = value.as_object().cloned() else {
            return Err("recent-log: a message is a JSON object".to_owned());
        };
        let op = object.remove("op");
        match op.as_ref().and_then(|op| op.as_str()) {
            Some("log") => serde_json::from_value(serde_json::Value::Object(object))
                .map(Message::Log)
                .map_err(|e| format!("recent-log: log: {e}")),
            Some(other) => Err(format!("recent-log: no op {other:?}")),
            None => Err("recent-log: a message has a string \"op\"".to_owned()),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Log(state) => {
                let mut value = serde_json::to_value(state).expect("a state serializes");
                value["op"] = json!("log");
                value.to_string()
            }
        }
    }
}

/// One entry's line, as `DiagnosticsLog.line` writes it.
pub fn line(entry: &Entry) -> String {
    format!("{}  {}  {}", entry.time, entry.category, entry.message)
}

/// What Copy puts on the clipboard: a line per entry, in the order shown.
pub fn plain_text(entries: &[Entry]) -> String {
    entries.iter().map(line).collect::<Vec<_>>().join("\n")
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

/// How many lines a message shows until it is pressed open.
pub const MESSAGE_LINES: u32 = 4;

/// The skeleton rows while the host reads the log.
const LOADING_ROWS: usize = 6;

/// How many entries the page draws, newest first: a plain list, not a
/// virtual one, holds this many without a stall (Android keeps up to
/// 2000). Copy still takes every entry.
pub const SHOWN: usize = 500;

/// Settings › Recent log (§6.12).
#[component]
pub fn RecentLogScreen(props: ScreenProps) -> impl IntoElement {
    let state = signal(LogState::Loading);
    let channel = Channel::find(props.surface.get_untracked());
    channel::hear(
        channel.as_ref(),
        Screen::RecentLog,
        Message::parse,
        move |message| match message {
            Message::Log(next) => state.set(next),
        },
    );
    let bar = nav_bar::BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let has_entries = computed(move || {
        state.with(|state| matches!(state, LogState::Loaded { entries } if !entries.is_empty()))
    });
    let back = {
        let tell = tell.clone();
        move || tell(json!({"ev": "back"}))
    };
    let toasts = toasts();
    let copy = move || {
        let count = state.with_untracked(|state| match state {
            LogState::Loaded { entries } if !entries.is_empty() => {
                set_clipboard(plain_text(entries));
                entries.len()
            }
            _ => 0,
        });
        if count > 0 {
            toasts.show(crate::plural!("Copied %lld lines", count as i64));
        }
    };
    let reload: Rc<dyn Fn()> = {
        let tell = tell.clone();
        Rc::new(move || tell(json!({"ev": "reload"})))
    };
    // From the start: the page has its own loading state, shown until the
    // host posts. Behind B366's boundary all the same.
    let body = move || {
        let reload = reload.clone();
        page::page(
            bar,
            scrolled,
            "recent-log-list",
            [dynamic(move || {
                let shown = state.get();
                let reload = reload.clone();
                untracked(move || content(shown, reload))
            })
            .into_element()],
        )
        .into_element()
    };
    let failed = tell.clone();
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("recent-log")
        .child(crate::boundary::guarded(body, failed))
        .child(nav_bar::nav_bar(nav_bar::NavBar {
            metrics: bar,
            back_title: t!("Settings"),
            title: nav_bar::fixed(t!("Recent log")),
            leading: None,
            trailing: vec![nav_bar::BarButton {
                label: t!("Copy"),
                content: nav_bar::BarContent::Text,
                enabled: has_entries,
                emphasized: false,
                test_id: "recent-log-copy",
                on_press: Box::new(copy),
                menu: None,
            }],
            on_back: Box::new(back),
            scrolled,
        }))
        .child(toasts.host().test_id("recent-log-toast"))
}

/// What the page shows under its bar, by the host's state.
fn content(state: LogState, reload: Rc<dyn Fn()>) -> AnyElement {
    match state {
        LogState::Loading => section()
            .item(
                skeleton_list(LOADING_ROWS)
                    .subtitle(true)
                    .test_id("recent-log-loading"),
                Inset::Content,
            )
            .into_element(),
        LogState::Failed { message } => empty(t!("Couldn't read the log"))
            .icon("exclamationmark.triangle")
            .message(message)
            .action(
                button(t!("Retry"))
                    .variant(Variant::Secondary)
                    .on_press(move || reload())
                    .test_id("recent-log-retry"),
            )
            .test_id("recent-log-error")
            .into_element(),
        LogState::Loaded { entries } if entries.is_empty() => {
            empty(t!("Nothing logged in the last 30 minutes"))
                .icon("doc.text")
                .test_id("recent-log-empty")
                .into_element()
        }
        LogState::Loaded { entries } => {
            let total = entries.len();
            let mut list = view().w_full();
            for (index, one) in entries.into_iter().take(SHOWN).enumerate() {
                if index > 0 {
                    list = list.child(separator(Inset::Content.points()));
                }
                list = list.child(entry(one));
            }
            let mut group = section()
                .item(list, Inset::Content)
                .test_id("recent-log-entries");
            if total > SHOWN {
                group = group.footer(t!(
                    "Showing the latest 500 of %lld. Copy takes them all.",
                    arg1 = total
                ));
            }
            group.into_element()
        }
    }
}

/// One entry: its time (mono, muted) and its category's badge, the
/// message under them in mono, four lines until a press opens it whole.
fn entry(entry: Entry) -> impl IntoElement {
    let Entry {
        time,
        category,
        message,
    } = entry;
    let open = signal(false);
    let spoken = format!("{time}, {category}, {message}");
    view()
        .w_full()
        .min_h(px(touch_target()))
        .px(px(PADDING_X))
        .py(px(PADDING_Y))
        .gap(px(4.0))
        .role(AccessibilityRole::Button)
        .label(spoken)
        .accessibility_state(move || AccessibilityState {
            expanded: Some(open.get()),
            ..AccessibilityState::default()
        })
        .test_id("recent-log-entry")
        .transition_colors()
        .duration(Duration::Instant.ms())
        .active(|s| s.bg(move || token("accent")))
        .on_press(move || open.update(|o| *o = !*o))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    TextToken::MonoFootnote
                        .apply(text(time))
                        .text_color(move || token("muted-foreground"))
                        .tabular_nums()
                        .number_of_lines(1u32),
                )
                .child(badge(category).mono()),
        )
        .child(
            TextToken::MonoFootnote
                .apply(text(message))
                .text_color(move || token("foreground"))
                .number_of_lines(move || if open.get() { 0u32 } else { MESSAGE_LINES }),
        )
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

    fn entry(time: &str, category: &str, message: &str) -> Entry {
        Entry {
            time: time.into(),
            category: category.into(),
            message: message.into(),
        }
    }

    #[test]
    fn messages_round_trip() {
        for state in [
            LogState::Loading,
            LogState::Failed {
                message: "Couldn't read the log: denied".into(),
            },
            LogState::Loaded { entries: vec![] },
            LogState::Loaded {
                entries: vec![entry("12:00:01.250", "layout", "resize 80x24 → 70x40")],
            },
        ] {
            let message = Message::Log(state);
            assert_eq!(Message::parse(&message.to_json()), Ok(message));
        }
    }

    #[test]
    fn the_host_messages_as_written() {
        assert_eq!(
            Message::parse(r#"{"op":"log","state":"loading"}"#),
            Ok(Message::Log(LogState::Loading))
        );
        assert_eq!(
            Message::parse(
                r#"{"state":"loaded","op":"log","entries":[{"time":"09:41:00.000","category":"health","message":"redial: path changed"}]}"#
            ),
            Ok(Message::Log(LogState::Loaded {
                entries: vec![entry("09:41:00.000", "health", "redial: path changed")]
            }))
        );
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        for (json, says) in [
            ("{", "not JSON"),
            ("[]", "JSON object"),
            (r#"{"state":"loading"}"#, "\"op\""),
            (r#"{"op":"focus"}"#, "no op \"focus\""),
            (r#"{"op":"log","state":"done"}"#, "log:"),
            (r#"{"op":"log","state":"failed"}"#, "message"),
            (
                r#"{"op":"log","state":"loaded","entries":[{"time":"1","category":"c"}]}"#,
                "message",
            ),
        ] {
            let error = Message::parse(json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }

    #[test]
    fn copy_writes_a_line_per_entry_as_the_swift_page_does() {
        let entries = [
            entry("12:00:02.000", "health", "redial"),
            entry("12:00:01.000", "push-extension", "decrypted"),
        ];
        assert_eq!(
            plain_text(&entries),
            "12:00:02.000  health  redial\n12:00:01.000  push-extension  decrypted"
        );
        assert_eq!(plain_text(&[]), "");
    }
}
