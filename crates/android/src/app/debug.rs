//! The test hooks of a debuggable build (`Core.debug(json)`, which the
//! host's debug receiver calls for `adb shell am broadcast`): the
//! emulator runs (android/test.sh, android/live.sh) drive the app through
//! them where tapping a screen would only make a test flaky — adding a
//! connection, importing a key, breaking a link — and through the same code
//! paths the screens use (the form's save, Add Key's submit).
//!
//! A release build answers nothing: `Options::debuggable` is the host's
//! `FLAG_DEBUGGABLE`.

use std::sync::Arc;

use serde_json::{Value, json};

use moshpit_seal::foundation::uuid::Uuid;
use moshpit_session::SessionState;

use crate::app::form::FormDraft;
use crate::app::keys::AddKeyDraft;
use crate::app::{App, Core, PageKind, uuid};

impl Core {
    /// Runs one hook; its answer as JSON (`{"error": …}` when it can't).
    pub fn debug(self: &Arc<Self>, json: &str) -> String {
        if !self.lock().options.debuggable {
            return json!({"error": "not a debuggable build"}).to_string();
        }
        let Ok(command) = serde_json::from_str::<Value>(json) else {
            return json!({"error": "not JSON"}).to_string();
        };
        let answer = self.run(&command);
        log::info!(target: "moshpit", "debug {}: {answer}", command["cmd"]);
        answer.to_string()
    }

