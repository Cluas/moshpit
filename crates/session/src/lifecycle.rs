//! A session's lifecycle as a pure state machine: when to dial, which
//! results still count, when a connection is lost or ended, when to probe,
//! redial or leave it alone.
//!
//! Inputs carry the caller's monotonic clock in milliseconds; outputs queue
//! up as [`Action`]s for the driver ([`crate::Sessions`]'s session task) to
//! carry out, in order. [`Lifecycle::next_deadline`] says when to call
//! [`Lifecycle::tick`]. Nothing here sleeps, spawns or touches a socket, so
//! every rule below is tested on a virtual clock (`tests` at the bottom).
//!
//! The rules, from the app's `SessionHub`/`TerminalViewModel` (and where
//! this differs, why):
//!
//! - **Generations.** Every attempt gets the next generation number, and so
//!   does every teardown: a dial, probe, loss or end reported for an older
//!   generation is nobody's business — a superseded dial's connection is
//!   closed, its failure dropped ([`Lifecycle::attempt_succeeded`] says
//!   which). The app numbered dials (`dialGeneration`) and starts
//!   (`startGeneration`) but checked them only at some awaits (B122); here
//!   the attempt is one task whose every result goes through this check.
//! - **End versus drop.** A clean remote end (the shell's `exit`,
//!   mosh-server's shutdown) ends the session ([`Lifecycle::ended`]): no
//!   keepalive, foreground return or network change redials it (B103). A
//!   lost transport is redialled at once ([`Lifecycle::lost`]), as an
//!   automatic attempt: the state reads `Reconnecting` until it is back.
//! - **Dial recovery.** A session with no live transport is redialled at
//!   once on the foreground return and on a network change: a dial out for
//!   longer than the grace (2 s; 1 s for a path change) is restarted on a
//!   fresh socket, a failed session dialled again. While in the foreground
//!   the 12 s keepalive tick retries it too.
//! - **Failures that retrying can't fix aren't retried.** A refused host
//!   key or rejected credentials stay `Failed` until the person tries again
//!   ([`Failure::retryable`]). The app retried every failed session on
//!   every tick and every return (`keepAlive`'s `.failed` case,
//!   `retryIfDown`): a declined "Trust this host?" card came back every
//!   12 s, and a wrong password went to the server every 12 s.
//! - **Keepalive.** Every 12 s in the foreground: a live SSH transport that
//!   delivered nothing for 11 s is probed (an exec of `true`, 8 s); an error
//!   is a drop, a silence only on the second in a row. mosh is nudged (its
//!   transport keeps its own liveness). In the background nothing runs.
//! - **The foreground return.** After more than 20 s away an SSH transport
//!   is redialled without asking it (iOS has killed the socket; a probe on a
//!   half-open one can pass), a mosh flow replaced and repainted. A shorter
//!   absence probes the SSH transport once with a 4 s deadline, any answer
//!   but alive reconnects; mosh gets a nudge.
//! - **Resuming in place** ([`Timing::resume_in_place`], a prototype, off by
//!   default): a live SSH transport is asked on every return, however long
//!   the app was away, with one round trip on the leg itself
//!   ([`Action::ResumeProbe`], [`Timing::resume_probe_timeout`]); an answer
//!   keeps it — nothing on screen changes — anything else reconnects.
//!   Keystrokes wait for the answer ([`Lifecycle::resume_pending`]).
//! - **Network changes** are acted on 400 ms after the last of a burst, only
//!   in the foreground: dials and down sessions as above, a live SSH
//!   transport checked like a keepalive tick, a live mosh flow roamed onto
//!   the new path.

use std::collections::VecDeque;
use std::time::Duration;

use crate::event::{EndReason, Failure, SessionState};
use crate::spec::{Protocol, Timing, ms};

/// What carries a live session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// An SSH connection: a shell, tmux's control client, herdr's frame
    /// channel, or mosh's fall back to SSH.
    Ssh,
    /// mosh over UDP; the bootstrap SSH connection is closed.
    Mosh,
}

/// Why a live transport is torn down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Teardown {
    /// It is being replaced: no courtesy round trip on a connection judged
    /// dead, and a multiplexer's last frame stays on screen until the
    /// replacement paints.
    Reconnect,
    /// The remote end finished the session.
    Ended,
    /// The person closed the session: hand back what we hold on the server
    /// (tmux's window pins) and stop a mosh-server we started.
    Disconnect,
    /// The person chose SSH over a mosh session that hears nothing back.
    SwitchToSsh,
}

/// What a liveness probe found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Alive,
    /// The probe failed outright (an error, a reset): proof of death.
    Failed,
    /// No answer by the deadline: only an absence of evidence.
    TimedOut,
}

