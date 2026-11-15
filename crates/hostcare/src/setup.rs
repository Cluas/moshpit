//! The Host Setup screen's model (`HostSetupModel`): what it says about one
//! host — its facts, the hooks' and the pairing's status, the proofs that
//! either works — and the steps it runs there. One phase at a time, so the
//! page can't show two contradictory states; the agent stays put while a
//! step or a proof runs, so what it ends with, a failure too, is shown as
//! the agent it was for (B235, B384).

use moshpit_install::installer::{
    ComponentStatus, HookProof, HostInstaller, InstallReport, InstallState, StepFailure,
    make_nonce, pairing_needs_rewrite_as,
};
use moshpit_install::{HookAgent, HostFacts, InstallComponent, InstallManifest, push_conf, push_conf_as, scripts};
use moshpit_seal::{Date, PushPairing, Uuid};

use crate::care::Stop;
use crate::exec::{Ctx, LaneTicket, Link, Shared, TaskId, look};
use crate::note::{Note, RelayFailure};
use crate::{Change, PushSupport, push};

/// What the screen is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Inspecting,
    /// A step runs; what to say while it does.
    Working(Note),
    /// A proof runs (the hooks' or the push's): every step waits for its
    /// verdict, as it waits for a step's (B383).
    Proving,
}

/// Whether a path is proven, as opposed to merely installed.
#[derive(Debug, Clone, PartialEq)]
pub enum Proof {
    Untested,
    Proving,
    Proven(Date),
    Failed(Note),
    /// The host can't be asked (no tmux pane, no push service).
    Unavailable(Note),
}

/// One open Host Setup screen.
#[derive(Debug, Clone)]
pub(crate) struct Page {
    pub connection: Uuid,
    pub host_label: String,
    /// The tmux pane the hook test fires into, when there is one.
    pub test_pane: Option<String>,
    pub phase: Phase,
    pub state: Option<InstallState>,
    pub last_report: Option<InstallReport>,
    pub error: Option<Note>,
    pub hooks_proof: Proof,
    pub push_proof: Proof,
    /// `HookAgent::ALL`'s index.
    pub agent: usize,
    /// The person picked the agent; until then the page follows the host
    /// (the first agent with hooks there, B381).
    pub chosen: bool,
    /// Asking before installing hooks for an agent the host doesn't have
    /// (B381).
    pub confirming: bool,
    /// The relay the page pairs with: the setting, or — when that is
    /// empty — the one the host is already paired with.
    pub relay_url: String,
}

impl Page {
    pub fn new(connection: Uuid, host_label: String, test_pane: Option<String>) -> Self {
        Page {
            connection,
            host_label,
            test_pane,
            phase: Phase::Idle,
            state: None,
            last_report: None,
            error: None,
            hooks_proof: Proof::Untested,
            push_proof: Proof::Untested,
            agent: 0,
            chosen: false,
            confirming: false,
            relay_url: String::new(),
        }
    }

    /// Whether the host has `agent` (the preflight's `agent.<id>` lines,
    /// B374): `None` until it is read, or when the preflight didn't say.
    pub fn found(&self, agent: &HookAgent) -> Option<bool> {
        self.state.as_ref()?.facts.agents.get(agent.id).copied()
    }

    /// The agent the page opens on, once the host is read (B381): the first
    /// with hooks there, else the first the host has, else Claude Code.
    fn first_installed(&self) -> usize {
        let Some(state) = &self.state else {
            return self.agent;
        };
        HookAgent::ALL
            .iter()
            .position(|a| state.hooks_status(a.id) != ComponentStatus::Absent)
            .or_else(|| {
                HookAgent::ALL
                    .iter()
                    .position(|a| self.found(a) == Some(true))
            })
            .unwrap_or(0)
    }

    pub fn agent(&self) -> &'static HookAgent {
        &HookAgent::ALL[self.agent.min(HookAgent::ALL.len() - 1)]
    }

    /// This phone's pairing on the host as recorded: its install's conf
    /// (B443), else the legacy one it wrote before installs were told apart.
    fn pairing_component(&self, manifest: &InstallManifest, install: Option<&str>) -> InstallComponent {
        let conn = self.connection.to_string();
        let own = InstallComponent::Pairing {
            conn: conn.clone(),
            install: install.map(str::to_owned),
        };
        if install.is_some() && manifest.get(&own).is_none() {
            return InstallComponent::Pairing { conn, install: None };
        }
        own
    }

    fn paired_relay(&self, install: Option<&str>) -> Option<String> {
        self.state.as_ref().and_then(|s| {
            s.manifest
                .get(&self.pairing_component(&s.manifest, install))
                .and_then(|c| c.relay_url.clone())
        })
    }
}

