//! Settings › Theme and a theme's editor on a headless engine (R14,
//! `docs/nsh/UI-DESIGN.md` §6.10): the themes the host posts in their two
//! sections, the pick (selected for screen readers), one target per row,
//! what the user does reaching the host (a pick, the add menu, the delete
//! question's answers, back), the system alert the host's state asks for;
//! the editor's sections, its color wells
//! (UIKit's, the host's components), the name, the bright switch, the
//! preview following each change, Save, Cancel and the menu; the app's
//! language and Dynamic Type.

mod common;

use common::{
    Natives, emit, hear, next, post, quiet, start, start_observing, wait_dialog, zh_hans,
};
use moshpit_ui::{
    screen::{Params, Screen},
    theme_editor::{self, Draft, EditorState},
    theme_gallery::{Message, Palette, Themes},
};
use nsh_testing::{DialogAnswer, Headless};
use serde_json::json;

fn palette(id: &str, name: &str, background: u32) -> Palette {
    Palette {
        id: id.into(),
        name: name.into(),
        background: nsh_core::color::rgb(background),
        foreground: nsh_core::color::rgb(0xC9D1D9),
        cursor: nsh_core::color::rgb(0x5FE3D8),
        ansi: [
            0x484F58, 0xFF7B72, 0x3FB950, 0xD29922, 0x58A6FF, 0xBC8CFF, 0x39C5CF, 0xB1BAC4,
        ]
        .map(nsh_core::color::rgb),
    }
}

fn mine() -> Palette {
    palette("custom-1a2b3c4d", "Harbor Night", 0x0B1420)
}

fn themes(custom: Vec<Palette>, selected: &str, deleting: Option<&str>) -> String {
    Message::Themes(Themes {
        built_in: vec![
            palette("github-dark", "GitHub Dark", 0x050507),
            palette("dracula", "Dracula", 0x282A36),
        ],
        custom,
        selected: selected.into(),
        deleting: deleting.map(Into::into),
    })
    .to_json()
}

/// The bounds of the view with test id `id`.
fn bounds(app: &Headless, id: &str) -> nsh_core::Bounds<nsh_core::Points> {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no view {id}"))
        .bounds
}

