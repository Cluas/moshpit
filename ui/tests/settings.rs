//! Settings on a headless engine (UI-DESIGN §6.4): the hub and its pages
//! show what the host posts, the switches, segments and menus are UIKit's
//! (the host's native components) and carry the settings' values, what the
//! user does reaches the host (a setting, a page to open, Done, back, the
//! version row's seven taps), the rows that come and go with the state, the
//! app's language and Dynamic Type; and R8's acceptance on an iPhone (§8):
//! the hub on one screen, footnotes of two lines at most, four cursor
//! colors, labels on one line.

mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use common::{
    App, assert_bar_follows_scrolling, emit, hear, next, post, quiet, settings_fixture::state,
    start, start_observing, start_observing_with, steady, zh_hans,
};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
    settings::{CursorShape, Message, SettingsState},
    ui::swatch::{CursorColor, palette_of},
    voice_engine::Engine,
};
use nsh_core::{
    MountItem, NodeId, ViewStyle,
    platform::{ColorScheme, HostEnvironment, Idiom, Os},
    prelude::*,
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;
use serde_json::json;

/// Settings on page `part` (`appearance`, `terminal`…).
fn on(part: &str) -> Params {
    Params {
        page: Some(part.into()),
        ..Params::default()
    }
}

fn show(app: &mut Headless, channel: &Channel, state: SettingsState) {
    post(app, channel, &Message::Settings(Box::new(state)).to_json());
}

/// Waits for `selector` to show.
fn find(app: &mut Headless, selector: &str) {
    app.wait_for(selector)
        .unwrap_or_else(|e| panic!("{selector}: {e}: {:?}", app.screen().texts()));
}

fn shows_all(app: &Headless, texts: &[&str]) {
    for text in texts {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
}

/// The bounds of the view `selector` names.
fn bounds(app: &Headless, selector: &str) -> Bounds<Points> {
    let id = app
        .screen()
        .id(selector)
        .unwrap_or_else(|e| panic!("{selector}: {e}"));
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.id == id)
        .unwrap_or_else(|| panic!("{selector} is laid out"))
        .bounds
}

#[test]
fn the_hub_shows_what_the_host_posted() {
    let (mut app, channel) = start(Screen::Settings, Params::default(), "settings");
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-version");
    shows_all(
        &app,
        &[
            "Settings",
            "Done",
            "Appearance",
            "Signal Room",
            "Terminal",
            "GitHub Dark",
            "Connections",
            "Shortcuts",
            "9/12",
            "SSH keys",
            "Notifications",
            "On",
            "Voice input",
            "Whisper (local model)",
            "Set up a host",
            "Send feedback",
            "Recent log",
            "Version",
            "1.0.9 (1)",
            "The pit never closes.",
        ],
    );
    assert!(
        !app.screen().matches("id:settings-icp").unwrap(),
        "no filing off the mainland"
    );
    for gone in ["Moshpit 1.0.9 (1) · dev", "Keep connections alive", "Blink"] {
        assert!(!app.screen().shows(gone), "{gone} is not on the hub");
    }
    channel.close();
}

#[test]
fn each_page_shows_its_settings() {
    for (part, texts) in [
        (
            "appearance",
            &[
                "Appearance",
                "Settings",
                "Accent",
                "Signal Room",
                "App Icon",
                "Default",
            ][..],
        ),
        (
            "terminal",
            &[
                "Terminal",
                "Font",
                "Menlo",
                "Size",
                "11 pt",
                "Theme",
                "GitHub Dark",
                "CURSOR",
                "Shape",
                "Color",
                "Accent",
                "Blue",
                "Green",
                "White",
                "Blink",
                "Trail while predicting",
                "mosh only",
                "Cursor settings apply to every session.",
                "git:(",
            ][..],
        ),
        (
            "connections",
            &[
                "Connections",
                "Keep connections alive",
                "Sends a keepalive so idle connections aren't dropped.",
                "Keyboard on open",
                "Remote clipboard read",
                "MOSH",
                "Mosh for new connections",
                "Predictive echo",
                "Never",
                "UDP ports",
                "60000–61000",
                "mosh-server path",
                "/opt/homebrew/bin/mosh-server",
                "Open the server's UDP ports in this range.",
                "ATTACHED IMAGES",
                "Keep",
            ][..],
        ),
        (
            "notifications",
            &[
                "Notifications",
                "Alert when an agent needs you",
                "Live Activity",
                "Alert sound",
                "Show detail on lock screen",
                "How notifications work",
            ][..],
        ),
        (
            "voice",
            &[
                "Voice input",
                "Adds a mic key to the shortcut bar",
                "Recognition",
                "Whisper (local model)",
                "Model",
                "None",
                "Language",
                "Auto-detect",
            ][..],
        ),
    ] {
        let (mut app, channel) = start(Screen::Settings, on(part), "settings");
        show(&mut app, &channel, state());
        find(&mut app, "id:settings-list");
        shows_all(&app, texts);
        assert!(
            !app.screen().matches("id:settings-version").unwrap(),
            "{part}: the version is the hub's"
        );
        channel.close();
    }
}

#[test]
fn the_controls_are_uikits_with_the_settings_values() {
    let (mut app, channel, natives) = start_observing(Screen::Settings, on("terminal"), "settings");
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-font-size");
    let shape = natives
        .find("moshpit/segmented", "settings-cursor-shape")
        .unwrap();
    assert_eq!(
        natives.prop(shape, "items"),
        Some(json!(["Block", "Bar", "Underline"]))
    );
    assert_eq!(natives.prop(shape, "selected"), Some(json!(1)));
    let font = natives.find("moshpit/menu", "settings-font").unwrap();
    assert_eq!(
        natives.prop(font, "items"),
        Some(json!([
            {"id": "system", "title": "SF Mono"},
            {"id": "Menlo", "title": "Menlo", "checked": true},
        ]))
    );
    assert_eq!(natives.prop(font, "label"), Some(json!("Font, Menlo")));
    let blink = natives
        .find("moshpit/switch", "settings-cursor-blink")
        .unwrap();
    assert_eq!(natives.prop(blink, "on"), Some(json!(false)));
    assert!(
        natives
            .find("moshpit/slider", "settings-font-slider")
            .is_none(),
        "steps, not a slider"
    );
    let mut next_state = state();
    next_state.cursor_shape = CursorShape::Underline;
    next_state.font_size = 14.0;
    show(&mut app, &channel, next_state);
    assert_eq!(natives.prop(shape, "selected"), Some(json!(2)));
    assert!(app.screen().shows("14 pt"));
    channel.close();

    let (mut app, channel, natives) =
        start_observing(Screen::Settings, on("connections"), "settings");
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-server-binary");
    let switch_on = |id: &str| {
        natives.prop(
            natives
                .find("moshpit/switch", id)
                .unwrap_or_else(|| panic!("{id}")),
            "on",
        )
    };
    assert_eq!(switch_on("settings-keep-alive"), Some(json!(true)));
    assert_eq!(switch_on("settings-keyboard-on-open"), Some(json!(false)));
    assert_eq!(switch_on("settings-mosh-default"), Some(json!(true)));
    let retention = natives
        .find("moshpit/segmented", "settings-upload-retention")
        .unwrap();
    assert_eq!(
        natives.prop(retention, "items"),
        Some(json!(["24 hours", "7 days", "Forever"]))
    );
    assert_eq!(natives.prop(retention, "selected"), Some(json!(1)));
    let predict = natives.find("moshpit/menu", "settings-predict").unwrap();
    assert_eq!(
        natives.prop(predict, "items"),
        Some(json!([
            {"id": "adaptive", "title": "Adaptive"},
            {"id": "always", "title": "Always"},
            {"id": "never", "title": "Never", "checked": true},
            {"id": "experimental", "title": "Experimental"},
        ]))
    );
    channel.close();

    let (mut app, channel, natives) =
        start_observing(Screen::Settings, on("notifications"), "settings");
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-notification-info");
    assert_eq!(natives.count("moshpit/switch"), 4);
    let live = natives
        .find("moshpit/switch", "settings-live-activity")
        .unwrap();
    assert_eq!(natives.prop(live, "on"), Some(json!(false)));
    channel.close();
}

#[test]
fn what_the_user_does_reaches_the_host() {
    let (mut app, channel) = start(Screen::Settings, Params::default(), "settings");
    let told = hear(&channel);
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-version");
    for (row, page) in [
        ("settings-open-appearance", "appearance"),
        ("settings-open-terminal", "terminal"),
        ("settings-open-connections", "connections"),
        ("settings-shortcuts", "shortcuts"),
        ("settings-ssh-keys", "ssh-keys"),
        ("settings-open-notifications", "notifications"),
        ("settings-open-voice", "voice"),
        ("settings-host-setup", "host-setup"),
        ("settings-feedback", "feedback"),
        ("settings-recent-log", "recent-log"),
    ] {
        app.press(&format!("id:{row}")).unwrap();
        assert_eq!(next(&told), json!({"ev": "open", "page": page}), "{row}");
    }
    app.press("id:settings-done").unwrap();
    assert_eq!(next(&told), json!({"ev": "done"}));
    quiet(&told);
    channel.close();

    for (part, rows) in [
        (
            "appearance",
            &[
                ("settings-accent", "accent"),
                ("settings-app-icon", "app-icon"),
            ][..],
        ),
        ("terminal", &[("settings-theme", "theme")][..]),
        (
            "connections",
            &[
                ("settings-udp-range", "udp-range"),
                ("settings-server-binary", "server-binary"),
            ][..],
        ),
        (
            "notifications",
            &[("settings-notification-info", "notification-info")][..],
        ),
        (
            "voice",
            &[
                ("settings-voice-engine", "voice-engine"),
                ("settings-voice-language", "voice-language"),
                ("settings-voice-model", "whisper-model"),
            ][..],
        ),
    ] {
        let (mut app, channel) = start(Screen::Settings, on(part), "settings");
        let told = hear(&channel);
        show(&mut app, &channel, state());
        find(&mut app, "id:settings-list");
        for (row, page) in rows {
            app.press(&format!("id:{row}")).unwrap();
            assert_eq!(next(&told), json!({"ev": "open", "page": page}), "{row}");
        }
        app.press("id:nav-bar-back").unwrap();
        assert_eq!(next(&told), json!({"ev": "back"}), "{part}");
        quiet(&told);
        channel.close();
    }
}

#[test]
fn the_pages_controls_set_what_they_show() {
    let (mut app, channel, natives) = start_observing(Screen::Settings, on("terminal"), "settings");
    let told = hear(&channel);
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-font-size");
    let node = |name: &str, id: &str| natives.find(name, id).unwrap_or_else(|| panic!("{id}"));
    emit(
        &mut app,
        node("moshpit/segmented", "settings-cursor-shape"),
        "change",
        json!(0),
    );
    assert_eq!(
        next(&told),
        json!({"ev": "set", "key": "cursorShape", "value": "block"})
    );
    emit(
        &mut app,
        node("moshpit/menu", "settings-font"),
        "select",
        json!("system"),
    );
    assert_eq!(
        next(&told),
        json!({"ev": "set", "key": "font", "value": "system"})
    );
    app.press("id:settings-cursor-color-green").unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "set", "key": "cursorColor", "value": "green"})
    );
    app.press("id:settings-font-larger").unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "set", "key": "fontSize", "value": 12.0})
    );
    app.press("id:settings-font-smaller").unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "set", "key": "fontSize", "value": 10.0})
    );
    // At either end the step does nothing.
    let mut largest = state();
    largest.font_size = 18.0;
    show(&mut app, &channel, largest);
    app.press("id:settings-font-larger").ok();
    quiet(&told);
    channel.close();

    let (mut app, channel, natives) =
        start_observing(Screen::Settings, on("connections"), "settings");
    let told = hear(&channel);
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-server-binary");
    let node = |name: &str, id: &str| natives.find(name, id).unwrap_or_else(|| panic!("{id}"));
    emit(
        &mut app,
        node("moshpit/switch", "settings-keep-alive"),
        "change",
        json!(false),
    );
    assert_eq!(
        next(&told),
        json!({"ev": "set", "key": "keepAlive", "value": false})
    );
    emit(
        &mut app,
        node("moshpit/segmented", "settings-upload-retention"),
        "change",
        json!(2),
    );
    assert_eq!(
        next(&told),
        json!({"ev": "set", "key": "uploadRetentionDays", "value": 0})
    );
    emit(
        &mut app,
        node("moshpit/menu", "settings-predict"),
        "select",
        json!("experimental"),
    );
    assert_eq!(
        next(&told),
        json!({"ev": "set", "key": "predictMode", "value": "experimental"})
    );
    channel.close();

    let (mut app, channel, natives) = start_observing(Screen::Settings, on("voice"), "settings");
    let told = hear(&channel);
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-voice-engine");
    emit(
        &mut app,
        natives
            .find("moshpit/switch", "settings-voice-input")
            .unwrap(),
        "change",
        json!(false),
    );
    assert_eq!(
        next(&told),
        json!({"ev": "set", "key": "voiceInput", "value": false})
    );
    channel.close();
}

