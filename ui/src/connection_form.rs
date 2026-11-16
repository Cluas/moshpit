//! Home › Add connection and Edit connection
//! (`docs/nsh/UI-DESIGN.md` §6.5, R9): the form a saved host is made and
//! edited with, in Home's sheet, on the redesign's components (`crate::ui`:
//! [`Section`](crate::ui::section::Section), [`Field`](crate::ui::field::Field),
//! [`SecretField`](crate::ui::field::SecretField), the switch, segmented
//! control and select). Its model ([`FormState`], [`Secrets`],
//! [`Field`]) predates the redesign, which changed how it looks, not what it keeps.
//!
//! The host owns the draft and the saving: it posts what the form starts
//! from ([`Message`]): `{"op": "form", …}` ([`FormState`]), when the page is
//! made and when something it owns changes (a key picked in the key
//! picker). The page tells it each edit as it is made, `{"ev": "set",
//! "key": …, "value": …}` ([`Field`]), so the draft it keeps is the page's
//! (a page made again starts from it, G5's third rule); `{"ev":
//! "pick-key"}` opens the key picker, `{"ev": "save"}` saves the draft
//! through the app's connection store and keychain, `{"ev": "cancel"}`
//! closes the sheet, and, editing, `{"ev": "delete"}` asks to delete the
//! connection (the host confirms). A text field keeps its own text once
//! made: the host never writes over what is being typed.
//!
//! The password and the pasted private key never come here (NSH-GAPS #11,
//! as Add Key's key material): they are typed into the host's own views,
//! laid out by this page (`moshpit/connection-password`, a secure
//! `UITextField`, through [`SecretField`](crate::ui::field::SecretField),
//! and `moshpit/connection-pem`, a `UITextView`), which keep what is typed
//! in the host's draft. What the page knows of them is [`Secrets`]: whether
//! each holds anything, which the host's view says as it changes (its
//! `filled` event), and whether the connection edited keeps one already —
//! the password field shows that as "Saved", and "Replaces saved" once
//! something is typed over it.
//!
//! Every field has a visible label (pain point 7). Save needs a name, a
//! host, ports that are ports ([`port`]) and something to sign in with
//! ([`can_save`]); it stays disabled until then. A field says what is wrong
//! with it — "Required", or what a port must be — once it is left, never
//! while it is typed in. Return moves from Name to Host, Port and User,
//! and from the proxy's host to its port.
//!
//! The sections are §6.5's: CONNECTION, SIGN IN, MOSH, PROXY, ADVANCED,
//! and Delete connection… when editing; each long explanation of today's
//! form is one line. The identifiers UI tests know stay (`save-connection`,
//! `toggle-use-mosh`, `key-picker-row`, `private-key-editor`,
//! `multiplexer-picker`, `connection-form-password`); a field is
//! `connection-form-<name>`, its control `-input`, its label `-label`, its
//! error `-error`.
//!
//! It draws in either appearance (`Params::light`); the app asks for dark
//! until R18.

use std::{rc::Rc, sync::Arc};

use nsh_core::{color::Rgba, prelude::*};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    channel::{self, Channel},
    face,
    nav_bar::{self, BarMetrics},
    screen::Screen,
    settings::PredictMode,
    theme::hex_string,
    ui::{
        control::{option, segmented, select, switch},
        feedback::{separator, touch_target},
        field::{FieldLayout, SecretShown, field, number_input, remount, secret_field, text_input},
        notice::{BannerTone, banner},
        page,
        row::{Row, RowTone, row},
        section::{Inset, section},
        tokens::{number, token},
        typography::TextToken,
    },
};

/// How the host signs in: a password, or an SSH key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Auth {
    #[default]
    Password,
    Key,
}

/// The multiplexer a connection attaches (`Multiplexer`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mux {
    None,
    #[default]
    Tmux,
    Herdr,
}

impl Mux {
    /// All three, in the menu's order.
    pub const ALL: [Mux; 3] = [Mux::None, Mux::Tmux, Mux::Herdr];

    pub fn id(self) -> &'static str {
        match self {
            Mux::None => "none",
            Mux::Tmux => "tmux",
            Mux::Herdr => "herdr",
        }
    }

    fn label(self) -> String {
        match self {
            Mux::None => t!("None"),
            Mux::Tmux => "tmux".to_owned(),
            Mux::Herdr => "herdr".to_owned(),
        }
    }

    fn subtitle(self) -> String {
        match self {
            Mux::None => t!("Single shell, no session persistence"),
            Mux::Tmux => t!("Mature, already on nearly every host"),
            Mux::Herdr => t!("Built for coding agents — agent status needs no hooks"),
        }
    }
}

/// The SSH key a connection signs in with, as its row names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyChoice {
    pub id: String,
    /// "name · badge".
    pub label: String,
}

/// The draft the form shows (`AddConnectionView`'s state).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FormState {
    /// Editing a saved connection (else adding one).
    pub editing: bool,
    pub name: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub auth: Auth,
    /// What the host's password and private key fields hold, as far as
    /// the page may know.
    pub secrets: Secrets,
    /// The managed key picked; none: paste a PEM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<KeyChoice>,
    pub mosh: bool,
    /// The UDP port range from Settings, as the row shows it.
    pub udp_range: String,
    pub predict_mode: ConnectionPredict,
    pub mosh_server_path: String,
    pub socks: bool,
    pub socks_host: String,
    pub socks_port: String,
    pub mux: Mux,
    pub tmux_path: String,
    pub herdr_path: String,
    /// The emoji the host's mark shows in place of its key's face (§4.13),
    /// as typed; empty: the face. Saved only when it is one ([`emoji`]).
    #[serde(default)]
    pub emoji: String,
    /// The other saved connections, to warn of one this would duplicate
    /// ([`duplicate`], B532).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub others: Vec<Other>,
    /// The connection edited names a password or pasted key of its own
    /// that isn't on this device (B782): its field says so and takes the
    /// keyboard. Nothing is kept then (`secrets`' stored flags are off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
}

/// The secret a connection names that this device doesn't hold (B782): a
/// phone restored from a backup, or set up from another, has every
/// connection and none of their keychain items, which never leave the
/// device that saved them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Missing {
    /// The connection's name as saved (`user@host` with none).
    pub name: String,
    /// Its password, or the key pasted into it.
    pub secret: Auth,
}

impl Missing {
    /// What the field it is missing from says under it.
    pub fn words(&self) -> String {
        match self.secret {
            Auth::Password => t!(
                "The password for %@ isn't on this iPhone. Enter it again.",
                arg1 = self.name
            ),
            Auth::Key => t!(
                "The private key for %@ isn't on this iPhone. Paste it again.",
                arg1 = self.name
            ),
        }
    }
}

/// Another saved connection, as the form compares itself with it (B532).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Other {
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl Other {
    /// What the form calls it: its name, or `user@host` when it has none,
    /// as the list does (`display_name`). An unnamed one was “” (B550).
    pub fn label(&self) -> String {
        if self.name.trim().is_empty() {
            format!("{}@{}", self.user.trim(), self.host.trim())
        } else {
            self.name.clone()
        }
    }
}

/// The saved connection this one would duplicate, and how (B532).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Duplicate {
    /// The same user, host and port as the one named: one host's two
    /// connections, which share its known host key.
    Address(String),
    /// Another connection has this name.
    Name(String),
}

