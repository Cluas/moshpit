//! The Host Setup page's agents (B381): what the menu says about each, and
//! asking before installing hooks for an agent the host doesn't have.

mod support;

use moshpit_hostcare::Note;
use moshpit_install::installer::ComponentStatus;
use support::*;

/// A host with Claude Code's hooks, whose preflight finds Claude Code and
/// Codex but no other agent (B374's `agent.<id>` lines).
fn host() -> Host {
    let mut host = Host::hooked();
    host.rules[0].1 = format!(
        "{ALL_TOOLS}\nagent.claude=yes\nagent.codex=yes\nagent.gemini=no\nagent.qwen=no\n\
         agent.qoder=no\nagent.factory=no\nagent.codebuddy=no"
    );
    host
}

#[test]
fn the_menu_says_each_agents_hooks_and_whether_the_host_has_it() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, host());
    let page = rig.open(&c, None);
    let view = rig.view(page);
    assert_eq!(view.agents.len(), 7);
    let listed = |id: &str| {
        view.agents
            .iter()
            .find(|a| a.agent.id == id)
            .map(|a| (a.hooks.clone(), a.found))
            .unwrap()
    };
    assert_eq!(listed("claude"), (ComponentStatus::Current, Some(true)));
    assert_eq!(listed("codex"), (ComponentStatus::Absent, Some(true)));
    assert_eq!(listed("gemini"), (ComponentStatus::Absent, Some(false)));
}

#[test]
fn installing_for_an_agent_the_host_doesnt_have_asks_first() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, host());
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_select(page, "gemini", k));
    let written = rig.host(c.id).written_paths().len();
    rig.act(|m, k| m.setup_install_hooks(page, k));
    assert_eq!(
        rig.host(c.id).written_paths().len(),
        written,
        "installed without asking"
    );
    let Some(Note::AgentNotFound {
        agent,
        dir,
        command,
    }) = rig.view(page).confirm
    else {
        panic!("no question: {:?}", rig.view(page).confirm);
    };
    assert_eq!(
        (agent.as_str(), dir.as_str(), command.as_str()),
        ("Gemini CLI", "~/.gemini", "gemini")
    );
    // No: nothing written, the question goes.
    rig.act(|m, k| m.setup_dismiss(page, k));
    assert_eq!(rig.view(page).confirm, None);
    assert_eq!(rig.host(c.id).written_paths().len(), written);
    // Asked again, and yes this time.
    rig.act(|m, k| m.setup_install_hooks(page, k));
    assert!(rig.view(page).confirm.is_some());
    rig.act(|m, k| m.setup_install_hooks_anyway(page, k));
    let view = rig.view(page);
    assert_eq!(view.confirm, None);
    assert_eq!(view.hooks, ComponentStatus::Current);
    assert!(rig.host(c.id).file("$HOME/.gemini/settings.json").is_some());
}

#[test]
fn installing_for_an_agent_the_host_has_doesnt_ask() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, host());
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_select(page, "codex", k));
    rig.act(|m, k| m.setup_install_hooks(page, k));
    let view = rig.view(page);
    assert_eq!(view.confirm, None);
    assert_eq!(view.hooks, ComponentStatus::Current);
}

#[test]
fn the_question_goes_with_the_agent() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, host());
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_select(page, "gemini", k));
    rig.act(|m, k| m.setup_install_hooks(page, k));
    assert!(rig.view(page).confirm.is_some());
    rig.act(|m, k| m.setup_select(page, "codex", k));
    assert_eq!(rig.view(page).confirm, None);
    // Yes to a question no longer asked does nothing.
    let written = rig.host(c.id).written_paths().len();
    rig.act(|m, k| m.setup_dismiss(page, k));
    assert_eq!(rig.host(c.id).written_paths().len(), written);
}
