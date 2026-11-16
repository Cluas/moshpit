//! The `ui` layer's empty state and skeleton (`docs/nsh/UI-DESIGN.md`
//! §4.14, §4.15, R3): the gallery shows every variant, a page's title is a
//! heading and its action large while an inline one's is small, a skeleton
//! is one busy element to screen readers, and the content replaces it —
//! crossfading with full motion, at once with reduced motion, under which
//! the skeleton does not pulse either (§5.2).

mod common;

use std::sync::{Arc, Mutex};

use common::{App, steady};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
    ui::empty::{loaded, skeleton_list},
};
use nsh_core::{
    MountItem,
    animation::AnimationOp,
    element::Root,
    platform::{HostEnvironment, Idiom, Os},
    prelude::*,
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

/// The gallery on an iPhone-sized iOS engine, tall enough that every
/// section is on screen.
fn gallery() -> App {
    gallery_at(None)
}

/// [`gallery`] at text size `font_scale` (the host's; `None`: Large).
fn gallery_at(font_scale: Option<f32>) -> App {
    let params = Params::default();
    let channel = Channel::open();
    let mut config = EngineConfig::new(Screen::UiGallery.root(&channel, &params));
    config.viewport = nsh_core::Size::new(nsh_core::Points(402.0), nsh_core::Points(4000.0));
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

/// The drawn height of the view whose test id is `id`.
fn height(app: &Headless, id: &str) -> f32 {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no view {id}"))
        .bounds
        .size
        .height
        .0
}

#[test]
fn the_gallery_shows_every_empty_state_and_skeleton() {
    let app = gallery();
    for id in [
        "ui-gallery-empty-page",
        "ui-gallery-empty-inline",
        "ui-gallery-skeleton-row",
        "ui-gallery-skeleton-leading",
        "ui-gallery-skeleton-title",
        "ui-gallery-skeleton-list",
    ] {
        assert!(
            app.screen().matches(&format!("id:{id}")).unwrap(),
            "{id} shows"
        );
    }
}

#[test]
fn a_skeleton_row_is_as_tall_as_the_row_it_stands_for() {
    let app = gallery();
    assert_eq!(height(&app, "ui-gallery-skeleton-row"), 60.0, "two lines");
    assert_eq!(height(&app, "ui-gallery-skeleton-title"), 44.0, "one line");
}

#[test]
fn a_page_is_headed_by_its_title_and_its_action_is_large() {
    let app = gallery();
    assert!(
        app.screen()
            .matches("role:header[No connections yet]")
            .unwrap(),
        "the page's title is a heading"
    );
    assert!(
        !app.screen().matches("role:header[No keys yet]").unwrap(),
        "an inline state's is not: it stands in a group"
    );
    assert!(height(&app, "ui-gallery-press-empty") >= 50.0, "large");
    assert_eq!(height(&app, "ui-gallery-press-empty-inline"), 32.0, "small");
}

#[test]
fn the_gallery_swaps_the_skeleton_for_what_arrived() {
    let mut app = gallery();
    assert!(
        !app.screen()
            .matches("id:ui-gallery-loaded-content")
            .unwrap()
    );
    app.press("id:ui-gallery-toggle-loaded").unwrap();
    app.run_for(std::time::Duration::from_millis(400)).unwrap();
    assert!(
        app.screen()
            .matches("id:ui-gallery-loaded-content")
            .unwrap()
    );
    assert!(
        !app.screen().matches("id:ui-gallery-skeleton-list").unwrap(),
        "the skeleton has gone"
    );
    app.press("id:ui-gallery-toggle-loaded").unwrap();
    assert!(
        app.screen().matches("id:ui-gallery-skeleton-list").unwrap(),
        "and comes back while the content is on its way again"
    );
}

/// A skeleton of three rows until Arrive is pressed (or, `at_first`, the
/// content from the start). Arrive is a bare view, so a press animates
/// nothing of its own.
fn body(at_first: bool) -> impl IntoElement {
    let ready = signal(at_first);
    view()
        .test_id("probe")
        .child(
            view()
                .role(AccessibilityRole::Button)
                .on_press(move || ready.set(true))
                .test_id("probe-arrive")
                .child(text("Arrive")),
        )
        .child(loaded(
            move || ready.get(),
            || skeleton_list(3).test_id("probe-skeleton").into_element(),
            || text("Arrived").test_id("probe-content").into_element(),
        ))
}

#[derive(Clone, PartialEq, Props)]
pub struct ProbeProps {
    /// Unused: a component's props need a field.
    pub surface: Prop<u64>,
}

#[component]
fn Waiting(_props: ProbeProps) -> impl IntoElement {
    body(false)
}

#[component]
fn AlreadyThere(_props: ProbeProps) -> impl IntoElement {
    body(true)
}

/// How many animations the engine was told to start, counted from the
/// first mount.
#[derive(Clone, Default)]
struct Starts(Arc<Mutex<usize>>);

impl Starts {
    fn count(&self) -> usize {
        *self.0.lock().unwrap()
    }
}

fn probe(root: Root, reduce_motion: bool) -> (App, Starts) {
    // The app's translations, as a screen's root has them (`t!("Loading")`).
    let mut config = EngineConfig::new(root.translations(moshpit_ui::strings::translations()));
    config.environment.reduce_motion = reduce_motion;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).expect("the engine starts");
    let starts = Starts::default();
    let counted = starts.clone();
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let MountItem::Animation(
                AnimationOp::Start { .. } | AnimationOp::StartComposite { .. },
            ) = item
            {
                *counted.0.lock().unwrap() += 1;
            }
        }
    });
    app.wait_for("id:probe").unwrap();
    steady(&mut app);
    (App(app), starts)
}

