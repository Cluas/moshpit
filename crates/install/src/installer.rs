//! Putting Moshpit's pieces on a host and saying honestly whether it worked
//! (`HostInstaller`), over any way of running one command on it.
//!
//! Every write is confirmed by reading the file's digest back; the push
//! path is proven by firing it; what landed is recorded in the manifest,
//! content-addressed. The host needs only `cat`, `base64 -d` and a SHA-256
//! tool: configs are merged here, not there.

use std::future::Future;

use moshpit_seal::foundation::base64;
use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::text;
use moshpit_seal::pairing::PushPairing;

use crate::commands;
use crate::facts::{AgentFacts, HostFacts};
use crate::hooks::{self, HookAgent};
use crate::manifest::{Component, InstallComponent, InstallManifest};
use crate::scripts;

/// Why a command could not even run (the channel failed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelError(pub String);

impl std::fmt::Display for ChannelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ChannelError {}

/// A way to run one command on a host and read its stdout (`HostChannel`):
/// an SSH exec channel in the app, a fake host in tests.
pub trait HostChannel {
    fn run(&self, command: &str) -> impl Future<Output = Result<Vec<u8>, ChannelError>>;
}

/// stdout as text: invalid UTF-8 replaced, trimmed of white space and
/// newlines (`runText`).
pub async fn run_text<C: HostChannel>(channel: &C, command: &str) -> Result<String, ChannelError> {
    let bytes = channel.run(command).await?;
    Ok(text::trim_white_space(&String::from_utf8_lossy(&bytes)).to_owned())
}

/// Whether a component is on the host and current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentStatus {
    Absent,
    Current,
    /// Installed, but not the content this build ships.
    Stale {
        installed: String,
    },
}

/// Facts plus manifest, and the question the UI keeps asking.
#[derive(Debug, Clone, PartialEq)]
pub struct InstallState {
    pub facts: HostFacts,
    pub manifest: InstallManifest,
}

impl InstallState {
    pub fn status(&self, component: &InstallComponent) -> ComponentStatus {
        let Some(recorded) = self.manifest.get(component) else {
            return ComponentStatus::Absent;
        };
        let Some(want) = scripts::digest(component) else {
            // A hook registration or a pairing: its presence is all there
            // is, unless a look found a registration this build no longer
            // writes (B381).
            if recorded.digest.as_deref() == Some(crate::drift::OUTDATED_REGISTRATION) {
                return ComponentStatus::Stale {
                    installed: crate::drift::OUTDATED_REGISTRATION.to_owned(),
                };
            }
            return ComponentStatus::Current;
        };
        // A newer build's scripts are left as they are (B633): this build
        // has nothing to bring them up to.
        if recorded
            .scripts_version
            .is_some_and(|installed| installed > scripts::SCRIPTS_VERSION)
        {
            return ComponentStatus::Current;
        }
        match &recorded.digest {
            None => ComponentStatus::Stale {
                installed: "unknown".to_owned(),
            },
            Some(have) if *have == want => ComponentStatus::Current,
            Some(have) => ComponentStatus::Stale {
                installed: have.clone(),
            },
        }
    }

    /// The worse of an agent's registration and the stamp script it calls:
    /// a registration is worthless without the script.
    pub fn hooks_status(&self, agent: &str) -> ComponentStatus {
        let registration = self.status(&InstallComponent::Hooks {
            agent: agent.to_owned(),
        });
        if registration == ComponentStatus::Absent {
            return ComponentStatus::Absent;
        }
        match self.status(&InstallComponent::Stamp) {
            ComponentStatus::Current => registration,
            ComponentStatus::Absent => ComponentStatus::Absent,
            stale => stale,
        }
    }

    /// A paired host that will push nothing: its stamp script is missing
    /// or out of date, or no agent's hooks are current there.
    ///
    /// Fix (nothing-will-push, B374): the Swift original looks at the stamp
    /// script alone, so a host whose last agent's hooks were removed, still
    /// paired and with the script in place, never said that nothing would
    /// come from it.
    pub fn paired_but_nothing_will_push(&self) -> bool {
        !self.manifest.pairing_entries().is_empty()
            && (self.status(&InstallComponent::Stamp) != ComponentStatus::Current
                || !HookAgent::ALL
                    .iter()
                    .any(|agent| self.hooks_status(agent.id) == ComponentStatus::Current))
    }
}

/// The version a script this build writes is ([`scripts::SCRIPTS_VERSION`]);
/// `None` for what isn't one of the two scripts.
fn scripts_version(component: &InstallComponent) -> Option<u32> {
    matches!(
        component,
        InstallComponent::Stamp | InstallComponent::Sender
    )
    .then_some(scripts::SCRIPTS_VERSION)
}

