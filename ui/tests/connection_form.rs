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
    connection_form::{Auth, FormState, KeyChoice, Message, Mux, SaveFailure, Secrets},
    screen::{Params, Screen},
    settings::PredictMode,
};
use nsh_core::{Points, Size};
use serde_json::json;

fn draft() -> FormState {
    FormState {
        editing: false,
        port: "22".into(),
        udp_range: "60000 – 61000".into(),
        roam_on_cellular: true,
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
fn edits_reach_the_host_and_save_needs_a_name_and_a_host() {
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
    assert!(app.screen().shows("Adaptive"));
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
    state.predict_mode = PredictMode::Never;
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
        "自适应",
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

/// A draft Save takes as it is: a name, a host, a password.
fn savable() -> FormState {
    let mut state = draft();
    state.name = "box".into();
    state.host = "box.example.com".into();
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

/// B294: a password connection with no password used to save, and could
/// never sign in (the app asks for none when it connects). Save waits for
/// one — typed into the host's field, or the one the connection edited
/// keeps.
#[test]
fn password_auth_saves_only_with_a_password() {
    let (mut app, channel, natives) = start_tall(Params::default());
    let heard = hear(&channel);
    let mut state = savable();
    state.secrets.password = false;
    show(&mut app, &channel, state.clone());
    find(&mut app, "id:connection-form-name");
    assert!(!saves(&mut app, &heard), "saved with no password");
    let password = natives
        .find("moshpit/connection-password", "connection-form-password")
        .unwrap();
    emit(&mut app, password, "filled", json!(true));
    assert!(saves(&mut app, &heard));
    emit(&mut app, password, "filled", json!(false));
    assert!(!saves(&mut app, &heard));

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
    let mut state = draft();
    state.name = "box".into();
    state.host = "box.example.com".into();
    state.secrets.password = true;
    show(&mut app, &channel, state);
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
