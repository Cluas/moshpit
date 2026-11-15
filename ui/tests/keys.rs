//! Settings › SSH Keys, Add Key and Select Key on a headless engine: what
//! the host posts shows (metadata only: no key material ever comes this
//! way), what the user does reaches the host, the system's alerts and
//! action sheets the pages show, the app's language and the identifiers
//! the device tests find.

mod common;

use common::{emit, hear, next, post, quiet, start, start_observing, wait_dialog, zh_hans};
use moshpit_ui::{
    screen::{Params, Screen},
    ssh_keys::{self, Avatar, Key, Keys, Notice},
};
use nsh_testing::DialogAnswer;
use serde_json::json;

fn key(id: &str, name: &str, avatar: Avatar) -> Key {
    Key {
        id: id.into(),
        name: name.into(),
        badge: if avatar == Avatar::SecureEnclave {
            "SE · P256".into()
        } else {
            "ED25519".into()
        },
        avatar,
        stale: false,
        fingerprint: format!("SHA256:{id}fingerprint"),
        hosts: vec!["m1-pro".into(), "nas".into()],
        unused_since: None,
        has_public_key: true,
    }
}

fn keys(device: Vec<Key>, imported: Vec<Key>, deleting: Option<&str>) -> String {
    ssh_keys::Message::Keys(Keys {
        device,
        imported,
        deleting: deleting.map(Into::into),
        notice: None,
    })
    .to_json()
}

fn laptop() -> Key {
    key("1A2B", "laptop", Avatar::Imported)
}

fn enclave() -> Key {
    key("SE01", "iPad", Avatar::SecureEnclave)
}

fn old_rsa() -> Key {
    Key {
        badge: "RSA-3072".into(),
        fingerprint: String::new(),
        stale: true,
        hosts: vec![],
        unused_since: Some("Mar 2026".into()),
        ..key("OLD1", "old box", Avatar::Gray)
    }
}

