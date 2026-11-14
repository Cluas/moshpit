//! Running a host's setup from the phone, as a state machine with no I/O
//! (W15b): what `HostAutoCare`, `HostSetupModel`, `InstallAssistPlan` and
//! the pairing half of `PushService` did in the iOS app, for iOS and
//! Android alike. The pieces it drives — the scripts, the manifest, the
//! hook configs, the installer's steps ([`moshpit_install`]) and the push
//! pairing, its file and the relay's token ([`moshpit_seal`]) — are W3's
//! and W4's.
//!
//! # Shape
//!
//! [`HostCare`] is one machine for the app. **Inputs**, each with the
//! caller's clock ([`Clock`]): a session came up ([`HostCare::tend`]), the
//! person's asks (consent, a Host Setup screen's buttons), a delete's
//! handshake, the device token and the pushes that arrive, the saved
//! connections and the settings as they change — and the answers to what
//! it asked for: a command's output ([`HostCare::exec_done`]), a relay's
//! reply ([`HostCare::post_done`]), the time passing ([`HostCare::tick`]
//! once [`HostCare::next_deadline`] is due).
//!
//! **Outputs**, in order ([`HostCare::poll_output`]): a command to run on a
//! connection's host (on its session's own SSH connection: W14b's
//! `borrowSsh` on the Rust core), a JSON POST to a relay, asking the OS for
//! a device token, the person's "don't ask again" to persist, and which
//! views changed.
//!
//! The only thing it touches in place is the phone's pairings file
//! ([`PairingStore`]): local and synchronous.
//!
//! Inside, each operation — a care pass, a Host Setup step, a mint round —
//! is a coroutine the machine polls itself (`exec`): the installer's async
//! steps run unchanged, suspended on the machine's own leaves.
//! [`driver::Driver`] carries the outputs out on a tokio runtime for a
//! platform ([`driver::Platform`]).
//!
//! # The rules
//!
//! - Care acts only for a connection that is saved and not being deleted or
//!   unpaired, asked again after every wait that comes before acting (B70,
//!   B88); a delete or an unpair stands the pass out for it down.
//! - Care never installs hooks on a host that has none: that needs a yes,
//!   asked once per connection unless declined.
//! - One pairing operation per host at a time, care's or the screen's
//!   (B123); one after a wait reads the host again.
//! - The phone is asked before the host is touched: a pairing is saved on
//!   the phone and minted at the relay before it is installed; an unpair
//!   forgets it on the phone first, and a phone that can't stops there.
//! - A pairing that isn't ready (no token) never reaches a host; a relay's
//!   late answer for a pairing forgotten or replaced meanwhile is dropped.
//! - Every wait is bounded: the device token 10 s, a self-test push 20 s, a
//!   care pass a delete waits out 5 s, a relay request 15 s (the
//!   platform's), one retry after 2 s on a network failure only, a command
//!   on a host 60 s for care and 90 s for a Host Setup step.
//! - A step's result that comes back after the screen moved to another
//!   agent is not shown as that agent's (B235).
//!
//! Where it deliberately differs from the Swift it replaces is marked
//! `Fix (<id>)`.

pub mod assist;
mod care;
pub mod driver;
mod exec;
pub mod note;
mod push;
pub mod setup;
pub mod store;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::task::{Context, Waker};

use moshpit_seal::{Date, PushPairing, Uuid};

pub use assist::{AssistPlan, Recheck};
pub use care::consented_agents;
pub use driver::{Driver, Platform};
pub use exec::{CANCELLED, HOST_SILENT, HttpReply, TaskId};
pub use note::{Note, RelayFailure};
pub use setup::{AgentView, Phase, Proof, SetupView};
pub use store::{MemoryStore, PairingStore};

use exec::{BoxFuture, Ctx, Shared, lock};

/// The caller's clock: monotonic milliseconds for the waits, the wall
/// clock for what is recorded (a manifest's dates, a token's age).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clock {
    pub ms: u64,
    pub date: Date,
}

/// A connection, as care and the screens name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conn {
    pub id: Uuid,
    /// Its display name: the pairing's host label, the log's name.
    pub label: String,
}

