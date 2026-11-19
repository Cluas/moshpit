//! R12's top bar on a headless engine (UI-DESIGN §6.3, §8): the breadcrumb
//! names `session › project › agent` by Home's rules from the tree the
//! host sends (not the window's automatic name), the waiting agent's
//! segment is amber, every piece of the bar is a 44 pt target, and the
//! chrome follows the terminal theme's appearance, not the app's.

mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
};

use common::{App, hear, post, start_observing};
use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
    theme::AppTheme,
    ui::tokens,
};
use nsh_core::{
    EdgeInsets, NodeId, Points, Size,
    color::Rgba,
    platform::{Idiom, Os},
};
use nsh_engine::EngineConfig;
use serde_json::{Value, json};

/// The next intent the screen told the host, the anchors it reports as it
/// lays out skipped.
fn next_intent(rx: &mpsc::Receiver<String>) -> Value {
    loop {
        let value = common::next(rx);
        if value["ev"] != "anchor" {
            return value;
        }
    }
}

/// The hit slop each view was given, as the engine mounted it.
#[derive(Clone, Default)]
struct Slops(Arc<Mutex<HashMap<NodeId, EdgeInsets>>>);

impl Slops {
    fn apply(&self, batch: &nsh_core::MountBatch) {
        let mut slops = self.0.lock().unwrap();
        for item in &batch.items {
            if let nsh_core::MountItem::ViewProps { id, props } = item {
                slops.insert(*id, props.hit_slop);
            }
        }
    }

    fn of(&self, node: NodeId) -> EdgeInsets {
        self.0
            .lock()
            .unwrap()
            .get(&node)
            .copied()
            .unwrap_or_default()
    }
}

/// The screen on an iPhone-sized engine, the hit slops observed from its
/// first mount.
fn phone() -> (App, Arc<Channel>, Slops) {
    let channel = Channel::open();
    let mut config = EngineConfig::new(Screen::Terminal.root(&channel, &Params::default()));
    config.viewport = Size::new(Points(402.0), Points(874.0));
    config.environment.os = Os::Ios;
    config.environment.os_version = (26, 0);
    config.environment.idiom = Idiom::Phone;
    config.environment.pixel_ratio = 3.0;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = common::headless(config);
    let slops = Slops::default();
    let observed = slops.clone();
    app.observe(move |_, batch| observed.apply(batch));
    app.wait_for("id:terminal-screen").unwrap();
    (App(app), channel, slops)
}

/// A tmux session `train` whose window tmux named after Claude Code's
/// version, the pane working in `~/src/trainer`, its agent waiting.
fn chrome() -> Value {
    json!({
        "op": "chrome",
        "name": "gpu-01",
        "user": "cluas",
        "host": "gpu-01.example.com",
        "port": 22,
        "mosh": true,
        "mux": "tmux",
        "pill": "mosh",
        "state": "live",
        "crumbs": {
            "session": "train",
            "squeezable": true,
            "window": "1:2.1.322",
            "pane": "claude",
            "signal": "attention",
            "paneLabel": "claude, needs you",
            "sessionIcon": "square.stack.3d.up",
            "windowIcon": "macwindow",
            "live": true,
            "tree": {
                "mux": "tmux", "attached": true,
                "sessions": [{"id": "$0", "name": "train"}],
                "windows": [{"id": "@1", "session": "$0", "index": 1, "name": "2.1.322", "paneCount": 1}],
                "panes": [{"id": "%3", "window": "@1", "index": 0, "command": "2.1.322",
                           "path": "/home/cluas/src/trainer"}],
                "hooks": [{"pane": "%3", "state": "attention", "agent": "claude"}],
                "activeSession": "$0", "activeWindow": "@1", "activePane": "%3"
            }
        },
        "cover": {"kind": "none", "state": "live"},
        "content": "terminal"
    })
}

/// The switcher's live tree the host posts beside the chrome (the pane
/// page is drawn from it).
fn switcher() -> Value {
    json!({"op": "switcher", "tree": chrome()["crumbs"]["tree"], "hintSeen": true})
}

/// A token's value in one appearance.
fn token(name: &str, light: bool) -> Rgba {
    tokens::colors(&AppTheme::default())
        .into_iter()
        .find(|(n, ..)| *n == name)
        .map(|(_, l, d)| if light { l } else { d })
        .unwrap_or_else(|| panic!("no token {name}"))
}

