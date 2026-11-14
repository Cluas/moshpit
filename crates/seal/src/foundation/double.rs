//! How Foundation prints a `Double` into JSON.
//!
//! `JSONEncoder` writes Swift's shortest round-tripping form (`description`)
//! minus a trailing `.0`; `JSONSerialization` writes C's `%.17g`. Both are
//! rebuilt here from Rust's own correctly rounded digits.

/// The shortest decimal digits that round-trip `x` (finite, non-zero) and the
/// decimal exponent of the first one: `x = d1.d2… × 10^exponent`.
///
/// Of two shortest candidates equally close to `x`, Swift writes the one
/// with the even last digit (`1000000000000000.25` is `…000.2`); Rust's
/// shortest mode rounds such a tie up. Its exact mode rounds half to even,
/// so the correctly rounded value of the same length is Swift's choice
/// whenever it round-trips.
fn shortest_digits(x: f64) -> (String, i32) {
    let x = x.abs();
    let (digits, exponent) = split_scientific(&format!("{x:e}"));
    let closest = format!("{:.*e}", digits.len() - 1, x);
    if closest.parse::<f64>().ok() == Some(x) {
        let (even, even_exponent) = split_scientific(&closest);
        if even.len() == digits.len() {
            return (even, even_exponent);
        }
    }
    (digits, exponent)
}

/// `digits` and exponent of Rust's `{:e}` / `{:.Ne}` output (`"1.25e-7"`).
fn split_scientific(text: &str) -> (String, i32) {
    let (mantissa, exponent) = text.split_once('e').expect("scientific notation");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    (digits, exponent.parse().expect("an exponent"))
}

/// `digits` (first digit non-zero) with the decimal point after
/// `exponent + 1` of them, positional, no exponent.
fn positional(digits: &str, exponent: i32) -> String {
    let count = i32::try_from(digits.len()).expect("few digits");
    if exponent < 0 {
        let zeros = usize::try_from(-exponent - 1).expect("non-negative");
        format!("0.{}{digits}", "0".repeat(zeros))
    } else if exponent + 1 >= count {
        let zeros = usize::try_from(exponent + 1 - count).expect("non-negative");
        format!("{digits}{}", "0".repeat(zeros))
    } else {
        let point = usize::try_from(exponent + 1).expect("non-negative");
        format!("{}.{}", &digits[..point], &digits[point..])
    }
}

/// `d.ddde+XX`: at least two exponent digits, always signed.
fn exponential(digits: &str, exponent: i32) -> String {
    let sign = if exponent < 0 { '-' } else { '+' };
    let mantissa = if digits.len() > 1 {
        format!("{}.{}", &digits[..1], &digits[1..])
    } else {
        digits.to_owned()
    };
    format!("{mantissa}e{sign}{:02}", exponent.unsigned_abs())
}

/// A finite `Double` as `JSONEncoder` writes it: Swift's `description`
/// (shortest round-trip digits; positional from 1e-4 through 2^53, exponential
/// outside that) without a trailing `.0`. `784000000`, `784000000.123456`,
/// `1e-05`, `9.007199254740994e+15`, `-0`.
pub fn encoder(x: f64) -> String {
    assert!(x.is_finite(), "JSONEncoder refuses non-finite numbers");
    let sign = if x.is_sign_negative() { "-" } else { "" };
    if x == 0.0 {
        return format!("{sign}0");
    }
    let (digits, exponent) = shortest_digits(x);
    let body = if x.abs() <= 9_007_199_254_740_992.0 && exponent >= -4 {
        positional(&digits, exponent)
    } else {
        exponential(&digits, exponent)
    };
    format!("{sign}{body}")
}

