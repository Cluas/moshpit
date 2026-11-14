//! Add Connection and Edit Connection, the host's model: the draft the form
//! edits, Save's rules, and what saving does to the connection — its fields
//! and the credential it signs in with. One copy for the NSH screen on iOS
//! and Android and for the SwiftUI form iOS falls back to (W15d; it was
//! `ConnectionDraft.swift` and `ConnectionCredential.swift` on iOS and
//! `crates/android/src/app/form.rs` on Android).
//!
//! The password and the pasted private key are not here. They are typed
//! into the host's own fields (round 40, NSH-GAPS #11) and stay there; this
//! model hears only whether each holds one ([`Given`]) — a password that is
//! not empty, a key that is more than white space — and a save says which
//! secret the host still has to write, and where ([`SavePlan`]). The host
//! writes it (on iOS the pasted key converted first, `StoredKey`).
//!
//! The rules, each with the bug that made it:
//!
//! - A port is a whole number from 1 to 65535, empty its default
//!   ([`moshpit_ui::connection_form::port`]); Save waits for one (B294).
//! - Save needs a name and a host (more than white space; the host and the
//!   user are stored trimmed) and something to sign in with: a key picked
//!   that is still in SSH Keys, one pasted, or the one the connection
//!   edited keeps; a password typed, or the one it keeps (B294).
//! - A managed key's keychain item is never written or owned by a
//!   connection ([`apply_credential`]).
//! - A connection's own item holds the secret of the method it signs in
//!   with: switching the method takes the new method's secret, or is
//!   refused and changes nothing (B298).
//! - A new connection starts from the "Mosh by default" setting (B256).

use moshpit_seal::foundation::uuid::Uuid;
use moshpit_store::connections::{
    AuthMethod, ConnectionProtocol, Multiplexer, PredictMode, ServerConnection,
};
use moshpit_store::ssh_keys::SshKeyRecord;
pub use moshpit_ui::connection_form::SecretState;
use moshpit_ui::connection_form::{
    self as screen, Auth, DEFAULT_PORT, DEFAULT_PROXY_PORT, FormState, KeyChoice, Message, Mux,
    Secrets,
};
use moshpit_ui::settings::PredictMode as ScreenPredict;
use serde_json::Value;

/// What a saved connection keeps in the keychain for itself, which saving
/// with its field left empty keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Stored {
    #[default]
    None,
    /// A password of its own.
    Password,
    /// A key pasted into it (its own keychain item).
    PastedKey,
    /// A managed key from SSH Keys, by id (it points at the key's item).
    ManagedKey(Uuid),
}

impl Stored {
    /// What `c` keeps.
    pub fn of(c: &ServerConnection) -> Self {
        if c.keychain_ref.is_none() {
            return Stored::None;
        }
        match (c.auth_method, c.ssh_key_id) {
            (AuthMethod::Password, None) => Stored::Password,
            (AuthMethod::Password, Some(_)) => Stored::None,
            (AuthMethod::Key, None) => Stored::PastedKey,
            (AuthMethod::Key, Some(id)) => Stored::ManagedKey(id),
        }
    }
}

/// Which secrets the host's own fields hold now: what the model may know of
/// them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Given {
    /// The password field is not empty (white space is a password).
    pub password: bool,
    /// The private key editor holds more than white space.
    pub key: bool,
}

impl Given {
    /// Whether `pem` holds a pasted key: more than white space. The host
    /// asks it of its own text (it never comes here).
    pub fn holds_key(pem: &str) -> bool {
        !pem.trim().is_empty()
    }
}

/// What the form holds while it is open: the fields as the screen shows
/// them (its [`FormState`], with the given secrets' flags), the managed key
/// picked, and what the connection edited keeps.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// The fields. Its `key` row, `udp_range` and stored flags are the
    /// screen's view of the rest ([`Draft::shown`]); `secrets.password` and
    /// `secrets.pem` are what the host's fields hold.
    pub fields: FormState,
    /// The managed key picked; `None`: a PEM pasted.
    pub key: Option<Uuid>,
    /// What the connection edited keeps in the keychain for itself.
    pub stored: Stored,
}