/// What happened to one component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Installed { digest: String },
    AlreadyCurrent,
    Removed,
    NotApplicable,
    Failed(StepFailure),
}

/// Why a step failed, typed so a screen can say it in its own language.
/// Its `Display` is the English the Swift installer wrote, byte for byte
/// (`failures()`, the equivalence harness).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepFailure {
    /// A file landed with another digest, or could not be read back
    /// (`verifyFailure`: English in the app as well).
    Verify {
        path: String,
        want: String,
        got: String,
    },
    /// The agent's config could not be merged or unmerged.
    Config(hooks::Failure),
    /// The host's manifest names a config path no command is run against.
    UnsafePath(String),
}

impl std::fmt::Display for StepFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Verify { path, want, got } => f.write_str(&verify_failure(path, want, got)),
            Self::Config(failure) => failure.fmt(f),
            Self::UnsafePath(path) => write!(
                f,
                "The manifest on this host names a config path Moshpit won't run a command against: {path}"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallStep {
    pub component: InstallComponent,
    pub action: Action,
}

impl InstallStep {
    pub fn is_failure(&self) -> bool {
        matches!(self.action, Action::Failed(_))
    }
}

/// Why nothing was attempted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocked {
    MissingTools(Vec<String>),
}

/// Everything that happened, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct InstallReport {
    pub facts: HostFacts,
    pub steps: Vec<InstallStep>,
    pub manifest: Option<InstallManifest>,
    pub blocked: Option<Blocked>,
    /// The install landed but will do nothing until the user acts on the
    /// host (Codex's hook trust).
    pub user_action_required: Option<&'static str>,
}

impl InstallReport {
    fn new(facts: &HostFacts) -> Self {
        Self {
            facts: facts.clone(),
            steps: Vec::new(),
            manifest: None,
            blocked: None,
            user_action_required: None,
        }
    }

    pub fn succeeded(&self) -> bool {
        self.blocked.is_none() && !self.steps.iter().any(InstallStep::is_failure)
    }

    pub fn failures(&self) -> Vec<String> {
        self.failure_reasons().map(ToString::to_string).collect()
    }

    /// Why each failed step failed, in order.
    pub fn failure_reasons(&self) -> impl Iterator<Item = &StepFailure> {
        self.steps.iter().filter_map(|step| match &step.action {
            Action::Failed(why) => Some(why),
            _ => None,
        })
    }
}

/// What the stamp self-test found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampProof {
    Stamped,
    DidNotStamp(String),
    /// The host's tmux is not on the default socket (or absent).
    Unavailable,
}

/// What the Host Setup page's hook test found (B380): an agent's own
/// registration, run as its hook would run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookProof {
    /// The pane took the stamp the registered command set.
    Stamped,
    DidNotStamp(String),
    /// The host's tmux is not on the default socket (or absent).
    Unavailable,
    /// The config holds no Moshpit command for `event` (one this build
    /// registers for the agent).
    NotRegistered {
        event: String,
    },
    /// The config couldn't be read in its format.
    Unreadable(hooks::Failure),
    /// The manifest names a config path no command is run against.
    UnsafePath(String),
}

/// Whether what a stamp test read back is the test's stamp: `working`,
/// labelled as the test.
fn is_test_stamp(read: &str) -> bool {
    // `split(separator: "|", omittingEmptySubsequences: false)`, by
    // Character.
    let mut fields = Vec::new();
    let mut start = 0;
    let mut at = 0;
    for character in text::characters(read) {
        if character == "|" {
            fields.push(&read[start..at]);
            start = at + 1;
        }
        at += character.len();
    }
    fields.push(&read[start..]);
    fields.len() >= 2 && fields[0] == "working" && fields[1] == commands::SELF_TEST_AGENT
}

/// `push.d/<conn>.conf`, exactly as the sender parses it
/// (`HostInstaller.pushConf`). The only copy of a pairing secret that leaves
/// the phone; its digest decides whether a host's copy is rewritten.
///
/// Fix (conf-iat-overflow): Swift writes `SEND_IAT` with
/// `Int(timeIntervalSince1970)` (`HostInstaller.swift:159`), which traps on
/// a date beyond `Int`'s range read from a damaged pairings file; here it
/// saturates.
pub fn push_conf(pairing: &PushPairing) -> String {
    push_conf_as(pairing, false)
}

