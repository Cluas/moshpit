//! `HostSetupModelTests`: the Host Setup screen's model, against simulated
//! hosts. Its predecessor had no name for "installed but out of date",
//! resolved its verification on evidence it hadn't asked for, and showed a
//! failure like a success; each of those is a test here.

mod support;

use moshpit_hostcare::{Note, Phase, Proof, RelayFailure};
use moshpit_install::commands;
use moshpit_install::installer::ComponentStatus;
use moshpit_install::manifest::{Component, InstallComponent, InstallManifest};
use moshpit_install::scripts;
use moshpit_seal::Date;
use support::*;

fn paired_manifest(connection: &str) -> InstallManifest {
    let mut manifest = InstallManifest::empty();
    manifest.set(
        &InstallComponent::Pairing {
            conn: connection.into(),
            install: None,
        },
        Some(Component {
            connection_id: Some(connection.into()),
            relay_url: Some("r".into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    manifest
}

/// A host paired with `connection`: the manifest records it, and the conf
/// is there.
fn paired_host(connection: &str) -> Host {
    Host::healthy()
        .with_manifest(&paired_manifest(connection))
        .with_file(
            &format!("$HOME/.moshpit/push.d/{connection}.conf"),
            "RELAY_URL=r\n",
        )
}

// MARK: - Reading

#[test]
fn inspect_reports_what_the_host_can_do() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.save(&[c.id]);
    let page = rig.open(&c, None);
    let view = rig.view(page);
    assert_eq!(view.phase, Phase::Idle);
    assert!(view.facts.as_ref().unwrap().can_push());
    assert_eq!(view.error, None);
    assert_eq!(view.pairing, ComponentStatus::Absent);
}

#[test]
fn a_channel_that_cannot_be_opened_is_an_error_on_screen_not_a_spinner() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.host(c.id).fail_on = Some("printf".into());
    let page = rig.open(&c, None);
    let view = rig.view(page);
    assert_eq!(view.phase, Phase::Idle);
    assert!(view.error.is_some());
}

#[test]
fn opening_the_screen_reads_the_host_and_says_so_meanwhile() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.host(c.id).holding = true;
    let page = rig.open(&c, None);
    assert_eq!(rig.view(page).phase, Phase::Inspecting);
    rig.let_go(c.id);
    assert_eq!(rig.view(page).phase, Phase::Idle);
}

// MARK: - Installing

#[test]
fn a_failed_install_says_why_and_does_not_read_as_success() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(
        c.id,
        Host::healthy().rule("cat \"$HOME/.claude/settings.json\"", "{broken"),
    );
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_install_hooks(page, k));
    let view = rig.view(page);
    let error = view.error.expect("the failure is on screen");
    assert!(error.english().contains("valid JSON"), "{error:?}");
    assert!(
        matches!(error, Note::Step(_)),
        "typed, so the screen can translate it"
    );
    assert_eq!(
        view.state.unwrap().status(&InstallComponent::Hooks {
            agent: "claude".into()
        }),
        ComponentStatus::Absent
    );
}

#[test]
fn installing_hooks_lands_them_and_reads_the_host_again() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_install_hooks(page, k));
    let view = rig.view(page);
    assert_eq!(view.error, None);
    assert_eq!(view.hooks, ComponentStatus::Current);
    assert!(view.last_report.unwrap().succeeded());
    assert!(
        rig.host(c.id)
            .file("$HOME/.claude/settings.json")
            .unwrap()
            .contains("moshpit-stamp.sh")
    );
}

#[test]
fn removing_hooks_takes_them_back_out() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_install_hooks(page, k));
    rig.act(|m, k| m.setup_remove_hooks(page, k));
    let view = rig.view(page);
    assert_eq!(view.hooks, ComponentStatus::Absent);
    assert!(
        !rig.host(c.id)
            .file("$HOME/.claude/settings.json")
            .unwrap()
            .contains("moshpit-stamp.sh")
    );
}

#[test]
fn a_host_missing_tools_is_refused_with_the_tools_named() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.save(&[c.id]);
    rig.set_host(
        c.id,
        Host::default().rule("printf 'home=", "home=/h\ncurl=no\nopenssl=no\nbase64=yes"),
    );
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_pair(page, k));
    let error = rig.view(page).error.unwrap();
    let text = error.english();
    assert!(text.contains("openssl") && text.contains("curl"), "{text}");
    assert!(matches!(error, Note::MissingTools(_)));
    // Minted at the relay first, so nothing was left half-done on a host
    // that could never have used it.
    assert_eq!(rig.mints.len(), 1);
    assert!(rig.host(c.id).pairing_conf(c.id).is_none());
    assert!(rig.host(c.id).written_paths().is_empty());
}

#[test]
fn an_empty_relay_address_is_caught_before_anything_is_attempted() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.act(|m, k| {
        m.set_settings(
            moshpit_hostcare::Settings {
                relay_url: "   ".into(),
                ..Default::default()
            },
            k,
        )
    });
    let page = rig.open(&c, None);
    let commands = rig.host(c.id).commands.len();
    rig.act(|m, k| m.setup_pair(page, k));
    assert_eq!(rig.view(page).error, Some(Note::EnterRelay));
    assert!(rig.mints.is_empty());
    assert!(rig.store.read().is_empty(), "nothing minted on the phone");
    assert_eq!(
        rig.host(c.id).commands.len(),
        commands,
        "nothing sent to the host"
    );
}

// MARK: - Proving

#[test]
fn pairing_proves_itself_with_a_real_push_and_says_so() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.save(&[c.id]);
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_pair(page, k));
    assert_eq!(rig.view(page).push_proof, Proof::Proving);
    // The host's sender fired the test; its push arrives.
    let nonce = rig
        .host(c.id)
        .sent("--test")
        .and_then(|c| between(c, "--test '", "'"))
        .unwrap()
        .to_owned();
    rig.act(|m, k| m.self_test_arrived(nonce, k));
    let view = rig.view(page);
    assert_eq!(view.error, None);
    assert_eq!(rig.mints.len(), 1);
    let phone = rig.pairing(c.id).unwrap();
    let conf = rig.host(c.id).pairing_conf(c.id).unwrap().clone();
    assert!(conf.contains(&format!("SEND_TOKEN={}", phone.send_token)));
    assert_eq!(conf, moshpit_install::push_conf(&phone));
    assert!(
        matches!(view.push_proof, Proof::Proven(_)),
        "{:?}",
        view.push_proof
    );
    assert_eq!(view.pairing, ComponentStatus::Current);
}

