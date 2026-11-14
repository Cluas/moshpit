//! The handshake's clock, which stops while a person is deciding.
//!
//! Citadel gave the whole login ten seconds from the moment the socket was
//! made, prompt included: a person who took longer than that to read a new
//! host's fingerprint and tap Trust got "Couldn't reach the server", while
//! the trust they gave was saved anyway. Here the time a host-key prompt or
//! a hardware signer (Face ID) takes is not the server's time.

use std::future::Future;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

/// Lets a host-key policy or a signer say "this part waits on a person".
/// Cheap to clone; every clone stops the same clock.
#[derive(Clone, Debug)]
pub struct Waiting {
    paused: watch::Sender<usize>,
}

impl Default for Waiting {
    fn default() -> Self {
        Self::new()
    }
}

impl Waiting {
    pub fn new() -> Self {
        Self {
            paused: watch::Sender::new(0),
        }
    }

    /// Runs `work` with the clock stopped. Waits nest: the clock runs again
    /// when the last one ends.
    pub async fn on_person<F: Future>(&self, work: F) -> F::Output {
        self.paused.send_modify(|n| *n += 1);
        let _resume = Resume(&self.paused);
        work.await
    }

    /// Whether some part of the handshake is waiting on a person right now.
    pub fn is_paused(&self) -> bool {
        *self.paused.borrow() > 0
    }

    /// `work` within `budget`, not counting the time spent in
    /// [`Self::on_person`]. `None` when the budget ran out first.
    pub async fn within<F: Future>(&self, budget: Duration, work: F) -> Option<F::Output> {
        let mut paused = self.paused.subscribe();
        tokio::pin!(work);
        let mut remaining = budget;
        loop {
            let stopped = *paused.borrow_and_update() > 0;
            let since = Instant::now();
            tokio::select! {
                out = &mut work => return Some(out),
                () = tokio::time::sleep(remaining), if !stopped => return None,
                changed = paused.changed() => {
                    if !stopped {
                        remaining = remaining.saturating_sub(since.elapsed());
                    }
                    // `self` holds the sender, so the channel can't close
                    // while this runs.
                    debug_assert!(changed.is_ok());
                }
            }
        }
    }
}

struct Resume<'a>(&'a watch::Sender<usize>);

impl Drop for Resume<'_> {
    fn drop(&mut self) {
        self.0.send_modify(|n| *n = n.saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn the_budget_runs_out_without_a_person() {
        let waiting = Waiting::new();
        let slow = tokio::time::sleep(Duration::from_secs(20));
        assert!(
            waiting
                .within(Duration::from_secs(15), slow)
                .await
                .is_none()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn time_on_a_person_does_not_count() {
        let waiting = Waiting::new();
        let person = waiting.clone();
        let work = async move {
            // 10 s of network, a minute of a person reading a fingerprint,
            // 4 s more of network: 14 s of the server's time.
            tokio::time::sleep(Duration::from_secs(10)).await;
            person
                .on_person(tokio::time::sleep(Duration::from_secs(60)))
                .await;
            tokio::time::sleep(Duration::from_secs(4)).await;
            "signed in"
        };
        assert_eq!(
            waiting.within(Duration::from_secs(15), work).await,
            Some("signed in")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_clock_runs_again_after_the_person() {
        let waiting = Waiting::new();
        let person = waiting.clone();
        let work = async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            person
                .on_person(tokio::time::sleep(Duration::from_secs(60)))
                .await;
            tokio::time::sleep(Duration::from_secs(6)).await;
        };
        assert!(
            waiting
                .within(Duration::from_secs(15), work)
                .await
                .is_none()
        );
        assert!(!waiting.is_paused());
    }

    #[tokio::test(start_paused = true)]
    async fn nested_waits_resume_only_when_the_last_ends() {
        let waiting = Waiting::new();
        let outer = waiting.clone();
        let work = async move {
            let inner = outer.clone();
            outer
                .on_person(async move {
                    inner
                        .on_person(tokio::time::sleep(Duration::from_secs(30)))
                        .await;
                    tokio::time::sleep(Duration::from_secs(30)).await;
                })
                .await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        };
        assert!(waiting.within(Duration::from_secs(2), work).await.is_some());
    }
}