/// The color of `text` in the breadcrumb as shown (its measuring copy, an
/// invisible twin laid out first, is the first match).
fn color_of(app: &common::App, text: &str) -> Rgba {
    app.0
        .screen()
        .span_style(&format!("text:{text} >> nth=1"))
        .unwrap()
        .color
        .into()
}

fn close(a: Rgba, b: Rgba) -> bool {
    (a.r - b.r).abs() < 0.01 && (a.g - b.g).abs() < 0.01 && (a.b - b.b).abs() < 0.01
}

#[test]
fn the_breadcrumb_says_session_project_agent() {
    let (mut app, channel, _) =
        start_observing(Screen::Terminal, Params::default(), "terminal-screen");
    let heard = hear(&channel);
    post(&mut app.0, &channel, &chrome().to_string());
    app.0.wait_for("id:terminal-breadcrumb").unwrap();
    for text in ["train", "trainer", "claude", "mosh"] {
        assert!(
            app.0.screen().shows(text),
            "{text}: {:?}",
            app.0.screen().texts()
        );
    }
    assert!(
        !app.0.screen().shows("1:2.1.322"),
        "the window's automatic name"
    );
    app.0.press("label:trainer").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "crumb", "crumb": "window"})
    );
    // The pane's segment opens the window's panes' quick list (the tree
    // here or not).
    post(&mut app.0, &channel, &switcher().to_string());
    app.0.press("label:claude, needs you").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "crumb", "crumb": "pane"}));
    // The agent waits: its words are `warning-text`; the others `foreground`.
    assert!(close(
        color_of(&app, "claude"),
        token("warning-text", false)
    ));
    assert!(close(color_of(&app, "trainer"), token("foreground", false)));
    channel.close();
}

/// The terminal isn't on the device pictures' light list (`draws_light`):
/// its light comes from its terminal theme alone, and the light the host
/// asks for pictures doesn't make a dark terminal theme's chrome light.
#[test]
fn the_terminal_goes_light_from_its_theme_not_the_light_list() {
    assert!(!Screen::Terminal.draws_light());
    for (background, asked, light) in [("#FDF6E3", false, true), ("#0D1117", true, false)] {
        let params = Params {
            terminal: Some(background.to_owned()),
            light: Some(asked),
            ..Params::default()
        };
        let (mut app, channel, _) = start_observing(Screen::Terminal, params, "terminal-screen");
        post(&mut app.0, &channel, &chrome().to_string());
        app.0.wait_for("text:trainer").unwrap();
        assert!(
            close(color_of(&app, "trainer"), token("foreground", light)),
            "{background}, light asked {asked}: the chrome is {}",
            if light { "light" } else { "dark" }
        );
        channel.close();
    }
}

#[test]
fn the_chrome_follows_the_terminal_theme_not_the_app() {
    for (background, light) in [("#FDF6E3", true), ("#0D1117", false)] {
        let params = Params {
            terminal: Some(background.to_owned()),
            ..Params::default()
        };
        let (mut app, channel, _) = start_observing(Screen::Terminal, params, "terminal-screen");
        post(&mut app.0, &channel, &chrome().to_string());
        app.0.wait_for("text:trainer").unwrap();
        assert!(
            close(color_of(&app, "trainer"), token("foreground", light)),
            "{background}: the project's words in the {} appearance",
            if light { "light" } else { "dark" }
        );
        channel.close();
    }
}

#[test]
fn the_bar_pieces_take_a_44_pt_touch() {
    let (mut app, channel, slops) = phone();
    post(&mut app.0, &channel, &chrome().to_string());
    app.0.wait_for("id:terminal-breadcrumb").unwrap();
    for selector in [
        "id:terminal-back",
        "id:terminal-transport-switch",
        "label:trainer",
        "label:claude, needs you",
    ] {
        let node = app
            .0
            .screen()
            .press_target(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("{selector} takes no press"));
        let bounds = app
            .0
            .screen()
            .layout_nodes()
            .into_iter()
            .find(|n| n.id == node)
            .map(|n| n.bounds)
            .unwrap();
        let slop = slops.of(node);
        let height = bounds.size.height.0 + slop.top.0 + slop.bottom.0;
        assert!(height >= 44.0, "{selector}: {height} pt to the touch");
    }
    channel.close();
}

