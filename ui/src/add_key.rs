//! Add Key (`docs/nsh/UI-DESIGN.md` §6.6) in NSH, in its sheet: generate a
//! key pair on the device (in the Secure Enclave if asked), or import one.
//! Which connections sign in with it is theirs to say (each picks its key);
//! SSH Keys lists them (B602).
//!
//! Key material never comes here. The private key's text and its
//! passphrase are typed into the host's own views, laid out by this page
//! (`native("moshpit/key-pem")`, a `UITextView` as SwiftUI's `TextEditor`
//! is, and `native("moshpit/key-passphrase")`, a secure `UITextField`), so
//! they stay in the host's memory, never in the engine or on the channel.
//! What the page knows of them is what the host says ([`Pem`]): whether a
//! key was pasted and whether it is behind a passphrase. Generating,
//! importing (formats are converted there: whatever the user pastes is
//! stored as an unencrypted OpenSSH key behind the Keychain), the Keychain
//! and the Secure Enclave are all the host's.
//!
//! The host posts the form ([`Message`]): `{"op": "add-key", …}`
//! ([`AddKey`]), again whenever it changes; the text fields' contents only
//! count when their revision (`fields`) moves (the first post, a file
//! picked for the public key line), so what the user is typing is never
//! overwritten. The page tells it:
//!
//! - `{"ev": "method", "value": "generate" | "import"}`,
//!   `{"ev": "algorithm", "value": "ed25519" | "rsa"}`,
//!   `{"ev": "set", "key": "requireFaceID" | "secureEnclave", "value"}`;
//! - `{"ev": "name" | "comment" | "public-line", "value": …}` as a field
//!   changes;
//! - `{"ev": "import-file", "target": "private" | "public"}` (the host's
//!   document picker fills the field);
//! - `{"ev": "submit"}`, `{"ev": "cancel"}`, and `{"ev": "dismiss-error"}`
//!   from the alert saying why a key could not be made.
//!
//! Built from the ui layer (R10): Generate or Import in a segmented
//! control; Generate's name, algorithm (while the key is not the Secure
//! Enclave's) and comment, and the Secure Enclave and Face ID switches;
//! Import's name and comment, the private key in a stacked field, Import
//! File…, the passphrase in a secret field while the key needs one, the
//! public key line; BIND TO HOSTS as toggle chips, and the fingerprint.
//! Cancel, and Generate or Import in the bar (enabled when the host says it
//! can submit). The same identifiers as before (`add-key-*`).

use std::{rc::Rc, sync::Arc};

use nsh_core::{color::Rgba, prelude::*};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    nav_bar::{self, BarMetrics},
    os::Os,
    screen::Screen,
    theme,
    ui::{
        alert::alert_dialog,
        control::{option, segmented, select, switch},
        feedback::separator,
        field::{
            ControlState, FieldControl, FieldLayout, field, remount, secret_field, text_input,
        },
        page,
        row::{PADDING_X, Row, RowTone, row},
        section::{Inset, section},
        tokens::token,
        typography::TextToken,
    },
};

/// How the key is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Method {
    Generate,
    Import,
    /// A security key's (sk). Not offered: the page shows it as Generate.
    Hardware,
}

/// What a generated key is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Algorithm {
    Ed25519,
    Rsa,
}

/// What the host's private key editor holds, as far as the page may know.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pem {
    /// Nothing pasted yet: the placeholder shows.
    pub empty: bool,
    /// The pasted key is behind a passphrase: its field shows.
    pub encrypted: bool,
}