impl Duplicate {
    /// What the form says: a warning, Save still saves.
    pub fn words(&self) -> String {
        match self {
            Duplicate::Address(name) => {
                t!("Same user, host and port as “%@”.", arg1 = name.as_str())
            }
            Duplicate::Name(name) => t!(
                "“%@” is already another connection's name.",
                arg1 = name.as_str()
            ),
        }
    }
}

/// The saved connection `state` would duplicate: one with the same user,
/// host (in any case) and port first, else one with the same name (in any
/// case), spaces round them ignored.
pub fn duplicate(state: &FormState) -> Option<Duplicate> {
    let host = state.host.trim();
    let user = state.username.trim();
    let address = port(&state.port, DEFAULT_PORT).and_then(|port| {
        (!host.is_empty())
            .then(|| {
                state.others.iter().find(|o| {
                    o.port == port
                        && o.user.trim() == user
                        && o.host.trim().eq_ignore_ascii_case(host)
                })
            })
            .flatten()
    });
    if let Some(other) = address {
        return Some(Duplicate::Address(other.label()));
    }
    let name = state.name.trim().to_lowercase();
    (!name.is_empty())
        .then(|| {
            state
                .others
                .iter()
                .find(|o| o.name.trim().to_lowercase() == name)
        })
        .flatten()
        .map(|other| Duplicate::Name(other.name.clone()))
}

/// The password and the private key, as far as the page may know: whether
/// there is one, never what it is (the host's fields hold the text).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Secrets {
    /// The password field holds a password.
    pub password: bool,
    /// The private key editor holds a key (more than white space).
    pub pem: bool,
    /// The connection edited keeps a password of its own, which saving
    /// with the field left empty keeps.
    pub stored_password: bool,
    /// The connection edited keeps a key it signs in with (one pasted into
    /// it, or the managed key still picked), which saving with no key
    /// picked or pasted keeps.
    pub stored_key: bool,
}

/// What a secret field stands at, for the person: the redesign shows a
/// kept password or key as saved rather than as an empty field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretState {
    /// Nothing typed or pasted, and nothing kept: Save needs one.
    Empty,
    /// Nothing typed or pasted, and the connection edited keeps one, which
    /// Save keeps.
    Saved,
    /// Typed or pasted: Save writes it (over a kept one).
    Edited,
}

impl SecretState {
    fn of(given: bool, kept: bool) -> Self {
        match (given, kept) {
            (true, _) => SecretState::Edited,
            (false, true) => SecretState::Saved,
            (false, false) => SecretState::Empty,
        }
    }
}

impl Secrets {
    /// The password field's state.
    pub fn password_state(&self) -> SecretState {
        SecretState::of(self.password, self.stored_password)
    }

    /// The pasted key's state (a managed key picked is the key row's, not
    /// this).
    pub fn key_state(&self) -> SecretState {
        SecretState::of(self.pem, self.stored_key)
    }
}

/// The port `text` names: `default` when it is empty, else a whole number
/// from 1 to 65535 in ASCII digits (spaces, tabs and line breaks round it
/// aside); `None` when it is not one. The SwiftUI form's rule is the same
/// (`ConnectionDraft.port`): "abc" used to save as 22 and "70000" as it
/// was (B294).
pub fn port(text: &str, default: u16) -> Option<u16> {
    let text = text.trim_matches([' ', '\t', '\n', '\r']);
    if text.is_empty() {
        return Some(default);
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>()
        .ok()
        .filter(|n| (1..=65535).contains(n))
        .map(|n| n as u16)
}

/// The SSH port when its field is left empty.
pub const DEFAULT_PORT: u16 = 22;
/// The SOCKS5 proxy's port when its field is left empty.
pub const DEFAULT_PROXY_PORT: u16 = 1080;

/// The port field holds no port.
pub fn port_is_wrong(state: &FormState) -> bool {
    port(&state.port, DEFAULT_PORT).is_none()
}

/// The proxy's port field holds no port, while the proxy is on (off, the
/// field is not shown and saving writes SOCKS5's port).
pub fn proxy_port_is_wrong(state: &FormState) -> bool {
    state.socks && port(&state.socks_port, DEFAULT_PROXY_PORT).is_none()
}

/// A connection's predictive echo (B531): as Settings says, or its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionPredict {
    /// As Settings › Connections › Predictive echo says.
    #[default]
    Settings,
    Adaptive,
    Always,
    Never,
    Experimental,
}

impl ConnectionPredict {
    /// All five, in the menu's order.
    pub const ALL: [ConnectionPredict; 5] = [
        ConnectionPredict::Settings,
        ConnectionPredict::Adaptive,
        ConnectionPredict::Always,
        ConnectionPredict::Never,
        ConnectionPredict::Experimental,
    ];

    /// Its id, as the form and its host say it.
    pub fn id(self) -> &'static str {
        match self {
            ConnectionPredict::Settings => "settings",
            ConnectionPredict::Adaptive => PredictMode::Adaptive.id(),
            ConnectionPredict::Always => PredictMode::Always.id(),
            ConnectionPredict::Never => PredictMode::Never.id(),
            ConnectionPredict::Experimental => PredictMode::Experimental.id(),
        }
    }

    /// The one with id `id`.
    pub fn named(id: &str) -> Option<ConnectionPredict> {
        ConnectionPredict::ALL.into_iter().find(|p| p.id() == id)
    }

    /// The connection's own mode; none: as Settings says.
    pub fn mode(self) -> Option<PredictMode> {
        match self {
            ConnectionPredict::Settings => None,
            ConnectionPredict::Adaptive => Some(PredictMode::Adaptive),
            ConnectionPredict::Always => Some(PredictMode::Always),
            ConnectionPredict::Never => Some(PredictMode::Never),
            ConnectionPredict::Experimental => Some(PredictMode::Experimental),
        }
    }

    /// For the connection's own `mode`; none: as Settings says.
    pub fn of(mode: Option<PredictMode>) -> ConnectionPredict {
        match mode {
            None => ConnectionPredict::Settings,
            Some(PredictMode::Adaptive) => ConnectionPredict::Adaptive,
            Some(PredictMode::Always) => ConnectionPredict::Always,
            Some(PredictMode::Never) => ConnectionPredict::Never,
            Some(PredictMode::Experimental) => ConnectionPredict::Experimental,
        }
    }

    /// What the menu calls it.
    pub fn title(self) -> String {
        match self.mode() {
            None => t!("As in Settings"),
            Some(mode) => mode.title(),
        }
    }
}

/// What is wrong with the host field, when something is (B517).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostProblem {
    /// Nothing, or spaces.
    Empty,
    /// More than a host: ssh's own syntax pasted whole (`dev@box`,
    /// `ssh://box`, `box:2222`, `[2001:db8::1]`) or spaces inside. Saved,
    /// it never resolved; the user and the port have their own fields.
    NotJustAHost,
}

impl HostProblem {
    /// What the field says under it.
    fn words(self) -> String {
        match self {
            HostProblem::Empty => required_words(),
            HostProblem::NotJustAHost => {
                t!("Just the host — the user and port have their own fields.")
            }
        }
    }
}

