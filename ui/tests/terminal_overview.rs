//! The pane overview on a headless engine (`ui/src/terminal/overview.rs`):
//! the breadcrumb's window and pane open it (held, the old lists), ⌘K
//! toggles it, the host opens it for VoiceOver; a card per window with its
//! panes and their states; a pane picked switches and closes it; New
//! window asks a name; the card's ⋯ and a pane's long press split, rename
//! after asking, kill after the action sheet; past four panes, "+N" opens
//! the rest.

mod common;

use std::{sync::mpsc, time::Duration};

use common::{Natives, emit, hear, post, start_observing};
use moshpit_ui::screen::{Params, Screen};
use nsh_testing::DialogAnswer;
use serde_json::{Value, json};

/// The next event named `ev` the screen told the host, the others passed.
fn next_of(rx: &mpsc::Receiver<String>, ev: &str) -> Value {
    loop {
        let json = rx
            .recv_timeout(common::WAIT)
            .unwrap_or_else(|_| panic!("the screen tells the host {ev}"));
        let value: Value = serde_json::from_str(&json).unwrap();
        if value["ev"] == ev {
            return value;
        }
    }
}

/// What the host's long-press menu on pane `pane` says it picked.
fn act_message(action: &str, pane: &str) -> String {
    json!({"op": "overview-act", "action": action, "pane": pane}).to_string()
}

/// No event named `ev` comes (the others pass).
fn none_of(rx: &mpsc::Receiver<String>, ev: &str) {
    while let Ok(json) = rx.recv_timeout(Duration::from_millis(300)) {
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_ne!(value["ev"], ev, "told {value}");
    }
}

/// The mockup's session `work`, as a tree: api (codex working), trainer
/// (claude waiting, python, zsh), web (zsh, node), docs (claude done),
/// logs (six tails).
fn work(mux: &str, active: &str) -> Value {
    let mut panes = vec![
        json!({"id": "%1", "window": "@1", "index": 0, "command": "codex", "path": "/home/dev/src/api"}),
        json!({"id": "%2", "window": "@2", "index": 0, "command": "2.1.322", "path": "/home/dev/src/trainer"}),
        json!({"id": "%3", "window": "@2", "index": 1, "command": "python", "path": "/home/dev/src/trainer"}),
        json!({"id": "%4", "window": "@2", "index": 2, "command": "zsh", "path": "/home/dev/src/trainer"}),
        json!({"id": "%5", "window": "@3", "index": 0, "command": "zsh", "path": "/home/dev/src/web"}),
        json!({"id": "%6", "window": "@3", "index": 1, "command": "node", "path": "/home/dev/src/web"}),
        json!({"id": "%7", "window": "@4", "index": 0, "command": "2.1.322", "path": "/home/dev/src/docs"}),
    ];
    for i in 0..6 {
        panes.push(
            json!({"id": format!("%{}", 10 + i), "window": "@5", "index": i,
                          "command": "tail", "path": "/var/log"}),
        );
    }
    json!({
        "mux": mux, "attached": true,
        "sessions": [{"id": "$0", "name": "work"}],
        "windows": [
            {"id": "@1", "session": "$0", "index": 1, "name": "codex", "paneCount": 1},
            {"id": "@2", "session": "$0", "index": 2, "name": "2.1.322", "paneCount": 3},
            {"id": "@3", "session": "$0", "index": 3, "name": "zsh", "paneCount": 2},
            {"id": "@4", "session": "$0", "index": 4, "name": "docs", "paneCount": 1},
            {"id": "@5", "session": "$0", "index": 5, "name": "logs", "paneCount": 6}
        ],
        "panes": panes,
        "hooks": [
            {"pane": "%1", "state": "working", "agent": "codex"},
            {"pane": "%2", "state": "attention", "agent": "claude"},
            {"pane": "%7", "state": "done"}
        ],
        "worktrees": [],
        "activeSession": "$0", "activeWindow": "@2", "activePane": active
    })
}