/// The form, as the host has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AddKey {
    pub method: Method,
    pub algorithm: Algorithm,
    #[serde(rename = "requireFaceID")]
    pub require_face_id: bool,
    pub secure_enclave: bool,
    pub pem: Pem,
    /// The text fields, as the host has them: taken when `fields` moves.
    pub name: String,
    pub comment: String,
    pub public_line: String,
    pub fields: u64,
    /// Generate or Import can be pressed.
    pub can_submit: bool,
    /// A key is being made or imported.
    pub working: bool,
    /// The new key's fingerprint, once there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// Why the key could not be made, for the error card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Import Again (B781): a key whose private half isn't on this device
    /// is imported into its own record, which keeps its name — no method to
    /// pick, no name or comment to give.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub again: bool,
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `{"op": "add-key", …}`: the form now.
    AddKey(AddKey),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("add-key", json)?;
        match op.as_str() {
            "add-key" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::AddKey)
                .map_err(|e| format!("add-key: add-key: {e}")),
            other => Err(format!("add-key: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::AddKey(form) => {
                let mut value = serde_json::to_value(form).expect("a form serializes");
                value["op"] = json!("add-key");
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

/// The form as the page shows it: the host's last post.
type State = Signal<Option<AddKey>>;

/// Reads `f` of the form (its default before the host's first post).
fn read<T>(state: State, f: impl FnOnce(&AddKey) -> T) -> Option<T> {
    state.with(|s| s.as_ref().map(f))
}

/// The private key editor's least height: room for a few lines of a key.
const PEM_MIN_HEIGHT: f32 = 96.0;

/// Add Key.
#[component]
pub fn AddKeyScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let bar = BarMetrics::or_standard(channel.as_ref().and_then(|c| c.params().bar));
    let state: State = signal(None);
    // The text fields' revision the page last took.
    let fields = signal(None::<u64>);
    channel::hear(
        channel.as_ref(),
        Screen::AddKey,
        Message::parse,
        move |Message::AddKey(form)| {
            let revision = form.fields;
            // The form first: the fields made again read it.
            state.set(Some(form));
            if fields.get_untracked() != Some(revision) {
                fields.set(Some(revision));
            }
        },
    );
    let scrolled = nav_bar::ScrollEdge::new();
    let tell = tell_host(channel);
    let ready = computed(move || state.with(Option::is_some));
    let import = computed(move || read(state, |s| s.method == Method::Import).unwrap_or(false));
    let error = computed(move || read(state, |s| s.error.clone()).flatten());
    say_error(error, tell.clone());
    let body = {
        let tell = tell.clone();
        move || {
            let tell = tell.clone();
            page::page(
                bar,
                scrolled,
                "add-key-list",
                [
                    dynamic({
                        let tell = tell.clone();
                        move || {
                            if read(state, |s| s.again).unwrap_or(false) {
                                view().into_element()
                            } else {
                                method_section(state, tell.clone())
                            }
                        }
                    })
                    .into_element(),
                    dynamic({
                        let tell = tell.clone();
                        move || {
                            let import = import.get();
                            let tell = tell.clone();
                            // The fields are made again from the host's text
                            // when its revision moves (a field made earlier
                            // keeps its own).
                            untracked(move || {
                                remount(move || fields.get().unwrap_or(0), {
                                    let tell = tell.clone();
                                    move || {
                                        let sections = if import {
                                            import_sections(state, tell.clone())
                                        } else {
                                            generate_sections(state, tell.clone())
                                        };
                                        view()
                                            .w_full()
                                            .gap(px(page::BETWEEN))
                                            .children(sections)
                                            .into_element()
                                    }
                                })
                            })
                        }
                    })
                    .into_element(),
                    fingerprint_section(state),
                    // Room for the keyboard while it is up, so the last
                    // fields scroll above it.
                    view().w_full().pb_keyboard().into_element(),
                ],
            )
            .into_element()
        }
    };
    // The submit button's title follows the method: the bar is made again
    // when it changes.
    let nav = dynamic({
        let tell = tell.clone();
        move || {
            let title = if import.get() {
                t!("Import")
            } else {
                t!("Generate")
            };
            let tell = tell.clone();
            untracked(move || {
                nav_bar::nav_bar(nav_bar::NavBar {
                    metrics: bar,
                    back_title: String::new(),
                    title: nav_bar::fixed(t!("Add Key")),
                    leading: Some(nav_bar::BarButton {
                        label: t!("Cancel"),
                        content: nav_bar::BarContent::Text,
                        enabled: computed(|| true),
                        emphasized: false,
                        test_id: "add-key-cancel",
                        on_press: Box::new({
                            let tell = tell.clone();
                            move || tell(json!({"ev": "cancel"}))
                        }),
                        menu: None,
                    }),
                    trailing: vec![nav_bar::BarButton {
                        label: title,
                        content: nav_bar::BarContent::Text,
                        enabled: computed(move || {
                            read(state, |s| s.can_submit && !s.working).unwrap_or(false)
                        }),
                        emphasized: true,
                        test_id: "add-key-submit",
                        on_press: Box::new(move || {
                            dismiss_keyboard();
                            tell(json!({"ev": "submit"}));
                        }),
                        menu: None,
                    }],
                    on_back: Box::new(|| {}),
                    scrolled,
                })
                .into_element()
            })
        }
    });
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("add-key")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            body,
            tell.clone(),
        ))
        .child(nav)
}

