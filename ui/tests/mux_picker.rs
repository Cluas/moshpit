//! The terminal's pickers on a headless engine: the windows, sessions and
//! panes the host posts, in the multiplexer's words; a row picks, the list's
//! last row (New window, tab, session, workspace, pane) asks for a name
//! (the host's prompt) or splits a pane, a row's menu
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
fn the_new_row_asks_a_window_its_name_and_splits_a_pane_at_once() {
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

/// The breadcrumb's pane segment opens this list (the window's panes):
/// each row its agent or program and its dot, the current pane ticked, New
/// pane last; a row picked switches to that pane.
#[test]
fn panes_are_their_agents_first() {
    let (mut app, channel) = open(Params::default());
    let heard = hear(&channel);
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
    // Each pane's number in plain digits, where tmux's display-panes has it.
    app.wait_for("id:mux-picker-number-0").unwrap();
    app.wait_for("id:mux-picker-number-1").unwrap();
    for text in [
        "claude", "2.1.227", "working", "pane 1", "0", "1", "⌃", "b q",
    ] {
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
    assert_eq!(
        row_ids(&app),
        ["mux-picker-row-%1", "mux-picker-row-%2", "mux-picker-new"]
    );
    assert!(app.screen().shows("New Pane"));
    app.press("id:mux-picker-row-%2").unwrap();
    assert_eq!(next(&heard), json!({"ev": "select", "id": "%2"}));
    app.press("id:mux-picker-new").unwrap();
    assert_eq!(next(&heard), json!({"ev": "new"}));
    channel.close();
}

#[test]
fn other_panes_preview_what_their_screens_say_last() {
    let (mut app, channel) = open(Params::default());
    show(
        &mut app,
        &channel,
        &json!({
            "op": "picker", "kind": "panes", "mux": "tmux", "active": "%1",
            "rows": [
                {"id": "%1", "index": 0, "command": "zsh"},
                {"id": "%2", "index": 1, "command": "2.1.326", "agent": "claude", "signal": "done"},
                {"id": "%3", "index": 2, "command": "zsh"}
            ]
        }),
    );
    app.wait_for("id:mux-picker-row-%3").unwrap();
    // Words only until the host has a screen.
    assert!(app.screen().id("id:mux-picker-preview-%2").is_err());
    // Claude's screen: what it said, then its input box and mode line.
    let rule = "─".repeat(40);
    show(
        &mut app,
        &channel,
        &json!({"op": "pictures", "panes": {
            "%1": ["dev@gpu-01:~$ make", "ok"],
            "%2": [
                "Fixed the duplicate upload in run_local.cjs.",
                "",
                "* Worked for 3m 45s · done Thursday 11:59 PM",
                "",
                rule, "› ", rule,
                "  ▸▸ bypass permissions on (shift+tab to cycle)",
                "", ""
            ]
        }}),
    );
    app.wait_for("id:mux-picker-preview-%2").unwrap();
    let texts = app.screen().texts();
    for said in [
        "Fixed the duplicate upload in run_local.cjs.",
        "* Worked for 3m 45s · done Thursday 11:59 PM",
    ] {
        assert!(texts.contains(&said), "{said}: {texts:?}");
    }
    assert!(
        !texts.iter().any(|t| t.contains("bypass permissions")),
        "the box it waits in is no preview: {texts:?}"
    );
    // The shown pane is behind the sheet: no preview of it.
    assert!(app.screen().id("id:mux-picker-preview-%1").is_err());
    assert!(!texts.contains(&"ok"));
    // A later screen comes on its own and keeps the first; a row still picks.
    show(
        &mut app,
        &channel,
        &json!({"op": "pictures", "panes": {"%3": ["dev@gpu-01:~$ ls", "data  train.py", "dev@gpu-01:~$"]}}),
    );
    app.wait_for("id:mux-picker-preview-%3").unwrap();
    assert!(app.screen().shows("data  train.py"));
    assert!(app.screen().id("id:mux-picker-preview-%2").is_ok());
    let heard = hear(&channel);
    app.press("id:mux-picker-row-%3").unwrap();
    assert_eq!(next(&heard), json!({"ev": "select", "id": "%3"}));
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

/// A window tmux named after Claude Code's binary (its version) is called
/// what Home calls it, in its row and in the active session's meta; only
/// the windows list promises a swipe (it walks the session's panes, never
/// another session).
#[test]
fn version_named_windows_say_their_folder_and_sessions_are_tapped() {
    let (mut app, channel) = open(Params::default());
    show(
        &mut app,
        &channel,
        &json!({
            "op": "picker", "kind": "windows", "mux": "tmux", "active": "@2",
            "rows": [
                {"id": "@1", "index": 1, "name": "codex", "panes": 1, "command": "codex",
                 "path": "/home/dev/src/api", "agent": "codex"},
                {"id": "@2", "index": 2, "name": "2.1.322", "panes": 3, "command": "2.1.322",
                 "path": "/home/dev/src/trainer", "agent": "claude", "signal": "attention"},
                {"id": "@3", "index": 3, "name": "2.1.322", "panes": 1, "command": "2.1.322",
                 "path": "/home/dev"}
            ]
        }),
    );
    app.wait_for("id:mux-picker-row-@3").unwrap();
    for text in [
        "1: api",
        "2: trainer",
        "3: claude",
        "3 panes · claude",
        "Swipe to switch",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(
        !app.screen().texts().iter().any(|t| t.contains("2.1.322")),
        "a version string: {:?}",
        app.screen().texts()
    );

    show(
        &mut app,
        &channel,
        &json!({
            "op": "picker", "kind": "sessions", "mux": "tmux", "active": "$0",
            "rows": [
                {"id": "$0", "name": "work", "windows": 4, "window": "2:2.1.322",
                 "command": "2.1.322", "path": "/home/dev/src/trainer", "agent": "claude"},
                {"id": "$1", "name": "play"}
            ]
        }),
    );
    app.wait_for("id:mux-picker-row-$1").unwrap();
    for text in ["4 windows · 2: trainer", "Tap to switch"] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(
        !app.screen().shows("Swipe to switch"),
        "sessions never swipe"
    );

    // herdr's tabs are swiped through too now.
    show(&mut app, &channel, &windows("herdr"));
    app.wait_for("text:Tabs").unwrap();
    assert!(
        app.screen().shows("Swipe to switch"),
        "{:?}",
        app.screen().texts()
    );
    assert!(!app.screen().shows("Tap to switch"));
    channel.close();
}

/// B650: beside a herdr workspace named after its folder, its active tab
/// (herdr's default name "1", working there) is "Tab 1", not the
/// workspace's name again.
#[test]
fn a_sessions_window_never_says_its_name_again() {
    let (mut app, channel) = open(Params::default());
    show(
        &mut app,
        &channel,
        &json!({
            "op": "picker", "kind": "sessions", "mux": "herdr", "active": "wV",
            "rows": [
                {"id": "wV", "name": "moshpit", "windows": 3, "window": "1:1",
                 "command": "claude", "path": "/Users/dev/code/moshpit", "agent": "claude"},
                {"id": "wW", "name": "site"}
            ]
        }),
    );
    app.wait_for("id:mux-picker-row-wW").unwrap();
    assert!(
        app.screen().shows("3 tabs · Tab 1"),
        "{:?}",
        app.screen().texts()
    );
    assert!(!app.screen().shows("3 tabs · 1: moshpit"));
    channel.close();
}

/// B652: a session's windows counted in the language's own way. The meta
/// glued the number to a bare noun ("3 窗口", "3 ウィンドウ", "3 창").
#[test]
fn a_sessions_windows_are_counted_the_languages_way() {
    let (mut app, channel) = open(zh_hans());
    for (mux, said) in [
        ("tmux", "3 个窗口 · 1: api"),
        ("herdr", "3 个标签页 · 1: api"),
    ] {
        show(
            &mut app,
            &channel,
            &json!({
                "op": "picker", "kind": "sessions", "mux": mux, "active": "$0",
                "rows": [
                    {"id": "$0", "name": "work", "windows": 3, "window": "1:api"},
                    {"id": "$1", "name": "ops"}
                ]
            }),
        );
        app.wait_for("id:mux-picker-row-$1").unwrap();
        assert!(
            app.screen().shows(said),
            "{mux}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}

/// B622: the picker as a held share's: every pane of the tree with its
/// place, those running the share's agent first (the host's order), the
/// hint saying so; no ＋ and no keys. A row tells the host which pane.
#[test]
fn a_held_shares_picker_lists_panes_by_place_and_sends_to_the_one_picked() {
    let (mut app, channel) = open(Params::default());
    let heard = hear(&channel);
    show(
        &mut app,
        &channel,
        &json!({
            "op": "picker", "kind": "panes", "mux": "tmux", "send": "claude",
            "rows": [
                {"id": "%2", "index": 0, "command": "claude", "agent": "claude",
                 "window": "work · review", "signal": "working"},
                {"id": "%9", "index": 1, "command": "vim", "window": "work · claude"},
            ]
        }),
    );
    app.wait_for("id:mux-picker-row-%9").unwrap();
    for text in [
        "Select Pane",
        "work · review",
        "work · claude",
        "Panes running claude come first.",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(!app.screen().shows("⌃"), "a key under a share's picker");
    assert!(
        !app.screen()
            .layout_nodes()
            .iter()
            .any(|n| n.test_id.as_deref() == Some("mux-picker-new")),
        "a share's picker offers to make a pane"
    );
    app.press("id:mux-picker-row-%2").unwrap();
    assert_eq!(next(&heard), json!({"ev": "select", "id": "%2"}));
    quiet(&heard);
    channel.close();
}

/// The rows' ids in order, the New row's included.
fn row_ids(app: &Headless) -> Vec<String> {
    app.screen()
        .layout_nodes()
        .into_iter()
        .filter_map(|n| n.test_id.map(|t| t.to_string()))
        .filter(|t| {
            (t.starts_with("mux-picker-row-") && !t.ends_with("-below")) || t == "mux-picker-new"
        })
        .collect()
}

/// Every list ends in its New row, in the multiplexer's words, a full row
/// (never a nav-bar ＋); a share's list has none.
#[test]
fn every_list_ends_in_its_new_row() {
    let (mut app, channel) = open(Params::default());
    let sessions = |mux: &str| {
        json!({
            "op": "picker", "kind": "sessions", "mux": mux, "active": "$0",
            "rows": [
                {"id": "$0", "name": "work", "windows": 2, "command": "zsh"},
                {"id": "$1", "name": "play", "windows": 1, "command": "htop", "signal": "attention"}
            ]
        })
    };
    let mut panes = windows("tmux");
    panes["kind"] = json!("panes");
    for (list, title) in [
        (windows("tmux"), "New window"),
        (windows("herdr"), "New tab"),
        (sessions("tmux"), "New Session"),
        (sessions("herdr"), "New Workspace"),
        (panes, "New Pane"),
    ] {
        show(&mut app, &channel, &list);
        app.wait_for("id:mux-picker-new").unwrap();
        app.wait_for(&format!("label:{title}")).unwrap();
        let ids = row_ids(&app);
        assert_eq!(
            ids.last().map(String::as_str),
            Some("mux-picker-new"),
            "{title}: {ids:?}"
        );
        assert!(ids.len() > 1, "{title}: {ids:?}");
        let new = app
            .screen()
            .layout_nodes()
            .into_iter()
            .find(|n| n.test_id.as_deref() == Some("mux-picker-new"))
            .unwrap()
            .bounds;
        assert!(new.size.height.0 >= 44.0, "{title}: {new:?}");
    }
    // A session not in use says what runs in it.
    show(&mut app, &channel, &sessions("tmux"));
    app.wait_for("id:mux-picker-row-$1").unwrap();
    assert!(app.screen().shows("htop"), "{:?}", app.screen().texts());
    channel.close();
}
