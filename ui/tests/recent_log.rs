//! Settings › Recent log on a headless engine (R16, §6.12): what the host
//! posts through the surface's channel is what the page shows (skeleton
//! rows, the entries, an empty page, the failure with Retry); what the user
//! does reaches the host or the clipboard (and the toast says how much); an
//! entry opens past four lines; the page speaks the app's language, follows
//! Dynamic Type and the app theme, and carries the SwiftUI page's
//! accessibility identifiers.

mod common;

use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

use moshpit_ui::{
    channel::Channel,
    recent_log::{Entry, LogState, Message, plain_text},
    screen::{Params, Screen},
    theme::AppTheme,
};
use nsh_core::{
    Points, Size,
    platform::{HostEnvironment, Idiom, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

/// The page on an iPad mini in portrait (the device the app's tests run
/// on), running iOS 18.
fn start(params: Params) -> (common::App, Arc<Channel>) {
    let channel = Channel::open();
    let root = Screen::RecentLog.root(&channel, &params);
    let mut config = EngineConfig::new(root);
    config.viewport = Size::new(Points(744.0), Points(1133.0));
    config.environment.os = Os::Ios;
    config.environment.os_version = (18, 2);
    config.environment.idiom = Idiom::Pad;
    config.environment.pixel_ratio = 2.0;
    // A manual clock (the app's time moves only as the driver moves it): a
    // step settles when the engine says it is idle, not after 100 ms with
    // no batch, and a wait moves the clock a frame at a time — so nothing a
    // test reads depends on how loaded the machine is.
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    // What the iOS host does with the params' language (`AppView::set_locale`).
    HostEnvironment {
        locale: params.nsh_locale(),
        font_scale: None,
    }
    .apply(&mut config.environment);
    let mut app = Headless::start(config).expect("the engine starts");
    app.wait_for("id:recent-log").unwrap();
    (common::App(app), channel)
}

fn post(app: &mut Headless, channel: &Channel, state: LogState) {
    common::post(app, channel, &Message::Log(state).to_json());
}

fn entry(time: &str, category: &str, message: &str) -> Entry {
    Entry {
        time: time.into(),
        category: category.into(),
        message: message.into(),
    }
}

fn entries() -> Vec<Entry> {
    vec![
        entry(
            "12:00:02.500",
            "health",
            "reconnect: path changed (wifi → cellular)",
        ),
        entry("12:00:01.250", "layout", "resize 80x24 → 70x40"),
    ]
}

const WAIT: Duration = Duration::from_secs(10);

#[test]
fn it_reads_until_the_host_has_the_log() {
    let (mut app, channel) = start(Params::default());
    app.wait_for("id:recent-log-loading").unwrap();
    assert!(app.screen().matches("id:nav-bar-title").unwrap());
    assert!(app.screen().shows("Recent log"));
    assert!(
        !app.screen().enabled("id:recent-log-copy").unwrap(),
        "nothing to copy yet"
    );
    channel.close();
}

#[test]
fn the_posted_entries_show_in_order() {
    let (mut app, channel) = start(Params::default());
    post(&mut app, &channel, LogState::Loaded { entries: entries() });
    app.wait_for("id:recent-log-entries").unwrap();
    assert_eq!(app.screen().count("id:recent-log-entry").unwrap(), 2);
    let texts = app.screen().texts();
    let at = |text: &str| {
        texts
            .iter()
            .position(|t| *t == text)
            .unwrap_or_else(|| panic!("{text}"))
    };
    assert!(at("12:00:02.500") < at("health"));
    assert!(at("health") < at("reconnect: path changed (wifi → cellular)"));
    assert!(at("reconnect: path changed (wifi → cellular)") < at("12:00:01.250"));
    assert!(at("12:00:01.250") < at("layout"));
    assert!(at("layout") < at("resize 80x24 → 70x40"));
    assert!(!app.screen().matches("id:recent-log-loading").unwrap());
    assert!(app.screen().enabled("id:recent-log-copy").unwrap());
    channel.close();
}

#[test]
fn copy_puts_a_line_per_entry_on_the_clipboard() {
    let (mut app, channel) = start(Params::default());
    post(&mut app, &channel, LogState::Loaded { entries: entries() });
    app.wait_for("id:recent-log-entries").unwrap();
    app.press("id:recent-log-copy").unwrap();
    let expected = plain_text(&entries());
    assert_eq!(
        expected,
        "12:00:02.500  health  reconnect: path changed (wifi → cellular)\n\
         12:00:01.250  layout  resize 80x24 → 70x40"
    );
    assert_eq!(app.screen().clipboard(), Some(expected.as_str()));
    app.wait_for("text:Copied 2 lines").unwrap();
    channel.close();
}

#[test]
fn the_newest_500_show_and_copy_takes_them_all() {
    let (mut app, channel) = start(Params::default());
    let many: Vec<Entry> = (0..520)
        .map(|i| {
            entry(
                &format!("12:{:02}:{:02}.000", i / 60, i % 60),
                "health",
                &format!("line {i}"),
            )
        })
        .collect();
    post(
        &mut app,
        &channel,
        LogState::Loaded {
            entries: many.clone(),
        },
    );
    app.wait_for("id:recent-log-entries").unwrap();
    assert_eq!(app.screen().count("id:recent-log-entry").unwrap(), 500);
    assert!(app.screen().shows("line 0"), "the newest first");
    assert!(app.screen().shows("line 499"));
    assert!(!app.screen().shows("line 500"));
    assert!(
        app.screen()
            .shows("Showing the latest 500 of 520. Copy takes them all.")
    );
    app.press("id:recent-log-copy").unwrap();
    assert_eq!(app.screen().clipboard(), Some(plain_text(&many).as_str()));
    app.wait_for("text:Copied 520 lines").unwrap();
    // 500 or fewer: no footer.
    post(&mut app, &channel, LogState::Loaded { entries: entries() });
    app.wait_for_gone("text:line 0", WAIT).unwrap();
    assert!(
        !app.screen()
            .shows("Showing the latest 500 of 520. Copy takes them all.")
    );
    channel.close();
}

#[test]
fn nothing_logged_says_so_and_has_nothing_to_copy() {
    let (mut app, channel) = start(Params::default());
    post(&mut app, &channel, LogState::Loaded { entries: vec![] });
    app.wait_for("id:recent-log-empty").unwrap();
    assert!(app.screen().shows("Nothing logged in the last 30 minutes"));
    assert!(!app.screen().enabled("id:recent-log-copy").unwrap());
    channel.close();
}

#[test]
fn a_failed_read_shows_the_hosts_message() {
    let (mut app, channel) = start(Params::default());
    let message = "Couldn't read the log: The operation couldn’t be completed.";
    post(
        &mut app,
        &channel,
        LogState::Failed {
            message: message.into(),
        },
    );
    app.wait_for("id:recent-log-error").unwrap();
    assert!(app.screen().shows("Couldn't read the log"));
    assert!(app.screen().shows(message));
    let (tx, rx) = mpsc::channel();
    channel.set_listener(Some(Box::new(move |json| {
        let _ = tx.send(json.to_owned());
    })));
    app.press("id:recent-log-retry").unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), r#"{"ev":"reload"}"#);
    channel.close();
}

#[test]
fn later_posts_replace_what_shows() {
    let (mut app, channel) = start(Params::default());
    post(&mut app, &channel, LogState::Loaded { entries: entries() });
    app.wait_for("id:recent-log-entries").unwrap();
    post(&mut app, &channel, LogState::Loading);
    app.wait_for("id:recent-log-loading").unwrap();
    app.wait_for_gone("id:recent-log-entries", WAIT).unwrap();
    channel.close();
}

#[test]
fn back_tells_the_host() {
    let (mut app, channel) = start(Params::default());
    let (tx, rx) = mpsc::channel();
    channel.set_listener(Some(Box::new(move |json| {
        let _ = tx.send(json.to_owned());
    })));
    assert!(app.screen().matches("role:button[Settings]").unwrap());
    app.press("id:nav-bar-back").unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), r#"{"ev":"back"}"#);
    channel.close();
}

