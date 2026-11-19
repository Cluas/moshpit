//! The pane page on a headless engine (`ui/src/terminal/overview.rs`): the
//! breadcrumb's pane segment opens it (the session's and the window's open
//! their quick lists), ⌘K toggles it, the host opens it for VoiceOver; it
//! shows the panes of the window in use, a card each with its state, then
//! New pane; a pane picked switches and closes it; its capsule opens the
//! windows' list; a pane's ⋯ and long press split, rename the window after
//! asking, kill after the action sheet.

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

/// The pane page's cards, in order: the panes' ids.
fn pane_cards(app: &common::App) -> Vec<String> {
    app.0
        .screen()
        .layout_nodes()
        .into_iter()
        .filter_map(|n| n.test_id.map(|t| t.to_string()))
        .filter_map(|t| t.strip_prefix("overview-pane-card-").map(str::to_owned))
        .collect()
}

fn open_page(app: &mut common::App, channel: &moshpit_ui::channel::Channel) {
    post(
        &mut app.0,
        channel,
        &json!({"op": "overview", "open": true}).to_string(),
    );
}

/// The pane's segment opens the pane page; the window's and the session's
/// open their quick lists (the host's picker), never the page.
#[test]
fn the_pane_crumb_opens_the_pane_page_and_the_others_their_lists() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    app.0.wait_for("id:terminal-crumb-pane").unwrap();
    app.0.press("id:terminal-crumb-pane").unwrap();
    app.0.wait_for("id:overview-pane-page-@2").unwrap();
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
    app.0
        .wait_for_gone("id:terminal-overview", Duration::from_secs(2))
        .unwrap();
    app.0.press("id:terminal-crumb-window").unwrap();
    assert_eq!(
        next_of(&heard, "crumb"),
        json!({"ev": "crumb", "crumb": "window"})
    );
    app.0.press("id:terminal-crumb-session").unwrap();
    assert_eq!(
        next_of(&heard, "crumb"),
        json!({"ev": "crumb", "crumb": "session"})
    );
    none_of(&heard, "overview");
    assert!(!shows_overview(&app));
    channel.close();
}

/// The page is the window in use: its panes in order with their states,
/// the current one selected, then New pane, which splits it at once. No
/// other window's pane, no New window.
#[test]
fn the_page_shows_the_windows_panes_and_ends_in_new_pane() {
    for mux in ["tmux", "herdr"] {
        let (mut app, channel, heard, _) = open_screen(mux, "%3");
        open_page(&mut app, &channel);
        app.0.wait_for("id:overview-pane-page-@2").unwrap();
        for text in [
            "trainer · 3 panes",
            "1 waiting",
            "claude",
            "python",
            "New Pane",
        ] {
            assert!(
                app.0.screen().shows(text),
                "{mux} {text}: {:?}",
                app.0.screen().texts()
            );
        }
        assert!(
            !app.0.screen().texts().iter().any(|t| t.contains("2.1.322")),
            "never a version string"
        );
        assert!(!app.0.screen().shows("New window") && !app.0.screen().shows("New tab"));
        assert_eq!(pane_cards(&app), ["%2", "%3", "%4"], "{mux}");
        assert_eq!(app.0.screen().count("id:overview-pane-%1").unwrap(), 0);
        assert!(app.0.screen().selected("id:overview-pane-%3").unwrap());
        assert!(!app.0.screen().selected("id:overview-pane-%2").unwrap());
        let last = app
            .0
            .screen()
            .layout_nodes()
            .into_iter()
            .filter_map(|n| n.test_id.map(|t| t.to_string()))
            .rfind(|t| t.starts_with("overview-pane-card-") || t == "overview-new-pane");
        assert_eq!(last.as_deref(), Some("overview-new-pane"), "{mux}");
        app.0.press("id:overview-new-pane").unwrap();
        assert_eq!(
            next_of(&heard, "new-pane"),
            json!({"ev": "new-pane", "window": "@2"}),
            "{mux}"
        );
        channel.close();
    }
}

#[test]
fn a_pane_picked_switches_to_it_and_the_page_goes() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%5");
    open_page(&mut app, &channel);
    app.0.wait_for("id:overview-pane-%6").unwrap();
    assert_eq!(next_of(&heard, "overview")["open"], true);
    assert_eq!(pane_cards(&app), ["%5", "%6"]);
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

