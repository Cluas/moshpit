//! The install equivalence harness's Rust side: the case format both front
//! ends run (`harness/swift/main.swift` over the app's install sources,
//! `examples/install_harness.rs` over this crate) and the seeded case
//! generator. `harness/equivalence.py` drives both.
//!
//! Strings travel as hex of their UTF-8 (`~` empty, `-` none).
//!
//! | case | result |
//! | --- | --- |
//! | `scripts` | both scripts and their digests |
//! | `conf PAIRING-JSON` | the push conf and its digest |
//! | `merge AGENT TEXT` · `remove AGENT TEXT` | `ok TEXT` / `err MESSAGE` |
//! | `agent ID` | name, path, hint, format, trust step |
//! | `manifest OP…` | the encoded manifest (`key:digest:config:conn:relay:unix`) |
//! | `mread TEXT` | the manifest read off a host, or `empty` |
//! | `facts TEXT` · `caps TEXT` · `probe` · `install MUX MANAGER` | the parse / the command |
//! | `cmd NAME ARG…` | one `HostCommands` string |
//! | `flow KIND PREFLIGHT MANIFEST PATH CONFIG CHECKSUMS ARG…` | the result, then a digest of every command the host saw |
//!
//! Flows run the installer against a simulated host. Manifest dates are
//! masked: Swift stamps them with `Date()`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use moshpit_seal::foundation::base64;
use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::hex;
use moshpit_seal::foundation::json;
use moshpit_seal::pairing::PushPairing;

use crate::capabilities::{self, HostCapabilities, Multiplexer, PackageManager};
use crate::commands;
use crate::facts::HostFacts;
use crate::hooks::{self, Format, HookAgent};
use crate::installer::{
    self, Action, Blocked, ChannelError, ComponentStatus, HostChannel, HostInstaller,
    InstallReport, StampProof,
};
use crate::manifest::{Component, InstallComponent, InstallManifest};
use crate::scripts;

// MARK: - Tokens

fn arg(token: &str) -> String {
    if token == "~" {
        String::new()
    } else {
        String::from_utf8(hex::decode(token).expect("hex")).expect("UTF-8")
    }
}

fn opt_arg(token: &str) -> Option<String> {
    (token != "-").then(|| arg(token))
}

/// A string as a token.
pub fn out(text: &str) -> String {
    if text.is_empty() {
        "~".to_owned()
    } else {
        hex::encode(text.as_bytes())
    }
}

fn opt_out(text: Option<&str>) -> String {
    text.map_or_else(|| "-".to_owned(), out)
}

fn component(key: &str) -> InstallComponent {
    if key == "stamp" {
        InstallComponent::Stamp
    } else if key == "sender" {
        InstallComponent::Sender
    } else if let Some(agent) = key.strip_prefix("hooks.") {
        InstallComponent::Hooks {
            agent: agent.to_owned(),
        }
    } else {
        // `pairing.<conn>` or `pairing.<conn>.<install>` (B443).
        let rest = key.get(8..).unwrap_or("");
        match rest.split_once('.') {
            Some((conn, install)) if crate::manifest::is_install_id(install) => InstallComponent::Pairing {
                conn: conn.to_owned(),
                install: Some(install.to_owned()),
            },
            _ => InstallComponent::Pairing {
                conn: rest.to_owned(),
                install: None,
            },
        }
    }
}

