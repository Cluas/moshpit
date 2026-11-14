//! The client's timers: what the Swift code ran as `Task { sleep; … }` and
//! `WriteLane.wait(atMost:)`, each in a slot that re-arming replaces.
//!
//! Due timers fire in deadline order, ties in the order they were armed.

use std::collections::BTreeMap;

/// Where a timer lives; the slot names what firing it does.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Slot {
    /// The next read of the poll's cadence.
    Poll,
    /// An exec off the lanes (a poll, a status read, discovery, the
    /// restart's steps): nobody waits for it past this.
    Exec(u64),
    /// The running command-lane step stops holding the steps behind it.
    LaneHold(u64),
    /// The owner of a command-lane step stops waiting for its answer.
    Patience(u64),
    /// The write lane stops waiting for the key in flight.
    KeyHold,
    /// The release written for a switch never came back as
    /// `terminal.closed`.
    ReleaseAck,
    /// The login shell's prompt before a one-shot attach line.
    Settle,
    /// The reader loop never announced itself after the boot line.
    BootProbe,
    /// The attach line went out: past the attach timeout, an attach that
    /// never painted is asked for again.
    AttachWatch,
    /// An expected `terminal.closed` that never came is forgotten.
    ExpectedClose(u64),
    /// Input held for the pane being switched to.
    HeldInput,
    /// What follows a paste goes once the paste has had its time (B485).
    PasteSettle,
    /// A task's own waits (the settle, a retry, the submit delay).
    Task(u64),
    /// The restart's waits.
    Restart,
    /// The push pump never said it holds the socket.
    PumpReady,
    /// The subscribe never got its reply.
    Subscribe,
    /// The push invalidation's debounce.
    Debounce,
    /// Re-open a push channel that died.
    PushRetry,
    /// A one-off notice comes down (B491).
    NoticeFlash,
}

#[derive(Debug, Default)]
pub(crate) struct Timers {
    armed: BTreeMap<Slot, (u64, u64)>,
    seq: u64,
}

impl Timers {
    /// Arm `slot` for `deadline`, replacing what it held.
    pub fn arm(&mut self, slot: Slot, deadline: u64) {
        self.seq += 1;
        self.armed.insert(slot, (deadline, self.seq));
    }

    pub fn cancel(&mut self, slot: &Slot) {
        self.armed.remove(slot);
    }

    pub fn is_armed(&self, slot: &Slot) -> bool {
        self.armed.contains_key(slot)
    }

    pub fn next_deadline(&self) -> Option<u64> {
        self.armed.values().map(|&(deadline, _)| deadline).min()
    }

    /// The next timer due by `now`: earliest deadline, then armed first.
    pub fn pop_due(&mut self, now: u64) -> Option<(u64, Slot)> {
        let slot = self
            .armed
            .iter()
            .filter(|(_, (deadline, _))| *deadline <= now)
            .min_by_key(|(_, key)| **key)
            .map(|(slot, _)| slot.clone())?;
        let (deadline, _) = self.armed.remove(&slot)?;
        Some((deadline, slot))
    }
}