#[test]
fn the_text_is_in_the_apps_language() {
    let params = Params {
        locale: Some("zh-Hans".into()),
        ..Params::default()
    };
    let (mut app, channel) = start(params);
    app.wait_for("id:recent-log-loading").unwrap();
    assert!(app.screen().shows("最近日志"));
    assert!(app.screen().shows("设置"));
    assert!(app.screen().shows("复制"));
    post(&mut app, &channel, LogState::Loaded { entries: vec![] });
    app.wait_for("text:最近 30 分钟没有日志").unwrap();
    channel.close();
}

#[test]
fn entries_follow_dynamic_type_in_mono_footnote() {
    let (mut app, channel) = start(Params::default());
    post(&mut app, &channel, LogState::Loaded { entries: entries() });
    app.wait_for("id:recent-log-entries").unwrap();
    let size =
        |app: &Headless, selector: &str| app.screen().span_style(selector).unwrap().font_size.0;
    let message = "text:resize 80x24 → 70x40";
    assert_eq!(size(&app, message), 13.0, "mono-footnote");
    assert_eq!(size(&app, "text:12:00:01.250"), 13.0);
    assert_eq!(size(&app, "text:Recent log"), 17.0);
    // Extra extra extra large: the footnote is 19 pt.
    app.set_environment("xxxLarge", |env| env.font_scale = 23.0 / 17.0)
        .unwrap();
    assert_eq!(size(&app, message), 19.0, "mono-footnote at xxxLarge");
    assert_eq!(
        size(&app, "text:Recent log"),
        21.0,
        "a bar's text grows to 21 pt"
    );
    app.set_environment("AX5", |env| env.font_scale = 53.0 / 17.0)
        .unwrap();
    assert_eq!(size(&app, "text:Recent log"), 21.0, "and no further");
    channel.close();
}

