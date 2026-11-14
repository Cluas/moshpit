//! The core's host care, as its screens show it: a Host Setup view
//! (`moshpit_hostcare::SetupView`) turned into the page's message
//! ([`crate::host_setup::HostSetup`]), and an Install Assist plan into its
//! sheet's ([`crate::terminal::install::Install`]) — the same for every
//! platform's host. The core's texts are typed ([`Note`]); here each is the
//! app's catalog text in the screen's language (`locale`), as the iOS app
//! wrote them with `String(localized:)`. Text a host or the platform said
//! is shown as it came, and the step labels the app never localized
//! ("Installing hooks for …", "Removing hooks…", "Unpairing…") stay
//! English.

use moshpit_hostcare::note::TRUST_CODEX;
use moshpit_hostcare::{AgentView, AssistPlan, Note, Phase, Proof, RelayFailure, SetupView};
use moshpit_install::installer::{ComponentStatus, StepFailure};
use moshpit_install::hooks;
use serde_json::json;

use crate::host_setup::{self, Agent, Facts, HostSetup, ProofState, Status, Tool};
use crate::terminal::install::{Install, Recheck};
use crate::tl;

/// `note` in `locale`.
pub fn say(note: &Note, locale: &str) -> String {
    match note {
        Note::Said(text) => text.clone(),
        Note::LookingAtHost => tl!("Looking at the host…", locale),
        Note::InstallingHooks { .. }
        | Note::RemovingHooks
        | Note::Unpairing
        | Note::RemovingEverything => note.english(),
        Note::MintingSendToken => tl!("Minting a send token at the relay…", locale),
        Note::InstallingOn { host } => tl!("Installing on %@…", locale, arg1 = host),
        Note::EnterRelay => tl!("Enter the address of your push relay first.", locale),
        Note::MissingTools(tools) => tl!(
            "This host is missing %@ — install those and try again.",
            locale,
            arg1 = tools.join(", ")
        ),
        Note::Step(StepFailure::Verify { .. }) => note.english(),
        Note::Step(StepFailure::Config(hooks::Failure::UnparseableJson(detail))) => tl!(
            "That config file isn't valid JSON (%@). Fix it on the host, or move it aside and Moshpit will write a fresh one.",
            locale,
            arg1 = detail
        ),
        Note::Step(StepFailure::Config(hooks::Failure::NotAnObject)) => tl!(
            "That config file's top level isn't a JSON object, so Moshpit can't merge hooks into it.",
            locale
        ),
        Note::Step(StepFailure::UnsafePath(path)) => tl!(
            "The manifest on this host names a config path Moshpit won't run a command against: %@",
            locale,
            arg1 = path
        ),
        Note::TrustCodexHooks => trust_codex(locale),
        Note::OpenTmuxPane => tl!("Open a tmux pane on this host to test the hooks.", locale),
        Note::HooksNotInstalled { agent } => {
            tl!("Install the hooks for %@ to test them.", locale, arg1 = agent)
        }
        Note::HooksOutOfDate { agent } => tl!(
            "Update the hooks for %@ first: the test runs what this version of Moshpit installs.",
            locale,
            arg1 = agent
        ),
        Note::AgentNotFound { dir, command, .. } => tl!(
            "Moshpit found neither %@ nor a %@ command here. Its hooks will run once it's installed.",
            locale,
            arg1 = dir,
            arg2 = command
        ),
        Note::NotRegistered { config, event } => tl!(
            "%@ doesn't register Moshpit for %@. Reinstall to put it back.",
            locale,
            arg1 = config,
            arg2 = event
        ),
        Note::TmuxElsewhere => tl!(
            "This host's tmux isn't on the default socket, so Moshpit can't run the test. The hooks themselves are unaffected.",
            locale
        ),
        Note::StampEmpty { trust_hint } => {
            let hint = if *trust_hint {
                format!(" {}", trust_codex(locale))
            } else {
                String::new()
            };
            tl!(
                "The stamp script ran but the pane came back empty.%@",
                locale,
                arg1 = hint
            )
        }
        Note::StampOther(got) => tl!(
            "The pane came back as \"%@\" instead of the test stamp.",
            locale,
            arg1 = got
        ),
        Note::HostReported(output) => tl!("The host reported: %@", locale, arg1 = output),
        Note::PushNeverSeen => tl!(
            "The host sent one, but this screen never saw it arrive. Most likely Moshpit was not in the foreground — the test can only be confirmed while you are looking at this screen, so try again without leaving it. If it still fails, check that notifications are allowed and that the relay can reach Apple.",
            locale
        ),
        Note::HostNotAnswering => tl!(
            "The host isn't answering on this session's connection. Try again in a moment.",
            locale
        ),
        Note::Relay(failure) => match failure {
            RelayFailure::BadUrl => tl!("That relay address isn't a valid URL.", locale),
            RelayFailure::CouldNotSave => tl!(
                "Couldn't save the pairing on this phone, so nothing was installed on the host. The diagnostics log has the reason.",
                locale
            ),
            RelayFailure::CouldNotForget => tl!(
                "Couldn't remove the pairing from this phone, so the host was left paired. The diagnostics log has the reason.",
                locale
            ),
            RelayFailure::NoDeviceToken => tl!(
                "iOS hasn't issued a push token to this phone yet — check that notifications are allowed, then try again.",
                locale
            ),
            RelayFailure::Status { code, body } => tl!(
                "Relay refused to mint a send token (%lld): %@",
                locale,
                arg1 = code,
                arg2 = body
            ),
            RelayFailure::Network(_) | RelayFailure::Malformed | RelayFailure::Unavailable => {
                note.english()
            }
        },
    }
}

