//! The terminal screen on a headless engine: what the host posts shows (the
//! bar's crumbs, the pill, the covers, the empty state, the banners, the
//! shortcut bar, the panels), what the person does reaches the host as
//! intents, the remembered breadcrumb is inert, a rekey tells the host
//! before the terminal's element is made again, the app's language, and
//! the shell's text stopping at extra large.

mod common;

use std::time::Duration;

use common::{Natives, hear, post, start, start_observing, zh_hans};
use moshpit_ui::screen::{Params, Screen};
use nsh_testing::Headless;
use serde_json::{Value, json};

/// The next intent the screen told the host, the anchors it reports as it
/// lays out skipped.
fn next_intent(rx: &std::sync::mpsc::Receiver<String>) -> Value {
    loop {
        let value = common::next(rx);
        if value["ev"] != "anchor" {
            return value;
        }
    }
}

/// Nothing but anchors told.
fn no_intent(rx: &std::sync::mpsc::Receiver<String>) {
    while let Ok(json) = rx.recv_timeout(Duration::from_millis(200)) {
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["ev"], "anchor", "the screen told the host {value}");
    }
}

fn chrome() -> Value {
    json!({
        "op": "chrome",
        "name": "studio",
        "user": "cluas",
        "host": "studio.local",
        "port": 22,
        "mosh": false,
        "mux": "tmux",
        "pill": "ssh",
        "state": "live",
        "crumbs": {
            "session": "moshpit",
            "squeezable": true,
            "window": "2:agents",
            "pane": "claude",
            "signal": "attention",
            "paneLabel": "claude, needs you",
            "sessionIcon": "square.stack.3d.up",
            "windowIcon": "macwindow",
            "live": true
        },
        "cover": {"kind": "none", "state": "live"},
        "content": "terminal"
    })
}

fn with(mut base: Value, changes: Value) -> Value {
    for (key, value) in changes.as_object().unwrap() {
        base[key] = value.clone();
    }
    base
}

fn bar() -> Value {
    json!({
        "op": "bar",
        "chips": [
            {"id": "esc", "kind": "key", "label": "esc"},
            {"id": "tab", "kind": "key", "label": "tab"},
            {"id": "ctrl", "kind": "ctrl", "label": "ctrl"},
            {"id": "arrows", "kind": "arrows", "label": ""},
            {"id": "paste", "kind": "paste", "label": "paste"},
            {"id": "mic", "kind": "mic", "label": ""},
            {"id": "dpad", "kind": "dpad", "label": ""},
            {"id": "scroll", "kind": "scroll", "label": ""}
        ],
        "keyboardDown": true,
        "ctrlArmed": false,
        "voice": true,
        "micActive": false,
        "image": {"clipboard": false, "camera": true, "sent": []}
    })
}

fn open(
    params: Params,
) -> (
    common::App,
    std::sync::Arc<moshpit_ui::channel::Channel>,
    Natives,
) {
    start_observing(Screen::Terminal, params, "terminal-screen")
}

fn show(app: &mut Headless, channel: &moshpit_ui::channel::Channel, message: &Value) {
    post(app, channel, &message.to_string());
}

#[test]
fn the_bar_shows_the_plan_and_its_crumbs_open_their_sheets() {
    let (mut app, channel, natives) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &chrome());
    app.wait_for("id:terminal-breadcrumb").unwrap();
    for text in ["SSH", "moshpit", "2:agents", "claude"] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    // The terminal's slot and the bar's glass are the host's.
    assert_eq!(natives.count("moshpit/terminal"), 1);
    // The back button is the `ui` layer's glass icon button (R12).
    assert!(app.screen().matches("id:terminal-back").unwrap());

    app.press("label:moshpit").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "crumb", "crumb": "session"})
    );
    app.press("label:2:agents").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "crumb", "crumb": "window"})
    );
    app.press("label:claude, needs you").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "crumb", "crumb": "pane"}));
    app.press("id:terminal-back").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "back"}));
    no_intent(&heard);
    channel.close();
}

