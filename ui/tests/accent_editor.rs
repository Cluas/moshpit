//! An accent's editor on a headless engine (R14, `docs/nsh/UI-DESIGN.md`
//! §6.10): the accent the host posts, its preview in the tokens it derives
//! and its text in both appearances, the note when it reads too pale as
//! light-appearance text, the name and the color well (UIKit's, the host's
//! component) reaching the host and the preview, Save and Cancel, the title
//! following the name, the app's language and Dynamic Type.

mod common;

use common::{emit, hear, next, post, quiet, start_observing, zh_hans};
use moshpit_ui::{
    accent_editor::{AccentDraft, Message},
    screen::{Params, Screen},
};
use nsh_testing::Headless;
use serde_json::json;

fn mint(name: &str) -> String {
    colored(name, 0x2FD8A8)
}

fn colored(name: &str, rgb: u32) -> String {
    Message::Accent(AccentDraft {
        name: name.into(),
        accent: nsh_core::color::rgb(rgb),
    })
    .to_json()
}

#[test]
fn the_editor_shows_the_accent_and_its_preview() {
    let (mut app, channel, natives) =
        start_observing(Screen::AccentEditor, Params::default(), "accent-editor");
    post(&mut app, &channel, &mint("Mint Seed"));
    app.wait_for("id:accent-editor-name").unwrap();
    for text in [
        "Mint Seed",
        "Cancel",
        "Save",
        "Connect",
        "mosh",
        "selected row",
        "warn",
        "ok",
        "err",
        "Name",
        "Color",
        "The pressed state and the background wash are derived from this one color.",
        "Dark",
        "Light",
        "Accent",
        "#2FD8A8",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    let well = natives
        .find("moshpit/color-well", "accent-editor-color")
        .expect("the color well");
    assert_eq!(natives.prop(well, "color"), Some(json!("#2FD8A8FF")));
    assert_eq!(natives.prop(well, "label"), Some(json!("Accent")));
    assert!(app.screen().matches("label:Accent preview").unwrap());
    assert_eq!(
        app.screen()
            .input_text("id:accent-editor-name-input")
            .unwrap(),
        Some("Mint Seed")
    );
    assert!(app.screen().matches("id:accent-editor-dark-text").unwrap());
    assert!(app.screen().matches("id:accent-editor-light-text").unwrap());
    channel.close();
}

#[test]
fn a_pale_accent_says_the_light_appearance_darkens_it() {
    let (mut app, channel, natives) =
        start_observing(Screen::AccentEditor, Params::default(), "accent-editor");
    // A deep indigo reads as text on every light surface: no note.
    post(&mut app, &channel, &colored("Deep", 0x3433B8));
    app.wait_for("id:accent-editor-name").unwrap();
    assert!(!app.screen().matches("id:accent-editor-pale").unwrap());
    // Mint is too pale on white: the note shows, and goes with a deep pick.
    let well = natives
        .find("moshpit/color-well", "accent-editor-color")
        .unwrap();
    emit(&mut app, well, "change", json!("#2FD8A8"));
    app.wait_for("id:accent-editor-pale").unwrap();
    assert!(app.screen().shows(
        "As text in the light appearance this color is too pale, so the app darkens it there."
    ));
    channel.close();
}

#[test]
fn a_change_reaches_the_host_and_the_preview() {
    let (mut app, channel, natives) =
        start_observing(Screen::AccentEditor, Params::default(), "accent-editor");
    let told = hear(&channel);
    post(&mut app, &channel, &mint("Mint Seed"));
    app.wait_for("id:accent-editor-name").unwrap();
    let well = natives
        .find("moshpit/color-well", "accent-editor-color")
        .unwrap();
    emit(&mut app, well, "change", json!("#FF8800"));
    assert_eq!(next(&told), json!({"ev": "color", "value": "#FF8800"}));
    assert!(app.screen().shows("#FF8800"));
    assert_eq!(natives.prop(well, "color"), Some(json!("#FF8800FF")));
    // Cleared, the title says what the page is; typed, it is the name.
    app.clear("id:accent-editor-name-input").unwrap();
    assert_eq!(next(&told), json!({"ev": "name", "value": ""}));
    assert!(app.screen().shows("New Accent"));
    app.type_text("id:accent-editor-name-input", "Ember")
        .unwrap();
    for typed in ["E", "Em", "Emb", "Embe", "Ember"] {
        assert_eq!(next(&told), json!({"ev": "name", "value": typed}));
    }
    assert!(app.screen().shows("Ember"), "the title follows the name");
    app.press("id:accent-editor-save").unwrap();
    assert_eq!(next(&told), json!({"ev": "save"}));
    app.press("id:accent-editor-cancel").unwrap();
    assert_eq!(next(&told), json!({"ev": "cancel"}));
    quiet(&told);
    channel.close();
}

#[test]
fn the_editor_speaks_the_apps_language() {
    let (mut app, channel, _) = start_observing(Screen::AccentEditor, zh_hans(), "accent-editor");
    post(&mut app, &channel, &mint(""));
    app.wait_for("id:accent-editor-name").unwrap();
    for text in [
        "新建强调色",
        "取消",
        "保存",
        "连接",
        "选中行",
        "警告",
        "错误",
        "名称",
        "颜色",
        "强调色",
        "深色",
        "浅色",
        "这个颜色在浅色外观下做文字会偏淡，系统会自动调深。",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}

#[test]
fn text_follows_dynamic_type_as_face_resolves_it() {
    let (mut app, channel, _) =
        start_observing(Screen::AccentEditor, Params::default(), "accent-editor");
    post(&mut app, &channel, &mint("Mint Seed"));
    app.wait_for("id:accent-editor-name").unwrap();
    let size = |app: &Headless, s: &str| app.screen().span_style(s).unwrap().font_size.0;
    // The type tokens (§3.1): text-body, mono-caption, mono-subhead.
    assert_eq!(size(&app, "text:Connect"), 17.0);
    assert_eq!(size(&app, "text:warn"), 12.0);
    assert_eq!(size(&app, "text:mosh"), 12.0);
    assert_eq!(size(&app, "text:selected row"), 17.0);
    assert_eq!(size(&app, "text:#2FD8A8"), 15.0);
    app.set_environment("xxxLarge", |env| env.font_scale = 23.0 / 17.0)
        .unwrap();
    assert_eq!(size(&app, "text:Connect"), 23.0);
    assert_eq!(size(&app, "text:warn"), 18.0);
    assert_eq!(size(&app, "text:mosh"), 18.0);
    assert_eq!(size(&app, "text:selected row"), 23.0);
    assert_eq!(size(&app, "text:#2FD8A8"), 21.0);
    channel.close();
}