/// What is wrong with `host` as typed, if anything: a name or an address
/// (IPv6 written bare, `2001:db8::1`), spaces round it ignored.
pub fn host_problem(host: &str) -> Option<HostProblem> {
    let host = host.trim();
    if host.is_empty() {
        return Some(HostProblem::Empty);
    }
    let pasted = host
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '@' | '/' | '[' | ']'))
        // One colon is a port after the host; two or more, an IPv6 address.
        || host.matches(':').count() == 1;
    pasted.then_some(HostProblem::NotJustAHost)
}

/// The host field holds no host, or more than one ([`host_problem`]).
pub fn host_is_wrong(state: &FormState) -> bool {
    host_problem(&state.host).is_some()
}

/// No user to sign in as (B517): it used to save, and every sign-in failed.
pub fn user_is_wrong(state: &FormState) -> bool {
    state.username.trim().is_empty()
}

/// The proxy is on and its host is empty, or spaces (B511): saved, every
/// connect went direct, past the proxy the person turned on. Or it holds
/// more than a host (B606): a proxy app's `127.0.0.1:1080` or
/// `socks5://127.0.0.1:1080` pasted whole, as the host field refuses
/// (B517), never resolved, and every connect failed at the proxy.
pub fn proxy_host_problem(state: &FormState) -> Option<HostProblem> {
    if !state.socks {
        return None;
    }
    host_problem(&state.socks_host)
}

/// The proxy is on and its host holds no host, or more than one
/// ([`proxy_host_problem`]).
pub fn proxy_host_is_wrong(state: &FormState) -> bool {
    proxy_host_problem(state).is_some()
}

/// Why the host's Save failed (`{"op": "failed", …}`, §6.5): the form
/// shows it in a banner under the bar, its contents kept, until the person
/// edits, saves again or closes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveFailure {
    /// The secret the keychain did not keep (the password, the key); none:
    /// the save itself was refused or failed, for `reason`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<Auth>,
    /// The host's words for what went wrong: what the banner says when no
    /// secret failed.
    pub reason: String,
    /// Saving again may work (the keychain was locked or busy): the banner
    /// offers Try again.
    pub retry: bool,
}

impl SaveFailure {
    /// What the banner says.
    pub fn words(&self) -> String {
        match self.secret {
            Some(Auth::Password) => t!("Couldn't save the password."),
            Some(Auth::Key) => t!("Couldn't save the key."),
            None => self.reason.clone(),
        }
    }
}

/// A message from the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `{"op": "form", …}`: the draft to show.
    Form(Box<FormState>),
    /// `{"op": "failed", …}`: Save failed, and why.
    Failed(SaveFailure),
}

impl Message {
    /// The message of `json`, or why it is not one.
    pub fn parse(json: &str) -> Result<Message, String> {
        let (op, fields) = channel::split_op("connection-form", json)?;
        match op.as_str() {
            "form" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(|state| Message::Form(Box::new(state)))
                .map_err(|e| format!("connection-form: form: {e}")),
            "failed" => serde_json::from_value(serde_json::Value::Object(fields))
                .map(Message::Failed)
                .map_err(|e| format!("connection-form: failed: {e}")),
            other => Err(format!("connection-form: no op {other:?}")),
        }
    }

    /// The JSON a host posts for it.
    pub fn to_json(&self) -> String {
        match self {
            Message::Form(state) => {
                let mut value = serde_json::to_value(state).expect("a state serializes");
                value["op"] = json!("form");
                value.to_string()
            }
            Message::Failed(failure) => {
                let mut value = serde_json::to_value(failure).expect("a failure serializes");
                value["op"] = json!("failed");
                value.to_string()
            }
        }
    }
}

/// An edit the user made (`{"ev": "set", "key": …, "value": …}`): the key
/// is [`FormState`]'s field. The password and the private key are not
/// among them: they stay in the host's fields.
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Name(String),
    Host(String),
    Port(String),
    Username(String),
    Auth(Auth),
    Mosh(bool),
    PredictMode(ConnectionPredict),
    MoshServerPath(String),
    Socks(bool),
    SocksHost(String),
    SocksPort(String),
    Mux(Mux),
    TmuxPath(String),
    HerdrPath(String),
    Emoji(String),
}

impl Field {
    /// The event that tells the host.
    pub fn event(&self) -> serde_json::Value {
        let (key, value) = match self {
            Field::Name(v) => ("name", json!(v)),
            Field::Host(v) => ("host", json!(v)),
            Field::Port(v) => ("port", json!(v)),
            Field::Username(v) => ("username", json!(v)),
            Field::Auth(v) => ("auth", json!(v)),
            Field::Mosh(v) => ("mosh", json!(v)),
            Field::PredictMode(v) => ("predictMode", json!(v.id())),
            Field::MoshServerPath(v) => ("moshServerPath", json!(v)),
            Field::Socks(v) => ("socks", json!(v)),
            Field::SocksHost(v) => ("socksHost", json!(v)),
            Field::SocksPort(v) => ("socksPort", json!(v)),
            Field::Mux(v) => ("mux", json!(v.id())),
            Field::TmuxPath(v) => ("tmuxPath", json!(v)),
            Field::HerdrPath(v) => ("herdrPath", json!(v)),
            Field::Emoji(v) => ("emoji", json!(v)),
        };
        json!({"ev": "set", "key": key, "value": value})
    }

    /// The draft with this edit made.
    fn apply(&self, s: &mut FormState) {
        match self.clone() {
            Field::Name(v) => s.name = v,
            Field::Host(v) => s.host = v,
            Field::Port(v) => s.port = v,
            Field::Username(v) => s.username = v,
            Field::Auth(v) => s.auth = v,
            Field::Mosh(v) => s.mosh = v,
            Field::PredictMode(v) => s.predict_mode = v,
            Field::MoshServerPath(v) => s.mosh_server_path = v,
            Field::Socks(v) => s.socks = v,
            Field::SocksHost(v) => s.socks_host = v,
            Field::SocksPort(v) => s.socks_port = v,
            Field::Mux(v) => s.mux = v,
            Field::TmuxPath(v) => s.tmux_path = v,
            Field::HerdrPath(v) => s.herdr_path = v,
            Field::Emoji(v) => s.emoji = v,
        }
    }
}

/// Whether the draft can be saved: it has a name and a host (more than
/// white space), its ports are ports, and it has something to sign in with
/// ([`has_credential`]).
pub fn can_save(state: &FormState) -> bool {
    !state.name.trim().is_empty()
        && !host_is_wrong(state)
        && !user_is_wrong(state)
        && !port_is_wrong(state)
        && !proxy_port_is_wrong(state)
        && !proxy_host_is_wrong(state)
        && !emoji_is_wrong(state)
        && has_credential(state)
}

/// The emoji field holds something that isn't one emoji ([`emoji`]).
pub fn emoji_is_wrong(state: &FormState) -> bool {
    emoji(&state.emoji).is_err()
}

/// The emoji field's text isn't one emoji.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotOneEmoji;