/// An agent as the page's menu lists it (B381).
#[derive(Debug, Clone, PartialEq)]
pub struct AgentView {
    pub agent: &'static HookAgent,
    /// Its hooks on this host (absent until the host is read).
    pub hooks: ComponentStatus,
    /// Whether the host has the agent; `None` when that isn't known.
    pub found: Option<bool>,
}

/// What a Host Setup screen shows (`HostSetupSnapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct SetupView {
    pub phase: Phase,
    /// Whether the host has been read: until it is, the statuses below say
    /// nothing (they read absent), and no step is offered but reading it
    /// again (B382).
    pub known: bool,
    pub facts: Option<HostFacts>,
    pub error: Option<Note>,
    /// Paired, with no hook script that will ever send.
    pub nothing_will_push: bool,
    /// The agent the hooks' buttons act on.
    pub agent: &'static HookAgent,
    /// Every agent, with its hooks' status and whether the host has it: the
    /// menu (B381).
    pub agents: Vec<AgentView>,
    /// The question asked before installing hooks for an agent the host
    /// doesn't have (B381).
    pub confirm: Option<Note>,
    /// The registration and the script it calls, the worse of the two.
    pub hooks: ComponentStatus,
    /// The step left on the host before the agent's hooks will run, read
    /// from the host (B380): Codex's hooks installed and current, and no
    /// Codex hook seen running there yet — it skips one until it is trusted.
    pub user_action: Option<Note>,
    pub hooks_proof: Proof,
    /// Why this phone can't push from this host: the phone's reason, else
    /// this host's pairing's (another pairing's failure is its own).
    pub relay_error: Option<Note>,
    /// The relay the host is paired with.
    pub paired_relay: Option<String>,
    pub relay_url: String,
    /// This device's pairing on the host.
    pub pairing: ComponentStatus,
    pub push_proof: Proof,
    pub last_report: Option<InstallReport>,
    /// The full state read from the host, for a screen that wants more.
    pub state: Option<InstallState>,
}

impl Shared {
    pub(crate) fn setup_view(&self, page: &Page) -> SetupView {
        let unavailable = matches!(self.config.push, PushSupport::Unavailable);
        SetupView {
            phase: page.phase.clone(),
            known: page.state.is_some(),
            facts: page.state.as_ref().map(|s| s.facts.clone()),
            error: page.error.clone(),
            nothing_will_push: page
                .state
                .as_ref()
                .is_some_and(InstallState::paired_but_nothing_will_push),
            agent: page.agent(),
            agents: HookAgent::ALL
                .iter()
                .map(|agent| AgentView {
                    agent,
                    hooks: page
                        .state
                        .as_ref()
                        .map_or(ComponentStatus::Absent, |s| s.hooks_status(agent.id)),
                    found: page.found(agent),
                })
                .collect(),
            confirm: page.confirming.then(|| {
                let agent = page.agent();
                Note::AgentNotFound {
                    agent: agent.display_name.to_owned(),
                    dir: format!("~/{}", agent.config_dir()),
                    command: agent
                        .commands
                        .first()
                        .copied()
                        .unwrap_or(agent.id)
                        .to_owned(),
                }
            }),
            hooks: page
                .state
                .as_ref()
                .map_or(ComponentStatus::Absent, |s| s.hooks_status(page.agent().id)),
            user_action: trust_pending(page).then_some(Note::TrustCodexHooks),
            hooks_proof: hooks_proof(page),
            relay_error: if unavailable {
                Some(Note::Relay(RelayFailure::Unavailable))
            } else {
                self.relay_error_for(page.connection)
            },
            paired_relay: page.paired_relay(self.push.install_id.as_deref()),
            relay_url: page.relay_url.clone(),
            pairing: page
                .state
                .as_ref()
                .map_or(ComponentStatus::Absent, |s| self.pairing_status(page, s)),
            push_proof: if unavailable {
                Proof::Unavailable(Note::Relay(RelayFailure::Unavailable))
            } else {
                page.push_proof.clone()
            },
            last_report: page.last_report.clone(),
            state: page.state.clone(),
        }
    }

