//! The terminal's pickers on a headless engine: the windows, sessions and
//! panes the host posts, in the multiplexer's words; a row picks, the ＋
//! asks for a name (the host's prompt) or splits a pane, a row's menu
//! renames after asking and kills after NSH's action sheet; the app's
//! language.

mod common;

use common::{hear, next, post, quiet, start, zh_hans};
use moshpit_ui::screen::{Params, Screen};
use nsh_testing::{DialogAnswer, Headless};
use serde_json::{Value, json};

fn windows(mux: &str) -> Value {
    json!({
        "op": "picker",
        "kind": "windows",
        "mux": mux,
        "active": "@2",
        "rows": [
            {"id": "@1", "index": 1, "name": "agents", "panes": 2, "command": "zsh", "signal": "attention"},
            {"id": "@2", "index": 2, "name": "logs", "panes": 1, "command": "tail"},
            {"id": "@3", "index": 3, "name": "3", "panes": 1, "command": ""}
        ]
    })
}

fn open(params: Params) -> (common::App, std::sync::Arc<moshpit_ui::channel::Channel>) {
    let (mut app, channel) = start(Screen::MuxPicker, params, "mux-picker");
    app.answer_dialogs_myself();
    (app, channel)
}

fn show(app: &mut Headless, channel: &moshpit_ui::channel::Channel, message: &Value) {
    post(app, channel, &message.to_string());
}

#[test]
fn windows_show_in_tmux_words_and_a_row_picks() {
    let (mut app, channel) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &windows("tmux"));
    app.wait_for("id:mux-picker-row-@1").unwrap();
    for text in [
        "Windows",
        "1: agents",
        "2 panes · zsh",
        "2: logs",
        "1 pane · tail",
        "Window 3",
        "Swipe to switch",
        "⌃",
        "1-9",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    app.press("id:mux-picker-row-@1").unwrap();
    assert_eq!(next(&heard), json!({"ev": "select", "id": "@1"}));
    quiet(&heard);
}

#[test]
fn the_plus_asks_a_window_its_name_and_splits_a_pane_at_once() {
    let (mut app, channel) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &windows("tmux"));
    app.wait_for("id:mux-picker-new").unwrap();
    app.press("id:mux-picker-new").unwrap();
    let prompt = next(&heard);
    assert_eq!(prompt["ev"], "prompt");
    assert_eq!(prompt["title"], "New Window");
    assert_eq!(prompt["message"], "Leave blank to let the program name it.");
    assert_eq!(prompt["placeholder"], "Name (optional)");
    assert_eq!(prompt["confirm"], "Create");
    let id = prompt["id"].as_u64().unwrap();
    show(
        &mut app,
        &channel,
        &json!({"op": "prompted", "id": id, "text": "  build "}),
    );
    assert_eq!(next(&heard), json!({"ev": "new", "name": "build"}));
    // Blank: the program names it. Cancelled: nothing.
    app.press("id:mux-picker-new").unwrap();
    let id = next(&heard)["id"].as_u64().unwrap();
    show(
        &mut app,
        &channel,
        &json!({"op": "prompted", "id": id, "text": " "}),
    );
    assert_eq!(next(&heard), json!({"ev": "new", "name": null}));
    app.press("id:mux-picker-new").unwrap();
    let id = next(&heard)["id"].as_u64().unwrap();
    show(
        &mut app,
        &channel,
        &json!({"op": "prompted", "id": id, "text": null}),
    );
    quiet(&heard);

    let mut panes = windows("tmux");
    panes["kind"] = json!("panes");
    show(&mut app, &channel, &panes);
    app.press("id:mux-picker-new").unwrap();
    assert_eq!(next(&heard), json!({"ev": "new"}));
    assert!(app.screen().shows("Select Pane"));
    assert!(app.screen().shows("Tap to focus · ＋ splits a new pane"));
}

