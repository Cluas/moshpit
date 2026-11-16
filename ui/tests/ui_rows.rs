//! The `ui` layer's row, section and system controls
//! (`docs/nsh/UI-DESIGN.md` §4.5–§4.8, R3): the titles of a section start
//! on one line, a row without a leading icon included; values end on one
//! line; separators start at the titles; a label keeps its line while its
//! value gives way (B317); from AX1 the value goes under the title; a
//! choice row is selected; a loading or disabled row takes no press; the
//! switch, the segmented control and the select carry the tokens' colors
//! and report what the user picks.

mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use common::{App, emit, steady};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
    ui::{
        control::switch,
        row::row,
        section::{Inset, section},
    },
};
use nsh_core::{
    Bounds, MountItem, NodeId, Points,
    element::Root,
    platform::{HostEnvironment, Idiom, Os},
    prelude::*,
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;
use serde_json::json;

/// The bounds of the view `selector` names.
fn bounds(app: &Headless, selector: &str) -> Bounds<Points> {
    let id = app.screen().id(selector).unwrap();
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.id == id)
        .unwrap_or_else(|| panic!("{selector} is laid out"))
        .bounds
}

fn right(b: Bounds<Points>) -> f32 {
    b.origin.x.0 + b.size.width.0
}

#[derive(Clone, PartialEq, Props)]
pub struct ProbeProps {
    /// Unused: a component's props need a field.
    pub surface: Prop<u64>,
}

/// A settings-like section: a row with a leading icon, one without, one
/// with a subtitle and a path for its value, one with a switch; and a
/// section of rows none of which has a leading icon.
#[component]
fn Rows(_props: ProbeProps) -> impl IntoElement {
    let on = signal(true);
    view()
        .test_id("probe")
        .p(px(16.0))
        .gap(px(24.0))
        .child(
            section()
                .header("Terminal")
                .row(
                    row("Appearance")
                        .leading_icon("paintbrush")
                        .value("Automatic")
                        .chevron()
                        .on_press(|| {})
                        .test_id("probe-appearance"),
                )
                .row(row("Predictive echo").value("Adaptive"))
                .row(
                    row("Server binary")
                        .subtitle("Where mosh-server is on the host")
                        .value("~/.local/share/mosh/bin/mosh-server-from-a-long-prefix")
                        .mono_value(),
                )
                .row(
                    row("Notifications").trailing(
                        switch(move || on.get(), move |v| on.set(v)).label("Notifications"),
                    ),
                )
                .test_id("probe-lane"),
        )
        .child(
            section()
                .row(row("Font").value("SF Mono"))
                .row(row("Size").value("13 pt"))
                .test_id("probe-plain-lanes"),
        )
        .child(
            section()
                .row(row("Use the host's own mosh-server binary").value("Off"))
                .row(
                    row("Prediction")
                        .value("Adaptive when the network is slow")
                        .whole_value(),
                )
                .row(row("Echo").value("Always").whole_value())
                .test_id("probe-crowded"),
        )
}

fn start_rows(font_scale: Option<f32>) -> App {
    let mut config = EngineConfig::new(
        Root::new::<Rows>(|| ProbeProps {
            surface: 0u64.into(),
        })
        .translations(moshpit_ui::strings::translations()),
    );
    config.viewport = nsh_core::Size::new(Points(402.0), Points(1200.0));
    config.environment.os = Os::Ios;
    config.environment.idiom = Idiom::Phone;
    config.environment.pixel_ratio = 3.0;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    HostEnvironment {
        locale: Params::default().nsh_locale(),
        font_scale,
    }
    .apply(&mut config.environment);
    let mut app = common::headless(config);
    app.wait_for("id:probe").unwrap();
    steady(&mut app);
    App(app)
}

