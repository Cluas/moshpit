//! Standard base64 (RFC 4648 §4, padded), as `openssl base64 -A`, Go's
//! `base64.StdEncoding` and Foundation's `base64EncodedString()` write it.

use base64::Engine as _;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// Padded output; decoding demands canonical padding and, like Go's
/// `StdEncoding` and Foundation, ignores bits after the last whole byte.
const ENGINE: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_encode_padding(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical)
        .with_decode_allow_trailing_bits(true),
);

pub fn encode(bytes: &[u8]) -> String {
    ENGINE.encode(bytes)
}

/// `None` for anything but the standard alphabet in padded quads.
///
/// Fix (base64-padding): Foundation's `Data(base64Encoded:)` on macOS 27
/// also takes `=` in odd places: `"AAAA="` decodes to three zero bytes,
/// `"AA=Q"` to `00 10` (the third character's byte is dropped and the
/// fourth's kept) and `"===="` to one zero byte. Go and openssl refuse all
/// three, and all three senders only ever write canonical base64, so the
/// port refuses them too. Over an envelope this only changes which error a
/// garbage field gets — except that Swift still accepts a `mac` with extra
/// `=` appended to the canonical one; a key holder's own tag, re-encoded,
/// so nothing a forger can use.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    ENGINE.decode(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_what_foundation_and_go_both_take() {
        assert_eq!(decode(""), Some(vec![]));
        assert_eq!(decode("QQ=="), Some(vec![0x41]));
        // Bits past the last whole byte are ignored, as Foundation and Go's
        // StdEncoding do.
        assert_eq!(decode("QR=="), Some(vec![0x41]));
        assert_eq!(decode("QUJ="), Some(vec![0x41, 0x42]));
        assert_eq!(decode("+/8="), Some(vec![0xfb, 0xff]));
    }

    #[test]
    fn refuses_what_foundation_and_go_both_refuse() {
        for text in [
            "QQ", "QQ=", "Q Q==", "QQ==\n", "QUJ", "Q===", "QQ==QQ==", "-_8=",
        ] {
            assert_eq!(decode(text), None, "{text:?}");
        }
    }

    #[test]
    fn refuses_foundations_odd_padding() {
        for text in ["AAAA=", "AA=Q", "====", "AAAA==", "AA=A="] {
            assert_eq!(decode(text), None, "{text:?}");
        }
    }

    #[test]
    fn encodes_padded() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"A"), "QQ==");
        assert_eq!(encode(b"AB"), "QUI=");
        assert_eq!(encode(b"ABC"), "QUJD");
    }
}