/// C's `printf("%.17g", x)`, which `JSONSerialization` uses for a `Double`:
/// 17 significant digits, trailing zeros dropped, exponential when the
/// exponent is below -4 or at least 17. `0.10000000000000001`, `1e+17`.
pub fn printf_17g(x: f64) -> String {
    assert!(
        x.is_finite(),
        "JSONSerialization refuses non-finite numbers"
    );
    let sign = if x.is_sign_negative() { "-" } else { "" };
    if x == 0.0 {
        return format!("{sign}0");
    }
    let (digits, exponent) = split_scientific(&format!("{:.16e}", x.abs()));
    let digits = digits.trim_end_matches('0');
    let body = if (-4..17).contains(&exponent) {
        positional(digits, exponent)
    } else {
        exponential(digits, exponent)
    };
    format!("{sign}{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every pair printed by `JSONEncoder().encode([x])` on macOS 27. The
    /// literals are the exact doubles, ties included.
    #[test]
    #[allow(clippy::excessive_precision)]
    fn encoder_matches_swift() {
        for (x, swift) in [
            (0.0, "0"),
            (1.0, "1"),
            (-1.0, "-1"),
            (0.1, "0.1"),
            (784_000_000.0, "784000000"),
            (784_000_000.5, "784000000.5"),
            (784_123_456.789_012, "784123456.789012"),
            (1e21, "1e+21"),
            (1e-7, "1e-07"),
            (123_456_789_012_345_680_000.0, "1.2345678901234568e+20"),
            (5e-324, "5e-324"),
            (1.797_693_134_862_315_7e308, "1.7976931348623157e+308"),
            (100.0, "100"),
            (1e16, "1e+16"),
            (1e15 + 0.3, "1000000000000000.2"),
            (1_000_000_000_000_000.75, "1000000000000000.8"),
            (1_000_000_000_000_001.25, "1000000000000001.2"),
            (4_503_599_627_370_497.5, "4503599627370498"),
            (1_125_899_906_842_624.375, "1125899906842624.5"),
            (0.001, "0.001"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (0.000_123, "0.000123"),
            (9e15, "9000000000000000"),
            (9_007_199_254_740_992.0, "9007199254740992"),
            (9_007_199_254_740_994.0, "9.007199254740994e+15"),
            (9.5e15, "9.5e+15"),
            (1.5e16, "1.5e+16"),
            (123_456_789_012_345.6, "123456789012345.6"),
            (1_234_567_890_123_456.7, "1234567890123456.8"),
            (-0.0, "-0"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e100, "1e+100"),
            (1.0 / 3.0, "0.3333333333333333"),
        ] {
            assert_eq!(encoder(x), swift, "{x:e}");
        }
    }

    /// `JSONSerialization` output for the parsed literal, macOS 27.
    #[test]
    fn printf_17g_matches_jsonserialization() {
        for (x, apple) in [
            (0.1, "0.10000000000000001"),
            (0.7, "0.69999999999999996"),
            (1.5e300, "1.5000000000000001e+300"),
            (1e-7, "9.9999999999999995e-08"),
            (1.1, "1.1000000000000001"),
            (2.5e-5, "2.5000000000000001e-05"),
            (0.0001, "0.0001"),
            (0.00001, "1.0000000000000001e-05"),
            (1e15, "1000000000000000"),
            (1e16, "10000000000000000"),
            (1e17, "1e+17"),
            (-1.5, "-1.5"),
            (4e-324, "4.9406564584124654e-324"),
            (-0.0, "-0"),
            (1000.0, "1000"),
            (1e308, "1e+308"),
            (1_234_567_890_123_456.7, "1234567890123456.8"),
            (0.123_456_789_012_345_67, "0.12345678901234566"),
            (123_456.789_012_345_67, "123456.78901234567"),
            (1.5e-5, "1.5e-05"),
            (12_300_000.0, "12300000"),
            (150.0, "150"),
            (2.225_073_858_507_201_4e-308, "2.2250738585072014e-308"),
            (0.5, "0.5"),
            (123.456, "123.456"),
            (1e20, "1e+20"),
            (1.234_567_89e18, "1.23456789e+18"),
        ] {
            assert_eq!(printf_17g(x), apple, "{x:e}");
        }
    }
}