#[test]
fn a_theme_picked_while_settings_shows_reaches_its_colors() {
    let (mut app, channel, natives) =
        start_observing(Screen::Settings, on("connections"), "settings");
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-server-binary");
    let keep_alive = natives
        .find("moshpit/switch", "settings-keep-alive")
        .unwrap();
    let before = natives.prop(keep_alive, "tint");
    let mut teal = state();
    teal.app_theme.accent = nsh_core::color::rgb(0x53DCC9);
    show(&mut app, &channel, teal);
    assert_ne!(natives.prop(keep_alive, "tint"), before);
    channel.close();
}

#[test]
fn rows_come_and_go_with_the_state() {
    let (mut app, channel) = start(Screen::Settings, on("voice"), "settings");
    let mut state = state();
    state.voice_engine = Engine::Apple;
    show(&mut app, &channel, state.clone());
    find(&mut app, "id:settings-voice-engine");
    assert!(
        !app.screen().matches("id:settings-voice-model").unwrap(),
        "Apple: no model row"
    );
    assert!(app.screen().shows("Apple (built in)"));
    state.voice_input = false;
    show(&mut app, &channel, state.clone());
    assert!(!app.screen().matches("id:settings-voice-engine").unwrap());
    assert!(!app.screen().matches("id:settings-voice-language").unwrap());
    state.voice_input = true;
    state.voice_engine = Engine::Whisper;
    state.voice_model = Some("Small".into());
    show(&mut app, &channel, state.clone());
    find(&mut app, "id:settings-voice-model");
    assert!(app.screen().shows("Small"));
    channel.close();

    let (mut app, channel) = start(Screen::Settings, Params::default(), "settings");
    state.filing = Some("浙ICP备2025212603号-2A".into());
    state.voice_input = false;
    state.notifications = false;
    show(&mut app, &channel, state);
    find(&mut app, "id:settings-icp");
    assert!(app.screen().shows("APP filing 浙ICP备2025212603号-2A"));
    assert_eq!(
        app.screen().texts().iter().filter(|t| **t == "Off").count(),
        2,
        "Notifications and Voice input both say Off"
    );
    let told = hear(&channel);
    app.press("id:settings-icp").unwrap();
    assert_eq!(next(&told), json!({"ev": "open", "page": "filing"}));
    channel.close();
}

