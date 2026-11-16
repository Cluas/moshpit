//! The controller against a fake platform: the page stack, what each screen
//! is posted, and what its events change in the stores — the same stores and
//! formats the iOS app keeps (`moshpit-store`), here behind
//! `SharedPreferences` strings and a Keystore-wrapped keychain.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use moshpit_store::connections::{AuthMethod, ConnectionProtocol};
use moshpit_store::ssh_keys::{SshKeyAlgorithm, SshKeySource};

use super::*;
use crate::platform::testing::{Call, FakePlatform};

/// The host's side of the stack: each pushed surface run as a view would
/// run it, its posts kept.
struct Host {
    core: Arc<Core>,
    platform: Arc<FakePlatform>,
    inboxes: HashMap<SurfaceId, UnboundedReceiver<String>>,
    /// The surfaces on screen, bottom first, as the host has them.
    stack: Vec<SurfaceId>,
    /// Everything else the controller asked for.
    calls: Vec<Call>,
}

impl Host {
    fn start() -> Host {
        Host::on(FakePlatform::new())
    }

    fn on(platform: Arc<FakePlatform>) -> Host {
        let core = Core::new(
            platform.clone(),
            Options {
                debuggable: true,
                ..Options::default()
            },
        );
        core.start();
        let mut host = Host {
            core,
            platform,
            inboxes: HashMap::new(),
            stack: Vec::new(),
            calls: Vec::new(),
        };
        host.settle();
        host
    }

    /// Runs what the controller asked: pushes become views, pops go.
    fn settle(&mut self) {
        for call in self.platform.take_calls() {
            match call {
                Call::Push(surface) => {
                    self.stack.push(surface);
                    match self.core.take_pending(surface) {
                        Some(Pending::Screen {
                            screen,
                            params,
                            channel,
                        }) => {
                            let _root = screen.root(&channel, &params);
                            if let Some(inbox) = channel.take_inbox() {
                                self.inboxes.insert(surface, inbox);
                            }
                        }
                        None => panic!("surface {surface} has nothing to run"),
                    }
                    self.core.surface_ready(surface);
                }
                Call::Pop(surface) => {
                    self.stack.retain(|s| *s != surface);
                    self.inboxes.remove(&surface);
                    self.core.surface_gone(surface);
                }
                other => self.calls.push(other),
            }
        }
    }

    fn top(&self) -> SurfaceId {
        *self.stack.last().expect("a page")
    }

    fn name(&self, surface: SurfaceId) -> &'static str {
        self.core
            .lock()
            .stack()
            .into_iter()
            .find(|(s, _)| *s == surface)
            .map_or("gone", |(_, name)| name)
    }

    /// The posts surface `surface` got since the last look, as JSON.
    fn posts(&mut self, surface: SurfaceId) -> Vec<Value> {
        let mut out = Vec::new();
        if let Some(inbox) = self.inboxes.get_mut(&surface) {
            while let Ok(json) = inbox.try_recv() {
                out.push(serde_json::from_str(&json).unwrap());
            }
        }
        out
    }

    fn last_post(&mut self, surface: SurfaceId) -> Value {
        self.posts(surface).pop().expect("a post")
    }

    /// The page on top tells the host `event`.
    fn tell(&mut self, event: Value) {
        let top = self.top();
        self.core.on_event(top, &event.to_string());
        self.settle();
    }
}

#[test]
fn home_comes_first_and_shows_the_saved_connections() {
    let mut host = Host::start();
    let home = host.top();
    assert_eq!(host.name(home), "home");
    let posts = host.posts(home);
    assert_eq!(posts[0]["op"], "trees");
    assert_eq!(posts[1]["op"], "home");
    assert_eq!(posts[1]["connections"], json!([]));
    assert_eq!(posts[1]["version"], "v0.1.0 (1)");
}

/// Home keeps only its idle group in a tree event, the sessions page only
/// its sessions and windows (UI-DESIGN §6.1, §6.2): neither clears the
/// other's, and all of it is what a Home made again is handed.
#[test]
fn each_page_keeps_its_own_part_of_a_hosts_tree() {
    let mut host = Host::start();
    let id = "C0FFEE00-0000-4000-8000-00000000F001";
    host.tell(json!({"ev": "tree", "connection": id, "collapsed": ["$0"], "expanded": ["@2"]}));
    host.tell(json!({"ev": "tree", "connection": id, "idle": true}));
    let trees = moshpit_ui::home::Message::Trees(host.core.lock().trees.clone()).to_json();
    let trees: Value = serde_json::from_str(&trees).unwrap();
    assert_eq!(
        trees["trees"],
        json!([{"connection": id, "collapsed": ["$0"], "expanded": ["@2"], "idle": true, "told": true}])
    );
    host.tell(json!({"ev": "tree", "connection": id, "idle": false}));
    let app = host.core.lock();
    assert_eq!(
        (app.trees[0].idle, app.trees[0].collapsed.len()),
        (false, 1)
    );
}

#[test]
fn back_pops_a_page_and_leaves_the_app_from_home() {
    let mut host = Host::start();
    host.tell(json!({"ev": "settings"}));
    assert_eq!(host.name(host.top()), "settings");
    assert!(host.core.back());
    host.settle();
    assert_eq!(host.stack.len(), 1);
    assert!(host.core.back());
    host.settle();
    assert_eq!(host.calls, [Call::Finish]);
}

