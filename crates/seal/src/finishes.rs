//! The finishes that have rung (`PushFinishes`): per connection and pane,
//! the last turn whose `done` made a sound — shared between the app and the
//! notification service so the local finish card and the pushed one for the
//! same turn make one sound between them.
//!
//! A turn is told by when it ended, on the host's clock: the push carries its
//! send time (`ts`), the app the pane's `@moshpit_since` for `done` — the
//! same moment, a second or so apart. Two finishes of one pane within
//! [`SAME_TURN_SECONDS`] are one turn: a turn worth a sound runs minutes.
//! Records expire after [`LIFETIME_SECONDS`].
//!
//! The file (`rung-finishes.json`) is a JSON object, `"<conn>|<pane>"` →
//! `{"endedAt": <unix seconds>, "recordedAt": <Date>}` — what the Swift side
//! writes. Read, decide and write happen under an exclusive lock on
//! `<file>.lock`, across processes. What can't be read answers "ring" and is
//! never written over; a shape this build doesn't know starts over (it is a
//! memory of chimes, nothing more).

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::foundation::date::Date;
use crate::store;

/// Finishes this close together, on one pane, are one turn's.
pub const SAME_TURN_SECONDS: i64 = 60;
/// How long a ring is remembered.
pub const LIFETIME_SECONDS: f64 = 15.0 * 60.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Ring {
    #[serde(rename = "endedAt")]
    ended_at: i64,
    #[serde(rename = "recordedAt")]
    recorded_at: Date,
}

/// The rings file.
#[derive(Debug, Clone)]
pub struct PushFinishesFile {
    path: PathBuf,
}

impl PushFinishesFile {
    /// Its name in the App Group container.
    pub const FILE_NAME: &'static str = "rung-finishes.json";

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The finish of `pane`'s turn ending at `ended_at` (unix seconds, host
    /// clock) is about to ring: whether it is the first of its turn to — the
    /// caller's licence to make the sound. Remembered either way.
    pub fn note_ring(&self, conn: &str, pane: &str, ended_at: i64, now: Date) -> bool {
        let _lock = store::Lock::take(&self.path);
        let key = format!("{conn}|{pane}");
        let mut rings: BTreeMap<String, Ring> = match self.read() {
            Read::Unreadable => return true,
            Read::Rings(rings) => rings.into_iter().filter(|(_, ring)| live(*ring, now)).collect(),
        };
        if let Some(last) = rings.get(&key)
            && (last.ended_at - ended_at).abs() <= SAME_TURN_SECONDS
        {
            return false;
        }
        rings.insert(
            key,
            Ring {
                ended_at,
                recorded_at: now,
            },
        );
        self.write(&rings);
        true
    }

    fn read(&self) -> Read {
        match fs::read(&self.path) {
            Ok(bytes) => Read::Rings(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(error) if error.kind() == ErrorKind::NotFound => Read::Rings(BTreeMap::new()),
            Err(_) => Read::Unreadable,
        }
    }

    fn write(&self, rings: &BTreeMap<String, Ring>) {
        let Ok(bytes) = serde_json::to_vec(rings) else { return };
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
    Rings(BTreeMap<String, Ring>),
    Unreadable,
}

fn live(ring: Ring, now: Date) -> bool {
    now.seconds_since(ring.recorded_at) < LIFETIME_SECONDS && ring.recorded_at < now.adding(60.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    fn file() -> (TempDir, PushFinishesFile) {
        let dir = TempDir::new("finishes");
        let file = PushFinishesFile::new(dir.path("rung-finishes.json"));
        (dir, file)
    }

    const NOW: f64 = 800_000_000.0;

    #[test]
    fn one_turns_finish_rings_once_between_two_copies() {
        let (_dir, file) = file();
        let now = Date(NOW);
        assert!(file.note_ring("c", "%1", 1_790_000_600, now));
        assert!(!file.note_ring("c", "%1", 1_790_000_601, now), "the other copy of the turn");
        assert!(file.note_ring("c", "%2", 1_790_000_601, now), "another pane");
        assert!(file.note_ring("c", "%1", 1_790_001_500, now), "a later turn");
    }

    #[test]
    fn a_ring_is_forgotten_after_its_lifetime() {
        let (_dir, file) = file();
        assert!(file.note_ring("c", "%1", 100, Date(NOW)));
        assert!(file.note_ring("c", "%1", 100, Date(NOW + LIFETIME_SECONDS + 1.0)));
    }

    #[test]
    fn the_file_is_the_swift_shape() {
        let (_dir, file) = file();
        file.note_ring("C0", "%1", 1_790_000_600, Date(NOW));
        let json: serde_json::Value = serde_json::from_slice(&fs::read(file.path()).unwrap()).unwrap();
        assert_eq!(json["C0|%1"]["endedAt"], 1_790_000_600);
        assert_eq!(json["C0|%1"]["recordedAt"], NOW);
        // And one Swift wrote reads back.
        fs::write(file.path(), br#"{"C0|%3":{"endedAt":500,"recordedAt":800000000}}"#).unwrap();
        assert!(!file.note_ring("C0", "%3", 530, Date(NOW + 1.0)));
    }

    #[test]
    fn a_shape_it_doesnt_know_starts_over_and_rings() {
        let (_dir, file) = file();
        fs::write(file.path(), b"[1,2,3]").unwrap();
        assert!(file.note_ring("c", "%1", 100, Date(NOW)));
        assert!(!file.note_ring("c", "%1", 100, Date(NOW)));
    }
}
