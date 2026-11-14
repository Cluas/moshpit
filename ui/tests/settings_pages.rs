//! Settings' pages that a page of the hub's pushes (R18): Connections › UDP
//! port range, an edit page of two number fields (UI-DESIGN §6.4) whose
//! Save keeps a range the rule allows and says under the field what is
//! wrong with one it doesn't; and Notifications › How notifications work,
//! the explainer. Both were SwiftUI sheets on the dark-only palette.

mod common;

use common::{App, hear, next, post, quiet, settings_fixture::state, start, zh_hans};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
    settings::{Message, Part, PortsError, Side, udp_ports},
};
use serde_json::json;

/// Settings on `part`, in `params`' language.
fn on(part: &str, params: Params) -> Params {
    Params {
        page: Some(part.into()),
        ..params
    }
}

/// The page, with the fixture's settings posted (a UDP range of
/// 60000–61000).
fn open(part: &str, params: Params) -> (App, std::sync::Arc<Channel>) {
    let (mut app, channel) = start(Screen::Settings, on(part, params), "settings");
    post(
        &mut app,
        &channel,
        &Message::Settings(Box::new(state())).to_json(),
    );
    (app, channel)
}

/// A field's input, by the field's id.
fn input(id: &str) -> String {
    format!("id:{id}-input")
}

#[test]
fn the_pushed_pages_are_parts_a_host_can_run() {
    for part in Part::PUSHED {
        assert_eq!(Part::of(Some(part.id())), part, "{}", part.id());
        // Not a page of the hub's: a host routing the hub's rows by
        // `named` (Android) opens them as before.
        assert_eq!(Part::named(Some(part.id())), Part::Hub, "{}", part.id());
    }
    for part in Part::PAGES {
        assert_eq!(Part::of(Some(part.id())), part, "{}", part.id());
    }
    assert_eq!(Part::of(None), Part::Hub);
    assert_eq!(Part::of(Some("mystery")), Part::Hub);
}

/// The SwiftUI editor's rule (`UDPPortRange.validate`), case by case, with
/// the field each mistake shows under.
#[test]
fn the_udp_range_rule_is_the_swiftui_editors() {
    for (from, to, want) in [
        ("60000", "61000", Ok([60000, 61000])),
        (" 1 ", "65535", Ok([1, 65535])),
        ("5", "5", Ok([5, 5])),
        ("", "61000", Err(PortsError::NotNumeric(Side::From))),
        ("6.5", "7", Err(PortsError::NotNumeric(Side::From))),
        ("60000", "x", Err(PortsError::NotNumeric(Side::To))),
        ("x", "y", Err(PortsError::NotNumeric(Side::From))),
        ("0", "10", Err(PortsError::OutOfBounds(Side::From))),
        ("-1", "10", Err(PortsError::OutOfBounds(Side::From))),
        ("10", "65536", Err(PortsError::OutOfBounds(Side::To))),
        ("61000", "60000", Err(PortsError::Inverted)),
    ] {
        assert_eq!(udp_ports(from, to), want, "{from:?}–{to:?}");
    }
    // Inverted shows under To; the words are checked on the page.
    assert_eq!(PortsError::Inverted.side(), Side::To);
    assert_eq!(PortsError::OutOfBounds(Side::From).side(), Side::From);
}

