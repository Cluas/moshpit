//! The values a `UserDefaults` key holds — property list values — and how
//! the app's Swift reads them back.
//!
//! Every key the app keeps in `UserDefaults.standard` holds one of these:
//! the JSON lists as `Data`, the settings as `Bool`, `Int`, `Double`,
//! `String` or an array of strings, the backups of unreadable lists as an
//! array of whatever was stored. A host stores a [`Plist`] under a key and
//! gives the same value back (on iOS as the `NSNumber`, `NSString`, `NSData`,
//! `NSDate`, `NSArray`, `NSDictionary` it maps to; on Android in whatever
//! encoding its preferences need), and the store reads it with the bridging
//! rules below, measured on macOS 27 against `UserDefaults` itself.

/// A property list value.
#[derive(Debug, Clone, PartialEq)]
pub enum Plist {
    /// `kCFBooleanTrue` / `kCFBooleanFalse`.
    Bool(bool),
    /// An integer `NSNumber`.
    Int(i64),
    /// A floating `NSNumber`.
    Double(f64),
    String(String),
    Data(Vec<u8>),
    /// An `NSDate`: seconds since 2001-01-01T00:00:00Z. The app stores none,
    /// but a backup copies whatever a key held.
    Date(f64),
    Array(Vec<Plist>),
    /// Members in the order the host gave them; keys unique.
    Dictionary(Vec<(String, Plist)>),
}