/// What the screen asked of the host beyond an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Open the key picker.
    PickKey,
    Save,
    Cancel,
    /// Delete the connection edited (Delete connection…): the host asks
    /// first, as Home's own Delete does.
    Delete,
}

/// Why a save changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The sign-in method changes, the connection's own item holds the old
    /// method's secret, and no new one was given (B298).
    NewSecretNeeded(AuthMethod),
    /// No name, no host, a port that is not one, or nothing to sign in with
    /// ([`Draft::can_save`]): the screen's Save is off then, so only a
    /// caller that skipped it gets this.
    Incomplete,
}

impl Refusal {
    /// What the app says, in English (the iOS host says its own localized
    /// copy of it).
    pub fn message(self) -> &'static str {
        match self {
            Refusal::NewSecretNeeded(_) => NEW_SECRET_NEEDED,
            Refusal::Incomplete => "This connection can't be saved yet",
        }
    }
}

/// [`Refusal::NewSecretNeeded`]'s English.
pub const NEW_SECRET_NEEDED: &str =
    "To change how this connection signs in, enter the new password or key.";

/// What a save does: the connection to store, and the secret the host
/// still has to write to the keychain at its `keychain_ref`.
#[derive(Debug, Clone, PartialEq)]
pub struct SavePlan {
    pub connection: ServerConnection,
    /// The secret to write first (`Some(Password)`: the password typed;
    /// `Some(Key)`: the key pasted), or nothing new to write. Nothing is
    /// stored when the write fails.
    pub write: Option<AuthMethod>,
    /// A new connection (add it), else the one edited (update it).
    pub added: bool,
    /// The keychain item the connection edited owned and no longer names
    /// (it signs in with a managed key now, B325): the host drops it once
    /// the connection is saved, unless something else names it. It used to
    /// stay named by nothing: a password for good, a pasted key listed
    /// again as a "Recovered key".
    pub released: Option<String>,
    /// The connection edited now points at another host or port (B328):
    /// the push pairing it kept belongs to the box it pointed at before,
    /// and the host forgets it before saving, as a delete does.
    pub moved: bool,
}

/// Whether `after` points at another host or port than `before` (host
/// names compare as DNS does: case aside): the store's rule, which also
/// forgets what sessions remembered about the old box (B431).
pub fn moved(before: &ServerConnection, after: &ServerConnection) -> bool {
    after.points_elsewhere_than(before)
}

impl Draft {
    /// A new connection's draft, as the editor opens it: everything at its
    /// default, and mosh as the "Mosh by default" setting says (B256).
    pub fn new(mosh_by_default: bool) -> Self {
        Draft {
            fields: FormState {
                editing: false,
                port: DEFAULT_PORT.to_string(),
                auth: Auth::Password,
                mosh: mosh_by_default,
                predict_mode: ScreenPredict::Adaptive,
                roam_on_cellular: true,
                socks_port: DEFAULT_PROXY_PORT.to_string(),
                // tmux unless picked otherwise: every connection saved
                // before the picker existed ran tmux.
                mux: Mux::Tmux,
                ..FormState::default()
            },
            key: None,
            stored: Stored::None,
        }
    }

