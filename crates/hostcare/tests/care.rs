//! `HostAutoCarePairingTests`, `HostAutoCareTests` and the rest of host
//! care's rules: it pairs only a connection that is saved and not being
//! deleted or unpaired, asks before the first hook install, refreshes only
//! what was consented to, and pairs a host one operation at a time.

mod support;

use moshpit_hostcare::{Config, Note, Phase, PushSupport, RelayFailure, Settings};
use moshpit_install::installer::{ComponentStatus, InstallState};
use moshpit_install::manifest::{Component, InstallComponent, InstallManifest};
use moshpit_install::{HookAgent, HostFacts, scripts};
use moshpit_seal::{Date, PushPairing, Uuid};
use support::*;

/// Codex's config with Moshpit's hooks in it.
fn codex_config() -> String {
    moshpit_install::hooks::merged("", &agent("codex")).unwrap()
}

/// A saved connection whose host carries Claude Code's hooks.
fn hooked(rig: &mut Rig, label: &str) -> moshpit_hostcare::Conn {
    let c = conn(label);
    rig.save(&[c.id]);
    rig.set_host(c.id, Host::hooked());
    c
}

/// The positive control: what every test below expects NOT to happen does
/// happen to a connection nobody deletes.
#[test]
fn a_saved_connection_is_paired_and_its_host_gets_the_pairing() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    rig.tend(&c);
    assert!(!rig.machine.is_tending(c.id));
    assert_eq!(rig.mints, [c.id.to_string()]);
    let phone = rig.pairing(c.id).unwrap();
    assert_eq!(
        rig.host(c.id).pairing_conf(c.id),
        Some(&moshpit_install::push_conf(&phone))
    );
    let manifest = rig.host(c.id).manifest();
    let entry = manifest.pairing_entry(&c.id.to_string()).unwrap();
    assert_eq!(entry.relay_url.as_deref(), Some(RELAY));
}

#[test]
fn a_connection_deleted_while_care_reads_its_host_is_never_paired() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "doomed");
    rig.host(c.id).holding = true;
    rig.tend(&c);
    assert!(
        !rig.held_execs.is_empty(),
        "care never asked the host anything"
    );

    // The delete: begun before anything waits; the session's teardown lets
    // the host answer; the pass is waited out; the connection goes.
    rig.act(|m, k| m.deletion_began(c.id, k).unwrap());
    rig.let_go(c.id);
    assert!(
        !rig.machine.is_tending(c.id),
        "the delete waits out the pass"
    );
    rig.save(&[]);
    rig.act(|m, k| m.deletion_ended(c.id, k));

    assert!(
        rig.mints.is_empty(),
        "care paired a connection that was being deleted"
    );
    assert!(rig.pairing(c.id).is_none());
    assert!(
        rig.host(c.id).pairing_conf(c.id).is_none(),
        "a pairing was written to the deleted connection's host"
    );
}

#[test]
fn the_delete_stands_the_pass_down_at_once() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "doomed");
    rig.host(c.id).holding = true;
    rig.tend(&c);
    assert!(rig.machine.is_tending(c.id));
    rig.act(|m, k| m.deletion_began(c.id, k).unwrap());
    assert!(
        !rig.machine.is_tending(c.id),
        "the pass a delete cancels ends before the delete goes on"
    );
    // The host's late answer to the command already out lands nowhere.
    rig.let_go(c.id);
    assert_eq!(rig.host(c.id).commands.len(), 1, "nothing more was sent");
    assert!(rig.mints.is_empty());
}

/// Not only Home's delete: whatever took the connection out of the store,
/// a pass that comes back from the host finds it gone and pairs nothing.
#[test]
fn a_connection_no_longer_saved_when_care_comes_back_from_its_host_is_never_paired() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "gone");
    rig.host(c.id).holding = true;
    rig.tend(&c);
    rig.save(&[]);
    rig.let_go(c.id);
    assert!(!rig.machine.is_tending(c.id));
    assert!(
        rig.mints.is_empty(),
        "care paired a connection that is no longer saved"
    );
    assert!(rig.host(c.id).pairing_conf(c.id).is_none());
}

#[test]
fn a_connection_that_isnt_saved_gets_no_care_at_all() {
    let mut rig = Rig::new().with_token();
    let c = conn("stranger");
    rig.set_host(c.id, Host::hooked());
    rig.tend(&c);
    assert!(rig.mints.is_empty());
    assert!(
        rig.host(c.id).commands.is_empty(),
        "care went to the host of a connection that isn't saved"
    );
}

/// A delete the phone couldn't go through with keeps the connection — and
/// then care treats it as the saved connection it still is.
#[test]
fn a_delete_that_couldnt_forget_the_pairing_leaves_the_connection_to_care_as_before() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "stuck");
    rig.host(c.id).holding = true;
    rig.tend(&c);
    rig.store.0.lock().unwrap().refuse_writes = true;
    let refused = {
        let clock = rig.clock();
        let result = rig.machine.deletion_began(c.id, clock);
        rig.settle();
        result
    };
    assert_eq!(refused, Err(Note::Relay(RelayFailure::CouldNotForget)));
    rig.store.0.lock().unwrap().refuse_writes = false;
    rig.let_go(c.id);
    assert!(!rig.machine.is_tending(c.id));
    assert!(
        rig.mints.is_empty(),
        "the pass the delete cancelled went on to pair"
    );

    // Its next session.
    rig.tend(&c);
    assert_eq!(
        rig.mints,
        [c.id.to_string()],
        "a kept connection was left unpaired for good"
    );
    assert!(rig.host(c.id).pairing_conf(c.id).is_some());
}

#[test]
fn care_tends_a_connection_once_a_run_and_afresh_after_a_delete_is_called_off() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    rig.tend(&c);
    let commands = rig.host(c.id).commands.len();
    rig.tend(&c);
    assert_eq!(rig.host(c.id).commands.len(), commands, "tended once a run");
    rig.act(|m, k| m.forget(c.id, k));
    rig.tend(&c);
    assert!(
        rig.host(c.id).commands.len() > commands,
        "a fresh look after Host Setup acted"
    );
}

