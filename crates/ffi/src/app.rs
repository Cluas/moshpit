//! The screens' host models (`moshpit-app`) for the iOS host, through
//! UniFFI: the same models Android's host runs (`crates/android/src/app`),
//! so a rule lives once. Swift keeps the platform's pieces — the keychain
//! and the secrets in it, the pickers, QuickLook, the share sheet — and
//! drives these for everything else.
//!
//! - The connection form: [`ConnectionDraft`] and the `connection_*`
//!   functions.
//! - Settings › Shortcuts and the shortcut editor: `shortcuts_*` and
//!   [`ShortcutEditorHandle`].
//! - The remote-path model (W6): `remote_path_*` and `remote_file_*`.
//! - The folder sheet: [`RemoteFolderHandle`].
//! - Image attachments: `image_insert_*` and [`ImageAttachmentsHandle`].
//! - Settings › Accent's and › Theme's editors: [`AccentEditorHandle`],
//!   [`ThemeEditorHandle`].
//!
//! Models cross as records (Swift structs SwiftUI can bind to) and their
//! operations as functions that take one and give back the next: nothing
//! here holds state, so the SwiftUI form and the NSH page drive the same
//! draft the same way. Not a hot path: a call per edit or tap.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use moshpit_app::connection_form::{self as form, Draft, Given, Intent, Refusal, Stored};
use moshpit_app::image_attachment::{self, Attachments, InsertStyle, Phase};
use moshpit_app::looks::{AccentEditor, EditorIntent, Handled, ThemeEditor};
use moshpit_app::remote_folder::{Failure as FolderFailure, Folder, NameRefusal};
use moshpit_app::remote_paths::{self, Candidate, Entry, Kind};
use moshpit_app::shortcuts::{self, EditorSave, ListOp, ShortcutEditor};
use moshpit_seal::foundation::uuid::Uuid;
use moshpit_store::app_themes::AppTheme;
use moshpit_store::connections::{self, ServerConnection};
use moshpit_store::shortcuts::TerminalShortcut;
use moshpit_store::ssh_keys::SshKeyRecord;
use moshpit_ui::connection_form::{Auth, Mux};
use moshpit_ui::settings::PredictMode as ScreenPredict;

use crate::store::{
    AccentThemeRecord, AuthMethod, ConnectionRecord, KeyRecord, ShortcutRecord, StoreError,
    ThemeRecord,
};

// MARK: - Connection form

/// What a saved connection keeps in the keychain for itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum StoredCredential {
    None,
    /// A password of its own.
    Password,
    /// A key pasted into it (its own keychain item).
    PastedKey,
    /// A managed key from SSH Keys (it points at the key's item).
    ManagedKey {
        id: String,
    },
}

/// The connection form's draft (`moshpit_app::connection_form::Draft`),
/// field for field, without the secrets: of a password or a pasted key the
/// model hears only [`SecretsGiven`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ConnectionDraft {
    /// Editing a saved connection (else adding one).
    pub editing: bool,
    pub name: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub auth: AuthMethod,
    /// The managed key picked (a UUID); none: a PEM pasted.
    pub selected_key: Option<String>,
    pub use_mosh: bool,
    pub predict_mode_raw: String,
    pub roam_on_cellular: bool,
    pub multiplexer_raw: String,
    pub tmux_path: String,
    pub herdr_path: String,
    /// Empty: `mosh-server` on PATH.
    pub mosh_server_path: String,
    pub use_socks_proxy: bool,
    pub socks_proxy_host: String,
    pub socks_proxy_port: String,
    /// The emoji field as typed; empty: the host key's face.
    pub emoji: String,
    pub stored: StoredCredential,
}

/// Which secrets the host's own fields hold: a password that is not empty,
/// a pasted key that is more than white space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Record)]
pub struct SecretsGiven {
    pub password: bool,
    pub key: bool,
}

/// What the screen asked of the host beyond an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConnectionFormIntent {
    PickKey,
    Save,
    Cancel,
    /// Delete the connection edited, once the person confirms.
    Delete,
}

/// A message from the screen, handled: the draft after it, and what it
/// asks beyond an edit.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConnectionFormHandled {
    pub draft: ConnectionDraft,
    pub intent: Option<ConnectionFormIntent>,
}

/// The key row: the key picked, as SSH Keys has it now.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ConnectionKeyRow {
    pub id: String,
    /// "name · badge".
    pub label: String,
}

/// What saving does: the connection to store, and the secret to write
/// first at its `keychain_ref`.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConnectionSavePlan {
    pub connection: ConnectionRecord,
    /// `Password`: write the password typed; `Key`: the key pasted; none:
    /// nothing new to write.
    pub write: Option<AuthMethod>,
    /// A new connection (add it), else the one edited (update it).
    pub added: bool,
    /// The keychain item the connection no longer names (B325): drop it
    /// once the connection is saved, unless something else names it.
    pub released: Option<String>,
    /// It now points at another host or port (B328): forget its push
    /// pairing before saving.
    pub moved: bool,
}

/// Why a save changed nothing.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum ConnectionSaveRefusal {
    /// The sign-in method changes, the connection's own item holds the old
    /// method's secret, and no new one was given (B298).
    NewSecretNeeded { auth: AuthMethod },
    /// It cannot be saved yet: no name, no host, a port that is not one, or
    /// nothing to sign in with.
    Incomplete,
    /// A record the store can't take (an id that is not a UUID).
    Invalid { why: String },
}

impl std::fmt::Display for ConnectionSaveRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NewSecretNeeded { .. } => f.write_str(form::NEW_SECRET_NEEDED),
            Self::Incomplete => f.write_str(Refusal::Incomplete.message()),
            Self::Invalid { why } => f.write_str(why),
        }
    }
}

