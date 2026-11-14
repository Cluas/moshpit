//! The push channel: herdr's event subscription over a second connection
//! whose PTY runs a python bridge to the host's socket
//! (`HerdrPushDriver.swift`, `HerdrSocketClient.swift` and SessionHub's
//! `connectHerdrPush`). Events only say "something changed"; the answer is a
//! read now, debounced. The poll keeps running underneath as the safety net.
//!
//! herdr 0.9.5 (B118, recorded in `tests/fixtures/recorded/socket.jsonl`):
//! the subscribe is answered (`subscription_started`) but no longer replays
//! the current state as events, and after it the connection answers no
//! requests at all. So the subscribe is the only request this channel ever
//! carries, and the read that follows it is what brings the tree up to date.

use super::timers::Slot;
use super::{Client, DEFAULT_SOCKET_PATH, Event, Output};
use crate::socket::{self, InvalidateDebounce, SocketClient};
use std::time::Duration;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Off,
    /// The host is opening the channel, the pump its program.
    Opening,
    /// The channel is open; waiting for the pump to hold the socket.
    Booting,
    /// The subscribe is out.
    Subscribing,
    Active,
    /// Gone, or never came up.
    Dead,
}

#[derive(Debug, Default)]
pub(super) struct Push {
    state: State,
    socket: SocketClient,
    debounce: InvalidateDebounce,
    subscribe: Option<String>,
    /// Was active once on this attempt: a death after that is worth one
    /// more try (a server restart closes the socket under it).
    was_active: bool,
    /// Was active, then died with its server, and the try after found none
    /// yet: opened again once a read finds the server answering (B422).
    revive: bool,
    /// Lost with its connection: opened again on the next one.
    reopen: bool,
}

impl Push {
    pub fn active(&self) -> bool {
        self.state == State::Active
    }
}

/// The pump for `socket_path`, the push channel's program: the app's pump
/// for the default socket, the same script pointed elsewhere for another
/// session's. `None` for a path the line can't carry (it rides in a python
/// string inside shell single quotes).
///
/// The program of the channel ([`Output::OpenPush`]), never a line typed
/// into the login shell it opened (B267): a shell still starting swallowed
/// it — a startup that flushes pending input (Powerlevel10k's instant
/// prompt; a private sshd with a 3 s zsh rc that flushes: push never came
/// up, over SSH and over mosh) — and push stayed off for the session.
pub fn pump_boot_line_for(socket_path: &str) -> Option<String> {
    let line = socket::pump_boot_line();
    if socket_path == DEFAULT_SOCKET_PATH {
        return Some(line);
    }
    if socket_path.is_empty()
        || socket_path
            .chars()
            .any(|c| matches!(c, '\'' | '"' | '\\') || c.is_control())
    {
        return None;
    }
    Some(line.replace(DEFAULT_SOCKET_PATH, socket_path))
}

impl Client {
    pub(super) fn start_push(&mut self) {
        if !self.config.push || self.closed {
            return;
        }
        if matches!(self.push.state, State::Off | State::Dead) {
            let Some(program) = pump_boot_line_for(&self.config.socket_path) else {
                // A socket the pump can't be pointed at: the poll it is.
                self.push.state = State::Dead;
                return;
            };
            let revive = self.push.revive;
            self.push = Push {
                state: State::Opening,
                socket: SocketClient::new(),
                revive,
                ..Push::default()
            };
            self.outputs.push_back(Output::OpenPush { program });
            // The host's dial is bounded by the pump's own wait too: a channel
            // that never opens is given up like a pump that never answers.
            self.arm(Slot::PumpReady, self.config.budgets.pump_ready);
        }
    }

    /// The push channel is up, the pump its program: nothing is written
    /// until the pump says it holds the socket.
    pub fn push_opened(&mut self, now: u64) {
        self.begin(now);
        if self.push.state == State::Opening {
            self.push.state = State::Booting;
            self.arm(Slot::PumpReady, self.config.budgets.pump_ready);
        }
        self.end();
    }

    /// The push channel couldn't be opened.
    pub fn push_failed(&mut self, now: u64) {
        self.begin(now);
        if self.push.state == State::Opening {
            self.push.state = State::Dead;
        }
        self.end();
    }

    /// Bytes off the push channel.
    pub fn push_bytes(&mut self, bytes: &[u8], now: u64) {
        self.begin(now);
        if matches!(
            self.push.state,
            State::Booting | State::Subscribing | State::Active
        ) {
            let outputs = self.push.socket.feed(bytes);
            for output in outputs {
                self.push_output(output);
            }
        }
        self.end();
    }

