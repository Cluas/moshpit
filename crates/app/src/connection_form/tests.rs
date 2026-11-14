//! The connection form's rules, ported from the Swift suites that held them
//! (`ConnectionDraftDefaultsTests`, `ConnectionCredentialTests`,
//! `ConnectionFormNSHTests`' model parts) and Android's core tests
//! (`crates/android/src/app/tests.rs`), with the rules merged between them.

use super::*;
use moshpit_seal::foundation::date::Date;
use moshpit_store::ssh_keys::{SshKeyAlgorithm, SshKeySource};
use serde_json::json;

fn uuid() -> Uuid {
    Uuid::new_v4()
}

fn blank() -> ServerConnection {
    ServerConnection::new(uuid(), Date(0.0))
}

fn record(
    name: &str,
    algorithm: SshKeyAlgorithm,
    source: SshKeySource,
    account: &str,
) -> SshKeyRecord {
    let mut key = SshKeyRecord::new(uuid(), name, algorithm, source, Date(0.0));
    key.keychain_ref = Some(account.to_owned());
    key
}

fn laptop() -> SshKeyRecord {
    record(
        "laptop",
        SshKeyAlgorithm::Ed25519,
        SshKeySource::Generated,
        "moshpit.secret.key-k",
    )
}

fn phone() -> SshKeyRecord {
    record(
        "phone",
        SshKeyAlgorithm::SeP256,
        SshKeySource::SecureEnclave,
        "moshpit.secret.key-se",
    )
}

/// A saved connection signing with `key`.
fn using(key: &SshKeyRecord) -> ServerConnection {
    let mut c = blank();
    c.auth_method = AuthMethod::Key;
    c.keychain_ref = key.keychain_ref.clone();
    c.ssh_key_id = Some(key.id);
    c.ssh_key_algorithm_raw = Some(key.algorithm.raw_value().to_owned());
    c
}

const FRESH: &str = "moshpit.secret.fresh";

fn apply(
    c: &mut ServerConnection,
    auth: AuthMethod,
    selected: Option<Uuid>,
    keys: &[SshKeyRecord],
    given: Given,
) -> Result<Option<AuthMethod>, Refusal> {
    apply_credential(c, auth, selected, keys, given, || FRESH.to_owned())
}

const NONE: Given = Given {
    password: false,
    key: false,
};
const PASSWORD: Given = Given {
    password: true,
    key: false,
};
const KEY: Given = Given {
    password: false,
    key: true,
};

/// `studio`, a saved connection with every field set (Swift's
/// `ConnectionFormNSHTests.connection()`).
fn studio() -> ServerConnection {
    let mut c = blank();
    c.name = "studio".to_owned();
    c.host = "192.0.2.22".to_owned();
    c.port = 2222;
    c.ssh_port = 2222;
    c.username = "cluas".to_owned();
    c.auth_method = AuthMethod::Key;
    // The key pasted into it when it was made (Save keeps it).
    c.keychain_ref = Some("moshpit.secret.studio".to_owned());
    c.connection_protocol = ConnectionProtocol::Mosh;
    c.set_predict_mode(PredictMode::Never);
    c.roam_on_cellular = Some(false);
    c.set_multiplexer(Multiplexer::Herdr);
    c.herdr_path = Some("/opt/herdr/bin/herdr".to_owned());
    c.use_socks_proxy = Some(true);
    c.socks_proxy_host = Some("proxy.example.com".to_owned());
    c.socks_proxy_port = Some(1081);
    c
}

const UDP: (i64, i64) = (60000, 61000);

fn named(mut draft: Draft) -> Draft {
    draft.fields.name = "box".to_owned();
    draft.fields.host = "box.example.com".to_owned();
    draft.fields.username = "me".to_owned();
    draft
}

// MARK: - A new connection's draft (B256)

#[test]
fn a_new_connection_starts_at_mosh_as_the_setting_says() {
    let draft = Draft::new(true);
    assert!(draft.fields.mosh);
    let shown: serde_json::Value = serde_json::from_str(&draft.message(&[], UDP)).unwrap();
    assert_eq!(shown["mosh"], true, "the form shows the draft's protocol");
    // Nothing else moves off the empty draft.
    let mut rest = draft;
    rest.fields.mosh = false;
    assert_eq!(rest, Draft::new(false));
    assert!(!Draft::new(false).fields.mosh);
}

#[test]
fn an_edited_connection_keeps_its_own_protocol_whatever_the_setting() {
    let mut ssh = blank();
    ssh.connection_protocol = ConnectionProtocol::Ssh;
    assert!(!Draft::of(&ssh).fields.mosh);
    assert!(Draft::of(&studio()).fields.mosh);
}

