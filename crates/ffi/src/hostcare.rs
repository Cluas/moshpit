//! Host care for the iOS app (UniFFI): the core's machine
//! (`moshpit-hostcare`: care on connect, the Host Setup screen, Install
//! Assist, the phone's side of push pairing) on the core's runtime, with
//! Swift as its platform. Swift keeps what needs iOS: running a command on
//! a connection's host (the session's own connection — `borrowSsh` on the
//! Rust core), the HTTPS request to the relay, the device token and the
//! notification permission, and the App Group file the notification
//! service reads ([`PairingsFile`]).
//!
//! Every call is quick and may come from any thread; the platform's
//! callbacks come on core threads ([`HostCarePlatform::changed`] must hop
//! to the main thread and read the view again).

use std::sync::Arc;

use moshpit_hostcare::driver::{Driver, Platform};
use moshpit_hostcare::{
    AssistPlan, Change, Clock, Config, Conn, HostCare, HttpReply, PairingStore, PushSupport,
    Recheck, Settings,
};
use moshpit_install::{HostCapabilities, PackageManager};
use moshpit_seal::{Date, PushPairing, Uuid};

use crate::store::PairingRecord;

/// Why a platform call failed, in the person's language.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum HostCareFailure {
    Failed { why: String },
}

impl std::fmt::Display for HostCareFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed { why } => f.write_str(why),
        }
    }
}

impl std::error::Error for HostCareFailure {}

impl From<uniffi::UnexpectedUniFFICallbackError> for HostCareFailure {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed { why: error.reason }
    }
}

/// A relay's answer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HttpReplyValue {
    pub status: u16,
    pub body: Vec<u8>,
}

/// What changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HostCareChange {
    /// The consent question.
    Consent,
    /// Host Setup screen `page`.
    Page { page: u64 },
    /// The phone's push side (its relay error, its pairings).
    Push,
}

/// A connection, as care names it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CareConnection {
    pub id: String,
    pub label: String,
}

/// What the machine asks of the app.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait HostCarePlatform: Send + Sync {
    /// Run `command` on connection `connection`'s host — on its live
    /// session's own connection, under `sh -c` — and return its stdout;
    /// fail with why when there is no session, the connection failed, or
    /// the command exited non-zero.
    async fn exec(&self, connection: String, command: String) -> Result<Vec<u8>, HostCareFailure>;
    /// POST `body` (JSON) to `url`; fail only when it didn't reach the
    /// server.
    async fn post(
        &self,
        url: String,
        body: String,
        timeout_seconds: u32,
    ) -> Result<HttpReplyValue, HostCareFailure>;
    /// Ask iOS for a device token: prompt when undecided, register when
    /// allowed, nothing when denied.
    fn register_for_push(&self);
    /// Remember the person's "don't ask again" for `connection`.
    fn remember_declined(&self, connection: String);
    /// Remember (`unpaired`) or forget the person's Unpair of `connection`
    /// in Host Setup: care pairs nothing for it until they pair again.
    fn remember_unpaired(&self, connection: String, unpaired: bool);
    /// A view changed (called on a core thread: hop and read it again).
    fn changed(&self, change: HostCareChange);
}

/// The pairings file the notification service reads (the App Group's
/// `push-pairings.json`), as the app reaches it. Each call keeps
/// `PushPairingFile`'s rules and the file's protection.
#[uniffi::export(with_foreign)]
pub trait PairingsFile: Send + Sync {
    fn read(&self) -> Vec<PairingRecord>;
    /// Replace (or add) one connection's pairing; true once it reads back.
    fn upsert(&self, pairing: PairingRecord) -> bool;
    /// Forget one connection's pairing; true once nothing for it is left.
    fn remove(&self, connection_id: String) -> bool;
}

struct Pairings(Arc<dyn PairingsFile>);

impl PairingStore for Pairings {
    fn read(&self) -> Vec<PushPairing> {
        self.0
            .read()
            .into_iter()
            .filter_map(|r| PushPairing::try_from(r).ok())
            .collect()
    }

    fn upsert(&mut self, pairing: &PushPairing) -> bool {
        self.0.upsert(pairing.into())
    }

    fn remove(&mut self, connection: Uuid) -> bool {
        self.0.remove(connection.to_string())
    }
}

