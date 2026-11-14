//! The `ui` layer's banner and toast (`docs/nsh/UI-DESIGN.md` §4.16, R3):
//! the gallery shows every banner; a banner's action and close take
//! presses over the whole touch target and the close has a name; a banner
//! told to go fades out first. A toast has a popup's surface, is read out,
//! ends the one showing, and goes once it has stood its four seconds. With
//! full motion a banner and a toast come in moving; with reduced motion
//! they are simply there and simply go (§5.2).

mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use common::{App, steady};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
    ui::notice::{BannerTone, DWELL_MS, banner, toasts},
};
use nsh_core::{
    Command, EdgeInsets, MountItem, NodeId, ViewStyle,
    animation::AnimationOp,
    element::Root,
    platform::{HostEnvironment, Idiom, Os},
    prelude::*,
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

/// What the engine mounted, as the tests read it: each view's hit slop and
/// styles (in order), and how many animations it was told to start.
#[derive(Clone, Default)]
struct Seen {
    slops: Arc<Mutex<HashMap<NodeId, EdgeInsets>>>,
    styles: Arc<Mutex<HashMap<NodeId, Vec<ViewStyle>>>>,
    starts: Arc<Mutex<usize>>,
}

impl Seen {
    fn apply(&self, batch: &nsh_core::MountBatch) {
        for item in &batch.items {
            match item {
                MountItem::ViewProps { id, props } => {
                    self.slops.lock().unwrap().insert(*id, props.hit_slop);
                }
                MountItem::ViewStyle { id, style } => {
                    self.styles
                        .lock()
                        .unwrap()
                        .entry(*id)
                        .or_default()
                        .push((**style).clone());
                }
                MountItem::Animation(
                    AnimationOp::Start { .. } | AnimationOp::StartComposite { .. },
                ) => *self.starts.lock().unwrap() += 1,
                _ => {}
            }
        }
    }

    fn starts(&self) -> usize {
        *self.starts.lock().unwrap()
    }

    /// The styles `selector`'s view was given, in order.
    fn styles(&self, app: &Headless, selector: &str) -> Vec<ViewStyle> {
        let id = app.screen().id(selector).unwrap();
        self.styles
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .unwrap_or_default()
    }

    /// The size `selector`'s view takes presses over: its bounds and its
    /// hit slop.
    fn reach(&self, app: &Headless, selector: &str) -> (f32, f32) {
        let id = app.screen().id(selector).unwrap();
        let bounds = app
            .screen()
            .layout_nodes()
            .into_iter()
            .find(|node| node.id == id)
            .unwrap_or_else(|| panic!("{selector} is laid out"))
            .bounds;
        let slop = self
            .slops
            .lock()
            .unwrap()
            .get(&id)
            .copied()
            .unwrap_or_default();
        (
            bounds.size.width.0 + slop.left.0 + slop.right.0,
            bounds.size.height.0 + slop.top.0 + slop.bottom.0,
        )
    }
}

#[test]
fn the_gallery_shows_every_banner() {
    let params = Params::default();
    let channel = Channel::open();
    let mut config = EngineConfig::new(Screen::UiGallery.root(&channel, &params));
    config.viewport = nsh_core::Size::new(nsh_core::Points(402.0), nsh_core::Points(5000.0));
    config.environment.os = Os::Ios;
    config.environment.os_version = (26, 0);
    config.environment.idiom = Idiom::Phone;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    HostEnvironment {
        locale: params.nsh_locale(),
        font_scale: None,
    }
    .apply(&mut config.environment);
    let mut app = Headless::start(config).expect("the engine starts");
    app.wait_for("id:ui-gallery").unwrap();
    steady(&mut app);
    let mut app = App(app);
    for id in [
        "ui-gallery-banner-info",
        "ui-gallery-banner-warning",
        "ui-gallery-banner-destructive",
        "ui-gallery-banner-action",
        "ui-gallery-banner-close",
        "ui-gallery-toast-show",
    ] {
        assert!(
            app.screen().matches(&format!("id:{id}")).unwrap(),
            "{id} shows"
        );
    }
    app.press("id:ui-gallery-toast-show").unwrap();
    assert!(app.screen().matches("id:ui-gallery-toast").unwrap());
    app.press("id:ui-gallery-banner-close >> role:button[Close]")
        .unwrap();
    app.run_for(Duration::from_millis(400)).unwrap();
    assert!(!app.screen().matches("id:ui-gallery-banner-close").unwrap());
    app.press("id:ui-gallery-show-banner").unwrap();
    assert!(app.screen().matches("id:ui-gallery-banner-close").unwrap());
}

#[derive(Clone, PartialEq, Props)]
pub struct ProbeProps {
    /// Unused: a component's props need a field.
    pub surface: Prop<u64>,
}

/// A bare view to press, which animates nothing of its own.
fn pressable(id: &'static str, words: &'static str, on_press: impl Fn() + 'static) -> View {
    view()
        .role(AccessibilityRole::Button)
        .on_press(on_press)
        .test_id(id)
        .child(text(words))
}

/// A banner Hide and Show take away and bring back, a banner with an
/// action and a close, and toasts (Copy, Copy link).
#[component]
fn Notices(_props: ProbeProps) -> impl IntoElement {
    let visible = signal(true);
    let closed = signal(0_u32);
    let retried = signal(0_u32);
    let toasts = toasts();
    view()
        .test_id("probe")
        .flex_1()
        .child(pressable("probe-hide", "Hide", move || visible.set(false)))
        .child(pressable("probe-show", "Show", move || visible.set(true)))
        .child(pressable("probe-copy", "Copy", move || {
            toasts.show("Copied")
        }))
        .child(pressable("probe-copy-link", "Copy link", move || {
            toasts.show("Link copied")
        }))
        .child(text(move || {
            format!("Closed: {} Retried: {}", closed.get(), retried.get())
        }))
        .child(
            banner(BannerTone::Info, || "Reconnecting…".into())
                .shown(move || visible.get())
                .test_id("probe-banner"),
        )
        .child(
            banner(BannerTone::Destructive, || "The session ended".into())
                .action("Reconnect", move || retried.update(|n| *n += 1))
                .on_close(move || closed.update(|n| *n += 1))
                .test_id("probe-banner-actions"),
        )
        .child(toasts.host().test_id("probe-toast"))
}

fn start_notices(reduce_motion: bool) -> (App, Seen) {
    // The app's translations, as a screen's root has them (`t!("Close")`).
    let root = Root::new::<Notices>(|| ProbeProps {
        surface: 0u64.into(),
    })
    .translations(moshpit_ui::strings::translations());
    let mut config = EngineConfig::new(root);
    config.viewport = nsh_core::Size::new(nsh_core::Points(402.0), nsh_core::Points(874.0));
    config.environment.reduce_motion = reduce_motion;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).expect("the engine starts");
    let seen = Seen::default();
    let observed = seen.clone();
    app.observe(move |_, batch| observed.apply(batch));
    app.wait_for("id:probe").unwrap();
    steady(&mut app);
    (App(app), seen)
}