#[test]
fn a_new_draft_has_the_defaults_both_forms_had() {
    let d = Draft::new(false);
    assert_eq!(d.fields.port, "22");
    assert_eq!(d.fields.socks_port, "1080");
    assert_eq!(d.fields.mux, Mux::Tmux, "tmux unless picked otherwise");
    assert_eq!(d.fields.auth, Auth::Password);
    assert_eq!(d.fields.predict_mode, ScreenPredict::Adaptive);
    assert!(d.fields.roam_on_cellular);
    assert!(!d.fields.editing);
    assert_eq!(d.key, None);
    assert_eq!(d.stored, Stored::None);
}

// MARK: - The credential (ConnectionCredentialTests)

#[test]
fn picking_a_managed_key_points_at_its_own_item_and_writes_nothing() {
    let key = laptop();
    let mut c = blank();
    let write = apply(
        &mut c,
        AuthMethod::Key,
        Some(key.id),
        std::slice::from_ref(&key),
        NONE,
    )
    .unwrap();
    assert_eq!(write, None);
    assert_eq!(c.keychain_ref, key.keychain_ref);
    assert_eq!(c.ssh_key_id, Some(key.id));
    assert_eq!(c.ssh_key_algorithm_raw.as_deref(), Some("ED25519"));
}

#[test]
fn a_deleted_key_with_nothing_pasted_keeps_signing_with_it() {
    for key in [laptop(), phone()] {
        let mut c = using(&key);
        let before = c.clone();
        let write = apply(&mut c, AuthMethod::Key, Some(key.id), &[], NONE).unwrap();
        assert_eq!(write, None);
        assert_eq!(c, before, "{}", key.name);
    }
}

#[test]
fn a_deleted_key_replaced_by_a_pasted_pem_goes_to_a_fresh_item() {
    let key = laptop();
    let mut c = using(&key);
    let write = apply(&mut c, AuthMethod::Key, Some(key.id), &[], KEY).unwrap();
    assert_eq!(write, Some(AuthMethod::Key));
    assert_eq!(c.keychain_ref.as_deref(), Some(FRESH));
    assert_eq!(c.ssh_key_id, None);
    assert_eq!(c.ssh_key_algorithm_raw, None);
}

#[test]
fn switching_a_managed_key_connection_to_password_never_writes_into_the_keys_item() {
    let key = laptop();
    let mut c = using(&key);
    let write = apply(&mut c, AuthMethod::Password, Some(key.id), &[key], PASSWORD).unwrap();
    assert_eq!(write, Some(AuthMethod::Password));
    assert_eq!(c.keychain_ref.as_deref(), Some(FRESH));
    assert_eq!(c.ssh_key_id, None);
}

#[test]
fn switching_away_with_no_new_secret_lets_go_of_the_keys_item() {
    let key = laptop();
    let mut c = using(&key);
    let write = apply(&mut c, AuthMethod::Password, Some(key.id), &[key], NONE).unwrap();
    assert_eq!(write, None);
    // Holding on would make it the connection's own, and deleting the
    // connection would then delete the key.
    assert_eq!(c.keychain_ref, None);
    assert_eq!(c.ssh_key_id, None);
}

#[test]
fn a_connections_own_password_survives_an_edit_that_does_not_retype_it() {
    let mut c = blank();
    c.keychain_ref = Some("moshpit.secret.own".to_owned());
    let write = apply(&mut c, AuthMethod::Password, None, &[laptop()], NONE).unwrap();
    assert_eq!(write, None);
    assert_eq!(c.keychain_ref.as_deref(), Some("moshpit.secret.own"));
}

#[test]
fn a_new_password_for_its_own_item_reuses_that_item() {
    let mut c = blank();
    c.keychain_ref = Some("moshpit.secret.own".to_owned());
    let write = apply(&mut c, AuthMethod::Password, None, &[laptop()], PASSWORD).unwrap();
    assert_eq!(write, Some(AuthMethod::Password));
    assert_eq!(c.keychain_ref.as_deref(), Some("moshpit.secret.own"));
}

/// B298: a connection's own item holds the secret of the method it signs
/// in with. Switching the method with nothing new kept the item under the
/// new method: a pasted key sent as the password, a password read as a key.
#[test]
fn switching_the_method_of_its_own_item_with_no_new_secret_is_refused_and_changes_nothing() {
    for (from, to) in [
        (AuthMethod::Key, AuthMethod::Password),
        (AuthMethod::Password, AuthMethod::Key),
    ] {
        let mut c = blank();
        c.auth_method = from;
        c.keychain_ref = Some("moshpit.secret.own".to_owned());
        let before = c.clone();
        // White space pasted is no key (the host says `key: false`).
        assert_eq!(
            apply(&mut c, to, None, &[laptop()], NONE),
            Err(Refusal::NewSecretNeeded(to))
        );
        assert_eq!(c, before, "{from:?} → {to:?}");
    }
}