#[test]
fn the_pill_asks_before_it_switches_and_hangs_the_question_from_itself() {
    let (mut app, channel, _) = open(Params::default());
    app.answer_dialogs_myself();
    let heard = hear(&channel);
    show(&mut app, &channel, &chrome());
    app.wait_for("id:terminal-transport-switch").unwrap();
    app.press("id:terminal-transport-switch").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "dialog-anchor", "name": "pill"})
    );
    assert!(common::wait_dialog(&mut app), "the sheet never asked");
    let dialog = app.pending_dialog().cloned().expect("the sheet asks");
    assert!(dialog.sheet);
    assert_eq!(dialog.title.as_deref(), Some("Switch to Mosh?"));
    assert_eq!(
        dialog.message.as_deref(),
        Some("Reconnects this session over Mosh.")
    );
    let texts: Vec<_> = dialog.buttons.iter().map(|b| b.text.clone()).collect();
    assert_eq!(texts, ["Switch", "Cancel"]);
    app.answer_dialog(nsh_testing::DialogAnswer::Press("Cancel".into()))
        .unwrap();
    no_intent(&heard);
    // Over mosh it offers SSH; Switch switches.
    show(
        &mut app,
        &channel,
        &with(chrome(), json!({"mosh": true, "pill": "mosh"})),
    );
    app.press("id:terminal-transport-switch").unwrap();
    assert_eq!(next_intent(&heard)["ev"], "dialog-anchor");
    assert!(common::wait_dialog(&mut app), "the sheet never asked");
    assert_eq!(
        app.pending_dialog()
            .and_then(|d| d.title.clone())
            .as_deref(),
        Some("Switch to SSH?")
    );
    app.answer_dialog(nsh_testing::DialogAnswer::Press("Switch".into()))
        .unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "switch-transport"}));
    no_intent(&heard);
    channel.close();
}

#[test]
fn behind_a_jump_host_the_pill_asks_how_mosh_udp_gets_there() {
    let (mut app, channel, _) = open(Params::default());
    app.answer_dialogs_myself();
    let heard = hear(&channel);
    show(&mut app, &channel, &with(chrome(), json!({"jump": true})));
    app.wait_for("id:terminal-transport-switch").unwrap();
    app.press("id:terminal-transport-switch").unwrap();
    assert_eq!(next_intent(&heard)["ev"], "dialog-anchor");
    assert!(common::wait_dialog(&mut app), "the alert never asked");
    let dialog = app.pending_dialog().cloned().expect("the alert asks");
    assert_eq!(dialog.title.as_deref(), Some("Use mosh over UDP?"));
    let texts: Vec<_> = dialog.buttons.iter().map(|b| b.text.clone()).collect();
    assert_eq!(
        texts,
        ["UDP straight to the host", "UDP forwarded by the jump host", "Cancel"]
    );
    app.answer_dialog(nsh_testing::DialogAnswer::Press("UDP straight to the host".into()))
        .unwrap();
    let intent = next_intent(&heard);
    assert_eq!(intent["ev"], "jump-route");
    assert_eq!(intent["route"], "direct");
    // Cancel picks nothing.
    app.press("id:terminal-transport-switch").unwrap();
    assert_eq!(next_intent(&heard)["ev"], "dialog-anchor");
    assert!(common::wait_dialog(&mut app));
    app.answer_dialog(nsh_testing::DialogAnswer::Press("Cancel".into()))
        .unwrap();
    no_intent(&heard);
    channel.close();
}

/// The jump-host notice's "Use UDP…" asks the same, and goes.
#[test]
fn the_jump_host_notice_offers_udp_in_place() {
    let (mut app, channel, _) = open(Params::default());
    app.answer_dialogs_myself();
    let heard = hear(&channel);
    show(&mut app, &channel, &with(chrome(), json!({"jump": true, "jumpSsh": true})));
    app.wait_for("id:terminal-jump-notice-udp").unwrap();
    app.press("id:terminal-jump-notice-udp").unwrap();
    assert_eq!(next_intent(&heard)["ev"], "dialog-anchor");
    assert!(common::wait_dialog(&mut app), "the alert never asked");
    app.answer_dialog(nsh_testing::DialogAnswer::Press(
        "UDP forwarded by the jump host".into(),
    ))
    .unwrap();
    let intent = next_intent(&heard);
    assert_eq!(intent["ev"], "jump-route");
    assert_eq!(intent["route"], "viaJump");
    channel.close();
}

/// Through a jump host with mosh's UDP on a route of its own, the pill is a
/// normal switch (to SSH, for good), and the dead-return-path banner names
/// the end to check and asks "this time only" or "from now on".
#[test]
fn over_a_jump_route_the_banner_names_the_end_to_check_and_asks_how_long_to_use_ssh() {
    let (mut app, channel, _) = open(Params::default());
    app.answer_dialogs_myself();
    let heard = hear(&channel);
    let banner = |route: &str| {
        with(
            chrome(),
            json!({
                "mosh": true, "jumpMosh": true,
                "banner": {"kind": "returnPath", "route": route, "ports": "60000–61000"},
            }),
        )
    };
    show(&mut app, &channel, &banner("direct"));
    app.wait_for(
        "text:No UDP reply from mosh-server. Through a jump host, check that the target host allows UDP 60000–61000.",
    )
    .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
    show(&mut app, &channel, &banner("viaJump"));
    app.wait_for(
        "text:No UDP reply from mosh-server. Through a jump host, check that the jump host forwards UDP 60000–61000 and the target host allows it.",
    )
    .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));

    app.press("id:mosh-returnpath-switch-ssh").unwrap();
    assert_eq!(next_intent(&heard)["ev"], "dialog-anchor");
    assert!(common::wait_dialog(&mut app), "the sheet never asked");
    let dialog = app.pending_dialog().cloned().expect("the sheet asks");
    assert_eq!(dialog.title.as_deref(), Some("Use SSH?"));
    let texts: Vec<_> = dialog.buttons.iter().map(|b| b.text.clone()).collect();
    assert_eq!(texts, ["This time only", "From now on", "Cancel"]);
    app.answer_dialog(nsh_testing::DialogAnswer::Press("This time only".into()))
        .unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "banner-action"}));

    app.press("id:mosh-returnpath-switch-ssh").unwrap();
    assert_eq!(next_intent(&heard)["ev"], "dialog-anchor");
    assert!(common::wait_dialog(&mut app), "the sheet never asked");
    app.answer_dialog(nsh_testing::DialogAnswer::Press("From now on".into()))
        .unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "switch-transport"}));

    // Not through a jump host: the banner's action stays the plain one.
    show(
        &mut app,
        &channel,
        &with(chrome(), json!({"banner": {"kind": "returnPath"}})),
    );
    app.wait_for("text:No UDP reply from mosh-server.").unwrap();
    app.press("id:mosh-returnpath-switch-ssh").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "banner-action"}));
    channel.close();
}