#[test]
fn a_connection_saved_by_the_form_is_on_home_with_its_password_in_the_keychain() {
    let mut host = Host::start();
    host.tell(json!({"ev": "add"}));
    let form = host.top();
    assert_eq!(host.name(form), "connection-form");
    let first = host.last_post(form);
    assert_eq!(first["op"], "form");
    assert_eq!(first["port"], "22");
    assert_eq!(first["mux"], "tmux");
    // Settings › Mosh by default (on unless turned off) is what a new
    // connection starts with. (The iOS form starts with SSH whatever the
    // setting says: nothing there reads it.)
    assert_eq!(first["mosh"], true);
    for (key, value) in [
        ("name", json!("Studio")),
        ("host", json!("10.0.2.2")),
        ("port", json!("2222")),
        ("username", json!("me")),
        ("mux", json!("none")),
        ("mosh", json!(false)),
    ] {
        host.tell(json!({"ev": "set", "key": key, "value": value}));
    }
    // A secret told through the screen is not taken (NSH-GAPS #11)...
    host.tell(json!({"ev": "set", "key": "password", "value": "from-the-screen"}));
    // ...the platform's own field gives it, and the screen hears only that
    // there is one.
    host.core
        .lock()
        .key_field("connection-password", "hunter2".to_owned());
    {
        let app = host.core.lock();
        let Some(crate::app::PageKind::ConnectionForm(draft)) = app
            .pages
            .iter()
            .find(|p| p.surface == form)
            .map(|p| &p.kind)
        else {
            panic!("the form");
        };
        assert_eq!(draft.password, "hunter2");
        assert!(draft.model.given().password);
        let message = crate::app::form::message(&app, draft);
        assert!(!message.contains("hunter2"), "{message}");
    }
    host.tell(json!({"ev": "save"}));
    assert_eq!(host.stack.len(), 1, "the form closed");
    let app = host.core.lock();
    let saved = &app.connections.connections()[0];
    assert_eq!(saved.name, "Studio");
    assert_eq!(saved.port, 2222);
    assert_eq!(saved.auth_method, AuthMethod::Password);
    assert_eq!(saved.connection_protocol, ConnectionProtocol::Ssh);
    let account = saved.keychain_ref.clone().unwrap();
    assert!(account.starts_with("moshpit.secret."));
    assert_eq!(
        host.platform.secrets.lock().unwrap()[&account],
        b"hunter2".to_vec()
    );
    drop(app);
    let home = host.top();
    let state = host.last_post(home);
    assert_eq!(state["connections"][0]["name"], "Studio");
    assert_eq!(state["connections"][0]["link"], "saved");
}

/// B294: a key connection with no key picked and none pasted used to
/// save, and could never sign in. The core's Save asks what the screen's
/// does: a key picked, pasted (more than white space) or kept.
#[test]
fn a_key_connection_saves_only_with_a_key() {
    let mut host = Host::start();
    host.tell(json!({"ev": "add"}));
    let form = host.top();
    for (key, value) in [
        ("name", json!("Keyed")),
        ("host", json!("10.0.2.2")),
        ("username", json!("me")),
        ("auth", json!("key")),
    ] {
        host.tell(json!({"ev": "set", "key": key, "value": value}));
    }
    host.tell(json!({"ev": "save"}));
    assert_eq!(host.top(), form, "saved with no key");
    // The form keeps what it holds and says why under its bar (§6.5).
    let mut seen = host.posts(form);
    let failed = seen.last().expect("a post").clone();
    assert_eq!(failed["op"], "failed", "{failed}");
    assert_eq!(
        failed["retry"], false,
        "a refusal: saving again would not help"
    );
    assert!(host.core.lock().connections.connections().is_empty());

    host.core
        .lock()
        .key_field("connection-pem", " \n\t".to_owned());
    host.tell(json!({"ev": "save"}));
    assert_eq!(host.top(), form, "white space saved as a key");

    let pem = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ssh/tests/fixtures/keys/rsa2048-pem"),
    )
    .unwrap();
    host.core.lock().key_field("connection-pem", pem);
    // The refusal is said under the form's bar; the form's last state
    // still says it keeps no key.
    seen.extend(host.posts(form));
    assert_eq!(seen.last().map(|p| p["op"].clone()), Some(json!("failed")));
    let shown = seen
        .iter()
        .rev()
        .find(|p| p["op"] == "form")
        .expect("the form's state");
    assert_eq!(shown["secrets"]["storedKey"], false, "{shown}");
    host.tell(json!({"ev": "save"}));
    assert_eq!(host.stack.len(), 1, "the form closed");
    let app = host.core.lock();
    let saved = &app.connections.connections()[0];
    assert_eq!(saved.auth_method, AuthMethod::Key);
    assert!(saved.keychain_ref.is_some());
}

/// B325: a connection that kept a password (or a pasted key) of its own,
/// switched to a managed SSH key, pointed at the key's item and left its
/// own behind, named by nothing: the old password stayed in the keychain
/// for good, and a pasted key came back at the next launch as a
/// "Recovered key" in SSH Keys. The save drops it now, once nothing else
/// names it.
#[test]
fn switching_to_a_managed_key_drops_the_connections_own_item() {
    let mut host = Host::start();
    let added = host.core.debug(
        &json!({"cmd": "add-connection", "name": "box", "host": "h", "user": "u", "password": "p"})
            .to_string(),
    );
    assert!(added.contains("\"id\""), "{added}");
    host.core
        .debug(&json!({"cmd": "generate-key", "name": "phone"}).to_string());
    host.settle();
    let mut app = host.core.lock();
    let saved = app.connections.connections()[0].clone();
    let own = saved.keychain_ref.clone().expect("its own item");
    let key = app.keys.keys()[0].clone();
    assert!(host.platform.secrets.lock().unwrap().contains_key(&own));

    let mut draft = crate::app::form::FormDraft::editing(&app, &saved);
    assert!(draft.model.pick_key(Some(key.id)));
    crate::app::form::save(&mut app, &draft).unwrap();
    let now = app.connections.connections()[0].clone();
    assert_eq!(now.ssh_key_id, Some(key.id));
    assert_eq!(now.keychain_ref, key.keychain_ref);
    let secrets = host.platform.secrets.lock().unwrap();
    assert!(
        !secrets.contains_key(&own),
        "the password it no longer signs in with stayed in the keychain"
    );
    assert!(
        secrets.contains_key(key.keychain_ref.as_ref().unwrap()),
        "the key's own item stays"
    );
}