#[test]
fn switching_the_method_of_its_own_item_with_the_new_secret_replaces_what_it_held() {
    let mut c = blank();
    c.auth_method = AuthMethod::Key;
    c.keychain_ref = Some("moshpit.secret.own".to_owned());
    assert_eq!(
        apply(&mut c, AuthMethod::Password, None, &[laptop()], PASSWORD),
        Ok(Some(AuthMethod::Password))
    );
    assert_eq!(c.auth_method, AuthMethod::Password);
    assert_eq!(c.keychain_ref.as_deref(), Some("moshpit.secret.own"));
    assert_eq!(
        apply(&mut c, AuthMethod::Key, None, &[laptop()], KEY),
        Ok(Some(AuthMethod::Key))
    );
    assert_eq!(c.auth_method, AuthMethod::Key);
    assert_eq!(c.keychain_ref.as_deref(), Some("moshpit.secret.own"));
}

#[test]
fn a_new_connections_password_gets_a_fresh_item() {
    let mut c = blank();
    let write = apply(&mut c, AuthMethod::Password, None, &[], PASSWORD).unwrap();
    assert_eq!(write, Some(AuthMethod::Password));
    assert_eq!(c.keychain_ref.as_deref(), Some(FRESH));
}

/// The same through the save: it applied the draft's method first, so the
/// credential step could not tell the method had changed (B298).
#[test]
fn saving_a_pasted_key_connection_as_password_with_no_password_is_refused() {
    let mut saved = blank();
    saved.name = "box".to_owned();
    saved.host = "box.example.com".to_owned();
    saved.username = "me".to_owned();
    saved.auth_method = AuthMethod::Key;
    saved.keychain_ref = Some("moshpit.secret.r40b-own".to_owned());
    let mut draft = Draft::of(&saved);
    assert!(draft.set("auth", &json!("password")));
    let plan = draft.save_plan(Some(&saved), blank, &[], UDP, || FRESH.to_owned());
    assert_eq!(plan, Err(Refusal::NewSecretNeeded(AuthMethod::Password)));

    draft.set_given(PASSWORD);
    let plan = draft
        .save_plan(Some(&saved), blank, &[], UDP, || FRESH.to_owned())
        .unwrap();
    assert!(!plan.added);
    assert_eq!(plan.write, Some(AuthMethod::Password));
    assert_eq!(plan.connection.auth_method, AuthMethod::Password);
    assert_eq!(
        plan.connection.keychain_ref.as_deref(),
        Some("moshpit.secret.r40b-own")
    );
    assert_eq!(plan.connection.id, saved.id);
}

#[test]
fn a_save_that_skipped_the_screens_save_rule_is_refused() {
    let mut draft = Draft::new(false);
    draft.set_given(PASSWORD);
    assert_eq!(
        draft.save_plan(None, blank, &[], UDP, || FRESH.to_owned()),
        Err(Refusal::Incomplete),
        "no name, no host"
    );
    let draft = named(draft);
    let plan = draft
        .save_plan(None, blank, &[], UDP, || FRESH.to_owned())
        .unwrap();
    assert!(plan.added);
    assert_eq!(plan.write, Some(AuthMethod::Password));
    assert_eq!(plan.connection.keychain_ref.as_deref(), Some(FRESH));
    assert_eq!(plan.connection.mosh_port_range_start, 60000);
    assert_eq!(plan.connection.mosh_port_range_end, 61000);
}

// MARK: - The screen's view of it (ConnectionFormNSHTests)

#[test]
fn every_draft_is_a_message_the_screen_takes() {
    let fresh = Draft::new(false).message(&[], UDP);
    assert!(Message::parse(&fresh).is_ok(), "{fresh}");
    let fresh: serde_json::Value = serde_json::from_str(&fresh).unwrap();
    assert_eq!(fresh["editing"], false);
    assert_eq!(fresh["port"], "22");
    assert_eq!(fresh["mux"], "tmux");
    assert!(fresh.get("key").is_none());

    let edit = Draft::of(&studio()).message(&[], UDP);
    assert!(Message::parse(&edit).is_ok(), "{edit}");
    let edit: serde_json::Value = serde_json::from_str(&edit).unwrap();
    assert_eq!(edit["editing"], true);
    assert_eq!(edit["auth"], "key");
    assert_eq!(edit["predictMode"], "never");
    assert_eq!(edit["socksPort"], "1081");
    // iOS's spacing (Android's had none, B299).
    assert_eq!(edit["udpRange"], "60000 – 61000");
    assert_eq!(edit["secrets"]["storedKey"], true);
    assert_eq!(edit["secrets"]["storedPassword"], false);
}