    /// The draft of saved connection `c`, as the editor opens it.
    pub fn of(c: &ServerConnection) -> Self {
        Draft {
            fields: FormState {
                editing: true,
                name: c.name.clone(),
                host: c.host.clone(),
                port: c.port.to_string(),
                username: c.username.clone(),
                auth: match c.auth_method {
                    AuthMethod::Password => Auth::Password,
                    AuthMethod::Key => Auth::Key,
                },
                secrets: Secrets::default(),
                key: None,
                mosh: c.connection_protocol == ConnectionProtocol::Mosh,
                udp_range: String::new(),
                predict_mode: screen_predict(c.predict_mode()),
                roam_on_cellular: c.roam_on_cellular.unwrap_or(true),
                mosh_server_path: c.mosh_server_path.clone().unwrap_or_default(),
                socks: c.use_socks_proxy.unwrap_or(false),
                socks_host: c.socks_proxy_host.clone().unwrap_or_default(),
                socks_port: c
                    .socks_proxy_port
                    .unwrap_or(i64::from(DEFAULT_PROXY_PORT))
                    .to_string(),
                mux: match c.multiplexer() {
                    Multiplexer::None => Mux::None,
                    Multiplexer::Tmux => Mux::Tmux,
                    Multiplexer::Herdr => Mux::Herdr,
                },
                tmux_path: c.tmux_path.clone().unwrap_or_default(),
                herdr_path: c.herdr_path.clone().unwrap_or_default(),
                emoji: c.emoji.clone().unwrap_or_default(),
            },
            key: c.ssh_key_id,
            stored: Stored::of(c),
        }
    }

    /// The sign-in method.
    pub fn auth_method(&self) -> AuthMethod {
        match self.fields.auth {
            Auth::Password => AuthMethod::Password,
            Auth::Key => AuthMethod::Key,
        }
    }

    /// What the host's secret fields hold, as far as the model knows.
    pub fn given(&self) -> Given {
        Given {
            password: self.fields.secrets.password,
            key: self.fields.secrets.pem,
        }
    }

    /// The host's secret fields hold `given` now.
    pub fn set_given(&mut self, given: Given) {
        self.fields.secrets.password = given.password;
        self.fields.secrets.pem = given.key;
    }

    /// Sets field `key` (the screen's [`moshpit_ui::connection_form::Field`]
    /// names) to `value`. False for a key or a value it does not know — the
    /// password and the private key among them: only the host's own fields
    /// hold those, never told through the screen.
    pub fn set(&mut self, key: &str, value: &Value) -> bool {
        let s = &mut self.fields;
        let text = value.as_str().map(str::to_owned);
        let flag = value.as_bool();
        match (key, text, flag) {
            ("name", Some(t), _) => s.name = t,
            ("host", Some(t), _) => s.host = t,
            ("port", Some(t), _) => s.port = t,
            ("username", Some(t), _) => s.username = t,
            ("moshServerPath", Some(t), _) => s.mosh_server_path = t,
            ("socksHost", Some(t), _) => s.socks_host = t,
            ("socksPort", Some(t), _) => s.socks_port = t,
            ("tmuxPath", Some(t), _) => s.tmux_path = t,
            ("herdrPath", Some(t), _) => s.herdr_path = t,
            ("emoji", Some(t), _) => s.emoji = t,
            ("mosh", _, Some(f)) => s.mosh = f,
            ("roamOnCellular", _, Some(f)) => s.roam_on_cellular = f,
            ("socks", _, Some(f)) => s.socks = f,
            ("auth", Some(t), _) => match t.as_str() {
                "password" => s.auth = Auth::Password,
                "key" => s.auth = Auth::Key,
                _ => return false,
            },
            ("predictMode", Some(t), _) => match PredictMode::from_raw(&t) {
                Some(mode) => s.predict_mode = screen_predict(mode),
                None => return false,
            },
            ("mux", Some(t), _) => match t.as_str() {
                "none" => s.mux = Mux::None,
                "tmux" => s.mux = Mux::Tmux,
                "herdr" => s.mux = Mux::Herdr,
                _ => return false,
            },
            _ => return false,
        }
        true
    }

    /// What the screen told (`{"ev": …}`): an edit made to the draft
    /// (`None`), or an intent. An edit the draft does not know is logged
    /// and changes nothing.
    pub fn handle(&mut self, message: &Value) -> Option<Intent> {
        match message["ev"].as_str() {
            Some("set") => {
                let key = message["key"].as_str().unwrap_or_default();
                if !self.set(key, &message["value"]) {
                    log::warn!(target: "moshpit", "connection form: no field {key:?}");
                }
                None
            }
            Some("pick-key") => Some(Intent::PickKey),
            Some("save") => Some(Intent::Save),
            Some("cancel") => Some(Intent::Cancel),
            Some("delete") => Some(Intent::Delete),
            _ => None,
        }
    }