fn trust_codex(locale: &str) -> String {
    let text = tl!(
        "Codex won't run a newly added hook until you trust it. Run /hooks inside Codex on this host and trust the four Moshpit entries — until you do, nothing will be sent.",
        locale
    );
    // A catalog without it says the core's English.
    if text.starts_with(&format!("{locale}.")) {
        TRUST_CODEX.to_owned()
    } else {
        text
    }
}

fn status(status: &ComponentStatus) -> Status {
    match status {
        ComponentStatus::Absent => Status::Absent,
        ComponentStatus::Current => Status::Current,
        ComponentStatus::Stale { .. } => Status::Stale,
    }
}

fn proof(proof: &Proof, locale: &str) -> host_setup::Proof {
    let (state, message) = match proof {
        Proof::Untested => (ProofState::Untested, None),
        Proof::Proving => (ProofState::Proving, None),
        Proof::Proven(_) => (ProofState::Proven, None),
        Proof::Failed(why) => (ProofState::Failed, Some(say(why, locale))),
        Proof::Unavailable(why) => (ProofState::Unavailable, Some(say(why, locale))),
    };
    host_setup::Proof { state, message }
}

fn agent(view: &SetupView, listed: &AgentView) -> Agent {
    let agent = listed.agent;
    Agent {
        id: agent.id.to_owned(),
        name: agent.display_name.to_owned(),
        config_hint: agent.config_hint(),
        status: if view.known {
            status(&listed.hooks)
        } else {
            Status::Unknown
        },
        found: listed.found,
    }
}

/// What a Host Setup page shows for `view`, in `locale`.
pub fn host_setup(view: &SetupView, locale: &str) -> HostSetup {
    let (phase, phase_label) = match &view.phase {
        Phase::Idle => (host_setup::Phase::Idle, None),
        Phase::Inspecting => (
            host_setup::Phase::Inspecting,
            Some(say(&Note::LookingAtHost, locale)),
        ),
        Phase::Working(label) => (host_setup::Phase::Working, Some(say(label, locale))),
        Phase::Proving => (host_setup::Phase::Proving, None),
    };
    HostSetup {
        phase,
        phase_label,
        facts: view.facts.as_ref().map(|facts| Facts {
            uname: facts.uname.clone(),
            // The four the page shows, in its order.
            tools: ["openssl", "curl", "jq", "tmux"]
                .iter()
                .map(|name| Tool {
                    name: (*name).to_owned(),
                    present: facts.tools.get(*name) == Some(&true),
                })
                .collect(),
            missing_for_push: if facts.can_push() {
                Vec::new()
            } else {
                facts.missing_for_push()
            },
        }),
        error: view.error.as_ref().map(|n| say(n, locale)),
        nothing_will_push: view.nothing_will_push,
        agent: agent(
            view,
            &view
                .agents
                .iter()
                .find(|listed| listed.agent.id == view.agent.id)
                .cloned()
                .unwrap_or(AgentView {
                    agent: view.agent,
                    hooks: view.hooks.clone(),
                    found: None,
                }),
        ),
        agents: view.agents.iter().map(|listed| agent(view, listed)).collect(),
        confirm: view.confirm.as_ref().map(|note| say(note, locale)),
        hooks: if view.known {
            status(&view.hooks)
        } else {
            Status::Unknown
        },
        user_action: view.user_action.as_ref().map(|n| say(n, locale)),
        hooks_proof: proof(&view.hooks_proof, locale),
        relay_error: view.relay_error.as_ref().map(|n| say(n, locale)),
        paired_relay: view.paired_relay.clone(),
        relay_url: view.relay_url.clone(),
        pairing: if view.known {
            status(&view.pairing)
        } else {
            Status::Unknown
        },
        push_proof: proof(&view.push_proof, locale),
        removable: view
            .state
            .as_ref()
            .is_some_and(|s| !s.manifest.components.is_empty()),
    }
}

/// `{"op": "host-setup", …}` for `view`.
pub fn host_setup_message(view: &SetupView, locale: &str) -> String {
    host_setup::Message::HostSetup(Box::new(host_setup(view, locale))).to_json()
}