/// What tells the host `event` (`{"ev": …}`), if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

/// Says why the key could not be made (the host's words) in the system's
/// alert, once for each error; OK tells the host `dismiss-error`.
fn say_error(error: Memo<Option<String>>, tell: Tell) {
    let shown = stored(None::<String>);
    effect(move || {
        let Some(message) = error.get() else {
            shown.replace(None);
            return;
        };
        if shown.get().as_ref() == Some(&message) {
            return;
        }
        shown.replace(Some(message.clone()));
        let mut alert = alert_dialog().title(t!("Key error"));
        if !message.is_empty() {
            alert = alert.description(message);
        }
        let answer = alert.cancel("ok", t!("OK")).show();
        let tell = tell.clone();
        drop(spawn(async move {
            let _ = answer.await;
            tell(json!({"ev": "dismiss-error"}));
        }));
    });
}

/// How the key is made: Generate or Import, and what that means.
fn method_section(state: State, tell: Tell) -> AnyElement {
    let footer = computed(move || {
        if read(state, |s| s.method == Method::Import).unwrap_or(false) {
            String::new()
        } else {
            words(Os::current()).method
        }
    });
    view()
        .w_full()
        .gap(px(8.0))
        .child(
            segmented(
                [t!("Generate"), t!("Import")],
                move || {
                    Some(usize::from(
                        read(state, |s| s.method == Method::Import).unwrap_or(false),
                    ))
                },
                move |index| {
                    let method = if index == 1 { "import" } else { "generate" };
                    tell(json!({"ev": "method", "value": method}));
                },
            )
            .label(t!("METHOD"))
            .test_id("add-key-method"),
        )
        .child(when(
            move || !footer.with(String::is_empty),
            move || {
                view().px(px(PADDING_X)).child(
                    TextToken::Footnote
                        .apply(text(move || footer.get()))
                        .text_color(move || token("muted-foreground")),
                )
            },
        ))
        .into_element()
}

/// A text field of the form: `label`, starting from the host's `start`,
/// telling the host as it changes (`{"ev": event, "value": …}`).
fn text_field(
    label: String,
    placeholder: String,
    start: String,
    mono: bool,
    event: &'static str,
    test_id: &'static str,
    tell: Tell,
) -> AnyElement {
    let mut input = text_input()
        .placeholder(placeholder)
        .default_value(move || start.clone())
        .on_change(move |text| tell(json!({"ev": event, "value": text})));
    if mono {
        input = input.mono();
    }
    field(label, input).test_id(test_id).into_element()
}

/// A switch row: `title` over `subtitle`, on while `on()`, telling the host
/// `{"ev": "set", "key": key, "value": …}`.
fn switch_row(
    title: String,
    subtitle: String,
    key: &'static str,
    on: impl Fn() -> bool + Clone + 'static,
    test_id: &'static str,
    tell: Tell,
) -> Row {
    row(title.clone()).subtitle(subtitle).trailing(
        switch(on, move |value| {
            tell(json!({"ev": "set", "key": key, "value": value}))
        })
        .label(title)
        .test_id(test_id),
    )
}