    /// The key picker picked `id` (`None`: paste a PEM). A key picked signs
    /// in with a key. True when it changed the draft.
    pub fn pick_key(&mut self, id: Option<Uuid>) -> bool {
        let before = (self.key, self.fields.auth);
        self.key = id;
        if id.is_some() {
            self.fields.auth = Auth::Key;
        }
        before != (self.key, self.fields.auth)
    }

    /// The connection edited keeps a key it signs in with that saving with
    /// no key picked or pasted keeps: one pasted into it, or the managed key
    /// it points at while that is still the one picked.
    pub fn keeps_stored_key(&self) -> bool {
        match self.stored {
            Stored::PastedKey => true,
            Stored::ManagedKey(id) => Some(id) == self.key,
            Stored::None | Stored::Password => false,
        }
    }

    /// The connection edited keeps a password of its own, which saving with
    /// the field left empty keeps.
    pub fn keeps_stored_password(&self) -> bool {
        self.stored == Stored::Password
    }

    /// Where the password field stands: typed, kept by the connection
    /// edited, or neither (the redesign says "Saved" for a kept one).
    pub fn password_state(&self) -> SecretState {
        let mut secrets = self.fields.secrets;
        secrets.stored_password = self.keeps_stored_password();
        secrets.password_state()
    }

    /// Where the pasted key stands: pasted, kept by the connection edited
    /// (pasted into it, or the managed key it signs with still picked), or
    /// neither.
    pub fn key_state(&self) -> SecretState {
        let mut secrets = self.fields.secrets;
        secrets.stored_key = self.keeps_stored_key();
        secrets.key_state()
    }

    /// The key row: the key picked as SSH Keys has it now ("name · badge"),
    /// or none when none is picked or the one picked is gone (deleted in
    /// another window: the form shows "Paste PEM" then, B290).
    pub fn key_row(&self, keys: &[SshKeyRecord]) -> Option<KeyChoice> {
        let id = self.key?;
        let key = keys.iter().find(|k| k.id == id)?;
        Some(KeyChoice {
            id: id.to_string(),
            label: format!("{} · {}", key.name, key.badge_text()),
        })
    }

    /// The form as the screen shows it: the draft, with its key as `keys`
    /// have it, what it keeps already, and the UDP range from Settings.
    pub fn shown(&self, keys: &[SshKeyRecord], udp: (i64, i64)) -> FormState {
        let mut state = self.fields.clone();
        state.key = self.key_row(keys);
        state.secrets.stored_password = self.keeps_stored_password();
        state.secrets.stored_key = self.keeps_stored_key();
        state.udp_range = udp_range(udp);
        state
    }

    /// `{"op": "form", …}`: what the screen shows.
    pub fn message(&self, keys: &[SshKeyRecord], udp: (i64, i64)) -> String {
        Message::Form(Box::new(self.shown(keys, udp))).to_json()
    }

    /// The port field holds no port (the form says so under it).
    pub fn port_is_wrong(&self) -> bool {
        screen::port_is_wrong(&self.fields)
    }

    /// The proxy's port field holds no port, while the proxy is on.
    pub fn proxy_port_is_wrong(&self) -> bool {
        screen::proxy_port_is_wrong(&self.fields)
    }

    /// Saving leaves the connection something to sign in with: a key picked
    /// that is still in `keys`, one pasted, or the one the connection edited
    /// keeps; a password typed, or the one it keeps (B294).
    pub fn has_credential(&self, keys: &[SshKeyRecord]) -> bool {
        screen::has_credential(&self.shown(keys, (0, 0)))
    }

    /// It has a name and a host, its ports are ports, and saving it leaves
    /// the connection something to sign in with — the screen's Save
    /// ([`moshpit_ui::connection_form::can_save`]), `keys` being SSH Keys'
    /// now.
    pub fn can_save(&self, keys: &[SshKeyRecord]) -> bool {
        screen::can_save(&self.shown(keys, (0, 0)))
    }

