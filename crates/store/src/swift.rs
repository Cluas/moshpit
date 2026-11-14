//! Swift's `String` as the stores' code sees it: equality and ordering by
//! canonical equivalence (two strings are equal when their NFC forms are),
//! `Character` as an extended grapheme cluster, case mapping scalar by
//! scalar. Measured on macOS 27 (Swift 6.4); `moshpit_seal::foundation::text`
//! has the rest (trimming, character counts).

use std::cmp::Ordering;

use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// The form two strings share when Swift's `==` says they are equal.
pub fn canonical(text: &str) -> String {
    text.nfc().collect()
}

/// Swift's `a == b`.
pub fn eq(a: &str, b: &str) -> bool {
    a == b || canonical(a) == canonical(b)
}

/// Swift's `a < b` and friends: NFC scalars compared one by one
/// (`["b", "a\u{301}", "\u{e1}", "a"].sorted()` is `a, b, á, á`).
pub fn cmp(a: &str, b: &str) -> Ordering {
    canonical(a).cmp(&canonical(b))
}

/// `Set(strings)` in Swift: the first of each group of canonically equal
/// strings, in the order given.
pub fn dedup(strings: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    strings
        .into_iter()
        .filter(|s| seen.insert(canonical(s)))
        .collect()
}

/// `text.hasPrefix(prefix)`: the prefix's characters are the text's first
/// characters, canonically — so `"ecdsa-sha2-\u{301}x"` does not start with
/// `"ecdsa-sha2-"` (its last `-` carries the accent).
pub fn has_prefix(text: &str, prefix: &str) -> bool {
    let mut characters = text.graphemes(true);
    prefix
        .graphemes(true)
        .all(|wanted| characters.next().is_some_and(|got| eq(got, wanted)))
}

/// `text.lowercased()`: each scalar's full lowercase mapping, with no
/// context (`"ΑΣ"` is `"ασ"`, not `"ας"`).
pub fn lowercased(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// `text.uppercased()`: each scalar's full uppercase mapping (`"ß"` is
/// `"SS"`).
pub fn uppercased(text: &str) -> String {
    text.chars().flat_map(char::to_uppercase).collect()
}

/// `Character.isHexDigit` (its `hexDigitValue`): a character of one scalar,
/// an ASCII or fullwidth digit or letter a–f in either case.
pub fn is_hex_digit(character: &str) -> bool {
    let mut scalars = character.chars();
    let (Some(c), None) = (scalars.next(), scalars.next()) else {
        return false;
    };
    matches!(c,
        '0'..='9' | 'A'..='F' | 'a'..='f'
        | '\u{FF10}'..='\u{FF19}' | '\u{FF21}'..='\u{FF26}' | '\u{FF41}'..='\u{FF46}')
}

/// Swift's characters of `text`.
pub fn characters(text: &str) -> impl DoubleEndedIterator<Item = &str> {
    text.graphemes(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Swift answered on macOS 27.
    #[test]
    fn as_measured() {
        assert!(eq("\u{e9}", "e\u{301}"));
        let mut words = ["b", "a\u{301}", "\u{e1}", "a", "B", "Z", "\u{e9}"];
        words.sort_by(|a, b| cmp(a, b));
        assert_eq!(&words[..4], ["B", "Z", "a", "b"]);
        assert_eq!(words[6], "\u{e9}");
        assert_eq!(dedup(["e\u{301}".into(), "\u{e9}".into()]), ["e\u{301}"]);
        assert!(!has_prefix("ecdsa-sha2-\u{301}x", "ecdsa-sha2-"));
        assert!(has_prefix("ecdsa-sha2-x", "ecdsa-sha2-"));
        assert!(has_prefix("abc", ""));
        assert!(!has_prefix("ab", "abc"));
        assert_eq!(lowercased("ΑΣ"), "ασ");
        assert_eq!(lowercased("ΑΣ Β"), "ασ β");
        assert_eq!(lowercased("İ"), "i\u{307}");
        assert_eq!(uppercased("ß"), "SS");
        assert_eq!(uppercased("ﬁ"), "FI");
        for (c, hex) in [
            ("A", true),
            ("a", true),
            ("\u{FF21}", true),
            ("\u{FF41}", true),
            ("\u{FF10}", true),
            ("g", false),
            ("A\u{301}", false),
            ("\r\n", false),
        ] {
            assert_eq!(is_hex_digit(c), hex, "{c:?}");
        }
    }
}
