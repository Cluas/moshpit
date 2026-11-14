//! One JSON array under one preferences key, read an entry at a time
//! (`StoredList`, round two's B8 rule).
//!
//! Every list store — connections, SSH keys, shortcuts, terminal and app
//! themes — used to decode its array in one piece, and one piece it could
//! not decode read as an EMPTY list; the next add, edit or delete saved that
//! empty list over every entry. Any schema drift was enough: a field a newer
//! build started requiring, an enum case an older build never heard of, one
//! corrupt byte. So the read is per entry, and nothing it cannot read is
//! ever dropped:
//!
//! - entries this build decodes are the list it shows ([`Contents::elements`]);
//! - entries it cannot are kept exactly as stored and written back after
//!   the others on every save ([`Contents::carried`]);
//! - a value that is not a JSON array at all is copied untouched to
//!   `<key>.backup` before anything may be written over it — and so is the
//!   first list that had an entry skipped ([`secure`]).
//!
//! Nothing here writes unless the caller has established that the
//! preferences can be trusted ([`crate::readiness`]): a sealed file reads
//! as nothing stored, which is complete and needs no backup.
//!
//! Both halves follow `JSONSerialization` the way the Swift does: the stored
//! array must be one it parses (a malformed number anywhere makes the value
//! no list), each entry is written back out by it and only then decoded,
//! and a save that carries entries writes the whole list through it — so a
//! number keeps at most 17 significant digits as a `Double`, as in the app.
//!
//! Fix (bom-strip): one of its rules is not followed. `JSONSerialization`
//! drops a leading U+FEFF from every string it parses, keys included, and
//! the Swift stores parse every entry through it on every read
//! (`StoredList.swift:62,71,121`, `ConnectionStore.swift:164`): a name, host
//! or user that starts with one loses it at the next launch, and the next
//! save writes the change. Here the array is checked by its rules but read
//! with the strings as stored ([`serialization_tree`]).

use moshpit_seal::foundation::json::{self, Dialect, Style, Value};

use crate::host::Defaults;
use crate::plist::Plist;

/// How many distinct non-list values the backup holds at most, oldest
/// dropped first. Each takes something outside the app writing garbage under
/// the key; the cap only bounds a pathological loop of that.
pub const BACKUP_LIMIT: usize = 5;

/// One element type of a stored list: how the store's `JSONDecoder` reads
/// one entry and how its `JSONEncoder` writes the whole list.
pub trait Entry: Sized {
    /// Decode one entry, given as `JSONDecoder` parsed it.
    fn decode(value: &Value) -> Result<Self, String>;

    /// `encoder.encode(items)`: the JSON text of the list.
    fn encode_list(items: &[Self]) -> String;
}

/// What one read of a key found.
#[derive(Debug, Clone)]
pub struct Contents<T> {
    /// The entries this build decoded, in stored order.
    pub elements: Vec<T>,
    /// The entries it could not, as `JSONSerialization` read them. Written
    /// back after `elements` on every save.
    pub carried: Vec<Value>,
    /// Exactly what the key held; `None` when nothing was stored.
    pub stored: Option<Plist>,
    /// False when `stored` is not a JSON array at all — then none of it can
    /// be carried entry by entry.
    pub is_list: bool,
}

impl<T> Default for Contents<T> {
    fn default() -> Self {
        Self {
            elements: Vec::new(),
            carried: Vec::new(),
            stored: None,
            is_list: true,
        }
    }
}

impl<T> Contents<T> {
    /// Everything stored was read: nothing skipped, nothing unreadable.
    pub fn is_complete(&self) -> bool {
        self.carried.is_empty() && self.is_list
    }
}

/// Where values this build could not read are kept, untouched.
pub fn backup_key(key: &str) -> String {
    format!("{key}.backup")
}

/// Read `key`, decoding each entry on its own.
pub fn read<T: Entry>(defaults: &dyn Defaults, key: &str) -> Contents<T> {
    let mut contents = Contents::default();
    let Some(stored) = defaults.get(key) else {
        return contents;
    };
    let entries = match stored.as_data().and_then(serialization_tree) {
        Some(Value::Array(entries)) => entries,
        _ => {
            log::error!("{key}: stored value is not a list; nothing loaded, value kept");
            contents.stored = Some(stored);
            contents.is_list = false;
            return contents;
        }
    };
    contents.stored = Some(stored);
    let count = entries.len();
    for (index, entry) in entries.into_iter().enumerate() {
        match decode_entry::<T>(&entry) {
            Ok(element) => contents.elements.push(element),
            Err(error) => {
                log::error!("{key}: entry {index} of {count} unreadable, kept as stored: {error}");
                contents.carried.push(entry);
            }
        }
    }
    contents
}

/// What `JSONSerialization.jsonObject(with:)` reads from `data` (an array or
/// an object at the top; every number checked), with its strings as stored:
/// `None` where it would throw.
///
/// The tree comes from the `JSONDecoder` dialect, which keeps a leading
/// U+FEFF that `JSONSerialization` would drop (see the module's Fix
/// (bom-strip)); every number in it has passed `JSONSerialization`'s check,
/// so it writes back with [`json::write_serialization`].
pub fn serialization_tree(data: &[u8]) -> Option<Value> {
    json::parse(
        data,
        Dialect::Serialization {
            fragments_allowed: false,
        },
    )
    .ok()?;
    json::parse(data, Dialect::Decoder).ok()
}

