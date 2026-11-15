//! Whose grid a window follows, and putting it back.
//!
//! - The pin: a window on screen is resized to this client's grid
//!   (`resize-window -x -y`), so a full-screen program redraws at the phone's
//!   width; handed back when the phone leaves it (`set-option -u -w
//!   window-size`, the real "back to automatic").
//! - The size lease (`@moshpit_size_owner`, see [`crate::lease`]): two Moshpit
//!   devices on one window; the last one touched holds it, the other follows
//!   under a veil until the person touches it.
//! - The layout pin: tmux's `resize_window()` reflows the WHOLE layout tree,
//!   so pinning a split window to a phone grid squashes the user's layout for
//!   good. Its `#{window_layout}` is recorded BEFORE the first pin and put
//!   back (`select-layout`, then the user's own zoom) once the pin is gone.
//!   Our own size request squashes too (B150): tmux sizes a session's
//!   windows by its latest client, and a phone that just reported its size
//!   is that client — the only one, or the latest of several — so every
//!   window of the session is at the phone's grid before discovery. The
//!   session's layouts are read just before the request and again just
//!   after; a window whose size the request changed keeps its earlier
//!   layout as its record. Handing back, the phone leaves the sizing math
//!   (`ignore-size`, which tmux 3.6a honours only while another client is
//!   attached) and reports the recorded size as its own, so with no other
//!   client the windows return to it and the replayed layouts fit exactly.
//! - The immersive zoom: Moshpit shows one pane full-screen, so a split
//!   window's active pane is zoomed — and zoomed again when a split, a kill or
//!   a pane exit unzooms it.
//! - The war guard: a desktop client and `window-size latest` resizing the
//!   window back and forth; three reclaims within 5 s back off for 5 s, and a
//!   war that ends at the other width is re-pinned after a quiet spell.
//! - A dead phone's pins (B456): a phone that died with a window pinned —
//!   the app killed, the network gone, no teardown to hand it back — left
//!   it at the phone's grid (`window-size manual`) for good, the desktop's
//!   with it. Each attach hands back the windows whose lease names a client
//!   no longer attached ([`Controller::sweep_dead_pins`]).
//! - A dead phone's zoom (B458): the immersive zoom stays when the phone
//!   dies too, so the phone marks the zoom it makes (`@moshpit_zoomed` =
//!   `<client> <pane>`), and the same sweep unzooms a window still zoomed on
//!   that pane once its client is gone. A zoom with no mark is somebody
//!   else's — a desktop's own — and never touched.

use std::collections::{BTreeMap, BTreeSet};

use moshpit_herdr::text::trim_whitespaces;

use crate::control::CommandResponse;
use crate::format::{
    is_plausible_layout, is_valid_id, pane_count, swift_int, window_size, window_width, words,
};
use crate::lease::{LeaseState, SizeFollow, SizeLease};
use crate::model::id_cmp;

use super::timers::{Action, Slot};
use super::{Controller, Reply};

/// Why a pin is wanted: how far the lease may be overridden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinReason {
    /// First pin after discovery.
    Attach,
    /// Back from the background.
    Foreground,
    /// A layout change at another width (the war guard applies).
    Drift,
    /// The quiet-spell timer, or a probe's follow-up.
    Recheck,
    /// Our grid changed, or a capture came back oversized.
    Resize,
    /// The person did something to this window: always ours.
    User,
}

/// What a lease probe's reply goes on to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LeaseThen {
    /// Decide the pin with what was learned.
    Decide(PinReason),
    /// A drift or resize already pinned on an assumption: follow if a live
    /// device holds it after all; claim an unowned window we are using.
    Verify(PinReason),
}

/// A window's layout as it was before we touched it (`PristineLayout`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PristineLayout {
    /// `#{window_layout}`.
    pub layout: String,
    /// The pane the USER had zoomed; `None` when the window wasn't zoomed.
    pub zoomed_pane: Option<String>,
    /// How many panes the layout describes.
    pub pane_count: i64,
    /// The layouts our sizing left the window in (round thirty-four): the
    /// one tmux reflowed the record into at the phone's grid, and those the
    /// phone's later sizes reflowed that into. A window whose layout is none
    /// of these has been rearranged by someone else since — a desktop — and
    /// the hand-back leaves it alone. Empty: unknown, restored as before.
    pub ours: Vec<String>,
}

/// How many of a window's own layouts are kept, the latest last.
const OURS_KEPT: usize = 8;

/// What a controller still owes the server for the windows it shaped, handed
/// from a retired controller to its replacement on a reconnect (`PinLedger`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PinLedger {
    pub resized_windows: BTreeSet<String>,
    pub pristine_layouts: BTreeMap<String, PristineLayout>,
    /// The mosh sidecar's renderer, which a rebuilt sidecar never sees attach.
    pub renderer: Option<crate::clients::ClientIdentity>,
    /// The session whose status line the sidecar turned off.
    pub hidden_status: Option<String>,
    /// The sessions Moshpit created.
    pub owned_sessions: BTreeSet<String>,
}

impl PinLedger {
    /// Bare pins, as a test hands a controller standing in for a retired one.
    pub fn with_pins(resized_windows: BTreeSet<String>) -> Self {
        Self {
            resized_windows,
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.resized_windows.is_empty()
            && self.pristine_layouts.is_empty()
            && self.renderer.is_none()
            && self.hidden_status.is_none()
            && self.owned_sessions.is_empty()
    }
}

#[derive(Debug, Default)]
pub(super) struct SizeState {
    /// Between `release_window_pins` and the next repin: backgrounded.
    pub pins_released: bool,
    /// The layout each window's last `%layout-change` reported (B771,
    /// B775): the same one again is another session's copy of it. A window
    /// listing's layout is not one: it can show a resize whose notification
    /// is still on its way.
    pub reported_layouts: BTreeMap<String, String>,
    /// Windows pinned to our grid (`resizedWindows`).
    pub resized_windows: BTreeSet<String>,
    pub pristine_layouts: BTreeMap<String, PristineLayout>,
    pub pristine_captures_in_flight: BTreeSet<String>,
    /// The session's layouts just before our size request, by window
    /// (the probe's fields after the window id); `None` until that reply.
    pub presize: Option<BTreeMap<String, Vec<String>>>,
    /// A before-probe went out and its after-probe has not.
    pub presize_awaits_after: bool,
    /// The mosh sidecar: every window's layout as the host listed it before
    /// mosh-server started the phone's renderer (B270), by window; compared
    /// with a listing at the sidecar's first attach.
    pub before_renderer: Option<BTreeMap<String, Vec<String>>>,
    /// Bumped by every zoom change we send; a zoom probe carries the value
    /// it was sent at.
    pub zoom_epoch: u64,
    pub leases: BTreeMap<String, LeaseState>,
    /// The current window's lease, read at attach before the window list.
    pub pending_attach_lease: Option<LeaseState>,
    pub probes_in_flight: BTreeSet<String>,
    /// The client names of the `list-clients` sent right before a lease read.
    pub listed_clients: Option<BTreeSet<String>>,
    /// Our `#{client_name}`.
    pub our_client_name: Option<String>,
    /// The dead-pin sweep's window listing is on its way (B456).
    pub sweep_pending: bool,
    /// Windows whose immersive zoom this controller made and marked (B458).
    pub zoom_marks: BTreeSet<String>,
    /// The clients the attach's lease read listed, kept for that sweep;
    /// `None` until listed.
    pub sweep_clients: Option<BTreeSet<String>>,
    pub follow: Option<SizeFollow>,
    /// When we last wrote a stamped claim; `None` = never (distant past).
    pub last_stamp_write: Option<super::Wall>,
    pub active_window_at_foreign_width: bool,
    pub reclaim_tracked_window: Option<String>,
    pub recent_reclaims: Vec<super::Wall>,
    pub reclaim_suspended_until: Option<super::Wall>,
}

const RECLAIM_THRESHOLD: usize = 3;
/// Seconds.
const RECLAIM_WINDOW: f64 = 5.0;

impl Controller {
    // MARK: Reading the size state

    /// Windows pinned to this client's grid.
    pub fn resized_windows(&self) -> &BTreeSet<String> {
        &self.size.resized_windows
    }

    /// The follow veil: the active window is sized for another device.
    pub fn size_follow(&self) -> Option<&SizeFollow> {
        self.size.follow.as_ref()
    }

    /// Our tmux `#{client_name}`, read at attach.
    pub fn our_client_name(&self) -> Option<&str> {
        self.size.our_client_name.as_deref()
    }

    /// Set our client name (a host or test whose attach skipped the read).
    pub fn set_our_client_name(&mut self, name: Option<String>) {
        self.size.our_client_name = name;
    }

    // MARK: The gate every pin goes through

