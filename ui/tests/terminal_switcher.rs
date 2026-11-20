//! The terminal's sideways swipe on a headless engine (`ui/src/terminal/
//! switcher.rs`): the page tells the host the panes either side by the
//! session's order, window by window and never wrapping around, quiet
//! while a cover is up or the session has one pane; the strip under the
//! terminal names the pane a swipe would go to; the one-time hint shows
//! once and asks the host to nudge; herdr's tabs walk the same way.

mod common;

use std::{sync::mpsc, time::Duration};

use common::{hear, post, start_observing};
use moshpit_ui::screen::{Params, Screen};
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

/// The last event named `ev` told within 300 ms of quiet, if any.
fn last_of(rx: &mpsc::Receiver<String>, ev: &str) -> Option<Value> {
    let mut last = None;
    while let Ok(json) = rx.recv_timeout(Duration::from_millis(300)) {
        let value: Value = serde_json::from_str(&json).unwrap();
        if value["ev"] == ev {
            last = Some(value);
        }
    }
    last
}

/// No event named `ev` comes (the others pass).
fn none_of(rx: &mpsc::Receiver<String>, ev: &str) {
    while let Ok(json) = rx.recv_timeout(Duration::from_millis(300)) {
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_ne!(value["ev"], ev, "told {value}");
    }
}

fn chrome(mux: &str, cover: &str) -> Value {
    json!({
        "op": "chrome",
        "name": "gpu-01", "user": "dev", "host": "gpu-01.example.com", "port": 22,
        "mosh": false, "mux": mux, "pill": "ssh", "state": "live",
        "cover": {"kind": cover, "state": "live"},
        "content": "terminal"
    })
}

/// The mockup's session `work`: api (codex working), trainer (claude
/// waiting, python, zsh), web (zsh, node), docs (claude done), and a
/// session `play` the swipe never reaches. `active`: the pane shown.
fn work(mux: &str, active: &str) -> Value {
    json!({
        "mux": mux, "attached": true,
        "sessions": [{"id": "$0", "name": "work"}, {"id": "$1", "name": "play"}],
        "windows": [
            {"id": "@1", "session": "$0", "index": 1, "name": "codex", "paneCount": 1},
            {"id": "@2", "session": "$0", "index": 2, "name": "2.1.322", "paneCount": 3},
            {"id": "@3", "session": "$0", "index": 3, "name": "zsh", "paneCount": 2},
            {"id": "@4", "session": "$0", "index": 4, "name": "docs", "paneCount": 1},
            {"id": "@9", "session": "$1", "index": 1, "name": "zsh", "paneCount": 1}
        ],
        "panes": [
            {"id": "%1", "window": "@1", "index": 0, "command": "codex", "path": "/home/dev/src/api"},
            {"id": "%2", "window": "@2", "index": 0, "command": "2.1.322", "path": "/home/dev/src/trainer"},
            {"id": "%3", "window": "@2", "index": 1, "command": "python", "path": "/home/dev/src/trainer"},
            {"id": "%4", "window": "@2", "index": 2, "command": "zsh", "path": "/home/dev/src/trainer"},
            {"id": "%5", "window": "@3", "index": 0, "command": "zsh", "path": "/home/dev/src/web"},
            {"id": "%6", "window": "@3", "index": 1, "command": "node", "path": "/home/dev/src/web"},
            {"id": "%7", "window": "@4", "index": 0, "command": "2.1.322", "path": "/home/dev/src/docs"},
            {"id": "%9", "window": "@9", "index": 0, "command": "zsh", "path": "/home/dev/play"}
        ],
        "hooks": [
            {"pane": "%1", "state": "working", "agent": "codex"},
            {"pane": "%2", "state": "attention", "agent": "claude"},
            {"pane": "%7", "state": "done"}
        ],
        "worktrees": [],
        "activeSession": "$0", "activePane": active
    })
}

fn switcher(tree: Value, hint_seen: bool) -> String {
    json!({"op": "switcher", "tree": tree, "hintSeen": hint_seen}).to_string()
}

fn open() -> (
    common::App,
    std::sync::Arc<moshpit_ui::channel::Channel>,
    mpsc::Receiver<String>,
) {
    let (app, channel, _) = start_observing(Screen::Terminal, Params::default(), "terminal-screen");
    let heard = hear(&channel);
    (app, channel, heard)
}