#[test]
fn the_titles_of_a_section_start_on_one_line() {
    let app = start_rows(None);
    let card = bounds(&app, "id:probe-lane");
    let xs: Vec<f32> = [
        "Appearance",
        "Predictive echo",
        "Server binary",
        "Notifications",
    ]
    .iter()
    .map(|t| bounds(&app, &format!("text:{t}")).origin.x.0)
    .collect();
    assert!(
        xs.iter().all(|x| (x - xs[0]).abs() < 0.5),
        "one line for every title, an icon or not: {xs:?}"
    );
    assert!(
        (xs[0] - card.origin.x.0 - Inset::Lead.points()).abs() < 0.5,
        "past the leading lane"
    );
    let plain = bounds(&app, "id:probe-plain-lanes");
    let font = bounds(&app, "text:Font").origin.x.0;
    assert!(
        (font - plain.origin.x.0 - Inset::Content.points()).abs() < 0.5,
        "no lane where no row has an icon"
    );
}

#[test]
fn values_end_on_one_line() {
    let app = start_rows(None);
    let ends: Vec<f32> = ["Automatic", "Adaptive"]
        .iter()
        .map(|t| right(bounds(&app, &format!("text:{t}"))))
        .collect();
    assert!((ends[0] - ends[1]).abs() < 0.5, "{ends:?}");
    let font = right(bounds(&app, "text:SF Mono"));
    let size = right(bounds(&app, "text:13 pt"));
    assert!((font - size).abs() < 0.5);
}

#[test]
fn a_label_keeps_its_line_while_its_value_gives_way() {
    let app = start_rows(None);
    let label = bounds(&app, "text:Server binary");
    let one_line = bounds(&app, "text:Predictive echo").size.height.0;
    assert!(
        (label.size.height.0 - one_line).abs() < 0.5,
        "the label is one line: {} vs {one_line}",
        label.size.height.0
    );
    let value = bounds(
        &app,
        "text:~/.local/share/mosh/bin/mosh-server-from-a-long-prefix",
    );
    let subtitle = bounds(&app, "text:Where mosh-server is on the host");
    assert!(
        value.origin.x.0 > right(label),
        "the value stays beside it: label {label:?} subtitle {subtitle:?} value {value:?}"
    );
    let card = bounds(&app, "id:probe-lane");
    assert!(right(value) <= right(card), "and is shortened to fit");
}

#[test]
fn from_ax1_the_value_goes_under_the_title() {
    let app = start_rows(Some(2.0));
    let title = bounds(&app, "text:Appearance");
    let value = bounds(&app, "text:Automatic");
    assert!(
        value.origin.y.0 >= title.origin.y.0 + title.size.height.0 - 0.5,
        "under: title {title:?}, value {value:?}"
    );
    assert!(
        (value.origin.x.0 - title.origin.x.0).abs() < 0.5,
        "and starts where the title does"
    );
}

#[test]
fn separators_start_at_the_titles() {
    let app = start_rows(None);
    let hairline = 1.0 / 3.0;
    let nodes = app.screen().layout_nodes();
    let inside = |card: Bounds<Points>| {
        nodes
            .iter()
            .filter(|n| (n.bounds.size.height.0 - hairline).abs() < 0.01)
            .filter(|n| {
                let y = n.bounds.origin.y.0;
                y > card.origin.y.0 && y < card.origin.y.0 + card.size.height.0
            })
            .map(|n| n.bounds.origin.x.0 - card.origin.x.0)
            .collect::<Vec<_>>()
    };
    let lane = inside(bounds(&app, "id:probe-lane"));
    assert_eq!(lane.len(), 3, "between four rows: {lane:?}");
    assert!(lane.iter().all(|x| (x - 56.0).abs() < 0.5), "{lane:?}");
    let plain = inside(bounds(&app, "id:probe-plain-lanes"));
    assert_eq!(plain.len(), 1);
    assert!((plain[0] - 16.0).abs() < 0.5, "{plain:?}");
}

#[test]
fn rows_are_at_least_as_tall_as_the_touch_target() {
    let app = start_rows(None);
    for title in ["Appearance", "Predictive echo", "Font"] {
        let id = app.screen().id(&format!("text:{title}")).unwrap();
        // The row is the text's nearest ancestor 44 pt or taller.
        let nodes = app.screen().layout_nodes();
        let text = nodes.iter().find(|n| n.id == id).unwrap().bounds;
        let tallest = nodes
            .iter()
            .filter(|n| {
                n.bounds.origin.y.0 <= text.origin.y.0
                    && n.bounds.origin.y.0 + n.bounds.size.height.0
                        >= text.origin.y.0 + text.size.height.0
                    && n.bounds.size.height.0 < 80.0
            })
            .map(|n| n.bounds.size.height.0)
            .fold(0.0_f32, f32::max);
        assert!(tallest >= 44.0, "{title}: {tallest}");
    }
}