#[test]
fn mosh_counters_hang_from_the_pill_while_the_host_shows_them() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(
        &mut app,
        &channel,
        &with(chrome(), json!({"mosh": true, "pill": "mosh"})),
    );
    app.wait_for("id:terminal-transport-switch").unwrap();
    app.long_press("id:terminal-transport-switch").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "diagnostics"}));
    assert!(!app.screen().shows("MOSH DIAGNOSTICS"), "the host shows it");
    show(
        &mut app,
        &channel,
        &json!({"op": "diagnostics", "shown": true, "counters": null}),
    );
    app.wait_for("id:mosh-diagnostics-overlay").unwrap();
    assert!(app.screen().shows("MOSH DIAGNOSTICS"));
    assert!(app.screen().shows("No datagrams yet."));
    show(
        &mut app,
        &channel,
        &json!({"op": "diagnostics", "shown": true, "counters": {
            "datagrams": 412, "applied": 37, "parseFails": 0, "gaps": 2,
            "sent": 90, "user": 88, "acked": 88, "channel": "waiting", "held": true
        }}),
    );
    for text in [
        "datagrams",
        "412",
        "parse fails",
        "acked",
        "waiting",
        "input held until the server answers",
    ] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    app.press("id:mosh-diagnostics-repaint").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "repaint"}));
    show(
        &mut app,
        &channel,
        &json!({"op": "diagnostics", "shown": false}),
    );
    std::thread::sleep(Duration::from_millis(400));
    app.settle();
    assert!(!app.screen().shows("MOSH DIAGNOSTICS"), "it faded out");
    channel.close();
}

#[test]
fn the_host_hears_where_the_popovers_hang() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &chrome());
    app.wait_for("id:terminal-breadcrumb").unwrap();
    // A wider capsule moves the breadcrumb: both tell the host again.
    show(
        &mut app,
        &channel,
        &with(chrome(), json!({"pill": "mosh", "mosh": true})),
    );
    app.wait_for("text:mosh").unwrap();
    let mut seen = std::collections::HashMap::new();
    while let Ok(json) = heard.recv_timeout(Duration::from_millis(500)) {
        let value: Value = serde_json::from_str(&json).unwrap();
        if value["ev"] == "anchor" {
            seen.insert(
                value["name"].as_str().unwrap().to_owned(),
                value["rect"].clone(),
            );
        }
    }
    let crumb = seen.get("breadcrumb").expect("the breadcrumb's place");
    let pill = seen.get("pill").expect("the pill's place");
    let x = |r: &Value, i: usize| r[i].as_f64().unwrap();
    assert!(x(crumb, 0) > x(pill, 0), "the breadcrumb is after the pill");
    assert!(x(crumb, 2) > 20.0 && x(crumb, 3) >= 32.0, "{crumb}");
    channel.close();
}

#[test]
fn the_remembered_plan_is_dimmed_and_inert() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    let mut retained = chrome();
    retained["crumbs"]["live"] = json!(false);
    retained["state"] = json!("reconnecting");
    show(&mut app, &channel, &retained);
    app.wait_for("text:moshpit").unwrap();
    let _ = app.press("label:2:agents");
    no_intent(&heard);
    channel.close();
}

#[test]
fn without_a_tree_the_bar_names_the_connection() {
    let (mut app, channel, _) = open(Params::default());
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"crumbs": null, "mux": "herdr", "pill": "mosh", "mosh": true}),
        ),
    );
    app.wait_for("text:cluas@studio.local").unwrap();
    assert!(app.screen().shows("herdr"));
    assert!(app.screen().shows("mosh"));
    channel.close();
}

