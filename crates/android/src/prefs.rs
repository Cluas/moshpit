//! The app's preferences on Android: `SharedPreferences` strings holding
//! the property list values the store reads and writes (`moshpit_store::
//! Defaults`, `docs/nsh/DESIGN.md` §3.5).
//!
//! `SharedPreferences` keeps strings, numbers, Booleans and string sets,
//! not `NSData`, `NSDate`, arrays of mixed values or dictionaries, and its
//! `float` would round a `Double`. So every value is stored as one string
//! that says what it is: a tag, a colon, the value.
//!
//! | value | stored as |
//! | --- | --- |
//! | `Bool` | `b:1`, `b:0` |
//! | `Int` | `i:-42` |
//! | `Double` | `d:` and the 16 hex digits of its IEEE bits (exact, NaN too) |
//! | `String` | `s:` and the text |
//! | `Data` | `x:` and its standard base64 |
//! | `Date` | `t:` and the hex bits of its seconds since 2001 |
//! | `Array` | `a:` and a JSON array of the members, each encoded so |
//! | `Dictionary` | `m:` and a JSON array of `[key, member]` pairs, in order |
//!
//! A stored string this cannot read (no tag, a broken member) reads as no
//! value, which the store treats as a key never written — the same as a
//! `UserDefaults` value of a type the setting does not take.

use moshpit_store::Plist;

/// `value` as the string stored under its key.
pub fn encode(value: &Plist) -> String {
    match value {
        Plist::Bool(flag) => format!("b:{}", u8::from(*flag)),
        Plist::Int(number) => format!("i:{number}"),
        Plist::Double(number) => format!("d:{:016x}", number.to_bits()),
        Plist::String(text) => format!("s:{text}"),
        Plist::Data(bytes) => format!("x:{}", base64_encode(bytes)),
        Plist::Date(seconds) => format!("t:{:016x}", seconds.to_bits()),
        Plist::Array(members) => {
            let members: Vec<String> = members.iter().map(encode).collect();
            format!(
                "a:{}",
                serde_json::to_string(&members).expect("strings serialize")
            )
        }
        Plist::Dictionary(members) => {
            let pairs: Vec<(&str, String)> = members
                .iter()
                .map(|(key, member)| (key.as_str(), encode(member)))
                .collect();
            format!(
                "m:{}",
                serde_json::to_string(&pairs).expect("pairs serialize")
            )
        }
    }
}

/// The value a stored string holds, or `None` when it holds none this can
/// read.
pub fn decode(stored: &str) -> Option<Plist> {
    let (tag, body) = stored.split_once(':')?;
    match tag {
        "b" => match body {
            "1" => Some(Plist::Bool(true)),
            "0" => Some(Plist::Bool(false)),
            _ => None,
        },
        "i" => body.parse().ok().map(Plist::Int),
        "d" => bits(body).map(|b| Plist::Double(f64::from_bits(b))),
        "s" => Some(Plist::String(body.to_owned())),
        "x" => base64_decode(body).map(Plist::Data),
        "t" => bits(body).map(|b| Plist::Date(f64::from_bits(b))),
        "a" => {
            let members: Vec<String> = serde_json::from_str(body).ok()?;
            members
                .iter()
                .map(|m| decode(m))
                .collect::<Option<Vec<_>>>()
                .map(Plist::Array)
        }
        "m" => {
            let pairs: Vec<(String, String)> = serde_json::from_str(body).ok()?;
            let mut seen = std::collections::HashSet::new();
            let mut members = Vec::with_capacity(pairs.len());
            for (key, member) in pairs {
                if !seen.insert(key.clone()) {
                    return None;
                }
                members.push((key, decode(&member)?));
            }
            Some(Plist::Dictionary(members))
        }
        _ => None,
    }
}

fn bits(hex: &str) -> Option<u64> {
    (hex.len() == 16)
        .then(|| u64::from_str_radix(hex, 16).ok())
        .flatten()
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Standard base64, padded or not; `None` for anything else.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut held = 0;
    for byte in text.bytes() {
        let value = ALPHABET.iter().position(|&c| c == byte)? as u32;
        acc = acc << 6 | value;
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((acc >> held) as u8);
            acc &= (1 << held) - 1;
        }
    }
    // Leftover bits must be the zero padding of the last group.
    (acc == 0 && held < 6).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: Plist) {
        let stored = encode(&value);
        let back = decode(&stored).unwrap_or_else(|| panic!("{stored:?} decodes"));
        match (&value, &back) {
            (Plist::Double(a), Plist::Double(b)) | (Plist::Date(a), Plist::Date(b)) => {
                assert_eq!(a.to_bits(), b.to_bits(), "{stored}")
            }
            _ => assert_eq!(value, back, "{stored}"),
        }
    }

    #[test]
    fn every_kind_of_value_comes_back_as_itself() {
        round_trip(Plist::Bool(true));
        round_trip(Plist::Bool(false));
        round_trip(Plist::Int(0));
        round_trip(Plist::Int(i64::MIN));
        round_trip(Plist::Int(i64::MAX));
        round_trip(Plist::Double(14.0));
        round_trip(Plist::Double(0.1 + 0.2));
        round_trip(Plist::Double(-0.0));
        round_trip(Plist::Double(f64::NAN));
        round_trip(Plist::Double(f64::INFINITY));
        round_trip(Plist::String(String::new()));
        round_trip(Plist::String("a:b:c 你好 \u{0}\n".to_owned()));
        round_trip(Plist::Data(Vec::new()));
        round_trip(Plist::Data(b"[{\"id\":\"x\"}]".to_vec()));
        round_trip(Plist::Data((0..=255).collect()));
        round_trip(Plist::Date(784_111_777.5));
        round_trip(Plist::Array(vec![]));
        round_trip(Plist::Array(vec![
            Plist::String("b".into()),
            Plist::Int(3),
            Plist::Array(vec![Plist::Bool(true), Plist::Data(vec![1, 2])]),
        ]));
        round_trip(Plist::Dictionary(vec![
            ("z".into(), Plist::Int(1)),
            (
                "a".into(),
                Plist::Dictionary(vec![("k".into(), Plist::Double(2.5))]),
            ),
        ]));
    }

    #[test]
    fn the_stored_strings_are_readable() {
        assert_eq!(encode(&Plist::Bool(true)), "b:1");
        assert_eq!(encode(&Plist::Int(-42)), "i:-42");
        assert_eq!(encode(&Plist::Double(1.0)), "d:3ff0000000000000");
        assert_eq!(encode(&Plist::String("tmux".into())), "s:tmux");
        assert_eq!(encode(&Plist::Data(b"hi!".to_vec())), "x:aGkh");
        assert_eq!(
            encode(&Plist::Array(vec![Plist::String("a".into())])),
            r#"a:["s:a"]"#
        );
    }

    #[test]
    fn what_cannot_be_read_is_no_value() {
        for stored in [
            "",
            "tmux",
            "b:2",
            "i:1.5",
            "d:3ff",
            "d:zzzzzzzzzzzzzzzz",
            "x:a",
            "x:@@@@",
            "a:[1]",
            "a:not json",
            r#"m:[["k","b:1"],["k","b:0"]]"#,
            "q:1",
        ] {
            assert_eq!(decode(stored), None, "{stored:?}");
        }
    }

    #[test]
    fn base64_matches_the_standard_vectors() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(plain.as_bytes()), coded);
            assert_eq!(base64_decode(coded).unwrap(), plain.as_bytes());
        }
    }
}
