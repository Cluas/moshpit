//! Foundation's `UUID`: what `UUID(uuidString:)` accepts and what
//! `uuidString` (and `Codable`) writes.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A UUID. Displays (and encodes) as Swift's `uuidString`: 36 characters,
/// upper case — the form a connection id has in every file and every
/// envelope the app writes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uuid([u8; 16]);

impl Uuid {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// A random (version 4) UUID, as `UUID()` makes.
    pub fn new_v4() -> Self {
        let mut bytes = [0u8; 16];
        crate::random::fill(&mut bytes);
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Self(bytes)
    }

    /// `UUID(uuidString:)`: exactly `8-4-4-4-12` hex digits of either case.
    /// No braces, no surrounding space, no other length.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.as_bytes();
        if text.len() != 36 {
            return None;
        }
        let mut bytes = [0u8; 16];
        let mut nibbles = 0;
        for (index, &byte) in text.iter().enumerate() {
            if matches!(index, 8 | 13 | 18 | 23) {
                if byte != b'-' {
                    return None;
                }
                continue;
            }
            let value = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return None,
            };
            bytes[nibbles / 2] |= if nibbles % 2 == 0 { value << 4 } else { value };
            nibbles += 1;
        }
        Some(Self(bytes))
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
        let mut out = String::with_capacity(36);
        for (index, byte) in self.0.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                out.push('-');
            }
            out.push(char::from(DIGITS[usize::from(byte >> 4)]));
            out.push(char::from(DIGITS[usize::from(byte & 0xF)]));
        }
        f.write_str(&out)
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::str::FromStr for Uuid {
    type Err = InvalidUuid;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text).ok_or(InvalidUuid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidUuid;

impl fmt::Display for InvalidUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a UUID string")
    }
}

impl std::error::Error for InvalidUuid {}

impl Serialize for Uuid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Uuid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        // Swift's error for a malformed UUID string is a decoding error
        // ("Attempted to decode UUID from invalid UUID string.").
        Self::parse(&text).ok_or_else(|| {
            serde::de::Error::custom("Attempted to decode UUID from invalid UUID string.")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_foundation_parses_and_prints_upper_case() {
        let id = Uuid::parse("3f2504e0-4f89-11d3-9a0c-0305e82c3301").unwrap();
        assert_eq!(id.to_string(), "3F2504E0-4F89-11D3-9A0C-0305E82C3301");
        assert_eq!(
            Uuid::parse("3F2504E0-4F89-11D3-9A0C-0305E82C3301"),
            Some(id)
        );
    }

    #[test]
    fn refuses_what_foundation_refuses() {
        for text in [
            "{3F2504E0-4F89-11D3-9A0C-0305E82C3301}",
            "3F2504E04F8911D39A0C0305E82C3301",
            " 3F2504E0-4F89-11D3-9A0C-0305E82C3301",
            "3F2504E0-4F89-11D3-9A0C-0305E82C330",
            "3F2504E0-4F89-11D3-9A0C-0305E82C33011",
            "3F2504E-04F89-11D3-9A0C-0305E82C3301",
            "3F2504E0-4F89-11D3-9A0C-0305E82C33G1",
            "+F2504E0-4F89-11D3-9A0C-0305E82C3301",
        ] {
            assert_eq!(Uuid::parse(text), None, "{text}");
        }
    }

    #[test]
    fn random_ones_are_version_4_and_distinct() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        assert_ne!(a, b);
        assert_eq!(a.as_bytes()[6] >> 4, 4);
        assert_eq!(a.as_bytes()[8] >> 6, 0b10);
        assert_eq!(Uuid::parse(&a.to_string()), Some(a));
    }
}