#[test]
fn the_udp_range_page_shows_the_stored_range_under_its_title() {
    let (mut app, channel) = open("udp-range", Params::default());
    app.wait_for("id:settings-udp-group").unwrap();
    for text in [
        "UDP Port Range",
        "From",
        "To",
        "Save",
        "mosh binds one UDP port in this range per session. Open it outbound from your iPhone and inbound on the server (default 60000–61000).",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert_eq!(
        app.screen()
            .input_text(&input("settings-udp-from"))
            .unwrap(),
        Some("60000")
    );
    assert_eq!(
        app.screen().input_text(&input("settings-udp-to")).unwrap(),
        Some("61000")
    );
    channel.close();
}

#[test]
fn save_keeps_a_changed_range_and_goes_back() {
    let (mut app, channel) = open("udp-range", Params::default());
    let told = hear(&channel);
    app.wait_for("id:settings-udp-group").unwrap();
    app.clear(&input("settings-udp-from")).unwrap();
    app.type_text(&input("settings-udp-from"), "50000").unwrap();
    app.clear(&input("settings-udp-to")).unwrap();
    app.type_text(&input("settings-udp-to"), "50010").unwrap();
    app.press("id:settings-udp-save").unwrap();
    assert_eq!(
        next(&told),
        json!({"ev": "set", "key": "udpRange", "value": [50000, 50010]})
    );
    assert_eq!(next(&told), json!({"ev": "back"}));
    quiet(&told);
    channel.close();
}

#[test]
fn save_of_the_range_as_it_was_only_goes_back() {
    let (mut app, channel) = open("udp-range", Params::default());
    let told = hear(&channel);
    app.wait_for("id:settings-udp-group").unwrap();
    app.press("id:settings-udp-save").unwrap();
    assert_eq!(next(&told), json!({"ev": "back"}));
    quiet(&told);
    channel.close();
}

/// A range the rule refuses is kept from the host: what is wrong shows
/// under the field it names, and goes once that field is edited.
#[test]
fn a_wrong_range_says_why_under_its_field_and_stays() {
    let (mut app, channel) = open("udp-range", Params::default());
    let told = hear(&channel);
    app.wait_for("id:settings-udp-group").unwrap();
    app.clear(&input("settings-udp-to")).unwrap();
    app.type_text(&input("settings-udp-to"), "100").unwrap();
    app.press("id:settings-udp-save").unwrap();
    quiet(&told);
    assert!(app.screen().shows("From must be ≤ To."));
    // Editing the field takes the error away; Return on To saves again.
    app.type_text(&input("settings-udp-to"), "0").unwrap();
    assert!(!app.screen().shows("From must be ≤ To."));
    app.clear(&input("settings-udp-to")).unwrap();
    app.type_text(&input("settings-udp-to"), "70000\n").unwrap();
    quiet(&told);
    assert!(app.screen().shows("Ports must be 1–65535."));
    // A wrong From shows under From, not To.
    app.clear(&input("settings-udp-to")).unwrap();
    app.type_text(&input("settings-udp-to"), "61000").unwrap();
    app.clear(&input("settings-udp-from")).unwrap();
    app.press("id:settings-udp-save").unwrap();
    quiet(&told);
    assert!(app.screen().shows("Enter numeric ports."));
    app.type_text(&input("settings-udp-from"), "60000").unwrap();
    assert!(!app.screen().shows("Enter numeric ports."));
    app.press("id:settings-udp-save").unwrap();
    assert_eq!(next(&told), json!({"ev": "back"}));
    channel.close();
}

#[test]
fn the_explainer_says_how_notifications_work() {
    let (mut app, channel) = open("notification-info", Params::default());
    let told = hear(&channel);
    app.wait_for("id:settings-notification-info-group").unwrap();
    for text in [
        "How notifications work",
        "Agents stamp their state",
        "Live Activity",
        "Push, sealed end-to-end",
        "Quiet by design",
        "A question must stand for 30 seconds before any phone hears about it — answered at your desk means never announced. All waiting agents share one summary card; only the first rings. A finished turn shows the prompt it answered and only chimes if it ran three minutes or more. Parked agents stay silent.",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    for n in 0..4 {
        assert!(
            app.screen()
                .matches(&format!("id:settings-notification-info-{n}"))
                .unwrap(),
            "part {n}"
        );
    }
    assert!(!app.screen().matches("id:settings-udp-save").unwrap());
    app.press("id:nav-bar-back").unwrap();
    assert_eq!(next(&told), json!({"ev": "back"}));
    channel.close();
}

#[test]
fn the_pages_speak_the_apps_language() {
    let (mut app, channel) = open("udp-range", zh_hans());
    app.wait_for("id:settings-udp-group").unwrap();
    for text in ["UDP 端口范围", "起始", "结束", "保存"] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
    let (mut app, channel) = open("notification-info", zh_hans());
    app.wait_for("id:settings-notification-info-group").unwrap();
    for text in ["通知是怎么工作的", "智能体自报状态"] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}