#[test]
fn the_pages_speak_the_apps_language() {
    let (mut app, channel) = start(Screen::Settings, zh_hans(), "settings");
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-version");
    shows_all(
        &app,
        &[
            "设置",
            "完成",
            "外观",
            "终端",
            "连接",
            "快捷键",
            "SSH 密钥",
            "通知",
            "开",
            "语音输入",
            "配置主机",
            "反馈问题",
            "最近日志",
            "版本",
            "机器不眠。",
        ],
    );
    channel.close();
    let params = Params {
        page: Some("terminal".into()),
        ..zh_hans()
    };
    let (mut app, channel) = start(Screen::Settings, params, "settings");
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-font-size");
    shows_all(
        &app,
        &[
            "字体",
            "字号",
            "主题",
            "光标",
            "形状",
            "颜色",
            "闪烁",
            "预测时显示轨迹",
            "仅 mosh",
            "光标设置对所有会话生效。",
        ],
    );
    channel.close();
}

#[test]
fn text_follows_dynamic_type_and_rows_stack_at_accessibility_sizes() {
    let (mut app, channel) = start(Screen::Settings, on("connections"), "settings");
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-server-binary");
    let size = |app: &Headless, s: &str| app.screen().span_style(s).unwrap().font_size.0;
    assert_eq!(size(&app, "text:Keep connections alive"), 17.0);
    app.set_environment("xxxLarge", |env| env.font_scale = 23.0 / 17.0)
        .unwrap();
    assert_eq!(size(&app, "text:Keep connections alive"), 23.0);
    app.set_environment("AX3", |env| env.font_scale = 40.0 / 17.0)
        .unwrap();
    for text in ["60000–61000", "/opt/homebrew/bin/mosh-server"] {
        assert!(app.screen().shows(text), "{text}");
    }
    let title = bounds(&app, "text:UDP ports");
    let value = bounds(&app, "text:60000–61000");
    assert!(
        value.origin.y.0 >= title.origin.y.0 + title.size.height.0 - 0.5,
        "AX3: the value goes under its title"
    );
    channel.close();
}