#[test]
fn the_sidebar_button_stands_in_for_back_on_the_ipad() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(
        &mut app,
        &channel,
        &with(chrome(), json!({"sidebar": true})),
    );
    app.wait_for("id:terminal-sidebar").unwrap();
    app.press("id:terminal-sidebar").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "sidebar"}));
    channel.close();
}

#[test]
fn a_first_connect_has_no_cover_and_a_reconnect_only_the_veil() {
    let (mut app, channel, natives) = open(Params::default());
    // B657: a first connect is the terminal's background; the bar's pill
    // says it connects. No poster: no big mark, no grid, no name in the
    // middle of the pane.
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"crumbs": null, "state": "connecting", "content": "none",
                   "cover": {"kind": "none", "state": "connecting"}}),
        ),
    );
    app.wait_for("id:terminal-pane-area").unwrap();
    app.run_for(Duration::from_millis(400)).unwrap();
    assert_eq!(
        app.screen().count("id:terminal-connecting").unwrap(),
        0,
        "the poster"
    );
    assert_eq!(natives.count("moshpit/pane-loader"), 0, "the big mark");
    assert_eq!(app.screen().count("id:terminal-reconnect-veil").unwrap(), 0);
    assert!(
        !app.screen().shows("Connecting…"),
        "the pane says nothing: {:?}",
        app.screen().texts()
    );
    // A host that still says "poster" is refused, not drawn.
    assert!(
        moshpit_ui::terminal::Message::parse(
            &with(
                chrome(),
                json!({"cover": {"kind": "poster", "state": "connecting"}})
            )
            .to_string()
        )
        .is_err(),
        "no poster to draw"
    );
    // A reconnect over the kept frame: the veil, its capsule.
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"state": "reconnecting",
                   "cover": {"kind": "veil", "state": "reconnecting"}}),
        ),
    );
    app.wait_for("id:terminal-reconnect-veil").unwrap();
    app.wait_for("id:terminal-reconnect-capsule").unwrap();
    assert!(app.screen().shows("Reconnecting…"));
    // Nothing being attempted: the phone waits for a network; an ended
    // session says so.
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"mosh": true, "state": "offline",
                   "cover": {"kind": "veil", "state": "offline"}}),
        ),
    );
    app.wait_for("text:Waiting for network…").unwrap();
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"state": "offline", "cover": {"kind": "veil", "state": "offline", "ended": true}}),
        ),
    );
    app.wait_for("text:Session ended").unwrap();
    channel.close();
}

#[test]
fn a_veil_fading_out_keeps_saying_what_it_covered() {
    let (mut app, channel, _) = open(Params::default());
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"state": "offline", "cover": {"kind": "veil", "state": "offline"}}),
        ),
    );
    app.wait_for("text:Waiting for network…").unwrap();
    // The host's cover is gone (its state a default, connecting): the veil
    // fades out still saying what it covered, never the default's words.
    // As the host posts no cover: kind none, its state a default.
    channel
        .post(
            &with(
                chrome(),
                json!({"cover": {"kind": "none", "state": "connecting"}}),
            )
            .to_string(),
        )
        .unwrap();
    assert!(
        app.wait_for_within("text:Reconnecting…", Duration::from_millis(600))
            .is_err(),
        "the fading veil read the host's default state"
    );
    app.wait_for_gone("id:terminal-reconnect-veil", Duration::from_secs(5))
        .unwrap();
    channel.close();
}

#[test]
fn the_follower_veil_takes_the_window_back_on_a_tap() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"cover": {"kind": "follow", "state": "live", "owner": "iPad", "touches": true}}),
        ),
    );
    app.wait_for("id:terminal-size-follow").unwrap();
    assert!(
        app.screen().shows("This window is sized for iPad."),
        "{:?}",
        app.screen().texts()
    );
    // The card's one button takes it back (this test's device is an iPad).
    assert!(
        app.screen().shows("Use this iPad's size"),
        "{:?}",
        app.screen().texts()
    );
    app.press("id:terminal-size-follow-take").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "take-over"}));
    channel.close();
}

#[test]
fn the_empty_state_waits_then_offers_what_the_host_lacks() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"crumbs": null, "content": "empty",
                   "empty": {"mux": "tmux", "missing": false,
                             "session": "Session", "sessionLower": "session",
                             "sessionsLower": "sessions"}}),
        ),
    );
    app.wait_for("id:terminal-empty-state").unwrap();
    // The wait is the bare background (no poster, B657), then answers.
    assert_eq!(app.screen().count("id:terminal-connecting").unwrap(), 0);
    app.wait_for_within(
        "id:multiplexer-create-first-session",
        Duration::from_secs(8),
    )
    .unwrap();
    assert!(
        app.screen().shows("No tmux sessions on studio"),
        "{:?}",
        app.screen().texts()
    );
    app.press("id:multiplexer-create-first-session").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "create"}));
    // While it creates, the button says so and makes no second session
    // (`new-session` is not idempotent) — SwiftUI's `.disabled(creating)`.
    app.wait_for_within("text:Creating…", Duration::from_secs(2))
        .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
    app.press("id:multiplexer-create-first-session")
        .unwrap_or(());
    no_intent(&heard);
    // The creating failed (the controller's notice): Create is back.
    let empty = json!({"mux": "tmux", "missing": false,
                       "session": "Session", "sessionLower": "session",
                       "sessionsLower": "sessions"});
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"crumbs": null, "content": "empty", "empty": empty,
                   "notice": {"text": "Not connected to tmux yet. Try again in a moment."}}),
        ),
    );
    app.wait_for_within("text:New Session", Duration::from_secs(2))
        .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
    app.press("id:multiplexer-create-first-session").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "create"}));
    channel.close();
}

