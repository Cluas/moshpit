//! Host care (`HostAutoCare`): every time a session's control plane comes
//! up, its host is walked once and whatever drifted is put right, silently —
//! scripts an app update replaced, this phone's pairing missing or stale on
//! the host, no pairing at all. The one thing it never does is install
//! hooks on a host that has none: that first write edits the user's agent
//! config and needs a yes, asked once ([`crate::HostCare::consent`]).
//!
//! It acts only for a connection that is saved and not on its way out: a
//! delete or a Host Setup unpair stands it down first, cancelling the pass
//! out for the connection, and a pass asks again before every step that
//! would act (B70, B88). Pairing goes through the connection's lane: one
//! pairing operation per host at a time (B123).

use std::collections::HashSet;

use moshpit_install::installer::{
    self, ChannelError, ComponentStatus, HostInstaller, InstallState, installed_hook_agents,
    pairing_needs_rewrite_as,
};
use moshpit_install::{HookAgent, InstallComponent};
use moshpit_seal::Uuid;

use crate::exec::{Ctx, LaneTicket, Link, Shared, TaskId, look};
use crate::note::{Note, RelayFailure};
use crate::{Change, Conn, push};

/// What care remembers between passes.
#[derive(Debug, Default)]
pub(crate) struct CareState {
    /// The connection whose host has no hooks yet: the cue to ask.
    pub needs_consent: Option<Conn>,
    /// Tended this app run: drift comes from app updates, not minute to
    /// minute.
    pub tended: HashSet<Uuid>,
    /// The pass out for each connection (the one a delete cancels).
    pub in_flight: std::collections::HashMap<Uuid, TaskId>,
    /// Deletes begun and not ended.
    pub deleting: HashSet<Uuid>,
    /// Host Setup unpairs begun and not ended.
    pub unpairing: HashSet<Uuid>,
    /// Deleted connections whose pairing is on its way off the host (B377).
    pub forgetting: HashSet<Uuid>,
}

/// Why a pass stopped.
#[derive(Debug)]
pub(crate) enum Stop {
    Channel(ChannelError),
    Relay(RelayFailure),
    Cancelled,
}

impl From<ChannelError> for Stop {
    fn from(error: ChannelError) -> Self {
        Stop::Channel(error)
    }
}