struct Swift(Arc<dyn HostCarePlatform>);

#[async_trait::async_trait]
impl Platform for Swift {
    async fn exec(&self, connection: Uuid, command: String) -> Result<Vec<u8>, String> {
        self.0
            .exec(connection.to_string(), command)
            .await
            .map_err(|e| e.to_string())
    }

    async fn post(&self, url: String, body: String, timeout: u32) -> Result<HttpReply, String> {
        self.0
            .post(url, body, timeout)
            .await
            .map(|r| HttpReply {
                status: r.status,
                body: r.body,
            })
            .map_err(|e| e.to_string())
    }

    fn register_for_push(&self) {
        self.0.register_for_push();
    }

    fn remember_declined(&self, connection: Uuid) {
        self.0.remember_declined(connection.to_string());
    }

    fn remember_unpaired(&self, connection: Uuid, unpaired: bool) {
        self.0.remember_unpaired(connection.to_string(), unpaired);
    }

    fn changed(&self, change: Change) {
        self.0.changed(match change {
            Change::Consent => HostCareChange::Consent,
            Change::Page(page) => HostCareChange::Page { page },
            Change::Push => HostCareChange::Push,
        });
    }
}

fn uuid(text: &str) -> Option<Uuid> {
    Uuid::parse(text)
}

fn uuids(texts: &[String]) -> Vec<Uuid> {
    texts.iter().filter_map(|t| uuid(t)).collect()
}

/// The machine, driven on the core's runtime.
#[derive(uniffi::Object)]
pub struct HostCareCore {
    driver: Driver,
}

#[uniffi::export]
impl HostCareCore {
    /// `environment`: the build's APNs environment (`sandbox`,
    /// `production`); `autocare_off`: a harness's `-MOSHPIT_AUTOCARE_OFF`.
    #[uniffi::constructor]
    pub fn new(
        platform: Arc<dyn HostCarePlatform>,
        pairings: Arc<dyn PairingsFile>,
        environment: String,
        autocare_off: bool,
    ) -> Arc<Self> {
        let clock = Clock {
            ms: 0,
            date: Date::now(),
        };
        let machine = HostCare::new(
            Config {
                push: PushSupport::Apns { environment },
                autocare_off,
                budgets: Default::default(),
            },
            Box::new(Pairings(pairings)),
            clock,
        );
        Arc::new(HostCareCore {
            driver: Driver::new(
                machine,
                Arc::new(Swift(platform)),
                moshpit_runtime::handle(),
            ),
        })
    }

    // MARK: What the app tells it

    /// The settings care reads, as they are now.
    pub fn set_settings(
        &self,
        notifications: bool,
        live_activity: bool,
        relay_url: String,
        declined: Vec<String>,
        unpaired: Vec<String>,
    ) {
        self.driver.with(|m, k| {
            m.set_settings(
                Settings {
                    notifications,
                    live_activity,
                    relay_url,
                    declined,
                    unpaired,
                },
                k,
            )
        });
    }

    /// The saved connections: `listed` the ones the store decoded, `all`
    /// every one it holds (nil while that can't be told).
    pub fn set_saved(&self, listed: Vec<String>, all: Option<Vec<String>>) {
        let listed = uuids(&listed);
        let all = all.map(|a| uuids(&a));
        self.driver
            .with(|m, k| m.set_saved(&listed, all.as_deref(), k));
    }

    /// What the person calls `connection` now: its pairing takes the name
    /// (B451).
    pub fn name_connection(&self, connection: String, label: String) {
        if let Some(id) = uuid(&connection) {
            self.driver.with(|m, k| m.name_connection(id, &label, k));
        }
    }

    // MARK: Care

    /// A session's control plane for `connection` came up.
    pub fn tend(&self, connection: CareConnection) {
        let Some(id) = uuid(&connection.id) else {
            return;
        };
        self.driver.with(|m, k| {
            m.tend(
                Conn {
                    id,
                    label: connection.label,
                },
                k,
            )
        });
    }

    pub fn is_tending(&self, connection: String) -> bool {
        uuid(&connection).is_some_and(|id| self.driver.read(|m| m.is_tending(id)))
    }

    /// The connection whose host has no hooks yet: the one question.
    pub fn consent(&self) -> Option<CareConnection> {
        self.driver.read(|m| m.consent()).map(|c| CareConnection {
            id: c.id.to_string(),
            label: c.label,
        })
    }