/// The empty state's chrome: tmux with no sessions (nothing over the
/// terminal's ground while it waits), `settled` as the host says.
fn no_sessions(settled: bool) -> Value {
    with(
        chrome(),
        json!({"crumbs": null, "content": "empty",
               "empty": {"mux": "tmux", "missing": false, "settled": settled,
                         "session": "Session", "sessionLower": "session",
                         "sessionsLower": "sessions"}}),
    )
}

/// The host that knows the answer already (the server said it has no
/// session: B228, B337) says so: the answer shows at once, with no blank
/// seconds before it.
#[test]
fn an_answer_the_host_knows_shows_at_once() {
    let (mut app, channel, _) = open(Params::default());
    show(&mut app, &channel, &no_sessions(true));
    app.wait_for_within(
        "id:multiplexer-create-first-session",
        Duration::from_millis(300),
    )
    .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
    assert!(app.screen().shows("No tmux sessions on studio"));
    // Not installed, known from the host's probe: the same.
    let mut missing = no_sessions(true);
    missing["empty"]["missing"] = json!(true);
    missing["empty"]["mux"] = json!("herdr");
    show(
        &mut app,
        &channel,
        &with(chrome(), json!({"content": "terminal", "empty": null})),
    );
    show(&mut app, &channel, &missing);
    app.wait_for_within("id:multiplexer-install", Duration::from_millis(300))
        .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
    channel.close();
}

/// B789: the empty state is the pane's own, a centred column: the symbol
/// over the words over New Session, each centred in the pane. It was a
/// group's inline one, the symbol before the words and all of it on the
/// leading edge of a block left of centre.
#[test]
fn the_empty_state_is_a_centred_column() {
    let (mut app, channel, _) = open(Params::default());
    show(&mut app, &channel, &no_sessions(true));
    app.wait_for_within(
        "id:multiplexer-create-first-session",
        Duration::from_millis(300),
    )
    .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
    let nodes = app.screen().layout_nodes();
    let bounds = |selector: &str| {
        let id = app.screen().id(selector).unwrap();
        nodes
            .iter()
            .find(|node| node.id == id)
            .unwrap_or_else(|| panic!("{selector} is laid out"))
            .bounds
    };
    let centre = |b: nsh_core::Bounds<nsh_core::Points>| b.origin.x.0 + b.size.width.0 / 2.0;
    let pane = bounds("id:terminal-empty-state");
    let words = bounds("text:No tmux sessions on studio");
    let button = bounds("id:multiplexer-create-first-session");
    for (what, b) in [("the words", words), ("New Session", button)] {
        assert!(
            (centre(b) - centre(pane)).abs() < 1.0,
            "{what} centred at {} in a pane centred at {}",
            centre(b),
            centre(pane)
        );
    }
    assert!(
        button.origin.y.0 >= words.origin.y.0 + words.size.height.0,
        "New Session under the words"
    );
    channel.close();
}

/// An answer the host doesn't know yet (the attach may only be on its way)
/// still waits out the attach handshake's seconds, and the host saying it
/// knows mid-wait ends the wait there.
#[test]
fn an_answer_not_known_yet_waits_unless_the_host_learns_it() {
    let (mut app, channel, _) = open(Params::default());
    show(&mut app, &channel, &no_sessions(false));
    app.wait_for("id:terminal-empty-state").unwrap();
    assert!(
        app.wait_for_within(
            "id:multiplexer-create-first-session",
            Duration::from_secs(2)
        )
        .is_err(),
        "answered before the grace ran out"
    );
    show(&mut app, &channel, &no_sessions(true));
    app.wait_for_within(
        "id:multiplexer-create-first-session",
        Duration::from_millis(300),
    )
    .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
    channel.close();

    // Never learnt: the grace, then the answer.
    let (mut app, channel, _) = open(Params::default());
    show(&mut app, &channel, &no_sessions(false));
    app.wait_for_within(
        "id:multiplexer-create-first-session",
        Duration::from_secs(8),
    )
    .unwrap();
    channel.close();
}

