//! The controller's timers: what the Swift controller ran as `Task { sleep;
//! … }`, each in a slot it cancelled before re-arming.
//!
//! A slot holds at most one timer; arming a slot replaces what it held (the
//! Swift `task?.cancel(); task = Task { … }`). Due timers fire in deadline
//! order, ties in the order they were armed — the order their Swift tasks
//! started sleeping.

use std::collections::BTreeMap;

/// Where a timer lives. Slots with a pane or request in them are one per
/// pane or request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Slot {
    /// `pendingResize`: the resize debounce.
    PendingResize,
    /// `pendingSettleResync`: the settled repaint (and the return-key one).
    SettleResync,
    /// `foregroundProbeFallback`.
    ForegroundProbe,
    /// `foreignQuietRepinTask`.
    ForeignQuietRepin,
    /// `backfillTimeouts[pane]`.
    BackfillTimeout(String),
    /// `backfillRetryTasks[pane]`.
    BackfillRetry(String),
    /// `resyncTimeouts[pane]`.
    ResyncTimeout(String),
    /// `resyncRetryTask` (one for every pane).
    ResyncRetry,
    /// `convergenceResyncTask`.
    ConvergenceDebounce,
    /// `convergenceDeadlineTask`.
    ConvergenceDeadline,
    /// `deferredResizeTimeouts[pane]`.
    DeferredResize(String),
    /// `noticeClear`.
    NoticeClear,
    /// The refresh spinner's hold after `display-message -p done` (not
    /// cancellable in Swift: one per refresh).
    RefreshDone(u64),
    /// The dead-channel watchdog's 4 s beat.
    Watchdog,
    /// `panePath`'s 4 s bound, per request.
    PanePath(u64),
    /// `flushControlChannel`'s bound, per request.
    Flush(u64),
    /// What waits behind a paste goes (B485).
    PasteSettle,
}

/// What a timer does when it fires.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    CommitClientSize,
    /// The settled pass after a resize or switch: extend the cover, repaint.
    SettledResync,
    /// The fresh attach's settled pass: extend by 2 s, repaint.
    AttachSettledResync,
    /// The foreground's 300 ms repaint.
    ForegroundResync,
    /// Return key: repaint that pane after its redraw.
    ReturnResync(String),
    ForegroundProbeTimeout(String),
    ForeignQuietRepin(String),
    BackfillTimeout(String),
    BackfillRetry(String),
    ResyncTimeout(String),
    ResyncRetry {
        pane: String,
        reveal: bool,
    },
    Convergence,
    DeferredResize(String),
    ClearNotice,
    RefreshDone,
    Watchdog,
    PanePathTimeout(u64),
    FlushTimeout(u64),
    /// The paste's wait is over: what was held behind it goes (B485).
    PasteSettled,
}

#[derive(Debug, Clone)]
struct Armed {
    deadline: u64,
    seq: u64,
    action: Action,
}

#[derive(Debug, Default)]
pub(crate) struct Timers {
    armed: BTreeMap<Slot, Armed>,
    seq: u64,
}

impl Timers {
    /// Arm `slot` to fire `action` at `deadline`, replacing what it held.
    pub fn arm(&mut self, slot: Slot, deadline: u64, action: Action) {
        self.seq += 1;
        self.armed.insert(
            slot,
            Armed {
                deadline,
                seq: self.seq,
                action,
            },
        );
    }

    pub fn cancel(&mut self, slot: &Slot) {
        self.armed.remove(slot);
    }

    pub fn is_armed(&self, slot: &Slot) -> bool {
        self.armed.contains_key(slot)
    }

    /// The earliest deadline, if any timer is armed.
    pub fn next_deadline(&self) -> Option<u64> {
        self.armed.values().map(|armed| armed.deadline).min()
    }

    /// Take the next timer due at `now`: earliest deadline, then the one
    /// armed first.
    pub fn pop_due(&mut self, now: u64) -> Option<(u64, Action)> {
        let slot = self
            .armed
            .iter()
            .filter(|(_, armed)| armed.deadline <= now)
            .min_by_key(|(_, armed)| (armed.deadline, armed.seq))
            .map(|(slot, _)| slot.clone())?;
        let armed = self.armed.remove(&slot)?;
        Some((armed.deadline, armed.action))
    }
}
