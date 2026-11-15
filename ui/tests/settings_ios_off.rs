//! Settings › Notifications when iOS itself has Moshpit's notifications or
//! Live Activities off (B741). A first-run "Don't Allow" (or Live Activities
//! switched off in iOS) left the app's own switches reading On while
//! nothing could ever show: the page now says what iOS has off and opens
//! iOS's settings for it, and the hub's row no longer says On. The app's own
//! switches stay as the person set them; with one off, what iOS has doesn't
//! matter and nothing is said.

mod common;

use common::{App, hear, next, post, quiet, settings_fixture::state, start};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
    settings::{Message, SettingsState},
};
use serde_json::json;

fn on(part: &str) -> Params {
    Params {
        page: Some(part.into()),
        ..Params::default()
    }
}

fn show(app: &mut App, channel: &Channel, state: &SettingsState) {
    post(
        app,
        channel,
        &Message::Settings(Box::new(state.clone())).to_json(),
    );
}

fn says(app: &App) -> bool {
    app.screen().matches("id:settings-ios-off").unwrap()
}

#[test]
fn the_notifications_page_says_what_ios_has_off_and_opens_its_settings() {
    let (mut app, channel) = start(Screen::Settings, on("notifications"), "settings");
    let mut s = state();
    s.notifications = true;
    s.live_activity = true;
    show(&mut app, &channel, &s);
    app.wait_for("id:settings-notifications-group").unwrap();
    assert!(!says(&app), "both allowed: nothing to say");

    s.notifications_blocked = true;
    show(&mut app, &channel, &s);
    app.wait_for("id:settings-ios-off").unwrap();
    assert!(app.screen().shows("Allow in iOS Settings"));
    assert!(
        app.screen()
            .shows("iOS has notifications off for Moshpit, so no alert reaches you."),
        "{:?}",
        app.screen().texts()
    );
    let told = hear(&channel);
    quiet(&told);
    app.press("id:settings-ios-off").unwrap();
    assert_eq!(next(&told), json!({"ev": "open", "page": "ios-settings"}));

    s.notifications_blocked = false;
    s.live_activity_blocked = true;
    show(&mut app, &channel, &s);
    app.wait_for(
        "text:iOS has Live Activities off for Moshpit, so the Dynamic Island stays empty.",
    )
    .unwrap();

    s.notifications_blocked = true;
    show(&mut app, &channel, &s);
    app.wait_for("text:iOS has notifications and Live Activities off for Moshpit.")
        .unwrap();

    // The app's own switches off: iOS's answer changes nothing.
    s.notifications = false;
    s.live_activity = false;
    show(&mut app, &channel, &s);
    assert!(!says(&app), "{:?}", app.screen().texts());
    // Live Activities on in the app but off in iOS still counts.
    s.live_activity = true;
    show(&mut app, &channel, &s);
    assert!(says(&app));
    channel.close();
}

#[test]
fn the_hub_does_not_say_on_while_ios_has_notifications_off() {
    let (mut app, channel) = start(Screen::Settings, Params::default(), "settings");
    let mut s = state();
    s.notifications = true;
    s.voice_input = false;
    s.notifications_blocked = true;
    show(&mut app, &channel, &s);
    app.wait_for("id:settings-open-notifications").unwrap();
    assert!(
        app.screen().shows("Off in iOS"),
        "{:?}",
        app.screen().texts()
    );
    assert!(!app.screen().texts().contains(&"On"));
    channel.close();
}

/// A host that says nothing of iOS (Android, or one from before) shows the
/// page as before.
#[test]
fn a_host_that_sends_no_flags_has_nothing_off() {
    let mut message: serde_json::Value =
        serde_json::from_str(&Message::Settings(Box::new(state())).to_json()).unwrap();
    let fields = message.as_object_mut().unwrap();
    assert!(fields.remove("notificationsBlocked").is_some());
    assert!(fields.remove("liveActivityBlocked").is_some());
    let Ok(Message::Settings(parsed)) = Message::parse(&message.to_string()) else {
        panic!("parses without the flags");
    };
    assert!(!parsed.notifications_blocked && !parsed.live_activity_blocked);
}