    /// `User` claims and pins. `Attach`/`Foreground` take the window unless
    /// another live device used it within `foreground_steal_after` (probing
    /// first when the lease is unknown). `Drift`/`Resize`/`Recheck` take it
    /// only when unowned, ours, or its holder is gone; an unprobed window
    /// pins at once on drift and learns afterwards, a re-check always probes.
    pub(super) fn request_pin(&mut self, window: &str, reason: PinReason) {
        if !self.config.renders_output {
            return;
        }
        if reason == PinReason::User {
            let stamp = self.unix_seconds();
            self.claim_size_ownership(window, stamp);
            self.pin_now(window, reason);
            return;
        }
        if reason == PinReason::Attach
            && let Some(pending) = self.size.pending_attach_lease.take()
            && !self.size.leases.contains_key(window)
        {
            self.size.leases.insert(window.to_owned(), pending);
        }
        let known = self.size.leases.get(window).cloned();
        match reason {
            PinReason::Recheck => self.probe_size_lease(window, LeaseThen::Decide(reason)),
            PinReason::Attach | PinReason::Foreground => {
                if known.is_none() {
                    self.probe_size_lease(window, LeaseThen::Decide(reason));
                } else {
                    self.decide_pin(window, reason);
                }
            }
            PinReason::Drift | PinReason::Resize => {
                self.decide_pin(window, reason);
                if !self.is_held_by_another_live_device(known.as_ref()) {
                    self.probe_size_lease(window, LeaseThen::Verify(reason));
                }
            }
            PinReason::User => {}
        }
    }

    fn is_held_by_another_live_device(&self, state: Option<&LeaseState>) -> bool {
        match state {
            Some(LeaseState::Held(lease)) => {
                lease.live
                    && Some(lease.client_name.as_str()) != self.size.our_client_name.as_deref()
            }
            _ => false,
        }
    }

    fn decide_pin(&mut self, window: &str, reason: PinReason) {
        if !self.config.renders_output || !self.snapshot.is_attached {
            return;
        }
        let known = self.size.leases.get(window).cloned();
        if reason == PinReason::Foreground {
            self.end_foreground_probe();
        }
        let ours = self.size.our_client_name.clone();
        match &known {
            Some(LeaseState::Held(lease))
                if lease.live && Some(lease.client_name.as_str()) != ours.as_deref() =>
            {
                let idle = self.unix_seconds() - lease.stamp;
                let may_take = matches!(reason, PinReason::Attach | PinReason::Foreground)
                    && idle > self.config.foreground_steal_after;
                if !may_take {
                    let lease = lease.clone();
                    self.follow(window, &lease);
                    return;
                }
                self.claim_size_ownership(window, 0.0);
            }
            Some(LeaseState::Held(lease))
                if Some(lease.client_name.as_str()) == ours.as_deref() =>
            {
                // Ours already.
            }
            _ if reason != PinReason::Drift || known.is_some() => {
                // Unset, or a holder no longer attached. A drift on a window
                // never probed pins without writing: the probe that follows
                // may say someone holds it.
                self.claim_size_ownership(window, 0.0);
            }
            _ => {}
        }
        if reason == PinReason::Recheck
            && !self.size.active_window_at_foreign_width
            && self.size.follow.is_none()
        {
            // At our width and not following: a re-check has nothing to fix.
            return;
        }
        self.pin_now(window, reason);
    }

    fn pin_now(&mut self, window: &str, reason: PinReason) {
        if self
            .size
            .follow
            .as_ref()
            .is_some_and(|f| f.window_id == window)
        {
            self.set_follow(None);
        }
        match reason {
            PinReason::Drift => self.reclaim_window_on_drift(window),
            PinReason::Foreground => {
                self.fit_window_to_client(window);
                // The release handed the split layout back (unzoomed).
                self.ensure_immersive_zoom();
            }
            _ => self.fit_window_to_client(window),
        }
    }

    /// The window belongs to another device: leave its size alone. The veil
    /// (and the output gate behind it) only while it is at the other size.
    fn follow(&mut self, window: &str, lease: &SizeLease) {
        if self.snapshot.active_window_id.as_deref() != Some(window)
            || !self.size.active_window_at_foreign_width
        {
            return;
        }
        let next = SizeFollow {
            window_id: window.to_owned(),
            owner_label: lease.label.clone(),
        };
        if self.size.follow.as_ref() != Some(&next) {
            self.set_follow(Some(next));
        }
    }

    fn set_follow(&mut self, follow: Option<SizeFollow>) {
        if self.size.follow != follow {
            self.size.follow = follow;
            self.dirty = true;
        }
    }

    /// Read a window's lease and who is attached, as one FIFO pair (the
    /// clients' reply lands first, so the lease's liveness can be judged).
    /// One probe per window at a time.
    fn probe_size_lease(&mut self, window: &str, then: LeaseThen) {
        if !self.snapshot.is_attached || self.size.probes_in_flight.contains(window) {
            return;
        }
        self.size.probes_in_flight.insert(window.to_owned());
        self.send_command(
            "list-clients -F '#{client_name}'".to_owned(),
            Reply::LeaseClients,
        );
        self.send_command(
            format!("show-options -wqv -t {window} @moshpit_size_owner"),
            Reply::LeaseOwner {
                window: window.to_owned(),
                then,
            },
        );
    }

    pub(super) fn lease_clients_answered(&mut self, response: &CommandResponse) {
        // Fix (lease-clients-refused): a refused listing is a failed one —
        // no clients, so the holder reads live. Swift took tmux's complaint
        // for a client name, and the holder read dead.
        let clients = if response.is_error {
            BTreeSet::new()
        } else {
            response
                .lines
                .iter()
                .map(|line| trim_whitespaces(line))
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect()
        };
        if self.size.sweep_pending && self.size.sweep_clients.is_none() {
            // A refused listing reads as no clients: the sweep judges none.
            self.size.sweep_clients = Some(clients.clone());
        }
        self.size.listed_clients = Some(clients);
    }

    pub(super) fn lease_owner_answered(
        &mut self,
        window: &str,
        then: LeaseThen,
        response: &CommandResponse,
    ) {
        self.size.probes_in_flight.remove(window);
        let clients = self.size.listed_clients.take().unwrap_or_default();
        let state = lease_state(response, &clients);
        self.size.leases.insert(window.to_owned(), state.clone());
        match then {
            LeaseThen::Decide(reason) => self.decide_pin(window, reason),
            LeaseThen::Verify(reason) => {
                if self.is_held_by_another_live_device(Some(&state)) {
                    self.decide_pin(window, reason);
                } else if state == LeaseState::Unset
                    && self.snapshot.active_window_id.as_deref() == Some(window)
                {
                    self.claim_size_ownership(window, 0.0);
                }
            }
        }
    }

    /// At attach, ahead of discovery: who we are, and whose the current
    /// window is, so the first pin can defer to a device using it.
    pub(super) fn read_attach_lease(&mut self) {
        self.send_command(
            "display-message -p '#{client_name}'".to_owned(),
            Reply::ClientName,
        );
        self.send_command(
            "list-clients -F '#{client_name}'".to_owned(),
            Reply::LeaseClients,
        );
        self.send_command(
            "show-options -wqv @moshpit_size_owner".to_owned(),
            Reply::AttachLeaseOwner,
        );
    }

    /// Hand back what a dead phone left pinned (B456): every window's lease,
    /// read after the attach's lease read listed who is attached
    /// ([`Self::read_attach_lease`], whose listing it keeps). A window whose
    /// lease names a client no longer attached is unpinned and its lease
    /// unset; one a live device holds, one nobody leased (a pin can't be
    /// told from the user's own `window-size` without a lease) and the
    /// windows a retired controller handed over are left alone.
    ///
    /// The reply comes before discovery has named the active window, and
    /// before this client has written a lease of its own: a dead lease on
    /// the window this attach is about to pin is handed back like any other,
    /// and the attach's pin then claims and pins it again (both in tmux's
    /// order, the hand-back first).
    pub(super) fn sweep_dead_pins(&mut self) {
        self.size.sweep_pending = true;
        self.size.sweep_clients = None;
        self.send_command(format!("list-windows -a -F '{SWEEP_FORMAT}'"), Reply::SweepWindows);
    }