    /// This phone's pairing on the host, as it is (B372): Current only when
    /// the host's conf is the one this phone's pairing would write — a conf
    /// for a secret the phone no longer holds, or a pairing the phone can't
    /// send with, is Stale.
    fn pairing_status(&self, page: &Page, state: &InstallState) -> ComponentStatus {
        let install = self.push.install_id.as_deref();
        let component = page.pairing_component(&state.manifest, install);
        let status = state.status(&component);
        if status == ComponentStatus::Absent {
            return status;
        }
        let ready = self
            .store
            .pairing(page.connection)
            .filter(|p| !p.send_token.is_empty());
        match ready {
            // Current also while the legacy conf this phone wrote still
            // holds it: care moves it over on its next pass (B443). Muted
            // or not (B700): both are this phone's, and care keeps the
            // switch's state.
            Some(phone)
                if state.manifest.get(&component).and_then(|c| c.digest.as_deref()).is_some_and(|got| {
                    [false, true]
                        .into_iter()
                        .any(|muted| got == scripts::content_digest(&push_conf_as(&phone, muted)))
                }) =>
            {
                ComponentStatus::Current
            }
            _ => ComponentStatus::Stale {
                installed: state
                    .manifest
                    .get(&component)
                    .and_then(|c| c.digest.clone())
                    .unwrap_or_else(|| "unknown".to_owned()),
            },
        }
    }

    fn page(&mut self, id: u64) -> Option<&mut Page> {
        self.changes.insert(Change::Page(id));
        self.pages.get_mut(&id)
    }
}

/// Whether `page`'s agent waits on a step on the host before its hooks run:
/// one that skips a new hook until it is trusted, whose hooks are installed
/// and current, and none of whose hooks the host has seen run (B380).
fn trust_pending(page: &Page) -> bool {
    let agent = page.agent();
    agent.trust_step().is_some()
        && page.state.as_ref().is_some_and(|state| {
            state.hooks_status(agent.id) == ComponentStatus::Current
                && state
                    .facts
                    .marks
                    .as_ref()
                    .is_some_and(|marks| !marks.seen(agent.id))
        })
}

/// The hooks' proof as the page shows it: a test not run yet can only be
/// run on hooks this build installed (B380) — it runs what they register.
fn hooks_proof(page: &Page) -> Proof {
    let agent = page.agent();
    match (&page.hooks_proof, page.state.as_ref()) {
        (Proof::Proving, _) | (_, None) => page.hooks_proof.clone(),
        (proof, Some(state)) => match state.hooks_status(agent.id) {
            ComponentStatus::Current => proof.clone(),
            ComponentStatus::Absent => Proof::Unavailable(Note::HooksNotInstalled {
                agent: agent.display_name.to_owned(),
            }),
            ComponentStatus::Stale { .. } => Proof::Unavailable(Note::HooksOutOfDate {
                agent: agent.display_name.to_owned(),
            }),
        },
    }
}

/// Whether `page`'s hooks can be tested: installed by this build.
pub(crate) fn hooks_testable(page: &Page) -> bool {
    page.state
        .as_ref()
        .is_some_and(|state| state.hooks_status(page.agent().id) == ComponentStatus::Current)
}

/// Change page `id`, if it is still open.
fn update(ctx: &Ctx, id: u64, f: impl FnOnce(&mut Page)) {
    ctx.with(|s| {
        if let Some(page) = s.page(id) {
            f(page);
        }
    });
}

fn read<R>(ctx: &Ctx, id: u64, f: impl FnOnce(&Page) -> R) -> Option<R> {
    ctx.with(|s| s.pages.get(&id).map(f))
}