#[test]
fn a_push_that_never_arrives_is_a_failure_with_an_actionable_reason() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_prove_push(page, k));
    assert_eq!(rig.view(page).push_proof, Proof::Proving);
    rig.advance(19_999);
    assert_eq!(rig.view(page).push_proof, Proof::Proving);
    rig.advance(1);
    let Proof::Failed(why) = rig.view(page).push_proof else {
        panic!("expected a failure");
    };
    // It leads with the likeliest cause: the proof comes back only while
    // the app is in the foreground.
    let text = why.english().to_lowercase();
    assert!(text.contains("foreground"));
    assert!(
        text.contains("notifications"),
        "the fallbacks still get a mention"
    );
}

#[test]
fn a_push_that_beats_the_wait_still_counts() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    // The host holds the test command's answer: the push lands first.
    let page = rig.open(&c, None);
    rig.host(c.id).holding = true;
    rig.act(|m, k| m.setup_prove_push(page, k));
    let (_, _, command) = rig.held_execs.last().unwrap().clone();
    let nonce = between(&command, "--test '", "'").unwrap().to_owned();
    rig.act(|m, k| m.self_test_arrived(nonce, k));
    rig.let_go(c.id);
    assert!(matches!(rig.view(page).push_proof, Proof::Proven(_)));
}

#[test]
fn only_the_nonce_asked_for_satisfies_the_wait() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    let page = rig.open(&c, None);
    rig.act(|m, k| m.self_test_arrived("selftest-from-an-earlier-attempt".into(), k));
    rig.act(|m, k| m.setup_prove_push(page, k));
    rig.run_out_the_clock();
    assert!(matches!(rig.view(page).push_proof, Proof::Failed(_)));
}

#[test]
fn what_the_sender_said_is_the_reason_when_it_said_anything() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.set_host(c.id, Host::healthy().rule("--test", "relay refused: 401"));
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_prove_push(page, k));
    rig.run_out_the_clock();
    assert_eq!(
        rig.view(page).push_proof,
        Proof::Failed(Note::HostReported("relay refused: 401".into()))
    );
}

/// B359: whatever the sender said, the screen still waited out the whole
/// 20 s before showing it — though the sender prints only when the push
/// isn't coming (the relay refused it, no pairing to send with). It is the
/// answer at once now, with nothing left waiting.
#[test]
fn what_the_sender_said_is_the_answer_at_once() {
    for said in [
        "relay rejected this host's send token (401): this phone has not registered with https://push.example.org",
        "this host has no usable pairing to test: pair again from the app",
    ] {
        let mut rig = Rig::new().with_token();
        let c = conn("m1-pro");
        rig.set_host(c.id, Host::healthy().rule("--test", said));
        let page = rig.open(&c, None);
        rig.act(|m, k| m.setup_prove_push(page, k));
        assert_eq!(
            rig.view(page).push_proof,
            Proof::Failed(Note::HostReported(said.into())),
            "{said}"
        );
        assert_eq!(rig.machine.next_deadline(), None, "nothing left waiting");
    }
}

/// A push that arrived before the sender's words did still counts.
#[test]
fn a_push_that_beats_the_senders_words_still_counts() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.set_host(c.id, Host::healthy().rule("--test", "relay answered 500"));
    let page = rig.open(&c, None);
    rig.host(c.id).holding = true;
    rig.act(|m, k| m.setup_prove_push(page, k));
    let (_, _, command) = rig.held_execs.last().unwrap().clone();
    let nonce = between(&command, "--test '", "'").unwrap().to_owned();
    rig.act(|m, k| m.self_test_arrived(nonce, k));
    rig.let_go(c.id);
    assert!(matches!(rig.view(page).push_proof, Proof::Proven(_)));
}

/// Fix (prove-push-exec-fails): the app's wait outlived a failed command.
#[test]
fn a_test_the_host_couldnt_run_fails_at_once() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    let page = rig.open(&c, None);
    rig.host(c.id).fail_on = Some("--test".into());
    rig.act(|m, k| m.setup_prove_push(page, k));
    assert!(matches!(
        rig.view(page).push_proof,
        Proof::Failed(Note::Said(_))
    ));
    assert_eq!(rig.machine.next_deadline(), None, "nothing left waiting");
}

/// B353: the test named no connection, so the host sent it on every
/// pairing it held: another phone paired with the host showed it.
#[test]
fn the_push_test_names_the_connection_it_proves() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_prove_push(page, k));
    let command = rig.host(c.id).sent("--test").unwrap().to_owned();
    assert!(
        command.ends_with(&format!("' '{}' 2>&1", c.id)),
        "{command}"
    );
}

#[test]
fn every_self_test_uses_a_fresh_nonce() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_prove_push(page, k));
    rig.run_out_the_clock();
    rig.act(|m, k| m.setup_prove_push(page, k));
    rig.run_out_the_clock();
    let nonces: std::collections::HashSet<String> = rig
        .host(c.id)
        .commands
        .iter()
        .filter_map(|c| between(c, "--test '", "'").map(str::to_owned))
        .collect();
    assert_eq!(nonces.len(), 2);
}

#[test]
fn the_hook_test_needs_a_pane_and_says_so_instead_of_failing() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, Host::hooked());
    let page = rig.open(&c, None);
    let commands = rig.host(c.id).commands.len();
    rig.act(|m, k| m.setup_prove_hooks(page, k));
    let Proof::Unavailable(why) = rig.view(page).hooks_proof else {
        panic!("expected unavailable");
    };
    assert!(why.english().contains("tmux pane"));
    assert_eq!(rig.host(c.id).commands.len(), commands);
}

#[test]
fn a_host_whose_tmux_is_elsewhere_is_told_the_install_is_still_fine() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, Host::hooked());
    let page = rig.open(&c, Some("%3"));
    rig.act(|m, k| m.setup_prove_hooks(page, k));
    let Proof::Unavailable(why) = rig.view(page).hooks_proof else {
        panic!("expected unavailable");
    };
    assert!(why.english().contains("hooks themselves are unaffected"));
}

