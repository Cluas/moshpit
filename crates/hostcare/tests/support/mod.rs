//! A rig for the machine: simulated hosts that record every command and
//! keep the files they were told to write (as `RecordingChannel` does in
//! the app's tests), a simulated relay, a pairings store the test can see,
//! and a clock the test moves. Every output is carried out by `settle` —
//! unless the host or the relay is told to hold its answers.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use moshpit_hostcare::store::MemoryStore;
use moshpit_hostcare::{
    Change, Clock, Config, Conn, HostCare, HttpReply, Output, PairingStore, PushSupport, Settings,
};
use moshpit_install::manifest::{Component, InstallComponent, InstallManifest};
use moshpit_install::{HookAgent, scripts};
use moshpit_seal::foundation::base64;
use moshpit_seal::{Date, PushPairing, Uuid};

pub const ALL_TOOLS: &str = "home=/Users/dev\nshell=/bin/zsh\nuname=Darwin\nopenssl=yes\ncurl=yes\njq=yes\ntmux=yes\npython3=yes\nbase64=yes";
pub const DEVICE_TOKEN: &str = "abababababababababababababababababababababababababababababababab";
pub const RELAY: &str = "https://push.example.org";
pub const MASTER: &[u8] = b"moshpit-test-master";

pub fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)? + start.len();
    let to = text[from..].find(end)? + from;
    Some(&text[from..to])
}

/// One simulated host.
#[derive(Default)]
pub struct Host {
    pub commands: Vec<String>,
    pub files: BTreeMap<String, String>,
    /// First match wins; consulted before the simulated filesystem.
    pub rules: Vec<(String, String)>,
    pub fail_on: Option<String>,
    /// Answers nothing until let go.
    pub holding: bool,
    /// Answers no command holding this until let go.
    pub hold_on: Option<String>,
}

impl Host {
    pub fn healthy() -> Self {
        Host {
            rules: vec![("printf 'home=".into(), ALL_TOOLS.into())],
            ..Host::default()
        }
    }

    pub fn rule(mut self, needle: &str, reply: &str) -> Self {
        self.rules.push((needle.into(), reply.into()));
        self
    }

    /// A host carrying Claude Code's hooks and the scripts this build
    /// ships: nothing for care to do there but pair.
    pub fn hooked() -> Self {
        let mut host = Host::healthy();
        let now = Date::from_unix(1_762_800_000.0);
        let mut manifest = InstallManifest::empty();
        manifest.set(
            &InstallComponent::Hooks {
                agent: "claude".into(),
            },
            Some(Component {
                config_path: Some("$HOME/.claude/settings.json".into()),
                ..Component::new(now)
            }),
        );
        for component in [InstallComponent::Stamp, InstallComponent::Sender] {
            manifest.set(
                &component,
                Some(Component {
                    digest: scripts::digest(&component),
                    ..Component::new(now)
                }),
            );
        }
        host.files
            .insert(InstallManifest::PATH.into(), manifest.to_json());
        // What the manifest records is on the host: the scripts this build
        // ships, Claude Code's config with Moshpit's hooks in it.
        for component in [InstallComponent::Stamp, InstallComponent::Sender] {
            host.files.insert(
                component.path().unwrap(),
                scripts::body(&component).unwrap().to_owned(),
            );
        }
        host.files.insert(
            "$HOME/.claude/settings.json".into(),
            moshpit_install::hooks::merged("", &agent("claude")).unwrap(),
        );
        host
    }

    /// `body` at `path` on the host.
    pub fn with_file(mut self, path: &str, body: &str) -> Self {
        self.files.insert(path.into(), body.into());
        self
    }

    pub fn with_manifest(mut self, manifest: &InstallManifest) -> Self {
        self.files
            .insert(InstallManifest::PATH.into(), manifest.to_json());
        self
    }

    fn write_target(command: &str) -> Option<String> {
        moshpit_install::commands::written(command).map(|(path, _)| path)
    }

    pub fn written_paths(&self) -> Vec<String> {
        self.commands
            .iter()
            .filter_map(|c| Self::write_target(c))
            .collect()
    }

    pub fn sent(&self, needle: &str) -> Option<&String> {
        self.commands.iter().find(|c| c.contains(needle))
    }

    pub fn file(&self, path: &str) -> Option<&String> {
        self.files.get(path)
    }

    pub fn pairing_conf(&self, connection: Uuid) -> Option<&String> {
        InstallComponent::Pairing {
            conn: connection.to_string(),
        }
        .path()
        .and_then(|p| self.files.get(&p))
    }