/// B294: the password and the pasted key are the host's. Told through the
/// screen, the draft takes neither; what the screen hears is whether there
/// is one.
#[test]
fn a_secret_told_through_the_screen_is_not_taken() {
    let mut draft = Draft::new(false);
    let before = draft.clone();
    assert_eq!(
        draft.handle(&json!({"ev": "set", "key": "password", "value": "from-the-screen"})),
        None
    );
    assert_eq!(
        draft.handle(&json!({"ev": "set", "key": "pem", "value": "-----BEGIN"})),
        None
    );
    assert_eq!(draft, before);

    draft.set_given(Given {
        password: true,
        key: Given::holds_key("-----BEGIN OPENSSH PRIVATE KEY-----"),
    });
    let shown: serde_json::Value = serde_json::from_str(&draft.message(&[], UDP)).unwrap();
    assert_eq!(shown["secrets"]["password"], true);
    assert_eq!(shown["secrets"]["pem"], true);
    assert!(!Given::holds_key(" \n\t"), "white space is no key");
}

/// B290: a key picked in the form, then deleted (or renamed) in SSH Keys in
/// another window: the form went on showing it, and Save stored a key
/// connection with no key and no PEM.
#[test]
fn a_picked_key_deleted_elsewhere_shows_gone() {
    let key = laptop();
    let mut draft = Draft::new(false);
    assert!(draft.pick_key(Some(key.id)));
    assert_eq!(
        draft.fields.auth,
        Auth::Key,
        "a key picked signs in with a key"
    );
    assert!(!draft.pick_key(Some(key.id)), "nothing changed");
    let row = draft.key_row(std::slice::from_ref(&key)).unwrap();
    assert_eq!(row.label, "laptop · ED25519");
    let mut renamed = key.clone();
    renamed.name = "work laptop".to_owned();
    assert_ne!(draft.key_row(&[renamed]), Some(row), "the old name stayed");
    assert_eq!(draft.key_row(&[]), None, "the deleted key stayed");
    let shown: serde_json::Value = serde_json::from_str(&draft.message(&[], UDP)).unwrap();
    assert!(shown.get("key").is_none());
    assert!(draft.pick_key(None));
    assert_eq!(draft.key, None);
}

#[test]
fn what_the_screen_tells_is_an_edit_or_an_intent() {
    let mut draft = Draft::new(false);
    for (key, value) in [
        ("name", "box"),
        ("host", "box.example.com"),
        ("port", "2200"),
        ("username", "root"),
        ("moshServerPath", "/usr/bin/mosh-server"),
        ("socksHost", "127.0.0.1"),
        ("socksPort", "9050"),
        ("tmuxPath", "/usr/bin/tmux"),
        ("herdrPath", "/usr/bin/herdr"),
    ] {
        assert_eq!(
            draft.handle(&json!({"ev": "set", "key": key, "value": value})),
            None
        );
    }
    for (key, value) in [
        ("mosh", json!(true)),
        ("roamOnCellular", json!(false)),
        ("socks", json!(true)),
        ("auth", json!("key")),
        ("predictMode", json!("always")),
        ("mux", json!("none")),
    ] {
        assert_eq!(
            draft.handle(&json!({"ev": "set", "key": key, "value": value})),
            None
        );
    }
    let d = &draft.fields;
    assert_eq!(
        (&*d.name, &*d.host, &*d.port, &*d.username),
        ("box", "box.example.com", "2200", "root")
    );
    assert_eq!(d.mosh_server_path, "/usr/bin/mosh-server");
    assert_eq!((&*d.socks_host, &*d.socks_port), ("127.0.0.1", "9050"));
    assert_eq!(
        (&*d.tmux_path, &*d.herdr_path),
        ("/usr/bin/tmux", "/usr/bin/herdr")
    );
    assert!(d.mosh && !d.roam_on_cellular && d.socks);
    assert_eq!(d.auth, Auth::Key);
    assert_eq!(d.predict_mode, ScreenPredict::Always);
    assert_eq!(d.mux, Mux::None);
    assert!(!draft.can_save(&[]), "a key connection with no key");

    // What it does not know changes nothing.
    let mut draft = Draft::new(false);
    assert!(!draft.set("mosh", &json!("yes")), "a switch takes a flag");
    assert!(!draft.set("mux", &json!("screen")));
    assert!(!draft.set("auth", &json!("token")));
    assert!(!draft.set("predictMode", &json!("sometimes")));
    assert!(!draft.set("colour", &json!("red")));
    assert!(!draft.set("name", &json!(7)));
    assert_eq!(draft, Draft::new(false));
    assert!(!draft.can_save(&[]), "a name and a host first");

    assert_eq!(
        draft.handle(&json!({"ev": "pick-key"})),
        Some(Intent::PickKey)
    );
    assert_eq!(draft.handle(&json!({"ev": "save"})), Some(Intent::Save));
    assert_eq!(draft.handle(&json!({"ev": "cancel"})), Some(Intent::Cancel));
    assert_eq!(draft.handle(&json!({"ev": "delete"})), Some(Intent::Delete));
    assert_eq!(draft.handle(&json!({"ev": "jump"})), None);
    assert_eq!(draft.handle(&json!({})), None);
}