/// Generate's sections: the key (name, comment, and its algorithm while it
/// is not the Secure Enclave's), and where it is kept (the Secure Enclave,
/// Face ID before each signature).
fn generate_sections(state: State, tell: Tell) -> Vec<AnyElement> {
    let (name, comment) = read(state, |s| (s.name.clone(), s.comment.clone())).unwrap_or_default();
    let words = words(Os::current());
    let hardware = move || read(state, |s| s.secure_enclave).unwrap_or(false);
    // The algorithm shows under the name while the key is not the Secure
    // Enclave's (that one is ECDSA P-256): in the name's item, so the fields
    // around it stay.
    let algorithm = when(move || !hardware(), {
        let tell = tell.clone();
        move || {
            let tell = tell.clone();
            view()
                .w_full()
                .child(separator(Inset::Content.points()))
                .child(
                    select(
                        t!("Algorithm"),
                        [option("ed25519", "ED25519"), option("rsa", "RSA-4096")],
                        move || {
                            SharedString::from(match read(state, |s| s.algorithm) {
                                Some(Algorithm::Rsa) => "rsa",
                                _ => "ed25519",
                            })
                        },
                        move |id| tell(json!({"ev": "algorithm", "value": id.to_string()})),
                    )
                    .test_id("add-key-algorithm"),
                )
        }
    });
    let mut kept = section()
        .header(t!("PROTECTION"))
        .footer(words.protection)
        .row(switch_row(
            words.store,
            t!("Hardware-backed · private key cannot be exported"),
            "secureEnclave",
            hardware,
            "add-key-secure-enclave",
            tell.clone(),
        ));
    if biometry(Os::current()) {
        kept = kept.row(switch_row(
            t!("Require Face ID"),
            t!("Face ID confirmation before every signature"),
            "requireFaceID",
            move || read(state, |s| s.require_face_id).unwrap_or(true),
            "add-key-face-id",
            tell.clone(),
        ));
    }
    vec![
        section()
            .header(t!("KEY DETAILS"))
            .footer(t!(
                "ED25519 recommended: 32-byte keys, fast signatures, no practical break to date."
            ))
            .item(
                view()
                    .w_full()
                    .child(text_field(
                        t!("Name"),
                        t!("Name"),
                        name,
                        false,
                        "name",
                        "add-key-name",
                        tell.clone(),
                    ))
                    .child(algorithm)
                    .into_element(),
                Inset::Content,
            )
            .item(
                text_field(
                    t!("Comment"),
                    t!("Comment"),
                    comment,
                    true,
                    "comment",
                    "add-key-comment",
                    tell.clone(),
                ),
                Inset::Content,
            )
            .into_element(),
        kept.into_element(),
    ]
}

/// Import's sections: the key (its name and comment) and the private key
/// (the host's editor, Import File…, the passphrase while the key needs
/// one, the public key line, Face ID).
fn import_sections(state: State, tell: Tell) -> Vec<AnyElement> {
    let (name, comment, public_line) = read(state, |s| {
        (s.name.clone(), s.comment.clone(), s.public_line.clone())
    })
    .unwrap_or_default();
    let encrypted = computed(move || read(state, |s| s.pem.encrypted).unwrap_or(false));
    // The passphrase shows under the key while it needs one: in the key's
    // item, so the host's editor above it is never made again (it would
    // drop the keyboard).
    let passphrase = when(
        move || encrypted.get(),
        || {
            view()
                .w_full()
                .child(separator(Inset::Content.points()))
                .child(
                    secret_field(
                        t!("Passphrase"),
                        "moshpit/key-passphrase",
                        "add-key-passphrase",
                    )
                    .test_id("add-key-passphrase-field"),
                )
        },
    );
    let footer = dynamic(move || {
        let words = if encrypted.get() {
            words(Os::current()).passphrase
        } else {
            t!(
                "Paste an OpenSSH or PEM private key; the public key and fingerprint come from it, so the public key line is optional."
            )
        };
        untracked(move || {
            TextToken::Footnote
                .apply(text(words))
                .text_color(move || token("muted-foreground"))
        })
    });
    let mut key = section()
        .header(t!("PRIVATE KEY"))
        .item(
            view()
                .w_full()
                .child(
                    field(t!("Private key"), PemEditor { state })
                        .layout(FieldLayout::Stacked)
                        .test_id("add-key-pem"),
                )
                .child(passphrase)
                .into_element(),
            Inset::Content,
        )
        .row(
            row(t!("Import File…"))
                .leading_icon("folder")
                .tone(RowTone::Primary)
                .on_press({
                    let tell = tell.clone();
                    move || tell(json!({"ev": "import-file", "target": "private"}))
                })
                .test_id("add-key-import-file"),
        )
        .item(
            text_field(
                t!("Public key"),
                t!("Public key line (optional)"),
                public_line,
                true,
                "public-line",
                "add-key-public-line",
                tell.clone(),
            ),
            Inset::Content,
        )
        .row(
            row(t!("Import Public Key File…"))
                .leading_icon("folder")
                .tone(RowTone::Primary)
                .on_press({
                    let tell = tell.clone();
                    move || tell(json!({"ev": "import-file", "target": "public"}))
                })
                .test_id("add-key-import-public"),
        );
    if biometry(Os::current()) {
        key = key.row(switch_row(
            t!("Require Face ID"),
            t!("Face ID confirmation before every signature"),
            "requireFaceID",
            move || read(state, |s| s.require_face_id).unwrap_or(true),
            "add-key-face-id",
            tell.clone(),
        ));
    }
    let key = view()
        .w_full()
        .gap(px(8.0))
        .child(key)
        .child(view().px(px(PADDING_X)).child(footer))
        .into_element();
    // Import Again keeps the record's name and comment (B781).
    if read(state, |s| s.again).unwrap_or(false) {
        return vec![key];
    }
    vec![
        section()
            .header(t!("KEY DETAILS"))
            .item(
                text_field(
                    t!("Name"),
                    t!("Name"),
                    name,
                    false,
                    "name",
                    "add-key-name",
                    tell.clone(),
                ),
                Inset::Content,
            )
            .item(
                text_field(
                    t!("Comment"),
                    t!("Comment"),
                    comment,
                    true,
                    "comment",
                    "add-key-comment",
                    tell.clone(),
                ),
                Inset::Content,
            )
            .into_element(),
        key,
    ]
}

