//! B366: a screen's body behind the error boundary (`moshpit_ui::boundary`),
//! and what a screen whose state never lands shows. Before it, a body that
//! panicked after its state came showed nothing under the bar, and said so
//! only on stderr; a phone keeps that nowhere.

mod common;

use std::{
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use common::*;
use moshpit_ui::{
    boundary,
    channel::Channel,
    screen::{Params, Screen},
};
use nsh_core::{element::Root, prelude::*};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

/// What this test binary logged, through the `log` facade.
fn logged() -> &'static Mutex<Vec<String>> {
    static LINES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    static LOGGER: Capture = Capture;
    struct Capture;
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, r: &log::Record<'_>) {
            logged()
                .lock()
                .unwrap()
                .push(format!("[{}] {}: {}", r.target(), r.level(), r.args()));
        }
        fn flush(&self) {}
    }
    let lines = LINES.get_or_init(Mutex::default);
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let _ = log::set_logger(&LOGGER);
        log::set_max_level(log::LevelFilter::Trace);
    });
    lines
}

fn ids(app: &Headless) -> Vec<String> {
    app.screen()
        .layout_nodes()
        .into_iter()
        .filter_map(|n| n.test_id.map(|t| t.to_string()))
        .collect()
}

fn has(app: &Headless, id: &str) -> bool {
    ids(app).iter().any(|t| t == id)
}

#[derive(Clone, PartialEq, Props)]
pub struct ProbeProps {
    pub surface: Prop<u64>,
}

/// A screen as the redesigned ones are made: a bar beside a body that
/// shows once the state came; the body panics when it is made.
#[component]
pub fn ProbeScreen(props: ProbeProps) -> impl IntoElement {
    let channel = Channel::find(props.surface.get_untracked());
    let tell: boundary::Tell = Rc::new(move |event| {
        if let Some(channel) = &channel {
            channel.tell(&event);
        }
    });
    // Stands in for the host's first post arriving a beat later.
    let ready = signal(false);
    effect(move || ready.set(true));
    view()
        .flex_1()
        .relative()
        .test_id("probe")
        .child(boundary::ready_body(
            move || ready.get(),
            || -> AnyElement { panic!("probe: the body failed") },
            tell,
        ))
        .child(view().test_id("probe-bar").child(text("Bar")))
}

/// `root` in the app's tokens and words, as `Screen::root` dresses a screen.
fn dressed(root: Root) -> Root {
    root.tokens(moshpit_ui::theme::tokens(
        &moshpit_ui::theme::AppTheme::default(),
    ))
    .translations(moshpit_ui::strings::translations())
}

fn probe() -> (App, Arc<Channel>, std::sync::mpsc::Receiver<String>) {
    logged();
    let channel = Channel::open();
    let rx = hear(&channel);
    let id = channel.id();
    let root = dressed(Root::new::<ProbeScreen>(move || ProbeProps {
        surface: id.into(),
    }));
    let mut config = EngineConfig::new(root);
    config.viewport = nsh_core::Size::new(nsh_core::Points(390.0), nsh_core::Points(844.0));
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).expect("the engine starts");
    app.wait_for("id:probe").unwrap();
    steady(&mut app);
    (App(app), channel, rx)
}

#[test]
fn a_body_that_fails_says_so_tells_the_host_and_offers_retry() {
    let (mut app, channel, rx) = probe();
    assert!(has(&app, "probe-bar"), "the bar stays");
    assert!(
        has(&app, "page-failed"),
        "the failure shows: {:?}",
        ids(&app)
    );
    assert!(has(&app, "page-failed-retry"));
    let texts = app.screen().texts();
    assert!(texts.contains(&"Couldn't show this page"), "{texts:?}");
    let failed = next(&rx);
    assert_eq!(failed["ev"], "failed");
    assert!(
        failed["component"].as_str().is_some_and(|c| !c.is_empty()),
        "{failed}"
    );
    assert!(
        failed["message"]
            .as_str()
            .is_some_and(|m| m.contains("probe: the body failed")),
        "{failed}"
    );
    quiet(&rx);
    app.press("id:page-failed-retry").unwrap();
    assert_eq!(next(&rx), serde_json::json!({"ev": "retry"}));
    channel.close();
}

#[test]
fn the_failure_reads_in_the_apps_language() {
    logged();
    let channel = Channel::open();
    let _rx = hear(&channel);
    let id = channel.id();
    let root = dressed(Root::new::<ProbeScreen>(move || ProbeProps {
        surface: id.into(),
    }));
    let mut config = EngineConfig::new(root);
    config.viewport = nsh_core::Size::new(nsh_core::Points(390.0), nsh_core::Points(844.0));
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    nsh_core::platform::HostEnvironment {
        locale: zh_hans().nsh_locale(),
        font_scale: None,
    }
    .apply(&mut config.environment);
    let mut app = Headless::start(config).expect("the engine starts");
    app.wait_for("id:page-failed").unwrap();
    steady(&mut app);
    let texts = app.screen().texts();
    assert!(texts.contains(&"这一页没能显示出来"), "{texts:?}");
    assert!(texts.contains(&"重试"), "{texts:?}");
    channel.close();
}

#[test]
fn a_refused_post_is_logged_once_per_op_and_the_page_is_only_its_bar() {
    let (mut app, channel) = start(Screen::Home, Params::default(), "home");
    let bad = r#"{"op":"home","connections":[],"version":"v","bogus":1}"#;
    assert!(channel.post(bad).is_err());
    assert!(channel.post(bad).is_err());
    assert!(channel.post(r#"{"op":"trees","trees":"no"}"#).is_err());
    steady(&mut app);
    let lines = logged().lock().unwrap().clone();
    let refused: Vec<_> = lines
        .iter()
        .filter(|l| l.contains("post \"home\" refused") && l.contains("bogus"))
        .collect();
    assert_eq!(refused.len(), 1, "once for the op: {lines:#?}");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("[moshpit] ERROR") && l.contains("post \"trees\" refused")),
        "another op is logged too: {lines:#?}"
    );
    // What the owner saw (Canary 116): the bar, nothing under it.
    assert!(has(&app, "home-settings"));
    assert!(!has(&app, "home-list"));
    let counts = channel.counts();
    assert_eq!((counts.posted, counts.applied), (0, 0), "{counts:?}");
    assert!(counts.heard, "the screen listens: {counts:?}");
    channel.close();
}

#[test]
fn a_settings_page_without_state_is_only_its_bar() {
    for page in [None, Some("terminal")] {
        let params = Params {
            page: page.map(str::to_owned),
            back: page.map(|_| "Settings".to_owned()),
            ..Params::default()
        };
        let (mut app, channel) = start(Screen::Settings, params, "settings");
        steady(&mut app);
        assert!(!has(&app, "settings-list"), "{page:?}: {:?}", ids(&app));
        assert!(!has(&app, "page-failed"), "{page:?}: nothing failed");
        assert!(channel.counts().heard);
        channel.close();
    }
}

#[test]
fn a_screen_that_applied_its_state_counts_it() {
    let (mut app, channel) = start(Screen::Settings, Params::default(), "settings");
    std::thread::sleep(Duration::from_millis(1));
    let before = channel.counts();
    assert!(before.heard && before.posted == 0, "{before:?}");
    // The watchdog's healthy case: what was posted was applied.
    post(&mut app, &channel, r#"{"op":"reveal","anchor":"bottom"}"#);
    let after = channel.counts();
    assert_eq!(
        (after.posted, after.applied, after.lost),
        (1, 1, 0),
        "{after:?}"
    );
    channel.close();
}
