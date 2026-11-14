//! Every Chinese string the app ships, held to the copy rules (src/lib.rs):
//!
//! - the string catalogs, `*.xcstrings` under `Moshpit/`, `Extensions/` and
//!   `Packages/` (the app, the widget, the push and share extensions, the
//!   Info.plist texts), written by `scripts/gen/gen_xcstrings.py`;
//! - the zh-Hant table it reads, `scripts/gen/locales/zh-Hant.json`;
//! - the NSH screens' translations, `ui/locales/strings.json`, written from
//!   the app's catalog by `scripts/gen/gen_nsh_strings.py`;
//! - the notification card's sentences in Rust (`crates/seal/src/copy.rs`);
//! - the generator's own lines, for the banned words (its zh-Hans is the
//!   catalogs' source; a banned word may only sit in a comment there).
//!
//! A failure lists each place, the rule and the text. Fix the copy at its
//! source (the generator's inputs), regenerate, and run this again.

use moshpit_copy::{BANNED, Script, check};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

/// One Chinese text and the English it renders.
struct Text {
    place: String,
    en: String,
    zh: String,
    script: Script,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_json(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn relative(path: &Path) -> String {
    let root = root().canonicalize().expect("the repository root");
    let path = path.canonicalize().expect("a scanned file");
    path.strip_prefix(&root)
        .unwrap_or(&path)
        .display()
        .to_string()
}

/// Every `*.xcstrings` under the folders the app's targets build from.
fn catalog_paths() -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') {
                continue; // .build, .swiftpm
            }
            if path.is_dir() {
                walk(&path, found);
            } else if name.ends_with(".xcstrings") {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    for dir in ["Moshpit", "Extensions", "Packages"] {
        walk(&root().join(dir), &mut found);
    }
    found
}

/// A localization's texts: its one value, or each plural form's.
fn values(localization: &Value) -> Vec<String> {
    if let Some(value) = localization
        .pointer("/stringUnit/value")
        .and_then(Value::as_str)
    {
        return vec![value.to_owned()];
    }
    let Some(variations) = localization.get("variations").and_then(Value::as_object) else {
        return Vec::new();
    };
    variations
        .values()
        .filter_map(Value::as_object)
        .flat_map(|forms| forms.values())
        .filter_map(|form| form.pointer("/stringUnit/value").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// The catalogs' texts, and each key's English (the key itself, unless the
/// catalog gives an `en` value, as the Info.plist one does).
fn catalog_texts(texts: &mut Vec<Text>, english: &mut HashMap<String, String>) -> usize {
    let paths = catalog_paths();
    for path in &paths {
        let place = relative(path);
        let catalog = read_json(path);
        let strings = catalog
            .get("strings")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{place}: no strings"));
        for (key, entry) in strings {
            let localizations = entry.get("localizations");
            let en = localizations
                .and_then(|l| l.get("en"))
                .map(values)
                .and_then(|v| v.into_iter().last())
                .unwrap_or_else(|| key.clone());
            english.entry(key.clone()).or_insert_with(|| en.clone());
            for script in [Script::Hans, Script::Hant] {
                let Some(localization) = localizations.and_then(|l| l.get(script.code())) else {
                    continue;
                };
                for zh in values(localization) {
                    texts.push(Text {
                        place: format!("{place} {key:?}"),
                        en: en.clone(),
                        zh,
                        script,
                    });
                }
            }
        }
    }
    paths.len()
}

/// `scripts/gen/locales/zh-Hant.json`: key → text, or plural forms.
fn locale_texts(texts: &mut Vec<Text>, english: &HashMap<String, String>) {
    let path = root().join("scripts/gen/locales/zh-Hant.json");
    let place = relative(&path);
    let table = read_json(&path);
    for (key, value) in table.as_object().expect("a table") {
        let forms: Vec<String> = match value {
            Value::String(text) => vec![text.clone()],
            Value::Object(forms) => forms
                .values()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => panic!("{place}: {key:?} is neither text nor forms"),
        };
        let en = english.get(key).cloned().unwrap_or_else(|| key.clone());
        for zh in forms {
            texts.push(Text {
                place: format!("{place} {key:?}"),
                en: en.clone(),
                zh,
                script: Script::Hant,
            });
        }
    }
}

/// `ui/locales/strings.json`: {language: {key or key#category: text}}.
fn nsh_texts(texts: &mut Vec<Text>) {
    let path = root().join("ui/locales/strings.json");
    let place = relative(&path);
    let table = read_json(&path);
    let english = table.get("en").and_then(Value::as_object).expect("en");
    for script in [Script::Hans, Script::Hant] {
        let Some(language) = table.get(script.code()).and_then(Value::as_object) else {
            panic!("{place}: no {}", script.code());
        };
        for (key, zh) in language {
            let en = english
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_else(|| key.split('#').next().unwrap_or(key));
            texts.push(Text {
                place: format!("{place} {key:?}"),
                en: en.to_owned(),
                zh: zh.as_str().expect("a text").to_owned(),
                script,
            });
        }
    }
}

/// The notification card's sentences as `crates/seal` writes them.
fn rust_texts(texts: &mut Vec<Text>) {
    use moshpit_seal::Language;
    for (language, script) in [
        (Language::ZhHans, Script::Hans),
        (Language::ZhHant, Script::Hant),
    ] {
        for (en, zh) in [
            (
                Language::En.finished_template(),
                language.finished_template(),
            ),
            (
                Language::En.needs_you_template(),
                language.needs_you_template(),
            ),
        ] {
            texts.push(Text {
                place: format!("crates/seal/src/copy.rs {en:?}"),
                en: en.to_owned(),
                zh: zh.to_owned(),
                script,
            });
        }
    }
}

/// Banned words on the generator's lines that are not comments.
fn generator_findings() -> Vec<String> {
    let path = root().join("scripts/gen/gen_xcstrings.py");
    let place = relative(&path);
    let source = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{place}: {e}"));
    let mut found = Vec::new();
    for (number, line) in source.lines().enumerate() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        for word in BANNED {
            if line.contains(word) {
                found.push(format!(
                    "{place}:{} [banned] uses {word}\n    {}",
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    found
}

fn all_texts() -> (Vec<Text>, usize) {
    let mut texts = Vec::new();
    let mut english = HashMap::new();
    let catalogs = catalog_texts(&mut texts, &mut english);
    locale_texts(&mut texts, &english);
    nsh_texts(&mut texts);
    rust_texts(&mut texts);
    (texts, catalogs)
}

#[test]
fn the_chinese_copy_keeps_the_rules() {
    let (texts, _) = all_texts();
    let mut report = generator_findings();
    let mut distinct = BTreeSet::new();
    for text in &texts {
        for violation in check(&text.en, &text.zh, text.script) {
            distinct.insert((text.script.code(), text.en.clone(), text.zh.clone()));
            report.push(format!(
                "{} [{} {}] {}\n    en: {}\n    zh: {}",
                text.place,
                text.script.code(),
                violation.rule,
                violation.detail,
                text.en,
                text.zh
            ));
        }
    }
    assert!(
        report.is_empty(),
        "{}\n\n{} violations in {} distinct texts. Fix them at the source \
         (scripts/gen/gen_xcstrings.py, scripts/gen/locales/zh-Hant.json, \
         crates/seal/src/copy.rs), then run scripts/gen/gen_xcstrings.py and \
         scripts/gen/gen_nsh_strings.py",
        report.join("\n"),
        report.len(),
        distinct.len()
    );
}

/// The scan reaches every place, so a moved file fails here rather than
/// leaving the rules checking nothing.
#[test]
fn the_scan_finds_every_place() {
    let (texts, catalogs) = all_texts();
    assert!(
        catalogs >= 5,
        "the app's, three extensions' and Info.plist's catalogs: {catalogs}"
    );
    for place in [
        "Moshpit/Resources/Localizable.xcstrings",
        "Moshpit/Resources/InfoPlist.xcstrings",
        "Extensions/MoshpitPush/Localizable.xcstrings",
        "scripts/gen/locales/zh-Hant.json",
        "ui/locales/strings.json",
        "crates/seal/src/copy.rs",
    ] {
        for script in [Script::Hans, Script::Hant] {
            if place.ends_with("zh-Hant.json") && script == Script::Hans {
                continue;
            }
            assert!(
                texts
                    .iter()
                    .any(|t| t.place.starts_with(place) && t.script == script),
                "no {} text from {place}",
                script.code()
            );
        }
    }
    let agent = texts
        .iter()
        .find(|t| t.en == "Send to agent" && t.script == Script::Hans)
        .expect("the share sheet's button");
    assert!(agent.zh.contains("智能体"));
    let agent = texts
        .iter()
        .find(|t| t.en == "Send to agent" && t.script == Script::Hant)
        .expect("the share sheet's button, zh-Hant");
    assert!(agent.zh.contains("智能體"));
}
