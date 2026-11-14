//! `HostInstallerTests` and `HostAutoCareTests`, against a simulated host:
//! it records every command and keeps the files it was told to write, so a
//! write followed by a verify agrees and a tampered verify does not.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use moshpit_install::installer::{
    self, Action, Blocked, ChannelError, ComponentStatus, HostChannel, HostInstaller, InstallState,
    StampProof,
};
use moshpit_install::manifest::{Component, InstallComponent, InstallManifest};
use moshpit_install::{HookAgent, HostFacts, commands, hooks, scripts};
use moshpit_seal::foundation::base64;
use moshpit_seal::{Date, PushPairing, Uuid};

/// The fake host never makes anyone wait, so one poll finishes any call.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the fake host never makes anyone wait"),
    }
}

#[derive(Default)]
struct RecordingChannel {
    commands: RefCell<Vec<String>>,
    files: RefCell<BTreeMap<String, String>>,
    /// First match wins; consulted before the simulated filesystem.
    rules: Vec<(String, String)>,
    fail_on: Option<String>,
    digests_answer_empty: bool,
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)? + start.len();
    let to = text[from..].find(end)? + from;
    Some(&text[from..to])
}

impl RecordingChannel {
    fn with_rules(rules: &[(&str, &str)]) -> Self {
        Self {
            rules: rules
                .iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect(),
            ..Self::default()
        }
    }

    fn healthy() -> Self {
        Self::with_rules(&[("printf 'home=", ALL_TOOLS)])
    }

    fn commands(&self) -> Vec<String> {
        self.commands.borrow().clone()
    }

    fn write_target(command: &str) -> Option<String> {
        commands::written(command).map(|(path, _)| path)
    }

    fn written_paths(&self) -> Vec<String> {
        self.commands()
            .iter()
            .filter_map(|c| Self::write_target(c))
            .collect()
    }

    fn file(&self, path: &str) -> Option<String> {
        self.files.borrow().get(path).cloned()
    }

    fn sent(&self, needle: &str) -> Option<String> {
        self.commands().into_iter().find(|c| c.contains(needle))
    }

    /// The write whose target ends with `suffix`.
    fn write_to(&self, suffix: &str) -> Option<String> {
        self.commands()
            .into_iter()
            .find(|c| Self::write_target(c).is_some_and(|p| p.ends_with(suffix)))
    }

    fn simulate(&self, command: &str) -> String {
        if let (Some(path), Some(payload)) = (
            Self::write_target(command),
            between(command, "printf %s '", "' | base64 -d"),
        ) && let Some(text) = base64::decode(payload).and_then(|b| String::from_utf8(b).ok())
        {
            self.files.borrow_mut().insert(path, text);
            return String::new();
        }
        if let Some(rest) = command.strip_prefix("cat \"") {
            let path: String = rest.chars().take_while(|&c| c != '"').collect();
            return self.files.borrow().get(&path).cloned().unwrap_or_default();
        }
        if command.contains("awk '{print $1}'") {
            if self.digests_answer_empty {
                return String::new();
            }
            let Some(path) = between(command, "sha256sum \"", "\"") else {
                return String::new();
            };
            return self
                .files
                .borrow()
                .get(path)
                .map(|body| scripts::content_digest(body))
                .unwrap_or_default();
        }
        if let Some(rest) = command.strip_prefix("rm -f \"") {
            let path: String = rest.chars().take_while(|&c| c != '"').collect();
            self.files.borrow_mut().remove(&path);
        }
        String::new()
    }
}

impl HostChannel for RecordingChannel {
    fn run(&self, command: &str) -> impl Future<Output = Result<Vec<u8>, ChannelError>> {
        self.commands.borrow_mut().push(command.to_owned());
        let result = if self
            .fail_on
            .as_deref()
            .is_some_and(|needle| command.contains(needle))
        {
            Err(ChannelError("test".into()))
        } else if let Some((_, reply)) = self
            .rules
            .iter()
            .find(|(needle, _)| command.contains(needle.as_str()))
        {
            Ok(reply.clone().into_bytes())
        } else {
            Ok(self.simulate(command).into_bytes())
        };
        async move { result }
    }
}

