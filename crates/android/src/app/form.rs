//! Add Connection and Edit Connection, the Android host's side: the shared
//! model (`moshpit_app::connection_form`: the draft, Save's rules, the
//! credential a save points the connection at) and what only this host
//! keeps — the password and the pasted key, typed into the platform's own
//! fields (`moshpit/connection-password`, `moshpit/connection-pem`), which
//! hand them to [`App::key_field`]. They are held here and written to the
//! keychain here, never posted to the screen or taken from it (NSH-GAPS
//! #11); the shared model hears only whether each holds one.

use moshpit_app::connection_form::{
    Draft, Given, Intent, SavePlan, others, via_choices, via_users,
};
use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::uuid::Uuid;
use moshpit_store::connections::{AuthMethod, ServerConnection};
use moshpit_store::host::new_keychain_ref;
use moshpit_ui::connection_form::{Auth, Message, SaveFailure};
use serde_json::Value;

use crate::app::{App, Core, PageKind};

/// What the form holds while it is open.
#[derive(Debug, Clone, PartialEq)]
pub struct FormDraft {
    /// The connection edited; `None`: a new one.
    pub editing: Option<Uuid>,
    /// The shared model's draft.
    pub model: Draft,
    /// The password, as its field holds it (never in `model`).
    pub password: String,
    /// The pasted private key, as its editor holds it (never in `model`).
    pub pem: String,
}

impl FormDraft {
    /// A new connection's draft, with the app's defaults.
    pub fn new(app: &App) -> Self {
        FormDraft {
            editing: None,
            model: Draft::new(app.settings_flag("moshByDefault")),
            password: String::new(),
            pem: String::new(),
        }
    }

    /// The draft of a saved connection, as the editor opens it.
    pub fn editing(_app: &App, c: &ServerConnection) -> Self {
        FormDraft {
            editing: Some(c.id),
            model: Draft::of(c),
            password: String::new(),
            pem: String::new(),
        }
    }

    /// Sets field `key` to `value` ([`Draft::set`]).
    pub fn set(&mut self, key: &str, value: &Value) -> bool {
        self.model.set(key, value)
    }

    /// The password (`connection-password`) or the pasted key
    /// (`connection-pem`) as its field holds it now; false for another kind.
    pub fn set_secret(&mut self, kind: &str, text: String) -> bool {
        match kind {
            "connection-password" => self.password = text,
            "connection-pem" => self.pem = text,
            _ => return false,
        }
        self.model.set_given(Given {
            password: !self.password.is_empty(),
            key: Given::holds_key(&self.pem),
        });
        true
    }
}

/// `{"op": "form", …}`.
pub fn message(app: &App, draft: &FormDraft) -> String {
    let mut model = draft.model.clone();
    // The others, for the warning of a duplicate (B532).
    let saved = app.connections.connections();
    model.fields.others = others(saved, draft.editing);
    // What it may go through, and who goes through it (F1).
    model.fields.via_choices = via_choices(saved, draft.editing);
    model.fields.via_users = via_users(saved, draft.editing);
    model.message(app.keys.keys(), app.settings.udp_range())
}

/// What the form told the host.
pub fn on_event(app: &mut App, core: &Core, surface: u64, _ev: &str, event: &Value) {
    let Some(PageKind::ConnectionForm(draft)) = app.page_mut(surface).map(|p| &mut p.kind) else {
        return;
    };
    let editing = draft.editing;
    match draft.model.handle(event) {
        None => {}
        Some(Intent::PickKey) => {
            app.open(core, PageKind::KeyPicker, true);
        }
        Some(Intent::Save) => {
            let draft = (**draft).clone();
            match save(app, &draft) {
                Ok(()) => {
                    app.close(surface);
                    app.refresh();
                }
                // The form keeps what it holds and says why under its bar.
                Err(why) => {
                    app.post(surface, Message::Failed(why).to_json());
                }
            }
        }
        Some(Intent::Cancel) => app.close(surface),
        Some(Intent::Delete) => {
            if let Some(id) = editing {
                app.close(surface);
                super::home::ask_delete(app, id);
            }
        }
    }
}

/// Saves `draft` (the shared [`Draft::save_plan`]): the credential to the
/// keychain, the connection to the store. Nothing is saved when the plan is
/// refused or the keychain fails; the form says which ([`SaveFailure`]).
pub fn save(app: &mut App, draft: &FormDraft) -> Result<(), SaveFailure> {
    let existing = draft.editing.and_then(|id| {
        app.connections
            .connections()
            .iter()
            .find(|c| c.id == id)
            .cloned()
    });
    let SavePlan {
        connection,
        write,
        added,
        released,
        // Android pairs nothing for push (B328 is iOS's).
        moved: _,
    } = draft
        .model
        .save_plan(
            existing.as_ref(),
            || ServerConnection::new(Uuid::new_v4(), Date::now()),
            app.keys.keys(),
            app.settings.udp_range(),
            new_keychain_ref,
        )
        .map_err(|refusal| SaveFailure {
            secret: None,
            reason: refusal.message().to_owned(),
            retry: false,
        })?;
    if let Some(method) = write
        && let Some(account) = connection.keychain_ref.clone()
    {
        let secret = match method {
            AuthMethod::Password => draft.password.clone(),
            AuthMethod::Key => storable(&draft.pem),
        };
        app.keychain
            .save(&account, secret.as_bytes(), false)
            .map_err(|e| SaveFailure {
                secret: Some(match method {
                    AuthMethod::Password => Auth::Password,
                    AuthMethod::Key => Auth::Key,
                }),
                reason: e.to_string(),
                retry: true,
            })?;
    }
    let id = connection.id;
    if added {
        app.connections.add(connection);
    } else {
        app.connections.update(connection);
        app.respec_through(id);
    }
    // B325: the item it no longer names goes — unless its session is
    // running, which signs in again with the item it started with.
    if let Some(account) = released
        && app.sessions.conn(id).is_none()
    {
        let fate = moshpit_store::secrets::release_item(
            &account,
            &mut app.connections,
            &mut app.keys,
            app.keychain.as_ref(),
        );
        log::info!(target: "moshpit", "connection save: its old item {fate:?}");
    }
    Ok(())
}

/// A pasted key in the form a connection signs with, when it reads without
/// a passphrase; otherwise as pasted (`moshpit_ssh::keyfile::storable`, as
/// iOS's `ConnectionSecrets.storable`).
fn storable(pasted: &str) -> String {
    moshpit_ssh::keyfile::storable(pasted).to_string()
}

impl App {
    /// A Boolean setting, by its name in `AppSettings`.
    pub(crate) fn settings_flag(&self, name: &str) -> bool {
        super::settings::spec(name).is_some_and(|spec| {
            matches!(
                self.settings.get(spec),
                moshpit_store::settings::SettingValue::Bool(true)
            )
        })
    }
}