impl std::error::Error for ConnectionSaveRefusal {}

impl From<StoreError> for ConnectionSaveRefusal {
    fn from(e: StoreError) -> Self {
        Self::Invalid { why: e.to_string() }
    }
}

fn auth_method(a: connections::AuthMethod) -> AuthMethod {
    match a {
        connections::AuthMethod::Password => AuthMethod::Password,
        connections::AuthMethod::Key => AuthMethod::Key,
    }
}

impl From<&Draft> for ConnectionDraft {
    fn from(d: &Draft) -> Self {
        let f = &d.fields;
        Self {
            editing: f.editing,
            name: f.name.clone(),
            host: f.host.clone(),
            port: f.port.clone(),
            username: f.username.clone(),
            auth: match f.auth {
                Auth::Password => AuthMethod::Password,
                Auth::Key => AuthMethod::Key,
            },
            selected_key: d.key.map(|id| id.to_string()),
            use_mosh: f.mosh,
            predict_mode_raw: f.predict_mode.id().to_owned(),
            roam_on_cellular: f.roam_on_cellular,
            multiplexer_raw: f.mux.id().to_owned(),
            tmux_path: f.tmux_path.clone(),
            herdr_path: f.herdr_path.clone(),
            mosh_server_path: f.mosh_server_path.clone(),
            use_socks_proxy: f.socks,
            socks_proxy_host: f.socks_host.clone(),
            socks_proxy_port: f.socks_port.clone(),
            emoji: f.emoji.clone(),
            stored: match d.stored {
                Stored::None => StoredCredential::None,
                Stored::Password => StoredCredential::Password,
                Stored::PastedKey => StoredCredential::PastedKey,
                Stored::ManagedKey(id) => StoredCredential::ManagedKey { id: id.to_string() },
            },
        }
    }
}

/// The model's draft of `d`, with `given`. A key id that is not a UUID is
/// no key; a mode or multiplexer it does not know is its default.
fn draft(d: &ConnectionDraft, given: SecretsGiven) -> Draft {
    let mut draft = Draft::new(d.use_mosh);
    let f = &mut draft.fields;
    f.editing = d.editing;
    f.name = d.name.clone();
    f.host = d.host.clone();
    f.port = d.port.clone();
    f.username = d.username.clone();
    f.auth = match d.auth {
        AuthMethod::Password => Auth::Password,
        AuthMethod::Key => Auth::Key,
    };
    f.predict_mode = ScreenPredict::ALL
        .into_iter()
        .find(|m| m.id() == d.predict_mode_raw)
        .unwrap_or_default();
    f.roam_on_cellular = d.roam_on_cellular;
    f.mux = Mux::ALL
        .into_iter()
        .find(|m| m.id() == d.multiplexer_raw)
        .unwrap_or_default();
    f.tmux_path = d.tmux_path.clone();
    f.herdr_path = d.herdr_path.clone();
    f.mosh_server_path = d.mosh_server_path.clone();
    f.socks = d.use_socks_proxy;
    f.socks_host = d.socks_proxy_host.clone();
    f.socks_port = d.socks_proxy_port.clone();
    f.emoji = d.emoji.clone();
    draft.key = d.selected_key.as_deref().and_then(Uuid::parse);
    draft.stored = match &d.stored {
        StoredCredential::None => Stored::None,
        StoredCredential::Password => Stored::Password,
        StoredCredential::PastedKey => Stored::PastedKey,
        StoredCredential::ManagedKey { id } => {
            Uuid::parse(id).map_or(Stored::None, Stored::ManagedKey)
        }
    };
    draft.set_given(Given {
        password: given.password,
        key: given.key,
    });
    draft
}

/// The key records the store can take (one with an id that is not a UUID
/// is left out: no draft can have picked it).
fn keys(records: Vec<KeyRecord>) -> Vec<SshKeyRecord> {
    records
        .into_iter()
        .filter_map(|k| SshKeyRecord::try_from(k).ok())
        .collect()
}

fn connection(record: ConnectionRecord) -> Result<ServerConnection, StoreError> {
    ServerConnection::try_from(record)
}

/// A new connection's draft: everything at its default, mosh as the "Mosh
/// by default" setting says (B256).
#[uniffi::export]
pub fn connection_draft_new(mosh_by_default: bool) -> ConnectionDraft {
    (&Draft::new(mosh_by_default)).into()
}

/// The draft of saved connection `connection`, as the editor opens it.
#[uniffi::export]
pub fn connection_draft_of(connection: ConnectionRecord) -> Result<ConnectionDraft, StoreError> {
    Ok((&Draft::of(&self::connection(connection)?)).into())
}

/// What the screen told (`{"ev": …}`, as JSON): the draft after an edit,
/// or the intent. An edit the draft does not know changes nothing — the
/// password and the private key among them.
#[uniffi::export]
pub fn connection_draft_handle(
    draft: ConnectionDraft,
    message_json: String,
) -> ConnectionFormHandled {
    let mut model = self::draft(
        &draft,
        SecretsGiven {
            password: false,
            key: false,
        },
    );
    let message: serde_json::Value = serde_json::from_str(&message_json).unwrap_or_default();
    let intent = model.handle(&message).map(|i| match i {
        Intent::PickKey => ConnectionFormIntent::PickKey,
        Intent::Save => ConnectionFormIntent::Save,
        Intent::Cancel => ConnectionFormIntent::Cancel,
        Intent::Delete => ConnectionFormIntent::Delete,
    });
    ConnectionFormHandled {
        draft: (&model).into(),
        intent,
    }
}