#[test]
fn firing_the_stamp_and_reading_it_back_is_what_counts_as_proof() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(
        c.id,
        Host::hooked()
            .rule("socket_path", "/tmp/tmux-501/default")
            .rule("display-message -p -t", "working|moshpit-selftest|"),
    );
    let page = rig.open(&c, Some("%3"));
    rig.act(|m, k| m.setup_prove_hooks(page, k));
    assert!(matches!(rig.view(page).hooks_proof, Proof::Proven(_)));
}

#[test]
fn a_phone_the_relay_does_not_know_is_said_so_even_when_the_host_is_perfect() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.store
        .0
        .lock()
        .unwrap()
        .pairings
        .push(moshpit_seal::PushPairing::make(
            c.id, "m1-pro", RELAY, rig.date,
        ));
    rig.relay = Relay::Refuses(401, "unknown device".into());
    rig.act(|m, k| m.foreground(k));
    let page = rig.open(&c, None);
    assert_eq!(
        rig.view(page).relay_error,
        Some(Note::Relay(RelayFailure::Status {
            code: 401,
            body: "unknown device".into()
        }))
    );
}

// MARK: - The state the old design had no name for

#[test]
fn paired_with_a_stale_stamp_script_is_surfaced_not_inferred() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    let mut manifest = paired_manifest(&c.id.to_string());
    manifest.set(
        &InstallComponent::Stamp,
        Some(Component {
            digest: Some("from-an-older-build".into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    let old = "#!/bin/sh\n# an older build's stamp script\n";
    rig.set_host(
        c.id,
        paired_host(&c.id.to_string())
            .with_manifest(&manifest)
            .with_file(&InstallComponent::Stamp.path().unwrap(), old),
    );
    let page = rig.open(&c, None);
    let view = rig.view(page);
    assert_eq!(
        view.state.unwrap().status(&InstallComponent::Stamp),
        ComponentStatus::Stale {
            installed: scripts::content_digest(old)
        }
    );
    assert!(view.nothing_will_push);
}

#[test]
fn unpairing_forgets_the_host_on_both_sides() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.store
        .0
        .lock()
        .unwrap()
        .pairings
        .push(ready_pairing(c.id));
    rig.set_host(c.id, paired_host(&c.id.to_string()));
    let page = rig.open(&c, None);
    assert_ne!(rig.view(page).pairing, ComponentStatus::Absent);
    rig.act(|m, k| m.setup_unpair(page, k));
    assert!(
        rig.pairing(c.id).is_none(),
        "the phone must drop its copy of the secret too"
    );
    assert!(
        rig.host(c.id).sent("rm -f").unwrap().contains("push.d"),
        "unpair must remove THIS device's push.d file"
    );
    let view = rig.view(page);
    assert_eq!(view.push_proof, Proof::Untested);
    assert_eq!(view.pairing, ComponentStatus::Absent);
    assert_eq!(view.error, None);
}

#[test]
fn unpairing_forgets_on_the_phone_before_the_host_is_touched() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.store
        .0
        .lock()
        .unwrap()
        .pairings
        .push(ready_pairing(c.id));
    rig.set_host(c.id, paired_host(&c.id.to_string()));
    let page = rig.open(&c, None);
    // The host holds: by the time anything reaches it, the phone has let go.
    rig.host(c.id).holding = true;
    rig.act(|m, k| m.setup_unpair(page, k));
    assert!(rig.pairing(c.id).is_none());
    assert!(rig.host(c.id).sent("rm -f").is_none());
    rig.let_go(c.id);
    assert!(rig.host(c.id).sent("rm -f").unwrap().contains("push.d"));
}

#[test]
fn a_phone_that_cant_forget_the_pairing_leaves_the_host_paired_and_says_so() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.store
        .0
        .lock()
        .unwrap()
        .pairings
        .push(ready_pairing(c.id));
    rig.set_host(c.id, paired_host(&c.id.to_string()));
    let page = rig.open(&c, None);
    rig.store.0.lock().unwrap().refuse_writes = true;
    rig.act(|m, k| m.setup_unpair(page, k));
    let view = rig.view(page);
    assert_eq!(
        view.error,
        Some(Note::Relay(RelayFailure::CouldNotForget)),
        "the secret is still on the phone, and nothing said so"
    );
    assert!(rig.host(c.id).sent("rm -f").is_none());
    assert_ne!(view.pairing, ComponentStatus::Absent);
    assert_eq!(view.phase, Phase::Idle);
}

// MARK: - A proof is a phase (B383)

/// A hooked host this phone has paired, with its page open.
fn paired_page(rig: &mut Rig, c: &moshpit_hostcare::Conn) -> u64 {
    rig.save(&[c.id]);
    rig.set_host(
        c.id,
        Host::hooked()
            .rule("socket_path", "/tmp/tmux-501/default")
            .rule("show-options", "working|moshpit-selftest|"),
    );
    rig.tend(c);
    assert!(rig.pairing(c.id).is_some(), "care paired it");
    rig.open(c, Some("%3"))
}

/// Every step the page can ask for, asked for at once.
fn press_every_step(rig: &mut Rig, page: u64) {
    rig.act(|m, k| m.setup_install_hooks(page, k));
    rig.act(|m, k| m.setup_remove_hooks(page, k));
    rig.act(|m, k| m.setup_pair(page, k));
    rig.act(|m, k| m.setup_unpair(page, k));
    rig.act(|m, k| m.setup_prove_hooks(page, k));
    rig.act(|m, k| m.setup_prove_push(page, k));
}

#[test]
fn a_push_test_holds_every_step_until_its_verdict() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    let page = paired_page(&mut rig, &c);
    rig.host(c.id).hold_on = Some("--test".into());
    rig.act(|m, k| m.setup_prove_push(page, k));
    assert_ne!(rig.view(page).phase, Phase::Idle, "a proof holds the page");
    assert_eq!(rig.view(page).push_proof, Proof::Proving);
    let (sent, held, mints) = (
        rig.host(c.id).commands.len(),
        rig.held_execs.len(),
        rig.mints.len(),
    );
    press_every_step(&mut rig, page);
    assert_eq!(
        rig.host(c.id).commands.len(),
        sent,
        "a step reached the host"
    );
    assert_eq!(rig.held_execs.len(), held);
    assert_eq!(rig.mints.len(), mints);
    assert!(rig.pairing(c.id).is_some(), "the unpair ran under the test");
    rig.let_go(c.id);
    rig.run_out_the_clock();
    let view = rig.view(page);
    assert_eq!(view.phase, Phase::Idle);
    assert_eq!(view.push_proof, Proof::Failed(Note::PushNeverSeen));
    assert_eq!(view.pairing, ComponentStatus::Current);
}