#[test]
fn the_host_hears_the_panes_either_side_without_wrapping() {
    let (mut app, channel, heard) = open();
    post(&mut app.0, &channel, &chrome("tmux", "none").to_string());
    post(&mut app.0, &channel, &switcher(work("tmux", "%1"), true));
    assert_eq!(
        next_of(&heard, "neighbours"),
        json!({"ev": "neighbours", "previous": null, "next": "%2"}),
        "the first pane: nothing before it, trainer's first pane after"
    );
    // Across window edges: trainer's last pane sits between its python and
    // web's first pane.
    post(&mut app.0, &channel, &switcher(work("tmux", "%4"), true));
    assert_eq!(
        next_of(&heard, "neighbours"),
        json!({"ev": "neighbours", "previous": "%3", "next": "%5"})
    );
    // The last pane: nothing after it (tmux's next-pane wrapped to api).
    post(&mut app.0, &channel, &switcher(work("tmux", "%7"), true));
    assert_eq!(
        next_of(&heard, "neighbours"),
        json!({"ev": "neighbours", "previous": "%6", "next": null})
    );
    channel.close();
}

#[test]
fn a_cover_or_a_lone_pane_leaves_nothing_to_swipe_to() {
    let (mut app, channel, heard) = open();
    post(&mut app.0, &channel, &chrome("tmux", "none").to_string());
    post(&mut app.0, &channel, &switcher(work("tmux", "%4"), true));
    assert_eq!(next_of(&heard, "neighbours")["next"], "%5");
    // A reconnect's veil over the kept frame: no swipe through it.
    post(&mut app.0, &channel, &chrome("tmux", "veil").to_string());
    assert_eq!(
        next_of(&heard, "neighbours"),
        json!({"ev": "neighbours", "previous": null, "next": null})
    );
    // One pane in the session.
    post(&mut app.0, &channel, &chrome("tmux", "none").to_string());
    let mut lone = work("tmux", "%1");
    lone["windows"] =
        json!([{"id": "@1", "session": "$0", "index": 1, "name": "codex", "paneCount": 1}]);
    lone["panes"] = json!([{"id": "%1", "window": "@1", "index": 0, "command": "codex"}]);
    post(&mut app.0, &channel, &switcher(lone, false));
    assert_eq!(
        last_of(&heard, "neighbours"),
        Some(json!({"ev": "neighbours", "previous": null, "next": null})),
        "the lifted cover told the old tree's; the lone pane's says nothing either side"
    );
    assert!(
        !app.0.screen().shows("Swipe sideways to switch panes"),
        "no hint where there is nowhere to go"
    );
    channel.close();
}

#[test]
fn the_strip_under_the_terminal_names_where_a_swipe_goes() {
    let (mut app, channel, _heard) = open();
    post(&mut app.0, &channel, &chrome("tmux", "none").to_string());
    post(&mut app.0, &channel, &switcher(work("tmux", "%3"), true));
    app.0.wait_for("id:terminal-backstage-next").unwrap();
    // Before: trainer's claude, waiting, second of seven; after: trainer's
    // zsh, fourth. The window is its folder, never the version tmux named
    // it after.
    for text in ["2 / 7", "4 / 7", "trainer", "claude", "zsh"] {
        assert!(
            app.0.screen().shows(text),
            "{text}: {:?}",
            app.0.screen().texts()
        );
    }
    assert!(
        app.0.screen().shows("Needs you"),
        "{:?}",
        app.0.screen().texts()
    );
    assert!(!app.0.screen().shows("2.1.322"));
    channel.close();
}

#[test]
fn the_strip_shows_the_screens_either_side_the_host_has() {
    let (mut app, channel, heard) = open();
    post(&mut app.0, &channel, &chrome("tmux", "none").to_string());
    post(&mut app.0, &channel, &switcher(work("tmux", "%3"), true));
    assert_eq!(
        next_of(&heard, "swipe-pictures"),
        json!({"ev": "swipe-pictures", "panes": ["%2", "%4"]}),
        "asked for the panes either side as soon as they are known"
    );
    // A tmux attach keeps each pane's terminal: both screens come.
    post(
        &mut app.0,
        &channel,
        &json!({"op": "swipe-pictures", "panes": {
            "%2": ["dev@trainer:~$ claude", "", "> Allow edit to train.py?", "", ""],
            "%4": ["dev@trainer:~/src/trainer$ ls", "data  train.py"]
        }})
        .to_string(),
    );
    app.0
        .wait_for("id:terminal-backstage-previous-thumb")
        .unwrap();
    app.0.wait_for("id:terminal-backstage-next-thumb").unwrap();
    assert!(
        app.0
            .screen()
            .texts()
            .contains(&"dev@trainer:~$ claude\n\n> Allow edit to train.py?"),
        "the screen's end, trailing blanks cut: {:?}",
        app.0.screen().texts()
    );
    // Each swipe asks again for what they show now.
    post(
        &mut app.0,
        &channel,
        &json!({"op": "swipe", "active": true}).to_string(),
    );
    assert_eq!(
        next_of(&heard, "swipe-pictures")["panes"],
        json!(["%2", "%4"])
    );
    // A mosh session keeps only the shown pane's terminal: the host answers
    // with none at once, then each snapshot as it comes, on its own. One
    // pane's answer keeps the other's picture.
    post(
        &mut app.0,
        &channel,
        &json!({"op": "swipe-pictures", "panes": {}}).to_string(),
    );
    post(
        &mut app.0,
        &channel,
        &json!({"op": "swipe-pictures", "panes": {"%4": ["dev@trainer:~/src/trainer$ git status"]}})
            .to_string(),
    );
    app.0.run_for(Duration::from_millis(200)).unwrap();
    assert!(
        app.0
            .screen()
            .texts()
            .contains(&"dev@trainer:~/src/trainer$ git status"),
        "{:?}",
        app.0.screen().texts()
    );
    app.0
        .wait_for("id:terminal-backstage-previous-thumb")
        .unwrap();
    // Moved on: the panes no longer either side lose their pictures, and
    // the strip is words until the new ones' come.
    post(&mut app.0, &channel, &switcher(work("tmux", "%6"), true));
    assert_eq!(
        next_of(&heard, "swipe-pictures")["panes"],
        json!(["%5", "%7"])
    );
    app.0.run_for(Duration::from_millis(200)).unwrap();
    for thumb in [
        "id:terminal-backstage-previous-thumb",
        "id:terminal-backstage-next-thumb",
    ] {
        assert!(app.0.screen().id(thumb).is_err(), "{thumb}");
    }
    channel.close();
}