/// [`push_conf`] with the phone's notifications switched off (`muted`,
/// B700): a last line `NOTIFY=off`, and the sender sends nothing on it.
/// On, the conf is as it always was, so a host paired before is not
/// rewritten for it.
pub fn push_conf_as(pairing: &PushPairing, muted: bool) -> String {
    let iat = pairing
        .send_token_issued_at
        .unwrap_or(Date::from_unix(0.0))
        .unix()
        .trunc() as i64;
    let mut conf = format!(
        "RELAY_URL={}\nSEND_TOKEN={}\nSECRET={}\nCONN={}\nAPNS_TOKEN={}\nAPNS_ENV={}\nSEND_IAT={iat}\n",
        pairing.relay_url,
        pairing.send_token,
        pairing.secret_hex,
        pairing.connection_id,
        pairing.apns_token.as_deref().unwrap_or(""),
        pairing.apns_env.as_deref().unwrap_or("production"),
    );
    if muted {
        conf.push_str("NOTIFY=off\n");
    }
    conf
}

/// A nonce the phone can match a self-test push against.
pub fn make_nonce() -> String {
    format!("selftest-{}", moshpit_seal::random::hex(6))
}

fn verify(path: &str, want: &str, got: &str) -> StepFailure {
    StepFailure::Verify {
        path: path.to_owned(),
        want: want.to_owned(),
        got: got.to_owned(),
    }
}

fn verify_failure(path: &str, want: &str, got: &str) -> String {
    if got.is_empty() {
        return format!(
            "wrote {path} but could not read it back \u{2014} check the home directory is writable"
        );
    }
    format!(
        "{path} landed with a different checksum (wanted {}\u{2026}, got {}\u{2026})",
        text::character_prefix(want, 8),
        text::character_prefix(got, 8)
    )
}

/// The installer over one channel.
pub struct HostInstaller<'a, C: HostChannel> {
    pub channel: &'a C,
    /// The phone's notifications are off (B700): a pairing it writes says
    /// so ([`push_conf_as`]).
    pub muted: bool,
}

impl<'a, C: HostChannel> HostInstaller<'a, C> {
    pub fn new(channel: &'a C) -> Self {
        Self { channel, muted: false }
    }

    /// This installer, writing pairings `muted` or not.
    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    async fn run(&self, command: &str) -> Result<Vec<u8>, ChannelError> {
        self.channel.run(command).await
    }

    async fn text(&self, command: &str) -> Result<String, ChannelError> {
        run_text(self.channel, command).await
    }