    pub fn manifest(&self) -> InstallManifest {
        InstallManifest::from_text(
            self.files
                .get(InstallManifest::PATH)
                .map_or("", String::as_str),
        )
    }

    /// Answer `command` as the host would.
    pub fn answer(&mut self, command: &str) -> Result<Vec<u8>, String> {
        self.commands.push(command.to_owned());
        if self
            .fail_on
            .as_deref()
            .is_some_and(|needle| command.contains(needle))
        {
            return Err("test".into());
        }
        if let Some((_, reply)) = self
            .rules
            .iter()
            .find(|(needle, _)| command.contains(needle.as_str()))
        {
            return Ok(reply.clone().into_bytes());
        }
        Ok(self.simulate(command).into_bytes())
    }

    fn simulate(&mut self, command: &str) -> String {
        // `commands::PAIRING_PHONES`: each conf and the phone it pushes to.
        if command == moshpit_install::commands::PAIRING_PHONES {
            let mut out = String::new();
            for (path, body) in &self.files {
                let Some(name) = path.strip_prefix("$HOME/.moshpit/push.d/") else {
                    continue;
                };
                if name.ends_with(".conf") && !name.contains('/') {
                    let token = body
                        .lines()
                        .find_map(|l| l.strip_prefix("APNS_TOKEN="))
                        .unwrap_or("");
                    out.push_str(&format!("{name} {token}\n"));
                }
            }
            return out;
        }
        // `drift::command`: a line per check, from the files held here.
        if command.starts_with("printf '%s=%s\\n' 0 ") {
            let mut out = String::new();
            for (i, segment) in command.split("printf '%s=%s\\n' ").skip(1).enumerate() {
                let value = if let Some(path) = between(segment, "moshpit-stamp.sh' \"", "\"") {
                    // Then each event this build registers (B381): all
                    // there, or an older registration; and an agent whose
                    // hooks file is there ignores them in its config
                    // (B385).
                    let body = self.files.get(path);
                    let hooked = body.is_some_and(|body| body.contains("moshpit-stamp.sh"));
                    let ignored = between(segment, "then if [ -f \"", "\"")
                        .is_some_and(|file| self.files.contains_key(file));
                    let current = segment
                        .split("grep -qF '")
                        .skip(2)
                        .filter_map(|rest| rest.split('\'').next())
                        .all(|needle| body.is_some_and(|body| body.contains(needle)));
                    (match (hooked, ignored, current) {
                        (false, _, _) => "no",
                        (true, false, true) => "yes",
                        (true, _, _) => "old",
                    })
                    .to_owned()
                } else if let Some(path) = between(segment, "[ -f \"", "\"") {
                    self.files
                        .get(path)
                        .map_or("-".to_owned(), |body| scripts::content_digest(body))
                } else {
                    String::new()
                };
                out.push_str(&format!("{i}={value}\n"));
            }
            return out;
        }
        if let (Some(path), Some(payload)) = (
            Self::write_target(command),
            between(command, "printf %s '", "' | base64 -d"),
        ) && let Some(text) = base64::decode(payload).and_then(|b| String::from_utf8(b).ok())
        {
            self.files.insert(path, text);
            return String::new();
        }
        // `commands::file_exists` (B385).
        if let Some(path) = command
            .strip_prefix("[ -f \"")
            .and_then(|rest| rest.strip_suffix("\" ] && echo yes || true"))
        {
            return if self.files.contains_key(path) {
                "yes".to_owned()
            } else {
                String::new()
            };
        }
        if let Some(rest) = command.strip_prefix("cat \"") {
            let path: String = rest.chars().take_while(|&c| c != '"').collect();
            return self.files.get(&path).cloned().unwrap_or_default();
        }
        if command.contains("awk '{print $1}'") {
            let Some(path) = between(command, "sha256sum \"", "\"") else {
                return String::new();
            };
            return self
                .files
                .get(path)
                .map(|body| scripts::content_digest(body))
                .unwrap_or_default();
        }
        if let Some(rest) = command.strip_prefix("rm -f \"") {
            let path: String = rest.chars().take_while(|&c| c != '"').collect();
            self.files.remove(&path);
        }
        String::new()
    }
}

/// How the simulated relay answers a mint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relay {
    /// At once, with a real send token.
    Mints,
    /// Not until the test answers ([`Rig::answer_mint`]).
    Holds,
    /// With this status and body.
    Refuses(u16, String),
    /// Never reached.
    Down,
}

