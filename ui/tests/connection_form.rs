//! Add connection / Edit connection on a headless engine (R9, §6.5): the
//! draft the host posts shows in its sections, every field labelled; the
//! rows that come and go with it (the password or the key and its PEM
//! editor, mosh's tuning, the proxy's host and port, the multiplexer's
//! path); each edit reaches the host as it is made; Save only with a name,
//! a host, ports and a credential; a field's error once it is left; Return
//! from field to field; the host's controls and menus; the app's language;
//! both appearances. The password and the private key are the host's own
//! fields: the screen never holds or tells them.

mod common;

use common::{
    App, assert_bar_follows_scrolling, emit, hear, next, post, quiet, start, start_observing_with,
    zh_hans,
};
use moshpit_ui::{
    connection_form::{
        Auth, ConnectionPredict, FormState, KeyChoice, Message, Missing, Mux, Other, SaveFailure,
        Secrets, ViaChoice,
    },
    screen::{Params, Screen},
};
use nsh_core::{Points, Size};
use serde_json::json;

fn draft() -> FormState {
    FormState {
        editing: false,
        port: "22".into(),
        udp_range: "60000 – 61000".into(),
        socks_port: "1080".into(),
        mux: Mux::Tmux,
        ..FormState::default()
    }
}

fn show(app: &mut App, channel: &moshpit_ui::channel::Channel, state: FormState) {
    post(app, channel, &Message::Form(Box::new(state)).to_json());
}

fn find(app: &mut App, selector: &str) {
    app.wait_for(selector)
        .unwrap_or_else(|e| panic!("{selector}: {e}: {:?}", app.screen().texts()));
}

/// The form on a screen tall enough for every section (Delete connection…
/// at the end included).
fn start_tall(
    params: Params,
) -> (
    App,
    std::sync::Arc<moshpit_ui::channel::Channel>,
    common::Natives,
) {
    start_observing_with(
        Screen::ConnectionForm,
        params,
        "connection-form",
        |config| {
            config.viewport = Size::new(Points(580.0), Points(2600.0));
        },
    )
}

/// The control of field `id`.
fn input(id: &str) -> String {
    format!("id:{id}-input")
}

#[test]
fn a_new_connection_shows_every_section() {
    let (mut app, channel, natives) = start_tall(Params::default());
    show(&mut app, &channel, draft());
    find(&mut app, "id:connection-form-name");
    for text in [
        "Add connection",
        "Cancel",
        "Save",
        "CONNECTION",
        "Name",
        "Host",
        "Port",
        "User",
        "SIGN IN",
        "Password",
        "MOSH",
        "Use mosh",
        "Survives network changes; needs UDP.",
        "UDP ports are set in Settings, for every connection.",
        "PROXY",
        "SOCKS5",
        "Proxies SSH only; mosh's UDP goes direct.",
        "ADVANCED",
        "Multiplexer",
        "tmux",
        "tmux path",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    // Placeholders are examples.
    for (id, example) in [
        ("connection-form-name", "my-server"),
        ("connection-form-host", "example.com"),
        ("connection-form-port", "22"),
        ("connection-form-username", "root"),
    ] {
        assert!(app.screen().matches(&input(id)).unwrap(), "{id}");
        // `text:` matches an empty input by its placeholder.
        assert!(
            app.screen().matches(&format!("text:{example}")).unwrap(),
            "{id}: {example}"
        );
    }
    // The password is the host's secure field, nothing kept: no badge.
    assert!(
        natives
            .find("moshpit/connection-password", "connection-form-password")
            .is_some()
    );
    assert!(
        !app.screen()
            .matches("id:connection-form-password-field-badge")
            .unwrap()
    );
    // Mosh's tuning and the proxy's fields only exist when they are on; a
    // new connection has nothing to delete.
    assert!(!app.screen().shows("Predict Mode"));
    assert!(
        !app.screen()
            .matches("id:connection-form-proxy-host")
            .unwrap()
    );
    assert!(!app.screen().matches("id:key-picker-row").unwrap());
    assert!(!app.screen().shows("Delete connection…"));
    channel.close();
}

#[test]
fn edits_reach_the_host_and_save_needs_a_name_a_host_and_a_user() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, draft());
    find(&mut app, "id:connection-form-name");

    // Save does nothing yet.
    app.press("id:save-connection").unwrap_or(());
    quiet(&heard);

    app.type_text(&input("connection-form-name"), "studio")
        .unwrap();
    let mut last = serde_json::Value::Null;
    while let Ok(json) = heard.recv_timeout(std::time::Duration::from_millis(300)) {
        last = serde_json::from_str(&json).unwrap();
    }
    assert_eq!(last, json!({"ev": "set", "key": "name", "value": "studio"}));
    app.type_text(&input("connection-form-host"), "192.0.2.22")
        .unwrap();
    // ...a user to sign in as (B517)...
    app.type_text(&input("connection-form-username"), "me")
        .unwrap();
    while heard
        .recv_timeout(std::time::Duration::from_millis(300))
        .is_ok()
    {}
    // ...and a password, in the host's field.
    let password = natives
        .find("moshpit/connection-password", "connection-form-password")
        .unwrap();
    emit(&mut app, password, "filled", json!(true));
    app.press("id:save-connection").unwrap();
    assert_eq!(next(&heard), json!({"ev": "save"}));

    // UIKit's switch turns mosh on: its tuning rows come.
    let mosh = natives.find("moshpit/switch", "toggle-use-mosh").unwrap();
    emit(&mut app, mosh, "change", json!(true));
    assert_eq!(
        next(&heard),
        json!({"ev": "set", "key": "mosh", "value": true})
    );
    find(&mut app, "text:Predict Mode");
    assert!(app.screen().shows("As in Settings"));
    assert!(
        app.screen()
            .matches(&input("connection-form-mosh-server"))
            .unwrap()
    );

    // The key: the segment, then the picker row and the PEM editor.
    let auth = natives
        .find("moshpit/segmented", "connection-form-auth")
        .unwrap();
    emit(&mut app, auth, "change", json!(1));
    assert_eq!(
        next(&heard),
        json!({"ev": "set", "key": "auth", "value": "key"})
    );
    find(&mut app, "id:key-picker-row");
    assert!(app.screen().shows("SSH key"));
    assert!(
        natives
            .find("moshpit/connection-pem", "private-key-editor")
            .is_some()
    );
    app.press("id:key-picker-row").unwrap();
    assert_eq!(next(&heard), json!({"ev": "pick-key"}));

    // The multiplexer's menu is the host's, checked on the one picked.
    let menu = natives.find("moshpit/menu", "multiplexer-picker").unwrap();
    let items = natives.prop(menu, "items").unwrap();
    let ids: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["none", "tmux", "herdr"]);
    assert_eq!(items[1]["checked"], true);
    assert_eq!(
        items[2]["detail"],
        "Built for coding agents — agent status needs no hooks"
    );
    emit(&mut app, menu, "select", json!("herdr"));
    assert_eq!(
        next(&heard),
        json!({"ev": "set", "key": "mux", "value": "herdr"})
    );
    find(&mut app, "id:connection-form-herdr-path");

    app.press("id:connection-form-cancel").unwrap();
    assert_eq!(next(&heard), json!({"ev": "cancel"}));
    channel.close();
}

#[test]
fn an_edit_starts_from_the_saved_connection_and_can_delete_it() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut state = draft();
    state.editing = true;
    state.name = "studio".into();
    state.host = "192.0.2.22".into();
    state.auth = Auth::Key;
    state.key = Some(KeyChoice {
        id: "k1".into(),
        label: "laptop · ED25519".into(),
    });
    state.mosh = true;
    state.predict_mode = ConnectionPredict::Never;
    state.socks = true;
    state.socks_host = "proxy.example.com".into();
    show(&mut app, &channel, state);
    find(&mut app, "id:key-picker-row");
    for text in [
        "Edit connection",
        "laptop · ED25519",
        "Never",
        "Delete connection…",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert_eq!(
        natives.count("moshpit/connection-pem"),
        0,
        "a managed key needs no PEM"
    );
    assert_eq!(
        app.screen()
            .input_text(&input("connection-form-proxy-host"))
            .unwrap(),
        Some("proxy.example.com")
    );
    drain(&heard);
    app.press("id:connection-form-delete").unwrap();
    assert_eq!(next(&heard), json!({"ev": "delete"}));
    channel.close();
}