/// What a host's emoji field saves: none for an empty field (the mark
/// shows the key's face); else `text` without the spaces round it, which
/// must be exactly one grapheme cluster and an emoji (spaces alone are not). An emoji is
/// one shown as one by default (`Emoji_Presentation`: 🍓, 🐳, 👩‍💻, a flag),
/// or a character asked to show as one (followed by U+FE0F, ❤️) or a
/// keycap (1️⃣). Letters, digits and text symbols alone are not.
pub fn emoji(text: &str) -> Result<Option<String>, NotOneEmoji> {
    use unicode_properties::{EmojiStatus, UnicodeEmoji};
    use unicode_segmentation::UnicodeSegmentation;
    if text.is_empty() {
        return Ok(None);
    }
    let text = text.trim();
    let mut clusters = text.graphemes(true);
    let (Some(one), None) = (clusters.next(), clusters.next()) else {
        return Err(NotOneEmoji);
    };
    let shown_as_emoji = one.chars().any(|c| {
        matches!(
            c.emoji_status(),
            EmojiStatus::EmojiPresentation
                | EmojiStatus::EmojiPresentationAndModifierBase
                | EmojiStatus::EmojiPresentationAndEmojiComponent
                | EmojiStatus::EmojiPresentationAndModifierAndEmojiComponent
        )
    });
    let asked = one.chars().any(|c| c == '\u{FE0F}' || c == '\u{20E3}')
        && one.chars().next().is_some_and(|c| c.is_emoji_char());
    if shown_as_emoji || asked {
        Ok(Some(one.to_owned()))
    } else {
        Err(NotOneEmoji)
    }
}

/// Saving leaves the connection something to sign in with. A key: one
/// picked (the host names it only while it is in SSH Keys), one pasted,
/// or the one the connection edited keeps. A password: one typed, or the
/// one the connection edited keeps. A key connection with none used to
/// save and could never sign in, and so did a password one: the app asks
/// for no password when it connects, it fails on the missing keychain
/// item (B294).
pub fn has_credential(state: &FormState) -> bool {
    match state.auth {
        Auth::Password => state.secrets.password || state.secrets.stored_password,
        Auth::Key => state.key.is_some() || state.secrets.pem || state.secrets.stored_key,
    }
}

/// The page's props: the surface whose channel it talks through.
#[derive(Clone, PartialEq, Props)]
pub struct ScreenProps {
    /// The surface's id ([`Channel::find`]).
    pub surface: Prop<u64>,
}

type Tell = Rc<dyn Fn(serde_json::Value)>;

/// The draft as the page has it, and what tells the host.
#[derive(Clone)]
struct Draft {
    state: Signal<Option<FormState>>,
    /// The fields left so far: their errors show.
    checked: Signal<Checked>,
    /// Why the last Save failed, while it stands (an edit, Save again or
    /// the banner's close ends it).
    failure: Signal<Option<SaveFailure>>,
    tell: Tell,
}

impl Draft {
    fn read<T: Default>(&self, f: impl Fn(&FormState) -> T) -> T {
        self.state.with(|s| s.as_ref().map(&f).unwrap_or_default())
    }

    /// Makes `field`'s edit and tells the host. A failed Save's banner goes:
    /// the draft is not the one that failed any more.
    fn set(&self, field: Field) {
        self.failure.set(None);
        (self.tell)(field.event());
        self.state.update(|s| {
            if let Some(s) = s {
                field.apply(s);
            }
        });
    }

    /// What sets a field from a value.
    fn setter<T: 'static>(&self, field: fn(T) -> Field) -> impl Fn(T) + 'static {
        let draft = self.clone();
        move |value| draft.set(field(value))
    }

    /// What hears the host's secret field `which` say whether it holds
    /// anything (its `filled` event): the page's own [`Secrets`] follow; the
    /// host knows already.
    fn secret_filled(&self, which: fn(&mut Secrets) -> &mut bool) -> impl Fn(bool) + 'static {
        let state = self.state;
        move |filled| {
            state.update(|s| {
                if let Some(s) = s {
                    *which(&mut s.secrets) = filled;
                }
            });
        }
    }
}

/// Which fields have been left (or came from the host wrong already): a
/// field says what is wrong with it only then, never while it is being
/// typed in (§6.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Checked {
    name: bool,
    host: bool,
    user: bool,
    port: bool,
    proxy_port: bool,
    proxy_host: bool,
    emoji: bool,
}

/// The error under a field: `wrong` once the field has been left.
fn error_once(
    state: Signal<Option<FormState>>,
    checked: Signal<Checked>,
    left: fn(&Checked) -> bool,
    wrong: fn(&FormState) -> bool,
    words: fn() -> String,
) -> impl Fn() -> Option<SharedString> + 'static {
    move || {
        (checked.with(left) && state.with(|s| s.as_ref().is_some_and(wrong)))
            .then(|| SharedString::from(words()))
    }
}

/// What a port must be.
fn port_words() -> String {
    t!("Port must be between 1 and 65535.")
}

fn required_words() -> String {
    t!("Required")
}

/// What the emoji field must hold.
fn emoji_words() -> String {
    t!("One emoji, or leave it empty.")
}

/// Add connection / Edit connection.
#[component]
pub fn ConnectionFormScreen(props: ScreenProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let params = channel.as_ref().map(|c| c.params()).unwrap_or_default();
    let bar = BarMetrics::or_standard(params.bar);
    let state = signal(None::<FormState>);
    let checked = signal(Checked::default());
    let failure = signal(None::<SaveFailure>);
    // Each post from the host makes the fields again from it (the host
    // only posts when the page is made or when what it owns changes).
    let generation = signal(0_u64);
    channel::hear(
        channel.as_ref(),
        Screen::ConnectionForm,
        Message::parse,
        move |message| match message {
            Message::Form(next) => {
                // A port the saved connection already has wrong (an old app
                // saved "70000" as it was) is not being typed: it says so
                // at once. So does a saved connection's missing name or user
                // (B517: old apps saved them) and an emoji the form refuses
                // (B551: Save waited and nothing said why); a new
                // connection's empty fields stay quiet until left.
                let editing = next.editing;
                checked.update(|c| {
                    c.port |= port_is_wrong(&next);
                    c.proxy_port |= proxy_port_is_wrong(&next);
                    c.proxy_host |= proxy_host_is_wrong(&next);
                    c.host |= host_problem(&next.host) == Some(HostProblem::NotJustAHost);
                    c.name |= editing && next.name.trim().is_empty();
                    c.user |= editing && user_is_wrong(&next);
                    c.emoji |= emoji_is_wrong(&next);
                });
                state.set(Some(*next));
                generation.update(|g| *g += 1);
            }
            Message::Failed(why) => failure.set(Some(why)),
        },
    );
    let draft = Draft {
        state,
        checked,
        failure,
        tell: tell_host(channel),
    };
    let scrolled = nav_bar::ScrollEdge::new();
    let ready = computed(move || state.with(Option::is_some));
    let editing = computed(move || state.with(|s| s.as_ref().is_some_and(|s| s.editing)));
    let savable = computed(move || state.with(|s| s.as_ref().is_some_and(can_save)));
    let body = {
        let draft = draft.clone();
        move || {
            let _ = generation.get();
            let draft = draft.clone();
            untracked(move || sections(bar, scrolled, draft).into_element())
        }
    };
    let failure = draft.failure;
    let (cancel, save) = {
        let (a, b) = (draft.tell.clone(), draft.tell.clone());
        (
            move || a(json!({"ev": "cancel"})),
            move || {
                if savable.get_untracked() {
                    failure.set(None);
                    b(json!({"ev": "save"}))
                }
            },
        )
    };
    // Made again when whether one stands, or whether it may be retried,
    // changes; its words follow the failure.
    let failed = {
        let save = save.clone();
        move || {
            let shown = failure.with(|f| f.as_ref().map(|f| f.retry));
            let save = save.clone();
            untracked(move || match shown {
                None => view().into_element(),
                Some(retry) => failed_banner(bar, failure, retry, save).into_element(),
            })
        }
    };
    view()
        .flex_1()
        .relative()
        .bg(move || token("background"))
        .test_id("connection-form")
        .child(crate::boundary::ready_body(
            move || ready.get(),
            move || dynamic(body.clone()),
            draft.tell.clone(),
        ))
        .child(dynamic(failed))
        .child(dynamic(move || {
            let title = if editing.get() {
                t!("Edit connection")
            } else {
                t!("Add connection")
            };
            let (cancel, save) = (cancel.clone(), save.clone());
            untracked(move || {
                nav_bar::nav_bar(nav_bar::NavBar {
                    metrics: bar,
                    back_title: String::new(),
                    title: Box::new(move || title.clone()),
                    leading: Some(nav_bar::BarButton {
                        label: t!("Cancel"),
                        content: nav_bar::BarContent::Text,
                        enabled: computed(|| true),
                        emphasized: false,
                        test_id: "connection-form-cancel",
                        on_press: Box::new(cancel),
                        menu: None,
                    }),
                    trailing: vec![nav_bar::BarButton {
                        label: t!("Save"),
                        content: nav_bar::BarContent::Text,
                        enabled: savable,
                        emphasized: true,
                        test_id: "save-connection",
                        on_press: Box::new(save),
                        menu: None,
                    }],
                    on_back: Box::new(|| {}),
                    scrolled,
                })
            })
        }))
}

