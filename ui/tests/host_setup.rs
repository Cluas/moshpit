//! Settings › Set up a host on a headless engine (§6.8, R16): each state
//! the core can be in (looking at the host, a host it couldn't read, a
//! fresh host, installed, out of date, paired with nothing to push, a
//! phone with no push service credential, a host missing tools, a step or
//! a test running), what each row tells the host, the agent menu and its
//! statuses, the questions asked in the system's alert, the app's language
//! and its text size.

mod common;

use common::{emit, hear, next, post, quiet, start, start_observing, wait_dialog, zh_hans};
use moshpit_ui::{
    host_setup::{Agent, Facts, HostSetup, Message, Phase, Proof, ProofState, Status, Tool},
    screen::{Params, Screen},
};
use nsh_testing::DialogAnswer;
use serde_json::json;

fn tools(missing: &[&str]) -> Vec<Tool> {
    ["openssl", "curl", "jq", "tmux"]
        .map(|name| Tool {
            name: name.into(),
            present: !missing.contains(&name),
        })
        .into()
}

fn claude() -> Agent {
    Agent {
        id: "claude".into(),
        name: "Claude Code".into(),
        config_hint: "~/.claude/settings.json".into(),
        status: Status::Absent,
        found: Some(true),
    }
}

fn codex() -> Agent {
    Agent {
        id: "codex".into(),
        name: "Codex".into(),
        config_hint: "~/.codex/config.toml".into(),
        status: Status::Absent,
        found: Some(false),
    }
}

fn proof(state: ProofState, message: Option<&str>) -> Proof {
    Proof {
        state,
        message: message.map(str::to_owned),
    }
}

fn untested() -> Proof {
    proof(ProofState::Untested, None)
}

fn fresh() -> HostSetup {
    HostSetup {
        phase: Phase::Idle,
        phase_label: None,
        facts: Some(Facts {
            uname: "Darwin".into(),
            tools: tools(&[]),
            missing_for_push: vec![],
        }),
        error: None,
        nothing_will_push: false,
        agent: claude(),
        agents: vec![claude(), codex()],
        hooks: Status::Absent,
        user_action: None,
        confirm: None,
        hooks_proof: untested(),
        relay_error: None,
        paired_relay: None,
        relay_url: "https://push.example.com".into(),
        pairing: Status::Absent,
        push_proof: untested(),
        removable: false,
    }
}

/// Hooks and the pairing current, something of Moshpit's on the host.
fn installed() -> HostSetup {
    HostSetup {
        hooks: Status::Current,
        pairing: Status::Current,
        paired_relay: Some("https://push.example.com".into()),
        agent: Agent {
            status: Status::Current,
            ..claude()
        },
        agents: vec![
            Agent {
                status: Status::Current,
                ..claude()
            },
            codex(),
        ],
        removable: true,
        ..fresh()
    }
}

fn post_setup(
    app: &mut nsh_testing::Headless,
    channel: &moshpit_ui::channel::Channel,
    setup: HostSetup,
) {
    post(app, channel, &Message::HostSetup(Box::new(setup)).to_json());
}

/// `setup` with its statuses unknown: a host not read yet, or not at all.
fn post_unknown(
    app: &mut nsh_testing::Headless,
    channel: &moshpit_ui::channel::Channel,
    setup: HostSetup,
) {
    let mut message: serde_json::Value =
        serde_json::from_str(&Message::HostSetup(Box::new(setup)).to_json()).unwrap();
    message["hooks"] = json!("unknown");
    message["pairing"] = json!("unknown");
    post(app, channel, &message.to_string());
}

