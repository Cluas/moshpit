//! The prompts standing — who is waiting, per connection (`PushStanding`).
//!
//! Shared between the app and the notification service so both make the
//! same call on the one question that decides whether a notification makes
//! a sound: is this the moment "nobody is waiting on you" became "someone
//! is"? Only that 0→1 edge rings. An entry stands until its question is
//! answered or resolved (a poll, the turn's `done` push) or its pane or
//! agent ends; past [`LIFETIME_SECONDS`] (six hours, B660) it's taken for
//! stale. It used to expire at fifteen minutes, and a question still
//! waiting lost its card to the app opening for another host. An entry 1.0.8
//! recorded still expires at fifteen minutes ([`OLDER_LIFETIME_SECONDS`],
//! B727): this build's entries say how long they stand
//! ([`Entry::lifetime`]), and one that doesn't is that build's.
//!
//! The file (`standing-attention.json`) is a JSON object, connection id →
//! array of [`Entry`], read an entry at a time with the pairings file's
//! rules: entries this build can't decode are carried through every save
//! (unless the save replaces or clears that very pane, or they have plainly
//! expired) and never count toward the edge; a connection whose value is no
//! list is kept as it is; a file holding either is copied to
//! `<file>.backup` before the first write over it; an unreadable file is
//! never written over.
//!
//! Every read-modify-write holds the exclusive lock on `<file>.lock` the
//! finishes take (B347): the app and the notification service both write
//! this file, and without it the later writer dropped the earlier one's
//! change — a clear undone (a question standing 15 min, the next one silent)
//! or a pushed prompt's entry lost.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::foundation::date::Date;
use crate::foundation::json::{self, Dialect, Object, Style, Value};
use crate::store::{self, Read};

/// How long an unrefreshed entry keeps counting: six hours, a safety cap
/// (B660); its question's answer, its pane's or agent's end take it first.
pub const LIFETIME_SECONDS: f64 = 6.0 * 3600.0;

/// How long an entry 1.0.8 recorded stands — one without a
/// [`Entry::lifetime`]: that build's fifteen minutes (B727). It took a
/// question unanswered that long for answered at the desk; read with six
/// hours, one answered there while no app ran held the edge, and the next
/// prompt came in silent.
pub const OLDER_LIFETIME_SECONDS: f64 = 15.0 * 60.0;

/// One standing prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub pane: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Where the pane lives ("host · sess · 1: win"), to re-render a summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Episode start (the host's `@moshpit_since`) — the question's identity.
    pub since: i64,
    /// When this entry was recorded on the phone, for expiry.
    #[serde(rename = "recordedAt")]
    pub recorded_at: Date,
    /// How long it stands, in seconds: [`LIFETIME_SECONDS`], set when this
    /// build records it ([`PushStandingFile::note_standing`]). None on one
    /// 1.0.8 recorded, which keeps that build's [`OLDER_LIFETIME_SECONDS`]
    /// (B727).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<f64>,
}

impl Entry {
    /// Whether it still counts at `now`: younger than its own lifetime
    /// (never past this build's), and not from more than a minute in the
    /// future (a clock that jumped).
    pub fn is_live(&self, now: Date) -> bool {
        let lasts = self
            .lifetime
            .unwrap_or(OLDER_LIFETIME_SECONDS)
            .min(LIFETIME_SECONDS);
        now.seconds_since(self.recorded_at) < lasts && self.recorded_at < now.adding(60.0)
    }
}

/// Whether something recorded at `recorded_at` still counts under the
/// longest lifetime any build gives it: what an entry this build can't read
/// is held to.
pub fn is_live(recorded_at: Date, now: Date) -> bool {
    now.seconds_since(recorded_at) < LIFETIME_SECONDS && recorded_at < now.adding(60.0)
}

/// `raw` without expired entries, and without connections left empty.
pub fn prune(raw: &BTreeMap<String, Vec<Entry>>, now: Date) -> BTreeMap<String, Vec<Entry>> {
    raw.iter()
        .filter_map(|(conn, entries)| {
            let live: Vec<Entry> = entries.iter().filter(|e| e.is_live(now)).cloned().collect();
            (!live.is_empty()).then(|| (conn.clone(), live))
        })
        .collect()
}

