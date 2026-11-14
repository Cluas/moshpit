//! Hex, as the envelope and the pairing secret use it.

/// Lowercase hex, `Data.moshpitHexString`.
pub fn encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0xF)]));
    }
    out
}

/// Strict hex: ASCII digit pairs of either case, nothing else. `None` on odd
/// length or any other byte, so a malformed field can never be silently
/// truncated into a valid-looking key.
///
/// Fix (hex-sign): Swift's `Data(moshpitHex:)` parses each pair with
/// `UInt8(_:radix: 16)`, which takes a sign, so `"+f"` and `"-0"` are
/// accepted as bytes 0x0f and 0x00 (`PushSealedBox.swift:225-236`). Go's
/// `hex.DecodeString` and the shell's `openssl rand -hex` know no signs;
/// nothing the app, the host or the relay writes ever has one.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| Some((digit(pair[0])? << 4) | digit(pair[1])?))
        .collect()
}

fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PushSealedBoxTests.hexStrictness`.
    #[test]
    fn hex_decoding_refuses_anything_but_clean_pairs() {
        assert_eq!(decode("00ff"), Some(vec![0x00, 0xff]));
        assert_eq!(decode("0f0"), None, "odd length");
        assert_eq!(decode("00zz"), None, "non-hex");
        assert_eq!(decode("00 ff"), None, "whitespace");
        assert_eq!(encode(&[0x00, 0xff]), "00ff");
        assert_eq!(decode("ABcd"), Some(vec![0xab, 0xcd]));
    }

    #[test]
    fn signs_are_not_digits() {
        assert_eq!(decode("+f"), None);
        assert_eq!(decode("-0"), None);
        assert_eq!(decode("٣٣"), None);
    }
}