/// A connection's own keychain item holds the secret of the method it signs
/// in with. Switching the method with nothing new typed kept the item under
/// the new method — its password read as a key, or a pasted key sent as the
/// password. Refused now, changing nothing; with the new secret, it replaces
/// what the item held.
#[test]
fn switching_the_method_without_the_new_secret_is_refused() {
    let host = Host::start();
    let added = host.core.debug(
        &json!({"cmd": "add-connection", "name": "box", "host": "h", "user": "u", "password": "p"})
            .to_string(),
    );
    assert!(added.contains("\"id\""), "{added}");
    let mut app = host.core.lock();
    let saved = app.connections.connections()[0].clone();
    let account = saved.keychain_ref.clone().expect("its own item");
    let mut draft = crate::app::form::FormDraft::editing(&app, &saved);
    assert!(draft.set("auth", &json!("key")));
    assert_eq!(
        crate::app::form::save(&mut app, &draft).map_err(|why| (why.reason, why.retry)),
        Err((
            moshpit_app::connection_form::NEW_SECRET_NEEDED.to_owned(),
            false
        )),
        "a refusal: saving again would not help"
    );
    assert_eq!(
        app.connections.connections()[0],
        saved,
        "the store was written"
    );
    assert_eq!(
        host.platform.secrets.lock().unwrap()[&account],
        b"p".to_vec()
    );

    draft.set_secret("connection-pem", "not a key".to_owned());
    crate::app::form::save(&mut app, &draft).unwrap();
    let now = app.connections.connections()[0].clone();
    assert_eq!(now.auth_method, AuthMethod::Key);
    assert_eq!(now.keychain_ref.as_deref(), Some(account.as_str()));
    assert_eq!(
        host.platform.secrets.lock().unwrap()[&account],
        b"not a key".to_vec()
    );
}

#[test]
fn what_is_saved_reads_back_in_a_new_launch() {
    let platform = FakePlatform::new();
    let host = Host::on(platform.clone());
    let added = host
        .core
        .debug(&json!({"cmd": "add-connection", "name": "Box", "host": "h", "user": "u", "password": "p", "mosh": true, "mux": "tmux"}).to_string());
    assert!(added.contains("\"id\""), "{added}");
    let stored = platform.prefs.lock().unwrap()["moshpit.connections"].clone();
    assert!(stored.starts_with("x:"), "the list is Data: {stored}");
    drop(host);
    let again = Host::on(platform);
    let app = again.core.lock();
    let saved = &app.connections.connections()[0];
    assert_eq!(saved.name, "Box");
    assert_eq!(saved.connection_protocol, ConnectionProtocol::Mosh);
}

#[test]
fn settings_are_written_where_the_store_reads_them() {
    let mut host = Host::start();
    host.tell(json!({"ev": "settings"}));
    let settings = host.top();
    let state = host.last_post(settings);
    assert_eq!(state["op"], "settings");
    assert_eq!(state["moshByDefault"], true);
    // Gone with its switch (B516): nothing read it.
    assert!(state.get("keepAlive").is_none());
    host.tell(json!({"ev": "set", "key": "moshByDefault", "value": false}));
    host.tell(json!({"ev": "set", "key": "fontSize", "value": 15.4}));
    assert_eq!(
        host.platform.prefs.lock().unwrap()["moshpit.settings.moshByDefault"],
        "b:0"
    );
    let state = host.last_post(settings);
    assert_eq!(state["moshByDefault"], false);
    assert_eq!(state["fontSize"], 15.0);
    // The terminal's pinch range, not 18 (B515).
    host.tell(json!({"ev": "set", "key": "fontSize", "value": 24.0}));
    assert_eq!(host.last_post(settings)["fontSize"], 24.0);
    host.tell(json!({"ev": "open", "page": "ssh-keys"}));
    assert_eq!(host.name(host.top()), "ssh-keys");
}

/// B514: the UDP range's page sends `udpRange` on Save; Android had no
/// case for it, so the edit was refused and the range never changed. Kept
/// now, checked again as iOS checks it.
#[test]
fn the_udp_range_saved_is_kept() {
    let mut host = Host::start();
    host.tell(json!({"ev": "settings"}));
    let settings = host.top();
    host.tell(json!({"ev": "set", "key": "udpRange", "value": [60010, 60020]}));
    assert_eq!(host.last_post(settings)["udpRange"], json!([60010, 60020]));
    assert_eq!(
        host.platform.prefs.lock().unwrap()["moshpit.settings.udpRangeStart"],
        "i:60010"
    );
    for refused in [
        json!([61000, 60000]),
        json!([0, 60000]),
        json!([60000, 70000]),
        json!([60000]),
        json!("60000-61000"),
    ] {
        host.tell(json!({"ev": "set", "key": "udpRange", "value": refused}));
        let prefs = host.platform.prefs.lock().unwrap().clone();
        assert_eq!(
            (
                &*prefs["moshpit.settings.udpRangeStart"],
                &*prefs["moshpit.settings.udpRangeEnd"]
            ),
            ("i:60010", "i:60020"),
            "{refused} kept"
        );
    }
    assert!(
        host.posts(settings).is_empty(),
        "a refused range was posted"
    );
}