    fn run(self: &Arc<Self>, c: &Value) -> Value {
        let cmd = c["cmd"].as_str().unwrap_or_default();
        let connection = uuid(c, "connection");
        match cmd {
            "stack" => json!({"stack": self.lock().stack()}),
            "back" => json!({"took": self.back()}),
            "open" => {
                let page = c["page"].as_str().unwrap_or_default();
                let mut app = self.lock();
                let kind = match page {
                    "settings" => PageKind::Settings,
                    "ssh-keys" => PageKind::SshKeys {
                        deleting: None,
                        notice: None,
                    },
                    "add-key" => PageKind::AddKey(Box::new(AddKeyDraft::new(false))),
                    "shortcuts" => PageKind::Shortcuts,
                    "connection-form" => PageKind::ConnectionForm(Box::new(FormDraft::new(&app))),
                    "key-picker" => PageKind::KeyPicker,
                    "accent" => PageKind::Accent {
                        selected: app.settings.app_theme_id(),
                        deleting: None,
                    },
                    "theme" => PageKind::Theme { deleting: None },
                    "accent-editor" => {
                        let applied = app.accents.theme(&app.settings.app_theme_id());
                        PageKind::AccentEditor(Box::new(moshpit_app::looks::AccentEditor::new(
                            &app.accents.make_draft(&applied),
                        )))
                    }
                    "theme-editor" => PageKind::ThemeEditor(Box::new(
                        moshpit_app::looks::ThemeEditor::new(app.themes.make_draft()),
                    )),
                    "recent-log" => PageKind::RecentLog,
                    _ => return json!({"error": format!("no page {page}")}),
                };
                json!({"surface": app.open(self, kind, false)})
            }
            // The custom accents and themes, and the ones applied.
            "looks" => {
                let app = self.lock();
                json!({
                    "accents": app.accents.custom_themes().iter()
                        .map(|a| json!({"id": a.id, "name": a.name, "accent": a.accent_hex}))
                        .collect::<Vec<_>>(),
                    "accent": app.settings.app_theme_id(),
                    "themes": app.themes.custom_themes().iter()
                        .map(|t| json!({"id": t.id, "name": t.name, "background": t.background_hex}))
                        .collect::<Vec<_>>(),
                    "theme": app.settings.theme_id(),
                })
            }
            // What page `surface` (the top one when absent) was last posted
            // for `op` by a refresh that compares (the host setup's).
            "host-setup" => {
                let app = self.lock();
                let page = app.pages.iter().rev().find_map(|p| match &p.kind {
                    PageKind::HostSetup(page) => Some(page.clone()),
                    _ => None,
                });
                match page {
                    Some(page) => crate::app::host_setup::message(&app, &page)
                        .and_then(|json| serde_json::from_str(&json).ok())
                        .unwrap_or(Value::Null),
                    None => json!({"error": "no host setup page"}),
                }
            }
            "keys" => {
                let app = self.lock();
                let keys: Vec<_> = app
                    .keys
                    .keys()
                    .iter()
                    .map(|k| json!({"id": k.id.to_string(), "name": k.name, "fingerprint": k.fingerprint}))
                    .collect();
                json!({"keys": keys})
            }
            "connections" => {
                let app = self.lock();
                let list: Vec<_> = app
                    .connections
                    .connections()
                    .iter()
                    .map(|c| json!({"id": c.id.to_string(), "name": c.display_name()}))
                    .collect();
                json!({"connections": list})
            }
            "add-connection" => self.add_connection(c),
            "import-key" | "generate-key" => self.make_key(c, cmd == "import-key"),
            "enter" | "connect" => match connection {
                Some(id) => {
                    let mut app = self.lock();
                    if cmd == "enter" {
                        app.enter(self, id);
                    } else {
                        app.connect(id);
                    }
                    json!({"ok": true})
                }
                None => json!({"error": "no connection"}),
            },
            "state" => match connection {
                Some(id) => state(&self.lock(), id),
                None => json!({"error": "no connection"}),
            },
            "fault" => match connection {
                Some(id) => {
                    let kind = c["kind"].as_str().unwrap_or_default();
                    json!({"ok": self.lock().sessions.inject(id, kind)})
                }
                None => json!({"error": "no connection"}),
            },
            "disconnect" => match connection {
                Some(id) => {
                    self.lock().disconnect(self, id);
                    json!({"ok": true})
                }
                None => json!({"error": "no connection"}),
            },
            "answer-all" => {
                let ok = c["ok"].as_bool().unwrap_or(true);
                let tokens: Vec<u64> = self.lock().questions.keys().copied().collect();
                for token in &tokens {
                    self.answer(*token, ok, None);
                }
                json!({"answered": tokens.len()})
            }
            // What the page on top tells the host, as its screen would (an
            // intent the tests can't tap reliably: the empty state's Create).
            "tell" => {
                let top = self.lock().pages.last().map(|p| p.surface);
                match top {
                    Some(surface) => {
                        self.on_event(surface, &c["event"].to_string());
                        json!({"ok": true})
                    }
                    None => json!({"error": "no page"}),
                }
            }
            "share" => {
                let text = c["text"].as_str().unwrap_or_default().to_owned();
                self.lock().share(text);
                json!({"ok": true})
            }
            "notify" => {
                let app = self.lock();
                app.platform.notify(&crate::platform::Notification {
                    channel: "agents".to_owned(),
                    tag: "test".to_owned(),
                    title: c["title"].as_str().unwrap_or("Moshpit").to_owned(),
                    body: c["body"]
                        .as_str()
                        .unwrap_or("A test notification")
                        .to_owned(),
                    link: "moshpit://".to_owned(),
                });
                json!({"ok": true})
            }
            other => json!({"error": format!("no hook {other}")}),
        }
    }

    /// Saves a connection the way the form's Save does.
    fn add_connection(self: &Arc<Self>, c: &Value) -> Value {
        let mut app = self.lock();
        let mut draft = FormDraft::new(&app);
        for (key, value) in [
            ("name", &c["name"]),
            ("host", &c["host"]),
            (
                "port",
                &json!(
                    c["port"]
                        .as_u64()
                        .map_or("22".to_owned(), |p| p.to_string())
                ),
            ),
            ("username", &c["user"]),
            ("mosh", &json!(c["mosh"].as_bool().unwrap_or(false))),
            ("mux", &json!(c["mux"].as_str().unwrap_or("none"))),
            (
                "moshServerPath",
                &json!(c["moshServer"].as_str().unwrap_or("")),
            ),
            ("tmuxPath", &json!(c["tmuxPath"].as_str().unwrap_or(""))),
            ("herdrPath", &json!(c["herdrPath"].as_str().unwrap_or(""))),
        ] {
            if !value.is_null() {
                draft.set(key, value);
            }
        }
        if let Some(password) = c["password"].as_str() {
            draft.set("auth", &json!("password"));
            draft.set_secret("connection-password", password.to_owned());
        }
        if let Some(pem) = c["pem"].as_str() {
            draft.set("auth", &json!("key"));
            draft.set_secret("connection-pem", pem.to_owned());
        }
        if let Some(key) = uuid(c, "key") {
            draft.model.pick_key(Some(key));
        }
        let before: Vec<Uuid> = app.connections.connections().iter().map(|c| c.id).collect();
        match crate::app::form::save(&mut app, &draft) {
            Ok(()) => {
                app.refresh();
                let id = app
                    .connections
                    .connections()
                    .iter()
                    .find(|c| !before.contains(&c.id))
                    .map(|c| c.id.to_string());
                json!({"id": id})
            }
            Err(why) => json!({"error": why.reason}),
        }
    }