const ALL_TOOLS: &str = "home=/Users/dev\nshell=/bin/zsh\nuname=Darwin\nopenssl=yes\ncurl=yes\njq=yes\ntmux=yes\npython3=yes\nbase64=yes";

fn now() -> Date {
    Date::from_unix(1_762_800_000.0)
}

fn pairing() -> PushPairing {
    PushPairing {
        connection_id: Uuid::parse("3F2504E0-4F89-11D3-9A0C-0305E82C3301").unwrap(),
        host_label: "m1-pro".into(),
        secret_hex: "ab".repeat(32),
        send_token: "cd".repeat(32),
        relay_url: "https://push.example.org".into(),
        created_at: Date::from_unix(1_755_900_000.0),
        apns_token: Some("ef".repeat(32)),
        apns_env: Some("production".into()),
        send_token_issued_at: Some(Date::from_unix(1_756_000_000.0)),
    }
}

fn pairing_component() -> InstallComponent {
    InstallComponent::Pairing {
        conn: "3F2504E0-4F89-11D3-9A0C-0305E82C3301".into(),
        install: None,
    }
}

fn state(manifest: InstallManifest) -> InstallState {
    InstallState {
        facts: HostFacts::parse(ALL_TOOLS),
        manifest,
    }
}

// MARK: - Preflight

#[test]
fn preflight_is_one_round_trip_and_parses_every_fact() {
    let channel = RecordingChannel::healthy();
    let facts = block_on(HostInstaller::new(&channel).preflight()).unwrap();
    assert_eq!(
        channel.commands().len(),
        1,
        "preflight must not cost one exec per tool"
    );
    assert_eq!(facts.home, "/Users/dev");
    assert_eq!(facts.uname, "Darwin");
    assert!(facts.can_push());
    assert!(facts.has_jq());
    assert!(facts.missing_for_push().is_empty());
}

#[test]
fn a_host_missing_openssl_is_refused_before_anything_is_written() {
    let channel = RecordingChannel::with_rules(&[(
        "printf 'home=",
        "home=/home/x\ncurl=yes\nbase64=yes\nopenssl=no",
    )]);
    let installer = HostInstaller::new(&channel);
    let state = block_on(installer.inspect()).unwrap();
    assert!(!state.facts.can_push());
    assert_eq!(state.facts.missing_for_push(), ["openssl"]);
    let report = block_on(installer.install_pairing(&pairing(), &state, now())).unwrap();
    assert_eq!(
        report.blocked,
        Some(Blocked::MissingTools(vec!["openssl".into()]))
    );
    assert!(report.steps.is_empty());
    assert_eq!(channel.file(&pairing_component().path().unwrap()), None);
    assert!(channel.written_paths().is_empty());
}

// MARK: - Writing

#[test]
fn pairing_lands_the_sender_the_stamp_script_and_this_devices_conf() {
    let channel = RecordingChannel::healthy();
    let installer = HostInstaller::new(&channel);
    let report = block_on(installer.install_pairing(
        &pairing(),
        &block_on(installer.inspect()).unwrap(),
        now(),
    ))
    .unwrap();
    assert!(report.succeeded(), "{:?}", report.failures());
    assert_eq!(
        channel.file(&pairing_component().path().unwrap()),
        Some(installer::push_conf(&pairing()))
    );
    let installed: Vec<InstallComponent> = report
        .steps
        .iter()
        .filter(|s| matches!(s.action, Action::Installed { .. }))
        .map(|s| s.component.clone())
        .collect();
    assert!(installed.contains(&InstallComponent::Sender));
    assert!(installed.contains(&pairing_component()));
    assert!(installed.contains(&InstallComponent::Stamp));
}