/// What the driver is asked to do, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Start an attempt: dial, sign in, bring up the transport. Every result
    /// it reports carries `generation`.
    Dial {
        generation: u64,
        protocol: Protocol,
        automatic: bool,
    },
    /// The attempt of `generation` is given up: drop it, close whatever it
    /// made (its host-key card included).
    Abandon { generation: u64 },
    /// Tear down the live transport of `generation`.
    Teardown { generation: u64, why: Teardown },
    /// Run a liveness probe on the live SSH transport; its answer names
    /// `probe` ([`Lifecycle::probe_answered`]).
    Probe {
        generation: u64,
        probe: u64,
        timeout: Duration,
    },
    /// The in-place return's question ([`Timing::resume_in_place`]): one
    /// round trip on the live leg itself (tmux's control channel, else the
    /// SSH keepalive), answered with [`Lifecycle::probe_answered`]. Input
    /// waits for the answer ([`Lifecycle::resume_pending`]).
    ResumeProbe {
        generation: u64,
        /// What its answer names.
        probe: u64,
        timeout: Duration,
        /// How long the app was away (for the diagnostics).
        away_ms: u64,
    },
    /// The in-place return found the SSH transport that was live when the
    /// app left already gone (closed while away, and heard before the
    /// return): nothing to ask, but the diagnostics still hear the return's
    /// one outcome (dead).
    ResumeLost {
        /// How long the app was away.
        away_ms: u64,
    },
    /// The mosh flow: replace it and repaint (`force`), or just send now.
    ResumeMosh { generation: u64, force: bool },
    /// The network changed under a live mosh flow: move it to a fresh socket.
    RoamMosh { generation: u64 },
    /// The app went to the background: hand back what holds other clients
    /// (tmux's window pins).
    Background { generation: u64 },
    /// The app came back without a forced reconnect: take the pins again.
    Foreground { generation: u64 },
    /// The public state changed.
    State(SessionState),
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    /// Not connected yet.
    Idle,
    Dialing {
        since: u64,
    },
    Live {
        transport: Transport,
    },
    /// No transport: retried on its own if the failure is retryable.
    Down {
        failure: Failure,
    },
    Ended(EndReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbePurpose {
    Keepalive,
    Resume,
    /// The network changed under it (B446): any doubt reconnects, as on a
    /// return, but input doesn't wait for the answer.
    Path,
    /// A delivery asked for in the background (B524): any doubt reconnects,
    /// and deliveries wait for the answer.
    Delivery,
}

/// See the module documentation.
#[derive(Debug)]
pub struct Lifecycle {
    timing: Timing,
    protocol: Protocol,
    generation: u64,
    phase: Phase,
    /// The attempt in flight (or the one that left the session down) was
    /// automatic: the state reads `Reconnecting`, failures are not news.
    automatic: bool,
    foreground: bool,
    backgrounded_at: Option<u64>,
    /// How long the app was away, at the last return.
    away: u64,
    /// The app left with a live SSH transport and resume in place on: the
    /// return owes the diagnostics an outcome even if that transport is
    /// gone by then.
    left_live: bool,
    keepalive_at: Option<u64>,
    path_kick_at: Option<u64>,
    /// The network changed while the app was away (B447): the return acts
    /// on it.
    path_changed_away: bool,
    /// The dial waits on the person: its credential read (Face ID, the
    /// passphrase card) or a host-key question (B395).
    on_person: bool,
    /// The probe out, and its number: only its own answer counts (B394).
    probe: Option<(ProbePurpose, u64)>,
    /// The last probe's number.
    probes: u64,
    probe_timeouts: u32,
    last_inbound: Option<u64>,
    published: Option<SessionState>,
    actions: VecDeque<Action>,
}

impl Lifecycle {
    /// A session for a connection over `protocol`, created while the app is
    /// in the foreground or not.
    pub fn new(timing: Timing, protocol: Protocol, foreground: bool, now: u64) -> Self {
        let keepalive_at = foreground.then(|| now + ms(timing.keepalive_interval));
        Self {
            timing,
            protocol,
            generation: 0,
            phase: Phase::Idle,
            automatic: false,
            foreground,
            backgrounded_at: (!foreground).then_some(now),
            away: 0,
            left_live: false,
            keepalive_at,
            path_kick_at: None,
            path_changed_away: false,
            on_person: false,
            probe: None,
            probes: 0,
            probe_timeouts: 0,
            last_inbound: None,
            published: None,
            actions: VecDeque::new(),
        }
    }

    // MARK: Reading

    /// The current generation: the live transport's, or the dial's.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// What the session dials next.
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// The public state.
    pub fn state(&self) -> SessionState {
        match &self.phase {
            Phase::Idle => SessionState::Connecting,
            Phase::Dialing { .. } if self.automatic => SessionState::Reconnecting,
            Phase::Dialing { .. } => SessionState::Connecting,
            Phase::Live { .. } => SessionState::Connected,
            Phase::Down { failure } if self.automatic && failure.retryable => {
                SessionState::Reconnecting
            }
            Phase::Down { failure } => SessionState::Failed(failure.clone()),
            Phase::Ended(reason) => SessionState::Ended(reason.clone()),
        }
    }

    /// The live transport, if there is one.
    pub fn live(&self) -> Option<Transport> {
        match self.phase {
            Phase::Live { transport } => Some(transport),
            _ => None,
        }
    }

    pub fn is_ended(&self) -> bool {
        matches!(self.phase, Phase::Ended(_))
    }

    pub fn is_foreground(&self) -> bool {
        self.foreground
    }

    /// The in-place return is waiting for the live transport's answer:
    /// keystrokes wait with it, rather than go into a socket that may have
    /// died during the suspension.
    pub fn resume_pending(&self) -> bool {
        self.timing.resume_in_place
            && matches!(self.probe, Some((ProbePurpose::Resume, _)))
            && self.live() == Some(Transport::Ssh)
    }

    /// Resume in place turned on or off while the app runs (B365). The next
    /// move to the background and back follows it: the app reads its
    /// switch again around every such move, so a change takes effect at the
    /// next return, not at a launch the person never sees (iOS keeps a
    /// process for hours across returns).
    pub fn set_resume_in_place(&mut self, on: bool) {
        self.timing.resume_in_place = on;
    }

    /// The next action, oldest first.
    pub fn poll_action(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }

    /// When [`Lifecycle::tick`] has something to do.
    pub fn next_deadline(&self) -> Option<u64> {
        [self.keepalive_at, self.path_kick_at]
            .into_iter()
            .flatten()
            .min()
    }

    // MARK: The person

    /// Connect, or try again after a failure or an end. A session that is
    /// dialling or live is left alone.
    pub fn connect(&mut self, now: u64) {
        match self.phase {
            Phase::Dialing { .. } | Phase::Live { .. } => {}
            _ => self.begin(false, now),
        }
    }

    /// Close the session: whatever is dialling is abandoned, whatever is live
    /// torn down, and nothing brings it back.
    pub fn disconnect(&mut self, _now: u64) {
        match self.phase {
            Phase::Ended(_) => return,
            Phase::Live { .. } => self.push(Action::Teardown {
                generation: self.generation,
                why: Teardown::Disconnect,
            }),
            Phase::Dialing { .. } => self.push(Action::Abandon {
                generation: self.generation,
            }),
            Phase::Idle | Phase::Down { .. } => {}
        }
        self.generation += 1;
        self.phase = Phase::Ended(EndReason::Disconnected);
        self.probe = None;
        self.publish();
    }

    /// Use SSH from now on (the person's answer to a mosh session that hears
    /// nothing back): the mosh transport goes, SSH is dialled.
    pub fn switch_to_ssh(&mut self, now: u64) {
        if self.protocol == Protocol::Ssh || self.is_ended() {
            return;
        }
        self.protocol = Protocol::Ssh;
        match self.phase {
            Phase::Live { .. } => self.push(Action::Teardown {
                generation: self.generation,
                why: Teardown::SwitchToSsh,
            }),
            Phase::Dialing { .. } => self.push(Action::Abandon {
                generation: self.generation,
            }),
            _ => {}
        }
        self.begin(false, now);
    }

    // MARK: The attempt

    /// The attempt of `generation` brought `transport` up. `false`: the
    /// attempt was superseded or the session ended meanwhile, and the
    /// driver closes what it made.
    pub fn attempt_succeeded(&mut self, generation: u64, transport: Transport, now: u64) -> bool {
        if generation != self.generation || !matches!(self.phase, Phase::Dialing { .. }) {
            return false;
        }
        self.phase = Phase::Live { transport };
        self.automatic = false;
        self.last_inbound = Some(now);
        self.probe = None;
        self.probe_timeouts = 0;
        self.publish();
        true
    }

    /// The attempt of `generation` waits on the person (`true`) — its
    /// credential read (Face ID, the passphrase card) or a host-key question
    /// — or no longer does. Meanwhile a return or a network change leaves
    /// the dial alone (B395): restarting it replaced the card the person was
    /// answering — a passphrase fetched from another app — with a blank one,
    /// and while the credential is read there is no socket to replace. A
    /// host that hung up during a host-key question is dialled again after
    /// the answer (B391).
    pub fn attempt_waits_on_person(&mut self, generation: u64, waiting: bool) {
        if generation == self.generation && matches!(self.phase, Phase::Dialing { .. }) {
            self.on_person = waiting;
        }
    }

    /// The attempt of `generation` failed.
    pub fn attempt_failed(&mut self, generation: u64, failure: Failure, _now: u64) {
        if generation != self.generation || !matches!(self.phase, Phase::Dialing { .. }) {
            return;
        }
        self.phase = Phase::Down { failure };
        self.publish();
    }

    // MARK: The live transport

    /// The live transport of `generation` went away (a reset, a dead tmux
    /// control channel, a keepalive that failed): redial at once.
    pub fn lost(&mut self, generation: u64, now: u64) {
        if generation != self.generation || self.live().is_none() {
            return;
        }
        self.reconnect(now);
    }

    /// The remote end finished the session of `generation`: the end, not a
    /// drop.
    pub fn ended(&mut self, generation: u64, reason: EndReason, _now: u64) {
        if generation != self.generation
            || !matches!(self.phase, Phase::Live { .. } | Phase::Dialing { .. })
        {
            return;
        }
        if self.live().is_some() {
            self.push(Action::Teardown {
                generation,
                why: Teardown::Ended,
            });
        } else {
            self.push(Action::Abandon { generation });
        }
        self.generation += 1;
        self.phase = Phase::Ended(reason);
        self.probe = None;
        self.publish();
    }

    /// Bytes arrived on the live transport: proof of life.
    pub fn inbound(&mut self, now: u64) {
        self.last_inbound = Some(self.last_inbound.map_or(now, |at| at.max(now)));
    }

    /// Probe `probe` of `generation` answered. Only the probe out counts:
    /// a keepalive from before a suspension, or the question of a return the
    /// app left before it was answered, answers nothing now (B394) — its
    /// round trip says nothing of the socket after the suspension.
    pub fn probe_answered(&mut self, generation: u64, probe: u64, result: Probe, now: u64) {
        if generation != self.generation || self.live().is_none() {
            return;
        }
        let Some((purpose, _)) = self.probe.take_if(|(_, out)| *out == probe) else {
            return;
        };
        match (purpose, result) {
            (_, Probe::Alive) => {
                self.probe_timeouts = 0;
                self.last_inbound = Some(now);
            }
            // Coming back, with the person watching, onto another path, or
            // for a delivery: any doubt reconnects.
            (ProbePurpose::Resume | ProbePurpose::Path | ProbePurpose::Delivery, _) => {
                self.reconnect(now)
            }
            (ProbePurpose::Keepalive, Probe::Failed) => {
                self.probe_timeouts = 0;
                self.reconnect(now);
            }
            (ProbePurpose::Keepalive, Probe::TimedOut) => {
                self.probe_timeouts += 1;
                if self.probe_timeouts >= self.timing.probe_timeouts_to_drop {
                    self.probe_timeouts = 0;
                    self.reconnect(now);
                }
            }
        }
    }

    /// A delivery is asked for in the background (a notification's Allow,
    /// Deny or reply, the Live Activity's buttons): a live SSH transport not
    /// heard from lately is asked first, one round trip within the return's
    /// deadline (B524). iOS may have killed its socket while the app was
    /// suspended, and in the background nothing else asks it: the delivery
    /// was written into the dead socket and said sent. Any doubt reconnects;
    /// deliveries wait for the answer ([`Lifecycle::delivery_check_pending`]).
    pub fn check_for_delivery(&mut self, now: u64) {
        if self.foreground || self.live() != Some(Transport::Ssh) || self.delivery_check_pending() {
            return;
        }
        if self
            .last_inbound
            .is_some_and(|at| now.saturating_sub(at) < ms(self.timing.inbound_proof))
        {
            return;
        }
        // A keepalive's or a path change's question already out — one from
        // before the suspension answers nothing about the socket now (B394)
        // — is replaced by the delivery's own, which deliveries wait for
        // (B610). Skipped, the delivery went into a possibly dead socket and
        // was said sent. A return's question, which holds input, stands.
        if matches!(
            self.probe,
            Some((ProbePurpose::Keepalive | ProbePurpose::Path, _))
        ) {
            self.probe = None;
        }
        self.start_probe(ProbePurpose::Delivery, self.timing.foreground_probe_timeout);
    }

    /// A delivery's question is out: deliveries wait for its answer.
    pub fn delivery_check_pending(&self) -> bool {
        matches!(self.probe, Some((ProbePurpose::Delivery, _)))
    }

    // MARK: The app

    /// The app came to the foreground.
    pub fn foreground(&mut self, now: u64) {
        let away = self
            .backgrounded_at
            .take()
            .map_or(0, |at| now.saturating_sub(at));
        let was_foreground = self.foreground;
        self.foreground = true;
        self.away = away;
        let left_live = std::mem::take(&mut self.left_live);
        let path_changed = std::mem::take(&mut self.path_changed_away);
        if self.keepalive_at.is_none() {
            self.keepalive_at = Some(now + ms(self.timing.keepalive_interval));
        }
        if was_foreground && away == 0 {
            // Told twice: one return, one resume.
            return;
        }
        // A probe out from before the suspension answers nothing about the
        // socket now: the return asks its own (B394). A question the app
        // left before it was answered kept input waiting meanwhile; the new
        // one does from here.
        self.probe = None;
        // In place, a live SSH transport is asked however long the app was
        // away: a socket that survived the suspension is kept.
        let in_place = self.timing.resume_in_place
            && matches!(
                self.phase,
                Phase::Live {
                    transport: Transport::Ssh
                }
            );
        if left_live && !in_place && !self.is_ended() {
            self.push(Action::ResumeLost { away_ms: away });
        }
        if away > ms(self.timing.force_reconnect_after) && !in_place {
            self.force_resume(now);
        } else {
            if self.live().is_some() {
                self.push(Action::Foreground {
                    generation: self.generation,
                });
            }
            if path_changed && self.live() == Some(Transport::Mosh) {
                // The network changed while the app was away (B447): the
                // flow's socket is bound to the path it left on, and a nudge
                // would send from an address that may be gone. Roam it.
                // (SSH: the return asks the transport anyway, deadline and
                // all; a dial or a down session is retried as on any return.)
                self.push(Action::RoamMosh {
                    generation: self.generation,
                });
            } else {
                self.resume_if_needed(now);
            }
        }
    }

    /// The app went to the background: nothing runs until it is back.
    pub fn background(&mut self, now: u64) {
        if !self.foreground {
            return;
        }
        self.foreground = false;
        self.backgrounded_at = Some(now);
        self.keepalive_at = None;
        // A change still settling is acted on at the return (B447).
        self.path_changed_away |= self.path_kick_at.take().is_some();
        self.left_live = self.timing.resume_in_place
            && matches!(
                self.phase,
                Phase::Live {
                    transport: Transport::Ssh
                }
            );
        if self.live().is_some() {
            self.push(Action::Background {
                generation: self.generation,
            });
        }
    }

    /// The device was unlocked: a session down because the keychain was
    /// locked (the saved credentials, the trusted host keys) dials again at
    /// once (B201). Anything else down is left to its own rules.
    pub fn unlocked(&mut self, now: u64) {
        if let Phase::Down { failure } = &self.phase
            && failure.waits_for_unlock()
        {
            self.begin(true, now);
        }
    }

    /// The device's network path changed. Acted on once a burst settles in
    /// the foreground; heard in the background, at the return (B447). It
    /// used to be dropped there, as the return "resumes anyway" — but a
    /// short return only nudges a mosh flow, whose socket is bound to the
    /// path it left on: after locking on Wi-Fi and unlocking on cellular, it
    /// sent from an address that was gone.
    pub fn network_changed(&mut self, now: u64) {
        if self.is_ended() {
            return;
        }
        if !self.foreground {
            self.path_changed_away = true;
            return;
        }
        self.path_kick_at = Some(now + ms(self.timing.path_settle));
    }

    /// Fire the timers due by `now`.
    pub fn tick(&mut self, now: u64) {
        if self.path_kick_at.is_some_and(|at| at <= now) {
            self.path_kick_at = None;
            self.path_kick(now);
        }
        if self.keepalive_at.is_some_and(|at| at <= now) {
            self.keepalive_at = Some(now + ms(self.timing.keepalive_interval));
            self.keepalive_tick(now);
        }
    }

    // MARK: Rules

    fn begin(&mut self, automatic: bool, now: u64) {
        self.generation += 1;
        self.phase = Phase::Dialing { since: now };
        self.automatic = automatic;
        self.on_person = false;
        self.probe = None;
        self.probe_timeouts = 0;
        self.push(Action::Dial {
            generation: self.generation,
            protocol: self.protocol,
            automatic,
        });
        self.publish();
    }

    /// A dial still out, restarted on a fresh socket: the same attempt
    /// continued, so it keeps who asked for it.
    fn redial(&mut self, now: u64) {
        self.push(Action::Abandon {
            generation: self.generation,
        });
        let automatic = self.automatic;
        self.begin(automatic, now);
    }

    /// The live transport is dead or doubted: tear it down for a
    /// replacement and dial one.
    fn reconnect(&mut self, now: u64) {
        if self.live().is_some() {
            self.push(Action::Teardown {
                generation: self.generation,
                why: Teardown::Reconnect,
            });
        }
        self.begin(true, now);
    }

    /// Nothing up: a dial out for `stalled_after` restarted, a down session
    /// dialled again. Whether it acted.
    fn retry_if_down(&mut self, stalled_after: Duration, now: u64) -> bool {
        match &self.phase {
            // Stalled on the person, not the network (B395).
            Phase::Dialing { .. } if self.on_person => false,
            Phase::Dialing { since } => {
                if now.saturating_sub(*since) >= ms(stalled_after) {
                    self.redial(now);
                    true
                } else {
                    false
                }
            }
            Phase::Down { failure } if failure.retryable => {
                self.begin(true, now);
                true
            }
            _ => false,
        }
    }

    fn force_resume(&mut self, now: u64) {
        match &self.phase {
            Phase::Live {
                transport: Transport::Mosh,
            } => self.push(Action::ResumeMosh {
                generation: self.generation,
                force: true,
            }),
            Phase::Live {
                transport: Transport::Ssh,
            } => self.reconnect(now),
            // A dial left from before the suspension was frozen with the
            // app: restart it. One younger than the grace began on the way
            // back (the thawed process read the dead socket's error before
            // the foreground arrived) and is on the network the phone has
            // now: restarting it cost a second credential read and a
            // half-made connection on the server.
            // Waiting on the person, not frozen with the app (B395).
            Phase::Dialing { .. } if self.on_person => {}
            Phase::Dialing { since }
                if now.saturating_sub(*since) >= ms(self.timing.stalled_dial_grace) =>
            {
                self.redial(now)
            }
            Phase::Dialing { .. } => {}
            Phase::Down { failure } if failure.retryable => self.begin(true, now),
            _ => {}
        }
    }

    fn resume_if_needed(&mut self, now: u64) {
        if self.retry_if_down(self.timing.stalled_dial_grace, now) {
            return;
        }
        match self.phase {
            Phase::Live {
                transport: Transport::Mosh,
            } => self.push(Action::ResumeMosh {
                generation: self.generation,
                force: false,
            }),
            Phase::Live {
                transport: Transport::Ssh,
            } if self.timing.resume_in_place => self.start_resume_probe(),
            Phase::Live {
                transport: Transport::Ssh,
            } => self.start_probe(ProbePurpose::Resume, self.timing.foreground_probe_timeout),
            _ => {}
        }
    }

    /// The in-place return's question. A keepalive probe still out from
    /// before the suspension doesn't stand in for it: its answer would come
    /// on its own clock, and input would not wait for it.
    fn start_resume_probe(&mut self) {
        let probe = self.next_probe(ProbePurpose::Resume);
        self.push(Action::ResumeProbe {
            generation: self.generation,
            probe,
            timeout: self.timing.resume_probe_timeout,
            away_ms: self.away,
        });
    }

    /// The probe of `purpose` that is out from now on: its number.
    fn next_probe(&mut self, purpose: ProbePurpose) -> u64 {
        self.probes += 1;
        self.probe = Some((purpose, self.probes));
        self.probes
    }

    fn path_kick(&mut self, now: u64) {
        if self.retry_if_down(self.timing.path_redial_after, now) {
            return;
        }
        match self.phase {
            Phase::Live {
                transport: Transport::Mosh,
            } => self.push(Action::RoamMosh {
                generation: self.generation,
            }),
            // Asked whatever arrived lately (B446): bytes from before the
            // change came over the path it left, and said nothing of this
            // one. A recent agent's output skipped the check, and a silence
            // counted one of two: a frozen terminal under a green pill for
            // two keepalive ticks, about 40 s. Asked as the return asks.
            Phase::Live {
                transport: Transport::Ssh,
            } => {
                // A keepalive's question is replaced (its answer would count
                // one of two); a return's, which reconnects on any doubt and
                // holds input meanwhile, stands.
                if matches!(self.probe, Some((ProbePurpose::Keepalive, _))) {
                    self.probe = None;
                }
                self.start_probe(ProbePurpose::Path, self.timing.foreground_probe_timeout);
            }
            _ => {}
        }
    }

    fn keepalive_tick(&mut self, now: u64) {
        match &self.phase {
            Phase::Live {
                transport: Transport::Mosh,
            } => self.push(Action::ResumeMosh {
                generation: self.generation,
                force: false,
            }),
            Phase::Live {
                transport: Transport::Ssh,
            } => self.keepalive_check(now),
            Phase::Down { failure } if failure.retryable => self.begin(true, now),
            _ => {}
        }
    }

    /// Probe the live SSH transport unless bytes proved it alive lately.
    fn keepalive_check(&mut self, now: u64) {
        if self.probe.is_some() {
            return;
        }
        if self
            .last_inbound
            .is_some_and(|at| now.saturating_sub(at) < ms(self.timing.inbound_proof))
        {
            self.probe_timeouts = 0;
            return;
        }
        self.start_probe(ProbePurpose::Keepalive, self.timing.probe_timeout);
    }

    fn start_probe(&mut self, purpose: ProbePurpose, timeout: Duration) {
        if self.probe.is_some() {
            return;
        }
        let probe = self.next_probe(purpose);
        self.push(Action::Probe {
            generation: self.generation,
            probe,
            timeout,
        });
    }

    fn push(&mut self, action: Action) {
        self.actions.push_back(action);
    }

    fn publish(&mut self) {
        let state = self.state();
        if self.published.as_ref() != Some(&state) {
            self.published = Some(state.clone());
            self.push(Action::State(state));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Cause, CredentialsWhy, FailureKind};

    const S: u64 = 1_000;
    const T0: u64 = 10_000;

    fn ssh_life() -> Lifecycle {
        Lifecycle::new(Timing::default(), Protocol::Ssh, true, T0)
    }

    /// The probe out answers (an answer with nothing out answers nothing).
    fn answer(life: &mut Lifecycle, generation: u64, result: Probe, now: u64) {
        let probe = life.probe.map_or(0, |(_, probe)| probe);
        life.probe_answered(generation, probe, result, now);
    }

    fn drain(life: &mut Lifecycle) -> Vec<Action> {
        std::iter::from_fn(|| life.poll_action()).collect()
    }

    fn dials(actions: &[Action]) -> Vec<(u64, bool)> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Dial {
                    generation,
                    automatic,
                    ..
                } => Some((*generation, *automatic)),
                _ => None,
            })
            .collect()
    }

    fn states(actions: &[Action]) -> Vec<SessionState> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::State(state) => Some(state.clone()),
                _ => None,
            })
            .collect()
    }

    fn unreachable() -> Failure {
        Failure::new(FailureKind::Unreachable, Cause::Plain, "no route to host")
    }

    /// Runs the clock to `until` in steps, ticking whatever is due.
    fn run_to(life: &mut Lifecycle, until: u64) -> Vec<Action> {
        let mut out = Vec::new();
        while let Some(at) = life.next_deadline() {
            if at > until {
                break;
            }
            life.tick(at);
            out.extend(drain(life));
        }
        out
    }

    fn live(life: &mut Lifecycle, transport: Transport, now: u64) -> u64 {
        life.connect(now);
        drain(life);
        let generation = life.generation();
        assert!(life.attempt_succeeded(generation, transport, now));
        drain(life);
        generation
    }

    #[test]
    fn a_connect_dials_and_a_success_is_connected() {
        let mut life = ssh_life();
        life.connect(T0);
        let actions = drain(&mut life);
        assert_eq!(
            actions,
            vec![
                Action::Dial {
                    generation: 1,
                    protocol: Protocol::Ssh,
                    automatic: false
                },
                Action::State(SessionState::Connecting),
            ]
        );
        assert!(life.attempt_succeeded(1, Transport::Ssh, T0 + S));
        assert_eq!(
            drain(&mut life),
            vec![Action::State(SessionState::Connected)]
        );
        // Connecting again while live is nothing.
        life.connect(T0 + 2 * S);
        assert!(drain(&mut life).is_empty());
    }

    #[test]
    fn a_superseded_dial_is_disowned_whatever_it_brings() {
        let mut life = ssh_life();
        life.connect(T0);
        drain(&mut life);
        life.background(T0 + 100);
        life.foreground(T0 + 3 * S);
        let actions = drain(&mut life);
        assert_eq!(actions[0], Action::Abandon { generation: 1 });
        assert_eq!(
            dials(&actions),
            vec![(2, false)],
            "the same attempt, still the person's"
        );
        // The first dial lands late either way: neither counts.
        assert!(!life.attempt_succeeded(1, Transport::Ssh, T0 + 4 * S));
        life.attempt_failed(1, unreachable(), T0 + 4 * S);
        assert!(drain(&mut life).is_empty());
        assert_eq!(life.state(), SessionState::Connecting);
        assert!(life.attempt_succeeded(2, Transport::Ssh, T0 + 5 * S));
        assert_eq!(life.state(), SessionState::Connected);
        // A late loss of the old generation is not this transport's.
        life.lost(1, T0 + 6 * S);
        assert!(
            drain(&mut life)
                .iter()
                .all(|a| !matches!(a, Action::Dial { .. }))
        );
    }

    #[test]
    fn a_young_dial_is_left_to_finish_on_the_way_back() {
        let mut life = ssh_life();
        life.connect(T0);
        drain(&mut life);
        life.background(T0 + 200);
        life.foreground(T0 + 1_500);
        assert!(
            dials(&drain(&mut life)).is_empty(),
            "1.5 s out: within the grace"
        );
        life.background(T0 + 1_600);
        life.foreground(T0 + 2_000);
        assert_eq!(
            dials(&drain(&mut life)),
            vec![(2, false)],
            "2 s out: restarted"
        );
    }

    #[test]
    fn a_failed_connect_is_failed_then_retried_on_the_way_back_as_a_reconnect() {
        let mut life = ssh_life();
        life.connect(T0);
        drain(&mut life);
        life.attempt_failed(1, unreachable(), T0 + 15 * S);
        assert_eq!(
            drain(&mut life),
            vec![Action::State(SessionState::Failed(unreachable()))]
        );
        life.background(T0 + 16 * S);
        life.foreground(T0 + 18 * S);
        let actions = drain(&mut life);
        assert_eq!(dials(&actions), vec![(2, true)]);
        assert_eq!(states(&actions), vec![SessionState::Reconnecting]);
        // A failure inside the automatic retry is not news: still
        // reconnecting, and retried on the next tick.
        life.attempt_failed(2, unreachable(), T0 + 19 * S);
        assert!(states(&drain(&mut life)).is_empty());
        assert_eq!(life.state(), SessionState::Reconnecting);
        let actions = run_to(&mut life, T0 + 18 * S + 12 * S);
        assert_eq!(dials(&actions), vec![(3, true)]);
        assert!(life.attempt_succeeded(3, Transport::Ssh, T0 + 31 * S));
        assert_eq!(life.state(), SessionState::Connected);
    }

    #[test]
    fn a_refused_host_key_or_credentials_are_never_retried_on_their_own() {
        for kind in [FailureKind::HostKey, FailureKind::Credentials] {
            let mut life = ssh_life();
            life.connect(T0);
            drain(&mut life);
            let failure = Failure::new(kind, Cause::Plain, "no");
            assert!(!failure.retryable);
            life.attempt_failed(1, failure.clone(), T0 + S);
            drain(&mut life);
            life.network_changed(T0 + 2 * S);
            life.background(T0 + 3 * S);
            life.foreground(T0 + 60 * S);
            let actions = run_to(&mut life, T0 + 600 * S);
            assert!(dials(&actions).is_empty(), "{kind:?}");
            assert_eq!(life.state(), SessionState::Failed(failure));
            // The person can.
            life.connect(T0 + 601 * S);
            assert_eq!(dials(&drain(&mut life)), vec![(2, false)]);
        }
    }

    #[test]
    fn a_locked_keychain_is_retried_on_the_unlock_and_the_way_back_not_wrong_credentials() {
        // B201: a launch before the first unlock can't read the keychain.
        let locked = Failure::credentials(CredentialsWhy::Locked, "locked");
        assert!(locked.retryable);
        assert_eq!(locked.kind, FailureKind::CredentialsLocked);
        assert!(locked.waits_for_unlock());
        let mut life = ssh_life();
        life.connect(T0);
        drain(&mut life);
        life.attempt_failed(1, locked.clone(), T0 + S);
        assert_eq!(
            drain(&mut life),
            vec![Action::State(SessionState::Failed(locked.clone()))]
        );
        // The unlock dials at once, as a reconnect.
        life.unlocked(T0 + 2 * S);
        let actions = drain(&mut life);
        assert_eq!(dials(&actions), vec![(2, true)]);
        assert_eq!(states(&actions), vec![SessionState::Reconnecting]);
        // Still locked (the unlock came before the item was readable): the
        // way back from the background tries again.
        life.attempt_failed(2, locked.clone(), T0 + 3 * S);
        life.background(T0 + 4 * S);
        life.foreground(T0 + 6 * S);
        assert_eq!(dials(&drain(&mut life)), vec![(3, true)]);
        assert!(life.attempt_succeeded(3, Transport::Ssh, T0 + 7 * S));
        assert_eq!(life.state(), SessionState::Connected);

        // Wrong or missing credentials stay failed whatever happens.
        for why in [
            CredentialsWhy::Rejected { user: "me".into() },
            CredentialsWhy::Unavailable,
            CredentialsWhy::Cancelled,
        ] {
            let failure = Failure::credentials(why, "no");
            assert!(!failure.retryable);
            assert!(!failure.waits_for_unlock());
            let mut life = ssh_life();
            life.connect(T0);
            drain(&mut life);
            life.attempt_failed(1, failure.clone(), T0 + S);
            drain(&mut life);
            life.unlocked(T0 + 2 * S);
            life.background(T0 + 3 * S);
            life.foreground(T0 + 60 * S);
            let mut actions = drain(&mut life);
            actions.extend(run_to(&mut life, T0 + 600 * S));
            assert!(dials(&actions).is_empty(), "{failure:?}");
            assert_eq!(life.state(), SessionState::Failed(failure));
        }
    }

    #[test]
    fn an_unlock_leaves_a_live_or_dialling_session_alone() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.unlocked(T0 + S);
        assert!(drain(&mut life).is_empty());
        life.lost(generation, T0 + 2 * S);
        drain(&mut life);
        life.unlocked(T0 + 3 * S);
        assert!(dials(&drain(&mut life)).is_empty(), "the redial is out");
    }

    #[test]
    fn an_automatic_attempt_refused_by_a_person_stays_failed() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.lost(generation, T0 + S);
        drain(&mut life);
        assert_eq!(life.state(), SessionState::Reconnecting);
        let refused = Failure::new(FailureKind::HostKey, Cause::Plain, "the key changed");
        life.attempt_failed(generation + 1, refused.clone(), T0 + 2 * S);
        assert_eq!(
            drain(&mut life),
            vec![Action::State(SessionState::Failed(refused))]
        );
        assert!(dials(&run_to(&mut life, T0 + 100 * S)).is_empty());
    }

    #[test]
    fn a_lost_transport_is_redialled_at_once_and_reads_reconnecting() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.lost(generation, T0 + 5 * S);
        assert_eq!(
            drain(&mut life),
            vec![
                Action::Teardown {
                    generation,
                    why: Teardown::Reconnect
                },
                Action::Dial {
                    generation: generation + 1,
                    protocol: Protocol::Ssh,
                    automatic: true
                },
                Action::State(SessionState::Reconnecting),
            ]
        );
        // The same loss reported twice (the reader and the watchdog): once.
        life.lost(generation, T0 + 5 * S);
        assert!(drain(&mut life).is_empty());
    }

    #[test]
    fn a_clean_end_ends_and_nothing_brings_it_back() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        let reason = EndReason::ShellExited { status: Some(0) };
        life.ended(generation, reason.clone(), T0 + S);
        assert_eq!(
            drain(&mut life),
            vec![
                Action::Teardown {
                    generation,
                    why: Teardown::Ended
                },
                Action::State(SessionState::Ended(reason.clone())),
            ]
        );
        // Neither a late loss, a keepalive, a return nor a network change.
        life.lost(generation, T0 + 2 * S);
        life.network_changed(T0 + 3 * S);
        life.background(T0 + 4 * S);
        life.foreground(T0 + 60 * S);
        let mut actions = drain(&mut life);
        actions.extend(run_to(&mut life, T0 + 600 * S));
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(life.state(), SessionState::Ended(reason));
        // The person may reconnect.
        life.connect(T0 + 601 * S);
        assert_eq!(dials(&drain(&mut life)).len(), 1);
    }

    #[test]
    fn mosh_ending_is_an_end_too() {
        let mut life = Lifecycle::new(Timing::default(), Protocol::Mosh, true, T0);
        let generation = live(&mut life, Transport::Mosh, T0);
        life.ended(generation, EndReason::MoshServerEnded, T0 + S);
        let actions = drain(&mut life);
        assert!(actions.contains(&Action::State(SessionState::Ended(
            EndReason::MoshServerEnded
        ))));
    }
    /// B610: a keepalive's question still out when the app went to the
    /// background answers nothing about the socket now (B394's rule): a
    /// delivery in the background asks its own and waits for it. It was
    /// skipped: the delivery went into a possibly dead socket and was said
    /// sent, and the stale question then counted one of two.
    #[test]
    fn a_delivery_replaces_a_keepalive_question_from_before_the_background() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        run_to(&mut life, T0 + 12 * S);
        assert!(matches!(life.probe, Some((ProbePurpose::Keepalive, 1))));
        life.background(T0 + 13 * S);
        let _ = drain(&mut life);
        life.check_for_delivery(T0 + 60 * S);
        assert!(life.delivery_check_pending(), "its own question");
        let asked = drain(&mut life);
        assert_eq!(
            asked,
            vec![Action::Probe {
                generation,
                probe: 2,
                timeout: Duration::from_secs(4)
            }]
        );
        // The old answer, late, is nobody's.
        life.probe_answered(generation, 1, Probe::TimedOut, T0 + 61 * S);
        assert!(life.delivery_check_pending(), "still the delivery's");
    }

    /// B524: a delivery asked for in the background asks a live SSH
    /// transport not heard from lately first, within the return's 4 s;
    /// deliveries wait for the answer; any doubt reconnects. Not in the
    /// foreground, not with bytes just in, not over mosh (its delivery
    /// waits for the server's ack anyway).
    #[test]
    fn a_delivery_in_the_background_asks_the_transport_first() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.check_for_delivery(T0 + 20 * S);
        assert!(!life.delivery_check_pending(), "the foreground");
        life.background(T0 + 20 * S);
        let _ = drain(&mut life);
        life.inbound(T0 + 25 * S);
        life.check_for_delivery(T0 + 30 * S);
        assert!(!life.delivery_check_pending(), "bytes just in");
        life.check_for_delivery(T0 + 60 * S);
        assert!(life.delivery_check_pending());
        let probe = match drain(&mut life).as_slice() {
            [Action::Probe { probe, timeout, .. }] => {
                assert_eq!(*timeout, Duration::from_secs(4));
                *probe
            }
            other => panic!("{other:?}"),
        };
        life.probe_answered(generation, probe, Probe::TimedOut, T0 + 64 * S);
        assert!(!life.delivery_check_pending());
        let after = drain(&mut life);
        assert!(
            after.contains(&Action::Teardown {
                generation,
                why: Teardown::Reconnect
            }),
            "{after:?}"
        );

        // Alive: the transport stays.
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.background(T0 + S);
        life.check_for_delivery(T0 + 60 * S);
        let probe = match drain(&mut life).last() {
            Some(Action::Probe { probe, .. }) => *probe,
            other => panic!("{other:?}"),
        };
        life.probe_answered(generation, probe, Probe::Alive, T0 + 61 * S);
        assert!(!life.delivery_check_pending());
        assert_eq!(drain(&mut life), vec![]);
        assert_eq!(life.state(), SessionState::Connected);

        // mosh: no question.
        let mut mosh = Lifecycle::new(Timing::default(), Protocol::Mosh, true, T0);
        live(&mut mosh, Transport::Mosh, T0);
        mosh.background(T0 + S);
        mosh.check_for_delivery(T0 + 60 * S);
        assert!(!mosh.delivery_check_pending());
    }

    /// The end of a mosh transport the session has since replaced is that
    /// transport's, not the session's (the Swift client's
    /// MoshSessionEndTests, kept when it went, E2): it changes nothing,
    /// while the redial is out or once it has landed.
    #[test]
    fn a_replaced_transports_end_is_not_the_sessions() {
        let mut life = Lifecycle::new(Timing::default(), Protocol::Mosh, true, T0);
        let old = live(&mut life, Transport::Mosh, T0);
        life.lost(old, T0 + S);
        let _ = drain(&mut life);
        let redial = life.generation();
        assert_ne!(redial, old);
        let state = life.state();
        life.ended(old, EndReason::MoshServerEnded, T0 + 2 * S);
        assert_eq!(drain(&mut life), vec![], "nothing while the redial is out");
        assert_eq!(life.state(), state);
        assert!(life.attempt_succeeded(redial, Transport::Mosh, T0 + 3 * S));
        let _ = drain(&mut life);
        life.ended(old, EndReason::MoshServerEnded, T0 + 4 * S);
        assert_eq!(drain(&mut life), vec![], "nothing once it landed");
        assert_eq!(life.state(), SessionState::Connected);
        // The current one's end is the session's.
        life.ended(redial, EndReason::MoshServerEnded, T0 + 5 * S);
        assert!(
            drain(&mut life).contains(&Action::State(SessionState::Ended(
                EndReason::MoshServerEnded
            )))
        );
    }

    #[test]
    fn keepalive_trusts_recent_bytes_and_drops_on_the_second_silence() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        // Bytes 5 s before the first tick: no probe.
        life.inbound(T0 + 7 * S);
        let actions = run_to(&mut life, T0 + 12 * S);
        assert!(!actions.iter().any(|a| matches!(a, Action::Probe { .. })));
        // Quiet since: the next tick probes, with the 8 s deadline.
        let actions = run_to(&mut life, T0 + 24 * S);
        assert_eq!(
            actions,
            vec![Action::Probe {
                generation,
                probe: 1,
                timeout: Duration::from_secs(8)
            }]
        );
        answer(&mut life, generation, Probe::TimedOut, T0 + 32 * S);
        assert!(drain(&mut life).is_empty(), "one silence holds");
        let actions = run_to(&mut life, T0 + 36 * S);
        assert_eq!(actions.len(), 1);
        answer(&mut life, generation, Probe::TimedOut, T0 + 44 * S);
        let actions = drain(&mut life);
        assert_eq!(
            dials(&actions),
            vec![(generation + 1, true)],
            "two in a row drop"
        );
    }

    #[test]
    fn a_probe_that_errors_is_a_drop_and_an_answer_resets_the_count() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        run_to(&mut life, T0 + 12 * S);
        answer(&mut life, generation, Probe::TimedOut, T0 + 20 * S);
        run_to(&mut life, T0 + 24 * S);
        answer(&mut life, generation, Probe::Alive, T0 + 26 * S);
        // Alive counts as bytes: the next tick (10 s later) doesn't probe.
        let actions = run_to(&mut life, T0 + 36 * S);
        assert!(!actions.iter().any(|a| matches!(a, Action::Probe { .. })));
        run_to(&mut life, T0 + 48 * S);
        answer(&mut life, generation, Probe::TimedOut, T0 + 50 * S);
        assert!(drain(&mut life).is_empty(), "the count started over");
        run_to(&mut life, T0 + 60 * S);
        answer(&mut life, generation, Probe::Failed, T0 + 61 * S);
        assert_eq!(dials(&drain(&mut life)), vec![(generation + 1, true)]);
    }

    #[test]
    fn a_late_probe_answer_for_an_old_transport_is_ignored() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        run_to(&mut life, T0 + 12 * S);
        life.lost(generation, T0 + 13 * S);
        drain(&mut life);
        answer(&mut life, generation, Probe::Failed, T0 + 14 * S);
        assert!(drain(&mut life).is_empty());
    }

    #[test]
    fn nothing_ticks_in_the_background() {
        let mut life = ssh_life();
        live(&mut life, Transport::Ssh, T0);
        life.background(T0 + S);
        assert_eq!(life.next_deadline(), None);
        life.network_changed(T0 + 2 * S);
        assert_eq!(
            life.next_deadline(),
            None,
            "path changes wait for the return"
        );
    }

    #[test]
    fn a_long_absence_reconnects_ssh_without_asking_and_replaces_the_mosh_flow() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.background(T0 + S);
        drain(&mut life);
        life.foreground(T0 + 22 * S);
        let actions = drain(&mut life);
        assert_eq!(
            actions[0],
            Action::Teardown {
                generation,
                why: Teardown::Reconnect
            }
        );
        assert_eq!(dials(&actions), vec![(generation + 1, true)]);
        assert!(!actions.iter().any(|a| matches!(a, Action::Probe { .. })));

        let mut life = Lifecycle::new(Timing::default(), Protocol::Mosh, true, T0);
        let generation = live(&mut life, Transport::Mosh, T0);
        life.background(T0 + S);
        drain(&mut life);
        life.foreground(T0 + 22 * S);
        assert_eq!(
            drain(&mut life),
            vec![Action::ResumeMosh {
                generation,
                force: true
            }]
        );
    }

    #[test]
    fn a_short_absence_probes_once_with_the_short_deadline() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.background(T0 + S);
        drain(&mut life);
        life.foreground(T0 + 6 * S);
        assert_eq!(
            drain(&mut life),
            vec![
                Action::Foreground { generation },
                Action::Probe {
                    generation,
                    probe: 1,
                    timeout: Duration::from_secs(4)
                }
            ]
        );
        // One silence is enough on the way back.
        answer(&mut life, generation, Probe::TimedOut, T0 + 10 * S);
        assert_eq!(dials(&drain(&mut life)), vec![(generation + 1, true)]);
    }

    fn in_place(protocol: Protocol) -> Lifecycle {
        let timing = Timing {
            resume_in_place: true,
            ..Timing::default()
        };
        Lifecycle::new(timing, protocol, true, T0)
    }

    /// In place, a long absence asks the live SSH transport first — on the
    /// leg, with the short deadline — instead of tearing it down; an answer
    /// keeps it: no state change, no dial, input free again.
    #[test]
    fn in_place_a_long_absence_asks_first_and_an_answer_keeps_the_connection() {
        for away in [6 * S, 22 * S, 300 * S] {
            let mut life = in_place(Protocol::Ssh);
            let generation = live(&mut life, Transport::Ssh, T0);
            life.background(T0 + S);
            drain(&mut life);
            assert!(!life.resume_pending());
            life.foreground(T0 + S + away);
            assert_eq!(
                drain(&mut life),
                vec![
                    Action::Foreground { generation },
                    Action::ResumeProbe {
                        generation,
                        probe: 1,
                        timeout: Duration::from_millis(1_500),
                        away_ms: away,
                    }
                ],
                "away {away} ms"
            );
            assert!(life.resume_pending(), "input waits for the answer");
            assert_eq!(life.state(), SessionState::Connected, "no veil");
            answer(&mut life, generation, Probe::Alive, T0 + S + away + 80);
            assert!(drain(&mut life).is_empty(), "kept: nothing to do");
            assert!(!life.resume_pending());
            assert_eq!(life.live(), Some(Transport::Ssh));
            assert_eq!(life.generation(), generation);
        }
    }

    /// No answer by the deadline, or a failure: reconnect at once (the
    /// in-place return never keeps a doubted socket), and input no longer
    /// waits on it.
    #[test]
    fn in_place_a_silence_or_a_failure_reconnects() {
        for result in [Probe::TimedOut, Probe::Failed] {
            let mut life = in_place(Protocol::Ssh);
            let generation = live(&mut life, Transport::Ssh, T0);
            life.background(T0 + S);
            drain(&mut life);
            life.foreground(T0 + 60 * S);
            drain(&mut life);
            answer(&mut life, generation, result, T0 + 61 * S);
            let actions = drain(&mut life);
            assert_eq!(
                actions[0],
                Action::Teardown {
                    generation,
                    why: Teardown::Reconnect
                }
            );
            assert_eq!(dials(&actions), vec![(generation + 1, true)]);
            assert_eq!(states(&actions), vec![SessionState::Reconnecting]);
            assert!(!life.resume_pending());
        }
    }

    /// The connection that was live when the app left died while it was
    /// away, heard before the return: nothing to ask, but the return still
    /// owes the diagnostics one outcome, once. With the switch off, none.
    #[test]
    fn in_place_a_connection_lost_while_away_still_reports_the_return() {
        let lost_reports = |actions: &[Action]| {
            actions
                .iter()
                .filter(|a| matches!(a, Action::ResumeLost { .. }))
                .cloned()
                .collect::<Vec<_>>()
        };
        let mut life = in_place(Protocol::Ssh);
        let generation = live(&mut life, Transport::Ssh, T0);
        life.background(T0 + S);
        drain(&mut life);
        life.lost(generation, T0 + 26 * S);
        drain(&mut life);
        life.foreground(T0 + 26 * S);
        let actions = drain(&mut life);
        assert_eq!(
            lost_reports(&actions),
            vec![Action::ResumeLost { away_ms: 25 * S }],
            "{actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::ResumeProbe { .. })),
            "nothing to ask: {actions:?}"
        );
        assert!(!life.resume_pending());
        life.foreground(T0 + 27 * S);
        assert!(lost_reports(&drain(&mut life)).is_empty(), "once");

        // A connection that survived is asked instead: one outcome, the
        // question's.
        let mut life = in_place(Protocol::Ssh);
        live(&mut life, Transport::Ssh, T0);
        life.background(T0 + S);
        drain(&mut life);
        life.foreground(T0 + 26 * S);
        assert!(lost_reports(&drain(&mut life)).is_empty());

        let mut off = Lifecycle::new(Timing::default(), Protocol::Ssh, true, T0);
        let generation = live(&mut off, Transport::Ssh, T0);
        off.background(T0 + S);
        drain(&mut off);
        off.lost(generation, T0 + 26 * S);
        drain(&mut off);
        off.foreground(T0 + 26 * S);
        assert!(lost_reports(&drain(&mut off)).is_empty(), "switch off");
    }

    /// A keepalive probe still out from before the suspension doesn't stand
    /// in for the return's question, nor does its answer settle it (B394):
    /// its round trip says nothing of the socket after the suspension.
    #[test]
    fn in_place_a_keepalive_probe_out_before_the_suspension_is_asked_again() {
        let mut life = in_place(Protocol::Ssh);
        let generation = live(&mut life, Transport::Ssh, T0);
        // Silent past the inbound proof: the 12 s tick probes.
        let actions = run_to(&mut life, T0 + 12 * S);
        assert!(
            actions.iter().any(|a| matches!(a, Action::Probe { .. })),
            "{actions:?}"
        );
        life.background(T0 + 13 * S);
        drain(&mut life);
        life.foreground(T0 + 40 * S);
        let actions = drain(&mut life);
        assert!(
            actions.contains(&Action::ResumeProbe {
                generation,
                probe: 2,
                timeout: Duration::from_millis(1_500),
                away_ms: 27 * S,
            }),
            "{actions:?}"
        );
        assert!(life.resume_pending());
        // The keepalive's answer, from before the suspension: nothing.
        life.probe_answered(generation, 1, Probe::Alive, T0 + 40 * S + 50);
        assert!(
            life.resume_pending(),
            "the return's own question is still out"
        );
        // Its own answer settles it; a later one finds nothing to settle.
        life.probe_answered(generation, 2, Probe::Alive, T0 + 40 * S + 80);
        assert!(!life.resume_pending());
        life.probe_answered(generation, 2, Probe::TimedOut, T0 + 41 * S);
        assert!(drain(&mut life).is_empty());
        assert_eq!(life.live(), Some(Transport::Ssh));
    }

    /// B394: the app left again before the return's question was answered
    /// (two quick trips away). The next return asks its own question —
    /// input still waiting — and the old one's answer, a round trip from
    /// before the second suspension, settles nothing.
    #[test]
    fn in_place_a_question_left_unanswered_is_asked_again_on_the_next_return() {
        let mut life = in_place(Protocol::Ssh);
        let generation = live(&mut life, Transport::Ssh, T0);
        life.background(T0 + S);
        drain(&mut life);
        life.foreground(T0 + 60 * S);
        drain(&mut life);
        life.background(T0 + 60 * S + 200);
        drain(&mut life);
        assert!(life.resume_pending(), "input still waits while away");
        life.foreground(T0 + 600 * S);
        let actions = drain(&mut life);
        assert!(
            actions.contains(&Action::ResumeProbe {
                generation,
                probe: 2,
                timeout: Duration::from_millis(1_500),
                away_ms: 600 * S - 60 * S - 200,
            }),
            "this return asks its own question: {actions:?}"
        );
        life.probe_answered(generation, 1, Probe::Alive, T0 + 600 * S + 5);
        assert!(life.resume_pending(), "the old answer settles nothing");
        life.probe_answered(generation, 2, Probe::TimedOut, T0 + 602 * S);
        assert_eq!(dials(&drain(&mut life)), vec![(generation + 1, true)]);
    }

    /// Off, too: a short return probes anew, and a keepalive's answer from
    /// before the suspension doesn't stand in for its own (B394).
    #[test]
    fn a_short_return_answers_with_its_own_probe_not_one_from_before() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        let actions = run_to(&mut life, T0 + 12 * S);
        assert!(actions.iter().any(|a| matches!(a, Action::Probe { .. })));
        life.background(T0 + 13 * S);
        drain(&mut life);
        life.foreground(T0 + 18 * S);
        let actions = drain(&mut life);
        assert!(
            actions.contains(&Action::Probe {
                generation,
                probe: 2,
                timeout: Duration::from_secs(4)
            }),
            "{actions:?}"
        );
        life.probe_answered(generation, 1, Probe::Failed, T0 + 18 * S + 5);
        assert!(
            drain(&mut life).is_empty(),
            "the old probe's error says nothing now"
        );
        life.probe_answered(generation, 2, Probe::Alive, T0 + 18 * S + 40);
        assert_eq!(life.live(), Some(Transport::Ssh));
    }

    /// The switch touches only a live SSH transport: mosh is replaced and
    /// repainted as before, a dial or a down session handled as before, and
    /// off it is the app's rule exactly.
    #[test]
    fn in_place_leaves_mosh_dials_and_down_sessions_alone() {
        let mut life = in_place(Protocol::Mosh);
        let generation = live(&mut life, Transport::Mosh, T0);
        life.background(T0 + S);
        drain(&mut life);
        life.foreground(T0 + 22 * S);
        assert_eq!(
            drain(&mut life),
            vec![Action::ResumeMosh {
                generation,
                force: true
            }]
        );
        assert!(!life.resume_pending());

        let mut life = in_place(Protocol::Ssh);
        let generation = live(&mut life, Transport::Ssh, T0);
        life.lost(generation, T0 + S);
        drain(&mut life);
        life.attempt_failed(generation + 1, unreachable(), T0 + 2 * S);
        life.background(T0 + 3 * S);
        drain(&mut life);
        life.foreground(T0 + 30 * S);
        assert_eq!(dials(&drain(&mut life)), vec![(generation + 2, true)]);
        assert!(!life.resume_pending());
    }

    /// Back after a long absence, the connection already found dead: iOS
    /// closed the socket while the app was suspended, and the thawed
    /// process reads the error before the foreground arrives. The redial
    /// that starts then is on the network the phone has now; the return
    /// leaves it to finish instead of restarting it (a second credential
    /// read, a half-made connection on the server). Whatever the switch.
    #[test]
    fn a_dial_begun_on_the_return_is_not_restarted_by_it() {
        for resume_in_place in [false, true] {
            let timing = Timing {
                resume_in_place,
                ..Timing::default()
            };
            let mut life = Lifecycle::new(timing, Protocol::Ssh, true, T0);
            let generation = live(&mut life, Transport::Ssh, T0);
            life.background(T0 + S);
            drain(&mut life);
            life.lost(generation, T0 + 61 * S);
            assert_eq!(dials(&drain(&mut life)), vec![(generation + 1, true)]);
            life.foreground(T0 + 61 * S + 30);
            let actions = drain(&mut life);
            assert!(
                !actions
                    .iter()
                    .any(|a| matches!(a, Action::Dial { .. } | Action::Abandon { .. })),
                "in place {resume_in_place}: {actions:?}"
            );
            assert_eq!(life.state(), SessionState::Reconnecting);
            assert!(life.attempt_succeeded(generation + 1, Transport::Ssh, T0 + 62 * S));
        }
    }

    /// B365: resume in place switched while the app runs — the switch is
    /// read again around every move — is followed at the next return, both
    /// ways.
    #[test]
    fn the_switch_changed_while_the_app_runs_is_followed_at_the_next_return() {
        // Turned off: a long absence redials; nothing is asked in place.
        let mut life = in_place(Protocol::Ssh);
        live(&mut life, Transport::Ssh, T0);
        life.set_resume_in_place(false);
        life.background(T0 + S);
        drain(&mut life);
        life.foreground(T0 + 31 * S);
        let actions = drain(&mut life);
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::ResumeProbe { .. })),
            "{actions:?}"
        );
        assert!(!life.resume_pending());
        // Turned on in an app started without it: the return asks in place.
        let mut life = ssh_life();
        live(&mut life, Transport::Ssh, T0);
        life.set_resume_in_place(true);
        life.background(T0 + S);
        drain(&mut life);
        life.foreground(T0 + 31 * S);
        let actions = drain(&mut life);
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::ResumeProbe { .. })),
            "{actions:?}"
        );
        assert!(life.resume_pending());
    }

    /// B395: a dial waiting on the person — the passphrase card they went
    /// to another app to answer, a host-key question — is left alone by a
    /// return and by a network change; once it no longer waits, a stalled
    /// one is restarted as before. A word for an older dial counts for
    /// nothing.
    #[test]
    fn a_dial_waiting_on_the_person_is_left_alone_by_a_return() {
        let mut life = ssh_life();
        life.connect(T0);
        drain(&mut life);
        let generation = life.generation();
        life.attempt_waits_on_person(generation, true);
        // Away 10 s, and away 30 s: neither restarts it.
        for (left, back) in [(T0 + 30 * S, T0 + 40 * S), (T0 + 50 * S, T0 + 80 * S)] {
            life.background(left);
            drain(&mut life);
            life.foreground(back);
            let actions = drain(&mut life);
            assert!(
                !actions
                    .iter()
                    .any(|a| matches!(a, Action::Abandon { .. } | Action::Dial { .. })),
                "{actions:?}"
            );
        }
        life.network_changed(T0 + 81 * S);
        let actions = run_to(&mut life, T0 + 83 * S);
        assert!(
            !actions.iter().any(|a| matches!(a, Action::Abandon { .. })),
            "{actions:?}"
        );
        assert_eq!(life.generation(), generation);
        // Answered: a return restarts a stalled dial again.
        life.attempt_waits_on_person(generation, false);
        life.background(T0 + 90 * S);
        drain(&mut life);
        life.foreground(T0 + 95 * S);
        assert_eq!(dials(&drain(&mut life)), vec![(generation + 1, false)]);
        // A word for that older dial now counts for nothing.
        life.attempt_waits_on_person(generation, true);
        life.background(T0 + 100 * S);
        drain(&mut life);
        life.foreground(T0 + 105 * S);
        assert_eq!(dials(&drain(&mut life)), vec![(generation + 2, false)]);
    }

    /// A dial left from before the suspension was frozen with the app: the
    /// return after a long absence starts it again on a fresh socket.
    #[test]
    fn a_dial_from_before_the_suspension_is_restarted_by_the_return() {
        let mut life = ssh_life();
        life.connect(T0);
        drain(&mut life);
        life.background(T0 + 500);
        drain(&mut life);
        life.foreground(T0 + 60 * S);
        let actions = drain(&mut life);
        assert_eq!(actions[0], Action::Abandon { generation: 1 });
        assert_eq!(dials(&actions), vec![(2, false)]);
    }

    #[test]
    fn the_way_back_redials_a_down_session_at_once() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.lost(generation, T0 + S);
        drain(&mut life);
        life.attempt_failed(generation + 1, unreachable(), T0 + 2 * S);
        life.background(T0 + 3 * S);
        drain(&mut life);
        life.foreground(T0 + 5 * S);
        assert_eq!(dials(&drain(&mut life)), vec![(generation + 2, true)]);
    }

    #[test]
    fn a_network_change_settles_then_kicks_dials_down_sessions_and_live_ones() {
        // A burst is one kick, 400 ms after its last change.
        let mut life = ssh_life();
        life.connect(T0);
        drain(&mut life);
        life.network_changed(T0 + 1_000);
        life.network_changed(T0 + 1_200);
        life.network_changed(T0 + 1_300);
        assert_eq!(life.next_deadline(), Some(T0 + 1_700));
        let actions = run_to(&mut life, T0 + 1_700);
        assert_eq!(dials(&actions), vec![(2, false)], "1.7 s out: restarted");

        // A dial younger than 1 s is left alone.
        let mut life = ssh_life();
        life.connect(T0);
        drain(&mut life);
        life.network_changed(T0 + 100);
        assert!(dials(&run_to(&mut life, T0 + 600)).is_empty());

        // A down session dials again.
        let mut life = ssh_life();
        life.connect(T0);
        drain(&mut life);
        life.attempt_failed(1, unreachable(), T0 + S);
        life.network_changed(T0 + 2 * S);
        assert_eq!(dials(&run_to(&mut life, T0 + 3 * S)), vec![(2, true)]);

        // A live SSH transport is asked, however fresh its last bytes, with
        // the return's deadline (B446).
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.network_changed(T0 + 2 * S);
        assert_eq!(
            run_to(&mut life, T0 + 3 * S),
            vec![Action::Probe {
                generation,
                probe: 1,
                timeout: Duration::from_secs(4)
            }]
        );

        // A live mosh flow roams.
        let mut life = Lifecycle::new(Timing::default(), Protocol::Mosh, true, T0);
        let generation = live(&mut life, Transport::Mosh, T0);
        life.network_changed(T0 + S);
        assert_eq!(
            run_to(&mut life, T0 + 2 * S),
            vec![Action::RoamMosh { generation }]
        );
    }

    /// B446: bytes that arrived just before a network change came over the
    /// path it left. The check skipped the probe on them, and a silence
    /// counted one of two: a frozen terminal under a green pill for two
    /// keepalive ticks. The change asks at once, and a silence reconnects.
    #[test]
    fn a_network_change_asks_a_live_ssh_transport_whatever_arrived_before_it() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.inbound(T0 + 5 * S);
        life.network_changed(T0 + 6 * S);
        let asked = run_to(&mut life, T0 + 7 * S);
        assert_eq!(
            asked,
            vec![Action::Probe {
                generation,
                probe: 1,
                timeout: Duration::from_secs(4)
            }],
            "bytes from before the change skipped the check"
        );
        answer(&mut life, generation, Probe::TimedOut, T0 + 10 * S + 500);
        let after = drain(&mut life);
        assert!(
            after.contains(&Action::Teardown { generation, why: Teardown::Reconnect }),
            "one silence after a change held the session: {after:?}"
        );
        assert_eq!(dials(&after), vec![(generation + 1, true)]);

        // An answer keeps it.
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.network_changed(T0 + S);
        run_to(&mut life, T0 + 2 * S);
        answer(&mut life, generation, Probe::Alive, T0 + 2 * S);
        assert!(drain(&mut life).is_empty());

        // A keepalive's question out at the change is replaced.
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        run_to(&mut life, T0 + 12 * S);
        assert!(matches!(life.probe, Some((ProbePurpose::Keepalive, 1))));
        life.network_changed(T0 + 13 * S);
        let asked = run_to(&mut life, T0 + 14 * S);
        assert_eq!(
            asked,
            vec![Action::Probe { generation, probe: 2, timeout: Duration::from_secs(4) }]
        );
    }

    /// B447: a network change heard while the app was away was dropped (the
    /// return "resumes anyway"), and a short return only nudges a mosh flow,
    /// whose socket is bound to the path it left on. The return roams it.
    #[test]
    fn a_network_change_while_away_roams_mosh_at_a_short_return() {
        let mut life = Lifecycle::new(Timing::default(), Protocol::Mosh, true, T0);
        let generation = live(&mut life, Transport::Mosh, T0);
        life.background(T0 + S);
        drain(&mut life);
        life.network_changed(T0 + 3 * S);
        assert!(drain(&mut life).is_empty(), "acted on in the background");
        life.foreground(T0 + 6 * S);
        let back = drain(&mut life);
        assert!(back.contains(&Action::RoamMosh { generation }), "{back:?}");

        // Without a change, a short return only nudges.
        life.background(T0 + 10 * S);
        drain(&mut life);
        life.foreground(T0 + 15 * S);
        let back = drain(&mut life);
        assert!(!back.contains(&Action::RoamMosh { generation }), "{back:?}");
        assert!(back.contains(&Action::ResumeMosh { generation, force: false }), "{back:?}");

        // A change still settling when the app left counts too.
        life.network_changed(T0 + 20 * S);
        life.background(T0 + 20 * S + 100);
        drain(&mut life);
        life.foreground(T0 + 25 * S);
        assert!(drain(&mut life).contains(&Action::RoamMosh { generation }));

        // SSH: the return asks the transport, as it always does.
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.background(T0 + S);
        life.network_changed(T0 + 2 * S);
        life.foreground(T0 + 5 * S);
        assert!(drain(&mut life).iter().any(|a| matches!(a, Action::Probe { generation: g, .. } if *g == generation)));
    }

    #[test]
    fn disconnect_abandons_a_dial_and_late_results_are_closed() {
        let mut life = ssh_life();
        life.connect(T0);
        drain(&mut life);
        life.disconnect(T0 + S);
        assert_eq!(
            drain(&mut life),
            vec![
                Action::Abandon { generation: 1 },
                Action::State(SessionState::Ended(EndReason::Disconnected)),
            ]
        );
        assert!(!life.attempt_succeeded(1, Transport::Ssh, T0 + 2 * S));
        assert!(run_to(&mut life, T0 + 100 * S).is_empty());
    }

    #[test]
    fn disconnect_tears_a_live_transport_down_for_good() {
        let mut life = ssh_life();
        let generation = live(&mut life, Transport::Ssh, T0);
        life.disconnect(T0 + S);
        assert_eq!(
            drain(&mut life)[0],
            Action::Teardown {
                generation,
                why: Teardown::Disconnect
            }
        );
        life.lost(generation, T0 + 2 * S);
        assert!(drain(&mut life).is_empty());
    }

    #[test]
    fn switching_a_mosh_session_to_ssh_dials_ssh_for_the_person() {
        let mut life = Lifecycle::new(Timing::default(), Protocol::Mosh, true, T0);
        let generation = live(&mut life, Transport::Mosh, T0);
        life.switch_to_ssh(T0 + 9 * S);
        assert_eq!(
            drain(&mut life),
            vec![
                Action::Teardown {
                    generation,
                    why: Teardown::SwitchToSsh
                },
                Action::Dial {
                    generation: generation + 1,
                    protocol: Protocol::Ssh,
                    automatic: false
                },
                Action::State(SessionState::Connecting),
            ]
        );
        assert_eq!(life.protocol(), Protocol::Ssh);
        life.switch_to_ssh(T0 + 10 * S);
        assert!(drain(&mut life).is_empty(), "once");
    }

    #[test]
    fn mosh_is_nudged_by_the_keepalive_never_probed() {
        let mut life = Lifecycle::new(Timing::default(), Protocol::Mosh, true, T0);
        let generation = live(&mut life, Transport::Mosh, T0);
        let actions = run_to(&mut life, T0 + 25 * S);
        assert_eq!(
            actions,
            vec![
                Action::ResumeMosh {
                    generation,
                    force: false
                };
                2
            ]
        );
    }

    #[test]
    fn a_session_created_in_the_background_ticks_only_once_it_is_back() {
        let mut life = Lifecycle::new(Timing::default(), Protocol::Ssh, false, T0);
        assert_eq!(life.next_deadline(), None);
        life.connect(T0);
        drain(&mut life);
        life.foreground(T0 + S);
        assert_eq!(life.next_deadline(), Some(T0 + 13 * S));
    }
}