    /// Makes or imports a key the way Add Key's submit does, on a page of
    /// its own that closes when the key is kept.
    fn make_key(self: &Arc<Self>, c: &Value, import: bool) -> Value {
        let surface = {
            let mut app = self.lock();
            let mut draft = AddKeyDraft::new(false);
            draft.name = c["name"].as_str().unwrap_or("test key").to_owned();
            draft.secure_enclave = c["hardware"].as_bool().unwrap_or(false);
            if import {
                draft.method = Some(moshpit_ui::add_key::Method::Import);
            }
            app.open(self, PageKind::AddKey(Box::new(draft)), false)
        };
        if import {
            let mut app = self.lock();
            app.key_field("pem", c["text"].as_str().unwrap_or_default().to_owned());
            if let Some(passphrase) = c["passphrase"].as_str() {
                app.key_field("passphrase", passphrase.to_owned());
            }
        }
        let before: Vec<Uuid> = self.lock().keys.keys().iter().map(|k| k.id).collect();
        self.on_event(surface, &json!({"ev": "submit"}).to_string());
        let app = self.lock();
        match app.keys.keys().iter().find(|k| !before.contains(&k.id)) {
            Some(key) => json!({
                "id": key.id.to_string(),
                "fingerprint": key.fingerprint,
                "publicKey": key.public_key,
                "source": key.source.raw_value(),
            }),
            None => {
                let error = app.pages.iter().find_map(|p| match &p.kind {
                    PageKind::AddKey(draft) if p.surface == surface => draft.error.clone(),
                    _ => None,
                });
                json!({"error": error.unwrap_or_else(|| "no key made".to_owned())})
            }
        }
    }
}

/// A connection's session, as the tests read it.
fn state(app: &App, connection: Uuid) -> Value {
    let Some(conn) = app.sessions.conn(connection) else {
        return json!({"state": "none"});
    };
    let state = match &conn.state {
        SessionState::Connecting => "connecting".to_owned(),
        SessionState::Connected => "connected".to_owned(),
        SessionState::Reconnecting => "reconnecting".to_owned(),
        SessionState::Ended(why) => format!("ended {why:?}"),
        SessionState::Failed(failure) => format!("failed {:?}: {}", failure.kind, failure.log),
    };
    json!({
        "state": state,
        "stream": app.sessions.stream(connection),
        "attach": conn.attach,
        "panes": conn.panes.iter().map(|p| json!({"pane": p.pane, "stream": p.stream, "active": p.active})).collect::<Vec<_>>(),
        "tmuxAttached": conn.tmux.as_ref().map(|t| t.snapshot.is_attached),
        "activePane": conn.tmux.as_ref().and_then(|t| t.snapshot.active_pane_id.clone()),
        "herdr": conn.herdr.as_ref().map(|h| json!({
            "attached": h.snapshot.is_attached,
            "serverNotRunning": h.server_not_running,
            "workspaces": h.snapshot.sessions.values().map(|s| s.name.clone()).collect::<Vec<_>>(),
            "panes": h.snapshot.panes.keys().cloned().collect::<Vec<_>>(),
            "activePane": h.snapshot.active_pane_id.clone(),
            "framePane": h.frame_target.clone(),
            "notice": h.notice.as_ref().map(|(n, _)| n.english()),
        })),
        "protocol": app.connections.connections().iter().find(|c| c.id == connection)
            .map(|c| if c.connection_protocol == moshpit_store::connections::ConnectionProtocol::Mosh { "mosh" } else { "ssh" }),
        "returnPathDead": conn.return_path_dead,
        "degrade": conn.degrade.map(|d| format!("{d:?}")),
        "mosh": conn.mosh,
        "moshServerPid": conn.mosh_server_pid,
    })
}