#[test]
fn the_secret_rides_as_base64_so_no_quote_can_escape_into_the_shell() {
    let channel = RecordingChannel::healthy();
    let installer = HostInstaller::new(&channel);
    block_on(installer.install_pairing(&pairing(), &block_on(installer.inspect()).unwrap(), now()))
        .unwrap();
    let write = channel.write_to(".conf").unwrap();
    assert!(!write.contains(&pairing().secret_hex));
    assert!(!write.contains(&pairing().send_token));
    assert!(write.contains(&base64::encode(installer::push_conf(&pairing()).as_bytes())));
    assert!(write.find("umask 077").unwrap() < write.find("base64 -d >").unwrap());
    assert!(write.contains("chmod 600"));
}

#[test]
fn the_sender_lands_before_the_secrets_it_will_read() {
    let channel = RecordingChannel::healthy();
    let installer = HostInstaller::new(&channel);
    block_on(installer.install_pairing(&pairing(), &block_on(installer.inspect()).unwrap(), now()))
        .unwrap();
    let commands = channel.commands();
    let sender = commands
        .iter()
        .position(|c| {
            RecordingChannel::write_target(c).is_some_and(|p| p.ends_with("moshpit-push.sh"))
        })
        .unwrap();
    let conf = commands
        .iter()
        .position(|c| RecordingChannel::write_target(c).is_some_and(|p| p.ends_with(".conf")))
        .unwrap();
    assert!(sender < conf);
}

#[test]
fn push_conf_is_exactly_the_seven_lines_the_sender_parses() {
    assert_eq!(
        installer::push_conf(&pairing()),
        format!(
            "RELAY_URL=https://push.example.org\nSEND_TOKEN={}\nSECRET={}\nCONN=3F2504E0-4F89-11D3-9A0C-0305E82C3301\n\
             APNS_TOKEN={}\nAPNS_ENV=production\nSEND_IAT=1756000000\n",
            "cd".repeat(32),
            "ab".repeat(32),
            "ef".repeat(32)
        )
    );
    // A pairing missing its routing facts writes the empty fields the sender
    // skips; the date's fraction is dropped.
    let mut bare = pairing();
    bare.apns_token = None;
    bare.apns_env = None;
    bare.send_token_issued_at = None;
    assert!(
        installer::push_conf(&bare).ends_with("APNS_TOKEN=\nAPNS_ENV=production\nSEND_IAT=0\n")
    );
    bare.send_token_issued_at = Some(Date::from_unix(1_756_000_000.9));
    assert!(installer::push_conf(&bare).ends_with("SEND_IAT=1756000000\n"));
    bare.send_token_issued_at = Some(Date::from_unix(-0.5));
    assert!(installer::push_conf(&bare).ends_with("SEND_IAT=0\n"));
}

/// `PushPairingTests.noShellPathRemains`.
#[test]
fn nothing_here_can_put_a_secret_into_a_shell_command() {
    let fresh = PushPairing::make(Uuid::new_v4(), "m1-pro", "https://push.example.org", now());
    let conf = installer::push_conf(&fresh);
    assert!(conf.contains(&fresh.secret_hex));
    assert!(!conf.contains("sh -c"));
    assert!(!conf.contains("$HOME"));
}

#[test]
fn a_failed_script_write_stops_before_the_secrets_are_written() {
    let channel = RecordingChannel::with_rules(&[
        ("printf 'home=", ALL_TOOLS),
        ("awk '{print $1}'", "wrong"),
    ]);
    let installer = HostInstaller::new(&channel);
    let report = block_on(installer.install_pairing(
        &pairing(),
        &block_on(installer.inspect()).unwrap(),
        now(),
    ))
    .unwrap();
    assert!(!report.succeeded());
    assert!(channel.write_to(".conf").is_none());
    assert!(channel.write_to("manifest.json").is_none());
}

#[test]
fn a_checksum_that_does_not_match_is_a_reported_failure() {
    let channel = RecordingChannel::with_rules(&[
        ("printf 'home=", ALL_TOOLS),
        ("awk '{print $1}'", "deadbeef"),
    ]);
    let installer = HostInstaller::new(&channel);
    let report = block_on(installer.install_pairing(
        &pairing(),
        &block_on(installer.inspect()).unwrap(),
        now(),
    ))
    .unwrap();
    assert!(!report.succeeded());
    assert!(
        report
            .failures()
            .iter()
            .any(|f| f.contains("different checksum"))
    );
    assert!(
        report.failures()[0].contains("(wanted ")
            && report.failures()[0].contains(", got deadbeef\u{2026})")
    );
}