    /// A file as the host holds it, untrimmed: an agent config is edited
    /// in place (B375), so its last newline is the person's to keep.
    async fn file(&self, path: &str) -> Result<String, ChannelError> {
        let bytes = self.run(&commands::read_file(path)).await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// What this host can do, in one round trip.
    pub async fn preflight(&self) -> Result<HostFacts, ChannelError> {
        Ok(HostFacts::parse(&self.text(commands::PREFLIGHT).await?))
    }

    /// Facts plus what is already installed.
    pub async fn inspect(&self) -> Result<InstallState, ChannelError> {
        let facts = self.preflight().await?;
        let manifest = InstallManifest::from_text(
            &self
                .text(&commands::read_file(InstallManifest::PATH))
                .await?,
        );
        Ok(InstallState { facts, manifest })
    }

    /// Change the host's manifest by `change` alone (B373): under the host's
    /// manifest lock, read it as it is now, change it, write it, let go. A
    /// step used to write back the whole manifest as it read it when it
    /// began, erasing whatever another phone recorded in the meantime.
    pub async fn update_manifest(
        &self,
        change: impl FnOnce(&mut InstallManifest),
    ) -> Result<InstallManifest, ChannelError> {
        let token = moshpit_seal::random::hex(8);
        self.run(&commands::lock(&token)).await?;
        let result = async {
            let mut manifest = InstallManifest::from_text(
                &self
                    .text(&commands::read_file(InstallManifest::PATH))
                    .await?,
            );
            change(&mut manifest);
            self.write_manifest(&manifest).await?;
            Ok(manifest)
        }
        .await;
        let _ = self.run(&commands::unlock(&token)).await;
        result
    }

    async fn write_manifest(&self, manifest: &InstallManifest) -> Result<(), ChannelError> {
        let data = manifest.to_json();
        self.run(&commands::write_file(
            InstallManifest::PATH,
            "644",
            &base64::encode(data.as_bytes()),
        ))
        .await
        .map(|_| ())
    }

    /// `state` as the host actually holds it (B315): one more command
    /// looks at the files the manifest records — the scripts, `conn`'s push
    /// conf — and the agent configs it names ([`crate::drift`]); what was
    /// deleted or edited by hand reads so.
    pub async fn verify(
        &self,
        state: InstallState,
        conn: Option<&str>,
    ) -> Result<(InstallState, Vec<crate::drift::Drift>), ChannelError> {
        let checks = crate::drift::checks(&state.manifest, conn);
        let Some(command) = crate::drift::command(&checks) else {
            return Ok((state, Vec::new()));
        };
        let output = self.text(&command).await?;
        Ok(crate::drift::reconcile(&state, &checks, &output))
    }

    /// Write one of the fixed scripts unless the host already has exactly
    /// this content, and verify it by digest.
    pub async fn install_script(
        &self,
        component: &InstallComponent,
        state: &InstallState,
    ) -> Result<InstallStep, ChannelError> {
        let (Some(body), Some(path), Some(want)) = (
            scripts::body(component),
            component.path(),
            scripts::digest(component),
        ) else {
            return Ok(InstallStep {
                component: component.clone(),
                action: Action::NotApplicable,
            });
        };
        if state.status(component) == ComponentStatus::Current {
            return Ok(InstallStep {
                component: component.clone(),
                action: Action::AlreadyCurrent,
            });
        }
        self.run(&commands::write_file(
            &path,
            component.mode(),
            &base64::encode(body.as_bytes()),
        ))
        .await?;
        let got = self.text(&commands::sha256(&path)).await?;
        let action = if got == want {
            Action::Installed { digest: want }
        } else {
            Action::Failed(verify(&path, &want, &got))
        };
        Ok(InstallStep {
            component: component.clone(),
            action,
        })
    }

    /// Install (or repair) everything a push pairing needs, and record it:
    /// the sender, then the stamp script, and only once both landed the
    /// pairing's conf — a host never holds secrets with no program to use
    /// them. `now` stamps the manifest. The legacy conf, `<conn>.conf`:
    /// [`install_pairing_as`](Self::install_pairing_as) names an install.
    pub async fn install_pairing(
        &self,
        pairing: &PushPairing,
        state: &InstallState,
        now: Date,
    ) -> Result<InstallReport, ChannelError> {
        self.install_pairing_as(pairing, None, state, now).await
    }

    /// [`install_pairing`](Self::install_pairing) for install `install`'s
    /// own conf, `<conn>.<install>.conf` (B443). The legacy `<conn>.conf`
    /// it replaces goes with it, but only when its recorded digest is this
    /// very conf's — what this phone wrote before installs were told apart.
    /// Another digest is another phone's (a phone restored from this one's
    /// backup) and is left as it is.
    pub async fn install_pairing_as(
        &self,
        pairing: &PushPairing,
        install: Option<&str>,
        state: &InstallState,
        now: Date,
    ) -> Result<InstallReport, ChannelError> {
        let mut report = InstallReport::new(&state.facts);
        if !state.facts.can_push() {
            report.blocked = Some(Blocked::MissingTools(state.facts.missing_for_push()));
            return Ok(report);
        }
        report.steps.push(
            self.install_script(&InstallComponent::Sender, state)
                .await?,
        );
        report
            .steps
            .push(self.install_script(&InstallComponent::Stamp, state).await?);
        if report.steps.iter().any(InstallStep::is_failure) {
            return Ok(report);
        }
        let conn = pairing.connection_id.to_string();
        let install = install.filter(|id| crate::manifest::is_install_id(id));
        let component = InstallComponent::Pairing {
            conn: conn.clone(),
            install: install.map(str::to_owned),
        };
        let legacy = InstallComponent::Pairing {
            conn: conn.clone(),
            install: None,
        };
        let conf = push_conf_as(pairing, self.muted);
        let path = component.path().expect("a pairing is a file");
        self.run(&commands::write_file(
            &path,
            component.mode(),
            &base64::encode(conf.as_bytes()),
        ))
        .await?;
        let want = scripts::content_digest(&conf);
        let got = self.text(&commands::sha256(&path)).await?;
        if got != want {
            report.steps.push(InstallStep {
                component,
                action: Action::Failed(verify(&path, &want, &got)),
            });
            return Ok(report);
        }
        report.steps.push(InstallStep {
            component,
            action: Action::Installed { digest: want },
        });
        let steps = report.steps.clone();
        let mut migrated = false;
        let this_conf = scripts::content_digest(&conf);
        let manifest = self
            .update_manifest(|manifest| {
                // This phone's legacy conf moves to its install's (B443):
                // only the one that is this very conf.
                if install.is_some()
                    && manifest.get(&legacy).and_then(|c| c.digest.as_deref()) == Some(this_conf.as_str())
                {
                    manifest.set(&legacy, None);
                    migrated = true;
                }
                for step in &steps {
                    let Action::Installed { digest } = &step.action else {
                        continue;
                    };
                    let is_pairing = matches!(step.component, InstallComponent::Pairing { .. });
                    manifest.set(
                        &step.component,
                        Some(Component {
                            digest: Some(digest.clone()),
                            connection_id: is_pairing.then(|| conn.clone()),
                            relay_url: is_pairing.then(|| pairing.relay_url.clone()),
                            scripts_version: scripts_version(&step.component),
                            ..Component::new(now)
                        }),
                    );
                }
            })
            .await?;
        if migrated {
            let path = legacy.path().expect("a pairing is a file");
            self.run(&commands::remove_file(&path)).await?;
            report.steps.push(InstallStep {
                component: legacy,
                action: Action::Removed,
            });
        }
        report.manifest = Some(manifest);
        Ok(report)
    }

    /// Register Moshpit's hooks in one agent's config: the stamp script and
    /// the sender brought current, the config read, merged here and written
    /// back, a pre-Moshpit copy kept once, the old installer's backups
    /// pruned. A config that will not parse is reported, never overwritten.
    pub async fn install_hooks(
        &self,
        agent: &HookAgent,
        state: &InstallState,
        now: Date,
    ) -> Result<InstallReport, ChannelError> {
        let mut report = InstallReport::new(&state.facts);
        report
            .steps
            .push(self.install_script(&InstallComponent::Stamp, state).await?);
        report.steps.push(
            self.install_script(&InstallComponent::Sender, state)
                .await?,
        );
        if report.steps.iter().any(InstallStep::is_failure) {
            return Ok(report);
        }
        let component = InstallComponent::Hooks {
            agent: agent.id.to_owned(),
        };
        // The file the agent reads its hooks from (B385), read untrimmed:
        // only Moshpit's part of it changes (B375).
        let path = self.hooks_path(agent).await?;
        let existing = self.file(path).await?;
        let merged = match hooks::merged_at(&existing, agent, path) {
            Ok(merged) => merged,
            Err(error) => {
                report.steps.push(InstallStep {
                    component,
                    action: Action::Failed(StepFailure::Config(error)),
                });
                return Ok(report);
            }
        };
        self.run(&commands::backup_once(path)).await?;
        // The person's own file: it keeps its mode (B352).
        self.run(&commands::write_keeping_mode(
            path,
            "644",
            &base64::encode(merged.as_bytes()),
        ))
        .await?;
        let want = scripts::content_digest(&merged);
        let got = self.text(&commands::sha256(path)).await?;
        if got != want {
            report.steps.push(InstallStep {
                component,
                action: Action::Failed(verify(path, &want, &got)),
            });
            return Ok(report);
        }
        // Registered in the hooks file, ours come out of the config, whose
        // hooks the agent no longer reads (B385).
        if path != agent.config_path {
            self.strip_ours(agent, agent.config_path).await?;
        }
        report.steps.push(InstallStep {
            component: component.clone(),
            action: Action::Installed { digest: want },
        });
        // A registration that changed hasn't run yet: an agent that skips a
        // changed hook until it is trusted (Codex) must be seen running it
        // again before the page calls the trust step done (B380).
        if agent.trust_step().is_some()
            && text::trim_white_space(&merged) != text::trim_white_space(&existing)
        {
            self.run(&commands::forget_seen(agent.id)).await?;
        }
        report.user_action_required = agent.trust_step();
        self.run(&commands::prune_backups(path, 1)).await?;
        let steps = report.steps.clone();
        let manifest = self
            .update_manifest(|manifest| {
                for step in &steps {
                    let Action::Installed { digest } = &step.action else {
                        continue;
                    };
                    manifest.set(
                        &step.component,
                        Some(Component {
                            digest: Some(digest.clone()),
                            config_path: (step.component == component).then(|| path.to_owned()),
                            scripts_version: scripts_version(&step.component),
                            ..Component::new(now)
                        }),
                    );
                }
            })
            .await?;
        report.manifest = Some(manifest);
        Ok(report)
    }

    /// Where `agent` reads its hooks from: its hooks file when it has one
    /// and the host has it (droid reads `~/.factory/hooks.json` instead of
    /// its `settings.json`'s hooks then, B385), else its config.
    async fn hooks_path(&self, agent: &HookAgent) -> Result<&'static str, ChannelError> {
        if let Some(file) = agent.hooks_file
            && self.text(&commands::file_exists(file)).await? == "yes"
        {
            return Ok(file);
        }
        Ok(agent.config_path)
    }