#[test]
fn the_banner_says_what_is_wrong_and_offers_its_remedy() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(
        &mut app,
        &channel,
        &with(chrome(), json!({"banner": {"kind": "returnPath"}})),
    );
    app.wait_for("id:mosh-returnpath-banner").unwrap();
    assert!(app.screen().shows("No UDP reply from mosh-server."));
    // What was typed meanwhile waits for mosh-server, kept until it acks
    // (B618): the banner says it goes once the connection is back (B854).
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"banner": {"kind": "returnPath", "typed": true}}),
        ),
    );
    app.wait_for(
        "text:No UDP reply from mosh-server. What you've typed will be sent when the connection is back.",
    )
    .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
    app.press("id:mosh-returnpath-switch-ssh").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "banner-action"}));
    app.press("id:mosh-returnpath-dismiss").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "banner-dismiss"}));
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"banner": {"kind": "degrade", "missing": "tmux"}}),
        ),
    );
    app.wait_for("id:host-banner").unwrap();
    assert!(
        app.screen()
            .shows("tmux isn't installed here, so this is a plain shell.")
    );
    app.press("id:host-banner-install").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "banner-action"}));
    // Being installed: nothing to ask, the host reconnects once it's in.
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"banner": {"kind": "degrade", "missing": "tmux", "installing": true}}),
        ),
    );
    app.wait_for("id:host-banner-installing").unwrap();
    assert!(
        app.screen()
            .shows("Installing. Moshpit reconnects on its own when it's done."),
        "{:?}",
        app.screen().texts()
    );
    assert!(app.screen().id("id:host-banner-install").is_err());
    app.press("id:host-banner-dismiss").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "banner-dismiss"}));
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"banner": {"kind": "roam", "srtt": 182.4, "history": [120, 180, 140]}}),
        ),
    );
    app.wait_for("id:roam-banner").unwrap();
    assert!(
        app.screen()
            .shows("Network changed. mosh is reconnecting. · 182 ms"),
        "{:?}",
        app.screen().texts()
    );
    channel.close();
}

#[test]
fn a_notice_offers_its_remedy_and_a_copied_link_opens() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"notice": {"text": "This pane is open on another device", "action": "Open here"},
                   "link": "github.com"}),
        ),
    );
    app.wait_for("text:This pane is open on another device")
        .unwrap();
    app.press("id:terminal-notice-action").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "notice-action"}));
    app.press("id:terminal-open-link").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "open-link"}));
    channel.close();
}

#[test]
fn the_shortcut_bar_sends_its_keys() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &chrome());
    show(&mut app, &channel, &bar());
    app.wait_for("label:esc").unwrap();
    app.press("label:esc").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "chip", "id": "esc"}));
    app.press("label:Control").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "chip", "id": "ctrl"}));
    app.press("id:arrow-left").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "arrow", "dir": "left"}));
    app.press("id:arrow-up").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "arrow", "dir": "up"}));
    app.press("label:Paste").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "chip", "id": "paste"}));
    app.press("id:voice-input").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "mic"}));
    app.press("id:keyboard-toggle").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "keyboard"}));
    assert!(app.screen().matches("label:Show keyboard").unwrap());
    // Armed ctrl and the keyboard up read on the chips.
    let mut armed = bar();
    armed["ctrlArmed"] = json!(true);
    armed["keyboardDown"] = json!(false);
    show(&mut app, &channel, &armed);
    app.wait_for("label:Control armed").unwrap();
    assert!(app.screen().matches("label:Hide keyboard").unwrap());
    // Voice input off: no mic.
    let mut quiet_bar = bar();
    quiet_bar["voice"] = json!(false);
    show(&mut app, &channel, &quiet_bar);
    app.wait_for_gone("id:voice-input", Duration::from_secs(5))
        .unwrap();
    no_intent(&heard);
    channel.close();
}

#[test]
fn a_held_key_repeats_until_the_finger_lifts() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &chrome());
    show(&mut app, &channel, &bar());
    app.wait_for("label:esc").unwrap();
    app.long_press("label:esc").unwrap();
    // The long press (held past 0.3 s) sends at once and then repeats.
    assert_eq!(next_intent(&heard), json!({"ev": "chip", "id": "esc"}));
    // It lifted: the repeats stop.
    std::thread::sleep(Duration::from_millis(300));
    while let Ok(json) = heard.recv_timeout(Duration::from_millis(50)) {
        let value: Value = serde_json::from_str(&json).unwrap();
        assert!(value["ev"] == "chip" || value["ev"] == "anchor");
    }
    no_intent(&heard);
    channel.close();
}