#[test]
fn a_reveal_scrolls_the_list_to_a_group_or_its_end() {
    let (mut app, channel, _) =
        start_observing_with(Screen::Settings, on("connections"), "settings", |config| {
            config.viewport = nsh_core::Size::new(Points(402.0), Points(500.0))
        });
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-server-binary");
    let scrolled_to = |app: &Headless| -> Vec<f32> {
        app.screen()
            .commands()
            .iter()
            .filter_map(|(_, command)| match command {
                nsh_core::Command::ScrollTo {
                    y, animated: false, ..
                } => Some(y.0),
                _ => None,
            })
            .collect()
    };
    post(
        &mut app,
        &channel,
        &Message::Reveal(moshpit_ui::settings::Anchor::Behavior).to_json(),
    );
    let behavior = *scrolled_to(&app).last().expect("the list scrolled");
    post(
        &mut app,
        &channel,
        &Message::Reveal(moshpit_ui::settings::Anchor::Mosh).to_json(),
    );
    let mosh = *scrolled_to(&app).last().unwrap();
    assert!(0.0 <= behavior && behavior < mosh, "{behavior} {mosh}");
    post(
        &mut app,
        &channel,
        &Message::Reveal(moshpit_ui::settings::Anchor::Bottom).to_json(),
    );
    assert!(
        app.screen()
            .sent(&nsh_core::Command::ScrollToEnd { animated: false })
            // A short page: the group is already as far as the list goes.
            || scrolled_to(&app).last().is_some_and(|end| *end >= mosh),
        "{:?}",
        app.screen().commands().last()
    );
    channel.close();
}