/// Read the host (`inspect`); the page's relay is seeded from the host's
/// pairing when the setting is empty.
pub(crate) async fn inspect(ctx: Ctx, id: u64) {
    let Some((connection, label)) = read(&ctx, id, |p| (p.connection, p.host_label.clone())) else {
        return;
    };
    update(&ctx, id, |p| {
        p.phase = Phase::Inspecting;
        p.error = None;
    });
    let link = Link::setup(&ctx, connection);
    let result = look(&HostInstaller::new(&link), connection, &label).await;
    let install = ctx.with(|s| s.push.install_id.clone());
    update(&ctx, id, |p| {
        match result {
            Ok(state) => {
                p.state = Some(state);
                if !p.chosen {
                    p.agent = p.first_installed();
                }
            }
            Err(error) => p.error = Some(Note::channel(&error)),
        }
        p.phase = Phase::Idle;
        if p.relay_url.is_empty()
            && let Some(paired) = p.paired_relay(install.as_deref())
        {
            p.relay_url = paired;
        }
    });
}

/// What a step does on the host, given the installer and what the host
/// held.
pub(crate) enum Step {
    InstallHooks,
    RemoveHooks,
    Unpair {
        pass: Option<TaskId>,
    },
    /// Everything off the host (B375), after waiting out `pass`.
    RemoveEverything {
        pass: Option<TaskId>,
    },
}

/// Run a step (`run`): say so, wait out a care pass when told to (bounded),
/// act on what the host held, record the report — unless the screen moved
/// on to another agent — and read the host again.
pub(crate) async fn run(ctx: Ctx, id: u64, step: Step) {
    let Some((connection, agent, host_label)) =
        read(&ctx, id, |p| (p.connection, p.agent, p.host_label.clone()))
    else {
        return;
    };
    let label = match step {
        Step::InstallHooks => Note::InstallingHooks {
            agent: HookAgent::ALL[agent].display_name.to_owned(),
        },
        Step::RemoveHooks => Note::RemovingHooks,
        Step::Unpair { .. } => Note::Unpairing,
        Step::RemoveEverything { .. } => Note::RemovingEverything,
    };
    let for_agent = !matches!(step, Step::Unpair { .. } | Step::RemoveEverything { .. });
    update(&ctx, id, |p| {
        p.phase = Phase::Working(label);
        p.error = None;
    });
    if let Step::Unpair { pass: Some(pass) } | Step::RemoveEverything { pass: Some(pass) } = step {
        let deadline = ctx.with(|s| {
            s.clock
                .ms
                .saturating_add(s.config.budgets.care_wind_down_ms)
        });
        ctx.until(Some(deadline), move |s| {
            (!s.running.contains(&pass)).then_some(())
        })
        .await;
    }
    let link = Link::setup(&ctx, connection);
    let installer = HostInstaller::new(&link);
    let is_current = |ctx: &Ctx| !for_agent || read(ctx, id, |p| p.agent == agent).unwrap_or(false);
    let outcome: Result<(), Stop> = async {
        // What this phone's conf holds, before the phone lets go of it: the
        // legacy conf goes with this install's only when it is this one
        // (B443).
        let mine = ctx.with(|s| {
            s.store
                .pairing(connection)
                .map(|p| scripts::content_digest(&push_conf(&p)))
        });
        let install = ctx.with(|s| s.push.install_id.clone());
        if let Step::Unpair { .. } | Step::RemoveEverything { .. } = step {
            // The phone first: a phone that can't let go stops the unpair
            // before the host is touched. And it stands: care pairs nothing
            // for this connection until Pair (B371) — nor, after Remove
            // everything, asks to put the hooks back (B375).
            ctx.with(|s| {
                push::unpair(s, connection)?;
                s.mark_unpaired(connection, true);
                if let Step::RemoveEverything { .. } = step {
                    s.mark_declined(connection);
                }
                Ok::<(), RelayFailure>(())
            })?;
        }
        // The host as it is now, not as the page last read it: another phone
        // may have changed it since (B373).
        let current = look(&installer, connection, &host_label).await?;
        let hook_agent = &HookAgent::ALL[agent];
        let report = match step {
            Step::InstallHooks => {
                installer
                    .install_hooks(hook_agent, &current, ctx.now().date)
                    .await?
            }
            Step::RemoveHooks => {
                let report = installer.uninstall_hooks(hook_agent, &current).await?;
                // Taken off by hand: care doesn't ask to put hooks back on
                // this host, as after a "no" (B374).
                if report.succeeded() {
                    ctx.with(|s| s.mark_declined(connection));
                }
                report
            }
            Step::Unpair { .. } => {
                let components = crate::care::own_pairings(
                    &current.manifest,
                    &connection.to_string(),
                    install.as_deref(),
                    mine.as_deref(),
                );
                installer.uninstall(&components, &current).await?;
                InstallReport {
                    facts: current.facts.clone(),
                    steps: components
                        .into_iter()
                        .map(|component| moshpit_install::installer::InstallStep {
                            component,
                            action: moshpit_install::installer::Action::Removed,
                        })
                        .collect(),
                    manifest: None,
                    blocked: None,
                    user_action_required: None,
                }
            }
            Step::RemoveEverything { .. } => {
                let conn = connection.to_string();
                let components =
                    crate::care::own_pairings(&current.manifest, &conn, install.as_deref(), mine.as_deref());
                installer.remove_everything_of(&components, &current).await?
            }
        };
        let still = is_current(&ctx);
        if still {
            update(&ctx, id, |p| {
                p.last_report = Some(report.clone());
            });
        }
        let after = look(&installer, connection, &host_label).await?;
        update(&ctx, id, |p| {
            p.state = Some(after);
            if still && !report.succeeded() {
                p.error = first_failure(&report);
            }
        });
        Ok(())
    }
    .await;
    if let Err(stop) = outcome
        && is_current(&ctx)
    {
        update(&ctx, id, |p| p.error = Some(describe(stop)));
    }
    update(&ctx, id, |p| p.phase = Phase::Idle);
}

