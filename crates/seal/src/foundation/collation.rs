//! The key order `JSONSerialization` writes with `.sortedKeys` — which is
//! not `JSONEncoder`'s (that one sorts by code point).
//!
//! Foundation sorts the keys with `compare(_:options:range:locale:)`, options
//! `[.numeric, .caseInsensitive, .forcedOrdering]` and the system locale
//! (`JSONSerialization.swift` in swift-corelibs-foundation; the order
//! measured on macOS 27 is the same under every locale tried: en_US, da_DK,
//! lt_LT, et_EE, tr_TR, cs_CZ). That comparison is CoreFoundation's
//! `CFStringCompareWithOptionsAndLocale` (`CFString.c`), and [`compare`] is
//! a port of it, step for step:
//!
//! - both keys are walked a UTF-16 code unit at a time; where both hold an
//!   ASCII digit, the two runs of digits are read as numbers, in a `u64`
//!   that wraps (`18446744073709551616` reads as 0), and unequal numbers
//!   decide at once;
//! - the first equal pair written with different digit counts records the
//!   shorter as first (`1A` < `01a`), and from then on ASCII letters are
//!   compared without case;
//! - the first differing pair of ASCII code units hands the comparison to
//!   the collator, from one code unit before the pair (and further back
//!   over punctuation and combining marks);
//! - a differing pair with a non-ASCII side is case-folded first; if the
//!   folds still differ, the collator takes over from the last pair the
//!   walk matched outside a number; if they match, the pair's code-unit
//!   order is recorded and the collator settles it at the end, over both
//!   keys from that point, unless a digit count was recorded first;
//! - a key that runs out first sorts first.
//!
//! The collator is ICU's root collation as `_CFCompareStringsWithLocale` and
//! `__CompareTextDefault` (`CFStringUtilities.c`) drive it: numeric (leading
//! zeros ignored), punctuation not ignorable, and, with forced ordering,
//! every level down to code units. For ASCII its levels are, as measured:
//!
//! 1. primary: TAB < LF < VT < FF < CR < space < `_ - , ; : ! ? . ' " ( ) [ ]
//!    { } @ * / \ & # % `` ` `` ^ + < = > | ~ $` < numbers (by value) <
//!    letters, without case; the other control characters are ignored;
//! 2. case, element by element, lower first;
//! 3. code units.
//!
//! NEL, LS and PS sort with the line breaks, after CR; the other spaces sort
//! as a space, with a level of their own between case and code units: the
//! wide one, then the compatibility ones, then the no-break ones.
//!
//! With that, keys order as Apple's do as long as they hold ASCII, the
//! characters the collator ignores (U+FEFF, U+00AD, U+200B, U+2060…), the
//! other spaces and the line breaks: no difference in the equivalence
//! harness's cases, nor in 180,000 more twenty-key objects put through both
//! harnesses' `jsonser` command (60,000 each of printable ASCII, printable
//! ASCII with controls, and those with U+FEFF, U+00AD, U+200B, U+2060,
//! NBSP, U+2000, U+202F, U+3000, NEL, LS and PS mixed in).
//!
//! Two things are not reproduced. A comparison that reaches the collator
//! with more than 64 UTF-16 code units left, which CoreFoundation compares
//! piecewise and this in one go. And the collator beyond those characters,
//! which is approximated: combining marks are ignored like the controls, and
//! any other non-ASCII scalar sorts after every ASCII letter, by its
//! lower-case code point, an upper-case one after it at the case level.
//! Apple's puts `ä` beside `a`, expands `ß` to `ss` and orders scripts and
//! symbols by its own table, so keys with letters beyond ASCII, CJK or emoji
//! mostly come out in another order (a twenty-key object of them almost
//! always differs somewhere); case folding is approximated by lower-casing,
//! plus the few full foldings that differ from it. Only the ORDER of such
//! keys in a rewritten agent config can differ from what the app writes,
//! never the content.

use std::cmp::Ordering;