#[test]
fn in_the_apps_language() {
    let (mut app, channel) = start(Screen::ConnectionForm, zh_hans(), "connection-form");
    let mut state = draft();
    state.mosh = true;
    show(&mut app, &channel, state);
    find(&mut app, "id:connection-form-name");
    for text in [
        "添加连接",
        "取消",
        "保存",
        "连接",
        "名称",
        "主机",
        "端口",
        "用户",
        "登录",
        "密码",
        "使用 mosh",
        "换网络不断线；需要 UDP。",
        "跟随设置",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}

/// Every message heard until the screen has been quiet for a moment.
fn drain(heard: &std::sync::mpsc::Receiver<String>) -> Vec<serde_json::Value> {
    let mut all = Vec::new();
    while let Ok(json) = heard.recv_timeout(std::time::Duration::from_millis(300)) {
        all.push(serde_json::from_str(&json).unwrap());
    }
    all
}

/// The multiplexer's path field is the picked multiplexer's own: tmux's
/// and herdr's are two fields of the draft, and switching shows the other
/// one's text, never the field just left (an NSH input reused across the
/// switch keeps its text, NSH-GAPS #12).
#[test]
fn switching_the_multiplexer_shows_its_own_path() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut state = draft();
    state.tmux_path = "/opt/tmux".into();
    state.herdr_path = "/opt/herdr".into();
    show(&mut app, &channel, state);
    find(&mut app, "id:connection-form-tmux-path");
    assert_eq!(
        app.screen()
            .input_text("id:connection-form-tmux-path-input")
            .unwrap(),
        Some("/opt/tmux")
    );

    let menu = natives.find("moshpit/menu", "multiplexer-picker").unwrap();
    emit(&mut app, menu, "select", json!("herdr"));
    find(&mut app, "id:connection-form-herdr-path");
    assert_eq!(
        app.screen()
            .input_text("id:connection-form-herdr-path-input")
            .unwrap(),
        Some("/opt/herdr"),
        "herdr's path field shows tmux's"
    );

    // An edit to herdr's path stays herdr's, and tmux's comes back as it was.
    app.type_text("id:connection-form-herdr-path-input", "/x")
        .unwrap();
    let menu = natives.find("moshpit/menu", "multiplexer-picker").unwrap();
    emit(&mut app, menu, "select", json!("tmux"));
    find(&mut app, "id:connection-form-tmux-path");
    assert_eq!(
        app.screen()
            .input_text("id:connection-form-tmux-path-input")
            .unwrap(),
        Some("/opt/tmux"),
        "tmux's path field shows herdr's"
    );
    let menu = natives.find("moshpit/menu", "multiplexer-picker").unwrap();
    emit(&mut app, menu, "select", json!("herdr"));
    find(&mut app, "id:connection-form-herdr-path");
    assert_eq!(
        app.screen()
            .input_text("id:connection-form-herdr-path-input")
            .unwrap(),
        Some("/opt/herdr/x")
    );
    let paths: Vec<_> = drain(&heard)
        .into_iter()
        .filter(|m| m["key"] == "tmuxPath" || m["key"] == "herdrPath")
        .collect();
    assert_eq!(
        paths.last(),
        Some(&json!({"ev": "set", "key": "herdrPath", "value": "/opt/herdr/x"}))
    );
    assert!(
        paths.iter().all(|m| m["key"] == "herdrPath"),
        "tmux's path was edited: {paths:?}"
    );
    channel.close();
}

/// Typing in the path fields reaches the host whole: the rows they sit in
/// are not made again on each edit of the draft. Nor is the host's
/// password field, whatever the draft and its own `filled` do meanwhile:
/// a field made again would drop what UIKit holds of the typing (its
/// cursor, the keyboard's state).
#[test]
fn typing_in_a_row_that_follows_the_draft_loses_nothing() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, draft());
    find(&mut app, "id:connection-form-tmux-path");
    let password = natives
        .find("moshpit/connection-password", "connection-form-password")
        .unwrap();
    for (id, key) in [
        ("connection-form-tmux-path", "tmuxPath"),
        ("connection-form-name", "name"),
    ] {
        app.type_text(&input(id), "hunter2-and-more").unwrap();
        let sets: Vec<_> = drain(&heard)
            .into_iter()
            .filter(|m| m["key"] == key)
            .collect();
        assert_eq!(
            sets.last(),
            Some(&json!({"ev": "set", "key": key, "value": "hunter2-and-more"})),
            "{id}: {sets:?}"
        );
        assert_eq!(
            sets.len(),
            "hunter2-and-more".len(),
            "{id}: an edit was lost"
        );
        assert_eq!(
            app.screen().input_text(&input(id)).unwrap(),
            Some("hunter2-and-more")
        );
    }
    for filled in [true, false, true] {
        emit(&mut app, password, "filled", json!(filled));
    }
    assert_eq!(
        natives.find("moshpit/connection-password", "connection-form-password"),
        Some(password),
        "the host's password field was made again"
    );
    assert_eq!(natives.count("moshpit/connection-password"), 1);
    channel.close();
}

/// The password and the PEM are the host's fields, one for each method:
/// switching the method swaps one for the other (the host's new field
/// starts from its draft, which the app's tests check).
#[test]
fn switching_the_method_swaps_the_hosts_fields() {
    let (mut app, channel, natives) = start_tall(Params::default());
    show(&mut app, &channel, draft());
    find(&mut app, "id:connection-form-name");
    assert!(
        natives
            .find("moshpit/connection-password", "connection-form-password")
            .is_some()
    );
    let auth = natives
        .find("moshpit/segmented", "connection-form-auth")
        .unwrap();
    emit(&mut app, auth, "change", json!(1));
    find(&mut app, "id:key-picker-row");
    assert!(
        natives
            .find("moshpit/connection-pem", "private-key-editor")
            .is_some()
    );
    assert_eq!(natives.count("moshpit/connection-password"), 0);
    let auth = natives
        .find("moshpit/segmented", "connection-form-auth")
        .unwrap();
    emit(&mut app, auth, "change", json!(0));
    assert!(
        natives
            .find("moshpit/connection-password", "connection-form-password")
            .is_some()
    );
    assert_eq!(natives.count("moshpit/connection-pem"), 0);
    channel.close();
}

/// The password and the pasted key never reach the screen (B294, NSH-GAPS
/// #11): they are not NSH inputs, the screen hands its host fields no text
/// (only how to look), and nothing it tells the host — whatever the user
/// does meanwhile — names either. What it learns is whether each holds
/// anything.
#[test]
fn the_password_and_the_key_never_cross_the_channel() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, draft());
    find(&mut app, "id:connection-form-name");
    assert!(
        !app.screen().matches("id:connection-form-password").unwrap(),
        "the password is an NSH input"
    );
    let password = natives
        .find("moshpit/connection-password", "connection-form-password")
        .expect("the host's password field");
    // The host's fields are told how to look, never what they hold.
    told_only_looks(
        &natives,
        password,
        &[
            "alignment",
            "fontSize",
            "identifier",
            "placeholder",
            "placeholderColor",
            "textColor",
            "tint",
        ],
    );
    app.type_text("id:connection-form-name-input", "box")
        .unwrap();
    emit(&mut app, password, "filled", json!(true));
    let auth = natives
        .find("moshpit/segmented", "connection-form-auth")
        .unwrap();
    emit(&mut app, auth, "change", json!(1));
    find(&mut app, "id:key-picker-row");
    assert!(
        !app.screen().matches("id:private-key-editor").unwrap(),
        "the private key is an NSH input"
    );
    let pem = natives
        .find("moshpit/connection-pem", "private-key-editor")
        .expect("the host's private key editor");
    told_only_looks(
        &natives,
        pem,
        &["fontSize", "identifier", "label", "textColor", "tint"],
    );
    emit(&mut app, pem, "filled", json!(true));
    app.type_text("id:connection-form-host-input", "h").unwrap();
    app.type_text("id:connection-form-username-input", "me")
        .unwrap();
    app.press("id:save-connection").unwrap();

    let told = drain(&heard);
    assert!(told.iter().any(|m| m == &json!({"ev": "save"})), "{told:?}");
    for message in &told {
        let key = message["key"].as_str().unwrap_or_default();
        assert!(
            !["password", "pem"].contains(&key),
            "the screen told a secret: {message}"
        );
        assert!(
            message.get("password").is_none() && message.get("pem").is_none(),
            "{message}"
        );
    }
    channel.close();
}

