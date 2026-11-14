//! JSON the way the app reads it.
//!
//! Every herdr payload the Swift code looks at goes through
//! `JSONSerialization.jsonObject(with:)` and Swift's `as?` casts, so what
//! counts as JSON, and what a number turns into, is Foundation's rule, not
//! RFC 8259's. This module writes those rules out, as measured on macOS 27
//! (the probes behind every rule are `harness/swift/probe-foundation.swift`), so
//! that the Rust port reads every herdr line exactly as the app does:
//!
//! - Only an object or an array at the top, JSON whitespace (space, tab,
//!   LF, CR) around it, nothing after it. A leading UTF-8 byte order mark is
//!   skipped.
//! - One trailing comma is allowed before `]` and `}` (`[1,]`, `{"a":1,}`);
//!   `[,]` and `[1,,]` are not.
//! - A key that appears twice keeps its FIRST value.
//! - No value may sit inside 513 or more containers.
//! - Strings: no raw control characters, the eight escapes and `\uXXXX`
//!   (either case), surrogate pairs only as pairs, valid UTF-8 throughout.
//! - Numbers follow the JSON grammar. An integer literal is an `Int64`, a
//!   `UInt64` above that, or a decimal beyond. A literal with a fraction or
//!   an exponent is a double when it has at most 17 digits (not counting a
//!   leading `0` integer part) and a decimal otherwise. An exponent of more
//!   than four characters, sign included, is refused, and so is a double
//!   that overflows to +∞ (−∞ is kept).
//!
//! The casts ([`Value::as_i64`], [`Value::as_u64`], [`Value::as_bool`]) take
//! a number by exact value: `1.0` is the `Int` 1, `1` and `1.0` are `true`,
//! `0` is `false`, `true` is the `Int` 1, `2` is no `Bool` at all.
//!
//! Three things Foundation does are not ported (each is a divergence in
//! `harness/divergences.json`, with a fixture):
//!
//! - **json-bom**: Foundation drops one leading U+FEFF from every string,
//!   key or value, raw or escaped. Strings here keep every character.
//! - **json-encoding-sniff**: Foundation reads bytes whose second and fourth
//!   bytes are NUL (five bytes or more) as UTF-16 or UTF-32. Here JSON is
//!   UTF-8, as RFC 8259 requires of JSON on a network.
//! - **json-decimal**: Foundation's `as?` casts on an `NSDecimalNumber` are
//!   not exact (`100000000000000000.5` is the `Int` 100000000000000000,
//!   `0.999…9` with 41 nines is `true`). Here a decimal converts only when
//!   its value is exactly that integer or Boolean. herdr writes integers and
//!   Booleans, never such literals.

use std::collections::BTreeMap;

/// A parsed JSON value, as Swift sees what `JSONSerialization` returns.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Value>),
    Object(Object),
}

/// A JSON number, in the class `JSONSerialization` gives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    /// An integer literal that fits `Int64` (an `NSNumber` of type `q`).
    Int(i64),
    /// An integer literal above `Int64.max` that fits `UInt64` (`Q`).
    UInt(u64),
    /// A literal with a fraction or exponent and at most 17 digits (`d`).
    Double(f64),
    /// What `JSONSerialization` hands back as an `NSDecimalNumber`: an
    /// integer beyond 64 bits, or a fraction or exponent with more than 17
    /// digits.
    Decimal(Decimal),
}

/// `(-1)^negative × mantissa × 10^exponent`. The mantissa keeps the digits
/// that fit 128 bits, as `NSDecimal` does; later ones are dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decimal {
    pub negative: bool,
    pub mantissa: u128,
    pub exponent: i32,
}

/// A JSON object: one value per key, the first one written.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Object {
    entries: BTreeMap<String, Value>,
}

impl Object {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.get(key)
    }

    /// `object[key] as? String`.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// `object[key] as? Int`.
    pub fn i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Value::as_i64)
    }

    /// `object[key] as? UInt64`.
    pub fn u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(Value::as_u64)
    }

    /// `object[key] as? Bool`.
    pub fn bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Value::as_bool)
    }

    /// `object[key] as? [String: Any]`.
    pub fn object(&self, key: &str) -> Option<&Object> {
        self.get(key).and_then(Value::as_object)
    }

    /// `object[key] as? [[String: Any]]`: every element an object, or none.
    pub fn objects(&self, key: &str) -> Option<Vec<&Object>> {
        self.get(key).and_then(Value::as_objects)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries in key order (by UTF-8 bytes).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_str(), value))
    }

    fn insert_first(&mut self, key: String, value: Value) {
        self.entries.entry(key).or_insert(value);
    }
}