#[test]
fn the_voice_panel_composes_then_commits_on_the_persons_word() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &chrome());
    show(&mut app, &channel, &bar());
    show(
        &mut app,
        &channel,
        &json!({"op": "dictation", "panel": {
            "phase": {"phase": "listening"}, "engine": "Apple · en-US",
            "finalized": "", "volatile": "", "insertable": true
        }}),
    );
    app.wait_for("id:dictation-overlay").unwrap();
    assert!(app.screen().shows("LISTENING"));
    assert!(app.screen().shows("Apple · en-US"));
    post(&mut app, &channel, r#"{"op":"level","level":0.6}"#);
    app.press("id:dictation-send").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "dictation", "action": "send"})
    );
    show(
        &mut app,
        &channel,
        &json!({"op": "dictation", "panel": {
            "phase": {"phase": "failed", "message": "Moshpit can't use the microphone.", "settings": true}
        }}),
    );
    app.wait_for("text:Open Settings").unwrap();
    app.press("text:Open Settings").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "dictation", "action": "settings"})
    );
    show(
        &mut app,
        &channel,
        &json!({"op": "dictation", "panel": null}),
    );
    app.wait_for_gone("id:dictation-overlay", Duration::from_secs(5))
        .unwrap();
    channel.close();
}

#[test]
fn the_voice_panel_shows_exactly_what_insert_types() {
    // B673: the host sends the guess with the space its join puts before it
    // (none between Chinese and an English word), and the panel adds none.
    let (mut app, channel, _) = open(Params::default());
    show(&mut app, &channel, &chrome());
    show(&mut app, &channel, &bar());
    show(
        &mut app,
        &channel,
        &json!({"op": "dictation", "panel": {
            "phase": {"phase": "listening"}, "engine": "Apple · zh-CN",
            "finalized": "帮我把这个", "volatile": "PR merge 一下", "insertable": true
        }}),
    );
    app.wait_for("id:dictation-overlay").unwrap();
    assert!(app.screen().shows("帮我把这个PR merge 一下"));
    assert!(!app.screen().shows("帮我把这个 PR merge 一下"));
    channel.close();
}

#[test]
fn the_image_and_preview_panels_offer_their_buttons() {
    let (mut app, channel, natives) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &chrome());
    show(
        &mut app,
        &channel,
        &json!({"op": "attachment", "panel": {
            "phase": {"phase": "ready"},
            "images": [{"id": "a#thumb", "landed": true, "number": 3}],
            "size": "1.2 MB"
        }}),
    );
    app.wait_for("id:image-attachment-overlay").unwrap();
    assert!(app.screen().shows("READY TO INSERT"));
    assert!(app.screen().shows("#3"));
    assert_eq!(natives.count("moshpit/thumbnail"), 1);
    app.press("id:image-attach-insert").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "attachment", "action": "insert"})
    );
    show(
        &mut app,
        &channel,
        &json!({"op": "preview", "panel": {
            "phase": {"phase": "confirm", "size": "84 MB"}, "name": "demo.mov", "symbol": "film"
        }}),
    );
    app.wait_for("id:remote-preview-hud").unwrap();
    assert!(
        app.screen()
            .shows("This file is 84 MB. Download it to preview?")
    );
    app.press("text:Download").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "preview", "action": "confirm"})
    );
    channel.close();
}

#[test]
fn a_preview_downloading_shows_the_systems_bar_on_its_track() {
    let (mut app, channel, natives) = open(Params::default());
    show(&mut app, &channel, &chrome());
    show(
        &mut app,
        &channel,
        &json!({"op": "preview", "panel": {
            "phase": {"phase": "downloading", "fraction": 0.25}, "name": "demo.mov", "symbol": "film"
        }}),
    );
    app.wait_for("id:remote-preview-hud").unwrap();
    let bar = natives.only("moshpit/progress").unwrap();
    assert_eq!(natives.prop(bar, "value"), Some(json!(0.25)));
    assert_eq!(natives.prop(bar, "label"), Some(json!("DOWNLOADING 25%")));
    assert!(
        natives.prop(bar, "track").is_some(),
        "the bar lies on the muted track"
    );
    channel.close();
}

#[test]
fn a_rekey_tells_the_host_before_the_terminal_moves() {
    let (mut app, channel, natives) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &chrome());
    let first = natives.only("moshpit/terminal").unwrap();
    post(&mut app, &channel, r#"{"op":"rekey"}"#);
    assert_eq!(next_intent(&heard), json!({"ev": "handover"}));
    let second = natives.only("moshpit/terminal").unwrap();
    assert_ne!(first, second, "a new element");
    assert_eq!(natives.prop(second, "generation"), Some(json!(1)));
    channel.close();
}

#[test]
fn the_shell_speaks_the_apps_language() {
    let (mut app, channel, _) = open(zh_hans());
    show(
        &mut app,
        &channel,
        &with(
            chrome(),
            json!({"cover": {"kind": "follow", "state": "live", "owner": "iPad", "touches": true}}),
        ),
    );
    app.wait_for("id:terminal-size-follow").unwrap();
    let texts = app.screen().texts().join("|");
    assert!(texts.contains("这个窗口按 iPad 的尺寸显示。"), "{texts}");
    assert!(texts.contains("改用这台 iPad 的尺寸"), "{texts}");
    channel.close();
}