#[test]
fn a_generated_key_is_kept_and_listed() {
    let mut host = Host::start();
    let made: Value = serde_json::from_str(
        &host
            .core
            .debug(&json!({"cmd": "generate-key", "name": "phone"}).to_string()),
    )
    .unwrap();
    host.settle();
    assert!(
        made["publicKey"]
            .as_str()
            .unwrap()
            .starts_with("ssh-ed25519 ")
    );
    assert_eq!(made["source"], "generated");
    let app = host.core.lock();
    let key = &app.keys.keys()[0];
    assert_eq!(key.algorithm, SshKeyAlgorithm::Ed25519);
    let account = key.keychain_ref.clone().unwrap();
    let secret = host.platform.secrets.lock().unwrap()[&account].clone();
    assert!(secret.starts_with(b"-----BEGIN OPENSSH PRIVATE KEY-----"));
    drop(app);
    assert_eq!(host.stack.len(), 1, "Add Key closed");
}

#[test]
fn a_hardware_key_stays_in_the_keystore() {
    let mut host = Host::start();
    let made: Value = serde_json::from_str(
        &host
            .core
            .debug(&json!({"cmd": "generate-key", "name": "pixel", "hardware": true}).to_string()),
    )
    .unwrap();
    host.settle();
    assert!(
        made["publicKey"]
            .as_str()
            .unwrap()
            .starts_with("ecdsa-sha2-nistp256 "),
        "{made}"
    );
    let app = host.core.lock();
    let key = &app.keys.keys()[0];
    assert_eq!(key.source, SshKeySource::SecureEnclave);
    assert_eq!(key.algorithm, SshKeyAlgorithm::SeP256);
    let alias = key.keychain_ref.clone().unwrap();
    assert!(alias.starts_with("moshpit.hw."));
    assert!(host.platform.hardware.lock().unwrap().contains_key(&alias));
    assert!(
        host.platform.secrets.lock().unwrap().is_empty(),
        "no private key in the keychain"
    );
}

#[test]
fn an_imported_key_is_converted_and_a_bad_passphrase_says_so() {
    let fixture = |name: &str| {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../ssh/tests/fixtures/keys")
                .join(name),
        )
        .unwrap()
    };
    let mut host = Host::start();
    let wrong: Value = serde_json::from_str(&host.core.debug(
        &json!({"cmd": "import-key", "name": "enc", "text": fixture("ed25519-encrypted"), "passphrase": "nope"})
            .to_string(),
    ))
    .unwrap();
    host.settle();
    // The shared reader's words, as iOS says it.
    assert_eq!(
        wrong["error"],
        moshpit_ssh::keyfile::ImportError::WrongPassphrase.to_string()
    );
    assert_eq!(wrong["error"], "That passphrase doesn't unlock this key.");
    let made: Value = serde_json::from_str(&host.core.debug(
        &json!({"cmd": "import-key", "name": "pem", "text": fixture("rsa2048-pem")}).to_string(),
    ))
    .unwrap();
    host.settle();
    assert_eq!(
        made["fingerprint"],
        "SHA256:YI308USvR1zZ2NtAEtFF7tm0SR2whhSScC1VPQYyh8w"
    );
    let app = host.core.lock();
    let key = app.keys.keys().iter().find(|k| k.name == "pem").unwrap();
    assert_eq!(key.algorithm, SshKeyAlgorithm::Rsa4096);
    assert_eq!(key.bits, Some(2048));
    assert_eq!(key.source, SshKeySource::Imported);
}

#[test]
fn the_key_picker_sets_the_forms_key() {
    let mut host = Host::start();
    host.core
        .debug(&json!({"cmd": "generate-key", "name": "k"}).to_string());
    host.settle();
    let key = host.core.lock().keys.keys()[0].id.to_string();
    host.tell(json!({"ev": "add"}));
    let form = host.top();
    host.posts(form);
    host.tell(json!({"ev": "pick-key"}));
    let picker = host.top();
    assert_eq!(host.name(picker), "key-picker");
    let keys = host.last_post(picker);
    assert_eq!(keys["keys"][0]["id"], key.as_str());
    assert_eq!(keys["selected"], Value::Null);
    host.tell(json!({"ev": "select", "id": key}));
    assert_eq!(host.top(), form, "the picker closed");
    let draft = host.last_post(form);
    assert_eq!(draft["auth"], "key");
    assert_eq!(draft["key"]["id"], key.as_str());
    assert_eq!(draft["key"]["label"], "k · ED25519");
}

#[test]
fn deleting_a_connection_asks_first_and_takes_its_secret() {
    let mut host = Host::start();
    let added: Value = serde_json::from_str(&host.core.debug(
        &json!({"cmd": "add-connection", "name": "Gone", "host": "h", "user": "u", "password": "p"})
            .to_string(),
    ))
    .unwrap();
    host.settle();
    let id = added["id"].as_str().unwrap().to_owned();
    host.tell(json!({"ev": "delete", "connection": id}));
    let Some(Call::Ask(crate::platform::Ask::Confirm {
        token, destructive, ..
    })) = host.calls.pop()
    else {
        panic!("no question asked");
    };
    assert!(destructive);
    assert_eq!(host.core.lock().connections.connections().len(), 1);
    host.core.answer(token, true, None);
    host.settle();
    assert!(host.core.lock().connections.connections().is_empty());
    assert!(host.platform.secrets.lock().unwrap().is_empty());
}

