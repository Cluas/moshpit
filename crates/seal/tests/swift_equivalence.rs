//! The Rust port against the Swift push code's recorded output, without
//! swiftc: `tests/fixtures/swift/cases.swift.txt` is what
//! `harness/swift/main.swift` printed for `cases.txt` over the app's
//! unchanged MoshpitKit sources (`harness/equivalence.py update-goldens`),
//! as the app last ran them: E7b put the notification service on this
//! crate and deleted them, and they are kept in `harness/swift/frozen`.
//! Rust must print the same — store files compared as JSON, since Swift's
//! key order is a hash order — except on the documented divergences, where
//! it must print what `divergences.rust.txt` holds and never what Swift did.

use std::path::{Path, PathBuf};

use moshpit_seal::foundation::json::{self, Dialect, Number, Style, Value};
use moshpit_seal::harness;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/swift")
        .join(name)
}

fn lines(name: &str) -> Vec<String> {
    let path = fixture(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("seal-equivalence-{}", moshpit_seal::random::hex(8)));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Numbers as the values they are, object keys sorted.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Number(number) => {
            let x: f64 = number.literal().parse().expect("a number");
            Value::Number(Number::from_literal(format!("{x:?}")))
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(k, v)| (k.to_owned(), canonical(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn canonical_file(token: &str) -> String {
    if token == "-" || token == "~" {
        return token.to_owned();
    }
    let bytes = moshpit_seal::foundation::hex::decode(token).expect("hex");
    match json::parse(&bytes, Dialect::Decoder) {
        Ok(value) => format!(
            "json:{}",
            json::write_encoder(
                &canonical(&value),
                Style {
                    pretty: false,
                    sorted_keys: true,
                    escape_slashes: true
                }
            )
        ),
        Err(_) => format!("raw:{token}"),
    }
}

fn normalise(case: &str, result: &str) -> String {
    let command = case.split(' ').next().unwrap_or_default();
    if (command == "pfile" || command == "sfile")
        && let Some((results, files)) = result.split_once(" || ")
    {
        let parts: Vec<(&str, &str)> = files
            .split(' ')
            .filter_map(|part| part.split_once('='))
            .collect();
        let get = |name: &str| {
            parts
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| *v)
                .unwrap_or("?")
        };
        return format!(
            "{results} || file={} backup={} stamped={}",
            canonical_file(get("file")),
            get("backup"),
            get("stamped")
        );
    }
    result.to_owned()
}

#[test]
fn committed_cases_print_what_swift_printed() {
    let cases = lines("cases.txt");
    let swift = lines("cases.swift.txt");
    assert_eq!(cases.len(), swift.len());
    let scratch = Scratch::new();
    let mut differences = Vec::new();
    for (case, swift) in cases.iter().zip(&swift) {
        let rust = harness::run(case, &scratch.0).expect("a case");
        if normalise(case, &rust) != normalise(case, swift) {
            differences.push(format!(
                "{case:.200}\n  swift {swift:.300}\n  rust  {rust:.300}"
            ));
        }
    }
    assert!(
        differences.is_empty(),
        "{} differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

#[test]
fn every_divergence_still_tells_the_two_apart() {
    let cases = lines("divergences.txt");
    let swift = lines("divergences.swift.txt");
    let rust_recorded = lines("divergences.rust.txt");
    assert_eq!(cases.len(), swift.len());
    assert_eq!(cases.len(), rust_recorded.len());
    let scratch = Scratch::new();
    for ((case, swift), recorded) in cases.iter().zip(&swift).zip(&rust_recorded) {
        let rust = harness::run(case, &scratch.0).expect("a case");
        assert_eq!(&rust, recorded, "{case}");
        assert_ne!(
            normalise(case, &rust),
            normalise(case, swift),
            "no longer a divergence: {case}"
        );
    }
}

/// The two sentences of the notification service's own catalog, as the
/// Rust renderer holds them.
#[test]
fn the_card_sentences_are_the_extensions_catalog() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../Extensions/MoshpitPush/Localizable.xcstrings");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let catalog = json::parse(&bytes, Dialect::Decoder).expect("the catalog is JSON");
    let strings = catalog
        .as_object()
        .and_then(|c| c.get("strings"))
        .and_then(Value::as_object)
        .expect("strings");
    let keys: Vec<&str> = strings.keys().collect();
    assert_eq!(keys.len(), 2, "the extension's catalog grew: {keys:?}");
    for language in moshpit_seal::Language::ALL {
        for (key, ours) in [
            ("✓ %@ finished", language.finished_template()),
            ("%@ needs you", language.needs_you_template()),
        ] {
            let theirs = if language.code() == "en" {
                key.to_owned()
            } else {
                strings
                    .get(key)
                    .and_then(Value::as_object)
                    .and_then(|entry| entry.get("localizations"))
                    .and_then(Value::as_object)
                    .and_then(|l| l.get(language.code()))
                    .and_then(Value::as_object)
                    .and_then(|l| l.get("stringUnit"))
                    .and_then(Value::as_object)
                    .and_then(|unit| unit.get("value"))
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| panic!("{key} has no {}", language.code()))
                    .to_owned()
            };
            assert_eq!(ours, theirs, "{key} in {}", language.code());
        }
    }
}

/// `numeric_table.rs` is what the Swift runtime's `numericType` says.
#[test]
fn the_numeric_table_is_swifts() {
    let recorded = std::fs::read_to_string(fixture("numeric.txt")).expect("numeric.txt");
    let ranges: Vec<(u32, u32)> = recorded
        .split_whitespace()
        .map(|range| {
            let (low, high) = range.split_once('-').expect("a range");
            (
                u32::from_str_radix(low, 16).unwrap(),
                u32::from_str_radix(high, 16).unwrap(),
            )
        })
        .collect();
    assert_eq!(ranges.len(), 235);
    for c in (0..=0x10FFFFu32).filter_map(char::from_u32) {
        let swift = ranges
            .iter()
            .any(|&(low, high)| (low..=high).contains(&u32::from(c)));
        assert_eq!(
            moshpit_seal::foundation::text::has_numeric_type(c),
            swift,
            "U+{:04X}",
            u32::from(c)
        );
    }
}
