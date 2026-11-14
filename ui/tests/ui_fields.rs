//! The `ui` layer's form fields in its gallery (`docs/nsh/UI-DESIGN.md`
//! §4.4, R3): every field takes presses over the touch target and shows its
//! label; inline labels start together and their controls end together;
//! from the first accessibility size the control stands under its label; a
//! secret field says what it shows of the secret, an invalid field says its
//! error; Return moves to the next field, a press on a field focuses it,
//! and a disabled one takes no focus.

mod common;

use common::{App, steady};
use moshpit_ui::{
    channel::Channel,
    face,
    screen::{Params, Screen},
    ui::gallery::FIELDS,
};
use nsh_core::{
    Bounds, Points, Size,
    platform::{HostEnvironment, Idiom, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

/// The gallery on an iPhone-sized iOS engine, tall enough that every
/// section is on screen, at `font_scale` (NSH's; `None`: the default size)
/// in `locale`.
fn start_with(font_scale: Option<f32>, locale: Option<&str>) -> App {
    let params = Params {
        locale: locale.map(str::to_owned),
        ..Params::default()
    };
    let channel = Channel::open();
    let mut config = EngineConfig::new(Screen::UiGallery.root(&channel, &params));
    config.viewport = Size::new(Points(402.0), Points(6000.0));
    config.environment.os = Os::Ios;
    config.environment.os_version = (26, 0);
    config.environment.idiom = Idiom::Phone;
    config.environment.pixel_ratio = 3.0;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    HostEnvironment {
        locale: params.nsh_locale(),
        font_scale,
    }
    .apply(&mut config.environment);
    let mut app = Headless::start(config).expect("the engine starts");
    app.wait_for("id:ui-gallery").unwrap();
    steady(&mut app);
    App(app)
}

fn start() -> App {
    start_with(None, None)
}

/// NSH's font scale at the first accessibility size (AX1).
fn ax1() -> f32 {
    (100..400)
        .map(|i| i as f32 / 100.0)
        .find(|&scale| face::category(scale) >= face::AX1)
        .expect("a scale reaches AX1")
}

/// The bounds of the view whose test id is `id`, if it is laid out.
fn find(app: &Headless, id: &str) -> Option<Bounds<Points>> {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(id))
        .map(|node| node.bounds)
}

fn bounds(app: &Headless, id: &str) -> Bounds<Points> {
    find(app, id).unwrap_or_else(|| panic!("no view {id}"))
}

fn right(b: Bounds<Points>) -> f32 {
    b.origin.x.0 + b.size.width.0
}

fn bottom(b: Bounds<Points>) -> f32 {
    b.origin.y.0 + b.size.height.0
}

/// The inline fields (every one but the stacked one).
fn inline() -> impl Iterator<Item = &'static str> {
    FIELDS
        .iter()
        .copied()
        .filter(|id| *id != "ui-gallery-field-stacked")
}

#[test]
fn every_field_takes_presses_over_the_touch_target() {
    let app = start();
    for id in FIELDS {
        let row = bounds(&app, id);
        assert!(
            row.size.width.0 >= 44.0 && row.size.height.0 >= 44.0,
            "{id}: {} × {}",
            row.size.width.0,
            row.size.height.0
        );
        // The row takes the presses and puts them in its control, which
        // stands inside it.
        let input = bounds(&app, &format!("{id}-input"));
        assert!(
            input.origin.y.0 >= row.origin.y.0 && bottom(input) <= bottom(row) + 0.5,
            "{id}: its control is inside it"
        );
        assert!(input.size.width.0 >= 44.0, "{id}");
    }
    // At the default size a one-line field is a medium Row's height: the
    // touch target, insets 8 above and below a body line.
    for id in [
        "ui-gallery-field-empty",
        "ui-gallery-field-filled",
        "ui-gallery-field-number",
        "ui-gallery-field-focused",
        "ui-gallery-field-disabled",
        "ui-gallery-field-secret-empty",
        "ui-gallery-field-secret-saved",
        "ui-gallery-field-secret-replacing",
    ] {
        assert_eq!(bounds(&app, id).size.height.0, 44.0, "{id}");
    }
}