#[test]
fn bad_messages_are_refused() {
    let (_app, channel) = start(Screen::Terminal, Params::default(), "terminal-screen");
    assert!(channel.post(r#"{"op":"chrome","colour":1}"#).is_err());
    assert!(channel.post(r#"{"op":"focus"}"#).is_err());
    assert!(channel.post(r#"{"op":"level","level":"loud"}"#).is_err());
    assert!(channel.post(&chrome().to_string()).is_ok());
    let heard = hear(&channel);
    no_intent(&heard);
    channel.close();
}

#[test]
fn command_keys_ask_what_the_menu_says_once_per_press() {
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &chrome());
    app.wait_for("id:terminal-breadcrumb").unwrap();
    // NSH's keymap (the presses the terminal leaves to it).
    app.press_key("mod+k").unwrap();
    assert_eq!(
        next_intent(&heard),
        json!({"ev": "crumb", "crumb": "window"})
    );
    app.press_key("mod+2").unwrap();
    assert_eq!(next_intent(&heard), json!({"ev": "window", "number": 2}));
    // The host's key commands (the scene's menu took the press first).
    post(&mut app, &channel, r#"{"op":"key","stroke":"mod+w"}"#);
    assert_eq!(next_intent(&heard), json!({"ev": "back"}));
    // The same press reaching the keymap too runs once.
    app.press_key("mod+w").unwrap();
    no_intent(&heard);
    // Without a live tree there are no windows to switch.
    show(
        &mut app,
        &channel,
        &with(chrome(), json!({"crumbs": null, "mux": "none"})),
    );
    post(&mut app, &channel, r#"{"op":"key","stroke":"mod+k"}"#);
    post(&mut app, &channel, r#"{"op":"key","stroke":"mod+1"}"#);
    no_intent(&heard);
    channel.close();
}

#[test]
fn behind_a_jump_host_a_notice_says_ssh_once_and_goes_by_itself_or_on_tap() {
    let words = "This connection goes through a jump host over SSH. mosh can use UDP instead.";
    let (mut app, channel, _) = open(Params::default());
    let heard = hear(&channel);
    show(&mut app, &channel, &chrome());
    app.wait_for("id:terminal-breadcrumb").unwrap();
    assert!(!app.screen().shows(words), "no jump host, no notice");
    show(&mut app, &channel, &with(chrome(), json!({"jump": true, "jumpSsh": true})));
    app.wait_for("id:terminal-jump-notice")
        .unwrap_or_else(|e| panic!("{e}: {:?}", app.screen().texts()));
    assert!(app.screen().shows(words));
    // Not a modal: nothing was asked of the host.
    no_intent(&heard);
    app.press("id:terminal-jump-notice-dismiss").unwrap();
    for _ in 0..40 {
        if !app.screen().shows(words) {
            break;
        }
        app.run_for(Duration::from_millis(100)).unwrap();
    }
    assert!(!app.screen().shows(words), "tapped away");
    // Once per session: the next chrome with the jump flag doesn't bring it back.
    show(
        &mut app,
        &channel,
        &with(chrome(), json!({"jump": true, "jumpSsh": true, "name": "again"})),
    );
    app.run_for(Duration::from_millis(500)).unwrap();
    assert!(!app.screen().shows(words), "once per session");
    channel.close();
}

#[test]
fn a_roam_over_in_a_moment_shows_no_banner() {
    // Back from the background, iOS reports a new path and mosh's next
    // reply ends the roam: the banner used to flash by on every return.
    let (mut app, channel, _) = open(Params::default());
    show(
        &mut app,
        &channel,
        &with(chrome(), json!({"banner": {"kind": "roam", "srtt": 40.0, "history": []}})),
    );
    for _ in 0..5 {
        app.run_for(Duration::from_millis(100)).unwrap();
        assert!(
            !app.screen().shows("Network changed. mosh is reconnecting. · 40 ms"),
            "no banner within the grace"
        );
    }
    show(&mut app, &channel, &chrome());
    for _ in 0..25 {
        app.run_for(Duration::from_millis(100)).unwrap();
        assert!(
            !app.screen().shows("Network changed. mosh is reconnecting. · 40 ms"),
            "a roam that ended never shows"
        );
    }
    channel.close();
}

#[test]
fn the_jump_host_notice_goes_by_itself() {
    let words = "This connection goes through a jump host over SSH. mosh can use UDP instead.";
    let (mut app, channel, _) = open(Params::default());
    show(&mut app, &channel, &with(chrome(), json!({"jump": true, "jumpSsh": true})));
    app.wait_for("id:terminal-jump-notice").unwrap();
    for _ in 0..100 {
        if !app.screen().shows(words) {
            break;
        }
        app.run_for(Duration::from_millis(100)).unwrap();
    }
    assert!(!app.screen().shows(words), "gone after its hold");
    channel.close();
}