    pub fn grant_consent(&self, connection: CareConnection) {
        let Some(id) = uuid(&connection.id) else {
            return;
        };
        self.driver.with(|m, k| {
            m.grant_consent(
                Conn {
                    id,
                    label: connection.label,
                },
                k,
            )
        });
    }

    pub fn decline(&self, connection: String) {
        if let Some(id) = uuid(&connection) {
            self.driver.with(|m, k| m.decline(id, k));
        }
    }

    pub fn dismiss_consent(&self) {
        self.driver.with(|m, k| m.dismiss_consent(k));
    }

    // MARK: Deleting a connection

    /// A delete of `connection` begins: care stands down for it and its
    /// pairing is forgotten on this phone. False: the phone couldn't let
    /// go — nothing else may be removed, and the delete is over.
    pub fn deletion_began(&self, connection: String) -> bool {
        let Some(id) = uuid(&connection) else {
            return true;
        };
        self.driver.with(|m, k| m.deletion_began(id, k)).is_ok()
    }

    /// `connection` was saved pointing at another host or port (B328): its
    /// pairing is forgotten on this phone. False: the phone couldn't let go
    /// — the save must not go on.
    pub fn host_changed(&self, connection: String) -> bool {
        let Some(id) = uuid(&connection) else {
            return true;
        };
        self.driver.with(|m, k| m.host_changed(id, k)).is_ok()
    }

    /// Wait for the care pass the delete stood down to end (at most 5 s).
    pub async fn care_wind_down(&self, connection: String) {
        let Some(id) = uuid(&connection) else {
            return;
        };
        let driver = self.driver.clone();
        // On the core's runtime: tokio's timer needs it.
        let _ = moshpit_runtime::handle()
            .spawn(async move { driver.care_wind_down(id).await })
            .await;
    }

    /// Wait (at most 3 s) for a deleted connection's pairing to come off
    /// its host, before its session goes (B377).
    pub async fn forget_wind_down(&self, connection: String) {
        let Some(id) = uuid(&connection) else {
            return;
        };
        let driver = self.driver.clone();
        // On the core's runtime: tokio's timer needs it.
        let _ = moshpit_runtime::handle()
            .spawn(async move { driver.forget_wind_down(id).await })
            .await;
    }

    pub fn deletion_ended(&self, connection: String) {
        if let Some(id) = uuid(&connection) {
            self.driver.with(|m, k| m.deletion_ended(id, k));
        }
    }

    // MARK: The phone's push side

    /// This install's id (B443), read from where no backup carries it: what
    /// its pairing's conf on a host is named by. Anything but 8 to 32
    /// lowercase hex digits is ignored.
    pub fn install_id(&self, id: String) {
        self.driver.with(|m, k| m.install_id(id, k));
    }

    /// The device token remembered from an earlier launch.
    pub fn restore_device_token(&self, token: Option<String>) {
        self.driver.with(|m, k| m.restore_device_token(token, k));
    }

    /// iOS handed over a device token (hex).
    pub fn device_token(&self, token: String) {
        self.driver.with(|m, k| m.device_token(token, k));
    }

    /// iOS refused one; `why` in the person's language.
    pub fn registration_failed(&self, why: String) {
        self.driver.with(|m, k| m.registration_failed(why, k));
    }

    /// Ask iOS for a device token if anything is paired.
    pub fn register_for_push(&self) {
        self.driver.with(|m, k| m.register_for_push(k));
    }

    /// The app came to the foreground.
    pub fn foreground(&self) {
        self.driver.with(|m, k| m.foreground(k));
    }

    /// A self-test push carrying `nonce` arrived.
    pub fn self_test_arrived(&self, nonce: String) {
        self.driver.with(|m, k| m.self_test_arrived(nonce, k));
    }

    /// Forget pairings whose connection is gone, once a launch, as soon as
    /// the saved connections can be told; how many were forgotten.
    pub fn forget_orphaned_pairings(&self) -> u32 {
        self.driver.with(|m, k| m.forget_orphaned_pairings(k)).len() as u32
    }

    /// Why this phone has no usable relay credential, in `locale`.
    pub fn relay_error(&self, locale: String) -> Option<String> {
        self.driver
            .read(|m| m.relay_error())
            .map(|note| moshpit_ui::host_care::say(&note, &locale))
    }

