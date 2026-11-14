//! SSH keys' host side (`SSHKeysNSHPage.swift`, `AddKeyNSHPage.swift`,
//! `KeyPickerNSHPage.swift`): the key list, making and importing a key, and
//! picking the key a connection signs in with.
//!
//! Key material stays out of the screens and the channel: the private key
//! and its passphrase are typed into the host's own text fields
//! (`moshpit/key-pem`, `moshpit/key-passphrase`), which hand them to
//! [`App::key_field`] and nowhere else. Making or importing a key goes
//! through `moshpit_ssh::keyfile` (the one path every platform shares); the
//! private half goes to the keychain (wrapped by a Keystore key), or — for a
//! hardware key — never leaves the Android Keystore at all.

use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::uuid::Uuid;
use moshpit_ssh::keyfile::{self, Generate};
use moshpit_store::host::new_keychain_ref;
use moshpit_store::secrets::{self, KeyDeletion};
use moshpit_store::ssh_keys::{SshKeyAlgorithm, SshKeyRecord, SshKeySource};
use moshpit_ui::add_key::{AddKey, Algorithm, Method, Pem};
use moshpit_ui::key_picker::{Picker, PickerKey};
use moshpit_ui::ssh_keys::{Avatar, Key, Keys, Notice};
use serde_json::Value;

use crate::app::{App, Core, PageKind, text, uuid};

/// What the Add Key form holds (`AddKeyModel`).
#[derive(Default)]
pub struct AddKeyDraft {
    pub method: Option<Method>,
    pub algorithm: Option<Algorithm>,
    pub require_face_id: bool,
    pub secure_enclave: bool,
    pub name: String,
    pub comment: String,
    pub public_line: String,
    pub bound: Vec<String>,
    /// The private key as pasted, and its passphrase: from the host's
    /// fields, never posted.
    pem: Secret,
    passphrase: Secret,
    pub fields: u64,
    pub working: bool,
    pub fingerprint: Option<String>,
    pub error: Option<String>,
    /// Opened from the key picker: the new key becomes the form's.
    pub for_picker: bool,
}

/// Text wiped when dropped or replaced.
#[derive(Default)]
struct Secret(String);

impl Secret {
    fn set(&mut self, text: String) {
        self.wipe();
        self.0 = text;
    }