fn shows_all(app: &nsh_testing::Headless, texts: &[&str]) {
    for text in texts {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
}

fn shows_none(app: &nsh_testing::Headless, texts: &[&str]) {
    for text in texts {
        assert!(
            !app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
}

fn has(app: &nsh_testing::Headless, id: &str) -> bool {
    app.screen().matches(&format!("id:{id}")).unwrap()
}

fn done_is_busy(app: &nsh_testing::Headless) -> bool {
    app.screen()
        .accessibility_state("id:hostsetup-done")
        .unwrap()
        .busy
}

#[test]
fn a_fresh_host() {
    let (mut app, channel) = start(Screen::HostSetup, Params::default(), "hostsetup");
    post_setup(&mut app, &channel, fresh());
    app.wait_for("id:hostsetup-host").unwrap();
    shows_all(
        &app,
        &[
            "Set up a host",
            "Done",
            "Operating system",
            "Darwin",
            "openssl",
            "curl",
            "jq",
            "tmux",
            "Installed",
            "Status scripts",
            "~/.claude/settings.json",
            "Not installed",
            "Agent",
            "Claude Code",
            "Install",
            "Test the status script",
            "Runs the command Claude Code's config registers for its hooks, in this tmux pane, and reads the pane back. The agent itself isn't started.",
            "Pairing",
            "https://push.example.com",
            "Not paired",
            "Pair",
            "Send a test notification",
            "The host sends one for real. If it arrives here, the whole chain works.",
            "Reaches you when Moshpit isn't running. Your relay signs the push; it never sees what the agent said.",
        ],
    );
    shows_none(&app, &["Remove", "Unpair", "Remove Moshpit from this host"]);
    for id in [
        "hostsetup-error",
        "hostsetup-missing",
        "hostsetup-nothing-will-push",
        "hostsetup-user-action",
        "hostsetup-relay-error",
        "hostsetup-remove-everything",
    ] {
        assert!(!has(&app, id), "{id} on a fresh host");
    }
    assert!(!done_is_busy(&app));
    channel.close();
}

#[test]
fn installed_out_of_date_and_the_warnings() {
    let (mut app, channel) = start(Screen::HostSetup, Params::default(), "hostsetup");
    post_setup(
        &mut app,
        &channel,
        HostSetup {
            hooks: Status::Stale,
            nothing_will_push: true,
            relay_error: Some("Relay refused to mint a send token (401): unknown device".into()),
            error: Some("permission denied".into()),
            user_action: Some("Codex won't run a newly added hook until you trust it.".into()),
            hooks_proof: proof(
                ProofState::Unavailable,
                Some("Update the hooks for Claude Code first: the test runs what this version of Moshpit installs."),
            ),
            push_proof: proof(ProofState::Failed, Some("The host reported: curl: (6)")),
            ..installed()
        },
    );
    app.wait_for("id:hostsetup-error").unwrap();
    shows_all(
        &app,
        &[
            "Out of date",
            "Update",
            "Remove",
            "Paired",
            "Re-pair",
            "Unpair",
            "permission denied",
            "Paired, but nothing will push. Install the status scripts below.",
            "Relay refused to mint a send token (401): unknown device",
            "Codex won't run a newly added hook until you trust it.",
            "Update the hooks for Claude Code first: the test runs what this version of Moshpit installs.",
            "The host reported: curl: (6)",
            "Failed",
            "Remove Moshpit from this host",
        ],
    );
    for id in [
        "hostsetup-nothing-will-push",
        "hostsetup-relay-error",
        "hostsetup-user-action",
        "hostsetup-remove-everything",
    ] {
        assert!(has(&app, id), "{id}");
    }
    // A test that can't run waits; one that failed runs again.
    assert!(!app.screen().enabled("id:hostsetup-hooks-proof").unwrap());
    assert!(app.screen().enabled("id:hostsetup-push-proof").unwrap());
    channel.close();
}

#[test]
fn looking_at_the_host_then_a_host_missing_tools() {
    let (mut app, channel) = start(Screen::HostSetup, Params::default(), "hostsetup");
    post_unknown(
        &mut app,
        &channel,
        HostSetup {
            phase: Phase::Inspecting,
            phase_label: Some("Looking at the host…".into()),
            facts: None,
            ..fresh()
        },
    );
    app.wait_for("id:hostsetup-looking").unwrap();
    assert!(app.screen().shows("Looking at the host…"));
    assert!(done_is_busy(&app), "the spinner in Done's place");
    shows_none(&app, &["Install", "Pair", "Couldn't read this host"]);
    post_setup(
        &mut app,
        &channel,
        HostSetup {
            facts: Some(Facts {
                uname: "Linux".into(),
                tools: tools(&["openssl", "curl", "jq"]),
                missing_for_push: vec!["openssl".into(), "curl".into()],
            }),
            ..fresh()
        },
    );
    app.wait_for("id:hostsetup-missing").unwrap();
    shows_all(&app, &["Linux", "Pushes need openssl, curl on this host."]);
    assert!(!done_is_busy(&app));
    channel.close();
}

#[test]
fn every_row_and_the_agent_menu_reach_the_host() {
    let (mut app, channel, natives) =
        start_observing(Screen::HostSetup, Params::default(), "hostsetup");
    let told = hear(&channel);
    post_setup(&mut app, &channel, installed());
    app.wait_for("id:hostsetup-host").unwrap();
    for (id, ev) in [
        ("hostsetup-install", "install-hooks"),
        ("hostsetup-remove", "remove-hooks"),
        ("hostsetup-pair", "pair"),
        ("hostsetup-unpair", "unpair"),
        ("hostsetup-hooks-proof", "prove-hooks"),
        ("hostsetup-push-proof", "prove-push"),
        ("hostsetup-done", "done"),
    ] {
        app.press(&format!("id:{id}")).unwrap();
        assert_eq!(next(&told), json!({"ev": ev}), "{id}");
    }
    shows_all(&app, &["Reinstall", "Re-pair"]);
    let menu = natives.find("moshpit/menu", "hostsetup-agent").unwrap();
    let items = natives.prop(menu, "items").unwrap();
    assert_eq!(items[0]["checked"], json!(true), "the agent in use is checked");
    assert_eq!(items[1].get("checked"), None);
    emit(&mut app, menu, "select", json!("codex"));
    assert_eq!(next(&told), json!({"ev": "agent", "id": "codex"}));
    quiet(&told);
    channel.close();
}

/// B381: the menu says each agent's status scripts, or that the host
/// doesn't have it.
#[test]
fn the_agent_menu_says_each_agents_status_or_that_the_host_doesnt_have_it() {
    let (mut app, channel, natives) =
        start_observing(Screen::HostSetup, Params::default(), "hostsetup");
    post_setup(
        &mut app,
        &channel,
        HostSetup {
            agents: vec![
                Agent {
                    status: Status::Current,
                    ..claude()
                },
                codex(),
                Agent {
                    id: "gemini".into(),
                    name: "Gemini CLI".into(),
                    config_hint: "~/.gemini/settings.json".into(),
                    status: Status::Stale,
                    found: Some(true),
                },
            ],
            ..installed()
        },
    );
    app.wait_for("id:hostsetup-host").unwrap();
    let menu = natives.find("moshpit/menu", "hostsetup-agent").unwrap();
    let items = natives.prop(menu, "items").unwrap();
    let details: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["detail"].as_str().unwrap_or("-"))
        .collect();
    assert_eq!(details, ["Installed", "Not found on this host", "Out of date"]);
    channel.close();
}

/// B383, B384: while a step runs, its row holds the spinner, Done does too,
/// and every other row and the agent menu wait.
#[test]
fn a_running_step_holds_its_spinner_and_every_other_control_waits() {
    let (mut app, channel, natives) =
        start_observing(Screen::HostSetup, Params::default(), "hostsetup");
    let told = hear(&channel);
    post_setup(&mut app, &channel, installed());
    app.wait_for("id:hostsetup-host").unwrap();
    app.press("id:hostsetup-install").unwrap();
    assert_eq!(next(&told), json!({"ev": "install-hooks"}));
    post_setup(
        &mut app,
        &channel,
        HostSetup {
            phase: Phase::Working,
            phase_label: Some("Installing hooks for Claude Code…".into()),
            ..installed()
        },
    );
    app.wait_for("id:hostsetup-host").unwrap();
    assert!(done_is_busy(&app));
    for id in [
        "hostsetup-install",
        "hostsetup-remove",
        "hostsetup-pair",
        "hostsetup-unpair",
        "hostsetup-hooks-proof",
        "hostsetup-push-proof",
        "hostsetup-remove-everything",
        "hostsetup-done",
    ] {
        let _ = app.press(&format!("id:{id}"));
    }
    quiet(&told);
    assert!(
        natives.find("moshpit/menu", "hostsetup-agent").is_none(),
        "the agent menu opens mid-step"
    );
    // Between steps they act again.
    post_setup(&mut app, &channel, installed());
    app.wait_for("id:hostsetup-host").unwrap();
    assert!(!done_is_busy(&app));
    app.press("id:hostsetup-unpair").unwrap();
    assert_eq!(next(&told), json!({"ev": "unpair"}));
    assert!(natives.find("moshpit/menu", "hostsetup-agent").is_some());
    channel.close();
}

/// B383: a test running holds the steps; Done stays, its own row has the
/// spinner.
#[test]
fn a_running_test_holds_the_steps_and_keeps_done() {
    let (mut app, channel) = start(Screen::HostSetup, Params::default(), "hostsetup");
    let told = hear(&channel);
    post_setup(
        &mut app,
        &channel,
        HostSetup {
            phase: Phase::Proving,
            push_proof: proof(ProofState::Proving, None),
            ..installed()
        },
    );
    app.wait_for("id:hostsetup-host").unwrap();
    assert!(app.screen().shows("Waiting for it to arrive…"));
    assert!(!done_is_busy(&app), "Done stays");
    for id in ["hostsetup-install", "hostsetup-pair", "hostsetup-hooks-proof"] {
        let _ = app.press(&format!("id:{id}"));
    }
    quiet(&told);
    app.press("id:hostsetup-done").unwrap();
    assert_eq!(next(&told), json!({"ev": "done"}));
    channel.close();
}

/// B380: the hooks' test says what it does, not what the push's test
/// does: running in this pane, and the pane showing the status it set.
#[test]
fn the_hooks_test_says_what_it_did_not_what_the_push_test_does() {
    let (mut app, channel) = start(Screen::HostSetup, Params::default(), "hostsetup");
    post_setup(
        &mut app,
        &channel,
        HostSetup {
            phase: Phase::Proving,
            hooks_proof: proof(ProofState::Proving, None),
            ..installed()
        },
    );
    app.wait_for("id:hostsetup-host").unwrap();
    assert!(app.screen().shows("Running it in this pane…"));
    assert!(!app.screen().shows("Waiting for it to arrive…"));
    post_setup(
        &mut app,
        &channel,
        HostSetup {
            hooks_proof: proof(ProofState::Proven, None),
            ..installed()
        },
    );
    app.wait_for("id:hostsetup-host").unwrap();
    shows_all(&app, &["Works — the pane shows the status it set.", "Passed"]);
    assert!(!app.screen().shows("Proven — it arrived on this phone."));
    channel.close();
}

/// B382: a host that couldn't be read says so with the host's reason, and
/// offers nothing but reading it again.
#[test]
fn a_host_that_couldnt_be_read_offers_only_retry() {
    let (mut app, channel) = start(Screen::HostSetup, Params::default(), "hostsetup");
    let told = hear(&channel);
    post_unknown(
        &mut app,
        &channel,
        HostSetup {
            facts: None,
            error: Some("connection refused".into()),
            removable: true,
            ..fresh()
        },
    );
    app.wait_for("id:hostsetup-unread").unwrap();
    shows_all(&app, &["Couldn't read this host", "connection refused", "Retry"]);
    shows_none(
        &app,
        &[
            "Looking at the host…",
            "Not installed",
            "Install",
            "Pair",
            "Remove Moshpit from this host",
        ],
    );
    app.press("id:hostsetup-retry").unwrap();
    assert_eq!(next(&told), json!({"ev": "inspect"}));
    quiet(&told);
    channel.close();
}

/// B381: installing for an agent the host doesn't have asks first, in the
/// system's alert, when the core says so (`confirm`); Install Anyway and
/// Cancel answer it.
#[test]
fn installing_for_an_agent_the_host_doesnt_have_asks_first_in_the_alert() {
    let (mut app, channel) = start(Screen::HostSetup, Params::default(), "hostsetup");
    app.answer_dialogs_myself();
    let told = hear(&channel);
    let why = "Moshpit found neither ~/.gemini nor a gemini command here. Its hooks will run once it's installed.";
    let gemini = Agent {
        id: "gemini".into(),
        name: "Gemini CLI".into(),
        config_hint: "~/.gemini/settings.json".into(),
        status: Status::Absent,
        found: Some(false),
    };
    let asking = HostSetup {
        agent: gemini.clone(),
        confirm: Some(why.into()),
        ..fresh()
    };
    post_setup(&mut app, &channel, asking.clone());
    assert!(wait_dialog(&mut app), "it didn't ask");
    let dialog = app.pending_dialog().cloned().unwrap();
    assert!(!dialog.sheet);
    assert_eq!(dialog.title.as_deref(), Some("Gemini CLI isn't on this host"));
    assert_eq!(dialog.message.as_deref(), Some(why));
    app.answer_dialog(DialogAnswer::Press("Install Anyway".into()))
        .unwrap();
    assert_eq!(next(&told), json!({"ev": "install-hooks-anyway"}));
    // The same question posted again while it stands is asked once.
    post_setup(&mut app, &channel, asking.clone());
    assert!(app.pending_dialog().is_none(), "asked twice");
    // Answered, then asked again: Cancel says no.
    post_setup(
        &mut app,
        &channel,
        HostSetup {
            agent: gemini,
            ..fresh()
        },
    );
    post_setup(&mut app, &channel, asking);
    assert!(wait_dialog(&mut app), "it didn't ask again");
    app.answer_dialog(DialogAnswer::Press("Cancel".into())).unwrap();
    assert_eq!(next(&told), json!({"ev": "dismiss"}));
    quiet(&told);
    channel.close();
}

#[test]
fn removing_moshpit_from_the_host_asks_first_and_says_what_goes() {
    let (mut app, channel) = start(Screen::HostSetup, Params::default(), "hostsetup");
    app.answer_dialogs_myself();
    let told = hear(&channel);
    post_setup(&mut app, &channel, installed());
    app.wait_for("id:hostsetup-remove-everything").unwrap();
    assert!(app.screen().shows("Remove Moshpit from this host"));
    // No: nothing is told.
    app.press("id:hostsetup-remove-everything").unwrap();
    assert!(wait_dialog(&mut app), "it didn't ask");
    let dialog = app.pending_dialog().cloned().unwrap();
    assert!(!dialog.sheet, "the system's alert, not a sheet");
    assert_eq!(dialog.title.as_deref(), Some("Remove Moshpit from this host?"));
    assert_eq!(
        dialog.message.as_deref(),
        Some(
            "Moshpit's hooks come out of every agent's config, and this phone's pairing is removed. If no other phone is paired with this host, ~/.moshpit goes too. Everything else in the agents' configs stays."
        )
    );
    let mut buttons: Vec<(String, String)> = dialog
        .buttons
        .iter()
        .map(|b| (b.text.clone(), format!("{:?}", b.style)))
        .collect();
    buttons.sort();
    assert_eq!(
        buttons,
        [
            ("Cancel".to_owned(), "Cancel".to_owned()),
            ("Remove".to_owned(), "Destructive".to_owned())
        ]
    );
    app.answer_dialog(DialogAnswer::Press("Cancel".into())).unwrap();
    quiet(&told);
    // Yes: the host is told.
    app.press("id:hostsetup-remove-everything").unwrap();
    assert!(wait_dialog(&mut app), "it didn't ask again");
    app.answer_dialog(DialogAnswer::Press("Remove".into())).unwrap();
    assert_eq!(next(&told), json!({"ev": "remove-everything"}));
    // While a step runs it waits, and asks nothing.
    post_setup(
        &mut app,
        &channel,
        HostSetup {
            phase: Phase::Working,
            phase_label: Some("Removing Moshpit from this host…".into()),
            ..installed()
        },
    );
    app.wait_for("id:hostsetup-host").unwrap();
    let _ = app.press("id:hostsetup-remove-everything");
    assert!(app.pending_dialog().is_none(), "it asked mid-step");
    quiet(&told);
    channel.close();
}

#[test]
fn remove_moshpit_isnt_offered_with_nothing_on_the_host() {
    let (mut app, channel) = start(Screen::HostSetup, Params::default(), "hostsetup");
    post_setup(&mut app, &channel, fresh());
    app.wait_for("id:hostsetup-host").unwrap();
    assert!(!has(&app, "hostsetup-remove-everything"));
    channel.close();
}

#[test]
fn the_host_setup_speaks_the_apps_language() {
    let (mut app, channel) = start(Screen::HostSetup, zh_hans(), "hostsetup");
    post_setup(&mut app, &channel, fresh());
    app.wait_for("id:hostsetup-host").unwrap();
    shows_all(
        &app,
        &["配置主机", "智能体状态", "状态脚本", "安装", "未安装", "推送", "未配对"],
    );
    channel.close();
}

#[test]
fn the_host_setup_follows_dynamic_type() {
    let (mut app, channel) = start(Screen::HostSetup, Params::default(), "hostsetup");
    post_setup(&mut app, &channel, fresh());
    app.wait_for("id:hostsetup-host").unwrap();
    let size = |app: &nsh_testing::Headless, selector: &str| {
        app.screen().span_style(selector).unwrap().font_size.0
    };
    let body = size(&app, "text:Operating system");
    app.set_environment("xxxLarge", |env| env.font_scale = 23.0 / 17.0)
        .unwrap();
    assert!(
        size(&app, "text:Operating system") > body,
        "rows grow with the text size"
    );
    app.set_environment("AX5", |env| env.font_scale = 53.0 / 17.0)
        .unwrap();
    assert!(has(&app, "hostsetup-host"));
    channel.close();
}