/// Why Save failed, in R3's destructive banner just under the bar, over
/// the form (which keeps what it holds): Try again when saving again may
/// work, and a close button.
fn failed_banner(
    bar: BarMetrics,
    failure: Signal<Option<SaveFailure>>,
    retry: bool,
    save: impl Fn() + 'static,
) -> View {
    let words = move || {
        failure
            .with(|f| f.as_ref().map(SaveFailure::words).unwrap_or_default())
            .into()
    };
    let mut notice = banner(BannerTone::Destructive, words)
        .on_close(move || failure.set(None))
        .test_id("connection-form-failed");
    if retry {
        notice = notice
            .action(t!("Try again"), save)
            .action_test_id("connection-form-retry");
    }
    view()
        .absolute()
        .top(move || nav_bar::bar_space(bar))
        .left_0()
        .right_0()
        .px(move || px(nav_bar::margin()))
        .pt(px(8.0))
        .pointer_events(PointerEvents::BoxNone)
        .child(notice)
}

/// What tells the host `event`, if the page has a channel.
fn tell_host(channel: Option<Arc<Channel>>) -> Tell {
    Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    })
}

/// The sections, on the redesign's page ([`page::page`]): under the bar,
/// `space-6` apart, scrolling under it.
fn sections(bar: BarMetrics, scrolled: nav_bar::ScrollEdge, draft: Draft) -> Scroll {
    let editing = draft.read(|s| s.editing);
    let mut sections = vec![
        connection(&draft),
        duplicate_banner(&draft),
        sign_in(&draft),
        mosh(&draft),
        proxy(&draft),
        advanced(&draft),
    ];
    if editing {
        sections.push(delete(&draft));
    }
    page::page(bar, scrolled, "connection-form-list", sections)
}

/// A warning under CONNECTION when this would duplicate another saved
/// connection (B532), once its name or address has been left; Save still
/// saves.
fn duplicate_banner(draft: &Draft) -> AnyElement {
    let (state, checked) = (draft.state, draft.checked);
    let shown = move || {
        checked
            .with(|c| c.name || c.host || c.user)
            .then(|| state.with(|s| s.as_ref().and_then(duplicate)))
            .flatten()
    };
    banner(BannerTone::Warning, move || {
        shown().map(|d| d.words()).unwrap_or_default().into()
    })
    .shown(move || shown().is_some())
    .test_id("connection-form-duplicate")
    .into_element()
}

// --- CONNECTION ----------------------------------------------------------------------

/// What a field starts from: the draft's text when it is made (and made
/// again, crossing into or out of the accessibility sizes: B282).
fn start(draft: &Draft, f: fn(&FormState) -> String) -> impl Fn() -> String + 'static {
    let draft = draft.clone();
    move || draft.read(f)
}

/// Marks a field left, so its error (if any) shows.
fn leave(draft: &Draft, mark: fn(&mut Checked)) -> impl Fn() + 'static {
    let checked = draft.checked;
    move || checked.update(mark)
}

fn connection(draft: &Draft) -> AnyElement {
    let state = draft.state;
    let checked = draft.checked;
    let (name, host, port, user) = (element_ref(), element_ref(), element_ref(), element_ref());
    section()
        .header(t!("CONNECTION"))
        .item(
            field(
                t!("Name"),
                text_input()
                    .default_value(start(draft, |s| s.name.clone()))
                    .placeholder("my-server")
                    .element_ref(&name)
                    .next(&host)
                    .on_change(draft.setter(Field::Name))
                    .on_blur(leave(draft, |c| c.name = true)),
            )
            .error(error_once(
                state,
                checked,
                |c| c.name,
                |s| s.name.trim().is_empty(),
                required_words,
            ))
            .test_id("connection-form-name"),
            Inset::Content,
        )
        .item(
            field(
                t!("Emoji"),
                text_input()
                    .default_value(start(draft, |s| s.emoji.clone()))
                    .placeholder("🐳")
                    .max_length(32)
                    .on_change(draft.setter(Field::Emoji))
                    .on_blur(leave(draft, |c| c.emoji = true)),
            )
            .description(t!(
                "Shows on the host's mark in place of its key's pattern."
            ))
            .error(error_once(
                state,
                checked,
                |c| c.emoji,
                emoji_is_wrong,
                emoji_words,
            ))
            .test_id("connection-form-emoji"),
            Inset::Content,
        )
        .item(
            field(
                t!("Host"),
                text_input()
                    .mono()
                    .keyboard(KeyboardType::Url)
                    .default_value(start(draft, |s| s.host.clone()))
                    .placeholder("example.com")
                    .element_ref(&host)
                    .next(&port)
                    .on_change(draft.setter(Field::Host))
                    .on_blur(leave(draft, |c| c.host = true)),
            )
            .error(move || {
                checked
                    .with(|c| c.host)
                    .then(|| state.with(|s| s.as_ref().and_then(|s| host_problem(&s.host))))
                    .flatten()
                    .map(|problem| SharedString::from(problem.words()))
            })
            .test_id("connection-form-host"),
            Inset::Content,
        )
        .item(
            field(
                t!("Port"),
                number_input()
                    .default_value(start(draft, |s| s.port.clone()))
                    .placeholder("22")
                    .element_ref(&port)
                    .next(&user)
                    .on_change(draft.setter(Field::Port))
                    .on_blur(leave(draft, |c| c.port = true)),
            )
            .error(error_once(
                state,
                checked,
                |c| c.port,
                port_is_wrong,
                port_words,
            ))
            .test_id("connection-form-port"),
            Inset::Content,
        )
        .item(
            field(
                t!("User"),
                text_input()
                    .mono()
                    .default_value(start(draft, |s| s.username.clone()))
                    .placeholder("root")
                    .element_ref(&user)
                    .on_change(draft.setter(Field::Username))
                    .on_blur(leave(draft, |c| c.user = true)),
            )
            .error(error_once(
                state,
                checked,
                |c| c.user,
                user_is_wrong,
                required_words,
            ))
            .test_id("connection-form-username"),
            Inset::Content,
        )
        .test_id("connection-form-connection")
        .into_element()
}