/// The settings care reads (the app's `AppSettings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Agent notifications.
    pub notifications: bool,
    /// The Live Activity (the island).
    pub live_activity: bool,
    /// The relay pairings mint at.
    pub relay_url: String,
    /// Connections whose consent question was answered "don't ask again"
    /// (upper-case UUID strings, as stored).
    pub declined: Vec<String>,
    /// Connections the person unpaired in Host Setup: care pairs nothing
    /// for them until the person pairs again (B371). Upper-case UUID
    /// strings, as stored.
    pub unpaired: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            notifications: true,
            live_activity: true,
            relay_url: String::new(),
            declined: Vec::new(),
            unpaired: Vec::new(),
        }
    }
}

/// Whether this build can be pushed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushSupport {
    /// APNs, with the build's token environment (`sandbox` or
    /// `production`).
    Apns { environment: String },
    /// No push service (Android without FCM): nothing is paired.
    Unavailable,
}

/// Every wait the machine makes, in milliseconds (seconds for the relay's
/// request, which the platform bounds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budgets {
    /// For iOS to hand over a device token (`awaitDeviceToken`).
    pub device_token_ms: u64,
    /// For a self-test push to arrive.
    pub self_test_ms: u64,
    /// For a care pass a delete or an unpair stood down to end
    /// (`ConnectionDeletion.careWindDown`).
    pub care_wind_down_ms: u64,
    /// For a deleted connection's pairing to come off its host while its
    /// session is still up (B377); each command, and the delete's wait.
    pub forget_on_delete_ms: u64,
    /// For the host to answer one of care's commands. Past it the command
    /// has failed, and the pass ends: a host that never answers no longer
    /// holds the pass, and the host's pairing lane, for the rest of the
    /// run.
    pub care_exec_ms: u64,
    /// For the host to answer one of a Host Setup step's commands: as long
    /// as the slowest of them needs on a slow link (the push test's sender
    /// gives each paired phone's relay request 6 s), and no longer than a
    /// person waits on the screen before retrying.
    pub setup_exec_ms: u64,
    /// Before the one retry of a mint that didn't reach the relay.
    pub mint_retry_ms: u64,
    pub mint_timeout_seconds: u32,
}

impl Default for Budgets {
    fn default() -> Self {
        Budgets {
            device_token_ms: 10_000,
            self_test_ms: 20_000,
            care_wind_down_ms: 5_000,
            forget_on_delete_ms: 3_000,
            care_exec_ms: 60_000,
            setup_exec_ms: 90_000,
            mint_retry_ms: 2_000,
            mint_timeout_seconds: 15,
        }
    }
}

/// How the machine is set up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub push: PushSupport,
    /// Care does nothing (a harness's `-MOSHPIT_AUTOCARE_OFF`: its seeded
    /// connections point at real hosts).
    pub autocare_off: bool,
    pub budgets: Budgets,
}

/// What changed, for the views that show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Change {
    /// The consent question ([`HostCare::consent`]).
    Consent,
    /// A Host Setup screen ([`HostCare::setup_view`]).
    Page(u64),
    /// The phone's push side: its relay error, its pairings.
    Push,
}

/// What the machine asks of its platform, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// Run `command` on `connection`'s host — under `sh -c`, on the
    /// session's own connection — and answer with its stdout, or why it
    /// failed (a non-zero exit is a failure), through
    /// [`HostCare::exec_done`].
    Exec {
        id: u64,
        connection: Uuid,
        command: String,
    },
    /// POST `body` (JSON) to `url`, given up after `timeout_seconds`;
    /// answer with the status and body, or why it didn't reach the relay,
    /// through [`HostCare::post_done`].
    Post {
        id: u64,
        url: String,
        body: String,
        timeout_seconds: u32,
    },
    /// Ask the OS for a device token: prompt for notification permission
    /// when undecided, register when allowed, say nothing when denied.
    RegisterForPush,
    /// Remember that the person declined care for `connection`.
    RememberDeclined { connection: Uuid },
    /// Remember (`unpaired`) or forget that the person unpaired
    /// `connection` in Host Setup (B371).
    RememberUnpaired { connection: Uuid, unpaired: bool },
    /// A view changed.
    Changed(Change),
}

/// The machine.
pub struct HostCare {
    shared: Arc<Mutex<Shared>>,
    tasks: Vec<(TaskId, BoxFuture)>,
    next_task: TaskId,
}

