//! The machine on a tokio runtime: its outputs carried out by a
//! [`Platform`] — commands on a connection's host, requests to a relay,
//! the OS's device token — and their answers fed back; its deadlines kept
//! with tokio's timer. iOS (`crates/ffi`, Swift's platform) and Android
//! (`crates/android`) drive it the same way.
//!
//! The platform is never called with the machine locked: what the machine
//! asked for is taken out first, so a platform that reads a view back
//! inside [`Platform::changed`] can't deadlock.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use moshpit_seal::{Date, Uuid};
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::{Change, Clock, HostCare, HttpReply, Output};

/// What a platform does for the machine.
#[async_trait]
pub trait Platform: Send + Sync + 'static {
    /// Run `command` on `connection`'s host — under `sh -c`, on its
    /// session's own SSH connection — and return its stdout; `Err` with why
    /// (in the person's language) when the connection failed or the command
    /// exited non-zero.
    async fn exec(&self, connection: Uuid, command: String) -> Result<Vec<u8>, String>;
    /// POST `body` (JSON, `content-type: application/json`) to `url`;
    /// `Err` only when it didn't reach the server.
    async fn post(
        &self,
        url: String,
        body: String,
        timeout_seconds: u32,
    ) -> Result<HttpReply, String>;
    /// Ask the OS for a device token (prompt when undecided).
    fn register_for_push(&self);
    /// Persist the person's "don't ask again" for `connection`.
    fn remember_declined(&self, connection: Uuid);
    /// Persist (`unpaired`) or clear the person's Unpair of `connection`
    /// (B371), which care honours until they pair again.
    fn remember_unpaired(&self, connection: Uuid, unpaired: bool);
    /// A view changed: read it again ([`HostCare::setup_view`],
    /// [`HostCare::consent`], [`HostCare::relay_error`]).
    fn changed(&self, change: Change);
}

struct Inner {
    machine: Mutex<HostCare>,
    platform: Arc<dyn Platform>,
    runtime: Handle,
    origin: Instant,
    /// The tick armed now: its deadline, and the task that waits for it.
    timer: Mutex<Option<(u64, JoinHandle<()>)>>,
    /// Woken after every input, for [`Driver::care_wind_down`].
    moved: Notify,
}

/// The machine, driven.
#[derive(Clone)]
pub struct Driver {
    inner: Arc<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Driver {
    pub fn new(machine: HostCare, platform: Arc<dyn Platform>, runtime: Handle) -> Self {
        Driver {
            inner: Arc::new(Inner {
                machine: Mutex::new(machine),
                platform,
                runtime,
                origin: Instant::now(),
                timer: Mutex::new(None),
                moved: Notify::new(),
            }),
        }
    }

    /// Now, on the driver's clocks.
    pub fn clock(&self) -> Clock {
        Clock {
            ms: self.inner.origin.elapsed().as_millis() as u64,
            date: Date::now(),
        }
    }

    /// Give the machine an input (or read it), then carry out what it
    /// asked for.
    pub fn with<R>(&self, f: impl FnOnce(&mut HostCare, Clock) -> R) -> R {
        let clock = self.clock();
        let (result, outputs, deadline) = {
            let mut machine = lock(&self.inner.machine);
            let result = f(&mut machine, clock);
            let mut outputs = Vec::new();
            while let Some(output) = machine.poll_output() {
                outputs.push(output);
            }
            (result, outputs, machine.next_deadline())
        };
        self.arm(deadline);
        for output in outputs {
            self.dispatch(output);
        }
        self.inner.moved.notify_waiters();
        result
    }

    /// Read the machine without giving it an input.
    pub fn read<R>(&self, f: impl FnOnce(&HostCare) -> R) -> R {
        f(&lock(&self.inner.machine))
    }

    fn dispatch(&self, output: Output) {
        let platform = self.inner.platform.clone();
        match output {
            Output::Exec {
                id,
                connection,
                command,
            } => {
                let me = self.clone();
                self.inner.runtime.spawn(async move {
                    let result = platform.exec(connection, command).await;
                    me.with(|m, clock| m.exec_done(id, result, clock));
                });
            }
            Output::Post {
                id,
                url,
                body,
                timeout_seconds,
            } => {
                let me = self.clone();
                self.inner.runtime.spawn(async move {
                    let result = platform.post(url, body, timeout_seconds).await;
                    me.with(|m, clock| m.post_done(id, result, clock));
                });
            }
            Output::RegisterForPush => platform.register_for_push(),
            Output::RememberDeclined { connection } => platform.remember_declined(connection),
            Output::RememberUnpaired {
                connection,
                unpaired,
            } => platform.remember_unpaired(connection, unpaired),
            Output::Changed(change) => platform.changed(change),
        }
    }

    /// Wake the machine at `deadline` (only the earliest is armed).
    fn arm(&self, deadline: Option<u64>) {
        let mut timer = lock(&self.inner.timer);
        if timer.as_ref().map(|(at, _)| *at) == deadline {
            return;
        }
        if let Some((_, task)) = timer.take() {
            task.abort();
        }
        let Some(at) = deadline else {
            return;
        };
        let me = self.clone();
        let wake = self.inner.origin + Duration::from_millis(at);
        let task = self.inner.runtime.spawn(async move {
            tokio::time::sleep_until(wake.into()).await;
            {
                let mut timer = lock(&me.inner.timer);
                if timer.as_ref().map(|(armed, _)| *armed) == Some(at) {
                    *timer = None;
                }
            }
            me.with(|m, clock| m.tick(clock));
        });
        *timer = Some((at, task));
    }

    /// Wait for a deleted `connection`'s pairing to come off its host, at
    /// most the forget budget (B377): before the delete takes the session
    /// down, so the host can still be asked.
    pub async fn forget_wind_down(&self, connection: Uuid) {
        let budget = Duration::from_millis(crate::Budgets::default().forget_on_delete_ms);
        let wait = async {
            loop {
                let moved = self.inner.moved.notified();
                if !self.read(|m| m.is_forgetting(connection)) {
                    return;
                }
                moved.await;
            }
        };
        let _ = tokio::time::timeout(budget, wait).await;
    }

    /// Wait for the care pass out for `connection` to end, at most the
    /// machine's wind-down budget (`ConnectionDeletion`'s wait).
    pub async fn care_wind_down(&self, connection: Uuid) {
        let budget = Duration::from_millis(crate::Budgets::default().care_wind_down_ms);
        let wait = async {
            loop {
                let moved = self.inner.moved.notified();
                if !self.read(|m| m.is_tending(connection)) {
                    return;
                }
                moved.await;
            }
        };
        let _ = tokio::time::timeout(budget, wait).await;
    }
}