#[test]
fn a_write_that_cannot_be_read_back_names_the_likely_cause() {
    let channel = RecordingChannel {
        digests_answer_empty: true,
        ..RecordingChannel::healthy()
    };
    let installer = HostInstaller::new(&channel);
    let report = block_on(installer.install_pairing(
        &pairing(),
        &block_on(installer.inspect()).unwrap(),
        now(),
    ))
    .unwrap();
    assert!(
        report
            .failures()
            .iter()
            .any(|f| f.contains("home directory is writable"))
    );
}

#[test]
fn a_channel_that_fails_is_an_error_not_a_report() {
    let channel = RecordingChannel {
        fail_on: Some("printf 'home=".into()),
        ..RecordingChannel::healthy()
    };
    assert!(block_on(HostInstaller::new(&channel).inspect()).is_err());
}

// MARK: - Staleness

#[test]
fn a_script_the_host_already_has_current_is_not_rewritten() {
    let mut manifest = InstallManifest::empty();
    manifest.set(
        &InstallComponent::Sender,
        Some(Component {
            digest: Some(scripts::content_digest(scripts::sender())),
            ..Component::new(now())
        }),
    );
    let state = state(manifest);
    assert_eq!(
        state.status(&InstallComponent::Sender),
        ComponentStatus::Current
    );
    let channel = RecordingChannel::healthy();
    let step =
        block_on(HostInstaller::new(&channel).install_script(&InstallComponent::Sender, &state))
            .unwrap();
    assert_eq!(step.action, Action::AlreadyCurrent);
    assert!(channel.commands().is_empty());
}

#[test]
fn a_stale_script_is_detected_by_content() {
    let mut manifest = InstallManifest::empty();
    manifest.set(
        &InstallComponent::Stamp,
        Some(Component {
            digest: Some("an-older-build".into()),
            ..Component::new(now())
        }),
    );
    let unpaired = state(manifest.clone());
    assert_eq!(
        unpaired.status(&InstallComponent::Stamp),
        ComponentStatus::Stale {
            installed: "an-older-build".into()
        }
    );
    assert!(!unpaired.paired_but_nothing_will_push());
    manifest.set(
        &InstallComponent::Pairing { conn: "x".into(), install: None },
        Some(Component {
            connection_id: Some("x".into()),
            relay_url: Some("y".into()),
            ..Component::new(now())
        }),
    );
    assert!(state(manifest).paired_but_nothing_will_push());
}

#[test]
fn hooks_report_the_worse_of_the_registration_and_the_script_they_call() {
    let with = |stamp: Option<&str>, registered: bool| {
        let mut manifest = InstallManifest::empty();
        if let Some(stamp) = stamp {
            manifest.set(
                &InstallComponent::Stamp,
                Some(Component {
                    digest: Some(stamp.into()),
                    ..Component::new(now())
                }),
            );
        }
        if registered {
            manifest.set(
                &InstallComponent::Hooks {
                    agent: "claude".into(),
                },
                Some(Component {
                    digest: Some("cfg".into()),
                    config_path: Some("$HOME/.claude/settings.json".into()),
                    ..Component::new(now())
                }),
            );
        }
        state(manifest)
    };
    let current = scripts::content_digest(scripts::stamp());
    assert_eq!(
        with(Some(&current), true).hooks_status("claude"),
        ComponentStatus::Current
    );
    assert_eq!(
        with(Some("older"), true).hooks_status("claude"),
        ComponentStatus::Stale {
            installed: "older".into()
        }
    );
    assert_eq!(
        with(None, true).hooks_status("claude"),
        ComponentStatus::Absent
    );
    assert_eq!(
        with(Some(&current), false).hooks_status("claude"),
        ComponentStatus::Absent
    );
    // A recorded script with no digest is stale, of unknown content.
    let mut manifest = InstallManifest::empty();
    manifest.set(&InstallComponent::Stamp, Some(Component::new(now())));
    assert_eq!(
        state(manifest).status(&InstallComponent::Stamp),
        ComponentStatus::Stale {
            installed: "unknown".into()
        }
    );
}

