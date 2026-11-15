//! Settings › SSH Keys (`docs/nsh/UI-DESIGN.md` §6.6) in NSH, in its
//! sheet: the keys this device holds, the Secure Enclave's (the Android
//! Keystore's) on top and the imported or generated ones below, each with
//! its badge, its SHA256 fingerprint and the hosts it is used for.
//!
//! Key material never comes here: the host owns the keys (`SSHKeyStore`,
//! the Keychain, the Secure Enclave) and posts their metadata ([`Message`]),
//! `{"op": "keys", "device": [Key…], "imported": [Key…], "deleting": id}`,
//! again whenever it changes. The page tells it what the user asks for:
//!
//! - `{"ev": "done"}` and `{"ev": "add"}` (the bar's Done and +);
//!   `{"ev": "add", "method": "generate" | "import"}` from an empty
//!   group's button (Add Key opens on that method);
//! - from a key's action sheet (a press on its row, or a long press):
//!   `{"ev": "copy-public-key" | "copy-fingerprint" | "share-public-key" |
//!   "confirm-delete", "id": …}`; the host copies or shares what only it
//!   has (the page says a copy in a toast), and for a delete checks the key
//!   can go and names it (`deleting`);
//! - from the system's alert asking about it: `{"ev": "delete", "id": …}`
//!   or `{"ev": "cancel-delete"}`;
//! - from the alert saying why a key the user asked to delete wasn't
//!   (`notice`: connections still sign in with it, or the keychain kept
//!   it; the host words it): `{"ev": "dismiss-notice"}`.
//!
//! Built from the ui layer (R10): a Section per group, a Row per key (the
//! key's mark, its name, the fingerprint shortened in its middle, what it
//! is used for, its badge), an empty group's inline empty state with the
//! button that fills it, the system's action sheet and alerts. The
//! identifiers: `ssh-keys-add`, `ssh-keys-done`, `ssh-key-row-<id>`,
//! `ssh-keys-no-device` / `-no-imported` with `ssh-keys-generate` /
//! `-import`.

use std::{rc::Rc, sync::Arc};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    os::Os,
    screen::Screen,
    ui::{
        alert::{alert_dialog, delete_title},
        button::{Size, Variant, button},
        empty::{EmptyVariant, empty},
        notice::{Toasts, toasts},
        page,
        row::{Row, row},
        section::{Inset, section},
        status::{BadgeVariant, badge},
        tokens::token,
    },
};

/// A key's avatar: its kind, as its gradient tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Avatar {
    /// Made in this device's Secure Enclave (`Ink.avatarSE`).
    SecureEnclave,
    /// A hardware token's (`Ink.avatarYubikey`).
    Hardware,
    /// Imported or generated (`Ink.avatarImported`).
    Imported,
    /// An RSA key unused for 90 days (`Ink.avatarGray`).
    Gray,
}

/// One key's metadata, as the page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Key {
    pub id: String,
    pub name: String,
    /// The algorithm badge (`SE · P256`, `ED25519`, `RSA-4096`).
    pub badge: String,
    pub avatar: Avatar,
    /// Unused for 90 days: its avatar's symbol is light.
    pub stale: bool,
    /// `SHA256:…` (empty: none shown).
    pub fingerprint: String,
    /// The saved connections that sign in with it (B602: it was the
    /// hosts picked once at Add Key, a label that bound nothing).
    #[serde(default)]
    pub hosts: Vec<String>,
    /// When it was last used, as the app writes the month and year, for a
    /// stale key no connection signs in with ("unused since %@").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unused_since: Option<String>,
    /// Whether it has a public key to copy or share.
    pub has_public_key: bool,
    /// The kind of key it is when the app can't sign in with it
    /// (`"ECDSA brainpoolP256r1"`, B780): its row says so where a
    /// fingerprint would be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keys {
    /// The Secure Enclave's keys.
    pub device: Vec<Key>,
    /// The others.
    pub imported: Vec<Key>,
    /// The key the delete card asks about, while it shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleting: Option<String>,
    /// Why a key wasn't deleted, while the card saying so shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<Notice>,
}