/// B294: "abc" saved as port 22 and "70000" as it was.
#[test]
fn a_port_is_a_whole_number_from_1_to_65535_and_save_waits_for_one() {
    for (text, port) in [
        ("", 22),
        ("  ", 22),
        ("22", 22),
        ("1", 1),
        ("65535", 65535),
        ("0022", 22),
        (" 2222\n", 2222),
    ] {
        assert_eq!(screen::port(text, 22), Some(port), "{text:?}");
    }
    for text in [
        "0",
        "65536",
        "70000",
        "99999999999999999999",
        "abc",
        "22a",
        "-22",
        "+22",
        "2 2",
        "22.0",
        "２２",
        "\u{a0}22",
    ] {
        assert_eq!(screen::port(text, 22), None, "{text:?}");
    }
    assert_eq!(screen::port("", DEFAULT_PROXY_PORT), Some(1080));

    let mut draft = named(Draft::new(false));
    draft.set_given(PASSWORD);
    assert!(draft.can_save(&[]));
    draft.fields.port = "abc".to_owned();
    assert!(
        draft.port_is_wrong() && !draft.can_save(&[]),
        "abc saved as 22"
    );
    draft.fields.port = "70000".to_owned();
    assert!(
        draft.port_is_wrong() && !draft.can_save(&[]),
        "70000 saved as it was"
    );
    draft.fields.port = String::new();
    assert!(!draft.port_is_wrong() && draft.can_save(&[]));
    assert_eq!(draft.applied(&blank(), UDP).port, 22);

    draft.fields.port = " 2222 ".to_owned();
    draft.fields.socks = true;
    draft.fields.socks_host = "127.0.0.1".to_owned();
    draft.fields.socks_port = "99999".to_owned();
    assert!(draft.proxy_port_is_wrong() && !draft.can_save(&[]));
    draft.fields.socks_port = String::new();
    assert!(draft.can_save(&[]));
    let written = draft.applied(&blank(), UDP);
    assert_eq!(
        (written.port, written.ssh_port, written.socks_proxy_port),
        (2222, 2222, Some(1080))
    );
    draft.fields.socks_port = "x".to_owned();
    draft.fields.socks = false;
    assert!(
        !draft.proxy_port_is_wrong() && draft.can_save(&[]),
        "a hidden proxy port held Save"
    );
    assert_eq!(draft.applied(&blank(), UDP).socks_proxy_port, Some(1080));
}