/// B316 on a page of Settings: the bar sets itself off once rows are under
/// it (the scroll edge effect on iOS 26, the classic background below).
#[test]
fn a_pages_bar_sets_itself_off_from_the_rows_scrolled_under_it() {
    for (version, glass) in [((26, 0), true), ((18, 2), false)] {
        let (mut app, channel, _) = start_observing_with(
            Screen::Settings,
            on("connections"),
            "settings",
            move |config| {
                config.viewport = nsh_core::Size::new(Points(402.0), Points(500.0));
                config.environment.os_version = version;
            },
        );
        show(&mut app, &channel, state());
        find(&mut app, "id:settings-server-binary");
        assert_bar_follows_scrolling(&mut app, "settings-list", glass);
        channel.close();
    }
}

// --- UI-DESIGN §8, R8's acceptance: an iPhone 16 Pro (402 × 874 pt) -----------

/// What the engine mounted: each view's styles, in order.
#[derive(Clone, Default)]
struct Styles(Arc<Mutex<HashMap<NodeId, Vec<ViewStyle>>>>);

impl Styles {
    fn apply(&self, batch: &nsh_core::MountBatch) {
        for item in &batch.items {
            if let MountItem::ViewStyle { id, style } = item {
                self.0
                    .lock()
                    .unwrap()
                    .entry(*id)
                    .or_default()
                    .push((**style).clone());
            }
        }
    }

    fn last(&self, app: &Headless, selector: &str) -> ViewStyle {
        let id = app.screen().id(selector).unwrap();
        self.0
            .lock()
            .unwrap()
            .get(&id)
            .and_then(|styles| styles.last().cloned())
            .unwrap_or_else(|| panic!("{selector} is styled"))
    }
}

/// Settings on an iPhone 16 Pro, iOS 26, at the Large text size: its
/// screen, the status bar's and the home indicator's insets.
fn phone(params: Params) -> (App, Arc<Channel>, Styles) {
    phone_in(params, ColorScheme::default())
}