// ---------------------------------------------------------------------------
// The collator
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Element {
    /// A non-number collation element.
    Weight(u32),
    /// A run of ASCII digits, leading zeros stripped (`""` is zero).
    Number(String),
}

/// Primary weight of every ASCII character that has one.
fn ascii_weight(byte: u8) -> Option<u32> {
    const PUNCTUATION: &[u8] = b"_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
    match byte {
        b'\t' => Some(1),
        b'\n' => Some(2),
        0x0B => Some(3),
        0x0C => Some(4),
        b'\r' => Some(5),
        b' ' => Some(SPACE),
        b'a'..=b'z' => Some(100 + u32::from(byte - b'a')),
        b'A'..=b'Z' => Some(100 + u32::from(byte - b'A')),
        _ => PUNCTUATION
            .iter()
            .position(|&p| p == byte)
            .map(|index| 10 + u32::try_from(index).expect("small")),
    }
}

/// The primary weight of a space.
const SPACE: u32 = 9;

/// Where a number sorts: after every punctuation mark, before letters.
const NUMBER_WEIGHT: u32 = 99;

/// Scalars the collator skips at the primary level: controls, format and
/// other default-ignorable characters, combining marks.
fn ignorable(c: char) -> bool {
    matches!(
        u32::from(c),
        0x00..=0x08
            | 0x0E..=0x1F
            | 0x7F..=0x84
            | 0x86..=0x9F
            | 0xAD
            | 0x0300..=0x036F
            | 0x061C
            | 0x180B..=0x180F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0xFEFF
            | 0x1D173..=0x1D17A
            | 0xE0000..=0xE0FFF
    )
}

/// `c` lower-cased, when that is one scalar.
fn single_lowercase(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(only), None) => only,
        _ => c,
    }
}

/// A scalar's primary weight, whether it is an upper-case letter (the case
/// level) and which space other than U+0020 it is (the tertiary level,
/// after case); `None` for what the collator ignores.
fn collation_element(c: char) -> Option<(u32, u8, u8)> {
    if c.is_ascii() {
        let byte = u8::try_from(c).expect("ASCII");
        return ascii_weight(byte).map(|weight| (weight, u8::from(byte.is_ascii_uppercase()), 0));
    }
    if ignorable(c) {
        return None;
    }
    Some(match c {
        '\u{85}' => (6, 0, 0),
        '\u{2028}' => (7, 0, 0),
        '\u{2029}' => (8, 0, 0),
        // The other spaces: a space at every level above the tertiary,
        // where the wide one sorts first, then the compatibility ones, then
        // the no-break ones (measured).
        '\u{3000}' => (SPACE, 0, 1),
        '\u{1680}' | '\u{2000}'..='\u{2006}' | '\u{2008}'..='\u{200A}' | '\u{205F}' => {
            (SPACE, 0, 2)
        }
        '\u{A0}' | '\u{2007}' | '\u{202F}' => (SPACE, 0, 3),
        c => {
            let lower = single_lowercase(c);
            (1000 + u32::from(lower), u8::from(lower != c), 0)
        }
    })
}

/// What the collator sees of some code units: the primary elements and,
/// for each, its case and space-variant weight.
struct Key {
    elements: Vec<Element>,
    case: Vec<u8>,
    variant: Vec<u8>,
}

fn key(units: &[u16]) -> Key {
    let mut key = Key {
        elements: Vec::new(),
        case: Vec::new(),
        variant: Vec::new(),
    };
    let mut chars = char::decode_utf16(units.iter().copied())
        .map(|c| c.unwrap_or('\u{FFFD}'))
        .peekable();
    while let Some(c) = chars.next() {
        let (element, case, variant) = if c.is_ascii_digit() {
            let mut run = String::from(c);
            while let Some(digit) = chars.next_if(char::is_ascii_digit) {
                run.push(digit);
            }
            (
                Element::Number(run.trim_start_matches('0').to_owned()),
                0,
                0,
            )
        } else {
            match collation_element(c) {
                Some((weight, case, variant)) => (Element::Weight(weight), case, variant),
                None => continue,
            }
        };
        key.elements.push(element);
        key.case.push(case);
        key.variant.push(variant);
    }
    key
}