/// The report's first failure, else why nothing was attempted.
fn first_failure(report: &InstallReport) -> Option<Note> {
    report
        .failure_reasons()
        .next()
        .map(|why| Note::Step(why.clone()))
        .or_else(|| report.blocked.as_ref().map(Note::blocked))
}

fn describe(stop: Stop) -> Note {
    match stop {
        Stop::Channel(error) => Note::channel(&error),
        Stop::Relay(failure) => Note::Relay(failure),
        Stop::Cancelled => Note::Said(crate::exec::CANCELLED.to_owned()),
    }
}

/// Pair the host with `relay`, then prove it by making the host send one
/// real push (`pair`). The pairing is minted and registered before the host
/// is touched; one pairing operation per host at a time (B123): this waits
/// out a care pass pairing it, or care's waits out this, and the host is
/// read again after a wait.
pub(crate) async fn pair(ctx: Ctx, id: u64) {
    let Some((connection, host_label, relay)) = read(&ctx, id, |p| {
        (p.connection, p.host_label.clone(), p.relay_url.clone())
    }) else {
        return;
    };
    let trimmed = moshpit_seal::foundation::text::trim_white_space(&relay).to_owned();
    if trimmed.is_empty() {
        update(&ctx, id, |p| p.error = Some(Note::EnterRelay));
        return;
    }
    update(&ctx, id, |p| {
        p.phase = Phase::Working(Note::MintingSendToken);
        p.error = None;
        p.push_proof = Proof::Untested;
    });
    // Pairing by hand ends an Unpair (B371).
    ctx.with(|s| s.mark_unpaired(connection, false));
    let link = Link::setup(&ctx, connection);
    let installer = HostInstaller::new(&link);
    // What the phone held before: a Re-pair that fails goes back to it (B372).
    let previous = ctx.with(|s| s.store.pairing(connection));
    let outcome: Result<InstallReport, Stop> = async {
        let ticket = LaneTicket::enter(&ctx, connection)
            .await
            .ok_or(Stop::Cancelled)?;
        let pairing = push::pair(&ctx, connection, &host_label, &trimmed).await?;
        update(&ctx, id, |p| {
            p.phase = Phase::Working(Note::InstallingOn {
                host: host_label.clone(),
            })
        });
        // The host as it is now, not as the page last read it (B373).
        let current = look(&installer, connection, &host_label).await?;
        let install = ctx.with(|s| s.push.install_id.clone());
        let report = installer
            .install_pairing_as(&pairing, install.as_deref(), &current, ctx.now().date)
            .await?;
        drop(ticket);
        update(&ctx, id, |p| p.last_report = Some(report.clone()));
        let after = look(&installer, connection, &host_label).await?;
        update(&ctx, id, |p| p.state = Some(after));
        Ok(report)
    }
    .await;
    if !matches!(&outcome, Ok(report) if report.succeeded())
        && let Some(previous) = previous
    {
        // The conf's write went through when the report has its step (only
        // its read-back can have failed): the host holds the new conf.
        let landed = matches!(&outcome, Ok(report) if report.steps.iter().any(|step| {
            matches!(step.component, InstallComponent::Pairing { .. })
        }));
        if !landed {
            keep_what_worked(&ctx, id, &installer, &host_label, previous).await;
        }
    }
    match outcome {
        Ok(report) => {
            update(&ctx, id, |p| p.phase = Phase::Idle);
            if report.succeeded() {
                prove_push(ctx, id).await;
            } else {
                update(&ctx, id, |p| p.error = first_failure(&report));
            }
        }
        Err(stop) => update(&ctx, id, |p| {
            p.phase = Phase::Idle;
            p.error = Some(describe(stop));
        }),
    }
}