    fn push_output(&mut self, output: socket::Output) {
        match output {
            socket::Output::PumpReady => {
                if self.push.state != State::Booting {
                    return;
                }
                self.timers.cancel(&Slot::PumpReady);
                match self.push.socket.subscribe(&socket::GLOBAL_KINDS, &[]) {
                    Ok((id, line)) => {
                        self.push.subscribe = Some(id);
                        self.push.state = State::Subscribing;
                        self.outputs.push_back(Output::PushWrite(line.into_bytes()));
                        self.arm(Slot::Subscribe, self.config.budgets.subscribe);
                    }
                    Err(_) => self.push_give_up(),
                }
            }
            socket::Output::Reply { id, result } => {
                if self.push.subscribe.as_deref() != Some(id.as_str()) {
                    return;
                }
                self.timers.cancel(&Slot::Subscribe);
                self.push.subscribe = None;
                match result {
                    Ok(_) => {
                        self.push.state = State::Active;
                        self.push.was_active = true;
                        self.push.revive = false;
                        self.poll.set_push_active(true);
                        self.dirty = true;
                        self.event(Event::PushActive(true));
                        // herdr 0.9.5 replays nothing on subscribe: whatever
                        // changed since the last read is only in a read now.
                        self.quicken();
                    }
                    Err(_) => self.push_give_up(),
                }
            }
            socket::Output::Event(_) => {
                if self.push.state != State::Active {
                    return;
                }
                let now = Duration::from_millis(self.now);
                if self.push.debounce.event(now) {
                    self.timers.cancel(&Slot::Debounce);
                    self.quicken();
                } else if let Some(deadline) = self.push.debounce.deadline() {
                    self.timers.arm(Slot::Debounce, deadline.as_millis() as u64);
                }
            }
        }
    }

    pub(super) fn debounce_fired(&mut self) {
        if self.push.debounce.tick(Duration::from_millis(self.now)) {
            self.quicken();
        }
    }

    pub(super) fn pump_ready_timed_out(&mut self) {
        if matches!(self.push.state, State::Opening | State::Booting) {
            // No connection, or no python3 to run the pump: push isn't
            // coming.
            self.push_give_up();
        }
    }

    pub(super) fn subscribe_timed_out(&mut self) {
        if let Some(id) = self.push.subscribe.take() {
            self.push.socket.expire(&id);
        }
        if self.push.state == State::Subscribing {
            self.push_give_up();
        }
    }

    /// The push channel ended.
    pub fn push_closed(&mut self, now: u64) {
        self.begin(now);
        let was = self.push.state;
        if !matches!(was, State::Off | State::Dead) {
            self.push.socket.finish();
            self.push_died(was == State::Active);
        }
        self.end();
    }

    /// Push isn't coming up: close it, stay on the poll.
    fn push_give_up(&mut self) {
        let was_active = self.push.state == State::Active;
        self.outputs.push_back(Output::ClosePush);
        self.push_died(was_active);
    }

    fn push_died(&mut self, was_active: bool) {
        self.timers.cancel(&Slot::PumpReady);
        self.timers.cancel(&Slot::Subscribe);
        self.timers.cancel(&Slot::Debounce);
        self.push.state = State::Dead;
        self.push.subscribe = None;
        if was_active {
            // The poll goes back to carrying the updates, at once: the app
            // left its cadence at the 45 s safety net with no events coming.
            self.poll.set_push_active(false);
            self.dirty = true;
            self.event(Event::PushActive(false));
            self.quicken();
        }
        if self.push.was_active {
            self.arm(Slot::PushRetry, self.config.budgets.push_retry);
            self.push.revive = true;
        }
    }

    pub(super) fn push_retry(&mut self) {
        self.start_push();
    }

    /// A read came back a tree after one that found no server: push again,
    /// if it was up before the server went away and its retry found none
    /// (B422). The retry is one, after `push_retry`: a server down longer
    /// (stopped for an upgrade) left push off for the rest of the session,
    /// the agents' states as late as the poll. Tied to the server coming
    /// back, never to a timer, a server gone for good costs nothing.
    pub(super) fn revive_push(&mut self) {
        if self.push.revive
            && self.push.state == State::Dead
            && !self.timers.is_armed(&Slot::PushRetry)
        {
            self.start_push();
        }
    }

    /// The connection the push channel rode is gone, and the channel with
    /// it — unannounced: no close comes from a channel whose connection
    /// was dropped under it (mosh's control plane, redialled while mosh
    /// carries on). The poll carries the updates at once; push opens again
    /// on the next connection ([`Self::connection_restored`]): its losing
    /// the connection was no fault of push (B210: the client went on
    /// believing push up, the poll at its 45 s safety net and push never
    /// tried again — the tree and agent states up to 45 s late for the rest
    /// of the session).
    pub fn push_connection_lost(&mut self, now: u64) {
        self.begin(now);
        let was = self.push.state;
        if !matches!(was, State::Off | State::Dead) || self.timers.is_armed(&Slot::PushRetry) {
            self.push.socket.finish();
            self.timers.cancel(&Slot::PumpReady);
            self.timers.cancel(&Slot::Subscribe);
            self.timers.cancel(&Slot::Debounce);
            self.timers.cancel(&Slot::PushRetry);
            self.push.state = State::Dead;
            self.push.subscribe = None;
            self.push.reopen = true;
            if was == State::Active {
                self.poll.set_push_active(false);
                self.dirty = true;
                self.event(Event::PushActive(false));
                self.quicken();
            }
        }
        self.end();
    }

    /// A fresh connection is up: push again, if the last connection took
    /// it ([`Self::push_connection_lost`]).
    pub fn connection_restored(&mut self, now: u64) {
        self.begin(now);
        if self.push.reopen && self.push.state == State::Dead {
            self.push.reopen = false;
            self.start_push();
        }
        self.end();
    }

    pub(super) fn push_shut_down(&mut self) {
        if !matches!(self.push.state, State::Off | State::Dead) {
            self.outputs.push_back(Output::ClosePush);
        }
        self.push.state = State::Dead;
        self.push.was_active = false;
        self.push.revive = false;
    }
}
