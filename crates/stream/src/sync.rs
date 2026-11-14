//! The locks the channel's state lives behind: std's, or loom's when the
//! model checker runs (`RUSTFLAGS="--cfg moshpit_loom"`, `src/loom_tests.rs`;
//! loom is a dev-dependency, so only the crate's own test build has it).

#[cfg(all(test, moshpit_loom))]
pub(crate) use loom::sync::{Mutex, MutexGuard};
#[cfg(not(all(test, moshpit_loom)))]
pub(crate) use std::sync::{Mutex, MutexGuard};

/// Locks `mutex`. A panic while it was held cannot leave the queues half
/// written (every update is a few assignments after the one call that can
/// allocate), so a poisoned lock is taken over rather than spread.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}