    pub(super) fn sweep_windows_answered(&mut self, response: &CommandResponse) {
        self.size.sweep_pending = false;
        // Without a listing of the clients (or with an empty one: we are a
        // client ourselves) nothing can be judged dead.
        let Some(clients) = self.size.sweep_clients.take().filter(|c| !c.is_empty()) else {
            return;
        };
        if response.is_error {
            return;
        }
        let ours = self.size.our_client_name.clone();
        let active = self.snapshot.active_window_id.clone();
        let gone = |name: &str| !clients.contains(name) && Some(name) != ours.as_deref();
        // `-a` names a window once per session holding it (a phone's group,
        // B459; a linked window): each is swept once — a zoom toggled twice
        // is a zoom left on.
        let mut swept = std::collections::BTreeSet::new();
        for line in &response.lines {
            let Some(row) = SweepRow::parse(line) else {
                continue;
            };
            if !swept.insert(row.window.clone()) {
                continue;
            }
            let window = row.window.as_str();
            // B458: a zoom the phone made, its client gone. Unzoomed only
            // while the window is still zoomed on the pane the phone zoomed:
            // a desktop that zoomed another pane since keeps its zoom. One
            // that zoomed that same pane again after the phone died loses it
            // (rare, and one keystroke to redo).
            if let Some((owner, pane)) = &row.zoom
                && gone(owner)
                && !self.size.zoom_marks.contains(window)
            {
                if row.zoomed && row.active_pane == *pane {
                    self.send(zoom_command(window));
                    self.bump_zoom_epoch();
                }
                self.send(format!("set-option -wu -t {window} {ZOOM_MARK_OPTION}"));
            }
            // B456: a pin the phone left, its client gone.
            if active.as_deref() == Some(window)
                || self.size.resized_windows.contains(window)
                || self.size.pristine_layouts.contains_key(window)
            {
                continue;
            }
            let Some(lease) = SizeLease::parse(&row.lease, &clients) else {
                continue;
            };
            if lease.live || Some(lease.client_name.as_str()) == ours.as_deref() {
                continue;
            }
            self.send(format!("set-option -u -w -t {window} window-size"));
            self.send(format!("set-option -wu -t {window} @moshpit_size_owner"));
            self.size.leases.remove(window);
        }
    }

    pub(super) fn client_name_answered(&mut self, response: &CommandResponse) {
        let name = response
            .lines
            .first()
            .map(|line| trim_whitespaces(line))
            .unwrap_or("");
        // Fix (lease-clients-refused): a refusal's complaint is not our name.
        if !name.is_empty() && !response.is_error {
            self.size.our_client_name = Some(name.to_owned());
        }
    }

    pub(super) fn attach_lease_answered(&mut self, response: &CommandResponse) {
        let clients = self.size.listed_clients.take().unwrap_or_default();
        self.size.pending_attach_lease = Some(lease_state(response, &clients));
    }

    /// Write our lease. Stamp 0 is a passive claim; an interaction stamps now.
    fn claim_size_ownership(&mut self, window: &str, stamp: f64) {
        if !self.config.renders_output {
            return;
        }
        let Some(name) = self.size.our_client_name.clone() else {
            return;
        };
        let lease = SizeLease {
            client_name: name,
            label: self.config.size_owner_label.clone(),
            stamp,
            live: true,
        };
        // B139 (the Swift controller the same): as a command-string literal.
        // The client name is what a reply said, and a `"`, `$` or `;` in it
        // ran on into the line.
        self.send(format!(
            "set-option -w -t {window} @moshpit_size_owner {}",
            crate::format::command_string_literal(&lease.encoded())
        ));
        self.size
            .leases
            .insert(window.to_owned(), LeaseState::Held(lease));
        self.size.last_stamp_write = (stamp != 0.0).then_some(self.wall());
        if self
            .size
            .follow
            .as_ref()
            .is_some_and(|f| f.window_id == window)
        {
            self.set_follow(None);
        }
    }

    /// Unset our lease on a window we hand back, if it is ours.
    fn release_size_ownership(&mut self, window: &str) {
        if let Some(LeaseState::Held(lease)) = self.size.leases.get(window)
            && Some(lease.client_name.as_str()) == self.size.our_client_name.as_deref()
        {
            self.send(format!("set-option -wu -t {window} @moshpit_size_owner"));
            self.size
                .leases
                .insert(window.to_owned(), LeaseState::Unset);
        }
        if self
            .size
            .follow
            .as_ref()
            .is_some_and(|f| f.window_id == window)
        {
            self.set_follow(None);
        }
    }

    /// The lease unsets a full teardown owes, for a channel outside `-CC`
    /// (an in-band write races the detach). Consumes the records.
    pub fn size_lease_release_commands(&mut self) -> Vec<String> {
        let ours = self.size.our_client_name.clone();
        let commands = self
            .size
            .leases
            .iter()
            .filter(|(_, state)| {
                matches!(state, LeaseState::Held(lease) if Some(lease.client_name.as_str()) == ours.as_deref())
            })
            .map(|(window, _)| format!("set-option -wu -t {window} @moshpit_size_owner"))
            .collect();
        self.size.leases.clear();
        commands
    }

    /// A keystroke, a paste, a wheel tick on the active window: following
    /// ends here; otherwise the stamp says "in use" (at most every
    /// `lease_stamp_refresh_interval` seconds).
    pub(super) fn note_user_interaction(&mut self) {
        if !self.config.renders_output || !self.snapshot.is_attached {
            return;
        }
        // The pane another client moved off (B587): the touch takes it back.
        self.retake_pane();
        let Some(window) = self.snapshot.active_window_id.clone() else {
            return;
        };
        let following = self
            .size
            .follow
            .as_ref()
            .is_some_and(|f| f.window_id == window);
        let held = self.size.leases.get(&window).cloned();
        if following || self.is_held_by_another_live_device(held.as_ref()) {
            self.request_pin(&window, PinReason::User);
            return;
        }
        let stale = match self.size.last_stamp_write {
            None => true,
            Some(at) => self.wall().since(at) > self.config.lease_stamp_refresh_interval,
        };
        if stale {
            let stamp = self.unix_seconds();
            self.claim_size_ownership(&window, stamp);
        }
    }

    /// The follower's tap: take the active window back — and the pane
    /// another client moved off (B587).
    pub fn take_size_ownership(&mut self, now: u64) {
        self.begin_input(now);
        self.retake_pane();
        if let Some(window) = self.snapshot.active_window_id.clone() {
            self.request_pin(&window, PinReason::User);
        }
        self.finish_input();
    }

    fn end_foreground_probe(&mut self) {
        self.timers.cancel(&Slot::ForegroundProbe);
        self.size.pins_released = false;
    }

    // MARK: The pin and the layout under it

    /// Resize a window to our grid, recording its layout first.
    fn fit_window_to_client(&mut self, window: &str) {
        if !self.config.renders_output {
            return;
        }
        self.capture_pristine_layout(window);
        let (cols, rows) = self.last_client_size;
        self.send(format!("resize-window -t {window} -x {cols} -y {rows}"));
        self.size.resized_windows.insert(window.to_owned());
    }

    /// Ask for a window's layout BEFORE the command that changes it: tmux
    /// runs the control channel in order, so the reply describes the window
    /// as it was. Never behind a pin (a squashed layout must never be
    /// replayed at a desktop).
    pub(super) fn capture_pristine_layout(&mut self, window: &str) {
        if !self.snapshot.is_attached
            || self.size.pristine_layouts.contains_key(window)
            || self.size.pristine_captures_in_flight.contains(window)
            || self.size.resized_windows.contains(window)
        {
            return;
        }
        self.size
            .pristine_captures_in_flight
            .insert(window.to_owned());
        self.send_command(
            format!("display-message -p -t '{window}' '{PRISTINE_PROBE_FORMAT}'"),
            Reply::PristineProbe {
                window: window.to_owned(),
            },
        );
    }

    pub(super) fn pristine_probe_answered(&mut self, window: &str, response: &CommandResponse) {
        self.size.pristine_captures_in_flight.remove(window);
        if response.is_error {
            return;
        }
        if let Some(line) = response.lines.first() {
            let fields: Vec<String> = words(line, b' ').into_iter().map(str::to_owned).collect();
            self.note_pristine_layout(window, &fields);
        }
    }

    /// File a probe reply as a window's record, if it qualifies: more than
    /// one pane, and a layout that is plainly a layout.
    ///
    /// B772: a window pinned already (`window-size manual`, another Moshpit
    /// device's pin) is found squashed to that device's grid: the record is
    /// what that device found before its pin, shared on the window
    /// ([`SHARED_LAYOUT_OPTION`]), and what is found now is one of
    /// Moshpit's own layouts. A record of the window as found, unpinned, is
    /// shared for the next device. The second phone took the first one's
    /// squashed layout for the window's: both gone, the window stayed in
    /// it, at the first phone's grid.
    fn note_pristine_layout(&mut self, window: &str, fields: &[String]) {
        if self.size.pristine_layouts.contains_key(window) || fields.len() < 4 {
            return;
        }
        let Some(panes) = swift_int(&fields[0]).filter(|&panes| panes > 1) else {
            return;
        };
        if !is_plausible_layout(&fields[3]) {
            return;
        }
        let pinned = fields.get(4).is_some_and(|size| size == "manual");
        if pinned
            && let Some(shared) = fields
                .get(5)
                .and_then(|raw| SharedLayout::parse(raw))
                .filter(|shared| shared.pane_count == panes)
        {
            self.size.pristine_layouts.insert(
                window.to_owned(),
                PristineLayout {
                    layout: shared.layout,
                    zoomed_pane: shared.zoomed_pane,
                    pane_count: panes,
                    ours: vec![fields[3].clone()],
                },
            );
            return;
        }
        let zoomed_pane = (fields[1] == "1" && is_valid_id(&fields[2])).then(|| fields[2].clone());
        if !pinned && fields.len() >= 5 && is_valid_id(window) {
            let shared = SharedLayout {
                pane_count: panes,
                zoomed_pane: zoomed_pane.clone(),
                layout: fields[3].clone(),
            };
            self.send(format!(
                "set-option -w -t {window} {SHARED_LAYOUT_OPTION} '{}'",
                shared.encoded()
            ));
        }
        self.size.pristine_layouts.insert(
            window.to_owned(),
            PristineLayout {
                layout: fields[3].clone(),
                zoomed_pane,
                pane_count: panes,
                ours: Vec::new(),
            },
        );
    }