#[test]
fn the_keys_show_with_their_badges_fingerprints_and_hosts() {
    let (mut app, channel) = start(Screen::SshKeys, Params::default(), "ssh-keys");
    post(
        &mut app,
        &channel,
        &keys(vec![enclave()], vec![laptop(), old_rsa()], None),
    );
    app.wait_for("id:ssh-key-row-1A2B").unwrap();
    for text in [
        "SSH Keys",
        "THIS DEVICE · SECURE ENCLAVE",
        "IMPORTED",
        "iPad",
        "SE · P256",
        "laptop",
        "ED25519",
        "SHA256:1A2Bfingerprint",
        "Used for m1-pro, nas",
        "old box",
        "RSA-3072",
        "unused since Mar 2026",
        "Hardware-backed keys cannot be exported; every signature triggers Face ID.",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(!app.screen().matches("id:ssh-keys-no-device").unwrap());
    assert!(!app.screen().matches("id:ssh-keys-no-imported").unwrap());
    channel.close();
}

/// A key's row says its badge right after its name: for a screen reader
/// it is what tells two keys apart (no one reads a fingerprint out).
#[test]
fn a_keys_row_says_its_badge_after_its_name() {
    let (mut app, channel) = start(Screen::SshKeys, Params::default(), "ssh-keys");
    post(
        &mut app,
        &channel,
        &keys(vec![enclave()], vec![laptop(), old_rsa()], None),
    );
    app.wait_for("id:ssh-key-row-1A2B").unwrap();
    for label in [
        "iPad, SE · P256, SHA256:SE01fingerprint, Used for m1-pro, nas",
        "laptop, ED25519, SHA256:1A2Bfingerprint, Used for m1-pro, nas",
        "old box, RSA-3072, unused since Mar 2026",
    ] {
        assert!(
            app.screen().matches(&format!("label:{label}")).unwrap(),
            "{label}"
        );
    }
    channel.close();
}

/// R10 (§8): an empty group has the button that fills it — Generate key
/// for this device, Import key for the imported — and each opens Add Key
/// on its method.
#[test]
fn an_empty_group_has_the_button_that_fills_it() {
    let (mut app, channel) = start(Screen::SshKeys, Params::default(), "ssh-keys");
    let told = hear(&channel);
    post(&mut app, &channel, &keys(vec![], vec![], None));
    app.wait_for("id:ssh-keys-no-device").unwrap();
    assert!(app.screen().shows("No device key yet"));
    assert!(app.screen().shows("No imported keys"));
    assert!(app.screen().matches("label:Generate key").unwrap());
    app.press("id:ssh-keys-generate").unwrap();
    assert_eq!(next(&told), json!({"ev": "add", "method": "generate"}));
    app.press("id:ssh-keys-import").unwrap();
    assert_eq!(next(&told), json!({"ev": "add", "method": "import"}));
    post(&mut app, &channel, &keys(vec![], vec![laptop()], None));
    app.wait_for("id:ssh-key-row-1A2B").unwrap();
    assert!(!app.screen().matches("id:ssh-keys-no-imported").unwrap());
    assert!(!app.screen().matches("id:ssh-keys-import").unwrap());
    assert!(app.screen().matches("id:ssh-keys-generate").unwrap());
    quiet(&told);
    channel.close();
}

#[test]
fn a_keys_row_opens_its_actions_in_the_action_sheet() {
    let (mut app, channel) = start(Screen::SshKeys, Params::default(), "ssh-keys");
    app.answer_dialogs_myself();
    let told = hear(&channel);
    post(
        &mut app,
        &channel,
        &keys(vec![], vec![laptop(), old_rsa()], None),
    );
    app.wait_for("id:ssh-key-row-1A2B").unwrap();
    app.press("id:ssh-keys-add").unwrap();
    assert_eq!(next(&told), json!({"ev": "add"}));
    app.press("id:ssh-keys-done").unwrap();
    assert_eq!(next(&told), json!({"ev": "done"}));
    assert!(app.screen().matches("label:Add Key").unwrap());
    // A press opens the key's four actions; a key without a fingerprint
    // has no Copy Fingerprint.
    app.press("id:ssh-key-row-1A2B").unwrap();
    assert!(wait_dialog(&mut app), "the row's actions never showed");
    let sheet = app.pending_dialog().cloned().unwrap();
    assert!(sheet.sheet);
    assert_eq!(sheet.title.as_deref(), Some("laptop"));
    assert_eq!(
        common::dialog_buttons(&sheet),
        [
            "Copy Public Key",
            "Copy Fingerprint",
            "Share Public Key",
            "Delete Key",
            "Cancel"
        ]
    );
    app.answer_dialog(DialogAnswer::Press("Copy Fingerprint".into()))
        .unwrap();
    assert_eq!(next(&told), json!({"ev": "copy-fingerprint", "id": "1A2B"}));
    app.wait_for("text:Fingerprint copied").unwrap();
    app.press("id:ssh-key-row-OLD1").unwrap();
    assert!(wait_dialog(&mut app));
    let sheet = app.pending_dialog().cloned().unwrap();
    assert!(!common::dialog_buttons(&sheet).contains(&"Copy Fingerprint"));
    app.answer_dialog(DialogAnswer::Press("Delete Key".into()))
        .unwrap();
    assert_eq!(next(&told), json!({"ev": "confirm-delete", "id": "OLD1"}));
    quiet(&told);
    channel.close();
}

/// R10 (§8): the delete the host asks about is the system's alert
/// (`alert_dialog`), once for each time it asks.
#[test]
fn a_delete_asks_in_the_system_alert() {
    let (mut app, channel) = start(Screen::SshKeys, Params::default(), "ssh-keys");
    app.answer_dialogs_myself();
    let told = hear(&channel);
    post(
        &mut app,
        &channel,
        &keys(vec![], vec![laptop()], Some("1A2B")),
    );
    assert!(wait_dialog(&mut app), "the delete never asked");
    let alert = app.pending_dialog().cloned().unwrap();
    assert!(!alert.sheet);
    assert_eq!(alert.title.as_deref(), Some("Delete laptop?"));
    assert!(
        alert
            .message
            .as_deref()
            .is_some_and(|m| m.starts_with("Its private key is erased"))
    );
    assert_eq!(common::dialog_buttons(&alert), ["Delete", "Cancel"]);
    app.answer_dialog(DialogAnswer::Press("Cancel".into()))
        .unwrap();
    assert_eq!(next(&told), json!({"ev": "cancel-delete"}));
    post(
        &mut app,
        &channel,
        &keys(vec![], vec![laptop()], Some("1A2B")),
    );
    assert!(wait_dialog(&mut app), "a second ask never showed");
    // The host saying the same again before it heard asks nothing more.
    post(
        &mut app,
        &channel,
        &keys(vec![], vec![laptop()], Some("1A2B")),
    );
    app.answer_dialog(DialogAnswer::Press("Delete".into()))
        .unwrap();
    assert_eq!(next(&told), json!({"ev": "delete", "id": "1A2B"}));
    assert!(app.pending_dialog().is_none());
    // About a key that is gone, nothing asks.
    post(&mut app, &channel, &keys(vec![], vec![], Some("1A2B")));
    assert!(!wait_dialog(&mut app));
    quiet(&told);
    channel.close();
}

/// B232: a key a saved connection still signs in with isn't deleted, and
/// the host says so — in the system's alert, its words, with one way out.
#[test]
fn the_notice_the_host_sends_when_a_key_stays() {
    let (mut app, channel) = start(Screen::SshKeys, Params::default(), "ssh-keys");
    app.answer_dialogs_myself();
    let told = hear(&channel);
    let message = ssh_keys::Message::Keys(Keys {
        device: vec![],
        imported: vec![laptop()],
        deleting: None,
        notice: Some(Notice {
            title: "Key in use".into(),
            message: "Connections using it: pi and nas. Choose another key for them first, then delete this one.".into(),
        }),
    })
    .to_json();
    post(&mut app, &channel, &message);
    assert!(wait_dialog(&mut app), "the notice never showed");
    let alert = app.pending_dialog().cloned().unwrap();
    assert_eq!(alert.title.as_deref(), Some("Key in use"));
    assert_eq!(
        alert.message.as_deref(),
        Some(
            "Connections using it: pi and nas. Choose another key for them first, then delete this one."
        )
    );
    // Nothing to delete from here: only the way out.
    assert_eq!(common::dialog_buttons(&alert), ["OK"]);
    app.answer_dialog(DialogAnswer::Press("OK".into())).unwrap();
    assert_eq!(next(&told), json!({"ev": "dismiss-notice"}));
    quiet(&told);
    channel.close();
}

#[test]
fn the_keys_page_speaks_the_apps_language() {
    let (mut app, channel) = start(Screen::SshKeys, zh_hans(), "ssh-keys");
    post(&mut app, &channel, &keys(vec![], vec![old_rsa()], None));
    app.wait_for("id:ssh-key-row-OLD1").unwrap();
    for text in [
        "SSH 密钥",
        "完成",
        "本机 · SECURE ENCLAVE",
        "自 Mar 2026 起未使用",
        "还没有设备密钥",
        "生成密钥",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}

// --- Select Key -------------------------------------------------------------

mod picker {
    use super::*;
    use moshpit_ui::key_picker::{self, Picker, PickerKey};

    fn picker(keys: Vec<PickerKey>, selected: Option<&str>) -> String {
        key_picker::Message::Keys(Picker {
            keys,
            selected: selected.map(Into::into),
        })
        .to_json()
    }

    fn saved(id: &str, name: &str, refused: Option<&str>) -> PickerKey {
        PickerKey {
            id: id.into(),
            name: name.into(),
            badge: "ED25519".into(),
            refused: refused.map(Into::into),
        }
    }

    #[test]
    fn the_saved_keys_the_pick_and_a_refused_key() {
        let (mut app, channel) = start(Screen::KeyPicker, Params::default(), "key-picker");
        let told = hear(&channel);
        post(
            &mut app,
            &channel,
            &picker(
                vec![
                    saved("a", "laptop", None),
                    saved("b", "old p384", Some("ECDSA P-384")),
                ],
                Some("a"),
            ),
        );
        app.wait_for("id:key-picker-key-a").unwrap();
        for text in [
            "Select Key",
            "SAVED KEYS",
            "laptop",
            "ED25519",
            "old p384",
            "Can't sign in with ECDSA P-384 keys yet",
            "OR",
            "Paste a PEM instead…",
            "Enter a private key by hand",
        ] {
            assert!(
                app.screen().shows(text),
                "{text}: {:?}",
                app.screen().texts()
            );
        }
        assert!(app.screen().selected("id:key-picker-key-a").unwrap());
        assert!(!app.screen().selected("id:key-picker-paste-pem").unwrap());
        // A refused key can't be picked.
        let _ = app.press("id:key-picker-key-b");
        quiet(&told);
        app.press("id:key-picker-paste-pem").unwrap();
        assert_eq!(next(&told), json!({"ev": "select", "id": null}));
        app.press("id:key-picker-key-a").unwrap();
        assert_eq!(next(&told), json!({"ev": "select", "id": "a"}));
        app.press("id:key-picker-add").unwrap();
        assert_eq!(next(&told), json!({"ev": "add"}));
        app.press("id:key-picker-cancel").unwrap();
        assert_eq!(next(&told), json!({"ev": "cancel"}));
        // The paste row is the pick when no key is.
        post(
            &mut app,
            &channel,
            &picker(vec![saved("a", "laptop", None)], None),
        );
        assert!(app.screen().selected("id:key-picker-paste-pem").unwrap());
        assert!(!app.screen().selected("id:key-picker-key-a").unwrap());
        channel.close();
    }

    #[test]
    fn no_keys_offers_to_make_one() {
        let (mut app, channel) = start(Screen::KeyPicker, Params::default(), "key-picker");
        let told = hear(&channel);
        post(&mut app, &channel, &picker(vec![], None));
        app.wait_for("id:key-picker-empty-generate").unwrap();
        assert!(app.screen().shows("No keys yet"));
        assert!(app.screen().shows("Generate key"));
        app.press("id:key-picker-empty-generate").unwrap();
        assert_eq!(next(&told), json!({"ev": "add"}));
        channel.close();
    }

    #[test]
    fn the_picker_speaks_the_apps_language() {
        let (mut app, channel) = start(Screen::KeyPicker, zh_hans(), "key-picker");
        post(&mut app, &channel, &picker(vec![], None));
        app.wait_for("id:key-picker-empty-generate").unwrap();
        for text in ["选择密钥", "取消", "直接粘贴 PEM…"] {
            assert!(
                app.screen().shows(text),
                "{text}: {:?}",
                app.screen().texts()
            );
        }
        channel.close();
    }
}

// --- Add Key ----------------------------------------------------------------

mod add {
    use super::*;
    use moshpit_ui::add_key::{self, AddKey, Algorithm, Method, Pem};

    fn form() -> AddKey {
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
        }
    }

    fn post_form(
        app: &mut nsh_testing::Headless,
        channel: &moshpit_ui::channel::Channel,
        form: AddKey,
    ) {
        post(app, channel, &add_key::Message::AddKey(form).to_json());
    }

    #[test]
    fn generate_shows_its_groups_and_the_controls_carry_the_form() {
        let (mut app, channel, natives) =
            start_observing(Screen::AddKey, Params::default(), "add-key");
        let told = hear(&channel);
        post_form(&mut app, &channel, form());
        app.wait_for("id:add-key-name").unwrap();
        for text in [
            "Add Key",
            "KEY DETAILS",
            "Name",
            "Algorithm",
            "ED25519",
            "Comment",
            "PROTECTION",
            "Require Face ID",
            "Store in Secure Enclave",
            "Fingerprint",
            "—:—:—:—:—:—:—:—:—:—  (shown after generation)",
            "Generate",
        ] {
            assert!(
                app.screen().shows(text),
                "{text}: {:?}",
                app.screen().texts()
            );
        }
        let method = natives.find("moshpit/segmented", "add-key-method").unwrap();
        assert_eq!(natives.prop(method, "selected"), Some(json!(0)));
        let face_id = natives.find("moshpit/switch", "add-key-face-id").unwrap();
        assert_eq!(natives.prop(face_id, "on"), Some(json!(true)));
        // B602: no BIND TO HOSTS — a label stored on the key that bound
        // nothing, which SSH Keys then showed as the hosts it is used for.
        assert!(!app.screen().shows("BIND TO HOSTS"));
        assert!(app.screen().id("id:add-key-host-nas").is_err());
        // Generate waits for the host's word.
        assert!(!app.screen().enabled("id:add-key-submit").unwrap());
        post_form(
            &mut app,
            &channel,
            AddKey {
                can_submit: true,
                ..form()
            },
        );
        assert!(app.screen().enabled("id:add-key-submit").unwrap());
        app.press("id:add-key-submit").unwrap();
        assert_eq!(next(&told), json!({"ev": "submit"}));
        emit(&mut app, face_id, "change", json!(false));
        assert_eq!(
            next(&told),
            json!({"ev": "set", "key": "requireFaceID", "value": false})
        );
        let algorithm = natives.find("moshpit/menu", "add-key-algorithm").unwrap();
        emit(&mut app, algorithm, "select", json!("rsa"));
        assert_eq!(next(&told), json!({"ev": "algorithm", "value": "rsa"}));
        emit(&mut app, method, "change", json!(1));
        assert_eq!(next(&told), json!({"ev": "method", "value": "import"}));
        app.type_text("id:add-key-name-input", "work").unwrap();
        let named = next(&told);
        assert_eq!(named["ev"], json!("name"));
        app.press("id:add-key-cancel").unwrap();
        // What typing told is drained; the next is the cancel.
        let mut last = next(&told);
        while last["ev"] == json!("name") {
            last = next(&told);
        }
        assert_eq!(last, json!({"ev": "cancel"}));
        channel.close();
    }

    #[test]
    fn import_lays_out_the_hosts_editor_and_asks_for_a_passphrase_when_needed() {
        let (mut app, channel, natives) =
            start_observing(Screen::AddKey, Params::default(), "add-key");
        let told = hear(&channel);
        post_form(
            &mut app,
            &channel,
            AddKey {
                method: Method::Import,
                ..form()
            },
        );
        app.wait_for("id:add-key-import-file").unwrap();
        assert!(app.screen().shows("PRIVATE KEY"));
        assert!(app.screen().shows("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(
            app.screen().shows("Import"),
            "the bar's title follows the method"
        );
        assert_eq!(natives.count("moshpit/key-pem"), 1, "the host's editor");
        assert_eq!(natives.count("moshpit/key-passphrase"), 0);
        assert!(
            app.screen().shows("Private key"),
            "the editor's visible label"
        );
        assert!(app.screen().shows(
            "Paste an OpenSSH or PEM private key; the public key and fingerprint come from it, so the public key line is optional."
        ));
        // A key behind a passphrase: its field, the other footer, no placeholder.
        post_form(
            &mut app,
            &channel,
            AddKey {
                method: Method::Import,
                pem: Pem {
                    empty: false,
                    encrypted: true,
                },
                ..form()
            },
        );
        assert_eq!(natives.count("moshpit/key-passphrase"), 1);
        assert!(!app.screen().shows("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert_eq!(natives.count("moshpit/key-passphrase"), 1);
        assert!(app.screen().shows("Passphrase"), "the secret field's label");
        assert!(app.screen().shows(
            "This key is protected by a passphrase. It's used once, to unlock the key here; from then on the Keychain protects it, behind Face ID when required."
        ));
        app.press("id:add-key-import-file").unwrap();
        assert_eq!(
            next(&told),
            json!({"ev": "import-file", "target": "private"})
        );
        app.press("id:add-key-import-public").unwrap();
        assert_eq!(
            next(&told),
            json!({"ev": "import-file", "target": "public"})
        );
        channel.close();
    }

    #[test]
    fn a_file_picked_for_the_public_line_reaches_its_field() {
        let (mut app, channel) = start(Screen::AddKey, Params::default(), "add-key");
        post_form(
            &mut app,
            &channel,
            AddKey {
                method: Method::Import,
                ..form()
            },
        );
        app.wait_for("id:add-key-public-line-input").unwrap();
        post_form(
            &mut app,
            &channel,
            AddKey {
                method: Method::Import,
                public_line: "ssh-ed25519 AAAA laptop".into(),
                fields: 2,
                ..form()
            },
        );
        assert_eq!(
            app.screen()
                .input_text("id:add-key-public-line-input")
                .unwrap(),
            Some("ssh-ed25519 AAAA laptop")
        );
        channel.close();
    }

    #[test]
    fn a_passphrase_coming_and_going_keeps_what_was_typed() {
        let (mut app, channel, natives) =
            start_observing(Screen::AddKey, Params::default(), "add-key");
        post_form(
            &mut app,
            &channel,
            AddKey {
                method: Method::Import,
                ..form()
            },
        );
        app.wait_for("id:add-key-public-line-input").unwrap();
        app.type_text("id:add-key-public-line-input", "ssh-ed25519 AAAA typed")
            .unwrap();
        let editor = natives.only("moshpit/key-pem").unwrap();
        // The host's editor says the pasted key needs a passphrase.
        post_form(
            &mut app,
            &channel,
            AddKey {
                method: Method::Import,
                pem: Pem {
                    empty: false,
                    encrypted: true,
                },
                ..form()
            },
        );
        assert_eq!(natives.count("moshpit/key-passphrase"), 1);
        assert_eq!(
            app.screen()
                .input_text("id:add-key-public-line-input")
                .unwrap(),
            Some("ssh-ed25519 AAAA typed"),
            "the field is not made again"
        );
        assert_eq!(
            natives.only("moshpit/key-pem"),
            Some(editor),
            "nor is the host's editor (it would drop the keyboard)"
        );
        channel.close();
    }

    /// A security key (Hardware) is not offered: the page shows Generate.
    /// Why a key could not be made comes in the system's alert, once.
    #[test]
    fn hardware_shows_as_generate_and_errors_come_as_an_alert() {
        let (mut app, channel, natives) =
            start_observing(Screen::AddKey, Params::default(), "add-key");
        app.answer_dialogs_myself();
        let told = hear(&channel);
        post_form(
            &mut app,
            &channel,
            AddKey {
                method: Method::Hardware,
                ..form()
            },
        );
        app.wait_for("id:add-key-name").unwrap();
        let method = natives.find("moshpit/segmented", "add-key-method").unwrap();
        assert_eq!(natives.prop(method, "selected"), Some(json!(0)));
        post_form(
            &mut app,
            &channel,
            AddKey {
                error: Some("Unsupported: DSA keys".into()),
                ..form()
            },
        );
        assert!(wait_dialog(&mut app), "the error never showed");
        let alert = app.pending_dialog().cloned().unwrap();
        assert_eq!(alert.title.as_deref(), Some("Key error"));
        assert_eq!(alert.message.as_deref(), Some("Unsupported: DSA keys"));
        app.answer_dialog(DialogAnswer::Press("OK".into())).unwrap();
        assert_eq!(next(&told), json!({"ev": "dismiss-error"}));
        // The host's post before it heard the OK says nothing again.
        post_form(
            &mut app,
            &channel,
            AddKey {
                error: Some("Unsupported: DSA keys".into()),
                ..form()
            },
        );
        assert!(app.pending_dialog().is_none());
        channel.close();
    }

    #[test]
    fn add_key_speaks_the_apps_language() {
        let (mut app, channel) = start(Screen::AddKey, zh_hans(), "add-key");
        post_form(&mut app, &channel, form());
        app.wait_for("id:add-key-name").unwrap();
        for text in ["添加密钥", "生成", "需要 Face ID"] {
            assert!(
                app.screen().shows(text),
                "{text}: {:?}",
                app.screen().texts()
            );
        }
        channel.close();
    }
}

// --- Dynamic Type -------------------------------------------------------------

#[test]
fn the_keys_pages_follow_dynamic_type() {
    use moshpit_ui::ui::typography::TextToken;
    let (mut app, channel) = start(Screen::SshKeys, Params::default(), "ssh-keys");
    post(
        &mut app,
        &channel,
        &keys(vec![enclave()], vec![laptop(), old_rsa()], None),
    );
    app.wait_for("id:ssh-key-row-1A2B").unwrap();
    let size = |app: &nsh_testing::Headless, selector: &str| {
        app.screen().span_style(selector).unwrap().font_size.0
    };
    // A key's name is a row's title (body), its fingerprint the row's
    // subtitle (mono subhead), each the type scale's at the text size.
    for scale in [1.0, 23.0 / 17.0] {
        app.set_environment("size", |env| env.font_scale = scale)
            .unwrap();
        assert_eq!(size(&app, "text:laptop"), TextToken::Body.size(scale));
        assert_eq!(
            size(&app, "text:SHA256:1A2Bfingerprint"),
            TextToken::MonoSubhead.size(scale)
        );
    }
    assert_eq!(size(&app, "text:SSH Keys"), 21.0, "the bar's title");
    app.set_environment("AX5", |env| env.font_scale = 53.0 / 17.0)
        .unwrap();
    assert!(app.screen().matches("id:ssh-key-row-1A2B").unwrap());
    assert_eq!(size(&app, "text:SSH Keys"), 21.0, "and no further");
    channel.close();
}

/// Where the text `words` starts, in the root's points.
fn text_x(app: &common::App, words: &str) -> f32 {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find_map(|n| {
            n.text
                .as_ref()
                .filter(|t| t.text.text == words)
                .map(|t| t.content.origin.x.0)
        })
        .unwrap_or_else(|| panic!("no text {words:?}"))
}

/// The left edge of the view with test id `id`.
fn left(app: &common::App, id: &str) -> f32 {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|n| n.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no view {id}"))
        .bounds
        .origin
        .x
        .0
}

/// An empty group is a row of its card: its symbol in the row's leading
/// lane, its words where a key's name starts, not against the card's edge.
#[test]
fn an_empty_group_lines_up_with_the_keys_rows() {
    let (mut app, channel) = start(Screen::SshKeys, Params::default(), "ssh-keys");
    post(&mut app, &channel, &keys(vec![], vec![laptop()], None));
    app.wait_for("id:ssh-keys-no-device").unwrap();
    app.wait_for("id:ssh-key-row-1A2B").unwrap();
    let card = left(&app, "ssh-keys-no-device");
    assert_eq!(
        text_x(&app, "No device key yet"),
        text_x(&app, "laptop"),
        "the empty group's words start where a key's name does"
    );
    assert!(
        text_x(&app, "No device key yet") - card >= 16.0 + 28.0,
        "past the row's padding and its leading lane"
    );
    channel.close();
}