    // MARK: Host Setup

    /// Open a Host Setup screen on `connection`'s host; its page id.
    pub fn open_setup(&self, connection: CareConnection, test_pane: Option<String>) -> u64 {
        let Some(id) = uuid(&connection.id) else {
            return 0;
        };
        self.driver.with(|m, k| {
            m.open_setup(
                Conn {
                    id,
                    label: connection.label,
                },
                test_pane,
                k,
            )
        })
    }

    pub fn close_setup(&self, page: u64) {
        self.driver.with(|m, k| m.close_setup(page, k));
    }

    /// `{"op": "host-setup", …}` for screen `page`, in `locale`.
    pub fn setup_message(&self, page: u64, locale: String) -> Option<String> {
        self.driver
            .read(|m| m.setup_view(page))
            .map(|view| moshpit_ui::host_care::host_setup_message(&view, &locale))
    }

    /// What the page told the host (`{"ev": …}`): `agent` for `"agent"`.
    pub fn setup_event(&self, page: u64, ev: String, agent: Option<String>) {
        self.driver.with(|m, k| match ev.as_str() {
            "agent" => {
                if let Some(agent) = &agent {
                    m.setup_select(page, agent, k);
                }
            }
            "inspect" => m.setup_inspect(page, k),
            "install-hooks" => m.setup_install_hooks(page, k),
            "install-hooks-anyway" => m.setup_install_hooks_anyway(page, k),
            "dismiss" => m.setup_dismiss(page, k),
            "remove-hooks" => m.setup_remove_hooks(page, k),
            "prove-hooks" => m.setup_prove_hooks(page, k),
            "pair" => m.setup_pair(page, k),
            "unpair" => m.setup_unpair(page, k),
            "remove-everything" => m.setup_remove_everything(page, k),
            "prove-push" => m.setup_prove_push(page, k),
            _ => {}
        });
    }
}

// MARK: - Install Assist

/// The package manager a capability probe found (`PackageManager`'s raw
/// value: `aptGet`, `dnf`, …).
fn manager(raw: Option<String>) -> Option<PackageManager> {
    raw.and_then(|r| serde_json::from_value(serde_json::Value::String(r)).ok())
}

/// A host's tools as the capability probe found them
/// (`moshpit_install::HostCapabilities`), as Swift's `HostCapabilities`
/// holds them: the package manager by its raw id (`aptGet`, `brew`, …).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostCapabilitiesValue {
    pub has_tmux: bool,
    pub has_mosh_server: bool,
    /// `uname -s`; empty when unknown.
    pub os: String,
    pub package_manager: Option<String>,
    pub has_herdr: bool,
    pub has_python3: bool,
}

impl From<HostCapabilities> for HostCapabilitiesValue {
    fn from(caps: HostCapabilities) -> Self {
        let package_manager = caps
            .package_manager
            .and_then(|m| serde_json::to_value(m).ok())
            .and_then(|v| v.as_str().map(str::to_owned));
        HostCapabilitiesValue {
            has_tmux: caps.has_tmux,
            has_mosh_server: caps.has_mosh_server,
            os: caps.os,
            package_manager,
            has_herdr: caps.has_herdr,
            has_python3: caps.has_python3,
        }
    }
}

/// The capability probe a connection runs on its host: the core's, so the
/// app's re-check (Install Assist) asks exactly what a connect asks.
#[uniffi::export]
pub fn host_capabilities_probe_command() -> String {
    moshpit_install::capabilities::probe_command()
}

/// The directories the probe and the core's commands add to a host's
/// `PATH` (Homebrew's, `/usr/local`, `$HOME/.local/bin`): an exec channel's
/// login-less shell often has none of them.
#[uniffi::export]
pub fn host_extra_path_dirs() -> String {
    moshpit_install::capabilities::EXTRA_PATH_DIRS.to_owned()
}

/// What the probe's output says the host has.
#[uniffi::export]
pub fn host_capabilities_parse(output: String) -> HostCapabilitiesValue {
    HostCapabilities::parse(&output).into()
}

