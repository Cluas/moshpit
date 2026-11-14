//! Settings › Accent on a headless engine (R14, `docs/nsh/UI-DESIGN.md`
//! §6.10): the built-in and custom accents the host posts as a grid of
//! named 44 pt swatches, the one in use ringed (and selected for screen
//! readers), every swatch a 44 pt target, what the user does reaching the
//! host (a pick, New, back, the delete question's answers), the empty
//! custom section, the app's language.

mod common;

use common::{hear, next, post, quiet, start, wait_dialog, zh_hans};
use moshpit_ui::{
    accent::{Accent, Accents, Message},
    screen::{Params, Screen},
};
use nsh_testing::{DialogAnswer, Headless};
use serde_json::json;

fn accent(id: &str, name: &str, rgb: u32) -> Accent {
    Accent {
        id: id.into(),
        name: name.into(),
        accent: nsh_core::color::rgb(rgb),
        screen_bg: nsh_core::color::rgb(0x06070A),
    }
}

fn accents(custom: Vec<Accent>, selected: &str) -> String {
    asking(custom, selected, None)
}

/// [`accents`], the host asking about deleting `deleting`.
fn asking(custom: Vec<Accent>, selected: &str, deleting: Option<&str>) -> String {
    Message::Accents(Accents {
        built_in: vec![
            accent("signal-room", "Signal Room", 0x6C6BEF),
            accent("moshpit-classic", "Teal Line", 0x53DCC9),
        ],
        custom,
        selected: selected.into(),
        deleting: deleting.map(Into::into),
    })
    .to_json()
}

fn mine() -> Accent {
    accent("app-1a2b3c4d", "Mint Seed", 0x2FD8A8)
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
fn the_accents_show_as_named_swatches_the_pick_selected() {
    let (mut app, channel) = start(Screen::Accent, Params::default(), "accent");
    post(
        &mut app,
        &channel,
        &accents(vec![mine()], "moshpit-classic"),
    );
    app.wait_for("id:accent-row-signal-room").unwrap();
    for text in [
        // Section titles are eyebrows: upper case is their style.
        "BUILT-IN",
        "YOURS",
        "Signal Room",
        "Teal Line",
        "Mint Seed",
        "Long-press an accent to edit or delete it.",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(
        app.screen()
            .selected("id:accent-row-moshpit-classic")
            .unwrap()
    );
    assert!(!app.screen().selected("id:accent-row-signal-room").unwrap());
    assert!(!app.screen().matches("id:accent-none").unwrap());
    // Screen readers hear each swatch by its name.
    assert!(app.screen().matches("label:Mint Seed").unwrap());
    // Only a custom accent has the host's menu (which edits); no swatch has
    // a button of its own besides.
    assert!(app.screen().matches("id:accent-cell-app-1a2b3c4d").unwrap());
    assert!(!app.screen().matches("id:accent-cell-signal-room").unwrap());
    assert_eq!(app.screen().count("label:/^Edit /").unwrap(), 0);
    // The circle is 44 pt, and every swatch a 44 pt target at least.
    let circle = bounds(&app, "accent-row-signal-room-color");
    assert_eq!(circle.size.width.0, 44.0);
    for id in [
        "accent-row-signal-room",
        "accent-row-moshpit-classic",
        "accent-row-app-1a2b3c4d",
    ] {
        let target = bounds(&app, id);
        assert!(
            target.size.width.0 >= 44.0 && target.size.height.0 >= 44.0,
            "{id}: {target:?}"
        );
    }
    // Built-in first, then the user's own.
    assert!(bounds(&app, "accent-built-in").origin.y.0 < bounds(&app, "accent-yours").origin.y.0);
    channel.close();
}

#[test]
fn the_grid_fits_its_columns_in_the_card() {
    use moshpit_ui::accent::{COLUMN_GAP, COLUMN_MIN, columns};
    // An iPhone 16 Pro: 402 pt, less the page's and the card's insets.
    let (count, each) = columns(402.0 - 64.0);
    assert_eq!(count, 4);
    assert!(each >= COLUMN_MIN);
    assert!((count as f32 * each + (count as f32 - 1.0) * COLUMN_GAP - 338.0).abs() < 1e-3);
    assert_eq!(columns(10.0).0, 1);
}

#[test]
fn no_custom_accent_says_so() {
    let (mut app, channel) = start(Screen::Accent, Params::default(), "accent");
    post(&mut app, &channel, &accents(vec![], "signal-room"));
    app.wait_for("id:accent-none").unwrap();
    assert!(app.screen().shows("No custom accents yet."));
    post(&mut app, &channel, &accents(vec![mine()], "signal-room"));
    app.wait_for("id:accent-row-app-1a2b3c4d").unwrap();
    assert!(!app.screen().matches("id:accent-none").unwrap());
    channel.close();
}

#[test]
fn what_the_user_does_reaches_the_host() {
    let (mut app, channel) = start(Screen::Accent, Params::default(), "accent");
    let told = hear(&channel);
    post(&mut app, &channel, &accents(vec![mine()], "signal-room"));
    app.wait_for("id:accent-row-moshpit-classic").unwrap();
    app.press("id:accent-row-moshpit-classic").unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "select", "id": "moshpit-classic"})
    );
    app.press("id:accent-row-app-1a2b3c4d").unwrap();
    assert_eq!(next(&told), json!({"ev": "select", "id": "app-1a2b3c4d"}));
    app.press("id:accent-add").unwrap();
    assert_eq!(next(&told), json!({"ev": "add"}));
    app.press("id:nav-bar-back").unwrap();
    assert_eq!(next(&told), json!({"ev": "back"}));
    quiet(&told);
    // The host rings the pick.
    post(
        &mut app,
        &channel,
        &accents(vec![mine()], "moshpit-classic"),
    );
    assert!(
        app.screen()
            .selected("id:accent-row-moshpit-classic")
            .unwrap()
    );
    channel.close();
}