/// [`phone`] with the system in `scheme`.
fn phone_in(params: Params, scheme: ColorScheme) -> (App, Arc<Channel>, Styles) {
    let channel = Channel::open();
    let root = Screen::Settings.root(&channel, &params);
    let mut config = EngineConfig::new(root);
    config.environment.color_scheme = scheme;
    config.viewport = nsh_core::Size::new(Points(402.0), Points(874.0));
    config.environment.os = Os::Ios;
    config.environment.os_version = (26, 0);
    config.environment.idiom = Idiom::Phone;
    config.environment.pixel_ratio = 3.0;
    config.environment.safe_area = nsh_core::geometry::Edges {
        top: Points(62.0),
        right: Points(0.0),
        bottom: Points(HOME_INDICATOR),
        left: Points(0.0),
    };
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    HostEnvironment {
        locale: params.nsh_locale(),
        font_scale: None,
    }
    .apply(&mut config.environment);
    let mut app = Headless::start(config).expect("the engine starts");
    let styles = Styles::default();
    let seen = styles.clone();
    app.observe(move |_, batch| seen.apply(batch));
    app.wait_for("id:settings").unwrap();
    let mut app = App(app);
    show(&mut app, &channel, state());
    find(&mut app, "id:settings-list");
    steady(&mut app);
    (app, channel, styles)
}

fn with_page(params: Params, part: &str) -> Params {
    Params {
        page: Some(part.into()),
        ..params
    }
}

const SCREEN: f32 = 874.0;
/// The home indicator's inset on that phone ([`phone`]).
const HOME_INDICATOR: f32 = 34.0;

/// §8: the hub fits on one screen, in English and in Chinese, with the
/// mainland's filing line too: every row ends above the home indicator
/// before the list is scrolled. The list's end space (`page::BOTTOM`) may
/// run past the screen's bottom; with the filing line it does, by 6 pt
/// (R8's own end space was 8 pt shorter).
#[test]
fn the_hub_fits_on_one_screen() {
    for params in [Params::default(), zh_hans()] {
        let (mut app, channel, _) = phone(params.clone());
        let end = |app: &Headless| {
            let column = bounds(app, "id:settings-column");
            column.origin.y.0 + column.size.height.0
        };
        assert!(
            end(&app) <= SCREEN - HOME_INDICATOR,
            "{:?}: the hub's rows end at {} pt",
            params.locale,
            end(&app)
        );
        let mut mainland = state();
        mainland.filing = Some("浙ICP备2025212603号-2A".into());
        show(&mut app, &channel, mainland);
        find(&mut app, "id:settings-icp");
        assert!(
            end(&app) <= SCREEN - HOME_INDICATOR,
            "{:?}, with the filing: {} pt",
            params.locale,
            end(&app)
        );
        channel.close();
    }
}

/// §8: every page's footnote is at most two lines at 402 pt and Large.
#[test]
fn every_footnote_is_at_most_two_lines() {
    for params in [Params::default(), zh_hans()] {
        let zh = params.locale.is_some();
        let (app, channel, _) = phone(params.clone());
        let tagline = if zh {
            "机器不眠。"
        } else {
            "The pit never closes."
        };
        let line = bounds(&app, &format!("text:{tagline}")).size.height.0;
        channel.close();
        for (part, footnotes) in [
            (
                "appearance",
                if zh {
                    &["强调色给按钮和高亮上色；终端配色在「终端」里。"][..]
                } else {
                    &[
                        "The accent colors buttons and highlights; the terminal's colors are under Terminal.",
                    ][..]
                },
            ),
            (
                "terminal",
                if zh {
                    &["光标设置对所有会话生效。"][..]
                } else {
                    &["Cursor settings apply to every session."][..]
                },
            ),
            (
                "connections",
                if zh {
                    &[
                        "服务器要放开这段 UDP 端口。",
                        "图片上传到主机的 ~/.moshpit/uploads，每次连接时清掉过期的。",
                    ][..]
                } else {
                    &[
                        "Open the server's UDP ports in this range.",
                        "Images upload to the host's ~/.moshpit/uploads; each connect clears expired ones.",
                    ][..]
                },
            ),
            (
                "voice",
                if zh {
                    &["语音只在本机转写，不会离开这台设备。"][..]
                } else {
                    &["Speech is transcribed on this device and never leaves it."][..]
                },
            ),
        ] {
            let (app, channel, _) = phone(with_page(params.clone(), part));
            for footnote in footnotes {
                let height = bounds(&app, &format!("text:{footnote}")).size.height.0;
                assert!(
                    height <= 2.0 * line + 0.5,
                    "{part}: {footnote:?} is {height} pt, a line {line}"
                );
            }
            channel.close();
        }
    }
}

