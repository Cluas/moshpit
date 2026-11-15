//! A server's host key, asked about before a connection trusts it (the
//! SwiftUI card `hostKeyPromptCard` was, in `Moshpit/UI/Components.swift`):
//! NSH in its sheet, over Home or the terminal, whichever asked.
//!
//! Two kinds. A host never seen (`previous` absent): New Host, its SHA256
//! fingerprint and the command that prints it on the server, to compare
//! before trusting. A host whose key no longer matches the one stored:
//! Host Key Changed, the stored fingerprint and the one offered now.
//!
//! The page's words are the card's, key for key, so every translation
//! carries over. What changed is the frame: the sheet's bar has the way
//! out (Cancel, or Disconnect for a changed key) where every sheet has it,
//! and the one commitment (Trust, or Trust New Key, destructive) closes the
//! page, after the fingerprints it is about. The fingerprints and the
//! command are selectable, in mono. No key binding: Return trusts nothing,
//! here or anywhere (the card had none either). A swipe down is the host's
//! decline.
//!
//! The host posts the prompt ([`Message`]), `{"op": "hostKey", "host",
//! "port", "fingerprint", "previous", "olderAlgorithm"}`; the page tells it
//! `{"ev": "trust"}` or `{"ev": "decline"}`. A key that came by `ssh-rsa`
//! (`olderAlgorithm`, B564) is an RSA key: the command reads the RSA public
//! key, and a warning line says the algorithm is the older one.
//!
//! Identifiers: `host-key`, `host-key-list`, `host-key-message`,
//! `host-key-fingerprint`, `host-key-verify`, `host-key-older`,
//! `host-key-stored`, `host-key-offered`, `host-key-trust`,
//! `host-key-decline`.

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
        button::{Variant, button},
        icon::{IconSize, Tone, icon},
        page,
        row::PADDING_X,
        section::{Inset, section},
        tokens::token,
        typography::TextToken,
    },
};

/// What the host asks about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HostKey {
    pub host: String,
    pub port: u16,
    /// The key offered now, `SHA256:…`.
    pub fingerprint: String,
    /// The key stored for the host, when it no longer matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// The algorithm the key came by, when it is the older one (`ssh-rsa`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older_algorithm: Option<String>,
    /// An earlier version stored two different keys for the host, and the
    /// one offered now is one of them (B725): asked about, warned as a
    /// changed key is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflicting: bool,
    /// This phone connected to the host before, and its key isn't saved
    /// here any more (B783: a phone restored from a backup, a re-signed
    /// app): a new host's sheet says so instead of "First connection".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub connected_before: bool,
}

impl HostKey {
    /// Whether the stored key no longer matches.
    pub fn changed(&self) -> bool {
        self.previous.is_some()
    }

    /// Whether the sheet warns: a changed key, or two stored ones (B725).
    pub fn warns(&self) -> bool {
        self.changed() || self.conflicting
    }