/// Six panes: the page shows them all, none behind a "+N".
#[test]
fn a_window_of_many_panes_shows_them_all() {
    let (mut app, channel, _heard, _) = open_screen("tmux", "%12");
    open_page(&mut app, &channel);
    app.0.wait_for("id:overview-pane-%15").unwrap();
    assert_eq!(
        pane_cards(&app),
        (10..16).map(|i| format!("%{i}")).collect::<Vec<_>>()
    );
    assert!(!app.0.screen().shows("+3"));
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

/// The page's capsule names the window and opens the windows' quick list
/// (herdr: tabs), with its symbol; it takes a 44 pt touch (B654).
#[test]
fn the_window_capsule_opens_the_windows_list() {
    for (mux, symbol) in [("tmux", "macwindow"), ("herdr", "square.on.square")] {
        let (mut app, channel, heard, natives) = open_screen(mux, "%2");
        open_page(&mut app, &channel);
        app.0.wait_for("id:terminal-overview-window").unwrap();
        let target = app
            .0
            .screen()
            .layout_nodes()
            .into_iter()
            .find(|n| n.test_id.as_deref() == Some("terminal-overview-window"))
            .unwrap()
            .bounds;
        assert!(
            target.size.height.0 >= 44.0 && target.size.width.0 >= 44.0,
            "{target:?}"
        );
        let names: Vec<String> = natives
            .all("moshpit/symbol")
            .into_iter()
            .filter_map(|n| natives.prop(n, "name"))
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        assert!(names.iter().any(|n| n == symbol), "{mux}: {names:?}");
        app.0.press("id:terminal-overview-window").unwrap();
        assert_eq!(
            next_of(&heard, "crumb"),
            json!({"ev": "crumb", "crumb": "window"}),
            "{mux}"
        );
        channel.close();
    }
}

#[test]
fn a_panes_long_press_renames_its_window_and_kills_it_after_asking() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    open_page(&mut app, &channel);
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
    post(&mut app.0, &channel, &act_message("kill", "%3"));
    assert!(common::wait_dialog(&mut app), "the sheet never asked");
    let dialog = app.0.pending_dialog().cloned().expect("the sheet asks");
    assert!(dialog.sheet);
    assert_eq!(
        dialog.title.as_deref(),
        Some("Kill pane “python” in trainer? What runs in it stops.")
    );
    app.answer_dialog(DialogAnswer::Press("Cancel".into()))
        .unwrap();
    none_of(&heard, "kill-pane");
    post(&mut app.0, &channel, &act_message("kill", "%3"));
    app.answer_dialog(DialogAnswer::Press("Kill pane".into()))
        .unwrap();
    assert_eq!(
        next_of(&heard, "kill-pane"),
        json!({"ev": "kill-pane", "id": "%3"})
    );
    // New pane: its window split, at once.
    post(&mut app.0, &channel, &act_message("new-pane", "%4"));
    assert_eq!(
        next_of(&heard, "new-pane"),
        json!({"ev": "new-pane", "window": "@2"})
    );
    channel.close();
}

/// A pane's ⋯ speaks the multiplexer's words: tmux's windows killed,
/// herdr's tabs and panes closed.
#[test]
fn a_panes_more_menu_speaks_the_multiplexers_words() {
    for (mux, words) in [
        ("tmux", ["Open", "New Pane", "Rename window…", "Kill pane…"]),
        ("herdr", ["Open", "New Pane", "Rename tab…", "Close pane…"]),
    ] {
        let (mut app, channel, heard, natives) = open_screen(mux, "%2");
        open_page(&mut app, &channel);
        app.0.wait_for("id:overview-pane-card-%3").unwrap();
        let menu = natives
            .find("moshpit/menu", "overview-pane-menu-%3")
            .expect("python's ⋯");
        let titles: Vec<String> = natives
            .prop(menu, "items")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(titles, words, "{mux}");
        emit(&mut app.0, menu, "select", json!("new-pane"));
        assert_eq!(
            next_of(&heard, "new-pane"),
            json!({"ev": "new-pane", "window": "@2"})
        );
        channel.close();
    }
}

#[test]
fn without_a_live_tree_the_host_cannot_open_it() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "switcher", "tree": null, "hintSeen": true}).to_string(),
    );
    open_page(&mut app, &channel);
    none_of(&heard, "overview");
    assert!(!shows_overview(&app));
    channel.close();
}

/// B653: VoiceOver hears a pane's state in words: minutes said as
/// minutes (Home's rule; "13m" is the eye's).
#[test]
fn voiceover_hears_minutes() {
    let (mut app, channel, _heard, _) = open_screen("tmux", "%1");
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        - 13.0 * 60.0
        - 30.0;
    let mut tree = work("tmux", "%1");
    tree["hooks"][0]["since"] = json!(since);
    post(
        &mut app.0,
        &channel,
        &json!({"op": "switcher", "tree": tree, "hintSeen": true}).to_string(),
    );
    open_page(&mut app, &channel);
    app.0.wait_for("id:overview-pane-%1").unwrap();
    assert!(
        app.0
            .wait_for("label:codex, working 13 minutes, current")
            .is_ok(),
        "the pane's words: {:?}",
        app.0.screen().texts()
    );
    channel.close();
}

/// B655: the session's tree goes (a reconnect): the page goes with it,
/// never left showing no panes and a New pane that would land nowhere.
#[test]
fn a_tree_gone_takes_the_page_with_it() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    open_page(&mut app, &channel);
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

