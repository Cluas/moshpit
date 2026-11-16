//! The screens' text in the app's languages.
//!
//! One source for a text, whichever UI shows it: the app's string catalog,
//! `Moshpit/Resources/Localizable.xcstrings`. `scripts/gen/gen_nsh_strings.py`
//! writes the keys the screens use (every `t!` and [`plural!`] literal in
//! `ui/src`) from it to `ui/locales/strings.json`, which is compiled in;
//! run it after a screen uses a new key or the catalog changes. A test fails
//! while a key used here is missing from the file.
//!
//! - [`t!`](nsh_core::t) looks a key up in the language the surface shows
//!   (its host sets it to the app's: [`crate::screen::Params::locale`]),
//!   falling back as rust-i18n does. A key is its English text, as in the
//!   catalog. Format arguments are numbered as Foundation numbers them:
//!   `%@` and `%lld` become `%{arg1}`, `%{arg2}`, …, passed as
//!   `t!(key, arg1 = …)`.
//! - [`plural!`] is a key with plural forms (NSH has no plural rules): the
//!   CLDR category of the count in the shown language ([`plural_category`])
//!   picks the form ([`plural_form`]), the count is `%{arg1}`.

use std::sync::LazyLock;

use nsh_core::i18n::Translations;

/// The generated translations (the module doc).
pub const JSON: &str = include_str!("../locales/strings.json");

static TRANSLATIONS: LazyLock<Translations> =
    LazyLock::new(|| Translations::from_json(JSON).expect("ui/locales/strings.json parses"));

/// The screens' translations, for a root.
pub fn translations() -> Translations {
    TRANSLATIONS.clone()
}

/// The text of `key` in `locale`, outside any surface: what a host-side
/// renderer (a core view turned into a screen's message) says, with
/// `%{name}` filled as [`t!`](nsh_core::t) fills it. Written
/// `tl!(<the key, a literal>, locale, arg1 = …)` so the generator finds
/// the key.
#[macro_export]
macro_rules! tl {
    ($key:literal, $locale:expr $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::strings::translations().translate(
            $locale,
            $key,
            &[$((stringify!($name), ::std::string::ToString::to_string(&$value))),*],
        )
    };
}

/// The CLDR plural category of the integer `n` in `language` (a BCP 47
/// tag), for the languages the app ships; others count as English does.
/// A form the catalog lacks for a category is its `other` form (the
/// generator fills them), so a category with no form of its own is safe.
pub fn plural_category(language: &str, n: i64) -> &'static str {
    let n = n.unsigned_abs();
    let millions = n != 0 && n.is_multiple_of(1_000_000);
    let lower = language.to_ascii_lowercase();
    let primary = lower.split(['-', '_']).next().unwrap_or_default();
    match primary {
        "ja" | "ko" | "zh" => "other",
        // French and Brazilian Portuguese: 0 and 1 are singular.
        "fr" => {
            if n <= 1 {
                "one"
            } else if millions {
                "many"
            } else {
                "other"
            }
        }
        "pt" if lower == "pt-pt" => {
            if n == 1 {
                "one"
            } else if millions {
                "many"
            } else {
                "other"
            }
        }
        "pt" => {
            if n <= 1 {
                "one"
            } else if millions {
                "many"
            } else {
                "other"
            }
        }
        "es" | "it" => {
            if n == 1 {
                "one"
            } else if millions {
                "many"
            } else {
                "other"
            }
        }
        _ => {
            if n == 1 {
                "one"
            } else {
                "other"
            }
        }
    }
}

/// The text of plural key `key` for count `n` in `language`: the form of
/// `n`'s category there (`<key>#<category>`, as the generator writes it),
/// with `n` for `%{arg1}`.
pub fn plural_form(translations: &Translations, language: &str, key: &str, n: i64) -> String {
    let form = format!("{key}#{}", plural_category(language, n));
    translations.translate(language, &form, &[("arg1", n.to_string())])
}

/// The text of plural key `key` for count `n` in the language the surface
/// shows: what [`plural!`] calls.
#[doc(hidden)]
pub fn plural(key: &str, n: i64) -> String {
    let translations = nsh_core::element::global::<Translations>().get();
    plural_form(&translations, &shown_language(), key, n)
}

/// The language the surface shows its text in (what [`t!`](nsh_core::t)
/// picks): the first of the user's languages the app has, else the
/// surface's locale.
pub fn shown_language() -> String {
    let translations = nsh_core::element::global::<Translations>().get();
    let languages = nsh_core::element::locale_settings().languages;
    translations
        .pick(languages.iter())
        .unwrap_or_else(nsh_core::element::locale)
        .as_str()
        .to_owned()
}