/// A key that wasn't deleted, and why, in the host's words: deleting a key
/// erases its private key, so a key a saved connection still signs in with
/// stays, and the notice names those connections (B232).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notice {
    pub title: String,
    pub message: String,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "keys", …}`: what the page shows now.
    Keys(Keys),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("ssh-keys", json)?;
        match op.as_str() {
            "keys" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Keys)
                .map_err(|e| format!("ssh-keys: keys: {e}")),
            other => Err(format!("ssh-keys: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Keys(keys) => {
                let mut value = serde_json::to_value(keys).expect("keys serialize");
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

/// Settings › SSH Keys.
#[component]
pub fn SshKeysScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let device = signal(Vec::<Key>::new());
    let imported = signal(Vec::<Key>::new());
    let deleting = signal(None::<String>);
    let notice = signal(None::<Notice>);
    let ready = signal(false);
    channel::hear(
        channel.as_ref(),
        Screen::SshKeys,
        Message::parse,
        move |Message::Keys(keys)| {
            device.set(keys.device);
            imported.set(keys.imported);
            deleting.set(keys.deleting);
            notice.set(keys.notice);
            ready.set(true);
        },
    );
    let toasts = toasts();
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    // The key the host asks about deleting, while it has it.
    let asked = computed(move || {
        let id = deleting.get()?;
        device
            .with(|d| d.iter().find(|k| k.id == id).cloned())
            .or_else(|| imported.with(|i| i.iter().find(|k| k.id == id).cloned()))
    });
    ask_once(asked, deleting, tell.clone());
    say_once(notice, tell.clone());
    let body = {
        let tell = tell.clone();
        move || {
            page::page(
                bar,
                scrolled,
                "ssh-keys-list",
                [
                    group(Group::Device, device, tell.clone(), toasts),
                    group(Group::Imported, imported, tell.clone(), toasts),
                ],
            )
            .into_element()
        }
    };
    let failed = tell.clone();
    let nav = nav_bar::nav_bar(nav_bar::NavBar {
        metrics: bar,
        back_title: String::new(),
        title: nav_bar::fixed(t!("SSH Keys")),
        leading: Some(nav_bar::BarButton {
            label: t!("Done"),
            content: nav_bar::BarContent::Text,
            enabled: computed(|| true),
            emphasized: false,
            test_id: "ssh-keys-done",
            on_press: Box::new({
                let tell = tell.clone();
                move || tell(json!({"ev": "done"}))
            }),
            menu: None,
        }),
        trailing: vec![nav_bar::BarButton {
            label: t!("Add Key"),
            content: nav_bar::BarContent::Circled("plus"),
            enabled: computed(|| true),
            emphasized: false,
            test_id: "ssh-keys-add",
            on_press: Box::new(move || tell(json!({"ev": "add"}))),
            menu: None,
        }],
        on_back: Box::new(|| {}),
        scrolled,
    });
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("ssh-keys")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            failed,
        ))
        .child(nav)
        .child(toasts.host().test_id("ssh-keys-toast"))
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

/// Asks, in the system's alert, about the key the host names for deleting
/// (`deleting`), once for each time it names one: Delete tells the host
/// `delete`, Cancel `cancel-delete`. A post that names the same key again
/// before the host heard the answer does not ask again.
fn ask_once(asked: Memo<Option<Key>>, deleting: Signal<Option<String>>, tell: Tell) {
    let shown = stored(None::<String>);
    effect(move || {
        let Some(key) = asked.get() else {
            if deleting.get().is_none() {
                shown.replace(None);
            }
            return;
        };
        if shown.get().as_deref() == Some(key.id.as_str()) {
            return;
        }
        shown.replace(Some(key.id.clone()));
        let answer = alert_dialog()
            .title(delete_title(&key.name))
            .description(t!(
                "Its private key is erased from this device for good; Moshpit keeps no other copy. Servers that only trust this key will need another one."
            ))
            .destructive("delete", t!("Delete"))
            .cancel("cancel", t!("Cancel"))
            .show();
        let tell = tell.clone();
        drop(spawn(async move {
            let delete = answer
                .await
                .is_ok_and(|r| r.action.as_deref() == Some("delete"));
            deleting.set(None);
            if delete {
                tell(json!({"ev": "delete", "id": key.id}));
            } else {
                tell(json!({"ev": "cancel-delete"}));
            }
        }));
    });
}

/// Says why a key wasn't deleted (`notice`, the host's words) in the
/// system's alert, once for each notice; OK tells the host
/// `dismiss-notice`.
fn say_once(notice: Signal<Option<Notice>>, tell: Tell) {
    let shown = stored(None::<Notice>);
    effect(move || {
        let Some(said) = notice.get() else {
            shown.replace(None);
            return;
        };
        if shown.get().as_ref() == Some(&said) {
            return;
        }
        shown.replace(Some(said.clone()));
        let answer = alert_dialog()
            .title(said.title)
            .description(said.message)
            .cancel("ok", t!("OK"))
            .show();
        let tell = tell.clone();
        drop(spawn(async move {
            let _ = answer.await;
            notice.set(None);
            tell(json!({"ev": "dismiss-notice"}));
        }));
    });
}

/// The page's two groups.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    /// The Secure Enclave's (the Android Keystore's) keys.
    Device,
    /// The others.
    Imported,
}

/// A group of keys: a row for each, or, with none, what to do about it.
/// Made again when its keys change.
fn group(which: Group, keys: Signal<Vec<Key>>, tell: Tell, toasts: Toasts) -> AnyElement {
    dynamic(move || {
        let keys = keys.get();
        let tell = tell.clone();
        untracked(move || {
            let (header, footer, id) = match which {
                Group::Device => (
                    device_title(Os::current()),
                    Some(device_footer(Os::current())),
                    "ssh-keys-device",
                ),
                Group::Imported => (t!("IMPORTED"), None, "ssh-keys-imported"),
            };
            let mut section = section().header(header).test_id(id);
            if let Some(footer) = footer {
                section = section.footer(footer);
            }
            if keys.is_empty() {
                section = section.item(empty_group(which, tell), Inset::Content);
            } else {
                section = section.rows(
                    keys.into_iter()
                        .map(|key| key_row(key, tell.clone(), toasts)),
                );
            }
            section.into_element()
        })
    })
    .into_element()
}

/// An empty group: what is missing, and the button that makes one (no
/// hunting for the bar's +). Generate opens Add Key on Generate, Import on
/// Import (`{"ev": "add", "method": …}`).
fn empty_group(which: Group, tell: Tell) -> AnyElement {
    let (title, action, method, id, button_id) = match which {
        Group::Device => (
            t!("No device key yet"),
            t!("Generate key"),
            "generate",
            "ssh-keys-no-device",
            "ssh-keys-generate",
        ),
        Group::Imported => (
            t!("No imported keys"),
            t!("Import key"),
            "import",
            "ssh-keys-no-imported",
            "ssh-keys-import",
        ),
    };
    empty(title)
        .variant(EmptyVariant::Inline)
        .icon("key")
        .action(
            button(action)
                .variant(Variant::Secondary)
                .size(Size::Sm)
                .on_press(move || tell(json!({"ev": "add", "method": method})))
                .test_id(button_id),
        )
        .test_id(id)
        .into_element()
}

/// One key's row: the key's mark, its name, its fingerprint (shortened in
/// its middle), what it is used for (or since when it is not), and its
/// badge (Secure Enclave in green, an algorithm neutral). Screen readers
/// hear the badge right after the name: for someone who can't see the
/// rows it is what tells two keys apart. A press opens its actions in the
/// system's action sheet; so does a long press.
fn key_row(key: Key, tell: Tell, toasts: Toasts) -> Row {
    let open = {
        let key = key.clone();
        move || actions(&key, tell.clone(), toasts)
    };
    let detail = if !key.hosts.is_empty() {
        Some(t!("Used for %@", arg1 = key.hosts.join(", ")))
    } else {
        key.unused_since
            .as_ref()
            .map(|since| t!("unused since %@", arg1 = since))
    };
    let badge_variant = if key.avatar == Avatar::SecureEnclave {
        BadgeVariant::Success
    } else {
        BadgeVariant::Neutral
    };
    let mut row = row(key.name.clone())
        .leading_icon(match key.avatar {
            Avatar::SecureEnclave => "lock.shield",
            Avatar::Hardware => "mediastick",
            Avatar::Imported | Avatar::Gray => "key",
        })
        .badge(badge(key.badge.clone()).variant(badge_variant).mono())
        .on_press(open.clone())
        .on_long_press(open)
        .test_id(&format!("ssh-key-row-{}", key.id));
    if let Some(kind) = &key.refused {
        // B780: one the app can't sign in with says why, before a sign-in
        // fails; it can still go (Delete Key).
        row = row.subtitle(t!("Can't sign in with %@ keys yet", arg1 = kind));
    } else if !key.fingerprint.is_empty() {
        row = row.subtitle(key.fingerprint.clone()).mono_subtitle();
    }
    if let Some(detail) = detail {
        let detail: SharedString = detail.into();
        row = row.detail(move || detail.clone());
    }
    row
}

/// `key`'s actions in the system's action sheet: copy or share its public
/// key, copy its fingerprint (each only when there is one), delete it. The
/// pick goes to the host (`{"ev": …, "id": …}`), which copies, shares or
/// checks the delete; a copy says so in a toast.
fn actions(key: &Key, tell: Tell, toasts: Toasts) {
    let mut sheet = alert_dialog().title(key.name.clone()).action_sheet();
    if key.has_public_key {
        sheet = sheet.action("copy-public-key", t!("Copy Public Key"));
    }
    if !key.fingerprint.is_empty() {
        sheet = sheet.action("copy-fingerprint", t!("Copy Fingerprint"));
    }
    if key.has_public_key {
        sheet = sheet.action("share-public-key", t!("Share Public Key"));
    }
    let answer = sheet
        .destructive("confirm-delete", t!("Delete Key"))
        .cancel("cancel", t!("Cancel"))
        .show();
    let id = key.id.clone();
    drop(spawn(async move {
        let Ok(answer) = answer.await else { return };
        let Some(choice) = answer.action else { return };
        match &*choice {
            "copy-public-key" => toasts.show(t!("Public key copied")),
            "copy-fingerprint" => toasts.show(t!("Fingerprint copied")),
            "share-public-key" | "confirm-delete" => {}
            _ => return,
        }
        tell(json!({"ev": &*choice, "id": id}));
    }));
}

/// THIS DEVICE's title: where a hardware key lives on `os` (B258).
fn device_title(os: Os) -> String {
    match os {
        Os::Apple => t!("THIS DEVICE · SECURE ENCLAVE"),
        Os::Android => t!("THIS DEVICE · ANDROID KEYSTORE"),
    }
}

/// THIS DEVICE's footer: what a hardware key is on `os`. Android asks for no
/// biometric prompt on a signature (yet), so it doesn't promise one.
fn device_footer(os: Os) -> String {
    match os {
        Os::Apple => {
            t!("Hardware-backed keys cannot be exported; every signature triggers Face ID.")
        }
        Os::Android => t!(
            "Keys in the Android Keystore cannot be exported; every signature is made inside it."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn key(id: &str, avatar: Avatar) -> Key {
        Key {
            id: id.into(),
            name: format!("key {id}"),
            badge: "ED25519".into(),
            avatar,
            stale: false,
            fingerprint: "SHA256:abc".into(),
            hosts: vec!["m1-pro".into()],
            unused_since: None,
            has_public_key: true,
            refused: None,
        }
    }

    #[test]
    fn messages_round_trip() {
        for keys in [
            Keys {
                device: vec![],
                imported: vec![],
                deleting: None,
                notice: None,
            },
            Keys {
                device: vec![key("se", Avatar::SecureEnclave)],
                imported: vec![
                    key("a", Avatar::Imported),
                    Key {
                        stale: true,
                        hosts: vec![],
                        unused_since: Some("Mar 2026".into()),
                        ..key("b", Avatar::Gray)
                    },
                ],
                deleting: Some("a".into()),
                notice: Some(Notice {
                    title: "Key in use".into(),
                    message: "Connections using it: pi.".into(),
                }),
            },
        ] {
            let message = Message::Keys(keys);
            assert_eq!(Message::parse(&message.to_json()), Ok(message));
        }
    }

    #[test]
    fn the_host_message_as_written() {
        let json = r#"{"op":"keys","device":[],"imported":[{"id":"1","name":"laptop","badge":"RSA-3072","avatar":"gray","stale":true,"fingerprint":"","unusedSince":"Mar 2026","hasPublicKey":false}]}"#;
        let Message::Keys(keys) = Message::parse(json).unwrap();
        assert_eq!(keys.imported[0].avatar, Avatar::Gray);
        assert_eq!(keys.imported[0].hosts, Vec::<String>::new());
        assert_eq!(keys.imported[0].unused_since.as_deref(), Some("Mar 2026"));
        assert_eq!(keys.deleting, None);
        assert_eq!(keys.notice, None);
        let json = r#"{"op":"keys","device":[],"imported":[],"notice":{"title":"Key in use","message":"Connections using it: pi."}}"#;
        let Message::Keys(keys) = Message::parse(json).unwrap();
        assert_eq!(
            keys.notice,
            Some(Notice {
                title: "Key in use".into(),
                message: "Connections using it: pi.".into()
            })
        );
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        for (json, says) in [
            ("[", "not JSON"),
            (r#"{"op":"themes"}"#, "no op \"themes\""),
            (r#"{"op":"keys","device":[]}"#, "imported"),
            (
                r#"{"op":"keys","device":[{"id":"1","name":"a","badge":"b","avatar":"yubikey","stale":false,"fingerprint":"","hasPublicKey":true}],"imported":[]}"#,
                "yubikey",
            ),
            (
                r#"{"op":"keys","device":[],"imported":[],"privateKey":"x"}"#,
                "privateKey",
            ),
        ] {
            let error = Message::parse(json).unwrap_err();
            assert!(error.contains(says), "{json}: {error}");
        }
    }
}
