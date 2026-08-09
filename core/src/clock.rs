//! Minimal UTC clock helpers (RFC 3339) with no external dependencies.
//!
//! Used to write human-readable timestamps into instance configs.

const SECS_PER_DAY: i64 = 86_400;

/// Days since 1970-01-01 for a proleptic Gregorian civil date.
/// Howard Hinnant's `days_from_civil` algorithm.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date `(year, month, day)` for days since 1970-01-01.
/// Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// Format Unix epoch seconds as an RFC 3339 UTC timestamp
/// (e.g. `2026-08-10T12:34:56Z`).
#[must_use]
pub fn rfc3339_utc(secs: i64) -> String {
    let days = secs.div_euclid(SECS_PER_DAY);
    let rem = secs.rem_euclid(SECS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Parse an RFC 3339 UTC timestamp written by [`rfc3339_utc`] back into Unix
/// epoch seconds. Returns `None` for anything outside the fixed
/// `YYYY-MM-DDTHH:MM:SSZ` format (offset timestamps are not supported).
#[must_use]
pub fn parse_rfc3339_utc(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() != 20 || b[10] != b'T' || b[19] != b'Z' {
        return None;
    }
    let num = |i: usize, n: usize| -> Option<i64> {
        let mut value = 0i64;
        for &byte in &b[i..i + n] {
            if !byte.is_ascii_digit() {
                return None;
            }
            value = value * 10 + i64::from(byte - b'0');
        }
        Some(value)
    };
    let year = num(0, 4)?;
    let month = num(5, 2)?;
    let day = num(8, 2)?;
    let hour = num(11, 2)?;
    let minute = num(14, 2)?;
    let second = num(17, 2)?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * SECS_PER_DAY + hour * 3600 + minute * 60 + second)
}

/// Current Unix time in seconds.
#[must_use]
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Current time as an RFC 3339 UTC string.
#[must_use]
pub fn now_rfc3339() -> String {
    rfc3339_utc(now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_735_315_407), "2024-12-27T16:03:27Z");
        assert_eq!(rfc3339_utc(-86_400), "1969-12-31T00:00:00Z");
    }

    #[test]
    fn parses_what_it_formats() {
        for secs in [0, 86_399, 1_735_315_407, i64::from(u32::MAX)] {
            let text = rfc3339_utc(secs);
            assert_eq!(parse_rfc3339_utc(&text), Some(secs), "{text}");
        }
    }

    #[test]
    fn rejects_malformed_timestamps() {
        assert!(parse_rfc3339_utc("2024-13-01T00:00:00Z").is_none());
        assert!(parse_rfc3339_utc("2024-00-01T00:00:00Z").is_none());
        assert!(parse_rfc3339_utc("2024-01-32T00:00:00Z").is_none());
        assert!(parse_rfc3339_utc("2024-01-01T24:00:00Z").is_none());
        assert!(parse_rfc3339_utc("2024-01-01T00:00:00+02:00").is_none());
        assert!(parse_rfc3339_utc("not a date").is_none());
    }
}