/// The draft with key `id` picked (none: paste a PEM); a key picked signs
/// in with a key.
#[uniffi::export]
pub fn connection_draft_pick_key(draft: ConnectionDraft, id: Option<String>) -> ConnectionDraft {
    let mut model = self::draft(
        &draft,
        SecretsGiven {
            password: false,
            key: false,
        },
    );
    model.pick_key(id.as_deref().and_then(Uuid::parse));
    (&model).into()
}

/// Save's rule: a name, a host, ports that are ports, and something to sign
/// in with, `keys` being SSH Keys' now.
#[uniffi::export]
pub fn connection_draft_can_save(
    draft: ConnectionDraft,
    given: SecretsGiven,
    keys: Vec<KeyRecord>,
) -> bool {
    self::draft(&draft, given).can_save(&self::keys(keys))
}

/// The port field holds no port.
#[uniffi::export]
pub fn connection_draft_port_is_wrong(draft: ConnectionDraft) -> bool {
    self::draft(
        &draft,
        SecretsGiven {
            password: false,
            key: false,
        },
    )
    .port_is_wrong()
}

/// The proxy's port field holds no port, while the proxy is on.
#[uniffi::export]
pub fn connection_draft_proxy_port_is_wrong(draft: ConnectionDraft) -> bool {
    self::draft(
        &draft,
        SecretsGiven {
            password: false,
            key: false,
        },
    )
    .proxy_port_is_wrong()
}

/// The connection edited keeps a key that saving with none picked or
/// pasted keeps.
#[uniffi::export]
pub fn connection_draft_keeps_stored_key(draft: ConnectionDraft) -> bool {
    self::draft(
        &draft,
        SecretsGiven {
            password: false,
            key: false,
        },
    )
    .keeps_stored_key()
}

/// The connection edited keeps a password of its own.
#[uniffi::export]
pub fn connection_draft_keeps_stored_password(draft: ConnectionDraft) -> bool {
    self::draft(
        &draft,
        SecretsGiven {
            password: false,
            key: false,
        },
    )
    .keeps_stored_password()
}

/// Where a secret field stands, for the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ConnectionSecretState {
    /// Nothing typed or pasted, and nothing kept.
    Empty,
    /// Nothing typed or pasted; the connection keeps one, which Save keeps.
    Saved,
    /// Typed or pasted: Save writes it.
    Edited,
}

impl From<form::SecretState> for ConnectionSecretState {
    fn from(s: form::SecretState) -> Self {
        match s {
            form::SecretState::Empty => Self::Empty,
            form::SecretState::Saved => Self::Saved,
            form::SecretState::Edited => Self::Edited,
        }
    }
}

/// The password's and the pasted key's states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ConnectionSecretStates {
    pub password: ConnectionSecretState,
    pub key: ConnectionSecretState,
}

/// Where the draft's secret fields stand, `given` being what the host's
/// fields hold: typed, kept by the connection edited ("Saved"), or empty.
#[uniffi::export]
pub fn connection_draft_secret_states(
    draft: ConnectionDraft,
    given: SecretsGiven,
) -> ConnectionSecretStates {
    let model = self::draft(&draft, given);
    ConnectionSecretStates {
        password: model.password_state().into(),
        key: model.key_state().into(),
    }
}

/// The port `text` names: `fallback` when empty, else 1–65535 in ASCII
/// digits; none when it is not one.
#[uniffi::export]
pub fn connection_port(text: String, fallback: u16) -> Option<u16> {
    moshpit_ui::connection_form::port(&text, fallback)
}

/// `connection` with the draft's fields, mosh's UDP range being Settings'.
#[uniffi::export]
pub fn connection_draft_applied(
    draft: ConnectionDraft,
    connection: ConnectionRecord,
    udp_start: i64,
    udp_end: i64,
) -> Result<ConnectionRecord, StoreError> {
    let model = self::draft(
        &draft,
        SecretsGiven {
            password: false,
            key: false,
        },
    );
    Ok((&model.applied(&self::connection(connection)?, (udp_start, udp_end))).into())
}

/// What saving the draft does, over `existing` or as `fresh` (a new
/// connection: its id and date): the credential first, then the fields.
/// `new_ref` is the keychain account a connection that needs an item of
/// its own gets. Refused, nothing is to be written.
#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn connection_draft_save_plan(
    draft: ConnectionDraft,
    existing: Option<ConnectionRecord>,
    fresh: ConnectionRecord,
    keys: Vec<KeyRecord>,
    given: SecretsGiven,
    udp_start: i64,
    udp_end: i64,
    new_ref: String,
) -> Result<ConnectionSavePlan, ConnectionSaveRefusal> {
    let model = self::draft(&draft, given);
    let existing = existing.map(connection).transpose()?;
    let fresh = connection(fresh)?;
    let plan = model
        .save_plan(
            existing.as_ref(),
            || fresh,
            &self::keys(keys),
            (udp_start, udp_end),
            || new_ref,
        )
        .map_err(|refusal| match refusal {
            Refusal::NewSecretNeeded(auth) => ConnectionSaveRefusal::NewSecretNeeded {
                auth: auth_method(auth),
            },
            Refusal::Incomplete => ConnectionSaveRefusal::Incomplete,
        })?;
    Ok(ConnectionSavePlan {
        connection: (&plan.connection).into(),
        write: plan.write.map(auth_method),
        added: plan.added,
        released: plan.released,
        moved: plan.moved,
    })
}

/// `{"op": "form", …}` of the draft, as JSON: of the secrets, only whether
/// there is one.
#[uniffi::export]
pub fn connection_form_message(
    draft: ConnectionDraft,
    given: SecretsGiven,
    keys: Vec<KeyRecord>,
    udp_start: i64,
    udp_end: i64,
) -> String {
    self::draft(&draft, given).message(&self::keys(keys), (udp_start, udp_end))
}