/// The chrome of a live tmux (or herdr) session on `active`, its breadcrumb
/// live.
fn chrome(mux: &str, active: &str) -> Value {
    json!({
        "op": "chrome",
        "name": "gpu-01", "user": "dev", "host": "gpu-01.example.com", "port": 22,
        "mosh": false, "mux": mux, "pill": "ssh", "state": "live",
        "crumbs": {
            "session": "work", "squeezable": true, "window": "2:2.1.322", "pane": "claude",
            "signal": "attention", "paneLabel": "claude, needs you",
            "sessionIcon": "square.stack.3d.up", "windowIcon": "macwindow", "live": true,
            "tree": work(mux, active)
        },
        "cover": {"kind": "none", "state": "live"},
        "content": "terminal"
    })
}

fn switcher(mux: &str, active: &str) -> String {
    json!({"op": "switcher", "tree": work(mux, active), "hintSeen": true}).to_string()
}

fn open_screen(
    mux: &str,
    active: &str,
) -> (
    common::App,
    std::sync::Arc<moshpit_ui::channel::Channel>,
    mpsc::Receiver<String>,
    Natives,
) {
    let (mut app, channel, natives) =
        start_observing(Screen::Terminal, Params::default(), "terminal-screen");
    app.0.answer_dialogs_myself();
    let heard = hear(&channel);
    post(&mut app.0, &channel, &chrome(mux, active).to_string());
    post(&mut app.0, &channel, &switcher(mux, active));
    (app, channel, heard, natives)
}

/// Where view `id` is laid out, in the surface's points.
fn bounds(app: &common::App, id: &str) -> [f32; 4] {
    let b = app
        .0
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no view {id}"))
        .bounds;
    [b.origin.x.0, b.origin.y.0, b.size.width.0, b.size.height.0]
}

/// `rect` (an event's) is `expected`, to half a point.
fn same_place(rect: &Value, expected: [f32; 4]) {
    let rect: Vec<f32> = rect
        .as_array()
        .unwrap_or_else(|| panic!("a rect: {rect}"))
        .iter()
        .map(|v| v.as_f64().unwrap() as f32)
        .collect();
    assert_eq!(rect.len(), 4);
    for (a, b) in rect.iter().zip(expected) {
        assert!((a - b).abs() < 0.5, "{rect:?} vs {expected:?}");
    }
}

/// The events the screen tells the host up to and with the first named
/// `ev`.
fn events_until(rx: &mpsc::Receiver<String>, ev: &str) -> Vec<Value> {
    let mut events = Vec::new();
    loop {
        let json = rx
            .recv_timeout(common::WAIT)
            .unwrap_or_else(|_| panic!("the screen tells the host {ev}: {events:?}"));
        let value: Value = serde_json::from_str(&json).unwrap();
        let done = value["ev"] == ev;
        events.push(value);
        if done {
            return events;
        }
    }
}

fn shows_overview(app: &common::App) -> bool {
    app.0.screen().count("id:terminal-overview").unwrap() > 0
}

#[test]
fn the_window_crumb_opens_the_overview_and_held_opens_the_old_list() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    app.0.wait_for("id:terminal-crumb-window").unwrap();
    app.0.press("id:terminal-crumb-window").unwrap();
    app.0.wait_for("id:terminal-overview").unwrap();
    assert_eq!(
        next_of(&heard, "overview"),
        json!({"ev": "overview", "open": true}),
        "the host puts the keyboard away"
    );
    none_of(&heard, "crumb");
    app.0.press("id:terminal-overview-done").unwrap();
    assert_eq!(
        next_of(&heard, "overview"),
        json!({"ev": "overview", "open": false})
    );
    // Held: the windows list, as before, until the owner has tried this.
    app.0.long_press("id:terminal-crumb-window").unwrap();
    assert_eq!(
        next_of(&heard, "crumb"),
        json!({"ev": "crumb", "crumb": "window"})
    );
    // The pane's segment opens it too; the session's still opens its list.
    app.0.press("id:terminal-crumb-pane").unwrap();
    app.0.wait_for("id:terminal-overview").unwrap();
    channel.close();
}