    fn wipe(&mut self) {
        // SAFETY: zeroes are valid UTF-8.
        unsafe { self.0.as_bytes_mut() }.fill(0);
        self.0.clear();
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl AddKeyDraft {
    pub fn new(for_picker: bool) -> Self {
        AddKeyDraft {
            method: Some(Method::Generate),
            algorithm: Some(Algorithm::Ed25519),
            require_face_id: false,
            fields: 1,
            for_picker,
            ..Default::default()
        }
    }

    fn method(&self) -> Method {
        self.method.unwrap_or(Method::Generate)
    }

    fn needs_passphrase(&self) -> bool {
        keyfile::is_encrypted(&self.pem.0)
    }

    /// As the iOS page decides: a name, and for Import a key (and its
    /// passphrase when it needs one); never while working.
    pub fn can_submit(&self) -> bool {
        if self.name.is_empty() || self.working {
            return false;
        }
        match self.method() {
            Method::Generate => true,
            Method::Import => {
                !self.pem.0.trim().is_empty()
                    && (!self.needs_passphrase() || !self.passphrase.0.is_empty())
            }
            Method::Hardware => false,
        }
    }
}

// MARK: - SSH Keys

fn key_row(record: &SshKeyRecord) -> Key {
    Key {
        id: record.id.to_string(),
        name: record.name.clone(),
        badge: badge(record),
        avatar: if record.source == SshKeySource::SecureEnclave {
            Avatar::SecureEnclave
        } else {
            Avatar::Imported
        },
        stale: false,
        fingerprint: record.fingerprint.clone(),
        hosts: record.bound_hosts.clone(),
        unused_since: None,
        has_public_key: !record.public_key.is_empty(),
    }
}

/// The badge: the Android Keystore's keys say so (`KS · P256`), the rest
/// as on iOS.
fn badge(record: &SshKeyRecord) -> String {
    if record.source == SshKeySource::SecureEnclave {
        "KS · P256".to_owned()
    } else {
        record.badge_text()
    }
}

/// `{"op": "keys", …}` for SSH Keys.
pub fn keys_message(app: &App, deleting: Option<Uuid>, notice: Option<Notice>) -> String {
    let keys = Keys {
        device: app.keys.device_keys().into_iter().map(key_row).collect(),
        imported: app.keys.imported_keys().into_iter().map(key_row).collect(),
        deleting: deleting.map(|id| id.to_string()),
        notice,
    };
    moshpit_ui::ssh_keys::Message::Keys(keys).to_json()
}

pub fn on_keys_event(app: &mut App, core: &Core, surface: u64, ev: &str, event: &Value) {
    let id = uuid(event, "id");
    let record = id.and_then(|id| app.keys.keys().iter().find(|k| k.id == id).cloned());
    match ev {
        "done" => app.close(surface),
        "add" => {
            // The bar's + and Generate key open on Generate; Import key on
            // Import.
            let mut draft = AddKeyDraft::new(false);
            if event.get("method").and_then(Value::as_str) == Some("import") {
                draft.method = Some(Method::Import);
            }
            app.open(core, PageKind::AddKey(Box::new(draft)), true);
        }
        "copy-public-key" => {
            if let Some(record) = record {
                app.platform.copy_text(&record.public_key);
            }
        }
        "copy-fingerprint" => {
            if let Some(record) = record {
                app.platform.copy_text(&record.fingerprint);
            }
        }
        "share-public-key" => {
            if let Some(record) = record {
                app.platform.share_text(&record.public_key);
            }
        }
        "confirm-delete" | "delete" | "cancel-delete" | "dismiss-notice" => {
            let notice = match (ev, &record) {
                ("delete", Some(record)) => delete_key(app, record),
                _ => None,
            };
            if let Some(page) = app.page_mut(surface)
                && let PageKind::SshKeys {
                    deleting: shown,
                    notice: said,
                } = &mut page.kind
            {
                *shown = if ev == "confirm-delete" { id } else { None };
                if ev != "confirm-delete" {
                    *said = notice;
                }
            }
            app.refresh();
        }
        _ => {}
    }
}

/// Deletes a key the shared way (`moshpit_store::secrets::delete_key`, B232):
/// its private half first, then its record — unless a saved connection
/// still signs in with it, or the keychain keeps it. A Keystore key's own
/// key goes with it. What to tell the person when it was kept.
pub fn delete_key(app: &mut App, record: &SshKeyRecord) -> Option<Notice> {
    let keychain = app.keychain.clone();
    let deletion = secrets::delete_key(
        record.id,
        &mut app.keys,
        &mut app.connections,
        keychain.as_ref(),
    );
    match deletion {
        KeyDeletion::Deleted | KeyDeletion::NotFound => {
            if record.source == SshKeySource::SecureEnclave
                && let Some(alias) = record.keychain_ref.as_deref()
            {
                app.platform.hardware_delete(alias);
            }
            None
        }
        KeyDeletion::InUse(names) => Some(Notice {
            title: "Key kept".to_owned(),
            message: format!(
                "{} still sign in with \u{201c}{}\u{201d}. Pick another key for them first.",
                names.join(", "),
                record.name
            ),
        }),
        KeyDeletion::Unknown => Some(Notice {
            title: "Key kept".to_owned(),
            message: "Which connections use it can't be told right now. Try again in a moment."
                .to_owned(),
        }),
        KeyDeletion::Keychain(error) => Some(Notice {
            title: "Key kept".to_owned(),
            message: format!("The keychain kept its private key: {error}"),
        }),
    }
}

// MARK: - Add Key

/// `{"op": "add-key", …}`.
pub fn add_key_message(app: &App, draft: &AddKeyDraft) -> String {
    let form = AddKey {
        method: draft.method(),
        algorithm: draft.algorithm.unwrap_or(Algorithm::Ed25519),
        require_face_id: draft.require_face_id,
        secure_enclave: draft.secure_enclave,
        hosts: app
            .connections
            .connections()
            .iter()
            .map(|c| c.display_name())
            .collect(),
        bound: draft.bound.clone(),
        pem: Pem {
            empty: draft.pem.0.is_empty(),
            encrypted: draft.needs_passphrase(),
        },
        name: draft.name.clone(),
        comment: draft.comment.clone(),
        public_line: draft.public_line.clone(),
        fields: draft.fields,
        can_submit: draft.can_submit(),
        working: draft.working,
        fingerprint: draft.fingerprint.clone(),
        error: draft.error.clone(),
    };
    moshpit_ui::add_key::Message::AddKey(form).to_json()
}

pub fn on_add_key_event(app: &mut App, core: &Core, surface: u64, ev: &str, event: &Value) {
    let value = text(event, "value");
    let Some(PageKind::AddKey(draft)) = app.page_mut(surface).map(|p| &mut p.kind) else {
        return;
    };
    match ev {
        "method" => {
            draft.method = match value.as_deref() {
                Some("import") => Some(Method::Import),
                Some("hardware") => Some(Method::Hardware),
                _ => Some(Method::Generate),
            }
        }
        "algorithm" => {
            draft.algorithm = match value.as_deref() {
                Some("rsa") => Some(Algorithm::Rsa),
                _ => Some(Algorithm::Ed25519),
            }
        }
        "set" => {
            let on = event["value"].as_bool().unwrap_or(false);
            match event["key"].as_str() {
                Some("requireFaceID") => draft.require_face_id = on,
                Some("secureEnclave") => draft.secure_enclave = on,
                _ => {}
            }
        }
        "name" => draft.name = value.unwrap_or_default(),
        "comment" => draft.comment = value.unwrap_or_default(),
        "public-line" => draft.public_line = value.unwrap_or_default(),
        "host" => {
            if let Some(name) = text(event, "name") {
                if let Some(i) = draft.bound.iter().position(|h| *h == name) {
                    draft.bound.remove(i);
                } else {
                    draft.bound.push(name);
                }
            }
        }
        "dismiss-error" => draft.error = None,
        "import-file" => {
            // The system's document picker (the Storage Access Framework):
            // the file comes back through `Core::picked`.
            let public = event["target"].as_str() == Some("public");
            app.pick_file(super::Pick::KeyFile { surface, public });
            return;
        }
        "cancel" => {
            app.close(surface);
            return;
        }
        "submit" => {
            submit(app, core, surface);
            return;
        }
        _ => return,
    }
    app.refresh_page_public(surface);
}

impl App {
    /// The private key or its passphrase, as typed into the host's field
    /// (`kind`: `pem` or `passphrase`) on the Add Key page on top; or the
    /// password or the pasted key (`connection-password`,
    /// `connection-pem`) on the connection form on top, which hears only
    /// whether its field holds anything (from the field itself).
    pub fn key_field(&mut self, kind: &str, text: String) {
        if kind.starts_with("connection-") {
            if let Some(PageKind::ConnectionForm(draft)) = self
                .pages
                .iter_mut()
                .rev()
                .map(|p| &mut p.kind)
                .find(|k| matches!(k, PageKind::ConnectionForm(_)))
            {
                draft.set_secret(kind, text);
            }
            return;
        }
        let Some(page) = self
            .pages
            .iter_mut()
            .rev()
            .find(|p| matches!(p.kind, PageKind::AddKey(_)))
        else {
            return;
        };
        let surface = page.surface;
        let PageKind::AddKey(draft) = &mut page.kind else {
            return;
        };
        match kind {
            "pem" => draft.pem.set(text),
            "passphrase" => draft.passphrase.set(text),
            _ => return,
        }
        self.refresh_page_public(surface);
    }

    /// A key file the person picked for Add Key's private key (its field
    /// shows it) or public key line: read as UTF-8 text, as iOS reads it
    /// (`SSHKeyFactory.decodeImportedText`); why not, on the page's card.
    pub(crate) fn key_file_picked(&mut self, surface: u64, public: bool, bytes: Vec<u8>) {
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(not_text) => {
                let mut bytes = not_text.into_bytes();
                bytes.fill(0);
                if let Some(PageKind::AddKey(draft)) = self.page_mut(surface).map(|p| &mut p.kind) {
                    draft.error = Some("Unsupported: file is not valid UTF-8 text".to_owned());
                }
                self.refresh_page_public(surface);
                return;
            }
        };
        let platform = self.platform.clone();
        let Some(PageKind::AddKey(draft)) = self.page_mut(surface).map(|p| &mut p.kind) else {
            return;
        };
        if public {
            draft.public_line = text;
            draft.fields += 1;
        } else {
            // The host's field shows it and tells it back (`keyField`);
            // kept here at once, so a submit before that round trip has it.
            platform.fill_key_field("pem", &text);
            draft.pem.set(text);
        }
        self.refresh_page_public(surface);
    }

    pub(crate) fn refresh_page_public(&mut self, surface: u64) {
        let json = match self
            .pages
            .iter()
            .find(|p| p.surface == surface)
            .map(|p| &p.kind)
        {
            Some(PageKind::AddKey(draft)) => add_key_message(self, draft),
            _ => return,
        };
        self.post(surface, json);
    }
}

/// Makes or imports the key the form describes, keeps it, and closes the
/// form; or shows why not.
fn submit(app: &mut App, core: &Core, surface: u64) {
    let platform = app.platform.clone();
    let Some(PageKind::AddKey(draft)) = app.page_mut(surface).map(|p| &mut p.kind) else {
        return;
    };
    if !draft.can_submit() {
        return;
    }
    let made = make(platform, draft);
    let Some(PageKind::AddKey(draft)) = app.page_mut(surface).map(|p| &mut p.kind) else {
        return;
    };
    let (record, secret) = match made {
        Ok(made) => made,
        Err(why) => {
            draft.error = Some(why);
            app.refresh_page_public(surface);
            return;
        }
    };
    let for_picker = draft.for_picker;
    if let Some(secret) = secret {
        let account = record.keychain_ref.clone().unwrap_or_default();
        if let Err(why) = app
            .keychain
            .save(&account, secret.as_bytes(), record.require_biometry)
        {
            if let Some(PageKind::AddKey(draft)) = app.page_mut(surface).map(|p| &mut p.kind) {
                draft.error = Some(why.to_string());
            }
            app.refresh_page_public(surface);
            return;
        }
    }
    log::info!(target: "moshpit", "key {}: {} {}", record.id, record.badge_text(), record.fingerprint);
    let id = record.id;
    app.keys.add(record);
    app.close(surface);
    if for_picker {
        // The new key is the connection's, as on iOS.
        if let Some(form) = app.pages.iter_mut().rev().find_map(|p| match &mut p.kind {
            PageKind::ConnectionForm(draft) => Some(draft),
            _ => None,
        }) {
            form.model.pick_key(Some(id));
        }
        repost_forms(app);
    }
    app.refresh();
    let _ = core;
}

/// The record of the key the draft describes, and the secret to keep under
/// its keychain ref (`None` for a hardware key, which stays in the Keystore).
fn make(
    platform: std::sync::Arc<dyn crate::platform::Platform>,
    draft: &AddKeyDraft,
) -> Result<(SshKeyRecord, Option<zeroize_text::Text>), String> {
    let id = Uuid::new_v4();
    // As on iOS (`SSHKeyCreation`): a made key's comment is the one typed,
    // else its name; an imported key's is the one typed, else the pasted
    // `.pub` line's, else its own.
    let made_comment = if draft.comment.is_empty() {
        draft.name.clone()
    } else {
        draft.comment.clone()
    };
    // The shared reader's own words for why not.
    let explain = |error: keyfile::ImportError| error.to_string();
    if draft.method() == Method::Generate && draft.secure_enclave {
        let alias = format!("moshpit.hw.{}", id.to_string().to_lowercase());
        let public = platform.hardware_generate(&alias)?;
        let (line, fingerprint) =
            keyfile::hardware_public_key(&public, &made_comment).map_err(explain)?;
        let mut record = SshKeyRecord::new(
            id,
            &draft.name,
            SshKeyAlgorithm::SeP256,
            SshKeySource::SecureEnclave,
            Date::now(),
        );
        record.comment = draft.comment.clone();
        record.fingerprint = fingerprint;
        record.public_key = line;
        record.bound_hosts = draft.bound.clone();
        record.keychain_ref = Some(alias);
        record.require_biometry = draft.require_face_id;
        return Ok((record, None));
    }
    let (stored, source) = match draft.method() {
        Method::Import => (
            keyfile::import(
                &draft.pem.0,
                (!draft.passphrase.0.is_empty()).then_some(draft.passphrase.0.as_str()),
                (!draft.public_line.is_empty()).then_some(draft.public_line.as_str()),
                &draft.comment,
            )
            .map_err(explain)?,
            SshKeySource::Imported,
        ),
        _ => {
            let kind = match draft.algorithm.unwrap_or(Algorithm::Ed25519) {
                Algorithm::Ed25519 => Generate::Ed25519,
                Algorithm::Rsa => Generate::Rsa(4096),
            };
            (
                keyfile::generate(kind, &made_comment).map_err(explain)?,
                SshKeySource::Generated,
            )
        }
    };
    let mut record = SshKeyRecord::new(
        id,
        &draft.name,
        stored.record_algorithm(),
        source,
        Date::now(),
    );
    record.bits = stored.bits;
    record.comment = draft.comment.clone();
    record.fingerprint = stored.info.fingerprint.clone();
    record.public_key = stored.info.public_key.clone();
    record.bound_hosts = draft.bound.clone();
    record.keychain_ref = Some(new_keychain_ref());
    record.require_biometry = draft.require_face_id;
    Ok((record, Some(zeroize_text::Text(stored.openssh.to_string()))))
}

mod zeroize_text {
    //! The key text on its way to the keychain, wiped when dropped.
    pub struct Text(pub String);