/// A port typed in, then the text size crossing into the accessibility
/// sizes (the row stacks its field under its label): the field shows the
/// port typed, which is the one saved, not the one the page opened with.
#[test]
fn the_port_survives_the_row_restacking() {
    let (mut app, channel) = start(Screen::ConnectionForm, Params::default(), "connection-form");
    show(&mut app, &channel, draft());
    find(&mut app, "id:connection-form-port");
    app.clear("id:connection-form-port-input").unwrap();
    app.type_text("id:connection-form-port-input", "2222")
        .unwrap();
    app.set_environment("AX3", |env| env.font_scale = 40.0 / 17.0)
        .unwrap();
    find(&mut app, "id:connection-form-port");
    assert_eq!(
        app.screen()
            .input_text("id:connection-form-port-input")
            .unwrap(),
        Some("2222"),
        "the restacked field went back to the opening port"
    );
    app.set_environment("Large", |env| env.font_scale = 1.0)
        .unwrap();
    find(&mut app, "id:connection-form-port");
    assert_eq!(
        app.screen()
            .input_text("id:connection-form-port-input")
            .unwrap(),
        Some("2222")
    );
    channel.close();
}

/// Whether Save tells the host now: pressed, it says `save` or nothing.
fn saves(app: &mut App, heard: &std::sync::mpsc::Receiver<String>) -> bool {
    drain(heard);
    app.press("id:save-connection").unwrap_or(());
    drain(heard).iter().any(|m| m == &json!({"ev": "save"}))
}

/// Clears field `id`, types `text` into it (nothing: left empty) and
/// leaves it with Return.
fn retype(app: &mut App, id: &str, text: &str) {
    app.clear(&input(id)).unwrap();
    app.type_text(&input(id), &format!("{text}\n")).unwrap();
}

/// Whether field `id` says what is wrong with it.
fn says_wrong(app: &App, id: &str) -> bool {
    app.screen().matches(&format!("id:{id}-error")).unwrap()
}

/// A draft Save takes as it is: a name, a host, a user, a password.
fn savable() -> FormState {
    let mut state = draft();
    state.name = "box".into();
    state.host = "box.example.com".into();
    state.username = "me".into();
    state.secrets.password = true;
    state
}

/// B294: a port is a whole number from 1 to 65535 (empty: the default).
/// "abc" used to save as 22 and "70000" as it was; now Save waits, and the
/// field says what is wrong under it once it is left — not while it is
/// typed in (§6.5). The proxy's port the same, while the proxy is on.
#[test]
fn a_port_that_is_not_one_says_so_once_left_and_save_waits() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, savable());
    find(&mut app, "id:connection-form-port");
    assert!(saves(&mut app, &heard));
    assert!(!says_wrong(&app, "connection-form-port"));

    // Typed, not yet left: Save waits, nothing is said yet.
    app.clear(&input("connection-form-port")).unwrap();
    app.type_text(&input("connection-form-port"), "70000")
        .unwrap();
    assert!(!saves(&mut app, &heard), "70000 saved");
    assert!(
        !says_wrong(&app, "connection-form-port"),
        "said while typing"
    );
    for bad in [
        "abc", "70000", "0", "65536", "-22", "+22", "2 2", "22.0", "２２",
    ] {
        retype(&mut app, "connection-form-port", bad);
        find(&mut app, "id:connection-form-port-error");
        assert!(
            app.screen().shows("Port must be between 1 and 65535."),
            "{bad}: {:?}",
            app.screen().texts()
        );
        assert!(!saves(&mut app, &heard), "{bad} saved");
    }
    for good in ["", "1", "22", "2222", "65535", "0022", " 2222 "] {
        retype(&mut app, "connection-form-port", good);
        assert!(
            !says_wrong(&app, "connection-form-port"),
            "{good:?} said not a port"
        );
        assert!(saves(&mut app, &heard), "{good:?} did not save");
    }

    // The proxy's port, once the proxy is on.
    let socks = natives
        .find("moshpit/switch", "connection-form-socks")
        .unwrap();
    emit(&mut app, socks, "change", json!(true));
    find(&mut app, "id:connection-form-proxy-port");
    retype(&mut app, "connection-form-proxy-host", "127.0.0.1");
    assert!(saves(&mut app, &heard), "1080 is a port");
    retype(&mut app, "connection-form-proxy-port", "99999");
    find(&mut app, "id:connection-form-proxy-port-error");
    assert!(!saves(&mut app, &heard), "99999 saved");
    retype(&mut app, "connection-form-proxy-port", "");
    assert!(!says_wrong(&app, "connection-form-proxy-port"));
    assert!(saves(&mut app, &heard), "empty is SOCKS5's 1080");
    retype(&mut app, "connection-form-proxy-port", "x");
    assert!(!saves(&mut app, &heard));
    // The proxy off, its port is not asked for.
    let socks = natives
        .find("moshpit/switch", "connection-form-socks")
        .unwrap();
    emit(&mut app, socks, "change", json!(false));
    assert!(saves(&mut app, &heard), "a hidden proxy port held Save");
    channel.close();
}

/// B551: a saved connection an old app saved without a name or a user
/// (B517: it used to save, and every sign-in failed), or with an emoji the
/// form refuses, opened to edit: Save waited, and nothing said why until
/// each field was entered and left; the empty fields showed their grey
/// placeholders ("my-server", "root") as if filled. Nothing is being typed:
/// they say so at once. A new connection's empty fields stay quiet.
#[test]
fn a_saved_connection_missing_what_save_needs_says_so_at_once() {
    let (mut app, channel, _) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut state = savable();
    state.editing = true;
    state.name = String::new();
    state.username = String::new();
    state.emoji = "ab".into();
    show(&mut app, &channel, state);
    find(&mut app, "id:connection-form-name");
    for id in [
        "connection-form-name",
        "connection-form-username",
        "connection-form-emoji",
    ] {
        assert!(says_wrong(&app, id), "{id}");
    }
    assert!(!saves(&mut app, &heard));
    channel.close();

    let (mut app, channel, _) = start_tall(Params::default());
    show(&mut app, &channel, draft());
    find(&mut app, "id:connection-form-name");
    for id in ["connection-form-name", "connection-form-username"] {
        assert!(!says_wrong(&app, id), "a new connection's {id}");
    }
    channel.close();
}

/// A port that is not one, opened from a saved connection (an old app
/// saved "70000" as it was): nothing is being typed, so the field says so
/// at once, in the app's language.
#[test]
fn a_saved_port_that_is_not_one_says_so_at_once_in_the_apps_language() {
    let (mut app, channel) = start(Screen::ConnectionForm, zh_hans(), "connection-form");
    let mut state = savable();
    state.port = "70000".into();
    show(&mut app, &channel, state);
    find(&mut app, "id:connection-form-port-error");
    assert!(
        app.screen().shows("端口要在 1 到 65535 之间。"),
        "{:?}",
        app.screen().texts()
    );
    channel.close();
}

/// A name or a host left empty says it is needed, once left.
#[test]
fn a_needed_field_left_empty_says_so() {
    let (mut app, channel) = start(Screen::ConnectionForm, Params::default(), "connection-form");
    show(&mut app, &channel, draft());
    find(&mut app, "id:connection-form-name");
    assert!(!says_wrong(&app, "connection-form-name"));
    app.type_text(&input("connection-form-name"), "\n").unwrap();
    find(&mut app, "id:connection-form-name-error");
    assert!(app.screen().shows("Required"));
    assert!(!says_wrong(&app, "connection-form-host"), "not left yet");
    retype(&mut app, "connection-form-name", "box");
    assert!(!says_wrong(&app, "connection-form-name"));
    channel.close();
}

/// B294: a key connection with no key picked and none pasted used to save,
/// and could never sign in. Save waits for a key — picked, pasted, or the
/// one the connection edited keeps already.
#[test]
fn key_auth_saves_only_with_a_key() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut state = savable();
    state.auth = Auth::Key;
    show(&mut app, &channel, state.clone());
    find(&mut app, "id:key-picker-row");
    assert!(!saves(&mut app, &heard), "saved with no key");

    // Pasted into the host's editor: Save takes it; cleared, it waits again.
    let pem = natives
        .find("moshpit/connection-pem", "private-key-editor")
        .unwrap();
    emit(&mut app, pem, "filled", json!(true));
    assert!(saves(&mut app, &heard));
    emit(&mut app, pem, "filled", json!(false));
    assert!(!saves(&mut app, &heard));

    // Picked.
    let mut picked = state.clone();
    picked.key = Some(KeyChoice {
        id: "k1".into(),
        label: "laptop · ED25519".into(),
    });
    show(&mut app, &channel, picked);
    assert!(saves(&mut app, &heard));

    // Edited, keeping the key it has.
    let mut kept = state.clone();
    kept.editing = true;
    kept.secrets.stored_key = true;
    show(&mut app, &channel, kept);
    assert!(saves(&mut app, &heard));

    // A password kept from before is no key.
    let mut other = state;
    other.editing = true;
    other.secrets = Secrets {
        password: true,
        stored_password: true,
        ..Secrets::default()
    };
    show(&mut app, &channel, other);
    assert!(!saves(&mut app, &heard), "a password counted as a key");
    channel.close();
}