// --- The gallery ----------------------------------------------------------

/// A host component as mounted: its name and its props.
type Native = (String, HashMap<String, serde_json::Value>);

/// The host's components on the gallery: name and props, by node.
#[derive(Clone, Default)]
struct Natives(Arc<Mutex<HashMap<NodeId, Native>>>);

impl Natives {
    fn apply(&self, batch: &nsh_core::MountBatch) {
        let mut natives = self.0.lock().unwrap();
        for item in &batch.items {
            match item {
                MountItem::Component { id, name } => {
                    natives.insert(*id, (name.to_string(), HashMap::new()));
                }
                MountItem::ComponentProps { id, props } => {
                    if let Some((_, current)) = natives.get_mut(id) {
                        for (name, value) in props {
                            match value {
                                Some(value) => current.insert(name.to_string(), value.clone()),
                                None => current.remove(&**name),
                            };
                        }
                    }
                }
                MountItem::Delete { id } => {
                    natives.remove(id);
                }
                _ => {}
            }
        }
    }

    /// The node of the `name` component whose identifier is `id`.
    fn find(&self, name: &str, id: &str) -> NodeId {
        let natives = self.0.lock().unwrap();
        let found: Vec<_> = natives
            .iter()
            .filter(|(_, (n, props))| {
                n == name && props.get("identifier").and_then(|v| v.as_str()) == Some(id)
            })
            .map(|(node, _)| *node)
            .collect();
        assert_eq!(found.len(), 1, "{name} {id}");
        found[0]
    }

    fn prop(&self, node: NodeId, prop: &str) -> serde_json::Value {
        self.0.lock().unwrap()[&node]
            .1
            .get(prop)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }
}

fn gallery() -> (App, Natives) {
    let params = Params::default();
    let channel = Channel::open();
    let mut config = EngineConfig::new(Screen::UiGallery.root(&channel, &params));
    config.viewport = nsh_core::Size::new(Points(402.0), Points(6000.0));
    config.environment.os = Os::Ios;
    config.environment.os_version = (26, 0);
    config.environment.idiom = Idiom::Phone;
    config.environment.pixel_ratio = 3.0;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    HostEnvironment {
        locale: params.nsh_locale(),
        font_scale: None,
    }
    .apply(&mut config.environment);
    let mut app = common::headless(config);
    let natives = Natives::default();
    let observed = natives.clone();
    app.observe(move |_, batch| observed.apply(batch));
    app.wait_for("id:ui-gallery").unwrap();
    steady(&mut app);
    (App(app), natives)
}

fn counted(app: &Headless, n: u32) -> bool {
    app.screen().shows(&format!("Presses: {n}"))
}

#[test]
fn the_gallery_shows_every_row_section_and_control() {
    let (app, _) = gallery();
    for id in [
        "ui-gallery-section-rows",
        "ui-gallery-section-choice",
        "ui-gallery-section-controls",
        "ui-gallery-section-plain",
        "ui-gallery-row-value",
        "ui-gallery-row-toggle",
        "ui-gallery-row-disclosure",
        "ui-gallery-row-loading",
        "ui-gallery-row-disabled",
        // The switch, the segmented control and the select's menu are the
        // host's, found by their `identifier` prop (below), not a view's
        // test id.
    ] {
        assert!(
            app.screen().matches(&format!("id:{id}")).unwrap(),
            "{id} shows"
        );
    }
    assert!(
        app.screen().matches("role:header").unwrap(),
        "a section's header is a heading"
    );
}

#[test]
fn a_choice_row_is_selected_and_follows_the_choice() {
    let (mut app, _) = gallery();
    assert!(
        app.screen()
            .selected("id:ui-gallery-row-check-auto")
            .unwrap()
    );
    assert!(
        !app.screen()
            .selected("id:ui-gallery-row-check-light")
            .unwrap()
    );
    app.press("id:ui-gallery-row-check-light").unwrap();
    assert!(
        app.screen()
            .selected("id:ui-gallery-row-check-light")
            .unwrap()
    );
    assert!(
        !app.screen()
            .selected("id:ui-gallery-row-check-auto")
            .unwrap()
    );
}