/// B440: a pairing kept the name the connection was paired under, and a
/// pushed card names its place by it — after a rename the pushed copy of
/// an event and the local one read differently. The next care pass gives
/// the pairing the connection's name; the label never leaves the phone, so
/// the host's conf stays as it is and nothing is minted again. A blank
/// name leaves the label.
#[test]
fn a_renamed_connections_pairing_takes_its_new_name() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "old name");
    rig.tend(&c);
    let before = rig.pairing(c.id).unwrap();
    assert_eq!(before.host_label, "old name");
    let conf = rig.host(c.id).pairing_conf(c.id).cloned();
    let renamed = moshpit_hostcare::Conn { id: c.id, label: "studio".into() };
    rig.act(|m, k| m.forget(c.id, k));
    rig.tend(&renamed);
    let after = rig.pairing(c.id).unwrap();
    assert_eq!(after.host_label, "studio");
    assert_eq!(after.secret_hex, before.secret_hex);
    assert_eq!(after.send_token, before.send_token);
    assert_eq!(rig.host(c.id).pairing_conf(c.id).cloned(), conf, "the host was touched");
    assert_eq!(rig.mints, [c.id.to_string()], "minted again");

    let blank = moshpit_hostcare::Conn { id: c.id, label: "  ".into() };
    rig.act(|m, k| m.forget(c.id, k));
    rig.tend(&blank);
    assert_eq!(rig.pairing(c.id).unwrap().host_label, "studio");
}

// MARK: - Unpairing from Host Setup

/// Host Setup's Unpair, tapped while the care pass that started with the
/// session is still out reading the host (B88).
#[test]
fn unpairing_while_care_reads_the_host_leaves_the_host_unpaired() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    rig.host(c.id).holding = true;
    rig.tend(&c);
    assert!(
        !rig.held_execs.is_empty(),
        "care never asked the host anything"
    );

    // A screen opened before (its inspect answered).
    let page = {
        rig.host(c.id).holding = false;
        let held = std::mem::take(&mut rig.held_execs);
        let page = rig.open(&c, None);
        rig.held_execs = held;
        rig.host(c.id).holding = true;
        page
    };
    rig.act(|m, k| m.setup_unpair(page, k));
    assert_ne!(
        rig.view(page).phase,
        moshpit_hostcare::Phase::Idle,
        "the unpair started"
    );
    rig.let_go(c.id);
    rig.run_out_the_clock();

    assert!(
        rig.mints.is_empty(),
        "care paired the host the user had just unpaired"
    );
    assert!(rig.pairing(c.id).is_none());
    assert!(
        rig.host(c.id).pairing_conf(c.id).is_none(),
        "a pairing was written back"
    );
    assert_eq!(rig.view(page).error, None);
    assert!(
        !rig.machine.is_tending(c.id),
        "the unpair finished with care still at work"
    );
}

/// B371: an Unpair in Host Setup was undone at the connection's next
/// session — care minted a new pairing and wrote it back to the host (hs1's
/// review, #2). It stands until the person pairs again: care still tends
/// the host's scripts but pairs nothing, the mark is remembered across app
/// runs, and Pair ends it.
#[test]
fn an_unpair_stands_until_the_person_pairs_again() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    rig.tend(&c);
    assert!(rig.pairing(c.id).is_some(), "care paired the hooked host");
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_unpair(page, k));
    assert!(rig.pairing(c.id).is_none());
    assert!(rig.host(c.id).pairing_conf(c.id).is_none());
    assert_eq!(rig.unpaired, [(c.id, true)], "the Unpair is remembered");
    rig.act(|m, k| m.close_setup(page, k));

    // The session comes up again.
    let mints = rig.mints.len();
    rig.act(|m, k| m.forget(c.id, k));
    rig.tend(&c);
    assert_eq!(rig.mints.len(), mints, "care minted again after an Unpair");
    assert!(rig.pairing(c.id).is_none(), "re-paired on the phone");
    assert!(
        rig.host(c.id).pairing_conf(c.id).is_none(),
        "the conf is back on the host"
    );

    // The next app run: the platform hands the mark back.
    let host = rig.hosts.remove(&c.id).unwrap();
    let mut next = Rig::new().with_token();
    next.act(|m, k| {
        m.set_settings(
            Settings {
                relay_url: RELAY.into(),
                unpaired: vec![c.id.to_string()],
                ..Settings::default()
            },
            k,
        )
    });
    next.save(&[c.id]);
    next.set_host(c.id, host);
    next.tend(&c);
    assert!(next.mints.is_empty(), "paired again on the next run");
    assert!(next.host(c.id).pairing_conf(c.id).is_none());

    // Pair by hand: the mark goes, and the host is paired.
    let page = next.open(&c, None);
    next.act(|m, k| m.setup_pair(page, k));
    assert_eq!(next.unpaired, [(c.id, false)]);
    assert!(next.pairing(c.id).is_some());
    assert!(next.host(c.id).pairing_conf(c.id).is_some());
    // And care tends the pairing as any other from then on.
    next.act(|m, k| m.close_setup(page, k));
    next.host(c.id).files.remove(&conf_path(&c));
    next.act(|m, k| m.forget(c.id, k));
    next.tend(&c);
    assert!(
        next.host(c.id).pairing_conf(c.id).is_some(),
        "care left a deleted conf alone"
    );
}

// MARK: - Pairing by hand while care pairs (B123)

/// Host Setup's Pair tapped while the care pass that started with the
/// session is pairing the same host. One pairing operation per host at a
/// time: phone and host end up holding the same pairing whichever order
/// the relay answers in.
#[test]
fn pairing_by_hand_while_care_pairs_leaves_phone_and_host_with_the_same_pairing() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    let page = rig.open(&c, None);
    rig.relay = Relay::Holds;
    rig.tend(&c);
    assert_eq!(rig.held_posts.len(), 1, "care never minted");

    rig.act(|m, k| m.setup_pair(page, k));
    assert_eq!(
        rig.held_posts.len(),
        1,
        "the tap minted beside care's pairing"
    );
    // Care's conf write is slow; the tap is not let in meanwhile.
    rig.host(c.id).holding = true;
    rig.answer_mint(0);
    assert!(
        rig.held_posts.is_empty(),
        "the tap minted before care's install landed"
    );
    rig.let_go(c.id);
    assert_eq!(rig.held_posts.len(), 1, "the tap never minted");
    rig.answer_mint(0);
    rig.run_out_the_clock();
    assert!(!rig.machine.is_tending(c.id));
    let phone = rig.pairing(c.id).unwrap();
    assert_eq!(rig.mints.len(), 2);
    assert_eq!(
        rig.host(c.id).pairing_conf(c.id),
        Some(&moshpit_install::push_conf(&phone)),
        "the host holds another pairing than the phone"
    );
}