/// A Re-pair that failed (B372): the fresh secret was saved on the phone
/// before the relay was asked, so a mint the relay refused, or an install
/// that didn't land, left the phone with a secret the host's conf doesn't
/// carry — a working pairing broken, while the page still said Current.
/// The pairing the phone held before comes back, unless the host took the
/// new conf after all (its digest is the new pairing's; a host that can't be
/// read is taken not to have), and only over the pairing this attempt
/// staged (a delete meanwhile stays a delete).
async fn keep_what_worked(
    ctx: &Ctx,
    id: u64,
    installer: &HostInstaller<'_, Link>,
    host_label: &str,
    previous: PushPairing,
) {
    let Some(connection) = read(ctx, id, |p| p.connection) else {
        return;
    };
    let staged = ctx.with(|s| s.store.pairing(connection));
    let Some(staged) = staged.filter(|p| p.secret_hex != previous.secret_hex) else {
        return;
    };
    let host = look(installer, connection, host_label).await.ok();
    if !staged.send_token.is_empty()
        && host
            .as_ref()
            .is_some_and(|state| {
                let install = ctx.with(|s| s.push.install_id.clone());
                !pairing_needs_rewrite_as(&state.manifest, &staged, install.as_deref())
            })
    {
        return;
    }
    ctx.with(|s| {
        let still = s.store.pairing(connection);
        if still.is_some_and(|p| p.secret_hex == staged.secret_hex) && s.store.upsert(&previous) {
            log::info!(target: "moshpit::push", "re-pair of {host_label} failed \u{2014} kept the pairing that worked");
            s.changed(Change::Push);
        }
    });
    if let Some(state) = host {
        update(ctx, id, |p| p.state = Some(state));
    }
}

/// Ask the host to send one real push and wait for it to arrive here
/// (`provePush`): host → relay → Apple → this phone, with a fresh nonce so
/// an earlier test's push proves nothing. The wait starts before the host
/// is asked: on a fast link the push beats the command's own answer.
pub(crate) async fn prove_push(ctx: Ctx, id: u64) {
    let Some(connection) = read(&ctx, id, |p| p.connection) else {
        return;
    };
    update(&ctx, id, |p| {
        p.phase = Phase::Proving;
        p.push_proof = Proof::Proving;
    });
    // The pairing the test is about: one replaced or let go of while it
    // runs (a delete, care) makes its verdict about nothing (B383).
    let pairing = |ctx: &Ctx| ctx.with(|s| s.store.pairing(connection).map(|p| p.secret_hex));
    let tested = pairing(&ctx);
    let nonce = make_nonce();
    let deadline = ctx.with(|s| {
        s.push.waiting.insert(nonce.clone());
        s.clock.ms.saturating_add(s.config.budgets.self_test_ms)
    });
    let link = Link::setup(&ctx, connection);
    // This connection's pairing only: another phone paired with the host
    // isn't waiting for the test, and would show it (B353).
    let fired = HostInstaller::new(&link)
        .self_test_push(&nonce, &connection.to_string())
        .await;
    let proof = match fired {
        // B359: the sender prints nothing for a push the relay took, and
        // it tests this connection's pairing alone (B353) — so anything it
        // says is why the push isn't coming, and is the answer at once
        // (unless the push beat its words here). The 20 s wait is only for
        // a push that was sent and is on its way.
        Ok(output) => {
            let arrived = ctx.with(|s| s.push.arrived.contains(&nonce));
            if arrived || (output.is_empty() && push::self_test(&ctx, &nonce, deadline).await) {
                Proof::Proven(ctx.now().date)
            } else if output.is_empty() {
                Proof::Failed(Note::PushNeverSeen)
            } else {
                Proof::Failed(Note::HostReported(output))
            }
        }
        // Fix (prove-push-exec-fails): the app's `async let` wait outlived
        // a failed command and held the screen on "Testing…" for the
        // whole 20 s before it said why.
        Err(error) => Proof::Failed(Note::channel(&error)),
    };
    ctx.with(|s| {
        s.push.waiting.remove(&nonce);
        push::forget_self_test(s, &nonce);
    });
    let proof = if pairing(&ctx) == tested {
        proof
    } else {
        Proof::Untested
    };
    update(&ctx, id, |p| {
        p.push_proof = proof;
        p.phase = Phase::Idle;
    });
}