#[test]
fn a_hook_test_holds_every_step_until_its_verdict() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    let page = paired_page(&mut rig, &c);
    rig.host(c.id).hold_on = Some("socket_path".into());
    rig.act(|m, k| m.setup_prove_hooks(page, k));
    assert_ne!(rig.view(page).phase, Phase::Idle, "a proof holds the page");
    let (sent, held) = (rig.host(c.id).commands.len(), rig.held_execs.len());
    press_every_step(&mut rig, page);
    assert_eq!(
        rig.host(c.id).commands.len(),
        sent,
        "a step reached the host"
    );
    assert_eq!(rig.held_execs.len(), held);
    rig.let_go(c.id);
    let view = rig.view(page);
    assert_eq!(view.phase, Phase::Idle);
    assert!(
        matches!(view.hooks_proof, Proof::Proven(_)),
        "{:?}",
        view.hooks_proof
    );
}

#[test]
fn a_push_tests_verdict_is_dropped_when_its_pairing_changed_meanwhile() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    let page = paired_page(&mut rig, &c);
    rig.host(c.id).hold_on = Some("--test".into());
    rig.act(|m, k| m.setup_prove_push(page, k));
    let nonce = rig
        .held_execs
        .iter()
        .find_map(|(_, _, command)| between(command, "--test '", "'").map(str::to_owned))
        .expect("the test's command");
    // While it waits, the connection's pairing is replaced (another path
    // re-paired it): the host is sealing with a secret that is no longer
    // the one tested.
    {
        let mut store = rig.store.0.lock().unwrap();
        store.pairings.retain(|p| p.connection_id != c.id);
        store.pairings.push(ready_pairing(c.id));
    }
    rig.let_go(c.id);
    rig.act(|m, k| m.self_test_arrived(nonce, k));
    rig.run_out_the_clock();
    let view = rig.view(page);
    assert_eq!(view.phase, Phase::Idle);
    assert_eq!(
        view.push_proof,
        Proof::Untested,
        "a verdict about a pairing that's gone"
    );
}

// MARK: - The hooks' test is the agent's own registration (B380)

/// A host with Claude Code's hooks whose tmux takes the test's stamp.
fn stampable() -> Host {
    Host::hooked()
        .rule("socket_path", "/tmp/tmux-501/default")
        .rule("show-options", "working|moshpit-selftest|")
}

#[test]
fn the_hook_test_runs_only_on_hooks_this_build_installed() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, stampable());
    let page = rig.open(&c, Some("%3"));
    rig.act(|m, k| m.setup_select(page, "gemini", k));
    let sent = rig.host(c.id).commands.len();
    rig.act(|m, k| m.setup_prove_hooks(page, k));
    assert_eq!(
        rig.host(c.id).commands.len(),
        sent,
        "tested an agent with no hooks"
    );
    let Proof::Unavailable(why) = rig.view(page).hooks_proof else {
        panic!("{:?}", rig.view(page).hooks_proof);
    };
    assert_eq!(why.english(), "Install the hooks for Gemini CLI to test them.");
}

#[test]
fn the_hook_test_fires_the_command_the_agents_config_registers() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, stampable());
    let page = rig.open(&c, Some("%3"));
    rig.act(|m, k| m.setup_prove_hooks(page, k));
    assert!(matches!(rig.view(page).hooks_proof, Proof::Proven(_)));
    let fired = rig.host(c.id).sent("show-options").cloned().unwrap();
    assert!(
        fired.contains("MOSHPIT_SELFTEST=1 TMUX_PANE=$p sh ~/.moshpit/moshpit-stamp.sh working claude"),
        "not Claude Code's registered command: {fired}"
    );
}

#[test]
fn a_registration_missing_an_event_fails_the_hook_test_and_names_it() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    // Someone swapped Moshpit's Stop hook for one of their own by hand;
    // the rest is there (so the look reads the hooks as current).
    let config = moshpit_install::hooks::merged("", &agent("claude")).unwrap();
    let mut root: serde_json::Value = serde_json::from_str(&config).unwrap();
    root["hooks"]["Stop"] =
        serde_json::json!([{"hooks": [{"type": "command", "command": "say done"}]}]);
    rig.set_host(
        c.id,
        stampable().with_file("$HOME/.claude/settings.json", &root.to_string()),
    );
    let page = rig.open(&c, Some("%3"));
    assert_eq!(rig.view(page).hooks, ComponentStatus::Current);
    rig.act(|m, k| m.setup_prove_hooks(page, k));
    let Proof::Failed(why) = rig.view(page).hooks_proof else {
        panic!("{:?}", rig.view(page).hooks_proof);
    };
    assert_eq!(
        why.english(),
        "~/.claude/settings.json doesn't register Moshpit for Stop. Reinstall to put it back."
    );
}

#[test]
fn codexs_trust_step_is_read_from_the_host_not_from_the_last_install() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, stampable());
    let page = rig.open(&c, Some("%3"));
    rig.act(|m, k| m.setup_select(page, "codex", k));
    rig.act(|m, k| m.setup_install_hooks(page, k));
    assert_eq!(rig.view(page).user_action, Some(Note::TrustCodexHooks));
    // The test runs Codex's command itself: it proves nothing about trust.
    rig.act(|m, k| m.setup_prove_hooks(page, k));
    assert!(matches!(rig.view(page).hooks_proof, Proof::Proven(_)));
    assert_eq!(
        rig.view(page).user_action,
        Some(Note::TrustCodexHooks),
        "the test cleared the trust step"
    );
    // The sheet closed and opened again: the step is still left.
    rig.act(|m, k| m.close_setup(page, k));
    let page = rig.open(&c, Some("%3"));
    rig.act(|m, k| m.setup_select(page, "codex", k));
    assert_eq!(
        rig.view(page).user_action,
        Some(Note::TrustCodexHooks),
        "the trust step forgotten with the sheet"
    );
    // Codex ran one of its hooks on the host: trusted.
    rig.act(|m, k| m.close_setup(page, k));
    rig.host(c.id)
        .rules
        .push((".moshpit/seen\"".into(), "seen=codex".into()));
    let page = rig.open(&c, Some("%3"));
    rig.act(|m, k| m.setup_select(page, "codex", k));
    assert_eq!(rig.view(page).user_action, None);
}