/// The pairings store, seen by the machine and the test alike.
#[derive(Clone, Default)]
pub struct SharedStore(pub Arc<Mutex<MemoryStore>>);

impl SharedStore {
    pub fn read(&self) -> Vec<PushPairing> {
        self.0.lock().unwrap().pairings.clone()
    }
}

impl PairingStore for SharedStore {
    fn read(&self) -> Vec<PushPairing> {
        self.0.lock().unwrap().read()
    }
    fn upsert(&mut self, pairing: &PushPairing) -> bool {
        self.0.lock().unwrap().upsert(pairing)
    }
    fn remove(&mut self, connection: Uuid) -> bool {
        self.0.lock().unwrap().remove(connection)
    }
}

pub struct Rig {
    pub machine: HostCare,
    pub hosts: HashMap<Uuid, Host>,
    pub store: SharedStore,
    pub relay: Relay,
    pub ms: u64,
    pub date: Date,
    /// Commands a holding host hasn't answered: id, connection, command.
    pub held_execs: Vec<(u64, Uuid, String)>,
    /// Mints a holding relay hasn't answered: id, url, body.
    pub held_posts: Vec<(u64, String, String)>,
    /// Every mint request the relay saw: the connection it was for.
    pub mints: Vec<String>,
    pub registered: usize,
    pub declined: Vec<Uuid>,
    /// Every Unpair the machine asked to remember (`true`) or forget.
    pub unpaired: Vec<(Uuid, bool)>,
    pub changes: Vec<Change>,
}

pub fn conn(label: &str) -> Conn {
    Conn {
        id: Uuid::new_v4(),
        label: label.into(),
    }
}

impl Rig {
    pub fn new() -> Rig {
        Rig::with_config(Config {
            push: PushSupport::Apns {
                environment: "production".into(),
            },
            autocare_off: false,
            budgets: Default::default(),
        })
    }

    pub fn with_config(config: Config) -> Rig {
        let store = SharedStore::default();
        let date = Date::from_unix(1_762_800_000.0);
        let mut machine = HostCare::new(config, Box::new(store.clone()), Clock { ms: 0, date });
        machine.set_settings(
            Settings {
                relay_url: RELAY.into(),
                ..Settings::default()
            },
            Clock { ms: 0, date },
        );
        let mut rig = Rig {
            machine,
            hosts: HashMap::new(),
            store,
            relay: Relay::Mints,
            ms: 0,
            date,
            held_execs: Vec::new(),
            held_posts: Vec::new(),
            mints: Vec::new(),
            registered: 0,
            declined: Vec::new(),
            unpaired: Vec::new(),
            changes: Vec::new(),
        };
        rig.settle();
        rig
    }

    pub fn clock(&self) -> Clock {
        Clock {
            ms: self.ms,
            date: self.date,
        }
    }

    /// The phone has a device token.
    pub fn with_token(mut self) -> Rig {
        let clock = self.clock();
        self.machine
            .restore_device_token(Some(DEVICE_TOKEN.into()), clock);
        self.settle();
        self
    }

    /// `connection` is saved (and only the saved ones are).
    pub fn save(&mut self, connections: &[Uuid]) {
        let clock = self.clock();
        self.machine
            .set_saved(connections, Some(connections), clock);
        self.settle();
    }

    pub fn host(&mut self, connection: Uuid) -> &mut Host {
        self.hosts.entry(connection).or_insert_with(Host::healthy)
    }

    pub fn set_host(&mut self, connection: Uuid, host: Host) {
        self.hosts.insert(connection, host);
    }

    pub fn pairing(&self, connection: Uuid) -> Option<PushPairing> {
        self.store
            .read()
            .into_iter()
            .find(|p| p.connection_id == connection)
    }

    fn mint_reply(&self, body: &str) -> HttpReply {
        let request: serde_json::Value = serde_json::from_str(body).unwrap();
        let apns = request["apnsToken"].as_str().unwrap();
        let conn = request["conn"].as_str().unwrap();
        let iat = self.date.unix() as i64;
        let token = moshpit_seal::token::mint(MASTER, apns, conn, iat);
        HttpReply {
            status: 200,
            body: format!("{{\"ok\":true,\"sendToken\":\"{token}\",\"iat\":{iat}}}").into_bytes(),
        }
    }