/// The private key's editor: the host's own text view (`moshpit/key-pem`),
/// mono, growing with the key; the key's text stays in the host. Its
/// placeholder shows over it while it is empty.
struct PemEditor {
    state: State,
}

impl FieldControl for PemEditor {
    fn handle(&self) -> ElementRef {
        element_ref()
    }

    fn build(&self, control: &ControlState) -> AnyElement {
        let state = self.state;
        let empty = computed(move || read(state, |s| s.pem.empty).unwrap_or(true));
        let focused = control.focused;
        let spoken = control.spoken.clone();
        view()
            .w_full()
            .relative()
            .child(
                native("moshpit/key-pem")
                    .w_full()
                    .min_h(px(PEM_MIN_HEIGHT))
                    .prop("identifier", json!("add-key-private-key"))
                    .prop("fontSize", move || {
                        json!(TextToken::MonoSubhead.size(font_scale()))
                    })
                    .prop("textColor", move || hex("foreground"))
                    .prop("tint", move || hex("primary"))
                    .prop("label", move || json!(spoken().to_string()))
                    .element_ref(&control.handle)
                    .on_native_event(move |event: &NativeEvent| match &*event.name {
                        "focus" => focused.set(true),
                        "blur" => focused.set(false),
                        _ => {}
                    }),
            )
            .child(when(
                move || empty.get(),
                || {
                    view()
                        .absolute()
                        .top(px(8.0))
                        .left(px(5.0))
                        .right(px(0.0))
                        .pointer_events(PointerEvents::None)
                        .accessibility_elements_hidden(true)
                        .child(
                            TextToken::MonoSubhead
                                .apply(text("-----BEGIN OPENSSH PRIVATE KEY-----"))
                                .number_of_lines(1u32)
                                .text_color(move || token("subtle-foreground")),
                        )
                },
            ))
            .into_element()
    }
}

/// `name` as the host's props take a color: `#RRGGBB(AA)`.
fn hex(name: &'static str) -> serde_json::Value {
    json!(theme::hex_string(Rgba::from(color(token(name)))))
}

/// What Add Key says about where a key is kept, on `os` (B258).
struct Words {
    /// Generate's footer.
    method: String,
    /// PROTECTION's footer.
    protection: String,
    /// The hardware switch.
    store: String,
    /// An encrypted import's footer.
    passphrase: String,
}

/// The Require Face ID switch shows on `os`: Android asks for no biometric
/// prompt before a signature yet (its Keystore keys and secrets aren't
/// bound to one), so it offers none.
fn biometry(os: Os) -> bool {
    !os.is_android()
}