#[test]
fn the_shortcut_bar_is_edited_in_the_store() {
    let mut host = Host::start();
    host.tell(json!({"ev": "settings"}));
    host.tell(json!({"ev": "open", "page": "shortcuts"}));
    let page = host.top();
    let shortcuts = host.last_post(page);
    let toolbar = shortcuts["toolbar"].as_array().unwrap().clone();
    assert!(!toolbar.is_empty());
    let first = toolbar[0]["id"].as_str().unwrap().to_owned();
    host.tell(json!({"ev": "remove-from-toolbar", "id": first}));
    let after = host.last_post(page);
    assert_eq!(
        after["toolbar"].as_array().unwrap().len(),
        toolbar.len() - 1
    );
    assert!(
        after["available"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == first.as_str())
    );
}

/// Settings' hub opens its pages (UI-DESIGN §6.4) as the Settings screen
/// again, told the page; a page's back returns to the hub.
#[test]
fn settings_pages_open_on_the_same_screen_and_go_back() {
    let mut host = Host::start();
    host.tell(json!({"ev": "settings"}));
    let hub = host.top();
    host.tell(json!({"ev": "open", "page": "terminal"}));
    let page = host.top();
    assert_ne!(page, hub);
    assert_eq!(host.name(page), "settings");
    assert_eq!(host.last_post(page)["op"], "settings");
    host.tell(json!({"ev": "back"}));
    assert_eq!(host.top(), hub);
    assert_eq!(host.name(page), "gone");
}

#[test]
fn the_accent_picked_is_saved_as_the_page_goes() {
    let mut host = Host::start();
    host.tell(json!({"ev": "settings"}));
    host.tell(json!({"ev": "open", "page": "accent"}));
    let page = host.top();
    let accents = host.last_post(page);
    let other = accents["builtIn"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_owned())
        .find(|id| *id != accents["selected"])
        .unwrap();
    host.tell(json!({"ev": "select", "id": other}));
    host.tell(json!({"ev": "back"}));
    assert_eq!(host.core.lock().settings.app_theme_id(), other);
}

#[test]
fn shared_text_waits_when_no_terminal_is_open() {
    let mut host = Host::start();
    host.core.lock().share("ls -la".to_owned());
    host.settle();
    assert_eq!(host.core.lock().shares, ["ls -la"]);
    assert!(matches!(host.calls.last(), Some(Call::Toast(_))));
}