    impl Text {
        pub fn as_bytes(&self) -> &[u8] {
            self.0.as_bytes()
        }
    }

    impl Drop for Text {
        fn drop(&mut self) {
            // SAFETY: zeroes are valid UTF-8.
            unsafe { self.0.as_bytes_mut() }.fill(0);
        }
    }
}

// MARK: - Key picker

/// The connection form the picker picks for: the topmost one.
fn form_key(app: &App) -> Option<Uuid> {
    app.pages.iter().rev().find_map(|p| match &p.kind {
        PageKind::ConnectionForm(draft) => Some(draft.model.key),
        _ => None,
    })?
}

/// `{"op": "keys", …}` for the picker.
pub fn picker_message(app: &App) -> String {
    let selected = form_key(app);
    let picker = Picker {
        keys: app
            .keys
            .keys()
            .iter()
            .map(|k| PickerKey {
                id: k.id.to_string(),
                name: k.name.clone(),
                badge: badge(k),
                refused: keyfile::refused_type(&k.public_key).or_else(|| {
                    (k.algorithm == SshKeyAlgorithm::EcdsaSk
                        && !k.public_key.starts_with("ecdsa-sha2-"))
                    .then(|| "security key".to_owned())
                }),
            })
            .collect(),
        selected: selected.map(|id| id.to_string()),
    };
    moshpit_ui::key_picker::Message::Keys(picker).to_json()
}

pub fn on_picker_event(app: &mut App, core: &Core, surface: u64, ev: &str, event: &Value) {
    match ev {
        "select" => {
            let picked = uuid(event, "id");
            if let Some(form) = app.pages.iter_mut().rev().find_map(|p| match &mut p.kind {
                PageKind::ConnectionForm(draft) => Some(draft),
                _ => None,
            }) {
                form.model.pick_key(picked);
            }
            app.close(surface);
            repost_forms(app);
        }
        "add" => {
            app.open(
                core,
                PageKind::AddKey(Box::new(AddKeyDraft::new(true))),
                true,
            );
        }
        "cancel" => app.close(surface),
        _ => {}
    }
}

/// The connection forms, posted their drafts again (a key picked).
fn repost_forms(app: &App) {
    for page in &app.pages {
        if let PageKind::ConnectionForm(draft) = &page.kind {
            app.post(page.surface, crate::app::form::message(app, draft));
        }
    }
}
