//! The `ui` layer's gallery (`docs/nsh/UI-DESIGN.md` §4.20, R2): its
//! controls take presses over the whole touch target, every icon button
//! has a name, a disabled button takes no press, and a loading button keeps
//! its width and takes no press either. It runs in both appearances.

mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use common::{App, steady};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
    ui::gallery::{ICON_BUTTONS, PRESSABLE},
};
use nsh_core::{
    AccessibilityRole, Bounds, EdgeInsets, EventKind, NodeId, Points, Size,
    platform::{HostEnvironment, Idiom, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

/// The hit slop each view was given, as the engine mounted it; and, for a
/// display-only key or chip, whether it hears presses, tracks its pressed
/// state (an `active:` look, which on iOS needs a press recognizer too) and
/// is hidden from screen readers.
#[derive(Clone, Default)]
struct Slops {
    slops: Arc<Mutex<HashMap<NodeId, EdgeInsets>>>,
    seen: Arc<Mutex<HashMap<NodeId, Seen>>>,
}

/// A view's press listeners, tracked states and accessibility, as mounted.
#[derive(Clone, Copy, Default)]
struct Seen {
    presses: bool,
    tracks_pressed: bool,
    hidden: bool,
    role: AccessibilityRole,
}

impl Slops {
    fn apply(&self, batch: &nsh_core::MountBatch) {
        let mut slops = self.slops.lock().unwrap();
        let mut seen = self.seen.lock().unwrap();
        for item in &batch.items {
            match item {
                nsh_core::MountItem::ViewProps { id, props } => {
                    slops.insert(*id, props.hit_slop);
                    let view = seen.entry(*id).or_default();
                    view.hidden = props.accessibility.hidden;
                    view.role = props.accessibility.role;
                }
                nsh_core::MountItem::Events { id, events } => {
                    seen.entry(*id).or_default().presses = [
                        EventKind::Press,
                        EventKind::LongPress,
                        EventKind::PressDown,
                        EventKind::PressUp,
                    ]
                    .into_iter()
                    .any(|kind| events.contains(kind));
                }
                nsh_core::MountItem::TrackStates { id, states } => {
                    seen.entry(*id).or_default().tracks_pressed =
                        states & nsh_core::tailwind::state::PRESSED != 0;
                }
                _ => {}
            }
        }
    }

    fn of(&self, node: NodeId) -> EdgeInsets {
        self.slops
            .lock()
            .unwrap()
            .get(&node)
            .copied()
            .unwrap_or_default()
    }

    fn seen(&self, node: NodeId) -> Seen {
        self.seen
            .lock()
            .unwrap()
            .get(&node)
            .copied()
            .unwrap_or_default()
    }
}

/// The gallery on an iPhone-sized iOS engine, tall enough that every
/// section is on screen, the hit slops observed from its first mount.
fn start(light: bool) -> (App, Slops) {
    start_with(Params {
        light: Some(light),
        ..Params::default()
    })
}

/// [`start`] with `params`.
fn start_with(params: Params) -> (App, Slops) {
    let channel = Channel::open();
    let mut config = EngineConfig::new(Screen::UiGallery.root(&channel, &params));
    config.viewport = Size::new(Points(402.0), Points(3000.0));
    config.environment.os = Os::Ios;
    config.environment.os_version = (26, 0);
    config.environment.idiom = Idiom::Phone;
    config.environment.pixel_ratio = 3.0;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    HostEnvironment {
        locale: params.nsh_locale(),
        font_scale: params.font_scale,
    }
    .apply(&mut config.environment);
    let mut app = common::headless(config);
    let slops = Slops::default();
    let observed = slops.clone();
    app.observe(move |_, batch| observed.apply(batch));
    app.wait_for("id:ui-gallery").unwrap();
    steady(&mut app);
    (App(app), slops)
}

/// The node and the bounds of the view whose test id is `id`.
fn node(app: &Headless, id: &str) -> (NodeId, Bounds<Points>) {
    let node = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no view {id}"));
    (node.id, node.bounds)
}

/// The gallery says it counted `n` presses.
fn counted(app: &Headless, n: u32) -> bool {
    app.screen().shows(&format!("Presses: {n}"))
}

#[test]
fn every_control_takes_presses_over_the_touch_target() {
    let (mut app, slops) = start(false);
    let mut pressed = 0;
    assert!(counted(&app, 0));
    for id in PRESSABLE {
        let (node, bounds) = node(&app, id);
        let slop = slops.of(node);
        let (w, h) = (
            bounds.size.width.0 + slop.left.0 + slop.right.0,
            bounds.size.height.0 + slop.top.0 + slop.bottom.0,
        );
        assert!(w >= 44.0 && h >= 44.0, "{id}: takes presses over {w} × {h}");
        app.press(&format!("id:{id}")).unwrap();
        pressed += 1;
        assert!(counted(&app, pressed), "{id} counts a press");
    }
}

#[test]
fn every_icon_button_has_a_name() {
    let (app, _) = start(false);
    for (id, _, label, _, _) in ICON_BUTTONS {
        assert!(
            app.screen()
                .matches(&format!("role:button[{label}]"))
                .unwrap(),
            "{id}: screen readers say {label}"
        );
    }
}

#[test]
fn a_disabled_button_takes_no_press() {
    let (mut app, _) = start(false);
    let _ = app.press("id:ui-gallery-disabled");
    assert!(counted(&app, 0));
}

#[test]
fn a_loading_button_keeps_its_width_and_takes_no_press() {
    let (mut app, _) = start(false);
    let (_, at_rest) = node(&app, "ui-gallery-loading");
    app.press("id:ui-gallery-toggle-loading").unwrap();
    assert!(
        app.screen().matches("label:Loading").unwrap(),
        "a spinner shows"
    );
    let (_, loading) = node(&app, "ui-gallery-loading");
    assert_eq!(loading.size, at_rest.size, "the width does not move");
    let _ = app.press("id:ui-gallery-loading");
    assert!(counted(&app, 0), "no press while loading");
    app.press("id:ui-gallery-toggle-loading").unwrap();
    app.press("id:ui-gallery-loading").unwrap();
    assert!(counted(&app, 1), "a press once it is done");
}

#[test]
fn the_gallery_runs_light_and_dark() {
    for light in [false, true] {
        let (app, _) = start(light);
        assert!(app.screen().shows("UI gallery"), "light: {light}");
    }
}

// --- R3: Key, Chip, Swatch ------------------------------------------------

/// Whether screen readers hear the view `id` as selected.
fn selected(app: &Headless, id: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("id:{id}"))
        .unwrap_or_else(|e| panic!("{id}: {e}"))
        .selected
}

#[test]
fn a_key_is_drawn_36_tall_at_least_44_wide_and_a_disabled_one_takes_no_press() {
    let (mut app, slops) = start(false);
    for id in [
        "ui-gallery-key-esc",
        "ui-gallery-key-ctrl",
        "ui-gallery-key-image",
        "ui-gallery-key-keyboard",
    ] {
        let (node, bounds) = node(&app, id);
        assert_eq!(bounds.size.height.0, 36.0, "{id}");
        assert!(bounds.size.width.0 >= 44.0, "{id}: {}", bounds.size.width.0);
        let slop = slops.of(node);
        assert_eq!(slop.top.0 + slop.bottom.0, 8.0, "{id}: 44 pt to the touch");
    }
    let _ = app.press("id:ui-gallery-key-disabled");
    assert!(counted(&app, 0));
}

/// An armed modifier is selected for screen readers, and lets go.
#[test]
fn an_armed_modifier_says_so_and_lets_go() {
    let (mut app, _) = start(false);
    assert!(!selected(&app, "ui-gallery-key-ctrl"));
    app.press("id:ui-gallery-key-ctrl").unwrap();
    assert!(selected(&app, "ui-gallery-key-ctrl"), "armed");
    assert!(app.screen().matches("role:button[⌃]").unwrap());
    app.press("id:ui-gallery-key-ctrl").unwrap();
    assert!(!selected(&app, "ui-gallery-key-ctrl"), "let go");
    assert!(
        !selected(&app, "ui-gallery-key-esc"),
        "a plain key is never armed"
    );
}

#[test]
fn every_symbol_key_has_a_name() {
    let (app, _) = start(false);
    for label in ["Attach image", "Hide keyboard"] {
        assert!(
            app.screen()
                .matches(&format!("role:button[{label}]"))
                .unwrap(),
            "{label}"
        );
    }
}

#[test]
fn a_toggle_chip_is_selected_while_on() {
    let (mut app, slops) = start(false);
    let (_, bounds) = node(&app, "ui-gallery-chip-default");
    assert_eq!(bounds.size.height.0, 28.0);
    let (node, bounds) = node(&app, "ui-gallery-chip-x");
    let slop = slops.of(node);
    assert!(
        bounds.size.width.0 + slop.left.0 + slop.right.0 >= 44.0,
        "a one-letter chip is 44 pt to the touch"
    );
    assert!(selected(&app, "ui-gallery-chip-toggle"));
    app.press("id:ui-gallery-chip-toggle").unwrap();
    assert!(!selected(&app, "ui-gallery-chip-toggle"));
    assert!(
        !selected(&app, "ui-gallery-chip-custom"),
        "custom is a look, not a state"
    );
}

/// Every swatch is named, on screen and to screen readers, and the picked
/// one is selected.
#[test]
fn the_cursor_swatches_are_named_and_one_is_picked() {
    let (mut app, _) = start(false);
    for name in ["Accent", "Blue", "Green", "White"] {
        assert!(
            app.screen().shows(name),
            "{name} is written under its swatch"
        );
        assert!(
            app.screen()
                .matches(&format!("role:button[{name}]"))
                .unwrap(),
            "{name}"
        );
    }
    // The gallery stores `green`; Blue is stored as `teal` (as every
    // build before stored it).
    assert!(selected(&app, "ui-gallery-swatch-green"));
    app.press("id:ui-gallery-swatch-teal").unwrap();
    assert!(selected(&app, "ui-gallery-swatch-teal"));
    assert!(app.screen().shows("Stored: teal"));
    for id in [
        "ui-gallery-swatch-green",
        "ui-gallery-swatch-accent",
        "ui-gallery-swatch-white",
    ] {
        assert!(!selected(&app, id), "{id}");
    }
}

/// A swatch is a 28 pt circle, and 44 pt or more to the touch.
#[test]
fn a_swatch_is_28_round_and_44_to_the_touch() {
    let (app, slops) = start(false);
    for color in ["accent", "teal", "green", "white"] {
        let id = format!("ui-gallery-swatch-{color}");
        let (_, circle) = node(&app, &format!("{id}-color"));
        assert_eq!(
            (circle.size.width.0, circle.size.height.0),
            (28.0, 28.0),
            "{id}"
        );
        let (node, bounds) = node(&app, &id);
        let slop = slops.of(node);
        let (w, h) = (
            bounds.size.width.0 + slop.left.0 + slop.right.0,
            bounds.size.height.0 + slop.top.0 + slop.bottom.0,
        );
        assert!(w >= 44.0 && h >= 44.0, "{id}: {w} × {h}");
    }
}

/// B318 under Terminal Green, whose accent is a green: Green is not shown
/// (Accent is that color), and the stored `green` shows Accent picked
/// while the setting keeps `green`.
#[test]
fn under_terminal_green_the_green_swatch_hides_and_accent_stands_for_it() {
    let theme = serde_json::from_value(serde_json::json!({
        "accent": "#2FA871",
        "accentPressed": "#25865A",
        "screenBG": "#060B08",
        "terminalBG": "#030503",
    }))
    .unwrap();
    for light in [false, true] {
        let (app, _) = start_with(Params {
            theme,
            light: Some(light),
            ..Params::default()
        });
        for name in ["Accent", "Blue", "White"] {
            assert!(
                app.screen()
                    .matches(&format!("role:button[{name}]"))
                    .unwrap(),
                "light {light}: {name}"
            );
        }
        assert!(
            !app.screen().matches("role:button[Green]").unwrap(),
            "light {light}: no Green"
        );
        assert!(selected(&app, "ui-gallery-swatch-accent"), "light {light}");
        assert!(app.screen().shows("Stored: green"), "light {light}");
    }
}

/// A row's detail is a second line under its subtitle, said with the row;
/// a long press reaches the row's own (its press still counts as one).
#[test]
fn a_rows_detail_is_said_and_a_long_press_reaches_it() {
    let (mut app, _) = start(false);
    assert!(app.screen().shows("Used for build-box, gpu-01"));
    assert!(
        app.screen()
            .matches(
                "label:laptop, ED25519, SHA256:Xk3vQ8pL2mN9rT4wY7zB1cD6fG0hJ5sA9fQ, Used for build-box, gpu-01"
            )
            .unwrap(),
        "the badge, after the title, and the detail are part of what screen readers hear"
    );
    let (_, bounds) = node(&app, "ui-gallery-row-detail");
    assert!(bounds.size.height.0 >= 60.0, "{bounds:?}");
    assert!(app.screen().shows("Long presses: 0"));
    app.long_press("id:ui-gallery-row-detail").unwrap();
    assert!(app.screen().shows("Long presses: 1"));
    assert!(counted(&app, 0), "a long press is not a press");
    app.press("id:ui-gallery-row-detail").unwrap();
    assert!(counted(&app, 1));
}

/// A chip that is a symbol says its label; an icon button in a tone of its
/// own still has its name.
#[test]
fn a_symbol_chip_says_its_label_and_toned_icon_buttons_have_names() {
    let (app, _) = start(false);
    assert!(app.screen().matches("role:button[Files]").unwrap());
    assert!(
        !app.screen().shows("Files"),
        "the symbol shows, not the word"
    );
    for label in ["Remove esc from toolbar", "Add ^L to toolbar"] {
        assert!(
            app.screen()
                .matches(&format!("role:button[{label}]"))
                .unwrap(),
            "{label}"
        );
    }
}

/// A key or a chip given no press handler is display only (Settings'
/// preview of the bar, a shortcut's keycap in its row): it hears no press,
/// has no pressed look to track (either would give it a press recognizer
/// on iOS, over the row that takes the touch), and is hidden from screen
/// readers, as the row around it says what it shows. A pressable one
/// beside it still is all three.
#[test]
fn a_key_or_chip_without_a_press_is_display_only() {
    let (app, slops) = start(false);
    for id in ["ui-gallery-key-display", "ui-gallery-chip-display"] {
        let (node, _) = node(&app, id);
        let seen = slops.seen(node);
        assert!(!seen.presses, "{id} hears presses");
        assert!(!seen.tracks_pressed, "{id} tracks its pressed state");
        assert!(seen.hidden, "{id} is an element for screen readers");
        assert_eq!(seen.role, AccessibilityRole::None, "{id}");
        assert_eq!(
            app.screen().press_target(&format!("id:{id}")).unwrap(),
            None,
            "a press on {id} reaches something"
        );
    }
    for id in ["ui-gallery-key-esc", "ui-gallery-chip-default"] {
        let (node, _) = node(&app, id);
        let seen = slops.seen(node);
        assert!(seen.presses && seen.tracks_pressed, "{id}");
        assert!(!seen.hidden, "{id}");
        assert_eq!(seen.role, AccessibilityRole::Button, "{id}");
    }
}

/// How many lines the name under the swatch `id` may take (0: as many as
/// it needs).
fn name_lines(app: &Headless, id: &str) -> u32 {
    let nodes = app.screen().layout_nodes();
    let swatch = nodes
        .iter()
        .find(|n| n.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no view {id}"))
        .id;
    nodes
        .iter()
        .find_map(|n| (n.parent == swatch).then_some(n.text.as_ref()?.text.max_lines))
        .unwrap_or_else(|| panic!("{id} has no name"))
}

/// B840: a swatch's name takes one line up to the largest standard text
/// size, and as many as it needs from AX1, as a row's words do (B753).
/// French "Couleur d'accentuation" was cut on Settings › Terminal at AX5.
#[test]
fn from_ax1_a_swatchs_name_reads_in_full() {
    let (app, _) = start(false);
    assert_eq!(name_lines(&app, "ui-gallery-swatch-accent"), 1, "at Large");
    let (app, _) = start_with(Params {
        light: Some(false),
        font_scale: Some(2.0),
        ..Params::default()
    });
    assert_eq!(name_lines(&app, "ui-gallery-swatch-accent"), 0, "at AX1");
}