#[test]
fn a_banners_action_and_close_take_presses_over_the_touch_target() {
    let (mut app, seen) = start_notices(false);
    for selector in ["role:button[Reconnect]", "role:button[Close]"] {
        let (w, h) = seen.reach(&app, selector);
        assert!(
            w >= 44.0 && h >= 44.0,
            "{selector}: takes presses over {w} × {h}"
        );
    }
    app.press("role:button[Reconnect]").unwrap();
    app.press("role:button[Close]").unwrap();
    assert!(app.screen().shows("Closed: 1 Retried: 1"), "both are heard");
}

#[test]
fn with_full_motion_a_banner_slides_in_and_fades_out_before_it_goes() {
    let (mut app, seen) = start_notices(false);
    let entered = seen.starts();
    assert!(entered >= 2, "both banners come in: {entered} started");
    app.press("id:probe-hide").unwrap();
    assert!(seen.starts() > entered, "it fades out");
    app.run_for(Duration::from_millis(400)).unwrap();
    assert!(
        !app.screen().matches("id:probe-banner").unwrap(),
        "then goes"
    );
    app.press("id:probe-show").unwrap();
    assert!(app.screen().matches("id:probe-banner").unwrap());
}

#[test]
fn with_reduced_motion_a_banner_is_simply_there_and_simply_goes() {
    let (mut app, seen) = start_notices(true);
    assert!(app.screen().matches("id:probe-banner").unwrap());
    app.press("id:probe-hide").unwrap();
    assert!(
        !app.screen().matches("id:probe-banner").unwrap(),
        "gone at once"
    );
    assert_eq!(seen.starts(), 0, "nothing moved");
}

