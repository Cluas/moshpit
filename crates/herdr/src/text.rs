//! Foundation's text rules, as the Swift herdr code uses them.
//!
//! The character sets are Apple's, dumped on macOS 27
//! (`harness/swift/probe-foundation.swift` prints them): `CharacterSet.whitespaces`
//! still counts U+200B ZERO WIDTH SPACE, which Unicode moved out of the
//! space separators long ago, so it is trimmed here too. `newlines` is
//! U+000A–U+000D, U+0085, U+2028 and U+2029.
//!
//! What is deliberately not ported is Swift's grapheme-cluster matching
//! (`hasPrefix`, `contains`, `split(separator:)` and friends compare
//! Characters, so `"herdr: "` followed by a combining mark is not that
//! prefix, and `"5\u{301}"` is not a number). Protocol text is matched on
//! bytes everywhere in this crate: divergence `unicode-syntax`, the same fix
//! the tmux parser made (B17, B35).

/// `CharacterSet.whitespaces`.
pub fn is_whitespace(scalar: char) -> bool {
    matches!(
        scalar,
        '\t' | ' ' | '\u{A0}' | '\u{1680}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    ) || ('\u{2000}'..='\u{200B}').contains(&scalar)
}

/// `CharacterSet.newlines`, and `Character.isNewline` for every Character
/// that is one scalar.
pub fn is_newline(scalar: char) -> bool {
    matches!(scalar, '\n'..='\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// `CharacterSet.whitespacesAndNewlines`.
pub fn is_whitespace_or_newline(scalar: char) -> bool {
    is_whitespace(scalar) || is_newline(scalar)
}

/// `text.trimmingCharacters(in: .whitespaces)`.
pub fn trim_whitespaces(text: &str) -> &str {
    text.trim_matches(is_whitespace)
}

/// `text.trimmingCharacters(in: .whitespacesAndNewlines)`.
pub fn trim_whitespaces_and_newlines(text: &str) -> &str {
    text.trim_matches(is_whitespace_or_newline)
}

/// `text.components(separatedBy: .newlines)`: every newline scalar ends a
/// component, so CRLF leaves an empty one between.
pub fn components_by_newlines(text: &str) -> impl Iterator<Item = &str> {
    text.split(is_newline)
}

/// `text.split(whereSeparator: \.isNewline)`: the non-empty lines. (A CRLF
/// is one Character to Swift; with empty pieces dropped, splitting on each
/// newline scalar comes to the same lines.)
pub fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split(is_newline).filter(|line| !line.is_empty())
}

/// `String(bytes: bytes, encoding: .utf8)`: strict, and a leading byte order
/// mark is a signature, not text.
pub fn decode_utf8(bytes: &[u8]) -> Option<String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8(bytes.to_vec()).ok()
}

/// `String(decoding: bytes, as: UTF8.self)`: every maximal invalid
/// sequence becomes one U+FFFD, the same rule Rust's lossy decoding uses.
pub fn decode_utf8_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `Int(text)`: an optional sign, then ASCII digits only, within `Int64`.
pub fn swift_int(text: &str) -> Option<i64> {
    text.parse().ok()
}

/// The leading ASCII digits of `text`.
pub fn ascii_digits(text: &str) -> &str {
    let end = text
        .bytes()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(text.len());
    &text[..end]
}