#[test]
fn the_themes_show_in_two_sections_the_pick_selected() {
    let (mut app, channel) = start(Screen::Theme, Params::default(), "theme");
    post(&mut app, &channel, &themes(vec![mine()], "dracula", None));
    app.wait_for("id:theme-row-github-dark").unwrap();
    for text in [
        "Theme",
        "Settings",
        // Section titles are eyebrows: upper case is their style.
        "BUILT-IN",
        "YOURS",
        "GitHub Dark",
        "Dracula",
        "Harbor Night",
        "Long-press a theme to duplicate, edit or delete it.",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(app.screen().selected("id:theme-row-dracula").unwrap());
    assert!(!app.screen().selected("id:theme-row-github-dark").unwrap());
    assert!(!app.screen().matches("id:theme-none").unwrap());
    // Every row is a part of the host's long-press menus, which edit: a row
    // is one target, with no edit button of its own (§6.10).
    for id in [
        "theme-cell-github-dark",
        "theme-cell-dracula",
        "theme-cell-custom-1a2b3c4d",
    ] {
        assert!(app.screen().matches(&format!("id:{id}")).unwrap(), "{id}");
    }
    assert_eq!(app.screen().count("label:/^Edit /").unwrap(), 0);
    assert!(app.screen().matches("id:theme-built-in").unwrap());
    assert!(app.screen().matches("id:theme-yours").unwrap());
    channel.close();
}

#[test]
fn a_row_is_one_target_across_its_card_its_title_on_one_spine() {
    let (mut app, channel) = start(Screen::Theme, Params::default(), "theme");
    post(&mut app, &channel, &themes(vec![mine()], "dracula", None));
    app.wait_for("id:theme-row-dracula").unwrap();
    let card = bounds(&app, "theme-built-in");
    for id in [
        "theme-row-github-dark",
        "theme-row-dracula",
        "theme-row-custom-1a2b3c4d",
    ] {
        let row = bounds(&app, id);
        assert!(row.size.height.0 >= 44.0, "{id}: {row:?}");
        assert_eq!(row.size.width.0, card.size.width.0, "{id}");
        assert_eq!(row.origin.x.0, card.origin.x.0, "{id}");
    }
    // The page's inset follows the bar's items (ui::page): on the iPad the
    // headless engine stands for, space-page-regular.
    assert_eq!(card.origin.x.0, 20.0);
    channel.close();
}

#[test]
fn reveal_puts_the_users_own_themes_under_the_bar() {
    let (mut app, channel) = start(Screen::Theme, Params::default(), "theme");
    // Enough themes for the list to scroll the first custom one to its top.
    let built_in = (0..12)
        .map(|i| palette(&format!("built-in-{i}"), &format!("Theme {i}"), 0x050507))
        .collect();
    let mut custom = vec![mine()];
    custom.extend((0..30).map(|i| palette(&format!("custom-{i}"), &format!("Mine {i}"), 0x0B1420)));
    post(
        &mut app,
        &channel,
        &Message::Themes(Themes {
            built_in,
            custom,
            selected: "built-in-0".into(),
            deleting: None,
        })
        .to_json(),
    );
    app.wait_for("id:theme-row-custom-1a2b3c4d").unwrap();
    let section = bounds(&app, "theme-yours");
    post(&mut app, &channel, &Message::Reveal.to_json());
    // The engine resolves a reveal to the list's offset: the section just
    // under the iPad's bar, 50 pt (no status bar in the headless engine).
    let scrolled_to = app
        .screen()
        .commands()
        .iter()
        .rev()
        .find_map(|(_, command)| match command {
            nsh_core::Command::ScrollTo {
                y, animated: false, ..
            } => Some(y.0),
            _ => None,
        })
        .expect("the list scrolled");
    assert!(
        (scrolled_to - (section.origin.y.0 - 50.0)).abs() < 0.5,
        "{scrolled_to} for a section at {:?}",
        section.origin
    );
    channel.close();
}

#[test]
fn no_custom_theme_says_so() {
    let (mut app, channel) = start(Screen::Theme, Params::default(), "theme");
    post(&mut app, &channel, &themes(vec![], "github-dark", None));
    app.wait_for("id:theme-none").unwrap();
    assert!(app.screen().shows("No custom themes yet."));
    post(
        &mut app,
        &channel,
        &themes(vec![mine()], "github-dark", None),
    );
    app.wait_for("id:theme-row-custom-1a2b3c4d").unwrap();
    assert!(!app.screen().matches("id:theme-none").unwrap());
    channel.close();
}

#[test]
fn what_the_user_does_reaches_the_host() {
    let (mut app, channel, natives) = start_observing(Screen::Theme, Params::default(), "theme");
    let told = hear(&channel);
    post(
        &mut app,
        &channel,
        &themes(vec![mine()], "github-dark", None),
    );
    app.wait_for("id:theme-row-dracula").unwrap();
    app.press("id:theme-row-dracula").unwrap();
    assert_eq!(next(&told), json!({"ev": "select", "id": "dracula"}));
    app.press("id:theme-row-custom-1a2b3c4d").unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "select", "id": "custom-1a2b3c4d"})
    );
    // The add button is UIKit's menu: New… and Import…, each with its
    // symbol.
    let add = natives
        .find("moshpit/menu", "theme-add")
        .expect("the add menu");
    let items = natives.prop(add, "items").unwrap();
    assert_eq!(
        items,
        json!([
            {"id": "new", "title": "New…", "image": "plus"},
            {"id": "import", "title": "Import…", "image": "square.and.arrow.down"},
        ])
    );
    assert_eq!(natives.prop(add, "label"), Some(json!("Add theme")));
    emit(&mut app, add, "select", json!("new"));
    assert_eq!(next(&told), json!({"ev": "new"}));
    emit(&mut app, add, "select", json!("import"));
    assert_eq!(next(&told), json!({"ev": "import"}));
    app.press("id:nav-bar-back").unwrap();
    assert_eq!(next(&told), json!({"ev": "back"}));
    quiet(&told);
    // The host applies the pick.
    post(&mut app, &channel, &themes(vec![mine()], "dracula", None));
    assert!(app.screen().selected("id:theme-row-dracula").unwrap());
    channel.close();
}