// --- SIGN IN ----------------------------------------------------------------------------

fn sign_in(draft: &Draft) -> AnyElement {
    let state = draft.state;
    let auth = computed(move || state.with(|s| s.as_ref().map(|s| s.auth).unwrap_or_default()));
    let pick = draft.setter(Field::Auth);
    let segments = view()
        .w_full()
        .px(px(16.0))
        .py(px(8.0))
        .min_h(px(touch_target()))
        .justify_center()
        .child(
            segmented(
                [t!("Password"), t!("SSH key")],
                move || Some(if auth.get() == Auth::Password { 0 } else { 1 }),
                move |index| {
                    pick(if index == 0 {
                        Auth::Password
                    } else {
                        Auth::Key
                    })
                },
            )
            .label(t!("Sign in with"))
            .test_id("connection-form-auth"),
        );
    // Made again only when the method changes, from the draft as it is
    // then: a region that read the draft as it went would be made again on
    // every edit, and edits already queued for the old field would be lost.
    let below = {
        let draft = draft.clone();
        move || {
            let draft = draft.clone();
            untracked(move || match auth.get_untracked() {
                Auth::Password => password(&draft),
                Auth::Key => key_rows(draft),
            })
        }
    };
    let method = move || match auth.get() {
        Auth::Password => 0,
        Auth::Key => 1,
    };
    section()
        .header(t!("SIGN IN"))
        .item(segments, Inset::Content)
        .item(remount(method, below), Inset::Content)
        .test_id("connection-form-sign-in")
        .into_element()
}

/// The password, in the host's secure field (`moshpit/connection-password`:
/// what is typed stays there): "Saved" while the connection keeps one and
/// nothing is typed, "Replaces saved" once something is.
fn password(draft: &Draft) -> AnyElement {
    let state = draft.state;
    let secrets = move || state.with(|s| s.as_ref().map(|s| s.secrets).unwrap_or_default());
    let filled = draft.secret_filled(|s| &mut s.password);
    let mut field = secret_field(
        t!("Password"),
        "moshpit/connection-password",
        "connection-form-password",
    )
    .shown(move || {
        let s = secrets();
        SecretShown::of(s.password, s.stored_password)
    })
    .on_filled(filled)
    .test_id("connection-form-password-field");
    // Not on this iPhone (B782): it says whose, and takes the keyboard.
    if let Some(missing) = missing(draft, Auth::Password) {
        field = field.description(missing.words()).auto_focus();
    }
    field.into_element()
}

/// The secret of `kind` the connection edited names and this device
/// doesn't hold, if that is what is missing (B782).
fn missing(draft: &Draft, kind: Auth) -> Option<Missing> {
    draft.read(|s| s.missing.clone().filter(|m| m.secret == kind))
}

/// The key row (always: the way into the key picker's generate and import)
/// and, with no managed key picked, the PEM editor under it.
fn key_rows(draft: Draft) -> AnyElement {
    let state = draft.state;
    let picked = computed(move || state.with(|s| s.as_ref().and_then(|s| s.key.clone())));
    let tell = draft.tell.clone();
    let key_row = row(t!("SSH key"))
        .value_with(move || {
            picked
                .get()
                .map_or_else(|| t!("Paste PEM"), |key| key.label)
                .into()
        })
        .chevron()
        .on_press(move || tell(json!({"ev": "pick-key"})))
        .test_id("key-picker-row");
    let pem = {
        let draft = draft.clone();
        move || {
            view()
                .w_full()
                .child(separator(16.0))
                .child(pem_editor(&draft))
        }
    };
    view()
        .w_full()
        .child(key_row)
        .child(when(move || picked.with(Option::is_none), pem))
        .into_element()
}

/// "Private Key (PEM)" over the host's mono editor
/// (`moshpit/connection-pem`: what is pasted stays there, and it grows
/// with the key) on the terminal's ground. Props `identifier`, `label`,
/// `fontSize`, `textColor`, `tint`.
fn pem_editor(draft: &Draft) -> AnyElement {
    let filled = Rc::new(draft.secret_filled(|s| &mut s.pem));
    // Not on this iPhone (B782): it says whose under the editor, which
    // takes the keyboard.
    let missing = missing(draft, Auth::Key);
    let mut editor = native("moshpit/connection-pem")
        .w_full()
        .min_h(px(84.0))
        .prop("identifier", json!("private-key-editor"))
        .prop("label", json!(t!("Private Key (PEM)")))
        .prop("fontSize", move || json!(face::scaled(11.0, font_scale())))
        .prop("textColor", move || hex("foreground"))
        .prop("tint", move || hex("primary"))
        .on_native_event(move |event: &NativeEvent| {
            if &*event.name == "filled"
                && let Some(full) = event.data.as_bool()
            {
                filled(full);
            }
        });
    if let Some(missing) = &missing {
        editor = editor
            .accessibility_description(missing.words())
            .prop("autoFocus", json!(true));
    }
    let note = missing.map(|missing| {
        TextToken::Footnote
            .apply(text(missing.words()))
            .text_color(move || token("muted-foreground"))
            .accessibility_elements_hidden(true)
            .test_id("connection-form-pem-missing")
    });
    view()
        .w_full()
        .px(px(16.0))
        .py(px(12.0))
        .gap(px(6.0))
        .child(
            TextToken::Subhead
                .apply(text(t!("Private Key (PEM)")))
                .text_color(move || token("muted-foreground")),
        )
        .child(
            view()
                .w_full()
                .min_h(px(84.0 + 16.0))
                .p(px(8.0))
                .bg(move || token("terminal-bg"))
                .rounded(px(number("radius-sm")))
                .border_curve(BorderCurve::Continuous)
                .border(px(1.0))
                .border_color(move || token("border"))
                .child(editor),
        )
        .children(note)
        .into_element()
}

/// `name` as the host's props take a color: `#RRGGBB(AA)`.
fn hex(name: &'static str) -> serde_json::Value {
    json!(hex_string(Rgba::from(color(token(name)))))
}

// --- MOSH -------------------------------------------------------------------------------

/// A switch row, and under it while it is on, `more` (each with its
/// separator): one item, so a section draws no separator for rows that
/// are not there.
fn expanding(
    switch_row: Row,
    on: Memo<bool>,
    more: impl Fn() -> Vec<AnyElement> + 'static,
) -> AnyElement {
    let more = Rc::new(more);
    view()
        .w_full()
        .child(switch_row)
        .child(when(
            move || on.get(),
            move || {
                view().w_full().children(
                    more()
                        .into_iter()
                        .flat_map(|row| [separator(Inset::Content.points()).into_element(), row]),
                )
            },
        ))
        .into_element()
}