/// One entry the way `StoredList.read` decodes it: written out by
/// `JSONSerialization` inside a one-entry array (a bare string or number is
/// a fragment it won't write alone), then decoded by `JSONDecoder`.
fn decode_entry<T: Entry>(entry: &Value) -> Result<T, String> {
    let single = json::write_serialization(&Value::Array(vec![entry.clone()]), Style::COMPACT);
    match json::parse(single.as_bytes(), Dialect::Decoder) {
        Ok(Value::Array(mut items)) if items.len() == 1 => T::decode(&items.remove(0)),
        Ok(_) => Err("empty entry".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// Make sure what `contents` was read from survives the next save over it,
/// and say whether that save is now safe (`StoredList.secure`).
///
/// A complete read needs nothing. A list with entries skipped carries them
/// through every save, so one untouched copy of the FIRST such list is all
/// the backup it needs. A value that is not a list at all is the only copy
/// of whatever it held, so each distinct one is kept. Only call this once
/// the preferences are known to be readable — it writes.
pub fn secure<T>(defaults: &dyn Defaults, key: &str, contents: &Contents<T>) -> bool {
    let Some(stored) = contents.stored.as_ref().filter(|_| !contents.is_complete()) else {
        return true;
    };
    let backup_key = backup_key(key);
    let mut backups = defaults
        .get(&backup_key)
        .and_then(|value| value.as_array().map(<[Plist]>::to_vec))
        .unwrap_or_default();
    let already_kept = backups.iter().any(|kept| kept.ns_equal(stored));
    if already_kept || (contents.is_list && !backups.is_empty()) {
        return true;
    }
    backups.push(stored.clone());
    if backups.len() > BACKUP_LIMIT {
        backups.drain(..backups.len() - BACKUP_LIMIT);
    }
    defaults.set(&backup_key, Some(Plist::Array(backups)));
    // Read back rather than assumed: a store may write over the original
    // only once this copy exists.
    let kept = defaults
        .get(&backup_key)
        .and_then(|value| {
            value
                .as_array()
                .map(|items| items.iter().any(|kept| kept.ns_equal(stored)))
        })
        .unwrap_or(false);
    if kept {
        log::info!("{key}: value as stored copied to {backup_key}");
    } else {
        log::error!("{key}: could not keep a copy in {backup_key}; not writing over it");
    }
    kept
}

/// `elements` encoded, then `carried` exactly as it was read
/// (`StoredList.encode`). `None` when the encoded elements don't read back
/// as a list — the caller writes nothing then.
pub fn encode<T: Entry>(elements: &[T], carried: &[Value]) -> Option<Vec<u8>> {
    let data = T::encode_list(elements);
    if carried.is_empty() {
        return Some(data.into_bytes());
    }
    let Some(Value::Array(mut list)) = serialization_tree(data.as_bytes()) else {
        return None;
    };
    list.extend(carried.iter().cloned());
    Some(json::write_serialization(&Value::Array(list), Style::COMPACT).into_bytes())
}

/// The members of a JSON object, for reading a carried entry the way the
/// stores do (`entry as? [String: Any]`).
pub fn object(entry: &Value) -> Option<&json::Object> {
    entry.as_object()
}

/// `value as? Bool` on what `JSONSerialization` returned: a Boolean, or a
/// number exactly 0 or 1.
pub fn json_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::Number(_) => match value.as_swift_double()? {
            0.0 => Some(false),
            1.0 => Some(true),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::host::MemoryDefaults;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub(crate) struct Item {
        pub name: String,
        pub count: i64,
    }

    impl Entry for Item {
        fn decode(value: &Value) -> Result<Self, String> {
            crate::codec::decode(value)
        }
        fn encode_list(items: &[Self]) -> String {
            crate::codec::encode(items)
        }
    }

    fn data(text: &str) -> Option<Plist> {
        Some(Plist::Data(text.as_bytes().to_vec()))
    }

    /// `StoredListTests`.
    #[test]
    fn each_entry_is_decoded_on_its_own_and_the_rest_carried_as_stored() {
        let defaults = MemoryDefaults::new();
        defaults.set(
            "items",
            data(r#"[{"name":"a","count":1},{"name":"b","count":"two"},7,{"name":"c","count":3}]"#),
        );
        let contents: Contents<Item> = read(&defaults, "items");
        assert_eq!(
            contents.elements,
            [
                Item {
                    name: "a".into(),
                    count: 1
                },
                Item {
                    name: "c".into(),
                    count: 3
                }
            ]
        );
        assert_eq!(contents.carried.len(), 2);
        assert!(contents.is_list);
        assert!(!contents.is_complete());

        let written = encode(&contents.elements, &contents.carried).unwrap();
        assert_eq!(
            String::from_utf8(written).unwrap(),
            r#"[{"count":1,"name":"a"},{"count":3,"name":"c"},{"name":"b","count":"two"},7]"#
        );
    }

    #[test]
    fn nothing_stored_is_a_complete_empty_read_that_needs_no_backup() {
        let defaults = MemoryDefaults::new();
        let contents: Contents<Item> = read(&defaults, "items");
        assert!(contents.elements.is_empty() && contents.is_complete());
        assert!(contents.stored.is_none());
        assert!(secure(&defaults, "items", &contents));
        assert_eq!(defaults.get("items.backup"), None);
    }

    #[test]
    fn a_value_that_isnt_a_list_is_kept_in_the_backup_once_per_distinct_value() {
        let defaults = MemoryDefaults::new();
        defaults.set("items", data(r#"{"version":2,"items":[]}"#));
        let contents: Contents<Item> = read(&defaults, "items");
        assert!(!contents.is_list && contents.elements.is_empty());
        assert!(secure(&defaults, "items", &contents));
        assert!(secure(
            &defaults,
            "items",
            &read::<Item>(&defaults, "items")
        ));
        assert_eq!(
            defaults
                .get("items.backup")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );

        defaults.set(
            "items",
            Some(Plist::Array(vec![
                Plist::String("a".into()),
                Plist::String("b".into()),
            ])),
        );
        assert!(secure(
            &defaults,
            "items",
            &read::<Item>(&defaults, "items")
        ));
        assert_eq!(
            defaults
                .get("items.backup")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn a_list_with_skips_is_backed_up_once_later_skips_ride_along_in_the_list() {
        let defaults = MemoryDefaults::new();
        let first = r#"[{"name":"a","count":1},{"name":"b"}]"#;
        defaults.set("items", data(first));
        assert!(secure(
            &defaults,
            "items",
            &read::<Item>(&defaults, "items")
        ));
        defaults.set(
            "items",
            data(r#"[{"name":"a","count":1},{"name":"b"},{"name":"c"}]"#),
        );
        assert!(secure(
            &defaults,
            "items",
            &read::<Item>(&defaults, "items")
        ));
        let backups = defaults.get("items.backup").unwrap();
        let backups = backups.as_array().unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0], data(first).unwrap());
    }

    #[test]
    fn the_backup_keeps_the_last_five_distinct_values() {
        let defaults = MemoryDefaults::new();
        for n in 0..7 {
            defaults.set("items", data(&format!("not a list {n}")));
            assert!(secure(
                &defaults,
                "items",
                &read::<Item>(&defaults, "items")
            ));
        }
        let backups = defaults.get("items.backup").unwrap();
        let backups = backups.as_array().unwrap();
        assert_eq!(backups.len(), BACKUP_LIMIT);
        assert_eq!(backups[0], data("not a list 2").unwrap());
        assert_eq!(backups[4], data("not a list 6").unwrap());
    }

    /// Fix (bom-strip): Swift reads this name as "a".
    #[test]
    fn a_leading_byte_order_mark_survives_the_read_and_a_carried_save() {
        let defaults = MemoryDefaults::new();
        defaults.set(
            "items",
            data("[{\"name\":\"\u{FEFF}a\",\"count\":1},{\"name\":\"\u{FEFF}b\"}]"),
        );
        let contents: Contents<Item> = read(&defaults, "items");
        assert_eq!(contents.elements[0].name, "\u{FEFF}a");
        let written =
            String::from_utf8(encode(&contents.elements, &contents.carried).unwrap()).unwrap();
        assert_eq!(
            written,
            "[{\"count\":1,\"name\":\"\u{FEFF}a\"},{\"name\":\"\u{FEFF}b\"}]"
        );
    }

    #[test]
    fn negative_infinity_is_no_list() {
        // Fix (negative-overflow): Swift reads -1e400 as -infinity and then
        // crashes writing the entry back out (StoredList.swift:71).
        let defaults = MemoryDefaults::new();
        defaults.set(
            "items",
            data(r#"[{"name":"a","count":1},{"name":"b","count":-1e400}]"#),
        );
        let contents: Contents<Item> = read(&defaults, "items");
        assert!(!contents.is_list);
        assert!(contents.elements.is_empty());
    }

    #[test]
    fn a_number_only_its_decoder_takes_reads_as_an_int() {
        let defaults = MemoryDefaults::new();
        defaults.set(
            "items",
            data(r#"[{"name":"a","count":2.0},{"name":"b","count":1e2}]"#),
        );
        let contents: Contents<Item> = read(&defaults, "items");
        assert_eq!(
            contents
                .elements
                .iter()
                .map(|i| i.count)
                .collect::<Vec<_>>(),
            [2, 100]
        );
    }

    #[test]
    fn carried_booleans_read_as_swift_bridges_them() {
        assert_eq!(json_bool(&Value::Bool(true)), Some(true));
        let number = |text: &str| Value::Number(json::Number::from_literal(text));
        assert_eq!(json_bool(&number("1")), Some(true));
        assert_eq!(json_bool(&number("0.0")), Some(false));
        assert_eq!(json_bool(&number("2")), None);
        assert_eq!(json_bool(&Value::String("true".into())), None);
    }
}
