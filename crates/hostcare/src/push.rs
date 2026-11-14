//! The phone's side of push pairing (`PushService`, less what needs the
//! OS): minting a pairing's send token at its relay and keeping it fresh,
//! pairing and unpairing a connection, the device token's waiters, the
//! self-test pushes a screen waits for, and the pairings left by deleted
//! connections. Asking the OS for a device token, and the HTTP itself, are
//! outputs ([`crate::Output::RegisterForPush`], [`crate::Output::Post`]).

use std::collections::{BTreeMap, HashSet};

use moshpit_seal::service::{self, MintRounds};
use moshpit_seal::token::{MintRequest, MintResponse};
use moshpit_seal::{Date, PushPairing, Uuid};

use crate::exec::{Ctx, Shared};
use crate::note::{Note, RelayFailure};
use crate::{Change, PushSupport};

/// What the phone's push side remembers.
#[derive(Debug, Default)]
pub(crate) struct PushState {
    /// The current device token, hex (`PushService.deviceToken`).
    pub device_token: Option<String>,
    /// Why this phone can't be pushed to at all: iOS refused it a device
    /// token. A new token clears it.
    pub phone_error: Option<Note>,
    /// Why a pairing has no usable credential: its last mint's failure,
    /// until a mint of it works (or it is forgotten). The app's
    /// `lastError` was one for the phone: a pairing minted fine stood next
    /// to another's refusal until a whole round came back clean.
    pub pairing_errors: BTreeMap<Uuid, Note>,
    /// A mint failed, or couldn't run for want of a token.
    pub sync_pending: bool,
    pub rounds: MintRounds,
    pub last_success: Option<Date>,
    /// Self-test nonces that arrived and haven't been forgotten.
    pub arrived: HashSet<String>,
    /// Nonces a screen is waiting for.
    pub waiting: HashSet<String>,
    /// Waits given up on while still waiting (`forgetSelfTest` resumes a
    /// parked waiter rather than leave it).
    pub abandoned: HashSet<String>,
    /// This launch's sweep of pairings whose connection is gone ran.
    pub orphans_swept: bool,
}

impl Shared {
    fn environment(&self) -> Option<String> {
        match &self.config.push {
            PushSupport::Apns { environment } => Some(environment.clone()),
            PushSupport::Unavailable => None,
        }
    }

    /// `registerForRemoteNotifications`: only with something paired — an
    /// install with nothing paired must not prompt.
    pub fn register_for_push(&mut self) {
        if self.environment().is_none() {
            return;
        }
        let count = self.store.read().len();
        if count == 0 {
            log::info!(target: "moshpit::push", "no paired hosts \u{2014} not registering for remote notifications");
            return;
        }
        self.outputs.push_back(crate::Output::RegisterForPush);
    }

    pub fn set_phone_error(&mut self, error: Option<Note>) {
        if self.push.phone_error != error {
            self.push.phone_error = error;
            self.changed(Change::Push);
        }
    }

    pub fn set_pairing_error(&mut self, connection: Uuid, error: Option<Note>) {
        let moved = match error {
            Some(error) => self.push.pairing_errors.insert(connection, error.clone()) != Some(error),
            None => self.push.pairing_errors.remove(&connection).is_some(),
        };
        if moved {
            self.changed(Change::Push);
        }
    }

    /// Why `connection`'s host can't be pushed from this phone: the
    /// phone's reason, else its pairing's.
    pub fn relay_error_for(&self, connection: Uuid) -> Option<Note> {
        self.push
            .phone_error
            .clone()
            .or_else(|| self.push.pairing_errors.get(&connection).cloned())
    }

    /// Why this phone has no usable relay credential: its own reason,
    /// else a failing pairing's.
    pub fn relay_error(&self) -> Option<Note> {
        self.push
            .phone_error
            .clone()
            .or_else(|| self.push.pairing_errors.values().next().cloned())
    }
}

/// Wait up to the token budget for iOS to hand over a device token.
async fn device_token(ctx: &Ctx) -> Option<String> {
    let deadline = ctx.with(|s| {
        s.push
            .device_token
            .is_none()
            .then(|| s.clock.ms.saturating_add(s.config.budgets.device_token_ms))
    });
    let Some(deadline) = deadline else {
        return ctx.with(|s| s.push.device_token.clone());
    };
    ctx.until(Some(deadline), |s| s.push.device_token.clone())
        .await
}

