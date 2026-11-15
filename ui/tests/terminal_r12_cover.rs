//! R12's acceptance for the terminal's covers and banners
//! (`docs/nsh/UI-DESIGN.md` §6.3, §8), headless: a reconnect shows only
//! the veil's capsule, never a full-screen cover (there is no poster any
//! more, B657); the banner area
//! holds one banner at a time, 8 pt under the top bar, 16 pt in from the
//! sides; the multiplexer's empty state takes presses over 44 pt.

mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use common::{post, start_observing};
use moshpit_ui::screen::{Params, Screen};
use nsh_core::{Bounds, EdgeInsets, NodeId, Points};
use nsh_testing::Headless;
use serde_json::{Value, json};

fn chrome(changes: Value) -> Value {
    let mut base = json!({
        "op": "chrome",
        "name": "studio",
        "user": "cluas",
        "host": "studio.local",
        "port": 22,
        "mosh": true,
        "mux": "tmux",
        "pill": "mosh",
        "state": "live",
        "cover": {"kind": "none", "state": "live"},
        "content": "terminal"
    });
    for (key, value) in changes.as_object().unwrap() {
        base[key] = value.clone();
    }
    base
}

fn show(app: &mut Headless, channel: &moshpit_ui::channel::Channel, message: &Value) {
    post(app, channel, &message.to_string());
}

/// The bounds of the view whose test id is `id`, if it is mounted.
fn bounds(app: &Headless, id: &str) -> Option<(NodeId, Bounds<Points>)> {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(id))
        .map(|node| (node.id, node.bounds))
}

/// How many of `ids` are mounted.
fn mounted(app: &Headless, ids: &[&str]) -> usize {
    let nodes = app.screen().layout_nodes();
    ids.iter()
        .filter(|id| nodes.iter().any(|n| n.test_id.as_deref() == Some(**id)))
        .count()
}

const BANNERS: [&str; 3] = ["mosh-returnpath-banner", "host-banner", "roam-banner"];

#[test]
fn a_reconnect_shows_the_capsule_and_never_the_poster() {
    let (mut app, channel, natives) =
        start_observing(Screen::Terminal, Params::default(), "terminal-screen");
    show(
        &mut app,
        &channel,
        &chrome(
            json!({"state": "reconnecting", "cover": {"kind": "veil", "state": "reconnecting"}}),
        ),
    );
    app.wait_for("id:terminal-reconnect-capsule").unwrap();
    assert!(
        app.screen().shows("Reconnecting…"),
        "{:?}",
        app.screen().texts()
    );
    // No poster, nothing of it: not its view, its mark, its grid.
    assert_eq!(mounted(&app, &["terminal-connecting"]), 0);
    assert_eq!(
        natives.count("moshpit/pane-loader"),
        0,
        "the poster's mark under a veil"
    );
    assert_eq!(
        natives.count("moshpit/signal-grid"),
        0,
        "the poster's grid under a veil"
    );
    // The kept frame stays in its slot under the veil.
    assert_eq!(natives.count("moshpit/terminal"), 1);
    // The capsule: 32 tall, centered over the pane, near its top.
    let (_, pane) = bounds(&app, "terminal-pane-area").unwrap();
    let (_, capsule) = bounds(&app, "terminal-reconnect-capsule").unwrap();
    assert_eq!(capsule.size.height.0, 32.0);
    let middle = capsule.origin.x.0 + capsule.size.width.0 / 2.0;
    let pane_middle = pane.origin.x.0 + pane.size.width.0 / 2.0;
    assert!(
        (middle - pane_middle).abs() < 1.0,
        "{middle} vs {pane_middle}"
    );
    assert!(
        capsule.origin.y.0 - pane.origin.y.0 <= 20.0,
        "{capsule:?} in {pane:?}"
    );
    // Waiting for a network, then the session ended: still the capsule.
    for (state, ended, says) in [
        ("offline", false, "Waiting for network…"),
        ("offline", true, "Session ended"),
    ] {
        show(
            &mut app,
            &channel,
            &chrome(
                json!({"state": state, "cover": {"kind": "veil", "state": state, "ended": ended}}),
            ),
        );
        app.wait_for(&format!("text:{says}")).unwrap();
        assert_eq!(mounted(&app, &["terminal-connecting"]), 0, "{says}");
        assert_eq!(natives.count("moshpit/pane-loader"), 0, "{says}");
    }
    // The host lifts it (the first frame painted): it goes; nothing
    // stands in its place.
    show(&mut app, &channel, &chrome(json!({})));
    app.wait_for_gone("id:terminal-reconnect-veil", Duration::from_secs(5))
        .unwrap();
    assert_eq!(mounted(&app, &["terminal-connecting"]), 0);
    channel.close();
}

