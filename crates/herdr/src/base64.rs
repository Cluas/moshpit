//! Base64 as the app speaks it: what `Data.base64EncodedString()` writes and
//! what `Data(base64Encoded:)` accepts, measured on macOS 27 (every string of
//! up to eight characters over `A`, `Q`, `/` and `=`, 87 381 of them).
//!
//! Foundation's decoder takes the standard alphabet only, no whitespace,
//! padding where the last group needs it and any number of extra `=` after
//! it, and ignores the unused low bits of the last character. Two more of
//! its outcomes are bugs, not leniency, and are not ported (divergence
//! `base64-quirks`): a string of only `=`, four or more, decodes to one NUL
//! byte, and a last group shaped `xx=x` decodes to its first and third
//! bytes. Both are refused here.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding, no line breaks.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            if index <= chunk.len() {
                let sextet = (word >> (18 - 6 * index)) & 0x3F;
                out.push(char::from(ALPHABET[sextet as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn sextet(byte: u8) -> Option<u32> {
    match byte {
        b'A'..=b'Z' => Some(u32::from(byte - b'A')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// `Data(base64Encoded:)`, without its two bugs.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let body_len = bytes.len() - bytes.iter().rev().take_while(|&&byte| byte == b'=').count();
    let (body, padding) = bytes.split_at(body_len);
    if body.is_empty() {
        // "" is no data; "=" … is no base64 (Foundation: one NUL from "====").
        return padding.is_empty().then(Vec::new);
    }
    let needed = match body.len() % 4 {
        0 => 0,
        1 => return None,
        2 => 2,
        _ => 1,
    };
    if padding.len() < needed {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 2);
    for group in body.chunks(4) {
        let mut word = 0u32;
        for (index, &byte) in group.iter().enumerate() {
            word |= sextet(byte)? << (18 - 6 * index);
        }
        let produced = group.len() * 6 / 8;
        out.extend_from_slice(&word.to_be_bytes()[1..1 + produced]);
    }
    Some(out)
}
