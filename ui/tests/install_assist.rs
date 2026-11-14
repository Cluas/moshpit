//! Install Assist on a headless engine: what the host's facts say, the
//! buttons tell the host, Re-check's spinner and its results, Chinese.

mod common;

use common::{hear, next, post, quiet, start, zh_hans};
use moshpit_ui::screen::{Params, Screen};
use nsh_testing::Headless;
use serde_json::{Value, json};

/// Whether `text` shows, read without the word joiners that keep a
/// hyphenated word whole.
fn shows(app: &Headless, text: &str) -> bool {
    app.screen()
        .texts()
        .iter()
        .any(|t| t.replace('\u{2060}', "") == text)
}

fn facts(changes: Value) -> String {
    let mut base = json!({
        "op": "install", "herdr": false, "moshOnly": false, "manager": true,
        "command": "brew install tmux mosh", "note": false, "rechecking": false, "copied": false
    });
    for (key, value) in changes.as_object().unwrap() {
        base[key] = value.clone();
    }
    base.to_string()
}

#[test]
fn the_command_runs_copies_and_rechecks_on_the_persons_word() {
    let (mut app, channel) = start(Screen::InstallAssist, Params::default(), "install-assist");
    let heard = hear(&channel);
    post(&mut app, &channel, &facts(json!({})));
    app.wait_for("id:install-command").unwrap();
    for text in [
        "Install on host",
        "tmux + mosh aren't installed on this host.",
        "brew install tmux mosh",
        "Run in terminal",
        "Copy command",
        "Re-check",
    ] {
        assert!(shows(&app, text), "{text}: {:?}", app.screen().texts());
    }
    assert!(!shows(
        &app,
        "Installs to ~/.local/bin. Moshpit looks there when probing and launching, so you don't need to change PATH."
    ));
    for (id, ev) in [
        ("install-run", "run"),
        ("install-copy", "copy"),
        ("install-recheck", "recheck"),
        ("install-done", "done"),
    ] {
        app.press(&format!("id:{id}")).unwrap();
        assert_eq!(next(&heard), json!({"ev": ev}));
    }
    // Copied; a re-check running can't be pressed again.
    post(
        &mut app,
        &channel,
        &facts(json!({"copied": true, "rechecking": true})),
    );
    assert!(shows(&app, "Copied"));
    assert!(shows(&app, "Re-checking…"));
    let _ = app.press("id:install-recheck");
    quiet(&heard);
    // Found: Reconnect.
    post(&mut app, &channel, &facts(json!({"result": "resolved"})));
    assert!(shows(&app, "Installed — reconnect to enable it."));
    app.press("id:install-reconnect").unwrap();
    assert_eq!(next(&heard), json!({"ev": "reconnect"}));
    post(&mut app, &channel, &facts(json!({"result": "noChannel"})));
    app.wait_for("id:install-reconnect-nochannel").unwrap();
    post(
        &mut app,
        &channel,
        &facts(json!({"result": "stillMissing"})),
    );
    assert!(shows(
        &app,
        "Still not detected. Run the command, wait for it to finish, then re-check."
    ));
}

#[test]
fn herdr_mosh_and_no_package_manager_say_what_is_missing() {
    let (mut app, channel) = start(Screen::InstallAssist, Params::default(), "install-assist");
    post(
        &mut app,
        &channel,
        &facts(
            json!({"herdr": true, "note": true, "command": "curl -fsSL https://example.com/install | sh"}),
        ),
    );
    app.wait_for("id:install-note").unwrap();
    assert!(shows(&app, "herdr isn't installed on this host."));
    post(
        &mut app,
        &channel,
        &facts(json!({"moshOnly": true, "command": "brew install mosh"})),
    );
    assert!(shows(&app, "mosh isn't installed on this host."));
    post(
        &mut app,
        &channel,
        &facts(json!({"manager": false, "command": null})),
    );
    assert!(shows(&app, "Couldn't detect a package manager."));
    assert!(shows(
        &app,
        "Install tmux and mosh with your platform's package manager (or build from source), then re-check."
    ));
    app.wait_for("id:install-recheck").unwrap();
    // Only mosh missing (a plain mosh or a herdr + mosh connection): the
    // guidance names mosh alone; that connection never uses tmux.
    post(
        &mut app,
        &channel,
        &facts(json!({"manager": false, "moshOnly": true, "command": null})),
    );
    assert!(shows(&app, "Couldn't detect a package manager."));
    assert!(
        shows(
            &app,
            "Install mosh with your platform's package manager (or build from source), then re-check."
        ),
        "{:?}",
        app.screen().texts()
    );
    assert!(!shows(
        &app,
        "Install tmux and mosh with your platform's package manager (or build from source), then re-check."
    ));
}

#[test]
fn in_chinese() {
    let (mut app, channel) = start(Screen::InstallAssist, zh_hans(), "install-assist");
    post(&mut app, &channel, &facts(json!({})));
    app.wait_for("id:install-command").unwrap();
    for text in ["在终端运行", "复制命令", "在主机上安装"] {
        let shown = app.screen().texts().iter().any(|t| t.contains(text));
        assert!(shown, "{text}: {:?}", app.screen().texts());
    }
    assert!(!shows(&app, "Run in terminal"));
}

/// Install Assist follows the system's appearance now that it draws only
/// with the tokens (R18): its title is `foreground` and its Run button's
/// words `primary-foreground`, in whichever appearance it is drawn.
#[test]
fn install_assist_draws_with_the_tokens_light_and_dark() {
    let token = |name: &str, light: bool| {
        moshpit_ui::ui::tokens::colors(&moshpit_ui::theme::AppTheme::default())
            .into_iter()
            .find(|(n, ..)| *n == name)
            .map(|(_, l, d)| if light { l } else { d })
            .unwrap()
    };
    let close = |a: nsh_core::color::Rgba, b: nsh_core::color::Rgba| {
        (a.r - b.r).abs() < 0.01 && (a.g - b.g).abs() < 0.01 && (a.b - b.b).abs() < 0.01
    };
    assert!(Screen::InstallAssist.follows_system());
    for light in [false, true] {
        let params = Params {
            light: Some(light),
            ..Params::default()
        };
        let (mut app, channel) = start(Screen::InstallAssist, params, "install-assist");
        post(&mut app, &channel, &facts(json!({})));
        app.wait_for("text:Install on host").unwrap();
        let title: nsh_core::color::Rgba = app
            .screen()
            .span_style("text:Install on host")
            .unwrap()
            .color
            .into();
        assert!(
            close(title, token("foreground", light)),
            "light {light}: the title"
        );
        let run: nsh_core::color::Rgba = app
            .screen()
            .span_style("text:Run in terminal")
            .unwrap()
            .color
            .into();
        assert!(
            close(run, token("primary-foreground", light)),
            "light {light}: Run's words"
        );
        channel.close();
    }
}
