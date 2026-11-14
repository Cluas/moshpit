//! Foundation's rules as the Swift herdr code relies on them: what
//! `JSONSerialization` accepts, what Swift's `as?` casts make of its numbers,
//! what `Data(base64Encoded:)` accepts, and the text helpers. Each case was
//! measured on macOS 27 (`harness/swift/probe-foundation.swift` printed the same
//! table from Foundation itself; at cf25626b, the Swift side left with E1).

use moshpit_herdr::json::{self, Number, Value};
use moshpit_herdr::{base64, text};

fn parse(text: &str) -> Option<Value> {
    json::parse(text.as_bytes())
}

fn field(text: &str) -> Option<Value> {
    json::parse_object(format!("{{\"v\":{text}}}").as_bytes())
        .map(|object| object.get("v").cloned().expect("v"))
}

/// `(as? Int, as? UInt64, as? Bool)` of a number literal.
fn casts(literal: &str) -> (Option<i64>, Option<u64>, Option<bool>) {
    let value = field(literal).unwrap_or_else(|| panic!("{literal} refused"));
    (value.as_i64(), value.as_u64(), value.as_bool())
}

// MARK: - What JSONSerialization accepts

#[test]
fn only_an_object_or_an_array_at_the_top() {
    assert!(parse("\"x\"").is_none());
    assert!(parse("1").is_none());
    assert!(parse("{}").is_some());
    assert!(parse("[]").is_some());
    assert!(parse(" {} ").is_some());
    assert!(parse("{}\n").is_some());
    assert!(parse("{} x").is_none(), "garbage at end");
    // Only space, tab, LF and CR are whitespace.
    assert!(parse("\u{c}{}").is_none());
    assert!(parse("\u{a0}{}").is_none());
    assert!(parse("{ \"a\"\u{b}:1}").is_none());
    // A leading byte order mark is skipped.
    assert!(json::parse(b"\xEF\xBB\xBF{}").is_some());
}

#[test]
fn one_trailing_comma_is_allowed() {
    assert!(parse("{\"a\":1,}").is_some());
    assert!(parse("[1,]").is_some());
    assert!(parse("[1,2,]").is_some());
    assert!(parse("{\"a\":1 , }").is_some());
    assert!(parse("[1,\n]").is_some());
    for refused in [
        "[1,,]",
        "{,}",
        "[,]",
        "{\"a\":1,,}",
        "[ , 1]",
        "[1 2]",
        "{\"a\" 1}",
        "{\"a\":}",
    ] {
        assert!(parse(refused).is_none(), "{refused}");
    }
}

