//! Settings › Send feedback on a headless engine (R16, §6.12): what the
//! host posts shows, the message field starts from the host's text and
//! reports what is typed, the switches are UIKit's with the host's values,
//! Send waits while a send is prepared and carries the message, the banner
//! and the address show when Mail is missing, the GitHub row reaches the
//! host, and the app's language.

mod common;

use common::{emit, hear, next, post, quiet, start, start_observing, zh_hans};
use moshpit_ui::{
    feedback::{FeedbackState, Message},
    screen::{Params, Screen},
};
use serde_json::json;

fn state() -> FeedbackState {
    FeedbackState {
        message: None,
        environment: true,
        log: false,
        preparing: false,
        show_address: false,
        copied: false,
        recipient: "support@cluas.eu.org".into(),
    }
}

fn show(
    app: &mut nsh_testing::Headless,
    channel: &moshpit_ui::channel::Channel,
    state: FeedbackState,
) {
    post(app, channel, &Message::Feedback(state).to_json());
}

#[test]
fn the_page_shows_what_the_host_posted() {
    let (mut app, channel, natives) =
        start_observing(Screen::Feedback, Params::default(), "feedback");
    show(&mut app, &channel, state());
    app.wait_for("id:feedback-message").unwrap();
    for text in [
        "Send feedback",
        "Cancel",
        "Send",
        "Message",
        "Include device and version",
        "Include recent log",
        "The log is the last 30 minutes, the same lines as Recent log.",
        "Open an issue on GitHub",
        "Issues are public; keep security reports to mail.",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    assert!(!app.screen().matches("id:feedback-copy-address").unwrap());
    assert!(!app.screen().matches("id:feedback-no-mail").unwrap());
    let environment = natives
        .find("moshpit/switch", "feedback-environment")
        .unwrap();
    let log = natives.find("moshpit/switch", "feedback-log").unwrap();
    assert_eq!(natives.prop(environment, "on"), Some(json!(true)));
    assert_eq!(natives.prop(log, "on"), Some(json!(false)));
    channel.close();
}

#[test]
fn what_the_user_does_reaches_the_host() {
    let (mut app, channel, natives) =
        start_observing(Screen::Feedback, Params::default(), "feedback");
    let told = hear(&channel);
    show(&mut app, &channel, state());
    app.wait_for("id:feedback-message").unwrap();
    app.type_text("id:feedback-message", "It froze").unwrap();
    let mut last = json!(null);
    while let Ok(event) = told.recv_timeout(std::time::Duration::from_millis(300)) {
        last = serde_json::from_str(&event).unwrap();
    }
    assert_eq!(last, json!({"ev": "text", "value": "It froze"}));
    let log = natives.find("moshpit/switch", "feedback-log").unwrap();
    emit(&mut app, log, "change", json!(true));
    assert_eq!(
        next(&told),
        json!({"ev": "set", "key": "log", "value": true})
    );
    app.press("id:feedback-send").unwrap();
    assert_eq!(next(&told), json!({"ev": "send", "message": "It froze"}));
    app.press("id:feedback-cancel").unwrap();
    assert_eq!(next(&told), json!({"ev": "cancel"}));
    app.press("id:feedback-github-issue").unwrap();
    assert_eq!(next(&told), json!({"ev": "github-issue"}));
    quiet(&told);
    channel.close();
}

#[test]
fn send_waits_while_a_send_is_prepared() {
    let (mut app, channel) = start(Screen::Feedback, Params::default(), "feedback");
    let told = hear(&channel);
    show(
        &mut app,
        &channel,
        FeedbackState {
            preparing: true,
            ..state()
        },
    );
    app.wait_for("id:feedback-message").unwrap();
    // Send holds the spinner: busy for screen readers, no press.
    assert!(!app.screen().enabled("id:feedback-send").unwrap());
    assert!(
        app.screen()
            .accessibility_state("id:feedback-send")
            .unwrap()
            .busy
    );
    let _ = app.press("id:feedback-send");
    show(&mut app, &channel, state());
    assert!(app.screen().enabled("id:feedback-send").unwrap());
    assert!(
        !app.screen()
            .accessibility_state("id:feedback-send")
            .unwrap()
            .busy
    );
    quiet(&told);
    channel.close();
}

#[test]
fn the_field_starts_from_the_hosts_text() {
    let (mut app, channel) = start(Screen::Feedback, Params::default(), "feedback");
    show(
        &mut app,
        &channel,
        FeedbackState {
            message: Some("Half a sentence".into()),
            ..state()
        },
    );
    app.wait_for("id:feedback-message").unwrap();
    assert_eq!(
        app.screen().input_text("id:feedback-message").unwrap(),
        Some("Half a sentence")
    );
    channel.close();
}

#[test]
fn without_mail_the_address_shows_to_copy() {
    let (mut app, channel) = start(Screen::Feedback, Params::default(), "feedback");
    let told = hear(&channel);
    show(
        &mut app,
        &channel,
        FeedbackState {
            show_address: true,
            ..state()
        },
    );
    app.wait_for("id:feedback-copy-address").unwrap();
    assert!(app.screen().matches("id:feedback-no-mail").unwrap());
    assert!(app.screen().shows("support@cluas.eu.org"));
    assert!(app.screen().shows("Copy address"));
    assert!(app.screen().shows("Mail isn't set up on this device"));
    app.press("id:feedback-copy-address").unwrap();
    assert_eq!(next(&told), json!({"ev": "copy-address"}));
    show(
        &mut app,
        &channel,
        FeedbackState {
            show_address: true,
            copied: true,
            ..state()
        },
    );
    app.wait_for("text:Copied").unwrap();
    channel.close();
}

#[test]
fn the_message_field_is_four_lines_tall_at_least() {
    let (mut app, channel) = start(Screen::Feedback, Params::default(), "feedback");
    show(&mut app, &channel, state());
    app.wait_for("id:feedback-message").unwrap();
    let id = app.screen().id("id:feedback-message").unwrap();
    let field = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.id == id)
        .unwrap()
        .bounds;
    // Four body lines (22 pt apart at the default size), the first a
    // line's frame.
    assert!(field.size.height.0 >= 3.0 * 22.0 + 20.0, "{field:?}");
    channel.close();
}

#[test]
fn the_page_speaks_the_apps_language() {
    let (mut app, channel) = start(Screen::Feedback, zh_hans(), "feedback");
    show(&mut app, &channel, state());
    app.wait_for("id:feedback-message").unwrap();
    for text in [
        "反馈问题",
        "取消",
        "发送",
        "内容",
        "附上设备和版本",
        "附上最近日志",
        "在 GitHub 上提 issue",
    ] {
        assert!(
            app.screen().texts().iter().any(|t| t.contains(text)),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    channel.close();
}
