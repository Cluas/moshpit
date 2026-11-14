//! A symbol on a key is as large and as heavy as a key's words (Canary 118:
//! the shortcut bar's symbols drew at 22 pt regular beside 15 pt medium
//! words, half as heavy again as `esc` next to them). The rule, in the bar
//! and in Settings › Shortcuts' preview of it: every symbol on a key is
//! `mono-key`'s size at the user's text size, in its weight; the bar's
//! arrows, joystick, scroll thumb and keyboard key included, so the bar has
//! one symbol size. Around the keys nothing changes: the terminal's back
//! button keeps a bar button's size, a shortcut's row its chip's.

mod common;

use common::{App, Natives, post, start_observing};
use moshpit_ui::{
    screen::{Params, Screen},
    shortcuts::{self, Shortcut, Shortcuts},
    ui::{icon::IconSize, typography::TextToken},
};
use nsh_core::NodeId;
use nsh_testing::{Headless, LayoutNode};
use serde_json::{Value, json};

/// NSH's font scale at Large and at xxxLarge (body 23): the keys' words
/// stop at extra large, so the symbols too.
const LARGE: f32 = 1.0;
const XXX_LARGE: f32 = 23.0 / 17.0;

/// A symbol as the host was told to draw it.
#[derive(Debug)]
struct Symbol {
    name: String,
    points: f64,
    weight: String,
}

/// Whether `node` is `ancestor` or inside it.
fn within(nodes: &[LayoutNode], mut node: NodeId, ancestor: NodeId) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        match nodes.iter().find(|n| n.id == node) {
            Some(n) if n.parent != node => node = n.parent,
            _ => return false,
        }
    }
}

/// The symbols drawn inside the view `test_id`.
fn symbols_in(app: &Headless, natives: &Natives, test_id: &str) -> Vec<Symbol> {
    let nodes = app.screen().layout_nodes();
    let container = nodes
        .iter()
        .find(|n| n.test_id.as_deref() == Some(test_id))
        .unwrap_or_else(|| panic!("{test_id} is laid out"))
        .id;
    natives
        .all("moshpit/symbol")
        .into_iter()
        .filter(|node| within(&nodes, *node, container))
        .map(|node| Symbol {
            name: natives
                .prop(node, "name")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            points: natives
                .prop(node, "pointSize")
                .and_then(|v| v.as_f64())
                .unwrap_or_default(),
            weight: natives
                .prop(node, "weight")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
        })
        .collect()
}

/// Every symbol of `symbols` is a key's words' size and weight at
/// `scale`, and those named in `expected` are among them.
fn assert_key_sized(symbols: &[Symbol], scale: f32, expected: &[&str], place: &str) {
    let points = f64::from(TextToken::MonoKey.size(scale));
    for name in expected {
        assert!(
            symbols.iter().any(|s| s.name == *name),
            "{place}: {name} is drawn: {symbols:?}"
        );
    }
    for symbol in symbols {
        assert_eq!(
            symbol.points, points,
            "{place}: {} as large as the keys' words at scale {scale}",
            symbol.name
        );
        assert_eq!(
            symbol.weight, "medium",
            "{place}: {} as heavy as the keys' words",
            symbol.name
        );
    }
}

fn params(font_scale: f32) -> Params {
    Params {
        font_scale: Some(font_scale),
        ..Params::default()
    }
}

// --- The terminal's bar ---------------------------------------------------------

fn chrome() -> Value {
    json!({
        "op": "chrome",
        "name": "studio",
        "user": "cluas",
        "host": "studio.local",
        "port": 22,
        "mosh": false,
        "mux": "tmux",
        "pill": "ssh",
        "state": "live",
        "crumbs": {
            "session": "moshpit",
            "squeezable": true,
            "window": "2:agents",
            "pane": "claude",
            "signal": "attention",
            "paneLabel": "claude, needs you",
            "sessionIcon": "square.stack.3d.up",
            "windowIcon": "macwindow",
            "live": true
        },
        "cover": {"kind": "none", "state": "live"},
        "content": "terminal"
    })
}

