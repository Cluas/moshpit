//! JSON as Foundation reads and writes it.
//!
//! Reading, both of Foundation's parsers (macOS 27) differ from RFC 8259 the
//! same ways: a UTF-8 byte order mark is skipped, one trailing comma before
//! `]` or `}` is accepted, and of two equal keys the FIRST wins. They differ
//! from each other in two ways, which [`Dialect`] names:
//!
//! - `JSONDecoder` takes any top-level value, and checks a number only when
//!   it decodes it: an ignored field may hold `01`, `1.2.3` or `-`, while
//!   `"ts": 007` fails the decode.
//! - `JSONSerialization.jsonObject(with:)` wants an array or object at the
//!   top unless told `.fragmentsAllowed`, and checks every number as it reads
//!   it, deciding its type there: an integer fitting `Int64` or `UInt64` stays
//!   one, a literal with a fraction or exponent and at most 17 significant
//!   digits becomes a `Double`, anything longer an `NSDecimalNumber` (see
//!   [`Number::classify`]).
//!
//! Writing, [`write_encoder`] is `JSONEncoder` (`/` escaped as `\/`, shortest
//! `Double`s, keys by code point when sorted) and [`write_serialization`] is
//! `JSONSerialization` (`%.17g` `Double`s, keys in [`super::collation`] order).
//! Pretty output is theirs: two spaces, `"key" : value`, an empty container
//! as `[` newline, blank line, `]`.
//!
//! The Codable models in this crate are serde types; [`decode`] reads them
//! through this parser with `JSONDecoder`'s rules, and [`encode`] writes them
//! as `JSONEncoder` does.

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::double;

/// A JSON value with its numbers kept as written.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Value>),
    Object(Object),
}

/// A number literal as it appeared. Under [`Dialect::Decoder`] it may be a
/// token Foundation only checks once something decodes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Number(String);

/// An object's members in the order first seen, keys unique.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Object(Vec<(String, Value)>);