fn start_waiting(reduce_motion: bool) -> (App, Starts) {
    probe(
        Root::new::<Waiting>(|| ProbeProps {
            surface: 0u64.into(),
        }),
        reduce_motion,
    )
}

#[test]
fn a_skeleton_is_one_busy_element_to_screen_readers() {
    let (app, _) = start_waiting(false);
    assert_eq!(app.screen().count("label:Loading").unwrap(), 1);
    assert!(
        app.screen()
            .accessibility_state("id:probe-skeleton")
            .unwrap()
            .busy
    );
}

#[test]
fn with_full_motion_the_skeleton_pulses_and_crossfades_with_the_content() {
    let (mut app, starts) = start_waiting(false);
    let pulsing = starts.count();
    assert!(pulsing >= 3, "each row pulses: {pulsing} started");
    app.press("id:probe-arrive").unwrap();
    assert!(
        starts.count() > pulsing,
        "the content fades in and the skeleton fades out"
    );
    app.run_for(std::time::Duration::from_millis(400)).unwrap();
    assert!(app.screen().matches("id:probe-content").unwrap());
    assert!(
        !app.screen().matches("id:probe-skeleton").unwrap(),
        "the skeleton has gone once it faded"
    );
}

#[test]
fn with_reduced_motion_the_skeleton_rests_and_the_content_replaces_it_at_once() {
    let (mut app, starts) = start_waiting(true);
    assert!(app.screen().matches("id:probe-skeleton").unwrap());
    assert_eq!(starts.count(), 0, "no row pulses");
    app.press("id:probe-arrive").unwrap();
    assert!(app.screen().matches("id:probe-content").unwrap());
    assert!(
        !app.screen().matches("id:probe-skeleton").unwrap(),
        "the skeleton went at once, not over the content"
    );
    assert_eq!(starts.count(), 0, "nothing faded");
}

#[test]
fn content_that_is_there_from_the_start_shows_with_no_skeleton() {
    let (app, starts) = probe(
        Root::new::<AlreadyThere>(|| ProbeProps {
            surface: 0u64.into(),
        }),
        false,
    );
    assert!(app.screen().matches("id:probe-content").unwrap());
    assert!(!app.screen().matches("id:probe-skeleton").unwrap());
    assert_eq!(starts.count(), 0, "nothing fades in");
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

/// B839: an empty state's message takes two lines up to the largest
/// standard size, and as many as it needs from AX1, as a row's words do
/// (B753). Two lines cut Home's at AX5 in every language ("Add a machine
/// you can reach over SSH. Agents running there show up here.").
#[test]
fn from_ax1_an_empty_states_message_reads_in_full() {
    let page = "Add a host to keep your agents within reach.";
    let inline = "Generate one on this phone, or import one.";
    let app = gallery();
    assert_eq!(line_limit(&app, page), 2, "a page's, at Large");
    assert_eq!(line_limit(&app, inline), 2, "a group's, at Large");
    let app = gallery_at(Some(2.0));
    assert_eq!(line_limit(&app, page), 0, "a page's, at AX1");
    assert_eq!(line_limit(&app, inline), 0, "a group's, at AX1");
}