/// B294: a key connection with no key picked and none pasted used to save
/// — and could never sign in; so did a password one with no password.
#[test]
fn a_connection_saves_only_with_something_to_sign_in_with() {
    let key = laptop();
    let keys = std::slice::from_ref(&key);
    let mut draft = named(Draft::new(false));
    draft.fields.auth = Auth::Key;
    assert!(!draft.can_save(keys), "saved with no key");
    draft.set_given(Given {
        password: false,
        key: Given::holds_key(" \n\t"),
    });
    assert!(!draft.can_save(keys), "white space saved as a key");
    draft.set_given(KEY);
    assert!(draft.can_save(keys));
    draft.set_given(NONE);
    draft.key = Some(key.id);
    assert!(draft.can_save(keys));
    assert!(!draft.can_save(&[]), "a picked key deleted since is no key");

    // Edited: what the connection keeps.
    let mut pasted = blank();
    pasted.name = "box".to_owned();
    pasted.host = "box.example.com".to_owned();
    pasted.username = "me".to_owned();
    pasted.auth_method = AuthMethod::Key;
    pasted.keychain_ref = Some("moshpit.secret.own".to_owned());
    let mut kept = Draft::of(&pasted);
    assert_eq!(kept.stored, Stored::PastedKey);
    assert!(kept.keeps_stored_key());
    assert!(kept.can_save(&[]), "its own pasted key stays");
    // White space pasted over it: nothing written over the key it keeps.
    let plan = kept
        .save_plan(Some(&pasted), blank, &[], UDP, || "new".to_owned())
        .unwrap();
    assert_eq!(
        plan.write, None,
        "white space written over the key it keeps"
    );
    assert_eq!(
        plan.connection.keychain_ref.as_deref(),
        Some("moshpit.secret.own")
    );

    let mut managed = pasted.clone();
    managed.ssh_key_id = Some(key.id);
    managed.keychain_ref = key.keychain_ref.clone();
    let mut pointing = Draft::of(&managed);
    assert_eq!(pointing.stored, Stored::ManagedKey(key.id));
    assert!(
        pointing.can_save(&[]),
        "it still points at the key it signs in with"
    );
    pointing.key = Some(uuid());
    assert!(
        !pointing.can_save(&[]),
        "another key, gone since, is no key"
    );

    let mut password = pasted.clone();
    password.auth_method = AuthMethod::Password;
    let mut switched = Draft::of(&password);
    assert_eq!(switched.stored, Stored::Password);
    assert!(switched.keeps_stored_password());
    switched.fields.auth = Auth::Key;
    assert!(!switched.can_save(&[]), "a password kept is no key");

    // A password connection: a password typed, or the one it keeps.
    switched.fields.auth = Auth::Password;
    assert!(switched.can_save(&[]), "the password it keeps");
    let mut fresh = named(Draft::new(false));
    assert!(
        !fresh.can_save(&[]),
        "saved with no password, which nothing asks for later"
    );
    fresh.set_given(PASSWORD);
    assert!(fresh.can_save(&[]), "a password may be white space");
    kept.fields.auth = Auth::Password;
    assert!(!kept.can_save(&[]), "a key kept is no password");

    // What the screen hears of it.
    let shown: serde_json::Value =
        serde_json::from_str(&Draft::of(&pasted).message(&[], UDP)).unwrap();
    assert_eq!(shown["secrets"]["storedKey"], true);
    assert_eq!(shown["secrets"]["storedPassword"], false);
}

#[test]
fn a_saved_connections_draft_writes_it_back_as_it_was() {
    let saved = studio();
    let draft = Draft::of(&saved);
    assert_eq!(draft.fields.port, "2222");
    assert!(draft.fields.mosh);
    assert_eq!(draft.fields.mux, Mux::Herdr);
    assert!(
        draft.fields.mosh_server_path.is_empty(),
        "no path: mosh-server on PATH"
    );
    let written = draft.applied(&saved, UDP);
    assert_eq!(written.id, saved.id);
    assert_eq!(
        (&*written.name, &*written.host, written.port),
        ("studio", "192.0.2.22", 2222)
    );
    assert_eq!((written.ssh_port, &*written.username), (2222, "cluas"));
    assert_eq!(written.connection_protocol, ConnectionProtocol::Mosh);
    assert_eq!(written.predict_mode(), PredictMode::Never);
    assert_eq!(written.roam_on_cellular, Some(false));
    assert_eq!(written.multiplexer(), Multiplexer::Herdr);
    assert!(!written.use_tmux, "the legacy flag kept in step");
    assert_eq!(written.herdr_path.as_deref(), Some("/opt/herdr/bin/herdr"));
    assert_eq!(written.tmux_path, None);
    assert_eq!(written.mosh_server_path, None, "an empty path is none");
    assert_eq!(written.use_socks_proxy, Some(true));
    assert_eq!(
        written.socks_proxy_host.as_deref(),
        Some("proxy.example.com")
    );
    assert_eq!(written.socks_proxy_port, Some(1081));
    assert_eq!(written.mosh_port_range_start, 60000);
    assert_eq!(written.keychain_ref, saved.keychain_ref, "fields only");

    let mut typed = Draft::new(false);
    typed.fields.port = "ssh".to_owned();
    typed.fields.socks_port = String::new();
    let fallback = typed.applied(&blank(), UDP);
    assert_eq!(
        (fallback.port, fallback.socks_proxy_port),
        (22, Some(1080)),
        "what is not a number falls back"
    );
    assert!(fallback.use_tmux, "a new draft is tmux's");
}

#[test]
fn a_connection_saved_before_the_picker_opens_at_tmux_when_it_ran_tmux() {
    let mut legacy = blank();
    legacy.use_tmux = true;
    assert_eq!(Draft::of(&legacy).fields.mux, Mux::Tmux);
    legacy.use_tmux = false;
    assert_eq!(Draft::of(&legacy).fields.mux, Mux::None);
}

#[test]
fn what_a_connection_keeps() {
    let key = laptop();
    let mut c = blank();
    assert_eq!(Stored::of(&c), Stored::None);
    c.keychain_ref = Some("own".to_owned());
    assert_eq!(Stored::of(&c), Stored::Password);
    c.ssh_key_id = Some(key.id);
    assert_eq!(
        Stored::of(&c),
        Stored::None,
        "a password pointing at a key keeps none"
    );
    c.auth_method = AuthMethod::Key;
    assert_eq!(Stored::of(&c), Stored::ManagedKey(key.id));
    c.ssh_key_id = None;
    assert_eq!(Stored::of(&c), Stored::PastedKey);
}