#[test]
fn a_card_per_window_with_its_panes_and_their_states() {
    let (mut app, channel, _heard, _) = open_screen("tmux", "%3");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:overview-card-@5").unwrap();
    for text in [
        "work",
        "5 windows · 13 panes",
        "1 waiting",
        "api",
        "trainer",
        "web",
        "docs",
        "logs",
        "codex",
        "claude",
        "python",
        "node",
        "Needs you · 3 panes",
        "Finished",
        "New window",
        "+3",
    ] {
        assert!(
            app.0.screen().shows(text),
            "{text}: {:?}",
            app.0.screen().texts()
        );
    }
    assert!(
        !app.0.screen().texts().iter().any(|t| t.contains("2.1.322")),
        "never a version string"
    );
    // The current pane is the selected one.
    assert!(app.0.screen().selected("id:overview-pane-%3").unwrap());
    assert!(!app.0.screen().selected("id:overview-pane-%2").unwrap());
    channel.close();
}

#[test]
fn a_pane_picked_switches_to_it_and_the_overview_goes() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:overview-pane-%6").unwrap();
    assert_eq!(next_of(&heard, "overview")["open"], true);
    app.0.press("id:overview-pane-%6").unwrap();
    assert_eq!(
        next_of(&heard, "select-pane"),
        json!({"ev": "select-pane", "id": "%6"})
    );
    assert_eq!(next_of(&heard, "overview")["open"], false);
    app.0.run_for(Duration::from_millis(400)).unwrap();
    assert!(!shows_overview(&app));
    channel.close();
}

#[test]
fn command_k_opens_and_closes_it() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    let key = json!({"op": "key", "stroke": "mod+k"}).to_string();
    post(&mut app.0, &channel, &key);
    app.0.wait_for("id:terminal-overview").unwrap();
    assert_eq!(next_of(&heard, "overview")["open"], true);
    none_of(&heard, "crumb");
    // A second press, later (the twin rule drops one within 150 ms).
    app.0.run_for(Duration::from_millis(300)).unwrap();
    post(&mut app.0, &channel, &key);
    assert_eq!(next_of(&heard, "overview")["open"], false);
    channel.close();
}

#[test]
fn a_new_window_is_asked_its_name() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:overview-new-window").unwrap();
    app.0.press("id:overview-new-window").unwrap();
    let prompt = next_of(&heard, "prompt");
    assert_eq!(prompt["title"], "New Window");
    let id = prompt["id"].as_u64().unwrap();
    post(
        &mut app.0,
        &channel,
        &json!({"op": "prompted", "id": id, "text": " build "}).to_string(),
    );
    assert_eq!(
        next_of(&heard, "new-window"),
        json!({"ev": "new-window", "name": "build"})
    );
    channel.close();
}

