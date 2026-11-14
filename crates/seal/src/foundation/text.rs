//! Strings the way Swift and Foundation see them.
//!
//! Foundation's `trimmingCharacters(in:)` works on Unicode scalars; Swift's
//! `Character` is an extended grapheme cluster whose `isWhitespace`,
//! `isNewline` and `isNumber` look at its FIRST scalar only — so `" \u{301}"`
//! is whitespace and `"1\u{301}"` a number. `count`, `prefix` and `split`
//! work on characters. Measured on macOS 27 (Swift 6.4); grapheme clusters
//! here come from `unicode-segmentation` (Unicode 17 against Swift's 16 —
//! only characters assigned in Unicode 17 can tell them apart).

use unicode_segmentation::UnicodeSegmentation;

use super::numeric_table::NUMERIC_TYPE;

/// Unicode `White_Space`: `Unicode.Scalar.Properties.isWhitespace`, what
/// Swift's `Character.isWhitespace` tests (25 scalars).
pub fn is_white_space(c: char) -> bool {
    c.is_whitespace()
}

/// Foundation's `CharacterSet.whitespaces` on macOS 27: TAB, the space
/// separators — and U+200B ZERO WIDTH SPACE, which Unicode moved out of Zs
/// long ago and Foundation never did.
pub fn is_inline_space(c: char) -> bool {
    matches!(
        c,
        '\t' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200B}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// Foundation's `CharacterSet.whitespacesAndNewlines`: [`is_inline_space`]
/// and [`is_newline`] — Unicode `White_Space` plus U+200B.
pub fn is_space_or_newline(c: char) -> bool {
    is_inline_space(c) || is_newline(c)
}

/// Swift's newline scalars (`Character.isNewline`, Foundation's
/// `CharacterSet.newlines`): LF, VT, FF, CR, NEL, LS, PS.
pub fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\u{B}' | '\u{C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// `Unicode.Scalar.Properties.numericType != nil`.
pub fn has_numeric_type(c: char) -> bool {
    let c = u32::from(c);
    NUMERIC_TYPE
        .binary_search_by(|&(low, high)| {
            if high < c {
                std::cmp::Ordering::Less
            } else if low > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// `trimmingCharacters(in: .whitespaces)`.
pub fn trim_inline_spaces(text: &str) -> &str {
    text.trim_matches(is_inline_space)
}

/// `trimmingCharacters(in: .whitespacesAndNewlines)`.
pub fn trim_white_space(text: &str) -> &str {
    text.trim_matches(is_space_or_newline)
}

/// `trimmingCharacters(in: .newlines)`.
pub fn trim_newlines(text: &str) -> &str {
    text.trim_matches(is_newline)
}

/// The first scalar of a grapheme cluster.
fn first(character: &str) -> char {
    character.chars().next().expect("a character has a scalar")
}

/// `Character.isWhitespace`.
pub fn character_is_whitespace(character: &str) -> bool {
    is_white_space(first(character))
}

/// `Character.isNewline`.
pub fn character_is_newline(character: &str) -> bool {
    is_newline(first(character))
}

/// `Character.isNumber`.
pub fn character_is_number(character: &str) -> bool {
    has_numeric_type(first(character))
}

/// Swift's characters (extended grapheme clusters).
pub fn characters(text: &str) -> impl DoubleEndedIterator<Item = &str> {
    text.graphemes(true)
}

/// `String.count`.
pub fn character_count(text: &str) -> usize {
    characters(text).count()
}

/// `String(text.prefix(n))`.
pub fn character_prefix(text: &str, n: usize) -> &str {
    match text.grapheme_indices(true).nth(n) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

/// `text.split(whereSeparator: separator)`: runs of characters between
/// separator characters, empty runs omitted.
pub fn split_characters(text: &str, separator: impl Fn(&str) -> bool) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    for (index, character) in text.grapheme_indices(true) {
        if separator(character) {
            if let Some(begin) = start.take() {
                out.push(&text[begin..index]);
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(begin) = start {
        out.push(&text[begin..]);
    }
    out
}

/// `haystack.contains(needle)` as Swift's `String` answers it: the needle's
/// characters must be whole characters of the haystack, so
/// `"stamp.sh\u{301}"` does not contain `"stamp.sh"`.
pub fn contains_characters(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut boundaries: Vec<usize> = haystack
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .collect();
    boundaries.push(haystack.len());
    haystack.match_indices(needle).any(|(at, found)| {
        boundaries.binary_search(&at).is_ok()
            && boundaries.binary_search(&(at + found.len())).is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimming_works_on_scalars() {
        // " \u{301}a " trims to "\u{301}a" in Foundation: the space in front
        // of the combining mark goes, though the two are one Character.
        assert_eq!(trim_inline_spaces(" \u{301}a "), "\u{301}a");
        assert_eq!(trim_inline_spaces("a \u{301}"), "a \u{301}");
        assert_eq!(trim_inline_spaces("\u{A0}x\u{A0}"), "x");
        assert_eq!(trim_inline_spaces("\u{2028}x\u{2028}"), "\u{2028}x\u{2028}");
        assert_eq!(trim_white_space("\u{2028}x\u{2028}"), "x");
        assert_eq!(trim_white_space("\r\nx\r\n"), "x");
        assert_eq!(trim_inline_spaces("\u{3000}x"), "x");
        // Foundation's sets still hold U+200B; Swift's White_Space does not.
        assert_eq!(trim_inline_spaces("\u{200B}x\u{200B}"), "x");
        assert_eq!(trim_white_space("\u{200B}\nx"), "x");
        assert!(!is_white_space('\u{200B}'));
        let foundation: Vec<u32> = (0..=0x10FFFFu32)
            .filter_map(char::from_u32)
            .filter(|&c| is_space_or_newline(c))
            .map(u32::from)
            .collect();
        assert_eq!(
            foundation,
            [
                0x9, 0xA, 0xB, 0xC, 0xD, 0x20, 0x85, 0xA0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003,
                0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200A, 0x200B, 0x2028, 0x2029,
                0x202F, 0x205F, 0x3000
            ]
        );
    }

    #[test]
    fn character_properties_look_at_the_first_scalar() {
        assert!(character_is_whitespace(" \u{301}"));
        assert!(character_is_whitespace("\r\n"));
        assert!(character_is_newline("\r\n"));
        assert!(character_is_number("1\u{301}"));
        for numeral in [
            "一", "五", "٣", "½", "Ⅻ", "①", "𝟙", "〇", "零", "兆", "億", "廿",
        ] {
            assert!(character_is_number(numeral), "{numeral}");
        }
        assert!(!character_is_number("a"));
        assert!(!character_is_number("x\u{20DD}"));
    }

    #[test]
    fn splitting_swallows_a_separator_and_its_marks() {
        let words = split_characters("a \u{301}b\r\nc\u{2028}d  e", |c| {
            character_is_whitespace(c) || character_is_newline(c)
        });
        assert_eq!(words, ["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn containment_is_by_character() {
        assert!(contains_characters(
            "sh ~/.moshpit/moshpit-stamp.sh x",
            "moshpit-stamp.sh"
        ));
        assert!(!contains_characters(
            "sh moshpit-stamp.sh\u{301} x",
            "moshpit-stamp.sh"
        ));
        assert!(contains_characters(
            "x\u{301}moshpit-stamp.sh",
            "moshpit-stamp.sh"
        ));
        assert!(contains_characters("abc", ""));
    }

    #[test]
    fn counting_and_cutting_by_character() {
        assert_eq!(character_count("e\u{301}"), 1);
        assert_eq!(character_count("👨‍👩‍👧"), 1);
        assert_eq!(character_count("🇨🇳🇺🇸"), 2);
        assert_eq!(character_prefix("e\u{301}xyz", 1), "e\u{301}");
        assert_eq!(character_prefix("ab", 5), "ab");
    }
}