/// A password connection saves with no password (a host that lets the
/// account in on who it is, Tailscale SSH, asks for none; one that wants
/// one has it asked when it connects), with one typed, or with the one it
/// keeps. B294's rule stays for a key connection, and for one whose own
/// item holds a key: switched to a password, it needs one.
#[test]
fn password_auth_saves_with_or_without_a_password() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut state = savable();
    state.secrets.password = false;
    show(&mut app, &channel, state.clone());
    find(&mut app, "id:connection-form-name");
    assert!(saves(&mut app, &heard), "saved with no password");
    let password = natives
        .find("moshpit/connection-password", "connection-form-password")
        .unwrap();
    emit(&mut app, password, "filled", json!(true));
    assert!(saves(&mut app, &heard));
    emit(&mut app, password, "filled", json!(false));
    assert!(saves(&mut app, &heard));

    let mut kept = state.clone();
    kept.editing = true;
    kept.secrets.stored_password = true;
    show(&mut app, &channel, kept);
    assert!(saves(&mut app, &heard), "the password it keeps");

    let mut key = state;
    key.editing = true;
    key.secrets.stored_key = true;
    show(&mut app, &channel, key);
    assert!(!saves(&mut app, &heard), "a key kept is no password");
    channel.close();
}

/// Host field `node` was given only props among `allowed` (how it looks).
fn told_only_looks(natives: &common::Natives, node: nsh_core::NodeId, allowed: &[&str]) {
    let props = natives.props(node);
    assert!(!props.is_empty());
    assert!(
        props.iter().all(|p| allowed.contains(&p.as_str())),
        "a host field was given {props:?}"
    );
}

/// The owner's iPhone (B316): Cancel, the title and Save ran into the
/// AUTHENTICATION header scrolled under a bar with no background. iOS 26's
/// scroll edge effect comes in under the bar and goes at the list's top;
/// below iOS 26 the classic background does.
#[test]
fn the_bar_sets_itself_off_from_the_groups_scrolled_under_it() {
    for (version, glass) in [((26, 0), true), ((18, 2), false)] {
        // An iPhone's screen, with mosh's and the proxy's rows open: the
        // form is taller than it.
        let (mut app, channel, _) = start_observing_with(
            Screen::ConnectionForm,
            Params::default(),
            "connection-form",
            move |config| {
                config.environment.os_version = version;
                config.viewport = Size::new(Points(402.0), Points(874.0));
            },
        );
        show(&mut app, &channel, filled());
        find(&mut app, "id:connection-form-name");
        assert_bar_follows_scrolling(&mut app, "connection-form-list", glass);
        channel.close();
    }
}

/// The bounds of the view with test id `id`.
fn bounds(app: &mut App, id: &str) -> nsh_core::Bounds<nsh_core::Points> {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no view {id}"))
        .bounds
}

/// Every field of the form, by test id, and the label people see on it.
const LABELLED: [(&str, &str); 9] = [
    ("connection-form-name", "Name"),
    ("connection-form-host", "Host"),
    ("connection-form-port", "Port"),
    ("connection-form-username", "User"),
    ("connection-form-password-field", "Password"),
    ("connection-form-mosh-server", "mosh-server path"),
    ("connection-form-proxy-host", "Proxy Host"),
    ("connection-form-proxy-port", "Proxy Port"),
    ("connection-form-tmux-path", "tmux path"),
];

/// A saved connection with every field filled and a password kept, as the
/// owner's iPhone showed it (B319): its name, its address, its user and no
/// word of what each was.
fn filled() -> FormState {
    let mut state = draft();
    state.editing = true;
    state.name = "mac".into();
    state.host = "192.0.2.22".into();
    state.username = "cluas".into();
    state.mosh = true;
    state.mosh_server_path = "/opt/homebrew/bin/mosh-server".into();
    state.socks = true;
    state.socks_host = "127.0.0.1".into();
    state.tmux_path = "/opt/homebrew/bin/tmux".into();
    state.secrets.stored_password = true;
    state
}

/// R9 (pain point 7, B319): every input has a label people can see, on
/// its field, filled or not; inline fields put it before the control,
/// paths over it, and from the first accessibility size every label
/// stands over its control.
#[test]
fn every_input_has_a_visible_label() {
    let (mut app, channel, _) = start_tall(Params::default());
    show(&mut app, &channel, filled());
    find(&mut app, "id:connection-form-tmux-path");
    // Paths stand stacked; from AX1 on, every field does.
    let check = |app: &mut App, accessibility: bool| {
        for (id, label) in LABELLED {
            let label_id = format!("{id}-label");
            find(app, &format!("id:{label_id}"));
            assert!(
                app.screen().shows(label),
                "{label}: {:?}",
                app.screen().texts()
            );
            let (said, row) = (bounds(app, &label_id), bounds(app, id));
            assert!(
                said.size.width.0 > 0.0 && said.size.height.0 > 0.0,
                "{label} is not drawn"
            );
            assert!(
                said.origin.y.0 >= row.origin.y.0
                    && said.origin.y.0 + said.size.height.0
                        <= row.origin.y.0 + row.size.height.0 + 0.5,
                "{label} is not on its field: {said:?} {row:?}"
            );
            let control = bounds(app, &format!("{id}-input"));
            let stacked = accessibility
                || matches!(
                    id,
                    "connection-form-mosh-server" | "connection-form-tmux-path"
                );
            if stacked {
                assert!(
                    said.origin.y.0 + said.size.height.0 <= control.origin.y.0 + 0.5,
                    "{label} is not over its control: {said:?} {control:?}"
                );
            } else {
                assert!(
                    said.origin.x.0 + said.size.width.0 <= control.origin.x.0 + 0.5,
                    "{label} is not before its control: {said:?} {control:?}"
                );
            }
        }
    };
    check(&mut app, false);
    for (id, value) in [
        ("connection-form-name", "mac"),
        ("connection-form-host", "192.0.2.22"),
        ("connection-form-username", "cluas"),
    ] {
        assert_eq!(app.screen().input_text(&input(id)).unwrap(), Some(value));
    }
    app.set_environment("AX3", |env| env.font_scale = 40.0 / 17.0)
        .unwrap();
    find(&mut app, "id:connection-form-name-label");
    check(&mut app, true);
    assert_eq!(
        app.screen()
            .input_text(&input("connection-form-name"))
            .unwrap(),
        Some("mac"),
        "the restacked field went back to the opening name"
    );
    channel.close();
}

/// R9: a password the connection keeps shows as "Saved", the field saying
/// "Type to replace"; typed over, "Replaces saved". Nothing kept: no badge.
#[test]
fn a_kept_password_shows_saved_and_typing_replaces_it() {
    let (mut app, channel, natives) = start_tall(Params::default());
    show(&mut app, &channel, filled());
    find(&mut app, "id:connection-form-password-field-badge");
    assert!(app.screen().shows("Saved"), "{:?}", app.screen().texts());
    assert!(
        app.screen()
            .matches("label:Password, saved, type to replace")
            .unwrap()
    );
    let password = natives
        .find("moshpit/connection-password", "connection-form-password")
        .unwrap();
    assert_eq!(
        natives.prop(password, "placeholder"),
        Some(json!("Type to replace"))
    );
    emit(&mut app, password, "filled", json!(true));
    find(&mut app, "text:Replaces saved");
    assert!(
        app.screen()
            .matches("label:Password, replaces saved")
            .unwrap()
    );
    emit(&mut app, password, "filled", json!(false));
    find(&mut app, "text:Saved");

    let mut new = filled();
    new.secrets.stored_password = false;
    show(&mut app, &channel, new);
    find(&mut app, "id:connection-form-password-field");
    assert!(
        !app.screen()
            .matches("id:connection-form-password-field-badge")
            .unwrap(),
        "nothing kept, nothing saved"
    );
    channel.close();
}