impl Object {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Set `key`, in place when it is already there (Swift's
    /// `dictionary[key] = value`).
    pub fn insert(&mut self, key: impl Into<String>, value: Value) {
        let key = key.into();
        match self.get_mut(&key) {
            Some(slot) => *slot = value,
            None => self.0.push((key, value)),
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let index = self.0.iter().position(|(k, _)| k == key)?;
        Some(self.0.remove(index).1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| k.as_str())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<(String, Value)> for Object {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        let mut object = Self::new();
        for (key, value) in iter {
            object.insert(key, value);
        }
        object
    }
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Self::Object(object) => Some(object),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// `value as? Int` on what `JSONSerialization` returned: an integer, a
    /// floating number with an integral value, or a Boolean (an `NSNumber`
    /// too: `true` is 1), that fits `Int`.
    pub fn as_swift_int(&self) -> Option<i64> {
        match self {
            Self::Bool(flag) => Some(i64::from(*flag)),
            Self::Number(number) => match number.classify().ok()? {
                Classified::Int(value) => Some(value),
                Classified::UInt(value) => i64::try_from(value).ok(),
                Classified::Double(value) => exact_i64(value),
                Classified::Decimal(_) => None,
            },
            _ => None,
        }
    }

    /// `value as? Double` on what `JSONSerialization` returned: any number
    /// whose value a `Double` holds exactly, or a Boolean.
    pub fn as_swift_double(&self) -> Option<f64> {
        match self {
            Self::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
            Self::Number(number) => match number.classify().ok()? {
                Classified::Int(value) => {
                    let double = value as f64;
                    (double as i64 == value && double != 9_223_372_036_854_775_808.0)
                        .then_some(double)
                }
                Classified::UInt(value) => {
                    let double = value as f64;
                    (double < 18_446_744_073_709_551_616.0 && double as u64 == value)
                        .then_some(double)
                }
                Classified::Double(value) => Some(value),
                Classified::Decimal(_) => None,
            },
            _ => None,
        }
    }
}

fn exact_i64(value: f64) -> Option<i64> {
    (value.fract() == 0.0
        && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value))
    .then_some(value as i64)
}

/// What a number literal is once Foundation has read it.
#[derive(Debug, Clone, PartialEq)]
pub enum Classified {
    Int(i64),
    UInt(u64),
    Double(f64),
    /// An `NSDecimalNumber`, as it prints.
    Decimal(String),
}

/// Why a literal is no number Foundation keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberError {
    /// Not the JSON number grammar.
    Syntax,
    /// A `Double` past its range.
    Overflow,
}

impl Number {
    /// A literal as written. Checked only by [`Number::classify`].
    pub fn from_literal(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn literal(&self) -> &str {
        &self.0
    }

    /// The literal's type and value under `JSONSerialization`'s rules.
    ///
    /// Fix (negative-overflow): `JSONSerialization` refuses `1e400` but reads
    /// `-1e400` as negative infinity, and writing that back raises an
    /// Objective-C exception — `AgentHookConfig.mergedJSON` over an agent
    /// config holding `-1e400` crashes the app (`AgentHookConfig.swift:132`,
    /// then `:188`). Both signs are [`NumberError::Overflow`] here, so such a
    /// config is reported unreadable like any other.
    pub fn classify(&self) -> Result<Classified, NumberError> {
        let parts = Parts::of(&self.0).ok_or(NumberError::Syntax)?;
        if !parts.has_fraction_or_exponent {
            if let Ok(value) = self.0.parse::<i64>() {
                return Ok(Classified::Int(value));
            }
            if !parts.negative
                && let Ok(value) = self.0.parse::<u64>()
            {
                return Ok(Classified::UInt(value));
            }
            return Ok(Classified::Decimal(parts.decimal()));
        }
        if parts.significant_digits() > 17 {
            return Ok(Classified::Decimal(parts.decimal()));
        }
        let value: f64 = self.0.parse().map_err(|_| NumberError::Syntax)?;
        if value.is_infinite() {
            return Err(NumberError::Overflow);
        }
        Ok(Classified::Double(value))
    }

    /// The literal as a Swift `Int` field decodes it (`JSONDecoder`), as
    /// measured on macOS 27: a plain integer exactly; anything with a
    /// fraction or exponent only when it reads as an integral `Double` inside
    /// `Int`'s range — then that `Double` below 2^53, and above it the
    /// literal's own digits, truncated, so long as they fit 64 bits
    /// (`1.000000000000000000001` is 1, `9007199254740993.5` is
    /// 9007199254740993, `4611686018427387903.5` and `1.5` are no `Int`).
    pub fn decoder_int(&self) -> Option<i64> {
        let parts = Parts::of(&self.0)?;
        if !parts.has_fraction_or_exponent {
            return self.0.parse().ok();
        }
        let double: f64 = self.0.parse().ok()?;
        if !double.is_finite()
            || double.fract() != 0.0
            || !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&double)
        {
            return None;
        }
        if double.abs() < 9_007_199_254_740_992.0 {
            return Some(double as i64);
        }
        parts.truncated_i64()
    }

    /// The literal as a Swift `Double` field decodes it: any finite value.
    pub fn decoder_double(&self) -> Option<f64> {
        Parts::of(&self.0)?;
        self.0.parse::<f64>().ok().filter(|value| value.is_finite())
    }
}

/// A literal split per the JSON number grammar.
struct Parts<'a> {
    negative: bool,
    integer: &'a str,
    fraction: &'a str,
    exponent: i64,
    has_fraction_or_exponent: bool,
}