#[test]
fn a_press_opens_an_entry_past_four_lines() {
    let (mut app, channel) = start(Params::default());
    let long = "a long line ".repeat(60);
    post(
        &mut app,
        &channel,
        LogState::Loaded {
            entries: vec![entry("12:00:03.000", "health", long.trim())],
        },
    );
    app.wait_for("id:recent-log-entry").unwrap();
    let id = app.screen().id("id:recent-log-entry").unwrap();
    let height = |app: &Headless| {
        app.screen()
            .layout_nodes()
            .into_iter()
            .find(|node| node.id == id)
            .unwrap()
            .bounds
            .size
            .height
            .0
    };
    let closed = height(&app);
    app.press("id:recent-log-entry").unwrap();
    common::steady(&mut app);
    let open = height(&app);
    assert!(open > closed + 20.0, "{closed} → {open}");
    app.press("id:recent-log-entry").unwrap();
    common::steady(&mut app);
    assert!((height(&app) - closed).abs() < 0.5);
    channel.close();
}

#[test]
fn the_bar_takes_the_themes_accent() {
    let teal = AppTheme {
        accent: nsh_core::color::rgb(0x53DCC9),
        ..AppTheme::SIGNAL_ROOM
    };
    // Dark, where the accent reads as it is; the page follows the system,
    // and in the light appearance the accent is darkened until it reads
    // (`ui::tokens::readable_on`).
    let (mut app, channel) = start(Params {
        theme: teal,
        light: Some(false),
        ..Params::default()
    });
    post(&mut app, &channel, LogState::Loaded { entries: entries() });
    app.wait_for("id:recent-log-entries").unwrap();
    let color = |selector: &str| app.screen().span_style(selector).unwrap().color;
    assert_eq!(color("text:Settings"), nsh_core::Color::from(teal.accent));
    assert_eq!(color("text:Copy"), nsh_core::Color::from(teal.accent));
    channel.close();
}

#[test]
fn the_page_has_the_swiftui_pages_identifiers() {
    let (mut app, channel) = start(Params::default());
    for id in ["recent-log", "recent-log-loading", "recent-log-copy"] {
        assert!(app.screen().matches(&format!("id:{id}")).unwrap(), "{id}");
    }
    post(&mut app, &channel, LogState::Loaded { entries: entries() });
    app.wait_for("id:recent-log-entries").unwrap();
    assert!(app.screen().matches("id:recent-log-entry").unwrap());
    assert!(app.screen().matches("id:recent-log-list").unwrap());
    channel.close();
}