#[test]
fn a_loading_or_disabled_row_takes_no_press() {
    let (mut app, _) = gallery();
    assert!(counted(&app, 0));
    let _ = app.press("id:ui-gallery-row-loading");
    let _ = app.press("id:ui-gallery-row-disabled");
    assert!(counted(&app, 0));
    assert!(
        app.screen()
            .matches("id:ui-gallery-row-loading >> label:Loading")
            .unwrap(),
        "a loading row shows a spinner"
    );
}

/// No NSH view of the screen carries test id `id`, shown or not.
fn no_view_carries(app: &Headless, id: &str) -> bool {
    !app.screen()
        .layout_nodes()
        .iter()
        .any(|node| node.test_id.as_deref() == Some(id))
}

/// A host's control is found by the identifier its UIKit control carries;
/// a wrapper view with the same id would send device finders (and screen
/// readers' rotor) to the wrapper (NSH-GAPS #13).
#[test]
fn a_controls_id_is_its_uikit_controls_alone() {
    let (app, natives) = gallery();
    for (name, id) in [
        ("moshpit/switch", "ui-gallery-switch"),
        ("moshpit/segmented", "ui-gallery-segmented"),
        ("moshpit/menu", "ui-gallery-select"),
    ] {
        let node = natives.find(name, id);
        assert_eq!(natives.prop(node, "identifier"), json!(id), "{id}");
        assert!(no_view_carries(&app, id), "{id}: no NSH view carries it");
    }
}

#[test]
fn the_switch_is_primary_and_reports_a_flip() {
    let (mut app, natives) = gallery();
    let node = natives.find("moshpit/switch", "ui-gallery-switch");
    let primary = natives.prop(node, "tint");
    assert!(primary.is_string(), "a tint: {primary}");
    assert_eq!(natives.prop(node, "on"), json!(true));
    assert_eq!(natives.prop(node, "label"), json!("Notifications"));
    emit(&mut app, node, "change", json!(false));
    steady(&mut app);
    assert_eq!(
        natives.prop(node, "on"),
        json!(false),
        "it follows the state"
    );
}

#[test]
fn the_segmented_control_is_in_the_tokens_colors_and_reports_a_pick() {
    let (mut app, natives) = gallery();
    let node = natives.find("moshpit/segmented", "ui-gallery-segmented");
    for prop in ["thumb", "text", "selectedText"] {
        assert!(natives.prop(node, prop).is_string(), "{prop}");
    }
    assert_eq!(
        natives.prop(node, "items"),
        json!(["Block", "Bar", "Underline"])
    );
    assert_eq!(natives.prop(node, "selected"), json!(0));
    emit(&mut app, node, "change", json!(2));
    steady(&mut app);
    assert_eq!(natives.prop(node, "selected"), json!(2));
}

#[test]
fn the_select_checks_the_option_in_use_and_reports_a_pick() {
    let (mut app, natives) = gallery();
    let node = natives.find("moshpit/menu", "ui-gallery-select");
    let items = natives.prop(node, "items");
    let checked: Vec<_> = items
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["checked"] == json!(true))
        .map(|i| i["id"].clone())
        .collect();
    assert_eq!(checked, vec![json!("tmux")]);
    assert_eq!(items[0]["detail"], json!("On almost every host"));
    assert_eq!(natives.prop(node, "label"), json!("Multiplexer, tmux"));
    emit(&mut app, node, "select", json!("herdr"));
    steady(&mut app);
    assert_eq!(natives.prop(node, "label"), json!("Multiplexer, herdr"));
    assert!(app.screen().shows("herdr"), "the row shows the pick");
}