/// Whether field `id` shows its focus ring.
fn ringed(app: &App, id: &str) -> bool {
    app.screen().matches(&format!("id:{id}-ring")).unwrap()
}

/// R9: Return moves from Name to Host, Port and User; User's puts the
/// keyboard away. The proxy's host moves on to its port.
#[test]
fn return_moves_through_the_fields() {
    let (mut app, channel, natives) = start_tall(Params::default());
    show(&mut app, &channel, draft());
    find(&mut app, "id:connection-form-name");
    let chain = [
        "connection-form-name",
        "connection-form-host",
        "connection-form-port",
        "connection-form-username",
    ];
    app.type_text(&input(chain[0]), "box\n").unwrap();
    for pair in chain.windows(2) {
        assert!(
            !ringed(&app, pair[0]) && ringed(&app, pair[1]),
            "Return in {} moved to {}",
            pair[0],
            pair[1]
        );
        if pair[1] != chain[3] {
            app.type_text(&input(pair[1]), "\n").unwrap();
        }
    }
    app.type_text(&input(chain[3]), "\n").unwrap();
    assert!(
        chain.iter().all(|id| !ringed(&app, id)),
        "User's Return puts the keyboard away"
    );

    let socks = natives
        .find("moshpit/switch", "connection-form-socks")
        .unwrap();
    emit(&mut app, socks, "change", json!(true));
    find(&mut app, "id:connection-form-proxy-host");
    app.type_text(&input("connection-form-proxy-host"), "\n")
        .unwrap();
    assert!(ringed(&app, "connection-form-proxy-port"));
    channel.close();
}

/// R9 acceptance: port 70000 says what a port must be under the field, and
/// Save is disabled.
#[test]
fn port_70000_says_so_under_the_field_and_save_is_disabled() {
    let (mut app, channel) = start(Screen::ConnectionForm, Params::default(), "connection-form");
    show(&mut app, &channel, savable());
    find(&mut app, "id:connection-form-port");
    let save = |app: &App| {
        app.screen()
            .accessibility_state("id:save-connection")
            .unwrap()
            .disabled
    };
    assert!(!save(&app), "Save is enabled");
    retype(&mut app, "connection-form-port", "70000");
    find(&mut app, "id:connection-form-port-error");
    assert!(app.screen().shows("Port must be between 1 and 65535."));
    let (port, error) = (
        bounds(&mut app, "connection-form-port-input"),
        bounds(&mut app, "connection-form-port-error"),
    );
    assert!(
        error.origin.y.0 >= port.origin.y.0 + port.size.height.0 - 0.5,
        "the error stands under the field"
    );
    assert!(
        app.screen()
            .matches("label:Port, Port must be between 1 and 65535.")
            .unwrap(),
        "screen readers hear it with the field's name"
    );
    assert!(save(&app), "Save is disabled");
    channel.close();
}

/// The form draws in the light appearance when the host asks (the app asks
/// for dark until R18), and in the dark one.
#[test]
fn it_draws_light_and_dark() {
    for light in [false, true] {
        let (mut app, channel, _) = start_tall(Params {
            light: Some(light),
            ..Params::default()
        });
        show(&mut app, &channel, filled());
        find(&mut app, "id:connection-form-name");
        assert!(app.screen().shows("Edit connection"), "light {light}");
        assert!(app.screen().shows("Saved"), "light {light}");
        channel.close();
    }
}

/// r42: a name or host of white space alone saved (a connection named
/// "  " to a host "  "). Save waits until each holds more than that.
#[test]
fn a_name_or_host_of_white_space_alone_is_none() {
    let (mut app, channel) = start(Screen::ConnectionForm, Params::default(), "connection-form");
    let heard = hear(&channel);
    let mut state = savable();
    state.name = "   ".into();
    show(&mut app, &channel, state);
    find(&mut app, "id:connection-form-name");
    assert!(!saves(&mut app, &heard), "saved a name of spaces");
    let mut state = savable();
    state.host = " \t".into();
    show(&mut app, &channel, state);
    find(&mut app, "id:connection-form-host");
    assert!(!saves(&mut app, &heard), "saved a host of white space");
    let mut state = savable();
    state.host = "  box.example.com ".into();
    show(&mut app, &channel, state);
    find(&mut app, "id:connection-form-host");
    assert!(
        saves(&mut app, &heard),
        "a host with spaces round it is a host"
    );
    channel.close();
}

/// §6.5: a Save the keychain failed shows under the bar in a destructive
/// banner — what failed, Try again (saving again may work) and a close
/// button — and the form keeps what it holds. Try again saves; an edit or
/// the close button ends the banner.
#[test]
fn a_failed_save_says_why_under_the_bar_and_can_be_tried_again() {
    let (mut app, channel, _) = start_tall(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, savable());
    find(&mut app, "id:connection-form-name");
    let failed = |secret: Option<Auth>, retry: bool| {
        Message::Failed(SaveFailure {
            secret,
            reason: "Keychain interaction not allowed".into(),
            retry,
        })
        .to_json()
    };
    post(&mut app, &channel, &failed(Some(Auth::Password), true));
    find(&mut app, "id:connection-form-failed");
    assert!(app.screen().shows("Couldn't save the password."));
    let (bar, notice) = (
        bounds(&mut app, "save-connection"),
        bounds(&mut app, "connection-form-failed"),
    );
    assert!(
        notice.origin.y.0 >= bar.origin.y.0 + bar.size.height.0,
        "under the bar: {notice:?} {bar:?}"
    );
    assert_eq!(
        app.screen()
            .input_text(&input("connection-form-name"))
            .unwrap(),
        Some("box"),
        "the form keeps what it holds"
    );
    drain(&heard);
    app.press("id:connection-form-retry").unwrap();
    assert_eq!(next(&heard), json!({"ev": "save"}));
    assert!(
        !app.screen().matches("id:connection-form-failed").unwrap(),
        "saving again ends it"
    );

    // An edit ends it: the draft is not the one that failed.
    post(&mut app, &channel, &failed(Some(Auth::Key), true));
    find(&mut app, "text:Couldn't save the key.");
    app.type_text(&input("connection-form-name"), "2").unwrap();
    assert!(!app.screen().matches("id:connection-form-failed").unwrap());

    // The close button ends it too.
    post(&mut app, &channel, &failed(Some(Auth::Password), true));
    find(&mut app, "id:connection-form-failed");
    app.press("label:Close").unwrap();
    assert!(!app.screen().matches("id:connection-form-failed").unwrap());
    channel.close();
}

/// A save refused for a reason saving again will not change says the
/// host's reason, and offers no Try again.
#[test]
fn a_refused_save_says_the_hosts_reason_and_offers_no_retry() {
    let (mut app, channel) = start(Screen::ConnectionForm, Params::default(), "connection-form");
    show(&mut app, &channel, savable());
    find(&mut app, "id:connection-form-name");
    let reason = "To change how this connection signs in, enter the new password or key.";
    post(
        &mut app,
        &channel,
        &Message::Failed(SaveFailure {
            secret: None,
            reason: reason.into(),
            retry: false,
        })
        .to_json(),
    );
    find(&mut app, "id:connection-form-failed");
    assert!(app.screen().shows(reason), "{:?}", app.screen().texts());
    assert!(!app.screen().matches("id:connection-form-retry").unwrap());
    assert!(!app.screen().shows("Try again"));
    channel.close();
}

/// §4.13: what a host's emoji field takes. One emoji (a ZWJ sequence, a
/// skin tone, a flag, a keycap, a character asked to show as an emoji),
/// the spaces round it dropped; empty: none, the key's face. Not words,
/// not spaces alone, not two emoji.
#[test]
fn a_hosts_emoji_is_one_emoji_or_none() {
    use moshpit_ui::connection_form::emoji;
    for (typed, saved) in [
        ("🍓", "🍓"),
        ("🐳", "🐳"),
        ("👩\u{200D}💻", "👩\u{200D}💻"),
        (" 🐳 ", "🐳"),
        ("👍🏽", "👍🏽"),
        ("🇯🇵", "🇯🇵"),
        ("❤\u{FE0F}", "❤\u{FE0F}"),
        ("1\u{FE0F}\u{20E3}", "1\u{FE0F}\u{20E3}"),
    ] {
        assert_eq!(emoji(typed), Ok(Some(saved.to_owned())), "{typed:?}");
    }
    assert_eq!(emoji(""), Ok(None), "empty: the key's face");
    for wrong in ["ab", " ", "a", "1", "🍓🐳", "🍓 🐳", "❤", "#", "→"] {
        assert!(emoji(wrong).is_err(), "{wrong:?} taken");
    }
}