/// The key row the form shows: the key picked as `keys` have it, none when
/// none is picked or it is gone (B290).
#[uniffi::export]
pub fn connection_form_key_row(
    draft: ConnectionDraft,
    keys: Vec<KeyRecord>,
) -> Option<ConnectionKeyRow> {
    self::draft(
        &draft,
        SecretsGiven {
            password: false,
            key: false,
        },
    )
    .key_row(&self::keys(keys))
    .map(|row| ConnectionKeyRow {
        id: row.id,
        label: row.label,
    })
}

// MARK: - Shortcuts

/// What a tap on Settings › Shortcuts does (`moshpit_app::shortcuts::ListOp`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ShortcutListOp {
    Done,
    Add,
    Edit { id: String },
    Move { from: u32, to: u32 },
    RemoveFromToolbar { id: String },
    AddToToolbar { id: String },
    Delete { id: String },
}

/// What the shortcut editor's Save does.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum ShortcutEditorSave {
    /// Add a new shortcut; the store says whether it went on the bar.
    Add { shortcut: ShortcutRecord },
    /// Write over a shortcut the store has.
    Update { shortcut: ShortcutRecord },
}

/// The shipped set, as the store seeds it.
#[uniffi::export]
pub fn shortcut_builtins() -> Vec<ShortcutRecord> {
    moshpit_store::shortcuts::ShortcutStore::builtins()
        .iter()
        .map(Into::into)
        .collect()
}

fn shortcuts(records: Vec<ShortcutRecord>) -> Vec<TerminalShortcut> {
    records
        .into_iter()
        .filter_map(|r| TerminalShortcut::try_from(r).ok())
        .collect()
}

/// `{"op": "shortcuts", …}` for Settings › Shortcuts, as JSON: `shortcuts`
/// in the store's order, each called what `names` says (by id: a
/// built-in's name in the app's language, B234), else its summary.
#[uniffi::export]
pub fn shortcuts_page_message(
    shortcuts: Vec<ShortcutRecord>,
    names: HashMap<String, String>,
) -> String {
    shortcuts::page_message(&self::shortcuts(shortcuts), |s| {
        names
            .get(&s.id.to_string())
            .cloned()
            .unwrap_or_else(|| s.summary.clone())
    })
}

/// What Settings › Shortcuts told (`{"ev": …}`, as JSON), checked against
/// the store's `shortcuts`: none for an event it does not know, a shortcut
/// it does not have, a built-in's edit or delete, or a move off the bar.
#[uniffi::export]
pub fn shortcuts_list_op(
    message_json: String,
    shortcuts: Vec<ShortcutRecord>,
) -> Option<ShortcutListOp> {
    let message: serde_json::Value = serde_json::from_str(&message_json).ok()?;
    let to = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Some(
        match shortcuts::list_op(&message, &self::shortcuts(shortcuts))? {
            ListOp::Done => ShortcutListOp::Done,
            ListOp::Add => ShortcutListOp::Add,
            ListOp::Edit(id) => ShortcutListOp::Edit { id: id.to_string() },
            ListOp::Move { from, to: before } => ShortcutListOp::Move {
                from: to(from),
                to: to(before),
            },
            ListOp::RemoveFromToolbar(id) => {
                ShortcutListOp::RemoveFromToolbar { id: id.to_string() }
            }
            ListOp::AddToToolbar(id) => ShortcutListOp::AddToToolbar { id: id.to_string() },
            ListOp::Delete(id) => ShortcutListOp::Delete { id: id.to_string() },
        },
    )
}

/// The shortcut editor's model (`moshpit_app::shortcuts::ShortcutEditor`):
/// the shortcut edited, its draft as the page tells it, and the new one a
/// first Save added, so a second Save updates it (B288).
#[derive(uniffi::Object)]
pub struct ShortcutEditorHandle(Mutex<ShortcutEditor>);

#[uniffi::export]
impl ShortcutEditorHandle {
    /// The editor on `existing`, or on a new shortcut.
    #[uniffi::constructor]
    pub fn new(existing: Option<ShortcutRecord>) -> Arc<Self> {
        let existing = existing.and_then(|r| TerminalShortcut::try_from(r).ok());
        Arc::new(Self(Mutex::new(ShortcutEditor::new(existing.as_ref()))))
    }

    /// The page's draft (its `change` event's `draft`, as JSON).
    pub fn apply(&self, draft_json: String) {
        let draft: serde_json::Value = serde_json::from_str(&draft_json).unwrap_or_default();
        lock(&self.0).apply(&draft);
    }

    pub fn can_save(&self) -> bool {
        lock(&self.0).can_save()
    }

    /// The draft as the page has it, as JSON (`{"kind", "modifiers", …}`).
    pub fn draft_json(&self) -> String {
        serde_json::to_string(&lock(&self.0).draft).unwrap_or_default()
    }

    /// `{"op": "shortcut", …}`, as JSON.
    pub fn message(&self, hosts: Vec<String>, toolbar_count: u32) -> String {
        lock(&self.0).message(hosts, toolbar_count as usize)
    }