/// What Install Assist shows for `plan`.
pub fn install(
    plan: &AssistPlan,
    rechecking: bool,
    result: Option<moshpit_hostcare::Recheck>,
    copied: bool,
) -> Install {
    Install {
        herdr: plan.herdr,
        mosh_only: plan.mosh_only(),
        manager: plan.manager.is_some(),
        command: plan.command(),
        note: plan.shows_note(),
        rechecking,
        result: result.map(|r| match r {
            moshpit_hostcare::Recheck::Resolved => Recheck::Resolved,
            moshpit_hostcare::Recheck::StillMissing => Recheck::StillMissing,
            moshpit_hostcare::Recheck::NoChannel => Recheck::NoChannel,
        }),
        copied,
    }
}

/// `{"op": "install", …}` for `plan`.
pub fn install_message(
    plan: &AssistPlan,
    rechecking: bool,
    result: Option<moshpit_hostcare::Recheck>,
    copied: bool,
) -> String {
    let mut value = serde_json::to_value(install(plan, rechecking, result, copied))
        .expect("install serializes");
    value["op"] = json!("install");
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshpit_install::{HookAgent, PackageManager};

    /// A view of a host the page hasn't read: what it says before any
    /// read, or after one failed.
    fn unread_view() -> SetupView {
        SetupView {
            phase: Phase::Idle,
            known: false,
            facts: None,
            error: Some(Note::Said("connection refused".into())),
            nothing_will_push: false,
            agent: &HookAgent::ALL[0],
            agents: Vec::new(),
            confirm: None,
            hooks: ComponentStatus::Absent,
            user_action: None,
            hooks_proof: Proof::Untested,
            relay_error: None,
            paired_relay: None,
            relay_url: String::new(),
            pairing: ComponentStatus::Absent,
            push_proof: Proof::Untested,
            last_report: None,
            state: None,
        }
    }

    #[test]
    fn a_host_not_read_is_unknown_not_absent() {
        let unread = host_setup(&unread_view(), "en");
        assert_eq!(unread.hooks, Status::Unknown);
        assert_eq!(unread.pairing, Status::Unknown);
        let read = host_setup(
            &SetupView {
                known: true,
                ..unread_view()
            },
            "en",
        );
        assert_eq!(read.hooks, Status::Absent);
        assert_eq!(read.pairing, Status::Absent);
    }

    #[test]
    fn every_note_reads_in_the_catalogs_english_and_another_language() {
        let notes = [
            Note::LookingAtHost,
            Note::MintingSendToken,
            Note::InstallingOn { host: "m1".into() },
            Note::EnterRelay,
            Note::MissingTools(vec!["openssl".into(), "curl".into()]),
            Note::Step(StepFailure::Config(hooks::Failure::UnparseableJson(
                "x".into(),
            ))),
            Note::Step(StepFailure::Config(hooks::Failure::NotAnObject)),
            Note::Step(StepFailure::UnsafePath("$(x)".into())),
            Note::TrustCodexHooks,
            Note::OpenTmuxPane,
            Note::TmuxElsewhere,
            Note::StampEmpty { trust_hint: true },
            Note::StampEmpty { trust_hint: false },
            Note::StampOther("idle||".into()),
            Note::HostReported("401".into()),
            Note::PushNeverSeen,
            Note::HostNotAnswering,
            Note::Relay(RelayFailure::BadUrl),
            Note::Relay(RelayFailure::CouldNotSave),
            Note::Relay(RelayFailure::CouldNotForget),
            Note::Relay(RelayFailure::NoDeviceToken),
            Note::Relay(RelayFailure::Status {
                code: 401,
                body: "no".into(),
            }),
        ];
        for note in &notes {
            assert_eq!(say(note, "en"), note.english(), "{note:?}");
            let zh = say(note, "zh-Hans");
            assert!(!zh.starts_with("zh-Hans."), "{note:?} has no catalog entry");
            assert_ne!(zh, note.english(), "{note:?} isn't translated");
        }
    }

    #[test]
    fn what_a_host_said_is_shown_as_it_came() {
        assert_eq!(say(&Note::Said("boom".into()), "de"), "boom");
        assert_eq!(
            say(
                &Note::InstallingHooks {
                    agent: "Codex".into()
                },
                "zh-Hans"
            ),
            "Installing hooks for Codex\u{2026}",
            "a label the app never localized"
        );
    }

    #[test]
    fn the_install_message_parses_as_the_sheet_reads_it() {
        let plan = AssistPlan::new(&["mosh".to_owned()], false, Some(PackageManager::Brew));
        let json = install_message(
            &plan,
            false,
            Some(moshpit_hostcare::Recheck::Resolved),
            true,
        );
        let crate::terminal::install::Message::Install(install) =
            crate::terminal::install::Message::parse(&json).unwrap();
        assert!(install.mosh_only);
        assert_eq!(install.command.as_deref(), Some("brew install mosh"));
        assert_eq!(install.result, Some(Recheck::Resolved));
        let none = install_message(&AssistPlan::new(&["mosh".to_owned()], false, None), false, None, false);
        assert!(
            !none.contains("command") && !none.contains("result"),
            "absent fields are left out, as the app's message left them: {none}"
        );
    }
}