/// The text of a key with plural forms for a count, in the language the
/// surface shows: `plural!(key, count)` (the count is `%{arg1}`).
#[macro_export]
macro_rules! plural {
    ($key:expr, $n:expr $(,)?) => {
        $crate::strings::plural($key, ::core::convert::Into::<i64>::into($n))
    };
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// The generated file, as data.
    fn generated() -> HashMap<String, HashMap<String, String>> {
        serde_json::from_str(JSON).unwrap()
    }

    /// The keys `t!` and `plural!` literals in ui/src name, read as the
    /// generator reads them (simple literals: no escapes).
    fn used_keys() -> Vec<(String, bool)> {
        let mut keys = Vec::new();
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut dirs = vec![src];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                for (macro_name, is_plural) in [("t!(\"", false), ("plural!(\"", true)] {
                    for (at, _) in source.match_indices(macro_name) {
                        // A macro's name, not the end of another's
                        // (`format!(`), as the generator's `\b` reads it.
                        let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
                        if at > 0 && word(source.as_bytes()[at - 1]) {
                            continue;
                        }
                        let rest = &source[at + macro_name.len()..];
                        keys.push((literal(rest), is_plural));
                    }
                }
            }
        }
        keys
    }

    /// The text of a string literal's body at the start of `rest` (up to
    /// its closing quote), its `\"` and `\\` escapes read as the generator
    /// reads them.
    fn literal(rest: &str) -> String {
        let mut text = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => text.push('\n'),
                    Some('t') => text.push('\t'),
                    Some(other) => text.push(other),
                    None => break,
                },
                c => text.push(c),
            }
        }
        text
    }

    #[test]
    fn every_key_the_screens_use_is_generated() {
        let generated = generated();
        let english = &generated["en"];
        let keys = used_keys();
        assert!(
            keys.iter().any(|(key, _)| key == "Recent log"),
            "the scan finds the keys"
        );
        for (key, is_plural) in keys {
            let present = if is_plural {
                english.contains_key(&format!("{key}#other"))
            } else {
                english.contains_key(&key)
            };
            assert!(present, "{key:?}: run scripts/gen/gen_nsh_strings.py");
        }
    }

    #[test]
    fn the_catalog_languages_are_there() {
        let generated = generated();
        for language in [
            "en", "de", "es", "fr", "ja", "ko", "pt-BR", "zh-Hans", "zh-Hant",
        ] {
            assert!(generated.contains_key(language), "{language}");
        }
        assert_eq!(generated["zh-Hans"]["Recent log"], "最近日志");
        assert_eq!(generated["en"]["Recent log"], "Recent log");
        let translations = translations();
        assert_eq!(translations.translate("zh-Hant-TW", "Copy", &[]), "拷貝");
        assert_eq!(
            translations.translate("it", "Copy", &[]),
            "Copy",
            "English otherwise"
        );
    }

    #[test]
    fn plural_categories() {
        for n in [0, 2, 5, 1_000_000] {
            assert_eq!(plural_category("en", n), "other", "{n}");
        }
        assert_eq!(plural_category("en-GB", 1), "one");
        assert_eq!(plural_category("de", 1), "one");
        assert_eq!(plural_category("fr", 0), "one");
        assert_eq!(plural_category("fr", 1), "one");
        assert_eq!(plural_category("fr", 2), "other");
        assert_eq!(plural_category("fr", 2_000_000), "many");
        assert_eq!(plural_category("pt-BR", 0), "one");
        assert_eq!(plural_category("pt-PT", 0), "other");
        assert_eq!(plural_category("es", 1), "one");
        assert_eq!(plural_category("es", 0), "other");
        assert_eq!(plural_category("es", 1_000_000), "many");
        for language in ["ja", "ko", "zh-Hans", "zh-Hant"] {
            assert_eq!(plural_category(language, 1), "other", "{language}");
        }
        assert_eq!(plural_category("en", -1), "one");
    }

    #[test]
    fn plural_forms_by_category() {
        // As the generator writes a plural key: every category, a missing
        // one as `other`, a language of one form in all of them.
        let mut json = serde_json::Map::new();
        let forms = |one: &str, other: &str| {
            let mut forms = serde_json::Map::new();
            for category in ["zero", "one", "two", "few", "many", "other"] {
                let text = if category == "one" { one } else { other };
                forms.insert(format!("%lld keys#{category}"), text.into());
            }
            serde_json::Value::Object(forms)
        };
        json.insert("en".into(), forms("%{arg1} key", "%{arg1} keys"));
        json.insert("fr".into(), forms("%{arg1} clé", "%{arg1} clés"));
        json.insert("ja".into(), forms("%{arg1}個のキー", "%{arg1}個のキー"));
        let translations =
            Translations::from_json(&serde_json::Value::Object(json).to_string()).unwrap();
        assert_eq!(plural_form(&translations, "en", "%lld keys", 1), "1 key");
        assert_eq!(plural_form(&translations, "en", "%lld keys", 0), "0 keys");
        assert_eq!(plural_form(&translations, "fr", "%lld keys", 0), "0 clé");
        assert_eq!(plural_form(&translations, "fr", "%lld keys", 2), "2 clés");
        assert_eq!(
            plural_form(&translations, "ja", "%lld keys", 1),
            "1個のキー"
        );
        assert_eq!(
            plural_form(&translations, "de", "%lld keys", 2),
            "2 keys",
            "English otherwise"
        );
    }
}