// MARK: - The agents (B381)

#[test]
fn the_page_opens_on_the_first_agent_with_hooks_on_the_host() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, Host::healthy());
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_select(page, "codex", k));
    rig.act(|m, k| m.setup_install_hooks(page, k));
    rig.act(|m, k| m.close_setup(page, k));
    let page = rig.open(&c, None);
    let view = rig.view(page);
    assert_eq!(view.agent.id, "codex", "opened on an agent with nothing here");
    assert_eq!(view.hooks, ComponentStatus::Current);
}

#[test]
fn gemini_clis_hooks_go_under_its_own_event_names() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, Host::healthy());
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_select(page, "gemini", k));
    rig.act(|m, k| m.setup_install_hooks(page, k));
    let config = rig
        .host(c.id)
        .file("$HOME/.gemini/settings.json")
        .cloned()
        .expect("Gemini CLI's config");
    for event in ["BeforeAgent", "BeforeTool", "Notification", "AfterAgent"] {
        assert!(config.contains(&format!("\"{event}\"")), "{event}: {config}");
    }
    for skipped in ["UserPromptSubmit", "PreToolUse", "\"Stop\""] {
        assert!(!config.contains(skipped), "{skipped}: Gemini CLI skips it");
    }
}

#[test]
fn gemini_hooks_written_with_claude_codes_names_read_out_of_date_and_care_rewrites_them() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.save(&[c.id]);
    // An older build's Gemini registration: Claude Code's names, which
    // Gemini CLI skips but Notification.
    let old = moshpit_install::hooks::merged("", &agent("claude"))
        .unwrap()
        .replace(" claude", " gemini");
    let mut host = Host::hooked().with_file("$HOME/.gemini/settings.json", &old);
    let mut manifest = host.manifest();
    manifest.set(
        &InstallComponent::Hooks {
            agent: "gemini".into(),
        },
        Some(Component {
            config_path: Some("$HOME/.gemini/settings.json".into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    host = host.with_manifest(&manifest);
    rig.set_host(c.id, host);
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_select(page, "gemini", k));
    assert!(
        matches!(rig.view(page).hooks, ComponentStatus::Stale { .. }),
        "{:?}",
        rig.view(page).hooks
    );
    rig.act(|m, k| m.close_setup(page, k));
    rig.tend(&c);
    let config = rig
        .host(c.id)
        .file("$HOME/.gemini/settings.json")
        .cloned()
        .unwrap();
    assert!(config.contains("\"BeforeAgent\""), "{config}");
    assert!(!config.contains("UserPromptSubmit"), "{config}");
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_select(page, "gemini", k));
    assert_eq!(rig.view(page).hooks, ComponentStatus::Current);
}

// MARK: - A host not read (B382)

#[test]
fn a_host_that_couldnt_be_read_offers_nothing_but_reading_it_again() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.save(&[c.id]);
    let mut host = Host::hooked();
    host.fail_on = Some("printf".into());
    rig.set_host(c.id, host);
    let page = rig.open(&c, Some("%3"));
    assert!(rig.view(page).error.is_some());
    assert!(rig.view(page).facts.is_none());
    let sent = rig.host(c.id).commands.len();
    press_every_step(&mut rig, page);
    assert_eq!(
        rig.host(c.id).commands.len(),
        sent,
        "a step went to a host the page couldn't read"
    );
    assert!(rig.mints.is_empty(), "minted a pairing for an unread host");
    assert!(rig.pairing(c.id).is_none());
    // Retry, once the host answers.
    rig.host(c.id).fail_on = None;
    rig.act(|m, k| m.setup_inspect(page, k));
    let view = rig.view(page);
    assert!(view.facts.is_some());
    assert_eq!(view.error, None);
    assert_eq!(view.hooks, ComponentStatus::Current);
}

// MARK: - Switching agents (B235)

#[test]
fn switching_agents_clears_the_last_agents_test_result_pending_step_report_and_error() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(
        c.id,
        Host::healthy()
            .rule("socket_path", "/tmp/tmux-501/default")
            .rule("display-message -p -t", ""),
    );
    let page = rig.open(&c, Some("%3"));
    rig.act(|m, k| m.setup_select(page, "codex", k));
    rig.act(|m, k| m.setup_install_hooks(page, k));
    let view = rig.view(page);
    assert_eq!(
        view.user_action,
        Some(Note::TrustCodexHooks),
        "Codex's trust step"
    );
    assert!(view.last_report.is_some());
    rig.act(|m, k| m.setup_prove_hooks(page, k));
    let Proof::Failed(why) = rig.view(page).hooks_proof else {
        panic!("expected Codex's test to fail");
    };
    // The test runs the registered command itself, Codex isn't involved:
    // its trust step can't be why the pane came back empty (B380).
    assert!(
        !why.english().contains("/hooks"),
        "an empty pane blamed on Codex's trust step"
    );
    assert_eq!(
        rig.view(page).user_action,
        Some(Note::TrustCodexHooks),
        "a failed test settles nothing about the trust step"
    );

    rig.act(|m, k| m.setup_select(page, "gemini", k));
    let view = rig.view(page);
    // Codex's failed test went with it; Gemini CLI has nothing to test.
    assert!(
        matches!(&view.hooks_proof, Proof::Unavailable(why) if why.english().starts_with("Install the hooks")),
        "{:?}",
        view.hooks_proof
    );
    assert_eq!(view.user_action, None);
    assert!(view.last_report.is_none());
    assert_eq!(view.error, None);
    rig.act(|m, k| m.setup_select(page, "gemini", k));
    assert_eq!(rig.view(page).agent.id, "gemini");
}

