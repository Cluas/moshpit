//! Every key `AppSettings` (Moshpit/Models/AppSettings.swift) keeps
//! through this crate's settings store is a setting of its schema
//! ([`settings::ALL`]) (B745).
//!
//! The store keeps only the settings of its schema: a write under any other
//! key is refused and a read finds nothing, so the getter falls back to its
//! default. Nothing fails, the setting just never sticks. B510's
//! `hostSetupUnpaired` and B704's `appIconPickedByHand` both shipped that
//! way. This reads the Swift source, so a setting added there is checked
//! the day it is written, without anyone remembering to list it here.
//!
//! The rules, per `moshpit.…` key literal in the file:
//! - a key read or written through the store (`get`, `put`, `backing`) is
//!   one of the schema's;
//! - a key kept beside the settings, in the preferences directly
//!   (`store.…` / `defaults.…`, as B704's flag now is), is listed in
//!   [`DEFAULTS_SIDE`] and is no setting of the schema's;
//! - no key is both (read through the store, written beside it: the read
//!   never sees the write);
//! - every key of the schema is one `AppSettings` uses, or is listed in
//!   [`SCHEMA_ONLY`];
//! - no other file of the app or its extensions names a setting's key: the
//!   settings are `AppSettings`'.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use moshpit_store::settings;

/// Keys `AppSettings` keeps in its own preferences beside the settings
/// (`store.set` / `store.object`), not through the store.
const DEFAULTS_SIDE: &[&str] = &[
    // B704: whether the icon was picked by hand.
    "moshpit.appIcon.pickedByHand",
];

/// Keys of the schema `AppSettings` no longer uses.
const SCHEMA_ONLY: &[&str] = &[
    // B670 moved the pinned Whisper language to a new key; 1.0.8 still
    // reads this one.
    "moshpit.settings.whisperLanguage",
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// How a key is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Way {
    /// Through the store: `get`, `put`, `backing`.
    Store,
    /// In the preferences directly: `store.…`, `defaults.…`.
    Defaults,
}

/// A line without its `//` comment (a `//` inside a string literal, as in
/// a URL, is kept).
fn code(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        let c = bytes[i];
        if escaped {
            escaped = false;
        } else if c == b'\\' && quoted {
            escaped = true;
        } else if c == b'"' {
            quoted = !quoted;
        } else if !quoted && c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return &line[..i];
        }
    }
    line
}

/// The string literals of `line` that name a key (`moshpit.<…>`).
fn keys(line: &str) -> Vec<String> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .filter(|s| {
            s.starts_with("moshpit.")
                && s.len() > "moshpit.".len()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
        })
        .map(str::to_owned)
        .collect()
}

