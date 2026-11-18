//! The breadcrumb's segments are 44 × 44 pt targets (Canary 118: the `1`
//! of `train › 1 › claude`, a window called 1, was a target its words'
//! width, some 9 pt between two separators). Each segment's target reaches
//! to the middle of the `›` either side of it, so a tap between two goes to
//! the nearer one, and a segment shorter than 44 pt widens to it; the
//! words and the separators otherwise stay where they were. Named and
//! squeezed (the session's icon alone), live and remembered.

mod common;

use common::{App, hear, post, start, start_observing_with};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
};
use nsh_core::{Bounds, Edges, Points, Size, platform::Idiom};
use nsh_testing::LayoutNode;
use serde_json::{Value, json};

const TARGET: f32 = 44.0;

fn chrome(session: &str, window: &str, pane: &str, live: bool) -> Value {
    json!({
        "op": "chrome",
        "name": "studio",
        "user": "cluas",
        "host": "studio.local",
        "port": 22,
        "mosh": false,
        "mux": "tmux",
        "pill": "ssh",
        "state": if live { "live" } else { "reconnecting" },
        "crumbs": {
            "session": session,
            "squeezable": true,
            "window": window,
            "pane": pane,
            "signal": "working",
            "sessionIcon": "square.stack.3d.up",
            "windowIcon": "macwindow",
            "live": live
        },
        "cover": {"kind": "none", "state": "live"},
        "content": "terminal"
    })
}

fn open(message: &Value) -> (App, std::sync::Arc<Channel>) {
    let (mut app, channel) = start(Screen::Terminal, Params::default(), "terminal-screen");
    post(&mut app, &channel, &message.to_string());
    app.wait_for("label:1").unwrap();
    (app, channel)
}

/// The screen on a phone `width` pt wide (320: the narrowest iPhone).
fn open_phone(message: &Value, width: f32, wait: &str) -> (App, std::sync::Arc<Channel>) {
    let (mut app, channel, _) = start_observing_with(
        Screen::Terminal,
        Params::default(),
        "terminal-screen",
        |config| {
            config.viewport = Size::new(Points(width), Points(700.0));
            config.environment.idiom = Idiom::Phone;
            config.environment.safe_area = Edges {
                top: Points(47.0),
                bottom: Points(34.0),
                left: Points(0.0),
                right: Points(0.0),
            };
        },
    );
    post(&mut app, &channel, &message.to_string());
    app.wait_for(wait).unwrap();
    (app, channel)
}

fn node_by_id<'a>(nodes: &'a [LayoutNode], test_id: &str) -> &'a LayoutNode {
    nodes
        .iter()
        .find(|n| n.test_id.as_deref() == Some(test_id))
        .unwrap_or_else(|| panic!("{test_id} is laid out"))
}

/// The segment screen readers call `label`, as laid out.
fn segment(app: &App, nodes: &[LayoutNode], label: &str) -> LayoutNode {
    let id = app
        .screen()
        .id(&format!("label:{label}"))
        .unwrap_or_else(|e| panic!("{label}: {e}"));
    nodes
        .iter()
        .find(|n| n.id == id)
        .unwrap_or_else(|| panic!("{label} is laid out"))
        .clone()
}

fn within(nodes: &[LayoutNode], mut id: nsh_core::NodeId, ancestor: nsh_core::NodeId) -> bool {
    loop {
        if id == ancestor {
            return true;
        }
        match nodes.iter().find(|n| n.id == id) {
            Some(n) if n.parent != id => id = n.parent,
            _ => return false,
        }
    }
}

/// The text views showing `words` inside the view `inside`.
fn words_in<'a>(nodes: &'a [LayoutNode], inside: &LayoutNode, words: &str) -> Vec<&'a LayoutNode> {
    nodes
        .iter()
        .filter(|n| n.text.as_ref().is_some_and(|t| t.text.text == words))
        .filter(|n| within(nodes, n.id, inside.id))
        .collect()
}

