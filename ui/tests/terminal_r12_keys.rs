//! R12's shortcut bar and panels on a headless engine (`docs/nsh/UI-DESIGN.md`
//! §6.3, §4.17, §8): every key takes presses over at least 44 pt of height
//! and has a name screen readers say; the bar is a key and 8 pt around it
//! (52 pt, 38 in iPad landscape); an armed ctrl is the same key, selected,
//! not one made again (its fill can move); the voice panel's Insert and
//! Send take no press while there is nothing to insert.

mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use common::{App, hear, post};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
};
use nsh_core::{
    Bounds, EdgeInsets, NodeId, Points, Size,
    platform::{HostEnvironment, Idiom, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;
use serde_json::{Value, json};

/// The hit slop each view was given, as the engine mounted it.
#[derive(Clone, Default)]
struct Slops(Arc<Mutex<HashMap<NodeId, EdgeInsets>>>);

impl Slops {
    fn apply(&self, batch: &nsh_core::MountBatch) {
        let mut slops = self.0.lock().unwrap();
        for item in &batch.items {
            if let nsh_core::MountItem::ViewProps { id, props } = item {
                slops.insert(*id, props.hit_slop);
            }
        }
    }

    fn of(&self, node: NodeId) -> EdgeInsets {
        self.0
            .lock()
            .unwrap()
            .get(&node)
            .copied()
            .unwrap_or_default()
    }
}

/// The node and the bounds of the view whose test id is `id`.
fn node(app: &Headless, id: &str) -> (NodeId, Bounds<Points>) {
    let node = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no view {id}"));
    (node.id, node.bounds)
}

/// The next intent the screen told the host, the anchors skipped.
fn next_intent(rx: &std::sync::mpsc::Receiver<String>) -> Value {
    loop {
        let value = common::next(rx);
        if value["ev"] != "anchor" {
            return value;
        }
    }
}

/// Nothing but anchors told.
fn no_intent(rx: &std::sync::mpsc::Receiver<String>) {
    while let Ok(json) = rx.recv_timeout(Duration::from_millis(200)) {
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["ev"], "anchor", "the screen told the host {value}");
    }
}

fn chrome(compact: bool) -> Value {
    json!({
        "op": "chrome",
        "name": "build-box",
        "user": "cluas",
        "host": "build-box.example.com",
        "port": 22,
        "mux": "tmux",
        "pill": "mosh",
        "state": "live",
        "cover": {"kind": "none", "state": "live"},
        "content": "terminal",
        "compact": compact
    })
}

/// Every kind of chip the bar draws.
fn bar(ctrl_armed: bool) -> Value {
    json!({
        "op": "bar",
        "chips": [
            {"id": "esc", "kind": "key", "label": "esc"},
            {"id": "tab", "kind": "key", "label": "tab"},
            {"id": "ctrl", "kind": "ctrl", "label": "ctrl"},
            {"id": "arrows", "kind": "arrows", "label": ""},
            {"id": "ll", "kind": "text", "label": "ll"},
            {"id": "paste", "kind": "paste", "label": "paste"},
            {"id": "files", "kind": "files", "label": ""},
            {"id": "mic", "kind": "mic", "label": ""},
            {"id": "image", "kind": "image", "label": ""},
            {"id": "dpad", "kind": "dpad", "label": ""},
            {"id": "scroll", "kind": "scroll", "label": ""}
        ],
        "keyboardDown": true,
        "ctrlArmed": ctrl_armed,
        "voice": true,
        "micActive": false,
        "image": {"clipboard": true, "camera": true, "sent": []}
    })
}

/// The screen on the owner's iPhone (402 pt wide), the hit slops observed
/// from its first mount (the keyboard key is there before any chip),
/// showing `chrome` and the bar. The engine as `common` starts it, on a
/// manual clock.
fn open(compact: bool) -> (App, Arc<Channel>, Slops) {
    let channel = Channel::open();
    let params = Params::default();
    let mut config = EngineConfig::new(Screen::Terminal.root(&channel, &params));
    config.viewport = Size::new(Points(402.0), Points(874.0));
    config.environment.os = Os::Ios;
    config.environment.os_version = (26, 0);
    config.environment.idiom = Idiom::Phone;
    config.environment.pixel_ratio = 3.0;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    HostEnvironment {
        locale: params.nsh_locale(),
        font_scale: None,
    }
    .apply(&mut config.environment);
    let mut app = Headless::start(config).expect("the engine starts");
    let slops = Slops::default();
    let observed = slops.clone();
    app.observe(move |_, batch| observed.apply(batch));
    app.wait_for("id:terminal-screen").unwrap();
    let mut app = App(app);
    post(&mut app, &channel, &chrome(compact).to_string());
    post(&mut app, &channel, &bar(false).to_string());
    app.wait_for("id:keyboard-toggle").unwrap();
    (app, channel, slops)
}

/// The keys of [`bar`], by test id, and what screen readers say for each
/// (the image key's name is its menu's, which the host draws).
const KEYS: &[(&str, Option<&str>)] = &[
    ("shortcut-esc", Some("esc")),
    ("shortcut-tab", Some("tab")),
    ("shortcut-ctrl", Some("Control")),
    ("arrow-left", Some("Move left")),
    ("arrow-up", Some("Move up")),
    ("arrow-down", Some("Move down")),
    ("arrow-right", Some("Move right")),
    ("shortcut-ll", Some("ll")),
    ("shortcut-paste", Some("Paste")),
    ("shortcut-files", Some("Browse files")),
    ("voice-input", Some("Start voice input")),
    ("shortcut-image", None),
    ("dpad-joystick", Some("Arrow keys")),
    ("scroll-thumb", Some("Scroll history")),
    ("keyboard-toggle", Some("Show keyboard")),
];

#[test]
fn every_key_takes_presses_over_44_pt_of_height_and_has_a_name() {
    let (app, channel, slops) = open(false);
    for (id, name) in KEYS {
        let (node, bounds) = node(&app, id);
        let slop = slops.of(node);
        let tall = bounds.size.height.0 + slop.top.0 + slop.bottom.0;
        assert!(tall >= 44.0, "{id}: takes presses over {tall} pt of height");
        if let Some(name) = name {
            assert!(
                app.screen().matches(&format!("label:{name}")).unwrap(),
                "{id}: screen readers say {name}"
            );
        }
    }
    channel.close();
}

#[test]
fn the_bar_is_a_key_and_8_pt_around_it_and_trims_in_ipad_landscape() {
    for (compact, tall) in [(false, 52.0), (true, 38.0)] {
        let (app, channel, _) = open(compact);
        let (_, bounds) = node(&app, "terminal-shortcut-keys");
        assert_eq!(bounds.size.height.0, tall, "compact: {compact}");
        let (_, key) = node(&app, "shortcut-esc");
        assert_eq!(key.size.height.0, 36.0, "a key is drawn 36 pt tall");
        channel.close();
    }
}

#[test]
fn an_armed_ctrl_is_the_same_key_selected() {
    let (mut app, channel, _) = open(false);
    let (before, _) = node(&app, "shortcut-ctrl");
    let selected = |app: &Headless| {
        app.screen()
            .accessibility_state("id:shortcut-ctrl")
            .unwrap()
            .selected
    };
    assert!(!selected(&app));
    post(&mut app, &channel, &bar(true).to_string());
    app.wait_for("label:Control armed").unwrap();
    assert!(selected(&app), "screen readers hear it armed");
    let (after, _) = node(&app, "shortcut-ctrl");
    assert_eq!(before, after, "not made again: its fill moves");
    let (esc_after, _) = node(&app, "shortcut-esc");
    post(&mut app, &channel, &bar(false).to_string());
    app.wait_for("label:Control").unwrap();
    assert_eq!(
        node(&app, "shortcut-esc").0,
        esc_after,
        "a key held through the let-go stays under the finger"
    );
    channel.close();
}

#[test]
fn the_voice_panel_inserts_and_sends_only_what_it_heard() {
    let (mut app, channel, _) = open(false);
    let heard = hear(&channel);
    let dictation = |insertable: bool, finalized: &str| {
        json!({"op": "dictation", "panel": {
            "phase": {"phase": "listening"}, "engine": "Whisper · 中文",
            "finalized": finalized, "volatile": "", "insertable": insertable
        }})
        .to_string()
    };
    post(&mut app, &channel, &dictation(false, ""));
    app.wait_for("id:dictation-overlay").unwrap();
    assert!(
        app.screen().shows("Whisper · 中文"),
        "the engine, on a badge"
    );
    for id in ["dictation-insert", "dictation-send"] {
        let state = app
            .screen()
            .accessibility_state(&format!("id:{id}"))
            .unwrap();
        assert!(state.disabled, "{id}: nothing to insert");
        let _ = app.press(&format!("id:{id}"));
    }
    no_intent(&heard);
    post(&mut app, &channel, &dictation(true, "git status"));
    app.wait_for("text:git status").unwrap();
    app.press("id:dictation-insert").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "dictation", "action": "insert"})
    );
    channel.close();
}