/// What the core saved about connection `connection_id`'s host the last
/// time a connect probed it (`hostcaps.<id>` in the app's defaults,
/// written by the core's connect alone; E7c dropped the app's second
/// writer). `None` until a connect has probed it.
#[uniffi::export]
pub fn store_host_capabilities(
    host: std::sync::Arc<dyn crate::store::PreferencesHost>,
    connection_id: String,
) -> Option<HostCapabilitiesValue> {
    moshpit_store::session_memory::read::<HostCapabilities>(
        &crate::store::HostDefaults(host),
        &moshpit_store::session_memory::host_capabilities_key(&connection_id),
    )
    .map(Into::into)
}

/// What Install Assist offers for a notice's `packages` on a connection
/// whose multiplexer is tmux (`tmux`) or not.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AssistPlanValue {
    pub herdr: bool,
    /// What to install, from the connection.
    pub packages: Vec<String>,
    pub mosh_only: bool,
    pub has_manager: bool,
    /// The command for the host's package manager, if it has a known one.
    pub command: Option<String>,
    pub shows_note: bool,
}

#[uniffi::export]
pub fn install_assist_plan(
    packages: Vec<String>,
    tmux: bool,
    package_manager: Option<String>,
) -> AssistPlanValue {
    let plan = AssistPlan::new(&packages, tmux, manager(package_manager));
    AssistPlanValue {
        herdr: plan.herdr,
        packages: plan.packages.iter().map(|p| (*p).to_owned()).collect(),
        mosh_only: plan.mosh_only(),
        has_manager: plan.manager.is_some(),
        command: plan.command(),
        shows_note: plan.shows_note(),
    }
}

/// What a re-check's probe means for the plan: `resolved`,
/// `stillMissing` or (nothing found) `noChannel`.
#[uniffi::export]
pub fn install_assist_recheck(
    packages: Vec<String>,
    tmux: bool,
    package_manager: Option<String>,
    has_tmux: Option<bool>,
    has_mosh_server: Option<bool>,
    has_herdr: Option<bool>,
) -> String {
    let plan = AssistPlan::new(&packages, tmux, manager(package_manager));
    let caps = has_tmux.map(|tmux| {
        let mut caps = HostCapabilities::unknown();
        caps.has_tmux = tmux;
        caps.has_mosh_server = has_mosh_server.unwrap_or(false);
        caps.has_herdr = has_herdr.unwrap_or(false);
        caps
    });
    recheck_name(plan.recheck(caps.as_ref())).to_owned()
}

fn recheck_name(recheck: Recheck) -> &'static str {
    match recheck {
        Recheck::Resolved => "resolved",
        Recheck::StillMissing => "stillMissing",
        Recheck::NoChannel => "noChannel",
    }
}

/// `{"op": "install", …}` for the NSH sheet; `result` as
/// [`install_assist_recheck`] names it.
#[uniffi::export]
pub fn install_assist_message(
    packages: Vec<String>,
    tmux: bool,
    package_manager: Option<String>,
    rechecking: bool,
    result: Option<String>,
    copied: bool,
) -> String {
    let plan = AssistPlan::new(&packages, tmux, manager(package_manager));
    let result = result.and_then(|r| match r.as_str() {
        "resolved" => Some(Recheck::Resolved),
        "stillMissing" => Some(Recheck::StillMissing),
        "noChannel" => Some(Recheck::NoChannel),
        _ => None,
    });
    moshpit_ui::host_care::install_message(&plan, rechecking, result, copied)
}

/// The one command that says what a host can do (`preflight`), for a
/// probe of the side channel.
#[uniffi::export]
pub fn host_preflight_command() -> String {
    moshpit_install::commands::PREFLIGHT.to_owned()
}

// MARK: - The DEBUG demo

/// The one connection the Host Setup demo's scenarios share.
pub const DEMO_CONNECTION: &str = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";