impl<'a> Parts<'a> {
    /// `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`, or `None`.
    fn of(text: &'a str) -> Option<Self> {
        let bytes = text.as_bytes();
        let mut index = 0;
        let negative = bytes.first() == Some(&b'-');
        if negative {
            index += 1;
        }
        let digits = |from: usize| {
            bytes[from..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count()
        };
        let integer_len = digits(index);
        if integer_len == 0 || (integer_len > 1 && bytes[index] == b'0') {
            return None;
        }
        let integer = &text[index..index + integer_len];
        index += integer_len;
        let mut fraction = "";
        let mut has_fraction_or_exponent = false;
        if bytes.get(index) == Some(&b'.') {
            let len = digits(index + 1);
            if len == 0 {
                return None;
            }
            fraction = &text[index + 1..index + 1 + len];
            index += 1 + len;
            has_fraction_or_exponent = true;
        }
        let mut exponent = 0i64;
        if matches!(bytes.get(index), Some(b'e' | b'E')) {
            index += 1;
            let minus = match bytes.get(index) {
                Some(b'-') => {
                    index += 1;
                    true
                }
                Some(b'+') => {
                    index += 1;
                    false
                }
                _ => false,
            };
            let len = digits(index);
            if len == 0 {
                return None;
            }
            // Saturating: an exponent this large only ever means "huge" or
            // "tiny", which f64 parsing decides on its own.
            let magnitude = text[index..index + len]
                .parse::<i64>()
                .unwrap_or(i64::MAX / 4)
                .min(i64::MAX / 4);
            exponent = if minus { -magnitude } else { magnitude };
            index += len;
            has_fraction_or_exponent = true;
        }
        (index == bytes.len()).then_some(Self {
            negative,
            integer,
            fraction,
            exponent,
            has_fraction_or_exponent,
        })
    }

    /// The value truncated toward zero, computed from the digits — trailing
    /// zeros dropped, the rest required to fit 64 bits — when it fits `Int`.
    fn truncated_i64(&self) -> Option<i64> {
        let all = format!("{}{}", self.integer, self.fraction);
        let digits = all.trim_start_matches('0');
        let trimmed = digits.trim_end_matches('0');
        let mut exponent =
            self.last_digit_exponent() + i64::try_from(digits.len() - trimmed.len()).ok()?;
        let mut magnitude: u64 = if trimmed.is_empty() {
            0
        } else {
            trimmed.parse().ok()?
        };
        while exponent > 0 {
            magnitude = magnitude.checked_mul(10)?;
            exponent -= 1;
        }
        while exponent < 0 && magnitude > 0 {
            magnitude /= 10;
            exponent += 1;
        }
        let magnitude = i64::try_from(magnitude).ok()?;
        Some(if self.negative { -magnitude } else { magnitude })
    }

    /// How many digits decide between `Double` and `NSDecimalNumber`: the
    /// integer part's without its leading zeros, and every digit of the
    /// fraction, leading zeros included (`0.000000000000010852756446340499`
    /// has 30, and is a decimal).
    fn significant_digits(&self) -> usize {
        self.integer.trim_start_matches('0').len() + self.fraction.len()
    }

    /// The decimal exponent of the last mantissa digit.
    fn last_digit_exponent(&self) -> i64 {
        self.exponent - i64::try_from(self.fraction.len()).expect("short")
    }

    /// The value as an `NSDecimalNumber` prints it: digits accumulated into
    /// its 128-bit mantissa while they fit (later ones dropped), then written
    /// out positionally, trailing fractional zeros dropped.
    fn decimal(&self) -> String {
        let mut mantissa: u128 = 0;
        let mut exponent = self.last_digit_exponent();
        let all = format!("{}{}", self.integer, self.fraction);
        let mut full = false;
        for digit in all.bytes().map(|b| u128::from(b - b'0')) {
            if !full {
                if let Some(next) = mantissa.checked_mul(10).and_then(|m| m.checked_add(digit)) {
                    mantissa = next;
                    continue;
                }
                full = true;
            }
            exponent += 1;
        }
        let mut digits = mantissa.to_string();
        if mantissa == 0 {
            return "0".to_owned();
        }
        while exponent < 0 && digits.ends_with('0') {
            digits.pop();
            exponent += 1;
        }
        let sign = if self.negative { "-" } else { "" };
        if exponent >= 0 {
            let zeros = "0".repeat(usize::try_from(exponent).expect("non-negative"));
            return format!("{sign}{digits}{zeros}");
        }
        let point = usize::try_from(-exponent).expect("positive");
        if point >= digits.len() {
            format!("{sign}0.{}{digits}", "0".repeat(point - digits.len()))
        } else {
            let split = digits.len() - point;
            format!("{sign}{}.{}", &digits[..split], &digits[split..])
        }
    }
}

// MARK: - Parsing

/// Which of Foundation's two parsers to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// `JSONDecoder`.
    Decoder,
    /// `JSONSerialization.jsonObject(with:options:)`.
    Serialization { fragments_allowed: bool },
}

/// Why Foundation would not read the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub reason: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.reason, self.offset)
    }
}

impl std::error::Error for ParseError {}

/// How deep arrays and objects may nest.
const MAX_DEPTH: usize = 512;