#[test]
fn an_absent_or_unreadable_manifest_reads_as_nothing_installed() {
    let channel = RecordingChannel::healthy();
    let state = block_on(HostInstaller::new(&channel).inspect()).unwrap();
    assert_eq!(state.manifest, InstallManifest::empty());
    assert_eq!(
        state.status(&InstallComponent::Stamp),
        ComponentStatus::Absent
    );
    for body in ["{not json", r#"{"schema":99,"components":{"stamp":{}}}"#] {
        let channel =
            RecordingChannel::with_rules(&[("printf 'home=", ALL_TOOLS), ("manifest.json", body)]);
        let state = block_on(HostInstaller::new(&channel).inspect()).unwrap();
        assert_eq!(state.manifest, InstallManifest::empty());
    }
}

#[test]
fn the_manifest_records_what_landed_and_never_the_secret() {
    let channel = RecordingChannel::healthy();
    let installer = HostInstaller::new(&channel);
    let report = block_on(installer.install_pairing(
        &pairing(),
        &block_on(installer.inspect()).unwrap(),
        now(),
    ))
    .unwrap();
    let manifest = report.manifest.unwrap();
    let mine = manifest.get(&pairing_component()).unwrap();
    assert_eq!(
        mine.connection_id.as_deref(),
        Some("3F2504E0-4F89-11D3-9A0C-0305E82C3301")
    );
    assert_eq!(mine.relay_url.as_deref(), Some("https://push.example.org"));
    assert_eq!(
        manifest.get(&InstallComponent::Sender).unwrap().digest,
        Some(scripts::content_digest(scripts::sender()))
    );
    let written = channel.write_to("manifest.json").unwrap();
    let decoded = String::from_utf8(
        base64::decode(between(&written, "printf %s '", "' | base64 -d").unwrap()).unwrap(),
    )
    .unwrap();
    assert!(!decoded.contains(&pairing().secret_hex));
    assert!(!decoded.contains(&pairing().send_token));
    assert!(decoded.contains("\"schema\" : 1"));
    assert!(written.contains("chmod 644"));
    assert_eq!(decoded, manifest.to_json());
}

// MARK: - Proving and removing

#[test]
fn the_self_test_fires_the_real_sender_with_a_matchable_nonce() {
    let channel = RecordingChannel::healthy();
    let nonce = installer::make_nonce();
    block_on(HostInstaller::new(&channel).self_test_push(&nonce, "C1")).unwrap();
    let sent = channel.sent("--test").unwrap();
    assert!(sent.contains("moshpit-push.sh"));
    assert!(sent.contains(&format!("'{nonce}' 'C1'")));
    assert!(nonce.starts_with("selftest-"));
    assert_eq!(nonce.len(), "selftest-".len() + 12);
    assert_ne!(installer::make_nonce(), nonce);
}

#[test]
fn uninstall_removes_only_what_the_manifest_says_we_put_there() {
    let mut manifest = InstallManifest::empty();
    let mine = InstallComponent::Pairing { conn: "c".into(), install: None };
    let theirs = InstallComponent::Pairing {
        conn: "other-device".into(),
        install: None,
    };
    for (component, conn) in [(&mine, "c"), (&theirs, "other-device")] {
        manifest.set(
            component,
            Some(Component {
                connection_id: Some(conn.into()),
                relay_url: Some("r".into()),
                ..Component::new(now())
            }),
        );
    }
    manifest.set(
        &InstallComponent::Sender,
        Some(Component {
            digest: Some("d".into()),
            ..Component::new(now())
        }),
    );
    // The manifest the uninstall changes is the host's, read under its
    // lock (B373).
    let channel = RecordingChannel::healthy();
    channel
        .files
        .borrow_mut()
        .insert(InstallManifest::PATH.into(), manifest.to_json());
    let after = block_on(
        HostInstaller::new(&channel).uninstall(std::slice::from_ref(&mine), &state(manifest)),
    )
    .unwrap();
    assert!(after.get(&mine).is_none());
    assert!(
        after.get(&theirs).is_some(),
        "unpairing one device must never take another's pairing"
    );
    assert!(after.get(&InstallComponent::Sender).is_some());
    assert!(channel.sent("rm -f").unwrap().contains("push.d/c.conf"));
}