    /// Save: what to write, over the store's current `shortcuts`; none when
    /// it can't be saved yet.
    pub fn save(&self, shortcuts: Vec<ShortcutRecord>) -> Option<ShortcutEditorSave> {
        let current = self::shortcuts(shortcuts);
        let save = lock(&self.0).save(|id| current.iter().find(|s| s.id == id).cloned())?;
        Some(match save {
            EditorSave::Add(s) => ShortcutEditorSave::Add {
                shortcut: (&s).into(),
            },
            EditorSave::Update(s) => ShortcutEditorSave::Update {
                shortcut: (&s).into(),
            },
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

// MARK: - Remote paths (W6)

/// What a tap opens a host file as (`moshpit_app::remote_paths::Kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum RemoteFileKind {
    Image,
    Video,
    Audio,
    Pdf,
    Document,
    Text,
}

impl From<Kind> for RemoteFileKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Image => Self::Image,
            Kind::Video => Self::Video,
            Kind::Audio => Self::Audio,
            Kind::Pdf => Self::Pdf,
            Kind::Document => Self::Document,
            Kind::Text => Self::Text,
        }
    }
}

/// One file or folder on the host.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RemoteFileRecord {
    pub path: String,
    pub is_directory: bool,
    pub size: Option<u64>,
    /// Seconds since 1970.
    pub modified: Option<f64>,
}

impl From<&Entry> for RemoteFileRecord {
    fn from(e: &Entry) -> Self {
        Self {
            path: e.path.clone(),
            is_directory: e.is_directory,
            size: e.size,
            modified: e.modified,
        }
    }
}

impl From<RemoteFileRecord> for Entry {
    fn from(r: RemoteFileRecord) -> Self {
        Entry {
            path: r.path,
            is_directory: r.is_directory,
            size: r.size,
            modified: r.modified,
        }
    }
}

/// A path a terminal shows, as the probe asks the host about it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PathCandidateRecord {
    pub token: String,
    pub directory_only: bool,
}

/// A path the host confirmed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PathHitRecord {
    pub path: String,
    pub is_directory: bool,
}

/// An upload over exec: begin, a base64 chunk at a time (`append`), finish.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExecUploadPlan {
    pub begin: String,
    pub finish: String,
    /// What an upload given up runs: its side files removed (B331).
    pub abandon: String,
}

#[uniffi::export]
pub fn remote_file_kind(path: String) -> Option<RemoteFileKind> {
    remote_paths::kind(&path).map(Into::into)
}

/// Every extension worth a link, without its dot.
#[uniffi::export]
pub fn remote_file_extensions() -> Vec<String> {
    remote_paths::extensions().map(str::to_owned).collect()
}

#[uniffi::export]
pub fn remote_path_probe_command(
    cwd: Option<String>,
    candidates: Vec<PathCandidateRecord>,
) -> String {
    let candidates: Vec<Candidate> = candidates
        .into_iter()
        .map(|c| Candidate {
            token: c.token,
            directory_only: c.directory_only,
        })
        .collect();
    remote_paths::probe_command(cwd.as_deref(), &candidates)
}

/// The probe's output, back onto a batch of `count`, by index.
#[uniffi::export]
pub fn remote_path_parse_probe(output: String, count: u32) -> HashMap<u32, PathHitRecord> {
    remote_paths::parse_probe(&output, count as usize)
        .into_iter()
        .filter_map(|(i, hit)| {
            Some((
                u32::try_from(i).ok()?,
                PathHitRecord {
                    path: hit.path,
                    is_directory: hit.is_directory,
                },
            ))
        })
        .collect()
}

#[uniffi::export]
pub fn remote_path_tidy(path: String) -> String {
    remote_paths::tidy(&path)
}

/// The link a confirmed host path is tagged with.
#[uniffi::export]
pub fn remote_path_link(path: String) -> String {
    remote_paths::link(&path)
}

/// The host path a tapped link names (ours or `file://`), or none.
#[uniffi::export]
pub fn remote_path_from_link(link: String) -> Option<String> {
    remote_paths::link_path(&link)
}

#[uniffi::export]
pub fn remote_path_enclosing_folders(path: String) -> Vec<String> {
    remote_paths::enclosing_folders(&path)
}

#[uniffi::export]
pub fn remote_files_browsable(entries: Vec<RemoteFileRecord>) -> Vec<RemoteFileRecord> {
    let entries: Vec<Entry> = entries.into_iter().map(Into::into).collect();
    remote_paths::browsable(&entries)
        .iter()
        .map(Into::into)
        .collect()
}

#[uniffi::export]
pub fn remote_file_stat_command(path: String) -> String {
    remote_paths::stat_command(&path)
}

#[uniffi::export]
pub fn remote_file_parse_stat(output: String, path: String) -> Option<RemoteFileRecord> {
    remote_paths::parse_stat(&output, &path)
        .as_ref()
        .map(Into::into)
}

#[uniffi::export]
pub fn remote_file_list_command(path: String) -> String {
    remote_paths::list_command(&path)
}

#[uniffi::export]
pub fn remote_file_parse_list(output: String, folder: String) -> Vec<RemoteFileRecord> {
    remote_paths::parse_list(&output, &folder)
        .iter()
        .map(Into::into)
        .collect()
}

#[uniffi::export]
pub fn remote_file_cat_command(path: String) -> String {
    remote_paths::cat_command(&path)
}

/// The exec download's limit, in bytes.
#[uniffi::export]
pub fn remote_file_exec_download_limit() -> u64 {
    remote_paths::EXEC_DOWNLOAD_LIMIT
}

#[uniffi::export]
pub fn remote_file_valid_name(raw: String) -> Option<String> {
    remote_paths::valid_name(&raw)
}

#[uniffi::export]
pub fn remote_file_unique_name(name: String, taken: Vec<String>) -> String {
    remote_paths::unique_name(&name, &taken.into_iter().collect())
}

#[uniffi::export]
pub fn remote_file_child(name: String, directory: String) -> String {
    remote_paths::child(&name, &directory)
}

