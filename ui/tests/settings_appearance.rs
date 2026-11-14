//! Settings › Appearance's choice of appearance (R18, UI-DESIGN §6.4):
//! system, light or dark, a segmented control below the accent and the
//! app icon.

mod common;

use common::{emit, hear, next, post, settings_fixture::state, start_observing, zh_hans};
use moshpit_ui::{
    screen::{AppearanceChoice, Params, Screen},
    settings::{Message, SettingsState},
};
use serde_json::json;

/// Settings' Appearance page, with the choice shown, in `params`' language.
fn appearance_page(
    params: Params,
) -> (
    common::App,
    std::sync::Arc<moshpit_ui::channel::Channel>,
    common::Natives,
) {
    start_observing(
        Screen::Settings,
        Params {
            page: Some("appearance".into()),
            ..params
        },
        "settings",
    )
}

fn show(
    app: &mut nsh_testing::Headless,
    channel: &moshpit_ui::channel::Channel,
    state: SettingsState,
) {
    post(app, channel, &Message::Settings(Box::new(state)).to_json());
}

/// The control: its three choices, the one stored ticked; a pick tells the
/// host the setting.
#[test]
fn the_appearance_is_a_choice_of_three() {
    let (mut app, channel, natives) = appearance_page(Params::default());
    let told = hear(&channel);
    let mut chosen = state();
    chosen.appearance = AppearanceChoice::Light;
    show(&mut app, &channel, chosen);
    app.wait_for("id:settings-accent").unwrap();
    let control = natives
        .find("moshpit/segmented", "settings-appearance")
        .expect("the appearance's control");
    assert_eq!(
        natives.prop(control, "items"),
        Some(json!(["System", "Light", "Dark"]))
    );
    assert_eq!(natives.prop(control, "selected"), Some(json!(1)));
    assert!(app.screen().shows("Appearance"));
    for (index, value) in [(2, "dark"), (0, "system"), (1, "light")] {
        emit(&mut app, control, "change", json!(index));
        assert_eq!(
            next(&told),
            json!({"ev": "set", "key": "appearance", "value": value})
        );
    }
    // A host that sends no appearance: the system's.
    let mut unset: serde_json::Value =
        serde_json::from_str(&Message::Settings(Box::new(state())).to_json()).unwrap();
    unset.as_object_mut().unwrap().remove("appearance");
    post(&mut app, &channel, &unset.to_string());
    assert_eq!(natives.prop(control, "selected"), Some(json!(0)));
    channel.close();
}

/// In Chinese: 跟随系统 / 浅色 / 深色.
#[test]
fn in_the_apps_language() {
    let (mut app, channel, natives) = appearance_page(zh_hans());
    show(&mut app, &channel, state());
    app.wait_for("id:settings-accent").unwrap();
    let control = natives
        .find("moshpit/segmented", "settings-appearance")
        .unwrap();
    assert_eq!(
        natives.prop(control, "items"),
        Some(json!(["跟随系统", "浅色", "深色"]))
    );
    assert!(app.screen().shows("外观"));
    channel.close();
}
