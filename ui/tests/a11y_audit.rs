//! VoiceOver and long languages across the main screens (B744): each
//! screen on the owner's iPhone (402 pt wide), in English, German and
//! French (the longest), Japanese and Korean (line breaking), and at the
//! largest text size the app follows (AX5), read by `common::a11y::audit`:
//! every control a screen reader can name and knows to press, targets of
//! 44 pt, nothing inside a view of no size, nothing past the screen's
//! edges, no text cut short, clipped or drawn over another.

mod common;

use common::{App, Natives, a11y, home_fixture, post, settings_fixture, start_observing_with};
use moshpit_ui::{
    channel::Channel,
    connection_form::{FormState, Message as FormMessage, Mux},
    home::{Message as HomeMessage, model::HomeState},
    screen::{Params, Screen},
    settings::Message as SettingsMessage,
};
use nsh_core::{Edges, Points, Size, platform::Idiom};
use serde_json::json;

/// The owner's iPhone (iPhone 16 Pro), in points.
const WIDTH: f32 = 402.0;
const HEIGHT: f32 = 874.0;

/// The languages: the longest, and the ones whose lines break between
/// characters.
const LOCALES: [&str; 6] = ["en", "de", "fr", "ja", "ko", "zh-Hans"];

fn params(locale: &str, page: Option<&str>) -> Params {
    Params {
        locale: (locale != "en").then(|| locale.to_owned()),
        page: page.map(str::to_owned),
        ..Params::default()
    }
}

/// `screen` on the iPhone, `height` tall (a long form, to lay all of it
/// out).
fn phone(
    screen: Screen,
    params: Params,
    root: &str,
    height: f32,
) -> (App, std::sync::Arc<Channel>, Natives) {
    let (app, channel, natives) = start_observing_with(screen, params, root, |config| {
        config.viewport = Size::new(Points(WIDTH), Points(height));
        config.clock = nsh_core::time::Clock::manual(
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000),
        );
        config.environment.idiom = Idiom::Phone;
        config.environment.safe_area = Edges {
            top: Points(62.0),
            bottom: Points(34.0),
            left: Points(0.0),
            right: Points(0.0),
        };
    });
    (app, channel, natives)
}

/// One thing the audit found: on which screen, in which language, at
/// which text size, and what.
#[derive(Debug, Clone)]
struct Finding {
    screen: String,
    locale: &'static str,
    ax5: bool,
    what: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let size = if self.ax5 { " AX5" } else { "" };
        write!(f, "{} {}{size}: {}", self.screen, self.locale, self.what)
    }
}

/// What the audit finds on `app` now, and again at AX5.
fn both_sizes(
    app: &mut App,
    natives: &Natives,
    screen: &str,
    locale: &'static str,
    skip: &[&str],
) -> Vec<Finding> {
    let mut found: Vec<Finding> = a11y::audit(app, natives, WIDTH, skip)
        .into_iter()
        .map(|what| Finding {
            screen: screen.to_owned(),
            locale,
            ax5: false,
            what,
        })
        .collect();
    app.set_environment("AX5", |env| env.font_scale = 53.0 / 17.0)
        .unwrap();
    found.extend(
        a11y::audit(app, natives, WIDTH, skip)
            .into_iter()
            .map(|what| Finding {
                screen: screen.to_owned(),
                locale,
                ax5: true,
                what,
            }),
    );
    found
}

fn home(connections: Vec<moshpit_ui::home::model::Connection>) -> String {
    HomeMessage::Home(Box::new(HomeState {
        connections,
        version: "v1.0.9 (1)".into(),
        sidebar: false,
    }))
    .to_json()
}

/// The acceptance fixture at the clock's now (its stamps are 1000 to
/// 4500, at a now of 4000), as `ui/tests/home.rs` posts it.
fn fixture() -> Vec<moshpit_ui::home::model::Connection> {
    let now = 1_800_000_000.0;
    let mut hosts = home_fixture::fixture();
    for (host, (user, address, initial)) in hosts.iter_mut().zip([
        ("dev", "gpu-01.example.com", "G"),
        ("me", "build.example.com", "B"),
        ("root", "edge.example.com", "E"),
        ("pi", "lab.example.com", "L"),
    ]) {
        host.user = user.into();
        host.host = address.into();
        host.initial = initial.into();
        if let Some(control) = host.control.as_mut() {
            for hook in &mut control.hooks {
                hook.since = hook.since.map(|s| s + now - 4000.0);
            }
        }
    }
    hosts[0].connected_at = Some(now - 5.0 * 3600.0);
    hosts[1].connected_at = Some(now - 3.0 * 3600.0);
    hosts[2].lost_at = Some(now - 25.0 * 60.0);
    hosts
}

