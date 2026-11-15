//! R6 (UI-DESIGN §6.1, §8): Home on a headless engine at the owner's
//! iPhone size, on R5's acceptance fixture (a herdr host with 13 agents
//! over 12 workspaces, a tmux host with 3 agents and other panes, one
//! agent waiting, an offline host, a saved one).
//!
//! §8's checks: every row is one target and ⋯ is only on a card's head;
//! the fixture's page is at most two screens; screen readers meet NEEDS YOU
//! (or the summary), then each card's head, then its rows. And what §6.1
//! asks of the page: the large title 8 pt over the first group and on the
//! cards' edge, the summary that is no button, no version, tagline or grid
//! on a Home with hosts, each body's row and where its tap goes, the idle
//! group the host keeps, the app's language.

mod common;

use std::sync::Arc;

use common::{
    App, Natives, hear, hidden, home_fixture, next, post, quiet, start_observing_with, start_on,
    zh_hans,
};
use moshpit_ui::{
    channel::Channel,
    home::{
        Message, TreeState,
        model::{
            Connection, ConnectionFailure, Control, FailureReason, HomeState, Link, Mismatch, Mux,
        },
    },
    screen::{Params, Screen},
    ui::mark::Face,
};
use nsh_core::{Edges, Points, Size, geometry::px, platform::Idiom};
use nsh_testing::{Headless, LayoutNode};
use serde_json::json;

/// The owner's iPhone (iPhone 16 Pro), in points.
const WIDTH: f32 = 402.0;
const HEIGHT: f32 = 874.0;

/// The time the page's clock stands at (Unix seconds): the fixture's
/// stamps are set against it, so a duration reads the same on any run.
const NOW: f64 = 1_800_000_000.0;

/// Home on the owner's iPhone: its size, its bar, its safe area, its clock
/// at [`NOW`] (manual: it moves only as the steps move it, a couple of
/// seconds a step while the working dots breathe).
fn phone(params: Params) -> (App, Arc<Channel>, Natives) {
    start_observing_with(Screen::Home, params, "home", |config| {
        config.clock = nsh_core::time::Clock::manual(
            std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(NOW),
        );
        config.viewport = Size::new(Points(WIDTH), Points(HEIGHT));
        config.environment.idiom = Idiom::Phone;
        config.environment.safe_area = Edges {
            top: Points(62.0),
            bottom: Points(34.0),
            left: Points(0.0),
            right: Points(0.0),
        };
    })
}

/// R5's fixture with its logins, addresses and clocks set against now:
/// the waiting agent waits 50 minutes, the two at work have worked 33 and
/// 25, the done one finished 16 minutes ago; build-box has been up three
/// hours; edge was lost 25 minutes ago.
fn hosts() -> Vec<Connection> {
    hosts_at(NOW)
}

/// [`hosts`] with `now` as the time now (the fixture's stamps are 1000 to
/// 4500: at 4000, as they are).
fn hosts_at(now: f64) -> Vec<Connection> {
    let shift = now - 4000.0;
    let mut hosts = home_fixture::fixture();
    for (host, (user, address, initial)) in hosts.iter_mut().zip([
        ("dev", "gpu-01.example.com", "G"),
        ("me", "build.example.com", "B"),
        ("root", "edge.example.com", "E"),
        ("pi", "lab.example.com", "L"),
    ]) {
        host.user = user.into();
        host.host = address.into();
        host.initial = initial.into();
        if let Some(control) = host.control.as_mut() {
            for hook in &mut control.hooks {
                hook.since = hook.since.map(|s| s + shift);
            }
        }
    }
    hosts[0].connected_at = Some(now - 5.0 * 3600.0);
    hosts[1].connected_at = Some(now - 3.0 * 3600.0);
    hosts[2].lost_at = Some(now - 25.0 * 60.0);
    hosts
}

/// The device's copy of the fixture (`Tests/MoshpitTests/Fixtures/
/// home-acceptance.json`, Home's message at a "now" of 4000, which the
/// device test moves to its own now): what these tests post, so the device
/// draws the page they measure. `UPDATE_FIXTURES=1` writes it.
#[test]
fn the_devices_fixture_is_this_one() {
    let state = HomeState {
        connections: hosts_at(4000.0),
        version: "v1.0.9 (1)".into(),
        sidebar: false,
    };
    let message: serde_json::Value =
        serde_json::from_str(&Message::Home(Box::new(state)).to_json()).unwrap();
    let written = serde_json::to_string_pretty(&message).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../Tests/MoshpitTests/Fixtures/home-acceptance.json");
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &written).unwrap();
    }
    let kept = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        kept == written,
        "{} is not the fixture: run UPDATE_FIXTURES=1 cargo test -p moshpit-ui --test home",
        path.display()
    );
}

fn show(app: &mut Headless, channel: &Channel, connections: Vec<Connection>) {
    let state = HomeState {
        connections,
        version: "v1.0.9 (1)".into(),
        sidebar: false,
    };
    post(app, channel, &Message::Home(Box::new(state)).to_json());
}

fn find(app: &mut Headless, selector: &str) {
    app.wait_for(selector)
        .unwrap_or_else(|e| panic!("{selector}: {e}: {:?}", app.screen().texts()));
}

