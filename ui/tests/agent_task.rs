//! New Agent Task on a headless engine: the host's repositories and agents
//! in the menus, Other…'s typed path, a branch git would refuse said under
//! its field with Start waiting, what Start, Cancel and Done tell the host,
//! a failure and a task left behind, and Chinese.

mod common;

use common::{emit, hear, next, post, start_observing, steady, zh_hans};
use moshpit_ui::screen::{Params, Screen};
use serde_json::{Value, json};

fn facts(changes: Value) -> String {
    let mut base = json!({
        "op": "agentTask",
        "repos": ["/Users/dev/code/moshpit", "/home/dev/api"],
        "agents": ["aider", "codex", "claude"],
        "scanning": false, "starting": false, "taskExists": false
    });
    for (key, value) in changes.as_object().unwrap() {
        base[key] = value.clone();
    }
    base.to_string()
}

#[test]
fn a_task_starts_with_what_the_form_holds() {
    let (mut app, channel, natives) =
        start_observing(Screen::AgentTask, Params::default(), "agent-task");
    let heard = hear(&channel);
    post(&mut app, &channel, &facts(json!({})));
    app.wait_for("id:agent-task-repo-row").unwrap();
    let screen = app.screen();
    for text in [
        "New Agent Task",
        "Repo",
        "moshpit",
        "Branch",
        "Agent",
        "claude",
        "Prompt",
        "Cancel",
        "Start",
    ] {
        assert!(screen.shows(text), "{text}: {:?}", screen.texts());
    }
    // Nothing typed yet: Start waits, and no complaint shows.
    assert_eq!(screen.enabled("id:start-agent-task"), Ok(false));
    assert!(!screen.matches("id:agent-task-branch-error").unwrap());

    // The repositories, each named by its last part over its path.
    let repo = natives.find("moshpit/menu", "agent-task-repo").unwrap();
    let items = natives.prop(repo, "items").unwrap();
    assert_eq!(items[0]["title"], json!("moshpit"));
    assert_eq!(items[0]["detail"], json!("~/code/moshpit"));
    assert_eq!(items[1]["detail"], json!("~/api"));
    assert_eq!(
        items.as_array().unwrap().last().unwrap()["title"],
        json!("Other…")
    );
    emit(&mut app, repo, "select", json!("/home/dev/api"));
    assert!(app.screen().shows("api"));

    let agent = natives.find("moshpit/menu", "agent-task-agent").unwrap();
    emit(&mut app, agent, "select", json!("codex"));
    assert!(app.screen().shows("codex"));

    app.type_text("id:agent-task-branch-input", "fix-scroll")
        .unwrap();
    app.type_text("id:agent-task-prompt-input", "Fix the jump")
        .unwrap();
    steady(&mut app);
    assert_eq!(app.screen().enabled("id:start-agent-task"), Ok(true));
    app.press("id:start-agent-task").unwrap();
    assert_eq!(
        next(&heard),
        json!({
            "ev": "start", "repo": "/home/dev/api", "branch": "fix-scroll",
            "agent": "codex", "prompt": "Fix the jump"
        })
    );

    // While it starts, Start waits; Cancel stays.
    post(&mut app, &channel, &facts(json!({"starting": true})));
    assert_eq!(app.screen().enabled("id:start-agent-task"), Ok(false));
    app.press("id:agent-task-cancel").unwrap();
    assert_eq!(next(&heard), json!({"ev": "cancel"}));
}

#[test]
fn a_branch_git_would_refuse_says_why_and_start_waits() {
    let (mut app, channel, _natives) =
        start_observing(Screen::AgentTask, Params::default(), "agent-task");
    post(&mut app, &channel, &facts(json!({})));
    app.wait_for("id:agent-task-branch").unwrap();
    for (typed, why) in [
        ("two words", "No spaces in a branch name"),
        ("ends.lock", "Can't end with “.lock”"),
        ("a..b", "No “..” in a branch name"),
        ("-x", "Can't start with “-” or “/”, or end with “/”"),
        ("what?", "No ~ ^ : ? * [ \\ in a branch name"),
    ] {
        app.clear("id:agent-task-branch-input").unwrap();
        app.type_text("id:agent-task-branch-input", typed).unwrap();
        steady(&mut app);
        assert!(
            app.screen().shows(why),
            "{typed}: {:?}",
            app.screen().texts()
        );
        assert_eq!(
            app.screen().enabled("id:start-agent-task"),
            Ok(false),
            "{typed}"
        );
    }
    app.clear("id:agent-task-branch-input").unwrap();
    app.type_text("id:agent-task-branch-input", "feature/thing")
        .unwrap();
    steady(&mut app);
    assert_eq!(app.screen().enabled("id:start-agent-task"), Ok(true));
}