// MARK: - Hooks

#[test]
fn installing_hooks_reads_merges_writes_back_and_verifies() {
    let agent = HookAgent::named("claude").unwrap();
    let existing = r#"{"model":"opus"}"#;
    let merged = hooks::merged_json(existing, "claude").unwrap();
    let channel = RecordingChannel::with_rules(&[
        ("printf 'home=", ALL_TOOLS),
        ("cat \"$HOME/.claude/settings.json\"", existing),
    ]);
    let installer = HostInstaller::new(&channel);
    let report =
        block_on(installer.install_hooks(&agent, &block_on(installer.inspect()).unwrap(), now()))
            .unwrap();
    assert!(report.succeeded(), "{:?}", report.failures());
    let write = channel.write_to("settings.json").unwrap();
    assert!(write.contains(&base64::encode(merged.as_bytes())));
    assert!(
        !channel
            .commands()
            .iter()
            .any(|c| c.contains("jq --arg") || c.contains("python3 - "))
    );
    assert!(channel.sent(".moshpit.orig").is_some());
    assert!(channel.sent("moshpit.bak.").is_some());
    let manifest = report.manifest.unwrap();
    assert_eq!(
        manifest
            .get(&InstallComponent::Hooks {
                agent: "claude".into()
            })
            .unwrap()
            .config_path
            .as_deref(),
        Some("$HOME/.claude/settings.json")
    );
    // Scripts first: stamp, then sender, then the config.
    let order: Vec<InstallComponent> = report.steps.iter().map(|s| s.component.clone()).collect();
    assert_eq!(
        order,
        [
            InstallComponent::Stamp,
            InstallComponent::Sender,
            InstallComponent::Hooks {
                agent: "claude".into()
            }
        ]
    );
}

#[test]
fn installing_for_codex_reports_the_step_the_user_still_has_to_take() {
    let codex = HookAgent::named("codex").unwrap();
    let channel = RecordingChannel::with_rules(&[
        ("printf 'home=", ALL_TOOLS),
        ("cat \"$HOME/.codex/config.toml\"", "model = \"gpt-5\"\n"),
    ]);
    let installer = HostInstaller::new(&channel);
    let report =
        block_on(installer.install_hooks(&codex, &block_on(installer.inspect()).unwrap(), now()))
            .unwrap();
    assert!(report.succeeded());
    assert!(
        report
            .user_action_required
            .is_some_and(|step| step.contains("/hooks"))
    );

    let claude = HookAgent::named("claude").unwrap();
    let plain = RecordingChannel::with_rules(&[
        ("printf 'home=", ALL_TOOLS),
        ("cat \"$HOME/.claude/settings.json\"", "{}"),
    ]);
    let other = HostInstaller::new(&plain);
    let report =
        block_on(other.install_hooks(&claude, &block_on(other.inspect()).unwrap(), now())).unwrap();
    assert_eq!(report.user_action_required, None);
}

#[test]
fn a_config_that_will_not_parse_is_reported_and_nothing_is_written() {
    let agent = HookAgent::named("claude").unwrap();
    let channel = RecordingChannel::with_rules(&[
        ("printf 'home=", ALL_TOOLS),
        ("cat \"$HOME/.claude/settings.json\"", "{ this is not json"),
    ]);
    let installer = HostInstaller::new(&channel);
    let report =
        block_on(installer.install_hooks(&agent, &block_on(installer.inspect()).unwrap(), now()))
            .unwrap();
    assert!(!report.succeeded());
    assert!(report.failures().iter().any(|f| f.contains("valid JSON")));
    assert!(channel.write_to("settings.json").is_none());
    assert!(channel.sent(".moshpit.orig").is_none());
}