#[test]
fn delete_asks_in_the_systems_alert_about_the_theme_the_host_names() {
    let (mut app, channel) = start(Screen::Theme, Params::default(), "theme");
    let told = hear(&channel);
    post(
        &mut app,
        &channel,
        &themes(vec![mine()], "github-dark", Some("custom-1a2b3c4d")),
    );
    assert!(wait_dialog(&mut app), "the alert never asked");
    let dialog = app.pending_dialog().cloned().unwrap();
    assert!(!dialog.sheet);
    assert_eq!(dialog.title.as_deref(), Some("Delete Harbor Night?"));
    assert_eq!(
        dialog.message.as_deref(),
        Some("This theme will be removed. Built-in themes are unaffected.")
    );
    app.answer_dialog(DialogAnswer::Press("Cancel".into()))
        .unwrap();
    assert_eq!(next(&told), json!({"ev": "cancel-delete"}));
    // The host asks again (its state still names it): asked again.
    post(
        &mut app,
        &channel,
        &themes(vec![mine()], "github-dark", Some("custom-1a2b3c4d")),
    );
    assert!(wait_dialog(&mut app), "asked again");
    app.answer_dialog(DialogAnswer::Press("Delete".into()))
        .unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "delete", "id": "custom-1a2b3c4d"})
    );
    // A theme that is not one of the user's is never asked about.
    post(
        &mut app,
        &channel,
        &themes(vec![mine()], "github-dark", Some("dracula")),
    );
    assert!(!wait_dialog(&mut app));
    quiet(&told);
    channel.close();
}