#[test]
fn a_toast_has_a_popups_surface() {
    let (mut app, seen) = start_notices(false);
    app.press("id:probe-copy").unwrap();
    let styles = seen.styles(&app, "id:probe-toast");
    let last = styles.last().expect("the toast is styled");
    assert!(last.background_color.0 != 0, "a fill (popover)");
    assert_eq!(last.borders.widths.top.0, 0.5, "a hairline");
    assert!(last.borders.colors[0].0 != 0, "in border-strong");
    assert!(last.effects.is_some(), "and the large shadow");
}

#[test]
fn a_toast_is_read_out_and_goes_once_it_stood_its_time() {
    let (mut app, _) = start_notices(false);
    app.press("id:probe-copy").unwrap();
    assert!(app.screen().shows("Copied"));
    assert!(
        app.screen().sent(&Command::Announce("Copied".into())),
        "the screen reader reads it"
    );
    app.run_for(Duration::from_millis(DWELL_MS - 500)).unwrap();
    assert!(app.screen().shows("Copied"), "it stands its time");
    app.run_for(Duration::from_millis(1500)).unwrap();
    assert!(!app.screen().matches("id:probe-toast").unwrap(), "gone");
}

#[test]
fn a_new_toast_ends_the_one_showing_and_stands_its_own_time() {
    let (mut app, _) = start_notices(false);
    app.press("id:probe-copy").unwrap();
    app.run_for(Duration::from_millis(3000)).unwrap();
    app.press("id:probe-copy-link").unwrap();
    app.run_for(Duration::from_millis(500)).unwrap();
    assert!(app.screen().shows("Link copied"));
    assert!(!app.screen().shows("Copied"), "the first has gone");
    assert_eq!(app.screen().count("id:probe-toast").unwrap(), 1);
    app.run_for(Duration::from_millis(1500)).unwrap();
    assert!(
        app.screen().shows("Link copied"),
        "the first toast's time does not end the second"
    );
    app.run_for(Duration::from_millis(DWELL_MS)).unwrap();
    assert!(!app.screen().matches("id:probe-toast").unwrap());
}

#[test]
fn with_full_motion_a_toast_rises_in() {
    let (mut app, seen) = start_notices(false);
    app.press("id:probe-copy").unwrap();
    let styles = seen.styles(&app, "id:probe-toast");
    let first = styles.first().expect("the toast is styled");
    assert_eq!(first.opacity, 0.0, "it starts unseen");
    assert!(!first.transform.is_identity(), "and lower");
    let last = styles.last().unwrap();
    assert_eq!(last.opacity, 1.0);
    assert!(last.transform.is_identity(), "then is in place");
}

#[test]
fn with_reduced_motion_a_toast_is_simply_there_and_simply_goes() {
    let (mut app, seen) = start_notices(true);
    app.press("id:probe-copy").unwrap();
    assert!(app.screen().shows("Copied"));
    for style in seen.styles(&app, "id:probe-toast") {
        assert_eq!(style.opacity, 1.0, "never faded");
        assert!(style.transform.is_identity(), "never moved");
    }
    assert_eq!(seen.starts(), 0, "nothing animated");
    app.run_for(Duration::from_millis(DWELL_MS + 200)).unwrap();
    assert!(
        !app.screen().matches("id:probe-toast").unwrap(),
        "and goes at its time"
    );
}