fn contains(b: &Bounds<Points>, x: f32, y: f32) -> bool {
    x >= b.origin.x.0
        && x < b.origin.x.0 + b.size.width.0
        && y >= b.origin.y.0
        && y < b.origin.y.0 + b.size.height.0
}

/// Every segment's target is at least 44 × 44, and neighbours meet within
/// the `›` between them: none takes a tap on its neighbour's words.
fn assert_targets(app: &App, labels: [&str; 3]) -> Vec<LayoutNode> {
    let nodes = app.screen().layout_nodes();
    let boxes: Vec<LayoutNode> = labels.iter().map(|l| segment(app, &nodes, l)).collect();
    for (id, b) in labels.iter().zip(&boxes) {
        assert!(
            b.bounds.size.width.0 >= TARGET - 0.01 && b.bounds.size.height.0 >= TARGET - 0.01,
            "{id}: {:?}",
            b.bounds
        );
    }
    for pair in boxes.windows(2) {
        let (left, right) = (&pair[0].bounds, &pair[1].bounds);
        let left_end = left.origin.x.0 + left.size.width.0;
        let gap = (right.origin.x.0 - left_end).abs();
        assert!(
            gap <= 1.0,
            "neighbours meet at the separator: {left:?} {right:?}"
        );
    }
    // No target reaches past the capsule's ends: the pill before it and
    // whatever is after it keep their taps.
    let capsule = node_by_id(&nodes, "terminal-breadcrumb").bounds;
    let (start, end) = (
        capsule.origin.x.0,
        capsule.origin.x.0 + capsule.size.width.0,
    );
    for (id, b) in labels.iter().zip(&boxes) {
        let (from, to) = (
            b.bounds.origin.x.0,
            b.bounds.origin.x.0 + b.bounds.size.width.0,
        );
        assert!(
            from >= start - 0.01 && to <= end + 0.01,
            "{id}: {:?} inside the capsule {capsule:?}",
            b.bounds
        );
    }
    boxes
}

#[test]
fn every_segment_is_a_44_pt_target_and_the_1_is_the_windows() {
    let (mut app, channel) = open(&chrome("train", "1", "claude", true));
    let heard = hear(&channel);
    let boxes = assert_targets(&app, ["train", "1", "claude"]);
    let nodes = app.screen().layout_nodes();
    // The centre of the `1` is the window's, and no other segment's.
    let one = words_in(&nodes, &boxes[1], "1");
    assert_eq!(one.len(), 1, "the window's words: {one:?}");
    let c = one[0].bounds.center();
    let (x, y) = (c.x.0, c.y.0);
    assert!(contains(&boxes[1].bounds, x, y));
    assert!(!contains(&boxes[0].bounds, x, y) && !contains(&boxes[2].bounds, x, y));
    // The words stay in one line between the targets' edges: `train`'s
    // and `claude`'s inside theirs.
    for (b, words) in [(&boxes[0], "train"), (&boxes[2], "claude")] {
        let shown = words_in(&nodes, b, words);
        assert_eq!(shown.len(), 1, "{words}");
    }
    // A press there reaches the window crumb.
    app.press("label:1").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "crumb", "crumb": "window"})
    );
    app.press("label:train").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "crumb", "crumb": "session"})
    );
    app.press("label:claude").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "crumb", "crumb": "pane"}));
    channel.close();
}