#[test]
fn other_takes_a_typed_path_over_the_pick() {
    let (mut app, channel, natives) =
        start_observing(Screen::AgentTask, Params::default(), "agent-task");
    let heard = hear(&channel);
    post(&mut app, &channel, &facts(json!({})));
    app.wait_for("id:agent-task-repo-row").unwrap();
    let repo = natives.find("moshpit/menu", "agent-task-repo").unwrap();
    emit(&mut app, repo, "select", json!("\u{0}other"));
    app.wait_for("id:agent-task-custom").unwrap();
    assert!(app.screen().shows("Custom"));
    app.type_text("id:agent-task-custom-input", "/srv/repos/web")
        .unwrap();
    app.type_text("id:agent-task-branch-input", "try").unwrap();
    steady(&mut app);
    app.press("id:start-agent-task").unwrap();
    let told = next(&heard);
    assert_eq!(told["repo"], json!("/srv/repos/web"));
    assert_eq!(told["agent"], json!("claude"));
}

#[test]
fn the_scan_fills_in_and_says_when_it_found_nothing() {
    let (mut app, channel, natives) =
        start_observing(Screen::AgentTask, Params::default(), "agent-task");
    // The panes' guesses first, the scan running: no agent picked yet.
    post(
        &mut app,
        &channel,
        &facts(json!({"repos": [], "agents": [], "scanning": true})),
    );
    app.wait_for("id:agent-task-repo-row").unwrap();
    let repo = natives.find("moshpit/menu", "agent-task-repo").unwrap();
    assert_eq!(
        natives.prop(repo, "items").unwrap()[0]["title"],
        json!("Looking for repositories…")
    );
    assert!(app.screen().shows("Choose"));
    // Back with nothing: it says why, and the agent falls back to claude.
    post(
        &mut app,
        &channel,
        &facts(json!({"repos": [], "agents": []})),
    );
    let items = natives.prop(repo, "items").unwrap();
    assert_eq!(
        items[0]["title"],
        json!("None found — no panes in repos, and nothing under ~")
    );
    assert_eq!(items[0]["disabled"], json!(true));
    assert!(app.screen().shows("claude"));
}

#[test]
fn a_failure_shows_and_a_task_left_behind_offers_done() {
    let (mut app, channel, _natives) =
        start_observing(Screen::AgentTask, Params::default(), "agent-task");
    let heard = hear(&channel);
    post(
        &mut app,
        &channel,
        &facts(json!({"failure": "Creating the worktree failed"})),
    );
    app.wait_for("id:agent-task-failure").unwrap();
    assert!(app.screen().shows("Creating the worktree failed"));
    post(
        &mut app,
        &channel,
        &facts(json!({
            "failure": "The worktree is ready, but claude didn't start: no such command",
            "taskExists": true
        })),
    );
    app.wait_for("id:agent-task-done").unwrap();
    assert!(!app.screen().matches("id:agent-task-cancel").unwrap());
    assert!(!app.screen().matches("id:start-agent-task").unwrap());
    app.press("id:agent-task-done").unwrap();
    assert_eq!(next(&heard), json!({"ev": "done"}));
}

#[test]
fn in_chinese() {
    let (mut app, channel, _natives) = start_observing(Screen::AgentTask, zh_hans(), "agent-task");
    post(&mut app, &channel, &facts(json!({})));
    app.wait_for("id:agent-task-repo-row").unwrap();
    for text in ["新建智能体任务", "仓库", "分支", "智能体", "取消", "开始"] {
        assert!(
            app.screen().shows(text),
            "{text}: {:?}",
            app.screen().texts()
        );
    }
    app.type_text("id:agent-task-branch-input", "two words")
        .unwrap();
    steady(&mut app);
    assert!(
        app.screen()
            .texts()
            .iter()
            .all(|t| *t != "No spaces in a branch name"),
        "{:?}",
        app.screen().texts()
    );
}

#[test]
fn it_follows_the_system() {
    assert!(Screen::AgentTask.follows_system());
    assert!(Screen::AgentTask.draws_light());
}