/// Ask `relay_url` for a send token over `apns_token` and `connection`
/// (`PushRelayClient.mint`): one retry, two seconds on, on a NETWORK
/// failure only — a status is the relay's answer.
async fn mint(
    ctx: &Ctx,
    apns_token: &str,
    connection: Uuid,
    relay_url: &str,
) -> Result<(String, Date), RelayFailure> {
    let url = service::mint_url(relay_url).map_err(|_| RelayFailure::BadUrl)?;
    let body = serde_json::to_string(&MintRequest {
        apns_token: apns_token.to_owned(),
        conn: connection.to_string(),
    })
    .expect("a mint request serializes");
    let timeout = ctx.with(|s| s.config.budgets.mint_timeout_seconds);
    let reply = match ctx.post(url.clone(), body.clone(), timeout).await {
        Ok(reply) => reply,
        Err(_) => {
            let retry = ctx.with(|s| s.config.budgets.mint_retry_ms);
            if !ctx.sleep(retry).await {
                return Err(RelayFailure::Network(crate::exec::CANCELLED.into()));
            }
            ctx.post(url, body, timeout)
                .await
                .map_err(RelayFailure::Network)?
        }
    };
    if reply.status != 200 {
        let prefix = &reply.body[..reply.body.len().min(200)];
        return Err(RelayFailure::Status {
            code: reply.status,
            body: String::from_utf8(prefix.to_vec()).unwrap_or_default(),
        });
    }
    let minted = MintResponse::from_json(&reply.body).ok_or(RelayFailure::Malformed)?;
    Ok((minted.send_token, Date::from_unix(minted.iat as f64)))
}

/// Mint over `token` and store the pairing (`mintAndStore`); `Ok(None)`
/// when it was forgotten or replaced while the relay answered — then
/// nothing is stored.
async fn mint_and_store(
    ctx: &Ctx,
    pairing: &PushPairing,
    token: &str,
) -> Result<Option<PushPairing>, RelayFailure> {
    let (send_token, issued_at) =
        mint(ctx, token, pairing.connection_id, &pairing.relay_url).await?;
    ctx.with(|s| {
        let environment = s.environment().ok_or(RelayFailure::Unavailable)?;
        let stored = s.store.pairing(pairing.connection_id);
        let Some(updated) = service::store_minted(
            stored.as_ref(),
            pairing,
            &send_token,
            issued_at,
            token,
            &environment,
        ) else {
            if stored.is_none() {
                s.set_pairing_error(pairing.connection_id, None);
            }
            log::info!(target: "moshpit::push", "pairing for {} was forgotten or replaced while minting \u{2014} nothing stored", pairing.host_label);
            return Ok(None);
        };
        if !s.store.upsert(&updated) {
            return Err(RelayFailure::CouldNotSave);
        }
        s.set_pairing_error(pairing.connection_id, None);
        log::info!(target: "moshpit::push", "minted a send token for {} at {}", pairing.host_label, pairing.relay_url);
        Ok(Some(updated))
    })
}

/// The stored pairing for `connection`, minted first if its credential is
/// missing, stale or for another device token (`ensureReady`); `None` when
/// nothing is paired, or it can't be made ready now — a pairing that isn't
/// ready never reaches a host.
pub(crate) async fn ensure_ready(ctx: &Ctx, connection: Uuid) -> Option<PushPairing> {
    let (stored, needs) = ctx.with(|s| {
        let stored = s.store.pairing(connection)?;
        let needs = stored.needs_mint(s.push.device_token.as_deref(), s.clock.date);
        Some((stored, needs))
    })?;
    if !needs {
        return Some(stored);
    }
    ctx.with(Shared::register_for_push);
    let Some(token) = device_token(ctx).await else {
        log::info!(target: "moshpit::push", "credential for {} not ready \u{2014} no device token", stored.host_label);
        return None;
    };
    match mint_and_store(ctx, &stored, &token).await {
        Ok(ready) => ready,
        Err(failure) => {
            log::error!(target: "moshpit::push", "mint for {} failed: {}", stored.host_label, Note::Relay(failure.clone()).english());
            ctx.with(|s| s.set_pairing_error(connection, Some(Note::Relay(failure))));
            None
        }
    }
}