/// The emoji field: under the name, what it is for under it; text that
/// isn't one emoji says so once the field is left, and Save waits; one
/// emoji, or clearing it, saves.
#[test]
fn an_emoji_that_isnt_one_says_so_and_save_waits() {
    let (mut app, channel, _) = start_tall(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, savable());
    find(&mut app, "id:connection-form-emoji");
    assert!(app.screen().shows("Emoji"));
    assert!(
        app.screen()
            .shows("Shows on the host's mark in place of its key's pattern.")
    );
    assert!(saves(&mut app, &heard), "an empty emoji saves");
    for bad in ["ab", " ", "🍓🐳"] {
        retype(&mut app, "connection-form-emoji", bad);
        find(&mut app, "id:connection-form-emoji-error");
        assert!(
            app.screen().shows("One emoji, or leave it empty."),
            "{bad:?}: {:?}",
            app.screen().texts()
        );
        assert!(!saves(&mut app, &heard), "{bad:?} saved");
    }
    retype(&mut app, "connection-form-emoji", "🐳");
    assert!(!says_wrong(&app, "connection-form-emoji"));
    drain(&heard);
    app.clear(&input("connection-form-emoji")).unwrap();
    app.type_text(&input("connection-form-emoji"), "🐳")
        .unwrap();
    assert!(
        drain(&heard)
            .iter()
            .any(|m| m == &json!({"ev": "set", "key": "emoji", "value": "🐳"})),
        "the host hears the emoji"
    );
    assert!(saves(&mut app, &heard), "one emoji saves");
    retype(&mut app, "connection-form-emoji", "");
    assert!(saves(&mut app, &heard), "cleared, back to the key's face");
    channel.close();
}

/// B511: with the proxy on, its host is required. Saved empty (or spaces),
/// every connect went straight to the host, past the proxy the person had
/// turned on. Save waits, and once the field is left it says Required.
#[test]
fn a_proxy_without_a_host_doesnt_save() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, savable());
    find(&mut app, "id:connection-form-port");
    let socks = natives
        .find("moshpit/switch", "connection-form-socks")
        .unwrap();
    emit(&mut app, socks, "change", json!(true));
    find(&mut app, "id:connection-form-proxy-host");
    assert!(!saves(&mut app, &heard), "a proxy with no host saved");
    for blank in ["", "   "] {
        retype(&mut app, "connection-form-proxy-host", blank);
        find(&mut app, "id:connection-form-proxy-host-error");
        assert!(!saves(&mut app, &heard), "{blank:?} saved as the proxy");
    }
    retype(&mut app, "connection-form-proxy-host", "127.0.0.1");
    assert!(!says_wrong(&app, "connection-form-proxy-host"));
    assert!(saves(&mut app, &heard), "a proxy with a host");
    // The proxy off, its host isn't asked for.
    retype(&mut app, "connection-form-proxy-host", "");
    let socks = natives
        .find("moshpit/switch", "connection-form-socks")
        .unwrap();
    emit(&mut app, socks, "change", json!(false));
    assert!(saves(&mut app, &heard), "an unused proxy host held Save");
    channel.close();
}

/// B517: the host field takes a host's name or address (IPv6 written
/// bare), not ssh's own syntax pasted whole — saved, `dev@box`, `box:2222`
/// or `ssh://box` never resolved, with nothing to say why. And a
/// connection signs in as someone: an empty user used to save.
#[test]
fn a_host_is_just_a_host_and_a_user_is_needed() {
    let (mut app, channel, _) = start_tall(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, savable());
    find(&mut app, "id:connection-form-host");
    assert!(saves(&mut app, &heard));
    for pasted in [
        "dev@box",
        "ssh://box",
        "box:2222",
        "[2001:db8::7]",
        "my box",
    ] {
        retype(&mut app, "connection-form-host", pasted);
        find(&mut app, "id:connection-form-host-error");
        assert!(
            app.screen()
                .shows("Just the host — the user and port have their own fields."),
            "{pasted}: {:?}",
            app.screen().texts()
        );
        assert!(!saves(&mut app, &heard), "{pasted} saved");
    }
    for host in [
        "box.example.com",
        " box ",
        "192.0.2.7",
        "2001:db8::7",
        "fe80::1%en0",
    ] {
        retype(&mut app, "connection-form-host", host);
        assert!(!says_wrong(&app, "connection-form-host"), "{host:?}");
        assert!(saves(&mut app, &heard), "{host:?} did not save");
    }

    retype(&mut app, "connection-form-username", "   ");
    find(&mut app, "id:connection-form-username-error");
    assert!(!saves(&mut app, &heard), "saved with no user");
    retype(&mut app, "connection-form-username", "dev");
    assert!(!says_wrong(&app, "connection-form-username"));
    assert!(saves(&mut app, &heard));
    channel.close();
}

/// B530: "Roam on Cellular" was saved per connection and read by nothing
/// (mosh roams whatever the network; nothing dials over cellular on its
/// own). Its switch is gone from the mosh rows.
#[test]
fn the_mosh_rows_have_no_roam_on_cellular() {
    let (mut app, channel, _) = start_tall(Params::default());
    let mut mosh = savable();
    mosh.mosh = true;
    show(&mut app, &channel, mosh);
    find(&mut app, "text:Predict Mode");
    assert!(!app.screen().matches("id:connection-form-roam").unwrap());
    assert!(!app.screen().shows("Roam on Cellular"));
    channel.close();
}

/// B531: a connection's predictive echo is as Settings says (the first,
/// and a new connection's), or its own: Adaptive, Always, Never or
/// Experimental. "Adaptive" used to be the only way to say "as Settings
/// says", so a connection could never pin Adaptive itself.
#[test]
fn predict_mode_is_settings_or_its_own() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut mosh = savable();
    mosh.mosh = true;
    show(&mut app, &channel, mosh);
    find(&mut app, "text:Predict Mode");
    let menu = natives
        .find("moshpit/menu", "connection-form-predict")
        .unwrap();
    assert_eq!(
        natives.prop(menu, "items"),
        Some(json!([
            {"id": "settings", "title": "As in Settings", "checked": true},
            {"id": "adaptive", "title": "Adaptive"},
            {"id": "always", "title": "Always"},
            {"id": "never", "title": "Never"},
            {"id": "experimental", "title": "Experimental"},
        ]))
    );
    drain(&heard);
    emit(&mut app, menu, "select", json!("adaptive"));
    assert_eq!(
        drain(&heard),
        [json!({"ev": "set", "key": "predictMode", "value": "adaptive"})]
    );
    channel.close();
}

/// B532: a connection with the same user, host and port as a saved one, or
/// the same name, says which one under CONNECTION once its fields are
/// left. A warning only: Save still saves.
#[test]
fn a_duplicate_says_which_and_still_saves() {
    let (mut app, channel, _) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut state = savable();
    state.others = vec![Other {
        name: "gpu-01".into(),
        user: "me".into(),
        host: "BOX.example.com".into(),
        port: 22,
    }];
    show(&mut app, &channel, state);
    find(&mut app, "id:connection-form-host");
    assert!(
        !app.screen()
            .matches("id:connection-form-duplicate")
            .unwrap(),
        "said before a field was left"
    );
    retype(&mut app, "connection-form-host", "box.example.com");
    find(&mut app, "id:connection-form-duplicate");
    assert!(
        app.screen().shows("Same user, host and port as “gpu-01”."),
        "{:?}",
        app.screen().texts()
    );
    assert!(saves(&mut app, &heard), "a duplicate held Save");

    retype(&mut app, "connection-form-port", "2222");
    retype(&mut app, "connection-form-name", "GPU-01");
    find(
        &mut app,
        "text:“gpu-01” is already another connection's name.",
    );
    assert!(saves(&mut app, &heard));

    retype(&mut app, "connection-form-name", "other");
    assert!(
        !app.screen()
            .shows("“gpu-01” is already another connection's name.")
    );
    assert!(
        !app.screen()
            .matches("id:connection-form-duplicate")
            .unwrap()
    );
    channel.close();
}

/// `filled()`, its secret not on this iPhone (B782): the host marks the
/// draft so and posts nothing kept.
fn lacking(secret: Auth) -> FormState {
    let mut state = filled();
    state.name = "nas".into();
    state.auth = secret;
    state.secrets.stored_password = false;
    state.missing = Some(Missing {
        name: "nas".into(),
        secret,
    });
    state
}