/// A select that waits has no menu to open, and its row dims: a press
/// reaches nothing, and screen readers hear the dimmed row.
#[test]
fn a_waiting_select_has_no_menu_and_dims() {
    let (app, natives) = gallery();
    let menus = natives.0.lock().unwrap();
    let waiting = menus.values().any(|(name, props)| {
        name == "moshpit/menu"
            && props.get("identifier").and_then(|v| v.as_str()) == Some("ui-gallery-select-disabled")
    });
    assert!(!waiting, "a menu over the waiting select");
    drop(menus);
    assert!(app.screen().shows("Waiting"));
}

/// The value is under the title: below it, starting where it starts.
fn under(app: &Headless, title: &str, value: &str) -> bool {
    let (t, v) = (
        bounds(app, &format!("text:{title}")),
        bounds(app, &format!("text:{value}")),
    );
    v.origin.y.0 >= t.origin.y.0 + t.size.height.0 - 0.5
        && (v.origin.x.0 - t.origin.x.0).abs() < 0.5
}

#[test]
fn a_title_past_its_share_puts_the_value_under_it() {
    let app = start_rows(None);
    assert!(
        under(&app, "Use the host's own mosh-server binary", "Off"),
        "a title over 60% of the lane stacks the row"
    );
    assert!(
        !under(&app, "Predictive echo", "Adaptive"),
        "a short title keeps its value beside it"
    );
}

#[test]
fn a_whole_value_that_does_not_fit_goes_under_the_title_unshortened() {
    let app = start_rows(None);
    assert!(under(
        &app,
        "Prediction",
        "Adaptive when the network is slow"
    ));
    let value = bounds(&app, "text:Adaptive when the network is slow");
    let card = bounds(&app, "id:probe-crowded");
    assert!(
        value.origin.x.0 + value.size.width.0 <= card.origin.x.0 + card.size.width.0,
        "it fits on its own line"
    );
    assert!(
        !under(&app, "Echo", "Always"),
        "a whole value that fits stays beside the title"
    );
}

#[test]
fn a_disclosure_row_says_whether_it_is_open() {
    let (mut app, _) = gallery();
    let state = |app: &Headless| {
        app.screen()
            .accessibility_state("id:ui-gallery-row-disclosure")
            .unwrap()
            .expanded
    };
    assert_eq!(state(&app), Some(false));
    assert!(app.screen().matches("role:button[Advanced]").unwrap());
    app.press("id:ui-gallery-row-disclosure").unwrap();
    assert_eq!(state(&app), Some(true));
}

#[test]
fn the_switchs_off_track_is_muted() {
    let (_, natives) = gallery();
    let node = natives.find("moshpit/switch", "ui-gallery-switch");
    let (on, off) = (natives.prop(node, "tint"), natives.prop(node, "offTint"));
    assert!(off.is_string(), "an off track: {off}");
    assert_ne!(on, off, "not the on color");
}

/// How many lines the text saying `words` may take (0: as many as it
/// needs).
fn line_limit(app: &Headless, words: &str) -> u32 {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find_map(|n| {
            n.text
                .filter(|t| t.text.text.as_str() == words)
                .map(|t| t.text.max_lines)
        })
        .unwrap_or_else(|| panic!("no text says {words:?}"))
}

/// B837: from AX1 a value under its title wraps as the title does (B753),
/// not cut at its end: Settings' "Signal Room", "GitHub Dark",
/// "ローカルモデル" lost theirs with the whole lane to themselves. Beside
/// the title it keeps one line (B750, B752 at the default sizes). Below AX1
/// a crowded row also puts its value under the title (B750), and there it
/// keeps one line too: that is the design, not a miss.
#[test]
fn from_ax1_a_value_under_its_title_wraps() {
    let app = start_rows(Some(2.0));
    assert_eq!(line_limit(&app, "Automatic"), 0, "under the title at AX1");
    assert_eq!(
        line_limit(
            &app,
            "~/.local/share/mosh/bin/mosh-server-from-a-long-prefix"
        ),
        0,
        "a mono value (a path) too"
    );
    let app = start_rows(None);
    assert_eq!(line_limit(&app, "Adaptive"), 1, "beside the title");
    assert!(under(&app, "Use the host's own mosh-server binary", "Off"));
    assert_eq!(
        line_limit(&app, "Off"),
        1,
        "under a crowded title below AX1"
    );
}