fn weight(element: &Element) -> u32 {
    match element {
        Element::Weight(weight) => *weight,
        Element::Number(_) => NUMBER_WEIGHT,
    }
}

fn compare_elements(a: &Element, b: &Element) -> Ordering {
    match (a, b) {
        (Element::Number(x), Element::Number(y)) => x.len().cmp(&y.len()).then_with(|| x.cmp(y)),
        _ => weight(a).cmp(&weight(b)),
    }
}

/// `__CompareTextDefault` with forced ordering: primary, case, the space
/// variants, then code units; nothing comes out equal.
fn collate(x: &[u16], y: &[u16]) -> Ordering {
    let (p, q) = (key(x), key(y));
    p.elements
        .iter()
        .zip(&q.elements)
        .map(|(a, b)| compare_elements(a, b))
        .find(|order| order.is_ne())
        .unwrap_or_else(|| p.elements.len().cmp(&q.elements.len()))
        .then_with(|| p.case.cmp(&q.case))
        .then_with(|| p.variant.cmp(&q.variant))
        .then_with(|| x.cmp(y))
        .then(Ordering::Greater)
}

// ---------------------------------------------------------------------------
// CFStringCompareWithOptionsAndLocale
// ---------------------------------------------------------------------------

fn is_high_surrogate(unit: u32) -> bool {
    (0xD800..0xDC00).contains(&unit)
}

fn is_low_surrogate(unit: u32) -> bool {
    (0xDC00..0xE000).contains(&unit)
}

fn combine_surrogates(high: u32, low: u32) -> u32 {
    0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
}

/// Combining marks (`kCFUniCharNonBaseCharacterSet`), approximated by the
/// blocks of them.
fn non_base(unit: u32) -> bool {
    matches!(
        unit,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x05BD
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0x302A..=0x302F
            | 0x3099..=0x309A
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
    )
}

/// Punctuation (`kCFUniCharPunctuationCharacterSet`, general category P):
/// exact for ASCII, approximated beyond it.
fn punctuation(unit: u32) -> bool {
    if let Ok(byte) = u8::try_from(unit) {
        if byte.is_ascii() {
            return b"!\"#%&'()*,-./:;?@[\\]_{}".contains(&byte);
        }
        return matches!(byte, 0xA1 | 0xA7 | 0xAB | 0xB6 | 0xB7 | 0xBB | 0xBF);
    }
    matches!(
        unit,
        0x2010..=0x2027
            | 0x2030..=0x2043
            | 0x2045..=0x2051
            | 0x2053..=0x205E
            | 0x3001..=0x3003
            | 0xFF01..=0xFF03
            | 0xFF05..=0xFF0A
            | 0xFF0C..=0xFF0F
            | 0xFF1A..=0xFF1B
            | 0xFF1F..=0xFF20
    )
}

/// `__extendLocationBackward`: from `at`, back over combining marks and
/// punctuation. It stops at any surrogate pair: CoreFoundation hands the
/// pair to its decoder low half first, and the scalar that makes is never
/// in a plane it extends over.
fn extend_back(units: &[u16], mut at: usize) -> usize {
    while at > 0 {
        let unit = u32::from(units[at]);
        if is_low_surrogate(unit) && is_high_surrogate(u32::from(units[at - 1])) {
            break;
        }
        if !(non_base(unit) || punctuation(unit)) || (0x2E80..0xAC00).contains(&unit) {
            break;
        }
        at -= 1;
    }
    at
}

/// `_CFCompareStringsWithLocale` from `from1` and `from2` to the ends: the
/// collator over both, each from one code unit further back (and over
/// punctuation and combining marks before that).
fn localized(s1: &[u16], from1: usize, s2: &[u16], from2: usize) -> Ordering {
    let start1 = if from1 > 0 {
        extend_back(s1, from1 - 1)
    } else {
        0
    };
    let start2 = if from2 > 0 {
        extend_back(s2, from2 - 1)
    } else {
        0
    };
    collate(&s1[start1..], &s2[start2..])
}