fn egg() -> String {
    json!({
        "op": "pet", "style": "pixel", "shown": true,
        "egg": {"ready": false, "taps": 0}, "may_hatch": false, "seq": 0,
    })
    .to_string()
}

fn home_screens() -> Vec<Finding> {
    let mut found = Vec::new();
    for locale in LOCALES {
        for (what, connections) in [("empty", Vec::new()), ("fixture", fixture())] {
            let (mut app, channel, natives) =
                phone(Screen::Home, params(locale, None), "home", HEIGHT);
            post(&mut app, &channel, &home(connections));
            post(&mut app, &channel, &egg());
            app.wait_for("id:home-pet").unwrap();
            found.extend(both_sizes(
                &mut app,
                &natives,
                &format!("home {what}"),
                locale,
                &[],
            ));
            channel.close();
        }
    }
    found
}

fn settings_screens() -> Vec<Finding> {
    let mut found = Vec::new();
    for locale in LOCALES {
        for page in [
            None,
            Some("appearance"),
            Some("terminal"),
            Some("connections"),
            Some("notifications"),
            Some("voice"),
            Some("companion"),
        ] {
            let (mut app, channel, natives) =
                phone(Screen::Settings, params(locale, page), "settings", HEIGHT);
            post(
                &mut app,
                &channel,
                &SettingsMessage::Settings(Box::new(settings_fixture::state())).to_json(),
            );
            app.wait_for("id:settings-list").unwrap();
            found.extend(both_sizes(
                &mut app,
                &natives,
                &format!("settings {}", page.unwrap_or("hub")),
                locale,
                // The terminal's preview is a terminal: its lines run off its edge.
                &["settings-terminal-preview"],
            ));
            channel.close();
        }
    }
    found
}

fn form_screens() -> Vec<Finding> {
    let mut found = Vec::new();
    for locale in LOCALES {
        let (mut app, channel, natives) = phone(
            Screen::ConnectionForm,
            params(locale, None),
            "connection-form",
            2600.0,
        );
        let draft = FormState {
            editing: false,
            port: "22".into(),
            udp_range: "60000 – 61000".into(),
            socks_port: "1080".into(),
            mux: Mux::Tmux,
            ..FormState::default()
        };
        post(
            &mut app,
            &channel,
            &FormMessage::Form(Box::new(draft)).to_json(),
        );
        app.wait_for("id:connection-form-name").unwrap();
        found.extend(both_sizes(&mut app, &natives, "form", locale, &[]));
        channel.close();
    }
    found
}

fn host_key_screens() -> Vec<Finding> {
    let mut found = Vec::new();
    for locale in LOCALES {
        let (mut app, channel, natives) =
            phone(Screen::HostKey, params(locale, None), "host-key", HEIGHT);
        post(
            &mut app,
            &channel,
            &json!({
                "op": "hostKey", "host": "build-box.example.com", "port": 2222,
                "fingerprint": "SHA256:k3VJxZ2mJ1u4bS9eQq8yF0aJr6w5nTqP7cXo2Lh1dEs"
            })
            .to_string(),
        );
        app.wait_for("id:host-key-fingerprint").unwrap();
        found.extend(both_sizes(&mut app, &natives, "host key", locale, &[]));
        channel.close();
    }
    found
}

/// A tmux session's tree for the terminal's chrome and its overview
/// (`ui/tests/terminal_overview.rs`'s fixture).
fn work() -> serde_json::Value {
    let panes: Vec<serde_json::Value> = [
        ("%1", "@1", 0, "codex", "/home/dev/src/api"),
        ("%2", "@2", 0, "2.1.322", "/home/dev/src/trainer"),
        ("%3", "@2", 1, "python", "/home/dev/src/trainer"),
        ("%5", "@3", 0, "zsh", "/home/dev/src/web"),
    ]
    .into_iter()
    .map(|(id, window, index, command, path)| {
        json!({"id": id, "window": window, "index": index, "command": command, "path": path})
    })
    .collect();
    json!({
        "mux": "tmux", "attached": true,
        "sessions": [{"id": "$0", "name": "work"}],
        "windows": [
            {"id": "@1", "session": "$0", "index": 1, "name": "codex", "paneCount": 1},
            {"id": "@2", "session": "$0", "index": 2, "name": "2.1.322", "paneCount": 2},
            {"id": "@3", "session": "$0", "index": 3, "name": "zsh", "paneCount": 1}
        ],
        "panes": panes,
        "hooks": [
            {"pane": "%1", "state": "working", "agent": "codex"},
            {"pane": "%2", "state": "attention", "agent": "claude"}
        ],
        "worktrees": [],
        "activeSession": "$0", "activeWindow": "@2", "activePane": "%2"
    })
}