#[test]
fn a_repeated_key_keeps_its_first_value() {
    let object = json::parse_object(br#"{"a":1,"a":"x"}"#).unwrap();
    assert_eq!(object.i64("a"), Some(1));
    let object = json::parse_object(br#"{"a":{"b":1},"a":2}"#).unwrap();
    assert!(object.object("a").is_some());
}

#[test]
fn no_value_may_sit_inside_513_containers() {
    let arrays = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    let objects = |depth: usize| format!("{}1{}", "{\"a\":".repeat(depth), "}".repeat(depth));
    assert!(parse(&arrays(513)).is_some(), "the 513th array is empty");
    assert!(parse(&arrays(514)).is_none());
    assert!(parse(&objects(512)).is_some());
    assert!(parse(&objects(513)).is_none(), "its 1 sits inside 513");
    let empty_inside = format!("{}{{}}{}", "{\"a\":".repeat(512), "}".repeat(512));
    assert!(parse(&empty_inside).is_some());
    assert!(
        parse(&arrays(100_000)).is_none(),
        "no stack overflow either"
    );
}

#[test]
fn strings_follow_the_grammar_strictly() {
    let string = |text: &str| field(text).and_then(|value| value.as_str().map(str::to_owned));
    assert_eq!(
        string(r#""\/\b\f\n\r\t\"\\""#).as_deref(),
        Some("/\u{8}\u{c}\n\r\t\"\\")
    );
    assert_eq!(string(r#""\u0000""#).as_deref(), Some("\0"));
    assert_eq!(string(r#""éé""#).as_deref(), Some("éé"));
    assert_eq!(string(r#""😀""#).as_deref(), Some("😀"));
    assert_eq!(string(r#""\udbff\udfff""#).as_deref(), Some("\u{10FFFF}"));
    assert_eq!(string("\"\u{7f}\"").as_deref(), Some("\u{7f}"));
    for refused in [
        "\"\u{1}\"",         // raw control character
        "\"\n\"",            // raw newline
        r#""\ud800""#,       // lone high surrogate
        r#""\udc00""#,       // lone low surrogate
        r#""\ude00\ud83d""#, // reversed pair
        r#""\ud800A""#,      // high + not-a-low
        r#""\ud800\ud800""#, // high + high
        r#""\x""#,           // unknown escape
        r#""\U0041""#,       // uppercase U
        r#""\u12""#,         // short escape
        r#""\u12g4""#,       // not hex
    ] {
        assert!(field(refused).is_none(), "{refused:?}");
    }
    // Invalid UTF-8 anywhere in a string.
    for bytes in [
        &b"{\"a\":\"\xff\"}"[..],
        b"{\"a\":\"\xc0\xaf\"}",
        b"{\"a\":\"\xed\xa0\x80\"}",
    ] {
        assert!(json::parse(bytes).is_none());
    }
}

/// Divergence json-bom: Foundation drops a leading U+FEFF from every string;
/// the port keeps strings as written.
#[test]
fn a_leading_byte_order_mark_in_a_string_is_kept() {
    let object = json::parse_object("{\"a\":\"\u{feff}x\",\"\u{feff}k\":1}".as_bytes()).unwrap();
    assert_eq!(object.str("a"), Some("\u{feff}x"));
    assert_eq!(object.i64("\u{feff}k"), Some(1));
    assert_eq!(object.i64("k"), None);
}

/// Divergence json-encoding-sniff: Foundation reads NUL-interleaved bytes as
/// UTF-16 or UTF-32; JSON on a network is UTF-8.
#[test]
fn json_is_utf8_only() {
    assert!(json::parse(b"{\0\"\0a\0\"\0:\x001\0}\0").is_none());
    assert!(json::parse(b"{\0\0\0}\0\0\0").is_none());
}

#[test]
fn literals_are_exact() {
    assert_eq!(
        parse("[true,false,null]"),
        Some(Value::Array(vec![
            Value::Bool(true),
            Value::Bool(false),
            Value::Null
        ]))
    );
    for refused in [
        "{\"a\":True}",
        "{\"a\":nul}",
        "{\"a\":nulll}",
        "{\"a\":tru}",
    ] {
        assert!(parse(refused).is_none(), "{refused}");
    }
}

// MARK: - Numbers

#[test]
fn the_number_grammar_is_strict() {
    for refused in [
        "01", "-", "1.", "-.5", ".5", "1e", "1e+", "+1", "Infinity", "NaN", "00", "-01", "0.",
        "--1", "2.", "1ee5", "1.e5", "01.5",
    ] {
        assert!(field(refused).is_none(), "{refused}");
    }
}

#[test]
fn integers_are_int64_then_uint64_then_decimal() {
    assert_eq!(field("0"), Some(Value::Number(Number::Int(0))));
    assert_eq!(field("-0"), Some(Value::Number(Number::Int(0))));
    assert_eq!(
        field("9223372036854775807"),
        Some(Value::Number(Number::Int(i64::MAX)))
    );
    assert_eq!(
        field("-9223372036854775808"),
        Some(Value::Number(Number::Int(i64::MIN)))
    );
    assert_eq!(
        field("9223372036854775808"),
        Some(Value::Number(Number::UInt(1 << 63)))
    );
    assert_eq!(
        field("18446744073709551615"),
        Some(Value::Number(Number::UInt(u64::MAX)))
    );
    assert!(matches!(
        field("18446744073709551616"),
        Some(Value::Number(Number::Decimal(_)))
    ));
    assert!(matches!(
        field("-9223372036854775809"),
        Some(Value::Number(Number::Decimal(_)))
    ));
}

#[test]
fn fractions_and_exponents_are_doubles_up_to_17_digits_and_decimals_beyond() {
    let is_double =
        |literal: &str| matches!(field(literal), Some(Value::Number(Number::Double(_))));
    for literal in [
        "1.0",
        "1.5",
        "2e0",
        "1E+2",
        "0.12345678901234567",
        "123.45678901234567",
        "0.00000000000000001",
        "1234567890.1234567",
        "1.7976931348623157e308",
        "1e18",
    ] {
        assert!(is_double(literal), "{literal}");
    }
    for literal in [
        "0.123456789012345678",
        "12345678901234567.0",
        "9.223372036854775807e0",
        "0.000000000000000001",
        "1234567890.12345678",
        "1.0000000000000000000",
        "12345678901234567890e0",
    ] {
        assert!(
            matches!(field(literal), Some(Value::Number(Number::Decimal(_)))),
            "{literal}"
        );
    }
}

#[test]
fn number_limits_are_foundations() {
    // +∞ is refused, −∞ kept; underflow is zero (or a subnormal).
    assert!(field("1e400").is_none());
    assert!(field("1.8e308").is_none());
    assert_eq!(
        field("-1e309"),
        Some(Value::Number(Number::Double(f64::NEG_INFINITY)))
    );
    assert_eq!(field("1e-400"), Some(Value::Number(Number::Double(0.0))));
    assert!(
        matches!(field("-1e-400"), Some(Value::Number(Number::Double(v))) if v == 0.0 && v.is_sign_negative())
    );
    assert!(matches!(field("4e-320"), Some(Value::Number(Number::Double(v))) if v > 0.0));
    // An exponent of more than four characters, sign included, is refused.
    assert!(field("1e0001").is_some());
    assert!(field("1e+0001").is_none());
    assert!(field("1e-0001").is_none());
    assert!(field("1e00001").is_none());
    assert!(field("0e0000").is_some());
    assert!(field("0e00000").is_none());
    assert!(field("1e-9999").is_none());
    assert!(field("1e-400").is_some());
}

/// Swift's `as?` casts take a number by exact value.
#[test]
fn the_casts_take_a_number_by_exact_value() {
    assert_eq!(casts("1"), (Some(1), Some(1), Some(true)));
    assert_eq!(casts("0"), (Some(0), Some(0), Some(false)));
    assert_eq!(casts("-0"), (Some(0), Some(0), Some(false)));
    assert_eq!(casts("-0.0"), (Some(0), Some(0), Some(false)));
    assert_eq!(casts("2"), (Some(2), Some(2), None));
    assert_eq!(casts("-1"), (Some(-1), None, None));
    assert_eq!(casts("1.0"), (Some(1), Some(1), Some(true)));
    assert_eq!(casts("1.5"), (None, None, None));
    assert_eq!(casts("2e0"), (Some(2), Some(2), None));
    assert_eq!(casts("100e-2"), (Some(1), Some(1), Some(true)));
    assert_eq!(
        casts("1e19"),
        (None, Some(10_000_000_000_000_000_000), None)
    );
    assert_eq!(casts("1e20"), (None, None, None));
    assert_eq!(casts("18446744073709551615"), (None, Some(u64::MAX), None));
    assert_eq!(casts("true"), (Some(1), Some(1), Some(true)));
    assert_eq!(casts("false"), (Some(0), Some(0), Some(false)));
    assert_eq!(casts("-1e309"), (None, None, None));
    // Decimals: by exact value (divergence json-decimal for the inexact ones).
    assert_eq!(
        casts("1.8446744073709551615e19"),
        (None, Some(u64::MAX), None)
    );
    assert_eq!(
        casts("9223372036854775807.0"),
        (Some(i64::MAX), Some(i64::MAX as u64), None)
    );
    assert_eq!(
        casts("1.0000000000000000000"),
        (Some(1), Some(1), Some(true))
    );
    assert_eq!(casts("100000000000000000.5"), (None, None, None));
    assert_eq!(casts("99999999999999999999"), (None, None, None));
    assert_eq!(field("null").unwrap().as_i64(), None);
    assert_eq!(field("\"1\"").unwrap().as_i64(), None);
}

#[test]
fn an_array_cast_to_objects_is_all_or_nothing() {
    let object = json::parse_object(br#"{"a":[{},{"x":1}],"b":[{},2],"c":[]}"#).unwrap();
    assert_eq!(object.objects("a").map(|items| items.len()), Some(2));
    assert!(object.objects("b").is_none());
    assert_eq!(object.objects("c").map(|items| items.len()), Some(0));
}

// MARK: - first_object

#[test]
fn the_first_object_is_found_past_noise_and_through_braces_in_strings() {
    let object =
        json::first_object("Welcome\n{\"label\":\"weird{name}\",\"x\":\"\\\"}\"} trailing")
            .unwrap();
    assert_eq!(object.str("label"), Some("weird{name}"));
    assert_eq!(object.str("x"), Some("\"}"));
    // Only the first object is tried.
    assert!(json::first_object("{bad} {\"good\":1}").is_none());
    assert!(json::first_object("{\"never\":\"closes\"").is_none());
    assert!(json::first_object("no brace").is_none());
    // An array first doesn't count: the first `{` does.
    assert_eq!(json::first_object("[{\"a\":1}]").unwrap().i64("a"), Some(1));
}

// MARK: - Encoding

#[test]
fn strings_are_encoded_as_jsonencoder_writes_them() {
    assert_eq!(json::encode_string("plain"), "\"plain\"");
    assert_eq!(
        json::encode_string("\0\u{1}\u{7}\u{8}\t\n\u{b}\u{c}\r\u{e}\u{1f} \"/\\\u{7f}é\u{2028}😀"),
        "\"\\u0000\\u0001\\u0007\\b\\t\\n\\u000b\\f\\r\\u000e\\u001f \\\"\\/\\\\\u{7f}é\u{2028}😀\""
    );
}

// MARK: - Base64

#[test]
fn base64_encodes_the_standard_alphabet_with_padding() {
    assert_eq!(base64::encode(b""), "");
    assert_eq!(base64::encode(b"a"), "YQ==");
    assert_eq!(base64::encode(b"ab"), "YWI=");
    assert_eq!(base64::encode(b"abc"), "YWJj");
    assert_eq!(base64::encode(&[0xfb, 0xff]), "+/8=");
}

#[test]
fn base64_decodes_what_foundation_does() {
    let decode = |text: &str| base64::decode(text);
    assert_eq!(decode("aGk="), Some(b"hi".to_vec()));
    assert_eq!(decode(""), Some(Vec::new()));
    // Extra padding after a complete group is accepted.
    assert_eq!(decode("aGk=="), Some(b"hi".to_vec()));
    assert_eq!(decode("aGk==="), Some(b"hi".to_vec()));
    assert_eq!(decode("YWJj="), Some(b"abc".to_vec()));
    // Unused low bits of the last character are ignored.
    assert_eq!(decode("aGl="), Some(b"hi".to_vec()));
    assert_eq!(decode("YR=="), Some(b"a".to_vec()));
    // Padding is required, the alphabet is standard, nothing else is allowed.
    for refused in [
        "aGk",
        "YQ=",
        "YQ",
        "Y",
        "=",
        "==",
        "===",
        "YQ==YQ==",
        "-_8=",
        "aGk*",
        "aG k=",
        "aGk=\n",
        " YQ==",
        "YQ== ",
        "YQ=\u{0}=",
    ] {
        assert_eq!(decode(refused), None, "{refused:?}");
    }
}

/// Divergence base64-quirks: Foundation decodes these, which are not base64.
#[test]
fn foundations_two_base64_bugs_are_refused() {
    assert_eq!(base64::decode("===="), None, "Foundation: one NUL byte");
    assert_eq!(base64::decode("========"), None);
    assert_eq!(
        base64::decode("AAAAAA=Q"),
        None,
        "Foundation: bytes 0 and 2 of the last group"
    );
}

#[test]
fn base64_round_trips_every_byte() {
    let bytes: Vec<u8> = (0..=255).collect();
    for len in 0..bytes.len() {
        assert_eq!(
            base64::decode(&base64::encode(&bytes[..len])).as_deref(),
            Some(&bytes[..len])
        );
    }
}

// MARK: - Text

#[test]
fn apples_whitespace_sets_trim_as_foundation_does() {
    // U+200B ZERO WIDTH SPACE still counts as whitespace to Foundation.
    assert_eq!(text::trim_whitespaces("\u{200b}x\u{a0}"), "x");
    assert_eq!(
        text::trim_whitespaces("\u{85}x"),
        "\u{85}x",
        "a newline, not whitespace"
    );
    assert_eq!(text::trim_whitespaces_and_newlines("\u{85}x\u{2028}"), "x");
    assert_eq!(text::trim_whitespaces_and_newlines("\u{b}x\u{c}"), "x");
    assert_eq!(
        text::trim_whitespaces_and_newlines("\u{feff}x"),
        "\u{feff}x"
    );
    assert_eq!(
        text::trim_whitespaces_and_newlines("\u{180e}x"),
        "\u{180e}x"
    );
    // Scalars, not Characters: a space carrying a combining mark is trimmed.
    assert_eq!(text::trim_whitespaces_and_newlines(" \u{301}x"), "\u{301}x");
}

#[test]
fn newline_splitting_matches_foundation_and_swift() {
    let components: Vec<&str> =
        text::components_by_newlines("a\r\nb\u{2028}c\u{85}d\u{b}e").collect();
    assert_eq!(components, ["a", "", "b", "c", "d", "e"]);
    let lines: Vec<&str> = text::lines("a\r\n\r\nb\n\u{301}c").collect();
    assert_eq!(lines, ["a", "b", "\u{301}c"]);
}

#[test]
fn utf8_decoding_matches_foundation() {
    // String(bytes:encoding:): strict, a leading BOM is a signature.
    assert_eq!(text::decode_utf8(b"\xEF\xBB\xBFhi").as_deref(), Some("hi"));
    assert_eq!(
        text::decode_utf8(b"\xEF\xBB\xBF\xEF\xBB\xBFh").as_deref(),
        Some("\u{feff}h")
    );
    assert_eq!(text::decode_utf8(b"h\0i").as_deref(), Some("h\0i"));
    assert_eq!(text::decode_utf8(b"h\xff"), None);
    // String(decoding:as:): every maximal invalid sequence is one U+FFFD.
    assert_eq!(text::decode_utf8_lossy(b"h\xffi"), "h\u{fffd}i");
    assert_eq!(text::decode_utf8_lossy(b"\xc0\xafh"), "\u{fffd}\u{fffd}h");
    assert_eq!(
        text::decode_utf8_lossy(b"\xed\xa0\x80h"),
        "\u{fffd}\u{fffd}\u{fffd}h"
    );
    assert_eq!(text::decode_utf8_lossy(b"\xf0\x9f\x98"), "\u{fffd}");
    assert_eq!(text::decode_utf8_lossy(b"\xf0\x9f\x98A"), "\u{fffd}A");
    assert_eq!(text::decode_utf8_lossy(b"\xef\xbb\xbfh"), "\u{feff}h");
}

#[test]
fn swift_int_parsing() {
    assert_eq!(text::swift_int("+5"), Some(5));
    assert_eq!(text::swift_int("-0"), Some(0));
    assert_eq!(text::swift_int("007"), Some(7));
    for refused in ["", "+", "-", "3a", " 5", "٣", "99999999999999999999"] {
        assert_eq!(text::swift_int(refused), None, "{refused:?}");
    }
}