    /// Take Moshpit's entries out of the file at `path` if it holds any;
    /// one that won't parse is left alone.
    async fn strip_ours(&self, agent: &HookAgent, path: &str) -> Result<(), ChannelError> {
        let existing = self.file(path).await?;
        if !text::contains_characters(&existing, hooks::MARKER) {
            return Ok(());
        }
        let Ok(stripped) = hooks::removed_at(&existing, agent, path) else {
            return Ok(());
        };
        self.run(&commands::write_keeping_mode(
            path,
            "644",
            &base64::encode(stripped.as_bytes()),
        ))
        .await
        .map(|_| ())
    }

    /// Take Moshpit's hooks back out of the config the install edited (the
    /// manifest's path, validated) — surgically, keeping the user's edits.
    pub async fn uninstall_hooks(
        &self,
        agent: &HookAgent,
        state: &InstallState,
    ) -> Result<InstallReport, ChannelError> {
        let mut report = InstallReport::new(&state.facts);
        let component = InstallComponent::Hooks {
            agent: agent.id.to_owned(),
        };
        let path = state
            .manifest
            .get(&component)
            .and_then(|c| c.config_path.clone())
            .unwrap_or_else(|| agent.config_path.to_owned());
        if !commands::is_safe_path(&path) {
            report.steps.push(InstallStep {
                component,
                action: Action::Failed(StepFailure::UnsafePath(path)),
            });
            return Ok(report);
        }
        let existing = self.file(&path).await?;
        let stripped = match hooks::removed_at(&existing, agent, &path) {
            Ok(stripped) => stripped,
            Err(error) => {
                report.steps.push(InstallStep {
                    component,
                    action: Action::Failed(StepFailure::Config(error)),
                });
                return Ok(report);
            }
        };
        // The person's own file: it keeps its mode (B352).
        self.run(&commands::write_keeping_mode(
            &path,
            "644",
            &base64::encode(stripped.as_bytes()),
        ))
        .await?;
        let manifest = self
            .update_manifest(|manifest| manifest.set(&component, None))
            .await?;
        report.manifest = Some(manifest);
        report.steps.push(InstallStep {
            component,
            action: Action::Removed,
        });
        Ok(report)
    }

