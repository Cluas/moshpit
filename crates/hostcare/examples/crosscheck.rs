//! The Rust side of the host-care cross-check (`harness/crosscheck.py`):
//! the machine run through the same scenarios as the app's Swift host care
//! (`harness/swift/main.swift`) against the same simulated host, a pairings
//! file of its own and a relay that answers at once. One JSON object per
//! scenario on stdout.
//!
//! ```sh
//! cargo run -p moshpit-hostcare --example crosscheck -- inspect pair care-hooked
//! ```

#[path = "../tests/support/mod.rs"]
mod support;

use moshpit_hostcare::{
    Clock, Config, Conn, HostCare, HttpReply, Output, PairingStore, Phase, Proof, PushSupport,
    Settings,
};
use moshpit_install::installer::ComponentStatus;
use moshpit_install::manifest::{Component, InstallComponent, InstallManifest};
use moshpit_install::scripts;
use moshpit_seal::{Date, PushPairing, PushPairingFile, Uuid};
use serde_json::{Value, json};
use support::{ALL_TOOLS, Host, between};

const CONN: &str = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";
const RELAY: &str = "https://push.example.org";

fn token() -> String {
    "ab".repeat(32)
}

fn manifest(
    stamp: Option<&str>,
    sender: Option<&str>,
    hooks: &[(&str, &str)],
    paired: bool,
) -> String {
    let now = Date::from_unix(1_762_800_000.0);
    let mut m = InstallManifest::empty();
    if let Some(stamp) = stamp {
        m.set(
            &InstallComponent::Stamp,
            Some(Component {
                digest: Some(stamp.into()),
                ..Component::new(now)
            }),
        );
    }
    if let Some(sender) = sender {
        m.set(
            &InstallComponent::Sender,
            Some(Component {
                digest: Some(sender.into()),
                ..Component::new(now)
            }),
        );
    }
    for (agent, path) in hooks {
        m.set(
            &InstallComponent::Hooks {
                agent: (*agent).into(),
            },
            Some(Component {
                config_path: Some((*path).into()),
                ..Component::new(now)
            }),
        );
    }
    if paired {
        m.set(
            &InstallComponent::Pairing { conn: CONN.into() },
            Some(Component {
                connection_id: Some(CONN.into()),
                relay_url: Some("r".into()),
                ..Component::new(now)
            }),
        );
    }
    m.to_json()
}

/// What the host's manifest records is on the host, unless the scenario
/// put something else there — the bytes the Swift harness places
/// (`fixtureFiles` in harness/swift/main.swift).
fn place(host: &mut Host) {
    let Some(text) = host.files.get(InstallManifest::PATH) else {
        return;
    };
    let manifest = InstallManifest::from_text(text);
    let mut files = Vec::new();
    for (component, older) in [
        (InstallComponent::Stamp, "# an older stamp\n"),
        (InstallComponent::Sender, "# an older sender\n"),
    ] {
        if let Some(recorded) = manifest.get(&component) {
            let body = if recorded.digest == scripts::digest(&component) {
                scripts::body(&component).unwrap()
            } else {
                older
            };
            files.push((component.path().unwrap(), body.to_owned()));
        }
    }
    for agent in moshpit_install::HookAgent::ALL {
        let component = InstallComponent::Hooks {
            agent: agent.id.to_owned(),
        };
        if let Some(path) = manifest.get(&component).and_then(|c| c.config_path.clone()) {
            files.push((path, moshpit_install::hooks::merged("", &agent).unwrap()));
        }
    }
    for (path, body) in files {
        host.files.entry(path).or_insert(body);
    }
}

fn ready_pairing(relay: &str) -> PushPairing {
    let at = Date::now();
    PushPairing::make(Uuid::parse(CONN).unwrap(), "m1-pro", relay, at).minted(
        &"cd".repeat(32),
        at,
        &token(),
        "production",
    )
}

fn status(s: &ComponentStatus) -> &'static str {
    match s {
        ComponentStatus::Absent => "absent",
        ComponentStatus::Current => "current",
        ComponentStatus::Stale { .. } => "stale",
    }
}

fn proof(p: &Proof) -> String {
    match p {
        Proof::Untested => "untested".into(),
        Proof::Proving => "proving".into(),
        Proof::Proven(_) => "proven".into(),
        Proof::Failed(why) => format!("failed: {}", why.english()),
        Proof::Unavailable(why) => format!("unavailable: {}", why.english()),
    }
}