/// For the redesign: whether a secret field is empty, holds what was
/// typed, or stands for what the connection keeps ("Saved").
#[test]
fn a_secret_field_says_whether_it_is_saved_typed_or_empty() {
    let mut fresh = Draft::new(false);
    assert_eq!(fresh.password_state(), SecretState::Empty);
    assert_eq!(fresh.key_state(), SecretState::Empty);
    fresh.set_given(PASSWORD);
    assert_eq!(fresh.password_state(), SecretState::Edited);

    let mut own = blank();
    own.keychain_ref = Some("moshpit.secret.own".to_owned());
    let mut kept = Draft::of(&own);
    assert_eq!(kept.password_state(), SecretState::Saved);
    assert_eq!(
        kept.key_state(),
        SecretState::Empty,
        "a kept password is no key"
    );
    kept.set_given(PASSWORD);
    assert_eq!(
        kept.password_state(),
        SecretState::Edited,
        "typed over the kept one"
    );

    let mut pasted = own.clone();
    pasted.auth_method = AuthMethod::Key;
    let mut draft = Draft::of(&pasted);
    assert_eq!(draft.key_state(), SecretState::Saved);
    assert_eq!(draft.password_state(), SecretState::Empty);
    draft.set_given(KEY);
    assert_eq!(draft.key_state(), SecretState::Edited);

    let key = laptop();
    let managed = using(&key);
    let mut pointing = Draft::of(&managed);
    assert_eq!(
        pointing.key_state(),
        SecretState::Saved,
        "the managed key it signs with"
    );
    pointing.key = Some(uuid());
    assert_eq!(
        pointing.key_state(),
        SecretState::Empty,
        "another key picked"
    );

    // The screen reads the same from what it is posted.
    let shown = Draft::of(&own).shown(&[], UDP);
    assert_eq!(shown.secrets.password_state(), SecretState::Saved);
}

/// r42: a name or host of white space alone saved, and the host and user
/// were stored with the spaces round them (a paste brings a trailing one),
/// which the connect then looked up as typed.
#[test]
fn white_space_is_no_name_or_host_and_the_host_and_user_are_trimmed() {
    let mut draft = named(Draft::new(false));
    draft.set_given(PASSWORD);
    assert!(draft.can_save(&[]));
    draft.fields.name = "  ".to_owned();
    assert!(!draft.can_save(&[]), "a name of spaces");
    draft.fields.name = "box".to_owned();
    draft.fields.host = " \t\n".to_owned();
    assert!(!draft.can_save(&[]), "a host of white space");
    draft.fields.host = "  box.example.com \n".to_owned();
    draft.fields.username = " cluas ".to_owned();
    assert!(draft.can_save(&[]));
    let written = draft.applied(&blank(), UDP);
    assert_eq!(written.host, "box.example.com");
    assert_eq!(written.username, "cluas");
    assert_eq!(written.name, "box");
    assert_eq!(
        draft
            .save_plan(None, blank, &[], UDP, || FRESH.to_owned())
            .map(|p| p.connection.host),
        Ok("box.example.com".to_owned())
    );
    draft.fields.host = "  ".to_owned();
    assert_eq!(
        draft.save_plan(None, blank, &[], UDP, || FRESH.to_owned()),
        Err(Refusal::Incomplete)
    );
}

/// B325: the item the connection edited owned, and no longer names once it
/// signs in with a managed key, is in the plan for the host to drop; an
/// item it still names, or never owned, is not.
#[test]
fn a_save_that_leaves_the_connections_own_item_unnamed_releases_it() {
    let key = laptop();
    let mut saved = blank();
    saved.name = "box".to_owned();
    saved.host = "box.example.com".to_owned();
    saved.username = "me".to_owned();
    saved.auth_method = AuthMethod::Password;
    saved.keychain_ref = Some("moshpit.secret.own".to_owned());

    // Its password, then a managed key: the password's item is released.
    let mut draft = Draft::of(&saved);
    assert!(draft.pick_key(Some(key.id)));
    let plan = draft
        .save_plan(Some(&saved), blank, std::slice::from_ref(&key), UDP, || {
            FRESH.to_owned()
        })
        .unwrap();
    assert_eq!(plan.connection.keychain_ref, key.keychain_ref);
    assert_eq!(plan.released.as_deref(), Some("moshpit.secret.own"));

    // A new password into its own item: the same item, nothing released.
    let mut draft = Draft::of(&saved);
    draft.set_given(PASSWORD);
    let plan = draft
        .save_plan(Some(&saved), blank, &[], UDP, || FRESH.to_owned())
        .unwrap();
    assert_eq!(plan.write, Some(AuthMethod::Password));
    assert_eq!(plan.released, None);

    // The key's item is the key's: leaving it for a password releases nothing.
    let mut on_key = using(&key);
    on_key.name = "box".to_owned();
    on_key.host = "box.example.com".to_owned();
    on_key.username = "me".to_owned();
    let mut draft = Draft::of(&on_key);
    draft.pick_key(None);
    assert!(draft.set("auth", &json!("password")));
    draft.set_given(PASSWORD);
    let plan = draft
        .save_plan(
            Some(&on_key),
            blank,
            std::slice::from_ref(&key),
            UDP,
            || FRESH.to_owned(),
        )
        .unwrap();
    assert_eq!(plan.connection.keychain_ref.as_deref(), Some(FRESH));
    assert_eq!(plan.released, None);

    // A new connection owned nothing before.
    let draft = named(Draft::new(false));
    let mut draft = draft;
    draft.set_given(PASSWORD);
    let plan = draft
        .save_plan(None, blank, &[], UDP, || FRESH.to_owned())
        .unwrap();
    assert_eq!(plan.released, None);
}

