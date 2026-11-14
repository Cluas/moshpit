//! Foundation's `Date`: seconds since 2001-01-01T00:00:00Z in a `Double`,
//! encoded that way by `JSONEncoder`'s default strategy and as
//! `2025-11-10T18:40:00Z` by its `.iso8601` one.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Seconds between the Unix epoch and Foundation's reference date.
pub const REFERENCE_UNIX: f64 = 978_307_200.0;

/// A point in time as Swift holds it: `timeIntervalSinceReferenceDate`.
/// (De)serializes as that number, which is how `JSONEncoder` writes a `Date`
/// unless told otherwise — `push-pairings.json` and the standing set use it.
#[derive(Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Date(pub f64);

impl Date {
    pub fn now() -> Self {
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |elapsed| elapsed.as_secs_f64());
        Self::from_unix(unix)
    }

    /// `Date(timeIntervalSince1970:)`.
    pub fn from_unix(seconds: f64) -> Self {
        Self(seconds - REFERENCE_UNIX)
    }

    /// `timeIntervalSince1970`.
    pub fn unix(self) -> f64 {
        self.0 + REFERENCE_UNIX
    }

    /// `timeIntervalSinceReferenceDate`.
    pub fn since_reference(self) -> f64 {
        self.0
    }

    /// `addingTimeInterval(_:)`.
    pub fn adding(self, seconds: f64) -> Self {
        Self(self.0 + seconds)
    }

    /// `timeIntervalSince(_:)`.
    pub fn seconds_since(self, other: Self) -> f64 {
        self.0 - other.0
    }

    /// `JSONEncoder.DateEncodingStrategy.iso8601`: UTC, whole seconds, the
    /// fraction dropped toward the past (`18:40:00.75` is `18:40:00Z`).
    pub fn iso8601(self) -> String {
        let total = self.unix().floor();
        // i64 holds every date Foundation can print.
        let total = total as i64;
        let days = total.div_euclid(86_400);
        let second_of_day = total.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            second_of_day / 3600,
            second_of_day / 60 % 60,
            second_of_day % 60
        )
    }

    /// What `JSONDecoder.DateDecodingStrategy.iso8601` accepts on macOS 27
    /// (`Date.ISO8601FormatStyle`): `Y-M-DTh:m:s`, each field one or more
    /// digits (`1992-8-9T5:29:13Z`, a five-digit year), the month 1–12 and
    /// the day 1–31 checked but nothing else (`02-30` is March 1st, `25:00`
    /// the next day's 1 a.m.), an optional fraction of 1–9 digits, then a zone
    /// — `Z`, or `±h`, `±hh`, `±hh:m(m)` or `±hhmm` up to 18 hours — after
    /// which anything is ignored. No lower-case `t`/`z`, no space for `T`.
    ///
    /// Before 1583 Foundation counts in the Julian calendar, this in the
    /// proleptic Gregorian one; nothing the app writes is that old.
    pub fn parse_iso8601(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        let mut at = 0;
        let digits = |at: &mut usize, max: usize| -> Option<(i64, usize)> {
            let start = *at;
            while *at < bytes.len() && bytes[*at].is_ascii_digit() && *at - start < max {
                *at += 1;
            }
            let run = &text[start..*at];
            if run.is_empty() {
                return None;
            }
            // Leading zeros are allowed; a run too long for i64 is no date.
            Some((run.parse().ok()?, run.len()))
        };
        let expect = |at: &mut usize, byte: u8| -> Option<()> {
            (bytes.get(*at) == Some(&byte)).then(|| *at += 1)
        };
        let (year, _) = digits(&mut at, 18)?;
        expect(&mut at, b'-')?;
        let (month, _) = digits(&mut at, 18)?;
        expect(&mut at, b'-')?;
        let (day, _) = digits(&mut at, 18)?;
        expect(&mut at, b'T')?;
        let (hour, _) = digits(&mut at, 18)?;
        expect(&mut at, b':')?;
        let (minute, _) = digits(&mut at, 18)?;
        expect(&mut at, b':')?;
        let (second, _) = digits(&mut at, 18)?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let mut fraction = 0.0;
        if bytes.get(at) == Some(&b'.') {
            at += 1;
            let start = at;
            while at < bytes.len() && bytes[at].is_ascii_digit() {
                at += 1;
            }
            let run = &text[start..at];
            if run.is_empty() || run.len() > 9 {
                return None;
            }
            fraction = run.parse::<f64>().ok()? / 10f64.powi(i32::try_from(run.len()).ok()?);
        }
        let offset = match bytes.get(at)? {
            b'Z' => 0,
            sign @ (b'+' | b'-') => {
                let negative = *sign == b'-';
                at += 1;
                let (hours, _) = digits(&mut at, 2)?;
                if bytes.get(at) == Some(&b':') {
                    at += 1;
                }
                let minutes = if bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    digits(&mut at, 2)?.0
                } else {
                    0
                };
                let offset = hours * 3600 + minutes * 60;
                if minutes > 59 || offset > 18 * 3600 {
                    return None;
                }
                if negative { -offset } else { offset }
            }
            _ => return None,
        };
        let days = days_from_civil(year, month, 1).checked_add(day - 1)?;
        let whole = days
            .checked_mul(86_400)?
            .checked_add(hour.checked_mul(3600)?)?
            .checked_add(minute.checked_mul(60)?)?
            .checked_add(second)?
            .checked_sub(offset)?;
        // Seconds since the reference date first, then the fraction: the
        // order Foundation's arithmetic rounds in.
        Some(Self((whole - REFERENCE_UNIX as i64) as f64 + fraction))
    }
}

