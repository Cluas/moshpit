//! Settings › Set up a host's list on a headless engine: every saved
//! connection, its name over where it reaches, connected or not; a pick
//! tells the host which one (`{"ev": "setup", "id": …}`). The host routes
//! the pick and the hub's row (`HostSetupRoute`, Swift): Host Setup for a
//! connected one, Connect first for one that isn't, straight in when there
//! is just one.

mod common;

use common::{App, hear, next, post, quiet, settings_fixture::state, start, zh_hans};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
    settings::Message,
};
use serde_json::json;

/// Two saved connections, both connected.
const TWO_LIVE: &str = r#"{"op":"hosts","hosts":[
    {"id":"A1","name":"gpu-01","user":"dev","host":"192.0.2.10","port":22,"live":true},
    {"id":"B2","name":"build-box","user":"ci","host":"192.0.2.20","port":2222,"live":true}]}"#;

/// The list, with the fixture's settings and `hosts` posted.
fn list(params: Params, hosts: &str) -> (App, std::sync::Arc<Channel>) {
    let (mut app, channel) = start(
        Screen::Settings,
        Params {
            page: Some("hosts".into()),
            ..params
        },
        "settings",
    );
    post(
        &mut app,
        &channel,
        &Message::Settings(Box::new(state())).to_json(),
    );
    post(&mut app, &channel, hosts);
    (app, channel)
}

fn shows_all(app: &App, texts: &[&str]) {
    for text in texts {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
}

/// Two hosts connected: both listed, each by its name over where it
/// reaches; the one tapped is the one the host is told.
#[test]
fn two_live_hosts_are_listed_and_the_one_tapped_is_told() {
    let (mut app, channel) = list(Params::default(), TWO_LIVE);
    let told = hear(&channel);
    app.wait_for("id:settings-hosts-group").unwrap();
    shows_all(
        &app,
        &[
            "Set up a host",
            "gpu-01",
            "dev@192.0.2.10:22",
            "build-box",
            "ci@192.0.2.20:2222",
            "Connected",
        ],
    );
    app.press("id:settings-host-B2").unwrap();
    assert_eq!(next(&told), json!({"ev": "setup", "id": "B2"}));
    app.press("id:settings-host-A1").unwrap();
    assert_eq!(next(&told), json!({"ev": "setup", "id": "A1"}));
    quiet(&told);
    channel.close();
}

/// A host not connected says so, and is still a pick (the host offers to
/// connect it); the list follows the host as connections come and go.
#[test]
fn a_host_not_connected_says_so_and_the_list_follows() {
    let (mut app, channel) = list(Params::default(), TWO_LIVE);
    let told = hear(&channel);
    app.wait_for("id:settings-host-A1").unwrap();
    assert!(!app.screen().shows("Not connected"));
    post(
        &mut app,
        &channel,
        r#"{"op":"hosts","hosts":[
            {"id":"A1","name":"gpu-01","user":"dev","host":"192.0.2.10","port":22,"live":false},
            {"id":"B2","name":"build-box","user":"ci","host":"192.0.2.20","port":2222,"live":true},
            {"id":"C3","name":"nas","user":"root","host":"2001:db8::7","port":22,"live":false}]}"#,
    );
    app.wait_for("id:settings-host-C3").unwrap();
    shows_all(
        &app,
        &["Not connected", "Connected", "root@[2001:db8::7]:22"],
    );
    app.press("id:settings-host-A1").unwrap();
    assert_eq!(next(&told), json!({"ev": "setup", "id": "A1"}));
    quiet(&told);
    channel.close();
}

/// No saved connection: the list says so, and why setup lives on a
/// connection.
#[test]
fn no_connections_says_so() {
    let (mut app, channel) = list(Params::default(), r#"{"op":"hosts","hosts":[]}"#);
    app.wait_for("id:settings-hosts-none").unwrap();
    shows_all(
        &app,
        &[
            "No connections yet",
            "Host setup is automatic. Connecting to a host installs and repairs everything it needs — scripts, hooks registration, push pairing — and asks before its first install. Connect to a host to inspect or remove its setup here.",
        ],
    );
    channel.close();
}

/// Each host is a whole touch target (44 pt).
#[test]
fn each_host_is_a_whole_touch_target() {
    let (mut app, channel) = list(Params::default(), TWO_LIVE);
    app.wait_for("id:settings-host-A1").unwrap();
    for id in ["settings-host-A1", "settings-host-B2"] {
        let bounds = app
            .screen()
            .layout_nodes()
            .into_iter()
            .find(|node| node.test_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("no {id}"))
            .bounds;
        assert!(bounds.size.height.0 >= 44.0, "{id}: {bounds:?}");
    }
    channel.close();
}

#[test]
fn the_list_speaks_the_apps_language() {
    let (mut app, channel) = list(
        zh_hans(),
        r#"{"op":"hosts","hosts":[
            {"id":"A1","name":"gpu-01","user":"dev","host":"192.0.2.10","port":22,"live":true},
            {"id":"B2","name":"build-box","user":"ci","host":"192.0.2.20","port":22,"live":false}]}"#,
    );
    app.wait_for("id:settings-hosts-group").unwrap();
    shows_all(&app, &["配置主机", "已连接", "未连接"]);
    channel.close();
}