impl Plist {
    /// `value as? Bool`: a Boolean, or a number that is exactly 0 or 1.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(flag) => Some(*flag),
            Self::Int(0) => Some(false),
            Self::Int(1) => Some(true),
            Self::Double(value) if *value == 0.0 => Some(false),
            Self::Double(value) if *value == 1.0 => Some(true),
            _ => None,
        }
    }

    /// `value as? Int`: a Boolean (0 or 1), an integer, or a floating number
    /// with an integral value inside `Int`'s range.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Bool(flag) => Some(i64::from(*flag)),
            Self::Int(value) => Some(*value),
            Self::Double(value) => exact_i64(*value),
            _ => None,
        }
    }

    /// `value as? Double`: a Boolean, an integer a `Double` holds exactly,
    /// or a floating number.
    pub fn as_double(&self) -> Option<f64> {
        match self {
            Self::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
            Self::Int(value) => exact_f64(*value),
            Self::Double(value) => Some(*value),
            _ => None,
        }
    }

    /// `value as? String`: strings only; a number is no string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// `value as? Data`.
    pub fn as_data(&self) -> Option<&[u8]> {
        match self {
            Self::Data(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// `value as? [Any]`.
    pub fn as_array(&self) -> Option<&[Plist]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// `(a as AnyObject).isEqual(b)`: numbers (Booleans among them) by
    /// value, strings by their UTF-16 (so precomposed and decomposed forms
    /// differ, unlike Swift's `==`), data by bytes, collections member by
    /// member; values of different kinds are never equal.
    pub fn ns_equal(&self, other: &Plist) -> bool {
        match (self, other) {
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Data(a), Self::Data(b)) => a == b,
            (Self::Date(a), Self::Date(b)) => a == b,
            (Self::Array(a), Self::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.ns_equal(y))
            }
            (Self::Dictionary(a), Self::Dictionary(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(key, value)| {
                        b.iter()
                            .find(|(other, _)| other == key)
                            .is_some_and(|(_, theirs)| value.ns_equal(theirs))
                    })
            }
            _ => match (self.number(), other.number()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    }

    /// The value of a number (a Boolean is 0 or 1), compared exactly.
    fn number(&self) -> Option<Number> {
        match self {
            Self::Bool(flag) => Some(Number::Int(i64::from(*flag))),
            Self::Int(value) => Some(Number::Int(*value)),
            Self::Double(value) => Some(Number::Double(*value)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Number {
    Int(i64),
    Double(f64),
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Double(a), Self::Double(b)) => a == b,
            (Self::Int(i), Self::Double(d)) | (Self::Double(d), Self::Int(i)) => {
                exact_i64(d) == Some(i)
            }
        }
    }
}

/// `d` when it is integral and inside `Int`'s range.
fn exact_i64(value: f64) -> Option<i64> {
    (value.fract() == 0.0
        && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value))
    .then_some(value as i64)
}

/// `i` as a `Double`, when the `Double` holds it exactly.
fn exact_f64(value: i64) -> Option<f64> {
    let double = value as f64;
    (double != 9_223_372_036_854_775_808.0 && double as i64 == value).then_some(double)
}

/// `UserDefaults.bool(forKey:)` over what a key holds: a Boolean; any
/// non-zero number; the strings `YES` and `true` in any case, and `1`.
/// Anything else, an absent key included, is false.
pub fn bool_value(value: Option<&Plist>) -> bool {
    match value {
        Some(Plist::Bool(flag)) => *flag,
        Some(Plist::Int(number)) => *number != 0,
        Some(Plist::Double(number)) => *number != 0.0,
        Some(Plist::String(text)) => {
            text.eq_ignore_ascii_case("yes") || text.eq_ignore_ascii_case("true") || text == "1"
        }
        _ => false,
    }
}

/// `UserDefaults.stringArray(forKey:)`: an array of strings only.
pub fn string_array(value: Option<&Plist>) -> Option<Vec<String>> {
    value?
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// The strings in an array, whatever else is in it; `None` when it isn't
/// an array. Fix (B159): [`string_array`]'s "strings only" read an array
/// with one item of another type as no array at all.
pub fn strings_in(value: Option<&Plist>) -> Option<Vec<String>> {
    Some(
        value?
            .as_array()?
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `object(forKey:) as? T` and `bool(forKey:)` answered on macOS 27.
    #[test]
    fn bridging_as_measured() {
        use Plist::*;
        /// A value, then `as? Bool`, `as? Int`, `as? Double`, `bool(forKey:)`.
        type Case = (Plist, Option<bool>, Option<i64>, Option<f64>, bool);
        let cases: [Case; 13] = [
            (Bool(true), Some(true), Some(1), Some(1.0), true),
            (Bool(false), Some(false), Some(0), Some(0.0), false),
            (Int(0), Some(false), Some(0), Some(0.0), false),
            (Int(1), Some(true), Some(1), Some(1.0), true),
            (Int(2), None, Some(2), Some(2.0), true),
            (Int(-1), None, Some(-1), Some(-1.0), true),
            (Double(0.0), Some(false), Some(0), Some(0.0), false),
            (Double(0.5), None, None, Some(0.5), true),
            (Double(1.0), Some(true), Some(1), Some(1.0), true),
            (Double(1.5), None, None, Some(1.5), true),
            (Double(2.0), None, Some(2), Some(2.0), true),
            (Double(1e300), None, None, Some(1e300), true),
            (Int(i64::MAX), None, Some(i64::MAX), None, true),
        ];
        for (value, as_bool, as_int, as_double, defaults_bool) in cases {
            assert_eq!(value.as_bool(), as_bool, "{value:?}");
            assert_eq!(value.as_int(), as_int, "{value:?}");
            assert_eq!(value.as_double(), as_double, "{value:?}");
            assert_eq!(bool_value(Some(&value)), defaults_bool, "{value:?}");
            assert_eq!(value.as_str(), None);
        }
        for (text, expected) in [
            ("YES", true),
            ("yes", true),
            ("Yes", true),
            ("true", true),
            ("TRUE", true),
            ("trUE", true),
            ("1", true),
            ("0", false),
            ("2", false),
            ("01", false),
            ("11", false),
            ("1.0", false),
            (" 1", false),
            ("true ", false),
            ("y", false),
            ("Y", false),
            ("t", false),
            ("Yx", false),
            ("tomato", false),
            ("9", false),
            ("no", false),
            ("NO", false),
            ("false", false),
            ("", false),
            ("１", false),
        ] {
            assert_eq!(bool_value(Some(&String(text.into()))), expected, "{text:?}");
        }
        assert!(!bool_value(None));
        assert!(!bool_value(Some(&Data(vec![1]))));
        assert!(!bool_value(Some(&Array(vec![]))));
    }

    #[test]
    fn isequal_as_measured() {
        use Plist::*;
        assert!(Int(1).ns_equal(&Bool(true)));
        assert!(Bool(true).ns_equal(&Int(1)));
        assert!(Double(1.0).ns_equal(&Int(1)));
        assert!(!Double(1.5).ns_equal(&Int(1)));
        assert!(Data(vec![1]).ns_equal(&Data(vec![1])));
        assert!(!String("\u{e9}".into()).ns_equal(&String("e\u{301}".into())));
        assert!(!String("1".into()).ns_equal(&Int(1)));
        assert!(!Data(b"1".to_vec()).ns_equal(&String("1".into())));
        assert!(
            Array(vec![Int(1), String("a".into())])
                .ns_equal(&Array(vec![Bool(true), String("a".into())]))
        );
    }

    #[test]
    fn string_arrays_hold_only_strings() {
        use Plist::*;
        assert_eq!(
            string_array(Some(&Array(vec![String("a".into()), String("b".into())]))),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(
            string_array(Some(&Array(vec![String("a".into()), Int(1)]))),
            None
        );
        assert_eq!(string_array(Some(&Array(vec![]))), Some(vec![]));
        assert_eq!(string_array(Some(&String("a".into()))), None);
        assert_eq!(string_array(None), None);
        assert_eq!(
            strings_in(Some(&Array(vec![String("a".into()), Int(1), String("b".into())]))),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(strings_in(Some(&String("a".into()))), None);
        assert_eq!(strings_in(None), None);
    }
}