/// What a canned host answers `command` with in Host Setup demo
/// `scenario` (`fresh`, `installed`, `stale`, `nothingWillPush`,
/// `relayUnreachable`, `missingTools`): the app's screenshot and capture
/// seam, and the NSH page's tests, with no host to reach. `relay` is the
/// relay a paired scenario's host names.
#[uniffi::export]
pub fn host_setup_demo_reply(scenario: String, command: String, relay: String) -> Vec<u8> {
    use moshpit_install::drift::{self, Kind};
    if command.contains("printf 'home=") {
        let tools = if scenario == "missingTools" {
            "openssl=no\ncurl=no\njq=no\ntmux=yes\nbase64=yes"
        } else {
            "openssl=yes\ncurl=yes\njq=yes\ntmux=yes\nbase64=yes"
        };
        return format!("home=/Users/dev\nshell=/bin/zsh\nuname=Darwin\n{tools}").into_bytes();
    }
    let Some(manifest) = demo_manifest(&scenario, relay) else {
        return Vec::new();
    };
    if command.contains("manifest.json") {
        return manifest.to_json().into_bytes();
    }
    // What the host holds is what its manifest records (B315's look).
    let checks = drift::checks(&manifest, Some(DEMO_CONNECTION));
    if drift::command(&checks).as_deref() == Some(command.as_str()) {
        let lines: Vec<String> = checks
            .iter()
            .enumerate()
            .map(|(i, check)| match check.kind {
                Kind::File => format!(
                    "{i}={}",
                    manifest
                        .get(&check.component)
                        .and_then(|c| c.digest.clone())
                        .unwrap_or_else(|| "0".repeat(64))
                ),
                Kind::Hooks => format!("{i}=yes"),
            })
            .collect();
        return format!("{}\n", lines.join("\n")).into_bytes();
    }
    Vec::new()
}

/// The pairing the demo's phone holds for its host, minted, at `relay`:
/// what a paired scenario's host was given, so the page reads it as current
/// (B372 compares the host's conf with the phone's pairing).
#[uniffi::export]
pub fn host_setup_demo_pairing(relay: String) -> PairingRecord {
    (&demo_pairing(relay)).into()
}

fn demo_pairing(relay: String) -> PushPairing {
    let at = Date::from_unix(1_762_800_000.0);
    PushPairing {
        secret_hex: "ab".repeat(32),
        ..PushPairing::make(
            Uuid::parse(DEMO_CONNECTION).expect("a uuid"),
            "m1-pro",
            &relay,
            at,
        )
    }
    .minted(&"cd".repeat(32), at, &"ef".repeat(32), "sandbox")
}