    /// The pairings on the host that push to the phone with APNs token
    /// `phone` (B354), as their confs are named: `<conn>.conf` (the legacy
    /// one) or `<conn>.<install>.conf` (B443). A conf with no token, or one
    /// not named so, is no one's to judge.
    pub async fn pairings_reaching(&self, phone: &str) -> Result<Vec<InstallComponent>, ChannelError> {
        if phone.is_empty() {
            return Ok(Vec::new());
        }
        let listing = self.text(commands::PAIRING_PHONES).await?;
        Ok(listing
            .lines()
            .filter_map(|line| {
                let (file, token) = line.split_once(' ')?;
                if !token.eq_ignore_ascii_case(phone) {
                    return None;
                }
                let stem = file.strip_suffix(".conf")?;
                let (conn, install) = match stem.split_once('.') {
                    None => (stem, None),
                    Some((conn, install)) if crate::manifest::is_install_id(install) => (conn, Some(install)),
                    Some(_) => return None,
                };
                moshpit_seal::Uuid::parse(conn)?;
                Some(InstallComponent::Pairing {
                    conn: conn.to_owned(),
                    install: install.map(str::to_owned),
                })
            })
            .collect())
    }

    /// Take Moshpit off this host for connection `conn` (B375): every agent's
    /// hooks (those the manifest records and any config still calling the
    /// stamp script), this phone's pairing, and — once no other pairing is
    /// left there — the scripts, the manifest, the backups beside each
    /// agent config and Moshpit's own directories. Another phone's conf is
    /// never touched; while one is there the scripts and the manifest stay,
    /// as that phone's.
    pub async fn remove_everything(
        &self,
        conn: &str,
        state: &InstallState,
    ) -> Result<InstallReport, ChannelError> {
        let legacy = InstallComponent::Pairing {
            conn: conn.to_owned(),
            install: None,
        };
        self.remove_everything_of(&[legacy], state).await
    }