#[test]
fn the_agent_stays_while_a_step_runs_and_the_steps_failure_shows() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    // Claude Code's config won't parse: its install fails.
    rig.set_host(
        c.id,
        Host::healthy().with_file("$HOME/.claude/settings.json", "{not json"),
    );
    let page = rig.open(&c, Some("%3"));
    rig.host(c.id).holding = true;
    rig.act(|m, k| m.setup_install_hooks(page, k));
    // The menu asked for another agent while the install ran.
    rig.act(|m, k| m.setup_select(page, "gemini", k));
    rig.let_go(c.id);
    let view = rig.view(page);
    assert_eq!(view.agent.id, "claude", "the agent changed under its step");
    assert!(
        matches!(&view.error, Some(Note::Step(_))),
        "the install's failure was dropped: {:?}",
        view.error
    );
    // Between steps the menu acts.
    rig.act(|m, k| m.setup_select(page, "gemini", k));
    assert_eq!(rig.view(page).agent.id, "gemini");
}

#[test]
fn the_agent_stays_while_a_hook_test_runs() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(
        c.id,
        Host::hooked()
            .rule("socket_path", "/tmp/tmux-501/default")
            .rule("show-options", "working|moshpit-selftest|"),
    );
    let page = rig.open(&c, Some("%3"));
    rig.host(c.id).holding = true;
    rig.act(|m, k| m.setup_prove_hooks(page, k));
    rig.act(|m, k| m.setup_select(page, "codex", k));
    rig.let_go(c.id);
    let view = rig.view(page);
    assert_eq!(view.agent.id, "claude");
    assert!(
        matches!(view.hooks_proof, Proof::Proven(_)),
        "the test's verdict was dropped: {:?}",
        view.hooks_proof
    );
}

#[test]
fn a_step_asked_for_while_another_runs_is_not_started() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    let page = rig.open(&c, None);
    rig.host(c.id).holding = true;
    rig.act(|m, k| m.setup_install_hooks(page, k));
    let held = rig.held_execs.len();
    rig.act(|m, k| m.setup_remove_hooks(page, k));
    rig.act(|m, k| m.setup_unpair(page, k));
    assert_eq!(rig.held_execs.len(), held);
    rig.let_go(c.id);
    assert_eq!(rig.view(page).hooks, ComponentStatus::Current);
}

#[test]
fn the_relay_field_falls_back_to_the_relay_the_host_is_paired_with() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.act(|m, k| {
        m.set_settings(
            moshpit_hostcare::Settings {
                relay_url: String::new(),
                ..Default::default()
            },
            k,
        )
    });
    rig.set_host(c.id, paired_host(&c.id.to_string()));
    let page = rig.open(&c, None);
    let view = rig.view(page);
    assert_eq!(view.paired_relay.as_deref(), Some("r"));
    assert_eq!(view.relay_url, "r");
}

#[test]
fn the_scripts_this_build_ships_read_as_current() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    rig.set_host(c.id, Host::hooked());
    let page = rig.open(&c, None);
    let view = rig.view(page);
    assert_eq!(view.hooks, ComponentStatus::Current);
    assert_eq!(
        view.state.unwrap().status(&InstallComponent::Sender),
        ComponentStatus::Current
    );
    assert!(scripts::digest(&InstallComponent::Stamp).is_some());
}

// MARK: - What was changed on the host by hand (B315)

#[test]
fn a_push_conf_deleted_by_hand_shows_as_unpaired() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    // The manifest records this phone's pairing; its conf isn't there.
    rig.set_host(
        c.id,
        Host::hooked().with_manifest(&{
            let mut m = Host::hooked().manifest();
            m.set(
                &InstallComponent::Pairing {
                    conn: c.id.to_string(),
                    install: None,
                },
                Some(Component {
                    digest: Some("d".repeat(64)),
                    connection_id: Some(c.id.to_string()),
                    relay_url: Some(RELAY.into()),
                    ..Component::new(Date::from_unix(1_762_800_000.0))
                }),
            );
            m
        }),
    );
    let page = rig.open(&c, None);
    assert_eq!(
        rig.view(page).pairing,
        ComponentStatus::Absent,
        "a host with no conf shown as paired"
    );
}

#[test]
fn hooks_taken_out_by_hand_show_as_not_installed() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    let mut host = Host::hooked();
    host.files
        .insert("$HOME/.claude/settings.json".into(), "{}".into());
    rig.set_host(c.id, host);
    let page = rig.open(&c, None);
    assert_eq!(rig.view(page).hooks, ComponentStatus::Absent);
}

#[test]
fn a_stamp_script_deleted_by_hand_shows_and_says_nothing_will_push() {
    let mut rig = Rig::new();
    let c = conn("m1-pro");
    let mut host = Host::hooked();
    host.files.remove(&InstallComponent::Stamp.path().unwrap());
    rig.set_host(c.id, host);
    let page = rig.open(&c, None);
    let view = rig.view(page);
    assert_eq!(
        view.state.unwrap().status(&InstallComponent::Stamp),
        ComponentStatus::Absent
    );
    assert_eq!(
        view.hooks,
        ComponentStatus::Absent,
        "the hooks call a script that isn't there"
    );
}

/// A relay's refusal is the pairing's it was for. A mint that then works —
/// Host Setup's Pair, or care's — clears that pairing's error; it used to
/// stand, phone-wide, next to a pairing that worked until a whole round
/// came back clean. Another pairing's failure stays, on its own page and
/// the phone's.
#[test]
fn a_mint_that_works_clears_its_own_pairings_relay_error_and_only_that() {
    let mut rig = Rig::new().with_token();
    let a = conn("m1-pro");
    let b = conn("nas");
    rig.save(&[a.id, b.id]);
    for c in [&a, &b] {
        let pairing = moshpit_seal::PushPairing::make(c.id, &c.label, RELAY, rig.date);
        rig.store.0.lock().unwrap().pairings.push(pairing);
    }
    rig.relay = Relay::Refuses(503, "busy".into());
    rig.act(|m, k| m.foreground(k));
    let refused = Some(Note::Relay(RelayFailure::Status {
        code: 503,
        body: "busy".into(),
    }));
    let page_a = rig.open(&a, None);
    let page_b = rig.open(&b, None);
    assert_eq!(rig.view(page_a).relay_error, refused);

    rig.relay = Relay::Mints;
    rig.act(|m, k| m.setup_pair(page_a, k));
    assert_eq!(rig.view(page_a).error, None);
    assert_eq!(rig.view(page_a).relay_error, None, "a's pairing works");
    assert_eq!(rig.view(page_b).relay_error, refused, "b's still doesn't");
    assert_eq!(rig.machine.relay_error(), refused);

    // Care mints b's when its host is next tended.
    rig.set_host(b.id, Host::hooked());
    rig.tend(&b);
    assert!(!rig.pairing(b.id).unwrap().send_token.is_empty());
    assert_eq!(rig.view(page_b).relay_error, None);
    assert_eq!(rig.machine.relay_error(), None);
}