fn phase(p: &Phase) -> String {
    match p {
        Phase::Idle => "idle".into(),
        Phase::Inspecting => "inspecting".into(),
        Phase::Working(label) => format!("working: {}", label.english()),
        Phase::Proving => "proving".into(),
    }
}

struct Run {
    /// The real time the run began: the pairings' mint dates are now, as
    /// the Swift side's are.
    start: Date,
    machine: HostCare,
    host: Host,
    file: PushPairingFile,
    ms: u64,
    mints: usize,
    declined: Vec<String>,
}

impl Run {
    fn clock(&self) -> Clock {
        Clock {
            ms: self.ms,
            date: self.start.adding(self.ms as f64 / 1000.0),
        }
    }

    fn settle(&mut self) {
        while let Some(output) = self.machine.poll_output() {
            let clock = self.clock();
            match output {
                Output::Exec { id, command, .. } => {
                    let nonce = between(&command, "--test '", "'").map(str::to_owned);
                    let result = self.host.answer(&command);
                    self.machine.exec_done(id, result, clock);
                    if let Some(nonce) = nonce.filter(|_| !self.host.holding) {
                        self.machine.self_test_arrived(nonce, clock);
                    }
                }
                Output::Post { id, .. } => {
                    self.mints += 1;
                    let body = format!(
                        "{{\"ok\":true,\"sendToken\":\"{}\",\"iat\":1762800000}}",
                        "cd".repeat(32)
                    );
                    self.machine.post_done(
                        id,
                        Ok(HttpReply {
                            status: 200,
                            body: body.into_bytes(),
                        }),
                        clock,
                    );
                }
                Output::RememberDeclined { connection } => {
                    self.declined.push(connection.to_string())
                }
                Output::RegisterForPush
                | Output::Changed(_)
                | Output::RememberUnpaired { .. } => {}
            }
        }
    }

    fn act(&mut self, f: impl FnOnce(&mut HostCare, Clock)) {
        let clock = self.clock();
        f(&mut self.machine, clock);
        self.settle();
        // Time passes until nothing waits on it.
        while let Some(at) = self.machine.next_deadline() {
            self.ms = at.max(self.ms + 1);
            let clock = self.clock();
            self.machine.tick(clock);
            self.settle();
        }
    }
}

/// One thing a Host Setup screen is told, with its page and the clock.
type Step = Box<dyn Fn(&mut HostCare, u64, Clock)>;