/// Parse `bytes` the way `dialect` does.
pub fn parse(bytes: &[u8], dialect: Dialect) -> Result<Value, ParseError> {
    let mut parser = Parser {
        bytes,
        at: 0,
        dialect,
        depth: 0,
    };
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        parser.at = 3;
    }
    parser.skip_space();
    if let Dialect::Serialization {
        fragments_allowed: false,
    } = dialect
        && !matches!(parser.peek(), Some(b'[' | b'{'))
    {
        return Err(parser.error("top level is not an array or object"));
    }
    let value = parser.value()?;
    parser.skip_space();
    if parser.at != bytes.len() {
        return Err(parser.error("garbage after the value"));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    dialect: Dialect,
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, reason: &'static str) -> ParseError {
        ParseError {
            offset: self.at,
            reason,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn literal(&mut self, word: &[u8], value: Value) -> Result<Value, ParseError> {
        if self.bytes[self.at..].starts_with(word) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error("unexpected token"))
        }
    }

    fn value(&mut self) -> Result<Value, ParseError> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character")),
            None => Err(self.error("unexpected end")),
        }
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.at;
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at]).expect("ASCII");
        let number = Number::from_literal(text);
        if let Dialect::Serialization { .. } = self.dialect
            && let Err(error) = number.classify()
        {
            self.at = start;
            return Err(self.error(match error {
                NumberError::Syntax => "malformed number",
                NumberError::Overflow => "number out of range",
            }));
        }
        Ok(Value::Number(number))
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("nested too deep"));
        }
        Ok(())
    }

    fn array(&mut self) -> Result<Value, ParseError> {
        self.enter()?;
        self.at += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek() == Some(b']') {
            self.at += 1;
            self.depth -= 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_space();
            match self.peek() {
                Some(b',') => {
                    self.at += 1;
                    self.skip_space();
                    // One trailing comma is fine with Foundation.
                    if self.peek() == Some(b']') {
                        self.at += 1;
                        break;
                    }
                }
                Some(b']') => {
                    self.at += 1;
                    break;
                }
                _ => return Err(self.error("expected , or ]")),
            }
        }
        self.depth -= 1;
        Ok(Value::Array(items))
    }

    fn object(&mut self) -> Result<Value, ParseError> {
        self.enter()?;
        self.at += 1;
        let mut object = Object::new();
        self.skip_space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            self.depth -= 1;
            return Ok(Value::Object(object));
        }
        loop {
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a key"));
            }
            let key = self.string()?;
            self.skip_space();
            if self.peek() != Some(b':') {
                return Err(self.error("expected :"));
            }
            self.at += 1;
            self.skip_space();
            let value = self.value()?;
            // The first of two equal keys wins.
            if !object.contains_key(&key) {
                object.0.push((key, value));
            }
            self.skip_space();
            match self.peek() {
                Some(b',') => {
                    self.at += 1;
                    self.skip_space();
                    if self.peek() == Some(b'}') {
                        self.at += 1;
                        break;
                    }
                }
                Some(b'}') => {
                    self.at += 1;
                    break;
                }
                _ => return Err(self.error("expected , or }")),
            }
        }
        self.depth -= 1;
        Ok(Value::Object(object))
    }

    fn hex4(&mut self) -> Result<u16, ParseError> {
        let digits = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or_else(|| self.error("short \\u escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.error("bad \\u escape"))?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(self.error("bad \\u escape"));
        }
        self.at += 4;
        Ok(u16::from_str_radix(text, 16).expect("hex"))
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.at += 1;
        let mut out = Vec::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error("unterminated string"));
            };
            match byte {
                b'"' => {
                    self.at += 1;
                    break;
                }
                b'\\' => {
                    self.at += 1;
                    let escape = self
                        .peek()
                        .ok_or_else(|| self.error("unterminated escape"))?;
                    self.at += 1;
                    let c = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let unit = self.hex4()?;
                            match unit {
                                0xD800..=0xDBFF => {
                                    if self.bytes.get(self.at..self.at + 2) != Some(b"\\u") {
                                        return Err(self.error("lone high surrogate"));
                                    }
                                    self.at += 2;
                                    let low = self.hex4()?;
                                    if !(0xDC00..=0xDFFF).contains(&low) {
                                        return Err(self.error("bad low surrogate"));
                                    }
                                    let scalar = 0x10000
                                        + ((u32::from(unit) - 0xD800) << 10)
                                        + (u32::from(low) - 0xDC00);
                                    char::from_u32(scalar).expect("a valid pair")
                                }
                                0xDC00..=0xDFFF => return Err(self.error("lone low surrogate")),
                                _ => char::from_u32(u32::from(unit)).expect("not a surrogate"),
                            }
                        }
                        _ => return Err(self.error("unknown escape")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                0x00..=0x1F => return Err(self.error("control character in a string")),
                _ => {
                    out.push(byte);
                    self.at += 1;
                }
            }
        }
        let text = String::from_utf8(out).map_err(|_| self.error("invalid UTF-8"))?;
        // JSONSerialization's strings (keys too) lose a leading U+FEFF, which
        // NSString takes for a byte order mark; JSONDecoder keeps it.
        if let Dialect::Serialization { .. } = self.dialect
            && let Some(rest) = text.strip_prefix('\u{FEFF}')
        {
            return Ok(rest.to_owned());
        }
        Ok(text)
    }
}