// MARK: - A Re-pair that fails (B372)

/// A paired, hooked host, as care leaves it; the phone's pairing then.
fn paired(rig: &mut Rig, label: &str) -> (moshpit_hostcare::Conn, moshpit_seal::PushPairing) {
    let c = conn(label);
    rig.save(&[c.id]);
    rig.set_host(c.id, Host::hooked());
    rig.tend(&c);
    let phone = rig.pairing(c.id).expect("care paired the host");
    assert!(
        rig.host(c.id)
            .pairing_conf(c.id)
            .unwrap()
            .contains(&phone.secret_hex)
    );
    (c, phone)
}

/// B372 (hs1's P4): a Re-pair saved its fresh secret before the relay was
/// asked; with the relay down the phone was left holding a secret the
/// host's conf doesn't carry, and no send token — a working pairing broken,
/// while the page still said Current. The pairing that worked stays.
#[test]
fn a_repair_the_relay_refuses_keeps_the_pairing_that_worked() {
    for relay in [Relay::Down, Relay::Refuses(500, "no".into())] {
        let mut rig = Rig::new().with_token();
        let (c, before) = paired(&mut rig, "m1-pro");
        let page = rig.open(&c, None);
        rig.relay = relay.clone();
        rig.act(|m, k| m.setup_pair(page, k));
        rig.run_out_the_clock();
        let after = rig.pairing(c.id).unwrap();
        assert_eq!(
            after.secret_hex, before.secret_hex,
            "{relay:?}: the phone's secret was replaced"
        );
        assert_eq!(after.send_token, before.send_token);
        assert!(
            rig.host(c.id)
                .pairing_conf(c.id)
                .unwrap()
                .contains(&before.secret_hex)
        );
        let view = rig.view(page);
        assert!(view.error.is_some(), "{relay:?}");
        assert_eq!(view.pairing, ComponentStatus::Current, "{relay:?}");
    }
}

/// The same when the new conf can't be written to the host.
#[test]
fn a_repair_the_host_cant_take_keeps_the_pairing_that_worked() {
    let mut rig = Rig::new().with_token();
    let (c, before) = paired(&mut rig, "m1-pro");
    let page = rig.open(&c, None);
    rig.host(c.id).fail_on = Some(format!("p=\"$HOME/.moshpit/push.d/{}.conf\"", c.id));
    rig.act(|m, k| m.setup_pair(page, k));
    rig.host(c.id).fail_on = None;
    rig.run_out_the_clock();
    assert_eq!(rig.pairing(c.id).unwrap().secret_hex, before.secret_hex);
    assert!(
        rig.host(c.id)
            .pairing_conf(c.id)
            .unwrap()
            .contains(&before.secret_hex)
    );
    assert!(rig.view(page).error.is_some());
}

/// A conf that landed but read back wrong is the host's now: the phone
/// keeps the pairing that wrote it.
#[test]
fn a_repair_whose_conf_landed_keeps_the_new_pairing() {
    let mut rig = Rig::new().with_token();
    let (c, before) = paired(&mut rig, "m1-pro");
    let page = rig.open(&c, None);
    let conf = format!("sha256sum \"$HOME/.moshpit/push.d/{}.conf\"", c.id);
    rig.host(c.id).rules.insert(0, (conf, "deadbeef".into()));
    rig.act(|m, k| m.setup_pair(page, k));
    rig.run_out_the_clock();
    let after = rig.pairing(c.id).unwrap();
    assert_ne!(after.secret_hex, before.secret_hex);
    assert!(
        rig.host(c.id)
            .pairing_conf(c.id)
            .unwrap()
            .contains(&after.secret_hex)
    );
    assert!(
        rig.view(page).error.is_some(),
        "the read-back is still said"
    );
}

/// B372: the pairing's status compared presence only; a host conf for a
/// secret the phone no longer holds read Current. It is Stale.
#[test]
fn a_host_conf_the_phone_cant_open_reads_stale() {
    let mut rig = Rig::new().with_token();
    let (c, before) = paired(&mut rig, "m1-pro");
    let page = rig.open(&c, None);
    assert_eq!(rig.view(page).pairing, ComponentStatus::Current);
    {
        let mut store = rig.store.0.lock().unwrap();
        let mine = store
            .pairings
            .iter_mut()
            .find(|p| p.connection_id == c.id)
            .unwrap();
        mine.secret_hex = "ef".repeat(32);
    }
    rig.act(|m, k| m.setup_inspect(page, k));
    assert!(
        matches!(rig.view(page).pairing, ComponentStatus::Stale { .. }),
        "{:?}",
        rig.view(page).pairing
    );
    let _ = before;
}

// MARK: - A step and another phone (B373)

/// B373 (hs1's P2a): a step wrote back the manifest as the page had read
/// it, so another phone's pairing record, written while the page was open,
/// was erased (its conf left on the host, recorded nowhere).
#[test]
fn a_step_keeps_what_another_phone_recorded_meanwhile() {
    let mut rig = Rig::new().with_token();
    let a = conn("m1-pro");
    rig.save(&[a.id]);
    rig.set_host(a.id, Host::hooked());
    let page = rig.open(&a, None);
    // Another phone pairs with the same host while A's page is open.
    let b = moshpit_seal::Uuid::new_v4().to_string();
    let conf_b = "RELAY_URL=r\nSEND_TOKEN=t\nSECRET=s\n";
    {
        let date = rig.date;
        let host = rig.host(a.id);
        let mut manifest = host.manifest();
        manifest.set(
            &InstallComponent::Pairing { conn: b.clone(), install: None },
            Some(Component {
                digest: Some(scripts::content_digest(conf_b)),
                connection_id: Some(b.clone()),
                relay_url: Some("r".into()),
                ..Component::new(date)
            }),
        );
        host.files
            .insert(InstallManifest::PATH.into(), manifest.to_json());
        host.files
            .insert(format!("$HOME/.moshpit/push.d/{b}.conf"), conf_b.into());
    }
    rig.act(|m, k| m.setup_select(page, "codex", k));
    rig.act(|m, k| m.setup_install_hooks(page, k));
    let manifest = rig.host(a.id).manifest();
    assert!(
        manifest.pairing_entry(&b).is_some(),
        "another phone's pairing record was erased"
    );
    assert!(
        manifest
            .get(&InstallComponent::Hooks {
                agent: "codex".into()
            })
            .is_some()
    );
}