    /// `connection` with the draft's fields (the ones saving writes), the
    /// UDP range being Settings' `udp`.
    pub fn applied(&self, connection: &ServerConnection, udp: (i64, i64)) -> ServerConnection {
        let s = &self.fields;
        let mut c = connection.clone();
        c.name = s.name.clone();
        // A pasted host or user brings spaces round it; the connect looks
        // them up as stored.
        c.host = s.host.trim().to_owned();
        let port = screen::port(&s.port, DEFAULT_PORT).unwrap_or(DEFAULT_PORT);
        c.port = port.into();
        c.ssh_port = port.into();
        c.username = s.username.trim().to_owned();
        c.auth_method = self.auth_method();
        c.connection_protocol = if s.mosh {
            ConnectionProtocol::Mosh
        } else {
            ConnectionProtocol::Ssh
        };
        c.mosh_port_range_start = udp.0;
        c.mosh_port_range_end = udp.1;
        // Empty: `mosh-server` on PATH, the capability probe intact.
        c.mosh_server_path = non_empty(&s.mosh_server_path);
        c.use_socks_proxy = Some(s.socks);
        // Spaces round a pasted address are not the proxy's (B511).
        c.socks_proxy_host = non_empty(s.socks_host.trim());
        c.socks_proxy_port = Some(
            screen::port(&s.socks_port, DEFAULT_PROXY_PORT)
                .unwrap_or(DEFAULT_PROXY_PORT)
                .into(),
        );
        c.set_predict_mode(store_predict(s.predict_mode));
        c.roam_on_cellular = Some(s.roam_on_cellular);
        c.tmux_path = non_empty(&s.tmux_path);
        c.herdr_path = non_empty(&s.herdr_path);
        // One emoji, trimmed; anything else (Save refuses it) none.
        c.emoji = screen::emoji(&s.emoji).ok().flatten();
        // Keeps the legacy `useTmux` flag in step too.
        c.set_multiplexer(match s.mux {
            Mux::None => Multiplexer::None,
            Mux::Tmux => Multiplexer::Tmux,
            Mux::Herdr => Multiplexer::Herdr,
        });
        c
    }

    /// What saving the draft does — over `existing`, or as `fresh` (a new
    /// connection with its id and date) — `keys` being SSH Keys' now and
    /// `udp` Settings' range. The credential first, on the connection as
    /// saved: its method is what its own item holds, which a switch of
    /// method must not keep ([`apply_credential`]); then the draft's
    /// fields. Refused, nothing is to be written.
    pub fn save_plan(
        &self,
        existing: Option<&ServerConnection>,
        fresh: impl FnOnce() -> ServerConnection,
        keys: &[SshKeyRecord],
        udp: (i64, i64),
        new_ref: impl FnOnce() -> String,
    ) -> Result<SavePlan, Refusal> {
        let added = existing.is_none();
        // The item the connection edited holds for itself (a managed key's
        // item is the key's).
        let owned = existing
            .filter(|c| c.ssh_key_id.is_none())
            .and_then(|c| c.keychain_ref.clone());
        let mut connection = existing.cloned().unwrap_or_else(fresh);
        let write = apply_credential(
            &mut connection,
            self.auth_method(),
            self.key,
            keys,
            self.given(),
            new_ref,
        )?;
        if !self.can_save(keys) {
            return Err(Refusal::Incomplete);
        }
        let connection = self.applied(&connection, udp);
        let released = owned.filter(|item| connection.keychain_ref.as_ref() != Some(item));
        let moved = existing.is_some_and(|before| moved(before, &connection));
        Ok(SavePlan {
            connection,
            write,
            added,
            released,
            moved,
        })
    }
}

/// Settings' UDP range as the mosh row shows it: "60000 – 61000", as on
/// iOS (Android's said "60000–61000", B299).
pub fn udp_range((low, high): (i64, i64)) -> String {
    format!("{low} – {high}")
}