/// B782: a connection restored onto another iPhone has its record and not
/// its password. The form said "Saved" and saved the password that isn't
/// there. It says whose password is missing under the field, which takes
/// the keyboard, and Save waits for it.
#[test]
fn a_password_not_on_this_iphone_says_so_under_its_field_and_takes_the_keyboard() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, lacking(Auth::Password));
    find(&mut app, "id:connection-form-password-field-description");
    assert!(
        app.screen()
            .shows("The password for nas isn't on this iPhone. Enter it again."),
        "{:?}",
        app.screen().texts()
    );
    assert!(
        !app.screen()
            .matches("id:connection-form-password-field-badge")
            .unwrap(),
        "nothing kept, nothing saved"
    );
    let password = natives
        .find("moshpit/connection-password", "connection-form-password")
        .unwrap();
    assert_eq!(natives.prop(password, "autoFocus"), Some(json!(true)));
    assert!(!saves(&mut app, &heard), "Save waits for the password");
    emit(&mut app, password, "filled", json!(true));
    assert!(saves(&mut app, &heard));

    // Nothing missing: no note, and the keyboard waits for a tap.
    show(&mut app, &channel, filled());
    find(&mut app, "id:connection-form-password-field-badge");
    assert!(
        !app.screen()
            .matches("id:connection-form-password-field-description")
            .unwrap()
    );
    let password = natives
        .find("moshpit/connection-password", "connection-form-password")
        .unwrap();
    assert_eq!(natives.prop(password, "autoFocus"), None);
    channel.close();
}

#[test]
fn a_pasted_key_not_on_this_iphone_says_so_under_its_editor_and_takes_the_keyboard() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, lacking(Auth::Key));
    find(&mut app, "id:connection-form-pem-missing");
    assert!(
        app.screen()
            .shows("The private key for nas isn't on this iPhone. Paste it again."),
        "{:?}",
        app.screen().texts()
    );
    let pem = natives
        .find("moshpit/connection-pem", "private-key-editor")
        .unwrap();
    assert_eq!(natives.prop(pem, "autoFocus"), Some(json!(true)));
    assert!(!saves(&mut app, &heard), "Save waits for the key");
    emit(&mut app, pem, "filled", json!(true));
    assert!(saves(&mut app, &heard));

    // The password's note is not the key's: switched to Password, the
    // field neither says it nor takes the keyboard.
    let auth = natives
        .find("moshpit/segmented", "connection-form-auth")
        .unwrap();
    emit(&mut app, auth, "change", json!(0));
    find(&mut app, "id:connection-form-password-field");
    assert!(
        !app.screen()
            .matches("id:connection-form-password-field-description")
            .unwrap()
    );
    let password = natives
        .find("moshpit/connection-password", "connection-form-password")
        .unwrap();
    assert_eq!(natives.prop(password, "autoFocus"), None);
    channel.close();
}

#[test]
fn a_secret_not_on_this_iphone_says_so_in_the_apps_language() {
    let (mut app, channel, _) = start_observing_with(
        Screen::ConnectionForm,
        zh_hans(),
        "connection-form",
        |config| {
            config.viewport = Size::new(Points(580.0), Points(2600.0));
        },
    );
    show(&mut app, &channel, lacking(Auth::Password));
    find(&mut app, "id:connection-form-password-field-description");
    assert!(
        app.screen()
            .shows("这台 iPhone 上没有 nas 的密码，请重新输入。"),
        "{:?}",
        app.screen().texts()
    );
    show(&mut app, &channel, lacking(Auth::Key));
    find(&mut app, "id:connection-form-pem-missing");
    assert!(
        app.screen()
            .shows("这台 iPhone 上没有 nas 的私钥，请重新粘贴。"),
        "{:?}",
        app.screen().texts()
    );
    channel.close();
}

// MARK: - JUMP HOST (F1)

fn bastion() -> ViaChoice {
    ViaChoice {
        id: "B1".into(),
        name: "bastion".into(),
        detail: "dev@bastion.example.com".into(),
    }
}

/// With another saved connection to go through, JUMP HOST offers it under
/// Connect via, Direct first, its address under its name. Picking it tells
/// the host and takes PROXY away (the jump host's own proxy carries its
/// leg); mosh's switch stays as it was and usable; Direct again brings
/// PROXY back.
#[test]
fn f1_picking_a_jump_host_takes_the_proxy_away_and_leaves_mosh_alone() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut state = draft();
    state.mosh = true;
    state.via_choices = vec![bastion()];
    show(&mut app, &channel, state);
    // The select's identifier is its menu's (a native control).
    find(&mut app, "text:Connect via");
    for text in ["JUMP HOST", "Connect via", "Direct", "PROXY"] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    let menu = natives.find("moshpit/menu", "connection-form-via").unwrap();
    assert_eq!(
        natives.prop(menu, "items"),
        Some(json!([
            {"id": "", "title": "Direct", "checked": true},
            {"id": "B1", "title": "bastion", "detail": "dev@bastion.example.com"},
            {"id": "+new", "title": "New connection…"},
        ]))
    );
    assert!(
        natives
            .find("moshpit/menu", "connection-form-jump-route")
            .is_none(),
        "no jump host, no route row"
    );

    emit(&mut app, menu, "select", json!("B1"));
    assert_eq!(
        next(&heard),
        json!({"ev": "set", "key": "via", "value": "B1"})
    );
    find(&mut app, "text:When going through a jump host");
    assert!(app.screen().shows("bastion"));
    assert!(!app.screen().shows("PROXY"), "{:?}", app.screen().texts());
    assert!(!app.screen().shows("SOCKS5"));
    let mosh = natives.find("moshpit/switch", "toggle-use-mosh").unwrap();
    assert_eq!(natives.prop(mosh, "on"), Some(json!(true)));
    assert_eq!(natives.prop(mosh, "enabled"), Some(json!(true)));

    let menu = natives.find("moshpit/menu", "connection-form-via").unwrap();
    emit(&mut app, menu, "select", json!(""));
    assert_eq!(
        next(&heard),
        json!({"ev": "set", "key": "via", "value": ""})
    );
    find(&mut app, "text:PROXY");
    assert!(!app.screen().shows("When going through a jump host"));
    channel.close();
}

/// The route row is there only with mosh on and a jump host picked; it
/// lists the three routes (SSH the default) and tells the host the pick.
/// Only "forwarded" shows the rule to copy, with the target host and
/// Settings' range in it, and the note on what else the hosts need.
#[test]
fn the_jump_route_row_shows_with_mosh_and_a_jump_host_and_the_rule_only_when_forwarded() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut state = draft();
    state.mosh = true;
    state.host = "192.0.2.20".into();
    state.udp_ports = [60000, 61000];
    state.via = "B1".into();
    state.via_choices = vec![bastion()];
    show(&mut app, &channel, state.clone());
    find(&mut app, "text:When going through a jump host");
    let menu = natives
        .find("moshpit/menu", "connection-form-jump-route")
        .unwrap();
    assert_eq!(
        natives.prop(menu, "items"),
        Some(json!([
            {"id": "ssh", "title": "Over SSH", "checked": true},
            {"id": "direct", "title": "UDP straight to the host"},
            {"id": "viaJump", "title": "UDP forwarded by the jump host"},
        ]))
    );
    assert!(!app.screen().shows("Forwarding rule for the jump host"));

    emit(&mut app, menu, "select", json!("viaJump"));
    assert_eq!(
        next(&heard),
        json!({"ev": "set", "key": "jumpRoute", "value": "viaJump"})
    );
    find(&mut app, "text:Forwarding rule for the jump host");
    let rule = "nft add rule ip moshpit pre udp dport 60000-61000 dnat to 192.0.2.20";
    assert!(
        app.screen().texts().iter().any(|t| t.contains(rule)),
        "{:?}",
        app.screen().texts()
    );
    assert!(app.screen().shows("Also needed"));

    // Mosh off: the whole row goes.
    let mut off = state;
    off.mosh = false;
    show(&mut app, &channel, off);
    assert!(
        natives
            .find("moshpit/menu", "connection-form-jump-route")
            .is_none()
    );
    channel.close();
}