/// B488: the terminal page's ground (the `terminal-bg` token: around the
/// slot, under the bar, the panels' strip) was the app theme's terminal
/// colour, near black, beside a terminal drawn on the user's terminal
/// theme. On the terminal it is that theme's background now, in both
/// appearances; other screens keep the app theme's.
#[test]
fn the_terminal_pages_ground_is_its_terminal_themes_background() {
    use moshpit_ui::screen::{Params, Screen};
    use nsh_core::{color::rgb, element::AdaptiveColor};
    let grey = Params {
        terminal: Some("#282A36".to_owned()),
        ..Params::default()
    };
    let ground = AdaptiveColor::new(rgb(0x282A36), rgb(0x282A36));
    assert_eq!(
        Screen::Terminal.tokens(&grey).color("terminal-bg"),
        Some(ground)
    );
    assert_ne!(
        Screen::Settings.tokens(&grey).color("terminal-bg"),
        Some(ground),
        "only the terminal's"
    );
    let light = Params {
        terminal: Some("#FDF6E3".to_owned()),
        ..Params::default()
    };
    assert_eq!(
        Screen::Terminal.tokens(&light).color("terminal-bg"),
        Some(AdaptiveColor::new(rgb(0xFDF6E3), rgb(0xFDF6E3))),
        "a light terminal theme's too, not the fixed light ground"
    );
    // No terminal theme said: the app theme's, as before.
    assert_eq!(
        Screen::Terminal
            .tokens(&Params::default())
            .color("terminal-bg"),
        Screen::Settings
            .tokens(&Params::default())
            .color("terminal-bg")
    );
}

/// B496: on herdr the grid shows herdr's own ground (its grey), not the
/// terminal theme's; B495 matched the terminal's margins to it, and the
/// page's ground (bar strip, panels) follows the host's `ground` now, as
/// tokens swapped in place.
#[test]
fn the_terminal_pages_ground_follows_the_grids() {
    use moshpit_ui::screen::Params;
    use moshpit_ui::terminal::ground_tokens;
    use nsh_core::{color::rgb, element::AdaptiveColor};
    let params = Params {
        terminal: Some("#000000".to_owned()),
        ..Params::default()
    };
    let grey = AdaptiveColor::new(rgb(0x3A3A3A), rgb(0x3A3A3A));
    assert_eq!(
        ground_tokens(&params, Some("#3A3A3A")).color("terminal-bg"),
        Some(grey),
        "the grid's ground"
    );
    assert_eq!(
        ground_tokens(&params, None).color("terminal-bg"),
        Some(AdaptiveColor::new(rgb(0x000000), rgb(0x000000))),
        "none said: the terminal theme's (B488)"
    );
    // The chrome message carries it.
    let message = serde_json::json!({
        "op": "chrome", "name": "studio", "user": "cluas", "host": "studio.local", "port": 22,
        "mosh": false, "mux": "herdr", "pill": "ssh", "state": "live",
        "cover": {"kind": "none", "state": "live"}, "content": "terminal", "ground": "#3A3A3A"
    });
    assert!(
        moshpit_ui::terminal::Message::parse(&message.to_string()).is_ok(),
        "the page takes a chrome with its ground"
    );
}

