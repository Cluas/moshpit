//! The pairings file (`push-pairings.json`): a JSON array of
//! [`PushPairing`], one per paired connection — the file every build and the
//! notification service read (`PushPairingFile`, round five's B23 fix).
//!
//! Read an entry at a time, and nothing it can't read is ever dropped:
//!
//! - entries this build decodes are the pairings it uses;
//! - entries it can't are written back as stored (as `JSONSerialization`
//!   rewrites them), after the others, on every save — unless the save
//!   re-pairs or unpairs that very connection;
//! - a file holding anything this build couldn't decode is copied to
//!   `<file>.backup` before the first write over it;
//! - a file that is there but can't be read at all is never written over.

use std::path::{Path, PathBuf};

use crate::foundation::date::Date;
use crate::foundation::json::{self, Dialect, Style, Value};
use crate::foundation::uuid::Uuid;
use crate::pairing::PushPairing;
use crate::store::{self, Read};

/// What one read of the file found.
#[derive(Debug, Default)]
struct Contents {
    /// The pairings this build decoded, in stored order.
    pairings: Vec<PushPairing>,
    /// The entries it couldn't, as stored.
    carried: Vec<Value>,
    /// The file's bytes; `None` when there is no file.
    raw: Option<Vec<u8>>,
    /// False when the bytes are not a JSON array at all.
    is_list: bool,
    /// The file is there but couldn't be read.
    unreadable: bool,
}

impl Contents {
    fn is_complete(&self) -> bool {
        self.carried.is_empty() && self.is_list && !self.unreadable
    }
}

/// The pairings file at a path of the platform's choosing.
#[derive(Debug, Clone)]
pub struct PushPairingFile {
    path: PathBuf,
}

impl PushPairingFile {
    /// The file's name in the iOS App Group container.
    pub const FILE_NAME: &'static str = "push-pairings.json";

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where a file this build couldn't fully read is kept, untouched,
    /// before anything is written over it.
    pub fn backup_path(&self) -> PathBuf {
        store::with_extension(&self.path, "backup")
    }

    /// The pairings this build can read. Never fails and never guesses:
    /// whatever it can't read is skipped here and kept on disk.
    pub fn read(&self) -> Vec<PushPairing> {
        self.contents().pairings
    }

    /// The pairing for one connection.
    pub fn pairing(&self, connection_id: Uuid) -> Option<PushPairing> {
        self.read()
            .into_iter()
            .find(|p| p.connection_id == connection_id)
    }

    /// Every secret this file holds, newest pairing first — the order the
    /// notification service tries them in (`secretsNewestFirst`).
    pub fn secrets_newest_first(&self) -> Vec<String> {
        newest_first(self.read())
            .into_iter()
            .map(|p| p.secret_hex)
            .collect()
    }

    /// Replace every pairing with `pairings`, keeping a copy of a file that
    /// held anything this build couldn't read. True once it reads back.
    pub fn write(&self, pairings: &[PushPairing]) -> bool {
        self.save(pairings, &[], &self.contents())
    }

    /// Replace (or add) the pairing for one connection. Re-pairing mints new
    /// material on purpose: the host's old conf stops decrypting.
    pub fn upsert(&self, pairing: &PushPairing) -> bool {
        let contents = self.contents();
        let mut pairings: Vec<PushPairing> = contents
            .pairings
            .iter()
            .filter(|p| p.connection_id != pairing.connection_id)
            .cloned()
            .collect();
        pairings.push(pairing.clone());
        let carried: Vec<Value> = contents
            .carried
            .iter()
            .filter(|e| !names(e, pairing.connection_id))
            .cloned()
            .collect();
        self.save(&pairings, &carried, &contents)
    }