    /// Carry out everything the machine asked for, answering what isn't
    /// held, until it asks for nothing more.
    pub fn settle(&mut self) {
        for _ in 0..10_000 {
            let Some(output) = self.machine.poll_output() else {
                return;
            };
            let clock = self.clock();
            match output {
                Output::Exec {
                    id,
                    connection,
                    command,
                } => {
                    let host = self.hosts.entry(connection).or_insert_with(Host::healthy);
                    let held = host
                        .hold_on
                        .as_deref()
                        .is_some_and(|needle| command.contains(needle));
                    if host.holding || held {
                        self.held_execs.push((id, connection, command));
                    } else {
                        let result = host.answer(&command);
                        self.machine.exec_done(id, result, clock);
                    }
                }
                Output::Post { id, url, body, .. } => {
                    let conn = serde_json::from_str::<serde_json::Value>(&body).unwrap()["conn"]
                        .as_str()
                        .unwrap()
                        .to_owned();
                    self.mints.push(conn);
                    match self.relay.clone() {
                        Relay::Mints => {
                            let reply = self.mint_reply(&body);
                            self.machine.post_done(id, Ok(reply), clock);
                        }
                        Relay::Holds => self.held_posts.push((id, url, body)),
                        Relay::Refuses(status, text) => self.machine.post_done(
                            id,
                            Ok(HttpReply {
                                status,
                                body: text.into_bytes(),
                            }),
                            clock,
                        ),
                        Relay::Down => self.machine.post_done(
                            id,
                            Err("The Internet connection appears to be offline.".into()),
                            clock,
                        ),
                    }
                }
                Output::RegisterForPush => self.registered += 1,
                Output::RememberDeclined { connection } => self.declined.push(connection),
                Output::RememberUnpaired {
                    connection,
                    unpaired,
                } => self.unpaired.push((connection, unpaired)),
                Output::Changed(change) => self.changes.push(change),
            }
        }
        panic!("the machine never stopped asking");
    }

    /// Let `connection`'s host answer everything it holds, and from now on.
    pub fn let_go(&mut self, connection: Uuid) {
        self.host(connection).holding = false;
        self.host(connection).hold_on = None;
        let held: Vec<_> = std::mem::take(&mut self.held_execs);
        let clock = self.clock();
        for (id, conn, command) in held {
            if conn == connection {
                let result = self.hosts.get_mut(&conn).unwrap().answer(&command);
                self.machine.exec_done(id, result, clock);
            } else {
                self.held_execs.push((id, conn, command));
            }
        }
        self.settle();
    }

    /// The relay answers the held mint at `index` (0: the oldest).
    pub fn answer_mint(&mut self, index: usize) {
        let (id, _, body) = self.held_posts.remove(index);
        let reply = self.mint_reply(&body);
        let clock = self.clock();
        self.machine.post_done(id, Ok(reply), clock);
        self.settle();
    }

    /// Time passes.
    pub fn advance(&mut self, ms: u64) {
        self.ms += ms;
        self.date = self.date.adding(ms as f64 / 1000.0);
        let clock = self.clock();
        self.machine.tick(clock);
        self.settle();
    }

    /// Time passes until nothing waits on it (the waits are bounded).
    pub fn run_out_the_clock(&mut self) {
        for _ in 0..100 {
            match self.machine.next_deadline() {
                Some(at) if at > self.ms => self.advance(at - self.ms),
                Some(_) => self.advance(1),
                None => return,
            }
        }
    }

    // Inputs, with the rig's clock, settled.

    pub fn tend(&mut self, conn: &Conn) {
        let clock = self.clock();
        self.machine.tend(conn.clone(), clock);
        self.settle();
    }

    pub fn open(&mut self, conn: &Conn, pane: Option<&str>) -> u64 {
        let clock = self.clock();
        let page = self
            .machine
            .open_setup(conn.clone(), pane.map(str::to_owned), clock);
        self.settle();
        page
    }

    pub fn act(&mut self, f: impl FnOnce(&mut HostCare, Clock)) {
        let clock = self.clock();
        f(&mut self.machine, clock);
        self.settle();
    }

    pub fn view(&self, page: u64) -> moshpit_hostcare::SetupView {
        self.machine.setup_view(page).expect("the page is open")
    }
}

/// A pairing the way the phone holds one, minted and ready.
pub fn ready_pairing(connection: Uuid) -> PushPairing {
    let now = Date::from_unix(1_762_800_000.0);
    PushPairing::make(connection, "m1-pro", RELAY, now).minted(
        &"cd".repeat(32),
        now,
        DEVICE_TOKEN,
        "production",
    )
}

pub fn agent(id: &str) -> HookAgent {
    HookAgent::named(id).unwrap()
}