// MARK: - Codable

/// Why a Codable model could not be read.
#[derive(Debug)]
pub enum DecodeError {
    /// Not JSON as `JSONDecoder` reads it.
    Syntax(ParseError),
    /// JSON, but not this type (a missing key, a wrong type, a number the
    /// field's Swift type does not take).
    Shape(super::decode::Error),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(error) => write!(f, "not JSON: {error}"),
            Self::Shape(error) => write!(f, "not the expected shape: {error}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A serde tree as Foundation's writers print it (numbers in `JSONEncoder`'s
/// form).
pub fn from_serde(value: &serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(flag) => Value::Bool(*flag),
        serde_json::Value::String(text) => Value::String(text.clone()),
        serde_json::Value::Array(items) => Value::Array(items.iter().map(from_serde).collect()),
        serde_json::Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), from_serde(v)))
                .collect(),
        ),
        serde_json::Value::Number(number) => {
            let text = if let Some(int) = number.as_i64() {
                int.to_string()
            } else if let Some(int) = number.as_u64() {
                int.to_string()
            } else {
                double::encoder(number.as_f64().expect("a finite number"))
            };
            Value::Number(Number::from_literal(text))
        }
    }
}

/// `JSONDecoder().decode(T.self, from: bytes)`.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, DecodeError> {
    let value = parse(bytes, Dialect::Decoder).map_err(DecodeError::Syntax)?;
    decode_value(&value)
}

/// Decode an already parsed value as `JSONDecoder` would have.
pub fn decode_value<T: DeserializeOwned>(value: &Value) -> Result<T, DecodeError> {
    T::deserialize(super::decode::ValueDeserializer(value)).map_err(DecodeError::Shape)
}

/// `JSONEncoder().encode(value)`: compact; members in code point order, where
/// Swift's is a per-process hash order.
pub fn encode<T: Serialize>(value: &T) -> String {
    write_encoder(&to_value(value), Style::COMPACT)
}

/// `value` as a tree, numbers in `JSONEncoder`'s form.
pub fn to_value<T: Serialize>(value: &T) -> Value {
    from_serde(&serde_json::to_value(value).expect("models serialize"))
}

// MARK: - Writing

/// How a writer lays out its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub pretty: bool,
    pub sorted_keys: bool,
    /// Write `/` as `\/` (the default of both writers).
    pub escape_slashes: bool,
}

impl Style {
    pub const COMPACT: Self = Self {
        pretty: false,
        sorted_keys: false,
        escape_slashes: true,
    };
    /// `[.prettyPrinted, .sortedKeys]`.
    pub const PRETTY_SORTED: Self = Self {
        pretty: true,
        sorted_keys: true,
        escape_slashes: true,
    };
}

/// `JSONEncoder`'s output for `value` (numbers are printed as held).
pub fn write_encoder(value: &Value, style: Style) -> String {
    let mut out = String::new();
    Writer {
        style,
        serialization: false,
    }
    .value(value, 0, &mut out);
    out
}

/// `JSONSerialization.data(withJSONObject:options:)` for `value`.
///
/// # Panics
///
/// On a number [`Number::classify`] refuses, which [`parse`] in a
/// `Serialization` dialect never returns.
pub fn write_serialization(value: &Value, style: Style) -> String {
    let mut out = String::new();
    Writer {
        style,
        serialization: true,
    }
    .value(value, 0, &mut out);
    out
}