    /// Forget one connection's pairing, and only that one. True once nothing
    /// for it is left — including when nothing was there: a file that is a
    /// list it could read and names the connection nowhere, decoded or
    /// carried, is left exactly as it is.
    pub fn remove(&self, connection_id: Uuid) -> bool {
        let contents = self.contents();
        if contents.is_list
            && !contents.unreadable
            && !contents
                .pairings
                .iter()
                .any(|p| p.connection_id == connection_id)
            && !contents.carried.iter().any(|e| names(e, connection_id))
        {
            return true;
        }
        let pairings: Vec<PushPairing> = contents
            .pairings
            .iter()
            .filter(|p| p.connection_id != connection_id)
            .cloned()
            .collect();
        let carried: Vec<Value> = contents
            .carried
            .iter()
            .filter(|e| !names(e, connection_id))
            .cloned()
            .collect();
        self.save(&pairings, &carried, &contents)
    }

    /// What the last read found: entries carried, whether the top level was
    /// a list, whether the file was unreadable (for the harness).
    #[doc(hidden)]
    pub fn inspect(&self) -> (usize, bool, bool) {
        let contents = self.contents();
        (
            contents.carried.len(),
            contents.is_list,
            contents.unreadable,
        )
    }

    fn contents(&self) -> Contents {
        let mut contents = Contents {
            is_list: true,
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
        let parsed = as_stored(&bytes);
        contents.raw = Some(bytes);
        let Some(Value::Array(entries)) = parsed else {
            contents.is_list = false;
            return contents;
        };
        for entry in entries {
            match json::decode_value::<PushPairing>(&entry) {
                Ok(pairing) => contents.pairings.push(pairing),
                Err(_) => contents.carried.push(entry),
            }
        }
        contents
    }

    /// Write `pairings`, then `carried`, over the file `contents` was read
    /// from — and read it back.
    fn save(&self, pairings: &[PushPairing], carried: &[Value], contents: &Contents) -> bool {
        if contents.unreadable {
            return false;
        }
        if !contents.is_complete()
            && let Some(raw) = &contents.raw
        {
            let now = Date::now().unix() as i64;
            if !store::secure(&self.path, raw, contents.is_list, now) {
                return false;
            }
        }
        let text = if carried.is_empty() {
            json::encode(&pairings)
        } else {
            // As the app does it: the encoded pairings read back through
            // JSONSerialization, the carried entries after them — with every
            // string as written (Fix (B264), see `as_stored`).
            let encoded = json::encode(&pairings);
            let mut list = match as_stored(encoded.as_bytes()) {
                Some(Value::Array(list)) => list,
                _ => Vec::new(),
            };
            list.extend(carried.iter().cloned());
            json::write_serialization(&Value::Array(list), Style::COMPACT)
        };
        if store::atomic_write(&self.path, text.as_bytes()).is_err() {
            return false;
        }
        let read_back = self.contents();
        read_back.pairings.len() == pairings.len() && read_back.carried.len() == carried.len()
    }
}

/// What `JSONSerialization.jsonObject(with:)` reads from `bytes` (an array or
/// an object at the top, every number checked), with its strings as stored:
/// `None` where it would throw.
///
/// Fix (B264): `JSONSerialization` drops a leading U+FEFF from every string
/// it parses, and the file was read (and, carrying an entry, written)
/// through it: a host label starting with one lost it at the next save of
/// any pairing. The bytes are still checked by its rules; the tree comes
/// from `JSONDecoder`'s, which keeps the string as stored (the store crate's
/// `serialization_tree`, Fix (bom-strip)).
fn as_stored(bytes: &[u8]) -> Option<Value> {
    json::parse(
        bytes,
        Dialect::Serialization {
            fragments_allowed: false,
        },
    )
    .ok()?;
    json::parse(bytes, Dialect::Decoder).ok()
}

/// Whether a carried entry is (some shape of) the pairing for `id`: its
/// `connectionId` is a string naming that UUID.
fn names(entry: &Value, id: Uuid) -> bool {
    entry
        .as_object()
        .and_then(|fields| fields.get("connectionId"))
        .and_then(Value::as_str)
        .and_then(Uuid::parse)
        == Some(id)
}

/// Pairings, newest first (stable for equal dates).
pub fn newest_first(mut pairings: Vec<PushPairing>) -> Vec<PushPairing> {
    pairings.sort_by(|a, b| {
        b.created_at
            .partial_cmp(&a.created_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pairings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    fn make(label: &str, relay: &str) -> PushPairing {
        PushPairing::make(Uuid::new_v4(), label, relay, Date::now())
    }

    /// A pairing entry in a shape this build can't decode — what a newer
    /// build might write (its secret moved under a versioned key).
    fn foreign(id: Uuid, host: &str) -> String {
        format!(
            r#"{{"connectionId":"{id}","hostLabel":"{host}","secret":{{"v":2,"hex":"{}"}},"relayURL":"https://push.example.org","createdAt":784000000.0}}"#,
            "ab".repeat(32)
        )
    }

    fn stored(pairing: &PushPairing) -> String {
        json::encode(pairing)
    }

    /// The connection ids in a pairings file, in stored order — read as
    /// plain JSON, so entries no build decodes are counted too.
    fn stored_ids(path: &Path) -> Vec<String> {
        let bytes = std::fs::read(path).unwrap();
        let Value::Array(entries) = json::parse(&bytes, Dialect::Decoder).unwrap() else {
            panic!("not a list")
        };
        entries
            .iter()
            .filter_map(|e| {
                e.as_object()?
                    .get("connectionId")?
                    .as_str()
                    .map(str::to_owned)
            })
            .collect()
    }

    fn sorted(mut ids: Vec<String>) -> Vec<String> {
        ids.sort();
        ids
    }

    #[test]
    fn pairing_a_host_keeps_an_entry_this_build_cant_decode_and_the_ones_it_can() {
        let dir = TempDir::new("pairings");
        let file = PushPairingFile::new(dir.path(PushPairingFile::FILE_NAME));
        let a = make("a", "https://a");
        let foreign_id = Uuid::new_v4();
        std::fs::write(
            file.path(),
            format!("[{},{}]", stored(&a), foreign(foreign_id, "b")),
        )
        .unwrap();
        assert_eq!(
            file.read()
                .iter()
                .map(|p| p.connection_id)
                .collect::<Vec<_>>(),
            [a.connection_id],
            "one bad entry hid every pairing"
        );

        let c = make("c", "https://c");
        assert!(file.upsert(&c));

        assert_eq!(
            sorted(stored_ids(file.path())),
            sorted(vec![
                a.connection_id.to_string(),
                c.connection_id.to_string(),
                foreign_id.to_string()
            ]),
            "pairing one host deleted the others"
        );
        assert_eq!(
            file.read()
                .iter()
                .map(|p| p.connection_id)
                .collect::<Vec<_>>(),
            [a.connection_id, c.connection_id]
        );
    }

    /// B264: a host label that starts with U+FEFF (pasted from a file with
    /// a byte order mark) is read as stored and kept through the saves of
    /// other pairings — with an entry carried too, the save that goes
    /// through `JSONSerialization`'s shape.
    #[test]
    fn a_label_starting_with_a_byte_order_mark_keeps_it() {
        let dir = TempDir::new("pairings");
        let file = PushPairingFile::new(dir.path(PushPairingFile::FILE_NAME));
        let marked = make("\u{FEFF}box", "https://a");
        assert!(file.upsert(&marked));
        assert_eq!(file.pairing(marked.connection_id).unwrap().host_label, "\u{FEFF}box");

        assert!(file.upsert(&make("b", "https://b")));
        assert_eq!(
            file.pairing(marked.connection_id).unwrap().host_label,
            "\u{FEFF}box",
            "another pairing's save dropped it"
        );

        let raw = std::fs::read_to_string(file.path()).unwrap();
        let with_foreign = format!(
            "{},{}]",
            raw.trim_end().strip_suffix(']').unwrap(),
            foreign(Uuid::new_v4(), "\u{FEFF}far")
        );
        std::fs::write(file.path(), with_foreign).unwrap();
        assert!(file.upsert(&make("c", "https://c")));
        assert_eq!(
            file.pairing(marked.connection_id).unwrap().host_label,
            "\u{FEFF}box",
            "a save carrying an entry dropped it"
        );
        let bytes = std::fs::read(file.path()).unwrap();
        let Value::Array(entries) = json::parse(&bytes, Dialect::Decoder).unwrap() else {
            panic!("not a list")
        };
        let labels: Vec<&str> = entries
            .iter()
            .filter_map(|e| e.as_object()?.get("hostLabel")?.as_str())
            .collect();
        assert!(labels.contains(&"\u{FEFF}box"), "{labels:?}");
        assert!(labels.contains(&"\u{FEFF}far"), "the carried entry lost it: {labels:?}");
    }

    #[test]
    fn unpairing_one_host_keeps_every_other_entry_readable_or_not() {
        let dir = TempDir::new("pairings");
        let file = PushPairingFile::new(dir.path(PushPairingFile::FILE_NAME));
        let a = make("a", "https://a");
        let c = make("c", "https://c");
        let foreign_id = Uuid::new_v4();
        std::fs::write(
            file.path(),
            format!(
                "[{},{},{}]",
                stored(&a),
                foreign(foreign_id, "b"),
                stored(&c)
            ),
        )
        .unwrap();

        assert!(file.remove(a.connection_id));
        assert_eq!(
            sorted(stored_ids(file.path())),
            sorted(vec![c.connection_id.to_string(), foreign_id.to_string()])
        );

        // Unpairing the host whose entry this build can't read removes it.
        assert!(file.remove(foreign_id));
        assert_eq!(stored_ids(file.path()), [c.connection_id.to_string()]);
    }

    /// Deleting a connection asks the file to forget its pairing whether or
    /// not it had one; a write would only give that a way to fail.
    #[test]
    fn forgetting_a_connection_the_file_doesnt_name_leaves_the_file_as_it_is() {
        let dir = TempDir::new("pairings");
        let file = PushPairingFile::new(dir.path(PushPairingFile::FILE_NAME));
        let a = make("a", "https://a");
        assert!(file.write(std::slice::from_ref(&a)));
        #[cfg(unix)]
        let identity =
            || std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(file.path()).unwrap());
        #[cfg(unix)]
        let before = identity();

        assert!(file.remove(Uuid::new_v4()));

        #[cfg(unix)]
        assert_eq!(
            identity(),
            before,
            "the file was written over to forget nothing"
        );
        assert_eq!(
            file.read()
                .iter()
                .map(|p| p.connection_id)
                .collect::<Vec<_>>(),
            [a.connection_id]
        );
        // And with no file at all there is nothing to forget either.
        let none = PushPairingFile::new(dir.path("absent.json"));
        assert!(none.remove(Uuid::new_v4()));
        assert!(!none.path().exists());
    }

    #[test]
    fn a_file_that_isnt_a_list_at_all_is_copied_untouched_before_the_first_pairing_is_written() {
        let dir = TempDir::new("pairings");
        let file = PushPairingFile::new(dir.path(PushPairingFile::FILE_NAME));
        let garbage = br#"{"pairings":"in a shape nobody here knows"}"#;
        std::fs::write(file.path(), garbage).unwrap();
        assert!(file.read().is_empty());

        let c = make("c", "https://c");
        assert!(file.upsert(&c));
        assert_eq!(
            std::fs::read(file.backup_path()).unwrap(),
            garbage,
            "the only copy of the old store was written over"
        );
        assert_eq!(
            file.read()
                .iter()
                .map(|p| p.connection_id)
                .collect::<Vec<_>>(),
            [c.connection_id]
        );

        // A second, different garbage file gets a copy of its own.
        std::fs::write(file.path(), b"also not a list").unwrap();
        assert!(file.upsert(&c));
        let copies: Vec<_> = std::fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|name| name.starts_with("push-pairings.json.backup-"))
            .collect();
        assert_eq!(copies.len(), 1);
        assert_eq!(
            std::fs::read(dir.path(&copies[0])).unwrap(),
            b"also not a list"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_is_there_but_cant_be_read_is_never_written_over() {
        let dir = TempDir::new("pairings");
        let file = PushPairingFile::new(dir.path(PushPairingFile::FILE_NAME));
        let a = make("a", "https://a");
        assert!(file.write(std::slice::from_ref(&a)));
        let before = std::fs::read(file.path()).unwrap();
        {
            let Some(_guard) = crate::store::testing::Unreadable::new(file.path()) else {
                return; // root reads a 000 file; nothing to test here
            };
            let c = make("c", "https://c");
            assert!(
                !file.upsert(&c),
                "a save went ahead over a store it couldn't read"
            );
            assert!(!file.remove(a.connection_id));
            assert!(file.read().is_empty());
        }
        assert_eq!(std::fs::read(file.path()).unwrap(), before);
    }

    #[test]
    fn what_the_store_writes_is_still_the_array_every_build_and_the_extension_decode() {
        let dir = TempDir::new("pairings");
        let file = PushPairingFile::new(dir.path(PushPairingFile::FILE_NAME));
        let a = make("a", "https://a");
        let b = make("b", "https://b");
        assert!(file.upsert(&a));
        assert!(file.upsert(&b));
        let decoded: Vec<PushPairing> = json::decode(&std::fs::read(file.path()).unwrap()).unwrap();
        assert_eq!(
            decoded.iter().map(|p| p.connection_id).collect::<Vec<_>>(),
            [a.connection_id, b.connection_id]
        );
        assert_eq!(
            decoded
                .iter()
                .map(|p| p.secret_hex.clone())
                .collect::<Vec<_>>(),
            [a.secret_hex, b.secret_hex]
        );
    }

    /// `PushPairingStorePersistenceTests`.
    #[test]
    fn re_pairing_one_host_replaces_it_and_leaves_the_others_alone() {
        let dir = TempDir::new("pairings");
        let file = PushPairingFile::new(dir.path(PushPairingFile::FILE_NAME));
        let a = PushPairing::make(Uuid::new_v4(), "a", "https://a", Date::from_unix(1_000.0));
        let b = PushPairing::make(Uuid::new_v4(), "b", "https://b", Date::from_unix(2_000.0));
        assert!(file.upsert(&a));
        assert!(file.upsert(&b));
        let again = PushPairing::make(a.connection_id, "a", "https://a2", Date::from_unix(3_000.0));
        assert!(file.upsert(&again));
        assert_eq!(file.read().len(), 2);
        assert_eq!(
            file.pairing(a.connection_id).unwrap().relay_url,
            "https://a2"
        );
        assert_eq!(
            file.pairing(b.connection_id).unwrap().relay_url,
            "https://b"
        );
        // Newest first is what the extension relies on when it tries keys.
        assert_eq!(
            file.secrets_newest_first(),
            [again.secret_hex, b.secret_hex]
        );
    }

    #[test]
    fn a_carried_entry_survives_in_the_shape_foundation_writes() {
        let dir = TempDir::new("pairings");
        let file = PushPairingFile::new(dir.path(PushPairingFile::FILE_NAME));
        let foreign_id = Uuid::new_v4();
        std::fs::write(file.path(), format!("[{}]", foreign(foreign_id, "b"))).unwrap();
        let a = make("a", "https://a/");
        assert!(file.upsert(&a));
        let text = std::fs::read_to_string(file.path()).unwrap();
        assert!(
            text.contains(r#""relayURL":"https:\/\/push.example.org""#),
            "{text}"
        );
        assert!(text.contains(r#""createdAt":784000000"#), "{text}");
        assert_eq!(file.read().len(), 1);
        assert_eq!(stored_ids(file.path()).len(), 2);
    }
}