    /// [`remove_everything`](Self::remove_everything), taking `pairings` —
    /// this phone's, its install's conf and the legacy one when it is this
    /// phone's (B443) — off the host; another phone's stays.
    pub async fn remove_everything_of(
        &self,
        pairings: &[InstallComponent],
        state: &InstallState,
    ) -> Result<InstallReport, ChannelError> {
        let mut report = InstallReport::new(&state.facts);
        let configs: Vec<&str> = HookAgent::ALL.iter().map(|a| a.config_path).collect();
        let calling = self
            .text(&commands::configs_calling_stamp(&configs))
            .await?;
        let calling: Vec<usize> = calling.lines().filter_map(|l| l.parse().ok()).collect();
        let recorded = installed_hook_agents(&state.manifest);
        let mut current = state.clone();
        for (index, agent) in HookAgent::ALL.into_iter().enumerate() {
            if !recorded.contains(&agent) && !calling.contains(&index) {
                continue;
            }
            let removed = self.uninstall_hooks(&agent, &current).await?;
            if let Some(manifest) = removed.manifest.clone() {
                current.manifest = manifest;
            }
            report.steps.extend(removed.steps);
        }
        current.manifest = self.uninstall(pairings, &current).await?;
        for pairing in pairings {
            report.steps.push(InstallStep {
                component: pairing.clone(),
                action: Action::Removed,
            });
        }
        // The rest only once no other pairing is left: read under the
        // manifest's lock, so a phone pairing at this moment is seen.
        let token = moshpit_seal::random::hex(8);
        self.run(&commands::lock(&token)).await?;
        let rest = async {
            let manifest = InstallManifest::from_text(
                &self
                    .text(&commands::read_file(InstallManifest::PATH))
                    .await?,
            );
            // The first builds' push.conf that can't push is no one's
            // pairing (B638).
            self.run(commands::REMOVE_INERT_LEGACY_CONF).await?;
            let left = self.text(commands::PAIRINGS_LEFT).await?;
            if !left.is_empty() || !manifest.pairing_entries().is_empty() {
                return Ok(Some(manifest));
            }
            self.run(&commands::remove_the_rest(&configs)).await?;
            Ok(None)
        }
        .await;
        let _ = self.run(&commands::unlock(&token)).await;
        match rest? {
            Some(manifest) => report.manifest = Some(manifest),
            None => {
                self.run(commands::REMOVE_DIR_IF_EMPTY).await?;
                for component in [InstallComponent::Stamp, InstallComponent::Sender] {
                    report.steps.push(InstallStep {
                        component,
                        action: Action::Removed,
                    });
                }
                report.manifest = Some(InstallManifest::empty());
            }
        }
        Ok(report)
    }

    /// Ask the host to send one real push carrying `nonce`, over connection
    /// `conn`'s pairing alone (B353); the caller starts waiting for it
    /// first.
    pub async fn self_test_push(&self, nonce: &str, conn: &str) -> Result<String, ChannelError> {
        self.text(&commands::self_test_push(nonce, conn)).await
    }

    /// Fire the stamp script and read the pane back, leaving the pane as
    /// it was (B355: one command, [`commands::stamp_test`]).
    pub async fn self_test_stamp(&self, pane: &str) -> Result<StampProof, ChannelError> {
        let socket = self.text(commands::TMUX_SOCKET_PATH).await?;
        if socket.is_empty() {
            return Ok(StampProof::Unavailable);
        }
        let nonce = moshpit_seal::random::hex(8);
        let read = self
            .text(&commands::stamp_test(pane, &socket, &nonce))
            .await?;
        if is_test_stamp(&read) {
            Ok(StampProof::Stamped)
        } else {
            Ok(StampProof::DidNotStamp(read))
        }
    }

    /// Which agents have run Moshpit's hooks here
    /// ([`commands::AGENTS_PROBE`]).
    pub async fn agents(&self) -> Result<AgentFacts, ChannelError> {
        Ok(AgentFacts::parse(&self.text(commands::AGENTS_PROBE).await?))
    }