/// `__CFStringFoldCharacterClusterAtIndex` for `.caseInsensitive`: an ASCII
/// capital's small letter, a non-ASCII scalar's case folding (lower-casing,
/// plus the full foldings that differ from it); empty when nothing folds.
fn fold(c: u32) -> Vec<u32> {
    if c < 0x80 {
        return if (0x41..=0x5A).contains(&c) {
            vec![c + 0x20]
        } else {
            Vec::new()
        };
    }
    let Some(scalar) = char::from_u32(c) else {
        return Vec::new();
    };
    let folded: Vec<u32> = match scalar {
        'ß' | 'ẞ' => vec![0x73, 0x73],
        'ſ' => vec![0x73],
        'ς' => vec![0x3C3],
        'ﬀ' => vec![0x66, 0x66],
        'ﬁ' => vec![0x66, 0x69],
        'ﬂ' => vec![0x66, 0x6C],
        'ﬃ' => vec![0x66, 0x66, 0x69],
        'ﬄ' => vec![0x66, 0x66, 0x6C],
        'ﬅ' | 'ﬆ' => vec![0x73, 0x74],
        _ => scalar.to_lowercase().map(u32::from).collect(),
    };
    if folded == [c] { Vec::new() } else { folded }
}

/// The code unit at `at`, 0 past the end (`CFStringGetCharacterFromInlineBuffer`).
fn unit(units: &[u16], at: usize) -> u32 {
    units.get(at).map_or(0, |&unit| u32::from(unit))
}

fn is_digit(c: u32) -> bool {
    (0x30..=0x39).contains(&c)
}

/// One side of the walk: where it stands, the folded characters it still
/// has to compare, and how many code units the current character takes.
struct Side<'a> {
    units: &'a [u16],
    index: usize,
    folded: Vec<u32>,
    next_folded: usize,
    used: usize,
}

impl<'a> Side<'a> {
    fn new(units: &'a [u16]) -> Self {
        Side {
            units,
            index: 0,
            folded: Vec::new(),
            next_folded: 0,
            used: 1,
        }
    }

    /// The next character: a folded one still pending, or the code unit at
    /// the index, an ASCII capital lowered once `case_blind`.
    fn read(&mut self, case_blind: bool) -> u32 {
        if self.folded.is_empty() {
            self.used = 1;
            let c = unit(self.units, self.index);
            if case_blind && (0x41..=0x5A).contains(&c) {
                c + 0x20
            } else {
                c
            }
        } else {
            self.next_folded += 1;
            self.folded[self.next_folded - 1]
        }
    }

    /// The number written at the index, the index moved past it.
    fn number(&mut self) -> (u64, usize) {
        let start = self.index;
        let mut value: u64 = 0;
        while is_digit(unit(self.units, self.index)) {
            value = value
                .wrapping_mul(10)
                .wrapping_add(u64::from(unit(self.units, self.index) - 0x30));
            self.index += 1;
        }
        (value, self.index - start)
    }

    /// A surrogate pair at the index read as one scalar.
    fn scalar(&mut self, c: u32) -> u32 {
        let low = unit(self.units, self.index + 1);
        if is_high_surrogate(c) && is_low_surrogate(low) {
            self.used = 2;
            combine_surrogates(c, low)
        } else {
            c
        }
    }

    /// Fold `c`; the first folded character, or `c` if nothing folds.
    fn fold(&mut self, c: u32) -> u32 {
        let folded = fold(c);
        if folded.is_empty() {
            return c;
        }
        self.folded = folded;
        self.next_folded = 1;
        self.folded[0]
    }

    fn pending(&self) -> bool {
        !self.folded.is_empty()
    }

    fn advance(&mut self) {
        if self.pending() && self.next_folded == self.folded.len() {
            self.folded.clear();
        }
        if !self.pending() {
            self.index += self.used;
        }
    }
}