/// §8, B318: the four cursor colors are four colors.
#[test]
fn the_four_cursor_colors_differ() {
    let (app, channel, styles) = phone(on("terminal"));
    let fills: Vec<u32> = ["accent", "teal", "green", "white"]
        .iter()
        .map(|id| {
            styles
                .last(&app, &format!("id:settings-cursor-color-{id}-color"))
                .background_color
                .0
        })
        .collect();
    for (i, a) in fills.iter().enumerate() {
        for b in &fills[i + 1..] {
            assert_ne!(a, b, "{fills:x?}");
        }
    }
    let palette = palette_of(&moshpit_ui::theme::AppTheme::SIGNAL_ROOM);
    assert_eq!(CursorColor::visible(&palette).len(), 4);
    channel.close();
}

/// The Terminal page's preview is the terminal's picture: it draws on the
/// scheme's own background in either appearance (a dark scheme's stays
/// dark on a light page, a light one's light on a dark page), never on the
/// app's terminal color for the page's appearance.
#[test]
fn the_terminal_preview_is_in_the_schemes_colors_in_either_appearance() {
    let solarized_light = || {
        let mut state = state();
        state.theme.name = "Solarized Light".into();
        state.theme.background = nsh_core::color::rgb(0xFDF6E3);
        state.theme.foreground = nsh_core::color::rgb(0x657B83);
        state
    };
    for scheme in [ColorScheme::Light, ColorScheme::Dark] {
        let (mut app, channel, styles) = phone_in(on("terminal"), scheme);
        let drawn = |app: &App| {
            styles
                .last(app, "id:settings-terminal-preview")
                .background_color
                .0
        };
        // The fixture's GitHub Dark (#0D1117).
        assert_eq!(drawn(&app), 0x0D1117FF, "GitHub Dark, {scheme:?}");
        show(&mut app, &channel, solarized_light());
        steady(&mut app);
        assert_eq!(drawn(&app), 0xFDF6E3FF, "Solarized Light, {scheme:?}");
        channel.close();
    }
}

/// §8, B317: a row's label never wraps; its value gives way first.
#[test]
fn labels_keep_one_line() {
    for params in [Params::default(), zh_hans()] {
        let zh = params.locale.is_some();
        let (app, channel, _) = phone(params.clone());
        let line = bounds(&app, if zh { "text:外观" } else { "text:Appearance" })
            .size
            .height
            .0;
        channel.close();
        let mut long = state();
        long.mosh_server_binary =
            "/home/somebody/.local/share/mosh/bin/mosh-server-from-a-long-prefix".into();
        long.theme.name = "A Theme With A Very Long Name Indeed".into();
        for (part, titles) in [
            (
                "terminal",
                if zh {
                    &["字体", "字号", "主题", "闪烁", "预测时显示轨迹"][..]
                } else {
                    &["Font", "Size", "Theme", "Blink", "Trail while predicting"][..]
                },
            ),
            (
                "connections",
                if zh {
                    &[
                        "保持连接",
                        "打开终端时弹出键盘",
                        "允许远端读剪贴板",
                        "新连接默认用 mosh",
                        "预测回显",
                        "UDP 端口",
                        "mosh-server 路径",
                    ][..]
                } else {
                    &[
                        "Keep connections alive",
                        "Keyboard on open",
                        "Remote clipboard read",
                        "Mosh for new connections",
                        "Predictive echo",
                        "UDP ports",
                        "mosh-server path",
                    ][..]
                },
            ),
            (
                "notifications",
                if zh {
                    &["实时活动", "提示音", "在锁屏上显示详情", "通知是怎么工作的"][..]
                } else {
                    &[
                        "Live Activity",
                        "Alert sound",
                        "Show detail on lock screen",
                        "How notifications work",
                    ][..]
                },
            ),
        ] {
            let (mut app, channel, _) = phone(with_page(params.clone(), part));
            show(&mut app, &channel, long.clone());
            for title in titles {
                let height = bounds(&app, &format!("text:{title}")).size.height.0;
                assert!(
                    (height - line).abs() < 0.5,
                    "{part}: {title} is {height} pt, a line {line}"
                );
            }
            channel.close();
        }
    }
}