impl From<RelayFailure> for Stop {
    fn from(failure: RelayFailure) -> Self {
        Stop::Relay(failure)
    }
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stop::Channel(error) => f.write_str(&error.0),
            Stop::Relay(failure) => f.write_str(&Note::Relay(failure.clone()).english()),
            Stop::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl Shared {
    /// Saved, and neither being deleted nor unpaired (`isKept`).
    pub fn is_kept(&self, connection: Uuid) -> bool {
        !self.care.deleting.contains(&connection)
            && !self.care.unpairing.contains(&connection)
            && self.saved.listed.contains(&connection)
    }

    /// Whether `task` may still act for `connection` (`mayAct`).
    pub fn may_act(&self, task: TaskId, connection: Uuid) -> bool {
        !self.cancelled.contains(&task) && self.is_kept(connection)
    }

    /// Cancel the pass out for `connection`; it, for a delete or an unpair
    /// to wait out.
    pub fn stand_down(&mut self, connection: Uuid) -> Option<TaskId> {
        let pass = self.care.in_flight.get(&connection).copied();
        if let Some(pass) = pass
            && self.running.contains(&pass)
        {
            self.cancelled.insert(pass);
        }
        pass
    }

    fn declined(&self, connection: Uuid) -> bool {
        let id = connection.to_string();
        self.settings.declined.contains(&id)
    }

    /// The person unpaired `connection` in Host Setup and hasn't paired it
    /// since (B371).
    pub(crate) fn unpaired(&self, connection: Uuid) -> bool {
        let id = connection.to_string();
        self.settings.unpaired.contains(&id)
    }

    /// Remember that care is not to ask about `connection` again, here and
    /// where the platform keeps it: the person said no, or took the hooks
    /// off by hand (B374).
    pub(crate) fn mark_declined(&mut self, connection: Uuid) {
        let id = connection.to_string();
        if !self.settings.declined.contains(&id) {
            self.settings.declined.push(id);
        }
        self.outputs
            .push_back(crate::Output::RememberDeclined { connection });
    }

    /// Remember (`unpaired`) or forget an Unpair of `connection`, here and
    /// where the platform keeps it.
    pub(crate) fn mark_unpaired(&mut self, connection: Uuid, unpaired: bool) {
        let id = connection.to_string();
        let had = self.settings.unpaired.contains(&id);
        if unpaired && !had {
            self.settings.unpaired.push(id);
        } else if !unpaired && had {
            self.settings.unpaired.retain(|c| *c != id);
        } else {
            return;
        }
        self.outputs.push_back(crate::Output::RememberUnpaired {
            connection,
            unpaired,
        });
    }
}

fn may_act(ctx: &Ctx, connection: Uuid) -> bool {
    ctx.with(|s| s.may_act(ctx.task, connection))
}

/// One care pass (`tendNow`); silent by design, the sheet is the
/// diagnostic surface.
pub(crate) async fn pass(ctx: Ctx, conn: Conn) {
    let link = Link::care(&ctx, conn.id);
    let installer = HostInstaller::new(&link);
    if let Err(stop) = tend(&ctx, &conn, &installer).await {
        log::info!(target: "moshpit::care", "autocare: {} skipped: {stop}", conn.label);
    }
    ctx.with(|s| finish(s, conn.id, ctx.task));
}

/// The pass for `connection` ends: it is no longer the one out.
fn finish(shared: &mut Shared, connection: Uuid, task: TaskId) {
    if shared.care.in_flight.get(&connection) == Some(&task) {
        shared.care.in_flight.remove(&connection);
    }
}

async fn tend(ctx: &Ctx, conn: &Conn, installer: &HostInstaller<'_, Link>) -> Result<(), Stop> {
    let state = look(installer, conn.id, &conn.label).await?;
    // Reading the host takes a while; the connection may be on its way out.
    if !may_act(ctx, conn.id) {
        log::info!(target: "moshpit::care", "autocare: {} is being deleted or no longer saved \u{2014} nothing done", conn.label);
        return Ok(());
    }
    let agents = ctx.with(|s| {
        s.care.tended.insert(conn.id);
        let agents = installed_hook_agents(&state.manifest);
        if agents.is_empty() && !s.declined(conn.id) {
            // No hooks: nothing was consented to on this host. Ask.
            s.care.needs_consent = Some(conn.clone());
            s.changed(Change::Consent);
        }
        agents
    });
    if agents.is_empty() {
        return Ok(());
    }
    // Scripts drift when the app updates; bring every consented agent's
    // registration current. The sender is checked explicitly:
    // `hooks_status` folds in the stamp script but not the sender. Each
    // agent's status is read from the host as it is after the one before.
    let mut current = state;
    let sender_stale = current.status(&InstallComponent::Sender) != ComponentStatus::Current;
    for agent in agents {
        if !sender_stale && current.hooks_status(agent.id) == ComponentStatus::Current {
            continue;
        }
        if !may_act(ctx, conn.id) {
            return Ok(());
        }
        log::info!(target: "moshpit::care", "autocare: refreshing {} hooks on {}", agent.id, conn.label);
        installer
            .install_hooks(&agent, &current, ctx.now().date)
            .await?;
        current = look(installer, conn.id, &conn.label).await?;
    }
    ensure_pairing(ctx, conn, installer, current).await
}

/// Care's pairing step, in the connection's lane: an operation waited out
/// first (Host Setup's Pair) has changed what phone and host hold, so the
/// host is read again.
async fn ensure_pairing(
    ctx: &Ctx,
    conn: &Conn,
    installer: &HostInstaller<'_, Link>,
    state: InstallState,
) -> Result<(), Stop> {
    if !ctx.with(|s| s.settings.notifications) {
        return Ok(());
    }
    // B371: an Unpair in Host Setup stands until the person pairs again;
    // care used to pair the connection again at its next session.
    if ctx.with(|s| s.unpaired(conn.id)) {
        log::info!(target: "moshpit::care", "autocare: {} was unpaired by hand \u{2014} not pairing it", conn.label);
        return Ok(());
    }
    let Some(ticket) = LaneTicket::enter(ctx, conn.id).await else {
        return Err(Stop::Cancelled);
    };
    let current = if ticket.waited {
        look(installer, conn.id, &conn.label).await?
    } else {
        state
    };
    let result = ensure_pairing_now(ctx, conn, installer, current).await;
    drop(ticket);
    result
}

async fn ensure_pairing_now(
    ctx: &Ctx,
    conn: &Conn,
    installer: &HostInstaller<'_, Link>,
    state: InstallState,
) -> Result<(), Stop> {
    // The moment care touches a pairing the connection must still be saved
    // and not being deleted: nothing between here and the pairing being
    // saved waits.
    if !may_act(ctx, conn.id) {
        log::info!(target: "moshpit::care", "autocare: not pairing {} \u{2014} it is being deleted or no longer saved", conn.label);
        return Ok(());
    }
    let (local, relay) = ctx.with(|s| {
        (
            s.store.pairing(conn.id),
            moshpit_seal::pairing::normalise_relay_url(&s.settings.relay_url),
        )
    });
    if let Some(mut local) = local {
        // The name the phone gives the connection now (B440): the pairing
        // kept the one it was paired under, and a pushed card names its
        // place by the pairing's label — after a rename the pushed copy of
        // an event and the local one read differently. The label never
        // leaves the phone (the conf doesn't carry it), so the host is not
        // touched; a mint in flight keeps it (it stores over what's stored).
        if !conn.label.trim().is_empty() && local.host_label != conn.label {
            log::info!(target: "moshpit::care", "autocare: the pairing for {} takes the connection's new name", conn.label);
            local.host_label = conn.label.clone();
            ctx.with(|s| s.store.upsert(&local));
        }
        // Heal a pairing minted while the relay setting was wrong: the
        // secret is random, only the address (and the token the old relay
        // issued) need replacing.
        if local.relay_url != relay {
            log::info!(target: "moshpit::care", "autocare: correcting relay address on {}", conn.label);
            local.relay_url = relay;
            local.send_token = String::new();
            ctx.with(|s| s.store.upsert(&local));
        }
        let Some(ready) = push::ensure_ready(ctx, conn.id).await else {
            log::info!(target: "moshpit::care", "autocare: pairing for {} not ready \u{2014} will retry next pass", conn.label);
            return Ok(());
        };
        // The relay took a while.
        if !may_act(ctx, conn.id) {
            return Ok(());
        }
        // The host's copy must match ours byte for byte.
        let mut state = state;
        let install = ctx.with(|s| s.push.install_id.clone());
        if pairing_needs_rewrite_as(&state.manifest, &ready, install.as_deref()) {
            log::info!(target: "moshpit::care", "autocare: rewriting pairing on {}", conn.label);
            let report = installer
                .install_pairing_as(&ready, install.as_deref(), &state, ctx.now().date)
                .await?;
            match report.manifest {
                Some(manifest) if report.succeeded() => state.manifest = manifest,
                _ => return Ok(()),
            }
        }
        return forget_left_behind(ctx, conn, installer, &state).await;
    }
    // Never paired from this device: mint, register, install.
    log::info!(target: "moshpit::care", "autocare: pairing {}", conn.label);
    let pairing = push::pair(ctx, conn.id, &conn.label, &relay).await?;
    if !may_act(ctx, conn.id) {
        return Ok(());
    }
    let install = ctx.with(|s| s.push.install_id.clone());
    let report = installer
        .install_pairing_as(&pairing, install.as_deref(), &state, ctx.now().date)
        .await?;
    match report.manifest {
        Some(manifest) if report.succeeded() => {
            let state = InstallState { manifest, ..state };
            forget_left_behind(ctx, conn, installer, &state).await
        }
        _ => Ok(()),
    }
}

/// Take off the host the pairings this phone has let go of (B354).
///
/// Deleting a connection forgets its pairing on the phone, but its conf
/// stayed on the host, and the relay keeps no registry to revoke its send
/// token with: for the token's 45 days every agent turn on that host was
/// sealed with a secret the phone no longer has and arrived as the fallback
/// card. Worse beside a connection still kept to the same host: the sender
/// pushes once per phone (B313), and when the forgotten conf came first it
/// took the push, the live pairing was skipped, and the phone never saw a
/// real card from that host again.
///
/// A conf goes when it pushes to this phone's APNs token and names a
/// connection that is neither saved here nor paired here. Only once this
/// connection's own pairing is in place and every saved connection can be
/// told: an unreadable store never reads as "every pairing let go of".
async fn forget_left_behind(
    ctx: &Ctx,
    conn: &Conn,
    installer: &HostInstaller<'_, Link>,
    state: &InstallState,
) -> Result<(), Stop> {
    let Some(phone) = ctx.with(|s| s.push.device_token.clone()) else {
        return Ok(());
    };
    let reaching = installer.pairings_reaching(&phone).await?;
    let gone: Vec<InstallComponent> = ctx.with(|s| {
        // Every connection the store holds, decoded or not; while that
        // can't be told (before the first unlock), nothing is judged.
        let Some(saved) = s.saved.all.as_ref() else {
            return Vec::new();
        };
        let paired: HashSet<Uuid> = s.store.read().iter().map(|p| p.connection_id).collect();
        let mine = s.push.install_id.as_deref();
        reaching
            .into_iter()
            .filter(|component| {
                let InstallComponent::Pairing { conn, install } = component else {
                    return false;
                };
                let Some(id) = Uuid::parse(conn) else {
                    return false;
                };
                let let_go = !paired.contains(&id) && !saved.contains(&id) && !s.saved.listed.contains(&id);
                // Another install's conf pushing to this phone's token is an
                // earlier install of the app on this very phone: its secret is
                // gone with it (B443).
                let earlier_install = matches!((install.as_deref(), mine), (Some(theirs), Some(mine)) if theirs != mine);
                // So is a legacy conf pushing to this phone's token once this
                // install has its own: an APNs token is the device's and the
                // app install's, and a phone restored from another's backup is
                // handed a new one — a conf that pushes to this token can only
                // be this phone's. One the move over didn't take (its digest
                // is an older conf of this phone's, minted again since) went on
                // costing a refused push each time it sorted first.
                let own_legacy = install.is_none() && mine.is_some();
                let_go || earlier_install || own_legacy
            })
            .collect()
    });
    if gone.is_empty() || !may_act(ctx, conn.id) {
        return Ok(());
    }
    log::info!(target: "moshpit::care", "autocare: removing {} pairing(s) this phone let go of from {}", gone.len(), conn.label);
    installer.uninstall(&gone, state).await?;
    Ok(())
}

/// This phone's pairings of `conn` on a host (B443): its install's conf,
/// and the legacy `<conn>.conf` while no install is known — or when its
/// recorded digest is this phone's conf (`mine`), what it wrote before
/// installs were told apart. A legacy conf with another digest is another
/// phone's, and is never this phone's to take.
pub(crate) fn own_pairings(
    manifest: &moshpit_install::InstallManifest,
    conn: &str,
    install: Option<&str>,
    mine: Option<&str>,
) -> Vec<InstallComponent> {
    let legacy = InstallComponent::Pairing {
        conn: conn.to_owned(),
        install: None,
    };
    let Some(install) = install else {
        return vec![legacy];
    };
    let mut own = vec![InstallComponent::Pairing {
        conn: conn.to_owned(),
        install: Some(install.to_owned()),
    }];
    if mine.is_some() && manifest.get(&legacy).and_then(|c| c.digest.as_deref()) == mine {
        own.push(legacy);
    }
    own
}

/// A deleted connection's pairing taken off its host while the session is
/// still up (B377): its conf and its manifest entry. Deleting a connection
/// forgot the pairing on the phone only, and the host went on pushing with
/// it — the fallback card, or, beside another connection to the host, the
/// one push per phone spent on it (B354 takes it off at the host's next
/// pass, if there is one). Best effort: a host that can't be reached keeps
/// it until then.
pub(crate) async fn forget_on_host(ctx: Ctx, connection: Uuid) {
    let link = Link::forget(&ctx, connection);
    let installer = HostInstaller::new(&link);
    let state = InstallState {
        facts: moshpit_install::HostFacts::default(),
        manifest: moshpit_install::InstallManifest::empty(),
    };
    // This install's conf; the legacy one goes too while no install is
    // known (a build before B443's install id). Once one is, a legacy conf
    // left pushing to this phone is B354's, at the host's next pass.
    let install = ctx.with(|s| s.push.install_id.clone());
    let component = InstallComponent::Pairing {
        conn: connection.to_string(),
        install,
    };
    match installer
        .uninstall(std::slice::from_ref(&component), &state)
        .await
    {
        Ok(_) => log::info!(target: "moshpit::care", "delete: took its pairing off the host"),
        Err(error) => {
            log::info!(target: "moshpit::care", "delete: the host keeps its pairing for now: {}", error.0)
        }
    }
    ctx.with(|s| {
        s.care.forgetting.remove(&connection);
    });
}

/// The user said yes (`grantConsent`): the one consented write — Claude
/// Code's hook registration — then the pairing, after whatever pass was out.
pub(crate) async fn consented(ctx: Ctx, conn: Conn, previous: Option<TaskId>) {
    if let Some(previous) = previous {
        ctx.until(None, move |s| {
            (!s.running.contains(&previous)).then_some(())
        })
        .await;
    }
    let link = Link::care(&ctx, conn.id);
    let installer = HostInstaller::new(&link);
    let result = async {
        if !may_act(&ctx, conn.id) {
            return Ok(());
        }
        let mut state = look(&installer, conn.id, &conn.label).await?;
        // The agents this host has (B374): consent used to mean Claude
        // Code's hooks whether or not Claude Code was there. A host the
        // preflight found none on (or an answer from before it asked) still
        // gets Claude Code's.
        let mut agents: Vec<HookAgent> = HookAgent::ALL
            .into_iter()
            .filter(|agent| state.facts.has_agent(agent.id))
            .collect();
        if agents.is_empty() {
            agents.push(HookAgent::named("claude").expect("Claude Code is an agent"));
        }
        for agent in agents {
            if !may_act(&ctx, conn.id) {
                return Ok(());
            }
            installer
                .install_hooks(&agent, &state, ctx.now().date)
                .await?;
            state = look(&installer, conn.id, &conn.label).await?;
        }
        ensure_pairing(&ctx, &conn, &installer, state).await
    }
    .await;
    if let Err(stop) = result {
        log::info!(target: "moshpit::care", "autocare: enable on {} failed: {stop}", conn.label);
    }
    ctx.with(|s| finish(s, conn.id, ctx.task));
}

/// The agents care refreshes on a host: what was consented to there.
pub fn consented_agents(manifest: &moshpit_install::InstallManifest) -> Vec<HookAgent> {
    installer::installed_hook_agents(manifest)
}