/// Points `connection` — as saved: its `auth_method` says what its own item
/// holds — at its credential for `auth`, and sets that method. Returns the
/// secret the host still has to write at `connection.keychain_ref` (the
/// one `given` for `auth`), or `None` when there is nothing new to write.
///
/// A managed SSH key owns its keychain item, and every connection that
/// picked the key points at that same item. So a connection only ever
/// writes a secret to — or keeps holding — a ref it owns itself
/// (`ssh_key_id == None`): a password written into a managed key's ref
/// would overwrite the private key under every other connection using it,
/// and keeping that ref as its own would let deleting the connection delete
/// the key.
///
/// A connection's own item holds the secret of the method it signs in with,
/// and the connect path reads it as that method's. So an item never stays
/// under another method: switching the method takes the new method's
/// secret, which replaces what the item held, or is refused
/// ([`Refusal::NewSecretNeeded`]) and changes nothing — nothing is deleted.
pub fn apply_credential(
    connection: &mut ServerConnection,
    auth: AuthMethod,
    selected_key: Option<Uuid>,
    keys: &[SshKeyRecord],
    given: Given,
    new_ref: impl FnOnce() -> String,
) -> Result<Option<AuthMethod>, Refusal> {
    let mut applied = connection.clone();
    let write = point(&mut applied, auth, selected_key, keys, given, new_ref)?;
    *connection = applied;
    Ok(write)
}

fn point(
    connection: &mut ServerConnection,
    auth: AuthMethod,
    selected_key: Option<Uuid>,
    keys: &[SshKeyRecord],
    given: Given,
    new_ref: impl FnOnce() -> String,
) -> Result<Option<AuthMethod>, Refusal> {
    // What its own item holds: the method it signed in with until now.
    let before = connection.auth_method;
    connection.auth_method = auth;
    if auth == AuthMethod::Key
        && let Some(id) = selected_key
    {
        if let Some(key) = keys.iter().find(|k| k.id == id) {
            // A managed key: point at the key's own item; the algorithm
            // tells the connect path how to read it (a Secure Enclave key
            // is a handle, not a PEM).
            connection.keychain_ref = key.keychain_ref.clone();
            connection.ssh_key_id = Some(key.id);
            connection.ssh_key_algorithm_raw = Some(key.algorithm.raw_value().to_owned());
            return Ok(None);
        }
        if Some(id) == connection.ssh_key_id && !given.key {
            // Deleted from SSH Keys since: the record is gone but its item
            // is not, and this connection still signs with it. Nothing new
            // was pasted, so it stays pointed there.
            return Ok(None);
        }
    }
    let owns_ref = connection.ssh_key_id.is_none();
    connection.ssh_key_id = None;
    connection.ssh_key_algorithm_raw = None;
    if !owns_ref {
        connection.keychain_ref = None;
    }
    // White space is no key: what the connection keeps stays (B294).
    let has_secret = match auth {
        AuthMethod::Password => given.password,
        AuthMethod::Key => given.key,
    };
    if !has_secret {
        // Keep what the connection stored for itself — if it is this
        // method's.
        if owns_ref && connection.keychain_ref.is_some() && before != auth {
            return Err(Refusal::NewSecretNeeded(auth));
        }
        return Ok(None);
    }
    if connection.keychain_ref.is_none() {
        connection.keychain_ref = Some(new_ref());
    }
    Ok(Some(auth))
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

fn screen_predict(mode: PredictMode) -> ScreenPredict {
    match mode {
        PredictMode::Adaptive => ScreenPredict::Adaptive,
        PredictMode::Always => ScreenPredict::Always,
        PredictMode::Never => ScreenPredict::Never,
        PredictMode::Experimental => ScreenPredict::Experimental,
    }
}

fn store_predict(mode: ScreenPredict) -> PredictMode {
    PredictMode::from_raw(mode.id()).unwrap_or(PredictMode::Adaptive)
}

#[cfg(test)]
mod tests;