    /// Run `agent`'s hooks as they stand in its config at `config_path`, in
    /// `pane`, and read the pane back (B380): the config must hold
    /// Moshpit's command for every event this build registers for the
    /// agent, and the `working` one is then fired as a test
    /// (`MOSHPIT_SELFTEST=1`), so its stamp is labelled as the test and leaves
    /// no mark of the agent having run. The pane is left as it was
    /// ([`commands::stamp_test_running`]).
    pub async fn self_test_registration(
        &self,
        agent: &HookAgent,
        config_path: &str,
        pane: &str,
    ) -> Result<HookProof, ChannelError> {
        if !commands::is_safe_path(config_path) {
            return Ok(HookProof::UnsafePath(config_path.to_owned()));
        }
        let existing = self.text(&commands::read_file(config_path)).await?;
        let registered = match hooks::registered_at(&existing, agent, config_path) {
            Ok(registered) => registered,
            Err(failure) => return Ok(HookProof::Unreadable(failure)),
        };
        let mut fired = None;
        for (event, state, _) in agent.events() {
            let want = hooks::command(state, agent.id);
            if !registered.iter().any(|(e, c)| e == event && *c == want) {
                return Ok(HookProof::NotRegistered {
                    event: (*event).to_owned(),
                });
            }
            if fired.is_none() && *state == "working" {
                fired = Some(want);
            }
        }
        let Some(fired) = fired else {
            return Ok(HookProof::NotRegistered {
                event: "working".to_owned(),
            });
        };
        let socket = self.text(commands::TMUX_SOCKET_PATH).await?;
        if socket.is_empty() {
            return Ok(HookProof::Unavailable);
        }
        let nonce = moshpit_seal::random::hex(8);
        let read = self
            .text(&commands::stamp_test_running(&fired, pane, &socket, &nonce))
            .await?;
        if is_test_stamp(&read) {
            Ok(HookProof::Stamped)
        } else {
            Ok(HookProof::DidNotStamp(read))
        }
    }

    /// Remove components and forget them — only what the manifest says was
    /// put there. (`_state`: the host as last read; the manifest itself is
    /// read again under its lock, B373.)
    pub async fn uninstall(
        &self,
        components: &[InstallComponent],
        _state: &InstallState,
    ) -> Result<InstallManifest, ChannelError> {
        for component in components {
            if let Some(path) = component.path() {
                let command = match component {
                    InstallComponent::Pairing { conn, .. } => commands::remove_pairing(&path, conn),
                    _ => commands::remove_file(&path),
                };
                self.run(&command).await?;
            }
        }
        self.update_manifest(|manifest| {
            for component in components {
                manifest.set(component, None);
            }
        })
        .await
    }
}

/// The agents whose hooks this host already carries — what was consented to
/// (`HostAutoCare.installedHookAgents`). Care refreshes only these.
pub fn installed_hook_agents(manifest: &InstallManifest) -> Vec<HookAgent> {
    HookAgent::ALL
        .into_iter()
        .filter(|agent| {
            manifest
                .get(&InstallComponent::Hooks {
                    agent: agent.id.to_owned(),
                })
                .is_some()
        })
        .collect()
}

/// The agents a care pass would re-install, judged on `state` alone
/// (`HostAutoCare.tendNow`'s rule): every consented one when the sender is
/// not current (`hooks_status` folds in the stamp script but not the
/// sender), else those whose hooks are not. A pass judges each agent on the
/// host as it is after the one before (`moshpit-hostcare`'s care), so one
/// agent's refresh bringing the stamp current spares the next.
pub fn agents_to_refresh(state: &InstallState) -> Vec<HookAgent> {
    let sender_stale = state.status(&InstallComponent::Sender) != ComponentStatus::Current;
    installed_hook_agents(&state.manifest)
        .into_iter()
        .filter(|agent| sender_stale || state.hooks_status(agent.id) != ComponentStatus::Current)
        .collect()
}

/// Whether the host's copy of this device's pairing must be rewritten: its
/// recorded digest is not the digest of the conf the phone would write now.
pub fn pairing_needs_rewrite(manifest: &InstallManifest, ready: &PushPairing) -> bool {
    pairing_needs_rewrite_as(manifest, ready, None)
}

/// [`pairing_needs_rewrite`] for install `install`'s own conf (B443): also
/// when this phone's legacy conf is still there to move over.
pub fn pairing_needs_rewrite_as(manifest: &InstallManifest, ready: &PushPairing, install: Option<&str>) -> bool {
    pairing_needs_rewrite_muted(manifest, ready, install, false)
}

/// [`pairing_needs_rewrite_as`] for a conf written `muted` or not (B700):
/// the switch turned is a rewrite too.
pub fn pairing_needs_rewrite_muted(
    manifest: &InstallManifest,
    ready: &PushPairing,
    install: Option<&str>,
    muted: bool,
) -> bool {
    let expected = scripts::content_digest(&push_conf_as(ready, muted));
    let conn = ready.connection_id.to_string();
    let install = install.filter(|id| crate::manifest::is_install_id(id));
    let digest = |install: Option<&str>| manifest.pairing_entry_of(&conn, install).and_then(|c| c.digest.clone());
    digest(install).as_deref() != Some(expected.as_str())
        || (install.is_some() && digest(None).as_deref() == Some(expected.as_str()))
}

