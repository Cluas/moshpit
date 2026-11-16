//! A page's button stays inside its page (B749, B744's audit): at the
//! accessibility sizes its words wrap onto a second line, then are cut at
//! their tail; it never runs off the screen, and its words never shrink
//! below the user's size.

mod common;

use common::{App, post, start_observing_with};
use moshpit_ui::{
    channel::Channel,
    home::{Message as HomeMessage, model::HomeState},
    screen::{Params, Screen},
};
use nsh_core::{Bounds, Edges, Points, Size, platform::Idiom};
use nsh_testing::LayoutNode;
use serde_json::json;

const WIDTH: f32 = 402.0;
const HEIGHT: f32 = 874.0;

fn phone(screen: Screen, locale: &str, root: &str) -> (App, std::sync::Arc<Channel>) {
    let params = Params {
        locale: (locale != "en").then(|| locale.to_owned()),
        ..Params::default()
    };
    let (app, channel, _) = start_observing_with(screen, params, root, move |config| {
        config.viewport = Size::new(Points(WIDTH), Points(HEIGHT));
        config.environment.idiom = Idiom::Phone;
        config.environment.safe_area = Edges {
            top: Points(62.0),
            bottom: Points(34.0),
            left: Points(0.0),
            right: Points(0.0),
        };
    });
    (app, channel)
}

fn empty_home(locale: &str) -> (App, std::sync::Arc<Channel>) {
    let (mut app, channel) = phone(Screen::Home, locale, "home");
    let state = HomeState {
        connections: Vec::new(),
        version: "v1.0.9 (1)".into(),
        sidebar: false,
        jump_hosts: 0,
    };
    post(
        &mut app,
        &channel,
        &HomeMessage::Home(Box::new(state)).to_json(),
    );
    app.wait_for("id:home-add-empty").unwrap();
    (app, channel)
}

fn host_key(locale: &str) -> (App, std::sync::Arc<Channel>) {
    let (mut app, channel) = phone(Screen::HostKey, locale, "host-key");
    post(
        &mut app,
        &channel,
        &json!({
            "op": "hostKey", "host": "build-box.example.com", "port": 2222,
            "fingerprint": "SHA256:k3VJxZ2mJ1u4bS9eQq8yF0aJr6w5nTqP7cXo2Lh1dEs"
        })
        .to_string(),
    );
    app.wait_for("id:host-key-trust").unwrap();
    (app, channel)
}

fn node(app: &App, id: &str) -> LayoutNode {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|n| n.test_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no {id}"))
}

/// The views in `id` (it included) that show text.
fn texts_in(app: &App, id: &str) -> Vec<LayoutNode> {
    let nodes = app.screen().layout_nodes();
    let root = node(app, id).id;
    nodes
        .iter()
        .filter(|n| n.text.is_some())
        .filter(|n| {
            let mut at = n.id;
            loop {
                if at == root {
                    return true;
                }
                match nodes.iter().find(|m| m.id == at) {
                    Some(m) if m.parent != at => at = m.parent,
                    _ => return false,
                }
            }
        })
        .cloned()
        .collect()
}

fn on_screen(b: &Bounds<Points>) -> bool {
    b.origin.x.0 >= -0.5 && b.origin.x.0 + b.size.width.0 <= WIDTH + 0.5
}

fn ax5(app: &mut App) {
    app.set_environment("AX5", |env| env.font_scale = 53.0 / 17.0)
        .unwrap();
}

/// Empty Home's Add Connection at AX5, in English, German and French (it
/// ran from −129 to 532 in German), and the host key's "Faire confiance":
/// the button and its words inside the screen, on two lines at most.
#[test]
fn a_page_button_stays_on_the_screen_at_ax5() {
    let mut pages: Vec<(String, App, std::sync::Arc<Channel>, &str)> = Vec::new();
    for locale in ["en", "de", "fr"] {
        let (app, channel) = empty_home(locale);
        pages.push((format!("home {locale}"), app, channel, "home-add-empty"));
    }
    let (app, channel) = host_key("fr");
    pages.push(("host key fr".into(), app, channel, "host-key-trust"));
    for (what, mut app, channel, id) in pages {
        ax5(&mut app);
        let button = node(&app, id);
        assert!(
            on_screen(&button.bounds),
            "{what}: {id} {:?}",
            button.bounds
        );
        for words in texts_in(&app, id) {
            assert!(
                on_screen(&words.bounds),
                "{what}: its words {:?}",
                words.bounds
            );
            let size = words
                .text
                .as_ref()
                .and_then(|t| t.text.spans.first().map(|s| s.style.font_size.0))
                .unwrap();
            assert!(
                size >= 17.0 * 53.0 / 17.0 - 0.5,
                "{what}: never smaller than AX5's body ({size})"
            );
            assert!(
                words.bounds.size.height.0 <= size * 1.3 * 2.0 + 1.0,
                "{what}: two lines at most ({:?})",
                words.bounds
            );
        }
        channel.close();
    }
}

/// At the default size, where they fit, the buttons are as they were
/// (measured on int143).
#[test]
fn default_size_buttons_are_as_they_were() {
    let at = |b: Bounds<Points>| (b.origin.x.0, b.size.width.0, b.size.height.0);
    let (app, channel) = empty_home("en");
    assert_eq!(
        at(node(&app, "home-add-empty").bounds),
        (111.5, 179.0, 50.0)
    );
    channel.close();
    let (app, channel) = host_key("fr");
    assert_eq!(at(node(&app, "host-key-trust").bounds), (16.0, 370.0, 44.0));
    channel.close();
}
