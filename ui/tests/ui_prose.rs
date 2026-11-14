//! R15 and R16's pieces of the `ui` layer (`docs/nsh/UI-DESIGN.md` §4.4,
//! §4.16, §4.18): a search field is named by what it searches, takes the
//! touch target and rings itself while it has the keyboard; a textarea
//! starts four lines tall and grows with its text, to twelve; a banner's
//! action is found by its own id; a bar's text button that is busy shows a
//! spinner at its width, takes no press and is heard busy.

mod common;

use common::{App, steady};
use moshpit_ui::{
    channel::Channel,
    nav_bar::{BarButton, BarContent, BarMetrics, NavBar, ScrollEdge, fixed, nav_bar},
    screen::{Params, Screen},
};
use nsh_core::{
    Bounds, Points, Size,
    element::Root,
    platform::{HostEnvironment, Idiom, Os},
    prelude::*,
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

/// The gallery on an iPhone-sized iOS engine, tall enough that every
/// section is on screen.
fn gallery() -> App {
    let params = Params::default();
    let channel = Channel::open();
    let mut config = EngineConfig::new(Screen::UiGallery.root(&channel, &params));
    config.viewport = Size::new(Points(402.0), Points(8000.0));
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
    let mut app = Headless::start(config).expect("the engine starts");
    app.wait_for("id:ui-gallery").unwrap();
    steady(&mut app);
    App(app)
}

fn bounds(app: &Headless, id: &str) -> Bounds<Points> {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no view {id}"))
        .bounds
}

fn counted(app: &Headless, n: u32) -> bool {
    app.screen().shows(&format!("Presses: {n}"))
}

#[test]
fn a_search_field_is_named_by_what_it_searches_and_rings_while_it_has_the_keyboard() {
    let mut app = gallery();
    assert!(
        app.screen().matches("label:Search languages").unwrap(),
        "screen readers hear its placeholder as its name"
    );
    assert!(!app.screen().matches("id:ui-gallery-search-ring").unwrap());
    app.type_text("id:ui-gallery-search", "deu").unwrap();
    assert_eq!(
        app.screen().input_text("id:ui-gallery-search").unwrap(),
        Some("deu")
    );
    let ring = bounds(&app, "ui-gallery-search-ring");
    assert!(
        ring.size.height.0 >= 44.0 + 2.0 * 3.0 - 0.5,
        "the ring stands outside a box the touch target tall: {}",
        ring.size.height.0
    );
}

#[test]
fn a_textarea_starts_four_lines_tall_and_grows_with_its_text_to_twelve() {
    let mut app = gallery();
    let id = "ui-gallery-field-textarea-input";
    let start = bounds(&app, id).size.height.0;
    assert!(start >= 4.0 * 17.0, "four body lines at least: {start}");
    app.type_text(&format!("id:{id}"), &"a line\n".repeat(30))
        .unwrap();
    let full = bounds(&app, id).size.height.0;
    assert!(full > start, "it grew: {start} → {full}");
    assert!(
        full <= 12.0 * 24.0 + 8.0,
        "twelve lines at most, then it scrolls: {full}"
    );
}

#[test]
fn a_banners_action_is_found_by_its_own_id() {
    let mut app = gallery();
    assert!(counted(&app, 0));
    app.press("id:ui-gallery-banner-take-over").unwrap();
    assert!(counted(&app, 1));
}

#[derive(Clone, PartialEq, Props)]
pub struct ProbeProps {
    /// Unused: a component's props need a field.
    pub surface: Prop<u64>,
}

/// A bar whose Send is busy while Busy is on, and the sends it counted.
#[component]
fn Sending(_props: ProbeProps) -> impl IntoElement {
    let busy = signal(false);
    let sent = signal(0_u32);
    view()
        .test_id("probe")
        .flex_1()
        .child(nav_bar(NavBar {
            metrics: BarMetrics::standard(Idiom::Phone),
            back_title: String::new(),
            title: fixed("Feedback".to_owned()),
            leading: None,
            trailing: vec![
                BarButton {
                    label: "Send".to_owned(),
                    content: BarContent::Text,
                    enabled: computed(|| true),
                    emphasized: true,
                    test_id: "probe-send",
                    on_press: Box::new(move || sent.update(|n| *n += 1)),
                    menu: None,
                }
                .busy(computed(move || busy.get())),
            ],
            on_back: Box::new(|| {}),
            scrolled: ScrollEdge::new(),
        }))
        .child(
            view()
                .mt(px(120.0))
                .role(AccessibilityRole::Button)
                .on_press(move || busy.update(|b| *b = !*b))
                .test_id("probe-busy")
                .child(text("Busy")),
        )
        .child(text(move || format!("Sent: {}", sent.get())))
}

fn start_sending() -> App {
    let mut config = EngineConfig::new(
        Root::new::<Sending>(|| ProbeProps {
            surface: 0u64.into(),
        })
        .translations(moshpit_ui::strings::translations()),
    );
    config.viewport = Size::new(Points(402.0), Points(874.0));
    config.environment.os = Os::Ios;
    config.environment.idiom = Idiom::Phone;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).expect("the engine starts");
    app.wait_for("id:probe").unwrap();
    steady(&mut app);
    App(app)
}

#[test]
fn a_busy_bar_button_keeps_its_width_takes_no_press_and_is_heard_busy() {
    let mut app = start_sending();
    let state = |app: &Headless| app.screen().accessibility_state("id:probe-send").unwrap();
    assert!(!state(&app).busy);
    let idle = bounds(&app, "probe-send");
    app.press("id:probe-send").unwrap();
    assert!(app.screen().shows("Sent: 1"));
    app.press("id:probe-busy").unwrap();
    assert!(state(&app).busy, "screen readers hear it busy");
    assert!(!app.screen().enabled("id:probe-send").unwrap());
    assert_eq!(
        bounds(&app, "probe-send").size,
        idle.size,
        "the width holds"
    );
    let _ = app.press("id:probe-send");
    assert!(app.screen().shows("Sent: 1"), "no press while busy");
    app.press("id:probe-busy").unwrap();
    app.press("id:probe-send").unwrap();
    assert!(app.screen().shows("Sent: 2"), "a press once it is done");
}