#[test]
fn every_field_shows_its_label() {
    let app = start();
    for id in FIELDS {
        let (row, label) = (bounds(&app, id), bounds(&app, &format!("{id}-label")));
        assert!(
            label.size.width.0 > 0.0 && label.size.height.0 > 0.0,
            "{id}"
        );
        assert!(
            label.origin.x.0 >= row.origin.x.0
                && label.origin.y.0 >= row.origin.y.0
                && bottom(label) <= bottom(row) + 0.5,
            "{id}: its label is inside it"
        );
    }
    for words in ["Name", "Host", "Port", "Proxy port", "mosh-server path"] {
        assert!(app.screen().shows(words), "{words}");
    }
}

#[test]
fn inline_labels_start_together_and_plain_controls_end_together() {
    let app = start();
    let starts: Vec<(&str, f32)> = inline()
        .map(|id| (id, bounds(&app, &format!("{id}-label")).origin.x.0))
        .collect();
    for (id, x) in &starts {
        assert_eq!(*x, starts[0].1, "{id}'s label starts where the others' do");
    }
    // The fields with nothing after their control (no badge, no error).
    let plain = [
        "ui-gallery-field-empty",
        "ui-gallery-field-filled",
        "ui-gallery-field-number",
        "ui-gallery-field-focused",
        "ui-gallery-field-disabled",
        "ui-gallery-field-secret-empty",
    ];
    let ends: Vec<f32> = plain
        .iter()
        .map(|id| right(bounds(&app, &format!("{id}-input"))))
        .collect();
    for (id, end) in plain.iter().zip(&ends) {
        assert!((end - ends[0]).abs() < 0.5, "{id}'s control ends at {end}");
    }
    // At the default size the control is beside its label, the label's
    // column at least 88 pt.
    for id in inline() {
        let (label, input) = (
            bounds(&app, &format!("{id}-label")),
            bounds(&app, &format!("{id}-input")),
        );
        assert!(input.origin.x.0 >= label.origin.x.0 + 88.0, "{id}");
        assert!(
            input.origin.y.0 < bottom(label),
            "{id}: on the label's line"
        );
    }
    // A number keeps to the trailing end, 100 pt wide.
    let number = bounds(&app, "ui-gallery-field-number-input");
    assert_eq!(number.size.width.0, 100.0);
}

#[test]
fn from_the_first_accessibility_size_the_control_stands_under_its_label() {
    let app = start_with(Some(ax1()), None);
    for id in FIELDS {
        let (label, input) = (
            bounds(&app, &format!("{id}-label")),
            bounds(&app, &format!("{id}-input")),
        );
        assert!(
            input.origin.y.0 >= bottom(label) - 0.5,
            "{id}: the control at {} is under the label ending at {}",
            input.origin.y.0,
            bottom(label)
        );
        assert!(input.size.height.0 > 0.0, "{id}");
        assert!(
            bounds(&app, id).size.height.0 > 44.0,
            "{id}: it grew with its text"
        );
    }
}

#[test]
fn a_secret_field_says_what_it_shows_of_the_secret() {
    let app = start();
    for label in [
        "Password",
        "Password, saved, type to replace",
        "Password, replaces saved",
    ] {
        assert!(
            app.screen().matches(&format!("label:{label}")).unwrap(),
            "screen readers hear {label:?}"
        );
    }
    assert!(find(&app, "ui-gallery-field-secret-empty-badge").is_none());
    let saved = bounds(&app, "ui-gallery-field-secret-saved-badge");
    let replacing = bounds(&app, "ui-gallery-field-secret-replacing-badge");
    assert!(saved.size.width.0 > 0.0 && replacing.size.width.0 > 0.0);
    assert!(app.screen().shows("Saved"));
    assert!(app.screen().shows("Replaces saved"));
    // The badge stands after the control, on its line.
    let input = bounds(&app, "ui-gallery-field-secret-saved-input");
    assert!(saved.origin.x.0 >= right(input));
}