fn terminal_screens() -> Vec<Finding> {
    let mut found = Vec::new();
    for locale in LOCALES {
        for overview in [false, true] {
            let (mut app, channel, natives) = phone(
                Screen::Terminal,
                params(locale, None),
                "terminal-screen",
                HEIGHT,
            );
            app.answer_dialogs_myself();
            post(&mut app, &channel, &json!({
                "op": "chrome",
                "name": "gpu-01", "user": "dev", "host": "gpu-01.example.com", "port": 22,
                "mosh": false, "mux": "tmux", "pill": "ssh", "state": "live",
                "crumbs": {
                    "session": "work", "squeezable": true, "window": "2:2.1.322", "pane": "claude",
                    "signal": "attention", "paneLabel": "claude, needs you",
                    "sessionIcon": "square.stack.3d.up", "windowIcon": "macwindow", "live": true,
                    "tree": work()
                },
                "cover": {"kind": "none", "state": "live"},
                "content": "terminal"
            }).to_string());
            post(
                &mut app,
                &channel,
                &json!({"op": "switcher", "tree": work(), "hintSeen": true}).to_string(),
            );
            if overview {
                post(
                    &mut app,
                    &channel,
                    &json!({"op": "overview", "open": true}).to_string(),
                );
                app.wait_for("id:overview-new-window").unwrap();
            }
            let what = if overview { "overview" } else { "terminal" };
            found.extend(both_sizes(&mut app, &natives, what, locale, &[]));
            channel.close();
        }
    }
    found
}

fn folder_screens() -> Vec<Finding> {
    let mut found = Vec::new();
    for locale in LOCALES {
        let (mut app, channel, natives) = phone(
            Screen::RemoteFolder,
            params(locale, None),
            "remote-folder-sheet",
            HEIGHT,
        );
        app.answer_dialogs_myself();
        post(&mut app, &channel, &json!({
            "op": "folder", "name": "work", "path": "/srv/work", "parents": ["/srv", "/"],
            "pushed": false, "loading": false,
            "media": [
                {"path": "/srv/work/shot.png", "name": "shot.png", "symbol": "photo", "meta": "2 min. ago · 120 KB", "thumb": true},
                {"path": "/srv/work/demo.mov", "name": "demo.mov", "video": true, "symbol": "film", "meta": "1 hr. ago · 2 MB", "duration": "0:42"}
            ],
            "rows": [
                {"path": "/srv/work/logs", "name": "logs", "dir": true, "symbol": "folder", "meta": "3 hr. ago"},
                {"path": "/srv/work/notes.md", "name": "notes.md", "symbol": "doc.text", "meta": "5 days ago · 2 KB"}
            ]
        }).to_string());
        app.wait_for("text:notes.md").unwrap();
        found.extend(both_sizes(&mut app, &natives, "folder", locale, &[]));
        channel.close();
    }
    found
}

/// Every screen's findings, surveyed once for all the tests below.
fn findings() -> &'static [Finding] {
    static FOUND: std::sync::OnceLock<Vec<Finding>> = std::sync::OnceLock::new();
    FOUND.get_or_init(|| {
        let mut found = Vec::new();
        for screens in [
            home_screens,
            settings_screens,
            form_screens,
            host_key_screens,
            terminal_screens,
            folder_screens,
        ] {
            found.extend(screens());
        }
        found
    })
}

fn none(what: &str, found: Vec<&Finding>) {
    let lines: Vec<String> = found.iter().map(|f| f.to_string()).collect();
    assert!(lines.is_empty(), "{what}:\n{}", lines.join("\n"));
}

/// `A11Y_SURVEY=1 cargo test -p moshpit-ui --test a11y_audit survey --
/// --nocapture` prints everything the audit finds, settled or not.
#[test]
fn survey() {
    let found = findings();
    if std::env::var_os("A11Y_SURVEY").is_some() {
        for finding in found {
            println!("{finding}");
        }
        println!("{} findings", found.len());
    }
}

/// Every control a screen reader reaches is announced as something to
/// press, by a name: in every language, at every size.
#[test]
fn every_control_has_a_role_and_a_name() {
    none(
        "controls a screen reader can't name or doesn't know to press",
        findings()
            .iter()
            .filter(|f| f.what.contains("has no role") || f.what.contains("with no name"))
            .collect(),
    );
}

/// Nothing sits inside a view of no size, where UIKit hands it to neither
/// VoiceOver nor XCUITest (B659's class). The pane overview is B659's own,
/// fixed there.
#[test]
fn nothing_sits_inside_a_view_of_no_size() {
    none(
        "out of a screen reader's reach",
        findings()
            .iter()
            .filter(|f| f.screen != "overview" && f.what.contains("inside a view of no size"))
            .collect(),
    );
}