#[uniffi::export]
pub fn remote_file_deletable(path: String) -> bool {
    remote_paths::deletable(&path)
}

#[uniffi::export]
pub fn remote_file_make_directory_command(path: String) -> String {
    remote_paths::make_directory_command(&path)
}

#[uniffi::export]
pub fn remote_file_rename_command(path: String, new_path: String) -> String {
    remote_paths::rename_command(&path, &new_path)
}

#[uniffi::export]
pub fn remote_file_remove_command(path: String, is_directory: bool) -> Option<String> {
    remote_paths::remove_command(&path, is_directory)
}

#[uniffi::export]
pub fn remote_file_upload_plan(path: String) -> ExecUploadPlan {
    let plan = remote_paths::upload_plan(&path);
    ExecUploadPlan {
        begin: plan.begin,
        finish: plan.finish,
        abandon: plan.abandon,
    }
}

/// One base64 chunk of the exec upload to `path`.
#[uniffi::export]
pub fn remote_file_upload_append(path: String, chunk: String) -> String {
    remote_paths::upload_plan(&path).append(&chunk)
}

/// The markers the exec edits print.
#[uniffi::export]
pub fn remote_file_edit_markers() -> Vec<String> {
    vec![
        remote_paths::EDIT_OK.to_owned(),
        remote_paths::EDIT_TAKEN.to_owned(),
    ]
}

// MARK: - The folder sheet

/// A failure the folder sheet says, and its number (B286).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FolderFailureRecord {
    pub title: String,
    pub message: String,
    pub id: u64,
}

/// What the folder shows now.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RemoteFolderState {
    pub loading: bool,
    pub failure: Option<String>,
    /// Everything shown, newest first.
    pub entries: Vec<RemoteFileRecord>,
    /// The grid: images and video.
    pub media: Vec<RemoteFileRecord>,
    /// Folders, then the other files.
    pub rows: Vec<RemoteFileRecord>,
    /// Fetches under way, by path.
    pub opening: HashMap<String, f64>,
    pub uploading: Option<f64>,
    pub open_failure: Option<FolderFailureRecord>,
    pub edit_failure: Option<FolderFailureRecord>,
}

/// Why a typed name was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FolderNameOutcome {
    /// Nothing to do, nothing said.
    Ignored,
    /// Something with that name is already there.
    Taken,
}

/// Where a typed name goes, or why it doesn't.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FolderNameTarget {
    Path { path: String },
    Refused { why: FolderNameOutcome },
}

fn target(result: Result<String, NameRefusal>) -> FolderNameTarget {
    match result {
        Ok(path) => FolderNameTarget::Path { path },
        Err(NameRefusal::Ignored) => FolderNameTarget::Refused {
            why: FolderNameOutcome::Ignored,
        },
        Err(NameRefusal::Taken) => FolderNameTarget::Refused {
            why: FolderNameOutcome::Taken,
        },
    }
}

fn failure(f: Option<&FolderFailure>) -> Option<FolderFailureRecord> {
    f.map(|f| FolderFailureRecord {
        title: f.title.clone(),
        message: f.message.clone(),
        id: f.id,
    })
}

/// One folder in the folder sheet (`moshpit_app::remote_folder::Folder`):
/// the host does the I/O and tells it what happened.
#[derive(uniffi::Object)]
pub struct RemoteFolderHandle(Mutex<Folder>);

#[uniffi::export]
impl RemoteFolderHandle {
    #[uniffi::constructor]
    pub fn new(path: String) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Folder::new(&path))))
    }

    pub fn state(&self) -> RemoteFolderState {
        let f = lock(&self.0);
        RemoteFolderState {
            loading: f.loading(),
            failure: f.failure().map(str::to_owned),
            entries: f.entries().iter().map(Into::into).collect(),
            media: f.media().iter().map(Into::into).collect(),
            rows: f.rows().iter().map(Into::into).collect(),
            opening: f
                .entries()
                .iter()
                .filter_map(|e| Some((e.path.clone(), f.opening(&e.path)?)))
                .collect(),
            uploading: f.uploading(),
            open_failure: failure(f.open_failure()),
            edit_failure: failure(f.edit_failure()),
        }
    }

    pub fn load_started(&self) {
        lock(&self.0).load_started();
    }

    /// The host listed the folder: everything in it.
    pub fn listed(&self, listing: Vec<RemoteFileRecord>) {
        let listing: Vec<Entry> = listing.into_iter().map(Into::into).collect();
        lock(&self.0).listed(&listing);
    }

    pub fn list_failed(&self, why: String) {
        lock(&self.0).list_failed(&why);
    }

    /// What still wants a thumbnail, in order.
    pub fn thumbnail_queue(&self) -> Vec<RemoteFileRecord> {
        lock(&self.0)
            .thumbnail_queue()
            .iter()
            .map(Into::into)
            .collect()
    }

    pub fn thumbnail_arrived(&self, path: String) {
        lock(&self.0).thumbnail_arrived(&path);
    }

    pub fn cover_arrived(&self, path: String, seconds: f64) {
        lock(&self.0).cover_arrived(&path, seconds);
    }

    pub fn duration(&self, path: String) -> Option<f64> {
        lock(&self.0).duration(&path)
    }

    pub fn open_started(&self, path: String) {
        lock(&self.0).open_started(&path);
    }

    pub fn open_progress(&self, path: String, fraction: f64) {
        lock(&self.0).open_progress(&path, fraction);
    }

    pub fn open_finished(&self, path: String) {
        lock(&self.0).open_finished(&path);
    }

    pub fn new_folder_target(&self, raw: String) -> FolderNameTarget {
        target(lock(&self.0).new_folder_target(&raw))
    }

    pub fn rename_target(&self, entry: RemoteFileRecord, raw: String) -> FolderNameTarget {
        target(lock(&self.0).rename_target(&entry.into(), &raw))
    }

    /// Where an upload named `name` goes (never over a name there).
    pub fn upload_target(&self, name: String) -> String {
        lock(&self.0).upload_target(&name)
    }

    pub fn upload_started(&self) {
        lock(&self.0).upload_started();
    }

    pub fn upload_progress(&self, fraction: f64) {
        lock(&self.0).upload_progress(fraction);
    }

    pub fn upload_finished(&self) {
        lock(&self.0).upload_finished();
    }

    /// Why the file just tapped couldn't be opened; none clears it.
    pub fn set_open_failure(&self, message: Option<String>) {
        lock(&self.0).set_open_failure(message.as_deref());
    }

    /// A change that didn't go through; none clears it.
    pub fn set_edit_failure(&self, title: Option<String>, message: Option<String>) {
        let failure = title.zip(message);
        lock(&self.0).set_edit_failure(failure.as_ref().map(|(t, m)| (t.as_str(), m.as_str())));
    }
}