#[test]
fn the_hint_shows_once_and_asks_for_a_nudge() {
    let (mut app, channel, heard) = open();
    post(&mut app.0, &channel, &chrome("tmux", "none").to_string());
    post(&mut app.0, &channel, &switcher(work("tmux", "%1"), false));
    assert_eq!(
        next_of(&heard, "swipe-hint"),
        json!({"ev": "swipe-hint", "forward": true})
    );
    app.0.wait_for("id:terminal-swipe-hint").unwrap();
    assert!(app.0.screen().shows("Swipe sideways to switch panes"));
    // The host remembers it: posts say seen from now on, and another pane
    // shown brings no second hint.
    post(&mut app.0, &channel, &switcher(work("tmux", "%2"), true));
    none_of(&heard, "swipe-hint");
    // A first swipe takes it away.
    post(
        &mut app.0,
        &channel,
        &json!({"op": "swipe", "active": true}).to_string(),
    );
    app.0.run_for(Duration::from_millis(600)).unwrap();
    assert!(!app.0.screen().shows("Swipe sideways to switch panes"));
    channel.close();

    // Seen on this device before: never.
    let (mut app, channel, heard) = open();
    post(&mut app.0, &channel, &chrome("tmux", "none").to_string());
    post(&mut app.0, &channel, &switcher(work("tmux", "%1"), true));
    none_of(&heard, "swipe-hint");
    assert!(!app.0.screen().shows("Swipe sideways to switch panes"));
    channel.close();
}

#[test]
fn the_hint_leans_back_from_the_last_pane() {
    let (mut app, channel, heard) = open();
    post(&mut app.0, &channel, &chrome("tmux", "none").to_string());
    post(&mut app.0, &channel, &switcher(work("tmux", "%7"), false));
    assert_eq!(
        next_of(&heard, "swipe-hint"),
        json!({"ev": "swipe-hint", "forward": false}),
        "nothing after the last pane: the nudge goes back"
    );
    channel.close();
}

#[test]
fn herdr_tabs_are_swiped_through_too() {
    let (mut app, channel, heard) = open();
    post(&mut app.0, &channel, &chrome("herdr", "none").to_string());
    post(&mut app.0, &channel, &switcher(work("herdr", "%5"), true));
    assert_eq!(
        next_of(&heard, "neighbours"),
        json!({"ev": "neighbours", "previous": "%4", "next": "%6"}),
        "herdr's frame channel gets the same neighbours to retarget to"
    );
    channel.close();
}

#[test]
fn the_dots_say_where_the_pane_is() {
    let (mut app, channel, _heard) = open();
    post(&mut app.0, &channel, &chrome("tmux", "none").to_string());
    post(&mut app.0, &channel, &switcher(work("tmux", "%2"), true));
    app.0.wait_for("id:terminal-pane-dots").unwrap();
    assert!(
        app.0.wait_for("label:Pane 2 of 7, trainer").is_ok(),
        "the dots' words"
    );
    // They go a moment after the pane changed, and come back for a swipe.
    app.0.run_for(Duration::from_millis(1800)).unwrap();
    assert_eq!(
        app.0.screen().count("id:terminal-pane-dots").unwrap(),
        0,
        "gone 1.2 s after the pane changed"
    );
    post(
        &mut app.0,
        &channel,
        &json!({"op": "swipe", "active": true}).to_string(),
    );
    app.0.wait_for("id:terminal-pane-dots").unwrap();
    channel.close();
}