fn run(name: &str) -> Value {
    let dir = std::env::temp_dir().join(format!("w15b-x-{}", moshpit_seal::random::hex(6)));
    std::fs::create_dir_all(&dir).unwrap();
    let file = PushPairingFile::new(dir.join("push-pairings.json"));
    let conn = Conn {
        id: Uuid::parse(CONN).unwrap(),
        label: "m1-pro".into(),
    };
    let start = Date::now();
    let clock = Clock { ms: 0, date: start };
    let machine = HostCare::new(
        Config {
            push: PushSupport::Apns {
                environment: "production".into(),
            },
            autocare_off: false,
            budgets: Default::default(),
        },
        Box::new(PushPairingFile::new(file.path())),
        clock,
    );
    let mut r = Run {
        start,
        machine,
        host: Host::healthy(),
        file,
        ms: 0,
        mints: 0,
        declined: Vec::new(),
    };
    let mut settings = Settings {
        relay_url: RELAY.into(),
        ..Settings::default()
    };
    let mut saved = vec![conn.id];
    let current = (
        scripts::digest(&InstallComponent::Stamp).unwrap(),
        scripts::digest(&InstallComponent::Sender).unwrap(),
    );
    let claude = [("claude", "$HOME/.claude/settings.json")];
    let both = [
        ("claude", "$HOME/.claude/settings.json"),
        ("codex", "$HOME/.codex/config.toml"),
    ];
    let mut page = None;
    let mut tend = false;
    let mut grant = false;
    let mut steps: Vec<Step> = Vec::new();
    let mut pane: Option<&str> = None;
    let mut read_only = false;
    let mut waited = 0;

    match name {
        "inspect" | "inspect-fails" => {
            if name == "inspect-fails" {
                r.host.fail_on = Some("printf".into());
            }
        }
        "install-claude" => steps.push(Box::new(|m, p, k| m.setup_install_hooks(p, k))),
        "install-codex" => {
            steps.push(Box::new(|m, p, k| m.setup_select(p, "codex", k)));
            steps.push(Box::new(|m, p, k| m.setup_install_hooks(p, k)));
        }
        "install-broken-json" => {
            r.host.rules.push((
                "cat \"$HOME/.claude/settings.json\"".into(),
                "{broken".into(),
            ));
            steps.push(Box::new(|m, p, k| m.setup_install_hooks(p, k)));
        }
        "install-then-remove" => {
            steps.push(Box::new(|m, p, k| m.setup_install_hooks(p, k)));
            steps.push(Box::new(|m, p, k| m.setup_remove_hooks(p, k)));
        }
        "pair" => steps.push(Box::new(|m, p, k| m.setup_pair(p, k))),
        "pair-missing-tools" => {
            r.host.rules = vec![(
                "printf 'home=".into(),
                "home=/h\ncurl=no\nopenssl=no\nbase64=yes".into(),
            )];
            steps.push(Box::new(|m, p, k| m.setup_pair(p, k)));
        }
        "pair-empty-relay" => {
            settings.relay_url = "   ".into();
            steps.push(Box::new(|m, p, k| m.setup_pair(p, k)));
        }
        "unpair" | "unpair-cannot-forget" => {
            r.file.write(&[ready_pairing(RELAY)]);
            r.host.files.insert(
                InstallManifest::PATH.into(),
                manifest(Some(&current.0), Some(&current.1), &claude, true),
            );
            r.host.files.insert(
                format!("$HOME/.moshpit/push.d/{CONN}.conf"),
                moshpit_install::push_conf(&ready_pairing(RELAY)),
            );
            read_only = name == "unpair-cannot-forget";
            steps.push(Box::new(|m, p, k| m.setup_unpair(p, k)));
        }
        "prove-hooks-no-pane" => steps.push(Box::new(|m, p, k| m.setup_prove_hooks(p, k))),
        "prove-hooks-elsewhere" => {
            pane = Some("%3");
            steps.push(Box::new(|m, p, k| m.setup_prove_hooks(p, k)));
        }
        "prove-hooks-stamped" | "prove-hooks-codex-empty" => {
            pane = Some("%3");
            let stamp = if name == "prove-hooks-stamped" {
                "working|moshpit-selftest|"
            } else {
                ""
            };
            r.host
                .rules
                .push(("socket_path".into(), "/tmp/tmux-501/default".into()));
            r.host
                .rules
                .push(("display-message -p -t".into(), stamp.into()));
            if name == "prove-hooks-codex-empty" {
                steps.push(Box::new(|m, p, k| m.setup_select(p, "codex", k)));
            }
            steps.push(Box::new(|m, p, k| m.setup_prove_hooks(p, k)));
        }
        "prove-push-exec-fails" => {
            steps.push(Box::new(|m, p, k| m.setup_prove_push(p, k)));
        }
        "prove-push-sender-output" => {
            r.host.holding = false;
            r.host
                .rules
                .push(("--test".into(), "relay refused: 401".into()));
            // The host's push never arrives here.
            r.host.fail_on = None;
            steps.push(Box::new(|m, p, k| m.setup_prove_push(p, k)));
        }
        "care-fresh" => tend = true,
        "care-hooked" | "care-ready-pairing" | "care-wrong-relay" | "care-notifications-off" => {
            r.host.files.insert(
                InstallManifest::PATH.into(),
                manifest(Some(&current.0), Some(&current.1), &claude, false),
            );
            match name {
                "care-ready-pairing" => {
                    r.file.write(&[ready_pairing(RELAY)]);
                }
                "care-wrong-relay" => {
                    r.file.write(&[ready_pairing("https://push.example")]);
                }
                "care-notifications-off" => settings.notifications = false,
                _ => {}
            }
            tend = true;
        }
        "care-stale-sender" => {
            r.host.files.insert(
                InstallManifest::PATH.into(),
                manifest(Some(&current.0), Some("old"), &both, false),
            );
            tend = true;
        }
        "care-stale-stamp" => {
            r.host.files.insert(
                InstallManifest::PATH.into(),
                manifest(Some("old"), Some(&current.1), &both, false),
            );
            tend = true;
        }
        "care-unsaved" => {
            saved.clear();
            tend = true;
        }
        "care-grant" => {
            tend = true;
            grant = true;
        }
        "care-declined" => {
            settings.declined = vec![CONN.into()];
            tend = true;
        }
        "care-relay-trailing-slash" => {
            settings.relay_url = format!("{RELAY}/");
            r.file.write(&[ready_pairing(RELAY)]);
            r.host.files.insert(
                InstallManifest::PATH.into(),
                manifest(Some(&current.0), Some(&current.1), &claude, false),
            );
            tend = true;
        }
        "care-conf-deleted" => {
            let phone = ready_pairing(RELAY);
            r.file.write(std::slice::from_ref(&phone));
            let mut m = InstallManifest::from_text(&manifest(
                Some(&current.0),
                Some(&current.1),
                &claude,
                false,
            ));
            m.set(
                &InstallComponent::Pairing { conn: CONN.into() },
                Some(Component {
                    digest: Some(scripts::content_digest(&moshpit_install::push_conf(&phone))),
                    connection_id: Some(CONN.into()),
                    relay_url: Some(RELAY.into()),
                    ..Component::new(Date::from_unix(1_762_800_000.0))
                }),
            );
            r.host
                .files
                .insert(InstallManifest::PATH.into(), m.to_json());
            tend = true;
        }
        "care-hooks-removed" => {
            r.host.files.insert(
                "$HOME/.claude/settings.json".into(),
                "{\"model\":\"opus\"}".into(),
            );
            r.host.files.insert(
                InstallManifest::PATH.into(),
                manifest(Some(&current.0), Some(&current.1), &claude, false),
            );
            tend = true;
        }
        other => panic!("no scenario {other}"),
    }
    place(&mut r.host);
    let suppress_arrival = name == "prove-push-sender-output";
    r.act(|m, k| {
        m.set_settings(settings.clone(), k);
        m.set_saved(&saved, Some(&saved), k);
        m.restore_device_token(Some(token()), k);
    });
    if tend {
        r.act(|m, k| m.tend(conn.clone(), k));
        if grant {
            r.act(|m, k| m.grant_consent(conn.clone(), k));
        }
    } else {
        let mut id = 0;
        r.act(|m, k| id = m.open_setup(conn.clone(), pane.map(str::to_owned), k));
        if read_only {
            set_mode(&dir, 0o555);
        }
        let began = r.ms;
        if name == "prove-push-exec-fails" {
            r.host.fail_on = Some("--test".into());
        }
        for step in &steps {
            if suppress_arrival || name == "prove-push-exec-fails" {
                // The Swift harness's host never delivers this one.
                r.host.holding = true;
            }
            r.act(|m, k| step(m, id, k));
        }
        page = Some(id);
        waited = (r.ms - began) / 1000;
    }
    set_mode(&dir, 0o755);

    let mut declined = settings.declined.clone();
    declined.extend(r.declined.clone());
    declined.sort();
    declined.dedup();
    let phone: Vec<Value> = r
        .file
        .read()
        .iter()
        .map(|p| {
            json!({
                "conn": p.connection_id.to_string(), "label": p.host_label, "secret": p.secret_hex,
                "token": p.send_token, "relay": p.relay_url,
                "apns": p.apns_token.clone().unwrap_or_default(),
                "env": p.apns_env.clone().unwrap_or_default(),
            })
        })
        .collect();
    let mut out = json!({
        "scenario": name,
        "commands": r.host.commands,
        "files": r.host.files,
        "phone": phone,
        "consent": r.machine.consent().is_some_and(|c| c.id == conn.id),
        "declined": declined,
        "mints": r.mints,
        "waited": waited,
    });
    if let Some(id) = page {
        let view = r.machine.setup_view(id).unwrap();
        out["page"] = json!({
            "phase": phase(&view.phase),
            "error": view.error.as_ref().map(|n| n.english()).unwrap_or_default(),
            "hooks": status(&view.hooks),
            "pairing": status(&view.pairing),
            "hooksProof": proof(&view.hooks_proof),
            "pushProof": proof(&view.push_proof),
            "userAction": view.user_action.as_ref().map(|n| n.english()).unwrap_or_default(),
            "relayError": view.relay_error.as_ref().map(|n| n.english()).unwrap_or_default(),
        });
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = ALL_TOOLS;
    out
}

fn set_mode(dir: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode));
}

/// The store the machine owns is the same file the runner reads.
#[allow(dead_code)]
fn _store_is_a_file(file: &PushPairingFile) -> Vec<PushPairing> {
    PairingStore::read(file)
}

fn main() {
    for name in std::env::args().skip(1) {
        println!("{}", run(&name));
    }
}