struct Writer {
    style: Style,
    serialization: bool,
}

impl Writer {
    fn indent(&self, level: usize, out: &mut String) {
        for _ in 0..level {
            out.push_str("  ");
        }
    }

    fn value(&self, value: &Value, level: usize, out: &mut String) {
        match value {
            Value::Null => out.push_str("null"),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            Value::Number(number) => out.push_str(&self.number(number)),
            Value::String(text) => self.string(text, out),
            Value::Array(items) => {
                let items: Vec<&Value> = items.iter().collect();
                self.container('[', ']', &items, level, out, |writer, item, out| {
                    writer.value(item, level + 1, out);
                });
            }
            Value::Object(object) => {
                let mut members: Vec<(&str, &Value)> = object.iter().collect();
                if self.style.sorted_keys {
                    if self.serialization {
                        members.sort_by(|a, b| super::collation::compare(a.0, b.0));
                    } else {
                        members.sort_by(|a, b| a.0.cmp(b.0));
                    }
                }
                let separator = if self.style.pretty { " : " } else { ":" };
                self.container(
                    '{',
                    '}',
                    &members,
                    level,
                    out,
                    |writer, (key, member), out| {
                        writer.string(key, out);
                        out.push_str(separator);
                        writer.value(member, level + 1, out);
                    },
                );
            }
        }
    }