    /// The command that prints the key's fingerprint on the server: the
    /// RSA public key's for a key that came by `ssh-rsa`, the ed25519
    /// one's otherwise. Two lines, as a shell takes them.
    pub fn verify_command(&self) -> &'static str {
        if self.older_algorithm.is_some() {
            "ssh-keygen -lf \\\n  /etc/ssh/ssh_host_rsa_key.pub"
        } else {
            "ssh-keygen -lf \\\n  /etc/ssh/ssh_host_ed25519_key.pub"
        }
    }
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "hostKey", …}`.
    HostKey(HostKey),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("host-key", json)?;
        match op.as_str() {
            "hostKey" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::HostKey)
                .map_err(|e| format!("host-key: hostKey: {e}")),
            other => Err(format!("host-key: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::HostKey(key) => {
                let mut value = serde_json::to_value(key).expect("a host key serializes");
                value["op"] = json!("hostKey");
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

/// The sheet's title.
pub fn title(key: &HostKey) -> String {
    if key.warns() {
        t!("Host Key Changed")
    } else {
        t!("New Host")
    }
}

/// What the page says first: what this key is, what to do with it.
pub fn message(key: &HostKey) -> String {
    let port = key.port.to_string();
    if key.conflicting {
        t!(
            "Moshpit saved two different keys for %@:%@ in an earlier version. The key it sent now is %@. Trust this key?",
            arg1 = key.host,
            arg2 = port,
            arg3 = key.fingerprint
        )
    } else if key.changed() {
        t!(
            "The key for %@:%@ does **not** match what's stored here. The server may have been reinstalled — or the connection is being intercepted.",
            arg1 = key.host,
            arg2 = port
        )
    } else if key.connected_before {
        t!(
            "You've connected to this server before, but its key isn't saved on this iPhone. Compare the fingerprint with the server before trusting it."
        )
    } else {
        t!(
            "First connection to %@:%@. Verify this fingerprint matches the server before you trust it.",
            arg1 = key.host,
            arg2 = port
        )
    }
}

/// `words` with the parts between `**` pairs as runs of their own, marked
/// (the catalog's emphasis, as SwiftUI's `Text` read it): `(run, strong)`.
/// An unpaired `**` is kept as it is.
pub fn emphasis(words: &str) -> Vec<(String, bool)> {
    let parts: Vec<&str> = words.split("**").collect();
    if parts.len().is_multiple_of(2) {
        return vec![(words.to_owned(), false)];
    }
    parts
        .into_iter()
        .enumerate()
        .filter(|(_, part)| !part.is_empty())
        .map(|(i, part)| (part.to_owned(), i % 2 == 1))
        .collect()
}

/// The host-key sheet.
#[component]
pub fn HostKeyScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let state = signal(None::<HostKey>);
    channel::hear(
        channel.as_ref(),
        Screen::HostKey,
        Message::parse,
        move |Message::HostKey(next)| {
            if state.with_untracked(|s| s.as_ref() != Some(&next)) {
                state.set(Some(next));
            }
        },
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let ready = computed(move || state.with(Option::is_some));
    let body = {
        let tell = tell.clone();
        move || {
            let tell = tell.clone();
            page::page(
                bar,
                scrolled,
                "host-key-list",
                [dynamic(move || {
                    let key = state.get();
                    let tell = tell.clone();
                    untracked(move || key.map(|key| content(&key, tell)))
                })
                .into_element()],
            )
            .into_element()
        }
    };
    let changed = computed(move || state.with(|s| s.as_ref().is_some_and(HostKey::changed)));
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("host-key")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(dynamic(move || {
            let changed = changed.get();
            let tell = tell.clone();
            untracked(move || sheet_bar(bar, scrolled, state, changed, tell))
        }))
}

/// The sheet's bar: its title, and the way out where every sheet has it
/// (Cancel; Disconnect for a changed key). Nothing trailing: the one
/// commitment is the page's last control, after what it is about.
fn sheet_bar(
    bar: BarMetrics,
    scrolled: nav_bar::ScrollEdge,
    state: Signal<Option<HostKey>>,
    changed: bool,
    tell: Tell,
) -> AnyElement {
    nav_bar::nav_bar(nav_bar::NavBar {
        metrics: bar,
        back_title: String::new(),
        title: Box::new(move || state.with(|s| s.as_ref().map(title).unwrap_or_default())),
        leading: Some(nav_bar::BarButton {
            label: if changed {
                t!("Disconnect")
            } else {
                t!("Cancel")
            },
            content: nav_bar::BarContent::Text,
            enabled: computed(|| true),
            emphasized: false,
            test_id: "host-key-decline",
            on_press: Box::new(move || tell(json!({"ev": "decline"}))),
            menu: None,
        }),
        trailing: Vec::new(),
        on_back: Box::new(|| {}),
        scrolled,
    })
    .into_element()
}

/// The message, the fingerprints (and the command, the warning) and the
/// commitment, 24 pt apart.
fn content(key: &HostKey, tell: Tell) -> AnyElement {
    let mut column = view()
        .gap(px(page::BETWEEN))
        .child(intro(key))
        .children(blocks(key));
    if let Some(older) = &key.older_algorithm
        && !key.changed()
    {
        column = column.child(older_line(older));
    }
    let trust = if key.changed() {
        button(t!("Trust New Key")).variant(Variant::Destructive)
    } else if key.conflicting {
        button(t!("Trust")).variant(Variant::Destructive)
    } else {
        button(t!("Trust")).variant(Variant::Primary)
    };
    column
        .child(
            trust
                .on_press(move || tell(json!({"ev": "trust"})))
                .test_id("host-key-trust"),
        )
        .into_element()
}

/// The kind's mark and what the page says first.
fn intro(key: &HostKey) -> AnyElement {
    let (symbol, tone) = if key.warns() {
        ("exclamationmark.triangle.fill", Tone::Destructive)
    } else {
        ("lock.shield.fill", Tone::Primary)
    };
    let mut words = TextToken::Body.apply(text(""));
    for (run, strong) in emphasis(&message(key)) {
        words = if strong {
            words.child(text(run).font_weight(FontWeight::BOLD))
        } else {
            words.child(run)
        };
    }
    view()
        .flex_row()
        .items_start()
        .gap(px(12.0))
        .px(px(PADDING_X))
        .child(icon(symbol).size(IconSize::Lg).tone(tone))
        .child(
            words
                .text_color(move || token("foreground"))
                .flex_1()
                .test_id("host-key-message"),
        )
        .into_element()
}

/// The two blocks: the fingerprint and the command for a new host, the
/// stored and the offered fingerprints for a changed one.
fn blocks(key: &HostKey) -> Vec<AnyElement> {
    if let Some(previous) = &key.previous {
        vec![
            block(t!("Stored"), previous.clone(), false, "host-key-stored"),
            block(
                t!("Offered now"),
                key.fingerprint.clone(),
                true,
                "host-key-offered",
            ),
        ]
    } else {
        vec![
            block(
                t!("SHA256 fingerprint"),
                key.fingerprint.clone(),
                true,
                "host-key-fingerprint",
            ),
            block(
                t!("Verify on server"),
                key.verify_command().to_owned(),
                false,
                "host-key-verify",
            ),
        ]
    }
}

/// A labeled block of machine text: selectable, in mono, in the accent when
/// it is the key being decided on.
fn block(label: String, words: String, accent: bool, id: &str) -> AnyElement {
    let ink = if accent { "primary-text" } else { "foreground" };
    section()
        .header(label)
        .item(
            view().px(px(PADDING_X)).py(px(12.0)).child(
                TextToken::MonoFootnote
                    .apply(text(words))
                    .text_color(move || token(ink))
                    .selectable(true)
                    .w_full()
                    .test_id(id),
            ),
            Inset::Content,
        )
        .into_element()
}

/// That the key came by the older algorithm (B564).
fn older_line(older: &str) -> AnyElement {
    view()
        .flex_row()
        .items_start()
        .gap(px(8.0))
        .px(px(PADDING_X))
        .test_id("host-key-older")
        .child(
            icon("exclamationmark.triangle.fill")
                .size(IconSize::Sm)
                .tone(Tone::Warning),
        )
        .child(
            TextToken::Footnote
                .apply(text(t!(
                    "This server uses an older host-key algorithm (%@, SHA-1).",
                    arg1 = older
                )))
                .text_color(move || token("warning-text"))
                .flex_1(),
        )
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

    fn new_host() -> HostKey {
        HostKey {
            host: "h".into(),
            port: 22,
            fingerprint: "SHA256:x".into(),
            previous: None,
            older_algorithm: None,
            conflicting: false,
            connected_before: false,
        }
    }

    #[test]
    fn a_message_round_trips() {
        let key = HostKey {
            previous: Some("SHA256:y".into()),
            older_algorithm: Some("ssh-rsa".into()),
            ..new_host()
        };
        let json = Message::HostKey(key.clone()).to_json();
        assert_eq!(Message::parse(&json), Ok(Message::HostKey(key)));
        assert_eq!(
            Message::parse(r#"{"op":"hostKey","host":"h","port":22,"fingerprint":"SHA256:x"}"#),
            Ok(Message::HostKey(new_host()))
        );
        assert!(Message::parse(r#"{"op":"hostKey","host":"h","port":22}"#).is_err());
        assert!(Message::parse(r#"{"op":"other"}"#).is_err());
    }

    #[test]
    fn the_command_reads_the_key_the_algorithm_brought() {
        assert_eq!(
            new_host().verify_command(),
            "ssh-keygen -lf \\\n  /etc/ssh/ssh_host_ed25519_key.pub"
        );
        let rsa = HostKey {
            older_algorithm: Some("ssh-rsa".into()),
            ..new_host()
        };
        assert_eq!(
            rsa.verify_command(),
            "ssh-keygen -lf \\\n  /etc/ssh/ssh_host_rsa_key.pub"
        );
    }

    #[test]
    fn emphasis_marks_the_runs_between_pairs() {
        assert_eq!(
            emphasis("does **not** match"),
            vec![
                ("does ".to_owned(), false),
                ("not".to_owned(), true),
                (" match".to_owned(), false)
            ]
        );
        assert_eq!(
            emphasis("**不一样**。"),
            vec![("不一样".to_owned(), true), ("。".to_owned(), false)]
        );
        assert_eq!(emphasis("plain"), vec![("plain".to_owned(), false)]);
        assert_eq!(emphasis("a ** b"), vec![("a ** b".to_owned(), false)]);
    }
}