// MARK: - Image attachments

/// How the pane's agent best receives an image path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ImagePasteStyle {
    BarePath,
    AtMention,
    AiderAdd,
}

impl From<ImagePasteStyle> for InsertStyle {
    fn from(s: ImagePasteStyle) -> Self {
        match s {
            ImagePasteStyle::BarePath => InsertStyle::BarePath,
            ImagePasteStyle::AtMention => InsertStyle::AtMention,
            ImagePasteStyle::AiderAdd => InsertStyle::AiderAdd,
        }
    }
}

/// The style for the pane's agent (its hook stamp, else its command).
#[uniffi::export]
pub fn image_insert_style_for_agent(agent: Option<String>) -> ImagePasteStyle {
    match InsertStyle::for_agent(agent.as_deref()) {
        InsertStyle::BarePath => ImagePasteStyle::BarePath,
        InsertStyle::AtMention => ImagePasteStyle::AtMention,
        InsertStyle::AiderAdd => ImagePasteStyle::AiderAdd,
    }
}

/// The text pasted at the prompt for uploaded `paths`.
#[uniffi::export]
pub fn image_insert_text(paths: Vec<String>, style: ImagePasteStyle) -> String {
    image_attachment::insert_text(&paths, style.into())
}

/// Where an attachment flow is.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum AttachmentPhase {
    Processing,
    Uploading { fraction: f64 },
    Ready,
    Failed { why: String },
}

/// One picker-to-paste flow (`moshpit_app::image_attachment::Attachments`).
#[derive(uniffi::Object)]
pub struct ImageAttachmentsHandle(Mutex<Attachments>);

#[uniffi::export]
impl ImageAttachmentsHandle {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Attachments::new())))
    }

    pub fn phase(&self) -> AttachmentPhase {
        match lock(&self.0).phase() {
            Phase::Processing => AttachmentPhase::Processing,
            Phase::Uploading(fraction) => AttachmentPhase::Uploading {
                fraction: *fraction,
            },
            Phase::Ready => AttachmentPhase::Ready,
            Phase::Failed(why) => AttachmentPhase::Failed { why: why.clone() },
        }
    }

    /// Where each file landed, in the order picked.
    pub fn remote_paths(&self) -> Vec<Option<String>> {
        lock(&self.0).remote_paths().to_vec()
    }

    /// Each file's number in the session's log, once numbered.
    pub fn numbers(&self) -> Vec<Option<u64>> {
        lock(&self.0)
            .numbers()
            .iter()
            .map(|n| n.map(|n| n as u64))
            .collect()
    }

    /// The picked images are processed, these sizes in bytes.
    pub fn processed(&self, sizes: Vec<u64>) {
        lock(&self.0).processed(sizes);
    }

    /// The next file to upload, or none.
    pub fn next(&self) -> Option<u32> {
        lock(&self.0).next().and_then(|i| u32::try_from(i).ok())
    }

    /// File `index` is `fraction` of the way up; the overall fraction now.
    pub fn progress(&self, index: u32, fraction: f64) -> Option<f64> {
        lock(&self.0).progress(index as usize, fraction)
    }

    /// File `index` landed at `path`; true when it was the last.
    pub fn landed(&self, index: u32, path: String) -> bool {
        lock(&self.0).landed(index as usize, &path)
    }

    /// The files' numbers in the session's log, in the order picked.
    pub fn numbered(&self, numbers: Vec<u64>) {
        let numbers: Vec<usize> = numbers.into_iter().map(|n| n as usize).collect();
        lock(&self.0).numbered(&numbers);
    }

    pub fn failed(&self, why: String) {
        lock(&self.0).failed(&why);
    }

    /// Try again after a failure; false when there is nothing to retry.
    pub fn retry(&self) -> bool {
        lock(&self.0).retry()
    }

    /// The paste, none until every file landed.
    pub fn insert_text(&self, style: ImagePasteStyle) -> Option<String> {
        lock(&self.0).insert_text(style.into())
    }
}

// MARK: - Accent and theme editors

/// What a message from an editor's page did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LooksEditorOutcome {
    /// A change, made to the draft.
    Changed,
    /// Nothing the editor reads: nothing changed.
    Unknown,
    Save,
    Cancel,
    CopyJson,
}

fn outcome(handled: Handled) -> LooksEditorOutcome {
    match handled {
        Handled::Changed => LooksEditorOutcome::Changed,
        Handled::Unknown => LooksEditorOutcome::Unknown,
        Handled::Intent(EditorIntent::Save) => LooksEditorOutcome::Save,
        Handled::Intent(EditorIntent::Cancel) => LooksEditorOutcome::Cancel,
        Handled::Intent(EditorIntent::CopyJson) => LooksEditorOutcome::CopyJson,
    }
}