#[test]
fn uninstalling_hooks_edits_the_file_the_install_actually_touched() {
    let agent = HookAgent::named("claude").unwrap();
    let mut manifest = InstallManifest::empty();
    manifest.set(
        &InstallComponent::Hooks {
            agent: "claude".into(),
        },
        Some(Component {
            digest: Some("d".into()),
            config_path: Some("$HOME/dotfiles/claude.json".into()),
            ..Component::new(now())
        }),
    );
    let installed = hooks::merged_json(r#"{"model":"opus"}"#, "claude").unwrap();
    let channel =
        RecordingChannel::with_rules(&[("cat \"$HOME/dotfiles/claude.json\"", &installed)]);
    let report =
        block_on(HostInstaller::new(&channel).uninstall_hooks(&agent, &state(manifest))).unwrap();
    assert!(
        report
            .manifest
            .unwrap()
            .get(&InstallComponent::Hooks {
                agent: "claude".into()
            })
            .is_none()
    );
    let write = channel.write_to("claude.json").unwrap();
    let restored = String::from_utf8(
        base64::decode(between(&write, "printf %s '", "' | base64 -d").unwrap()).unwrap(),
    )
    .unwrap();
    assert!(!restored.contains("moshpit-stamp.sh"));
    assert!(restored.contains("opus"));
}

#[test]
fn a_manifest_naming_an_unsafe_path_is_never_run_against() {
    let agent = HookAgent::named("claude").unwrap();
    let mut manifest = InstallManifest::empty();
    manifest.set(
        &InstallComponent::Hooks {
            agent: "claude".into(),
        },
        Some(Component {
            config_path: Some("$HOME/x\"; rm -rf ~; \"".into()),
            ..Component::new(now())
        }),
    );
    let channel = RecordingChannel::healthy();
    let report =
        block_on(HostInstaller::new(&channel).uninstall_hooks(&agent, &state(manifest))).unwrap();
    assert!(report.failures()[0].starts_with(
        "The manifest on this host names a config path Moshpit won't run a command against: "
    ));
    assert!(channel.commands().is_empty());
}

// MARK: - Proving the hook path

#[test]
fn the_stamp_self_test_fires_the_real_script_and_reads_the_pane_back() {
    let channel = RecordingChannel::with_rules(&[
        ("socket_path", "/private/tmp/tmux-501/default"),
        ("display-message -p -t", "working|moshpit-selftest|"),
    ]);
    assert_eq!(
        block_on(HostInstaller::new(&channel).self_test_stamp("%3")).unwrap(),
        StampProof::Stamped
    );
    // One command (B355): it fires, reads back and restores the pane.
    let fired = channel.sent("moshpit-stamp.sh").unwrap();
    assert!(fired.starts_with("TMUX='/private/tmp/tmux-501/default,0,0' sh -c "));
    assert!(fired.contains("MOSHPIT_SELFTEST=1 TMUX_PANE=$p sh "));
    assert!(fired.contains(" working moshpit-selftest < /dev/null"));
    assert!(fired.ends_with(" sh '%3' 2>/dev/null || true"));
    assert!(fired.contains("display-message -p -t"));
    assert!(fired.contains("r state \"$s\""));
    assert_eq!(channel.commands().len(), 2, "the socket, then the test");
}

#[test]
fn a_host_whose_tmux_is_elsewhere_reports_unavailable() {
    let channel = RecordingChannel::default();
    assert_eq!(
        block_on(HostInstaller::new(&channel).self_test_stamp("%1")).unwrap(),
        StampProof::Unavailable
    );
    assert!(channel.sent("moshpit-stamp.sh").is_none());
}

#[test]
fn a_stamp_that_did_not_land_is_a_distinct_answer() {
    let channel = RecordingChannel::with_rules(&[
        ("socket_path", "/tmp/sock"),
        ("display-message -p -t", "||"),
    ]);
    assert_eq!(
        block_on(HostInstaller::new(&channel).self_test_stamp("%1")).unwrap(),
        StampProof::DidNotStamp("||".into())
    );
}

// MARK: - HostAutoCareTests

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
            ..Component::new(now())
        }),
    );
    let agents: Vec<&str> = installer::installed_hook_agents(&manifest)
        .iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(agents, ["claude"]);
    assert!(installer::installed_hook_agents(&InstallManifest::empty()).is_empty());
}

