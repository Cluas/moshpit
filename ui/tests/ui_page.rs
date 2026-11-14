//! The `ui` layer's additions for R13/R14 (`docs/nsh/UI-DESIGN.md` §3.2,
//! §4.5, §4.19, §6.10): a row found by an id made at run time, what a row
//! holds under its title (under the title, hidden from screen readers), a
//! 44 pt swatch whose name stays inside a narrow cell, and the page of
//! sections (16 pt under the bar's space, 24 pt between sections).

mod common;

use std::sync::{Arc, Mutex};

use common::{App, steady};
use moshpit_ui::{
    nav_bar::{self, BarMetrics},
    screen::Params,
    ui::{
        page::{BETWEEN, TOP, page},
        row::row,
        section::{Inset, section},
        swatch::swatch,
        tokens::token,
    },
};
use nsh_core::{
    Bounds, MountItem, Points,
    element::Root,
    platform::{HostEnvironment, Idiom, Os},
    prelude::*,
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

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

#[derive(Clone, PartialEq, Props)]
pub struct ProbeProps {
    /// Unused: a component's props need a field.
    pub surface: Prop<u64>,
}

/// The bar the probe's page stands under: iPhone's, 44 pt.
const BAR: BarMetrics = BarMetrics { height: 44.0 };

/// A page of two sections: a theme's row, its id made at run time and its
/// colors under its title; a 44 pt swatch with a long name in a 72 pt cell.
#[component]
fn Probe(_props: ProbeProps) -> impl IntoElement {
    let id = format!("probe-row-{}", "dracula");
    view().flex_1().relative().test_id("probe").child(page(
        BAR,
        nav_bar::ScrollEdge::new(),
        "probe-page",
        [
            section()
                .test_id("probe-first")
                .row(
                    row("Dracula")
                        .below(view().w(px(110.0)).h(px(12.0)).test_id("probe-strip"))
                        .on_press(|| {})
                        .test_id(&id),
                )
                .into_element(),
            section()
                .test_id("probe-second")
                .item(
                    view().w(px(72.0)).test_id("probe-cell").child(
                        swatch("Moshpit Classic Midnight", || token("primary"))
                            .size(44.0)
                            .test_id("probe-swatch"),
                    ),
                    Inset::Content,
                )
                .into_element(),
        ],
    ))
}

/// The probe on an iPhone-sized iOS engine, and the test ids of the views
/// screen readers were told to skip (with all inside them).
fn start() -> (App, Arc<Mutex<Vec<String>>>) {
    let mut config = EngineConfig::new(
        Root::new::<Probe>(|| ProbeProps {
            surface: 0u64.into(),
        })
        .translations(moshpit_ui::strings::translations()),
    );
    config.viewport = nsh_core::Size::new(Points(402.0), Points(874.0));
    config.environment.os = Os::Ios;
    config.environment.idiom = Idiom::Phone;
    config.environment.pixel_ratio = 3.0;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    HostEnvironment {
        locale: Params::default().nsh_locale(),
        font_scale: None,
    }
    .apply(&mut config.environment);
    let mut app = Headless::start(config).expect("the engine starts");
    let hidden = Arc::new(Mutex::new(Vec::new()));
    let seen = hidden.clone();
    app.observe(move |_, batch: &nsh_core::MountBatch| {
        for item in &batch.items {
            if let MountItem::ViewProps { props, .. } = item
                && props.accessibility.hidden
                && let Some(id) = &props.test_id
            {
                seen.lock().unwrap().push(id.to_string());
            }
        }
    });
    app.wait_for("id:probe-row-dracula").unwrap();
    steady(&mut app);
    (App(app), hidden)
}

#[test]
fn a_row_is_found_by_an_id_made_at_run_time() {
    let (mut app, _) = start();
    assert!(app.screen().matches("id:probe-row-dracula").unwrap());
    app.press("id:probe-row-dracula").unwrap();
}

#[test]
fn what_a_row_holds_under_its_title_is_under_it_and_hidden_from_screen_readers() {
    let (app, hidden) = start();
    let title = bounds(&app, "text:Dracula");
    let strip = bounds(&app, "id:probe-strip");
    assert!(
        strip.origin.y.0 >= title.origin.y.0 + title.size.height.0,
        "under the title: {strip:?} {title:?}"
    );
    assert!(
        (strip.origin.x.0 - title.origin.x.0).abs() < 0.5,
        "on its line"
    );
    assert!(
        hidden
            .lock()
            .unwrap()
            .iter()
            .any(|id| id == "probe-row-dracula-below"),
        "{:?}",
        hidden.lock().unwrap()
    );
    // The row says its words, and only those.
    assert!(app.screen().matches("label:Dracula").unwrap());
}

#[test]
fn a_44_pt_swatch_keeps_its_name_inside_a_narrow_cell() {
    let (app, _) = start();
    let circle = bounds(&app, "id:probe-swatch-color");
    assert_eq!(circle.size.width.0, 44.0);
    assert_eq!(circle.size.height.0, 44.0);
    let cell = bounds(&app, "id:probe-cell");
    let name = bounds(&app, "text:Moshpit Classic Midnight");
    assert!(
        name.size.width.0 <= cell.size.width.0 + 0.5 && name.origin.x.0 >= cell.origin.x.0 - 0.5,
        "the name {name:?} stays in its cell {cell:?}"
    );
    let target = bounds(&app, "id:probe-swatch");
    assert!(target.size.width.0 >= 44.0 && target.size.height.0 >= 44.0);
}

#[test]
fn a_page_starts_16_pt_under_the_bar_and_puts_24_pt_between_sections() {
    let (app, _) = start();
    let first = bounds(&app, "id:probe-first");
    let second = bounds(&app, "id:probe-second");
    // No status bar headless: the bar's space is its height.
    assert_eq!(first.origin.y.0, BAR.height + TOP);
    let gap = second.origin.y.0 - (first.origin.y.0 + first.size.height.0);
    assert!((gap - BETWEEN).abs() < 0.01, "{gap}");
    assert_eq!((TOP, BETWEEN), (16.0, 24.0));
    // The page's inset is space-page.
    assert_eq!(first.origin.x.0, 16.0);
    assert_eq!(first.size.width.0, 402.0 - 32.0);
}
