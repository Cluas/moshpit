//! R4 (UI-DESIGN §4.18, §5.1–5.2, §8): Home's large title moves with the
//! list as iOS's does. (a) pulled past the top it grows from its leading
//! baseline; (b) scrolled up it shrinks and fades as the inline title comes
//! in and rises, the bar's look over the collapse's last points; (c) let go
//! part way, the list settles at the nearer end, at once with reduced
//! motion. (a) and (b) are bound natively to the list's offset and pull:
//! the headless engine runs no native bindings a test could read, so their
//! ramps are read from the table the bar binds, and the bindings are
//! checked to be there; (c) runs end to end.

mod common;

use common::{post, start_observing_with};
use moshpit_ui::{
    home::{
        Message,
        bar::{BASELINE_UP, FADE_FROM, LARGE_ROW, LargeTitleMotion, at},
        model::{Connection, HomeState},
    },
    screen::{Params, Screen},
    theme::bar_edge,
};
use nsh_core::{Bounds, Command, Points, Size, platform::Idiom};
use nsh_testing::{Headless, LayoutNode};

fn motion() -> LargeTitleMotion {
    LargeTitleMotion {
        collapse: LARGE_ROW,
        appear: bar_edge::APPEAR,
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// (a) and (b) as the bar binds them, at the default text size (a 52 pt
/// collapse, the bar's look over its last 8 pt): §8 R4's points.
#[test]
fn the_large_title_moves_with_the_list() {
    let m = motion();
    // (a) The pull stretches it to 1.08 over 64 pt, no further.
    for (pull, scale) in [(0.0, 1.0), (32.0, 1.04), (64.0, 1.08), (120.0, 1.08)] {
        assert!(close(at(m.stretch(), pull), scale), "pull {pull}");
    }
    // (b) Over the collapse the large title shrinks to 0.96 and fades from
    // 40% of it until its baseline reaches the bar, 17 pt short of the end
    // (under the bar then only its descenders' tips would show, a stray
    // dash); the inline title comes in from 60% of it, rising 4 pt (where
    // T3's bar cut it in over the last 6 pt); the bar's look only once the
    // large title has gone, over the last `bar-edge-appear`.
    // (offset, large scale, large opacity, inline opacity, rise, edge)
    for (offset, scale, opacity, inline, rise, edge) in [
        (0.0, 1.0, 1.0, 0.0, 4.0, 0.0),
        (20.8, 0.984, 1.0, 0.0, 4.0, 0.0), // 40%
        (27.9, 0.978_538, 0.5, 0.0, 4.0, 0.0),
        (31.2, 0.976, 0.267_606, 0.0, 4.0, 0.0), // 60%
        (35.0, 0.973_077, 0.0, 0.182_692, 3.269_231, 0.0), // the baseline at the bar
        (41.6, 0.968, 0.0, 0.5, 2.0, 0.0),       // 80%
        (48.0, 0.963_077, 0.0, 0.807_692, 0.769_231, 0.5),
        (52.0, 0.96, 0.0, 1.0, 0.0, 1.0), // 100%
        (80.0, 0.96, 0.0, 1.0, 0.0, 1.0),
    ] {
        let read = |ramp| at(ramp, offset);
        assert!(close(read(m.shrink()), scale), "scale at {offset}");
        assert!(close(read(m.fade()), opacity), "opacity at {offset}");
        assert!(close(read(m.inline_in()), inline), "inline at {offset}");
        assert!(close(read(m.inline_rise()), rise), "rise at {offset}");
        assert!(close(read(m.edge()), edge), "edge at {offset}");
    }
    // At any text size the large title is gone once its baseline meets the
    // bar.
    for collapse in [LARGE_ROW, 56.0, 60.0] {
        let m = LargeTitleMotion { collapse, ..m };
        assert!(
            close(at(m.fade(), collapse - BASELINE_UP), 0.0),
            "{collapse}"
        );
        assert!(close(at(m.fade(), FADE_FROM * collapse), 1.0), "{collapse}");
    }
    // A pull scrolls nothing up: the collapse's ramps hold at rest.
    assert!(close(at(m.shrink(), -30.0), 1.0) && close(at(m.fade(), -30.0), 1.0));
    assert!(close(at(m.edge(), -30.0), 0.0));
}

/// (c) Where a list at rest settles: half collapsed or less back to the
/// top, more to the collapse; nothing outside it.
#[test]
fn a_list_let_go_part_way_settles_at_the_nearer_end() {
    let m = motion();
    for (offset, settles) in [
        (-12.0, None),
        (0.0, None),
        (10.0, Some(0.0)),
        (25.9, Some(0.0)),
        (26.0, Some(52.0)),
        (51.0, Some(52.0)),
        (52.0, None),
        (300.0, None),
    ] {
        assert_eq!(m.settle(offset), settles, "at {offset}");
    }
}

/// A phone's width (the iPhone 16 Pro's).
const PHONE: f32 = 402.0;

/// Home with a dozen saved hosts on a phone's height, so the list scrolls:
/// `width` wide, on an iPhone at a phone's width (its 44 pt bar), an iPad
/// otherwise.
fn home(width: f32, reduced: bool) -> (common::App, std::sync::Arc<moshpit_ui::channel::Channel>) {
    let (mut app, channel, _) =
        start_observing_with(Screen::Home, Params::default(), "home", move |config| {
            config.viewport = Size::new(Points(width), Points(700.0));
            if width <= PHONE {
                config.environment.idiom = Idiom::Phone;
            }
            config.environment.reduce_motion = reduced;
        });
    let state = HomeState {
        connections: (0..12)
            .map(|i| Connection {
                id: format!("c{i}"),
                name: format!("host-{i}"),
                initial: "H".into(),
                user: "dev".into(),
                host: format!("10.0.0.{i}"),
                ..Connection::default()
            })
            .collect(),
        ..HomeState::default()
    };
    post(
        &mut app,
        &channel,
        &Message::Home(Box::new(state)).to_json(),
    );
    app.wait_for("id:connection-card-host-11").unwrap();
    (app, channel)
}

/// The list's `scroll_to`s so far: where to, animated or not.
fn settles(app: &Headless) -> Vec<(f32, bool)> {
    app.screen()
        .commands()
        .iter()
        .filter_map(|(_, command)| match command {
            Command::ScrollTo { y, animated, .. } => Some((y.0, *animated)),
            _ => None,
        })
        .collect()
}

/// (c) on Home: a drag let go part way through the collapse settles at the
/// nearer end, animated; with reduced motion, at once; past it, not at all.
#[test]
fn home_settles_its_large_title() {
    for reduced in [false, true] {
        let (mut app, channel) = home(PHONE, reduced);
        // The bindings (a) and (b) are there either way: they follow the
        // finger, they are not an animation.
        for id in [
            "nav-bar-large-stretch",
            "nav-bar-large-shrink",
            "nav-bar-inline-title",
        ] {
            assert!(app.screen().matches(&format!("id:{id}")).unwrap(), "{id}");
        }
        app.scroll("id:home-list", Points(40.0)).unwrap();
        common::steady(&mut app);
        assert_eq!(
            settles(&app),
            [(LARGE_ROW, !reduced)],
            "past half: collapsed"
        );
        app.scroll("id:home-list", Points(-30.0)).unwrap();
        common::steady(&mut app);
        assert_eq!(
            settles(&app).last(),
            Some(&(0.0, !reduced)),
            "under half: back to the top"
        );
        let before = settles(&app).len();
        app.scroll("id:home-list", Points(200.0)).unwrap();
        common::steady(&mut app);
        assert_eq!(
            settles(&app).len(),
            before,
            "past the collapse: left where it is"
        );
        channel.close();
    }
}

/// A view's frame by its test id.
fn frame(nodes: &[LayoutNode], id: &str) -> Bounds<Points> {
    nodes
        .iter()
        .find(|n| n.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no {id}"))
        .bounds
}

fn bottom(b: Bounds<Points>) -> f32 {
    b.origin.y.0 + b.size.height.0
}

/// §6.1, §8 R4: the large title starts on the cards' outer edge (the
/// page's margin on a phone; the list's centered column on a wide window),
/// not on the text in them.
#[test]
fn the_large_title_starts_on_the_cards_edge() {
    for width in [PHONE, 1024.0] {
        let (app, channel) = home(width, false);
        let nodes = app.screen().layout_nodes();
        let title = nodes
            .iter()
            .find(|n| n.test_id.as_deref() == Some("nav-bar-title"))
            .and_then(|n| n.text.as_ref())
            .expect("the large title's text")
            .content
            .origin
            .x
            .0;
        let card = frame(&nodes, "home-head-c0").origin.x.0;
        assert!(
            (title - card).abs() < 0.5,
            "{width} wide: the title at {title}, the card at {card}"
        );
        if width == PHONE {
            assert!((title - 16.0).abs() < 0.5, "space-page on a phone: {title}");
        }
        channel.close();
    }
}

/// The texts in Home's list, top first.
fn list_texts(nodes: &[LayoutNode]) -> Vec<f32> {
    let list = nodes
        .iter()
        .find(|n| n.test_id.as_deref() == Some("home-list"))
        .expect("the list")
        .id;
    let inside = |node: &LayoutNode| {
        let mut parent = node.parent;
        loop {
            if parent == list {
                return true;
            }
            match nodes.iter().find(|n| n.id == parent) {
                Some(up) => parent = up.parent,
                None => return false,
            }
        }
    };
    let mut tops: Vec<f32> = nodes
        .iter()
        .filter(|n| n.text.is_some() && inside(n))
        .map(|n| n.bounds.origin.y.0)
        .collect();
    tops.sort_by(f32::total_cmp);
    tops
}

/// §8 R4 (B316): while the large title collapses no row's text reaches the
/// bar's row (the inline title, the buttons): at rest the list starts under
/// the large title's 52 pt row, right under the 44 pt bar; collapsed, its
/// first text is still under the bar. Past the collapse rows go under the
/// bar, which by then has its look (the edge's ramp is 1 there).
#[test]
fn no_row_reaches_the_bar_while_the_title_collapses() {
    let (mut app, channel) = home(PHONE, false);
    let nodes = app.screen().layout_nodes();
    // The inline title is centered in the bar's row: its box is the row.
    let row = nodes
        .iter()
        .find(|n| n.test_id.as_deref() == Some("nav-bar-inline-title"))
        .and_then(|n| nodes.iter().find(|p| p.id == n.parent))
        .and_then(|n| nodes.iter().find(|p| p.id == n.parent))
        .expect("the bar's row")
        .bounds;
    assert!(
        (row.size.height.0 - 44.0).abs() < 0.5,
        "the bar's row: {row:?}"
    );
    let bar = bottom(row);
    let large = frame(&nodes, "nav-bar-large-stretch");
    assert!(
        (bottom(large) - (bar + LARGE_ROW)).abs() < 0.5,
        "the large title's row ends {LARGE_ROW} pt under the bar: {large:?}, the bar at {bar}"
    );
    let first = list_texts(&nodes)[0];
    assert!(
        first >= bar + LARGE_ROW - 0.5,
        "at rest: a row's text at {first}"
    );

    // Scrolled to the collapse's end (no settling there).
    app.scroll("id:home-list", Points(LARGE_ROW)).unwrap();
    common::steady(&mut app);
    assert!(settles(&app).is_empty(), "{:?}", settles(&app));
    let first = list_texts(&app.screen().layout_nodes())[0];
    assert!(
        first >= bar - 0.5,
        "collapsed: a row's text at {first}, the bar ends at {bar}"
    );
    channel.close();
}