/// `{"op": "theme", …}` for `theme`, as JSON.
#[uniffi::export]
pub fn theme_editor_message(theme: ThemeRecord, override_bright: bool) -> String {
    ThemeEditor {
        draft: theme.into(),
        override_bright,
    }
    .message()
}

/// `{"op": "accent", …}` for an accent named `name`, as JSON.
#[uniffi::export]
pub fn accent_editor_message(name: String, accent_hex: String) -> String {
    AccentEditor {
        id: String::new(),
        name,
        accent_hex,
    }
    .message()
}

fn message_value(json: &str) -> serde_json::Value {
    serde_json::from_str(json).unwrap_or_default()
}

/// A custom accent's editor (`moshpit_app::looks::AccentEditor`).
#[derive(uniffi::Object)]
pub struct AccentEditorHandle(Mutex<AccentEditor>);

#[uniffi::export]
impl AccentEditorHandle {
    #[uniffi::constructor]
    pub fn new(accent: AccentThemeRecord) -> Arc<Self> {
        let accent = AppTheme::custom(&accent.id, &accent.name, &accent.accent_hex);
        Arc::new(Self(Mutex::new(AccentEditor::new(&accent))))
    }

    /// What the page told (JSON) did.
    pub fn handle(&self, message_json: String) -> LooksEditorOutcome {
        outcome(lock(&self.0).handle(&message_value(&message_json)))
    }

    /// The draft, its pressed shade and washes derived.
    pub fn accent(&self) -> AccentThemeRecord {
        (&lock(&self.0).accent()).into()
    }

    /// `{"op": "accent", …}`, as JSON.
    pub fn message(&self) -> String {
        lock(&self.0).message()
    }
}

/// A custom theme's editor (`moshpit_app::looks::ThemeEditor`).
#[derive(uniffi::Object)]
pub struct ThemeEditorHandle(Mutex<ThemeEditor>);

#[uniffi::export]
impl ThemeEditorHandle {
    #[uniffi::constructor]
    pub fn new(theme: ThemeRecord) -> Arc<Self> {
        Arc::new(Self(Mutex::new(ThemeEditor::new(theme.into()))))
    }

    /// What the page told (JSON) did.
    pub fn handle(&self, message_json: String) -> LooksEditorOutcome {
        outcome(lock(&self.0).handle(&message_value(&message_json)))
    }

    pub fn draft(&self) -> ThemeRecord {
        (&lock(&self.0).draft).into()
    }

    pub fn override_bright(&self) -> bool {
        lock(&self.0).override_bright
    }

    /// `{"op": "theme", …}`, as JSON.
    pub fn message(&self) -> String {
        lock(&self.0).message()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> ConnectionDraft {
        connection_draft_new(true)
    }

    #[test]
    fn a_draft_crosses_and_comes_back_as_it_was() {
        let mut d = record();
        d.name = "box".into();
        d.host = "h".into();
        d.selected_key = Some(Uuid::new_v4().to_string());
        d.auth = AuthMethod::Key;
        d.predict_mode_raw = "never".into();
        d.multiplexer_raw = "herdr".into();
        d.stored = StoredCredential::ManagedKey {
            id: Uuid::new_v4().to_string(),
        };
        let back: ConnectionDraft = (&draft(
            &d,
            SecretsGiven {
                password: true,
                key: false,
            },
        ))
            .into();
        assert_eq!(back, d);
    }

    #[test]
    fn a_message_handled_gives_the_next_draft_and_the_intent() {
        let d = record();
        let set = connection_draft_handle(
            d.clone(),
            r#"{"ev":"set","key":"name","value":"box"}"#.into(),
        );
        assert_eq!(set.draft.name, "box");
        assert_eq!(set.intent, None);
        let secret = connection_draft_handle(
            d.clone(),
            r#"{"ev":"set","key":"password","value":"x"}"#.into(),
        );
        assert_eq!(
            secret.draft, d,
            "a secret told through the screen is not taken"
        );
        let save = connection_draft_handle(d.clone(), r#"{"ev":"save"}"#.into());
        assert_eq!(save.intent, Some(ConnectionFormIntent::Save));
        assert_eq!(connection_draft_handle(d, "not json".into()).intent, None);
    }

    #[test]
    fn a_save_is_planned_or_refused() {
        let mut d = record();
        d.name = "box".into();
        d.host = "h".into();
        let fresh: ConnectionRecord =
            (&ServerConnection::new(Uuid::new_v4(), moshpit_seal::foundation::date::Date(0.0)))
                .into();
        let none = SecretsGiven {
            password: false,
            key: false,
        };
        assert_eq!(
            connection_draft_save_plan(
                d.clone(),
                None,
                fresh.clone(),
                vec![],
                none,
                1,
                2,
                "r".into()
            ),
            Err(ConnectionSaveRefusal::Incomplete)
        );
        let plan = connection_draft_save_plan(
            d,
            None,
            fresh.clone(),
            vec![],
            SecretsGiven {
                password: true,
                key: false,
            },
            1,
            2,
            "r".into(),
        )
        .unwrap();
        assert!(plan.added);
        assert_eq!(plan.write, Some(AuthMethod::Password));
        assert_eq!(plan.connection.keychain_ref.as_deref(), Some("r"));
        assert_eq!(plan.connection.id, fresh.id);
        assert_eq!(
            plan.connection.connection_protocol,
            crate::store::ConnectionProtocol::Mosh
        );
    }
}