const I64_BOUND: f64 = 9_223_372_036_854_775_808.0; // 2^63
const U64_BOUND: f64 = 18_446_744_073_709_551_616.0; // 2^64

impl Value {
    /// `value as? String`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    /// `value as? [String: Any]`.
    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Value::Object(object) => Some(object),
            _ => None,
        }
    }

    /// `value as? [[String: Any]]`. All or nothing: one element that is not
    /// an object fails the whole cast, as the Swift cast does.
    pub fn as_objects(&self) -> Option<Vec<&Object>> {
        match self {
            Value::Array(items) => items.iter().map(Value::as_object).collect(),
            _ => None,
        }
    }

    /// `value as? Int`: the number's exact value, when it is an integer that
    /// fits. `true` and `false` are 1 and 0.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Bool(flag) => Some(i64::from(*flag)),
            Value::Number(Number::Int(value)) => Some(*value),
            Value::Number(Number::UInt(_)) => None,
            Value::Number(Number::Double(value)) => {
                (value.fract() == 0.0 && *value >= -I64_BOUND && *value < I64_BOUND)
                    .then_some(*value as i64)
            }
            Value::Number(Number::Decimal(decimal)) => decimal
                .integer()
                .and_then(|value| i64::try_from(value).ok()),
            _ => None,
        }
    }

    /// `value as? UInt64`, by the same rule as [`Value::as_i64`].
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Bool(flag) => Some(u64::from(*flag)),
            Value::Number(Number::Int(value)) => u64::try_from(*value).ok(),
            Value::Number(Number::UInt(value)) => Some(*value),
            Value::Number(Number::Double(value)) => {
                (value.fract() == 0.0 && *value >= 0.0 && *value < U64_BOUND)
                    .then_some(*value as u64)
            }
            Value::Number(Number::Decimal(decimal)) => decimal
                .integer()
                .and_then(|value| u64::try_from(value).ok()),
            _ => None,
        }
    }

    /// `value as? Bool`: a Boolean, or a number that is exactly 0 or 1.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(flag) => Some(*flag),
            Value::Number(Number::Int(0)) => Some(false),
            Value::Number(Number::Int(1)) => Some(true),
            Value::Number(Number::Double(value)) if *value == 0.0 => Some(false),
            Value::Number(Number::Double(value)) if *value == 1.0 => Some(true),
            Value::Number(Number::Decimal(decimal)) => match decimal.integer() {
                Some(0) => Some(false),
                Some(1) => Some(true),
                _ => None,
            },
            _ => None,
        }
    }
}

impl Decimal {
    /// The value, when it is an integer within ±2^127.
    pub fn integer(&self) -> Option<i128> {
        let magnitude = if self.mantissa == 0 {
            0
        } else if self.exponent >= 0 {
            let scale = 10u128.checked_pow(u32::try_from(self.exponent).ok()?)?;
            self.mantissa.checked_mul(scale)?
        } else {
            // A divisor past 10^38 exceeds every nonzero mantissa.
            let scale = 10u128.checked_pow(self.exponent.unsigned_abs())?;
            if !self.mantissa.is_multiple_of(scale) {
                return None;
            }
            self.mantissa / scale
        };
        let magnitude = i128::try_from(magnitude).ok()?;
        Some(if self.negative { -magnitude } else { magnitude })
    }
}

/// How deep a value may sit: one inside 513 containers is refused.
const MAX_DEPTH: usize = 513;

/// Longest exponent, sign included.
const MAX_EXPONENT_CHARS: usize = 4;

/// Digits a fraction or exponent literal may have and still be a double.
const MAX_DOUBLE_DIGITS: usize = 17;

/// `NSDecimal`'s exponent range.
const DECIMAL_EXPONENTS: std::ops::RangeInclusive<i32> = -128..=127;

/// `JSONSerialization.jsonObject(with:)` with no options: an object or an
/// array, or `None` where Foundation throws.
pub fn parse(bytes: &[u8]) -> Option<Value> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mut parser = Parser { bytes, at: 0 };
    parser.skip_whitespace();
    if !matches!(parser.peek(), Some(b'{' | b'[')) {
        return None;
    }
    let value = parser.value(0)?;
    parser.skip_whitespace();
    (parser.at == bytes.len()).then_some(value)
}