/// The manifest demo `scenario`'s host holds, if it holds one.
fn demo_manifest(scenario: &str, relay: String) -> Option<moshpit_install::InstallManifest> {
    use moshpit_install::manifest::{Component, InstallComponent, InstallManifest};
    use moshpit_install::scripts;
    let (stamp, paired) = match scenario {
        "installed" => (scripts::digest(&InstallComponent::Stamp), true),
        "stale" => (Some("an-older-build".to_owned()), false),
        "nothingWillPush" => (Some("an-older-build".to_owned()), true),
        _ => return None,
    };
    let now = Date::now();
    let mut manifest = InstallManifest::empty();
    manifest.set(
        &InstallComponent::Stamp,
        Some(Component {
            digest: stamp,
            ..Component::new(now)
        }),
    );
    manifest.set(
        &InstallComponent::Sender,
        Some(Component {
            digest: scripts::digest(&InstallComponent::Sender),
            ..Component::new(now)
        }),
    );
    manifest.set(
        &InstallComponent::Hooks {
            agent: "claude".into(),
        },
        Some(Component {
            digest: Some("cfg".into()),
            config_path: Some("$HOME/.claude/settings.json".into()),
            ..Component::new(now)
        }),
    );
    if paired {
        let conf = moshpit_install::push_conf(&demo_pairing(relay.clone()));
        manifest.set(
            &InstallComponent::Pairing {
                conn: DEMO_CONNECTION.into(),
                install: None,
            },
            Some(Component {
                digest: Some(scripts::content_digest(&conf)),
                connection_id: Some(DEMO_CONNECTION.into()),
                relay_url: Some(relay),
                ..Component::new(now)
            }),
        );
    }
    Some(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capability_probe_is_the_cores_and_its_answer_keeps_swifts_raw_ids() {
        assert_eq!(
            host_capabilities_probe_command(),
            moshpit_install::capabilities::probe_command()
        );
        let found = host_capabilities_parse(
            "/usr/bin/tmux\n/opt/homebrew/bin/mosh-server\n::Darwin::/opt/homebrew/bin/brew\n"
                .into(),
        );
        assert_eq!(
            found,
            HostCapabilitiesValue {
                has_tmux: true,
                has_mosh_server: true,
                os: "Darwin".into(),
                package_manager: Some("brew".into()),
                has_herdr: false,
                has_python3: false,
            }
        );
        let apt = host_capabilities_parse("::Linux::/usr/bin/apt-get\n".into());
        assert_eq!(apt.package_manager.as_deref(), Some("aptGet"));
        assert_eq!(
            host_capabilities_parse("::Linux::\n".into()).package_manager,
            None
        );
    }
    use std::collections::HashSet;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Memory(Mutex<Vec<PairingRecord>>);

    impl PairingsFile for Memory {
        fn read(&self) -> Vec<PairingRecord> {
            self.0.lock().unwrap().clone()
        }
        fn upsert(&self, pairing: PairingRecord) -> bool {
            let mut all = self.0.lock().unwrap();
            all.retain(|p| p.connection_id != pairing.connection_id);
            all.push(pairing);
            true
        }
        fn remove(&self, connection_id: String) -> bool {
            self.0
                .lock()
                .unwrap()
                .retain(|p| p.connection_id != connection_id);
            true
        }
    }

    #[derive(Default)]
    struct App {
        changes: Mutex<HashSet<String>>,
    }

    #[async_trait::async_trait]
    impl HostCarePlatform for App {
        async fn exec(&self, _c: String, command: String) -> Result<Vec<u8>, HostCareFailure> {
            if command.starts_with("printf 'home=") {
                return Ok(b"home=/h\nuname=Linux\nopenssl=yes\ncurl=yes\nbase64=yes".to_vec());
            }
            Ok(Vec::new())
        }
        async fn post(
            &self,
            _: String,
            _: String,
            _: u32,
        ) -> Result<HttpReplyValue, HostCareFailure> {
            Err(HostCareFailure::Failed {
                why: "offline".into(),
            })
        }
        fn register_for_push(&self) {}
        fn remember_declined(&self, _: String) {}
        fn remember_unpaired(&self, _: String, _: bool) {}
        fn changed(&self, change: HostCareChange) {
            self.changes.lock().unwrap().insert(format!("{change:?}"));
        }
    }

    #[test]
    fn a_setup_page_reads_its_host_through_the_app_and_posts_the_screens_message() {
        let app = Arc::new(App::default());
        let core = HostCareCore::new(
            app.clone(),
            Arc::new(Memory::default()),
            "sandbox".into(),
            false,
        );
        let page = core.open_setup(
            CareConnection {
                id: Uuid::new_v4().to_string(),
                label: "m1".into(),
            },
            None,
        );
        for _ in 0..200 {
            let json = core.setup_message(page, "zh-Hans".into()).unwrap();
            if json.contains("\"phase\":\"idle\"") {
                assert!(json.contains("\"uname\":\"Linux\""), "{json}");
                assert!(
                    app.changes
                        .lock()
                        .unwrap()
                        .contains(&format!("{:?}", HostCareChange::Page { page }))
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the host was never read");
    }

    #[test]
    fn the_demo_hosts_read_as_their_scenarios() {
        use moshpit_install::installer::{ComponentStatus, InstallState};
        use moshpit_install::{HostFacts, InstallComponent, InstallManifest, commands};
        let read = |scenario: &str| {
            let reply = |command: &str| {
                String::from_utf8(host_setup_demo_reply(
                    scenario.into(),
                    command.into(),
                    "https://r".into(),
                ))
                .unwrap()
            };
            InstallState {
                facts: HostFacts::parse(&reply(commands::PREFLIGHT)),
                manifest: InstallManifest::from_text(&reply(&commands::read_file(
                    InstallManifest::PATH,
                ))),
            }
        };
        let pairing = InstallComponent::Pairing {
            conn: DEMO_CONNECTION.into(),
            install: None,
        };
        assert_eq!(read("installed").status(&pairing), ComponentStatus::Current);
        assert_eq!(
            read("installed").hooks_status("claude"),
            ComponentStatus::Current
        );
        assert!(matches!(
            read("stale").hooks_status("claude"),
            ComponentStatus::Stale { .. }
        ));
        assert!(read("nothingWillPush").paired_but_nothing_will_push());
        assert!(!read("missingTools").facts.can_push());
        assert_eq!(
            read("fresh").hooks_status("claude"),
            ComponentStatus::Absent
        );
    }

    /// The demo's platform: commands answered by the canned host.
    struct Demo(String);

    #[async_trait::async_trait]
    impl HostCarePlatform for Demo {
        async fn exec(&self, _c: String, command: String) -> Result<Vec<u8>, HostCareFailure> {
            Ok(host_setup_demo_reply(
                self.0.clone(),
                command,
                "https://r".into(),
            ))
        }
        async fn post(
            &self,
            _: String,
            _: String,
            _: u32,
        ) -> Result<HttpReplyValue, HostCareFailure> {
            Err(HostCareFailure::Failed {
                why: "no relay".into(),
            })
        }
        fn register_for_push(&self) {}
        fn remember_declined(&self, _: String) {}
        fn remember_unpaired(&self, _: String, _: bool) {}
        fn changed(&self, _: HostCareChange) {}
    }

    /// The iPad's `KeysShortcutsSetupNSHTests.hostSetupMessages`, on the
    /// same machine and canned host as the app's DEBUG demo
    /// (`DemoHostCare`): each scenario reads as it always did now the
    /// machine also looks at the host's files (B315).
    #[test]
    fn every_demo_scenario_reads_as_before() {
        for scenario in [
            "fresh",
            "installed",
            "stale",
            "nothingWillPush",
            "relayUnreachable",
            "missingTools",
        ] {
            // The phone holds what a paired scenario's host was given, as
            // the app's demo seeds it.
            let paired = scenario == "installed" || scenario == "nothingWillPush";
            let phone = Memory(Mutex::new(if paired {
                vec![host_setup_demo_pairing("https://r".into())]
            } else {
                Vec::new()
            }));
            let core = HostCareCore::new(
                Arc::new(Demo(scenario.into())),
                Arc::new(phone),
                "sandbox".into(),
                true,
            );
            core.set_settings(true, true, "https://r".into(), vec![], vec![]);
            if scenario == "relayUnreachable" {
                core.registration_failed("Relay rejected the registration (401)".into());
            }
            let stale = scenario == "stale" || scenario == "nothingWillPush";
            let page = core.open_setup(
                CareConnection {
                    id: DEMO_CONNECTION.into(),
                    label: "m1-pro".into(),
                },
                (!stale).then(|| "%3".to_owned()),
            );
            if stale {
                core.setup_event(page, "prove-hooks".into(), None);
            }
            let mut message = serde_json::Value::Null;
            for _ in 0..300 {
                message =
                    serde_json::from_str(&core.setup_message(page, "en".into()).unwrap()).unwrap();
                if message["phase"] == "idle" {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert_eq!(message["phase"], "idle", "{scenario}");
            match scenario {
                "installed" => {
                    assert_eq!(message["pairing"], "current", "{scenario}: {message}");
                    assert_eq!(message["hooks"], "current", "{scenario}: {message}");
                }
                "stale" | "nothingWillPush" => {
                    assert_eq!(message["hooks"], "stale", "{scenario}: {message}");
                    assert_eq!(message["hooksProof"]["state"], "unavailable", "{scenario}");
                    assert_eq!(
                        message["nothingWillPush"],
                        scenario == "nothingWillPush",
                        "{scenario}"
                    );
                }
                "missingTools" => assert!(
                    message["facts"]["missingForPush"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|t| t == "openssl")
                ),
                "relayUnreachable" => assert!(message["relayError"].is_string()),
                _ => assert_eq!(message["hooks"], "absent", "{scenario}"),
            }
        }
    }

    #[test]
    fn install_assist_is_planned_by_the_core() {
        let plan = install_assist_plan(vec!["mosh".into()], true, Some("aptGet".into()));
        assert_eq!(plan.packages, ["tmux", "mosh"]);
        assert_eq!(
            plan.command.as_deref(),
            Some("sudo apt-get install -y tmux mosh")
        );
        assert_eq!(
            install_assist_recheck(
                vec!["mosh".into()],
                true,
                None,
                Some(true),
                Some(false),
                None
            ),
            "stillMissing"
        );
        assert_eq!(
            install_assist_recheck(vec!["mosh".into()], true, None, None, None, None),
            "noChannel"
        );
        let json = install_assist_message(vec!["herdr".into()], false, None, true, None, false);
        assert!(json.contains("\"herdr\":true") && json.contains("\"rechecking\":true"));
    }
}