    /// `layout` is one our sizing left `window` in (round thirty-four).
    fn note_ours(&mut self, window: &str, layout: &str) {
        let Some(record) = self.size.pristine_layouts.get_mut(window) else {
            return;
        };
        if layout == record.layout || record.ours.iter().any(|ours| ours == layout) {
            return;
        }
        if !is_plausible_layout(layout) {
            return;
        }
        record.ours.push(layout.to_owned());
        if record.ours.len() > OURS_KEPT {
            record.ours.remove(0);
        }
    }

    /// Whether a record still describes the window: panes coming or going
    /// since make it describe one that no longer exists.
    fn pristine_still_applies(&self, window: &str, pristine: &PristineLayout) -> bool {
        self.snapshot
            .windows
            .get(window)
            .is_none_or(|info| info.pane_count == pristine.pane_count)
    }

    /// Put a window's layout (and the user's own zoom) back, consuming the
    /// record. The pin must be released first: replayed under it, tmux fits
    /// the layout straight back to the phone grid.
    ///
    /// The mosh sidecar keeps the record (B270): the phone's renderer is
    /// still attached and still sizes the session. A replay at another size
    /// resizes the window to the layout's for a moment — so the check after
    /// it read the record back and dropped it — and tmux's next sizing put
    /// the window back at the phone's grid, squashed, with nothing left to
    /// restore on the disconnect.
    fn restore_pristine_layout(&mut self, window: &str) -> Option<PristineLayout> {
        self.replay_pristine_layout(window, !self.config.renders_output)
    }

    /// [`Self::restore_pristine_layout`], with whether the record is `kept`
    /// spelled out: not once the renderer has left the window's session.
    fn replay_pristine_layout(&mut self, window: &str, kept: bool) -> Option<PristineLayout> {
        let pristine = self.size.pristine_layouts.remove(window)?;
        if !self.pristine_still_applies(window, &pristine) {
            return None;
        }
        self.send(format!("select-layout -t '{window}' '{}'", pristine.layout));
        if let Some(pane) = &pristine.zoomed_pane {
            self.send(format!("resize-pane -Z -t '{pane}'"));
        }
        self.bump_zoom_epoch();
        if kept {
            self.size
                .pristine_layouts
                .insert(window.to_owned(), pristine);
            return None;
        }
        // B772: put back, the shared record is nobody's any more.
        self.send(format!("set-option -wu -t {window} {SHARED_LAYOUT_OPTION}"));
        Some(pristine)
    }

    /// The mosh sidecar's renderer has left `session` for another one: no
    /// window of it is the phone's any more. The window it pinned (`window`)
    /// goes back to automatic sizing, and every window of the session with a
    /// record gets its layout back — the record consumed this time, the
    /// check after it, as a `-CC` client's leave does: nothing of the phone
    /// sizes that session now (with the phone alone tmux's only client, its
    /// windows are nobody's, and a replay resizes them to their record for
    /// good).
    pub(super) fn hand_back_left_session(&mut self, session: &str, window: Option<&str>) {
        if let Some(window) = window
            && self.size.resized_windows.remove(window)
        {
            self.send(format!("set-option -u -w -t {window} window-size"));
        }
        let mut recorded: Vec<String> = self
            .size
            .pristine_layouts
            .keys()
            .filter(|window| {
                self.snapshot
                    .windows
                    .get(*window)
                    .is_some_and(|info| info.session_id == session)
            })
            .cloned()
            .collect();
        recorded.sort_by(|a, b| id_cmp(a, b));
        for window in &recorded {
            self.check_replay(window, false);
        }
    }

    /// Replay a window's record, then read the window back: a replay that
    /// didn't take keeps the record.
    fn check_replay(&mut self, window: &str, kept: bool) {
        if let Some(pristine) = self.replay_pristine_layout(window, kept) {
            self.send_command(
                format!("display-message -p -t '{window}' '#{{window_layout}}'"),
                Reply::RestoreCheck {
                    window: window.to_owned(),
                    pristine,
                },
            );
        }
    }

    /// The size to report as ours when handing the windows back (B150): the
    /// recorded size of the active window, else the largest recorded one;
    /// `None` without a record that still applies.
    fn hand_back_size(&self, layouts: &BTreeMap<String, PristineLayout>) -> Option<(i64, i64)> {
        let applying = |window: &String, pristine: &PristineLayout| {
            self.pristine_still_applies(window, pristine)
                .then(|| window_size(&pristine.layout))
                .flatten()
        };
        if let Some(window) = self.snapshot.active_window_id.as_ref()
            && let Some(size) = layouts
                .get(window)
                .and_then(|pristine| applying(window, pristine))
        {
            return Some(size);
        }
        let mut windows: Vec<&String> = layouts.keys().collect();
        windows.sort_by(|a, b| id_cmp(a, b));
        let mut best: Option<(i64, i64)> = None;
        for window in windows {
            if let Some((cols, rows)) = applying(window, &layouts[window])
                && best.is_none_or(|(c, r)| cols * rows > c * r)
            {
                best = Some((cols, rows));
            }
        }
        best
    }

    /// The same restore as commands for a teardown channel outside `-CC`
    /// (`@id`/`%id` targets survive a login shell; the layout is
    /// single-quoted), run after the windows are unpinned. Consumes the
    /// records.
    ///
    /// B150: with our client still attached they first take it out of the
    /// sizing math and report the recorded size as its own (see
    /// [`Self::release_window_pins`]); a ledger's records, whose client is
    /// gone, size each window to its record first and unset that again —
    /// with no client attached tmux keeps a window's size, and a layout
    /// replayed at another size is refitted.
    pub fn pristine_layout_restore_commands(&mut self) -> Vec<String> {
        let layouts = std::mem::take(&mut self.size.pristine_layouts);
        let mut commands = Vec::new();
        let client = self
            .size
            .our_client_name
            .clone()
            .filter(|name| self.config.renders_output && is_plain_client_name(name));
        if let Some(name) = &client
            && let Some((cols, rows)) = self.hand_back_size(&layouts)
        {
            commands.push(format!("refresh-client -t '{name}' -f ignore-size"));
            commands.push(format!("refresh-client -t '{name}' -C {cols}x{rows}"));
        }
        for (window, pristine) in &layouts {
            if !self.pristine_still_applies(window, pristine) {
                continue;
            }
            commands.extend(window_restore_commands(window, pristine, client.is_none()));
        }
        commands
    }

    /// The whole hand-back a full teardown owes, as ONE command for a
    /// channel outside `-CC` (round thirty-four): `first` (the phone's mosh
    /// renderer out, B270), then the pins unset, then the layouts — each
    /// only if the window is still in a layout of ours. Consumes the pins
    /// and the records; `None` when nothing is owed.
    ///
    /// With a desktop attached, the disconnect used to replay a record over
    /// whatever the window had become: a desktop that rearranged the panes
    /// since (the same count) had its arrangement replaced by the one from
    /// before the phone came. Every window's layout is read first, before
    /// anything here changes a size (unpinned, a window at once refits to
    /// the desktop and reads as nothing of ours); a record is replayed only
    /// when that layout is its own or one our sizing left the window in
    /// ([`PristineLayout::ours`]), and a record that knows none of those is
    /// replayed as before. Under `sh -c`, for any login shell.
    pub fn pin_hand_back_script(&mut self, tmux: &str, first: &[String]) -> Option<String> {
        let mut pinned: Vec<String> = std::mem::take(&mut self.size.resized_windows)
            .into_iter()
            .filter(|window| is_valid_id(window))
            .collect();
        pinned.sort_by(|a, b| id_cmp(a, b));
        let layouts = std::mem::take(&mut self.size.pristine_layouts);
        let client = self
            .size
            .our_client_name
            .clone()
            .filter(|name| self.config.renders_output && is_plain_client_name(name));
        let mut reads = Vec::new();
        let mut steps: Vec<String> = first.to_vec();
        steps.extend(
            pinned
                .iter()
                .map(|window| format!("{tmux} set-option -u -w -t {window} window-size")),
        );
        if let Some(name) = &client
            && let Some((cols, rows)) = self.hand_back_size(&layouts)
        {
            steps.push(format!("{tmux} refresh-client -t '{name}' -f ignore-size"));
            steps.push(format!("{tmux} refresh-client -t '{name}' -C {cols}x{rows}"));
        }
        let mut restores = 0;
        for (index, (window, pristine)) in layouts.iter().enumerate() {
            if !is_valid_id(window) || !self.pristine_still_applies(window, pristine) {
                continue;
            }
            restores += 1;
            let commands: Vec<String> = window_restore_commands(window, pristine, client.is_none())
                .into_iter()
                .map(|command| format!("{tmux} {command}"))
                .collect();
            if pristine.ours.is_empty() {
                steps.extend(commands);
                continue;
            }
            let read = format!("l{index}");
            reads.push(format!(
                "{read}=$({tmux} display-message -p -t {window} '#{{window_layout}}' 2>/dev/null)"
            ));
            let patterns: Vec<String> = std::iter::once(&pristine.layout)
                .chain(&pristine.ours)
                .map(|layout| format!("\"{layout}\""))
                .collect();
            steps.push(format!(
                "case \"${read}\" in {}) {};; esac",
                patterns.join("|"),
                commands.join("; ")
            ));
        }
        if pinned.is_empty() && restores == 0 && first.is_empty() {
            return None;
        }
        let mut script = reads;
        script.extend(steps);
        script.push("true".to_owned());
        Some(format!(
            "sh -c {}",
            moshpit_herdr::launch::quote(&script.join("; "))
        ))
    }