#[test]
fn care_refreshes_consented_agents_when_a_script_drifted() {
    let current = |component: &InstallComponent| Component {
        digest: scripts::digest(component),
        ..Component::new(now())
    };
    let mut manifest = InstallManifest::empty();
    manifest.set(
        &InstallComponent::Hooks {
            agent: "codex".into(),
        },
        Some(Component::new(now())),
    );
    manifest.set(
        &InstallComponent::Stamp,
        Some(current(&InstallComponent::Stamp)),
    );
    manifest.set(
        &InstallComponent::Sender,
        Some(current(&InstallComponent::Sender)),
    );
    assert!(
        installer::agents_to_refresh(&state(manifest.clone())).is_empty(),
        "everything current"
    );
    manifest.set(
        &InstallComponent::Sender,
        Some(Component {
            digest: Some("old".into()),
            ..Component::new(now())
        }),
    );
    let ids: Vec<&str> = installer::agents_to_refresh(&state(manifest.clone()))
        .iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(ids, ["codex"], "a stale sender alone refreshes");
}

#[test]
fn a_hosts_pairing_is_rewritten_only_when_its_bytes_would_change() {
    let mut manifest = InstallManifest::empty();
    assert!(installer::pairing_needs_rewrite(&manifest, &pairing()));
    manifest.set(
        &pairing_component(),
        Some(Component {
            digest: Some(scripts::content_digest(&installer::push_conf(&pairing()))),
            ..Component::new(now())
        }),
    );
    assert!(!installer::pairing_needs_rewrite(&manifest, &pairing()));
    let mut reminted = pairing();
    reminted.send_token = "ee".repeat(32);
    assert!(installer::pairing_needs_rewrite(&manifest, &reminted));
}

#[test]
fn commands_are_the_ones_the_app_sends() {
    let channel = RecordingChannel::healthy();
    let installer = HostInstaller::new(&channel);
    block_on(installer.install_pairing(&pairing(), &block_on(installer.inspect()).unwrap(), now()))
        .unwrap();
    let commands = channel.commands();
    assert_eq!(commands[0], commands::PREFLIGHT);
    assert_eq!(
        commands[1],
        "cat \"$HOME/.moshpit/manifest.json\" 2>/dev/null || true"
    );
    assert!(commands[2].starts_with(&format!(
        "set -e; mkdir -p \"$HOME/.moshpit\"; umask 077; p=\"$HOME/.moshpit/moshpit-push.sh\"; {} \
         t=\"$p.moshpit-new.$$\"; trap 'rm -f \"$t\"' EXIT; printf %s 'IyEvYmluL3No",
        commands::RESOLVE
    )));
    assert!(commands[2].ends_with("' | base64 -d > \"$t\"; chmod 755 \"$t\"; mv -f \"$t\" \"$p\""));
    assert_eq!(
        commands[3],
        commands::sha256("$HOME/.moshpit/moshpit-push.sh")
    );
    // The manifest last: under the host's lock, read again, written, let go
    // (B373).
    assert_eq!(commands.len(), 12);
    assert!(commands[8].starts_with("d=\"$HOME/.moshpit/.lock\"; mkdir -p "));
    assert_eq!(commands[9], commands[1]);
    assert!(commands[10].contains("p=\"$HOME/.moshpit/manifest.json\""));
    assert!(commands[11].starts_with("d=\"$HOME/.moshpit/.lock\"; a=$(cat "));
    // The same holder's token in both (B376).
    let token =
        &commands[8][commands[8].rfind("' > ").unwrap() - 16..commands[8].rfind("' > ").unwrap()];
    assert_eq!(commands[11], commands::unlock(token));
}