#[derive(Debug, Default)]
struct Contents {
    entries: BTreeMap<String, Vec<Entry>>,
    carried: BTreeMap<String, Vec<Value>>,
    odd: BTreeMap<String, Value>,
    raw: Option<Vec<u8>>,
    is_map: bool,
    unreadable: bool,
}

impl Contents {
    fn is_complete(&self) -> bool {
        self.carried.is_empty() && self.odd.is_empty() && self.is_map && !self.unreadable
    }
}

/// The standing set's file, at a path of the platform's choosing
/// (`PushStandingFile`).
#[derive(Debug, Clone)]
pub struct PushStandingFile {
    path: PathBuf,
}

impl PushStandingFile {
    /// The file's name in the iOS App Group container.
    pub const FILE_NAME: &'static str = "standing-attention.json";

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn backup_path(&self) -> PathBuf {
        store::with_extension(&self.path, "backup")
    }

    /// conn → the entries this build can read, pruned of expired ones.
    pub fn read(&self, now: Date) -> BTreeMap<String, Vec<Entry>> {
        prune(&self.contents().entries, now)
    }

    /// Everything standing for a connection, newest first.
    pub fn standing(&self, conn: &str, now: Date) -> Vec<Entry> {
        let mut entries = self.read(now).remove(conn).unwrap_or_default();
        entries.sort_by(|a, b| {
            b.recorded_at
                .partial_cmp(&a.recorded_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries
    }

    /// Record a prompt as standing. Returns whether this was the 0→1 edge for
    /// its connection — the caller's licence to make a sound. Re-recording
    /// the same episode (same pane, same `since`) refreshes its expiry and is
    /// never an edge: the push and the local announcement of one prompt both
    /// land here, and only one of them may ring.
    pub fn note_standing(&self, conn: &str, entry: Entry, now: Date) -> bool {
        // Recorded by this build: it stands this build's lifetime (B727).
        let entry = Entry {
            lifetime: Some(LIFETIME_SECONDS),
            ..entry
        };
        let _lock = store::Lock::take(&self.path);
        let contents = self.contents();
        let mut all = prune(&contents.entries, now);
        let mut entries = all.remove(conn).unwrap_or_default();
        // Nobody else waiting (B450): the pane's own earlier entry is the
        // question this one replaces, not another wait. It counted, and a
        // second question on a pane rang nothing while the first one's
        // entry stood (no app running to clear it). The same question —
        // the pushed and the local copy name one `since` — is a repeat.
        let was_empty = entries.iter().all(|e| e.pane == entry.pane);
        let is_repeat = entries
            .iter()
            .any(|e| e.pane == entry.pane && e.since == entry.since);
        entries.retain(|e| e.pane != entry.pane);
        let pane = entry.pane.clone();
        entries.push(entry);
        all.insert(conn.to_owned(), entries);
        let mut carried = contents.carried.clone();
        if let Some(items) = carried.get_mut(conn) {
            items.retain(|item| pane_of(item) != Some(pane.as_str()));
        }
        let mut odd = contents.odd.clone();
        odd.remove(conn);
        self.save(&all, &carried, &odd, &contents, now);
        was_empty && !is_repeat
    }

    /// A pane stopped waiting. Returns the entries still standing for that
    /// connection, so the caller can re-render or withdraw the summary.
    pub fn clear(&self, conn: &str, pane: &str, now: Date) -> Vec<Entry> {
        let _lock = store::Lock::take(&self.path);
        let contents = self.contents();
        let mut all = prune(&contents.entries, now);
        let mut entries = all.remove(conn).unwrap_or_default();
        entries.retain(|e| e.pane != pane);
        if !entries.is_empty() {
            all.insert(conn.to_owned(), entries.clone());
        }
        let mut carried = contents.carried.clone();
        if let Some(items) = carried.get_mut(conn) {
            items.retain(|item| pane_of(item) != Some(pane));
        }
        self.save(&all, &carried, &contents.odd, &contents, now);
        entries
    }

    /// The host said `pane`'s turn ended at `ended_at` (its clock, B636):
    /// the question it asked up to then stops waiting — not one the pane
    /// asked after, which a `done` that came again (a relay can send an
    /// envelope twice) used to clear. Returns what still stands.
    pub fn clear_ended(&self, conn: &str, pane: &str, ended_at: i64, now: Date) -> Vec<Entry> {
        let _lock = store::Lock::take(&self.path);
        let contents = self.contents();
        let mut all = prune(&contents.entries, now);
        let mut entries = all.remove(conn).unwrap_or_default();
        let before = entries.len();
        entries.retain(|e| e.pane != pane || e.since > ended_at);
        let cleared = entries.len() != before;
        if !entries.is_empty() {
            all.insert(conn.to_owned(), entries.clone());
        }
        let mut carried = contents.carried.clone();
        if cleared && let Some(items) = carried.get_mut(conn) {
            items.retain(|item| pane_of(item) != Some(pane));
        }
        self.save(&all, &carried, &contents.odd, &contents, now);
        entries
    }

    /// Forget a connection's whole standing set.
    pub fn clear_all(&self, conn: &str, now: Date) {
        let _lock = store::Lock::take(&self.path);
        let contents = self.contents();
        let mut all = prune(&contents.entries, now);
        all.remove(conn);
        let mut carried = contents.carried.clone();
        carried.remove(conn);
        let mut odd = contents.odd.clone();
        odd.remove(conn);
        self.save(&all, &carried, &odd, &contents, now);
    }

    fn contents(&self) -> Contents {
        let mut contents = Contents {
            is_map: true,
            ..Contents::default()
        };
        let bytes = match store::read(&self.path) {
            Read::Absent => return contents,
            Read::Unreadable => {
                contents.unreadable = true;
                return contents;
            }
            Read::Bytes(bytes) => bytes,
        };
        let parsed = json::parse(
            &bytes,
            Dialect::Serialization {
                fragments_allowed: false,
            },
        );
        contents.raw = Some(bytes);
        let Ok(Value::Object(map)) = parsed else {
            contents.is_map = false;
            return contents;
        };
        for (conn, value) in map.iter() {
            let Value::Array(items) = value else {
                contents.odd.insert(conn.to_owned(), value.clone());
                continue;
            };
            for item in items {
                match json::decode_value::<Entry>(item) {
                    Ok(entry) => contents
                        .entries
                        .entry(conn.to_owned())
                        .or_default()
                        .push(entry),
                    Err(_) => contents
                        .carried
                        .entry(conn.to_owned())
                        .or_default()
                        .push(item.clone()),
                }
            }
        }
        contents
    }

    fn save(
        &self,
        all: &BTreeMap<String, Vec<Entry>>,
        carried: &BTreeMap<String, Vec<Value>>,
        odd: &BTreeMap<String, Value>,
        contents: &Contents,
        now: Date,
    ) -> bool {
        if contents.unreadable {
            return false;
        }
        if !contents.is_complete()
            && let Some(raw) = &contents.raw
            && !store::secure(&self.path, raw, contents.is_map, Date::now().unix() as i64)
        {
            return false;
        }
        let carried: BTreeMap<&String, Vec<&Value>> = carried
            .iter()
            .map(|(conn, items)| {
                (
                    conn,
                    items
                        .iter()
                        .filter(|item| !has_expired(item, now))
                        .collect::<Vec<_>>(),
                )
            })
            .filter(|(_, items)| !items.is_empty())
            .collect();
        let text = if carried.is_empty() && odd.is_empty() {
            json::encode(all)
        } else {
            let encoded = json::encode(all);
            let mut map = match json::parse(
                encoded.as_bytes(),
                Dialect::Serialization {
                    fragments_allowed: false,
                },
            ) {
                Ok(Value::Object(map)) => map,
                _ => Object::new(),
            };
            for (conn, items) in &carried {
                let mut list = match map.remove(conn.as_str()) {
                    Some(Value::Array(list)) => list,
                    _ => Vec::new(),
                };
                list.extend(items.iter().map(|&item| item.clone()));
                map.insert(conn.as_str(), Value::Array(list));
            }
            for (conn, value) in odd {
                if !map.contains_key(conn) {
                    map.insert(conn.as_str(), value.clone());
                }
            }
            json::write_serialization(&Value::Object(map), Style::COMPACT)
        };
        store::atomic_write(&self.path, text.as_bytes()).is_ok()
    }
}

/// The pane a carried entry names, if it names one as a string.
fn pane_of(item: &Value) -> Option<&str> {
    item.as_object()?.get("pane")?.as_str()
}

/// A carried entry whose `recordedAt` reads, in the shape this build writes
/// it, as past its lifetime — stale under any build's rule.
fn has_expired(item: &Value, now: Date) -> bool {
    let Some(seconds) = item
        .as_object()
        .and_then(|o| o.get("recordedAt"))
        .and_then(Value::as_swift_double)
    else {
        return false;
    };
    !is_live(Date(seconds), now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    const CONN: &str = "11111111-2222-3333-4444-555555555555";
    const OTHER: &str = "66666666-7777-8888-9999-000000000000";

    fn entry(pane: &str, since: i64, at: Date) -> Entry {
        Entry {
            pane: pane.into(),
            agent: Some("claude".into()),
            title: Some("may I".into()),
            location: Some("m1 · work".into()),
            since,
            recorded_at: at,
            lifetime: Some(LIFETIME_SECONDS),
        }
    }

    fn now() -> Date {
        Date::now()
    }

    fn file(dir: &TempDir) -> PushStandingFile {
        PushStandingFile::new(dir.path(PushStandingFile::FILE_NAME))
    }

    /// An entry as the store writes it.
    fn stored(entry: &Entry) -> String {
        json::encode(entry)
    }

    /// An entry this build can't decode — `since` as a string — with the
    /// `pane` and `recordedAt` it writes.
    fn foreign(pane: &str, at: Date) -> String {
        format!(
            r#"{{"pane":"{pane}","since":"1756000000-2","recordedAt":{}}}"#,
            at.since_reference()
        )
    }

    fn stored_panes(path: &Path, conn: &str) -> Vec<String> {
        let bytes = std::fs::read(path).unwrap();
        let value = json::parse(&bytes, Dialect::Decoder).unwrap();
        value
            .as_object()
            .and_then(|map| map.get(conn))
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(|item| pane_of(item).map(str::to_owned))
            .collect()
    }

    // MARK: - PushStandingTests

    #[test]
    fn the_first_waiting_prompt_is_the_edge_the_second_is_not() {
        let dir = TempDir::new("standing");
        let file = file(&dir);
        assert!(
            file.note_standing(CONN, entry("%1", 1_756_000_000, now()), now()),
            "an empty set gaining its first prompt IS the moment worth a sound"
        );
        assert!(
            !file.note_standing(CONN, entry("%2", 1_756_000_000, now()), now()),
            "a second agent joining changes a count"
        );
        assert_eq!(file.standing(CONN, now()).len(), 2);
    }

    #[test]
    fn the_push_and_the_local_path_cannot_both_ring_for_one_prompt() {
        let dir = TempDir::new("standing");
        let file = file(&dir);
        assert!(file.note_standing(CONN, entry("%1", 100, now()), now()));
        assert!(
            !file.note_standing(CONN, entry("%1", 100, now()), now()),
            "the second arrival of the SAME question must see 'already standing'"
        );
    }

    /// B450: a second question on a pane whose first one's entry still
    /// stands (no app running to clear it) is a new wait: it rings. It
    /// used to count the first as someone waiting, and came in silent.
    /// The same question again (the pushed and the local copy, one
    /// `since`) is still a repeat, and another pane waiting still holds
    /// the edge.
    #[test]
    fn a_second_question_on_the_same_pane_rings() {
        let dir = TempDir::new("standing-second");
        let s = PushStandingFile::new(dir.path(PushStandingFile::FILE_NAME));
        let now = Date::from_unix(1_790_000_000.0);
        assert!(s.note_standing("C", entry("%1", 100, now), now));
        assert!(!s.note_standing("C", entry("%1", 100, now), now), "the same question again");
        assert!(s.note_standing("C", entry("%1", 400, now), now), "a second question on the pane");
        assert_eq!(s.standing("C", now).len(), 1);
        assert!(!s.note_standing("C", entry("%2", 500, now), now), "another pane waits");
        assert!(!s.note_standing("C", entry("%1", 600, now), now), "%2 still waits");
    }

    #[test]
    fn answering_everyone_re_arms_the_edge() {
        let dir = TempDir::new("standing");
        let file = file(&dir);
        file.note_standing(CONN, entry("%1", 1_756_000_000, now()), now());
        file.clear(CONN, "%1", now());
        assert!(
            file.note_standing(CONN, entry("%1", 200, now()), now()),
            "once nobody is waiting, the next prompt is a fresh interruption again"
        );
    }

    #[test]
    fn clearing_one_pane_reports_who_is_still_waiting() {
        let dir = TempDir::new("standing");
        let file = file(&dir);
        file.note_standing(CONN, entry("%1", 1, now()), now());
        file.note_standing(CONN, entry("%2", 1, now()), now());
        let left = file.clear(CONN, "%1", now());
        assert_eq!(
            left.iter().map(|e| e.pane.as_str()).collect::<Vec<_>>(),
            ["%2"]
        );
    }

    #[test]
    fn stale_entries_stop_counting() {
        // One instant throughout: two reads of the clock a tick apart would
        // move the one-minute edge under the last assertion.
        let now = now();
        let old = now.adding(-LIFETIME_SECONDS - 60.0);
        let raw = BTreeMap::from([(CONN.to_owned(), vec![entry("%9", 1, old)])]);
        assert!(prune(&raw, now).is_empty());
        // And an entry stamped in the future is a moved clock, not a wait.
        let future = BTreeMap::from([(CONN.to_owned(), vec![entry("%9", 1, now.adding(3600.0))])]);
        assert!(prune(&future, now).is_empty());
        assert!(is_live(now.adding(59.0), now));
        assert!(!is_live(now.adding(60.0), now));
    }

    /// B727: an entry 1.0.8 recorded says nothing of how long it stands,
    /// and keeps that build's fifteen minutes. One twenty minutes old was
    /// answered at the desk long since — no app ran to clear it — and held
    /// the edge for six hours: the next prompt came in silent. One this
    /// build records stands six hours (B660), and the file says so.
    #[test]
    fn an_entry_1_0_8_recorded_keeps_its_fifteen_minutes() {
        let dir = TempDir::new("standing-108");
        let file = file(&dir);
        let now = Date::from_unix(1_790_000_000.0);
        // As 1.0.8's PushStanding wrote one (JSONEncoder: the date as
        // seconds since 2001, a nil field left out), `minutes` old.
        let older = |pane: &str, minutes: f64| {
            format!(
                r#"{{"pane":"{pane}","agent":"claude","title":"may I","since":1789990000,"recordedAt":{}}}"#,
                now.adding(-minutes * 60.0).since_reference()
            )
        };
        std::fs::write(
            file.path(),
            format!(
                r#"{{"{CONN}":[{}],"{OTHER}":[{}]}}"#,
                older("%1", 20.0),
                older("%2", 10.0)
            ),
        )
        .unwrap();
        assert!(
            file.standing(CONN, now).is_empty(),
            "20 minutes old: past 1.0.8's fifteen"
        );
        assert_eq!(file.standing(OTHER, now).len(), 1, "10 minutes old");
        assert!(
            file.note_standing(CONN, entry("%3", 1, now), now),
            "nobody waits on CONN: the next prompt rings"
        );
        assert!(
            !file.note_standing(OTHER, entry("%4", 1, now), now),
            "%2 still waits"
        );

        let panes = |conn: &str, at: Date| -> Vec<String> {
            file.standing(conn, at)
                .into_iter()
                .map(|e| e.pane)
                .collect()
        };
        let later = now.adding(LIFETIME_SECONDS - 60.0);
        assert_eq!(panes(CONN, later), ["%3"], "this build's stands six hours");
        assert_eq!(panes(OTHER, later), ["%4"], "1.0.8's %2 is gone");
        assert!(panes(CONN, now.adding(LIFETIME_SECONDS + 1.0)).is_empty());
        let bytes = std::fs::read(file.path()).unwrap();
        let stored = json::parse(&bytes, Dialect::Decoder).unwrap();
        let lifetime = stored
            .as_object()
            .and_then(|m| m.get(CONN))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_object)
            .and_then(|o| o.get("lifetime"))
            .and_then(Value::as_swift_double);
        assert_eq!(
            lifetime,
            Some(LIFETIME_SECONDS),
            "the file says how long it stands"
        );
    }

    #[test]
    fn an_entry_this_build_cant_decode_neither_wipes_the_set_nor_fakes_an_edge() {
        let dir = TempDir::new("standing");
        let file = file(&dir);
        let waiting = stored(&entry("%2", 1_756_000_000, now()));
        std::fs::write(
            file.path(),
            format!(
                r#"{{"{CONN}":[{waiting},{}],"{OTHER}":[{waiting}]}}"#,
                foreign("%3", now())
            ),
        )
        .unwrap();

        assert!(
            !file.note_standing(CONN, entry("%1", 1, now()), now()),
            "%2 was already waiting"
        );
        assert_eq!(
            file.standing(OTHER, now())
                .iter()
                .map(|e| e.pane.as_str())
                .collect::<Vec<_>>(),
            ["%2"],
            "recording a prompt on one connection wiped another's"
        );
        let mut panes: Vec<String> = file
            .standing(CONN, now())
            .into_iter()
            .map(|e| e.pane)
            .collect();
        panes.sort();
        assert_eq!(panes, ["%1", "%2"]);
        assert!(
            stored_panes(file.path(), CONN).contains(&"%3".to_owned()),
            "the entry this build can't read was dropped"
        );
    }

    // MARK: - PushStandingFileTests

    #[test]
    fn recording_a_prompt_keeps_an_entry_this_build_cant_decode_and_every_other_connections() {
        let dir = TempDir::new("standing");
        let file = file(&dir);
        std::fs::write(
            file.path(),
            format!(
                r#"{{"{CONN}":[{},{}],"{OTHER}":[{}]}}"#,
                stored(&entry("%2", 1_756_000_000, now())),
                foreign("%3", now()),
                stored(&entry("%5", 1_756_000_000, now()))
            ),
        )
        .unwrap();
        assert_eq!(
            file.read(now())[CONN]
                .iter()
                .map(|e| e.pane.as_str())
                .collect::<Vec<_>>(),
            ["%2"],
            "one bad entry hid the whole set"
        );

        assert!(!file.note_standing(CONN, entry("%1", 1_756_000_000, now()), now()));

        let mut panes = stored_panes(file.path(), CONN);
        panes.sort();
        assert_eq!(panes, ["%1", "%2", "%3"]);
        assert_eq!(stored_panes(file.path(), OTHER), ["%5"]);
        assert_eq!(
            file.read(now())[OTHER]
                .iter()
                .map(|e| e.pane.as_str())
                .collect::<Vec<_>>(),
            ["%5"]
        );
    }

    #[test]
    fn clearing_a_pane_removes_its_entry_even_in_a_shape_this_build_cant_decode() {
        let dir = TempDir::new("standing");
        let file = file(&dir);
        std::fs::write(
            file.path(),
            format!(
                r#"{{"{CONN}":[{},{}]}}"#,
                stored(&entry("%2", 1_756_000_000, now())),
                foreign("%3", now())
            ),
        )
        .unwrap();
        assert_eq!(
            file.clear(CONN, "%3", now())
                .iter()
                .map(|e| e.pane.as_str())
                .collect::<Vec<_>>(),
            ["%2"]
        );
        assert_eq!(stored_panes(file.path(), CONN), ["%2"]);

        // And an entry nobody can read that has plainly expired goes too.
        std::fs::write(
            file.path(),
            format!(
                r#"{{"{CONN}":[{}]}}"#,
                foreign("%4", now().adding(-LIFETIME_SECONDS - 60.0))
            ),
        )
        .unwrap();
        file.note_standing(CONN, entry("%1", 1, now()), now());
        assert_eq!(stored_panes(file.path(), CONN), ["%1"]);
    }

    #[test]
    fn a_file_that_isnt_a_map_at_all_is_copied_untouched_before_the_first_prompt_is_written() {
        let dir = TempDir::new("standing");
        let file = file(&dir);
        let garbage = br#"["standing", "in a shape nobody here knows"]"#;
        std::fs::write(file.path(), garbage).unwrap();
        assert!(file.read(now()).is_empty());
        file.note_standing(CONN, entry("%1", 1, now()), now());
        assert_eq!(std::fs::read(file.backup_path()).unwrap(), garbage);
        assert_eq!(
            file.read(now())[CONN]
                .iter()
                .map(|e| e.pane.as_str())
                .collect::<Vec<_>>(),
            ["%1"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_is_there_but_cant_be_read_is_never_written_over() {
        let dir = TempDir::new("standing");
        let file = file(&dir);
        file.note_standing(CONN, entry("%1", 1, now()), now());
        let before = std::fs::read(file.path()).unwrap();
        {
            let Some(_guard) = crate::store::testing::Unreadable::new(file.path()) else {
                return;
            };
            file.note_standing(OTHER, entry("%2", 1, now()), now());
            file.clear(CONN, "%1", now());
            file.clear_all(CONN, now());
        }
        assert_eq!(std::fs::read(file.path()).unwrap(), before);
    }

    #[test]
    fn a_connection_whose_value_is_no_list_is_kept_until_it_is_replaced() {
        let dir = TempDir::new("standing");
        let file = file(&dir);
        std::fs::write(
            file.path(),
            format!(r#"{{"{OTHER}":"what","{CONN}":{{"x":1}}}}"#),
        )
        .unwrap();
        file.clear(CONN, "%1", now());
        let text = std::fs::read_to_string(file.path()).unwrap();
        assert!(text.contains(&format!(r#""{OTHER}":"what""#)), "{text}");
        assert!(
            text.contains(&format!(r#""{CONN}":{{"x":1}}"#)),
            "clearing one pane keeps an odd value: {text}"
        );
        file.note_standing(CONN, entry("%1", 1, now()), now());
        assert_eq!(stored_panes(file.path(), CONN), ["%1"]);
        file.clear_all(OTHER, now());
        let text = std::fs::read_to_string(file.path()).unwrap();
        assert!(!text.contains(OTHER), "{text}");
    }

    /// B347: the app and the notification service write this file at the
    /// same time. One writer notes and at once clears its prompts, the other
    /// notes its own, each through its own open of the file: nothing either
    /// wrote may be lost, and nothing cleared may come back. Without the
    /// lock the later writer wrote back what it had read, dropping the
    /// other's change.
    #[test]
    fn two_writers_at_once_lose_nothing() {
        const ROUNDS: usize = 150;
        let dir = TempDir::new("standing");
        let path = dir.path(PushStandingFile::FILE_NAME);
        let start = std::sync::Arc::new(std::sync::Barrier::new(2));
        let answering = {
            let (path, start) = (path.clone(), start.clone());
            std::thread::spawn(move || {
                let file = PushStandingFile::new(path);
                start.wait();
                for i in 0..ROUNDS {
                    let pane = format!("%a{i}");
                    file.note_standing(CONN, entry(&pane, 1, now()), now());
                    file.clear(CONN, &pane, now());
                }
            })
        };
        let asking = {
            let (path, start) = (path.clone(), start.clone());
            std::thread::spawn(move || {
                let file = PushStandingFile::new(path);
                start.wait();
                for i in 0..ROUNDS {
                    file.note_standing(OTHER, entry(&format!("%b{i}"), 1, now()), now());
                }
            })
        };
        answering.join().unwrap();
        asking.join().unwrap();
        let file = PushStandingFile::new(path);
        assert_eq!(
            file.standing(OTHER, now()).len(),
            ROUNDS,
            "a prompt noted at the same time as another write was lost"
        );
        assert!(
            file.standing(CONN, now()).is_empty(),
            "a cleared prompt came back: {:?}",
            file.standing(CONN, now())
                .iter()
                .map(|e| &e.pane)
                .collect::<Vec<_>>()
        );
    }
}