    /// Hand the window we're leaving back: unpin, unset our lease, restore
    /// its layout — in that order. While we still size the session (the
    /// only client, or its latest) the replay is refitted to our grid: the
    /// record is kept for the hand-back that can restore it (B150).
    pub(super) fn release_window_pin(&mut self, window: &str) {
        if self.size.resized_windows.remove(window) {
            self.send(format!("set-option -u -w -t {window} window-size"));
        }
        self.release_size_ownership(window);
        self.hand_back_zoom(window);
        self.check_replay(window, !self.config.renders_output);
    }

    /// Our immersive zoom on a window we hand back. The replay of the
    /// layout we found puts the user's layout back and our zoom goes with
    /// it; with no replay to come — the panes changed since (a split on the
    /// desktop, a pane an agent opened), or the window had one pane when we
    /// came (our own split) — the zoom is undone here (B588: it stayed, on
    /// the desktop's window, marked with a client still attached, so no
    /// sweep took it either). Only while the window is still zoomed on the
    /// pane we zoomed: a zoom made since is somebody else's.
    fn hand_back_zoom(&mut self, window: &str) {
        if !self.size.zoom_marks.remove(window) {
            return;
        }
        let replays = self
            .size
            .pristine_layouts
            .get(window)
            .is_some_and(|pristine| self.pristine_still_applies(window, pristine));
        if replays {
            self.send(format!("set-option -wu -t {window} {ZOOM_MARK_OPTION}"));
            return;
        }
        self.send_command(
            format!(
                "display-message -p -t {window} '#{{window_zoomed_flag}} #{{pane_id}} #{{{ZOOM_MARK_OPTION}}}'"
            ),
            Reply::ZoomHandBack {
                window: window.to_owned(),
            },
        );
    }

    /// `zoomed active-pane mark`, read for [`Self::hand_back_zoom`].
    pub(super) fn zoom_hand_back_answered(&mut self, window: &str, response: &CommandResponse) {
        if response.is_error {
            return;
        }
        let Some(line) = response.lines.first() else {
            return;
        };
        let mut words = line.splitn(3, ' ');
        let (Some(zoomed), Some(active), mark) = (words.next(), words.next(), words.next()) else {
            return;
        };
        let mark = mark.map(trim_whitespaces).unwrap_or("");
        let Some(name) = self.size.our_client_name.clone() else {
            return;
        };
        let Some(pane) = mark.strip_prefix(&format!("{name} ")) else {
            // Somebody else's mark, or none: theirs to keep.
            return;
        };
        if zoomed == "1" && pane == active {
            self.send(zoom_command(window));
            self.bump_zoom_epoch();
        }
        self.send(format!("set-option -wu -t {window} {ZOOM_MARK_OPTION}"));
    }

    /// The window's layout after a replay: kept as its record again when the
    /// replay didn't take (and nothing newer was recorded meanwhile).
    pub(super) fn restore_check_answered(
        &mut self,
        window: &str,
        pristine: PristineLayout,
        response: &CommandResponse,
    ) {
        if response.is_error || self.size.pristine_layouts.contains_key(window) {
            return;
        }
        let Some(line) = response.lines.first() else {
            return;
        };
        let line = trim_whitespaces(line);
        if line != pristine.layout {
            self.size
                .pristine_layouts
                .insert(window.to_owned(), pristine);
            // Refitted to the phone's grid: a layout of ours.
            self.note_ours(window, line);
        }
    }

    // MARK: The layouts our size request squashes (B150)

    /// Read the layouts of `session`'s windows just before this client
    /// reports its size; [`Self::probe_layouts_after_sizing`] reads them
    /// again just after.
    pub(super) fn probe_layouts_before_sizing(&mut self, session: &str) {
        if !self.config.renders_output || !is_valid_id(session) {
            return;
        }
        self.size.presize = None;
        self.size.presize_awaits_after = true;
        self.send_command(
            format!("list-windows -t '{session}' -F '{SIZE_PROBE_FORMAT}'"),
            Reply::SizeProbe {
                session: session.to_owned(),
                before: true,
            },
        );
    }

    /// The layouts of `session`'s windows around a switch to it — ours, or
    /// the mosh sidecar's renderer's: the client that lands on a session
    /// becomes its latest, and tmux sizes its windows by it. Sent just
    /// before the switch; the second read goes out when the first one's
    /// reply is in (B150, round thirty-four): tmux sizes a session for a
    /// switch at the end of its loop, after the commands that came with the
    /// switch have run, so a read sent right behind it still found every
    /// window as it was, and nothing was recorded. A command that reaches
    /// tmux after a reply it sent runs in a later loop.
    pub(super) fn probe_switch_layouts(&mut self, session: &str) {
        if !is_valid_id(session) {
            return;
        }
        self.send_command(
            format!("list-windows -t '{session}' -F '{SIZE_PROBE_FORMAT}'"),
            Reply::SwitchProbe {
                session: session.to_owned(),
                earlier: None,
            },
        );
    }

    /// The first read asks for the second; on the second, every window the
    /// switch resized files its earlier layout as its record (if it
    /// qualifies, and has none yet).
    pub(super) fn switch_probe_answered(
        &mut self,
        session: &str,
        earlier: Option<BTreeMap<String, Vec<String>>>,
        response: &CommandResponse,
    ) {
        if response.is_error {
            return;
        }
        let rows = size_rows(response.lines.iter().map(String::as_str));
        let Some(earlier) = earlier else {
            self.send_command(
                format!("list-windows -t '{session}' -F '{SIZE_PROBE_FORMAT}'"),
                Reply::SwitchProbe {
                    session: session.to_owned(),
                    earlier: Some(rows),
                },
            );
            return;
        };
        for (window, now) in &rows {
            let Some(then) = earlier.get(window) else {
                continue;
            };
            if window_size(&then[3]) != window_size(&now[3]) {
                self.note_pristine_layout(window, then);
                self.note_ours(window, &now[3]);
            }
        }
    }

    /// The second read, when a first one went out.
    pub(super) fn probe_layouts_after_sizing(&mut self) {
        if !std::mem::take(&mut self.size.presize_awaits_after) {
            return;
        }
        let Some(session) = self.snapshot.active_session_id.clone() else {
            return;
        };
        self.send_command(
            format!("list-windows -t '{session}' -F '{SIZE_PROBE_FORMAT}'"),
            Reply::SizeProbe {
                session,
                before: false,
            },
        );
    }

    /// The first read is kept; on the second, every window the size request
    /// resized files its earlier layout as its record (if it qualifies, and
    /// has none yet). A window it left alone isn't ours to restore.
    pub(super) fn size_probe_answered(&mut self, before: bool, response: &CommandResponse) {
        let rows = if response.is_error {
            BTreeMap::new()
        } else {
            size_rows(response.lines.iter().map(String::as_str))
        };
        if before {
            self.size.presize = Some(rows);
            return;
        }
        let Some(earlier) = self.size.presize.take() else {
            return;
        };
        for (window, now) in &rows {
            let Some(then) = earlier.get(window) else {
                continue;
            };
            if window_size(&then[3]) != window_size(&now[3]) {
                self.note_pristine_layout(window, then);
                self.note_ours(window, &now[3]);
            }
        }
    }

    // MARK: The renderer's squash (B270)

