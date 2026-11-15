//! The turns the host said ended (`PushClosed`, B636): per connection and
//! pane, when the newest closing status the host pushed was sent, on the
//! host's clock.
//!
//! A pushed `attention` and the `done` that ends its question can arrive in
//! either order — APNs reorders, and a relay can send an envelope it carried
//! once again (sealed envelopes are opened, not checked for freshness). An
//! `attention` that came after its own `done` was taken for a new question:
//! it rang "needs you" and stood as waiting for six hours on a pane nobody
//! waited on. And a `done` that came again cleared the question the pane
//! asked after it. Both are told by the host's clock: a question is the
//! pane's `since` (the episode), a closing status its send time (`ts`).
//!
//! The file (`closed-turns.json`, beside the standing set) is a JSON object,
//! `"<conn>|<pane>"` → `{"closedAt": <unix seconds>, "recordedAt": <Date>}`,
//! the shape of the finishes file. Read and write happen under an exclusive
//! lock on `<file>.lock`. What can't be read answers "nothing ended" and is
//! never written over; a shape this build doesn't know starts over (it is a
//! memory of endings, nothing more). Records expire with the standing set's
//! [`crate::standing::LIFETIME_SECONDS`].

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::foundation::date::Date;
use crate::standing::LIFETIME_SECONDS;
use crate::store;

/// How long the lock is waited for (B636): what the file says refines a
/// card, and the notification service has seconds in all. Past this the
/// caller goes on as if nothing had ended — today's behaviour.
pub const LOCK_BUDGET: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Closed {
    #[serde(rename = "closedAt")]
    closed_at: i64,
    #[serde(rename = "recordedAt")]
    recorded_at: Date,
}

/// The endings file's lock stayed taken past [`LOCK_BUDGET`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

/// The endings file.
#[derive(Debug, Clone)]
pub struct PushClosedFile {
    path: PathBuf,
}