fn words(os: Os) -> Words {
    match os {
        Os::Apple => Words {
            method: t!(
                "Generates the key pair on device. The private key can optionally be held in the Secure Enclave — used only for signing, never leaving the chip."
            ),
            protection: t!(
                "Secure Enclave keys are limited to ECDSA-P256; the algorithm adjusts automatically when SE is enabled."
            ),
            store: t!("Store in Secure Enclave"),
            passphrase: t!(
                "This key is protected by a passphrase. It's used once, to unlock the key here; from then on the Keychain protects it, behind Face ID when required."
            ),
        },
        Os::Android => Words {
            method: t!(
                "Generates the key pair on device. The private key can optionally be held in the Android Keystore — used only for signing, never leaving it."
            ),
            protection: t!(
                "Android Keystore keys are limited to ECDSA-P256; the algorithm adjusts automatically when the Keystore is on."
            ),
            store: t!("Store in Android Keystore"),
            passphrase: t!(
                "This key is protected by a passphrase. It's used once, to unlock the key here; from then on the Android Keystore protects it."
            ),
        },
    }
}

/// The new key's fingerprint, once there is one.
///
/// B602: this was BIND TO HOSTS, a chip for each host the key would be
/// "bound" to — a label stored on the key and nothing else. No connection
/// signed in with the key for it, the footer sent the person to a detail
/// page that doesn't exist, and SSH Keys listed the chips as the hosts the
/// key is used for, whatever the connections said. A connection picks its
/// key; SSH Keys lists the ones that do.
fn fingerprint_section(state: State) -> AnyElement {
    section()
        .row(
            row(t!("Fingerprint"))
                .subtitle_with(move || {
                    read(state, |s| s.fingerprint.clone())
                        .flatten()
                        .unwrap_or_else(|| t!("—:—:—:—:—:—:—:—:—:—  (shown after generation)"))
                        .into()
                })
                .mono_subtitle()
                .test_id("add-key-fingerprint"),
        )
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_offers_no_face_id() {
        assert!(!biometry(Os::Android));
        assert!(biometry(Os::Apple));
    }

    pub(crate) fn form() -> AddKey {
        AddKey {
            method: Method::Generate,
            algorithm: Algorithm::Ed25519,
            require_face_id: true,
            secure_enclave: false,
            pem: Pem {
                empty: true,
                encrypted: false,
            },
            name: String::new(),
            comment: String::new(),
            public_line: String::new(),
            fields: 1,
            can_submit: false,
            working: false,
            fingerprint: None,
            error: None,
            again: false,
        }
    }

    #[test]
    fn messages_round_trip() {
        for form in [
            form(),
            AddKey {
                method: Method::Import,
                pem: Pem {
                    empty: false,
                    encrypted: true,
                },
                fingerprint: Some("SHA256:abc".into()),
                error: Some("Unsupported: DSA".into()),
                ..form()
            },
        ] {
            let message = Message::AddKey(form);
            assert_eq!(Message::parse(&message.to_json()), Ok(message));
        }
    }

    #[test]
    fn the_host_message_as_written() {
        let json = r#"{"op":"add-key","method":"hardware","algorithm":"rsa","requireFaceID":false,"secureEnclave":true,"pem":{"empty":true,"encrypted":false},"name":"a","comment":"","publicLine":"","fields":3,"canSubmit":false,"working":false}"#;
        let Message::AddKey(form) = Message::parse(json).unwrap();
        assert_eq!(form.method, Method::Hardware);
        assert_eq!(form.algorithm, Algorithm::Rsa);
        assert!(!form.require_face_id);
        assert_eq!(form.fields, 3);
    }

    #[test]
    fn what_is_not_a_message_says_why() {
        let mut no_key = serde_json::to_value(form()).unwrap();
        no_key["op"] = json!("add-key");
        no_key["privateKey"] = json!("-----BEGIN OPENSSH PRIVATE KEY-----");
        let mut bad_method = serde_json::to_value(form()).unwrap();
        bad_method["op"] = json!("add-key");
        bad_method["method"] = json!("yubikey");
        for (json, says) in [
            ("{", "not JSON".to_owned()),
            (r#"{"op":"keys"}"#, "no op \"keys\"".to_owned()),
            (&no_key.to_string(), "privateKey".to_owned()),
            (&bad_method.to_string(), "yubikey".to_owned()),
        ] {
            let error = Message::parse(json).unwrap_err();
            assert!(error.contains(&says), "{json}: {error}");
        }
    }
}