#[test]
fn the_new_button_is_labelled_for_screen_readers() {
    let (app, channel) = start(Screen::Accent, Params::default(), "accent");
    assert!(app.screen().matches("label:New accent").unwrap());
    assert!(app.screen().matches("id:accent-add").unwrap());
    channel.close();
}

#[test]
fn the_page_speaks_the_apps_language() {
    let (mut app, channel) = start(Screen::Accent, zh_hans(), "accent");
    post(&mut app, &channel, &accents(vec![], "signal-room"));
    app.wait_for("id:accent-none").unwrap();
    for text in [
        "强调色",
        "设置",
        "内置",
        "自己做的",
        "还没有自定义强调色。",
        "长按强调色可以编辑或删除。",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(app.screen().matches("label:新建强调色").unwrap());
    channel.close();
}

#[test]
fn names_follow_dynamic_type_as_caption() {
    let (mut app, channel) = start(Screen::Accent, Params::default(), "accent");
    post(&mut app, &channel, &accents(vec![], "signal-room"));
    app.wait_for("id:accent-row-signal-room").unwrap();
    let size = |app: &Headless, s: &str| app.screen().span_style(s).unwrap().font_size.0;
    assert_eq!(size(&app, "text:Signal Room"), 12.0);
    app.set_environment("xxxLarge", |env| env.font_scale = 23.0 / 17.0)
        .unwrap();
    assert_eq!(size(&app, "text:Signal Room"), 18.0);
    channel.close();
}

#[test]
fn delete_asks_in_the_systems_alert_about_the_accent_the_host_names() {
    let (mut app, channel) = start(Screen::Accent, Params::default(), "accent");
    let told = hear(&channel);
    post(
        &mut app,
        &channel,
        &asking(vec![mine()], "signal-room", Some("app-1a2b3c4d")),
    );
    assert!(wait_dialog(&mut app), "the alert never asked");
    let dialog = app.pending_dialog().cloned().unwrap();
    assert_eq!(dialog.title.as_deref(), Some("Delete Mint Seed?"));
    assert_eq!(
        dialog.message.as_deref(),
        Some("This accent will be removed. Built-in accents are unaffected.")
    );
    app.answer_dialog(DialogAnswer::Press("Cancel".into()))
        .unwrap();
    assert_eq!(next(&told), json!({"ev": "cancel-delete"}));
    post(
        &mut app,
        &channel,
        &asking(vec![mine()], "signal-room", Some("app-1a2b3c4d")),
    );
    assert!(wait_dialog(&mut app), "asked again");
    app.answer_dialog(DialogAnswer::Press("Delete".into()))
        .unwrap();
    assert_eq!(next(&told), json!({"ev": "delete", "id": "app-1a2b3c4d"}));
    // A built-in accent is never asked about.
    post(
        &mut app,
        &channel,
        &asking(vec![mine()], "signal-room", Some("signal-room")),
    );
    assert!(!wait_dialog(&mut app));
    quiet(&told);
    channel.close();
}

#[test]
fn the_delete_question_speaks_the_apps_language() {
    let (mut app, channel) = start(Screen::Accent, zh_hans(), "accent");
    post(
        &mut app,
        &channel,
        &asking(vec![mine()], "signal-room", Some("app-1a2b3c4d")),
    );
    assert!(wait_dialog(&mut app));
    let dialog = app.pending_dialog().cloned().unwrap();
    assert_eq!(dialog.title.as_deref(), Some("删除 Mint Seed？"));
    assert_eq!(
        dialog.message.as_deref(),
        Some("这个强调色会被删除。内置强调色不受影响。")
    );
    channel.close();
}