#[test]
fn a_panes_long_press_renames_its_window_and_kills_it_after_asking() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:overview-pane-%3").unwrap();
    // Rename: the window's own name to start from; a new one renames.
    post(&mut app.0, &channel, &act_message("rename", "%3"));
    let prompt = next_of(&heard, "prompt");
    assert_eq!(prompt["title"], "Rename Window");
    assert_eq!(prompt["text"], "2.1.322");
    let id = prompt["id"].as_u64().unwrap();
    post(
        &mut app.0,
        &channel,
        &json!({"op": "prompted", "id": id, "text": "trainer"}).to_string(),
    );
    assert_eq!(
        next_of(&heard, "rename-window"),
        json!({"ev": "rename-window", "id": "@2", "name": "trainer"})
    );
    // Kill: asked on the action sheet, named; Cancel kills nothing.
    post(&mut app.0, &channel, &act_message("kill", "%6"));
    assert!(common::wait_dialog(&mut app), "the sheet never asked");
    let dialog = app.0.pending_dialog().cloned().expect("the sheet asks");
    assert!(dialog.sheet);
    assert_eq!(
        dialog.title.as_deref(),
        Some("Kill pane “node” in web? What runs in it stops.")
    );
    app.answer_dialog(DialogAnswer::Press("Cancel".into()))
        .unwrap();
    none_of(&heard, "kill-pane");
    post(&mut app.0, &channel, &act_message("kill", "%6"));
    app.answer_dialog(DialogAnswer::Press("Kill pane".into()))
        .unwrap();
    assert_eq!(
        next_of(&heard, "kill-pane"),
        json!({"ev": "kill-pane", "id": "%6"})
    );
    // New pane: its window split, at once.
    post(&mut app.0, &channel, &act_message("new-pane", "%6"));
    assert_eq!(
        next_of(&heard, "new-pane"),
        json!({"ev": "new-pane", "window": "@3"})
    );
    channel.close();
}

#[test]
fn a_cards_more_menu_speaks_the_multiplexers_words() {
    let (mut app, channel, heard, natives) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:overview-card-@2").unwrap();
    let menu = natives
        .find("moshpit/menu", "overview-window-menu-@2")
        .expect("trainer's ⋯");
    let titles: Vec<String> = natives
        .prop(menu, "items")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(titles, ["New Pane", "Rename window…", "Kill window…"]);
    emit(&mut app.0, menu, "select", json!("new-pane"));
    assert_eq!(
        next_of(&heard, "new-pane"),
        json!({"ev": "new-pane", "window": "@2"})
    );
    channel.close();

    // herdr's words: tabs, closed.
    let (mut app, channel, _heard, natives) = open_screen("herdr", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:overview-card-@2").unwrap();
    let menu = natives
        .find("moshpit/menu", "overview-window-menu-@2")
        .expect("trainer's ⋯");
    let titles: Vec<String> = natives
        .prop(menu, "items")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(titles, ["New Pane", "Rename tab…", "Close tab…"]);
    for text in ["5 tabs · 13 panes", "New tab"] {
        assert!(
            app.0.screen().shows(text),
            "{text}: {:?}",
            app.0.screen().texts()
        );
    }
    channel.close();
}

#[test]
fn past_four_panes_plus_shows_the_rest() {
    let (mut app, channel, _heard, _) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:overview-more-panes-@5").unwrap();
    assert_eq!(app.0.screen().count("id:overview-pane-%15").unwrap(), 0);
    app.0.press("id:overview-more-panes-@5").unwrap();
    app.0.wait_for("id:overview-pane-%15").unwrap();
    channel.close();
}

#[test]
fn without_a_live_tree_the_host_cannot_open_it() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "switcher", "tree": null, "hintSeen": true}).to_string(),
    );
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    none_of(&heard, "overview");
    assert!(!shows_overview(&app));
    channel.close();
}

/// B653: VoiceOver hears a pane's state in words: minutes said as
/// minutes (Home's rule; "13m" is the eye's), and "+3" as what it is.
#[test]
fn voiceover_hears_minutes_and_what_more_opens() {
    let (mut app, channel, _heard, _) = open_screen("tmux", "%2");
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        - 13.0 * 60.0
        - 30.0;
    let mut tree = work("tmux", "%2");
    tree["hooks"][0]["since"] = json!(since);
    post(
        &mut app.0,
        &channel,
        &json!({"op": "switcher", "tree": tree, "hintSeen": true}).to_string(),
    );
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:overview-pane-%1").unwrap();
    assert!(
        app.0.wait_for("label:codex, working 13 minutes").is_ok(),
        "the pane's words: {:?}",
        app.0.screen().texts()
    );
    assert!(app.0.wait_for("label:3 more").is_ok(), "what +3 opens");
    channel.close();
}