#[test]
fn care_waits_out_a_pair_by_hand_and_reads_the_host_again() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    let page = rig.open(&c, None);
    rig.relay = Relay::Holds;
    rig.act(|m, k| m.setup_pair(page, k));
    assert_eq!(rig.held_posts.len(), 1);
    rig.tend(&c);
    assert_eq!(rig.held_posts.len(), 1, "care minted beside the tap");
    rig.relay = Relay::Mints;
    rig.answer_mint(0);
    rig.run_out_the_clock();
    // Care found the screen's pairing on phone and host: nothing re-minted.
    assert_eq!(rig.mints.len(), 1);
    let phone = rig.pairing(c.id).unwrap();
    assert_eq!(
        rig.host(c.id).pairing_conf(c.id),
        Some(&moshpit_install::push_conf(&phone))
    );
}

// MARK: - Consent

#[test]
fn a_host_with_no_hooks_is_asked_about_once_and_otherwise_left_alone() {
    let mut rig = Rig::new().with_token();
    let c = conn("fresh");
    rig.save(&[c.id]);
    rig.tend(&c);
    assert_eq!(rig.machine.consent(), Some(c.clone()));
    assert!(
        rig.host(c.id).written_paths().is_empty(),
        "nothing written without a yes"
    );
    assert!(rig.mints.is_empty());
}

#[test]
fn a_yes_installs_claude_codes_hooks_and_pairs() {
    let mut rig = Rig::new().with_token();
    let c = conn("fresh");
    rig.save(&[c.id]);
    rig.tend(&c);
    rig.act(|m, k| m.grant_consent(c.clone(), k));
    assert_eq!(rig.machine.consent(), None);
    let manifest = rig.host(c.id).manifest();
    assert!(
        manifest
            .get(&InstallComponent::Hooks {
                agent: "claude".into()
            })
            .is_some()
    );
    assert!(rig.host(c.id).pairing_conf(c.id).is_some());
    assert_eq!(rig.mints.len(), 1);
}

#[test]
fn a_no_is_remembered_and_never_asked_again() {
    let mut rig = Rig::new().with_token();
    let c = conn("fresh");
    rig.save(&[c.id]);
    rig.tend(&c);
    rig.act(|m, k| m.decline(c.id, k));
    assert_eq!(rig.machine.consent(), None);
    assert_eq!(rig.declined, [c.id]);
    rig.act(|m, k| m.forget(c.id, k));
    rig.tend(&c);
    assert_eq!(rig.machine.consent(), None, "asked again after a no");
}

#[test]
fn not_now_asks_again_next_run_not_next_session() {
    let mut rig = Rig::new().with_token();
    let c = conn("fresh");
    rig.save(&[c.id]);
    rig.tend(&c);
    rig.act(|m, k| m.dismiss_consent(k));
    assert_eq!(rig.machine.consent(), None);
    rig.tend(&c);
    assert_eq!(rig.machine.consent(), None, "tended once a run");
}

#[test]
fn a_connection_whose_delete_begins_is_no_longer_asked_about() {
    let mut rig = Rig::new().with_token();
    let c = conn("fresh");
    rig.save(&[c.id]);
    rig.tend(&c);
    assert!(rig.machine.consent().is_some());
    rig.act(|m, k| m.deletion_began(c.id, k).unwrap());
    assert_eq!(rig.machine.consent(), None);
}

#[test]
fn a_yes_for_a_connection_being_deleted_does_nothing() {
    let mut rig = Rig::new().with_token();
    let c = conn("fresh");
    rig.save(&[c.id]);
    rig.tend(&c);
    let commands = rig.host(c.id).commands.len();
    rig.act(|m, k| m.deletion_began(c.id, k).unwrap());
    rig.act(|m, k| m.grant_consent(c.clone(), k));
    assert_eq!(rig.host(c.id).commands.len(), commands);
}

// MARK: - What care refreshes