    /// The mosh sidecar is told how every window was before mosh-server
    /// started the phone's renderer: `listing`, the host's `list-windows -a`
    /// in [`SIZE_PROBE_FORMAT`], read over the bootstrap's connection just
    /// before. The renderer is tmux's client, sized to the phone and sizing
    /// the session from the moment it attaches — with the phone the only
    /// client, the session's windows are at the phone's grid before this
    /// sidecar exists, and the layout probe ahead of its pin read the
    /// squashed layout. At the sidecar's first attach the windows are listed
    /// again, and every window the renderer resized files its earlier layout
    /// as its record, as our own size request's do (B150).
    pub fn note_layouts_before_renderer(&mut self, listing: &str) {
        if self.config.renders_output {
            return;
        }
        self.size.before_renderer = Some(size_rows(listing.lines()));
    }

    /// The listing after the renderer, at the sidecar's attach, when one was
    /// taken before it.
    pub(super) fn probe_layouts_after_renderer(&mut self) {
        let Some(rows) = self.size.before_renderer.take() else {
            return;
        };
        self.size.presize = Some(rows);
        self.send_command(
            format!("list-windows -a -F '{SIZE_PROBE_FORMAT}'"),
            Reply::SizeProbe {
                session: String::new(),
                before: false,
            },
        );
    }

    /// The command that takes the phone's mosh renderer out of the session
    /// before the windows are handed back (B270): `detach-client`, only
    /// while tmux still lists that client by its pid AND name. With the
    /// phone the only client, the renderer — mosh-server's tmux client —
    /// sized the session to the end: the hand-back's last step, back to
    /// automatic sizing, sized every window straight back to the phone's
    /// grid, and the renderer, stopped right after with its server, left
    /// them there. Detached first, no client is left to size them (tmux
    /// counts a client on its way out for nothing), and they stay at their
    /// record. `None` but for the sidecar that knows its renderer. Only on
    /// the session's real end: the renderer is mosh's program, so its
    /// detach ends the mosh session.
    pub fn renderer_detach_command(&self, tmux: &str) -> Option<String> {
        if self.config.renders_output {
            return None;
        }
        self.sidecar
            .renderer_client
            .as_ref()
            .map(|renderer| crate::boot::renderer_detach_command(tmux, renderer))
    }

    // MARK: The immersive zoom

    /// Zoom the active window's active pane if it has splits and isn't
    /// zoomed. One probe serves the zoom and, off a pin, the layout record.
    pub(super) fn ensure_immersive_zoom(&mut self) {
        let Some(window) = self.snapshot.active_window_id.clone() else {
            return;
        };
        let epoch = self.size.zoom_epoch;
        self.send_command(
            format!("display-message -p -t '{window}' '{PRISTINE_PROBE_FORMAT}'"),
            Reply::ZoomProbe { window, epoch },
        );
    }

    /// Something we sent changes a window's zoom (a toggle, a replayed
    /// layout): a zoom probe sent before it describes a zoom that is gone.
    pub(super) fn bump_zoom_epoch(&mut self) {
        self.size.zoom_epoch = self.size.zoom_epoch.wrapping_add(1);
    }

    /// The probe's zoom decision: a split window not zoomed is zoomed with a
    /// plain toggle (`resize-pane -Z`, exactly one reply block) — when the
    /// probe is newer than every zoom change we sent. An older one may read
    /// "unzoomed" for a window our own toggle has zoomed since (a reconnect's
    /// attach zoom and the restore's pane selection both probed before either
    /// toggle landed, and the second toggle undid the first): it asks again.
    ///
    /// The toggle used to be decided by tmux as it ran (`if-shell -F … 'resize-
    /// pane -Z'`). tmux answers an `if-shell` that runs its command with two
    /// reply blocks — its own, then the command's (3.6a, a private server) —
    /// and replies are matched to commands in send order: the command sent
    /// after the zoom got the zoom's empty block, and its own reply went to
    /// the next one. Over mosh the command right behind the attach's zoom is
    /// often the sidecar's renderer lookup: about one connect in fifteen on
    /// the loopback, the renderer was never identified and no window pinned
    /// (round thirty-four).
    pub(super) fn zoom_probe_answered(
        &mut self,
        window: &str,
        epoch: u64,
        response: &CommandResponse,
    ) {
        // A refusal describes no window (Swift parsed the complaint, which
        // never reads as a split window: the same outcome).
        if response.is_error {
            return;
        }
        let Some(line) = response.lines.first() else {
            return;
        };
        let fields: Vec<String> = words(line, b' ').into_iter().map(str::to_owned).collect();
        // Behind a pin this reply describes the squashed layout already.
        if !self.size.resized_windows.contains(window) {
            self.note_pristine_layout(window, &fields);
        }
        if fields.len() >= 2 && swift_int(&fields[0]).unwrap_or(1) > 1 && fields[1] == "0" {
            if epoch == self.size.zoom_epoch {
                self.send(zoom_command(window));
                self.bump_zoom_epoch();
                // The zoom is ours: marked, so a sweep after our death can
                // tell it from a desktop's own (B458). The probe's pane is
                // the active one, the pane the toggle zooms.
                if let (true, Some(name), Some(pane)) = (
                    self.config.renders_output,
                    self.size.our_client_name.clone(),
                    fields.get(2).filter(|pane| is_valid_id(pane)),
                ) {
                    self.send(format!(
                        "set-option -w -t {window} {ZOOM_MARK_OPTION} {}",
                        crate::format::command_string_literal(&format!("{name} {pane}"))
                    ));
                    self.size.zoom_marks.insert(window.to_owned());
                }
            } else {
                self.ensure_immersive_zoom();
            }
        }
    }

    // MARK: Layout changes

    pub(super) fn handle_layout_change(&mut self, window: &str, layout: &str, zoomed: bool) {
        let panes = pane_count(layout);
        let mut previous_panes = None;
        let mut previous_layout = None;
        let active_session = self.snapshot.active_session_id.clone().unwrap_or_default();
        let snapshot = self.snap();
        match snapshot.windows.get_mut(window) {
            Some(info) => {
                previous_panes = Some(pane_count(&info.layout).unwrap_or(info.pane_count));
                previous_layout = Some(std::mem::replace(&mut info.layout, layout.to_owned()));
                info.is_zoomed = zoomed;
                if let Some(panes) = panes {
                    info.pane_count = panes;
                }
            }
            None => {
                let mut info = crate::model::WindowInfo::new(window, &active_session);
                info.layout = layout.to_owned();
                info.pane_count = panes.unwrap_or(1);
                info.is_zoomed = zoomed;
                snapshot.windows.insert(window.to_owned(), info);
            }
        }
        // B771: tmux reports a change once per session holding the window
        // (each phone's group, B459): the same layout again is no change,
        // and no drift. Each copy re-pinned and counted for the war guard.
        // B775: the same as the last NOTIFICATION, never as a listing that
        // already showed the resize (the drift went unhandled: no reclaim,
        // no lease read, two phones stuck with nobody following).
        if self
            .size
            .reported_layouts
            .insert(window.to_owned(), layout.to_owned())
            .as_deref()
            == Some(layout)
        {
            return;
        }
        // Our layouts, followed through a resize (round thirty-four): a
        // window of ours (its record's layout, or one our sizing left it in)
        // reflowed to another size is still ours — the phone's grid moved
        // (the keyboard, a rotation) and tmux refitted it. The same size in
        // another layout is a rearrangement, and not ours.
        if let Some(previous) = previous_layout
            && let Some(record) = self.size.pristine_layouts.get(window)
            && window_size(&previous) != window_size(layout)
            && (previous == record.layout || record.ours.contains(&previous))
        {
            self.note_ours(window, layout);
        }
        let is_active = self.snapshot.active_window_id.as_deref() == Some(window);
        let renders = self.config.renders_output && !self.size.pins_released;
        // tmux unzooms a window before a split, a kill, a pane exit: the
        // phone then shows the squashed layout under the zoom. Zoom again —
        // when the pane count moved (an unzoom at the same count is a
        // person's prefix-z), and only as the lease holder.
        if renders
            && !zoomed
            && is_active
            && panes.is_some_and(|panes| panes > 1 && Some(panes) != previous_panes)
            && self.size.follow.is_none()
        {
            let held = self.size.leases.get(window).cloned();
            if !self.is_held_by_another_live_device(held.as_ref()) {
                self.ensure_immersive_zoom();
            }
        }
        // Reclaim the window when another client sized it otherwise.
        if renders
            && is_active
            && let Some(width) = window_width(layout)
        {
            if width != self.last_client_size.0 {
                self.size.active_window_at_foreign_width = true;
                self.request_pin(window, PinReason::Drift);
                self.arm_foreign_quiet_repin(window);
            } else if self.size.active_window_at_foreign_width {
                self.timers.cancel(&Slot::ForeignQuietRepin);
                if self
                    .size
                    .follow
                    .as_ref()
                    .is_some_and(|f| f.window_id == window)
                {
                    self.set_follow(None);
                    // B774: the holder let go and the window came back to
                    // us: ours now, claimed as the quiet re-check claims a
                    // freed window — unclaimed, the next device attaching
                    // took it from the person using it.
                    self.request_pin(window, PinReason::Recheck);
                }
                // Back at our width: repaint whatever the war painted.
                self.size.active_window_at_foreign_width = false;
                self.resync_active_pane_settling();
            }
        }
    }