/// The trailing fade says there is more past the edge: only while a key
/// is. Every key in view (only the row's own inset past the edge), there
/// is none over the last one.
#[test]
fn the_trailing_fade_shows_only_while_a_key_is_past_the_edge() {
    let (mut app, channel, _) = open(false);
    let fits = json!({
        "op": "bar",
        "chips": [
            {"id": "esc", "kind": "key", "label": "esc"},
            {"id": "tab", "kind": "key", "label": "tab"},
            {"id": "ctrl", "kind": "ctrl", "label": "ctrl"}
        ],
        "keyboardDown": true, "ctrlArmed": false, "voice": false, "micActive": false,
        "image": {"clipboard": false, "camera": false, "sent": []}
    });
    post(&mut app, &channel, &fits.to_string());
    app.wait_for("id:shortcut-ctrl").unwrap();
    assert!(
        !app.screen()
            .matches("id:terminal-chip-fade-trailing")
            .unwrap(),
        "three keys fit, and the last is faded"
    );
    // Every kind of chip: more than a phone's width.
    post(&mut app, &channel, &bar(false).to_string());
    app.wait_for("id:shortcut-esc").unwrap();
    assert!(
        app.screen()
            .matches("id:terminal-chip-fade-trailing")
            .unwrap(),
        "keys past the edge, and no fade says so"
    );
    channel.close();
}