#[test]
fn the_banner_area_holds_one_banner_under_the_bar() {
    let (mut app, channel, _) =
        start_observing(Screen::Terminal, Params::default(), "terminal-screen");
    let kinds = [
        json!({"kind": "returnPath"}),
        json!({"kind": "degrade", "missing": "tmux"}),
        json!({"kind": "roam", "srtt": 90.0, "history": [90.0]}),
        json!({"kind": "returnPath"}),
    ];
    for (banner, id) in kinds.iter().zip([
        "mosh-returnpath-banner",
        "host-banner",
        "roam-banner",
        "mosh-returnpath-banner",
    ]) {
        show(&mut app, &channel, &chrome(json!({"banner": banner})));
        app.wait_for(&format!("id:{id}")).unwrap();
        assert_eq!(
            mounted(&app, &BANNERS),
            1,
            "{banner}: {:?}",
            app.screen().texts()
        );
    }
    // 8 pt under the top bar, 16 pt in from the screen's sides.
    let (_, bar) = bounds(&app, "terminal-top-bar").unwrap();
    let (_, screen) = bounds(&app, "terminal-screen").unwrap();
    let (_, banner) = bounds(&app, "mosh-returnpath-banner").unwrap();
    let under = banner.origin.y.0 - (bar.origin.y.0 + bar.size.height.0);
    assert!((under - 8.0).abs() < 0.5, "{under} under the bar");
    assert!(
        (banner.origin.x.0 - screen.origin.x.0 - 16.0).abs() < 0.5,
        "{banner:?}"
    );
    let right = screen.origin.x.0 + screen.size.width.0 - (banner.origin.x.0 + banner.size.width.0);
    assert!((right - 16.0).abs() < 0.5, "{right} from the right");
    // A roam's round trip changes its words in place; still one banner.
    show(
        &mut app,
        &channel,
        &chrome(json!({"banner": {"kind": "roam", "srtt": 120.0, "history": [90.0, 120.0]}})),
    );
    app.wait_for("text:Network changed. mosh is reconnecting. · 120 ms")
        .unwrap();
    assert_eq!(mounted(&app, &BANNERS), 1);
    // The notice stands under the banner, not in its place.
    show(
        &mut app,
        &channel,
        &chrome(
            json!({"banner": {"kind": "roam", "srtt": 120.0, "history": []},
                       "notice": {"text": "This pane is open on another device", "action": "Open here"}}),
        ),
    );
    app.wait_for("id:terminal-notice").unwrap();
    assert_eq!(mounted(&app, &BANNERS), 1);
    let (_, roam) = bounds(&app, "roam-banner").unwrap();
    let (_, notice) = bounds(&app, "terminal-notice").unwrap();
    assert!(
        notice.origin.y.0 >= roam.origin.y.0 + roam.size.height.0,
        "{notice:?} under {roam:?}"
    );
    // Taken away: it goes.
    show(&mut app, &channel, &chrome(json!({})));
    app.wait_for_gone("id:roam-banner", Duration::from_secs(5))
        .unwrap();
    app.wait_for_gone("id:terminal-notice", Duration::from_secs(5))
        .unwrap();
    assert_eq!(mounted(&app, &BANNERS), 0);
    channel.close();
}

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

#[test]
fn the_empty_states_and_the_follower_take_presses_over_44_points() {
    let (mut app, channel, _) =
        start_observing(Screen::Terminal, Params::default(), "terminal-screen");
    let slops = Slops::default();
    let observed = slops.clone();
    app.observe(move |_, batch| observed.apply(batch));
    let target = |app: &Headless, id: &str| {
        let (node, b) = bounds(app, id).unwrap_or_else(|| panic!("no view {id}"));
        let slop = slops.of(node);
        (
            b.size.width.0 + slop.left.0 + slop.right.0,
            b.size.height.0 + slop.top.0 + slop.bottom.0,
        )
    };
    for (missing, id) in [
        (false, "multiplexer-create-first-session"),
        (true, "multiplexer-install"),
    ] {
        show(
            &mut app,
            &channel,
            &chrome(json!({"content": "empty",
                           "empty": {"mux": "tmux", "missing": missing,
                                     "session": "Session", "sessionLower": "session",
                                     "sessionsLower": "sessions"}})),
        );
        app.wait_for_within(&format!("id:{id}"), Duration::from_secs(8))
            .unwrap();
        let (w, h) = target(&app, id);
        assert!(w >= 44.0 && h >= 44.0, "{id}: takes presses over {w} × {h}");
        show(&mut app, &channel, &chrome(json!({})));
        app.wait_for_gone("id:terminal-empty-state", Duration::from_secs(5))
            .unwrap();
    }
    show(
        &mut app,
        &channel,
        &chrome(
            json!({"cover": {"kind": "follow", "state": "live", "owner": "iPhone", "touches": true}}),
        ),
    );
    app.wait_for("id:terminal-size-follow-take").unwrap();
    let (w, h) = target(&app, "terminal-size-follow-take");
    assert!(w >= 44.0 && h >= 44.0, "the follower's button: {w} × {h}");
    channel.close();
}