/// Run the agent's hooks as its config registers them and read the pane
/// back (`proveHooks`, B380): the config must hold Moshpit's command for
/// every event this build registers, and the `working` one runs in the
/// session's pane, labelled as the test. It says nothing of the trust step
/// (the agent isn't involved), which only the host's record of a real run
/// settles.
pub(crate) async fn prove_hooks(ctx: Ctx, id: u64) {
    let Some((connection, pane, agent, config_path)) = read(&ctx, id, |p| {
        let hook_agent = p.agent();
        let config_path = p
            .state
            .as_ref()
            .and_then(|s| {
                s.manifest.get(&InstallComponent::Hooks {
                    agent: hook_agent.id.to_owned(),
                })
            })
            .and_then(|c| c.config_path.clone())
            .unwrap_or_else(|| hook_agent.config_path.to_owned());
        (p.connection, p.test_pane.clone(), p.agent, config_path)
    }) else {
        return;
    };
    let Some(pane) = pane.filter(|p| !p.is_empty()) else {
        update(&ctx, id, |p| {
            p.hooks_proof = Proof::Unavailable(Note::OpenTmuxPane)
        });
        return;
    };
    update(&ctx, id, |p| {
        p.phase = Phase::Proving;
        p.hooks_proof = Proof::Proving;
    });
    let hook_agent = &HookAgent::ALL[agent];
    let link = Link::setup(&ctx, connection);
    let outcome = HostInstaller::new(&link)
        .self_test_registration(hook_agent, &config_path, &pane)
        .await;
    let now = ctx.now().date;
    update(&ctx, id, |p| {
        p.phase = Phase::Idle;
        // The agent can't change while a proof runs (B384); a page that
        // somehow moved on doesn't take another agent's verdict (B235).
        if p.agent != agent {
            return;
        }
        p.hooks_proof = match outcome {
            Ok(HookProof::Stamped) => Proof::Proven(now),
            Ok(HookProof::Unavailable) => Proof::Unavailable(Note::TmuxElsewhere),
            Ok(HookProof::DidNotStamp(got)) if got.is_empty() => {
                Proof::Failed(Note::StampEmpty { trust_hint: false })
            }
            Ok(HookProof::DidNotStamp(got)) => Proof::Failed(Note::StampOther(got)),
            Ok(HookProof::NotRegistered { event }) => Proof::Failed(Note::NotRegistered {
                config: hook_agent.config_hint(),
                event,
            }),
            Ok(HookProof::Unreadable(failure)) => {
                Proof::Failed(Note::Step(StepFailure::Config(failure)))
            }
            Ok(HookProof::UnsafePath(path)) => {
                Proof::Failed(Note::Step(StepFailure::UnsafePath(path)))
            }
            Err(error) => Proof::Failed(Note::channel(&error)),
        };
    });
}

/// Act on `agent` from now on (`select`): what was said about the last one
/// goes with it; the pairing and its proof are the host's, and stay.
pub(crate) fn select(page: &mut Page, agent: usize) -> bool {
    page.chosen = true;
    if agent == page.agent {
        return false;
    }
    page.confirming = false;
    page.agent = agent;
    page.hooks_proof = Proof::Untested;
    page.last_report = None;
    page.error = None;
    true
}