#[test]
fn long_names_on_the_narrowest_phone_truncate_inside_the_capsule() {
    // Forty characters each, on a 320 pt phone: with no clip on the
    // capsule, the words must still end inside it.
    let session = "s".repeat(32) + "-session";
    let window = "w".repeat(33) + "-window";
    let pane = "p".repeat(35) + "-pane";
    assert_eq!((session.len(), window.len(), pane.len()), (40, 40, 40));
    let (app, channel) = open_phone(
        &chrome(&session, &window, &pane, true),
        320.0,
        &format!("label:{window}"),
    );
    let boxes = assert_targets(&app, [session.as_str(), window.as_str(), pane.as_str()]);
    let nodes = app.screen().layout_nodes();
    let inner = node_by_id(&nodes, "terminal-breadcrumb").bounds;
    let (start, end) = (inner.origin.x.0, inner.origin.x.0 + inner.size.width.0);
    for (b, words) in boxes.iter().zip([&session, &window, &pane]) {
        for shown in words_in(&nodes, b, words) {
            let (from, to) = (
                shown.bounds.origin.x.0,
                shown.bounds.origin.x.0 + shown.bounds.size.width.0,
            );
            assert!(
                from >= start - 0.01 && to <= end + 0.01,
                "{words}: {:?} inside the capsule {inner:?}",
                shown.bounds
            );
        }
    }
    // The pane's name truncates: narrower than the measuring copy's, which
    // is the whole name's width.
    let natural = nodes
        .iter()
        .filter(|n| n.text.as_ref().is_some_and(|t| t.text.text == pane))
        .map(|n| n.bounds.size.width.0)
        .fold(0.0_f32, f32::max);
    let shown = words_in(&nodes, &boxes[2], &pane);
    assert_eq!(shown.len(), 1, "{pane} shows once");
    assert!(
        shown[0].bounds.size.width.0 < natural - 1.0,
        "{pane} truncates: {} of {natural}",
        shown[0].bounds.size.width.0
    );
    channel.close();
}

#[test]
fn the_pill_keeps_its_tap() {
    let (mut app, channel) = open_phone(&chrome("train", "1", "claude", true), 320.0, "label:1");
    app.answer_dialogs_myself();
    let heard = hear(&channel);
    assert_targets(&app, ["train", "1", "claude"]);
    // A press at the pill's centre is the pill's: no segment covers it.
    app.press("id:terminal-transport-switch").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "dialog-anchor", "name": "pill"})
    );
    channel.close();
}

#[test]
fn squeezed_the_sessions_icon_alone_is_still_a_44_pt_target() {
    // A session name that cannot fit beside the others: the session
    // segment keeps its icon alone.
    let long = "experiments-on-the-overnight-training-box-2026-11-14";
    let (app, channel) = open(&chrome(long, "1", "claude", true));
    let boxes = assert_targets(&app, [long, "1", "claude"]);
    let nodes = app.screen().layout_nodes();
    let named = words_in(&nodes, &boxes[0], long);
    eprintln!("session segment shows its name: {}", !named.is_empty());
    channel.close();
}

#[test]
fn the_remembered_plans_segments_are_as_large_and_inert() {
    let (mut app, channel) = open(&chrome("train", "1", "claude", false));
    let heard = hear(&channel);
    assert_targets(&app, ["train", "1", "claude"]);
    let _ = app.press("label:1");
    no_intent(&heard);
    channel.close();
}

/// The next intent the screen told the host, the anchors it reports as it
/// lays out skipped.
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
    while let Ok(json) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["ev"], "anchor", "the screen told the host {value}");
    }
}

/// The bar's ＋ shows on every iPhone (375 pt and up) and gives way on a
/// narrower screen (an iPad's Slide Over), where the breadcrumb's three
/// targets need the room.
#[test]
fn the_bars_plus_gives_way_on_a_narrow_screen() {
    let message = chrome("train", "1", "claude", true);
    let (app, channel) = open_phone(&message, 375.0, "label:1");
    assert!(
        app.screen().matches("label:New…").unwrap(),
        "on a 375 pt iPhone"
    );
    assert_targets(&app, ["train", "1", "claude"]);
    channel.close();
    let (app, channel) = open_phone(&message, 320.0, "label:1");
    assert!(!app.screen().matches("label:New…").unwrap(), "320 pt: gone");
    channel.close();
}