fn mosh(draft: &Draft) -> AnyElement {
    let state = draft.state;
    let on = computed(move || state.with(|s| s.as_ref().is_some_and(|s| s.mosh)));
    let set = draft.setter(Field::Mosh);
    let toggle = row(t!("Use mosh"))
        .subtitle(t!("Survives network changes; needs UDP."))
        .wrap_subtitle()
        .trailing(
            switch(move || on.get(), set)
                .label(t!("Use mosh"))
                .test_id("toggle-use-mosh"),
        );
    let rows = {
        let draft = draft.clone();
        move || mosh_rows(&draft)
    };
    section()
        .header(t!("MOSH"))
        .item(expanding(toggle, on, rows), Inset::Content)
        .footer(t!("UDP ports are set in Settings, for every connection."))
        .test_id("connection-form-mosh")
        .into_element()
}

/// The mosh rows that only exist when mosh does.
fn mosh_rows(draft: &Draft) -> Vec<AnyElement> {
    let state = draft.state;
    let mode =
        computed(move || state.with(|s| s.as_ref().map(|s| s.predict_mode).unwrap_or_default()));
    let pick = draft.setter(Field::PredictMode);
    vec![
        select(
            t!("Predict Mode"),
            ConnectionPredict::ALL.map(|p| option(p.id(), p.title())),
            move || mode.get().id().into(),
            move |id| {
                if let Some(predict) = ConnectionPredict::named(&id) {
                    pick(predict);
                }
            },
        )
        .test_id("connection-form-predict")
        .into_element(),
        field(
            t!("mosh-server path"),
            text_input()
                .mono()
                .default_value(start(draft, |s| s.mosh_server_path.clone()))
                .placeholder(t!("Automatic"))
                .on_change(draft.setter(Field::MoshServerPath)),
        )
        .layout(FieldLayout::Stacked)
        .test_id("connection-form-mosh-server")
        .into_element(),
    ]
}

// --- PROXY ------------------------------------------------------------------------------

fn proxy(draft: &Draft) -> AnyElement {
    let state = draft.state;
    let on = computed(move || state.with(|s| s.as_ref().is_some_and(|s| s.socks)));
    let set = draft.setter(Field::Socks);
    let toggle = row(t!("SOCKS5")).trailing(
        switch(move || on.get(), set)
            .label(t!("SOCKS5"))
            .test_id("connection-form-socks"),
    );
    let rows = {
        let draft = draft.clone();
        move || proxy_rows(&draft)
    };
    section()
        .header(t!("PROXY"))
        .item(expanding(toggle, on, rows), Inset::Content)
        .footer(t!("Proxies SSH only; mosh's UDP goes direct."))
        .test_id("connection-form-proxy")
        .into_element()
}

fn proxy_rows(draft: &Draft) -> Vec<AnyElement> {
    let (host, port) = (element_ref(), element_ref());
    vec![
        field(
            t!("Proxy Host"),
            text_input()
                .mono()
                .keyboard(KeyboardType::Url)
                .default_value(start(draft, |s| s.socks_host.clone()))
                .placeholder("127.0.0.1")
                .element_ref(&host)
                .next(&port)
                .on_change(draft.setter(Field::SocksHost))
                .on_blur(leave(draft, |c| c.proxy_host = true)),
        )
        .error({
            let (state, checked) = (draft.state, draft.checked);
            move || {
                checked
                    .with(|c| c.proxy_host)
                    .then(|| state.with(|s| s.as_ref().and_then(proxy_host_problem)))
                    .flatten()
                    .map(|problem| SharedString::from(problem.words()))
            }
        })
        .test_id("connection-form-proxy-host")
        .into_element(),
        field(
            t!("Proxy Port"),
            number_input()
                .default_value(start(draft, |s| s.socks_port.clone()))
                .placeholder("1080")
                .element_ref(&port)
                .on_change(draft.setter(Field::SocksPort))
                .on_blur(leave(draft, |c| c.proxy_port = true)),
        )
        .error(error_once(
            draft.state,
            draft.checked,
            |c| c.proxy_port,
            proxy_port_is_wrong,
            port_words,
        ))
        .test_id("connection-form-proxy-port")
        .into_element(),
    ]
}

// --- ADVANCED ---------------------------------------------------------------------------

fn advanced(draft: &Draft) -> AnyElement {
    let state = draft.state;
    let mux = computed(move || state.with(|s| s.as_ref().map(|s| s.mux).unwrap_or_default()));
    let pick = draft.setter(Field::Mux);
    // A new field for each multiplexer (tmux's and herdr's paths are two
    // fields of the draft; an input carried across would keep the other
    // one's text, NSH-GAPS #12), from the draft as it is then.
    let path = {
        let draft = draft.clone();
        move || {
            let draft = draft.clone();
            untracked(move || {
                let (label, id, read, write): (_, _, fn(&FormState) -> String, _) =
                    match mux.get_untracked() {
                        Mux::Tmux => (
                            t!("tmux path"),
                            "connection-form-tmux-path",
                            |s| s.tmux_path.clone(),
                            Field::TmuxPath as fn(String) -> Field,
                        ),
                        Mux::Herdr => (
                            t!("herdr path"),
                            "connection-form-herdr-path",
                            |s| s.herdr_path.clone(),
                            Field::HerdrPath as fn(String) -> Field,
                        ),
                        Mux::None => return view().into_element(),
                    };
                view()
                    .w_full()
                    .child(separator(Inset::Content.points()))
                    .child(
                        field(
                            label,
                            text_input()
                                .mono()
                                .default_value(start(&draft, read))
                                .placeholder(t!("Automatic"))
                                .on_change(draft.setter(write)),
                        )
                        .layout(FieldLayout::Stacked)
                        .test_id(id),
                    )
                    .into_element()
            })
        }
    };
    let picked = move || match mux.get() {
        Mux::None => 0,
        Mux::Tmux => 1,
        Mux::Herdr => 2,
    };
    let multiplexer = select(
        t!("Multiplexer"),
        Mux::ALL.map(|m| option(m.id(), m.label()).detail(m.subtitle())),
        move || mux.get().id().into(),
        move |id| {
            if let Some(m) = Mux::ALL.into_iter().find(|m| m.id() == &*id) {
                pick(m);
            }
        },
    )
    .test_id("multiplexer-picker");
    section()
        .header(t!("ADVANCED"))
        .item(
            view()
                .w_full()
                .child(multiplexer)
                .child(remount(picked, path)),
            Inset::Content,
        )
        .footer(t!(
            "If the host doesn't have the multiplexer you pick, Moshpit says so and opens a plain shell; it never switches to the other."
        ))
        .test_id("connection-form-advanced")
        .into_element()
}

// --- DELETE -----------------------------------------------------------------------------