/// `"installedAt" : "<date>"` masked.
fn masked(text: &str) -> String {
    const KEY: &str = "\"installedAt\" : \"";
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(KEY) {
        out.push_str(&rest[..at + KEY.len()]);
        rest = &rest[at + KEY.len()..];
        match rest.find('"') {
            Some(end) => {
                out.push('*');
                rest = &rest[end..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

fn manifest_out(manifest: &InstallManifest) -> String {
    if *manifest == InstallManifest::empty() {
        return "empty".to_owned();
    }
    let parts: Vec<String> = manifest
        .components
        .iter()
        .map(|(key, c)| {
            [
                out(key),
                opt_out(c.digest.as_deref()),
                format!("{:x}", c.installed_at.unix().to_bits()),
                opt_out(c.config_path.as_deref()),
                opt_out(c.connection_id.as_deref()),
                opt_out(c.relay_url.as_deref()),
            ]
            .join("|")
        })
        .collect();
    format!(
        "schema={} {}",
        manifest.schema,
        if parts.is_empty() {
            "-".to_owned()
        } else {
            parts.join(",")
        }
    )
}

fn action(action: &Action) -> String {
    match action {
        Action::Installed { digest } => format!("installed:{digest}"),
        Action::AlreadyCurrent => "alreadyCurrent".to_owned(),
        Action::Removed => "removed".to_owned(),
        Action::NotApplicable => "notApplicable".to_owned(),
        Action::Failed(why) => format!("failed:{}", out(&why.to_string())),
    }
}

fn report_out(report: &InstallReport) -> String {
    let steps: Vec<String> = report
        .steps
        .iter()
        .map(|s| format!("{}={}", s.component.key(), action(&s.action)))
        .collect();
    let blocked = match &report.blocked {
        Some(Blocked::MissingTools(tools)) => tools.join(","),
        None => "-".to_owned(),
    };
    let manifest = report
        .manifest
        .as_ref()
        .map_or_else(|| "-".to_owned(), |m| out(&masked(&m.to_json())));
    format!(
        "steps={} blocked={blocked} user={} ok={} manifest={manifest}",
        if steps.is_empty() {
            "-".to_owned()
        } else {
            steps.join(",")
        },
        opt_out(report.user_action_required),
        u8::from(report.succeeded())
    )
}

fn status_out(status: &ComponentStatus) -> String {
    match status {
        ComponentStatus::Absent => "absent".to_owned(),
        ComponentStatus::Current => "current".to_owned(),
        ComponentStatus::Stale { installed } => format!("stale:{}", out(installed)),
    }
}

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

fn between(text: &str, start: &str, end: &str) -> Option<(usize, usize)> {
    let from = text.find(start)? + start.len();
    let to = text[from..].find(end)? + from;
    Some((from, to))
}

/// The simulated host, the Swift harness's `FakeHost` rule for rule.
#[derive(Default)]
struct FakeHost {
    commands: RefCell<Vec<String>>,
    files: RefCell<BTreeMap<String, String>>,
    preflight: String,
    checksums: String,
    socket: RefCell<String>,
    stamp: RefCell<String>,
}

impl FakeHost {
    fn answer(&self, command: &str) -> (String, String) {
        let mut recorded = masked_tokens(command);
        if command.contains("printf 'home=") {
            return (self.preflight.clone(), recorded);
        }
        if command.contains("socket_path") {
            return (self.socket.borrow().clone(), recorded);
        }
        if command.contains("display-message -p -t") {
            return (self.stamp.borrow().clone(), recorded);
        }
        if let (true, Some((p_from, p_to)), Some((t_from, t_to))) = (
            command.contains("t=\"$p.moshpit-new.$$\""),
            between(command, "printf %s '", "' | base64 -d"),
            // The target the bytes replace once whole (atomic-write), as
            // named before its links are followed (link-write).
            between(command, "; p=\"", "\"; "),
        ) {
            let path = command[t_from..t_to].to_owned();
            let text =
                String::from_utf8_lossy(&base64::decode(&command[p_from..p_to]).expect("base64"))
                    .into_owned();
            if path.ends_with("manifest.json") {
                recorded = format!(
                    "{}{}{}",
                    &command[..p_from],
                    base64::encode(masked(&text).as_bytes()),
                    &command[p_to..]
                );
            }
            self.files.borrow_mut().insert(path, text);
            return (String::new(), recorded);
        }
        if let Some(rest) = command.strip_prefix("cat \"") {
            let path: String = rest.chars().take_while(|&c| c != '"').collect();
            return (
                self.files.borrow().get(&path).cloned().unwrap_or_default(),
                recorded,
            );
        }
        if command.contains("awk '{print $1}'") {
            let answer = match self.checksums.as_str() {
                "wrong" => "deadbeefdeadbeef".to_owned(),
                "empty" => String::new(),
                _ => between(command, "sha256sum \"", "\"")
                    .and_then(|(from, to)| {
                        self.files
                            .borrow()
                            .get(&command[from..to])
                            .map(|b| scripts::content_digest(b))
                    })
                    .unwrap_or_default(),
            };
            return (answer, recorded);
        }
        if let Some(rest) = command.strip_prefix("rm -f \"") {
            let path: String = rest.chars().take_while(|&c| c != '"').collect();
            self.files.borrow_mut().remove(&path);
        }
        (String::new(), recorded)
    }
}

impl HostChannel for FakeHost {
    fn run(&self, command: &str) -> impl Future<Output = Result<Vec<u8>, ChannelError>> {
        let (answer, recorded) = self.answer(command);
        self.commands.borrow_mut().push(recorded);
        async move { Ok(answer.into_bytes()) }
    }
}

/// The nonce a `cmd stamptest` case's hooks test names (B376).
const HARNESS_NONCE: &str = "0123456789abcdef";

/// The install a flow's pairing is written as (B443): every shipped build
/// has one, so the committed flows are written as one does.
const HARNESS_INSTALL: &str = "a0a0a0a0b1b1b1b1";

/// `command` with what a run draws at random (B376: the manifest lock's
/// token, the hooks test's nonce) written as fixed text, so a flow's
/// fingerprints don't move from run to run.
fn masked_tokens(command: &str) -> String {
    let mut out = command.to_owned();
    for (from, len) in [
        ("/run/selftest-", 16),
        ("[ \"${a#*.}\" = '", 16),
        ("\" '", 16),
        // The lock's own (B566).
        ("t='", 16),
    ] {
        let mut at = 0;
        while let Some(found) = out[at..].find(from) {
            let start = at + found + from.len();
            let end = start + len;
            if out.len() >= end && out[start..end].bytes().all(|b| b.is_ascii_hexdigit()) {
                out.replace_range(start..end, HARNESS_NONCE);
            }
            at = start;
        }
    }
    out
}

/// The connection a `push` flow proves (B353: the self-test names it).
const FLOW_CONN: &str = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";

fn flow(f: &[&str]) -> String {
    let host = FakeHost {
        preflight: arg(f[2]),
        checksums: f[6].to_owned(),
        ..FakeHost::default()
    };
    if f[3] != "-" {
        host.files
            .borrow_mut()
            .insert(InstallManifest::PATH.to_owned(), arg(f[3]));
    }
    if f[4] != "-" && f[5] != "-" {
        host.files.borrow_mut().insert(arg(f[4]), arg(f[5]));
    }
    let installer = HostInstaller::new(&host);
    let now = Date::from_unix(1_762_800_000.0);
    let result = block_on(async {
        let state = installer.inspect().await?;
        Ok::<String, ChannelError>(match f[1] {
            "pairing" => match json::decode::<PushPairing>(&hex::decode(f[7]).expect("hex")) {
                Ok(pairing) => report_out(
                    &installer
                        .install_pairing_as(&pairing, Some(HARNESS_INSTALL), &state, now)
                        .await?,
                ),
                Err(_) => "threw".to_owned(),
            },
            "hooks" => report_out(
                &installer
                    .install_hooks(&HookAgent::named(f[7]).expect("agent"), &state, now)
                    .await?,
            ),
            "unhooks" => report_out(
                &installer
                    .uninstall_hooks(&HookAgent::named(f[7]).expect("agent"), &state)
                    .await?,
            ),
            "uninstall" => {
                let components: Vec<InstallComponent> =
                    f[7].split(',').map(|k| component(&arg(k))).collect();
                out(&masked(
                    &installer.uninstall(&components, &state).await?.to_json(),
                ))
            }
            "stamp" => {
                *host.socket.borrow_mut() = arg(f[8]);
                *host.stamp.borrow_mut() = arg(f[9]);
                match installer.self_test_stamp(&arg(f[7])).await? {
                    StampProof::Stamped => "stamped".to_owned(),
                    StampProof::DidNotStamp(read) => format!("didNotStamp:{}", out(&read)),
                    StampProof::Unavailable => "unavailable".to_owned(),
                }
            }
            "push" => {
                installer.self_test_push(&arg(f[7]), FLOW_CONN).await?;
                "pushed".to_owned()
            }
            "status" => {
                let agents: Vec<String> = HookAgent::ALL
                    .iter()
                    .map(|a| format!("{}={}", a.id, status_out(&state.hooks_status(a.id))))
                    .collect();
                let installed: Vec<&str> = installer::installed_hook_agents(&state.manifest)
                    .iter()
                    .map(|a| a.id)
                    .collect();
                format!(
                    "stamp={} sender={} {} paired={} installed={} manifest={}",
                    status_out(&state.status(&InstallComponent::Stamp)),
                    status_out(&state.status(&InstallComponent::Sender)),
                    agents.join(" "),
                    u8::from(state.paired_but_nothing_will_push()),
                    if installed.is_empty() {
                        "-".to_owned()
                    } else {
                        installed.join(",")
                    },
                    manifest_out(&state.manifest)
                )
            }
            other => panic!("bad flow {other}"),
        })
    })
    .unwrap_or_else(|_| "threw".to_owned());
    // Each command as the first 64 bits of its SHA-256 (the Swift side's
    // rule: the scripts' payloads would make lines of 40 KB).
    let commands: Vec<String> = host
        .commands
        .borrow()
        .iter()
        .map(|c| scripts::content_digest(c)[..16].to_owned())
        .collect();
    format!("{result} || {}", commands.join(","))
}

/// Runs one case line; `None` for blank and `#` lines.
pub fn run(line: &str) -> Option<String> {
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let f: Vec<&str> = line.split(' ').collect();
    Some(match f[0] {
        "scripts" => format!(
            "{} {} {} {}",
            out(scripts::stamp()),
            out(scripts::sender()),
            scripts::digest(&InstallComponent::Stamp).expect("a digest"),
            scripts::digest(&InstallComponent::Sender).expect("a digest")
        ),
        "conf" => {
            let pairing: PushPairing =
                json::decode(&hex::decode(f[1]).expect("hex")).expect("a pairing");
            let conf = installer::push_conf(&pairing);
            format!("{} {}", out(&conf), scripts::content_digest(&conf))
        }
        "merge" | "remove" => {
            let agent = HookAgent::named(f[1]).expect("an agent");
            let result = if f[0] == "merge" {
                hooks::merged(&arg(f[2]), &agent)
            } else {
                hooks::removed(&arg(f[2]), &agent)
            };
            match result {
                Ok(text) => format!("ok {}", out(&text)),
                Err(error) => format!("err {}", out(&error.to_string())),
            }
        }
        "agent" => {
            let a = HookAgent::named(f[1]).expect("an agent");
            [
                out(a.display_name),
                out(a.config_path),
                out(&a.config_hint()),
                if a.format == Format::CodexToml {
                    "toml"
                } else {
                    "json"
                }
                .to_owned(),
                opt_out(a.trust_step()),
            ]
            .join(" ")
        }
        "manifest" => {
            let mut manifest = InstallManifest::empty();
            for op in &f[1..] {
                let p: Vec<&str> = op.split(':').collect();
                manifest.set(
                    &component(&arg(p[0])),
                    Some(Component {
                        digest: opt_arg(p[1]),
                        installed_at: Date::from_unix(p[5].parse::<i64>().expect("unix") as f64),
                        config_path: opt_arg(p[2]),
                        connection_id: opt_arg(p[3]),
                        relay_url: opt_arg(p[4]),
                        scripts_version: None,
                    }),
                );
            }
            out(&manifest.to_json())
        }
        "mread" => manifest_out(&InstallManifest::from_text(
            moshpit_seal::foundation::text::trim_white_space(&arg(f[1])),
        )),
        "facts" => {
            let facts = HostFacts::parse(&arg(f[1]));
            let tools: Vec<String> = facts
                .tools
                .iter()
                .map(|(k, v)| format!("{}={}", out(k), if *v { "1" } else { "0" }))
                .collect();
            [
                out(&facts.home),
                out(&facts.shell),
                out(&facts.uname),
                if tools.is_empty() {
                    "-".to_owned()
                } else {
                    tools.join(",")
                },
                if facts.can_push() { "1" } else { "0" }.to_owned(),
                facts.missing_for_push().join(","),
            ]
            .join(" ")
        }
        "caps" => {
            let c = HostCapabilities::parse(&arg(f[1]));
            let flags: String = [c.has_tmux, c.has_mosh_server, c.has_herdr, c.has_python3]
                .iter()
                .map(|&b| if b { '1' } else { '0' })
                .collect();
            format!(
                "{flags} {} {}",
                out(&c.os),
                c.package_manager.map_or("-", |m| match m {
                    PackageManager::AptGet => "aptGet",
                    PackageManager::Dnf => "dnf",
                    PackageManager::Yum => "yum",
                    PackageManager::Pacman => "pacman",
                    PackageManager::Apk => "apk",
                    PackageManager::Brew => "brew",
                })
            )
        }
        "probe" => format!(
            "{} {}",
            out(&capabilities::probe_command()),
            out(capabilities::EXTRA_PATH_DIRS)
        ),
        "install" => {
            let mux = match f[1] {
                "none" => Multiplexer::None,
                "tmux" => Multiplexer::Tmux,
                _ => Multiplexer::Herdr,
            };
            let manager = PackageManager::ALL.into_iter().find(|m| {
                matches!(
                    (m, f[2]),
                    (PackageManager::AptGet, "aptGet")
                        | (PackageManager::Dnf, "dnf")
                        | (PackageManager::Yum, "yum")
                        | (PackageManager::Pacman, "pacman")
                        | (PackageManager::Apk, "apk")
                        | (PackageManager::Brew, "brew")
                )
            });
            opt_out(mux.install_command(manager).as_deref())
        }
        "cmd" => match f[1] {
            "write" => out(&commands::write_file(&arg(f[2]), &arg(f[3]), &arg(f[4]))),
            "writekeep" => out(&commands::write_keeping_mode(
                &arg(f[2]),
                &arg(f[3]),
                &arg(f[4]),
            )),
            "read" => out(&commands::read_file(&arg(f[2]))),
            "sha" => out(&commands::sha256(&arg(f[2]))),
            "preflight" => out(commands::PREFLIGHT),
            "backup" => out(&commands::backup_once(&arg(f[2]))),
            "socket" => out(commands::TMUX_SOCKET_PATH),
            "pushtest" => out(&commands::self_test_push(&arg(f[2]), &arg(f[3]))),
            "stamptest" => out(&commands::stamp_test(&arg(f[2]), &arg(f[3]), HARNESS_NONCE)),
            "rest" => out(&commands::remove_the_rest(
                &HookAgent::ALL.map(|a| a.config_path),
            )),
            "left" => out(commands::PAIRINGS_LEFT),
            "lock" => out(&commands::lock(&arg(f[2]))),
            "unlock" => out(&commands::unlock(&arg(f[2]))),
            "calling" => out(&commands::configs_calling_stamp(
                &HookAgent::ALL.map(|a| a.config_path),
            )),
            "remove" => out(&commands::remove_file(&arg(f[2]))),
            "prune" => out(&commands::prune_backups(
                &arg(f[2]),
                f[3].parse().expect("keep"),
            )),
            "quote" => out(&commands::quote(&arg(f[2]))),
            "safe" => if commands::is_safe_path(&arg(f[2])) {
                "1"
            } else {
                "0"
            }
            .to_owned(),
            "path" => {
                let pairing = InstallComponent::Pairing { conn: arg(f[2]), install: None };
                let other = component(&arg(f[3]));
                format!(
                    "{} {} {} {}",
                    out(&pairing.path().expect("a path")),
                    out(&other.key()),
                    opt_out(other.path().as_deref()),
                    other.mode()
                )
            }
            other => panic!("bad cmd {other}"),
        },
        "flow" => flow(&f),
        other => panic!("unknown command {other}"),
    })
}

// MARK: - Generating cases

/// SplitMix64, the generator's only source of randomness.
#[derive(Debug, Clone)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[usize::try_from(self.below(items.len() as u64)).expect("small")]
    }
}

/// Text pieces for values: every whitespace kind, combining marks, CJK,
/// emoji, JSON and shell specials. No CR (the CR LF fixes are documented
/// divergences with cases of their own).
const PIECES: &[&str] = &[
    "a",
    "Z",
    "0",
    "7",
    "42",
    " ",
    "  ",
    "\t",
    "\n",
    "\u{A0}",
    "\u{2028}",
    "\u{3000}",
    "\u{200B}",
    "\u{85}",
    "\u{301}",
    "e\u{301}",
    "长",
    "一",
    "😀",
    "👨\u{200D}👩\u{200D}👧",
    "🇨🇳",
    "\"",
    "\\",
    "/",
    "%",
    "$",
    "$(x)",
    "`",
    "-",
    "_",
    ".",
    ":",
    "|",
    "=",
    "#",
    "[",
    "]",
    "{",
    "}",
    "\u{1}",
    "\u{7f}",
    "\u{FEFF}",
    "build",
    "opus",
    "claude",
];

fn text_from(rng: &mut SplitMix64, pieces: &[&str], max: u64) -> String {
    (0..rng.below(max + 1)).map(|_| *rng.pick(pieces)).collect()
}

fn json_string(rng: &mut SplitMix64, text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0}'..='\u{1f}' => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            '/' if rng.chance(50) => out.push_str("\\/"),
            c if !c.is_ascii() && rng.chance(15) => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Keys of ASCII only (JSONSerialization's collation is reproduced for
/// ASCII), built to probe it.
fn random_key(rng: &mut SplitMix64) -> String {
    const PIECES: &[&str] = &[
        "a",
        "A",
        "b",
        "B",
        "z",
        "Z",
        "0",
        "1",
        "2",
        "9",
        "00",
        "01",
        "10",
        "007",
        "99",
        "100",
        "_",
        "-",
        ".",
        " ",
        "$",
        "@",
        "/",
        "\\",
        ":",
        "#",
        "~",
        "|",
        "\t",
        "\u{1}",
        "hooks",
        "Hooks",
        "env",
        "model",
        "PreToolUse",
        "Stop",
        "x1",
        "x01",
        "x10",
        "matcher",
        "command",
        "type",
        "permissions",
        "allow",
        "statusLine",
        "\u{FEFF}",
    ];
    text_from(rng, PIECES, 3)
}

/// A JSON number JSONSerialization reads and writes back the way the port
/// models it (no overflow, at most 38 significant digits).
fn number(rng: &mut SplitMix64) -> String {
    match rng.below(9) {
        0 => rng
            .pick(&["0", "-0", "1", "-1", "30", "100", "4096"])
            .to_string(),
        1 => ((rng.next_u64() >> rng.below(63)) as i64).to_string(),
        2 => rng
            .pick(&[
                "9223372036854775807",
                "-9223372036854775808",
                "18446744073709551615",
                "18446744073709551616",
                "-9223372036854775809",
                "123456789012345678901234567890",
            ])
            .to_string(),
        3 => format!("{}.{}", rng.below(1000), rng.below(1_000_000)),
        4 => rng
            .pick(&[
                "0.1", "0.7", "1.0", "1.10", "-0.0", "0.5", "2.5e-5", "1e3", "1E3", "0.1e1",
                "1.5e300", "1e-7", "1e16", "1e17", "1e21", "1e-400", "4e-324", "-1.5", "0.0001",
                "0.00001",
            ])
            .to_string(),
        5 => format!(
            "{}e{}{}",
            rng.below(100),
            rng.pick(&["", "+", "-"]),
            rng.below(30)
        ),
        6 => format!(
            "0.{}{}",
            "0".repeat(usize::try_from(rng.below(20)).expect("small")),
            rng.below(1_000_000_000_000)
        ),
        7 => format!(
            "{}.{}e{}",
            1 + rng.below(9),
            rng.below(1_000_000_000_000_000_000),
            rng.below(40) as i64 - 20
        ),
        _ => format!("{}.{}", rng.below(u64::MAX), rng.below(1000)),
    }
}

fn value(rng: &mut SplitMix64, depth: u64) -> String {
    match rng.below(if depth > 2 { 4 } else { 6 }) {
        0 => number(rng),
        1 | 2 => {
            let text = text_from(rng, PIECES, 6);
            json_string(rng, &text)
        }
        3 => rng.pick(&["true", "false", "null"]).to_string(),
        4 => {
            let items: Vec<String> = (0..rng.below(4)).map(|_| value(rng, depth + 1)).collect();
            format!(
                "[{}{}]",
                items.join(","),
                if !items.is_empty() && rng.chance(10) {
                    ","
                } else {
                    ""
                }
            )
        }
        _ => object(rng, depth + 1, false),
    }
}

/// A hook group: ours (any agent, any state), the user's, or a near miss.
fn group(rng: &mut SplitMix64) -> String {
    let command = match rng.below(6) {
        0 | 1 => hooks::command(
            rng.pick(&["working", "attention", "done"]),
            rng.pick(&["claude", "gemini", "codex"]),
        ),
        2 => "my-own-audit.sh".to_owned(),
        3 => "sh /opt/moshpit-stamp.sh.bak".to_owned(),
        4 => "moshpit-stamp.sh\u{301}".to_owned(),
        _ => text_from(rng, PIECES, 4),
    };
    let entry = format!(
        "{{\"type\":\"command\",\"command\":{}}}",
        json_string(rng, &command)
    );
    let entries = match rng.below(5) {
        0 => format!("[{entry},{{\"type\":\"command\",\"command\":\"greet.sh\"}}]"),
        1 => "[]".to_owned(),
        2 => format!("[{entry},{}]", object(rng, 3, false)),
        _ => format!("[{entry}]"),
    };
    let mut fields = vec![format!("\"hooks\":{entries}")];
    if rng.chance(40) {
        let matcher = *rng.pick(&["*", "Bash", "Edit|Write"]);
        fields.push(format!("\"matcher\":{}", json_string(rng, matcher)));
    }
    if rng.chance(10) {
        let key = random_key(rng);
        let key = json_string(rng, &key);
        let member = value(rng, 3);
        fields.push(format!("{key}:{member}"));
    }
    format!("{{{}}}", fields.join(","))
}

fn object(rng: &mut SplitMix64, depth: u64, top: bool) -> String {
    let mut fields: Vec<String> = (0..rng.below(if top { 6 } else { 4 }))
        .map(|_| {
            let key = random_key(rng);
            format!("{}:{}", json_string(rng, &key), value(rng, depth + 1))
        })
        .collect();
    if top && rng.chance(70) {
        let hooks = match rng.below(8) {
            0 => value(rng, 2),
            1 => "{}".to_owned(),
            _ => {
                let events: Vec<String> = (0..1 + rng.below(4))
                    .map(|_| {
                        let event = *rng.pick(&[
                            "PreToolUse",
                            "Stop",
                            "Notification",
                            "UserPromptSubmit",
                            "SessionStart",
                            "x",
                        ]);
                        let list = match rng.below(6) {
                            0 => value(rng, 3),
                            _ => format!(
                                "[{}]",
                                (0..rng.below(4))
                                    .map(|_| group(rng))
                                    .collect::<Vec<_>>()
                                    .join(",")
                            ),
                        };
                        format!("\"{event}\":{list}")
                    })
                    .collect();
                format!("{{{}}}", events.join(","))
            }
        };
        let at = usize::try_from(rng.below(fields.len() as u64 + 1)).expect("small");
        fields.insert(at, format!("\"hooks\":{hooks}"));
    }
    if rng.chance(10) {
        // A repeated key: Foundation keeps the first.
        let key = random_key(rng);
        let key = json_string(rng, &key);
        for _ in 0..2 {
            let member = value(rng, depth + 1);
            fields.push(format!("{key}:{member}"));
        }
    }
    let space = |rng: &mut SplitMix64| rng.pick(&["", "", " ", "\n  ", "\t"]).to_string();
    let joined: Vec<String> = fields
        .iter()
        .map(|field| format!("{}{field}{}", space(rng), space(rng)))
        .collect();
    format!(
        "{{{}{}}}",
        joined.join(","),
        if !fields.is_empty() && rng.chance(10) {
            ","
        } else {
            ""
        }
    )
}

fn json_config(rng: &mut SplitMix64) -> String {
    match rng.below(20) {
        0 => rng
            .pick(&[
                "",
                " \n ",
                "{}",
                "[]",
                "7",
                "null",
                "\"s\"",
                "{\"model\": }",
                "{oops",
                "{\"a\":1}x",
                "[1,2,3]",
            ])
            .to_string(),
        1 => {
            // A config this harness already merged once.
            let base = object(rng, 0, true);
            hooks::merged_json(&base, rng.pick(&["claude", "gemini"])).unwrap_or(base)
        }
        2 => format!("\u{FEFF}{}", object(rng, 0, true)),
        _ => {
            let mut doc = object(rng, 0, true);
            if rng.chance(10) {
                doc = format!(" \n{doc}\n\n");
            }
            doc
        }
    }
}

fn toml_config(rng: &mut SplitMix64) -> String {
    let lines: Vec<String> = (0..rng.below(8))
        .map(|_| match rng.below(10) {
            0 => format!(
                "{}{}",
                rng.pick(&["", " ", "\t", "\u{A0}", "\u{200B}"]),
                hooks::TOML_BEGIN
            ),
            1 => format!("{}{}", hooks::TOML_END, rng.pick(&["", " ", "\t", "  "])),
            2 => String::new(),
            3 => rng.pick(&["   ", "\t", "\u{3000}"]).to_string(),
            4 => "[[hooks.Stop]]".to_owned(),
            5 => format!("command = \"{}\"", hooks::command("done", "codex")),
            6 => "model = \"gpt-5\"".to_owned(),
            7 => "[tools]".to_owned(),
            _ => text_from(
                rng,
                &PIECES
                    .iter()
                    .copied()
                    .filter(|p| !p.contains('\n'))
                    .collect::<Vec<_>>(),
                5,
            ),
        })
        .collect();
    let mut text = lines.join("\n");
    if rng.chance(60) {
        text.push('\n');
    }
    if rng.chance(20) {
        text = hooks::merged_toml(&text, "codex");
    }
    text
}

fn hex_text(text: &str) -> String {
    out(text)
}

fn pairing_json(rng: &mut SplitMix64) -> String {
    let conn = format!(
        "{:08X}-{:04X}-4{:03X}-8{:03X}-{:012X}",
        rng.next_u64() as u32,
        rng.next_u64() as u16,
        rng.below(0x1000),
        rng.below(0x1000),
        rng.next_u64() & 0xFFFF_FFFF_FFFF
    );
    let secret: String = (0..4).map(|_| format!("{:016x}", rng.next_u64())).collect();
    let label = text_from(rng, PIECES, 3);
    let label = json_string(rng, &label);
    let relay = *rng.pick(&[
        "https://push.example.org",
        "https://r",
        "http://localhost:8080",
        "https://push.example.org/sub/",
        "https://例え.jp",
    ]);
    let relay = json_string(rng, relay);
    let mut fields = vec![
        format!("\"connectionId\":\"{conn}\""),
        format!("\"hostLabel\":{label}"),
        format!(
            "\"secretHex\":\"{}\"",
            if rng.chance(10) {
                secret.to_uppercase()
            } else {
                secret
            }
        ),
        format!(
            "\"sendToken\":\"{}\"",
            if rng.chance(20) {
                String::new()
            } else {
                format!("{:016x}{:016x}", rng.next_u64(), rng.next_u64())
            }
        ),
        format!("\"relayURL\":{relay}"),
        format!("\"createdAt\":{}", rng.below(800_000_000)),
    ];
    if rng.chance(70) {
        let ef = "ef".repeat(32);
        let token = *rng.pick(&["", "ABCDEF0123", ef.as_str()]);
        let token = json_string(rng, token);
        fields.push(format!("\"apnsToken\":{token}"));
    }
    if rng.chance(70) {
        let env = *rng.pick(&["production", "sandbox", "", "weird env"]);
        let env = json_string(rng, env);
        fields.push(format!("\"apnsEnv\":{env}"));
    }
    if rng.chance(80) {
        let issued = match rng.below(5) {
            0 => "0".to_owned(),
            1 => format!("{}.{}", rng.below(800_000_000), rng.below(1000)),
            2 => format!("-{}.5", rng.below(1_000_000_000)),
            3 => format!("{}", rng.below(1_000_000_000_000)),
            _ => format!("{}", 777_000_000 + rng.below(10_000_000)),
        };
        fields.push(format!("\"sendTokenIssuedAt\":{issued}"));
    }
    format!("{{{}}}", fields.join(","))
}

fn manifest_ops(rng: &mut SplitMix64) -> Vec<String> {
    (0..rng.below(5))
        .map(|_| {
            let key = match rng.below(5) {
                0 => "stamp".to_owned(),
                1 => "sender".to_owned(),
                2 => format!(
                    "hooks.{}",
                    rng.pick(&["claude", "codex", "gemini", "qwen", "x"])
                ),
                _ => format!(
                    "pairing.{:08X}-AAAA-4BBB-8CCC-{:012X}",
                    rng.next_u64() as u32,
                    rng.next_u64() & 0xFFFF_FFFF_FFFF
                ),
            };
            let field = |rng: &mut SplitMix64, pool: &[&str]| -> String {
                if rng.chance(40) {
                    "-".to_owned()
                } else {
                    hex_text(rng.pick(pool))
                }
            };
            let digest = field(rng, &["deadbeef", &scripts::content_digest("x"), ""]);
            let config = field(
                rng,
                &[
                    "$HOME/.claude/settings.json",
                    "$HOME/dotfiles/a b.json",
                    "/x/\"y\"/é",
                ],
            );
            let conn = field(rng, &["3F2504E0-4F89-11D3-9A0C-0305E82C3301", "c"]);
            let relay = field(rng, &["https://push.example.org", "https://r/x?y=1&z=\\"]);
            format!(
                "{}:{digest}:{config}:{conn}:{relay}:{}",
                hex_text(&key),
                1_600_000_000 + rng.below(400_000_000)
            )
        })
        .collect()
}

fn preflight_output(rng: &mut SplitMix64) -> String {
    let mut lines = vec![
        format!(
            "home={}",
            rng.pick(&["/Users/dev", "/home/x", "", "/home/a=b", " /h \u{A0}"])
        ),
        format!(
            "shell={}",
            rng.pick(&["/bin/zsh", "unknown", "/usr/bin/fish"])
        ),
        format!("uname={}", rng.pick(&["Darwin", "Linux", "unknown"])),
    ];
    for tool in ["openssl", "curl", "jq", "tmux", "python3", "base64"] {
        lines.push(format!(
            "{tool}={}",
            rng.pick(&["yes", "yes", "yes", "no", " yes", "yes\u{200B}"])
        ));
    }
    if rng.chance(20) {
        lines.push(text_from(rng, &["=", "x", "\u{301}", " ", "yes", "é"], 4));
    }
    if rng.chance(30) {
        lines.swap(0, 3);
    }
    lines.join(rng.pick(&["\n", "\n", "\n\n"]))
}

/// `count` seeded cases.
pub fn generate(seed: u64, count: usize) -> Vec<String> {
    let mut rng = SplitMix64(seed);
    let mut cases = vec![
        "scripts".to_owned(),
        "probe".to_owned(),
        "cmd preflight".to_owned(),
        "cmd socket".to_owned(),
    ];
    for agent in HookAgent::ALL {
        cases.push(format!("agent {}", agent.id));
    }
    for mux in ["none", "tmux", "herdr"] {
        for manager in ["-", "aptGet", "dnf", "yum", "pacman", "apk", "brew"] {
            cases.push(format!("install {mux} {manager}"));
        }
    }
    while cases.len() < count {
        let case = match rng.below(20) {
            0..=4 => {
                let agent = *rng.pick(&HookAgent::ALL);
                let text = if agent.format == Format::CodexToml {
                    toml_config(&mut rng)
                } else {
                    json_config(&mut rng)
                };
                format!(
                    "{} {} {}",
                    if rng.chance(70) { "merge" } else { "remove" },
                    agent.id,
                    out(&text)
                )
            }
            5 | 6 => format!("conf {}", hex::encode(pairing_json(&mut rng).as_bytes())),
            7 => format!("manifest {}", manifest_ops(&mut rng).join(" "))
                .trim_end()
                .to_owned(),
            8 => {
                let mut manifest = InstallManifest::empty();
                for op in manifest_ops(&mut rng) {
                    let p: Vec<&str> = op.split(':').collect();
                    manifest.set(
                        &component(&arg(p[0])),
                        Some(Component {
                            digest: opt_arg(p[1]),
                            installed_at: Date::from_unix(p[5].parse::<i64>().expect("unix") as f64),
                            config_path: opt_arg(p[2]),
                            connection_id: opt_arg(p[3]),
                            relay_url: opt_arg(p[4]),
                            scripts_version: None,
                        }),
                    );
                }
                let text = manifest.to_json();
                let text = match rng.below(10) {
                    0 => text.replace("\"schema\" : 1", "\"schema\" : 2"),
                    1 => text.replace("Z\"", "+08:00\""),
                    2 => text.replace("Z\"", ".5Z\""),
                    3 => text.replace("\"installedAt\"", "\"installedat\""),
                    4 => format!("\u{FEFF}{text}"),
                    5 => text.replace("\"schema\" : 1", "\"schema\" : 1.0"),
                    6 => text.replace('}', ",}"),
                    7 => String::new(),
                    _ => text,
                };
                format!("mread {}", out(&text))
            }
            9 => format!("facts {}", out(&preflight_output(&mut rng))),
            10 => {
                let mut lines: Vec<String> = Vec::new();
                for tool in ["tmux", "mosh-server", "herdr", "python3"] {
                    if rng.chance(60) {
                        lines.push(format!(
                            "{}{}/{tool}{}",
                            rng.pick(&["", " ", "\t"]),
                            rng.pick(&[
                                "/usr/bin",
                                "/opt/homebrew/bin",
                                "/home/dev/.local/bin",
                                "/x/\u{301}y"
                            ]),
                            rng.pick(&["", " ", "/"])
                        ));
                    }
                }
                if rng.chance(85) {
                    lines.push(format!(
                        "::{}::{}{}",
                        rng.pick(&["Linux", "Darwin", "", " FreeBSD "]),
                        rng.pick(&[
                            "/usr/bin/apt-get",
                            "/usr/bin/dnf",
                            "/usr/bin/yum",
                            "/usr/bin/pacman",
                            "/sbin/apk",
                            "/opt/homebrew/bin/brew",
                            "/usr/bin/zypper",
                            "",
                            "brew"
                        ]),
                        rng.pick(&["", "::extra", " "])
                    ));
                }
                if rng.chance(15) {
                    lines.push(text_from(
                        &mut rng,
                        &["tmux", "/", ":", "\u{301}", " ", "herdr"],
                        4,
                    ));
                }
                let separator = *rng.pick(&["\n", "\r\n", "\n\n", "\u{2028}", "\r"]);
                format!("caps {}", out(&lines.join(separator)))
            }
            11..=13 => {
                let value = |rng: &mut SplitMix64| {
                    let text = text_from(rng, PIECES, 5);
                    // quote-combining is a documented divergence.
                    if text.contains('\'') {
                        text.replace('\u{301}', "")
                    } else {
                        text
                    }
                };
                let path = |rng: &mut SplitMix64| -> String {
                    text_from(
                        rng,
                        &[
                            "$HOME",
                            "/",
                            ".claude",
                            "settings.json",
                            "a",
                            "..",
                            "~",
                            "//",
                            "x.y",
                            "push.d",
                        ],
                        5,
                    )
                };
                // No hooks test (`cmd stamptest`): since B355 it is one
                // command on this side and three the Swift code never
                // joined, so the Swift side has nothing to answer it with
                // (it stopped the run: "bad cmd stamptest"). The difference
                // is documented (stamp-test-restores) and its minimal cases
                // are divergence_cases'; the draws went to the quoting the
                // test's commands were here for.
                match rng.below(13) {
                    0 => format!(
                        "cmd write {} {} {}",
                        out(&path(&mut rng)),
                        out(rng.pick(&["600", "644", "755"])),
                        out(&base64::encode(value(&mut rng).as_bytes()))
                    ),
                    1 => format!("cmd read {}", out(&path(&mut rng))),
                    2 => format!("cmd sha {}", out(&path(&mut rng))),
                    3 => format!("cmd backup {}", out(&path(&mut rng))),
                    4..=6 => format!("cmd quote {}", out(&value(&mut rng).replace('\u{301}', ""))),
                    7 => format!("cmd remove {}", out(&path(&mut rng))),
                    8 => format!("cmd prune {} {}", out(&path(&mut rng)), rng.below(4)),
                    9 => format!("cmd quote {}", out(&value(&mut rng).replace('\u{301}', ""))),
                    10 => {
                        let candidate = if rng.chance(50) {
                            path(&mut rng)
                        } else {
                            value(&mut rng)
                        };
                        format!("cmd safe {}", out(&candidate))
                    }
                    11 => {
                        let key = rng
                            .pick(&["stamp", "sender", "hooks.claude", "pairing.C"])
                            .to_string();
                        format!("cmd path {} {}", out(&value(&mut rng)), out(&key))
                    }
                    _ => format!(
                        "cmd quote {}",
                        out(&"'".repeat(usize::try_from(rng.below(4)).expect("small")))
                    ),
                }
            }
            _ => flow_case(&mut rng),
        };
        cases.push(case);
    }
    cases
}

fn flow_case(rng: &mut SplitMix64) -> String {
    let all_tools = "home=/Users/dev\nshell=/bin/zsh\nuname=Darwin\nopenssl=yes\ncurl=yes\njq=yes\ntmux=yes\npython3=yes\nbase64=yes";
    let preflight = if rng.chance(85) {
        all_tools.to_owned()
    } else {
        preflight_output(rng)
    };
    // An installed manifest: current scripts, old ones, hooks, pairings.
    let manifest = match rng.below(5) {
        0 => "-".to_owned(),
        1 => out(rng.pick(&["{not json", "", "{\"schema\":99,\"components\":{}}"])),
        _ => {
            let mut manifest = InstallManifest::empty();
            let at = Date::from_unix(1_700_000_000.0);
            for component in [InstallComponent::Stamp, InstallComponent::Sender] {
                match rng.below(3) {
                    0 => {}
                    1 => manifest.set(
                        &component,
                        Some(Component {
                            digest: scripts::digest(&component),
                            ..Component::new(at)
                        }),
                    ),
                    _ => manifest.set(
                        &component,
                        Some(Component {
                            digest: Some("old".into()),
                            ..Component::new(at)
                        }),
                    ),
                }
            }
            for agent in ["claude", "codex", "gemini"] {
                if rng.chance(40) {
                    let path = if rng.chance(20) {
                        "$HOME/dotfiles/x.json"
                    } else {
                        HookAgent::named(agent).expect("agent").config_path
                    };
                    let path = if rng.chance(5) { "$HOME/x;rm" } else { path };
                    manifest.set(
                        &InstallComponent::Hooks {
                            agent: agent.into(),
                        },
                        Some(Component {
                            config_path: Some(path.into()),
                            ..Component::new(at)
                        }),
                    );
                }
            }
            if rng.chance(40) {
                manifest.set(
                    &InstallComponent::Pairing {
                        conn: "OTHER".into(),
                        install: None,
                    },
                    Some(Component {
                        connection_id: Some("OTHER".into()),
                        relay_url: Some("r".into()),
                        ..Component::new(at)
                    }),
                );
            }
            out(&manifest.to_json())
        }
    };
    let checksums = *rng.pick(&["ok", "ok", "ok", "ok", "wrong", "empty"]);
    let agent = *rng.pick(&HookAgent::ALL);
    let config = if agent.format == Format::CodexToml {
        toml_config(rng)
    } else {
        json_config(rng)
    };
    let (path, config) = if rng.chance(80) {
        (out(agent.config_path), out(&config))
    } else {
        ("-".to_owned(), "-".to_owned())
    };
    let head = format!(
        "{preflight} {manifest} {path} {config} {checksums}",
        preflight = out(&preflight)
    );
    match rng.below(7) {
        0 | 1 => format!(
            "flow pairing {head} {}",
            hex::encode(pairing_json(rng).as_bytes())
        ),
        2 | 3 => format!("flow hooks {head} {}", agent.id),
        4 => format!("flow unhooks {head} {}", agent.id),
        5 => {
            let keys: Vec<String> = (0..1 + rng.below(3))
                .map(|_| {
                    out(rng.pick(&[
                        "stamp",
                        "sender",
                        "hooks.claude",
                        "pairing.OTHER",
                        "pairing.MINE",
                    ]))
                })
                .collect();
            format!("flow uninstall {head} {}", keys.join(","))
        }
        _ => match rng.below(3) {
            0 => format!(
                "flow stamp {head} {} {} {}",
                out(rng.pick(&["%3", "%0", "%12"])),
                out(rng.pick(&["/private/tmp/tmux-501/default", "", "/tmp/a b"])),
                out(rng.pick(&[
                    "working|moshpit-selftest|",
                    "||",
                    "working|claude|x",
                    "",
                    "working|moshpit-selftest",
                    "working\u{301}|moshpit-selftest|"
                ]))
            ),
            1 => format!(
                "flow push {head} {}",
                out(&format!("selftest-{:012x}", rng.below(1 << 48)))
            ),
            _ => format!("flow status {head} -"),
        },
    }
}

/// One minimal case per documented divergence, each after a `# <id>: …`
/// comment line.
pub fn divergence_cases() -> Vec<String> {
    let all_tools = "home=/Users/dev\nshell=/bin/zsh\nuname=Darwin\nopenssl=yes\ncurl=yes\njq=yes\ntmux=yes\npython3=yes\nbase64=yes";
    let mut cases = vec!["# quote-combining: an apostrophe carrying a combining mark".to_owned()];
    cases.push(format!("cmd quote {}", out("it'\u{301}s")));
    cases.push(format!("cmd quote {}", out("x'\u{301}; touch /tmp/p; '")));
    cases.push(format!(
        "cmd stamptest {} {}",
        out("%3'\u{301}; id; '"),
        out("/tmp/s")
    ));
    cases.push("# clear-stamp-quoting: a pane string that is not inert".to_owned());
    cases.push(format!(
        "cmd stamptest {} {}",
        out("%3; touch x"),
        out("/tmp/s")
    ));
    cases.push(format!("cmd stamptest {} {}", out("a b"), out("/tmp/s")));
    cases.push("# facts-crlf: preflight output with CR LF line ends".to_owned());
    cases.push(format!("facts {}", out(&all_tools.replace('\n', "\r\n"))));
    cases.push(format!(
        "flow pairing {} - - - ok {}",
        out(&all_tools.replace('\n', "\r\n")),
        hex::encode(
            br#"{"connectionId":"3F2504E0-4F89-11D3-9A0C-0305E82C3301","hostLabel":"h","secretHex":"ab","sendToken":"t","relayURL":"r","createdAt":0,"apnsToken":"ef","sendTokenIssuedAt":1}"#
        )
    ));
    cases.push(
        "# hooks-array-elements: an event array holding something that is not an object".to_owned(),
    );
    let mixed = r#"{"hooks":{"Stop":["note",{"hooks":[{"type":"command","command":"mine.sh"}]}]}}"#;
    cases.push(format!("merge claude {}", out(mixed)));
    let merged_mixed = hooks::merged_json(mixed, "claude").expect("merges");
    cases.push(format!("remove claude {}", out(&merged_mixed)));
    cases.push("# toml-crlf: a Codex config whose line ends are CR LF".to_owned());
    let crlf = hooks::merged_toml("model = \"gpt-5\"\n", "codex").replace('\n', "\r\n");
    cases.push(format!("merge codex {}", out(&crlf)));
    cases.push(format!("remove codex {}", out(&crlf)));
    cases.push(
        "# negative-overflow: a config number Foundation reads as negative infinity (Swift crashed on it before B106)"
            .to_owned(),
    );
    cases.push(format!("merge claude {}", out(r#"{"x":-1e400}"#)));
    cases.push(format!("remove claude {}", out(r#"{"x":-1e400}"#)));
    cases.push(
        "# atomic-write: a file written whole beside its target, which it then replaces (B351)"
            .to_owned(),
    );
    cases.push(format!(
        "cmd write {} {} {}",
        out("$HOME/.claude/settings.json"),
        out("644"),
        out("eyJhIjoxfQo=")
    ));
    cases.push("# config-mode: an agent config written keeping the mode it has (B352)".to_owned());
    cases.push(format!(
        "cmd writekeep {} {} {}",
        out("$HOME/.claude/settings.json"),
        out("644"),
        out("eyJhIjoxfQo=")
    ));
    cases.push(
        "# self-test-scope: the push self-test for one connection's pairing (B353)".to_owned(),
    );
    cases.push(format!(
        "cmd pushtest {} {}",
        out("selftest-ab12"),
        out(FLOW_CONN)
    ));
    cases.push(
        "# hooks-shared-group: a hook the person added to one of our groups (B356)".to_owned(),
    );
    let shared = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"sh ~/.moshpit/moshpit-stamp.sh done claude"},{"type":"command","command":"say done"}]}]}}"#;
    cases.push(format!("merge claude {}", out(shared)));
    cases.push(format!("remove claude {}", out(shared)));
    cases.push(
        "# stamp-test-restores: the hooks' self-test puts the pane's stamps back (B355)".to_owned(),
    );
    cases.push(format!("cmd stamptest {} {}", out("%3"), out("/tmp/s")));
    cases
        .push("# agent-probe: the preflight also says which agents the host has (B374)".to_owned());
    cases.push("cmd preflight".to_owned());
    cases.push(
        "# nothing-will-push: paired, the stamp script current, no agent's hooks (B374)".to_owned(),
    );
    cases.push("flow status 686f6d653d2f55736572732f6465760a7368656c6c3d2f62696e2f7a73680a756e616d653d44617277696e0a6f70656e73736c3d7965730a6375726c3d7965730a6a713d7965730a746d75783d7965730a707974686f6e333d7965730a6261736536343d796573 7b0a202022636f6d706f6e656e747322203a207b0a202020202270616972696e672e4f5448455222203a207b0a20202020202022636f6e6e656374696f6e496422203a20224f54484552222c0a20202020202022696e7374616c6c6564417422203a2022323032332d31312d31345432323a31333a32305a222c0a2020202020202272656c617955524c22203a202272220a202020207d2c0a202020202273656e64657222203a207b0a2020202020202264696765737422203a20226f6c64222c0a20202020202022696e7374616c6c6564417422203a2022323032332d31312d31345432323a31333a32305a220a202020207d2c0a20202020227374616d7022203a207b0a2020202020202264696765737422203a202231666465636433373833303330333238393966666132643065653365373034323833336566323837363737666438666435636165353932666437353431326661222c0a20202020202022696e7374616c6c6564417422203a2022323032332d31312d31345432323a31333a32305a220a202020207d0a20207d2c0a202022736368656d6122203a20310a7d 24484f4d452f2e636f6465782f636f6e6669672e746f6d6c 5b5b686f6f6b732e53746f705d5d0a5b746f6f6c735d ok -".to_owned());
    cases.push(
        "# conf-iat-overflow: a mint date beyond Int (Swift trapped on it before B112)".to_owned(),
    );
    cases.push(format!(
        "conf {}",
        hex::encode(br#"{"connectionId":"3F2504E0-4F89-11D3-9A0C-0305E82C3301","hostLabel":"h","secretHex":"ab","sendToken":"t","relayURL":"r","createdAt":0,"sendTokenIssuedAt":1e300}"#)
    ));
    cases
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masking_hides_only_the_dates() {
        assert_eq!(
            masked(
                "{\n  \"installedAt\" : \"2025-11-10T18:40:00Z\",\n  \"x\" : \"installedAt\"\n}"
            ),
            "{\n  \"installedAt\" : \"*\",\n  \"x\" : \"installedAt\"\n}"
        );
    }

    #[test]
    fn generation_is_seeded() {
        assert_eq!(generate(3, 300), generate(3, 300));
    }
}