/// `try? JSONSerialization.jsonObject(with: bytes) as? [String: Any]`.
pub fn parse_object(bytes: &[u8]) -> Option<Object> {
    match parse(bytes)? {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

/// The first top-level object in `text`: from its first `{` to the brace
/// that closes it (string-aware, so a `{` inside a label or a path doesn't
/// count), parsed. A port of `HerdrSnapshot.firstJSONObject(in:)`, which
/// every herdr reply in the app goes through: a remote shell can print a
/// banner or an MOTD before the CLI's output. `None` when there is no `{`,
/// when the object never closes, or when what it spans is not JSON. Only the
/// first object is tried.
pub fn first_object(text: &str) -> Option<Object> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|&byte| byte == b'{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, &byte) in bytes.iter().enumerate().skip(start) {
        if escaped {
            escaped = false;
            continue;
        }
        if in_string {
            match byte {
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return parse_object(&bytes[start..=index]);
                }
            }
            _ => {}
        }
    }
    None
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth >= MAX_DEPTH {
            return None;
        }
        match self.peek()? {
            b'{' => self.object(depth).map(Value::Object),
            b'[' => self.array(depth).map(Value::Array),
            b'"' => self.string().map(Value::String),
            b't' => self.literal(b"true", Value::Bool(true)),
            b'f' => self.literal(b"false", Value::Bool(false)),
            b'n' => self.literal(b"null", Value::Null),
            b'-' | b'0'..=b'9' => self.number().map(Value::Number),
            _ => None,
        }
    }

    fn literal(&mut self, word: &[u8], value: Value) -> Option<Value> {
        if self.bytes[self.at..].starts_with(word) {
            self.at += word.len();
            Some(value)
        } else {
            None
        }
    }

    fn object(&mut self, depth: usize) -> Option<Object> {
        self.at += 1; // {
        let mut object = Object::default();
        self.skip_whitespace();
        if self.peek()? == b'}' {
            self.at += 1;
            return Some(object);
        }
        loop {
            if self.peek()? != b'"' {
                return None;
            }
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek()? != b':' {
                return None;
            }
            self.at += 1;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            object.insert_first(key, value);
            self.skip_whitespace();
            match self.peek()? {
                b'}' => {
                    self.at += 1;
                    return Some(object);
                }
                b',' => {
                    self.at += 1;
                    self.skip_whitespace();
                    // One trailing comma is fine.
                    if self.peek()? == b'}' {
                        self.at += 1;
                        return Some(object);
                    }
                }
                _ => return None,
            }
        }
    }

    fn array(&mut self, depth: usize) -> Option<Vec<Value>> {
        self.at += 1; // [
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek()? == b']' {
            self.at += 1;
            return Some(items);
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek()? {
                b']' => {
                    self.at += 1;
                    return Some(items);
                }
                b',' => {
                    self.at += 1;
                    self.skip_whitespace();
                    if self.peek()? == b']' {
                        self.at += 1;
                        return Some(items);
                    }
                }
                _ => return None,
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.bytes.get(self.at..self.at + 4)?;
        let mut value = 0;
        for &digit in digits {
            value = value * 16 + char::from(digit).to_digit(16)?;
        }
        self.at += 4;
        Some(value)
    }

    fn string(&mut self) -> Option<String> {
        self.at += 1; // "
        let mut out = Vec::new();
        loop {
            let byte = self.peek()?;
            self.at += 1;
            match byte {
                b'"' => break,
                0x00..=0x1F => return None,
                b'\\' => {
                    let escape = self.peek()?;
                    self.at += 1;
                    let scalar = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{C}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.escaped_scalar()?,
                        _ => return None,
                    };
                    let mut buffer = [0; 4];
                    out.extend_from_slice(scalar.encode_utf8(&mut buffer).as_bytes());
                }
                _ => out.push(byte),
            }
        }
        String::from_utf8(out).ok()
    }

    /// After `\u`: one scalar, or a surrogate pair written as two escapes.
    fn escaped_scalar(&mut self) -> Option<char> {
        let first = self.hex4()?;
        match first {
            0xD800..=0xDBFF => {
                if !self.bytes[self.at..].starts_with(b"\\u") {
                    return None;
                }
                self.at += 2;
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return None;
                }
                char::from_u32(0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00))
            }
            0xDC00..=0xDFFF => None,
            _ => char::from_u32(first),
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Option<Number> {
        let start = self.at;
        let negative = self.peek() == Some(b'-');
        if negative {
            self.at += 1;
        }
        let integer_start = self.at;
        let integer_digits = self.digits();
        if integer_digits == 0 || (integer_digits > 1 && self.bytes[integer_start] == b'0') {
            return None;
        }
        let integer = &self.bytes[integer_start..self.at];
        let mut fraction: &[u8] = &[];
        if self.peek() == Some(b'.') {
            self.at += 1;
            let fraction_start = self.at;
            if self.digits() == 0 {
                return None;
            }
            fraction = &self.bytes[fraction_start..self.at];
        }
        let mut exponent: Option<&[u8]> = None;
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            let exponent_start = self.at;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return None;
            }
            exponent = Some(&self.bytes[exponent_start..self.at]);
        }
        let literal = std::str::from_utf8(&self.bytes[start..self.at]).ok()?;

        if fraction.is_empty() && exponent.is_none() {
            return integer_number(negative, integer);
        }
        let exponent = match exponent {
            Some(text) if text.len() > MAX_EXPONENT_CHARS => return None,
            Some(text) => std::str::from_utf8(text).ok()?.parse::<i32>().ok()?,
            None => 0,
        };
        let counted = if integer == b"0" { 0 } else { integer.len() } + fraction.len();
        if counted <= MAX_DOUBLE_DIGITS {
            let value: f64 = literal.parse().ok()?;
            // +∞ is refused, −∞ kept: Foundation's own asymmetry.
            return (value != f64::INFINITY).then_some(Number::Double(value));
        }
        decimal_number(negative, integer, fraction, exponent).map(Number::Decimal)
    }
}