    /// The reclaim behind a drift, with the war guard: three within 5 s back
    /// off for 5 s.
    fn reclaim_window_on_drift(&mut self, window: &str) {
        let now = self.wall();
        if self.size.reclaim_tracked_window.as_deref() != Some(window) {
            self.size.reclaim_tracked_window = Some(window.to_owned());
            self.size.recent_reclaims.clear();
            self.size.reclaim_suspended_until = None;
        }
        if let Some(until) = self.size.reclaim_suspended_until {
            if now < until {
                return;
            }
            self.size.reclaim_suspended_until = None;
            self.size.recent_reclaims.clear();
        }
        self.size.recent_reclaims.push(now);
        self.size
            .recent_reclaims
            .retain(|&at| now.since(at) <= RECLAIM_WINDOW);
        if self.size.recent_reclaims.len() >= RECLAIM_THRESHOLD {
            self.size.reclaim_suspended_until = Some(now.adding(RECLAIM_WINDOW));
            self.size.recent_reclaims.clear();
            return;
        }
        self.fit_window_to_client(window);
    }

    /// A war that ENDS at the other client's width sends no more layout
    /// changes: re-check after a quiet spell (through the lease gate),
    /// bypassing the backoff.
    fn arm_foreign_quiet_repin(&mut self, window: &str) {
        let delay = (self.config.foreign_quiet_repin_delay * 1000.0).round() as u64;
        self.arm(
            Slot::ForeignQuietRepin,
            delay,
            Action::ForeignQuietRepin(window.to_owned()),
        );
    }

    pub(super) fn foreign_quiet_repin(&mut self, window: &str) {
        if !self.size.active_window_at_foreign_width
            || !self.config.renders_output
            || self.size.pins_released
            || !self.snapshot.is_attached
            || self.snapshot.active_window_id.as_deref() != Some(window)
        {
            return;
        }
        self.request_pin(window, PinReason::Recheck);
        // If the resize's own layout change is lost, keep trying.
        self.arm_foreign_quiet_repin(window);
    }

    // MARK: Background and foreground

    /// Hand our windows back without tearing the connection down: the
    /// terminal left the foreground (`releaseWindowPins`). Leaves the sizing
    /// math (`ignore-size`), unpins, releases our leases, restores layouts —
    /// all before iOS may suspend us.
    ///
    /// Before the attach it holds the pins back instead: a controller that
    /// attaches for a terminal nobody is looking at (a reconnect while the
    /// app is on Home or in the background) stays out of the window's
    /// sizing — no pin, `ignore-size` from its first size — until
    /// [`Controller::repin_active_window`].
    pub fn release_window_pins(&mut self, now: u64) {
        self.begin_input(now);
        self.timers.cancel(&super::timers::Slot::PendingResize);
        self.timers.cancel(&Slot::SettleResync);
        if !self.snapshot.is_attached {
            self.size.pins_released = true;
        } else {
            self.size.pins_released = true;
            self.send("refresh-client -f ignore-size".to_owned());
            // B150: alone, tmux still sizes by us whatever the flag; at the
            // recorded size the windows return to where they were.
            if self.config.renders_output
                && let Some((cols, rows)) = self.hand_back_size(&self.size.pristine_layouts)
            {
                self.send(format!("refresh-client -C {cols}x{rows}"));
            }
            let mut pinned: Vec<String> = self.size.resized_windows.iter().cloned().collect();
            pinned.sort_by(|a, b| id_cmp(a, b));
            for window in &pinned {
                self.send(format!("set-option -u -w -t {window} window-size"));
            }
            self.size.resized_windows.clear();
            let leased: Vec<String> = self.size.leases.keys().cloned().collect();
            for window in &leased {
                self.release_size_ownership(window);
            }
            // The other device may hold any of these when we come back.
            self.size.leases.clear();
            self.set_follow(None);
            self.timers.cancel(&Slot::ForegroundProbe);
            let mut zoomed: Vec<String> = self.size.zoom_marks.iter().cloned().collect();
            zoomed.sort_by(|a, b| id_cmp(a, b));
            for window in &zoomed {
                self.hand_back_zoom(window);
            }
            let mut recorded: Vec<String> = self.size.pristine_layouts.keys().cloned().collect();
            recorded.sort_by(|a, b| id_cmp(a, b));
            for window in &recorded {
                self.restore_pristine_layout(window);
            }
            for detector in self.bell_detectors.values_mut() {
                detector.reset();
            }
        }
        self.finish_input();
    }

    /// Back in the foreground: rejoin the sizing math, re-assert our size,
    /// ask who holds the window (a device may have taken it meanwhile), pin,
    /// and repaint (`repinActiveWindow`).
    pub fn repin_active_window(&mut self, now: u64) {
        self.begin_input(now);
        self.repin_active_window_now();
        self.finish_input();
    }

    fn repin_active_window_now(&mut self) {
        // What moved while away, numbers and order included (B586).
        self.relist_now();
        let Some(window) = self
            .snapshot
            .active_window_id
            .clone()
            .filter(|_| self.snapshot.is_attached)
        else {
            self.size.pins_released = false;
            return;
        };
        if !self.config.renders_output && self.renderer_window_id().is_none() {
            self.size.pins_released = false;
            return;
        }
        let (cols, rows) = self.last_client_size;
        if self.config.renders_output
            && let Some(session) = self.snapshot.active_session_id.clone()
        {
            self.probe_layouts_before_sizing(&session);
        }
        if self.config.renders_output {
            self.send("refresh-client -f !ignore-size".to_owned());
            self.send(format!("refresh-client -C {cols}x{rows}"));
        }
        // The mosh sidecar stays sizeless (B270): a size reported here made
        // it a client tmux sizes the session by — the windows went back to
        // the phone's grid with it, after the renderer was gone, whenever
        // they were handed back to automatic sizing (and tmux streamed it
        // every pane's output it never reads). Its pin is `resize-window`.
        self.probe_layouts_after_sizing();
        if self.config.renders_output {
            let timeout = (self.config.foreground_probe_timeout * 1000.0).round() as u64;
            self.arm(
                Slot::ForegroundProbe,
                timeout,
                Action::ForegroundProbeTimeout(window.clone()),
            );
            self.request_pin(&window, PinReason::Foreground);
        } else {
            self.size.pins_released = false;
            self.capture_pristine_layout(&window);
            self.send(format!("resize-window -t {window} -x {cols} -y {rows}"));
            self.size.resized_windows.insert(window);
            self.ensure_immersive_zoom();
        }
        if !self.config.renders_output {
            return;
        }
        // Catch up on what happened while away: one repaint after the settle
        // (no veil: a few seconds stale beats a blank).
        self.arm_convergence_resync();
        self.arm(Slot::SettleResync, 300, Action::ForegroundResync);
    }

    pub(super) fn foreground_probe_timed_out(&mut self, window: &str) {
        if !self.size.pins_released {
            return;
        }
        self.end_foreground_probe();
        self.pin_now(window, PinReason::Foreground);
    }

    /// While the pin is handed back, or the window follows another device's
    /// size, its output is laid out for a grid that is not ours: not painted
    /// (only scanned for bells).
    pub(super) fn output_gated(&self, pane: &str) -> bool {
        if self.size.pins_released {
            return true;
        }
        self.size.follow.as_ref().is_some_and(|follow| {
            self.snapshot
                .panes
                .get(pane)
                .map(|info| info.window_id.as_str())
                == Some(follow.window_id.as_str())
        })
    }

    // MARK: The client size

    /// The view's grid (`resizeClient`), debounced 180 ms: a keyboard or IME
    /// animation reports a storm of sizes; tmux gets the last. The view's
    /// first report always commits, even when it equals the seeded guess.
    /// Returns whether a commit is coming.
    pub fn resize_client(&mut self, rows: i64, cols: i64, now: u64) -> bool {
        self.begin_input(now);
        let changed = self.resize_client_now(rows, cols);
        self.finish_input();
        changed
    }

    pub(super) fn resize_client_now(&mut self, rows: i64, cols: i64) -> bool {
        if (cols, rows) == self.last_client_size && self.client_size_confirmed {
            return false;
        }
        self.client_size_confirmed = true;
        self.last_client_size = (cols, rows);
        self.arm(Slot::PendingResize, 180, Action::CommitClientSize);
        true
    }

    /// The settled size: tell tmux, re-pin the visible window, repaint.
    pub(super) fn commit_client_size(&mut self) {
        let (cols, rows) = self.last_client_size;
        self.send(format!("refresh-client -C {cols}x{rows}"));
        if !self.size.pins_released
            && let Some(window) = self.snapshot.active_window_id.clone()
        {
            self.request_pin(&window, PinReason::Resize);
        }
        self.resync_active_pane_settling();
    }

    // MARK: Reconnect handoff