/// Every kind of key that draws a symbol, beside two that spell their name.
fn bar() -> Value {
    json!({
        "op": "bar",
        "chips": [
            {"id": "esc", "kind": "key", "label": "esc"},
            {"id": "ctrl", "kind": "ctrl", "label": "ctrl"},
            {"id": "arrows", "kind": "arrows", "label": ""},
            {"id": "paste", "kind": "paste", "label": "paste"},
            {"id": "files", "kind": "files", "label": "files"},
            {"id": "image", "kind": "image", "label": ""},
            {"id": "mic", "kind": "mic", "label": ""},
            {"id": "dpad", "kind": "dpad", "label": ""},
            {"id": "scroll", "kind": "scroll", "label": ""}
        ],
        "keyboardDown": true,
        "ctrlArmed": false,
        "voice": true,
        "micActive": false,
        "image": {"clipboard": false, "camera": true, "sent": []}
    })
}

fn terminal(scale: f32) -> (App, Natives) {
    let (mut app, channel, natives) =
        start_observing(Screen::Terminal, params(scale), "terminal-screen");
    post(&mut app, &channel, &chrome().to_string());
    post(&mut app, &channel, &bar().to_string());
    app.wait_for("id:keyboard-toggle").unwrap();
    (app, natives)
}

#[test]
fn the_bars_symbols_are_the_keys_words_size() {
    for scale in [LARGE, XXX_LARGE] {
        let (app, natives) = terminal(scale);
        let keys = symbols_in(&app, &natives, "terminal-shortcut-keys");
        assert_key_sized(
            &keys,
            scale,
            &[
                "arrow.left",
                "arrow.up",
                "arrow.down",
                "arrow.right",
                "doc.on.clipboard",
                "folder",
                "photo",
                "mic",
                "arrow.up.and.down.and.arrow.left.and.right",
                "arrow.up.and.down",
                "keyboard",
                "keyboard.chevron.compact.down",
            ],
            "the bar",
        );
        // A navigation bar's button keeps a bar button's size.
        let back = symbols_in(&app, &natives, "terminal-back");
        assert!(!back.is_empty(), "the back button draws a symbol");
        for symbol in back {
            assert_eq!(
                symbol.points,
                f64::from(IconSize::Lg.points_at(scale)),
                "{symbol:?}"
            );
        }
    }
}

// --- Settings › Shortcuts' preview ------------------------------------------------

fn shortcut(id: &str, kind: &str, label: &str, summary: &str) -> Shortcut {
    Shortcut {
        id: id.into(),
        kind: kind.into(),
        label: label.into(),
        summary: summary.into(),
        builtin: true,
    }
}

#[test]
fn the_previews_symbols_are_the_keys_words_size() {
    let toolbar = vec![
        shortcut("esc", "keyCombo", "esc", "Escape"),
        shortcut("files", "files", "files", "Browse files"),
        shortcut("image", "image", "image", "Attach image"),
        shortcut("mic", "mic", "mic", "Voice input"),
        shortcut("dpad", "dpad", "dpad", "Arrow keys"),
        shortcut("scroll", "scroll", "scroll", "Scroll history"),
    ];
    let message = shortcuts::Message::Shortcuts(Shortcuts {
        toolbar,
        custom: vec![],
        available: vec![],
        limit: 12,
    })
    .to_json();
    for scale in [LARGE, XXX_LARGE] {
        let (mut app, channel, natives) =
            start_observing(Screen::Shortcuts, params(scale), "shortcuts");
        post(&mut app, &channel, &message);
        app.wait_for("id:shortcut-row-files").unwrap();
        let preview = symbols_in(&app, &natives, "shortcuts-preview");
        assert_key_sized(
            &preview,
            scale,
            &[
                "folder",
                "photo",
                "mic",
                "arrow.up.and.down.and.arrow.left.and.right",
                "arrow.up.and.down",
            ],
            "the preview",
        );
        // A shortcut's row keeps its chip's symbol.
        let row = symbols_in(&app, &natives, "shortcut-row-files");
        assert!(
            row.iter().any(|s| s.name == "folder"),
            "the row draws its folder: {row:?}"
        );
        for symbol in row.iter().filter(|s| s.name == "folder") {
            assert_eq!(
                symbol.points,
                f64::from(IconSize::Sm.points_at(scale)),
                "{symbol:?}"
            );
        }
        channel.close();
    }
}