/// B373 (hs1's P2b): the same for an agent's hooks: another phone installs
/// Codex's hooks while A's page is open; A pairs; Codex's record stays, and
/// the page says its hooks are installed.
#[test]
fn a_pair_keeps_the_hooks_another_phone_installed_meanwhile() {
    let mut rig = Rig::new().with_token();
    let a = conn("m1-pro");
    rig.save(&[a.id]);
    rig.set_host(a.id, Host::hooked());
    let page = rig.open(&a, None);
    {
        let date = rig.date;
        let host = rig.host(a.id);
        let mut manifest = host.manifest();
        let codex = agent("codex");
        manifest.set(
            &InstallComponent::Hooks {
                agent: "codex".into(),
            },
            Some(Component {
                config_path: Some(codex.config_path.into()),
                ..Component::new(date)
            }),
        );
        host.files
            .insert(InstallManifest::PATH.into(), manifest.to_json());
        host.files.insert(
            codex.config_path.into(),
            moshpit_install::hooks::merged("", &codex).unwrap(),
        );
    }
    rig.act(|m, k| m.setup_pair(page, k));
    let manifest = rig.host(a.id).manifest();
    assert!(
        manifest
            .get(&InstallComponent::Hooks {
                agent: "codex".into()
            })
            .is_some(),
        "Codex's hooks record was erased"
    );
    rig.act(|m, k| m.setup_select(page, "codex", k));
    assert_ne!(rig.view(page).hooks, ComponentStatus::Absent);
}

/// Every manifest write takes the host's lock, reads the manifest again,
/// and lets go after.
#[test]
fn every_manifest_write_is_read_again_under_the_lock() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.save(&[c.id]);
    rig.set_host(c.id, Host::hooked());
    rig.tend(&c);
    let sent = &rig.host(c.id).commands;
    let write = sent
        .iter()
        .position(|c| c.contains("p=\"$HOME/.moshpit/manifest.json\""))
        .expect("the manifest was written");
    // The lock and its release are the installer's own commands, under the
    // one token the write was made with (B566's symlink lock): compared
    // whole, so a change to the lock can't leave this test pinning an old
    // shape.
    let lock = &sent[write - 2];
    let token = lock
        .strip_prefix("t=")
        .and_then(|rest| rest.split(';').next())
        .map(|quoted| quoted.trim_matches('\''))
        .expect("the lock names its token first");
    assert_eq!(lock, &commands::lock(token));
    assert_eq!(
        sent[write - 1],
        commands::read_file(InstallManifest::PATH)
    );
    assert_eq!(sent[write + 1], commands::unlock(token));
}

// MARK: - Nothing will push; a Remove remembered (B374)

/// B374 (hs1's P6): removing the only agent's hooks on a paired host left
/// it pushing nothing, and the card that says so stayed hidden (it looked
/// at the stamp script alone); the next app run, care asked to enable
/// notifications again. It says so, and the Remove is remembered like a no.
#[test]
fn removing_the_last_hooks_says_nothing_will_push_and_isnt_asked_about_again() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.save(&[c.id]);
    rig.set_host(c.id, Host::hooked());
    rig.tend(&c);
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_remove_hooks(page, k));
    let view = rig.view(page);
    assert_eq!(view.hooks, ComponentStatus::Absent);
    assert_eq!(view.pairing, ComponentStatus::Current);
    assert!(view.nothing_will_push, "no 'nothing will push' card");
    assert_eq!(rig.declined, [c.id], "the Remove is remembered");

    // The next app run, same host, with what the platform kept.
    let host = rig.hosts.remove(&c.id).unwrap();
    let mut next = Rig::new().with_token();
    next.store = rig.store.clone();
    next.act(|m, k| {
        m.set_settings(
            moshpit_hostcare::Settings {
                relay_url: RELAY.into(),
                declined: vec![c.id.to_string()],
                ..Default::default()
            },
            k,
        )
    });
    next.save(&[c.id]);
    next.set_host(c.id, host);
    next.tend(&c);
    assert_eq!(next.machine.consent(), None, "asked again after the Remove");
}

// MARK: - Remove everything (B375)

/// B375: Remove everything lets go of this phone's pairing first (a phone
/// that can't would stop it before the host is touched), takes Moshpit off
/// the host, and is remembered: care neither pairs the connection again
/// nor asks to put the hooks back.
#[test]
fn remove_everything_lets_go_on_the_phone_and_is_remembered() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.save(&[c.id]);
    rig.set_host(c.id, Host::hooked());
    rig.tend(&c);
    assert!(rig.pairing(c.id).is_some());
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_remove_everything(page, k));
    let view = rig.view(page);
    assert_eq!(view.error, None, "{:?}", view.error);
    assert!(rig.pairing(c.id).is_none());
    assert!(rig.host(c.id).pairing_conf(c.id).is_none());
    assert!(
        !rig.host(c.id)
            .file("$HOME/.claude/settings.json")
            .unwrap()
            .contains("moshpit-stamp.sh")
    );
    assert!(
        rig.host(c.id)
            .commands
            .iter()
            .any(|c| c.starts_with("rm -f \"$HOME/.moshpit/moshpit-stamp.sh\"")),
        "the rest wasn't taken off a host no other phone uses"
    );
    assert_eq!(rig.unpaired, [(c.id, true)]);
    assert_eq!(rig.declined, [c.id]);
    rig.act(|m, k| m.close_setup(page, k));
    let mints = rig.mints.len();
    rig.act(|m, k| m.forget(c.id, k));
    rig.tend(&c);
    assert_eq!(rig.mints.len(), mints, "paired again");
    assert_eq!(rig.machine.consent(), None, "asked again");
}
