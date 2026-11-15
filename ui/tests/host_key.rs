//! The host-key sheet on a headless engine: what a new host and a changed
//! key say, the command and the older algorithm's line (B564), what each
//! button tells the host, that no key trusts, and Chinese.

mod common;

use common::{hear, next, post, quiet, start, zh_hans};
use moshpit_ui::screen::{Params, Screen};
use serde_json::{Value, json};

fn prompt(changes: Value) -> String {
    let mut base = json!({
        "op": "hostKey", "host": "build-box", "port": 2222,
        "fingerprint": "SHA256:k3VJxZ2mJ1u4bS9eQq8yF0aJr6w5nTqP7cXo2Lh1dEs"
    });
    for (key, value) in changes.as_object().unwrap() {
        base[key] = value.clone();
    }
    base.to_string()
}

#[test]
fn a_new_host_shows_its_fingerprint_and_how_to_check_it() {
    let (mut app, channel) = start(Screen::HostKey, Params::default(), "host-key");
    let heard = hear(&channel);
    post(&mut app, &channel, &prompt(json!({})));
    app.wait_for("id:host-key-fingerprint").unwrap();
    let screen = app.screen();
    for text in [
        "New Host",
        "First connection to build-box:2222. Verify this fingerprint matches the server before you trust it.",
        "SHA256:k3VJxZ2mJ1u4bS9eQq8yF0aJr6w5nTqP7cXo2Lh1dEs",
        "ssh-keygen -lf \\\n  /etc/ssh/ssh_host_ed25519_key.pub",
        "Cancel",
        "Trust",
    ] {
        assert!(screen.shows(text), "{text:?}: {:?}", screen.texts());
    }
    // The section titles are eyebrows: compare without their case.
    let texts = screen.texts().join("\n").to_lowercase();
    assert!(texts.contains("sha256 fingerprint"), "{texts}");
    assert!(texts.contains("verify on server"), "{texts}");
    assert!(!screen.matches("id:host-key-older").unwrap());
    assert!(!screen.matches("id:host-key-stored").unwrap());
    for id in ["host-key-fingerprint", "host-key-verify"] {
        assert_eq!(
            screen.in_selectable_view(&format!("id:{id}")),
            Ok(true),
            "{id} can be selected and copied"
        );
    }
    // Return, Space, Enter: no key decides.
    for key in ["enter", "space", "mod+enter"] {
        app.press_key(key).unwrap();
    }
    quiet(&heard);
    app.press("id:host-key-trust").unwrap();
    assert_eq!(next(&heard), json!({"ev": "trust"}));
    app.press("id:host-key-decline").unwrap();
    assert_eq!(next(&heard), json!({"ev": "decline"}));
}

#[test]
fn a_key_by_the_older_algorithm_says_so_and_reads_the_rsa_key() {
    let (mut app, channel) = start(Screen::HostKey, Params::default(), "host-key");
    post(
        &mut app,
        &channel,
        &prompt(json!({"olderAlgorithm": "ssh-rsa"})),
    );
    app.wait_for("id:host-key-older").unwrap();
    let screen = app.screen();
    for text in [
        "ssh-keygen -lf \\\n  /etc/ssh/ssh_host_rsa_key.pub",
        "This server uses an older host-key algorithm (ssh-rsa, SHA-1).",
    ] {
        assert!(screen.shows(text), "{text:?}: {:?}", screen.texts());
    }
    assert!(!screen.shows("ssh-keygen -lf \\\n  /etc/ssh/ssh_host_ed25519_key.pub"));
}

#[test]
fn a_changed_key_shows_both_and_trusting_it_is_destructive() {
    let (mut app, channel) = start(Screen::HostKey, Params::default(), "host-key");
    let heard = hear(&channel);
    post(
        &mut app,
        &channel,
        &prompt(json!({"previous": "SHA256:old0ld0ld0ld0ld0ld0ld0ld0ld0ld0ld0ld0ld0ld"})),
    );
    app.wait_for("id:host-key-offered").unwrap();
    let screen = app.screen();
    for text in [
        "Host Key Changed",
        // The catalog's emphasis is a bold run, never its asterisks.
        "The key for build-box:2222 does not match what's stored here. The server may have been reinstalled — or the connection is being intercepted.",
        "SHA256:old0ld0ld0ld0ld0ld0ld0ld0ld0ld0ld0ld0ld0ld",
        "SHA256:k3VJxZ2mJ1u4bS9eQq8yF0aJr6w5nTqP7cXo2Lh1dEs",
        "Disconnect",
        "Trust New Key",
    ] {
        assert!(screen.shows(text), "{text:?}: {:?}", screen.texts());
    }
    assert!(!screen.texts().iter().any(|t| t.contains("**")));
    let texts = screen.texts().join("\n").to_lowercase();
    assert!(
        texts.contains("stored") && texts.contains("offered now"),
        "{texts}"
    );
    // A changed key has no command to run and no Cancel: Disconnect is the
    // way out.
    assert!(!screen.matches("id:host-key-verify").unwrap());
    assert!(!screen.shows("Cancel"));
    assert!(!screen.shows("Trust"));
    for id in ["host-key-stored", "host-key-offered"] {
        assert_eq!(
            screen.in_selectable_view(&format!("id:{id}")),
            Ok(true),
            "{id}"
        );
    }
    for key in ["enter", "space"] {
        app.press_key(key).unwrap();
    }
    quiet(&heard);
    app.press("id:host-key-decline").unwrap();
    assert_eq!(next(&heard), json!({"ev": "decline"}));
    app.press("id:host-key-trust").unwrap();
    assert_eq!(next(&heard), json!({"ev": "trust"}));
}

#[test]
fn in_chinese() {
    let (mut app, channel) = start(Screen::HostKey, zh_hans(), "host-key");
    post(&mut app, &channel, &prompt(json!({})));
    app.wait_for("id:host-key-fingerprint").unwrap();
    let screen = app.screen();
    for text in ["新主机", "信任", "取消"] {
        assert!(screen.shows(text), "{text}: {:?}", screen.texts());
    }
    assert!(
        screen
            .texts()
            .iter()
            .any(|t| t.starts_with("第一次连接 build-box:2222。")),
        "{:?}",
        screen.texts()
    );
    post(
        &mut app,
        &channel,
        &prompt(json!({"previous": "SHA256:old"})),
    );
    app.wait_for("id:host-key-offered").unwrap();
    let screen = app.screen();
    assert!(screen.shows("主机密钥已变更"), "{:?}", screen.texts());
    assert!(screen.shows("断开"), "{:?}", screen.texts());
    assert!(screen.shows("信任新密钥"), "{:?}", screen.texts());
    assert!(
        screen
            .texts()
            .iter()
            .any(|t| t.contains("不一样") && !t.contains("**")),
        "{:?}",
        screen.texts()
    );
}

/// The sheet draws only with the tokens, so it follows the system's
/// appearance.
#[test]
fn it_follows_the_system() {
    assert!(Screen::HostKey.follows_system());
    assert!(Screen::HostKey.draws_light());
}