impl PushClosedFile {
    pub const FILE_NAME: &'static str = "closed-turns.json";

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The endings file beside the file at `path` (the standing set's).
    pub fn beside(path: &Path) -> Self {
        Self::new(path.with_file_name(Self::FILE_NAME))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The host said `pane`'s turn ended at `closed_at` (unix seconds, its
    /// clock): remembered, unless a later ending already is. The file keeps
    /// one entry per pane, and every call drops what is past the lifetime,
    /// so it holds only the panes that ended lately. `Err(Busy)` when the
    /// lock stayed taken past [`LOCK_BUDGET`]: not remembered.
    pub fn note_closed(
        &self,
        conn: &str,
        pane: &str,
        closed_at: i64,
        now: Date,
    ) -> Result<(), Busy> {
        let Some(_lock) = store::Lock::try_take(&self.path, LOCK_BUDGET) else {
            return Err(Busy);
        };
        let key = key(conn, pane);
        let Read::Closed(stored) = self.read() else {
            return Ok(());
        };
        let mut all: BTreeMap<String, Closed> = stored
            .iter()
            .filter(|(_, closed)| live(**closed, now))
            .map(|(key, closed)| (key.clone(), *closed))
            .collect();
        let newer_known = all
            .get(&key)
            .is_some_and(|known| known.closed_at >= closed_at);
        if !newer_known {
            all.insert(
                key,
                Closed {
                    closed_at,
                    recorded_at: now,
                },
            );
        }
        if all != stored {
            self.write(&all);
        }
        Ok(())
    }

    /// When the host last said `pane`'s turn ended, within the lifetime.
    /// `Err(Busy)` when the lock stayed taken past [`LOCK_BUDGET`]: the
    /// caller goes on as if nothing had ended — today's behaviour.
    pub fn closed_at(&self, conn: &str, pane: &str, now: Date) -> Result<Option<i64>, Busy> {
        let Some(_lock) = store::Lock::try_take(&self.path, LOCK_BUDGET) else {
            return Err(Busy);
        };
        Ok(match self.read() {
            Read::Unreadable => None,
            Read::Closed(all) => all
                .get(&key(conn, pane))
                .filter(|closed| live(**closed, now))
                .map(|closed| closed.closed_at),
        })
    }

    fn read(&self) -> Read {
        match fs::read(&self.path) {
            Ok(bytes) => Read::Closed(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(error) if error.kind() == ErrorKind::NotFound => Read::Closed(BTreeMap::new()),
            Err(_) => Read::Unreadable,
        }
    }

    fn write(&self, all: &BTreeMap<String, Closed>) {
        let Ok(bytes) = serde_json::to_vec(all) else {
            return;
        };
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let temporary = self.path.with_extension("json.tmp");
        if fs::write(&temporary, bytes).is_ok() {
            let _ = fs::rename(&temporary, &self.path);
        }
    }
}

enum Read {
    Closed(BTreeMap<String, Closed>),
    Unreadable,
}

fn key(conn: &str, pane: &str) -> String {
    format!("{conn}|{pane}")
}

fn live(closed: Closed, now: Date) -> bool {
    now.seconds_since(closed.recorded_at) < LIFETIME_SECONDS
        && closed.recorded_at < now.adding(60.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    const NOW: f64 = 800_000_000.0;

    fn file() -> (TempDir, PushClosedFile) {
        let dir = TempDir::new("closed");
        let file = PushClosedFile::new(dir.path(PushClosedFile::FILE_NAME));
        (dir, file)
    }

    fn at(file: &PushClosedFile, conn: &str, pane: &str, now: Date) -> Option<i64> {
        file.closed_at(conn, pane, now).expect("not busy")
    }

    #[test]
    fn the_newest_ending_of_a_pane_is_kept() {
        let (_dir, file) = file();
        let now = Date(NOW);
        assert_eq!(at(&file, "c", "%1", now), None);
        file.note_closed("c", "%1", 1_790_000_600, now).unwrap();
        file.note_closed("c", "%1", 1_790_000_500, now).unwrap();
        assert_eq!(
            at(&file, "c", "%1", now),
            Some(1_790_000_600),
            "an older one came after"
        );
        file.note_closed("c", "%1", 1_790_000_700, now).unwrap();
        assert_eq!(at(&file, "c", "%1", now), Some(1_790_000_700));
        assert_eq!(at(&file, "c", "%2", now), None, "another pane");
        assert_eq!(at(&file, "d", "%1", now), None, "another connection");
    }

    #[test]
    fn an_ending_is_forgotten_after_the_lifetime() {
        let (_dir, file) = file();
        file.note_closed("c", "%1", 100, Date(NOW)).unwrap();
        assert_eq!(
            at(&file, "c", "%1", Date(NOW + LIFETIME_SECONDS + 1.0)),
            None
        );
    }

    fn stored_keys(file: &PushClosedFile) -> Vec<String> {
        let bytes = fs::read(file.path()).unwrap();
        let all: BTreeMap<String, Closed> = serde_json::from_slice(&bytes).unwrap();
        all.into_keys().collect()
    }

    /// The file can't grow with every pane ever seen: each write keeps one
    /// entry per pane and drops what is past the lifetime.
    #[test]
    fn every_write_drops_what_is_past_the_lifetime() {
        let (_dir, file) = file();
        file.note_closed("c", "%1", 100, Date(NOW)).unwrap();
        file.note_closed("c", "%1", 200, Date(NOW)).unwrap();
        assert_eq!(
            stored_keys(&file),
            vec!["c|%1".to_owned()],
            "one entry per pane"
        );
        let later = Date(NOW + LIFETIME_SECONDS + 1.0);
        file.note_closed("c", "%2", 300, later).unwrap();
        assert_eq!(stored_keys(&file), vec!["c|%2".to_owned()]);
        // An older ending of a known pane writes nothing new, but still
        // drops what expired.
        file.note_closed("c", "%2", 250, later).unwrap();
        assert_eq!(stored_keys(&file), vec!["c|%2".to_owned()]);
    }

    /// A file a thousand stale entries long (panes over months, written by
    /// a build without the prune) comes back to what is live on the next
    /// write.
    #[test]
    fn a_thousand_stale_entries_go_on_the_next_write() {
        let (_dir, file) = file();
        let old = Date(NOW - LIFETIME_SECONDS - 10.0);
        let stale: BTreeMap<String, Closed> = (0..1000)
            .map(|i| {
                (
                    format!("c|%{i}"),
                    Closed {
                        closed_at: i,
                        recorded_at: old,
                    },
                )
            })
            .collect();
        fs::write(file.path(), serde_json::to_vec(&stale).unwrap()).unwrap();
        assert_eq!(
            at(&file, "c", "%5", Date(NOW)),
            None,
            "a stale ending still counts"
        );
        file.note_closed("c", "%7", 1_790_000_000, Date(NOW))
            .unwrap();
        assert_eq!(stored_keys(&file), vec!["c|%7".to_owned()]);
    }

    /// B636: the lock held by another past the budget reads as nothing
    /// ended, and the ending as not remembered — in time, never waiting.
    #[test]
    fn a_busy_lock_fails_open_within_the_budget() {
        let (_dir, file) = file();
        let _held = store::Lock::take(file.path());
        let started = std::time::Instant::now();
        // flock is per open file description: another open in this process
        // contends as another process would.
        let other = PushClosedFile::new(file.path().to_owned());
        assert_eq!(other.closed_at("c", "%1", Date(NOW)), Err(Busy));
        assert_eq!(other.note_closed("c", "%1", 1, Date(NOW)), Err(Busy));
        assert!(
            started.elapsed() < LOCK_BUDGET * 4,
            "{:?}",
            started.elapsed()
        );
    }
}