#[test]
fn a_release_build_answers_no_hooks() {
    let core = Core::new(FakePlatform::new(), Options::default());
    assert!(
        core.debug(r#"{"cmd":"stack"}"#)
            .contains("not a debuggable build")
    );
}

#[test]
fn a_terminal_page_runs_the_shared_terminal_screen() {
    let mut host = Host::start();
    let added: Value = serde_json::from_str(&host.core.debug(
        &json!({"cmd": "add-connection", "name": "box", "host": "127.0.0.1", "port": 9, "user": "u", "password": "p", "mux": "none", "mosh": false})
            .to_string(),
    ))
    .unwrap();
    host.settle();
    let id = added["id"].as_str().unwrap().to_owned();
    host.tell(json!({"ev": "enter", "connection": id}));
    let page = host.top();
    assert_eq!(host.name(page), "terminal");
    let posts = host.posts(page);
    let chrome = posts
        .iter()
        .find(|p| p["op"] == "chrome")
        .expect("a chrome");
    assert_eq!(chrome["name"], "box");
    assert_eq!(chrome["pill"], "ssh");
    // A first connect has no cover (B657), only the pill's state.
    assert_eq!(chrome["cover"]["kind"], "none", "{chrome}");
    let bar = posts.iter().find(|p| p["op"] == "bar").expect("a bar");
    assert!(!bar["chips"].as_array().unwrap().is_empty());
    assert!(
        bar["chips"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["kind"] != "mic" && c["kind"] != "image")
    );
    host.tell(json!({"ev": "keyboard"}));
    assert!(host.calls.contains(&Call::Keyboard(false)));
    let bar = host
        .posts(page)
        .into_iter()
        .find(|p| p["op"] == "bar")
        .unwrap();
    assert_eq!(bar["keyboardDown"], true);
    host.calls.clear();
    host.tell(json!({"ev": "back"}));
    assert_eq!(host.stack.len(), 1);
    assert!(
        host.calls.contains(&Call::Keyboard(false)),
        "the keyboard goes with the page"
    );
}

/// A refresh while a page's view is still being made posts nothing, and
/// the view's first post still carries everything (the bar was lost on the
/// emulator: deduplicated against a post that never went).
#[test]
fn what_a_page_shows_reaches_it_when_refreshed_before_its_view_runs() {
    let mut host = Host::start();
    let added: Value = serde_json::from_str(&host.core.debug(
        &json!({"cmd": "add-connection", "name": "box", "host": "127.0.0.1", "port": 9, "user": "u", "password": "p", "mux": "none"})
            .to_string(),
    ))
    .unwrap();
    host.settle();
    let home = host.top();
    host.core.on_event(
        home,
        &json!({"ev": "enter", "connection": added["id"]}).to_string(),
    );
    host.core.lock().refresh();
    host.settle();
    let page = host.top();
    let ops: Vec<String> = host
        .posts(page)
        .iter()
        .map(|p| p["op"].as_str().unwrap().to_owned())
        .collect();
    assert!(ops.contains(&"bar".to_owned()), "{ops:?}");
    assert!(ops.contains(&"chrome".to_owned()), "{ops:?}");
}

#[test]
fn a_new_accent_is_edited_saved_and_picked() {
    let mut host = Host::start();
    host.tell(json!({"ev": "settings"}));
    host.tell(json!({"ev": "open", "page": "accent"}));
    let gallery = host.top();
    host.tell(json!({"ev": "add"}));
    let editor = host.top();
    assert_ne!(editor, gallery);
    assert_eq!(host.name(editor), "accent-editor");
    let draft = host.last_post(editor);
    assert_eq!(draft["op"], "accent");
    host.tell(json!({"ev": "name", "value": "Coral"}));
    host.tell(json!({"ev": "color", "value": "#ff7f50"}));
    host.tell(json!({"ev": "save"}));
    assert_eq!(host.top(), gallery, "Save closes the editor");
    let saved = host
        .core
        .lock()
        .accents
        .custom_themes()
        .iter()
        .find(|a| a.name == "Coral")
        .cloned()
        .expect("the accent is kept");
    assert_eq!(saved.accent_hex, "FF7F50");
    // Picked on the page under it, applied as that page goes — by the
    // system's back too.
    assert!(host.core.back());
    host.settle();
    assert_eq!(host.core.lock().settings.app_theme_id(), saved.id);
}

#[test]
fn a_new_theme_is_edited_and_saved_and_its_json_copied() {
    let mut host = Host::start();
    host.tell(json!({"ev": "settings"}));
    host.tell(json!({"ev": "open", "page": "theme"}));
    let gallery = host.top();
    host.tell(json!({"ev": "new"}));
    let editor = host.top();
    assert_eq!(host.name(editor), "theme-editor");
    let draft = host.last_post(editor);
    assert_eq!(draft["op"], "theme");
    assert_eq!(draft["overrideBright"], false);
    host.tell(json!({"ev": "name", "value": "Reef"}));
    host.tell(json!({"ev": "color", "slot": "background", "value": "#001122"}));
    host.tell(json!({"ev": "color", "slot": "red", "value": "#cc0000"}));
    host.tell(json!({"ev": "override-bright", "value": true}));
    host.tell(json!({"ev": "color", "slot": "brightRed", "value": "#ff4444"}));
    host.tell(json!({"ev": "copy-json"}));
    assert!(
        host.calls
            .iter()
            .any(|c| matches!(c, Call::Copy(json) if json.contains("Reef"))),
        "{:?}",
        host.calls
    );
    host.tell(json!({"ev": "save"}));
    assert_eq!(host.top(), gallery);
    let app = host.core.lock();
    let theme = app
        .themes
        .custom_themes()
        .iter()
        .find(|t| t.name == "Reef")
        .cloned()
        .expect("the theme is kept");
    assert_eq!(theme.background_hex, "001122");
    assert_eq!(
        theme.ansi(moshpit_store::themes::AnsiSlot::ALL[1]),
        "CC0000"
    );
    assert_eq!(
        theme.bright(moshpit_store::themes::AnsiSlot::ALL[1]),
        "FF4444"
    );
    assert_eq!(
        app.settings.theme_id(),
        theme.id,
        "editing implies wanting it"
    );
}

#[test]
fn themes_import_from_pasted_json_and_bad_json_is_said() {
    let mut host = Host::start();
    host.tell(json!({"ev": "settings"}));
    host.tell(json!({"ev": "open", "page": "theme"}));
    host.tell(json!({"ev": "import"}));
    let Some(Call::Ask(Ask::Prompt {
        token, multiline, ..
    })) = host.calls.last().cloned()
    else {
        panic!("no prompt: {:?}", host.calls)
    };
    assert!(multiline);
    let json = moshpit_store::themes::ThemeStore::export_json(&{
        let mut theme = moshpit_store::themes::TerminalTheme::dracula();
        theme.name = "Imported".to_owned();
        theme
    });
    host.core.answer(token, true, Some(json));
    host.settle();
    let imported = host
        .core
        .lock()
        .themes
        .custom_themes()
        .iter()
        .find(|t| t.name.starts_with("Imported"))
        .map(|t| t.id.clone())
        .expect("imported");
    assert_eq!(host.core.lock().settings.theme_id(), imported);
    host.tell(json!({"ev": "import"}));
    let Some(Call::Ask(Ask::Prompt { token, .. })) = host.calls.last().cloned() else {
        panic!()
    };
    host.core.answer(token, true, Some("not json".to_owned()));
    host.settle();
    assert_eq!(
        host.calls.last(),
        Some(&Call::Toast("That isn't theme JSON".to_owned()))
    );
}

#[test]
fn a_key_file_picked_fills_the_private_key_and_a_public_line() {
    let mut host = Host::start();
    host.tell(json!({"ev": "settings"}));
    host.tell(json!({"ev": "open", "page": "ssh-keys"}));
    host.core.lock().open(
        &host.core,
        PageKind::AddKey(Box::new(keys::AddKeyDraft::new(false))),
        false,
    );
    host.settle();
    let page = host.top();
    host.tell(json!({"ev": "method", "value": "import"}));
    host.tell(json!({"ev": "import-file", "target": "private"}));
    let Some(Call::PickFile(token)) = host.calls.last().cloned() else {
        panic!("no picker: {:?}", host.calls)
    };
    let pem = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ssh/tests/fixtures/keys/rsa2048-pem"),
    )
    .unwrap();
    host.core.picked(token, Some(pem.clone()));
    host.settle();
    assert_eq!(
        host.calls.last(),
        Some(&Call::FillKeyField("pem".to_owned(), pem.len())),
        "the host's field shows the key"
    );
    let shown = host.last_post(page);
    assert_eq!(shown["pem"]["encrypted"], false, "{shown}");
    // A public key line from a file; a file that isn't text says so.
    host.tell(json!({"ev": "import-file", "target": "public"}));
    let Some(Call::PickFile(token)) = host.calls.last().cloned() else {
        panic!()
    };
    host.core
        .picked(token, Some(b"ssh-ed25519 AAAA test@host\n".to_vec()));
    host.settle();
    let shown = host.last_post(page);
    assert_eq!(shown["publicLine"], "ssh-ed25519 AAAA test@host\n");
    host.tell(json!({"ev": "import-file", "target": "private"}));
    let Some(Call::PickFile(token)) = host.calls.last().cloned() else {
        panic!()
    };
    host.core.picked(token, Some(vec![0xff, 0xfe, 0x00]));
    host.settle();
    let shown = host.last_post(page);
    assert_eq!(shown["error"], "Unsupported: file is not valid UTF-8 text");
    // Cancelled: nothing changes, and an answer for no question is dropped.
    host.core.picked(token, None);
    host.core.picked(9_999, Some(b"x".to_vec()));
    host.settle();
}