#[test]
fn panes_are_their_agents_first() {
    let (mut app, channel) = open(Params::default());
    show(
        &mut app,
        &channel,
        &json!({
            "op": "picker", "kind": "panes", "mux": "tmux", "active": "%1",
            "rows": [
                {"id": "%1", "index": 0, "command": "2.1.227", "agent": "claude", "signal": "working"},
                {"id": "%2", "index": 1, "command": ""}
            ]
        }),
    );
    app.wait_for("id:mux-picker-row-%2").unwrap();
    // The agent's state in words, in its color, never the color alone.
    for text in ["claude", "2.1.227", "working", "pane 1", "⌃", "b q"] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    // The active row is ticked and selected; each row a target 44 pt tall
    // at least, as wide as the card.
    assert!(app.screen().selected("id:mux-picker-row-%1").unwrap());
    assert!(!app.screen().selected("id:mux-picker-row-%2").unwrap());
    let bounds = |id: &str| {
        app.screen()
            .layout_nodes()
            .into_iter()
            .find(|node| node.test_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("no view {id}"))
            .bounds
    };
    let card = bounds("mux-picker-rows");
    for id in ["mux-picker-row-%1", "mux-picker-row-%2"] {
        let row = bounds(id);
        assert!(row.size.height.0 >= 44.0, "{id}: {row:?}");
        assert_eq!(row.size.width.0, card.size.width.0, "{id}");
    }
}

#[test]
fn a_rename_asks_and_a_cleared_or_unchanged_name_renames_nothing() {
    let (mut app, channel) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &windows("tmux"));
    app.wait_for("id:mux-picker-row-@1").unwrap();
    for (answer, expect) in [
        (
            json!("ops"),
            Some(json!({"ev": "rename", "id": "@1", "name": "ops"})),
        ),
        (json!("agents"), None),
        (json!("  "), None),
        (json!(null), None),
    ] {
        show(
            &mut app,
            &channel,
            &json!({"op": "act", "action": "rename", "id": "@1"}),
        );
        let prompt = next(&heard);
        assert_eq!(prompt["title"], "Rename Window");
        assert_eq!(prompt["text"], "agents");
        assert_eq!(prompt["confirm"], "Rename");
        let id = prompt["id"].as_u64().unwrap();
        show(
            &mut app,
            &channel,
            &json!({"op": "prompted", "id": id, "text": answer}),
        );
        match expect {
            Some(event) => assert_eq!(next(&heard), event),
            None => quiet(&heard),
        }
    }
}

#[test]
fn a_kill_asks_on_the_action_sheet_first() {
    let (mut app, channel) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &windows("herdr"));
    app.wait_for("id:mux-picker-row-@1").unwrap();
    show(
        &mut app,
        &channel,
        &json!({"op": "act", "action": "kill", "id": "@1"}),
    );
    assert!(common::wait_dialog(&mut app), "the sheet never asked");
    let dialog = app.pending_dialog().cloned().expect("the sheet asks");
    assert!(dialog.sheet);
    assert_eq!(
        dialog.title.as_deref(),
        Some("Close tab 1: agents? Every pane in it dies.")
    );
    let texts: Vec<_> = dialog.buttons.iter().map(|b| b.text.clone()).collect();
    assert_eq!(texts, ["Close Tab", "Cancel"]);
    app.answer_dialog(DialogAnswer::Press("Cancel".into()))
        .unwrap();
    quiet(&heard);
    show(
        &mut app,
        &channel,
        &json!({"op": "act", "action": "kill", "id": "@1"}),
    );
    app.answer_dialog(DialogAnswer::Press("Close Tab".into()))
        .unwrap();
    assert_eq!(next(&heard), json!({"ev": "kill", "id": "@1"}));
}

#[test]
fn herdr_workspaces_in_chinese() {
    let (mut app, channel) = open(zh_hans());
    show(
        &mut app,
        &channel,
        &json!({
            "op": "picker", "kind": "sessions", "mux": "herdr", "active": "w1",
            "rows": [
                {"id": "w1", "name": "~", "windows": 3, "window": "1:agents"},
                {"id": "w4", "name": "~", "attached": true},
                {"id": "w5", "name": "api"}
            ]
        }),
    );
    app.wait_for("id:mux-picker-row-w5").unwrap();
    for text in [
        "工作区",
        "~ · w1",
        "~ · w4",
        "已附加",
        "已分离",
        "点按切换",
        "b w",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
}
