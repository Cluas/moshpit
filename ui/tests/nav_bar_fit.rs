//! The bar on a pushed page fits its words (B748, B744's audit): as
//! UIKit's, the back button's title gives way first (to "Back" in the
//! app's language, then to the chevron alone), and only then is the
//! centered title cut, at its tail; a button never leaves the screen, and
//! the bar's words stop growing at extra extra large, as UIKit's.

mod common;

use common::{App, post, settings_fixture::state, start_observing_with};
use moshpit_ui::{
    channel::Channel,
    connection_form::{FormState, Message as FormMessage, Mux},
    screen::{Params, Screen},
    settings::Message as SettingsMessage,
};
use nsh_core::{Bounds, Edges, Points, Size, platform::Idiom};
use nsh_testing::LayoutNode;

const WIDTH: f32 = 402.0;
const HEIGHT: f32 = 874.0;

fn phone(
    screen: Screen,
    locale: &str,
    page: Option<&str>,
    root: &str,
    os: (u16, u16),
) -> (App, std::sync::Arc<Channel>) {
    let params = Params {
        locale: (locale != "en").then(|| locale.to_owned()),
        page: page.map(str::to_owned),
        ..Params::default()
    };
    let (app, channel, _) = start_observing_with(screen, params, root, move |config| {
        config.viewport = Size::new(Points(WIDTH), Points(HEIGHT));
        config.environment.idiom = Idiom::Phone;
        config.environment.os_version = os;
        config.environment.safe_area = Edges {
            top: Points(62.0),
            bottom: Points(34.0),
            left: Points(0.0),
            right: Points(0.0),
        };
    });
    (app, channel)
}

/// iOS 18 (the chevron and the previous page's title) and iOS 26 (Liquid
/// Glass: the chevron alone, in a glass circle).
const OSES: [(u16, u16); 2] = [(18, 2), (26, 0)];

fn settings_page(locale: &str, page: &str, os: (u16, u16)) -> (App, std::sync::Arc<Channel>) {
    let (mut app, channel) = phone(Screen::Settings, locale, Some(page), "settings", os);
    post(
        &mut app,
        &channel,
        &SettingsMessage::Settings(Box::new(state())).to_json(),
    );
    app.wait_for("id:nav-bar-title").unwrap();
    (app, channel)
}

fn form(locale: &str, os: (u16, u16)) -> (App, std::sync::Arc<Channel>) {
    let (mut app, channel) = phone(Screen::ConnectionForm, locale, None, "connection-form", os);
    let draft = FormState {
        editing: false,
        port: "22".into(),
        udp_range: "60000 – 61000".into(),
        socks_port: "1080".into(),
        mux: Mux::Tmux,
        ..FormState::default()
    };
    post(
        &mut app,
        &channel,
        &FormMessage::Form(Box::new(draft)).to_json(),
    );
    app.wait_for("id:nav-bar-title").unwrap();
    (app, channel)
}

fn node(nodes: &[LayoutNode], id: &str) -> Option<LayoutNode> {
    nodes
        .iter()
        .find(|n| n.test_id.as_deref() == Some(id))
        .cloned()
}

fn right(b: &Bounds<Points>) -> f32 {
    b.origin.x.0 + b.size.width.0
}

/// The title's words: the text inside the title's view, as laid out.
fn title(nodes: &[LayoutNode]) -> Bounds<Points> {
    node(nodes, "nav-bar-title").expect("a title").bounds
}

/// Every view of the bar, inside the screen's sides.
fn inside(app: &App, what: &str) {
    let nodes = app.screen().layout_nodes();
    let bar = node(&nodes, "nav-bar").expect("a bar").id;
    let in_bar = |n: &LayoutNode| {
        let mut at = n.parent;
        while let Some(around) = nodes.iter().find(|m| m.id == at) {
            if around.id == bar {
                return true;
            }
            at = around.parent;
        }
        false
    };
    for n in nodes
        .iter()
        .filter(|n| in_bar(n) && !n.hidden && n.bounds.size.width.0 > 0.0)
    {
        assert!(
            n.bounds.origin.x.0 >= -0.5 && right(&n.bounds) <= WIDTH + 0.5,
            "{what}: {:?} {:?} runs off the screen: {:?}",
            n.test_id,
            n.text.as_ref().map(|t| t.text.text.clone()),
            n.bounds
        );
    }
}

/// German's "Einstellungen" beside Settings' pages' titles: the back
/// button and the title never meet (it overlapped all five at the
/// default size).
#[test]
fn a_german_back_title_gives_way_to_the_pages_title() {
    for os in OSES {
        for page in [
            "appearance",
            "terminal",
            "connections",
            "notifications",
            "voice",
        ] {
            let (app, channel) = settings_page("de", page, os);
            let nodes = app.screen().layout_nodes();
            let back = node(&nodes, "nav-bar-back").expect("a back button").bounds;
            let title = title(&nodes);
            assert!(
                right(&back) <= title.origin.x.0 + 0.5,
                "{page} on iOS {os:?}: the back button {back:?} meets the title {title:?}"
            );
            inside(&app, page);
            channel.close();
        }
    }
}

