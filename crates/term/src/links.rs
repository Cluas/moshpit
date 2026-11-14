//! Links a program never declared: bare `http(s)://` URLs in the text, as
//! the app's `PlainLinkDetector` finds them on the SwiftTerm path (scheme
//! anchored, so bare paths are never taken for links). A URL that wraps
//! onto the next row is one link (the rows are joined first).

use std::ops::Range;

/// What may follow the scheme in a detected URL (ASCII letters and digits
/// and URL punctuation; `PlainLinkDetector.isURLTailChar`), and what the
/// rest of one a program broke at the right edge may hold.
pub(crate) fn is_tail_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-._~/%?#=&:@+".contains(c)
}

/// Whether the run that starts the row after a URL broken at the right
/// edge reads as the rest of that address rather than as the word a
/// sentence goes on with (`PlainLinkDetector.looksLikeAddressTail`):
/// anything but letters (a digit, a slash, a dash) is one tell; capitals
/// inside the run next to lowercase (`OFywpjh`, a document id) is the
/// other. A lowercase or Capitalized word stays prose, so a complete URL
/// that happens to end at the last column is not run into the next word.
pub(crate) fn looks_like_address_tail(run: &str) -> bool {
    if run.chars().count() < 2 {
        return false;
    }
    if run.chars().any(|c| !c.is_ascii_alphabetic()) {
        return true;
    }
    let inner_capital = run.chars().skip(1).any(|c| c.is_ascii_uppercase());
    let lower = run.chars().any(|c| c.is_ascii_lowercase());
    inner_capital && lower
}

/// The byte ranges of the URLs in `text`.
pub fn detect(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("http").map(|i| i + from) {
        let rest = &lower[i..];
        let scheme = if rest.starts_with("https://") {
            8
        } else if rest.starts_with("http://") {
            7
        } else {
            from = i + 4;
            continue;
        };
        // A scheme glued to a word before it ("xhttp://") is not a link.
        if i > 0
            && text[..i]
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            from = i + scheme;
            continue;
        }
        let mut end = i + scheme;
        for c in text[end..].chars() {
            if !is_tail_char(c) {
                break;
            }
            end += c.len_utf8();
        }
        while end > i + scheme && text[..end].ends_with(|c: char| ".,:;!?".contains(c)) {
            end -= 1;
        }
        if end > i + scheme {
            out.push(i..end);
        }
        from = end.max(i + scheme);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(s: &str) -> Vec<&str> {
        detect(s).into_iter().map(|r| &s[r]).collect()
    }

    #[test]
    fn urls_in_prose() {
        assert_eq!(
            found("see https://example.com/a?b=1, then"),
            ["https://example.com/a?b=1"]
        );
        assert_eq!(found("(http://example.com/x)."), ["http://example.com/x"]);
        assert_eq!(
            found("two: http://a.example http://b.example!"),
            ["http://a.example", "http://b.example"]
        );
        assert!(
            found("no links in /tmp/foo or src/bar").is_empty(),
            "paths are not links"
        );
        assert!(found("xhttp://glued").is_empty());
        assert!(found("https://").is_empty());
        assert_eq!(
            found("中文 https://example.com/路径"),
            ["https://example.com/"]
        );
    }

    #[test]
    fn what_reads_as_the_rest_of_an_address() {
        for tail in ["3dEfGh", "OFywpjh", "a/b", "x-y", "42"] {
            assert!(looks_like_address_tail(tail), "{tail}");
        }
        for prose in ["and", "Then", "API", "a", ""] {
            assert!(!looks_like_address_tail(prose), "{prose}");
        }
    }
}