/// B654: the bar's session capsule is drawn 36 pt tall; the finger gets
/// the platform's 44.
#[test]
fn the_overviews_bar_is_big_enough_to_press() {
    let (mut app, channel, _heard, _) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:terminal-overview-session").unwrap();
    let target = app
        .0
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|n| n.test_id.as_deref() == Some("terminal-overview-session"))
        .unwrap()
        .bounds;
    assert!(
        target.size.height.0 >= 44.0 && target.size.width.0 >= 44.0,
        "{target:?}"
    );
    channel.close();
}

/// B655: the session's tree goes (a reconnect): the overview goes with
/// it, never left showing "0 windows · 0 panes" and a New window that
/// would land nowhere.
#[test]
fn a_tree_gone_takes_the_overview_with_it() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:terminal-overview").unwrap();
    assert_eq!(next_of(&heard, "overview")["open"], true);
    post(
        &mut app.0,
        &channel,
        &json!({"op": "switcher", "tree": null, "hintSeen": true}).to_string(),
    );
    assert_eq!(
        next_of(&heard, "overview"),
        json!({"ev": "overview", "open": false})
    );
    app.0.run_for(Duration::from_millis(400)).unwrap();
    assert!(!shows_overview(&app));
    channel.close();
}

/// B656: herdr's workspace has its own symbol (the breadcrumb's); the
/// overview's capsule wore tmux's session stack.
#[test]
fn the_overview_names_herdrs_workspace_with_its_symbol() {
    let (mut app, channel, _heard, natives) = open_screen("herdr", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:terminal-overview-session").unwrap();
    let names: Vec<String> = natives
        .all("moshpit/symbol")
        .into_iter()
        .filter_map(|n| natives.prop(n, "name"))
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    assert!(names.iter().any(|n| n == "rectangle.grid.2x2"), "{names:?}");
    channel.close();
}

#[test]
fn opening_asks_for_the_pictures_once_and_a_cell_shows_its_screens_end() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    // Every pane of the session, once, and which one shows (a mosh or herdr
    // session has a picture of that one only).
    let ask = next_of(&heard, "pictures");
    assert_eq!(ask["current"], "%2");
    let asked: Vec<&str> = ask["panes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert_eq!(asked.len(), 13, "{asked:?}");
    assert!(asked.contains(&"%1") && asked.contains(&"%15"), "{asked:?}");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "pictures", "panes": {
            "%1": ["dev@api:~/src/api$ make test", "ok  api 0.42s  ", "dev@api:~/src/api$ ", "", ""],
            "%6": ["", "  "]
        }})
        .to_string(),
    );
    app.0.wait_for("id:overview-picture-%1").unwrap();
    let picture = app
        .0
        .screen()
        .texts()
        .into_iter()
        .map(|t| t.to_string())
        .find(|t| t.contains("make test"))
        .expect("the picture's text");
    assert_eq!(
        picture, "dev@api:~/src/api$ make test\nok  api 0.42s\ndev@api:~/src/api$",
        "the screen's end, its blank tail cut"
    );
    // A blank screen and a pane the host had nothing for: no picture.
    for pane in ["%6", "%5"] {
        assert_eq!(
            app.0
                .screen()
                .count(&format!("id:overview-picture-{pane}"))
                .unwrap(),
            0,
            "{pane}"
        );
    }
    // The pane's cell still says what it is, the picture unsaid.
    assert!(app.0.screen().shows("codex"));
    // The tree again (a tick): nothing asked again.
    post(&mut app.0, &channel, &switcher("tmux", "%2"));
    none_of(&heard, "pictures");
    // A pane that comes while it is open: asked for alone.
    let mut tree = work("tmux", "%2");
    tree["panes"].as_array_mut().unwrap().push(
        json!({"id": "%20", "window": "@3", "index": 2, "command": "zsh", "path": "/home/dev/src/web"}),
    );
    tree["windows"][2]["paneCount"] = json!(3);
    post(
        &mut app.0,
        &channel,
        &json!({"op": "switcher", "tree": tree, "hintSeen": true}).to_string(),
    );
    assert_eq!(next_of(&heard, "pictures")["panes"], json!(["%20"]));
    // Closed: the pictures go; opened again, asked again.
    app.0.press("id:terminal-overview-done").unwrap();
    assert_eq!(next_of(&heard, "overview")["open"], false);
    app.0
        .wait_for_gone("id:terminal-overview", Duration::from_secs(2))
        .unwrap();
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:overview-pane-%1").unwrap();
    assert_eq!(app.0.screen().count("id:overview-picture-%1").unwrap(), 0);
    assert_eq!(
        next_of(&heard, "pictures")["panes"]
            .as_array()
            .unwrap()
            .len(),
        14
    );
    channel.close();
}