/// The route row and its note in Chinese, as the app says them.
#[test]
fn the_jump_route_in_chinese() {
    let (mut app, channel, natives) = start_tall(zh_hans());
    let mut state = draft();
    state.mosh = true;
    state.host = "192.0.2.20".into();
    state.via = "B1".into();
    state.jump_route = moshpit_ui::connection_form::JumpRoute::ViaJump;
    state.via_choices = vec![bastion()];
    show(&mut app, &channel, state);
    find(&mut app, "id:connection-form-forward-note");
    for text in [
        "经跳板机时",
        "跳板机上的转发规则",
        "另外还需要",
        "两台主机的安全组都要放行这段 UDP 端口，跳板机要开启 net.ipv4.ip_forward。",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    let menu = natives
        .find("moshpit/menu", "connection-form-jump-route")
        .unwrap();
    assert_eq!(
        natives.prop(menu, "items"),
        Some(json!([
            {"id": "ssh", "title": "走 SSH"},
            {"id": "direct", "title": "UDP 直连目标机"},
            {"id": "viaJump", "title": "UDP 经跳板机转发", "checked": true},
        ]))
    );
    channel.close();
}

/// The rule, filled with the target and the range; unset, mosh's own.
#[test]
fn the_forwarding_rule_names_the_target_and_the_range() {
    use moshpit_ui::connection_form::forward_rule;
    let rule = forward_rule("192.0.2.20", [60100, 60200]);
    assert!(
        rule.contains("pre udp dport 60100-60200 dnat to 192.0.2.20"),
        "{rule}"
    );
    assert!(
        rule.contains("post ip daddr 192.0.2.20 udp dport 60100-60200 masquerade"),
        "{rule}"
    );
    assert!(rule.starts_with("nft add table ip moshpit\n"));
    assert!(forward_rule("", [0, 0]).contains("udp dport 60000-61000 dnat to TARGET_IP"));
}

/// No other saved connection: JUMP HOST still offers Direct and, last, a
/// new one to make on the spot.
#[test]
fn f1_without_another_connection_the_picker_offers_a_new_one() {
    let (mut app, channel, natives) = start_tall(Params::default());
    show(&mut app, &channel, draft());
    find(&mut app, "text:JUMP HOST");
    let menu = natives.find("moshpit/menu", "connection-form-via").unwrap();
    assert_eq!(
        natives.prop(menu, "items"),
        Some(json!([
            {"id": "", "title": "Direct", "checked": true},
            {"id": "+new", "title": "New connection…"},
        ]))
    );
    channel.close();
}

/// "New connection…" asks the host for the second form and changes nothing
/// in this one: the picker still says Direct, and what was typed stays.
#[test]
fn choosing_new_connection_opens_the_nested_form_and_keeps_the_draft() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut state = draft();
    state.name = "hpc".into();
    state.via_choices = vec![bastion()];
    show(&mut app, &channel, state);
    find(&mut app, "text:Connect via");
    let menu = natives.find("moshpit/menu", "connection-form-via").unwrap();
    let items = natives.prop(menu, "items").unwrap();
    assert_eq!(
        items.as_array().unwrap().last(),
        Some(&json!({"id": "+new", "title": "New connection…"}))
    );
    emit(&mut app, menu, "select", json!("+new"));
    assert_eq!(next(&heard), json!({"ev": "new-jump"}));
    quiet(&heard);
    let menu = natives.find("moshpit/menu", "connection-form-via").unwrap();
    let items = natives.prop(menu, "items").unwrap();
    assert_eq!(
        items.as_array().unwrap().first(),
        Some(&json!({"id": "", "title": "Direct", "checked": true})),
        "cancelling the second form leaves the picker as it was"
    );
    channel.close();
}

/// The host answers a saved jump host by posting the draft going through
/// it: the picker shows it chosen, with Edit under it.
#[test]
fn a_saved_jump_host_comes_back_selected_with_edit_under_it() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut state = draft();
    state.name = "hpc".into();
    show(&mut app, &channel, state.clone());
    find(&mut app, "text:Connect via");
    state.via_choices = vec![bastion()];
    state.via = "B1".into();
    show(&mut app, &channel, state);
    find(&mut app, "text:Edit jump host…");
    let menu = natives.find("moshpit/menu", "connection-form-via").unwrap();
    let items = natives.prop(menu, "items").unwrap();
    assert_eq!(
        items.as_array().unwrap()[1],
        json!({"id": "B1", "title": "bastion", "detail": "dev@bastion.example.com", "checked": true})
    );
    find(&mut app, "id:connection-form-edit-jump");
    app.press("id:connection-form-edit-jump").unwrap();
    assert_eq!(next(&heard), json!({"ev": "edit-jump", "id": "B1"}));
    channel.close();
}

/// The nested form has no JUMP HOST section (one hop) and no Delete, and
/// shows "Jump host only", on.
#[test]
fn the_nested_form_has_no_jump_host_section() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let mut state = draft();
    state.nested = true;
    state.jump_only = true;
    state.via_choices = vec![bastion()];
    show(&mut app, &channel, state.clone());
    find(&mut app, "text:Jump host only");
    assert!(
        !app.screen().shows("JUMP HOST"),
        "{:?}",
        app.screen().texts()
    );
    assert!(!app.screen().shows("Connect via"));
    assert!(
        natives
            .find("moshpit/menu", "connection-form-via")
            .is_none()
    );
    let on = natives.find("moshpit/switch", "toggle-jump-only").unwrap();
    assert_eq!(natives.prop(on, "on"), Some(json!(true)));
    // Editing one: still no Delete here.
    state.editing = true;
    show(&mut app, &channel, state);
    find(&mut app, "text:Jump host only");
    assert!(!app.screen().shows("Delete connection…"));
    channel.close();
}

/// "Jump host only" is off in an ordinary form, says what it does, and
/// tells the host as it is flipped.
#[test]
fn jump_host_only_is_off_by_default_and_tells_the_host() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, draft());
    find(&mut app, "text:Jump host only");
    assert!(
        app.screen()
            .shows("Not listed on Home; pick it under Connect via.")
    );
    let toggle = natives.find("moshpit/switch", "toggle-jump-only").unwrap();
    assert_eq!(natives.prop(toggle, "on"), Some(json!(false)));
    emit(&mut app, toggle, "change", json!(true));
    assert_eq!(
        next(&heard),
        json!({"ev": "set", "key": "jumpOnly", "value": true})
    );
    let toggle = natives.find("moshpit/switch", "toggle-jump-only").unwrap();
    assert_eq!(natives.prop(toggle, "on"), Some(json!(true)));
    channel.close();
}

/// A connection others go through connects directly (one hop): its row is
/// off and names them, and there is no menu to open.
#[test]
fn f1_a_jump_host_connects_directly_and_says_why() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let mut state = draft();
    state.editing = true;
    state.via_users = vec!["hpc".into(), "gpu".into()];
    show(&mut app, &channel, state);
    find(&mut app, "id:connection-form-via");
    assert!(
        app.screen()
            .shows("This connection is the jump host for hpc, gpu, so it connects directly."),
        "{:?}",
        app.screen().texts()
    );
    assert!(
        natives
            .find("moshpit/menu", "connection-form-via")
            .is_none()
    );
    channel.close();
}

/// Saved through a connection since deleted: the row says so (not Direct,
/// which it isn't: it fails rather than dial direct) until another is
/// picked.
#[test]
fn f1_a_deleted_jump_host_shows_as_such() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let mut state = draft();
    state.editing = true;
    state.via = "GONE".into();
    state.via_choices = vec![bastion()];
    show(&mut app, &channel, state);
    find(&mut app, "text:Deleted connection");
    let menu = natives.find("moshpit/menu", "connection-form-via").unwrap();
    let items = natives.prop(menu, "items").unwrap();
    assert_eq!(
        items.as_array().unwrap().iter().rev().nth(1),
        Some(&json!({"id": "GONE", "title": "Deleted connection", "checked": true}))
    );
    channel.close();
}

/// In Chinese, as the app says it.
#[test]
fn f1_the_jump_host_in_chinese() {
    let (mut app, channel, natives) = start_tall(zh_hans());
    let mut state = draft();
    state.mosh = true;
    state.via_choices = vec![bastion()];
    show(&mut app, &channel, state);
    find(&mut app, "text:经由");
    for text in [
        "跳板主机",
        "经由",
        "直接连接",
        "先用那条连接自己的密钥或密码登录，再从那里连到这台主机。",
        "仅作跳板主机",
        "不在首页显示，在“经由”里选它。",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    let menu = natives.find("moshpit/menu", "connection-form-via").unwrap();
    emit(&mut app, menu, "select", json!("B1"));
    find(&mut app, "text:经跳板机时");
    channel.close();
}