/// The view `id`, laid out.
fn node(nodes: &[LayoutNode], id: &str) -> LayoutNode {
    nodes
        .iter()
        .find(|n| n.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no {id}"))
        .clone()
}

/// Whether `node` is `ancestor` or inside it.
fn within(nodes: &[LayoutNode], node: nsh_core::NodeId, ancestor: nsh_core::NodeId) -> bool {
    let mut at = node;
    loop {
        if at == ancestor {
            return true;
        }
        match nodes.iter().find(|n| n.id == at) {
            Some(n) if n.parent != at => at = n.parent,
            _ => return false,
        }
    }
}

const GPU: &str = "c-gpu";
const BOX: &str = "c-box";
const EDGE: &str = "c-edge";
const LAB: &str = "c-lab";

/// The rows of a page: every view whose identifier says it is one.
fn rows(nodes: &[LayoutNode]) -> Vec<LayoutNode> {
    nodes
        .iter()
        .filter(|n| {
            n.test_id.as_deref().is_some_and(|id| {
                [
                    "home-pane-",
                    "home-idle-",
                    "home-all-",
                    "home-more-working-",
                    "card-dead-",
                    "card-edit-",
                    "card-open-terminal-",
                    "card-attach-stalled-retry-",
                    "connection-card-",
                ]
                .iter()
                .any(|p| id.starts_with(p))
            })
        })
        .cloned()
        .collect()
}

/// §8 R6: the fixture "herdr 13 + tmux 3 + 1 waiting + 1 offline" (and the
/// saved host) is at most two screens tall on the owner's iPhone.
#[test]
fn the_acceptance_fixture_fits_two_screens() {
    let (mut app, channel, _) = phone(Params::default());
    show(&mut app, &channel, hosts());
    find(&mut app, "id:home-needs-you");
    let list = node(&app.screen().layout_nodes(), "home-list");
    let content = list
        .scroll
        .and_then(|s| s.content_size)
        .expect("the list's content")
        .height
        .0;
    assert!(
        content <= 2.0 * HEIGHT,
        "the page is {content} pt, more than two screens ({})",
        2.0 * HEIGHT
    );
    // gpu-01's card: three agents, the idle group, all workspaces (R5's
    // five rows); the waiting one is in NEEDS YOU only.
    let nodes = app.screen().layout_nodes();
    let gpu = node(&nodes, "home-card-gpu-01");
    let in_gpu: Vec<String> = rows(&nodes)
        .into_iter()
        .filter(|r| within(&nodes, r.id, gpu.id))
        .filter_map(|r| r.test_id.map(|t| t.to_string()))
        .collect();
    assert_eq!(
        in_gpu,
        [
            "connection-card-gpu-01".to_owned(),
            format!("home-pane-{GPU}-w1:p1"),
            format!("home-pane-{GPU}-w2:p1"),
            format!("home-pane-{GPU}-w5:p1"),
            format!("home-idle-{GPU}"),
            format!("home-all-{GPU}"),
        ]
    );
    assert_eq!(
        app.screen()
            .count(&format!("id:home-pane-{GPU}-w3:p1"))
            .unwrap(),
        1,
        "the waiting agent once, in NEEDS YOU"
    );
    let needs = node(&nodes, "home-needs-you");
    let waiting = node(&nodes, &format!("home-pane-{GPU}-w3:p1"));
    assert!(within(&nodes, waiting.id, needs.id));
    channel.close();
}

/// §8 R6: every row is one target (a press anywhere in it is the row's),
/// and ⋯ is only on a card's head.
#[test]
fn every_row_is_one_target_and_more_is_only_on_a_head() {
    let (mut app, channel, natives) = phone(Params::default());
    show(&mut app, &channel, hosts());
    find(&mut app, "id:home-needs-you");
    let nodes = app.screen().layout_nodes();
    let rows = rows(&nodes);
    assert!(rows.len() >= 15, "{} rows", rows.len());
    for row in &rows {
        let id = row.test_id.as_deref().unwrap();
        assert_eq!(
            app.screen().press_target(&format!("view:{}", row.id)),
            Ok(Some(row.id)),
            "{id} takes its own presses"
        );
        // Nothing in it takes a press of its own: a press on any view in
        // it is the row's, or (a view that takes no touches) passes
        // through to it.
        for inside in nodes
            .iter()
            .filter(|n| n.id != row.id && within(&nodes, n.id, row.id))
        {
            let target = app.screen().press_target(&format!("view:{}", inside.id));
            assert!(
                matches!(target, Ok(None)) || target == Ok(Some(row.id)),
                "{id}: a press on {:?} ({:?}) goes to {target:?}",
                inside.test_id,
                inside.text.as_ref().map(|t| t.text.text.clone())
            );
        }
    }
    // One ⋯ per card with a session, each on its card's head.
    assert_eq!(natives.count("moshpit/action-menu"), 3);
    for (name, id) in [("gpu-01", GPU), ("build-box", BOX), ("edge", EDGE)] {
        let menu = natives
            .find("moshpit/action-menu", &format!("card-menu-{name}"))
            .unwrap_or_else(|| panic!("{name}'s ⋯"));
        let head = node(&nodes, &format!("home-head-{id}"));
        assert!(within(&nodes, menu, head.id), "{name}'s ⋯ on its head");
    }
    channel.close();
}

/// §8 R6: screen readers meet NEEDS YOU first, then each card's head
/// before its rows; a row is one element that says its project, agent,
/// state and time, and where it works.
#[test]
fn screen_readers_meet_needs_you_then_heads_then_rows() {
    let (mut app, channel, _) = phone(Params::default());
    show(&mut app, &channel, hosts());
    find(&mut app, "id:home-needs-you");
    let nodes = app.screen().layout_nodes();
    let at = |id: &str| {
        nodes
            .iter()
            .position(|n| n.test_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("{id}"))
    };
    let needs = at("home-needs-you");
    let mut last = needs;
    for (name, first_row) in [
        ("gpu-01", format!("home-pane-{GPU}-w1:p1")),
        ("build-box", format!("home-pane-{BOX}-%1")),
        ("edge", "card-dead-edge".to_owned()),
    ] {
        let head = at(&format!("connection-card-{name}"));
        assert!(head > last, "{name}'s head after what is above it");
        assert!(at(&first_row) > head, "{name}'s rows after its head");
        last = at(&first_row);
    }
    assert!(at("connection-card-lab") > last, "SAVED last");
    // A row is one element, read as §6.1 says it.
    assert!(
        app.screen()
            .matches("label:api, claude, working 33 minutes, ~/src/api")
            .unwrap(),
        "{:?}",
        app.screen().texts()
    );
    assert!(
        app.screen()
            .matches("label:trainer, claude, gpu-01, needs you 50 minutes, ~/src/trainer")
            .unwrap()
    );
    assert!(
        app.screen()
            .matches("label:gpu-01, SSH, Online, dev@gpu-01.example.com, 1 waiting")
            .unwrap(),
        "the head says its state and the badge"
    );
    channel.close();
}

/// §6.1 structure 1: the large title on the cards' edge, its row ending 8
/// pt above the first group; the groups' titles on the rows' text (32 pt
/// in).
#[test]
fn the_large_title_sits_over_the_first_group() {
    let (mut app, channel, _) = phone(Params::default());
    show(&mut app, &channel, hosts());
    find(&mut app, "id:home-needs-you");
    let nodes = app.screen().layout_nodes();
    let large = node(&nodes, "nav-bar-large-stretch");
    let first = node(&nodes, "home-needs-you");
    let bottom = large.bounds.origin.y.0 + large.bounds.size.height.0;
    assert!(
        (first.bounds.origin.y.0 - bottom - 8.0).abs() < 0.5,
        "the first group at {}, the large title's row ends at {bottom}",
        first.bounds.origin.y.0
    );
    let title_x = node(&nodes, "nav-bar-title")
        .text
        .expect("the title's text")
        .content
        .origin
        .x
        .0;
    let card_x = node(&nodes, "home-card-gpu-01").bounds.origin.x.0;
    assert!((title_x - card_x).abs() < 0.5, "{title_x} vs {card_x}");
    assert!((card_x - 16.0).abs() < 0.5, "space-page: {card_x}");
    let eyebrow = nodes
        .iter()
        .find(|n| {
            n.text
                .as_ref()
                .is_some_and(|t| t.text.text.eq_ignore_ascii_case("needs you"))
        })
        .expect("NEEDS YOU's title");
    let x = eyebrow.text.as_ref().unwrap().content.origin.x.0;
    assert!((x - 32.0).abs() < 0.5, "the title on the rows' text: {x}");
    channel.close();
}

/// §6.1 decision 4: where each row and head goes; ⋯ is the host's menu.
#[test]
fn what_the_user_does_reaches_the_host() {
    let (mut app, channel, natives) = phone(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, hosts());
    find(&mut app, "id:home-needs-you");

    for (press, said) in [
        (
            format!("id:home-pane-{GPU}-w3:p1"),
            json!({"ev": "pane", "connection": GPU, "pane": "w3:p1"}),
        ),
        (
            format!("id:home-pane-{BOX}-%1"),
            json!({"ev": "pane", "connection": BOX, "pane": "%1"}),
        ),
        (
            format!("id:home-pane-{BOX}-%5"),
            json!({"ev": "pane", "connection": BOX, "pane": "%5"}),
        ),
        (
            "id:connection-card-gpu-01".to_owned(),
            json!({"ev": "enter", "connection": GPU}),
        ),
        (
            "id:connection-card-edge".to_owned(),
            json!({"ev": "connect", "connection": EDGE}),
        ),
        (
            "id:card-dead-edge".to_owned(),
            json!({"ev": "connect", "connection": EDGE}),
        ),
        (
            "id:connection-card-lab".to_owned(),
            json!({"ev": "connect", "connection": LAB}),
        ),
        (
            format!("id:home-all-{GPU}"),
            json!({"ev": "sessions", "connection": GPU}),
        ),
        (
            format!("id:home-all-{BOX}"),
            json!({"ev": "sessions", "connection": BOX}),
        ),
        ("id:home-add".to_owned(), json!({"ev": "add"})),
        ("id:home-settings".to_owned(), json!({"ev": "settings"})),
    ] {
        app.press(&press).unwrap();
        assert_eq!(next(&heard), said, "{press}");
    }
    assert!(app.screen().shows("Lost 25 minutes ago"));

    // ⋯: Open, Edit…, Disconnect, Delete… apart; Connect for a host whose
    // session is gone.
    let items = |menu: &str| -> Vec<String> {
        let node = natives.find("moshpit/action-menu", menu).unwrap();
        natives
            .prop(node, "items")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(
        items("card-menu-gpu-01"),
        ["open", "edit", "disconnect", "delete"]
    );
    assert_eq!(
        items("card-menu-edge"),
        ["connect", "edit", "disconnect", "delete"]
    );
    let menu = natives
        .find("moshpit/action-menu", "card-menu-gpu-01")
        .unwrap();
    common::emit(&mut app, menu, "select", json!("delete"));
    assert_eq!(next(&heard), json!({"ev": "delete", "connection": GPU}));
    quiet(&heard);
    channel.close();
}

/// §6.1: the idle group opens in place and says so; the host keeps it,
/// and a page made again opens it from what the host hands back.
#[test]
fn the_idle_group_opens_and_the_host_keeps_it() {
    let (mut app, channel, _) = phone(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, hosts());
    let idle = format!("id:home-idle-{GPU}");
    find(&mut app, &idle);
    assert!(app.screen().shows("9 idle"));
    assert!(app.screen().shows("api, proj-4, infra, proj-7 +5"));
    assert_eq!(
        app.screen().accessibility_state(&idle).unwrap().expanded,
        Some(false)
    );
    let infra = format!("id:home-pane-{GPU}-w6:p1");
    assert!(!app.screen().matches(&infra).unwrap());
    app.press(&idle).unwrap();
    assert_eq!(
        next(&heard),
        json!({"ev": "tree", "connection": GPU, "idle": true}),
        "only Home's own field: the sessions page keeps the rest"
    );
    assert!(
        app.screen().matches(&infra).unwrap(),
        "the idle agents in place"
    );
    assert_eq!(
        app.screen().accessibility_state(&idle).unwrap().expanded,
        Some(true)
    );
    channel.close();

    let (mut app, channel, _) = phone(Params::default());
    post(
        &mut app,
        &channel,
        &Message::Trees(vec![TreeState {
            connection: GPU.into(),
            idle: true,
            ..TreeState::default()
        }])
        .to_json(),
    );
    show(&mut app, &channel, hosts());
    find(&mut app, &infra);
    channel.close();
}

/// §6.1 structure 2: nobody waits, a line of facts that is no button;
/// nothing at all with no host up. No version, tagline or grid on a Home
/// with hosts (§6.1, §10.1).
#[test]
fn the_summary_says_the_facts_and_takes_no_tap() {
    let (mut app, channel, natives) = phone(Params::default());
    let mut quiet_hosts = hosts();
    for hook in &mut quiet_hosts[0].control.as_mut().unwrap().hooks {
        if hook.pane == "w3:p1" {
            hook.state = Some("working".into());
        }
    }
    show(&mut app, &channel, quiet_hosts);
    find(&mut app, "id:home-summary");
    assert!(!app.screen().matches("id:home-needs-you").unwrap());
    assert!(
        app.screen()
            .matches("label:Nothing needs you · 4 working · 2 done · 10 idle")
            .unwrap(),
        "{:?}",
        app.screen().texts()
    );
    assert_eq!(app.screen().press_target("id:home-summary"), Ok(None));
    for gone in ["v1.0.9 (1)", "THE PIT NEVER CLOSES", "The pit never closes"] {
        assert!(!app.screen().shows(gone), "{gone} on a Home with hosts");
    }
    assert_eq!(
        natives.count("moshpit/signal-grid"),
        0,
        "no grid with hosts"
    );

    // Only a plain shell up: how many are online.
    let plain = Connection {
        id: "c-plain".into(),
        name: "plain".into(),
        initial: "P".into(),
        user: "me".into(),
        host: "plain.example.com".into(),
        link: Link::Live,
        ..Connection::default()
    };
    show(&mut app, &channel, vec![plain]);
    find(&mut app, "id:card-open-terminal-plain");
    assert!(app.screen().matches("label:1 connection online").unwrap());
    // Nothing up: nothing at the top, CONNECTIONS has nothing, SAVED first.
    let mut saved = hosts();
    saved.retain(|c| c.link == Link::Saved);
    show(&mut app, &channel, saved);
    find(&mut app, "id:connection-card-lab");
    assert!(!app.screen().matches("id:home-summary").unwrap());
    assert!(!app.screen().matches("id:home-connections").unwrap());
    channel.close();
}

/// §6.1 states: each body a host's session can have, and where it goes.
#[test]
fn a_cards_body_says_where_its_host_is() {
    let (mut app, channel, _) = phone(Params::default());
    let heard = hear(&channel);
    let host = |id: &str, name: &str, link: Link| Connection {
        id: id.into(),
        name: name.into(),
        initial: "H".into(),
        user: "me".into(),
        host: format!("{name}.example.com"),
        link,
        ..Connection::default()
    };
    let mut failed = host("c-auth", "auth", Link::Dead);
    failed.failure = Some(ConnectionFailure {
        reason: FailureReason::Auth,
        detail: "The password was wrong.".into(),
    });
    let mut attaching = host("c-att", "att", Link::Attaching);
    attaching.control = Some(Control {
        mux: Mux::Tmux,
        ..Control::default()
    });
    let mut skew = host("c-skew", "skew", Link::Live);
    skew.control = Some(Control {
        mux: Mux::Herdr,
        attached: true,
        mismatch: Some(Mismatch {
            notice: "herdr versions differ (client 0.9, server 0.8)".into(),
            restart: true,
            restarting: false,
        }),
        ..Control::default()
    });
    show(
        &mut app,
        &channel,
        vec![
            host("c-dial", "dial", Link::Connecting),
            attaching,
            host("c-stall", "stall", Link::Stalled),
            failed,
            skew,
            host("c-shell", "shell", Link::Live),
        ],
    );
    find(&mut app, "id:card-connecting-dial");
    assert!(app.screen().shows("Connecting…"));
    assert!(app.screen().shows("Connecting to tmux…"));
    assert!(app.screen().shows("tmux isn't answering"));
    // Worded from the reason: the failure's detail is the session
    // layer's log line, never shown.
    assert!(app.screen().shows("Sign-in failed"));
    assert!(!app.screen().shows("The password was wrong."));
    assert!(
        app.screen()
            .shows("herdr versions differ (client 0.9, server 0.8)")
    );
    for (press, said) in [
        (
            "id:card-attach-stalled-retry-stall",
            json!({"ev": "retry-attach", "connection": "c-stall"}),
        ),
        (
            "id:card-edit-auth",
            json!({"ev": "edit", "connection": "c-auth"}),
        ),
        (
            "id:card-mismatch-skew >> label:Restart server",
            json!({"ev": "restart-server", "connection": "c-skew"}),
        ),
        (
            "id:card-open-terminal-shell",
            json!({"ev": "enter", "connection": "c-shell"}),
        ),
    ] {
        app.press(press).unwrap();
        assert_eq!(next(&heard), said, "{press}");
    }
    // A connecting row is no button.
    assert_eq!(
        app.screen().press_target("id:card-connecting-dial"),
        Ok(None)
    );
    channel.close();
}

/// §6.1 AX: at an accessibility size (AX3) a row's time goes under its
/// title and the page still has every card, ⋯ unshrunk.
#[test]
fn at_an_accessibility_text_size() {
    let params = Params {
        font_scale: Some(40.0 / 17.0),
        ..Params::default()
    };
    let (mut app, channel, natives) = phone(params);
    show(&mut app, &channel, hosts());
    find(&mut app, "id:home-needs-you");
    assert_eq!(natives.count("moshpit/action-menu"), 3);
    let nodes = app.screen().layout_nodes();
    let row = node(&nodes, &format!("home-pane-{GPU}-w3:p1"));
    let text = |words: &str| {
        nodes
            .iter()
            .find(|n| {
                within(&nodes, n.id, row.id)
                    && n.text.as_ref().is_some_and(|t| t.text.text == words)
            })
            .unwrap_or_else(|| panic!("{words}"))
            .bounds
    };
    let (title, time) = (text("trainer"), text("50m"));
    assert!(
        time.origin.y.0 >= title.origin.y.0 + title.size.height.0 - 0.5,
        "the time under the title: {time:?}, {title:?}"
    );
    let more = node(&nodes, &format!("home-head-{GPU}"));
    assert!(more.bounds.size.height.0 >= 68.0);
    channel.close();
}

/// No connections yet: the grid, the app's mark, what Home will show, the
/// tagline, and Add connection.
#[test]
fn no_connections_is_the_empty_page() {
    let (mut app, channel, natives) = phone(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, Vec::new());
    find(&mut app, "id:home-add-empty");
    assert!(app.screen().shows("No connections yet"));
    assert!(
        app.screen()
            .shows("Add a machine you can reach over SSH. Agents running there show up here.")
    );
    assert!(app.screen().shows("THE PIT NEVER CLOSES"));
    assert_eq!(natives.count("moshpit/signal-grid"), 1);
    app.press("id:home-add-empty").unwrap();
    assert_eq!(next(&heard), json!({"ev": "add"}));
    channel.close();
}

/// In the app's language: §6.1's copy in Chinese.
#[test]
fn in_the_apps_language() {
    let (mut app, channel, _) = phone(zh_hans());
    show(&mut app, &channel, hosts());
    find(&mut app, "id:home-needs-you");
    for words in [
        "在等你",
        "连接",
        "已保存",
        "9 个空闲",
        "已完成",
        "全部工作区",
        "全部会话",
        "25 分钟前断开",
        "重新连接",
        "1 个在等你",
    ] {
        assert!(
            app.screen().shows(words),
            "{words}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}

/// The head's swipe: Edit, Disconnect, Delete; a saved row's: Edit,
/// Delete.
#[test]
fn a_heads_swipe_opens_its_tray() {
    let (mut app, channel, _) = phone(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, hosts());
    // The drag starts on the head's target, under the tray's pan.
    let head = "id:connection-card-build-box";
    find(&mut app, head);
    app.drag(head, nsh_core::geometry::point(px(-220.0), px(0.0)))
        .unwrap();
    app.settle();
    app.press(&format!("id:home-tray-head-{BOX} >> label:Disconnect"))
        .unwrap();
    assert_eq!(next(&heard), json!({"ev": "disconnect", "connection": BOX}));
    app.drag(
        "id:connection-card-lab",
        nsh_core::geometry::point(px(-220.0), px(0.0)),
    )
    .unwrap();
    app.settle();
    app.press(&format!("id:home-tray-saved-{LAB} >> label:Delete…"))
        .unwrap();
    assert_eq!(next(&heard), json!({"ev": "delete", "connection": LAB}));
    channel.close();
}

/// The owner's iPhone (B316): once the large title had gone, rows ran
/// under the inline title and the glass buttons. On iOS 26 the bar has iOS
/// 26's scroll edge effect, coming in with the inline title as the list
/// scrolls (natively, by the list's offset); below iOS 26 its classic
/// background does. Neither is drawn by the other OS.
#[test]
fn the_bar_has_the_oses_look_once_the_large_title_has_gone() {
    for (version, drawn, never) in [
        ((26, 0), "nav-bar-edge", "nav-bar-background"),
        ((18, 2), "nav-bar-background", "nav-bar-edge"),
    ] {
        let (mut app, channel) = start_on(Screen::Home, Params::default(), "home", version);
        show(&mut app, &channel, hosts());
        find(&mut app, "id:home-add");
        assert_eq!(hidden(&app, drawn), Some(false), "{version:?}: {drawn}");
        assert_eq!(hidden(&app, never), None, "{version:?}: {never}");
        channel.close();
    }
}

/// §4.13: a host's mark is the emoji picked for it, else its trusted key's
/// face, else (no key trusted yet) its initial: on a card's head and in
/// SAVED's rows alike.
#[test]
fn a_hosts_mark_is_its_emoji_its_keys_face_or_its_initial() {
    let (mut app, channel, _) = phone(Params::default());
    let host = |id: &str, name: &str, initial: &str, link: Link| Connection {
        id: id.into(),
        name: name.into(),
        initial: initial.into(),
        user: "me".into(),
        host: format!("{name}.example.com"),
        link,
        ..Connection::default()
    };
    let face = Some(Face {
        side: 3,
        cells: vec![true, false, true, false, true, false, true, false, true],
        hue: 5,
        kinds: Vec::new(),
        asleep: Vec::new(),
    });
    let mut whale = host("c-whale", "whale", "W", Link::Live);
    whale.face = face.clone();
    whale.emoji = Some("🐳".into());
    let mut keyed = host("c-keyed", "keyed", "K", Link::Live);
    keyed.face = face.clone();
    let fresh = host("c-fresh", "fresh", "F", Link::Live);
    let mut berry = host("c-berry", "berry", "B", Link::Saved);
    berry.emoji = Some("🍓".into());
    let mut kept = host("c-kept", "kept", "Q", Link::Saved);
    kept.face = face;
    let never = host("c-never", "never", "N", Link::Saved);
    show(
        &mut app,
        &channel,
        vec![whale, keyed, fresh, berry, kept, never],
    );
    find(&mut app, "id:connection-card-never");
    for shown in ["🐳", "🍓", "F", "N"] {
        assert!(
            app.screen().shows(shown),
            "{shown}: {:?}",
            app.screen().texts()
        );
    }
    for hidden in ["W", "K", "B", "Q"] {
        assert!(
            !app.screen().shows(hidden),
            "{hidden} shown under a face or an emoji"
        );
    }
    channel.close();
}

/// B512: an edit (name, address, emoji) or a key trusted since (its face)
/// shows on the card's head and in SAVED's row as soon as Home hears it,
/// not only once the host is added, removed or reconnected.
#[test]
fn an_edit_or_a_key_trusted_since_shows_at_once() {
    let (mut app, channel, _) = phone(Params::default());
    let host = |id: &str, name: &str, initial: &str, link: Link| Connection {
        id: id.into(),
        name: name.into(),
        initial: initial.into(),
        user: "me".into(),
        host: format!("{name}.example.com"),
        link,
        ..Connection::default()
    };
    show(
        &mut app,
        &channel,
        vec![
            host("c-live", "alpha", "A", Link::Live),
            host("c-saved", "beta", "B", Link::Saved),
        ],
    );
    find(&mut app, "id:connection-card-beta");
    for shown in ["alpha", "A", "beta", "B", "me@beta.example.com"] {
        assert!(app.screen().shows(shown), "{shown}");
    }

    let mut live = host("c-live", "alpha-2", "A", Link::Live);
    live.emoji = Some("🦊".into());
    let mut saved = host("c-saved", "gamma", "G", Link::Saved);
    saved.face = Some(Face {
        side: 3,
        cells: vec![true, false, true, false, true, false, true, false, true],
        hue: 2,
        kinds: Vec::new(),
        asleep: Vec::new(),
    });
    show(&mut app, &channel, vec![live, saved]);
    find(&mut app, "id:connection-card-gamma");
    for shown in ["alpha-2", "🦊", "gamma", "me@gamma.example.com"] {
        assert!(
            app.screen().shows(shown),
            "{shown} not shown: {:?}",
            app.screen().texts()
        );
    }
    for gone in ["alpha", "A", "beta", "B", "G", "me@beta.example.com"] {
        assert!(
            !app.screen().shows(gone),
            "{gone} still shown: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}

/// B519: mosh that fell back to SSH (the host has no mosh-server) runs
/// over SSH, and doesn't roam: its card says SSH, in the warning's
/// colours, not mosh.
#[test]
fn mosh_that_fell_back_says_ssh() {
    let (mut app, channel, _) = phone(Params::default());
    let host = |id: &str, name: &str, degraded: bool| Connection {
        id: id.into(),
        name: name.into(),
        initial: "M".into(),
        user: "me".into(),
        host: format!("{name}.example.com"),
        mosh: true,
        degraded,
        link: Link::Live,
        ..Connection::default()
    };
    show(&mut app, &channel, vec![host("c-fell", "fell", true)]);
    find(&mut app, "id:connection-card-fell");
    assert!(app.screen().shows("SSH"), "{:?}", app.screen().texts());
    assert!(!app.screen().shows("mosh"), "{:?}", app.screen().texts());
    show(&mut app, &channel, vec![host("c-fell", "fell", false)]);
    find(&mut app, "text:mosh");
    assert!(!app.screen().shows("SSH"));
    channel.close();
}

/// The egg on Home (PET-DESIGN §15.2): on the top edge of the card it
/// perches on (NEEDS YOU's, a host's, SAVED's), in a 44 pt target that a
/// tap reaches, not the card under it or the list: each tap tells the
/// host. Once the host says it may hatch (tapped three times, or ready by
/// itself, and iCloud settled), the page asks for the hatch.
#[test]
fn a_tap_reaches_the_egg_on_each_cards_edge_and_the_page_asks_to_hatch() {
    let pet = |taps: u8, ready: bool, may_hatch: bool| {
        json!({
            "op": "pet",
            "style": "pixel",
            "shown": true,
            "egg": {"ready": ready, "taps": taps},
            "may_hatch": may_hatch,
            "seq": 0,
        })
        .to_string()
    };
    // What the page told the host, until `want` (else none of it).
    let told = |heard: &std::sync::mpsc::Receiver<String>, want: &serde_json::Value| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match heard.recv_timeout(left) {
                Ok(json) if serde_json::from_str::<serde_json::Value>(&json).ok().as_ref() == Some(want) => {
                    return true;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        false
    };
    let host = |id: &str, name: &str, link: Link| Connection {
        id: id.into(),
        name: name.into(),
        initial: "H".into(),
        user: "me".into(),
        host: format!("{name}.example.com"),
        link,
        ..Connection::default()
    };
    for (perch, connections) in [
        ("needs you", hosts()),
        ("a host", vec![host("c-up", "up", Link::Live)]),
        ("saved", vec![host("c-kept", "kept", Link::Saved)]),
    ] {
        let (mut app, channel, _) = phone(Params::default());
        let heard = hear(&channel);
        show(&mut app, &channel, connections);
        post(&mut app, &channel, &pet(0, false, false));
        find(&mut app, "id:home-pet");
        let target = node(&app.screen().layout_nodes(), "home-pet").bounds;
        assert!(
            target.size.width.0 >= 44.0 && target.size.height.0 >= 44.0,
            "{perch}: the egg's target is {:?}",
            target.size
        );
        for taps in 0..3u8 {
            app.press("id:home-pet")
                .unwrap_or_else(|e| panic!("{perch}: tap {}: {e}", taps + 1));
            assert!(
                told(&heard, &json!({"ev": "egg-tap"})),
                "{perch}: tap {} didn't reach the egg",
                taps + 1
            );
            post(&mut app, &channel, &pet(taps + 1, taps + 1 >= 2, false));
        }
        // iCloud settled: the host says it may, and the page asks.
        post(&mut app, &channel, &pet(3, true, true));
        assert!(told(&heard, &json!({"ev": "hatch"})), "{perch}: no hatch asked");
        channel.close();
    }
}

/// B535: a disconnect flushes the session for up to ~2 s. Until it is
/// over the card says so — "Disconnecting…", a spinner in ⋯'s place —
/// and offers nothing to tap: an Open or Connect asked of a session being
/// torn down was lost.
#[test]
fn a_disconnect_under_way_says_so_and_takes_no_taps() {
    let (mut app, channel, natives) = phone(Params::default());
    let heard = hear(&channel);
    let live = |disconnecting: bool| Connection {
        id: "c-box".into(),
        name: "box".into(),
        initial: "B".into(),
        user: "me".into(),
        host: "box.example.com".into(),
        link: Link::Live,
        disconnecting,
        ..Connection::default()
    };
    show(&mut app, &channel, vec![live(true)]);
    find(&mut app, "id:card-disconnecting-box");
    assert!(app.screen().shows("Disconnecting…"));
    assert!(!app.screen().shows("Open terminal"));
    assert_eq!(natives.count("moshpit/action-menu"), 0, "⋯ while it goes");
    app.press("id:connection-card-box").unwrap();
    quiet(&heard);

    show(&mut app, &channel, vec![live(false)]);
    find(&mut app, "text:Open terminal");
    assert!(!app.screen().shows("Disconnecting…"));
    assert_eq!(natives.count("moshpit/action-menu"), 1);
    app.press("id:connection-card-box").unwrap();
    assert_eq!(next(&heard), json!({"ev": "enter", "connection": "c-box"}));
    channel.close();
}

/// B668, the owner's call: once the pet has hatched (and its hatch has
/// played), Home asks whether to make it the app icon — 「把 App 图标换成
/// 墩墩吗？」, 「换」 and 「先不换」 — and tells the host the answer; the
/// host switches nothing on its own. No offer, no card.
#[test]
fn home_asks_about_the_pets_icon_and_tells_the_host_the_answer() {
    let pet = |icon: Option<serde_json::Value>| {
        let mut message = json!({
            "op": "pet", "style": "pixel", "shown": true, "may_hatch": false, "seq": 0,
            "pet": {
                "seed": "07070707070707070707070707070707", "stage": 1, "hatch_hour": 14,
                "flags": 0, "crowned": false, "name": "", "species": "tun",
                "hatched_at": 1_800_000_000, "tasks": 12,
                "traits": {"fur": "natural", "shiny": false, "pattern": "plain", "eyes": "dot", "size": "regular"},
                "history": [], "found": []
            }
        });
        if let Some(icon) = icon {
            message["icon"] = icon;
        }
        message.to_string()
    };
    let offer = |keep_default: bool| json!({"id": "tun", "species": "tun", "keep_default": keep_default});
    for (answer, keep_default) in [("home-icon-change", false), ("home-icon-not-yet", true)] {
        let (mut app, channel, _) = phone(Params::default());
        let heard = hear(&channel);
        show(&mut app, &channel, hosts());
        post(&mut app, &channel, &pet(None));
        find(&mut app, "id:home-pet");
        assert!(!app.screen().shows("Make Tun the app icon?"), "no offer, no card");
        post(&mut app, &channel, &pet(Some(offer(keep_default))));
        find(&mut app, "text:Make Tun the app icon?");
        for id in ["home-icon-change", "home-icon-not-yet"] {
            let bounds = node(&app.screen().layout_nodes(), id).bounds;
            assert!(bounds.size.height.0 >= 44.0, "{id}: {:?}", bounds.size);
        }
        app.press(&format!("id:{answer}")).unwrap();
        let switch = answer == "home-icon-change";
        loop {
            let event = next(&heard);
            if event["ev"] == "icon" {
                assert_eq!(event, json!({"ev": "icon", "switch": switch}));
                break;
            }
        }
        // Answered: the host stops offering, and the card goes.
        post(&mut app, &channel, &pet(None));
        assert!(!app.screen().shows("Make Tun the app icon?"));
        channel.close();
    }
}

/// It talks (PET-DESIGN §17): a tap on the pet answers with a line in a
/// bubble beside it, which takes no touches; with 「让它说话」 off, a tap
/// only pets it.
#[test]
fn a_tap_on_the_pet_answers_in_a_bubble_unless_it_is_told_not_to_talk() {
    let pet = |talk: bool| {
        json!({
            "op": "pet", "style": "pixel", "shown": true, "may_hatch": false, "talk": talk, "seq": 0,
            "pet": {
                "seed": "07070707070707070707070707070707", "stage": 1, "hatch_hour": 14,
                "flags": 0, "crowned": false, "name": "", "species": "tun",
                "hatched_at": 1_800_000_000, "tasks": 12,
                "traits": {"fur": "natural", "shiny": false, "pattern": "plain", "eyes": "dot", "size": "regular"},
                "history": [], "found": []
            }
        })
        .to_string()
    };
    for talk in [true, false] {
        let (mut app, channel, _) = phone(Params::default());
        show(&mut app, &channel, hosts());
        post(&mut app, &channel, &pet(talk));
        post(&mut app, &channel, r#"{"op":"visible","visible":true,"hour":14,"day":20000}"#);
        find(&mut app, "id:home-pet");
        app.press("id:home-pet").unwrap();
        let says = app
            .screen()
            .layout_nodes()
            .into_iter()
            .any(|n| n.test_id.as_deref() == Some("home-pet-says"));
        assert_eq!(says, talk, "talk {talk}: {:?}", app.screen().texts());
        channel.close();
    }
}

/// B661: the island's "+N more" opens Home at "Needs you", where every
/// agent waiting is listed: the host posts `{"op": "top"}` and the list
/// scrolls back to its top.
#[test]
fn the_host_s_top_brings_needs_you_back_into_view() {
    let (mut app, channel, _) = phone(Params::default());
    show(&mut app, &channel, hosts());
    find(&mut app, "id:home-needs-you");
    let at = |app: &mut Headless| node(&app.screen().layout_nodes(), "home-needs-you").bounds.origin.y.0;
    let top = at(&mut app);
    app.scroll("id:home-list", Points(700.0)).unwrap();
    assert!(at(&mut app) < top - 100.0, "scrolled away: {} from {top}", at(&mut app));
    post(&mut app, &channel, r#"{"op":"top"}"#);
    app.wait_for("id:home-needs-you").unwrap();
    for _ in 0..40 {
        if (at(&mut app) - top).abs() < 1.0 {
            break;
        }
        app.run_for(std::time::Duration::from_millis(50)).unwrap();
    }
    assert!((at(&mut app) - top).abs() < 1.0, "back at the top: {} vs {top}", at(&mut app));
    assert!(Message::parse(r#"{"op":"top"}"#).is_ok());
    channel.close();
}

/// B622: a share the host held back (its pane no longer runs the agent it
/// was queued for) is a quiet notice at the top of Home, one per share:
/// what happened, Send to…, and a close that discards it, each reaching
/// the host with the share. A share the host no longer holds goes.
#[test]
fn a_held_share_is_a_quiet_notice_that_sends_or_discards() {
    let held = |shares: serde_json::Value| json!({"op": "held", "shares": shares}).to_string();
    let (mut app, channel, _) = phone(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, hosts());
    post(
        &mut app,
        &channel,
        &held(json!([
            {"id": "s1", "connection": BOX, "agent": "claude", "reason": "anotherProgram"},
            {"id": "s2", "connection": GPU, "agent": "codex", "reason": "paneGone"},
        ])),
    );
    find(&mut app, "id:home-held-s1");
    for words in [
        "A share wasn't delivered: that pane isn't running claude any more.",
        "A share wasn't delivered: that pane is closed.",
    ] {
        assert!(
            app.screen().shows(words),
            "{words}: {:?}",
            app.screen().texts()
        );
    }
    // Above everything else on the page: it is about something already
    // gone wrong.
    let nodes = app.screen().layout_nodes();
    assert!(
        node(&nodes, "home-held-s1").bounds.origin.y
            < node(&nodes, "home-needs-you").bounds.origin.y
    );

    app.press("id:home-held-send-s1").unwrap();
    assert_eq!(
        next(&heard),
        json!({"ev": "share-send", "connection": BOX, "share": "s1"})
    );
    app.press("id:home-held-discard-s2").unwrap();
    assert_eq!(
        next(&heard),
        json!({"ev": "share-discard", "connection": GPU, "share": "s2"})
    );
    quiet(&heard);

    post(
        &mut app,
        &channel,
        &held(json!([
            {"id": "s1", "connection": BOX, "agent": "claude", "reason": "agentExited"},
        ])),
    );
    app.wait_for_gone("id:home-held-s2", std::time::Duration::from_secs(2))
        .unwrap_or_else(|e| panic!("s2 stayed: {e}"));
    assert!(
        app.screen()
            .shows("A share wasn't delivered: that pane isn't running claude any more.")
    );
    channel.close();

    // In the app's language.
    let (mut app, channel, _) = phone(zh_hans());
    show(&mut app, &channel, hosts());
    post(
        &mut app,
        &channel,
        &held(json!([
            {"id": "s1", "connection": BOX, "agent": "claude", "reason": "anotherProgram"},
        ])),
    );
    find(&mut app, "id:home-held-s1");
    for words in [
        "有 1 个分享没送出去：那个窗格里已经不是 claude 了。",
        "换个窗格…",
    ] {
        assert!(
            app.screen().shows(words),
            "{words}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}