    fn container<T: Copy>(
        &self,
        open: char,
        close: char,
        items: &[T],
        level: usize,
        out: &mut String,
        write: impl Fn(&Self, T, &mut String),
    ) {
        out.push(open);
        if self.style.pretty {
            out.push('\n');
            if items.is_empty() {
                out.push('\n');
            }
            for (index, &item) in items.iter().enumerate() {
                self.indent(level + 1, out);
                write(self, item, out);
                if index + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            self.indent(level, out);
        } else {
            for (index, &item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write(self, item, out);
            }
        }
        out.push(close);
    }

    fn number(&self, number: &Number) -> String {
        if !self.serialization {
            return number.literal().to_owned();
        }
        match number
            .classify()
            .expect("only numbers JSONSerialization reads are written")
        {
            Classified::Int(value) => value.to_string(),
            Classified::UInt(value) => value.to_string(),
            Classified::Double(value) => double::printf_17g(value),
            Classified::Decimal(text) => text,
        }
    }

    fn string(&self, text: &str, out: &mut String) {
        out.push('"');
        for c in text.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '/' if self.style.escape_slashes => out.push_str("\\/"),
                '\u{8}' => out.push_str("\\b"),
                '\u{c}' => out.push_str("\\f"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{0}'..='\u{1f}' => out.push_str(&format!("\\u{:04x}", u32::from(c))),
                _ => out.push(c),
            }
        }
        out.push('"');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serialization(text: &str) -> Result<String, ParseError> {
        let value = parse(
            text.as_bytes(),
            Dialect::Serialization {
                fragments_allowed: true,
            },
        )?;
        Ok(write_serialization(
            &value,
            Style {
                pretty: false,
                sorted_keys: true,
                escape_slashes: false,
            },
        ))
    }

    /// Pairs `JSONSerialization` produced on macOS 27 (`{"n":<in>}` read,
    /// then written with `[.sortedKeys, .withoutEscapingSlashes]`).
    #[test]
    fn numbers_as_jsonserialization_writes_them() {
        for (literal, apple) in [
            ("0", "0"),
            ("-0", "0"),
            ("1.0", "1"),
            ("1e3", "1000"),
            ("0.1", "0.10000000000000001"),
            ("0.7", "0.69999999999999996"),
            ("12345678901234567890", "12345678901234567890"),
            (
                "123456789012345678901234567890",
                "123456789012345678901234567890",
            ),
            ("-9223372036854775808", "-9223372036854775808"),
            ("18446744073709551616", "18446744073709551616"),
            ("3.14159265358979323846", "3.14159265358979323846"),
            ("0.1e1", "1"),
            ("1.10", "1.1000000000000001"),
            ("1e16", "10000000000000000"),
            ("1e17", "1e+17"),
            ("123456789.123456789", "123456789.123456789"),
            ("-0.0", "-0"),
            ("9007199254740993", "9007199254740993"),
            ("1e-400", "0"),
            ("1234567890123456.7", "1234567890123456.8"),
            ("12345678901234567.8", "12345678901234567.8"),
            ("0.12345678901234567", "0.12345678901234566"),
            ("0.123456789012345678", "0.123456789012345678"),
            ("1.23456789012345678e5", "123456.789012345678"),
            ("1.10000000000000000000", "1.1"),
            ("100000000000000000000.0", "100000000000000000000"),
            ("1.0000000000000000000000000000000000000000001", "1"),
            (
                "123456789012345678901234567890123456789",
                "123456789012345678901234567890123456789",
            ),
            (
                "1234567890123456789012345678901234567890",
                "1234567890123456789012345678901234567890",
            ),
            (
                "12345678901234567890123456789012345678901234567890",
                "12345678901234567890123456789012345678900000000000",
            ),
            ("-12345678901234567890", "-12345678901234567890"),
            ("-9223372036854775809", "-9223372036854775809"),
            (
                "0.000000000000000000001234567890123456789",
                "0.000000000000000000001234567890123456789",
            ),
            ("9.999999999999999999e22", "99999999999999999990000"),
            ("123e5", "12300000"),
            ("-0e0", "-0"),
            ("123456789e10", "1.23456789e+18"),
        ] {
            assert_eq!(
                serialization(&format!("{{\"n\":{literal}}}")).unwrap(),
                format!("{{\"n\":{apple}}}"),
                "{literal}"
            );
        }
    }

    #[test]
    fn numbers_jsonserialization_refuses() {
        for literal in [
            "1e400",
            "1e309",
            "1.7976931348623159e308",
            "00",
            "-",
            "1.",
            "1.e5",
            ".1",
            "1e",
            "+1",
            "01",
        ] {
            assert!(
                serialization(&format!("{{\"n\":{literal}}}")).is_err(),
                "{literal}"
            );
        }
        // Swift crashes writing this one back; the port refuses to read it.
        assert!(serialization("{\"n\":-1e400}").is_err());
        assert!(serialization("{\"n\":-1e309}").is_err());
    }

    #[test]
    fn foundations_leniencies() {
        assert_eq!(serialization("{\"a\":1,\"a\":2}").unwrap(), "{\"a\":1}");
        assert_eq!(serialization("[1,]").unwrap(), "[1]");
        assert_eq!(serialization("{\"a\":[1,2,],}").unwrap(), "{\"a\":[1,2]}");
        assert_eq!(serialization("\u{FEFF}{\"a\":1}").unwrap(), "{\"a\":1}");
        assert_eq!(serialization("  {\"a\":1}  ").unwrap(), "{\"a\":1}");
        for bad in [
            "[1,,]",
            "[,]",
            "{,}",
            "{\"a\":1} x",
            "/*c*/{}",
            "{'a':1}",
            "[\u{A0}1]",
            "[1\u{B}]",
            "{\"a\":\"\u{1}\"}",
            "{\"a\":\"\\x\"}",
            "{\"a\":\"\\ud800\"}",
            "{\"a\":\"\\udc00x\"}",
            "{\"a\":NaN}",
        ] {
            assert!(serialization(bad).is_err(), "{bad:?}");
        }
        let fragments = |text: &str| {
            parse(
                text.as_bytes(),
                Dialect::Serialization {
                    fragments_allowed: false,
                },
            )
        };
        assert!(fragments("1").is_err());
        assert!(fragments("\"x\"").is_err());
        assert!(fragments("[1]").is_ok());
    }

    #[test]
    fn strings_escape_as_foundation_does() {
        let text: String = (0u8..0x80)
            .map(char::from)
            .chain("\u{2028}\u{2029}\u{FEFF}é".chars())
            .collect();
        let value = Value::Array(vec![Value::String(text)]);
        let encoder = write_encoder(&value, Style::COMPACT);
        assert_eq!(
            encoder,
            "[\"\\u0000\\u0001\\u0002\\u0003\\u0004\\u0005\\u0006\\u0007\\b\\t\\n\\u000b\\f\\r\\u000e\\u000f\\u0010\\u0011\\u0012\\u0013\\u0014\\u0015\\u0016\\u0017\\u0018\\u0019\\u001a\\u001b\\u001c\\u001d\\u001e\\u001f !\\\"#$%&'()*+,-.\\/0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\u{7f}\u{2028}\u{2029}\u{FEFF}é\"]"
        );
        let unescaped = write_serialization(
            &value,
            Style {
                pretty: false,
                sorted_keys: false,
                escape_slashes: false,
            },
        );
        assert!(unescaped.contains("-./0"));
    }

    #[test]
    fn pretty_layout_is_foundations() {
        let value = parse(
            br#"{"b":1,"a":[1,2,{"z":null,"y":true}],"c":{}, "d":[], "e":"x/y"}"#,
            Dialect::Decoder,
        )
        .unwrap();
        assert_eq!(
            write_serialization(
                &value,
                Style {
                    pretty: true,
                    sorted_keys: true,
                    escape_slashes: false
                }
            ),
            "{\n  \"a\" : [\n    1,\n    2,\n    {\n      \"y\" : true,\n      \"z\" : null\n    }\n  ],\n  \"b\" : 1,\n  \"c\" : {\n\n  },\n  \"d\" : [\n\n  ],\n  \"e\" : \"x/y\"\n}"
        );
        assert_eq!(
            write_serialization(
                &Value::Array(vec![]),
                Style {
                    pretty: true,
                    sorted_keys: true,
                    escape_slashes: false
                }
            ),
            "[\n\n]"
        );
    }

    #[derive(serde::Deserialize, Debug)]
    struct T {
        ts: i64,
        #[serde(default)]
        x: Option<i64>,
    }

    /// What `JSONDecoder` decoded (or refused) on macOS 27.
    #[test]
    fn decoder_rules() {
        let ts = |literal: &str| {
            decode::<T>(format!("{{\"ts\":{literal}}}").as_bytes())
                .ok()
                .map(|t| t.ts)
        };
        assert_eq!(ts("1"), Some(1));
        assert_eq!(ts("1.0"), Some(1));
        assert_eq!(ts("1.5"), None);
        assert_eq!(ts("1e3"), Some(1000));
        assert_eq!(ts("1E3"), Some(1000));
        assert_eq!(ts("-0"), Some(0));
        assert_eq!(ts("-0.0"), Some(0));
        assert_eq!(ts("9223372036854775807"), Some(i64::MAX));
        assert_eq!(ts("9223372036854775808"), None);
        assert_eq!(ts("-9223372036854775808"), Some(i64::MIN));
        assert_eq!(ts("1e19"), None);
        assert_eq!(ts("007"), None);
        assert_eq!(ts("\"1\""), None);
        assert_eq!(ts("true"), None);
        assert_eq!(ts("1e400"), None);
        assert_eq!(ts("1.000000000000000000001"), Some(1));
        assert_eq!(ts("12345678901234567.0"), Some(12_345_678_901_234_567));
        let ok = |text: &str| decode::<T>(text.as_bytes()).is_ok();
        // An ignored member's number is never checked…
        for lazy in ["01", "1.", "-", "1e", "1e+", "--1", "1.2.3", "1e5e5"] {
            assert!(ok(&format!("{{\"ts\":1,\"u\":{lazy}}}")), "{lazy}");
        }
        // …but it still has to look like one.
        for bad in [
            ".5", "+1", "0x10", "Infinity", "NaN", "1_000", "[,]", "{,}", "tru", "[1 2]",
            "\u{A0}1", "[]]",
        ] {
            assert!(!ok(&format!("{{\"ts\":1,\"u\":{bad}}}")), "{bad}");
        }
        assert!(ok("{\"ts\":1,\"u\":[1,],\"v\":{\"a\":1,},}"));
        assert!(!ok("{\"ts\":1}  x"));
        assert!(!ok("{\"ts\":1}\u{0}"));
        assert!(ok(" \t\n\r{\"ts\":1}"));
        assert!(ok("\u{FEFF}{\"ts\":1}"));
        assert!(!ok("[{\"ts\":1}]"));
        assert_eq!(decode::<T>(b"{\"ts\":1,\"ts\":2}").unwrap().ts, 1);
        assert_eq!(decode::<T>(b"{\"ts\":1,\"x\":2.0}").unwrap().x, Some(2));
        assert_eq!(decode::<T>(b"{\"ts\":1,\"x\":null}").unwrap().x, None);
        assert!(decode::<T>(b"{\"ts\":1,\"x\":1.2.3}").is_err());
    }
}