#[test]
fn set_up_this_host_needs_a_live_session() {
    let mut host = Host::start();
    host.tell(json!({"ev": "settings"}));
    let settings = host.top();
    host.tell(json!({"ev": "open", "page": "host-setup"}));
    assert_eq!(host.top(), settings, "no page without a live session");
    assert!(
        matches!(host.calls.last(), Some(Call::Toast(t)) if t.starts_with("Connect to a host"))
    );
}

/// The page names the connection it sets up: its name and where it
/// reaches, before what the host is.
#[test]
fn the_host_setup_page_names_its_connection() {
    let host = Host::start();
    let mut app = host.core.lock();
    let id = moshpit_seal::foundation::uuid::Uuid::new_v4();
    let mut saved = moshpit_store::connections::ServerConnection::new(
        id,
        moshpit_seal::foundation::date::Date::from_unix(1_762_800_000.0),
    );
    saved.name = "gpu-01".into();
    saved.username = "dev".into();
    saved.host = "192.0.2.10".into();
    saved.port = 2222;
    app.connections.add(saved);
    let page = host_setup::HostSetupPage {
        connection: id,
        page: 1,
    };
    let named: Value = serde_json::from_str(&host_setup::identity(&app, &page).unwrap()).unwrap();
    assert_eq!(
        named,
        json!({"op": "host", "name": "gpu-01", "user": "dev", "host": "192.0.2.10", "port": 2222})
    );
    let gone = host_setup::HostSetupPage {
        connection: moshpit_seal::foundation::uuid::Uuid::new_v4(),
        page: 1,
    };
    assert_eq!(host_setup::identity(&app, &gone), None);
}

#[test]
fn the_host_setup_message_says_push_is_not_on_this_build() {
    let host = Host::start();
    let core = host.core.clone();
    let mut app = host.core.lock();
    let care = app.care(&core);
    let connection = moshpit_seal::foundation::uuid::Uuid::new_v4();
    let page = care.with(|m, k| {
        m.open_setup(
            moshpit_hostcare::Conn {
                id: connection,
                label: "box".into(),
            },
            None,
            k,
        )
    });
    let page = host_setup::HostSetupPage { connection, page };
    let message: Value = serde_json::from_str(&host_setup::message(&app, &page).unwrap()).unwrap();
    assert_eq!(message["op"], "host-setup");
    assert_eq!(message["phase"], "inspecting");
    assert_eq!(message["pushProof"]["state"], "unavailable");
    assert_eq!(
        message["pushProof"]["message"],
        moshpit_hostcare::note::PUSH_UNAVAILABLE
    );
    assert_eq!(message["agents"].as_array().unwrap().len(), 7);
    moshpit_ui::host_setup::Message::parse(&message.to_string()).expect("the screen reads it");
}

#[test]
fn install_assist_offers_the_hosts_package_manager_command() {
    let mut host = Host::start();
    let answer: Value = serde_json::from_str(&host.core.debug(
        &json!({"cmd": "add-connection", "name": "box", "host": "192.0.2.1", "user": "me", "password": "x", "mux": "tmux"})
            .to_string(),
    ))
    .unwrap();
    host.settle();
    let id = moshpit_seal::foundation::uuid::Uuid::parse(answer["id"].as_str().unwrap()).unwrap();
    {
        let app = host.core.lock();
        let mut caps = moshpit_install::HostCapabilities::unknown();
        caps.has_tmux = false;
        caps.package_manager = Some(moshpit_install::PackageManager::AptGet);
        moshpit_store::session_memory::write(
            app.defaults.as_ref(),
            &moshpit_store::Readiness::System,
            &moshpit_store::session_memory::host_capabilities_key(&id.to_string()),
            &caps,
        );
    }
    let tmux = install_assist::InstallPage::new(id, vec!["tmux".to_owned()]);
    let message: Value =
        serde_json::from_str(&install_assist::message(&host.core.lock(), &tmux)).unwrap();
    assert_eq!(message["op"], "install");
    assert_eq!(message["manager"], true);
    assert_eq!(message["command"], "sudo apt-get install -y tmux mosh");
    assert_eq!(message["herdr"], false);
    let herdr = install_assist::InstallPage::new(id, vec!["herdr".to_owned()]);
    let message: Value =
        serde_json::from_str(&install_assist::message(&host.core.lock(), &herdr)).unwrap();
    assert_eq!(
        message["command"],
        "curl -fsSL https://herdr.dev/install.sh | sh"
    );
    assert_eq!(
        message["note"], true,
        "the installer's ~/.local/bin is said"
    );
}