/// At the default text size, in every language, nothing runs off the
/// screen's sides.
#[test]
fn nothing_runs_off_the_screen_at_the_default_size() {
    none(
        "past the screen's edge",
        findings()
            .iter()
            .filter(|f| !f.ax5 && f.what.contains("past the screen's edge"))
            .collect(),
    );
}

/// B747: at the default text size, in every language, nothing on Settings'
/// pages or the connection form is cut short. A row's description (what
/// a switch does, what the egg wants) was one line, and lost its end in
/// English too: "Tap it three times on Home to hatch it. It also hat…".
#[test]
fn settings_and_the_form_are_read_in_full_at_the_default_size() {
    none(
        "cut short",
        findings()
            .iter()
            .filter(|f| {
                !f.ax5
                    && (f.screen.starts_with("settings") || f.screen == "form")
                    && f.what.contains("cut short")
            })
            .collect(),
    );
}

/// Every control is at least 44 × 44 pt to touch, in every language, at
/// every size: its bounds and its hit slop, cut to what clips it. The
/// audit read the bounds alone, and called the terminal bar's keyboard
/// toggle and transport switch (36 pt tall, 4 pt of slop each way),
/// Settings' font steppers (32 pt, 6 pt each side) and a short bar button
/// (French "OK", 35 pt wide, 22 pt each side) too small to hit: iOS hits
/// them over their slop.
#[test]
fn every_control_is_44_pt_to_touch() {
    none(
        "targets under 44 pt",
        findings()
            .iter()
            .filter(|f| f.what.contains("a target "))
            .collect(),
    );
}

/// B833: a short bar button is 44 pt wide to touch though it is drawn
/// narrower. Settings' "OK" in French is some 35 pt wide; a labeled
/// button's slop reaches 22 pt past each side (button.rs
/// `slop(height, 0.0)`), the screen's edge cutting what lies past it.
/// German's "Fertig" alongside, as a wider one.
#[test]
fn a_short_bar_button_is_44_pt_wide_to_touch() {
    for locale in ["fr", "de"] {
        let (mut app, channel, natives) =
            phone(Screen::Settings, params(locale, None), "settings", HEIGHT);
        post(
            &mut app,
            &channel,
            &SettingsMessage::Settings(Box::new(settings_fixture::state())).to_json(),
        );
        app.wait_for("id:settings-list").unwrap();
        let drawn = app
            .screen()
            .layout_nodes()
            .into_iter()
            .find(|n| n.test_id.as_deref() == Some("settings-done"))
            .expect("Settings has its Done")
            .bounds
            .size;
        let (w, h) = a11y::touch_size_of(&app, &natives, "settings-done");
        println!(
            "{locale} settings-done: drawn {:.0} × {:.0} pt, touched {w:.0} × {h:.0} pt",
            drawn.width.0, drawn.height.0
        );
        assert!(
            w >= a11y::TARGET && h >= a11y::TARGET,
            "{locale}: Done is {w:.0} × {h:.0} pt to touch (drawn {:.0} × {:.0})",
            drawn.width.0,
            drawn.height.0
        );
        channel.close();
    }
}

/// B835, B836: no text is drawn over another, in every language, at every
/// size. The audit took what no one sees for what is drawn: the terminal
/// bar's breadcrumb measures itself with a copy at no opacity (it read as
/// "trainer" over "claude", "›" over "trainer"), and the pane overview,
/// opaque and as wide as the screen, is drawn over the terminal (the
/// backstage's "3 / 4" read as over the overview's "New window").
#[test]
fn no_text_is_drawn_over_another() {
    none(
        "texts over one another",
        findings()
            .iter()
            .filter(|f| f.what.contains(" overlaps "))
            .collect(),
    );
}

/// B834: at the default text size, in every language, the pane overview's
/// words are read in full. A window card's state line ("Needs you · 2
/// panes") and a pane's state ("Needs you") were one line each, in a card
/// 127 pt wide and a pane 72 pt wide: Japanese, Korean, German and French
/// lost their ends.
#[test]
fn the_overview_is_read_in_full_at_the_default_size() {
    none(
        "cut short on the overview",
        findings()
            .iter()
            .filter(|f| !f.ax5 && f.screen == "overview" && f.what.contains("cut short"))
            .collect(),
    );
}

/// B836: the pane overview stays on the screen at the largest text size.
/// Its count's badge ("1 waiting") follows the user's text size past the
/// chrome's extra large, and beside the windows' and panes' count it ran
/// off the screen's side in French, Japanese and Korean.
#[test]
fn the_overview_stays_on_the_screen_at_ax5() {
    none(
        "past the screen's edge on the overview",
        findings()
            .iter()
            .filter(|f| f.screen == "overview" && f.what.contains("past the screen's edge"))
            .collect(),
    );
}