#[test]
fn only_agents_the_host_actually_carries_are_ever_refreshed() {
    let mut manifest = InstallManifest::empty();
    manifest.set(
        &InstallComponent::Hooks {
            agent: "claude".into(),
        },
        Some(Component {
            digest: Some("d".into()),
            config_path: Some("$HOME/.claude/settings.json".into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    let agents = moshpit_hostcare::consented_agents(&manifest);
    assert_eq!(
        agents.iter().map(|a| a.id).collect::<Vec<_>>(),
        ["claude"],
        "refreshing an agent the user never enabled would BE the consent violation"
    );
    assert!(moshpit_hostcare::consented_agents(&InstallManifest::empty()).is_empty());
}

#[test]
fn a_device_finds_its_own_pairing_and_no_other_devices() {
    let conn = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";
    let mut manifest = InstallManifest::empty();
    manifest.set(
        &InstallComponent::Pairing { conn: conn.into(), install: None },
        Some(Component {
            digest: Some("x".into()),
            connection_id: Some(conn.into()),
            relay_url: Some("r".into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    assert_eq!(
        manifest
            .pairing_entry(conn)
            .and_then(|c| c.digest.as_deref()),
        Some("x")
    );
    assert!(manifest.pairing_entry("another-device").is_none());
}

#[test]
fn removing_one_devices_pairing_leaves_every_other_devices_standing() {
    let mut manifest = InstallManifest::empty();
    for conn in ["a", "b"] {
        manifest.set(
            &InstallComponent::Pairing { conn: conn.into(), install: None },
            Some(Component {
                connection_id: Some(conn.into()),
                relay_url: Some("r".into()),
                ..Component::new(Date::from_unix(1_762_800_000.0))
            }),
        );
    }
    manifest.remove_pairing("a");
    assert!(manifest.pairing_entry("a").is_none());
    assert!(manifest.pairing_entry("b").is_some());
    assert_eq!(manifest.pairing_entries().len(), 1);
}

/// A host whose sender is stale gets every consented agent re-installed:
/// `hooks_status` folds in the stamp but not the sender, and a stale
/// sender under a fresh pairing fails silently.
#[test]
fn a_stale_sender_refreshes_every_consented_agent() {
    let mut rig = Rig::new().with_token();
    let c = conn("kept");
    rig.save(&[c.id]);
    let mut manifest = Host::hooked().manifest();
    manifest.set(
        &InstallComponent::Hooks {
            agent: "codex".into(),
        },
        Some(Component {
            config_path: Some("$HOME/.codex/config.toml".into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    manifest.set(
        &InstallComponent::Sender,
        Some(Component {
            digest: Some("old".into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    rig.set_host(
        c.id,
        Host::hooked()
            .with_manifest(&manifest)
            .with_file(
                &InstallComponent::Sender.path().unwrap(),
                "# an older sender\n",
            )
            .with_file("$HOME/.codex/config.toml", &codex_config()),
    );
    rig.tend(&c);
    let host = rig.host(c.id);
    assert!(
        host.file("$HOME/.claude/settings.json")
            .unwrap()
            .contains("moshpit-stamp.sh")
    );
    assert!(
        host.file("$HOME/.codex/config.toml")
            .unwrap()
            .contains("moshpit-stamp.sh")
    );
    let after = InstallState {
        facts: HostFacts::parse(ALL_TOOLS),
        manifest: host.manifest(),
    };
    assert_eq!(
        after.status(&InstallComponent::Sender),
        ComponentStatus::Current
    );
}

/// With the sender current and the stamp script stale, the first agent's
/// refresh brings the stamp current, and the next agent's hooks read
/// current from the host as it is now: not installed twice.
#[test]
fn each_agents_status_is_read_from_the_host_as_it_is_after_the_one_before() {
    let mut rig = Rig::new().with_token();
    let c = conn("kept");
    rig.save(&[c.id]);
    let mut manifest = Host::hooked().manifest();
    manifest.set(
        &InstallComponent::Hooks {
            agent: "codex".into(),
        },
        Some(Component {
            config_path: Some("$HOME/.codex/config.toml".into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    manifest.set(
        &InstallComponent::Stamp,
        Some(Component {
            digest: Some("old".into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    rig.set_host(
        c.id,
        Host::hooked()
            .with_manifest(&manifest)
            .with_file(
                &InstallComponent::Stamp.path().unwrap(),
                "# an older stamp\n",
            )
            .with_file("$HOME/.codex/config.toml", &codex_config()),
    );
    rig.tend(&c);
    let host = rig.host(c.id);
    assert!(host.file("$HOME/.claude/settings.json").is_some());
    assert!(
        host.sent("cat \"$HOME/.codex/config.toml\"").is_none(),
        "codex's hooks were re-merged though the host had them current"
    );
}

#[test]
fn a_host_with_everything_current_and_this_phones_pairing_is_left_alone() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    rig.tend(&c);
    let writes = rig.host(c.id).written_paths().len();
    rig.act(|m, k| m.forget(c.id, k));
    rig.tend(&c);
    assert_eq!(
        rig.host(c.id).written_paths().len(),
        writes,
        "nothing to put right"
    );
    assert_eq!(rig.mints.len(), 1);
}

#[test]
fn a_pairing_stale_on_the_host_is_rewritten() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    let phone = ready_pairing(c.id);
    rig.store.0.lock().unwrap().pairings.push(phone.clone());
    rig.tend(&c);
    assert!(rig.mints.is_empty(), "a ready pairing isn't minted again");
    assert_eq!(
        rig.host(c.id).pairing_conf(c.id),
        Some(&moshpit_install::push_conf(&phone))
    );
}

#[test]
fn a_pairing_minted_at_another_relay_is_corrected_and_minted_again() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    let mut wrong = ready_pairing(c.id);
    wrong.relay_url = "https://push.example".into();
    rig.store.0.lock().unwrap().pairings.push(wrong.clone());
    rig.tend(&c);
    let phone = rig.pairing(c.id).unwrap();
    assert_eq!(phone.relay_url, RELAY);
    assert_eq!(phone.secret_hex, wrong.secret_hex, "the secret is kept");
    assert_eq!(rig.mints.len(), 1, "the old relay's token is replaced");
    assert_eq!(
        rig.host(c.id).pairing_conf(c.id),
        Some(&moshpit_install::push_conf(&phone))
    );
}

#[test]
fn a_pairing_that_cant_be_made_ready_is_never_written_to_the_host() {
    let mut rig = Rig::new(); // no device token
    let c = hooked(&mut rig, "kept");
    let mut unminted = ready_pairing(c.id);
    unminted.send_token = String::new();
    rig.store.0.lock().unwrap().pairings.push(unminted);
    rig.tend(&c);
    assert_eq!(rig.registered, 1, "the OS was asked for a token");
    rig.run_out_the_clock();
    assert!(!rig.machine.is_tending(c.id));
    assert!(rig.host(c.id).pairing_conf(c.id).is_none());
}

#[test]
fn a_first_pair_waits_for_the_token_ios_is_about_to_hand_over() {
    let mut rig = Rig::new(); // no token yet
    let c = hooked(&mut rig, "kept");
    rig.tend(&c);
    assert!(rig.machine.is_tending(c.id), "waiting for the token");
    rig.advance(1_500);
    rig.act(|m, k| m.device_token(DEVICE_TOKEN.into(), k));
    assert!(!rig.machine.is_tending(c.id));
    assert!(rig.host(c.id).pairing_conf(c.id).is_some());
}

#[test]
fn notifications_off_tends_scripts_but_pairs_nothing() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    rig.act(|m, k| {
        m.set_settings(
            Settings {
                notifications: false,
                live_activity: true,
                relay_url: RELAY.into(),
                declined: vec![],
                unpaired: vec![],
            },
            k,
        )
    });
    rig.tend(&c);
    assert!(!rig.host(c.id).commands.is_empty());
    assert!(rig.mints.is_empty());
    assert!(rig.store.read().is_empty());
}

#[test]
fn with_notifications_and_the_island_off_care_does_nothing() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    rig.act(|m, k| {
        m.set_settings(
            Settings {
                notifications: false,
                live_activity: false,
                relay_url: RELAY.into(),
                declined: vec![],
                unpaired: vec![],
            },
            k,
        )
    });
    rig.tend(&c);
    assert!(rig.host(c.id).commands.is_empty());
}

#[test]
fn a_harness_that_turned_care_off_gets_none() {
    let mut rig = Rig::with_config(Config {
        push: PushSupport::Apns {
            environment: "production".into(),
        },
        autocare_off: true,
        budgets: Default::default(),
    })
    .with_token();
    let c = hooked(&mut rig, "kept");
    rig.tend(&c);
    assert!(rig.host(c.id).commands.is_empty());
}

#[test]
fn every_care_command_is_one_the_installer_would_send() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    rig.tend(&c);
    let host = rig.host(c.id);
    let preflight = moshpit_install::commands::PREFLIGHT;
    assert_eq!(host.commands[0], preflight);
    assert!(host.commands[1].starts_with("cat \"$HOME/.moshpit/manifest.json\""));
    assert!(
        host.commands
            .iter()
            .all(|c| !c.contains('\n') || c == preflight),
        "one line each"
    );
    let _ = (
        HookAgent::ALL.len(),
        scripts::digest(&InstallComponent::Stamp),
    );
}

/// Fix (relay-setting-normalised): a setting with a trailing slash is the
/// relay a pairing at the same address was minted at — not a correction to
/// make, with a re-mint at `<relay>//v1/mint`.
#[test]
fn a_relay_setting_with_a_trailing_slash_is_the_same_relay() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    let phone = ready_pairing(c.id);
    rig.store.0.lock().unwrap().pairings.push(phone.clone());
    rig.act(|m, k| {
        m.set_settings(
            Settings {
                relay_url: format!("{RELAY}/ "),
                ..Settings::default()
            },
            k,
        )
    });
    rig.tend(&c);
    assert!(rig.mints.is_empty(), "re-minted at the same relay");
    assert_eq!(rig.pairing(c.id).unwrap().relay_url, RELAY);
    assert_eq!(
        rig.host(c.id).pairing_conf(c.id),
        Some(&moshpit_install::push_conf(&phone))
    );
}

// MARK: - What was changed on the host by hand (B315)

fn conf_path(c: &moshpit_hostcare::Conn) -> String {
    InstallComponent::Pairing {
        conn: c.id.to_string(),
        install: None,
    }
    .path()
    .unwrap()
}

/// A paired, hooked host, then a session later.
fn paired_then_again(rig: &mut Rig, c: &moshpit_hostcare::Conn, change: impl FnOnce(&mut Host)) {
    rig.tend(c);
    assert!(rig.host(c.id).pairing_conf(c.id).is_some(), "paired");
    change(rig.host(c.id));
    rig.act(|m, k| m.forget(c.id, k));
    rig.tend(c);
}

/// B315: a push conf deleted by hand left the manifest saying it was
/// there, so care never wrote it again and pushes stopped without a word.
#[test]
fn a_push_conf_deleted_by_hand_on_the_host_is_written_again() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    paired_then_again(&mut rig, &c, |host| {
        host.files.remove(&conf_path(&c));
    });
    let phone = rig.pairing(c.id).unwrap();
    assert_eq!(
        rig.host(c.id).pairing_conf(c.id),
        Some(&moshpit_install::push_conf(&phone)),
        "the conf deleted by hand was never written again"
    );
    assert_eq!(rig.mints.len(), 1, "the phone's pairing was still good");
}

#[test]
fn a_push_conf_edited_by_hand_is_written_again() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    paired_then_again(&mut rig, &c, |host| {
        let conf = host.files.get_mut(&conf_path(&c)).unwrap();
        *conf = conf.replace(RELAY, "https://elsewhere.example.org");
    });
    let phone = rig.pairing(c.id).unwrap();
    assert_eq!(
        rig.host(c.id).pairing_conf(c.id),
        Some(&moshpit_install::push_conf(&phone))
    );
}

#[test]
fn a_script_deleted_by_hand_is_installed_again() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    let sender = InstallComponent::Sender;
    paired_then_again(&mut rig, &c, |host| {
        host.files.remove(&sender.path().unwrap());
    });
    assert_eq!(
        rig.host(c.id)
            .file(&sender.path().unwrap())
            .map(String::as_str),
        scripts::body(&sender),
        "the sender deleted by hand was never installed again"
    );
}

#[test]
fn a_stamp_script_edited_by_hand_is_installed_again() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    let stamp = InstallComponent::Stamp;
    paired_then_again(&mut rig, &c, |host| {
        host.files
            .insert(stamp.path().unwrap(), "#!/bin/sh\nexit 0\n".into());
    });
    assert_eq!(
        rig.host(c.id)
            .file(&stamp.path().unwrap())
            .map(String::as_str),
        scripts::body(&stamp)
    );
}

/// Hooks taken out of the agent's config by hand are the person's call:
/// care doesn't put them back, and asks again as it asks on a host with
/// none.
#[test]
fn hooks_taken_out_by_hand_are_not_put_back_and_care_asks_again() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    paired_then_again(&mut rig, &c, |host| {
        host.files.insert(
            "$HOME/.claude/settings.json".into(),
            "{\"model\":\"opus\"}".into(),
        );
    });
    assert_eq!(
        rig.host(c.id)
            .file("$HOME/.claude/settings.json")
            .map(String::as_str),
        Some("{\"model\":\"opus\"}"),
        "care put back hooks the person took out"
    );
    assert_eq!(rig.machine.consent(), Some(c.clone()), "nobody was asked");
}

/// Edits elsewhere in the agent's config leave Moshpit's hooks there:
/// nothing to do.
#[test]
fn an_agent_config_edited_elsewhere_is_left_alone() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "kept");
    let path = "$HOME/.claude/settings.json".to_owned();
    paired_then_again(&mut rig, &c, |host| {
        let config = host.files.get_mut(&path).unwrap();
        *config = config.replacen('{', "{\"model\":\"opus\",", 1);
    });
    let writes = rig.host(c.id).written_paths();
    assert!(
        !writes
            .iter()
            .skip_while(|p| !p.contains("push.d"))
            .skip(1)
            .any(|p| *p == path),
        "care re-merged a config whose hooks were all there: {writes:?}"
    );
    assert_eq!(rig.machine.consent(), None);
}

/// A command the host never answers (a shell hung on a dead mount, a link
/// the session hasn't noticed is gone) held care's pass, and with it the
/// host's pairing lane, for the rest of the app run: Host Setup's Pair,
/// queued behind it, sat on "Minting a send token…" for good. Each command
/// has a deadline now. Past it the command has failed: the pass ends, the
/// lane frees, the screen's step runs (with a deadline of its own) and
/// ends in an error the person can retry from; a late answer changes
/// nothing.
#[test]
fn a_command_the_host_never_answers_fails_and_frees_the_hosts_pairing_lane() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.save(&[c.id]);
    rig.set_host(c.id, Host::hooked());
    rig.host(c.id).hold_on = Some("push.d/".into());
    rig.tend(&c);
    assert!(rig.machine.is_tending(c.id), "writing the conf, unanswered");
    let page = rig.open(&c, None);
    rig.act(|m, k| m.setup_pair(page, k));
    assert_eq!(
        rig.view(page).phase,
        Phase::Working(Note::MintingSendToken),
        "queued behind care's pass"
    );

    rig.advance(60_000);
    assert!(
        !rig.machine.is_tending(c.id),
        "care's pass gave up on the host"
    );
    assert!(
        matches!(
            rig.view(page).phase,
            Phase::Working(Note::InstallingOn { .. })
        ),
        "the lane freed: {:?}",
        rig.view(page).phase
    );
    rig.advance(10 * 60_000);
    let view = rig.view(page);
    assert_eq!(view.phase, Phase::Idle, "the screen's own step gave up too");
    assert_eq!(
        view.error,
        Some(Note::HostNotAnswering),
        "one to retry from"
    );

    // The host answers again: the late answers change nothing, and Pair works.
    rig.let_go(c.id);
    rig.act(|m, k| m.setup_pair(page, k));
    assert_eq!(rig.view(page).error, None);
    let phone = rig.pairing(c.id).unwrap();
    assert_eq!(
        rig.host(c.id).pairing_conf(c.id),
        Some(&moshpit_install::push_conf(&phone))
    );
}

// MARK: - Pairings this phone let go of (B354)

const OTHER_PHONE: &str = "1212121212121212121212121212121212121212121212121212121212121212";

/// Put `pairing`'s conf on `connection`'s host, recorded in its manifest,
/// as the installer leaves one.
fn conf_on_host(rig: &mut Rig, connection: Uuid, pairing: &PushPairing) {
    let host = rig.host(connection);
    let component = InstallComponent::Pairing {
        conn: pairing.connection_id.to_string(),
        install: None,
    };
    let conf = moshpit_install::push_conf(pairing);
    let mut manifest = host.manifest();
    manifest.set(
        &component,
        Some(Component {
            digest: Some(scripts::content_digest(&conf)),
            connection_id: Some(pairing.connection_id.to_string()),
            relay_url: Some(RELAY.into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    host.files.insert(component.path().unwrap(), conf);
    host.files
        .insert(InstallManifest::PATH.into(), manifest.to_json());
}

/// Another phone's pairing of `connection`.
fn other_phones(connection: Uuid) -> PushPairing {
    let now = Date::from_unix(1_762_800_000.0);
    PushPairing::make(connection, "m1-pro", RELAY, now).minted(
        &"ef".repeat(32),
        now,
        OTHER_PHONE,
        "production",
    )
}

/// A phone with two connections to one host, both paired there; the
/// second is deleted on the phone.
fn one_of_two_deleted(rig: &mut Rig) -> (moshpit_hostcare::Conn, Uuid, Uuid) {
    let kept = hooked(rig, "m1-pro");
    let deleted = conn("m1-pro, the old one");
    let elsewhere = Uuid::new_v4();
    rig.save(&[kept.id, deleted.id]);
    for c in [kept.id, deleted.id] {
        let pairing = ready_pairing(c);
        rig.store.0.lock().unwrap().pairings.push(pairing.clone());
        conf_on_host(rig, kept.id, &pairing);
    }
    conf_on_host(rig, kept.id, &other_phones(elsewhere));
    rig.act(|m, k| m.deletion_began(deleted.id, k).unwrap());
    rig.save(&[kept.id]);
    rig.act(|m, k| m.deletion_ended(deleted.id, k));
    (kept, deleted.id, elsewhere)
}

fn conf_of(connection: Uuid) -> String {
    InstallComponent::Pairing {
        conn: connection.to_string(),
        install: None,
    }
    .path()
    .unwrap()
}

/// B354: deleting a connection forgot its pairing on the phone and left
/// its conf on the host, where the relay can't revoke its token: for 45
/// days that host's pushes came sealed with a secret the phone no longer
/// had, as the fallback card — and beside a connection still kept to the
/// host, the one push per phone (B313) went out on the forgotten conf
/// first and the phone never saw a real card from the host again. The next
/// pass on that host takes it off; another phone's stays.
#[test]
fn a_deleted_connections_pairing_comes_off_its_host_at_the_next_pass_there() {
    let mut rig = Rig::new().with_token();
    let (kept, deleted, elsewhere) = one_of_two_deleted(&mut rig);
    rig.tend(&kept);
    let host = rig.host(kept.id);
    assert!(
        host.file(&conf_of(deleted)).is_none(),
        "the deleted connection's conf is still on the host"
    );
    let manifest = host.manifest();
    assert!(manifest.pairing_entry(&deleted.to_string()).is_none());
    assert!(host.file(&conf_of(kept.id)).is_some(), "the kept one stays");
    assert!(manifest.pairing_entry(&kept.id.to_string()).is_some());
    assert!(
        host.file(&conf_of(elsewhere)).is_some(),
        "another phone's pairing is that phone's"
    );
    assert!(manifest.pairing_entry(&elsewhere.to_string()).is_some());
    assert!(rig.mints.is_empty());
}

/// The same when the pass also pairs the kept connection for the first
/// time.
#[test]
fn a_first_pairing_also_takes_off_what_this_phone_let_go_of() {
    let mut rig = Rig::new().with_token();
    let (kept, deleted, _) = one_of_two_deleted(&mut rig);
    rig.store
        .0
        .lock()
        .unwrap()
        .pairings
        .retain(|p| p.connection_id != kept.id);
    rig.host(kept.id).files.remove(&conf_of(kept.id));
    rig.tend(&kept);
    assert_eq!(rig.mints, [kept.id.to_string()]);
    let host = rig.host(kept.id);
    assert!(host.file(&conf_of(kept.id)).is_some());
    assert!(host.file(&conf_of(deleted)).is_none());
    assert!(
        host.manifest()
            .pairing_entry(&kept.id.to_string())
            .is_some()
    );
}

/// A conf of this phone's for a connection still saved here stays, paired
/// on the phone or not (the store may hold it undecoded); and while the
/// phone can't tell every connection it saved (before the first unlock),
/// nothing is taken off.
#[test]
fn nothing_is_taken_off_for_a_connection_still_saved_or_while_that_cant_be_told() {
    let mut rig = Rig::new().with_token();
    let kept = hooked(&mut rig, "m1-pro");
    let saved = Uuid::new_v4();
    let gone = Uuid::new_v4();
    rig.store
        .0
        .lock()
        .unwrap()
        .pairings
        .push(ready_pairing(kept.id));
    for c in [kept.id, saved, gone] {
        conf_on_host(&mut rig, kept.id, &ready_pairing(c));
    }
    // `saved` is held by the store, though not decoded by this build.
    rig.act(|m, k| m.set_saved(&[kept.id], Some(&[kept.id, saved]), k));
    rig.tend(&kept);
    let host = rig.host(kept.id);
    assert!(host.file(&conf_of(saved)).is_some());
    assert!(host.file(&conf_of(gone)).is_none(), "the positive control");

    let mut rig = Rig::new().with_token();
    let kept = hooked(&mut rig, "m1-pro");
    rig.store
        .0
        .lock()
        .unwrap()
        .pairings
        .push(ready_pairing(kept.id));
    for c in [kept.id, gone] {
        conf_on_host(&mut rig, kept.id, &ready_pairing(c));
    }
    rig.act(|m, k| m.set_saved(&[kept.id], None, k));
    rig.tend(&kept);
    assert!(rig.host(kept.id).file(&conf_of(gone)).is_some());
}

/// B385: a Factory registration in its settings.json, on a host where the
/// person has since saved a hook through droid's /hooks (so droid reads
/// `~/.factory/hooks.json` and nothing in settings.json), reads out of
/// date; care registers it again in the hooks file, beside the person's
/// hook, and takes ours out of settings.json.
#[test]
fn care_moves_factorys_hooks_to_the_file_droid_reads() {
    let mut rig = Rig::new().with_token();
    let c = conn("m1-pro");
    rig.save(&[c.id]);
    let factory = agent("factory");
    let settings = moshpit_install::hooks::merged("", &factory).unwrap();
    let theirs = "{\"Stop\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"say done\"}]}]}";
    let mut host = Host::hooked()
        .with_file("$HOME/.factory/settings.json", &settings)
        .with_file("$HOME/.factory/hooks.json", theirs);
    let mut manifest = host.manifest();
    manifest.set(
        &InstallComponent::Hooks {
            agent: "factory".into(),
        },
        Some(Component {
            config_path: Some("$HOME/.factory/settings.json".into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    host = host.with_manifest(&manifest);
    rig.set_host(c.id, host);
    rig.tend(&c);
    let hooks = rig.host(c.id).file("$HOME/.factory/hooks.json").cloned().unwrap();
    assert!(hooks.contains("moshpit-stamp.sh working factory"), "{hooks}");
    assert!(hooks.contains("say done"), "{hooks}");
    let settings = rig
        .host(c.id)
        .file("$HOME/.factory/settings.json")
        .cloned()
        .unwrap();
    assert!(!settings.contains("moshpit-stamp.sh"), "{settings}");
    assert_eq!(
        rig.host(c.id)
            .manifest()
            .get(&InstallComponent::Hooks {
                agent: "factory".into()
            })
            .and_then(|c| c.config_path.clone())
            .as_deref(),
        Some("$HOME/.factory/hooks.json")
    );
}

// MARK: - Consent for the agents a host has (B374)

/// B374: a yes installed Claude Code's hooks whether or not Claude Code
/// was on the host. It installs the agents the preflight found.
#[test]
fn a_yes_installs_the_agents_the_host_has() {
    let mut rig = Rig::new().with_token();
    let c = conn("fresh");
    rig.save(&[c.id]);
    rig.set_host(
        c.id,
        Host::default().rule(
            "printf 'home=",
            &format!(
                "{ALL_TOOLS}\nagent.claude=no\nagent.codex=yes\nagent.gemini=yes\nagent.qwen=no"
            ),
        ),
    );
    rig.tend(&c);
    rig.act(|m, k| m.grant_consent(c.clone(), k));
    let manifest = rig.host(c.id).manifest();
    let installed: Vec<&str> = HookAgent::ALL
        .iter()
        .filter(|a| {
            manifest
                .get(&InstallComponent::Hooks {
                    agent: a.id.to_owned(),
                })
                .is_some()
        })
        .map(|a| a.id)
        .collect();
    assert_eq!(installed, ["codex", "gemini"]);
    assert!(rig.host(c.id).file("$HOME/.claude/settings.json").is_none());
    assert!(rig.host(c.id).pairing_conf(c.id).is_some());
}

// MARK: - A delete takes its pairing off the host (B377)

/// B377: deleting a connection forgot its pairing on the phone only; the
/// host went on pushing with it (until B354's next pass there, if any).
/// While the session is still up its conf comes off the host too.
#[test]
fn deleting_a_connection_while_its_session_is_up_takes_its_pairing_off_the_host() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "doomed");
    rig.tend(&c);
    assert!(rig.host(c.id).pairing_conf(c.id).is_some(), "paired");
    rig.act(|m, k| m.deletion_began(c.id, k).unwrap());
    assert!(rig.pairing(c.id).is_none());
    assert!(
        rig.host(c.id).pairing_conf(c.id).is_none(),
        "the deleted connection's conf stayed on the host"
    );
    assert!(
        rig.host(c.id)
            .manifest()
            .pairing_entry(&c.id.to_string())
            .is_none()
    );
    assert!(!rig.machine.is_forgetting(c.id));
    rig.save(&[]);
    rig.act(|m, k| m.deletion_ended(c.id, k));
}

/// A host that doesn't answer holds the delete up only for the budget: the
/// pairing stays there (until a pass finds it) and the delete goes on.
#[test]
fn a_host_that_doesnt_answer_holds_a_delete_up_only_for_a_moment() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "doomed");
    rig.tend(&c);
    rig.host(c.id).holding = true;
    rig.act(|m, k| m.deletion_began(c.id, k).unwrap());
    assert!(rig.machine.is_forgetting(c.id));
    rig.advance(3_000);
    assert!(
        !rig.machine.is_forgetting(c.id),
        "still waiting on the host"
    );
    assert!(rig.pairing(c.id).is_none());
}

/// A connection never paired here sends its host nothing on delete.
#[test]
fn a_connection_never_paired_sends_its_host_nothing_on_delete() {
    let mut rig = Rig::new().with_token();
    let c = conn("never");
    rig.save(&[c.id]);
    rig.act(|m, k| m.deletion_began(c.id, k).unwrap());
    assert!(rig.host(c.id).commands.is_empty());
    assert!(!rig.machine.is_forgetting(c.id));
}

// MARK: - One conf per install (B443)

/// `pairing`'s conf on `connection`'s host as install `install` leaves it
/// (`None`: the legacy `<conn>.conf`).
fn conf_on_host_as(rig: &mut Rig, connection: Uuid, pairing: &PushPairing, install: Option<&str>) {
    let host = rig.host(connection);
    let component = InstallComponent::Pairing {
        conn: pairing.connection_id.to_string(),
        install: install.map(str::to_owned),
    };
    let conf = moshpit_install::push_conf(pairing);
    let mut manifest = host.manifest();
    manifest.set(
        &component,
        Some(Component {
            digest: Some(scripts::content_digest(&conf)),
            connection_id: Some(pairing.connection_id.to_string()),
            relay_url: Some(RELAY.into()),
            ..Component::new(Date::from_unix(1_762_800_000.0))
        }),
    );
    host.files.insert(component.path().unwrap(), conf);
    host.files
        .insert(InstallManifest::PATH.into(), manifest.to_json());
}

fn install_conf_path(connection: Uuid, install: Option<&str>) -> String {
    InstallComponent::Pairing {
        conn: connection.to_string(),
        install: install.map(str::to_owned),
    }
    .path()
    .unwrap()
}

const MINE: &str = "aaaaaaaa0000aaaa";
const THEIRS: &str = "bbbbbbbb0000bbbb";

/// B443: a phone restored from another's backup has the same connection,
/// id and all. Both wrote `<conn>.conf`, each pass wrote over the other's,
/// and the host pushed to whichever phone connected last. Each install
/// now keeps a conf of its own, and a pass of one never touches the
/// other's.
#[test]
fn a_phone_restored_from_anothers_backup_keeps_a_conf_of_its_own() {
    let mut rig = Rig::new().with_token();
    rig.act(|m, k| m.install_id(MINE.into(), k));
    let c = hooked(&mut rig, "m1-pro");
    let theirs = other_phones(c.id);
    conf_on_host_as(&mut rig, c.id, &theirs, Some(THEIRS));
    rig.tend(&c);
    let mine = rig.pairing(c.id).expect("paired");
    let host = rig.host(c.id);
    assert_eq!(
        host.file(&install_conf_path(c.id, Some(MINE))),
        Some(&moshpit_install::push_conf(&mine)),
        "this phone's own conf"
    );
    assert_eq!(
        host.file(&install_conf_path(c.id, Some(THEIRS))),
        Some(&moshpit_install::push_conf(&theirs)),
        "the other phone's was touched"
    );
    assert!(host.file(&install_conf_path(c.id, None)).is_none(), "a legacy conf was written");
    let manifest = host.manifest();
    assert!(manifest.pairing_entry_of(&c.id.to_string(), Some(MINE)).is_some());
    assert!(manifest.pairing_entry_of(&c.id.to_string(), Some(THEIRS)).is_some());

    // The next pass finds its own conf current, and writes nothing.
    let writes = rig.host(c.id).written_paths().len();
    rig.act(|m, k| m.forget(c.id, k));
    rig.tend(&c);
    assert_eq!(rig.host(c.id).written_paths().len(), writes, "rewrote a conf");
}

/// B443: the legacy conf this phone wrote before installs were told apart
/// moves to its install's — only when its digest is this phone's conf. A
/// legacy conf with another digest (the other phone's) is left as it is.
#[test]
fn this_phones_legacy_conf_moves_to_its_install_and_anothers_stays() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "m1-pro");
    let mine = ready_pairing(c.id);
    rig.store.0.lock().unwrap().pairings.push(mine.clone());
    conf_on_host_as(&mut rig, c.id, &mine, None);
    rig.act(|m, k| m.install_id(MINE.into(), k));
    rig.tend(&c);
    let host = rig.host(c.id);
    assert_eq!(host.file(&install_conf_path(c.id, Some(MINE))), Some(&moshpit_install::push_conf(&mine)));
    assert!(host.file(&install_conf_path(c.id, None)).is_none(), "the legacy conf stayed");
    assert!(host.manifest().pairing_entry(&c.id.to_string()).is_none(), "its key stayed");

    // Another phone's legacy conf of the same connection.
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "m1-pro");
    let mine = ready_pairing(c.id);
    rig.store.0.lock().unwrap().pairings.push(mine.clone());
    let theirs = other_phones(c.id);
    conf_on_host_as(&mut rig, c.id, &theirs, None);
    rig.act(|m, k| m.install_id(MINE.into(), k));
    rig.tend(&c);
    let host = rig.host(c.id);
    assert_eq!(host.file(&install_conf_path(c.id, Some(MINE))), Some(&moshpit_install::push_conf(&mine)));
    assert_eq!(
        host.file(&install_conf_path(c.id, None)),
        Some(&moshpit_install::push_conf(&theirs)),
        "another phone's legacy conf was touched"
    );
    assert!(host.manifest().pairing_entry(&c.id.to_string()).is_some());
}

/// B443 with B354: a conf of another install that pushes to this very
/// phone is an earlier install of the app here (its secret went with it):
/// it goes. Another phone's install stays.
#[test]
fn an_earlier_install_on_this_phone_leaves_its_conf_to_this_one() {
    let mut rig = Rig::new().with_token();
    rig.act(|m, k| m.install_id(MINE.into(), k));
    let c = hooked(&mut rig, "m1-pro");
    let earlier = ready_pairing(c.id);
    conf_on_host_as(&mut rig, c.id, &earlier, Some("cccccccc0000cccc"));
    conf_on_host_as(&mut rig, c.id, &other_phones(c.id), Some(THEIRS));
    rig.tend(&c);
    let host = rig.host(c.id);
    assert!(host.file(&install_conf_path(c.id, Some("cccccccc0000cccc"))).is_none(), "the earlier install's conf stayed");
    assert!(host.file(&install_conf_path(c.id, Some(THEIRS))).is_some(), "another phone's conf went");
    assert!(host.file(&install_conf_path(c.id, Some(MINE))).is_some());
}

/// B443: a legacy conf of this phone's that the move over didn't take —
/// minted again since, so its digest is an older conf — still pushes to
/// this phone's APNs token, which only this phone has (a restored phone is
/// handed a new one): it goes once the install has its own. Another
/// phone's legacy conf pushes to another token and stays.
#[test]
fn this_phones_older_legacy_conf_goes_by_its_token() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "m1-pro");
    let mine = ready_pairing(c.id);
    rig.store.0.lock().unwrap().pairings.push(mine.clone());
    let mut older = mine.clone();
    older.send_token = "99".repeat(32);
    conf_on_host_as(&mut rig, c.id, &older, None);
    rig.act(|m, k| m.install_id(MINE.into(), k));
    rig.tend(&c);
    let host = rig.host(c.id);
    assert_eq!(host.file(&install_conf_path(c.id, Some(MINE))), Some(&moshpit_install::push_conf(&mine)));
    assert!(host.file(&install_conf_path(c.id, None)).is_none(), "this phone's older legacy conf stayed");
    assert!(host.manifest().pairing_entry(&c.id.to_string()).is_none());
}

/// B451: a rename reached the pairing's name — and pushed cards — only on
/// care's next pass, once a run. Saving the connection names it at once;
/// the host isn't touched, a blank name and an unpaired connection change
/// nothing.
#[test]
fn a_saved_rename_names_the_pairing_at_once() {
    let mut rig = Rig::new().with_token();
    let c = hooked(&mut rig, "old name");
    rig.tend(&c);
    let commands = rig.host(c.id).commands.len();
    rig.act(|m, k| m.name_connection(c.id, "studio", k));
    assert_eq!(rig.pairing(c.id).unwrap().host_label, "studio");
    assert_eq!(rig.host(c.id).commands.len(), commands, "the host was asked");
    rig.act(|m, k| m.name_connection(c.id, "  ", k));
    assert_eq!(rig.pairing(c.id).unwrap().host_label, "studio");
    let unpaired = Uuid::new_v4();
    rig.act(|m, k| m.name_connection(unpaired, "elsewhere", k));
    assert!(rig.pairing(unpaired).is_none());
}