impl HostCare {
    pub fn new(config: Config, store: Box<dyn PairingStore>, clock: Clock) -> Self {
        HostCare {
            shared: Arc::new(Mutex::new(Shared::new(config, store, clock))),
            tasks: Vec::new(),
            next_task: 1,
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Shared) -> R) -> R {
        f(&mut lock(&self.shared))
    }

    fn set_clock(&mut self, clock: Clock) {
        self.with(|s| {
            // Monotonic: a clock that steps back changes nothing.
            if clock.ms >= s.clock.ms {
                s.clock = clock;
            } else {
                s.clock.date = clock.date;
            }
        });
    }

    fn spawn<F>(&mut self, make: impl FnOnce(Ctx) -> F) -> TaskId
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let id = self.next_task;
        self.next_task += 1;
        let ctx = Ctx {
            shared: self.shared.clone(),
            task: id,
        };
        self.with(|s| s.running.insert(id));
        self.tasks.push((id, Box::pin(make(ctx))));
        id
    }

    /// Poll every operation until none can move: each wait that ends may
    /// let another move.
    fn pump(&mut self) {
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            let before = self.with(|s| {
                s.deadlines.clear();
                s.generation
            });
            let mut finished = false;
            let mut i = 0;
            while i < self.tasks.len() {
                if self.tasks[i].1.as_mut().poll(&mut cx).is_ready() {
                    let (id, future) = self.tasks.remove(i);
                    drop(future);
                    self.with(|s| {
                        s.running.remove(&id);
                        s.cancelled.remove(&id);
                        s.generation += 1;
                    });
                    finished = true;
                } else {
                    i += 1;
                }
            }
            if !finished && self.with(|s| s.generation) == before {
                break;
            }
        }
        self.with(|s| {
            let changes = std::mem::take(&mut s.changes);
            for change in changes {
                s.outputs.push_back(Output::Changed(change));
            }
        });
    }

    /// Run `f` on the state at `clock`, then let every operation move.
    fn input<R>(&mut self, clock: Clock, f: impl FnOnce(&mut Self) -> R) -> R {
        self.set_clock(clock);
        let result = f(self);
        self.pump();
        result
    }

    // MARK: - Outputs and time

    /// The next thing the platform should do.
    pub fn poll_output(&mut self) -> Option<Output> {
        self.with(|s| s.outputs.pop_front())
    }

    /// When [`HostCare::tick`] is next due, on the caller's monotonic clock.
    pub fn next_deadline(&self) -> Option<u64> {
        self.with(|s| s.deadlines.first().copied())
    }

    pub fn tick(&mut self, clock: Clock) {
        self.input(clock, |_| ());
    }

    /// Command `id`'s stdout, or why it failed.
    pub fn exec_done(&mut self, id: u64, result: Result<Vec<u8>, String>, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| s.exec_done(id, result));
        });
    }

    /// Request `id`'s reply, or why it didn't reach the relay.
    pub fn post_done(&mut self, id: u64, result: Result<HttpReply, String>, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| s.post_done(id, result));
        });
    }

    /// Whether anything is still running.
    pub fn is_idle(&self) -> bool {
        self.tasks.is_empty()
    }

    // MARK: - What the app tells it

    /// The settings care reads, as they are now.
    pub fn set_settings(&mut self, settings: Settings, clock: Clock) {
        self.input(clock, |m| m.with(|s| s.settings = settings));
    }

    /// The saved connections: `listed` the ones the store decoded, `all`
    /// every one it holds (`None` while that can't be told — a launch
    /// before the first unlock).
    pub fn set_saved(&mut self, listed: &[Uuid], all: Option<&[Uuid]>, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| {
                s.saved.listed = listed.iter().copied().collect();
                s.saved.all = all.map(|ids| ids.iter().copied().collect());
            })
        });
    }

    // MARK: - Care

    /// A session's control plane for `conn` came up: walk its host once
    /// this app run (`tend`).
    pub fn tend(&mut self, conn: Conn, clock: Clock) {
        self.input(clock, |m| {
            let go = m.with(|s| {
                !s.config.autocare_off
                    && (s.settings.notifications || s.settings.live_activity)
                    && !s.care.tended.contains(&conn.id)
                    && !s.care.in_flight.contains_key(&conn.id)
                    && s.is_kept(conn.id)
            });
            if go {
                let id = conn.id;
                let task = m.spawn(|ctx| care::pass(ctx, conn));
                m.with(|s| s.care.in_flight.insert(id, task));
            }
        });
    }

    /// Whether a care pass is running for `connection`.
    pub fn is_tending(&self, connection: Uuid) -> bool {
        self.with(|s| {
            s.care
                .in_flight
                .get(&connection)
                .is_some_and(|t| s.running.contains(t))
        })
    }

    /// The connection whose host has no hooks yet: ask the one question.
    pub fn consent(&self) -> Option<Conn> {
        self.with(|s| s.care.needs_consent.clone())
    }

    /// The person said yes (`grantConsent`): Claude Code's hooks, then the
    /// pairing, after whatever pass is out for the connection.
    pub fn grant_consent(&mut self, conn: Conn, clock: Clock) {
        self.input(clock, |m| {
            let previous = m.with(|s| {
                s.care.needs_consent = None;
                s.changed(Change::Consent);
                s.is_kept(conn.id)
                    .then(|| s.care.in_flight.get(&conn.id).copied())
            });
            let Some(previous) = previous else {
                return;
            };
            let id = conn.id;
            let task = m.spawn(|ctx| care::consented(ctx, conn, previous));
            m.with(|s| s.care.in_flight.insert(id, task));
        });
    }

    /// The person said no: never ask for this connection again.
    pub fn decline(&mut self, connection: Uuid, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| {
                s.care.needs_consent = None;
                s.changed(Change::Consent);
                s.mark_declined(connection);
            })
        });
    }

    /// "Not now": asked again next app run.
    pub fn dismiss_consent(&mut self, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| {
                s.care.needs_consent = None;
                s.changed(Change::Consent);
            })
        });
    }

    /// Forget that `connection` was tended this run: an explicit change in
    /// Host Setup is followed by a fresh look next session.
    pub fn forget(&mut self, connection: Uuid, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| s.care.tended.remove(&connection));
        });
    }

    /// A delete of `connection` begins (`ConnectionDeletion`): from here on
    /// care pairs nothing for it, the pass out for it is cancelled, and its
    /// pairing is forgotten on this phone. `Err`: the phone couldn't let go
    /// — nothing else may be removed, and the delete is over.
    pub fn deletion_began(&mut self, connection: Uuid, clock: Clock) -> Result<(), Note> {
        self.input(clock, |m| {
            let outcome = m.with(|s| {
                s.care.deleting.insert(connection);
                if s.care.needs_consent.as_ref().map(|c| c.id) == Some(connection) {
                    s.care.needs_consent = None;
                    s.changed(Change::Consent);
                }
                s.stand_down(connection);
                let paired = s.store.pairing(connection).is_some();
                match push::unpair(s, connection) {
                    Ok(()) => {
                        if paired {
                            s.care.forgetting.insert(connection);
                        }
                        Ok(paired)
                    }
                    Err(failure) => {
                        s.care.deleting.remove(&connection);
                        s.care.tended.remove(&connection);
                        Err(Note::Relay(failure))
                    }
                }
            });
            // Its conf on the host too, while the session is up (B377):
            // best effort, waited for at most a moment before the session
            // goes (`forget_wind_down`).
            if let Ok(true) = outcome {
                m.spawn(move |ctx| care::forget_on_host(ctx, connection));
            }
            outcome.map(|_| ())
        })
    }

    /// Whether a deleted connection's pairing is still on its way off its
    /// host (B377).
    pub fn is_forgetting(&self, connection: Uuid) -> bool {
        self.shared
            .lock()
            .unwrap()
            .care
            .forgetting
            .contains(&connection)
    }

    /// `connection` was saved pointing at another host or port (B328): the
    /// pairing this phone kept for it is the box it pointed at before.
    /// Forget it on this phone, as a delete does, so that box can no longer
    /// push under the connection (without the secret it can only ring the
    /// generic line until the relay's TTL); stand down the pass out for
    /// it, and let the next pass pair the new host. `Err`: the phone
    /// couldn't let go, and nothing changed.
    pub fn host_changed(&mut self, connection: Uuid, clock: Clock) -> Result<(), Note> {
        self.input(clock, |m| {
            m.with(|s| {
                // A pass out for it is reading the box it pointed at
                // before, and would pair that one.
                s.stand_down(connection);
                s.care.tended.remove(&connection);
                if s.store.pairing(connection).is_none() {
                    return Ok(());
                }
                push::unpair(s, connection).map_err(Note::Relay)
            })
        })
    }

    /// The delete of `connection` is over, done or not.
    pub fn deletion_ended(&mut self, connection: Uuid, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| {
                s.care.deleting.remove(&connection);
                s.care.tended.remove(&connection);
            })
        });
    }

    // MARK: - The phone's push side

    /// The device token iOS remembered from an earlier launch, without a
    /// mint round (`PushService.init`).
    pub fn restore_device_token(&mut self, token: Option<String>, clock: Clock) {
        self.input(clock, |m| m.with(|s| s.push.device_token = token));
    }

    /// iOS handed over a device token (`handle(deviceToken:)`): anything
    /// waiting for one goes on, and a changed token (or a pending failure)
    /// re-mints what needs it.
    pub fn device_token(&mut self, token: String, clock: Clock) {
        self.input(clock, |m| {
            let mint = m.with(|s| {
                let changed = s.push.device_token.as_deref() != Some(token.as_str());
                s.push.device_token = Some(token);
                s.set_phone_error(None);
                changed || s.push.sync_pending
            });
            if mint {
                m.spawn(push::mint_needed);
            }
        });
    }

    /// iOS refused a device token: say so, and try again on the next
    /// foreground.
    pub fn registration_failed(&mut self, why: String, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| {
                s.set_phone_error(Some(Note::Said(why)));
                s.push.sync_pending = true;
            })
        });
    }

    /// Ask the OS for a device token if anything is paired (launch).
    pub fn register_for_push(&mut self, clock: Clock) {
        self.input(clock, |m| m.with(Shared::register_for_push));
    }

    /// The app came to the foreground (`refreshRegistration`): re-mint
    /// only what needs it, at most once a minute after a success.
    pub fn foreground(&mut self, clock: Clock) {
        self.input(clock, |m| {
            let mint = m.with(|s| {
                let paired = !s.store.read().is_empty();
                if !paired || s.push.device_token.is_none() {
                    if s.push.sync_pending {
                        s.register_for_push();
                    }
                    return false;
                }
                !moshpit_seal::service::should_skip_refresh(
                    s.push.sync_pending,
                    s.push.last_success,
                    s.clock.date,
                    moshpit_seal::service::REFRESH_FLOOR_SECONDS,
                )
            });
            if mint {
                m.spawn(push::mint_needed);
            }
        });
    }

    /// Why this phone has no usable relay credential, if it has none.
    pub fn relay_error(&self) -> Option<Note> {
        self.with(|s| s.relay_error())
    }

    /// The pairings on this phone.
    pub fn pairings(&self) -> Vec<PushPairing> {
        self.with(|s| s.store.read())
    }

    /// A self-test push arrived carrying `nonce`.
    pub fn self_test_arrived(&mut self, nonce: String, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| s.push.arrived.insert(nonce));
        });
    }

    /// Forget every pairing whose connection is no longer saved, once per
    /// launch, the first time the saved connections can be told
    /// (`forgetOrphanedPairingsOnce`). Returns what was forgotten.
    pub fn forget_orphaned_pairings(&mut self, clock: Clock) -> Vec<PushPairing> {
        self.input(clock, |m| {
            m.with(|s| {
                if s.push.orphans_swept {
                    return Vec::new();
                }
                let Some(saved) = s.saved.all.clone() else {
                    log::info!(target: "moshpit::push", "orphaned pairings: which connections are saved can't be told yet \u{2014} not sweeping");
                    return Vec::new();
                };
                s.push.orphans_swept = true;
                let pairings = s.store.read();
                let orphans: HashSet<Uuid> =
                    moshpit_seal::service::orphaned(&pairings, Some(&saved))
                        .into_iter()
                        .collect();
                let mut forgotten = Vec::new();
                for pairing in pairings
                    .into_iter()
                    .filter(|p| orphans.contains(&p.connection_id))
                {
                    if s.store.remove(pairing.connection_id) {
                        s.push.pairing_errors.remove(&pairing.connection_id);
                        log::info!(target: "moshpit::push", "forgot the pairing for {} ({}): its connection is no longer saved", pairing.host_label, pairing.connection_id);
                        forgotten.push(pairing);
                    } else {
                        log::error!(target: "moshpit::push", "couldn't forget the pairing for {} ({}), whose connection is no longer saved", pairing.host_label, pairing.connection_id);
                    }
                }
                if !forgotten.is_empty() {
                    s.changed(Change::Push);
                }
                forgotten
            })
        })
    }

    // MARK: - Host Setup

    /// Open a Host Setup screen for `conn` and read its host. `test_pane`:
    /// the tmux pane the hook test fires into.
    pub fn open_setup(&mut self, conn: Conn, test_pane: Option<String>, clock: Clock) -> u64 {
        self.input(clock, |m| {
            let id = m.with(|s| {
                let id = s.fresh_id();
                let mut page = setup::Page::new(conn.id, conn.label, test_pane);
                page.relay_url = s.settings.relay_url.clone();
                s.pages.insert(id, page);
                id
            });
            m.spawn(move |ctx| setup::inspect(ctx, id));
            id
        })
    }

    /// The screen went away; what it started finishes.
    pub fn close_setup(&mut self, page: u64, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| s.pages.remove(&page));
        });
    }

    /// What screen `page` shows.
    pub fn setup_view(&self, page: u64) -> Option<SetupView> {
        self.with(|s| s.pages.get(&page).map(|p| s.setup_view(p)))
    }

    /// Read the host again (Retry, once a read failed), between steps.
    pub fn setup_inspect(&mut self, page: u64, clock: Clock) {
        self.input(clock, |m| {
            if m.page_idle(page) {
                m.spawn(move |ctx| setup::inspect(ctx, page));
            }
        });
    }

    /// Act on agent `agent_id` from now on — between steps only: a step or
    /// a proof running is about the agent it started for, and its result,
    /// a failure too, is shown as that agent's (B384).
    pub fn setup_select(&mut self, page: u64, agent_id: &str, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| {
                let Some(index) = moshpit_install::HookAgent::ALL
                    .iter()
                    .position(|a| a.id == agent_id)
                else {
                    return;
                };
                if let Some(p) = s.pages.get_mut(&page)
                    && p.phase == Phase::Idle
                    && setup::select(p, index)
                {
                    s.changed(Change::Page(page));
                }
            })
        });
    }

    /// Whether `page` is between steps (its buttons act).
    fn page_idle(&self, page: u64) -> bool {
        self.with(|s| s.pages.get(&page).is_some_and(|p| p.phase == Phase::Idle))
    }

    /// Whether `page` is between steps and has read its host: a step on a
    /// host that couldn't be read would act on nothing it knows (B382).
    fn page_ready(&self, page: u64) -> bool {
        self.with(|s| {
            s.pages
                .get(&page)
                .is_some_and(|p| p.phase == Phase::Idle && p.state.is_some())
        })
    }

    /// Install the agent's hooks — after asking, for an agent the host
    /// doesn't have (B381): the page then shows the question instead
    /// ([`HostCare::setup_install_hooks_anyway`], [`HostCare::setup_dismiss`]).
    pub fn setup_install_hooks(&mut self, page: u64, clock: Clock) {
        self.install_hooks(page, clock, false);
    }

    /// The person said yes to installing hooks for an agent the host
    /// doesn't have.
    pub fn setup_install_hooks_anyway(&mut self, page: u64, clock: Clock) {
        self.install_hooks(page, clock, true);
    }

    /// The person said no to the page's question.
    pub fn setup_dismiss(&mut self, page: u64, clock: Clock) {
        self.input(clock, |m| {
            m.with(|s| {
                if let Some(p) = s.pages.get_mut(&page)
                    && p.confirming
                {
                    p.confirming = false;
                    s.changed(Change::Page(page));
                }
            })
        });
    }

    fn install_hooks(&mut self, page: u64, clock: Clock, anyway: bool) {
        self.input(clock, |m| {
            if !m.page_ready(page) {
                return;
            }
            let ask = m.with(|s| {
                let p = s.pages.get_mut(&page)?;
                let ask = !anyway && p.found(p.agent()) == Some(false);
                p.confirming = ask;
                Some(ask)
            });
            m.with(|s| s.changed(Change::Page(page)));
            if ask != Some(false) {
                return;
            }
            if let Some(connection) = m.with(|s| s.pages.get(&page).map(|p| p.connection)) {
                m.with(|s| s.care.tended.remove(&connection));
            }
            // A test was about the registration this replaces (B380).
            m.with(|s| {
                if let Some(p) = s.pages.get_mut(&page) {
                    p.hooks_proof = Proof::Untested;
                }
            });
            m.spawn(move |ctx| setup::run(ctx, page, setup::Step::InstallHooks));
        });
    }

    pub fn setup_remove_hooks(&mut self, page: u64, clock: Clock) {
        self.input(clock, |m| {
            if m.page_ready(page) {
                m.with(|s| {
                    if let Some(p) = s.pages.get_mut(&page) {
                        p.hooks_proof = Proof::Untested;
                    }
                });
                m.spawn(move |ctx| setup::run(ctx, page, setup::Step::RemoveHooks));
            }
        });
    }

    /// Test the hooks, between steps: a proof holds the page as a step
    /// does (B383).
    pub fn setup_prove_hooks(&mut self, page: u64, clock: Clock) {
        self.input(clock, |m| {
            // Only hooks this build installed: the test runs what they
            // register (B380).
            let testable = m.with(|s| s.pages.get(&page).is_some_and(setup::hooks_testable));
            if m.page_ready(page) && testable {
                m.spawn(move |ctx| setup::prove_hooks(ctx, page));
            }
        });
    }

    pub fn setup_pair(&mut self, page: u64, clock: Clock) {
        self.input(clock, |m| {
            if !m.page_ready(page) {
                return;
            }
            if m.with(|s| matches!(s.config.push, PushSupport::Unavailable)) {
                m.with(|s| {
                    if let Some(p) = s.pages.get_mut(&page) {
                        p.error = Some(Note::Relay(RelayFailure::Unavailable));
                        s.changed(Change::Page(page));
                    }
                });
                return;
            }
            m.spawn(move |ctx| setup::pair(ctx, page));
        });
    }

    /// Forget the pairing on this phone, then on the host; care stands
    /// down for the host meanwhile, and the pass out for it is waited out
    /// first (B88).
    pub fn setup_unpair(&mut self, page: u64, clock: Clock) {
        self.unpairing(page, clock, false);
    }

    /// Take Moshpit off the page's host (B375): every agent's hooks, this
    /// phone's pairing, and the rest once no other phone is paired there.
    pub fn setup_remove_everything(&mut self, page: u64, clock: Clock) {
        self.unpairing(page, clock, true);
    }

    /// An Unpair, or with `everything` a Remove everything: both let go of
    /// this phone's pairing first, waiting out care.
    fn unpairing(&mut self, page: u64, clock: Clock, everything: bool) {
        self.input(clock, |m| {
            if !m.page_ready(page) {
                return;
            }
            let Some(connection) = m.with(|s| s.pages.get(&page).map(|p| p.connection)) else {
                return;
            };
            let pass = m.with(|s| {
                s.care.unpairing.insert(connection);
                s.stand_down(connection)
            });
            let step = if everything {
                setup::Step::RemoveEverything { pass }
            } else {
                setup::Step::Unpair { pass }
            };
            m.spawn(move |ctx| async move {
                setup::run(ctx.clone(), page, step).await;
                ctx.with(|s| {
                    if let Some(p) = s.pages.get_mut(&page)
                        && p.error.is_none()
                    {
                        // A proof is about a pairing; void once it is gone.
                        p.push_proof = Proof::Untested;
                        s.changes.insert(Change::Page(page));
                    }
                    s.care.unpairing.remove(&connection);
                    s.care.tended.remove(&connection);
                });
            });
        });
    }

    /// Test the push, between steps (B383).
    pub fn setup_prove_push(&mut self, page: u64, clock: Clock) {
        self.input(clock, |m| {
            if !m.page_ready(page)
                || m.with(|s| matches!(s.config.push, PushSupport::Unavailable))
            {
                return;
            }
            m.spawn(move |ctx| setup::prove_push(ctx, page));
        });
    }
}