impl fmt::Debug for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Date({} = {})", self.0, self.iso8601())
    }
}

impl Serialize for Date {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for Date {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        f64::deserialize(deserializer).map(Self)
    }
}

/// (De)serializes a [`Date`] the `.iso8601` way, as the install manifest
/// stores it.
pub mod iso8601 {
    use super::Date;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(date: &Date, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&date.iso8601())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Date, D::Error> {
        let text = String::deserialize(deserializer)?;
        Date::parse_iso8601(&text).ok_or_else(|| {
            serde::de::Error::custom("Expected date string to be ISO8601-formatted.")
        })
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_matches_foundation() {
        // JSONEncoder(.iso8601) printed this for Date(timeIntervalSince1970:
        // 1762800000.75) on macOS 27.
        assert_eq!(
            Date::from_unix(1_762_800_000.75).iso8601(),
            "2025-11-10T18:40:00Z"
        );
        assert_eq!(Date::from_unix(0.0).iso8601(), "1970-01-01T00:00:00Z");
        assert_eq!(Date(0.0).iso8601(), "2001-01-01T00:00:00Z");
        assert_eq!(
            Date::from_unix(951_782_400.0).iso8601(),
            "2000-02-29T00:00:00Z"
        );
    }

    #[test]
    fn iso8601_round_trips_and_takes_zones() {
        for unix in [
            0.0,
            951_782_400.0,
            1_762_800_000.0,
            -86_401.0,
            4_102_444_800.0,
        ] {
            let date = Date::from_unix(unix);
            assert_eq!(Date::parse_iso8601(&date.iso8601()), Some(date));
        }
        assert_eq!(
            Date::parse_iso8601("2025-11-11T02:40:00+08:00"),
            Some(Date::from_unix(1_762_800_000.0))
        );
        assert_eq!(
            Date::parse_iso8601("2025-11-10T13:40:00-0500"),
            Some(Date::from_unix(1_762_800_000.0))
        );
        for bad in [
            "2025-11-10T18:40:00",
            "2025-11-10t18:40:00Z",
            "2025-13-10T18:40:00Z",
            "2025-11-32T18:40:00Z",
            "2025-11-10 18:40:00Z",
            "2025-11-10T18:40:00.Z",
            "2025-11-10T18:40:00,5Z",
            "2025-11-10T18:40:00.1234567891Z",
            "2025-11-10T18:40:00+18:01",
            "+2025-11-10T18:40:00Z",
            " 2025-11-10T18:40:00Z",
            "",
        ] {
            assert_eq!(Date::parse_iso8601(bad), None, "{bad}");
        }
        // Foundation's leniency, as measured.
        let unix = |text: &str| Date::parse_iso8601(text).map(Date::unix);
        assert_eq!(unix("1992-08-09T05:29:13.5Z"), Some(713_338_153.5));
        assert_eq!(unix("1992-8-9T5:29:13Z"), Some(713_338_153.0));
        assert_eq!(unix("1992-08-09T24:00:00Z"), Some(713_404_800.0));
        assert_eq!(unix("1992-02-30T05:29:13Z"), Some(699_427_753.0));
        assert_eq!(unix("1992-08-09T05:29:60Z"), Some(713_338_200.0));
        assert_eq!(unix("1992-08-09T05:29:13+08"), Some(713_309_353.0));
        assert_eq!(unix("1992-08-09T05:29:13+8:00"), Some(713_309_353.0));
        assert_eq!(unix("1992-08-09T05:29:13+0830"), Some(713_307_553.0));
        assert_eq!(unix("1992-08-09T05:29:13+18:00"), Some(713_273_353.0));
        assert_eq!(unix("1992-08-09T05:29:13Zx"), Some(713_338_153.0));
        assert_eq!(unix("10000-08-09T05:29:13Z"), Some(253_421_414_953.0));
    }

    #[test]
    fn reference_date_is_foundations() {
        assert_eq!(
            Date::from_unix(1_755_900_000.0).since_reference(),
            777_592_800.0
        );
        assert_eq!(Date(777_592_800.0).unix(), 1_755_900_000.0);
    }
}