/// Mint a pairing for `connection` and return it ready to install
/// (`pair`): the secret from this phone, saved before the relay is asked —
/// a pairing the phone couldn't save must never reach a host — then the
/// send token. A re-pair mints a fresh secret on purpose.
pub(crate) async fn pair(
    ctx: &Ctx,
    connection: Uuid,
    host_label: &str,
    relay_url: &str,
) -> Result<PushPairing, RelayFailure> {
    let pairing = ctx.with(|s| {
        if s.environment().is_none() {
            return Err(RelayFailure::Unavailable);
        }
        let pairing = PushPairing::make(connection, host_label, relay_url, s.clock.date);
        if !s.store.upsert(&pairing) {
            return Err(RelayFailure::CouldNotSave);
        }
        s.register_for_push();
        Ok(pairing)
    })?;
    let Some(token) = device_token(ctx).await else {
        // Stored but credential-less: a mint round completes it the moment
        // a token exists.
        ctx.with(|s| s.push.sync_pending = true);
        return Err(RelayFailure::NoDeviceToken);
    };
    mint_and_store(ctx, &pairing, &token)
        .await?
        .ok_or(RelayFailure::CouldNotSave)
}

/// Forget `connection`'s pairing on this phone (`unpair`).
pub(crate) fn unpair(shared: &mut Shared, connection: Uuid) -> Result<(), RelayFailure> {
    if shared.store.remove(connection) {
        shared.push.pairing_errors.remove(&connection);
        shared.changed(Change::Push);
        Ok(())
    } else {
        Err(RelayFailure::CouldNotForget)
    }
}

/// Re-mint every pairing that needs it over the current token
/// (`mintNeededPairings`), one round at a time; a request made during a
/// round runs one more when it ends (B73).
pub(crate) async fn mint_needed(ctx: Ctx) {
    let start = ctx.with(|s| {
        if s.push.device_token.is_none() {
            log::info!(target: "moshpit::push", "mint deferred \u{2014} no device token yet");
            s.push.sync_pending = true;
            return false;
        }
        s.push.rounds.request()
    });
    if !start {
        return;
    }
    loop {
        let Some(token) = ctx.with(|s| s.push.device_token.clone()) else {
            ctx.with(|s| s.push.rounds.abandon());
            return;
        };
        mint_round(&ctx, &token).await;
        if !ctx.with(|s| s.push.rounds.round_ended()) {
            return;
        }
    }
}

async fn mint_round(ctx: &Ctx, token: &str) {
    let mut failed = false;
    let listed = ctx.with(|s| service::needing_mint(&s.store.read(), token, s.clock.date));
    for connection in listed {
        // Read again: a pairing forgotten since the list was made is not
        // asked about again.
        let Some(pairing) = ctx.with(|s| {
            s.store
                .pairing(connection)
                .filter(|p| p.needs_mint(Some(token), s.clock.date))
        }) else {
            continue;
        };
        if let Err(failure) = mint_and_store(ctx, &pairing, token).await {
            failed = true;
            log::error!(target: "moshpit::push", "relay {} refused to mint: {}", pairing.relay_url, Note::Relay(failure.clone()).english());
            ctx.with(|s| s.set_pairing_error(connection, Some(Note::Relay(failure))));
        }
    }
    ctx.with(|s| {
        s.push.sync_pending = failed;
        if !failed {
            s.push.last_success = Some(s.clock.date);
        }
    });
}

/// Wait for self-test `nonce`'s push until `deadline_ms`; `true` only if
/// this nonce arrived (`awaitSelfTest`).
pub(crate) async fn self_test(ctx: &Ctx, nonce: &str, deadline_ms: u64) -> bool {
    ctx.with(|s| s.push.waiting.insert(nonce.to_owned()));
    let wanted = nonce.to_owned();
    ctx.until(Some(deadline_ms), move |s| {
        (s.push.arrived.contains(&wanted) || s.push.abandoned.contains(&wanted)).then_some(())
    })
    .await;
    ctx.with(|s| {
        s.push.waiting.remove(nonce);
        s.push.abandoned.remove(nonce);
        s.push.arrived.contains(nonce)
    })
}

/// Forget a finished attempt (`forgetSelfTest`); a wait still parked on it
/// ends, with `false`.
pub(crate) fn forget_self_test(shared: &mut Shared, nonce: &str) {
    shared.push.arrived.remove(nonce);
    if shared.push.waiting.contains(nonce) {
        shared.push.abandoned.insert(nonce.to_owned());
    }
}