#[test]
fn the_page_speaks_the_apps_language() {
    let (mut app, channel) = start(Screen::Theme, zh_hans(), "theme");
    post(
        &mut app,
        &channel,
        &themes(vec![mine()], "github-dark", Some("custom-1a2b3c4d")),
    );
    app.wait_for("id:theme-row-github-dark").unwrap();
    for text in [
        "主题",
        "设置",
        "内置",
        "自己做的",
        "长按主题可以复制、编辑或删除。",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(wait_dialog(&mut app));
    let dialog = app.pending_dialog().cloned().unwrap();
    assert_eq!(dialog.title.as_deref(), Some("删除 Harbor Night？"));
    assert_eq!(
        dialog.message.as_deref(),
        Some("这个主题会被删除。内置主题不受影响。")
    );
    channel.close();
}

#[test]
fn names_follow_dynamic_type() {
    let (mut app, channel) = start(Screen::Theme, Params::default(), "theme");
    post(&mut app, &channel, &themes(vec![], "github-dark", None));
    app.wait_for("id:theme-row-github-dark").unwrap();
    let size = |app: &Headless, s: &str| app.screen().span_style(s).unwrap().font_size.0;
    assert_eq!(size(&app, "text:GitHub Dark"), 17.0);
    app.set_environment("xxxLarge", |env| env.font_scale = 23.0 / 17.0)
        .unwrap();
    assert_eq!(size(&app, "text:GitHub Dark"), 23.0);
    channel.close();
}

// --- The editor ----------------------------------------------------------------

fn draft() -> Draft {
    Draft {
        name: "Harbor Night".into(),
        background: nsh_core::color::rgb(0x0B1420),
        foreground: nsh_core::color::rgb(0xD6E2F0),
        cursor: nsh_core::color::rgb(0xF2C14E),
        ansi: [
            0x1B2633, 0xF07178, 0x9BD67E, 0xF2C14E, 0x6CB6FF, 0xC39BFF, 0x5FD7D7, 0xE6EDF3,
        ]
        .map(nsh_core::color::rgb),
        bright: [None; 8],
    }
}

fn editing(theme: Draft, override_bright: bool) -> String {
    theme_editor::Message::Theme(EditorState {
        theme,
        override_bright,
    })
    .to_json()
}

/// The editor showing `theme`.
fn editor(
    theme: Draft,
    override_bright: bool,
) -> (
    common::App,
    std::sync::Arc<moshpit_ui::channel::Channel>,
    Natives,
) {
    let (mut app, channel, natives) =
        start_observing(Screen::ThemeEditor, Params::default(), "theme-editor");
    post(&mut app, &channel, &editing(theme, override_bright));
    app.wait_for("id:theme-editor-name").unwrap();
    (app, channel, natives)
}

#[test]
fn the_editor_shows_the_theme_in_its_groups() {
    let (app, channel, natives) = editor(draft(), false);
    for text in [
        "Harbor Night",
        "Cancel",
        "Save",
        "Name",
        "TERMINAL",
        "Background",
        "#0B1420",
        "Foreground",
        "Cursor",
        "ANSI COLORS",
        "These eight are what shells, diffs and TUIs paint with.",
        "Black",
        "Magenta",
        "#C39BFF",
        "BRIGHT COLORS",
        "Override bright colors",
        "Bright colors follow the eight above. Many tools draw dim text in bright black, so keep it apart from black.",
        // The preview's lines, each one text of colored runs.
        "❯ git status",
        "On branch main",
        "+ MoshTransport.swift",
        "- MoshpitMark.swift",
        "warning: 2 files changed",
        "hint: use --staged",
        "❯ █",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    // Eleven wells, UIKit's, each with its color and label; no bright rows
    // while the switch is off.
    assert_eq!(natives.count("moshpit/color-well"), 11);
    let background = natives
        .find("moshpit/color-well", "theme-color-background")
        .unwrap();
    assert_eq!(natives.prop(background, "color"), Some(json!("#0B1420FF")));
    assert_eq!(natives.prop(background, "label"), Some(json!("Background")));
    assert!(
        natives
            .find("moshpit/color-well", "theme-color-brightBlack")
            .is_none()
    );
    let switch = natives
        .find("moshpit/switch", "theme-editor-bright")
        .unwrap();
    assert_eq!(natives.prop(switch, "on"), Some(json!(false)));
    assert!(app.screen().matches("label:Theme preview").unwrap());
    channel.close();
}

#[test]
fn a_change_reaches_the_host_and_the_preview() {
    let (mut app, channel, natives) = editor(draft(), false);
    let told = hear(&channel);
    let background = natives
        .find("moshpit/color-well", "theme-color-background")
        .unwrap();
    emit(&mut app, background, "change", json!("#112233"));
    assert_eq!(
        next(&told),
        json!({"ev": "color", "slot": "background", "value": "#112233"})
    );
    assert!(app.screen().shows("#112233"));
    assert_eq!(natives.prop(background, "color"), Some(json!("#112233FF")));
    let red = natives
        .find("moshpit/color-well", "theme-color-red")
        .unwrap();
    emit(&mut app, red, "change", json!("#FF0000"));
    assert_eq!(
        next(&told),
        json!({"ev": "color", "slot": "red", "value": "#FF0000"})
    );
    app.clear("id:theme-editor-name-input").unwrap();
    assert_eq!(next(&told), json!({"ev": "name", "value": ""}));
    assert!(app.screen().shows("New Theme"), "an empty name");
    app.type_text("id:theme-editor-name-input", "Dusk").unwrap();
    for typed in ["D", "Du", "Dus", "Dusk"] {
        assert_eq!(next(&told), json!({"ev": "name", "value": typed}));
    }
    assert!(app.screen().shows("Dusk"), "the title follows the name");
    channel.close();
}

#[test]
fn the_bright_switch_shows_the_bright_rows_and_off_drops_them() {
    let (mut app, channel, natives) = editor(draft(), false);
    let told = hear(&channel);
    let switch = natives
        .find("moshpit/switch", "theme-editor-bright")
        .unwrap();
    emit(&mut app, switch, "change", json!(true));
    assert_eq!(next(&told), json!({"ev": "override-bright", "value": true}));
    assert_eq!(natives.count("moshpit/color-well"), 19);
    assert!(app.screen().matches("id:theme-editor-bright-rows").unwrap());
    assert!(app.screen().shows("Bright Black"));
    // Derived until overridden: black 32% of the way to white.
    let bright_black = natives
        .find("moshpit/color-well", "theme-color-brightBlack")
        .unwrap();
    assert_eq!(
        natives.prop(bright_black, "color"),
        Some(json!("#646B74FF"))
    );
    emit(&mut app, bright_black, "change", json!("#7A7468"));
    assert_eq!(
        next(&told),
        json!({"ev": "color", "slot": "brightBlack", "value": "#7A7468"})
    );
    let switch = natives
        .find("moshpit/switch", "theme-editor-bright")
        .unwrap();
    emit(&mut app, switch, "change", json!(false));
    assert_eq!(
        next(&told),
        json!({"ev": "override-bright", "value": false})
    );
    assert_eq!(natives.count("moshpit/color-well"), 11);
    assert!(!app.screen().matches("id:theme-editor-bright-rows").unwrap());
    quiet(&told);
    channel.close();
}

#[test]
fn a_theme_that_overrides_a_bright_color_opens_with_the_rows() {
    let mut theme = draft();
    theme.bright[0] = Some(nsh_core::color::rgb(0x7A7468));
    let (app, channel, natives) = editor(theme, true);
    assert_eq!(natives.count("moshpit/color-well"), 19);
    let bright_black = natives
        .find("moshpit/color-well", "theme-color-brightBlack")
        .unwrap();
    assert_eq!(
        natives.prop(bright_black, "color"),
        Some(json!("#7A7468FF"))
    );
    assert!(app.screen().shows("#7A7468"));
    channel.close();
}

#[test]
fn save_cancel_and_the_menu_reach_the_host() {
    let (mut app, channel, natives) = editor(draft(), false);
    let told = hear(&channel);
    let more = natives
        .find("moshpit/menu", "theme-editor-more")
        .expect("the bar's menu");
    assert_eq!(
        natives.prop(more, "items"),
        Some(json!([{"id": "copy-json", "title": "Copy as JSON", "image": "doc.on.doc"}]))
    );
    emit(&mut app, more, "select", json!("copy-json"));
    assert_eq!(next(&told), json!({"ev": "copy-json"}));
    app.press("id:theme-editor-save").unwrap();
    assert_eq!(next(&told), json!({"ev": "save"}));
    app.press("id:theme-editor-cancel").unwrap();
    assert_eq!(next(&told), json!({"ev": "cancel"}));
    quiet(&told);
    channel.close();
}

#[test]
fn the_editor_speaks_the_apps_language() {
    let (mut app, channel, natives) =
        start_observing(Screen::ThemeEditor, zh_hans(), "theme-editor");
    post(
        &mut app,
        &channel,
        &editing(
            Draft {
                name: String::new(),
                ..draft()
            },
            true,
        ),
    );
    app.wait_for("id:theme-editor-name").unwrap();
    for text in [
        "新建主题",
        "取消",
        "保存",
        "名称",
        "终端",
        "背景",
        "前景",
        "光标",
        "黑色",
        "品红色",
        "亮黑色",
        "ANSI 颜色",
        "亮色",
        "自定义高亮颜色",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    let well = natives
        .find("moshpit/color-well", "theme-color-brightCyan")
        .unwrap();
    assert_eq!(natives.prop(well, "label"), Some(json!("亮青色")));
    channel.close();
}

#[test]
fn the_editors_text_follows_dynamic_type_and_the_preview_does_not() {
    let (mut app, channel, _) = editor(draft(), false);
    let size = |app: &Headless, s: &str| app.screen().span_style(s).unwrap().font_size.0;
    // A row's title is text-body, its code mono-subhead (§3.1).
    assert_eq!(size(&app, "text:Background"), 17.0);
    assert_eq!(size(&app, "text:#0B1420"), 15.0);
    assert_eq!(size(&app, "text:MoshTransport.swift"), 12.0);
    app.set_environment("xxxLarge", |env| env.font_scale = 23.0 / 17.0)
        .unwrap();
    assert_eq!(size(&app, "text:Background"), 23.0);
    assert_eq!(size(&app, "text:#0B1420"), 21.0);
    assert_eq!(size(&app, "text:MoshTransport.swift"), 12.0);
    channel.close();
}