/// Editing: deleting the connection, which the host confirms.
fn delete(draft: &Draft) -> AnyElement {
    let tell = draft.tell.clone();
    section()
        .row(
            row(t!("Delete connection…"))
                .tone(RowTone::Destructive)
                .on_press(move || tell(json!({"ev": "delete"})))
                .test_id("connection-form-delete"),
        )
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn sample() -> FormState {
        FormState {
            editing: true,
            name: "studio".into(),
            host: "192.0.2.22".into(),
            port: "22".into(),
            username: "cluas".into(),
            auth: Auth::Key,
            key: Some(KeyChoice {
                id: "k1".into(),
                label: "laptop · ED25519".into(),
            }),
            mosh: true,
            udp_range: "60000 – 61000".into(),
            predict_mode: ConnectionPredict::Settings,
            socks_port: "1080".into(),
            mux: Mux::Herdr,
            ..FormState::default()
        }
    }

    #[test]
    fn messages_round_trip_and_say_why_not() {
        let message = Message::Form(Box::new(sample()));
        assert_eq!(Message::parse(&message.to_json()), Ok(message));
        // A host that sends a secret is refused: the page takes only
        // whether there is one (NSH-GAPS #11).
        for (key, value) in [
            ("password", json!("hunter2")),
            ("pem", json!("-----BEGIN OPENSSH PRIVATE KEY-----")),
        ] {
            let mut secret: serde_json::Value =
                serde_json::from_str(&Message::Form(Box::new(sample())).to_json()).unwrap();
            secret[key] = value.clone();
            assert!(
                Message::parse(&secret.to_string())
                    .unwrap_err()
                    .contains(key),
                "{key}"
            );
            let mut inside: serde_json::Value =
                serde_json::from_str(&Message::Form(Box::new(sample())).to_json()).unwrap();
            inside["secrets"][key] = value;
            assert!(
                Message::parse(&inside.to_string()).is_err(),
                "secrets.{key}"
            );
        }
        let mut bad: serde_json::Value =
            serde_json::from_str(&Message::Form(Box::new(sample())).to_json()).unwrap();
        bad["mux"] = json!("screen");
        assert!(
            Message::parse(&bad.to_string())
                .unwrap_err()
                .contains("screen")
        );
        bad["mux"] = json!("tmux");
        bad["auth"] = json!("token");
        assert!(
            Message::parse(&bad.to_string())
                .unwrap_err()
                .contains("token")
        );
        assert!(Message::parse(r#"{"op":"save"}"#).is_err());
    }

    #[test]
    fn a_failed_save_round_trips_and_carries_no_secret() {
        let failure = SaveFailure {
            secret: Some(Auth::Key),
            reason: "Keychain interaction not allowed".into(),
            retry: true,
        };
        // What it says needs the screen's translations: the headless tests
        // read it on screen.
        let message = Message::Failed(failure);
        assert_eq!(Message::parse(&message.to_json()), Ok(message));
        let refused = Message::Failed(SaveFailure {
            secret: None,
            reason: "Could not save".into(),
            retry: false,
        });
        assert_eq!(Message::parse(&refused.to_json()), Ok(refused));
        assert!(
            Message::parse(r#"{"op":"failed","reason":"x","retry":true,"password":"hunter2"}"#)
                .is_err(),
            "a failure carries no secret"
        );
    }

    #[test]
    fn a_port_is_a_whole_number_from_1_to_65535() {
        for (text, port) in [
            ("", Some(22)),
            ("  ", Some(22)),
            ("22", Some(22)),
            ("1", Some(1)),
            ("65535", Some(65535)),
            ("0022", Some(22)),
            (" 2222\n", Some(2222)),
            ("0", None),
            ("65536", None),
            ("70000", None),
            ("99999999999999999999", None),
            ("abc", None),
            ("22a", None),
            ("-22", None),
            ("+22", None),
            ("2 2", None),
            ("22.0", None),
            ("２２", None),
            ("\u{a0}22", None),
        ] {
            assert_eq!(super::port(text, 22), port, "{text:?}");
        }
        assert_eq!(super::port("", DEFAULT_PROXY_PORT), Some(1080));
        let mut state = sample();
        state.port = "70000".into();
        assert!(!can_save(&state), "70000 is no port");
        state.port = String::new();
        assert!(can_save(&state), "empty is 22");
        state.socks = true;
        state.socks_port = "x".into();
        assert!(!can_save(&state), "x is no proxy port");
        state.socks = false;
        assert!(can_save(&state), "the proxy off, its port is not asked");
    }

    #[test]
    fn edits_as_the_host_reads_them() {
        assert_eq!(
            Field::Name("box".into()).event(),
            json!({"ev": "set", "key": "name", "value": "box"})
        );
        assert_eq!(
            Field::Mux(Mux::None).event(),
            json!({"ev": "set", "key": "mux", "value": "none"})
        );
        assert_eq!(
            Field::PredictMode(ConnectionPredict::Never).event(),
            json!({"ev": "set", "key": "predictMode", "value": "never"})
        );
        assert_eq!(
            Field::Auth(Auth::Password).event(),
            json!({"ev": "set", "key": "auth", "value": "password"})
        );
        let mut state = sample();
        Field::Host(String::new()).apply(&mut state);
        assert!(!can_save(&state), "a host is needed");
        Field::Host("h".into()).apply(&mut state);
        assert!(can_save(&state));
        // A key connection: a key picked, pasted or kept.
        state.key = None;
        assert!(!can_save(&state), "a key connection with no key");
        state.secrets.pem = true;
        assert!(can_save(&state));
        state.secrets.pem = false;
        state.secrets.stored_key = true;
        assert!(can_save(&state));
        state.secrets.stored_key = false;
        state.secrets.stored_password = true;
        assert!(!can_save(&state), "a password kept is no key");
        // A password connection: a password typed or kept.
        state.auth = Auth::Password;
        assert!(can_save(&state), "the password it keeps");
        state.secrets = Secrets::default();
        assert!(!can_save(&state), "a password connection with no password");
        state.secrets.password = true;
        assert!(can_save(&state));
    }

    /// B550: an unnamed saved connection is named as its list names it,
    /// `user@host`: the warning said “” for it.
    #[test]
    fn a_duplicate_of_an_unnamed_connection_names_it_by_its_address() {
        let mut state = sample();
        state.name = String::new();
        state.others = vec![Other {
            name: String::new(),
            user: "cluas".into(),
            host: "192.0.2.22".into(),
            port: 22,
        }];
        assert_eq!(
            duplicate(&state),
            Some(Duplicate::Address("cluas@192.0.2.22".into()))
        );
    }

    /// B606: the proxy's host holds a host, as the host field does (B517);
    /// a proxy app's address pasted whole never resolved.
    #[test]
    fn the_proxy_host_is_just_a_host() {
        let proxied = |host: &str| FormState {
            socks: true,
            socks_host: host.into(),
            ..sample()
        };
        for pasted in [
            "127.0.0.1:1080",
            "socks5://127.0.0.1:1080",
            "me@proxy.local",
            "proxy local",
        ] {
            assert_eq!(
                proxy_host_problem(&proxied(pasted)),
                Some(HostProblem::NotJustAHost),
                "{pasted}"
            );
            assert!(!can_save(&proxied(pasted)), "{pasted}");
        }
        for host in ["127.0.0.1", " proxy.local ", "::1"] {
            assert_eq!(proxy_host_problem(&proxied(host)), None, "{host}");
        }
        assert_eq!(
            proxy_host_problem(&proxied("  ")),
            Some(HostProblem::Empty),
            "empty, as before (B511)"
        );
        let off = FormState {
            socks: false,
            ..proxied("127.0.0.1:1080")
        };
        assert_eq!(proxy_host_problem(&off), None, "no proxy, nothing to check");
    }
}