#[test]
fn a_page_over_the_terminal_puts_its_keyboard_away_and_back() {
    let mut host = Host::start();
    let answer: Value = serde_json::from_str(&host.core.debug(
        &json!({"cmd": "add-connection", "name": "box", "host": "192.0.2.1", "user": "me", "password": "x"})
            .to_string(),
    ))
    .unwrap();
    host.settle();
    let id = moshpit_seal::foundation::uuid::Uuid::parse(answer["id"].as_str().unwrap()).unwrap();
    {
        let core = host.core.clone();
        core.lock().open(
            &core,
            PageKind::Terminal(Box::new(terminal::TerminalPage::new(id))),
            false,
        );
    }
    host.settle();
    host.calls.clear();
    host.core
        .debug(&json!({"cmd": "open", "page": "settings"}).to_string());
    host.settle();
    assert!(
        host.calls.contains(&Call::Keyboard(false)),
        "{:?}",
        host.calls
    );
    host.calls.clear();
    assert!(host.core.back());
    host.settle();
    assert!(
        host.calls.contains(&Call::Keyboard(true)),
        "{:?}",
        host.calls
    );
}

/// B430: Settings › UDP ports said "for every connection", but a connect
/// asked mosh-server for the range the connection stored at its last save:
/// a narrowed range never reached a saved connection, and mosh landed
/// outside the firewall (connected, black).
#[test]
fn a_mosh_connect_asks_for_settings_udp_ports_not_the_saved_ones() {
    let mut connection = moshpit_store::connections::ServerConnection::new(
        moshpit_seal::foundation::uuid::Uuid::new_v4(),
        moshpit_seal::foundation::date::Date::from_unix(1_762_800_000.0),
    );
    connection.host = "box.example.com".into();
    connection.connection_protocol = moshpit_store::connections::ConnectionProtocol::Mosh;
    connection.mosh_port_range_start = 60001;
    connection.mosh_port_range_end = 60999;
    let spec = super::sessions::spec(&connection, &[], &Options::default(), (60000, 60010));
    assert_eq!(spec.mosh_port_range, (Some(60000), Some(60010)));
}

/// F1: a connection through another dials that one's host as its jump
/// host — by its own id (its credentials), name, address and proxy — SSH
/// only and without a proxy of its own; one whose jump host was deleted,
/// or that would be a second hop, is a route that fails at once, never a
/// direct dial.
#[test]
fn f1_a_connection_through_another_dials_through_it() {
    use moshpit_session::{Hop, HopBroken, Protocol};
    use moshpit_store::connections::{ConnectionProtocol, ServerConnection};
    let made = || {
        ServerConnection::new(
            moshpit_seal::foundation::uuid::Uuid::new_v4(),
            moshpit_seal::foundation::date::Date::from_unix(1_762_800_000.0),
        )
    };
    let mut bastion = made();
    bastion.name = "bastion".into();
    bastion.host = "bastion.example.com".into();
    bastion.username = "dev".into();
    bastion.port = 2222;
    bastion.use_socks_proxy = Some(true);
    bastion.socks_proxy_host = Some("192.0.2.9".into());
    let mut hpc = made();
    hpc.name = "hpc".into();
    hpc.host = "node01".into();
    hpc.username = "me".into();
    hpc.connection_protocol = ConnectionProtocol::Mosh;
    hpc.use_socks_proxy = Some(true);
    hpc.socks_proxy_host = Some("192.0.2.10".into());
    hpc.via_connection_id = Some(bastion.id);
    let saved = vec![bastion.clone(), hpc.clone()];
    let spec = super::sessions::spec(&hpc, &saved, &Options::default(), (60000, 60010));
    let Hop::Through(via) = &spec.hop else {
        panic!("{:?}", spec.hop)
    };
    assert_eq!(via.connection_id, bastion.id.to_string());
    assert_eq!(via.name, "bastion");
    assert_eq!(
        via.target,
        moshpit_ssh::Target::new("dev", "bastion.example.com", 2222)
    );
    assert_eq!(
        via.proxy,
        Some(moshpit_ssh::Socks5Proxy::new("192.0.2.9", 1080))
    );
    assert_eq!(spec.target, moshpit_ssh::Target::new("me", "node01", 22));
    assert_eq!(spec.protocol, Protocol::Ssh, "mosh can't take the way");
    assert_eq!(spec.proxy, None, "the jump host's proxy, not its own");

    // The jump host deleted: a route that fails, not a direct dial.
    let gone = super::sessions::spec(&hpc, &[hpc.clone()], &Options::default(), (60000, 60010));
    assert_eq!(gone.hop, Hop::Broken(HopBroken::Deleted));
    // A jump host that goes through another: a second hop, refused.
    let mut chained = bastion.clone();
    chained.via_connection_id = Some(made().id);
    let deeper = super::sessions::spec(
        &hpc,
        &[chained, hpc.clone()],
        &Options::default(),
        (60000, 60010),
    );
    assert_eq!(deeper.hop, Hop::Broken(HopBroken::Chained));
}