/// Whether `line` reaches the preferences directly.
fn direct(line: &str) -> bool {
    ["store.", "defaults."].iter().any(|d| {
        line.match_indices(d).any(|(i, _)| {
            // `store.` / `defaults.` on its own, not the tail of a longer
            // name (`suite.defaults.` is a test's, never in this file).
            line[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
        })
    })
}

/// Every key `source` names, and the ways it is reached. A key on a
/// `static let <name> = "…"` line is reached as the lines naming
/// `Self.<name>` reach it; a notification's name is no key.
fn uses(source: &str) -> BTreeMap<String, BTreeSet<Way>> {
    let lines: Vec<&str> = source.lines().map(code).collect();
    let mut found: BTreeMap<String, BTreeSet<Way>> = BTreeMap::new();
    for line in &lines {
        if line.contains("Notification.Name(") {
            continue;
        }
        for key in keys(line) {
            let ways = found.entry(key).or_default();
            let constant = line
                .trim_start()
                .strip_prefix("static let ")
                .and_then(|rest| rest.split_once('='))
                .map(|(name, _)| name.trim().to_owned());
            match constant {
                Some(name) => {
                    let named = format!("Self.{name}");
                    for other in lines.iter().filter(|l| l.contains(&named)) {
                        ways.insert(if direct(other) {
                            Way::Defaults
                        } else {
                            Way::Store
                        });
                    }
                }
                None => {
                    ways.insert(if direct(line) {
                        Way::Defaults
                    } else {
                        Way::Store
                    });
                }
            }
        }
    }
    found
}

/// What is wrong with the keys `source` uses, one line each.
fn problems(source: &str) -> Vec<String> {
    let schema: BTreeSet<&str> = settings::ALL.iter().map(|s| s.key).collect();
    let found = uses(source);
    let mut wrong = Vec::new();
    for (key, ways) in &found {
        let store = ways.contains(&Way::Store);
        let defaults = ways.contains(&Way::Defaults);
        if store && defaults {
            wrong.push(format!(
                "{key}: read or written through the store and in the preferences directly: one side never sees the other"
            ));
        }
        if store && !schema.contains(key.as_str()) {
            wrong.push(format!(
                "{key}: through the store, but no setting of crates/store/src/settings.rs's ALL: the store drops it (add it to ALL, or keep it in `store` and list it in DEFAULTS_SIDE)"
            ));
        }
        if defaults && !DEFAULTS_SIDE.contains(&key.as_str()) {
            wrong.push(format!(
                "{key}: in the preferences directly, but not listed in DEFAULTS_SIDE"
            ));
        }
        if defaults && schema.contains(key.as_str()) {
            wrong.push(format!(
                "{key}: a setting of the schema's, written past the store (its sealing and mirror)"
            ));
        }
    }
    for key in DEFAULTS_SIDE {
        if !found.contains_key(*key) {
            wrong.push(format!("{key}: listed in DEFAULTS_SIDE, used nowhere"));
        }
    }
    for key in &schema {
        if !found.contains_key(*key) && !SCHEMA_ONLY.contains(key) {
            wrong.push(format!(
                "{key}: a setting of the schema's that AppSettings doesn't use (list it in SCHEMA_ONLY if it is kept for an older build)"
            ));
        }
    }
    wrong
}

#[test]
fn every_key_app_settings_keeps_through_the_store_is_in_its_schema() {
    let path = repo().join("Moshpit/Models/AppSettings.swift");
    let source = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let found = uses(&source);
    assert!(
        found.len() >= settings::ALL.len(),
        "the scan found {} keys: is AppSettings still where it was? {found:?}",
        found.len()
    );
    let wrong = problems(&source);
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

/// No other file of the app or its extensions names a setting's key: one
/// that did would read or write it past `AppSettings`.
#[test]
fn only_app_settings_names_the_settings_keys() {
    fn swift_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                swift_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "swift") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    for dir in ["Moshpit", "Extensions"] {
        swift_files(&repo().join(dir), &mut files);
    }
    let schema: BTreeSet<&str> = settings::ALL.iter().map(|s| s.key).collect();
    let mut elsewhere = Vec::new();
    for file in files
        .iter()
        .filter(|f| !f.ends_with("Models/AppSettings.swift"))
    {
        let source = fs::read_to_string(file).unwrap();
        for (n, line) in source.lines().enumerate() {
            for key in keys(code(line)) {
                if schema.contains(key.as_str()) || key.starts_with("moshpit.settings.") {
                    elsewhere.push(format!("{}:{}: {key}", file.display(), n + 1));
                }
            }
        }
    }
    assert!(elsewhere.is_empty(), "\n{}", elsewhere.join("\n"));
}

/// The ways the class shipped, each caught: B704's flag through the store
/// under a key the schema lacks; a setting read through the store and
/// written past it; a direct key nobody listed; and the right shapes pass.
#[test]
fn the_ways_a_setting_was_dropped_are_caught() {
    let b704 = r#"
        var appIconPickedByHand: Bool {
            get { return get("moshpit.settings.appIconPickedByHand", false) }
            set { withMutation(keyPath: \.appIconPickedByHand) { put(newValue, "moshpit.settings.appIconPickedByHand") } }
        }
    "#;
    let half = r#"
        var showHiddenFiles: Bool {
            get { access(keyPath: \.showHiddenFiles); return get("moshpit.settings.showHiddenFiles", false) }
            set { withMutation(keyPath: \.showHiddenFiles) { defaults.set(newValue, forKey: "moshpit.settings.showHiddenFiles") } }
        }
    "#;
    let unlisted = r#"
        static let seenKey = "moshpit.pet.seen"
        var seen: Bool { store.bool(forKey: Self.seenKey) }
    "#;
    let fine = r#"
        // A URL in a comment isn't a key: "moshpit.settings.nothing" // nor this
        static let moshpitPredictionChanged = Notification.Name("moshpit.predictionChanged")
        static let appIconPickedByHandKey = "moshpit.appIcon.pickedByHand"
        var picked: Bool { store.object(forKey: Self.appIconPickedByHandKey) as? Bool ?? false }
        var size: Double { get("moshpit.settings.fontSize", 9.0) }
        let relay = "https://push.moshpit.cluas.eu.org"
    "#;
    let says = |source: &str, key: &str, what: &str| {
        let wrong = problems(source);
        assert!(
            wrong.iter().any(|w| w.starts_with(key) && w.contains(what)),
            "{key} {what}: {wrong:?}"
        );
    };
    says(
        b704,
        "moshpit.settings.appIconPickedByHand",
        "the store drops it",
    );
    says(
        half,
        "moshpit.settings.showHiddenFiles",
        "one side never sees the other",
    );
    says(
        half,
        "moshpit.settings.showHiddenFiles",
        "the store drops it",
    );
    says(unlisted, "moshpit.pet.seen", "not listed in DEFAULTS_SIDE");
    let fine_wrong: Vec<String> = problems(fine)
        .into_iter()
        .filter(|w| !w.contains("doesn't use"))
        .collect();
    assert!(fine_wrong.is_empty(), "{fine_wrong:?}");
    assert_eq!(
        uses(fine).keys().cloned().collect::<Vec<_>>(),
        ["moshpit.appIcon.pickedByHand", "moshpit.settings.fontSize"]
    );
}