#[test]
fn opening_asks_for_the_pictures_once_and_a_cell_shows_its_screens_end() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    open_page(&mut app, &channel);
    // The window's panes, once, and which one shows (a mosh or herdr
    // session has a picture of that one only).
    let ask = next_of(&heard, "pictures");
    assert_eq!(ask["current"], "%2");
    assert_eq!(ask["panes"], json!(["%2", "%3", "%4"]));
    post(
        &mut app.0,
        &channel,
        &json!({"op": "pictures", "panes": {
            "%3": ["dev@gpu:~/src/trainer$ make test", "ok  trainer 0.42s  ", "dev@gpu:~/src/trainer$ ", "", ""],
            "%4": ["", "  "]
        }})
        .to_string(),
    );
    app.0.wait_for("id:overview-picture-%3").unwrap();
    let picture = app
        .0
        .screen()
        .texts()
        .into_iter()
        .map(|t| t.to_string())
        .find(|t| t.contains("make test"))
        .expect("the picture's text");
    assert_eq!(
        picture, "dev@gpu:~/src/trainer$ make test\nok  trainer 0.42s\ndev@gpu:~/src/trainer$",
        "the screen's end, its blank tail cut"
    );
    // A blank screen and a pane the host had nothing for: no picture.
    for pane in ["%4", "%2"] {
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
    assert!(app.0.screen().shows("zsh"));
    // The tree again (a tick): nothing asked again.
    post(&mut app.0, &channel, &switcher("tmux", "%2"));
    none_of(&heard, "pictures");
    // A pane that comes while it is open: asked for alone.
    let mut tree = work("tmux", "%2");
    tree["panes"].as_array_mut().unwrap().push(
        json!({"id": "%20", "window": "@2", "index": 3, "command": "zsh", "path": "/home/dev/src/trainer"}),
    );
    tree["windows"][1]["paneCount"] = json!(4);
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
    open_page(&mut app, &channel);
    app.0.wait_for("id:overview-pane-%3").unwrap();
    assert_eq!(app.0.screen().count("id:overview-picture-%3").unwrap(), 0);
    assert_eq!(
        next_of(&heard, "pictures")["panes"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    channel.close();
}

#[test]
fn the_shown_panes_cell_and_a_picked_one_tell_the_host_where_they_are() {
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    open_page(&mut app, &channel);
    // The shown pane's cell: the live terminal shrinks into it.
    let anchor = next_of(&heard, "overview-anchor");
    assert_eq!(anchor["pane"], "%2");
    assert_eq!(anchor["picked"], false);
    app.0.wait_for("id:overview-pane-%4").unwrap();
    same_place(&anchor["rect"], bounds(&app, "overview-pane-%2"));
    // Laid out again (a tick of the tree), still there: not told again.
    post(&mut app.0, &channel, &switcher("tmux", "%2"));
    none_of(&heard, "overview-anchor");
    // A pane picked: where its cell is first (its terminal grows from
    // there), then the switch at once, then the page goes.
    app.0.run_for(Duration::from_millis(300)).unwrap();
    let cell = bounds(&app, "overview-pane-%4");
    app.0.press("id:overview-pane-%4").unwrap();
    let events: Vec<Value> = events_until(&heard, "overview")
        .into_iter()
        .filter(|e| {
            ["overview-anchor", "select-pane", "overview"].contains(&e["ev"].as_str().unwrap())
        })
        .collect();
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(events[0]["ev"], "overview-anchor");
    assert_eq!(events[0]["pane"], "%4");
    assert_eq!(events[0]["picked"], true);
    same_place(&events[0]["rect"], cell);
    assert_eq!(events[1], json!({"ev": "select-pane", "id": "%4"}));
    assert_eq!(events[2], json!({"ev": "overview", "open": false}));
    channel.close();
}

#[test]
fn the_page_and_every_view_around_it_have_a_size() {
    // An empty view around it (the fade's, the page hung outside it) hid
    // the whole page from VoiceOver and XCUITest though it was drawn
    // (B659): every view from the screen down to each of the page's ids
    // has a width and a height.
    let (mut app, channel, _heard, _) = open_screen("tmux", "%2");
    open_page(&mut app, &channel);
    app.0.wait_for("id:overview-new-pane").unwrap();
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
    assert!(checked > 8, "the page's views: {checked}");
    channel.close();
}

#[test]
fn a_panes_snapshot_replaces_the_fragments_its_terminal_had() {
    // A pane never shown has only what tmux sent since the attach in its
    // terminal (a spinner's line); its snapshot, when the core sends one,
    // is its whole screen and takes the card (B751). The core sends none
    // for a pane whose live screen came.
    let (mut app, channel, heard, _) = open_screen("tmux", "%2");
    open_page(&mut app, &channel);
    next_of(&heard, "pictures");
    post(
        &mut app.0,
        &channel,
        &json!({"op": "pictures", "panes": {"%3": ["", "", "⠋ building"]}}).to_string(),
    );
    app.0.wait_for("id:overview-picture-%3").unwrap();
    post(
        &mut app.0,
        &channel,
        &json!({"op": "pictures", "panes": {"%3": ["dev@gpu:~/src/trainer$ python train.py", "epoch 1", "⠋ building"]}})
            .to_string(),
    );
    app.0.run_for(Duration::from_millis(200)).unwrap();
    assert!(
        app.0
            .screen()
            .texts()
            .contains(&"dev@gpu:~/src/trainer$ python train.py\nepoch 1\n⠋ building"),
        "{:?}",
        app.0.screen().texts()
    );
    channel.close();
}