#[test]
fn a_secret_field_speaks_chinese_in_chinese() {
    let app = start_with(None, Some("zh-Hans"));
    assert!(app.screen().shows("已保存"));
    assert!(app.screen().shows("替换已保存的"));
    assert!(
        app.screen()
            .matches("label:Password，已保存，输入可替换")
            .unwrap()
    );
}

#[test]
fn an_invalid_field_says_its_error_with_its_name() {
    let app = start();
    assert!(
        app.screen()
            .matches("label:Proxy port, Port must be between 1 and 65535.")
            .unwrap()
    );
    let (input, error) = (
        bounds(&app, "ui-gallery-field-invalid-input"),
        bounds(&app, "ui-gallery-field-invalid-error"),
    );
    assert!(error.origin.y.0 >= bottom(input) - 0.5, "right under it");
    for id in FIELDS
        .iter()
        .filter(|id| **id != "ui-gallery-field-invalid")
    {
        assert!(find(&app, &format!("{id}-error")).is_none(), "{id}");
    }
    // The other fields are said by their names alone.
    assert!(app.screen().matches("label:Port").unwrap());
}

/// Whether the field `id` shows its focus ring.
fn ringed(app: &Headless, id: &str) -> bool {
    find(app, &format!("{id}-ring")).is_some()
}

#[test]
fn return_moves_to_the_next_field_and_the_last_puts_the_keyboard_away() {
    let mut app = start();
    let [first, second, third] = [
        "ui-gallery-field-empty",
        "ui-gallery-field-filled",
        "ui-gallery-field-number",
    ];
    for id in [first, second, third] {
        assert!(!ringed(&app, id), "{id} starts unfocused");
    }
    app.type_text(&format!("id:{first}-input"), "box\n")
        .unwrap();
    assert!(
        !ringed(&app, first) && ringed(&app, second),
        "Return moved on"
    );
    app.type_text(&format!("id:{second}-input"), "\n").unwrap();
    assert!(!ringed(&app, second) && ringed(&app, third));
    app.type_text(&format!("id:{third}-input"), "\n").unwrap();
    assert!(!ringed(&app, third), "the last field's Return puts it away");
    assert_eq!(
        app.screen()
            .input_text(&format!("id:{first}-input"))
            .unwrap(),
        Some("box")
    );
}

#[test]
fn a_press_anywhere_on_a_field_puts_the_keyboard_in_it() {
    let mut app = start();
    assert!(!ringed(&app, "ui-gallery-field-stacked"));
    app.press("id:ui-gallery-field-stacked-label").unwrap();
    assert!(ringed(&app, "ui-gallery-field-stacked"));
}

#[test]
fn a_disabled_field_takes_no_focus() {
    let mut app = start();
    let _ = app.press("id:ui-gallery-field-disabled-label");
    assert!(!ringed(&app, "ui-gallery-field-disabled"));
    let _ = app.type_text("id:ui-gallery-field-disabled-input", "9");
    assert_eq!(
        app.screen()
            .input_text("id:ui-gallery-field-disabled-input")
            .unwrap()
            .unwrap_or("22"),
        "22",
        "nothing is typed into it"
    );
}

#[test]
fn the_galleries_focused_field_shows_its_ring_without_the_keyboard() {
    let app = start();
    assert!(ringed(&app, "ui-gallery-field-focused"));
    let (ring, input) = (
        bounds(&app, "ui-gallery-field-focused-ring"),
        bounds(&app, "ui-gallery-field-focused-input"),
    );
    assert!(
        ring.origin.x.0 < input.origin.x.0 && right(ring) > right(input),
        "the ring stands outside the control"
    );
}