/// An integer literal: `Int64`, else `UInt64`, else a decimal (`None` when
/// that overflows, as Foundation's does).
fn integer_number(negative: bool, digits: &[u8]) -> Option<Number> {
    let mut magnitude: u128 = 0;
    let mut overflow = false;
    for &digit in digits {
        match magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(u128::from(digit - b'0')))
        {
            Some(value) if !overflow => magnitude = value,
            _ => overflow = true,
        }
    }
    if !overflow {
        if negative {
            if magnitude <= 1u128 << 63 {
                return Some(Number::Int((magnitude as i128).wrapping_neg() as i64));
            }
        } else if let Ok(value) = i64::try_from(magnitude) {
            return Some(Number::Int(value));
        } else if let Ok(value) = u64::try_from(magnitude) {
            return Some(Number::UInt(value));
        }
    }
    decimal_number(negative, digits, &[], 0).map(Number::Decimal)
}

/// A decimal: digits kept while the mantissa fits 128 bits, the rest of the
/// integer part moving the exponent and the rest of the fraction dropped.
fn decimal_number(
    negative: bool,
    integer: &[u8],
    fraction: &[u8],
    exponent: i32,
) -> Option<Decimal> {
    let mut mantissa: u128 = 0;
    let mut exponent = i64::from(exponent);
    let mut full = false;
    for (index, &digit) in integer.iter().chain(fraction).enumerate() {
        let in_fraction = index >= integer.len();
        if !full {
            match mantissa
                .checked_mul(10)
                .and_then(|value| value.checked_add(u128::from(digit - b'0')))
            {
                Some(value) => {
                    mantissa = value;
                    if in_fraction {
                        exponent -= 1;
                    }
                    continue;
                }
                None => full = true,
            }
        }
        if !in_fraction {
            exponent += 1;
        }
    }
    let exponent = i32::try_from(exponent).ok()?;
    if mantissa != 0 && !DECIMAL_EXPONENTS.contains(&exponent) {
        return None;
    }
    Some(Decimal {
        negative,
        mantissa,
        exponent,
    })
}

/// A string the way `JSONEncoder` writes it: `"`, `\` and `/` escaped, the
/// usual short escapes for backspace, form feed, newline, return and tab,
/// `\u00xx` (lowercase) for the other control characters, everything else
/// as itself.
pub fn encode_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for scalar in text.chars() {
        match scalar {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            '\u{8}' => out.push_str("\\b"),
            '\u{C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0}'..='\u{1F}' => out.push_str(&format!("\\u{:04x}", u32::from(scalar))),
            _ => out.push(scalar),
        }
    }
    out.push('"');
    out
}
