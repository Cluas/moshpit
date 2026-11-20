//! The overview's pictures of herdr panes the phone doesn't show (B751).
//!
//! The phone's terminal shows one pane (the frame channel's, or mosh's
//! renderer); the others' screens are nowhere on the phone. While the
//! overview is open it asks for them once: one `herdr pane read <id>
//! --format ansi` each, at most [`READS_IN_FLIGHT`] at a time, the rest
//! queued. A read takes nothing: not the focus, not a pane, not a size
//! (checked on a private herdr: the server's snapshot reads the same after
//! it). Its text goes to the overview as [`Event::PaneRead`], never into
//! the terminal. The pane shown, or one the phone moves to before the answer
//! comes, is never read: its live screen is on the phone. The overview
//! closing ([`Action::StopReads`]) drops what is queued and on its way.

use std::collections::{BTreeSet, VecDeque};

use super::exec::{ExecResult, read_result};
use super::{Client, Event, Purpose};
use crate::control;

#[cfg(doc)]
use super::Action;

/// The most reads out at once: the command lane and the poll keep theirs.
pub(super) const READS_IN_FLIGHT: usize = 2;

/// The reads asked for and not answered.
#[derive(Debug, Default)]
pub(super) struct Reads {
    /// Bumped by a stop: an answer to an older ask is dropped.
    generation: u64,
    queued: VecDeque<String>,
    in_flight: BTreeSet<String>,
}

impl Client {
    /// Asks for the screens of `panes` the phone doesn't show: each one
    /// answered with [`Event::PaneRead`].
    pub(super) fn read_panes(&mut self, panes: Vec<String>) {
        for pane in panes {
            let shown = self.poll.snapshot.active_pane_id.as_deref() == Some(pane.as_str());
            let asked = self.reads.queued.contains(&pane) || self.reads.in_flight.contains(&pane);
            if !shown && !asked && !pane.is_empty() {
                self.reads.queued.push_back(pane);
            }
        }
        self.send_reads();
    }

    /// The overview closed: nothing more is read, answers on their way are
    /// dropped.
    pub(super) fn stop_reads(&mut self) {
        self.reads.generation = self.reads.generation.wrapping_add(1);
        self.reads.queued.clear();
        self.reads.in_flight.clear();
    }

    /// The panes asked for and not answered.
    pub fn reads_pending(&self) -> usize {
        self.reads.queued.len() + self.reads.in_flight.len()
    }

    fn send_reads(&mut self) {
        while self.reads.in_flight.len() < READS_IN_FLIGHT {
            let Some(pane) = self.reads.queued.pop_front() else {
                break;
            };
            self.reads.in_flight.insert(pane.clone());
            let command = self.herdr(&control::pane_read(&pane));
            let generation = self.reads.generation;
            let budget = self.config.budgets.status;
            self.exec(
                command,
                Purpose::PaneRead { pane, generation },
                Some(budget),
            );
        }
    }

    pub(super) fn pane_read_answered(
        &mut self,
        pane: String,
        generation: u64,
        result: Option<ExecResult>,
    ) {
        if generation != self.reads.generation {
            return;
        }
        self.reads.in_flight.remove(&pane);
        self.send_reads();
        let Some(ExecResult::Output(bytes)) = result else {
            return;
        };
        let read = read_result(&bytes);
        if read.failed() {
            return;
        }
        // Shown meanwhile: its live screen is on the phone.
        if self.poll.snapshot.active_pane_id.as_deref() == Some(pane.as_str()) {
            return;
        }
        let rows = read.output.lines().map(str::to_owned).collect();
        self.event(Event::PaneRead { pane, rows });
    }
}