/// French's "Enregistrer" (Save) and "Annuler" (Cancel) beside "Nouvelle
/// connexion": the buttons and the title never meet.
#[test]
fn the_french_forms_buttons_and_title_never_meet() {
    for os in OSES {
        let (app, channel) = form("fr", os);
        let nodes = app.screen().layout_nodes();
        let cancel = node(&nodes, "connection-form-cancel")
            .expect("Cancel")
            .bounds;
        let save = node(&nodes, "save-connection").expect("Save").bounds;
        let title = title(&nodes);
        assert!(
            right(&cancel) <= title.origin.x.0 + 0.5 && right(&title) <= save.origin.x.0 + 0.5,
            "iOS {os:?}: {cancel:?} | {title:?} | {save:?}"
        );
        inside(&app, "form");
        channel.close();
    }
}

/// At the largest text size, in German and French, nothing in a bar runs
/// off the screen: its words stop growing at extra extra large.
#[test]
fn at_ax5_the_bar_stays_on_the_screen() {
    for os in OSES {
        for locale in ["de", "fr", "en"] {
            for page in ["terminal", "notifications", "voice"] {
                let (mut app, channel) = settings_page(locale, page, os);
                app.set_environment("AX5", |env| env.font_scale = 53.0 / 17.0)
                    .unwrap();
                inside(&app, &format!("{locale} {page} iOS {os:?}"));
                let nodes = app.screen().layout_nodes();
                let back = node(&nodes, "nav-bar-back").expect("a back button").bounds;
                assert!(
                    right(&back) <= title(&nodes).origin.x.0 + 0.5,
                    "{locale} {page}"
                );
                channel.close();
            }
            let (mut app, channel) = form(locale, os);
            app.set_environment("AX5", |env| env.font_scale = 53.0 / 17.0)
                .unwrap();
            inside(&app, &format!("{locale} form iOS {os:?}"));
            channel.close();
        }
    }
}

/// In English at the default size, where everything fits, the bars are as
/// they were before B748: the back button says the page under it, and
/// nothing moved (measured on int143).
#[test]
fn english_bars_at_the_default_size_are_as_they_were() {
    let at = |b: Bounds<Points>| (b.origin.x.0, b.size.width.0);
    let (app, channel) = settings_page("en", "terminal", (18, 2));
    let nodes = app.screen().layout_nodes();
    assert_eq!(
        at(node(&nodes, "nav-bar-back").unwrap().bounds),
        (1.0, 114.0)
    );
    assert_eq!(at(title(&nodes)), (163.5, 75.0));
    assert!(app.screen().shows("Settings"), "{:?}", app.screen().texts());
    channel.close();

    let (app, channel) = form("en", (18, 2));
    let nodes = app.screen().layout_nodes();
    assert_eq!(
        at(node(&nodes, "connection-form-cancel").unwrap().bounds),
        (8.0, 72.5)
    );
    assert_eq!(
        at(node(&nodes, "save-connection").unwrap().bounds),
        (340.5, 53.5)
    );
    assert_eq!(at(title(&nodes)), (135.5, 131.0));
    channel.close();
}

/// What the back button gives way to: German's "Einstellungen" beside
/// "Erscheinungsbild" becomes "Zurück"; a title too long for even that
/// leaves the chevron, which still says where it goes.
#[test]
fn the_back_title_gives_way_to_back_then_to_the_chevron() {
    let (app, channel) = settings_page("de", "appearance", (18, 2));
    assert!(app.screen().shows("Zurück"), "{:?}", app.screen().texts());
    assert!(!app.screen().shows("Einstellungen"));
    channel.close();

    // AX5's bar text (extra extra large) and Japanese's widest title.
    let (mut app, channel) = settings_page("de", "notifications", (18, 2));
    app.set_environment("AX5", |env| env.font_scale = 53.0 / 17.0)
        .unwrap();
    let nodes = app.screen().layout_nodes();
    let back = node(&nodes, "nav-bar-back").unwrap().bounds;
    assert!(right(&back) <= title(&nodes).origin.x.0 + 0.5);
    if !app.screen().shows("Zurück") {
        assert!(
            app.screen().matches("label:Einstellungen").unwrap(),
            "the chevron alone still names the page under it"
        );
    }
    channel.close();
}

/// On glass a sheet's buttons are iOS 26's own (the owner's report: Add
/// Connection's Cancel and Save were glass capsules too big beside the
/// title, SSH Keys' ＋ a small accent circle beside Done's capsule): one
/// 44 pt circle each, the xmark to leave, the checkmark to keep, their
/// words for screen readers. Below iOS 26, the words.
#[test]
fn on_glass_a_sheets_buttons_are_the_systems_circles() {
    let circle = |app: &App, id: &str| {
        let nodes = app.screen().layout_nodes();
        let b = node(&nodes, id).unwrap_or_else(|| panic!("no {id}")).bounds;
        assert_eq!((b.size.width.0, b.size.height.0), (44.0, 44.0), "{id}");
    };
    let (app, channel) = form("en", (26, 0));
    circle(&app, "connection-form-cancel");
    circle(&app, "save-connection");
    assert!(!app.screen().shows("Cancel"), "{:?}", app.screen().texts());
    for label in ["label:Cancel", "label:Save"] {
        assert!(app.screen().matches(label).unwrap(), "{label}");
    }
    channel.close();

    let (app, channel) = form("en", (18, 2));
    for words in ["Cancel", "Save"] {
        assert!(app.screen().shows(words), "{words}");
    }
    channel.close();

    let (mut app, channel) = phone(Screen::SshKeys, "en", None, "ssh-keys", (26, 0));
    app.wait_for("id:ssh-keys-add").unwrap();
    circle(&app, "ssh-keys-done");
    circle(&app, "ssh-keys-add");
    channel.close();
}
