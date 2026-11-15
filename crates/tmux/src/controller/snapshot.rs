//! The overview's pictures of panes never shown (B751).
//!
//! A pane's terminal gets its screen when it is first shown (the backfill,
//! [`super::repair`]); until then tmux sends it only what changes, so a pane
//! never visited has an empty terminal and the overview an empty card. While
//! the overview is open it asks for those panes' screens once: one
//! `capture-pane -p` per pane (the visible rows only, plain text), at most
//! [`SNAPSHOTS_IN_FLIGHT`] at a time, the rest queued.
//!
//! A snapshot is a picture, never the pane's state: its rows go to the
//! overview as [`Notification::PaneSnapshot`], never into a pane's terminal
//! or its byte stream ([`Output::PaneOutput`]), so the ordered delivery of
//! the pane's own bytes is untouched. Nothing is subscribed and no size
//! changes. A pane shown meanwhile (its backfill or a repaint sent) gets its
//! live screen in its own stream: its snapshot, queued or on its way, is
//! dropped, never standing in for that. The overview closing
//! ([`Controller::stop_snapshots`]) drops what is queued and what is still
//! on its way.

use crate::control::CommandResponse;
use crate::format::is_valid_id;

use super::{Controller, Notification, Reply};

#[cfg(doc)]
use super::Output;

/// The most `capture-pane`s on their way at once: the control channel stays
/// free for what the person does.
pub(crate) const SNAPSHOTS_IN_FLIGHT: usize = 4;

/// The snapshots asked for and not answered yet.
#[derive(Debug, Default)]
pub(crate) struct Snapshots {
    /// Bumped by a stop: an answer to an older ask is dropped.
    pub generation: u64,
    /// Asked, not sent yet, in order.
    pub queued: std::collections::VecDeque<String>,
    /// Sent, not answered yet.
    pub in_flight: std::collections::BTreeSet<String>,
    /// Asked, then shown before the answer came: the answer is dropped.
    pub superseded: std::collections::BTreeSet<String>,
}

impl Controller {
    /// Asks for the visible screens of `panes` that have never been shown
    /// (their terminal is empty): each answered with
    /// [`Notification::PaneSnapshot`]. A pane already asked for, or shown,
    /// is not asked again.
    pub fn snapshot_panes(&mut self, panes: &[String], now: u64) {
        self.begin_input(now);
        if self.snapshot.is_attached {
            for pane in panes {
                let asked =
                    self.snapshots.queued.contains(pane) || self.snapshots.in_flight.contains(pane);
                if is_valid_id(pane) && !asked && !self.backfilled_panes.contains(pane) {
                    self.snapshots.queued.push_back(pane.clone());
                }
            }
            self.send_snapshots();
        }
        self.finish_input();
    }

    /// The overview closed: nothing more is asked, and answers still on
    /// their way are dropped.
    pub fn stop_snapshots(&mut self, now: u64) {
        self.begin_input(now);
        self.forget_snapshots();
        self.finish_input();
    }

    /// Drops every ask (a stop, a detach).
    pub(super) fn forget_snapshots(&mut self) {
        self.snapshots.generation = self.snapshots.generation.wrapping_add(1);
        self.snapshots.queued.clear();
        self.snapshots.in_flight.clear();
        self.snapshots.superseded.clear();
    }

    /// Pane `pane`'s live screen is on its way (a backfill, a repaint): a
    /// snapshot still queued is not sent, one on its way is dropped.
    pub(super) fn supersede_snapshot(&mut self, pane: &str) {
        self.snapshots.queued.retain(|p| p != pane);
        if self.snapshots.in_flight.contains(pane) {
            self.snapshots.superseded.insert(pane.to_owned());
        }
    }

    /// The panes asked for and not answered: queued, or on their way.
    pub fn snapshots_pending(&self) -> usize {
        self.snapshots.queued.len() + self.snapshots.in_flight.len()
    }

    /// Sends what is queued, up to [`SNAPSHOTS_IN_FLIGHT`] on the way.
    fn send_snapshots(&mut self) {
        while self.snapshots.in_flight.len() < SNAPSHOTS_IN_FLIGHT {
            let Some(pane) = self.snapshots.queued.pop_front() else {
                break;
            };
            self.snapshots.in_flight.insert(pane.clone());
            let generation = self.snapshots.generation;
            self.send_command(
                format!("capture-pane -p -t {pane}"),
                Reply::Snapshot { pane, generation },
            );
        }
    }

    pub(super) fn snapshot_answered(
        &mut self,
        pane: &str,
        generation: u64,
        response: &CommandResponse,
    ) {
        if generation != self.snapshots.generation {
            // Asked before a stop: nobody waits for it.
            return;
        }
        self.snapshots.in_flight.remove(pane);
        let superseded = self.snapshots.superseded.remove(pane);
        self.send_snapshots();
        // Refused (the pane is gone), or the channel closed first.
        if response.is_error || response.command_id < 0 {
            return;
        }
        // Shown meanwhile: its live screen came in its own stream, which a
        // picture must not stand in for.
        if superseded || self.backfilled_panes.contains(pane) {
            return;
        }
        self.notify(Notification::PaneSnapshot {
            pane: pane.to_owned(),
            rows: response.lines.clone(),
        });
    }
}