    /// Hand over what this controller owes the server and forget it: the
    /// caller retires it without a teardown round trip.
    pub fn take_pin_ledger(&mut self) -> PinLedger {
        let ledger = PinLedger {
            resized_windows: std::mem::take(&mut self.size.resized_windows),
            pristine_layouts: std::mem::take(&mut self.size.pristine_layouts),
            renderer: self.sidecar.renderer_client.take(),
            hidden_status: self.sidecar.status_hidden_session_id.take(),
            owned_sessions: self.sidecar.owned_session_ids.clone(),
        };
        self.sidecar.renderer_session_id = None;
        ledger
    }

    /// Take on a retired controller's debt; what this one learned itself is
    /// newer and wins.
    pub fn inherit_pin_ledger(&mut self, ledger: PinLedger) {
        self.size.resized_windows.extend(ledger.resized_windows);
        for (window, layout) in ledger.pristine_layouts {
            self.size.pristine_layouts.entry(window).or_insert(layout);
        }
        if self.sidecar.renderer_client.is_none() {
            self.sidecar.renderer_client = ledger.renderer;
        }
        self.sidecar.owned_session_ids.extend(ledger.owned_sessions);
        if self.sidecar.status_hidden_session_id.is_none() {
            self.sidecar.status_hidden_session_id = ledger.hidden_status;
        }
    }

    /// The agent stamps as last read, for the replacement; `None` if never
    /// read.
    pub fn take_agent_hooks(&self) -> Option<BTreeMap<String, crate::hooks::AgentHook>> {
        self.agent_hooks_loaded.then(|| self.agent_hooks.clone())
    }

    /// Start from a retired controller's stamps; this attach's first poll
    /// replaces them.
    pub fn inherit_agent_hooks(&mut self, hooks: BTreeMap<String, crate::hooks::AgentHook>) {
        if self.agent_hooks_loaded {
            return;
        }
        self.agent_hooks = hooks;
        self.agent_hooks_loaded = true;
    }

    // MARK: Tearing down

    pub(super) fn detach_size_state(&mut self) {
        self.size.active_window_at_foreign_width = false;
        self.timers.cancel(&Slot::ForeignQuietRepin);
        self.sidecar.renderer_locate_in_flight = false;
        self.size.leases.clear();
        self.size.pending_attach_lease = None;
        self.size.probes_in_flight.clear();
        self.size.listed_clients = None;
        self.size.our_client_name = None;
        self.set_follow(None);
        self.timers.cancel(&Slot::ForegroundProbe);
    }

    /// Nothing on its way will be answered: what waited for a reply must not
    /// block the next ones.
    pub(super) fn closed_in_flight(&mut self) {
        self.sidecar.renderer_locate_in_flight = false;
        self.size.pristine_captures_in_flight.clear();
        self.size.presize = None;
        self.size.presize_awaits_after = false;
        // Fix (probe-stuck-after-exit): Swift kept the lease probes marked
        // in flight, so after `%exit` and a new control session on the same
        // channel no window's lease was ever read again.
        self.size.probes_in_flight.clear();
        self.size.listed_clients = None;
    }
}

/// The window id, then [`PRISTINE_PROBE_FORMAT`]'s fields: a session's
/// layouts around our size request (B150).
pub const SIZE_PROBE_FORMAT: &str = "#{window_id} #{window_panes} #{window_zoomed_flag} #{pane_id} #{window_layout} #{window-size} #{@moshpit_layout}";

/// A size probe's rows (window id, then [`PRISTINE_PROBE_FORMAT`]'s fields),
/// by window.
fn size_rows<'a>(lines: impl Iterator<Item = &'a str>) -> BTreeMap<String, Vec<String>> {
    let mut rows = BTreeMap::new();
    for line in lines {
        let fields: Vec<String> = words(line, b' ').into_iter().map(str::to_owned).collect();
        if fields.len() >= 5 && fields[0].starts_with('@') && is_valid_id(&fields[0]) {
            rows.insert(fields[0].clone(), fields[1..].to_vec());
        }
    }
    rows
}

/// One window's record as teardown commands (no tmux prefix): sized to the
/// record first when no client of ours is left to size it (`sized`), the
/// layout, back to automatic sizing, the user's own zoom.
fn window_restore_commands(window: &str, pristine: &PristineLayout, sized: bool) -> Vec<String> {
    let mut commands = Vec::new();
    let size = window_size(&pristine.layout).filter(|_| sized);
    if let Some((cols, rows)) = size {
        commands.push(format!("resize-window -t {window} -x {cols} -y {rows}"));
    }
    commands.push(format!("select-layout -t {window} '{}'", pristine.layout));
    // B772: put back, the shared record is nobody's any more.
    commands.push(format!("set-option -wu -t {window} {SHARED_LAYOUT_OPTION}"));
    if size.is_some() {
        commands.push(format!("set-option -u -w -t {window} window-size"));
    }
    if let Some(pane) = &pristine.zoomed_pane {
        commands.push(format!("resize-pane -Z -t {pane}"));
    }
    commands
}

/// A client name that can go single-quoted into a teardown command through
/// a login shell: a tty path or tmux's `client-<pid>`.
fn is_plain_client_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
}

/// `#{window_panes} #{window_zoomed_flag} #{pane_id} #{window_layout}`: one
/// probe for the zoom and the layout record; then whether the window is
/// pinned (`#{window-size}`) and the record shared on it (B772).
pub const PRISTINE_PROBE_FORMAT: &str = "#{window_panes} #{window_zoomed_flag} #{pane_id} #{window_layout} #{window-size} #{@moshpit_layout}";

/// The window option a window's record is shared in (B772), for a device
/// that comes while another one has it pinned: [`SharedLayout`].
pub const SHARED_LAYOUT_OPTION: &str = "@moshpit_layout";

/// A record shared on a window (B772): `<panes>|<the user's zoomed pane, or
/// ->|<window_layout>`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SharedLayout {
    pane_count: i64,
    zoomed_pane: Option<String>,
    layout: String,
}

impl SharedLayout {
    fn parse(raw: &str) -> Option<Self> {
        let mut parts = raw.splitn(3, '|');
        let pane_count = swift_int(parts.next()?).filter(|&panes| panes > 1)?;
        let zoomed_pane = match parts.next()? {
            "-" => None,
            pane if is_valid_id(pane) => Some(pane.to_owned()),
            _ => return None,
        };
        let layout = parts.next().filter(|layout| is_plausible_layout(layout))?;
        Some(Self {
            pane_count,
            zoomed_pane,
            layout: layout.to_owned(),
        })
    }

    fn encoded(&self) -> String {
        format!(
            "{}|{}|{}",
            self.pane_count,
            self.zoomed_pane.as_deref().unwrap_or("-"),
            self.layout
        )
    }
}

/// Zoom a window's active pane: a toggle, sent only on a probe that saw it
/// unzoomed and is newer than every zoom change we sent
/// ([`Controller::zoom_probe_answered`]).
/// The window option that marks a zoom this phone made: `<client> <pane>`
/// (B458).
pub const ZOOM_MARK_OPTION: &str = "@moshpit_zoomed";

/// The dead-pin sweep's window listing (B456, B458): the window, whether it
/// is zoomed, its active pane, the phone's zoom mark and the size lease.
pub const SWEEP_FORMAT: &str = "#{window_id} #{window_zoomed_flag} #{pane_id} zoom=#{@moshpit_zoomed} lease=#{@moshpit_size_owner}";

/// One row of [`SWEEP_FORMAT`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct SweepRow {
    window: String,
    zoomed: bool,
    active_pane: String,
    /// The zoom mark's client and pane, when it has both.
    zoom: Option<(String, String)>,
    /// The lease as stored ("" when none).
    lease: String,
}

impl SweepRow {
    fn parse(line: &str) -> Option<Self> {
        let mut words = line.splitn(4, ' ');
        let window = words.next().filter(|id| is_valid_id(id))?;
        let zoomed = words.next()? == "1";
        let active_pane = words.next()?.to_owned();
        let rest = words.next()?.strip_prefix("zoom=")?;
        let (zoom, lease) = rest.split_once(" lease=")?;
        let zoom = zoom
            .split_once(' ')
            .filter(|(client, pane)| !client.is_empty() && is_valid_id(pane))
            .map(|(client, pane)| (client.to_owned(), pane.to_owned()));
        Some(Self {
            window: window.to_owned(),
            zoomed,
            active_pane,
            zoom,
            lease: trim_whitespaces(lease).to_owned(),
        })
    }
}

pub fn zoom_command(window: &str) -> String {
    format!("resize-pane -Z -t {window}")
}

fn lease_state(response: &CommandResponse, clients: &BTreeSet<String>) -> LeaseState {
    let raw = response
        .lines
        .first()
        .map(|line| trim_whitespaces(line))
        .unwrap_or("");
    match SizeLease::parse(raw, clients) {
        Some(lease) if !response.is_error => LeaseState::Held(lease),
        _ => LeaseState::Unset,
    }
}