/// `a` against `b` in `JSONSerialization`'s `.sortedKeys` order.
pub fn compare(a: &str, b: &str) -> Ordering {
    let s1: Vec<u16> = a.encode_utf16().collect();
    let s2: Vec<u16> = b.encode_utf16().collect();
    let (mut one, mut two) = (Side::new(&s1), Side::new(&s2));
    // Where the collator starts when a non-ASCII pair differs.
    let (mut local1, mut local2) = (0, 0);
    // Where it starts to settle a recorded non-ASCII case difference.
    let (mut forced1, mut forced2) = (0, 0);
    let mut result = Ordering::Equal;
    let mut digit_count_decides = false;
    while one.index < s1.len() && two.index < s2.len() {
        let case_blind = result.is_ne();
        let mut c1 = one.read(case_blind);
        let mut c2 = two.read(case_blind);

        if !one.pending() && !two.pending() && is_digit(c1) && is_digit(c2) {
            let (start1, start2) = (one.index, two.index);
            let ((v1, n1), (v2, n2)) = (one.number(), two.number());
            if v1 != v2 {
                return v1.cmp(&v2);
            }
            if result.is_eq() && n1 != n2 {
                result = n1.cmp(&n2);
                digit_count_decides = true;
                (forced1, forced2) = (start1, start2);
            }
            continue;
        }

        if c1 != c2 {
            if result.is_eq() {
                result = c1.cmp(&c2);
                (forced1, forced2) = (local1, local2);
            }
            if c1 < 0x80 && c2 < 0x80 {
                return localized(&s1, one.index, &s2, two.index);
            }
            c1 = one.scalar(c1);
            c2 = two.scalar(c2);
            if c1 != c2 {
                if !one.pending() {
                    c1 = one.fold(c1);
                }
                if !one.pending() && two.pending() {
                    return localized(&s1, local1, &s2, local2);
                }
                if !two.pending() && (!one.pending() || c1 != c2) {
                    c2 = two.fold(c2);
                    if !two.pending() || c1 != c2 {
                        return localized(&s1, local1, &s2, local2);
                    }
                }
            }
            if one.pending() && two.pending() {
                while one.next_folded < one.folded.len()
                    && two.next_folded < two.folded.len()
                    && one.folded[one.next_folded] == two.folded[two.next_folded]
                {
                    one.next_folded += 1;
                    two.next_folded += 1;
                }
                if one.next_folded < one.folded.len() && two.next_folded < two.folded.len() {
                    return localized(&s1, local1, &s2, local2);
                }
            }
        }

        one.advance();
        two.advance();
        if !one.pending() && !two.pending() {
            (local1, local2) = (one.index, two.index);
        }
    }
    if !digit_count_decides && result.is_ne() && one.index == s1.len() && two.index == s2.len() {
        result = localized(&s1, forced1, &s2, forced2);
    }
    if one.index < s1.len() {
        Ordering::Greater
    } else if two.index < s2.len() {
        Ordering::Less
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(keys: &[&str]) -> Vec<String> {
        let mut keys: Vec<String> = keys.iter().map(|&k| k.to_owned()).collect();
        keys.sort_by(|a, b| compare(a, b));
        keys
    }

    /// Orders `JSONSerialization` wrote on macOS 27.
    #[test]
    fn printable_ascii_in_apples_order() {
        let all: Vec<String> = (0x20u8..0x7f).map(|b| char::from(b).to_string()).collect();
        let refs: Vec<&str> = all.iter().map(String::as_str).collect();
        assert_eq!(
            sorted(&refs).concat(),
            " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789aAbBcCdDeEfFgGhHiIjJkKlLmMnNoOpPqQrRsStTuUvVwWxXyYzZ"
        );
    }

    #[test]
    fn controls_whitespace_and_ignorables() {
        let keys: Vec<String> = (0x01u8..0x20)
            .map(|b| format!("a{}b", char::from(b)))
            .chain(["ab".to_owned(), "a b".to_owned(), "a-b".to_owned()])
            .collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let got = sorted(&refs);
        let mut want = vec!["a\tb", "a\nb", "a\u{b}b", "a\u{c}b", "a\rb", "a b", "a-b"];
        let ignorable: Vec<String> = (0x01u8..0x20)
            .filter(|b| !(0x09..=0x0d).contains(b))
            .map(|b| format!("a{}b", char::from(b)))
            .collect();
        want.extend(ignorable.iter().map(String::as_str));
        want.push("ab");
        assert_eq!(got, want);
    }

    /// Orders `JSONSerialization` wrote on macOS 27 for each scalar X.
    #[test]
    fn ignorables_line_breaks_and_spaces_beyond_ascii() {
        for x in [
            "\u{AD}",
            "\u{200B}",
            "\u{200C}",
            "\u{200D}",
            "\u{2060}",
            "\u{FEFF}",
            "\u{301}",
            "\u{34F}",
            "\u{FE0F}",
            "\u{180E}",
            "\u{2061}",
            "\u{206A}",
            "\u{E0001}",
            "\u{7F}",
            "\u{80}",
            "\u{9F}",
        ] {
            let keys = [
                "ab",
                "a b",
                "a-b",
                "abc",
                &format!("a{x}b"),
                "aa",
                "ac",
                &format!("a{x}"),
                "a",
            ];
            let want = vec![
                "a".to_owned(),
                format!("a{x}"),
                "a b".into(),
                "a-b".into(),
                "aa".into(),
                "ab".into(),
                format!("a{x}b"),
                "abc".into(),
                "ac".into(),
            ];
            assert_eq!(
                sorted(&keys),
                want,
                "U+{:04X}",
                u32::from(x.chars().next().unwrap())
            );
        }
        for x in ["\u{85}", "\u{2028}"] {
            let keys = [
                "ab",
                "a b",
                "a-b",
                "abc",
                &format!("a{x}b"),
                "aa",
                "ac",
                &format!("a{x}"),
                "a",
            ];
            let want = vec![
                "a".to_owned(),
                format!("a{x}"),
                format!("a{x}b"),
                "a b".into(),
                "a-b".into(),
                "aa".into(),
                "ab".into(),
                "abc".into(),
                "ac".into(),
            ];
            assert_eq!(sorted(&keys), want);
        }
        for x in ["\u{A0}", "\u{3000}"] {
            let keys = [
                "ab",
                "a b",
                "a-b",
                "abc",
                &format!("a{x}b"),
                "aa",
                "ac",
                &format!("a{x}"),
                "a",
            ];
            let want = vec![
                "a".to_owned(),
                format!("a{x}"),
                "a b".into(),
                format!("a{x}b"),
                "a-b".into(),
                "aa".into(),
                "ab".into(),
                "abc".into(),
                "ac".into(),
            ];
            assert_eq!(sorted(&keys), want);
        }
    }

    /// Pairs `JSONSerialization` ordered on macOS 27, each the same both ways
    /// round.
    fn assert_pairs(pairs: &[(&str, &str)]) {
        for &(first, second) in pairs {
            assert_eq!(
                compare(first, second),
                Ordering::Less,
                "{first:?} < {second:?}"
            );
            assert_eq!(
                compare(second, first),
                Ordering::Greater,
                "{second:?} > {first:?}"
            );
        }
    }

    #[test]
    fn zeros_case_and_ignorables() {
        assert_pairs(&[
            ("1A", "01a"),
            ("x", "\u{1}X"),
            ("\u{1}01", "1"),
            ("1a", "1A"),
            ("1a", "01a"),
            ("01a", "\u{1}1A"),
            ("\u{1}01a", "1A"),
            ("a1", "A01"),
            ("a01", "A1"),
            ("\u{1}a", "A"),
            ("\u{1}\u{1}01", "1"),
            ("\u{1}#\u{FEFF}", "#"),
            ("9", "09"),
            ("\u{1}00", "0"),
            ("\u{1}0", "0"),
            ("0\u{FEFF}", "\u{AD}0\u{FEFF}"),
        ]);
        assert_eq!(
            sorted(&["1", "\u{1}1", "01", "\u{1}01", "001"]),
            ["\u{1}1", "\u{1}01", "1", "01", "001"]
        );
    }

    /// After a digit-count tie the walk is blind to ASCII case, and a key
    /// that runs out first sorts first, whatever is left of the other —
    /// controls the collator would ignore included.
    #[test]
    fn after_a_digit_count_tie() {
        assert_pairs(&[
            ("001Y", "01y\u{1}"),
            ("001y", "01Y\u{1}"),
            ("01y", "001Y\u{1}"),
            ("01Y", "001y\u{1}"),
            ("01Y", "1y\u{1}"),
            ("a01Y", "a1y\u{1}"),
            ("01A", "001a\u{1}"),
            ("01Y", "1y\u{200B}"),
            ("1y", "01Y\u{200B}"),
            ("01", "001\u{1}"),
            ("001", "01\u{1}"),
            ("1aB", "01Ab"),
            ("1Ab", "01aB"),
            ("y", "Y\u{1}"),
            ("y\u{1}", "Y"),
        ]);
    }

    /// The collator takes over from one code unit before an ASCII pair that
    /// differs (back over punctuation), and from the last pair matched
    /// outside a number when the pair is not ASCII.
    #[test]
    fn where_the_collator_takes_over() {
        assert_pairs(&[
            ("001\u{1}", "01\u{2}"),
            ("1\u{1}", "01\u{2}"),
            ("01\u{1}", "1\u{2}"),
            ("1a\u{2}", "01A\u{1}"),
            ("01a\u{2}", "1A\u{1}"),
            ("01Ab\u{1}", "1ab\u{2}"),
            ("01A1\u{1}", "1a01\u{2}"),
            ("1Ab\u{2}", "01aB\u{1}"),
            ("01Ab\u{1}", "1aB\u{2}"),
            ("1a02\u{1}", "01a2\u{2}"),
            ("1-a\u{2}", "01-A\u{1}"),
            ("01-a\u{1}", "1-A\u{2}"),
            ("x-a\u{2}", "x-A\u{1}"),
            ("01\u{200B}", "1\u{1}"),
            ("01\u{200B}", "1\u{AD}"),
            ("01\u{AD}", "1\u{200B}"),
            ("01\u{1}", "1\u{200B}"),
            ("1a02\u{200B}", "01a2\u{1}"),
            ("1Ab\u{1}", "01aB\u{200B}"),
            ("01Ab\u{200B}", "1aB\u{1}"),
            ("1x\u{1}", "01x\u{200B}"),
            ("01a1\u{200B}", "1A01\u{1}"),
            ("1a01\u{1}", "01A1\u{200B}"),
            ("01\u{200B}x", "1\u{1}x"),
            ("x01\u{200B}", "x1\u{1}"),
            ("1a\u{AD}", "01A\u{200B}"),
            ("01a\u{AD}", "1A\u{200B}"),
            ("0\u{1}1", "\u{2}1"),
            ("\u{1}01", "\u{2}1"),
        ]);
    }

    /// Numbers are read into a `u64` that wraps.
    #[test]
    fn numbers_wrap_at_two_to_the_sixty_fourth() {
        assert_pairs(&[
            ("18446744073709551616", "1"),
            ("18446744073709551617", "2"),
            ("a18446744073709551616", "a1"),
            ("18446744073709551616", "18446744073709551615"),
            ("0y", "18446744073709551616x"),
            ("A1", "a18446744073709551616"),
        ]);
    }

    /// The other spaces sort as a space, after it only once case has been
    /// compared: the wide one first, then the compatibility ones, then the
    /// no-break ones.
    #[test]
    fn spaces_other_than_u0020() {
        let spaces = [
            " ", "\u{3000}", "\u{1680}", "\u{2000}", "\u{2001}", "\u{2002}", "\u{2003}",
            "\u{2004}", "\u{2005}", "\u{2006}", "\u{2008}", "\u{2009}", "\u{200A}", "\u{205F}",
            "\u{A0}", "\u{2007}", "\u{202F}",
        ];
        let mut shuffled = spaces;
        shuffled.reverse();
        assert_eq!(sorted(&shuffled), spaces);
        assert_pairs(&[
            ("\u{2}\u{3000}", "\u{1}\u{2000}"),
            ("\u{2}\u{2000}", "\u{1}\u{A0}"),
            ("\u{2} ", "\u{1}\u{1680}"),
            ("a\u{2007}", "A\u{3000}"),
            ("a\u{3000}", "A\u{2007}"),
            ("\u{2}a b", "\u{1}a\u{A0}b"),
            ("a\u{A0}a", "a A"),
            ("a a", "a\u{A0}A"),
            ("a\u{A0}b", "A b"),
            ("a\u{A0}", "A "),
            ("1 ", "01\u{A0}"),
            ("01 ", "1\u{A0}"),
        ]);
    }

    /// Non-ASCII letters fold in the walk; the case they differ in is
    /// settled by the collator, lower first, unless a digit count was
    /// recorded first.
    #[test]
    fn case_beyond_ascii() {
        assert_pairs(&[
            ("ä", "Ä"),
            ("äb", "ÄB"),
            ("äB", "Äb"),
            ("äa", "Äb"),
            ("Äa", "äb"),
            ("ä01", "Ä1"),
            ("ä1", "Ä01"),
            ("é01", "É1"),
            ("1é01", "01É1"),
            ("1ä", "01Ä"),
            ("1Ä", "01ä"),
            ("1ä", "01ä"),
            ("ä1", "ä01"),
            ("ä1A", "ä01a"),
            ("ä", "ä\u{1}"),
            ("äa", "äA"),
            ("1Bä", "01bä"),
            ("1bä", "01Bä"),
            ("ss", "ß"),
            ("ssb", "ßa"),
            ("ssa", "ßb"),
        ]);
    }

    #[test]
    fn case_and_numbers() {
        assert_eq!(
            sorted(&[
                "ab", "aB", "Ab", "AB", "a", "A", "b", "B", "aa", "AA", "Aa", "aA", "a_", "A_",
                "_a", "_A"
            ]),
            [
                "_a", "_A", "a", "A", "a_", "A_", "aa", "aA", "Aa", "AA", "ab", "aB", "Ab", "AB",
                "b", "B"
            ]
        );
        assert_eq!(
            sorted(&[
                "a1",
                "a01",
                "a001",
                "a10",
                "a2",
                "a1b",
                "a01b",
                "a1a",
                "1",
                "01",
                "001",
                "0",
                "00",
                "1.5",
                "1.10",
                "1.9",
                "a1.2",
                "a1.10",
                "-1",
                "-2",
                "a-1",
                "a1-",
                "99999999999999999999",
                "100000000000000000000",
                "x1y2",
                "x1y10",
                "x01y2",
                "x1y02",
                "1a",
                "1A",
                "01a"
            ]),
            [
                "-1",
                "-2",
                "0",
                "00",
                "1",
                "01",
                "001",
                "1.5",
                "1.9",
                "1.10",
                "1a",
                "1A",
                "01a",
                "99999999999999999999",
                "100000000000000000000",
                "a-1",
                "a1",
                "a01",
                "a001",
                "a1-",
                "a1.2",
                "a1.10",
                "a1a",
                "a1b",
                "a01b",
                "a2",
                "a10",
                "x1y2",
                "x1y02",
                "x01y2",
                "x1y10"
            ]
        );
        assert_eq!(
            sorted(&["abc", "ab", "abcd", "a", "", "b", "abd", "aB", "abC", "ABC"]),
            ["", "a", "ab", "aB", "abc", "abC", "ABC", "abcd", "abd", "b"]
        );
    }
}