/// B650, the owner's herdr: the workspace is named after its folder and
/// its tab ("1", herdr's default) works there. The bar said `moshpit ›
/// moshpit › claude`; it says the tab by its number, its symbol before it.
#[test]
fn the_session_is_never_named_twice() {
    let (mut app, channel, _) =
        start_observing(Screen::Terminal, Params::default(), "terminal-screen");
    let heard = hear(&channel);
    let mut herdr = chrome();
    herdr["mux"] = json!("herdr");
    herdr["crumbs"]["sessionIcon"] = json!("rectangle.grid.2x2");
    herdr["crumbs"]["windowIcon"] = json!("square.on.square");
    herdr["crumbs"]["tree"] = json!({
        "mux": "herdr", "attached": true,
        "sessions": [{"id": "wV", "name": "moshpit"}],
        "windows": [{"id": "wV:t3", "session": "wV", "index": 1, "name": "1", "paneCount": 1}],
        "panes": [{"id": "wV:p4", "window": "wV:t3", "index": 0, "command": "claude",
                   "path": "/Users/dev/code/moshpit"}],
        "hooks": [{"pane": "wV:p4", "state": "working", "agent": "claude"}],
        "activeSession": "wV", "activeWindow": "wV:t3", "activePane": "wV:p4"
    });
    post(&mut app.0, &channel, &herdr.to_string());
    app.0.wait_for("id:terminal-crumb-window-number").unwrap();
    // The breadcrumb is laid out twice (a measuring copy, then the one
    // shown): the workspace's name once in each.
    let said = |text: &str| {
        app.0
            .screen()
            .texts()
            .iter()
            .filter(|t| t.to_string() == text)
            .count()
    };
    assert_eq!(said("moshpit"), 2, "{:?}", app.0.screen().texts());
    assert!(said("1") >= 1, "{:?}", app.0.screen().texts());
    // Still the windows' segment: screen readers hear the tab, a tap opens
    // what the window segment opens.
    app.0.press("label:Tab 1").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "crumb", "crumb": "window"})
    );
    channel.close();
}

/// Each segment of the breadcrumb opens what makes and picks its level:
/// the session's (herdr: workspace's) and the window's (herdr: tab's) quick
/// lists, and the window's panes' quick list, each ending in its New row
/// (New pane). The bar has no ＋ of its own.
#[test]
fn each_segment_opens_its_own_level() {
    let (mut app, channel, natives) =
        start_observing(Screen::Terminal, Params::default(), "terminal-screen");
    let heard = hear(&channel);
    post(&mut app.0, &channel, &chrome().to_string());
    app.0.wait_for("id:terminal-breadcrumb").unwrap();
    assert!(
        natives.find("moshpit/menu", "terminal-new").is_none(),
        "no ＋ menu"
    );
    assert!(!app.0.screen().matches("id:terminal-new").unwrap(), "no ＋");
    app.0.press("label:train").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "crumb", "crumb": "session"})
    );
    app.0.press("label:trainer").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "crumb", "crumb": "window"})
    );
    // The pane's: the window's panes' quick list, the tree here or not
    // (it is the host's list, which ends in New pane): never a page.
    post(&mut app.0, &channel, &switcher().to_string());
    app.0.press("label:claude, needs you").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "crumb", "crumb": "pane"}));
    assert!(
        !app.0.screen().matches("id:terminal-overview").unwrap(),
        "no pane page"
    );

    let mut bare = chrome();
    bare["crumbs"].as_object_mut().unwrap().remove("tree");
    let (mut app, channel2, _) =
        start_observing(Screen::Terminal, Params::default(), "terminal-screen");
    let heard = hear(&channel2);
    post(&mut app.0, &channel2, &bare.to_string());
    app.0.wait_for("id:terminal-breadcrumb").unwrap();
    app.0.press("label:claude, needs you").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "crumb", "crumb": "pane"}));
    channel.close();
    channel2.close();
}

/// The back button is drawn as tall as the breadcrumb beside it (it was
/// 44 pt, taller than every other piece of the bar), and still takes a
/// 44 pt touch.
#[test]
fn the_back_button_is_the_breadcrumbs_height() {
    let (mut app, channel, slops) = phone();
    post(&mut app.0, &channel, &chrome().to_string());
    app.0.wait_for("id:terminal-breadcrumb").unwrap();
    let nodes = app.0.screen().layout_nodes();
    let height_of = |id: &str| {
        nodes
            .iter()
            .find(|n| n.test_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("{id}"))
            .bounds
            .size
    };
    let back = height_of("terminal-back");
    let crumb = height_of("terminal-breadcrumb");
    assert!(
        (back.height.0 - crumb.height.0).abs() < 0.5,
        "back {back:?}, breadcrumb {crumb:?}"
    );
    assert!(back.height.0 < 44.0 && (back.width.0 - back.height.0).abs() < 0.5);
    let node = app
        .0
        .screen()
        .press_target("id:terminal-back")
        .unwrap()
        .unwrap();
    let slop = slops.of(node);
    assert!(back.height.0 + slop.top.0 + slop.bottom.0 >= 44.0);
    assert!(back.width.0 + slop.left.0 + slop.right.0 >= 44.0);
    channel.close();
}