/// B328: a saved connection pointed at another host or port is `moved`,
/// so the host forgets the push pairing the box it pointed at before
/// holds; a new name or user, or the host's letters in another case, is
/// not a move, and a new connection moves nothing.
#[test]
fn a_save_pointing_at_another_host_or_port_is_a_move() {
    let mut saved = blank();
    saved.name = "box".to_owned();
    saved.host = "box.example.com".to_owned();
    saved.username = "me".to_owned();
    saved.auth_method = AuthMethod::Password;
    saved.keychain_ref = Some("moshpit.secret.own".to_owned());
    let plan = |edit: &dyn Fn(&mut Draft)| {
        let mut draft = Draft::of(&saved);
        edit(&mut draft);
        draft
            .save_plan(Some(&saved), blank, &[], UDP, || FRESH.to_owned())
            .unwrap()
    };
    assert!(plan(&|d| assert!(d.set("host", &json!("other.example.com")))).moved);
    assert!(plan(&|d| assert!(d.set("port", &json!("2222")))).moved);
    assert!(!plan(&|d| assert!(d.set("name", &json!("renamed")))).moved);
    assert!(!plan(&|d| assert!(d.set("username", &json!("root")))).moved);
    assert!(
        !plan(&|d| assert!(d.set("host", &json!("Box.Example.COM")))).moved,
        "the same host in other letters"
    );
    assert!(!plan(&|_| {}).moved);
    let mut new = named(Draft::new(false));
    new.set_given(PASSWORD);
    let added = new
        .save_plan(None, blank, &[], UDP, || FRESH.to_owned())
        .unwrap();
    assert!(!added.moved);
}

/// B511: a proxy's host is saved without the spaces round it (a pasted
/// `127.0.0.1 ` was looked up as typed and never answered), and with the
/// proxy on and no host the draft doesn't save at all.
#[test]
fn a_proxys_host_is_trimmed_and_required() {
    let mut draft = Draft::of(&studio());
    assert!(draft.set("socksHost", &json!("  127.0.0.1 ")));
    let saved = draft.applied(&studio(), UDP);
    assert_eq!(saved.socks_proxy_host.as_deref(), Some("127.0.0.1"));
    assert_eq!(saved.use_socks_proxy, Some(true));
    assert!(draft.set("socksHost", &json!("   ")));
    assert!(!draft.can_save(&[]), "the proxy on with no host saved");
    assert!(draft.set("socks", &json!(false)));
    assert!(draft.can_save(&[]), "a proxy turned off asks for no host");
}

/// B517: Save waits for a host that is just a host and for a user; what
/// is saved is trimmed.
#[test]
fn a_host_is_just_a_host_and_a_user_is_needed() {
    let mut draft = named(Draft::new(false));
    draft.set_given(PASSWORD);
    assert!(draft.can_save(&[]));
    for pasted in ["dev@box", "ssh://box", "box:2222", "[::1]", "my box"] {
        draft.fields.host = pasted.to_owned();
        assert!(!draft.can_save(&[]), "{pasted} saved");
    }
    draft.fields.host = " 2001:db8::7 ".to_owned();
    assert!(draft.can_save(&[]));
    draft.fields.username = "  ".to_owned();
    assert!(!draft.can_save(&[]), "saved with no user");
    draft.fields.username = " dev ".to_owned();
    assert!(draft.can_save(&[]));
    let saved = draft.applied(&blank(), UDP);
    assert_eq!((&*saved.host, &*saved.username), ("2001:db8::7", "dev"));
}
