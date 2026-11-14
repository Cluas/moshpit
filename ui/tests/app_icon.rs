//! Settings › App Icon on a headless engine: the icons the host posts show
//! in its order, the one in use is ticked (and selected for screen
//! readers), picking one asks the host, iOS's refusal shows in the app's
//! language, back reaches the host, and the SwiftUI page's identifiers.

mod common;

use common::{WAIT, hear, next, post, quiet, start, zh_hans};
use moshpit_ui::{
    app_icon::{Icon, Icons, Message, columns},
    screen::{Params, Screen},
};
use serde_json::json;

fn icons(selected: &str, failure: Option<&str>) -> String {
    let icon = |id: &str, name: &str| Icon {
        id: id.into(),
        name: name.into(),
        image: Some(format!("asset:/IconPreview-{name}@3x.png")),
    };
    Message::Icons(Icons {
        icons: vec![
            icon("default", "Moshpit"),
            icon("pit", "Pit"),
            Icon {
                id: "wq".into(),
                name: ":wq".into(),
                image: None,
            },
        ],
        selected: selected.into(),
        failure: failure.map(str::to_owned),
    })
    .to_json()
}

#[test]
fn the_posted_icons_show_in_order_the_one_in_use_selected() {
    let (mut app, channel) = start(Screen::AppIcon, Params::default(), "app-icon");
    assert!(app.screen().shows("App Icon"));
    assert!(app.screen().matches("role:button[Settings]").unwrap());
    post(&mut app, &channel, &icons("pit", None));
    app.wait_for("id:app-icon-pit").unwrap();
    let texts = app.screen().texts();
    let at = |t: &str| texts.iter().position(|s| *s == t).unwrap();
    assert!(at("Moshpit") < at("Pit") && at("Pit") < at(":wq"));
    assert!(app.screen().selected("id:app-icon-pit").unwrap());
    assert!(!app.screen().selected("id:app-icon-default").unwrap());
    assert!(app.screen().shows(
        "The icon is separate from the accent color, because iOS only allows switching between icons bundled with the app — a custom accent can't have matching artwork generated for it."
    ));
    assert!(!app.screen().matches("id:app-icon-failure").unwrap());
    // The host moves the tick once iOS switched.
    post(&mut app, &channel, &icons("default", None));
    assert!(app.screen().selected("id:app-icon-default").unwrap());
    assert!(!app.screen().selected("id:app-icon-pit").unwrap());
    channel.close();
}

#[test]
fn picking_an_icon_asks_the_host_and_back_reaches_it() {
    let (mut app, channel) = start(Screen::AppIcon, Params::default(), "app-icon");
    let told = hear(&channel);
    post(&mut app, &channel, &icons("default", None));
    app.wait_for("id:app-icon-pit").unwrap();
    app.press("id:app-icon-pit").unwrap();
    assert_eq!(next(&told), json!({"ev": "select", "id": "pit"}));
    // The tick stays until the host says iOS switched.
    assert!(app.screen().selected("id:app-icon-default").unwrap());
    app.press("label::wq").unwrap();
    assert_eq!(next(&told), json!({"ev": "select", "id": "wq"}));
    app.press("id:nav-bar-back").unwrap();
    assert_eq!(next(&told), json!({"ev": "back"}));
    quiet(&told);
    channel.close();
}

#[test]
fn a_refusal_shows_what_ios_said_until_a_change_works() {
    let (mut app, channel) = start(Screen::AppIcon, Params::default(), "app-icon");
    post(
        &mut app,
        &channel,
        &icons("default", Some("The operation couldn’t be completed.")),
    );
    app.wait_for("id:app-icon-failure").unwrap();
    let failure = "Couldn't change the icon: The operation couldn’t be completed.";
    assert!(app.screen().shows(failure));
    // A destructive banner over the grid (§6.10).
    let top = |id: &str| {
        app.screen()
            .layout_nodes()
            .into_iter()
            .find(|node| node.test_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("no view {id}"))
            .bounds
            .origin
            .y
            .0
    };
    assert!(top("app-icon-failure") < top("app-icon-default"));
    post(&mut app, &channel, &icons("pit", None));
    app.wait_for_gone("id:app-icon-failure", WAIT).unwrap();
    channel.close();
}

#[test]
fn the_page_speaks_the_apps_language() {
    let (mut app, channel) = start(Screen::AppIcon, zh_hans(), "app-icon");
    post(&mut app, &channel, &icons("default", Some("错误")));
    app.wait_for("id:app-icon-failure").unwrap();
    assert!(app.screen().shows("应用图标"), "{:?}", app.screen().texts());
    assert!(app.screen().shows("设置"));
    assert!(
        app.screen().shows("无法更换图标：错误"),
        "{:?}",
        app.screen().texts()
    );
    channel.close();
}

#[test]
fn names_follow_dynamic_type_as_caption() {
    let (mut app, channel) = start(Screen::AppIcon, Params::default(), "app-icon");
    post(&mut app, &channel, &icons("default", None));
    app.wait_for("id:app-icon-pit").unwrap();
    let size =
        |app: &nsh_testing::Headless, s: &str| app.screen().span_style(s).unwrap().font_size.0;
    assert_eq!(size(&app, "text:Pit"), 12.0);
    app.set_environment("xxxLarge", |env| env.font_scale = 23.0 / 17.0)
        .unwrap();
    assert_eq!(size(&app, "text:Pit"), 18.0);
    channel.close();
}

#[test]
fn the_grid_is_swiftuis_in_the_sheet() {
    // Settings' sheet on the iPad mini is 580 pt wide: five columns.
    assert_eq!(columns(580.0 - 32.0).0, 5);
}