#[test]
fn the_shown_panes_cell_and_a_picked_one_tell_the_host_where_they_are() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    // The shown pane's cell: the live terminal shrinks into it.
    let anchor = next_of(&heard, "overview-anchor");
    assert_eq!(anchor["pane"], "%2");
    assert_eq!(anchor["picked"], false);
    app.0.wait_for("id:overview-pane-%6").unwrap();
    same_place(&anchor["rect"], bounds(&app, "overview-pane-%2"));
    // Laid out again (a tick of the tree), still there: not told again.
    post(&mut app.0, &channel, &switcher("tmux", "%2"));
    none_of(&heard, "overview-anchor");
    // A pane picked: where its cell is first (its terminal grows from
    // there), then the switch at once, then the overview goes.
    let cell = bounds(&app, "overview-pane-%6");
    app.0.press("id:overview-pane-%6").unwrap();
    let events: Vec<Value> = events_until(&heard, "overview")
        .into_iter()
        .filter(|e| {
            ["overview-anchor", "select-pane", "overview"].contains(&e["ev"].as_str().unwrap())
        })
        .collect();
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(events[0]["ev"], "overview-anchor");
    assert_eq!(events[0]["pane"], "%6");
    assert_eq!(events[0]["picked"], true);
    same_place(&events[0]["rect"], cell);
    assert_eq!(events[1], json!({"ev": "select-pane", "id": "%6"}));
    assert_eq!(events[2], json!({"ev": "overview", "open": false}));
    channel.close();
}

#[test]
fn a_shown_pane_behind_more_has_no_cell_to_shrink_into() {
    // %15 is the sixth of logs' panes: behind "+3".
    let (mut app, channel, heard, _) = open_screen("tmux", "%15");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:overview-more-panes-@5").unwrap();
    // The host fades the page over the terminal alone.
    none_of(&heard, "overview-anchor");
    channel.close();
}

#[test]
fn the_overview_and_every_view_around_it_have_a_size() {
    // An empty view around it (the fade's, the page hung outside it) hid
    // the whole overview from VoiceOver and XCUITest though it was drawn
    // (B659): every view from the screen down to each of the overview's
    // ids has a width and a height.
    let (mut app, channel, _heard, _) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
    app.0.wait_for("id:overview-new-window").unwrap();
    app.0.run_for(Duration::from_millis(400)).unwrap();
    let nodes = app.0.screen().layout_nodes();
    let mut checked = 0;
    for node in &nodes {
        let Some(id) = node.test_id.as_deref() else {
            continue;
        };
        if !(id.starts_with("terminal-overview") || id.starts_with("overview-")) {
            continue;
        }
        checked += 1;
        let mut at = node.parent;
        while let Some(around) = nodes.iter().find(|n| n.id == at) {
            let size = around.bounds.size;
            assert!(
                size.width.0 > 0.0 && size.height.0 > 0.0,
                "{id} is inside an empty view {:?} ({:?})",
                around.id,
                around.test_id
            );
            at = around.parent;
        }
    }
    assert!(checked > 10, "the overview's views: {checked}");
    channel.close();
}
