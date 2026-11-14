//! A protocol switch (mosh ⇄ SSH) moves nothing (the owner: 「mosh 切换 ssh
//! 有个动画 我们直接去掉」): while the host says so (`still`), the veil is
//! there at once and gone at once, even when the post that lifts it no
//! longer says it is a switch, and a banner leaves at once. An ordinary
//! reconnect's veil and banner still fade. The host never posts a poster
//! for a switch (Swift: `TerminalScreen.coverKind`).

mod common;

use common::{flushed, post, start_observing};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
};
use nsh_testing::Headless;
use serde_json::{Value, json};

const HANG: std::time::Duration = std::time::Duration::from_secs(300);

fn chrome(changes: Value) -> Value {
    let mut base = json!({
        "op": "chrome",
        "name": "studio",
        "user": "cluas",
        "host": "studio.local",
        "port": 22,
        "mosh": false,
        "mux": "none",
        "pill": "ssh",
        "state": "live",
        "cover": {"kind": "none", "state": "live"},
        "content": "terminal"
    });
    for (key, value) in changes.as_object().unwrap() {
        base[key] = value.clone();
    }
    base
}

/// A veil over a line that is not being redialled: its capsule's ring is
/// still, so nothing on the page runs a loop and a step settles.
fn veil(still: bool) -> Value {
    chrome(json!({
        "state": "offline",
        "cover": {"kind": "veil", "state": "offline"},
        "still": still,
    }))
}

/// Posts `message` and applies what it makes, the app's clock unmoved: a
/// fade under way is still mounted afterwards.
fn apply(app: &mut Headless, channel: &Channel, message: &Value) {
    channel.post(&message.to_string()).unwrap();
    let deadline = std::time::Instant::now() + HANG;
    loop {
        let taken = channel.all_taken();
        flushed(app);
        if taken {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "never applied");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn shows(app: &Headless, id: &str) -> bool {
    app.screen()
        .layout_nodes()
        .iter()
        .any(|n| n.test_id.as_deref() == Some(id))
}

fn open() -> (common::App, std::sync::Arc<Channel>) {
    let (app, channel, _) = start_observing(Screen::Terminal, Params::default(), "terminal-screen");
    (app, channel)
}

#[test]
fn a_switchs_veil_comes_and_goes_at_once() {
    let (mut app, channel) = open();
    post(&mut app, &channel, &chrome(json!({})).to_string());
    apply(&mut app, &channel, &veil(true));
    assert!(shows(&app, "terminal-reconnect-veil"));
    assert!(shows(&app, "terminal-reconnect-capsule"));
    // Lifted by a post that no longer says it is a switch (the host ends
    // the switch as the veil lifts): the veil stood through the switch and
    // leaves as it stood, at once.
    apply(&mut app, &channel, &chrome(json!({})));
    assert!(
        !shows(&app, "terminal-reconnect-veil"),
        "the switch's veil goes at once"
    );
    channel.close();
}

#[test]
fn an_ordinary_reconnects_veil_still_fades() {
    let (mut app, channel) = open();
    post(&mut app, &channel, &veil(false).to_string());
    assert!(shows(&app, "terminal-reconnect-veil"));
    apply(&mut app, &channel, &chrome(json!({})));
    assert!(
        shows(&app, "terminal-reconnect-veil"),
        "fading out: still there before its fade has run"
    );
    common::steady(&mut app);
    assert!(!shows(&app, "terminal-reconnect-veil"), "gone once faded");
    channel.close();
}

#[test]
fn a_switch_takes_the_banner_down_at_once() {
    let banner = json!({"kind": "returnPath"});
    for (still, after) in [(true, false), (false, true)] {
        let (mut app, channel) = open();
        post(
            &mut app,
            &channel,
            &chrome(json!({"mosh": true, "pill": "mosh", "banner": banner})).to_string(),
        );
        app.wait_for("id:mosh-returnpath-banner").unwrap();
        // "Use SSH": the switch is under way, and the new session has no
        // dead return path.
        apply(&mut app, &channel, &chrome(json!({"still": still})));
        assert_eq!(
            shows(&app, "mosh-returnpath-banner"),
            after,
            "still {still}: the banner {}",
            if after { "fades" } else { "goes at once" }
        );
        channel.close();
    }
}
