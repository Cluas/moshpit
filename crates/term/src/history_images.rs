//! What the images of every terminal's history keep, together (B733): each
//! terminal holds its own history's images to its budget
//! (`Options::history_image_bytes`), and the process holds them all to a
//! total. A phone keeps a terminal for every tmux pane it has shown (the
//! panes of windows since closed too), every herdr pane and every plain
//! session: ten or fifteen, at 32 MB apiece far past what iOS lets a
//! terminal app keep alongside its models. Past the total, the terminals
//! whose images came longest ago let theirs go first, from their oldest
//! rows; the text stays.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

/// The process's total, unless a host sets another.
pub const DEFAULT_TOTAL: usize = 96 << 20;

static TOTAL: AtomicUsize = AtomicUsize::new(DEFAULT_TOTAL);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static LEDGER: Mutex<Option<Ledger>> = Mutex::new(None);

/// Sets the process's total for every terminal's history images.
pub fn set_total(bytes: usize) {
    TOTAL.store(bytes, Ordering::Relaxed);
}

/// What the ledger asks of a terminal: let its history's images go down
/// to `target` bytes, at its next chance (its next bytes or frame, or at
/// once through its trimmer when it is idle).
#[derive(Debug, Default)]
pub(crate) struct Share {
    pub due: AtomicBool,
    pub target: AtomicUsize,
}

/// Lets a terminal shed what its share asks now, if it is free (a pane's
/// thread holds its lock while it parses: then it sheds at its next
/// bytes).
pub(crate) type Trimmer = Arc<dyn Fn() + Send + Sync>;

struct Entry {
    share: Arc<Share>,
    bytes: usize,
    /// When it last weighed its history's images (the ledger's clock).
    tick: u64,
    trim: Option<Trimmer>,
}

#[derive(Default)]
struct Ledger {
    tick: u64,
    entries: HashMap<u64, Entry>,
}

fn ledger() -> MutexGuard<'static, Option<Ledger>> {
    LEDGER.lock().unwrap_or_else(|e| e.into_inner())
}

/// A new terminal's place in the ledger.
pub(crate) fn register() -> (u64, Arc<Share>) {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let share = Arc::new(Share::default());
    ledger().get_or_insert_with(Ledger::default).entries.insert(
        id,
        Entry {
            share: share.clone(),
            bytes: 0,
            tick: 0,
            trim: None,
        },
    );
    (id, share)
}

pub(crate) fn unregister(id: u64) {
    if let Some(l) = ledger().as_mut() {
        l.entries.remove(&id);
    }
}

/// How terminal `id` sheds at once when the ledger asks.
pub(crate) fn set_trimmer(id: u64, trim: Trimmer) {
    if let Some(e) = ledger().as_mut().and_then(|l| l.entries.get_mut(&id)) {
        e.trim = Some(trim);
    }
}

/// What terminal `id`'s history's images keep after it let some go.
pub(crate) fn settle(id: u64, bytes: usize) {
    if let Some(e) = ledger().as_mut().and_then(|l| l.entries.get_mut(&id)) {
        e.bytes = bytes;
    }
}

/// Terminal `id` weighed its history's images (an image arrived): `bytes`.
/// Past the total, the terminals that weighed longest ago are asked to let
/// theirs go first; what they can't cover is `id`'s own to let go (the
/// bytes it may keep). The trimmers to call, with no lock of the ledger's
/// held (each sheds, then settles here).
pub(crate) fn weighed(id: u64, bytes: usize) -> (Option<usize>, Vec<Trimmer>) {
    let total = TOTAL.load(Ordering::Relaxed);
    let mut guard = ledger();
    let Some(l) = guard.as_mut() else {
        return (None, Vec::new());
    };
    l.tick += 1;
    let tick = l.tick;
    if let Some(e) = l.entries.get_mut(&id) {
        e.bytes = bytes;
        e.tick = tick;
    }
    let held: usize = l.entries.values().map(|e| e.bytes).sum();
    if held <= total {
        return (None, Vec::new());
    }
    let mut excess = held - total;
    let mut others: Vec<(&u64, &mut Entry)> = l
        .entries
        .iter_mut()
        .filter(|(k, e)| **k != id && e.bytes > 0)
        .collect();
    others.sort_by_key(|(_, e)| e.tick);
    let mut trims = Vec::new();
    for (_, e) in others {
        if excess == 0 {
            break;
        }
        let take = e.bytes.min(excess);
        e.bytes -= take;
        excess -= take;
        e.share.target.store(e.bytes, Ordering::Relaxed);
        e.share.due.store(true, Ordering::Release);
        if let Some(t) = &e.trim {
            trims.push(t.clone());
        }
    }
    let own = (excess > 0).then(|| bytes.saturating_sub(excess));
    if let (Some(own), Some(e)) = (own, l.entries.get_mut(&id)) {
        e.bytes = own;
    }
    (own, trims)
}
